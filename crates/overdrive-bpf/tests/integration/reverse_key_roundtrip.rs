//! S-2.2-17 — Endianness lockstep wire/host roundtrip per
//! architecture.md § 11.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
//! `@property`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN` to drive the kernel-side
//! `reverse_key_from_packet` helper end-to-end through `tc_reverse_nat`).
//!
//! Sibling: userspace mod-tests proptest in
//! `crates/overdrive-dataplane/src/maps/reverse_nat_map_handle.rs`
//! covers host-order writes against host-order reads.
//!
//! # Wire-bit-layout fixture
//!
//! The packet is built with **wire-order bytes** (network byte order)
//! at the IPv4 src-IP and TCP src-port positions:
//!
//! ```text
//! src IP  bytes [10, 1, 0, 5]   == 0x0a010005 in network order
//! src port bytes [0x23, 0x28]    == 0x2328     in network order  (= 9000 dec)
//! ```
//!
//! The userspace seed of `REVERSE_NAT_MAP` uses the **host-order**
//! `BackendKey { ip_host: u32::from(Ipv4Addr::new(10, 1, 0, 5)),
//! port_host: 9000, proto: 6, _pad: 0 }`. On a little-endian host
//! `u32::from(Ipv4Addr::new(10, 1, 0, 5)) == 0x0a010005`, so the
//! host-order `u32` numerically matches the wire-order bytes —
//! that is the whole point of the architecture.md § 11 lockstep:
//! userspace stores host-order numerically; kernel-side
//! `u32::from_be_bytes` of wire bytes also produces the host-order
//! numeric. Lockstep means the two values *match without any
//! userspace flip*.
//!
//! The kernel-side `reverse_key_from_packet` helper performs the
//! wire→host conversion and the resulting key is used to look up
//! REVERSE_NAT_MAP. If the helper or the userspace handle smuggle an
//! `htonl`/`ntohl` flip in either direction, the lookup misses and
//! `tc_reverse_nat` returns `TC_ACT_OK` *without* rewriting the
//! source IP — the post-condition assertions then fail with a
//! diff between the expected `VIP_OCTETS` and the unrewritten
//! `BACKEND_OCTETS`.

#![cfg(target_os = "linux")]
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

use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;

use aya::{
    Ebpf, EbpfLoader,
    maps::HashMap,
    programs::{ProgramFd, SchedClassifier},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

const TC_ACT_OK: u32 = 0;
const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const TCP_HDR_LEN: usize = 20;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// Wire-order bytes for the backend source IP. `[10, 1, 0, 5]` on
/// the wire == network-order `0x0a010005` == host-order
/// `u32::from(Ipv4Addr::new(10, 1, 0, 5))` on every little-endian
/// matrix kernel.
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
/// Wire-order bytes for the backend source port. `0x23, 0x28` ==
/// network-order `0x2328` == 9000 host-order.
const BACKEND_PORT: u16 = 9000;

/// VIP rewritten to on hit. Distinct from BACKEND_OCTETS so the
/// assertion is unambiguous.
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;

const CLIENT_OCTETS: [u8; 4] = [10, 0, 0, 100];
const CLIENT_PORT: u16 = 12345;
const IPV4_PROTO_TCP: u8 = 6;

// ----- workspace plumbing -----

fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop();
    p.pop();
    p
}

fn bpf_artifact_path() -> PathBuf {
    workspace_root().join("target/xtask/bpf-objects/overdrive_bpf.o")
}

// ----- syscall helper -----

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

    #[allow(clippy::cast_possible_truncation)]
    let attr_size = std::mem::size_of::<bpf_attr>() as libc::c_uint;
    // SAFETY: standard kernel ABI for BPF; size_of::<bpf_attr>()
    // matches the kernel's expected layout.
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

// ----- POD shapes that must match the kernel-side declarations -----

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

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Vip {
    ip_host: u32,
    port_host: u16,
    _pad: u16,
}
unsafe impl aya::Pod for Vip {}

// ----- packet synthesis with explicit wire-order placement -----

/// Build an IPv4+TCP frame with `BACKEND_OCTETS:BACKEND_PORT` placed
/// in wire-order bytes at the canonical IPv4-src and TCP-src offsets.
/// Computes valid IPv4 and TCP checksums so the program's tc_csum
/// rewrites land on a well-formed input.
fn synthesise_backend_response_wire_order() -> Vec<u8> {
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
    // *** Wire-order placement of the source IP ***
    pkt[ip + 12..ip + 16].copy_from_slice(&BACKEND_OCTETS);
    pkt[ip + 16..ip + 20].copy_from_slice(&CLIENT_OCTETS);

    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    // *** Wire-order placement of the source port ***
    pkt[tcp..tcp + 2].copy_from_slice(&BACKEND_PORT.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&CLIENT_PORT.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes());
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes());
    pkt[tcp + 12] = 0x50;
    pkt[tcp + 13] = 0x12; // SYN+ACK
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes());
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes());
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes());

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
    sum += u32::from(0x0006_u16);
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

// ----- common load / SETUP plumbing (mirrors tc_reverse_nat.rs) -----

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
        "/sys/fs/bpf/overdrive-test-reverse-key-{}-{:?}",
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
    // SAFETY: inner_raw is a kernel-issued FD.
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
    // SAFETY: outer_raw is a kernel-issued FD.
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

/// S-2.2-17 — A synthetic packet with known wire-order bytes
/// through `reverse_key_from_packet` produces the host-order
/// `BackendKey` that the userspace test seeded into the map.
///
/// The structural assertion: kernel-side helper conversion +
/// userspace host-order seed are bit-for-bit equivalent. If either
/// side smuggles a `to_be`/`from_be` flip the lookup misses and
/// `tc_reverse_nat` returns `TC_ACT_OK` *without rewriting* — the
/// post-condition assertion on `VIP_OCTETS` then fails with a
/// telling diff.
#[test]
#[serial(env)]
fn wire_order_packet_produces_host_order_reverse_key() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_tc_reverse_nat_program();

    // Userspace seed — the host-order BackendKey for the same
    // logical (ip, port, proto) triple. On a little-endian host
    // (every matrix kernel per testing.md § Kernel matrix) the
    // numeric value of `u32::from(Ipv4Addr::new(10,1,0,5))` ==
    // 0x0a010005, mirroring the wire bytes [10, 1, 0, 5]. Lockstep:
    // userspace stores host-order; kernel-side `from_be_bytes` of
    // wire-order bytes also produces host-order — same numeric.
    let key = BackendKey {
        ip_host: u32::from(Ipv4Addr::new(
            BACKEND_OCTETS[0],
            BACKEND_OCTETS[1],
            BACKEND_OCTETS[2],
            BACKEND_OCTETS[3],
        )),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_TCP,
        _pad: 0,
    };
    let value = Vip {
        ip_host: u32::from(Ipv4Addr::new(
            VIP_OCTETS[0],
            VIP_OCTETS[1],
            VIP_OCTETS[2],
            VIP_OCTETS[3],
        )),
        port_host: VIP_PORT,
        _pad: 0,
    };

    {
        let mut rnm: HashMap<_, BackendKey, Vip> =
            HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
                .expect("REVERSE_NAT_MAP HashMap::try_from");
        rnm.insert(key, value, 0).expect("REVERSE_NAT_MAP insert");
    }

    // PKTGEN — wire-order bytes laid down explicitly. The
    // kernel-side `reverse_key_from_packet` helper reads them via
    // `u32::from_be_bytes` / `u16::from_be_bytes`, producing
    // host-order numerics. If either side flips, the lookup
    // misses.
    let pkt = synthesise_backend_response_wire_order();
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    // Lockstep contract: if the kernel-side helper produced the
    // host-order key matching the userspace seed, the lookup hits
    // and `tc_reverse_nat` rewrites. The structural assertion is
    // the post-rewrite source IP — `VIP_OCTETS` confirms hit;
    // `BACKEND_OCTETS` (unrewritten) confirms miss / lockstep
    // break.
    assert_eq!(action, TC_ACT_OK, "expected TC_ACT_OK (=0), got {action}");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    let ip_src = &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16];
    assert_eq!(
        ip_src, &VIP_OCTETS,
        "lockstep break: source IP not rewritten — kernel-side host-order key did not match userspace-seeded host-order key (got {ip_src:?}, expected {VIP_OCTETS:?})"
    );

    let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
    let src_port = u16::from_be_bytes([out[tcp], out[tcp + 1]]);
    assert_eq!(
        src_port, VIP_PORT,
        "lockstep break: source port not rewritten — got {src_port}, expected {VIP_PORT}"
    );
}
