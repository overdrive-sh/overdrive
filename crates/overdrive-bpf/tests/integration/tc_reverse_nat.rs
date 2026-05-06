//! S-2.2-16 — `tc_reverse_nat` PKTGEN/SETUP/CHECK triptych.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN` for TC programs per
//! `.claude/rules/testing.md` § "Tier 2 — BPF Unit Tests").
//!
//! Each sub-test follows the same shape as `xdp_service_map_lookup.rs`:
//! load `target/xtask/bpf-objects/overdrive_bpf.o` via `aya::Ebpf`,
//! resolve the `tc_reverse_nat` program and `REVERSE_NAT_MAP` map,
//! drive `BPF_PROG_TEST_RUN` directly via the `bpf(2)` syscall, assert
//! on returned action and (where relevant) the rewritten output bytes.
//!
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall and aya's
//! userspace API requires libbpf-sys.

#![cfg(target_os = "linux")]
// See `xdp_service_map_lookup.rs` for the rationale on these allows —
// the Tier 2 BPF unit tests work directly with the `bpf(2)` syscall
// surface.
#![allow(
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::items_after_statements,
    clippy::doc_markdown
)]

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;

use aya::{
    Ebpf, EbpfLoader,
    maps::HashMap,
    programs::{ProgramFd, SchedClassifier},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

/// `TC_ACT_OK` from `<linux/pkt_cls.h>`. Hardcoded per the same
/// reasoning as the XDP verdict constants in
/// `xdp_service_map_lookup.rs` — kernel ABI, will not change.
const TC_ACT_OK: u32 = 0;

// ----- workspace plumbing -----

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

// ----- syscall helper (mirrors xdp_service_map_lookup.rs) -----

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
        #[allow(clippy::cast_sign_loss)]
        let fd = self.as_raw_fd() as u32;
        fd
    }
}

// ----- header layout -----

const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const TCP_HDR_LEN: usize = 20;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// Backend → VIP rewrite test fixture.
///
/// In the reverse-NAT path, the *source* of the response is the backend
/// (which received the rewritten forward-path packet) and we rewrite it
/// back to the VIP so the client sees the response from the address it
/// originally sent to.
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
/// Client (response destination) — arbitrary; not rewritten.
const CLIENT_OCTETS: [u8; 4] = [10, 0, 0, 100];
const CLIENT_PORT: u16 = 12345;
/// IPv4 protocol — TCP for these tests; matches the kernel-side
/// `BackendKey { proto }` field.
const IPV4_PROTO_TCP: u8 = 6;

/// PKTGEN — a backend response from `src_octets:src_port` to
/// `CLIENT_OCTETS:CLIENT_PORT`. The frame is a TCP SYN-ACK shape so
/// that flag-sanity prologues won't reject it.
fn synthesise_backend_response(src_octets: [u8; 4], src_port: u16) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet (14B): dst MAC, src MAC, ethertype 0x0800 (IPv4).
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
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
    pkt[ip + 9] = IPV4_PROTO_TCP;
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[ip + 12..ip + 16].copy_from_slice(&src_octets); // src IP (backend)
    pkt[ip + 16..ip + 20].copy_from_slice(&CLIENT_OCTETS); // dst IP (client)

    // Compute IPv4 header checksum.
    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    pkt[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes()); // src port (backend)
    pkt[tcp + 2..tcp + 4].copy_from_slice(&CLIENT_PORT.to_be_bytes()); // dst port (client)
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5 (no options)
    pkt[tcp + 13] = 0x12; // flags = SYN+ACK
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes()); // window
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes()); // urg ptr

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
    for chunk in [src_ip, dst_ip].iter() {
        for w in chunk.chunks(2) {
            sum += u32::from(u16::from_be_bytes([w[0], w[1]]));
        }
    }
    sum += u32::from(0x0006_u16); // proto = TCP
    sum += u32::from(tcp.len() as u16);
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

/// `Vip` — host-order POD matching the kernel-side `Vip` value type in
/// `reverse_nat_map.rs`. 8 bytes: `ip_host` (u32) + `port_host` (u16) +
/// `_pad` (u16).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Vip {
    ip_host: u32,
    port_host: u16,
    _pad: u16,
}
unsafe impl aya::Pod for Vip {}

// ----- common load / SETUP plumbing -----

/// Loaded BPF object + handles for tc_reverse_nat testing.
///
/// `_outer_fd` owns the SERVICE_MAP outer HoM FD (created + pinned by
/// userspace per the pin-by-name workaround for aya 0.13.x — see
/// `.claude/rules/development.md` § "Sharing the outer HoM between
/// userspace and the kernel-side ELF — `pinning = ByName`"). Kept
/// alive so the kernel doesn't reclaim the map; the pin survives
/// across `EbpfLoader::load_file` so aya's loader picks it up via
/// `BPF_OBJ_GET`. `_pin_dir_guard` cleans the bpffs directory on drop.
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

fn load_tc_reverse_nat_program() -> LoadedTestBpf {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-tc-reverse-nat-{}-{:?}",
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
        let prog: &mut SchedClassifier = bpf
            .program_mut("tc_reverse_nat")
            .expect("tc_reverse_nat program not found in BPF object")
            .try_into()
            .expect("tc_reverse_nat is not a SchedClassifier program");
        prog.load().expect("tc_reverse_nat.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    LoadedTestBpf { bpf, prog_fd, _outer_fd: outer_fd, _pin_dir_guard: pin_dir_guard }
}

/// Pre-create + pre-pin the SERVICE_MAP outer HoM at
/// `<pin_dir>/SERVICE_MAP`. Returns the userspace-owned outer FD.
/// Mirrors the XDP triptych's helper (see
/// `xdp_service_map_lookup.rs::pre_pin_service_map`).
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

    // Inner-map prototype — ARRAY of u32 (BackendId), size =
    // MaglevTableSize::DEFAULT (16_381).
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
    // SAFETY: inner_raw is a kernel-issued FD.
    let inner_fd = unsafe { OwnedFd::from_raw_fd(inner_raw as c_int) };

    // Outer HoM with inner_map_fd set.
    let outer_attr = CreateAttr {
        map_type: BPF_MAP_TYPE_HASH_OF_MAPS,
        key_size: 8, // ServiceKey
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
    // SAFETY: outer_raw is a kernel-issued FD.
    let outer_fd = unsafe { OwnedFd::from_raw_fd(outer_raw as c_int) };

    // Pin outer to <pin_dir>/SERVICE_MAP so aya's loader picks it up
    // via BPF_OBJ_GET (the `pinning = ByName` workaround).
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
/// TCP)` entry from REVERSE_NAT_MAP regardless of prior state. Tier 2
/// tests run with `serial_test::serial(env)` so cross-test state
/// leakage cannot occur in parallel — but we still clear to make each
/// sub-test self-contained.
fn clear_reverse_nat_entry(bpf: &mut Ebpf, key: &BackendKey) {
    let mut rnm: HashMap<_, BackendKey, Vip> =
        HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
            .expect("REVERSE_NAT_MAP HashMap::try_from");
    let _ = rnm.remove(key); // ENOENT is fine — idempotent.
}

/// S-2.2-16 — `REVERSE_NAT_MAP` lookup hit rewrites source IP/port
/// back to VIP and returns `TC_ACT_OK` with valid checksums.
///
/// SETUP populates REVERSE_NAT_MAP with key `(10.1.0.5, 9000, TCP)` →
/// value `(10.0.0.1, 8080)`. PKTGEN builds a backend response from
/// `10.1.0.5:9000`. CHECK asserts BPF_PROG_TEST_RUN returns TC_ACT_OK
/// with rewritten source IP/port and recomputed checksums.
#[test]
#[serial(env)]
fn reverse_nat_lookup_hit_rewrites_source_to_vip() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_tc_reverse_nat_program();

    let key = BackendKey {
        ip_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_TCP,
        _pad: 0,
    };
    let value = Vip {
        ip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
        port_host: VIP_PORT,
        _pad: 0,
    };

    // SETUP: REVERSE_NAT_MAP[(backend_ip, backend_port, TCP)] → VIP.
    {
        let mut rnm: HashMap<_, BackendKey, Vip> =
            HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
                .expect("REVERSE_NAT_MAP HashMap::try_from");
        rnm.insert(key, value, 0).expect("REVERSE_NAT_MAP insert");
    }

    // PKTGEN: TCP SYN-ACK from BACKEND:BACKEND_PORT to CLIENT:CLIENT_PORT.
    let pkt = synthesise_backend_response(BACKEND_OCTETS, BACKEND_PORT);
    let mut out = vec![0u8; pkt.len()];

    // CHECK: drive BPF_PROG_TEST_RUN.
    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, TC_ACT_OK, "expected TC_ACT_OK (=0), got {action}");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    // (a) Source IP rewritten to VIP.
    let ip_src = &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16];
    assert_eq!(ip_src, &VIP_OCTETS, "source IP not rewritten to VIP");

    // (b) Source port rewritten.
    let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
    let src_port = u16::from_be_bytes([out[tcp], out[tcp + 1]]);
    assert_eq!(src_port, VIP_PORT, "source port not rewritten to VIP port");

    // (c) IPv4 header checksum is valid post-rewrite.
    let recomputed_ip_csum = ipv4_header_checksum(&out[ETH_HDR_LEN..ETH_HDR_LEN + IPV4_HDR_LEN]);
    assert_eq!(recomputed_ip_csum, 0, "IPv4 checksum invalid after rewrite");

    // (d) TCP checksum is valid post-rewrite (covers the rewritten
    // pseudo-header source IP AND the rewritten source port).
    let tcp_csum_recomputed = tcp_checksum(
        &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16],
        &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20],
        &out[tcp..tcp + TCP_HDR_LEN],
    );
    assert_eq!(tcp_csum_recomputed, 0, "TCP checksum invalid after rewrite");
}

/// S-2.2-16 (sibling) — `REVERSE_NAT_MAP` lookup miss returns
/// `TC_ACT_OK` with the frame unmodified (non-LB pass-through).
///
/// This is the structurally-required complement to the hit case: the
/// reverse-NAT path must not rewrite traffic from non-backend sources.
#[test]
#[serial(env)]
fn reverse_nat_lookup_miss_returns_tc_act_ok_no_rewrite() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_tc_reverse_nat_program();

    let key = BackendKey {
        ip_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_TCP,
        _pad: 0,
    };

    // SETUP: ensure no entry for our key. Idempotent — fresh-loaded
    // REVERSE_NAT_MAP is empty by default; we still call clear so a
    // residual entry from a prior test pass is gone.
    clear_reverse_nat_entry(&mut bpf, &key);

    // PKTGEN: TCP SYN-ACK from a backend address that has no entry.
    let pkt = synthesise_backend_response(BACKEND_OCTETS, BACKEND_PORT);
    let mut out = vec![0u8; pkt.len()];

    // CHECK.
    let (action, _) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, TC_ACT_OK, "expected TC_ACT_OK (=0) on miss, got {action}");

    // No rewrite means the output bytes match the input bytes exactly.
    assert_eq!(out, pkt, "miss path must not modify the frame");
}
