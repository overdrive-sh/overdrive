#![allow(clippy::used_underscore_binding)]
//! S-2.2-22 — Mixed legitimate + pathological batch hits per-class
//! `DROP_COUNTER` slots.
//!
//! Tags: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! ```text
//!   client-ns                       lb-ns                          backend-ns
//!     ┌──────────────┐                ┌──────────────────┐           ┌──────────────┐
//!     │ client_veth  │ <──── pair ──> │ lb_veth_a        │           │              │
//!     │ AF_PACKET    │                │ XDP attached     │           │              │
//!     │ inject 80    │                │ here             │           │              │
//!     │ frames       │                │                  │           │              │
//!     └──────────────┘                └──────────────────┘           └──────────────┘
//! ```
//!
//! 80 frames are injected on `client_veth` via a raw `AF_PACKET`
//! socket. They arrive on `lb_veth_a` where `xdp_service_map_lookup`
//! is attached:
//!
//! - 50 well-formed IPv4+TCP SYNs to a VIP that is NOT in SERVICE_MAP
//!   (the empty-map case proves the sanity prologue short-circuits
//!   BEFORE SERVICE_MAP — a regression that ran lookup-then-miss
//!   would surface as the same XDP_PASS verdict but with a different
//!   slot-attribution path; the prologue-short-circuit proof is
//!   structural in `MalformedHeader == 20` exactly).
//! - 10 truncated IPv4 (IHL=4) → `XDP_DROP` + `DROP_COUNTER[MalformedHeader]++`.
//! - 10 SYN+RST TCP frames → `XDP_DROP` + `DROP_COUNTER[MalformedHeader]++`.
//! - 10 IPv6 EtherType frames → `XDP_PASS` (LB is IPv4-only; non-IPv4
//!   passes to the kernel networking stack) + NO drop counter
//!   increment.
//!
//! Acceptance assertions:
//! - `DROP_COUNTER[MalformedHeader]` delta = exactly 20 (not 80,
//!   not 0; the precision distinguishes "sanity fired on the
//!   right packets" from "sanity fired on every packet" or
//!   "didn't fire at all").
//! - `DROP_COUNTER[UnknownVip]` unchanged — SERVICE_MAP miss for
//!   IPv4 traffic returns `XDP_PASS`, not `UnknownVip` drop (per
//!   `xdp_service_map.rs:341`).
//! - `DROP_COUNTER[SanityPrologue]` unchanged — the 06-02 helper
//!   attributes ALL prologue drops to `MalformedHeader` (slot 0);
//!   `SanityPrologue` is reserved for future operator-tunable
//!   sanity rules per architecture.md § 6.

#![allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::items_after_statements,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::print_stderr,
    clippy::too_many_lines
)]

use std::path::PathBuf;

use aya::{
    EbpfLoader,
    programs::{Xdp, XdpFlags},
};
use overdrive_core::dataplane::DropClass;
use overdrive_dataplane::maps::ServiceKey;
use overdrive_dataplane::maps::drop_counter_handle::DropCounterHandle;
use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;

use super::helpers::netns::{NetNsError, ThreeIfaceTopology};
use super::helpers::packets::{
    ETH_HDR_LEN, IPV4_HDR_LEN, TCP_HDR_LEN, ipv4_header_checksum, tcp_checksum,
};

const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// VIP octets used for valid + truncated + SYN+RST frames. SERVICE_MAP
/// is left empty in this test, so a lookup against this VIP misses;
/// the path that matters is the sanity-prologue short-circuit.
const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const SRC_OCTETS: [u8; 4] = [10, 0, 0, 10];

/// Build a well-formed IPv4 + TCP-SYN frame; mirrors `synthesise_tcp_syn`
/// from helpers/packets.rs but parameterised on IHL and TCP flags so
/// the same fn produces the valid baseline + truncated + SYN+RST
/// variants. IHL=5 + flags=SYN gives the canonical valid SYN.
fn synthesise_ipv4_tcp(ihl: u8, tcp_flags: u8, src_port: u16) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet: dst MAC + src MAC + ethertype 0x0800 (IPv4).
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]);

    // IPv4: version=4 | IHL=ihl.
    let ip = ETH_HDR_LEN;
    pkt[ip] = (4 << 4) | (ihl & 0x0F);
    pkt[ip + 1] = 0x00;
    let total_len: u16 = (IPV4_HDR_LEN + TCP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes());
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes());
    pkt[ip + 8] = 0x40;
    pkt[ip + 9] = 0x06;
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes());
    pkt[ip + 12..ip + 16].copy_from_slice(&SRC_OCTETS);
    pkt[ip + 16..ip + 20].copy_from_slice(&VIP_OCTETS);
    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP.
    let tcp = ip + IPV4_HDR_LEN;
    pkt[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&VIP_PORT.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes());
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes());
    pkt[tcp + 12] = 0x50;
    pkt[tcp + 13] = tcp_flags;
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes());
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes());
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes());
    let tcp_csum =
        tcp_checksum(&pkt[ip + 12..ip + 16], &pkt[ip + 16..ip + 20], &pkt[tcp..tcp + TCP_HDR_LEN]);
    pkt[tcp + 16..tcp + 18].copy_from_slice(&tcp_csum.to_be_bytes());

    pkt
}

/// Synthesise an IPv6-EtherType frame. We only need the EtherType
/// to be `0x86DD` — sanity prologue check 1 rejects this before
/// looking at any IPv6 header content.
fn synthesise_ipv6() -> Vec<u8> {
    let mut pkt = synthesise_ipv4_tcp(5, 0x02, 12345);
    pkt[12..14].copy_from_slice(&[0x86, 0xDD]);
    pkt
}

/// Inject `frames` into `iface` via a raw `AF_PACKET` socket bound to
/// the iface's ifindex inside `ns_name`. Mirrors `inject_tcp_syns`
/// from atomic_swap.rs but routed through `ip netns exec` semantics
/// — we execute the inject from inside the namespace via setns.
fn inject_frames_into_ns(
    ns_name: &str,
    iface: &str,
    frames: &[Vec<u8>],
) -> Result<(), std::io::Error> {
    use std::os::fd::RawFd;

    // Enter the target namespace for the duration of the socket
    // bind + send loop. Mirrors the `enter_netns` pattern in
    // reverse_nat_e2e.rs (RAII guard restores the original namespace
    // on Drop).
    let ns_path = format!("/var/run/netns/{ns_name}");
    let original_ns = std::fs::File::open("/proc/self/ns/net").expect("open /proc/self/ns/net");
    let target_ns = std::fs::File::open(&ns_path)?;
    use std::os::fd::AsRawFd;
    // SAFETY: setns(2) with a valid fd to a netns and CLONE_NEWNET.
    let rc = unsafe { libc::setns(target_ns.as_raw_fd(), libc::CLONE_NEWNET) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct NsRestore(std::fs::File);
    impl Drop for NsRestore {
        fn drop(&mut self) {
            // SAFETY: setns to original ns; restore best-effort.
            let _ = unsafe { libc::setns(self.0.as_raw_fd(), libc::CLONE_NEWNET) };
        }
    }
    let _restore = NsRestore(original_ns);

    let cstr = std::ffi::CString::new(iface).expect("iface name");
    // SAFETY: if_nametoindex on a NUL-terminated C string.
    let ifindex = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if ifindex == 0 {
        return Err(std::io::Error::last_os_error());
    }

    const ETH_P_ALL: i32 = 0x0003;
    // SAFETY: AF_PACKET / SOCK_RAW socket.
    let fd: RawFd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, (ETH_P_ALL).to_be()) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: zero-init sockaddr_ll; we populate the live fields below.
    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETH_P_ALL as u16).to_be();
    sll.sll_ifindex = ifindex as i32;
    sll.sll_halen = 6;

    for frame in frames {
        // SAFETY: sendto with sockaddr_ll for the bound iface.
        let rc = unsafe {
            libc::sendto(
                fd,
                frame.as_ptr() as *const _,
                frame.len(),
                0,
                (&sll as *const _) as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            // SAFETY: fd was returned by socket() above.
            unsafe { libc::close(fd) };
            return Err(err);
        }
    }
    // SAFETY: fd was returned by socket() above.
    unsafe { libc::close(fd) };
    Ok(())
}

/// S-2.2-22 — Mixed batch (50 valid + 10 truncated + 10 SYN+RST +
/// 10 IPv6) increments per-class counters correctly.
#[test]
#[serial_test::serial(env)]
fn mixed_batch_increments_per_class_counters_correctly() {
    // CAP_NET_ADMIN + CAP_BPF gate. Lima default-runs as root.
    // SAFETY: `geteuid` reads a kernel-managed numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] mixed-batch needs root for CAP_BPF + CAP_NET_ADMIN; euid={euid}");
        return;
    }

    let topo = match ThreeIfaceTopology::create("smb") {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("[skip] S-2.2-22 needs CAP_NET_ADMIN");
            return;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    // Per-test bpffs pin dir.
    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-smb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_guard = PinDirGuard(pin_dir.clone());

    // Pre-create + pre-pin SERVICE_MAP outer HoM. The kernel-side
    // `xdp_service_map_lookup` references SERVICE_MAP (HoM) which
    // aya 0.13.x cannot create from the ELF alone — the
    // pin-by-name workaround per .claude/rules/development.md
    // § "Sharing the outer HoM ... pinning = ByName".
    let _service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        "SERVICE_MAP",
        4096,
        overdrive_core::dataplane::MaglevTableSize::DEFAULT.get(),
        &pin_dir,
    )
    .expect("SERVICE_MAP pre-create+pin");

    // `OVERDRIVE_BPF_OBJECT_PATH` is emitted as a `cargo:rustc-env=`
    // by `crates/overdrive-dataplane/build.rs`, resolved against the
    // `OVERDRIVE_BPF_OBJECT` override (when set by `cargo xtask
    // mutants`) or the workspace-relative fallback. Single source of
    // truth — no cwd-walking.
    let artifact = std::path::PathBuf::from(env!("OVERDRIVE_BPF_OBJECT_PATH"));
    let mut bpf = EbpfLoader::new()
        .map_pin_path(&pin_dir)
        .allow_unsupported_maps()
        .load_file(&artifact)
        .unwrap_or_else(|e| panic!("EbpfLoader.load_file({}): {e}", artifact.display()));

    // Enter lb-ns to attach XDP — `attach()` resolves the iface
    // index against the calling thread's netns.
    let ns_path = format!("/var/run/netns/{}", topo.lb_ns.name);
    let original_ns = std::fs::File::open("/proc/self/ns/net").expect("open ns");
    let target_ns = std::fs::File::open(&ns_path).expect("open lb-ns");
    use std::os::fd::AsRawFd;
    let rc = unsafe { libc::setns(target_ns.as_raw_fd(), libc::CLONE_NEWNET) };
    assert!(rc == 0, "setns lb-ns: {}", std::io::Error::last_os_error());
    struct NsRestore(std::fs::File);
    impl Drop for NsRestore {
        fn drop(&mut self) {
            let _ = unsafe { libc::setns(self.0.as_raw_fd(), libc::CLONE_NEWNET) };
        }
    }
    let _ns_guard = NsRestore(original_ns);

    // Attach `xdp_service_map_lookup` to lb_veth_a (the iface that
    // receives traffic from client-ns).
    let _xdp_link = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_service_map_lookup")
            .expect("xdp_service_map_lookup not found")
            .try_into()
            .expect("not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.attach(&topo.lb_veth_a, XdpFlags::DRV_MODE)
            .or_else(|_| prog.attach(&topo.lb_veth_a, XdpFlags::SKB_MODE))
            .unwrap_or_else(|e| panic!("attach({}): {e}", topo.lb_veth_a))
    };
    // Restore caller's netns; the BPF link survives — it's bound to
    // the iface ifindex in lb-ns, not to our process's namespace
    // membership.
    drop(_ns_guard);

    // Construct the typed DROP_COUNTER handle. Move out of `bpf` —
    // `from_ebpf` calls `take_map`, so DROP_COUNTER is no longer
    // accessible via `bpf` after this point.
    let drop_counter = DropCounterHandle::from_ebpf(&mut bpf).expect("DropCounterHandle");

    // Baseline drop counter snapshot.
    let baseline = drop_counter.snapshot().expect("baseline snapshot");

    // Build 80-frame batch.
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(80);
    // 50 valid IPv4 TCP SYNs, distinct src ports.
    for i in 0..50_u16 {
        frames.push(synthesise_ipv4_tcp(/* ihl */ 5, /* SYN */ 0x02, 40000 + i));
    }
    // 10 truncated IPv4 (IHL=4).
    for i in 0..10_u16 {
        frames.push(synthesise_ipv4_tcp(/* ihl */ 4, /* SYN */ 0x02, 50000 + i));
    }
    // 10 SYN+RST.
    for i in 0..10_u16 {
        frames.push(synthesise_ipv4_tcp(/* ihl */ 5, /* SYN+RST */ 0x06, 51000 + i));
    }
    // 10 IPv6 EtherType.
    for _ in 0..10 {
        frames.push(synthesise_ipv6());
    }
    assert_eq!(frames.len(), 80, "exactly 80 frames");

    // Inject all 80 from client-ns onto client_veth.
    inject_frames_into_ns(&topo.client_ns.name, &topo.client_veth, &frames)
        .expect("inject 80 frames into client-ns");

    // Allow per-CPU counters time to settle. The kernel updates the
    // per-CPU slot synchronously inside the XDP program, but the
    // userspace PerCpuArray::get reads each CPU's slot via separate
    // bpf(BPF_MAP_LOOKUP_ELEM) syscalls — give the in-flight
    // sendto burst a brief window to drain.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let after = drop_counter.snapshot().expect("after snapshot");

    // Compute deltas.
    let delta = |class: DropClass| {
        let i = class.as_index() as usize;
        after[i].saturating_sub(baseline[i])
    };
    let malformed = delta(DropClass::MalformedHeader);
    let unknown_vip = delta(DropClass::UnknownVip);
    let sanity_prologue = delta(DropClass::SanityPrologue);

    // Diagnostic on failure.
    let diag = format!(
        "baseline={baseline:?} after={after:?} delta(MalformedHeader)={malformed} \
         delta(UnknownVip)={unknown_vip} delta(SanityPrologue)={sanity_prologue}"
    );

    assert_eq!(
        malformed, 20,
        "DROP_COUNTER[MalformedHeader] delta must be exactly 20 (10 truncated + 10 SYN+RST); {diag}"
    );
    assert_eq!(
        unknown_vip, 0,
        "DROP_COUNTER[UnknownVip] must be unchanged (sanity prologue short-circuits before SERVICE_MAP); {diag}"
    );
    assert_eq!(
        sanity_prologue, 0,
        "DROP_COUNTER[SanityPrologue] must be unchanged (06-02 helper attributes to MalformedHeader slot 0); {diag}"
    );
}
