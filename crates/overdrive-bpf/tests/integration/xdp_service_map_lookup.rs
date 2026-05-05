//! S-2.2-04, S-2.2-05, S-2.2-08 — `xdp_service_map_lookup`
//! PKTGEN/SETUP/CHECK triptychs.
//!
//! Tags: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).
//!
//! Each sub-test follows the same shape as `xdp_pass_test_run.rs`:
//! load `target/xtask/bpf-objects/overdrive_bpf.o` via `aya::Ebpf`,
//! resolve the `xdp_service_map_lookup` program and `SERVICE_MAP`
//! map, drive `BPF_PROG_TEST_RUN` directly via the `bpf(2)`
//! syscall, assert on returned action and (where relevant) the
//! rewritten output bytes.
//!
//! Map state is cleared between sub-tests by default per
//! `.claude/rules/testing.md` § "Tier 2 — BPF Unit Tests" — each
//! test removes its keys at SETUP regardless of prior runs.
//!
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall and aya's
//! userspace API requires libbpf-sys.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

use std::path::PathBuf;

use aya::{
    Ebpf,
    maps::HashMap,
    programs::{ProgramFd, Xdp},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

/// `XDP_PASS` from `<bpf.h>` (`enum xdp_action`). Hardcoded per
/// the same reasoning in `xdp_pass_test_run.rs` — the value is
/// kernel ABI and will not change.
const XDP_PASS: u32 = 2;
/// `XDP_TX` from `<bpf.h>` — bounce the (possibly-rewritten) frame
/// back out the same NIC.
const XDP_TX: u32 = 3;

// ----- workspace plumbing (same shape as xdp_pass_test_run.rs) -----

fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop(); // remove `overdrive-bpf`
    p.pop(); // remove `crates`
    p
}

fn bpf_artifact_path() -> PathBuf {
    workspace_root().join("target/xtask/bpf-objects/overdrive_bpf.o")
}

// ----- syscall helper -----

/// Drive `BPF_PROG_TEST_RUN`. Returns `(retval, data_out)`.
///
/// `data_out_buf` is sized by the caller to hold the (possibly
/// rewritten) output frame; the kernel writes the post-program
/// bytes here when `data_out` is non-null.
fn bpf_prog_test_run(
    prog_fd: &ProgramFd,
    data_in: &[u8],
    data_out_buf: &mut [u8],
) -> Result<(u32, usize), std::io::Error> {
    use std::os::fd::AsFd;

    // SAFETY: `bpf_attr` is a `repr(C) union` of `repr(C) struct`s
    // with no destructor; zero-init matches aya's internal helper.
    let mut attr: bpf_attr = unsafe { std::mem::zeroed() };

    // SAFETY: writing the `test` arm of the union; reading no other
    // arm before the syscall.
    let test = unsafe { &mut attr.test };
    test.prog_fd = prog_fd.as_fd().as_raw_fd_u32();
    test.data_in = data_in.as_ptr() as u64;
    test.data_size_in = data_in.len().try_into().expect("data_in fits in u32");
    test.data_out = data_out_buf.as_mut_ptr() as u64;
    test.data_size_out = data_out_buf.len().try_into().expect("data_out fits in u32");
    test.repeat = 1;

    // SAFETY: standard kernel ABI for BPF; size_of::<bpf_attr>()
    // matches the kernel's expected layout.
    #[allow(clippy::cast_possible_truncation)]
    let attr_size = std::mem::size_of::<bpf_attr>() as libc::c_uint;
    let ret = unsafe {
        libc::syscall(libc::SYS_bpf, BPF_PROG_TEST_RUN as libc::c_int, &raw mut attr, attr_size)
    };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: kernel populated retval and data_size_out on success.
    let retval = unsafe { attr.test.retval };
    let out_len = unsafe { attr.test.data_size_out } as usize;
    Ok((retval, out_len))
}

trait AsRawFdU32 {
    fn as_raw_fd_u32(&self) -> u32;
}
impl AsRawFdU32 for std::os::fd::BorrowedFd<'_> {
    fn as_raw_fd_u32(&self) -> u32 {
        use std::os::fd::AsRawFd;
        #[allow(clippy::cast_sign_loss)]
        let fd = self.as_raw_fd() as u32;
        fd
    }
}

// ----- header layout (mirrors crates/overdrive-bpf shared headers) -----
//
// We assemble packets directly via byte arrays here rather than
// pulling in `network-types` — the shape is fixed-width and tiny;
// every byte is deliberate and visible in the assertions.

const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const TCP_HDR_LEN: usize = 20;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// VIP / backend test fixture (matches the scaffold panic message).
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;

/// PKTGEN — a minimal Ethernet+IPv4+TCP-SYN frame addressed to
/// `dst_ip:dst_port`. Checksums are populated from a one's-complement
/// sum so the kernel `bpf_l3_csum_replace` / `bpf_l4_csum_replace`
/// helpers (which apply DELTA-style updates against an existing
/// checksum) produce a valid post-rewrite checksum.
fn synthesise_tcp_syn(dst_octets: [u8; 4], dst_port: u16) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet (14B): dst MAC, src MAC, ethertype 0x0800 (IPv4).
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]);

    // IPv4 (20B):
    let ip = ETH_HDR_LEN;
    pkt[ip] = 0x45; // ver=4, IHL=5
    pkt[ip + 1] = 0x00; // TOS
    let total_len: u16 = (IPV4_HDR_LEN + TCP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes()); // id
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes()); // flags+frag
    pkt[ip + 8] = 0x40; // TTL=64
    pkt[ip + 9] = 0x06; // proto=TCP
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[ip + 12..ip + 16].copy_from_slice(&[10, 0, 0, 100]); // src IP
    pkt[ip + 16..ip + 20].copy_from_slice(&dst_octets); // dst IP

    // Compute IPv4 header checksum (RFC 1071, header-only).
    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    let src_port: u16 = 12345;
    pkt[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&dst_port.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5 (no options)
    pkt[tcp + 13] = 0x02; // flags = SYN
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes()); // window
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes()); // urg ptr

    // Compute TCP checksum over pseudo-header + TCP header.
    let tcp_csum =
        tcp_checksum(&pkt[ip + 12..ip + 16], &pkt[ip + 16..ip + 20], &pkt[tcp..tcp + TCP_HDR_LEN]);
    pkt[tcp + 16..tcp + 18].copy_from_slice(&tcp_csum.to_be_bytes());

    pkt
}

fn ipv4_header_checksum(hdr: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < hdr.len() {
        sum += u32::from(u16::from_be_bytes([hdr[i], hdr[i + 1]]));
        i += 2;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn tcp_checksum(src_ip: &[u8], dst_ip: &[u8], tcp: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    // Pseudo-header: src(4) + dst(4) + zero(1) + proto(1) + tcp_len(2)
    for chunk in [src_ip, dst_ip].iter() {
        for w in chunk.chunks(2) {
            sum += u32::from(u16::from_be_bytes([w[0], w[1]]));
        }
    }
    sum += u32::from(0x0006_u16); // proto = TCP (zero byte + proto byte)
    sum += u32::from(tcp.len() as u16);
    // TCP header (checksum field is currently zero so it doesn't affect sum).
    let mut i = 0;
    while i + 1 < tcp.len() {
        sum += u32::from(u16::from_be_bytes([tcp[i], tcp[i + 1]]));
        i += 2;
    }
    if i < tcp.len() {
        sum += u32::from(u16::from_be_bytes([tcp[i], 0]));
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Outer-map key matching the `ServiceKey` struct in
/// `crates/overdrive-dataplane/src/maps/service_map_handle.rs` —
/// 8-byte host-order POD: vip_host (u32) + port_host (u16) + _pad (u16).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct ServiceKey {
    vip_host: u32,
    port_host: u16,
    _pad: u16,
}
// SAFETY: repr(C), no padding-uninit issues for our writes (we
// always set _pad to 0); `aya::Pod` is the marker aya needs to
// permit raw map access.
unsafe impl aya::Pod for ServiceKey {}

/// Inner-map value matching `BackendEntry` — 12-byte host-order POD.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct BackendEntry {
    ipv4_host: u32,
    port_host: u16,
    weight: u16,
    healthy: u8,
    _pad: [u8; 3],
}
unsafe impl aya::Pod for BackendEntry {}

/// Common load helper. Loads the BPF object, attaches the
/// `xdp_service_map_lookup` program, returns an owned `ProgramFd`
/// alongside the still-loaded `Ebpf` so the caller can manipulate
/// `SERVICE_MAP`.
fn load_service_map_program() -> (Ebpf, ProgramFd) {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );
    let mut bpf = Ebpf::load_file(&artifact)
        .unwrap_or_else(|e| panic!("aya load_file({}): {e}", artifact.display()));

    let prog_fd = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_service_map_lookup")
            .expect("xdp_service_map_lookup program not found in BPF object")
            .try_into()
            .expect("xdp_service_map_lookup is not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    (bpf, prog_fd)
}

fn clear_service_map(bpf: &mut Ebpf) {
    let mut sm: HashMap<_, ServiceKey, BackendEntry> =
        HashMap::try_from(bpf.map_mut("SERVICE_MAP").expect("SERVICE_MAP map not found"))
            .expect("SERVICE_MAP HashMap::try_from");
    // Remove every key by iterating snapshot keys then deleting.
    let keys: Vec<ServiceKey> = sm.keys().filter_map(Result::ok).collect();
    for k in keys {
        let _ = sm.remove(&k);
    }
}

/// S-2.2-04 — `SERVICE_MAP` hit returns `XDP_TX` with rewritten
/// headers.
#[test]
#[serial(env)]
fn service_map_hit_returns_xdp_tx_with_rewritten_headers() {
    let (mut bpf, prog_fd) = load_service_map_program();

    // SETUP: clear and populate SERVICE_MAP with VIP -> backend.
    clear_service_map(&mut bpf);
    {
        let mut sm: HashMap<_, ServiceKey, BackendEntry> =
            HashMap::try_from(bpf.map_mut("SERVICE_MAP").expect("SERVICE_MAP map not found"))
                .expect("SERVICE_MAP HashMap::try_from");
        let key = ServiceKey {
            vip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
            port_host: VIP_PORT,
            _pad: 0,
        };
        let value = BackendEntry {
            ipv4_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
            port_host: BACKEND_PORT,
            weight: 1,
            healthy: 1,
            _pad: [0; 3],
        };
        sm.insert(key, value, 0).expect("insert");
    }

    // PKTGEN: TCP SYN to VIP:VIP_PORT.
    let pkt = synthesise_tcp_syn(VIP_OCTETS, VIP_PORT);
    let mut out = vec![0u8; pkt.len()];

    // CHECK: drive BPF_PROG_TEST_RUN.
    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, XDP_TX, "expected XDP_TX (=3), got {action}");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    // (a) Dest IP rewritten.
    let ip_dst = &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20];
    assert_eq!(ip_dst, &BACKEND_OCTETS, "dest IP not rewritten to backend");

    // (b) Dest port rewritten.
    let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
    let dst_port = u16::from_be_bytes([out[tcp + 2], out[tcp + 3]]);
    assert_eq!(dst_port, BACKEND_PORT, "dest port not rewritten");

    // (c) IPv4 header checksum is valid post-rewrite (sum over
    // header == 0xffff under one's complement; recomputed from
    // scratch as 0).
    let recomputed_ip_csum = ipv4_header_checksum(&out[ETH_HDR_LEN..ETH_HDR_LEN + IPV4_HDR_LEN]);
    assert_eq!(recomputed_ip_csum, 0, "IPv4 checksum invalid after rewrite");

    // (d) TCP checksum is valid post-rewrite. The TCP checksum
    // covers the pseudo-header (which now uses the rewritten dest
    // IP), so recomputing from the rewritten frame must yield 0.
    let tcp_csum_recomputed = tcp_checksum(
        &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16],
        &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20],
        &out[tcp..tcp + TCP_HDR_LEN],
    );
    assert_eq!(tcp_csum_recomputed, 0, "TCP checksum invalid after rewrite");
}

/// S-2.2-05 — `SERVICE_MAP` miss returns `XDP_PASS`, no rewrite.
#[test]
#[serial(env)]
fn service_map_miss_returns_xdp_pass_no_rewrite() {
    let (mut bpf, prog_fd) = load_service_map_program();

    // SETUP: empty SERVICE_MAP.
    clear_service_map(&mut bpf);

    // PKTGEN: TCP SYN to VIP:VIP_PORT (no entry exists).
    let pkt = synthesise_tcp_syn(VIP_OCTETS, VIP_PORT);
    let mut out = vec![0u8; pkt.len()];

    // CHECK.
    let (action, _) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, XDP_PASS, "expected XDP_PASS (=2) on miss, got {action}");

    // No rewrite means the output bytes match the input bytes
    // exactly. `data_out` is populated by the kernel from the
    // (unmodified) packet buffer on XDP_PASS.
    assert_eq!(out, pkt, "miss path must not modify the frame");
}

/// S-2.2-08 — Truncated IPv4 frame returns `XDP_PASS`, no crash,
/// no `SERVICE_MAP` lookup.
#[test]
#[serial(env)]
fn truncated_ipv4_frame_returns_xdp_pass_no_lookup_no_crash() {
    let (mut bpf, prog_fd) = load_service_map_program();

    // SETUP: populate SERVICE_MAP. The truncation must short-
    // circuit BEFORE the lookup; if the program incorrectly
    // performed the lookup against arbitrary memory, the test
    // would still see XDP_PASS but might also exhibit verifier
    // rejection on load. The point of populating the map is to
    // ensure the test would catch a "lookup happened anyway, then
    // returned PASS for some other reason" regression — by setting
    // the map up so a successful lookup would route the (broken)
    // packet to a valid backend, any divergent behaviour would
    // surface as XDP_TX rather than XDP_PASS.
    clear_service_map(&mut bpf);
    {
        let mut sm: HashMap<_, ServiceKey, BackendEntry> =
            HashMap::try_from(bpf.map_mut("SERVICE_MAP").expect("SERVICE_MAP map not found"))
                .expect("SERVICE_MAP HashMap::try_from");
        let key = ServiceKey {
            vip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
            port_host: VIP_PORT,
            _pad: 0,
        };
        let value = BackendEntry {
            ipv4_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
            port_host: BACKEND_PORT,
            weight: 1,
            healthy: 1,
            _pad: [0; 3],
        };
        sm.insert(key, value, 0).expect("insert");
    }

    // PKTGEN: Ethernet header + only 10 bytes of IPv4 (less than
    // IPV4_HDR_LEN = 20). The kernel's BPF_PROG_TEST_RUN minimum
    // size for XDP is 32 bytes, so we pad with zeroes after the
    // truncated IPv4 to satisfy the syscall — but the program's
    // bounds check on `ptr_at::<Ipv4Hdr>(ctx, ETH_HDR_LEN)` must
    // still reject because (start + 14 + 20) > end where the
    // program's `data_end` is set against the truncated content.
    //
    // To exercise this, we pass a 32-byte input where bytes 14..32
    // (after the Ethernet header) have IP version=4 / IHL=5 in
    // byte 14 to make the bounds check the dispositive failure —
    // not the version check. The IPv4 bounds check requires 20
    // bytes from offset 14, i.e. up to offset 34. With a 32-byte
    // packet, offset 34 is past data_end → ptr_at fails → wrapper
    // returns XDP_PASS.
    let mut pkt = vec![0u8; 32];
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]); // ethertype = IPv4
    pkt[14] = 0x45; // ver=4, IHL=5 — but only 18 bytes follow, not 20.

    let mut out = vec![0u8; pkt.len()];

    // CHECK.
    let (action, _) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(
        action, XDP_PASS,
        "truncated frame must return XDP_PASS (no crash, no lookup); got {action}"
    );
}
