//! S-2.2-31 — `xdp_service_map_lookup` forward-path
//! `bpf_fib_lookup` + L2 rewrite + `bpf_redirect_neigh` Tier 2
//! PKTGEN/SETUP/CHECK triptych.
//!
//! Tags: `@US-02` `@K2` `@slice-09` `@real-io @adapter-integration`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).
//!
//! Per ADR-0045 § Decision § 1, the post-pivot forward path is:
//!
//! 1. SERVICE_MAP / MAGLEV_MAP / BACKEND_MAP chained lookup
//!    (preserved from Slices 02–04).
//! 2. L3 + L4 rewrite with incremental checksum update (preserved
//!    from ADR-0040 Q1=A).
//! 3. **NEW**: `bpf_fib_lookup` against the post-rewrite
//!    `(src_ip, dst_ip)` to resolve next-hop MAC + egress ifindex.
//! 4. **NEW**: 12-byte L2-MAC memcpy — `eth_hdr->h_dest` /
//!    `eth_hdr->h_source` written from FIB-resolved `dmac` / `smac`.
//! 5. **NEW**: `bpf_redirect_neigh(ifindex, NULL, 0, 0)` — kernel's
//!    XDP fast path delivers the rewritten frame directly to the
//!    resolved egress iface, bypassing the IP-forwarder. Returns
//!    `XDP_REDIRECT` (=4) on success.
//!
//! On `bpf_fib_lookup` non-success (any `BPF_FIB_LKUP_RET_*` status
//! != `RET_SUCCESS`) the program falls back to `XDP_PASS` so the
//! kernel ARP machinery can populate the neighbour cache. No
//! `DROP_COUNTER` slot is consumed (per ADR-0040 Q7).
//!
//! ## What this test fails on (pre-pivot, RED)
//!
//! Today's `xdp_service_map.rs::fib_resolve_and_rewrite_mac` returns
//! `XDP_TX` on same-iface FIB success (the post-Slice 05-04 shape).
//! Under post-pivot ADR-0045 it MUST return whatever
//! `bpf_redirect_neigh` returns — `XDP_REDIRECT` (=4). The
//! `forward_path_redirects_via_neigh_on_fib_hit` test below pins
//! this distinction at the verdict level.
//!
//! On a FIB miss (backend IP routes nowhere), the post-pivot path
//! returns `XDP_PASS` after committing the L3+L4 rewrite — the
//! `forward_path_passes_to_kernel_on_fib_miss` test below pins this.
//! This particular shape is NOT a regression vs the pre-pivot code
//! (today's `bpf_fib_lookup` non-success branch already returns
//! `XDP_PASS`); the test is a load-bearing GREEN that confirms the
//! neigh-redirect pivot did not break the miss-fallback.
//!
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall and aya's
//! userspace API requires libbpf-sys.

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

use std::path::PathBuf;

use std::os::fd::{AsRawFd, OwnedFd};

use aya::{
    Ebpf, EbpfLoader,
    maps::HashMap,
    programs::{ProgramFd, Xdp},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

/// XDP verdict constants — kernel ABI from `<bpf.h>` (`enum xdp_action`).
/// Hardcoded per the same reasoning as in sibling tests; values will
/// not change.
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
    workspace_root().join("target/bpf/overdrive_bpf.o")
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

/// VIP / backend test fixture. The "routable" backend IP is
/// `BACKEND_OCTETS` = 10.1.0.5 — this lands on the Lima default
/// route and the kernel FIB returns `RET_SUCCESS` with a populated
/// dmac/smac/ifindex (or `RET_NO_NEIGH` if ARP hasn't fired). On a
/// freshly booted Lima the default route's gateway typically already
/// has a populated neighbour cache from prior traffic, so `RET_SUCCESS`
/// is the steady-state outcome here.
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const ROUTABLE_BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;

/// FIB-miss backend IP — `192.0.2.99` from RFC 5737 TEST-NET-1
/// (`192.0.2.0/24`, designated for documentation and never
/// accidentally routed). The FIB-miss test installs a blackhole
/// route over this IP so `bpf_fib_lookup` returns
/// `BPF_FIB_LKUP_RET_BLACKHOLE` (= 1).
///
/// **Why a blackhole route, not packet-shape provocation**: TTL=1
/// does NOT trigger FIB miss — `bpf_fib_lookup` with flags=0 does
/// not check TTL (that's the IP forwarder's job, not the FIB
/// helper's). Destination-IP-based misses don't work either because
/// Lima's default route (`0.0.0.0/0`) catches every IPv4 address
/// including reserved blocks. Installing an explicit blackhole
/// route is the only portable way to force a FIB-miss verdict at
/// PROG_TEST_RUN time. The route is set up + torn down per-test via
/// a RAII guard.
const FIB_MISS_BACKEND_OCTETS: [u8; 4] = [192, 0, 2, 99];

/// Sentinel source MAC the PKTGEN ships in the eth_hdr. After a
/// `bpf_fib_lookup` success the program rewrites this to the
/// FIB-resolved smac, so observing the value differ post-test
/// (or not differ, on a miss) is the L2-rewrite assertion.
const SENTINEL_SRC_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xab, 0xcd, 0xef];
const SENTINEL_DST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

fn synthesise_tcp_syn(dst_octets: [u8; 4], dst_port: u16) -> Vec<u8> {
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
    pkt[ip + 9] = 0x06; // proto=TCP
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum (filled below)
    pkt[ip + 12..ip + 16].copy_from_slice(&[10, 0, 0, 100]); // src IP
    pkt[ip + 16..ip + 20].copy_from_slice(&dst_octets); // dst IP

    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    let src_port: u16 = 12345;
    pkt[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&dst_port.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5
    pkt[tcp + 13] = 0x02; // flags = SYN
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

/// Outer-map key matching the `ServiceKey` struct in
/// `crates/overdrive-bpf/src/maps/service_map.rs`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct ServiceKey {
    vip_host: u32,
    port_host: u16,
    _pad: u16,
}
// SAFETY: repr(C); we always set `_pad = 0`; no padding-uninit
// concerns. `aya::Pod` is the marker aya needs to permit raw access.
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

// ----- loader plumbing (mirrors xdp_service_map_lookup.rs) -----

struct LoadedTestBpf {
    bpf: Ebpf,
    prog_fd: ProgramFd,
    outer_fd: OwnedFd,
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

fn load_service_map_program() -> LoadedTestBpf {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-rdrnh-tier2-{}-{:?}",
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
            .program_mut("xdp_service_map_lookup")
            .expect("xdp_service_map_lookup program not found in BPF object")
            .try_into()
            .expect("xdp_service_map_lookup is not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    LoadedTestBpf { bpf, prog_fd, outer_fd, _pin_dir_guard: pin_dir_guard }
}

fn create_inner_array_filled(backend_id: u32) -> OwnedFd {
    use std::mem;
    use std::os::fd::FromRawFd;

    use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

    const BPF_MAP_CREATE: c_long = 0;
    const BPF_MAP_UPDATE_ELEM: c_long = 2;
    const BPF_MAP_TYPE_ARRAY: u32 = 2;

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
    struct ElemAttr {
        map_fd: u32,
        _pad0: u32,
        key: u64,
        value: u64,
        flags: u64,
    }

    let inner_size: u32 = overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();
    let attr = CreateAttr {
        map_type: BPF_MAP_TYPE_ARRAY,
        key_size: mem::size_of::<u32>() as u32,
        value_size: mem::size_of::<u32>() as u32,
        max_entries: inner_size,
        ..Default::default()
    };
    // SAFETY: bpf() syscall with a valid `bpf_attr` struct.
    let raw = unsafe {
        syscall(
            SYS_bpf,
            BPF_MAP_CREATE,
            &attr as *const _ as *const c_void,
            mem::size_of::<CreateAttr>() as c_int,
        )
    };
    assert!(raw >= 0, "inner ARRAY create: {}", std::io::Error::last_os_error());
    // SAFETY: kernel-issued FD.
    let inner_fd = unsafe { OwnedFd::from_raw_fd(raw as c_int) };

    for slot in 0..inner_size {
        let key_bytes = slot.to_ne_bytes();
        let value_bytes = backend_id.to_ne_bytes();
        let elem = ElemAttr {
            map_fd: inner_fd.as_raw_fd() as u32,
            _pad0: 0,
            key: key_bytes.as_ptr() as u64,
            value: value_bytes.as_ptr() as u64,
            flags: 0,
        };
        // SAFETY: bpf() syscall with a valid attr.
        let rc = unsafe {
            syscall(
                SYS_bpf,
                BPF_MAP_UPDATE_ELEM,
                &elem as *const _ as *const c_void,
                mem::size_of::<ElemAttr>() as c_int,
            )
        };
        assert!(rc >= 0, "inner ARRAY slot {slot} populate: {}", std::io::Error::last_os_error());
    }
    inner_fd
}

fn outer_map_set(outer_fd: &OwnedFd, key: &ServiceKey, inner_fd: &OwnedFd) {
    use std::mem;

    use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

    const BPF_MAP_UPDATE_ELEM: c_long = 2;

    #[repr(C)]
    struct ElemAttr {
        map_fd: u32,
        _pad0: u32,
        key: u64,
        value: u64,
        flags: u64,
    }

    let inner_u32: u32 = inner_fd.as_raw_fd() as u32;
    let attr = ElemAttr {
        map_fd: outer_fd.as_raw_fd() as u32,
        _pad0: 0,
        key: key as *const ServiceKey as u64,
        value: &inner_u32 as *const u32 as u64,
        flags: 0,
    };
    // SAFETY: bpf() syscall with a valid attr.
    let rc = unsafe {
        syscall(
            SYS_bpf,
            BPF_MAP_UPDATE_ELEM,
            &attr as *const _ as *const c_void,
            mem::size_of::<ElemAttr>() as c_int,
        )
    };
    assert!(rc >= 0, "outer HoM set: {}", std::io::Error::last_os_error());
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

/// Common SETUP — populate BACKEND_MAP, fresh inner ARRAY filled
/// with `backend_id`, outer HoM slot for `(VIP, VIP_PORT)`.
fn populate_service_map(
    bpf: &mut Ebpf,
    outer_fd: &OwnedFd,
    backend_id: u32,
    backend_octets: [u8; 4],
) -> OwnedFd {
    {
        let mut bm: HashMap<_, u32, BackendEntry> =
            HashMap::try_from(bpf.map_mut("BACKEND_MAP").expect("BACKEND_MAP not found"))
                .expect("BACKEND_MAP HashMap::try_from");
        let value = BackendEntry {
            ipv4_host: u32::from(std::net::Ipv4Addr::from(backend_octets)),
            port_host: BACKEND_PORT,
            weight: 1,
            healthy: 1,
            _pad: [0; 3],
        };
        bm.insert(backend_id, value, 0).expect("BACKEND_MAP insert");
    }
    let inner_fd = create_inner_array_filled(backend_id);
    let key = ServiceKey {
        vip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
        port_host: VIP_PORT,
        _pad: 0,
    };
    outer_map_set(outer_fd, &key, &inner_fd);
    inner_fd
}

// ----- tests -----

/// S-2.2-31 (FIB-hit branch). A SERVICE_MAP hit on a routable
/// backend MUST return `XDP_REDIRECT` (=4) and the Ethernet
/// h_dest / h_source fields MUST be rewritten away from the PKTGEN
/// sentinels by the in-program L2 MAC memcpy.
///
/// **Tier 2 falsification scope.** Both `bpf_redirect`
/// (pre-pivot cross-iface branch) and `bpf_redirect_neigh`
/// (post-pivot uniform branch) return `XDP_REDIRECT` on success;
/// at PROG_TEST_RUN's verdict-return layer they are
/// indistinguishable. This test does NOT — and structurally cannot
/// — falsify the `bpf_redirect` → `bpf_redirect_neigh` swap on its
/// own. The genuine pivot falsifier is **Tier 3 S-2.2-17**
/// (`reverse_nat_e2e.rs`), which exercises real TCP traffic and
/// observes the kernel IP-forwarder bypass behaviourally per
/// ADR-0045 § 7.
///
/// What this Tier 2 test DOES pin (regression-protection scope):
///
/// 1. The contract surface AC #5 demands — FIB-hit returns
///    `XDP_REDIRECT`, the L3+L4 rewrite committed before FIB
///    lookup, the L2 MACs rewritten from FIB-resolved values.
/// 2. Survives a future refactor that skips the explicit L2 memcpy
///    on the (mistaken) theory that `bpf_redirect_neigh` resolves
///    L2 itself — ADR-0045 § Decision § 1 step 5 mandates the
///    rewrite happens IN-PROGRAM before the redirect call.
/// 3. Survives a future refactor that flips back to `XDP_DROP` on
///    FIB success (a pure logic regression).
///
/// On Lima the FIB resolves `ROUTABLE_BACKEND_OCTETS` via the
/// default route's gateway with a populated neighbour cache;
/// `bpf_fib_lookup` returns `RET_SUCCESS` and the cross-iface
/// branch fires (egress iface ≠ default ingress=0).
#[test]
#[serial(env)]
fn forward_path_redirects_via_neigh_on_fib_hit() {
    let LoadedTestBpf { mut bpf, prog_fd, outer_fd, _pin_dir_guard } = load_service_map_program();

    const BID_ONE: u32 = 1;
    let _inner_fd = populate_service_map(&mut bpf, &outer_fd, BID_ONE, ROUTABLE_BACKEND_OCTETS);

    let pkt = synthesise_tcp_syn(VIP_OCTETS, VIP_PORT);
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    assert_eq!(action, XDP_REDIRECT, "expected XDP_REDIRECT (=4) on FIB hit, got {action}",);

    // L2 rewrite assertion: h_dest and h_source MUST differ from
    // the PKTGEN sentinels. The actual MAC values depend on the
    // FIB resolution (Lima eth0 source MAC + default-gateway MAC),
    // so we assert "rewritten" not "rewritten to specific value".
    let h_dest = &out[0..6];
    let h_source = &out[6..12];
    assert_ne!(h_dest, &SENTINEL_DST_MAC, "h_dest not rewritten by FIB lookup");
    assert_ne!(h_source, &SENTINEL_SRC_MAC, "h_source not rewritten by FIB lookup");

    // L3 rewrite still happens (preserved from pre-pivot).
    let ip_dst = &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20];
    assert_eq!(ip_dst, &ROUTABLE_BACKEND_OCTETS, "dst IP not rewritten to backend");
}

/// S-2.2-31 (FIB-miss branch). When `bpf_fib_lookup` returns a
/// non-success status (any of the `BPF_FIB_LKUP_RET_*` codes), the
/// post-pivot program MUST return `XDP_PASS` so the kernel ARP
/// machinery can populate the neighbour cache (per ADR-0045 § 5).
/// No DROP_COUNTER slot is consumed (per ADR-0040 Q7 — preserved).
///
/// We force the miss with a blackhole route over RFC 5737 TEST-NET-1
/// (`192.0.2.99`). `bpf_fib_lookup` returns `RET_BLACKHOLE` (= 1)
/// in this case — independent of Lima's default route, independent
/// of TTL, and stable across the kernel matrix. The route is
/// installed at test entry and removed via RAII drop.
///
/// **What this pins, GREEN**: the post-pivot fallback shape is
/// correct (XDP_PASS not XDP_DROP, no DROP_COUNTER consumption,
/// L2 fields untouched). Pre-pivot already had `XDP_PASS` on FIB
/// miss, so this test alone is not a RED-flipper for the pivot —
/// its value is preventing regression of the fallback semantics
/// during the post-pivot rewrite of `fib_resolve_and_rewrite_mac`.
#[test]
#[serial(env)]
fn forward_path_passes_to_kernel_on_fib_miss() {
    let LoadedTestBpf { mut bpf, prog_fd, outer_fd, _pin_dir_guard } = load_service_map_program();

    // SETUP: install a blackhole route over the FIB-miss backend IP.
    // Drop order matters — the guard must outlive PROG_TEST_RUN.
    let _blackhole = BlackholeRouteGuard::new(std::net::Ipv4Addr::from(FIB_MISS_BACKEND_OCTETS));

    const BID_ONE: u32 = 1;
    let _inner_fd = populate_service_map(&mut bpf, &outer_fd, BID_ONE, FIB_MISS_BACKEND_OCTETS);

    // PKTGEN: VIP-bound TCP SYN; the SERVICE_MAP rewrites dst to
    // FIB_MISS_BACKEND_OCTETS, then `bpf_fib_lookup` finds the
    // blackhole route and returns RET_BLACKHOLE.
    let pkt = synthesise_tcp_syn(VIP_OCTETS, VIP_PORT);
    let mut out = vec![0u8; pkt.len()];

    let (action, out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");
    assert_eq!(out_len, pkt.len(), "output frame length mismatch");

    // Verdict must be XDP_PASS (=2) — the FIB-miss fallback per
    // ADR-0045 § 5.
    assert_eq!(
        action, XDP_PASS,
        "expected XDP_PASS (=2) on FIB miss (blackhole route → RET_BLACKHOLE), got {action}",
    );

    // The L3 rewrite happens BEFORE the FIB lookup (per
    // `xdp_service_map.rs::rewrite_and_tx`), so even on miss the
    // post-rewrite dst IP is observable on the wire. Asserting on
    // the DST IP confirms the lookup chain ran and committed before
    // the FIB fallback fired.
    let ip_dst = &out[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20];
    assert_eq!(
        ip_dst, &FIB_MISS_BACKEND_OCTETS,
        "dst IP not rewritten to backend (lookup chain must run before FIB fallback)",
    );

    // L2 fields MUST NOT have been rewritten — the FIB lookup
    // failed, so the program committed neither the smac nor the
    // dmac update. (The post-pivot rewrite is conditional on the
    // FIB success branch.)
    let h_dest = &out[0..6];
    let h_source = &out[6..12];
    assert_eq!(h_dest, &SENTINEL_DST_MAC, "h_dest must remain unchanged on FIB miss");
    assert_eq!(h_source, &SENTINEL_SRC_MAC, "h_source must remain unchanged on FIB miss");
}
