//! S-2.2-32 — `xdp_reverse_nat_lookup` reverse-path
//! `bpf_fib_lookup` + L2 rewrite + `bpf_redirect` Tier 2
//! PKTGEN/SETUP/CHECK triptych.
//!
//! Tags: `@US-05` `@K5` `@slice-09` `@real-io @adapter-integration`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).
//!
//! Per ADR-0045 § Decision § 2, the post-pivot reverse path is:
//!
//! 1. Sanity prologue (XDP-ingress; ADR-0040 Q3 amendment scope).
//! 2. REVERSE_NAT_MAP lookup keyed on
//!    `(backend_ip, backend_port, proto)` (the source 3-tuple of the
//!    backend's response). Miss → `XDP_PASS` (no DROP_COUNTER slot).
//! 3. L3 rewrite — source IP/port `(backend_ip, backend_port)` →
//!    `(VIP, vip_port)`; incremental IPv4 + L4 checksum update.
//! 4. **NEW**: `bpf_fib_lookup` against the post-rewrite
//!    `(src_ip, dst_ip)` to resolve next-hop MAC + egress ifindex.
//! 5. **NEW**: 12-byte L2-MAC memcpy — `eth_hdr->h_dest` /
//!    `eth_hdr->h_source` written from FIB-resolved `dmac` / `smac`.
//! 6. **NEW**: `bpf_redirect(ifindex, 0)` — kernel's XDP fast path
//!    delivers the rewritten frame directly to the resolved egress
//!    iface, bypassing the IP-forwarder. Returns `XDP_REDIRECT` (=4)
//!    on success.
//!
//! On `bpf_fib_lookup` non-success (any `BPF_FIB_LKUP_RET_*` status
//! != `RET_SUCCESS`) the program falls back to `XDP_PASS` so the
//! kernel ARP machinery can populate the neighbour cache. No
//! `DROP_COUNTER` slot is consumed (per ADR-0040 Q7).
//!
//! ## What this test pins (Tier 2 regression-protection scope)
//!
//! Both `bpf_redirect` (the XDP-side helper that ADR-0045's
//! amendment locked) and `bpf_redirect_neigh` (the TC-only helper
//! that the amendment reverted away from) return `XDP_REDIRECT` on
//! success; at PROG_TEST_RUN's verdict-return layer they are
//! indistinguishable. This test does NOT — and structurally cannot
//! — falsify the pivot end-to-end. The genuine pivot falsifier is
//! Tier 3 S-2.2-17 (`reverse_nat_e2e.rs`), which exercises real
//! TCP traffic and observes the kernel IP-forwarder bypass
//! behaviourally per ADR-0045 § 7.
//!
//! What this Tier 2 test DOES pin (regression-protection):
//!
//! 1. The reverse-path program exists and attaches as XDP-ingress.
//! 2. REVERSE_NAT_MAP hit + FIB hit returns `XDP_REDIRECT`, with
//!    L3+L4 rewrite committed before FIB lookup AND L2 MACs
//!    rewritten from FIB-resolved values.
//! 3. REVERSE_NAT_MAP hit + FIB miss falls back to `XDP_PASS` AFTER
//!    committing the L3+L4 rewrite (same shape as the forward path's
//!    miss fallback).
//! 4. REVERSE_NAT_MAP miss returns `XDP_PASS` with the frame
//!    unmodified — no DROP_COUNTER slot consumed.
//!
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall and aya's
//! userspace API requires libbpf-sys.

#![cfg(target_os = "linux")]
// See `xdp_pass_test_run.rs` and `xdp_service_map_lookup.rs` for the
// full rationale — Tier 2 BPF unit tests work directly with the
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
/// Hardcoded per the same reasoning as the sibling
/// `xdp_service_map_redirect_neigh.rs` test — kernel ABI, will not change.
const XDP_PASS: u32 = 2;
const XDP_REDIRECT: u32 = 4;

// ----- workspace plumbing (same shape as sibling tests) -----

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
const TCP_HDR_LEN: usize = 20;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// Backend → VIP rewrite test fixture.
///
/// In the reverse-NAT path, the *source* of the response is the
/// backend (which received the rewritten forward-path packet) and we
/// rewrite it back to the VIP so the client sees the response from
/// the address it originally sent to.
///
/// `ROUTABLE_CLIENT_OCTETS` is the response *destination* — to make
/// `bpf_fib_lookup` return `RET_SUCCESS` the client IP needs to be
/// routable on the test host. `10.0.0.100` lands on Lima's default
/// route, identical strategy to the sibling forward-path test's
/// `ROUTABLE_BACKEND_OCTETS`.
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const ROUTABLE_CLIENT_OCTETS: [u8; 4] = [10, 0, 0, 100];
const CLIENT_PORT: u16 = 12345;

/// IPv4 protocol — TCP for these tests; matches the kernel-side
/// `BackendKey { proto }` field.
const IPV4_PROTO_TCP: u8 = 6;

/// FIB-miss client IP — `192.0.2.99` from RFC 5737 TEST-NET-1
/// (`192.0.2.0/24`, designated for documentation and never
/// accidentally routed). The FIB-miss test installs a blackhole
/// route over this IP so `bpf_fib_lookup` returns
/// `BPF_FIB_LKUP_RET_BLACKHOLE` (= 1) regardless of Lima's default
/// route catch-all.
const FIB_MISS_CLIENT_OCTETS: [u8; 4] = [192, 0, 2, 99];

/// Sentinel source/dest MACs the PKTGEN ships in the eth_hdr. After
/// a `bpf_fib_lookup` success the program rewrites these to the
/// FIB-resolved smac/dmac, so observing the value differ post-test
/// (or not differ, on a miss) is the L2-rewrite assertion.
const SENTINEL_SRC_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xab, 0xcd, 0xef];
const SENTINEL_DST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// PKTGEN — a backend response from `BACKEND_OCTETS:BACKEND_PORT` to
/// `dst_octets:CLIENT_PORT`. The frame is a TCP SYN-ACK shape so
/// that flag-sanity prologues won't reject it.
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
    let total_len: u16 = (IPV4_HDR_LEN + TCP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes()); // id
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes()); // flags+frag
    pkt[ip + 8] = 0x40; // TTL=64
    pkt[ip + 9] = IPV4_PROTO_TCP;
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum (filled below)
    pkt[ip + 12..ip + 16].copy_from_slice(&BACKEND_OCTETS); // src IP (backend)
    pkt[ip + 16..ip + 20].copy_from_slice(&dst_octets); // dst IP (client)

    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    pkt[tcp..tcp + 2].copy_from_slice(&BACKEND_PORT.to_be_bytes()); // src port (backend)
    pkt[tcp + 2..tcp + 4].copy_from_slice(&CLIENT_PORT.to_be_bytes()); // dst port (client)
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5
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
// The same `overdrive_bpf.o` ELF carries every Phase 2.2 program
// (xdp_service_map_lookup, tc_reverse_nat, xdp_reverse_nat_lookup),
// which means the SERVICE_MAP HoM declaration is present in the ELF
// even though the reverse-NAT XDP program does not consume it.
// Aya's loader still tries to instantiate every `#[map]` it finds —
// and BPF_MAP_TYPE_HASH_OF_MAPS creation rejects without an
// `inner_map_fd` from the loader's side. The pin-by-name workaround
// from `.claude/rules/development.md` § "Sharing the outer HoM ..."
// is therefore mandatory for ANY test that loads this ELF.

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

/// RAII guard that installs an `ip route add blackhole <ip>` on
/// construction and removes it on drop. Used by the FIB-miss test
/// to force `bpf_fib_lookup` to return `BPF_FIB_LKUP_RET_BLACKHOLE`
/// against `<ip>` regardless of the default-route catch-all.
///
/// Requires NET_ADMIN — `cargo xtask lima run --` provides root,
/// CI's LVH harness provides root. If invoked unprivileged the test
/// fails at `Self::new()` with a clear EPERM message.
struct BlackholeRouteGuard {
    ip: std::net::Ipv4Addr,
}

impl BlackholeRouteGuard {
    fn new(ip: std::net::Ipv4Addr) -> Self {
        let status = std::process::Command::new("ip")
            .args(["route", "add", "blackhole", &ip.to_string()])
            .status()
            .expect("spawn `ip route add blackhole`");
        assert!(
            status.success(),
            "ip route add blackhole {ip} failed (status {status:?}); \
             test requires NET_ADMIN — invoke via `cargo xtask lima run --` or in CI",
        );
        Self { ip }
    }
}

impl Drop for BlackholeRouteGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("ip")
            .args(["route", "del", "blackhole", &self.ip.to_string()])
            .status();
    }
}

fn load_reverse_nat_xdp_program() -> LoadedTestBpf {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-xrnat-tier2-{}-{:?}",
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
/// Mirrors the helpers in the sibling forward-path
/// (`xdp_service_map_redirect_neigh.rs::pre_pin_service_map`) and
/// `tc_reverse_nat.rs::pre_pin_service_map` tests verbatim — the
/// shape is shared across every test that loads `overdrive_bpf.o`.
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
/// TCP)` entry from REVERSE_NAT_MAP regardless of prior state.
fn clear_reverse_nat_entry(bpf: &mut Ebpf, key: &BackendKey) {
    let mut rnm: HashMap<_, BackendKey, Vip> =
        HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
            .expect("REVERSE_NAT_MAP HashMap::try_from");
    let _ = rnm.remove(key);
}

/// Common SETUP — populate REVERSE_NAT_MAP with the `(backend, TCP)`
/// → VIP mapping the reverse-NAT path expects.
fn populate_reverse_nat(bpf: &mut Ebpf) {
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
    let mut rnm: HashMap<_, BackendKey, Vip> =
        HashMap::try_from(bpf.map_mut("REVERSE_NAT_MAP").expect("REVERSE_NAT_MAP not found"))
            .expect("REVERSE_NAT_MAP HashMap::try_from");
    rnm.insert(key, value, 0).expect("REVERSE_NAT_MAP insert");
}

// ----- tests -----

/// S-2.2-32 (REVERSE_NAT-hit + FIB-hit branch). A REVERSE_NAT_MAP hit
/// on a routable client destination MUST return `XDP_REDIRECT` (=4)
/// AND:
///
/// - The IPv4 source IP MUST be rewritten from BACKEND_OCTETS to
///   VIP_OCTETS.
/// - The TCP source port MUST be rewritten from BACKEND_PORT to
///   VIP_PORT.
/// - The IPv4 + TCP checksums MUST be valid post-rewrite.
/// - The Ethernet h_dest / h_source MUST be rewritten away from the
///   PKTGEN sentinels by the in-program L2 MAC memcpy.
///
/// On Lima the FIB resolves `ROUTABLE_CLIENT_OCTETS` via the default
/// route's gateway with a populated neighbour cache; `bpf_fib_lookup`
/// returns `RET_SUCCESS` and the program returns `XDP_REDIRECT` from
/// the `bpf_redirect(ifindex, 0)` call.
#[test]
#[serial(env)]
fn reverse_path_redirects_via_neigh_on_fib_hit() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_reverse_nat_xdp_program();

    populate_reverse_nat(&mut bpf);

    let pkt = synthesise_backend_response(ROUTABLE_CLIENT_OCTETS);
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    assert_eq!(action, XDP_REDIRECT, "expected XDP_REDIRECT (=4) on FIB hit, got {action}");

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

    // (d) TCP checksum is valid post-rewrite.
    let tcp_csum_recomputed = tcp_checksum(
        &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16],
        &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20],
        &out[tcp..tcp + TCP_HDR_LEN],
    );
    assert_eq!(tcp_csum_recomputed, 0, "TCP checksum invalid after rewrite");

    // (e) L2 rewrite assertion: h_dest and h_source MUST differ from
    // the PKTGEN sentinels. The actual MAC values depend on the FIB
    // resolution (Lima eth0 source MAC + default-gateway MAC), so we
    // assert "rewritten" not "rewritten to specific value".
    let h_dest = &out[0..6];
    let h_source = &out[6..12];
    assert_ne!(h_dest, &SENTINEL_DST_MAC, "h_dest not rewritten by FIB lookup");
    assert_ne!(h_source, &SENTINEL_SRC_MAC, "h_source not rewritten by FIB lookup");
}

/// S-2.2-32 (REVERSE_NAT-hit + FIB-miss branch). When
/// `bpf_fib_lookup` returns a non-success status, the post-pivot
/// program MUST return `XDP_PASS` so the kernel ARP machinery can
/// populate the neighbour cache (per ADR-0045 § 5). No DROP_COUNTER
/// slot is consumed (per ADR-0040 Q7).
///
/// SETUP: REVERSE_NAT_MAP hit for `(backend, TCP)` → VIP. PKTGEN: a
/// backend response targeted at `FIB_MISS_CLIENT_OCTETS`. A blackhole
/// route over `FIB_MISS_CLIENT_OCTETS` forces `bpf_fib_lookup` to
/// return `RET_BLACKHOLE` (= 1).
///
/// Asserts the L3 rewrite committed BEFORE the FIB lookup (the
/// post-rewrite source IP is observable on the wire), the verdict
/// is `XDP_PASS`, and the L2 fields stayed at their sentinel
/// values (the FIB-success branch did not fire).
#[test]
#[serial(env)]
fn reverse_path_passes_to_kernel_on_fib_miss() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_reverse_nat_xdp_program();

    let _blackhole = BlackholeRouteGuard::new(std::net::Ipv4Addr::from(FIB_MISS_CLIENT_OCTETS));

    populate_reverse_nat(&mut bpf);

    let pkt = synthesise_backend_response(FIB_MISS_CLIENT_OCTETS);
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    assert_eq!(
        action, XDP_PASS,
        "expected XDP_PASS (=2) on FIB miss (blackhole route → RET_BLACKHOLE), got {action}",
    );

    // L3 rewrite happens BEFORE the FIB lookup, so even on miss the
    // post-rewrite source IP is observable on the wire.
    let ip_src = &out[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16];
    assert_eq!(
        ip_src, &VIP_OCTETS,
        "source IP not rewritten to VIP (rewrite must commit before FIB fallback)",
    );

    // L2 fields MUST NOT have been rewritten — the FIB lookup failed,
    // so the program committed neither the smac nor the dmac update.
    let h_dest = &out[0..6];
    let h_source = &out[6..12];
    assert_eq!(h_dest, &SENTINEL_DST_MAC, "h_dest must remain unchanged on FIB miss");
    assert_eq!(h_source, &SENTINEL_SRC_MAC, "h_source must remain unchanged on FIB miss");
}

/// S-2.2-32 (REVERSE_NAT-miss branch). When the REVERSE_NAT_MAP
/// lookup misses, the program MUST return `XDP_PASS` with the frame
/// unmodified. No DROP_COUNTER slot is consumed (REVERSE_NAT miss is
/// "not LB traffic" — the kernel networking stack handles it).
///
/// SETUP: REVERSE_NAT_MAP cleared. PKTGEN: a TCP SYN-ACK from
/// `BACKEND_OCTETS:BACKEND_PORT`. CHECK: `XDP_PASS`, output frame ==
/// input frame.
#[test]
#[serial(env)]
fn reverse_path_passes_to_kernel_on_reverse_nat_miss() {
    let LoadedTestBpf { mut bpf, prog_fd, _outer_fd, _pin_dir_guard } =
        load_reverse_nat_xdp_program();

    let key = BackendKey {
        ip_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
        port_host: BACKEND_PORT,
        proto: IPV4_PROTO_TCP,
        _pad: 0,
    };
    clear_reverse_nat_entry(&mut bpf, &key);

    let pkt = synthesise_backend_response(ROUTABLE_CLIENT_OCTETS);
    let mut out = vec![0u8; pkt.len()];

    let (action, _) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(action, XDP_PASS, "expected XDP_PASS (=2) on REVERSE_NAT miss, got {action}");

    // No rewrite means the output bytes match the input bytes exactly.
    assert_eq!(out, pkt, "REVERSE_NAT-miss path must not modify the frame");
}
