//! P5 — the egress unframe program: boot attach to every interface, the
//! classified attach failures, byte-exact guest datagrams, and a USO
//! interface.
//!
//! netns-density-295 vsock attachment replacement, DISTILL piece P5
//! (`docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`,
//! P5-S1 and P5-S3). Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — host forwarder `overdrive-dataplane::guest_flow`"
//! (D5a, D5a-IFACE, D5a-IFACE-BOOT, USO-NO-SEGMENT; ADR-0165; P-29..P-31,
//! P-43, P-44).
//!
//! Adapter integration: every test drives the real `GuestFlowForwarder`
//! (`load` with a non-root owner cgroup, `converge_unframe_interfaces` over
//! veth interfaces the test creates, `install_datagram` over AF_VSOCK loopback
//! legs and a connected `SO_NO_CHECK` UDP socket) and observes kernel effects:
//! what the far peer receives, and the forwarder's `inventory()` and
//! `counters()`.
//!
//! Lane: Tier-3, example-based (Mandate 11). The whole `overdrive-dataplane`
//! `integration` binary is in the `host-kernel-shared` nextest group.
//!
//! Not covered here: the `ENOMEM` (`KernelMemory`) and `ERANGE` attach
//! classes need a fault seam or 63 foreign TCX programs; the boot refusals
//! (`GuestNetworkError::UnframeAttach` from `converge_shared`,
//! `GuestNetworkError::StackedInterface` from `probe_startup`) and the
//! owner's UDP sockets never setting `UDP_SEGMENT` are the shared owner's
//! (`overdrive-control-plane`).

#![allow(clippy::expect_used, clippy::doc_markdown, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::os::fd::AsFd as _;
use std::path::Path;

use overdrive_dataplane::guest_flow::{
    GUEST_FLOW_MAX_PAIRS, GuestFlowError, GuestFlowForwarder, GuestFlowLinks, UnframeAttachError,
};

use super::helpers::guest_net::{
    DatagramAssociation, OwnerCgroup, PeerLink, assert_cases_arrive_byte_exact, frame,
    interface_index, recv_datagram, require_vsock_loopback, root_cgroup_dir, run_ok, run_stdout,
};

/// A peer UDP socket on `lo` in the root namespace.
fn loopback_peer() -> UdpSocket {
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("precondition: loopback peer binds");
    sock.set_read_timeout(Some(super::helpers::guest_net::RECV_WAIT)).expect("read timeout");
    sock
}

fn v4(sock: &UdpSocket) -> SocketAddrV4 {
    match sock.local_addr().expect("socket has a local address") {
        std::net::SocketAddr::V4(a) => a,
        std::net::SocketAddr::V6(a) => panic!("IPv4 socket expected, got {a}"),
    }
}

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn load_attaches_every_link_and_the_unframe_program_to_lo_only() {
    let cgroup = OwnerCgroup::create("load");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    forwarder.probe().expect("the Earned-Trust probe passes");

    let inv = forwarder.inventory().expect("inventory");
    assert_eq!(
        inv.links_attached,
        GuestFlowLinks { verdict: true, sock_ops: true, unframe_lo: true, drain_counter: true },
        "every program link is attached",
    );
    assert!(inv.unframe_interfaces.is_empty(), "no non-lo interface before the boot attach");
    assert_eq!(inv.pairs, 0, "no pair after load and probe");
    assert_eq!(inv.free_cells, GUEST_FLOW_MAX_PAIRS, "the cell pool is whole");
    assert_eq!(inv.intake_listeners, 0, "no intake listener");

    // The program on `lo` already unframes a registered tuple's datagrams.
    require_vsock_loopback();
    let peer = loopback_peer();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
    assert_cases_arrive_byte_exact(&assoc, &peer, "lo");
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn load_refuses_the_root_cgroup() {
    let root = root_cgroup_dir();
    let result = GuestFlowForwarder::load(root.as_fd());
    assert!(
        matches!(result, Err(GuestFlowError::OwnerCgroupIsRoot)),
        "sock_ops on the root cgroup would apply to every host socket: {:?}",
        result.err(),
    );
}

// ---------------------------------------------------------------------------
// P5-S1 — every interface present at boot carries the egress unframe program
// (D5a-IFACE, D5a-IFACE-BOOT, R5-38, G-V1)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn every_boot_interface_carries_unframe_and_datagrams_arrive_byte_exact() {
    let cgroup = OwnerCgroup::create("converge");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    require_vsock_loopback();

    // GIVEN several independent interfaces present at boot.
    let links: Vec<PeerLink> =
        (0..3).map(|n| PeerLink::create(&format!("vsock-unframe-converge-{n}"), n)).collect();
    let requested: BTreeSet<_> = links.iter().map(|l| interface_index(l.host_ifindex())).collect();

    // WHEN boot converges the unframe program onto them,
    let convergence = forwarder.converge_unframe_interfaces(&requested).expect("converge runs");

    // THEN every requested interface is attached and none failed,
    assert_eq!(convergence.attached, requested, "every present interface is attached");
    assert!(convergence.failed.is_empty(), "no attach failed: {:?}", convergence.failed);
    let inv = forwarder.inventory().expect("inventory");
    assert_eq!(inv.unframe_interfaces, requested, "the inventory lists every converged interface");
    assert!(inv.links_attached.unframe_lo, "lo still carries the program");

    // AND every size class leaves through each interface byte-exact,
    let before = forwarder.counters().expect("counters");
    for link in &links {
        let peer = link.peer_udp();
        let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
        assert_cases_arrive_byte_exact(&assoc, &peer, &link.host_if);
    }

    // AND the verdict kept the frame exactly for the empty and the
    // frame-shaped payloads (two frame-shaped cases per interface).
    let after = forwarder.counters().expect("counters");
    let n = links.len() as u64;
    assert_eq!(
        after.frame_kept_empty - before.frame_kept_empty,
        n,
        "one empty datagram per interface"
    );
    assert_eq!(
        after.frame_kept_escaped - before.frame_kept_escaped,
        2 * n,
        "two frame-shaped payloads per interface",
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn on_a_registered_tuple_only_a_valid_frame_is_removed() {
    let cgroup = OwnerCgroup::create("tuple");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    require_vsock_loopback();
    let link = PeerLink::create("vsock-unframe-tuple", 0);
    let ifaces: BTreeSet<_> = BTreeSet::from([interface_index(link.host_ifindex())]);
    forwarder.converge_unframe_interfaces(&ifaces).expect("converge");
    let peer = link.peer_udp();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));

    // A valid frame written on the registered tuple's socket H is removed.
    assoc.host_udp.send(&frame(b"inner")).expect("send on H");
    assert_eq!(recv_datagram(&peer).as_deref(), Some(&b"inner"[..]), "a valid frame is removed");

    // Anything else passes unchanged: never dropped for bad magic.
    for (label, bytes) in [
        ("no frame", b"plain datagram".to_vec()),
        ("near miss", {
            let mut v = frame(b"abc");
            v[7] = 9;
            v
        }),
        ("too short", b"OVD".to_vec()),
    ] {
        assoc.host_udp.send(&bytes).expect("send on H");
        assert_eq!(recv_datagram(&peer), Some(bytes), "{label}: passes unchanged");
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn an_unregistered_datagram_passes_a_converged_interface_unchanged() {
    let cgroup = OwnerCgroup::create("unregistered");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    let link = PeerLink::create("vsock-unframe-unregistered", 0);
    let ifaces: BTreeSet<_> = BTreeSet::from([interface_index(link.host_ifindex())]);
    forwarder.converge_unframe_interfaces(&ifaces).expect("converge");
    let peer = link.peer_udp();

    // A socket the forwarder never registered sends a frame-shaped payload.
    let plain = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).expect("UDP binds");
    let payload = frame(b"not-an-association");
    plain.send_to(&payload, v4(&peer)).expect("send");
    assert_eq!(recv_datagram(&peer), Some(payload), "unregistered packets always pass");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn oversize_and_malformed_guest_datagrams_are_dropped_and_counted() {
    let cgroup = OwnerCgroup::create("drop");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    require_vsock_loopback();
    let peer = loopback_peer();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
    let before = forwarder.counters().expect("counters");

    // One byte over the largest guest datagram: dropped, counted.
    let over = usize::try_from(overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_PAYLOAD)
        .expect("fits usize")
        + 1;
    assoc.guest_sends(&vec![0x11; over]);
    assert_eq!(recv_datagram(&peer), None, "an oversize datagram is not delivered");

    // A record that is not a frame: dropped, counted.
    assoc.guest_sends_record(b"XXXX\0\0\0\x03abc");
    assert_eq!(recv_datagram(&peer), None, "a malformed record is not delivered");

    let after = forwarder.counters().expect("counters");
    assert_eq!(after.datagram_oversize - before.datagram_oversize, 1, "oversize counted");
    assert_eq!(after.datagram_malformed - before.datagram_malformed, 1, "malformed counted");

    // The association still carries a well-formed datagram afterwards.
    assoc.guest_sends(b"still-paired");
    assert_eq!(recv_datagram(&peer).as_deref(), Some(&b"still-paired"[..]));
}

// ---------------------------------------------------------------------------
// P5-S1 sad paths — each failure lands in `failed` with its interface; a
// failure never stops the others (D5a-IFACE-BOOT; P-44 errno map)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn a_second_attach_to_an_attached_interface_fails_naming_it() {
    let cgroup = OwnerCgroup::create("eexist");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    let link = PeerLink::create("vsock-unframe-eexist", 0);
    let index = interface_index(link.host_ifindex());
    let one: BTreeSet<_> = BTreeSet::from([index]);

    let first = forwarder.converge_unframe_interfaces(&one).expect("first converge");
    assert_eq!(first.attached, one, "first attach succeeds");

    // WHEN the same program is attached again (EEXIST),
    let second = forwarder.converge_unframe_interfaces(&one).expect("converge runs");
    // THEN the interface lands in `failed` as `Attach`, naming it,
    assert!(second.attached.is_empty(), "nothing newly attached");
    let err = second.failed.get(&index).expect("the interface is in `failed`");
    assert!(matches!(err, UnframeAttachError::Attach { .. }), "EEXIST -> Attach: {err:?}");
    assert_eq!(err.ifindex(), index, "the error names the interface");
    // AND nothing was detached: the interface still carries the link.
    let inv = forwarder.inventory().expect("inventory");
    assert_eq!(inv.unframe_interfaces, one, "never detaches anything");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn an_interface_gone_before_its_attach_fails_naming_it_and_the_others_attach() {
    let cgroup = OwnerCgroup::create("enodev");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    let present = PeerLink::create("vsock-unframe-enodev-0", 0);
    let gone = PeerLink::create("vsock-unframe-enodev-1", 1);
    let present_index = interface_index(present.host_ifindex());
    let gone_index = interface_index(gone.host_ifindex());

    // GIVEN an interface enumerated at boot is deleted before its attach (ENODEV).
    drop(gone);

    let requested: BTreeSet<_> = BTreeSet::from([present_index, gone_index]);
    let convergence = forwarder.converge_unframe_interfaces(&requested).expect("converge runs");

    // THEN the vanished interface fails as `Attach` naming it, and the
    // present one is still attached.
    assert_eq!(convergence.attached, BTreeSet::from([present_index]), "the other attaches");
    let err = convergence.failed.get(&gone_index).expect("the vanished interface is in `failed`");
    assert!(matches!(err, UnframeAttachError::Attach { .. }), "ENODEV -> Attach: {err:?}");
    assert_eq!(err.ifindex(), gone_index, "the error names the interface");
}

/// Caps this process's descriptor table at its current use so the next
/// descriptor any call opens fails `EMFILE`; restores the limit on `Drop`.
/// nextest runs each test in its own process, so only this test is affected.
struct DescriptorLimit {
    saved: libc::rlimit,
    _fillers: Vec<fs::File>,
}

impl DescriptorLimit {
    fn exhaust() -> Self {
        let mut saved = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: a valid out-pointer to an `rlimit`.
        let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut saved) };
        assert_eq!(rc, 0, "precondition: getrlimit");
        let highest = fs::read_dir("/proc/self/fd")
            .expect("precondition: /proc/self/fd is readable")
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u64>().ok())
            .max()
            .expect("at least one open descriptor");
        let capped = libc::rlimit { rlim_cur: highest + 1, rlim_max: saved.rlim_max };
        // SAFETY: a valid pointer to an `rlimit` whose soft limit is below the hard limit.
        let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const capped) };
        assert_eq!(rc, 0, "precondition: setrlimit");
        // Fill every free slot below the cap.
        let mut fillers = Vec::new();
        while let Ok(f) = fs::File::open("/dev/null") {
            fillers.push(f);
        }
        Self { saved, _fillers: fillers }
    }
}

impl Drop for DescriptorLimit {
    fn drop(&mut self) {
        // SAFETY: restores the limit read at construction.
        let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const self.saved) };
        if rc != 0 {
            eprintln!("cleanup: RLIMIT_NOFILE not restored");
        }
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn a_descriptor_limit_at_attach_fails_as_attach_naming_the_interface() {
    let cgroup = OwnerCgroup::create("emfile");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    let link = PeerLink::create("vsock-unframe-emfile", 0);
    let index = interface_index(link.host_ifindex());
    let one: BTreeSet<_> = BTreeSet::from([index]);

    let convergence = {
        let _limit = DescriptorLimit::exhaust();
        forwarder.converge_unframe_interfaces(&one).expect("converge runs (the program is loaded)")
    };
    let err = convergence.failed.get(&index).expect("the interface is in `failed`");
    assert!(matches!(err, UnframeAttachError::Attach { .. }), "EMFILE -> Attach: {err:?}");
    assert_eq!(err.ifindex(), index, "the error names the interface");
    assert!(convergence.attached.is_empty(), "nothing attached");
}

// ---------------------------------------------------------------------------
// P5-S3 — guest datagrams survive a UDP-segmentation-offload interface
// byte-exact (USO-NO-SEGMENT, R5-46, G-V6; P-43)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder)"]
fn guest_datagrams_survive_a_uso_interface_byte_exact() {
    let cgroup = OwnerCgroup::create("uso");
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    require_vsock_loopback();
    let link = PeerLink::create("vsock-unframe-uso", 0);
    run_ok("ethtool", &["-K", &link.host_if, "tx-udp-segmentation", "on"]);
    let features = run_stdout("ethtool", &["-k", &link.host_if]);
    assert!(
        features.lines().any(|l| l.trim() == "tx-udp-segmentation: on"),
        "precondition: the interface offers UDP segmentation offload",
    );
    let ifaces: BTreeSet<_> = BTreeSet::from([interface_index(link.host_ifindex())]);
    forwarder.converge_unframe_interfaces(&ifaces).expect("converge");

    let peer = link.peer_udp();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
    assert_cases_arrive_byte_exact(&assoc, &peer, &link.host_if);
}

/// USO-NO-SEGMENT source check, forwarder half: the forwarder crate never
/// references `UDP_SEGMENT`. (The owner crate's half belongs to
/// `overdrive-control-plane`.) A guard: it already holds before DELIVER.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (USO-NO-SEGMENT source check)"]
fn the_forwarder_crate_never_references_udp_segment() {
    fn scan(dir: &Path, hits: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("source dir is readable") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                scan(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).expect("source file is UTF-8");
                if text.contains("UDP_SEGMENT") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let mut hits = Vec::new();
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "UDP_SEGMENT referenced in the forwarder crate: {hits:?}");
}
