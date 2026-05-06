//! S-2.2-19, S-2.2-20, S-2.2-21 — Sanity prologue per-class drop
//! assertions.
//!
//! Tags: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).
//!
//! Each sub-test follows the same shape as `xdp_service_map_lookup.rs`:
//! load `target/xtask/bpf-objects/overdrive_bpf.o` via `aya::Ebpf`,
//! resolve the `xdp_service_map_lookup` (or `tc_reverse_nat`) program
//! and `DROP_COUNTER` / `SERVICE_MAP` maps, drive `BPF_PROG_TEST_RUN`
//! directly via the `bpf(2)` syscall, assert on the returned verdict
//! and the per-CPU `DROP_COUNTER[MalformedHeader]` slot.
//!
//! Map state is cleared between sub-tests by default per
//! `.claude/rules/testing.md` § "Tier 2 — BPF Unit Tests" — each
//! test reads the DROP_COUNTER baseline at SETUP and asserts on the
//! delta after PROG_TEST_RUN, which makes the test resilient to any
//! per-CPU values left behind by sibling tests in the same process.

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
    clippy::doc_markdown,
    clippy::explicit_iter_loop,
    clippy::explicit_counter_loop
)]

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::PathBuf;

use aya::{
    Ebpf, EbpfLoader,
    maps::PerCpuArray,
    programs::{ProgramFd, SchedClassifier, Xdp},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

// XDP / TC verdict constants — kernel ABI; will not change.
const XDP_DROP: u32 = 1;
const XDP_PASS: u32 = 2;
const TC_ACT_OK: u32 = 0;
const TC_ACT_SHOT: u32 = 2;

// `DropClass::MalformedHeader` slot index. Mirrored from
// `crates/overdrive-core/src/dataplane/drop_class.rs` (`Slot 0`).
const DROP_CLASS_MALFORMED_HEADER: u32 = 0;

// ---------- workspace plumbing ----------

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

// ---------- bpf(2) syscall helper ----------

fn bpf_prog_test_run(
    prog_fd: &ProgramFd,
    data_in: &[u8],
    data_out_buf: &mut [u8],
) -> Result<(u32, usize), std::io::Error> {
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
    // SAFETY: standard kernel ABI.
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

// ---------- DROP_COUNTER read helper ----------

/// Sum the `DropClass::MalformedHeader` per-CPU slot across all
/// online CPUs. Mirrors `overdrive_core::dataplane::aggregate_per_cpu`.
fn read_malformed_header_counter(bpf: &Ebpf) -> u64 {
    let map = bpf.map("DROP_COUNTER").expect("DROP_COUNTER map not found");
    let arr: PerCpuArray<_, u64> =
        PerCpuArray::try_from(map).expect("DROP_COUNTER PerCpuArray::try_from");
    let per_cpu = arr.get(&DROP_CLASS_MALFORMED_HEADER, 0).expect("DROP_COUNTER.get");
    per_cpu.iter().copied().fold(0u64, u64::saturating_add)
}

// ---------- pin-dir + outer HoM pre-create ----------
//
// The `xdp_service_map_lookup` program references SERVICE_MAP
// (HoM) which aya 0.13.x cannot create from the ELF alone — see
// `.claude/rules/development.md` § "Sharing the outer HoM …
// pinning = ByName". We pre-create + pre-pin the HoM here even
// though our test does not populate it; the loader needs the FD
// to be reachable by name.

struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    let cstr =
        CString::new(pin_path.as_os_str().to_string_lossy().as_bytes()).expect("pin path CString");
    let pin_attr = PinAttr {
        pathname: cstr.as_ptr() as u64,
        bpf_fd: outer_fd.as_raw_fd() as u32,
        file_flags: 0,
    };
    let pin_rc = raw_bpf(
        BPF_OBJ_PIN,
        &pin_attr as *const _ as *const c_void,
        mem::size_of::<PinAttr>() as c_int,
    );
    assert!(pin_rc >= 0, "outer HoM pin: {}", std::io::Error::last_os_error());

    outer_fd
}

// ---------- common load (XDP) ----------

struct LoadedXdp {
    bpf: Ebpf,
    prog_fd: ProgramFd,
    _outer_fd: OwnedFd,
    _pin_dir_guard: PinDirGuard,
}

fn load_xdp_program() -> LoadedXdp {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-sanity-xdp-{}-{:?}",
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
            .expect("xdp_service_map_lookup program not found")
            .try_into()
            .expect("not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    LoadedXdp { bpf, prog_fd, _outer_fd: outer_fd, _pin_dir_guard: pin_dir_guard }
}

// ---------- common load (TC) ----------

struct LoadedTc {
    bpf: Ebpf,
    prog_fd: ProgramFd,
    _outer_fd: OwnedFd,
    _pin_dir_guard: PinDirGuard,
}

fn load_tc_program() -> LoadedTc {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    let pin_dir = PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-sanity-tc-{}-{:?}",
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
            .expect("tc_reverse_nat program not found")
            .try_into()
            .expect("not a SchedClassifier program");
        prog.load().expect("tc_reverse_nat.load");
        prog.fd().expect("fd()").try_clone().expect("ProgramFd::try_clone")
    };
    LoadedTc { bpf, prog_fd, _outer_fd: outer_fd, _pin_dir_guard: pin_dir_guard }
}

// ---------- packet synthesis ----------

const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const TCP_HDR_LEN: usize = 20;
const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

const SRC_OCTETS: [u8; 4] = [10, 0, 0, 100];
const DST_OCTETS: [u8; 4] = [10, 0, 0, 1];
const SRC_PORT: u16 = 12345;
const DST_PORT: u16 = 8080;

/// Compute IPv4 header checksum (RFC 1071).
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

/// Synthesise a well-formed IPv4 + TCP frame with caller-controlled
/// flag-byte + IHL field. Returns the frame buffer.
fn synthesise_tcp(ihl: u8, tcp_flags: u8) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet (14B): dst + src MAC + ethertype 0x0800.
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]);

    // IPv4 (20B):
    let ip = ETH_HDR_LEN;
    pkt[ip] = (4 << 4) | (ihl & 0x0F); // version=4 | IHL=ihl
    pkt[ip + 1] = 0x00; // TOS
    let total_len: u16 = (IPV4_HDR_LEN + TCP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes()); // id
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes()); // flags+frag
    pkt[ip + 8] = 0x40; // TTL
    pkt[ip + 9] = 0x06; // proto = TCP
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[ip + 12..ip + 16].copy_from_slice(&SRC_OCTETS);
    pkt[ip + 16..ip + 20].copy_from_slice(&DST_OCTETS);
    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    pkt[tcp..tcp + 2].copy_from_slice(&SRC_PORT.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&DST_PORT.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5
    pkt[tcp + 13] = tcp_flags;
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes()); // window
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes()); // urg ptr
    let tcp_csum =
        tcp_checksum(&pkt[ip + 12..ip + 16], &pkt[ip + 16..ip + 20], &pkt[tcp..tcp + TCP_HDR_LEN]);
    pkt[tcp + 16..tcp + 18].copy_from_slice(&tcp_csum.to_be_bytes());

    pkt
}

/// Synthesise a frame with EtherType 0x86DD (IPv6) but the rest of
/// the bytes mirror the TCP-IPv4 layout. The sanity prologue must
/// reject this at check 1 (EtherType) and return `XDP_PASS` /
/// `TC_ACT_OK` — NO drop counter increment.
fn synthesise_ipv6_ethertype() -> Vec<u8> {
    let mut pkt = synthesise_tcp(5, 0x02 /* SYN — irrelevant */);
    pkt[12..14].copy_from_slice(&[0x86, 0xDD]); // EtherType IPv6
    pkt
}

// ---------- S-2.2-19 — XDP: truncated IPv4 (IHL=4) drops ----------
//
// **RED_ACCEPTANCE**: this is the dispatch's primary acceptance test.
// Exercises the sanity helper invocation from `xdp_service_map_lookup`
// end-to-end through PROG_TEST_RUN. Asserts both the verdict
// (`XDP_DROP`) AND the DROP_COUNTER[MalformedHeader] increment AND
// that no SERVICE_MAP entry was consulted (the SETUP populates no
// SERVICE_MAP slot — a "lookup happened anyway" regression would
// surface as XDP_PASS after a HoM miss path; the drop verdict
// distinguishes "sanity short-circuited" from "lookup-then-miss").

#[test]
#[serial(env)]
fn truncated_ipv4_header_drops_with_malformed_header_counter() {
    let LoadedXdp { bpf, prog_fd, _outer_fd, _pin_dir_guard } = load_xdp_program();

    // Baseline DROP_COUNTER[MalformedHeader] across CPUs.
    let baseline = read_malformed_header_counter(&bpf);

    // PKTGEN: IPv4 header with IHL=4 (would imply 16 bytes of header,
    // structurally malformed — minimum valid IHL is 5).
    let pkt = synthesise_tcp(/* ihl */ 4, /* flags */ 0x02);
    let mut out = vec![0u8; pkt.len()];

    // CHECK.
    let (action, _out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    assert_eq!(
        action, XDP_DROP,
        "IHL=4 must short-circuit at sanity prologue with XDP_DROP, got {action}"
    );

    let after = read_malformed_header_counter(&bpf);
    assert_eq!(
        after - baseline,
        1,
        "DROP_COUNTER[MalformedHeader] must increment by 1 (baseline={baseline}, after={after})"
    );
}

// ---------- S-2.2-20 — XDP: pathological TCP flags drop ----------

#[test]
#[serial(env)]
fn tcp_syn_plus_rst_flags_drops_with_malformed_header_counter() {
    let LoadedXdp { bpf, prog_fd, _outer_fd, _pin_dir_guard } = load_xdp_program();

    let baseline = read_malformed_header_counter(&bpf);

    // PKTGEN: well-formed IPv4 (IHL=5) + TCP with SYN+RST flags.
    // SYN=0x02, RST=0x04 — both set is a Cloudflare-flagged
    // pathological combination per the kernel sanity prologue.
    let pkt = synthesise_tcp(/* ihl */ 5, /* flags */ 0x02 | 0x04);
    let mut out = vec![0u8; pkt.len()];

    let (action, _out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    assert_eq!(
        action, XDP_DROP,
        "SYN+RST must short-circuit at sanity prologue with XDP_DROP, got {action}"
    );

    let after = read_malformed_header_counter(&bpf);
    assert_eq!(
        after - baseline,
        1,
        "DROP_COUNTER[MalformedHeader] must increment by 1 (baseline={baseline}, after={after})"
    );
}

// ---------- S-2.2-21 — XDP: IPv6 ethertype passes through ----------

#[test]
#[serial(env)]
fn ipv6_ethertype_returns_xdp_pass_no_drop_counter_increment() {
    let LoadedXdp { bpf, prog_fd, _outer_fd, _pin_dir_guard } = load_xdp_program();

    let baseline = read_malformed_header_counter(&bpf);

    // PKTGEN: EtherType 0x86DD (IPv6) — the LB is not a firewall.
    let pkt = synthesise_ipv6_ethertype();
    let mut out = vec![0u8; pkt.len()];

    let (action, _out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    assert_eq!(action, XDP_PASS, "IPv6 EtherType must return XDP_PASS, got {action}");

    let after = read_malformed_header_counter(&bpf);
    assert_eq!(
        after, baseline,
        "DROP_COUNTER[MalformedHeader] MUST NOT increment for IPv6 pass-through (baseline={baseline}, after={after})"
    );
}

// ---------- TC parallel coverage — the same helper, second call site ----------
//
// Per dispatch acceptance criterion: "Invoked from both
// `xdp_service_map_lookup` AND the TC `tc_reverse_nat` egress
// program." The XDP cases above exercise the first call site;
// these confirm the SAME helper fires from the TC path.

#[test]
#[serial(env)]
fn tc_truncated_ipv4_header_drops_with_malformed_header_counter() {
    let LoadedTc { bpf, prog_fd, _outer_fd, _pin_dir_guard } = load_tc_program();

    let baseline = read_malformed_header_counter(&bpf);

    let pkt = synthesise_tcp(/* ihl */ 4, /* flags */ 0x02);
    let mut out = vec![0u8; pkt.len()];

    let (action, _out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    assert_eq!(
        action, TC_ACT_SHOT,
        "IHL=4 on TC path must short-circuit with TC_ACT_SHOT, got {action}"
    );

    let after = read_malformed_header_counter(&bpf);
    assert_eq!(
        after - baseline,
        1,
        "DROP_COUNTER[MalformedHeader] must increment by 1 from TC path (baseline={baseline}, after={after})"
    );
}

#[test]
#[serial(env)]
fn tc_ipv6_ethertype_returns_tc_act_ok_no_drop_counter_increment() {
    let LoadedTc { bpf, prog_fd, _outer_fd, _pin_dir_guard } = load_tc_program();

    let baseline = read_malformed_header_counter(&bpf);

    let pkt = synthesise_ipv6_ethertype();
    let mut out = vec![0u8; pkt.len()];

    let (action, _out_len) =
        bpf_prog_test_run(&prog_fd, &pkt, &mut out).expect("BPF_PROG_TEST_RUN syscall");

    assert_eq!(action, TC_ACT_OK, "IPv6 EtherType on TC path must return TC_ACT_OK, got {action}");

    let after = read_malformed_header_counter(&bpf);
    assert_eq!(
        after, baseline,
        "DROP_COUNTER[MalformedHeader] MUST NOT increment for IPv6 pass-through on TC (baseline={baseline}, after={after})"
    );
}
