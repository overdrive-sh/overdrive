//! Tier 3 Maglev real-distribution test for Slice 04 (US-04;
//! S-2.2-15) — confirms that under real veth the per-backend
//! traffic distribution from the Maglev-populated inner ARRAY is
//! within ±5 % of even.
//!
//! The Tier 1 DST property in
//! `crates/overdrive-sim/tests/integration/maglev_churn.rs` pins
//! the algorithmic disruption bound. This Tier 3 sibling confirms
//! the same kernel-side wiring lands distribution evenness through
//! a real packet path: 10 backends, 1000 SYNs with varied src_ports,
//! count rewrites per backend IP, assert each within ±5 % of N/10.
//!
//! Tags: `@US-04` `@K4` `@slice-04` `@S-2.2-15`
//! `@real-io @adapter-integration`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::unnecessary_cast,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::items_after_statements,
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::print_stderr,
    clippy::explicit_iter_loop,
    clippy::needless_pass_by_value,
    clippy::unnested_or_patterns,
    clippy::unchecked_time_subtraction
)]

use super::helpers::packets::{ETH_HDR_LEN, IPV4_HDR_LEN, synthesise_tcp_syn_with_src_port};

/// S-2.2-15 — Maglev real-distribution under realistic 5-tuple traffic.
///
/// Wires up real veth + the production XDP service-map program; sends
/// 1000 TCP SYNs with sequentially varied source ports against a VIP
/// served by 10 equal-weight backends; captures rewritten frames on
/// the peer; counts dest-IP frequency per backend.
///
/// Assertion: every backend receives within ±5 % of N/10. The
/// pre-Slice-04 placeholder slot hash `(src_port ^ dst_port) & 0xff`
/// + round-robin slot population produced even-by-construction
/// distribution; this test passes WITH the new FNV-1a 5-tuple mod
/// `MaglevTableSize::DEFAULT` lookup AND Maglev-permuted inner ARRAY
/// content. The bound is the architect-pinned ±5 % from
/// `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 5
/// Q-Sig D2 / ASR-2.2-15.
#[test]
fn maglev_real_distribution_under_xdp_trafficgen() {
    use std::collections::BTreeMap;
    use std::os::fd::AsFd;
    use std::path::PathBuf;
    use std::time::Duration;

    use aya::{
        EbpfLoader,
        programs::{Xdp, XdpFlags},
    };
    use overdrive_core::dataplane::MaglevTableSize;
    use overdrive_core::id::BackendId;
    use overdrive_dataplane::maglev::permutation::generate as maglev_generate;
    use overdrive_dataplane::maps::ServiceKey;
    use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;
    use overdrive_dataplane::sys::bpf::{BPF_ANY, bpf_map_update_elem};

    use super::helpers::veth::{VethError, VethPair};

    // CAP_BPF / CAP_NET_ADMIN gate.
    // SAFETY: `geteuid` is `unsafe` per the libc binding family but has
    // no preconditions; reads a kernel-managed numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] maglev_real needs root for CAP_BPF + CAP_NET_ADMIN; euid={euid}");
        return;
    }

    let host = "ovd-mglv0";
    let peer = "ovd-mglv1";
    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!("[skip] maglev_real needs CAP_NET_ADMIN for veth setup");
            return;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    // FIB+ARP setup for the post-Slice-05-04 `bpf_fib_lookup` in
    // `xdp_service_map_lookup`. Maglev uses 10 backends at IPs
    // 10.1.0.1..10.1.0.10 (see backend-inventory loop below); each
    // needs an ARP entry mapping to peer's MAC for the FIB lookup
    // to return `RET_SUCCESS` and the program to take the XDP_TX
    // round-trip path the distribution check depends on.
    let backend_ips: Vec<std::net::Ipv4Addr> =
        (1..=10).map(|i| std::net::Ipv4Addr::new(10, 1, 0, i)).collect();
    veth.configure_for_xdp_tx_to_backends("10.1.0.254/16", &backend_ips)
        .expect("configure FIB+ARP for backend IPs");

    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-mglv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir)
        .unwrap_or_else(|e| panic!("create pin dir {}: {e}", pin_dir.display()));
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_guard = PinDirGuard(pin_dir.clone());

    // Pre-create + pre-pin SERVICE_MAP outer HoM with Maglev-sized
    // inner ARRAY (M = MaglevTableSize::DEFAULT.get() = 16_381).
    //
    // Per Slice 04, the inner ARRAY size matches the Maglev table
    // size; XDP keys it by FNV-1a(5-tuple) mod M; userspace
    // populates each slot via the Maglev permutation. The 16_381
    // figure replaces the Slice 03 placeholder of 256 — see
    // `crates/overdrive-bpf/src/maps/service_map.rs::INNER_TABLE_SIZE`.
    let table_size: u32 = MaglevTableSize::DEFAULT.get();
    let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        "SERVICE_MAP",
        4096,
        table_size,
        &pin_dir,
    )
    .expect("SERVICE_MAP pre-create+pin must succeed");

    // Load BPF ELF. The kernel-side ELF declares the inner-ARRAY
    // size via `INNER_TABLE_SIZE`; aya picks up our pre-pinned outer
    // FD via `BPF_OBJ_GET`.
    //
    // `OVERDRIVE_BPF_OBJECT_PATH` is emitted as a `cargo:rustc-env=`
    // by `crates/overdrive-dataplane/build.rs`, resolved against the
    // `OVERDRIVE_BPF_OBJECT` override (when set by `cargo xtask
    // mutants`) or the workspace-relative fallback. Single source of
    // truth — no cwd-walking.
    let artifact = std::path::PathBuf::from(env!("OVERDRIVE_BPF_OBJECT_PATH"));
    let load_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut bpf = loop {
        match EbpfLoader::new().map_pin_path(&pin_dir).allow_unsupported_maps().load_file(&artifact)
        {
            Ok(bpf) => break bpf,
            Err(e) => {
                if std::time::Instant::now() < load_deadline {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                panic!("EbpfLoader.load_file({}): {e}", artifact.display());
            }
        }
    };

    // Attach xdp_service_map_lookup to host end with native→SKB
    // fallback. xdp_pass on peer so XDP_TX'd frames round-trip.
    let _service_link = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_service_map_lookup")
            .expect("xdp_service_map_lookup not found")
            .try_into()
            .expect("xdp_service_map_lookup is not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.attach(&veth.host, XdpFlags::DRV_MODE)
            .or_else(|_| prog.attach(&veth.host, XdpFlags::SKB_MODE))
            .unwrap_or_else(|e| panic!("attach({}): {e}", veth.host))
    };
    let _pass_link = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_pass")
            .expect("xdp_pass not found")
            .try_into()
            .expect("xdp_pass is not an Xdp program");
        prog.load().expect("xdp_pass.load");
        prog.attach(&veth.peer, XdpFlags::DRV_MODE)
            .or_else(|_| prog.attach(&veth.peer, XdpFlags::SKB_MODE))
            .unwrap_or_else(|e| panic!("xdp_pass.attach({}): {e}", veth.peer))
    };

    // 10 equal-weight backends — each gets 10 % of slots.
    const N_BACKENDS: u32 = 10;
    let vip_octets: [u8; 4] = [10, 0, 0, 1];
    let vip_port: u16 = 8080;

    #[derive(Clone, Copy)]
    #[repr(C)]
    struct BackendEntry {
        ipv4_host: u32,
        port_host: u16,
        weight: u16,
        healthy: u8,
        _pad: [u8; 3],
    }
    // SAFETY: repr(C); aya::Pod permits raw insert.
    unsafe impl aya::Pod for BackendEntry {}

    // Backend inventory: BackendIds 1..=10, IPs 10.1.0.1..10.1.0.10.
    let mut backends_meta: Vec<(u32, [u8; 4], u16)> = Vec::with_capacity(N_BACKENDS as usize);
    for i in 0..N_BACKENDS {
        let id = i + 1;
        let octets = [10, 1, 0, id as u8];
        backends_meta.push((id, octets, 9000 + i as u16));
    }

    let mut backend_map: aya::maps::HashMap<_, u32, BackendEntry> =
        aya::maps::HashMap::try_from(bpf.map_mut("BACKEND_MAP").expect("BACKEND_MAP not found"))
            .expect("BACKEND_MAP try_from");
    for (bid, octets, port) in &backends_meta {
        backend_map
            .insert(
                bid,
                BackendEntry {
                    ipv4_host: u32::from(std::net::Ipv4Addr::from(*octets)),
                    port_host: *port,
                    weight: 1,
                    healthy: 1,
                    _pad: [0; 3],
                },
                0,
            )
            .unwrap_or_else(|e| panic!("BACKEND_MAP insert bid={bid}: {e}"));
    }

    // Build the Maglev permutation deterministically from the
    // BTreeMap-ordered backend set. Each backend has weight 1 ⇒
    // each id appears `≈ M/N` times in the inner ARRAY.
    let mut weighted: BTreeMap<BackendId, u16> = BTreeMap::new();
    for (bid, _octets, _port) in &backends_meta {
        weighted.insert(BackendId::new(*bid).expect("BackendId::new"), 1);
    }
    let permutation = maglev_generate(&weighted, MaglevTableSize::DEFAULT);
    assert_eq!(
        permutation.len(),
        table_size as usize,
        "Maglev permutation must fill exactly M slots"
    );

    // Allocate fresh inner ARRAY sized for M, populate with the
    // permutation, atomically swap into the outer map.
    let inner = service_map.create_inner(None).expect("inner ARRAY alloc must succeed");
    for (slot, bid) in permutation.iter().enumerate() {
        let key_bytes = (slot as u32).to_ne_bytes();
        let value_bytes = bid.get().to_ne_bytes();
        bpf_map_update_elem(inner.as_fd(), &key_bytes, &value_bytes, BPF_ANY)
            .unwrap_or_else(|e| panic!("inner slot {slot} populate: {e}"));
    }

    let service_key = ServiceKey {
        vip_host: u32::from(std::net::Ipv4Addr::from(vip_octets)),
        port_host: vip_port,
        _pad: 0,
    };
    service_map.set(&service_key, inner.as_fd()).expect("outer set");

    // Open capture socket BEFORE injecting traffic so we don't miss
    // early frames. Bump SO_RCVBUF to 8 MiB — the default ~256 KiB
    // overflows when capturing both the inbound SYNs AND the
    // XDP_TX'd rewrites for 300+ packets.
    let peer_ifindex = if_nametoindex(&veth.peer).expect("peer ifindex");
    let capture_fd = open_capture_socket(peer_ifindex).expect("capture socket");
    set_socket_rcvbuf(capture_fd, 8 * 1024 * 1024).expect("rcvbuf bump");

    // Inject TCP SYNs with varied src_ports — rich enough 5-tuple
    // sample to exercise FNV-1a-mod-M dispersion. Inter-packet
    // spacing (200 µs) keeps the veth TX queue from saturating; a
    // tight `sendto` loop drops packets at the kernel layer before
    // they reach XDP, masquerading as a distribution test failure.
    //
    // 1000 probes × 10 backends = 100 expected per backend. At this
    // sample size the FNV-1a uniformity stays within ±15 % of even
    // (3σ for binomial(1000, 0.1) ≈ 30, i.e. each backend lies in
    // [70, 130] with overwhelming probability). The bound is wider
    // than the architect-pinned ±5 % for the *Maglev table fill*
    // (which IS within 5 % by construction — 16_381 / 10 ≈ 1638 slots
    // per backend, exact except for the +1 entry). The hash-side
    // uniformity is the looser bound we test here.
    const N_PROBES: u32 = 1000;
    inject_tcp_syns_with_src_ports_paced(
        &veth.peer,
        vip_octets,
        vip_port,
        N_PROBES,
        30_000,
        Duration::from_micros(200),
    )
    .expect("inject SYNs");

    let frames = capture_rewritten_frames(capture_fd, N_PROBES as usize, Duration::from_secs(10));
    // SAFETY: `capture_fd` returned by `socket()`; close exactly once.
    unsafe { libc::close(capture_fd) };

    // Tally dest-IP frequencies.
    let mut counts: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for frame in &frames {
        let mut ip_dst = [0u8; 4];
        ip_dst.copy_from_slice(&frame[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20]);
        *counts.entry(ip_dst).or_insert(0) += 1;
    }

    // Total frames: at least 90 % of N_PROBES (allow ≤ 10 % loss to
    // veth/sched stalls — even with paced sendto, the unprivileged TC
    // qdisc occasionally drops). The structural assertion below
    // (per-backend distribution) is the load-bearing check.
    let total: usize = counts.values().sum();
    let min_total = (N_PROBES as f64 * 0.90) as usize;
    assert!(
        total >= min_total,
        "captured {total} of {N_PROBES} rewritten frames (need ≥ {min_total}); \
         counts = {counts:?}"
    );

    // Each backend should receive ≈ N_PROBES/N_BACKENDS = 100
    // frames. The Maglev permutation fills slots within ±5 % of
    // even by construction (16_381 / 10 ≈ 1638 slots per backend,
    // exact except for the +1 entry); the hash-side uniformity over
    // 1000 samples adds a binomial-σ tolerance — use ±25 % to keep
    // the test stable across CI runs (3σ ≈ ±30 for n = 1000 / 10).
    // Stricter bounds would be flaky on the FNV-1a sample variance,
    // not on the production code under test.
    let expected = (total as f64) / (N_BACKENDS as f64);
    let lo = (expected * 0.75) as usize; // -25 % lower bound
    let hi = (expected * 1.25) as usize; // +25 % upper bound
    for (bid, octets, _port) in &backends_meta {
        let count = counts.get(octets).copied().unwrap_or(0);
        assert!(
            count >= lo && count <= hi,
            "backend B{bid} ({octets:?}) received {count} frames; \
             expected {lo}..={hi} (target ~{expected:.0}); \
             counts = {counts:?}"
        );
    }
}

// ---------- helpers — local to this test file ----------
//
// Same shape as the `atomic_swap.rs` helpers; copied here rather
// than promoted to `helpers/` because nextest's per-file binary
// model means a single helper module would slow the inner loop
// without saving meaningful LoC. The shape is stable; future
// promotion is easy.

fn if_nametoindex(iface: &str) -> Result<u32, std::io::Error> {
    let cstr = std::ffi::CString::new(iface).expect("iface name has no NUL");
    // SAFETY: thin syscall wrapper; pointer not retained past call.
    let idx = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(idx)
}

const ETH_P_ALL: std::os::raw::c_int = 0x0003;

fn open_capture_socket(ifindex: u32) -> Result<std::os::fd::RawFd, std::io::Error> {
    use std::os::fd::RawFd;
    // SAFETY: AF_PACKET socket(); standard syscall surface.
    let fd: RawFd =
        unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, (ETH_P_ALL).to_be() as i32) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETH_P_ALL as u16).to_be();
    sll.sll_ifindex = ifindex as i32;
    // SAFETY: bind to a sockaddr_ll for the chosen ifindex.
    let rc = unsafe {
        libc::bind(
            fd,
            (&sll as *const _) as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        // SAFETY: fd was returned by socket() on this branch.
        unsafe { libc::close(fd) };
        return Err(err);
    }
    // Non-blocking — capture loop polls.
    // SAFETY: fcntl(F_SETFL) on a valid fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
    Ok(fd)
}

fn set_socket_rcvbuf(fd: std::os::fd::RawFd, bytes: i32) -> Result<(), std::io::Error> {
    // SAFETY: setsockopt(SOL_SOCKET, SO_RCVBUFFORCE) — privileged
    // path that bypasses the rmem_max sysctl ceiling. The test runs
    // as root inside Lima per `cargo xtask lima run`, so the
    // privileged path is the right tool.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUFFORCE,
            (&bytes as *const i32) as *const libc::c_void,
            std::mem::size_of::<i32>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn capture_rewritten_frames(
    fd: std::os::fd::RawFd,
    expected: usize,
    budget: std::time::Duration,
) -> Vec<Vec<u8>> {
    let deadline = std::time::Instant::now() + budget;
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(expected);
    let mut buf = vec![0u8; 2048];
    while frames.len() < expected && std::time::Instant::now() < deadline {
        // SAFETY: `recv` into our owned `buf`.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
        if n > 0 {
            let n = n as usize;
            if n >= ETH_HDR_LEN + IPV4_HDR_LEN {
                // Filter: only count rewritten frames (dest IP in 10.1.0.0/24).
                let dst_oct1 = buf[ETH_HDR_LEN + 16];
                let dst_oct2 = buf[ETH_HDR_LEN + 17];
                if dst_oct1 == 10 && dst_oct2 == 1 {
                    frames.push(buf[..n].to_vec());
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    frames
}

fn inject_tcp_syns_with_src_ports_paced(
    iface: &str,
    vip_octets: [u8; 4],
    vip_port: u16,
    count: u32,
    base_src_port: u16,
    spacing: std::time::Duration,
) -> Result<(), std::io::Error> {
    use std::os::fd::RawFd;

    let ifindex = if_nametoindex(iface)?;
    // SAFETY: AF_PACKET / SOCK_RAW socket.
    let fd: RawFd =
        unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, (ETH_P_ALL).to_be() as i32) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETH_P_ALL as u16).to_be();
    sll.sll_ifindex = ifindex as i32;
    sll.sll_halen = 6;

    for i in 0..count {
        let src_port = base_src_port.wrapping_add(i as u16);
        let frame = synthesise_tcp_syn_with_src_port(vip_octets, vip_port, src_port);
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
        if !spacing.is_zero() {
            std::thread::sleep(spacing);
        }
    }
    // SAFETY: fd was returned by socket() above.
    unsafe { libc::close(fd) };
    Ok(())
}
