//! Tier 2 — `xdp_reverse_nat_lookup` UDP (proto=17) reverse-NAT
//! `BPF_PROG_TEST_RUN` triptych (udp-service-support US-03; ADR-0060
//! § Enforcement Tier 2; K3).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-03-E: a populated REVERSE_NAT_MAP (ip,port,udp)→vip rewrites a
//!   proto=17 response's source 5-tuple to the VIP and returns the
//!   reverse-NAT egress verdict (`XDP_REDIRECT` on a routable FIB hit,
//!   identical to the TCP triptych's hit assertion).
//! - S-03-F: a REVERSE_NAT_MAP miss for a udp packet returns XDP_PASS
//!   with the frame byte-identical (no rewrite, no DROP_COUNTER slot).
//!
//! Mirrors the TCP triptych at `xdp_reverse_nat_redirect_neigh.rs`
//! verbatim — same loader plumbing, same `bpf_prog_test_run` helper,
//! same `BackendKey`/`Vip` POD layout — with the IPv4 proto byte = 17
//! (UDP) and an 8-byte UDP header in place of the 20-byte TCP header.
//! The kernel-side `xdp_reverse_nat_lookup` is already proto-aware
//! (reads the packet's proto byte, keys REVERSE_NAT_MAP on
//! `(ip, port, proto)`, and applies the RFC 768 UDP-checksum rule); the
//! proto=17 path is the kernel half of the lockstep step 01-01 narrowed
//! at Tier 1 (Sim) + core.
//!
//! Tier 2 (layer 3) — example-only per Mandate 11; no PBT machinery.
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall (the whole
//! `tests/integration` binary is gated behind `integration-tests` in
//! `integration.rs`).

// See `xdp_pass_test_run.rs` and `xdp_reverse_nat_redirect_neigh.rs`
// for the full rationale — Tier 2 BPF unit tests work directly with the
// `bpf(2)` syscall surface (raw FD <-> u32 casts, raw pointer borrows
// for syscall arg buffers, kernel POD structs). Pedantic lints flag
// these patterns; allow scoped to the test crate.
#![allow(
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::items_after_statements,
    clippy::doc_markdown,
    clippy::explicit_counter_loop,
    clippy::explicit_iter_loop
)]

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;

use aya::{
    Ebpf, EbpfLoader,
    maps::HashMap,
    programs::{ProgramFd, Xdp},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

/// XDP verdict constants — kernel ABI from `<bpf.h>` (`enum xdp_action`).
const XDP_PASS: u32 = 2;
const XDP_REDIRECT: u32 = 4;

// ----- syscall helper -----

/// Drive `BPF_PROG_TEST_RUN`. Returns `(retval, data_out_len)`.
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

// ----- header layout -----

const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const UDP_HDR_LEN: usize = 8;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + UDP_HDR_LEN;

/// Backend → VIP rewrite test fixture. Same shape as the TCP triptych;
/// only the L4 protocol differs.
///
/// `ROUTABLE_CLIENT_OCTETS` is the response *destination* — to make
/// `bpf_fib_lookup` return `RET_SUCCESS` the client IP needs to be
/// routable on the test host. `10.0.0.100` lands on Lima's default
/// route, identical strategy to the TCP triptych.
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const ROUTABLE_CLIENT_OCTETS: [u8; 4] = [10, 0, 0, 100];
const CLIENT_PORT: u16 = 12345;

/// IPv4 protocol — UDP for these tests; matches the kernel-side
/// `BackendKey { proto }` field and drives the program's `is_udp`
/// branch (RFC 768 UDP-checksum handling).
const IPV4_PROTO_UDP: u8 = 17;

/// PKTGEN — a backend UDP response from `BACKEND_OCTETS:BACKEND_PORT`
/// to `dst_octets:CLIENT_PORT` (IPv4 proto=17). 8-byte UDP header.
fn synthesise_backend_response(dst_octets: [u8; 4]) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet (14B): dst MAC, src MAC, ethertype 0x0800 (IPv4).
    pkt[0..6].copy_from_slice(&SENTINEL_DST_MAC);
    pkt[6..12].copy_from_slice(&SENTINEL_SRC_MAC);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]);

    // IPv4 (20B):
    let ip = ETH_HDR_LEN;
    pkt[ip] = 0x45; // ver=4, IHL=5
    pkt[ip + 1] = 0x00; // TOS
    let total_len: u16 = (IPV4_HDR_LEN + UDP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes()); // id
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes()); // flags+frag
    pkt[ip + 8] = 0x40; // TTL=64
    pkt[ip + 9] = IPV4_PROTO_UDP;
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum (filled below)
    pkt[ip + 12..ip + 16].copy_from_slice(&BACKEND_OCTETS); // src IP (backend)
    pkt[ip + 16..ip + 20].copy_from_slice(&dst_octets); // dst IP (client)

    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // UDP (8B): src port, dst port, length, checksum.
    let udp = ip + IPV4_HDR_LEN;
    pkt[udp..udp + 2].copy_from_slice(&BACKEND_PORT.to_be_bytes()); // src port (backend)
    pkt[udp + 2..udp + 4].copy_from_slice(&CLIENT_PORT.to_be_bytes()); // dst port (client)
    pkt[udp + 4..udp + 6].copy_from_slice(&(UDP_HDR_LEN as u16).to_be_bytes()); // length
    pkt[udp + 6..udp + 8].copy_from_slice(&0u16.to_be_bytes()); // checksum (filled below)

    let udp_csum =
        udp_checksum(&pkt[ip + 12..ip + 16], &pkt[ip + 16..ip + 20], &pkt[udp..udp + UDP_HDR_LEN]);
    // RFC 768: a computed UDP checksum of 0 is transmitted as 0xffff.
    let udp_csum = if udp_csum == 0 { 0xffff } else { udp_csum };
    pkt[udp + 6..udp + 8].copy_from_slice(&udp_csum.to_be_bytes());

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

fn udp_checksum(src_ip: &[u8], dst_ip: &[u8], udp: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in [src_ip, dst_ip].iter() {
        for w in chunk.chunks(2) {
            sum += u32::from(u16::from_be_bytes([w[0], w[1]]));
        }
    }
    sum += u32::from(u16::from(IPV4_PROTO_UDP)); // proto = UDP (17)
    sum += u32::from(udp.len() as u16);
    let mut i = 0;
    while i + 1 < udp.len() {
        sum += u32::from(u16::from_be_bytes([udp[i], udp[i + 1]]));
        i += 2;
    }
    if i < udp.len() {
        sum += u32::from(u16::from_be_bytes([udp[i], 0]));
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Sentinel source/dest MACs the PKTGEN ships in the eth_hdr. After a
/// `bpf_fib_lookup` success the program rewrites these to the
/// FIB-resolved smac/dmac, so observing the value differ post-test is
/// the L2-rewrite assertion.
const SENTINEL_SRC_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xab, 0xcd, 0xef];
const SENTINEL_DST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// `BackendKey` — host-order POD matching the kernel-side struct in
/// `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`. 8 bytes:
/// `ip_host` (u32) + `port_host` (u16) + `proto` (u8) + `_pad` (u8).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct BackendKey {
    ip_host: u32,
    port_host: u16,
    proto: u8,
    _pad: u8,
}
// SAFETY: `repr(C)`; we always set `_pad = 0`; no padding-uninit
// issues for our writes.
unsafe impl aya::Pod for BackendKey {}

/// `Vip` — host-order POD matching the kernel-side `Vip` value type
/// in `reverse_nat_map.rs`. 8 bytes: `ip_host` (u32) + `port_host`
/// (u16) + `_pad` (u16).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Vip {
    ip_host: u32,
    port_host: u16,
    _pad: u16,
}
unsafe impl aya::Pod for Vip {}

// ----- loader plumbing -----
//
// The same `overdrive_bpf.o` ELF carries every Phase 2.2 program, which
// means the SERVICE_MAP HoM declaration is present in the ELF even
// though the reverse-NAT XDP program does not consume it. Aya's loader
// still tries to instantiate every `#[map]` it finds — and
// BPF_MAP_TYPE_HASH_OF_MAPS creation rejects without an `inner_map_fd`
// from the loader's side. The pin-by-name workaround from
// `.claude/rules/development.md` § "Sharing the outer HoM ..." is
// therefore mandatory for ANY test that loads this ELF.

struct LoadedTestBpf {
    bpf: Ebpf,
    prog_fd: ProgramFd,
    _outer_fd: OwnedFd,
    _pin_dir_guard: PinDirGuard,
}

struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load_reverse_nat_xdp_program() -> LoadedTestBpf {
    let artifact = super::bpf_artifact::path();

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-xrnat-udp-tier2-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let pin_dir_guard = PinDirGuard(pin_dir.clone());

    let outer_fd = pre_pin_service_map(&pin_dir);

    let mut bpf = EbpfLoader::new()
        .map_pin_path(&pin_dir)
        .allow_unsupported_maps()
        .load_file(&artifact)
        .unwrap_or_else(|e| panic!("aya load_file({}): {e}", artifact.display()));

    let prog_fd = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_reverse_nat_lookup")
            .expect("xdp_reverse_nat_lookup program not found in BPF object")
            .try_into()
            .expect("xdp_reverse_nat_lookup is not an Xdp program");
        prog.load().expect("xdp_reverse_nat_lookup.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    LoadedTestBpf { bpf, prog_fd, _outer_fd: outer_fd, _pin_dir_guard: pin_dir_guard }
}

/// Pre-create + pre-pin the SERVICE_MAP outer HoM at
/// `<pin_dir>/SERVICE_MAP`. Returns the userspace-owned outer FD.
/// Mirrors the helper in the TCP triptych verbatim — the shape is
/// shared across every test that loads `overdrive_bpf.o`.
fn pre_pin_service_map(pin_dir: &std::path::Path) -> OwnedFd {
    use std::ffi::CString;
    use std::mem;
    use std::os::fd::FromRawFd;

    use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

    const BPF_MAP_CREATE: c_long = 0;
    const BPF_OBJ_PIN: c_long = 6;
    const BPF_MAP_TYPE_ARRAY: u32 = 2;
    const BPF_MAP_TYPE_HASH_OF_MAPS: u32 = 13;

    #[repr(C)]
    #[derive(Default)]
    struct CreateAttr {
        map_type: u32,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
        map_flags: u32,
        inner_map_fd: u32,
        numa_node: u32,
        map_name: [u8; 16],
        map_ifindex: u32,
        btf_fd: u32,
        btf_key_type_id: u32,
        btf_value_type_id: u32,
        _pad: [u8; 32],
    }
    #[repr(C)]
    #[derive(Default)]
    struct PinAttr {
        pathname: u64,
        bpf_fd: u32,
        file_flags: u32,
    }

    fn raw_bpf(cmd: c_long, attr: *const c_void, size: c_int) -> i64 {
        // SAFETY: `attr` valid `bpf_attr` of `size` bytes.
        unsafe { syscall(SYS_bpf, cmd, attr, size) as i64 }
    }

    let inner_attr = CreateAttr {
        map_type: BPF_MAP_TYPE_ARRAY,
        key_size: mem::size_of::<u32>() as u32,
        value_size: mem::size_of::<u32>() as u32,
        max_entries: overdrive_core::dataplane::MaglevTableSize::DEFAULT.get(),
        ..Default::default()
    };
    let inner_raw = raw_bpf(
        BPF_MAP_CREATE,
        &inner_attr as *const _ as *const c_void,
        mem::size_of::<CreateAttr>() as c_int,
    );
    assert!(inner_raw >= 0, "inner ARRAY prototype create: {}", std::io::Error::last_os_error());
    // SAFETY: kernel-issued FD.
    let inner_fd = unsafe { OwnedFd::from_raw_fd(inner_raw as c_int) };

    let outer_attr = CreateAttr {
        map_type: BPF_MAP_TYPE_HASH_OF_MAPS,
        key_size: 8,
        value_size: mem::size_of::<u32>() as u32,
        max_entries: 4096,
        inner_map_fd: inner_fd.as_raw_fd() as u32,
        ..Default::default()
    };
    let outer_raw = raw_bpf(
        BPF_MAP_CREATE,
        &outer_attr as *const _ as *const c_void,
        mem::size_of::<CreateAttr>() as c_int,
    );
    assert!(outer_raw >= 0, "outer HoM create: {}", std::io::Error::last_os_error());
    // SAFETY: kernel-issued FD.
    let outer_fd = unsafe { OwnedFd::from_raw_fd(outer_raw as c_int) };

    let pin_path = pin_dir.join("SERVICE_MAP");
    let cstr = CString::new(pin_path.as_os_str().to_string_lossy().as_bytes())
        .expect("pin path must not contain NUL byte");
    let pin_attr = PinAttr {
        pathname: cstr.as_ptr() as u64,
        bpf_fd: outer_fd.as_raw_fd() as u32,
        file_flags: 0,
    };
    let rc = raw_bpf(
        BPF_OBJ_PIN,
        &pin_attr as *const _ as *const c_void,
        mem::size_of::<PinAttr>() as c_int,
    );
    assert!(rc >= 0, "BPF_OBJ_PIN({}): {}", pin_path.display(), std::io::Error::last_os_error());

    outer_fd
}

/// Idempotent map clear — removes the `(BACKEND_OCTETS, BACKEND_PORT,
/// UDP)` entry from REVERSE_NAT_MAP regardless of prior state.
fn clear_reverse_nat_entry(bpf: &mut Ebpf, key: BackendKey) {
    let mut rnm: HashMap<_, BackendKey, Vip> =
        HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
            .expect("REVERSE_NAT_MAP HashMap::try_from");
    let _ = rnm.remove(&key);
}

/// Common SETUP — populate REVERSE_NAT_MAP with the `(backend, UDP)`
/// → VIP mapping the reverse-NAT path expects.
fn populate_reverse_nat(bpf: &mut Ebpf) {
    let key = BackendKey {
        ip_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_UDP,
        _pad: 0,
    };
    let value = Vip {
        ip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
        port_host: VIP_PORT,
        _pad: 0,
    };
    let mut rnm: HashMap<_, BackendKey, Vip> =
        HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
            .expect("REVERSE_NAT_MAP HashMap::try_from");
    rnm.insert(key, value, 0).expect("REVERSE_NAT_MAP insert");
}

// ----- tests -----

/// S-03-E — REVERSE_NAT-hit branch for a proto=17 UDP response. A
/// REVERSE_NAT_MAP hit on a routable client destination MUST return
/// the reverse-NAT egress verdict (`XDP_REDIRECT` = 4 on a FIB hit,
/// identical to the TCP triptych) AND:
///
/// - The IPv4 source IP MUST be rewritten from BACKEND_OCTETS to
///   VIP_OCTETS.
/// - The UDP source port MUST be rewritten from BACKEND_PORT to
///   VIP_PORT.
/// - The IPv4 checksum MUST be valid post-rewrite.
/// - The Ethernet h_dest / h_source MUST be rewritten away from the
///   PKTGEN sentinels by the in-program L2 MAC memcpy.
///
/// On Lima the FIB resolves `ROUTABLE_CLIENT_OCTETS` via the default
/// route's gateway with a populated neighbour cache; `bpf_fib_lookup`
/// returns `RET_SUCCESS` and the program returns `XDP_REDIRECT`.
#[test]
#[serial(env)]
fn udp_response_source_rewritten_to_vip_on_reverse_nat_hit() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_reverse_nat_xdp_program();

    populate_reverse_nat(&mut bpf);

    let pkt = synthesise_backend_response(ROUTABLE_CLIENT_OCTETS);
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    assert_eq!(
        action, XDP_REDIRECT,
        "expected reverse-NAT egress verdict XDP_REDIRECT (=4) on a proto=17 FIB hit, got {action}",
    );

    // (a) Source IP rewritten to VIP.
    let ip_src = &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16];
    assert_eq!(ip_src, &VIP_OCTETS, "UDP source IP not rewritten to VIP");

    // (b) Source port rewritten (UDP src port is at L4 offset 0).
    let udp = ETH_HDR_LEN + IPV4_HDR_LEN;
    let src_port = u16::from_be_bytes([out[udp], out[udp + 1]]);
    assert_eq!(src_port, VIP_PORT, "UDP source port not rewritten to VIP port");

    // (c) IPv4 header checksum is valid post-rewrite.
    let recomputed_ip_csum = ipv4_header_checksum(&out[ETH_HDR_LEN..ETH_HDR_LEN + IPV4_HDR_LEN]);
    assert_eq!(recomputed_ip_csum, 0, "IPv4 checksum invalid after rewrite");

    // (d) L2 rewrite assertion: h_dest and h_source MUST differ from
    // the PKTGEN sentinels.
    let h_dest = &out[0..6];
    let h_source = &out[6..12];
    assert_ne!(h_dest, &SENTINEL_DST_MAC, "h_dest not rewritten by FIB lookup");
    assert_ne!(h_source, &SENTINEL_SRC_MAC, "h_source not rewritten by FIB lookup");
}

/// S-03-F — REVERSE_NAT-miss branch for a proto=17 UDP response. When
/// the REVERSE_NAT_MAP lookup misses, the program MUST return
/// `XDP_PASS` with the frame unmodified. No DROP_COUNTER slot is
/// consumed (REVERSE_NAT miss is "not LB traffic" — the kernel
/// networking stack handles it).
///
/// SETUP: REVERSE_NAT_MAP cleared. PKTGEN: a UDP response from
/// `BACKEND_OCTETS:BACKEND_PORT`. CHECK: `XDP_PASS`, output frame ==
/// input frame.
#[test]
#[serial(env)]
fn udp_response_passes_unmodified_on_reverse_nat_miss() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_reverse_nat_xdp_program();

    let key = BackendKey {
        ip_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_UDP,
        _pad: 0,
    };
    clear_reverse_nat_entry(&mut bpf, key);

    let pkt = synthesise_backend_response(ROUTABLE_CLIENT_OCTETS);
    let mut out = vec![0u8; pkt.len()];

    let (action, _) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, XDP_PASS, "expected XDP_PASS (=2) on REVERSE_NAT miss, got {action}");

    // No rewrite means the output bytes match the input bytes exactly.
    assert_eq!(out, pkt, "REVERSE_NAT-miss path must not modify the frame");
}
