//! Gateway selection hook and connect-dataplane Tier-3 scenarios on a real
//! kernel (public-ingress-gateway; S-PIG-41, 56 (T3 half), 57 (T3 half), 63,
//! 71 (instance half), 72, 124, 125).
//!
//! Native x86_64 metal only (`cargo xtask metal run --`): every test builds the
//! production `EbpfDataplane` on a veth pair with the cgroup `connect4` attach at
//! the root cgroup, as the other Tier-3 tests in this binary do, and drives the
//! `Dataplane` and `GatewayConnectDataplane` ports of that one instance. The
//! oracles are what the kernel and the sockets show: receipts and identities
//! through the port, accepted peers at a real listener, `SO_MARK` of the
//! prepared socket, `nft`/`ip` snapshots taken before and after.
//!
//! No test in this file installs a map entry, nft rule or route that production
//! does not install. The shared node-owner nft rules (output chain, TPROXY
//! prerouting rule, table 100) belong to the `serve` composition, not to this
//! binary; their behaviour on the gateway path (a selected-then-stopped backend
//! reset at the output rule, the dial entering leg-C) is the native lane's
//! (S-PIG-56 and 63 N arms, `tests/conformance`). This file proves the part the
//! dataplane owns: the receipt, the applied identity, the prepared socket, the
//! rewritten destination and the absence of residue.
//!
//! The exact-peer wire evidence (S-PIG-64) and the mTLS negatives (S-PIG-38,
//! 43, 52) are not in this file: they need `HostGatewayClientMtls` through
//! `HostGatewayClientMtlsFactory`. S-PIG-75 is dropped (R-21).

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn,
    clippy::significant_drop_tightening,
    reason = "CONTRACT_SHAPE metadata, long real-kernel scenario bodies and fixture preconditions"
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::{MTLS_TPROXY_MARK, Proto};
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewaySelectionReceipt,
};

use super::helpers::gateway_connect_rig::{
    HostRig, alloc, backend, backend_target, dial, frontend, frontend_target, routing_snapshot,
    service_key, snapshot,
};
use super::helpers::relay_peers::WallClock;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
}

/// A scalar socket option of the socket behind `fd`, as the kernel reports it.
fn socket_option<T: Default>(fd: RawFd, name: libc::c_int) -> T {
    let mut value = T::default();
    let mut len = std::mem::size_of::<T>() as libc::socklen_t;
    // SAFETY: `fd` is a live socket owned by the caller for the duration of the
    // call; `value` and `len` are valid for writes of the sizes passed.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            name,
            std::ptr::addr_of_mut!(value).cast(),
            std::ptr::addr_of_mut!(len),
        )
    };
    assert_eq!(rc, 0, "getsockopt({name}): {}", std::io::Error::last_os_error());
    value
}

/// `(live intents, live receipts)` of the rig's dataplane.
fn live_counts(rig: &HostRig) -> (u32, usize) {
    (
        rig.dataplane.live_intent_count().expect("live intent count"),
        rig.dataplane.live_receipts().expect("live receipts").len(),
    )
}

/// S-PIG-41. A socket that was never registered keeps today's behaviour under
/// the same cgroup hooks: a connect to an existing local-backend VIP is rewritten
/// by `LOCAL_BACKEND_MAP` exactly as on `main`, a connect to a closed port is
/// refused by the kernel (not denied by a hook), and `LOCAL_BACKEND_MAP` and the
/// XDP attachments are unchanged. Expected GREEN from authoring: nothing here
/// touches a gateway scaffold.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn unregistered_cgroup_socket_keeps_non_service_behaviour() {
    let rig = HostRig::new("h41");
    let rt = runtime();

    let xdp = |iface: &str| snapshot("ip", &["-d", "link", "show", "dev", iface]);
    let entries = |rig: &HostRig| {
        let mut rows: Vec<(u32, u16, u8, u32, u16)> = rig
            .dataplane
            .local_backend_map_entries()
            .expect("dump LOCAL_BACKEND_MAP")
            .iter()
            .map(|(key, value)| {
                (
                    key.vip_host,
                    key.port_host,
                    key.proto,
                    value.backend_ip_host,
                    value.backend_port_host,
                )
            })
            .collect();
        rows.sort_unstable();
        rows
    };

    // The existing local-backend path (ADR-0053): VIP:7777 -> an echo listener.
    let local_vip = Ipv4Addr::new(198, 18, 251, 1);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind echo listener");
    let std::net::SocketAddr::V4(backend_address) = listener.local_addr().expect("local addr")
    else {
        unreachable!("bound an IPv4 listener")
    };
    rt.block_on(rig.dataplane.register_local_backend(local_vip, 7777, backend_address, Proto::Tcp))
        .expect("register the local backend through the existing path");

    let before_map = entries(&rig);
    let before_xdp = (xdp(&rig.veth.host), xdp(&rig.veth.peer));
    assert_eq!(before_map.len(), 1, "one local backend registered: {before_map:?}");

    let echo = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).expect("read");
        stream.write_all(&buf[..n]).expect("echo");
    });
    let mut stream = TcpStream::connect((local_vip, 7777))
        .expect("an unregistered connect to the local VIP is rewritten and completes");
    stream.write_all(b"unregistered").expect("write");
    let mut reply = [0u8; 12];
    stream.read_exact(&mut reply).expect("read the echo");
    assert_eq!(&reply, b"unregistered", "the existing rewrite reached the local backend");
    echo.join().expect("echo thread");

    // A connection to a closed port is refused by the kernel itself.
    let closed = {
        let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        probe.local_addr().expect("addr")
    };
    let refused = TcpStream::connect(closed).expect_err("nothing listens on the closed port");
    assert_eq!(refused.kind(), std::io::ErrorKind::ConnectionRefused, "a hook must not deny it");

    assert_eq!(entries(&rig), before_map, "LOCAL_BACKEND_MAP unchanged");
    assert_eq!((xdp(&rig.veth.host), xdp(&rig.veth.peer)), before_xdp, "XDP attachments unchanged");
}

/// S-PIG-63. A registered socket is selected by the hook: the receipt is
/// `Selected` with the applied identity of the backend, the rewritten dial
/// reaches the backend's address from the `127.0.0.1` source with the prepared
/// socket carrying `SO_MARK` = `MTLS_TPROXY_MARK`, one `cleanup` leaves both live
/// counts at zero, and the test installs no nft rule or route (snapshots equal).
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-selection-hook"]
fn registered_socket_is_selected_and_its_dial_reaches_the_backend_from_the_loopback_source() {
    let rig = HostRig::new("h63");
    let rt = runtime();
    let routing_before = routing_snapshot();

    let listener = TcpListener::bind(backend_target(63)).expect("bind the backend address");
    let accept = std::thread::spawn(move || listener.accept().map(|(_, peer)| peer));
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(63, 1, true)]))
        .expect("the Service is programmed through the production update path");

    let prepared = rig.dataplane.open_upstream(service_key(1)).expect("open_upstream");
    let (socket, intent) = prepared.into_parts();
    assert_eq!(
        socket_option::<u32>(socket.as_raw_fd(), libc::SO_MARK),
        MTLS_TPROXY_MARK,
        "the socket carries the mark"
    );
    assert_eq!(
        socket.local_addr().expect("bound local address").ip(),
        Ipv4Addr::LOCALHOST,
        "the socket is bound to the loopback source",
    );
    assert_eq!(live_counts(&rig), (1, 0), "one registered intent, no receipt before the dial");

    let stream = rt
        .block_on(dial(socket, frontend_target(1), Duration::from_secs(5)))
        .expect("the rewritten dial reaches the backend address");
    let peer = accept.join().expect("accept thread").expect("the backend accepted the dial");
    assert_eq!(peer.ip(), Ipv4Addr::LOCALHOST, "the dial arrives from 127.0.0.1");
    drop(stream);

    let receipt = rig.dataplane.take_receipt(&intent).expect("the receipt of this socket");
    let GatewaySelectionReceipt::Selected { socket_cookie, service_key: key, backend_id } = receipt
    else {
        panic!("a programmed Service is Selected, got {receipt:?}");
    };
    assert_eq!(socket_cookie, intent.socket_cookie, "the receipt is bound to this cookie");
    assert_eq!(key, intent.service_key, "the receipt is bound to this ServiceKey");
    assert_eq!(
        rig.dataplane.selected_backend_identity(backend_id).expect("applied identity"),
        alloc(1),
        "the selected id denotes the programmed allocation",
    );

    rig.dataplane.cleanup(&intent).expect("cleanup");
    assert_eq!(live_counts(&rig), (0, 0), "no intent and no receipt remain");
    assert_eq!(routing_snapshot(), routing_before, "the test installed no rule, route or table");
}

/// S-PIG-56 (Tier-3 half). The arm ignores `BACKEND_MAP.healthy` (R-2): a
/// Service whose only backend is marked unhealthy and listens nowhere is still
/// `Selected`, and the dial fails: a failed dial, never a wrong-peer success.
/// (The reset at the shared nft output rule for a stopped backend whose tuple
/// is gone is the native lane's.)
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER consuming-port-unavailable-outcome"]
fn a_stale_service_entry_with_a_dead_backend_selects_it_and_the_dial_fails() {
    let rig = HostRig::new("h56");
    let rt = runtime();
    let routing_before = routing_snapshot();

    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(56, 1, false)]))
        .expect("the Service is programmed with an unhealthy backend nobody listens for");

    let prepared = rig.dataplane.open_upstream(service_key(1)).expect("open_upstream");
    let (socket, intent) = prepared.into_parts();
    let dialled = rt.block_on(dial(socket, frontend_target(1), Duration::from_secs(5)));
    assert!(dialled.is_none(), "the dial to a dead backend must fail, it connected");

    let receipt = rig.dataplane.take_receipt(&intent).expect("the receipt of this socket");
    assert!(
        matches!(receipt, GatewaySelectionReceipt::Selected { .. }),
        "the arm ignores `healthy`, so the stale entry is still Selected, got {receipt:?}",
    );
    rig.dataplane.cleanup(&intent).expect("cleanup");
    assert_eq!(live_counts(&rig), (0, 0));
    assert_eq!(routing_snapshot(), routing_before, "the test installed no rule, route or table");
}

/// S-PIG-57 (Tier-3 half). Against the real hook the sentinel probe passes and
/// leaves no residue; a probe whose deadline has already elapsed is the typed
/// `ProbeTimeout` and also leaves none. (The boot refusal with reason
/// `gateway.connect_probe` is the control-plane integration binary's.)
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
fn the_sentinel_probe_passes_on_the_real_hook_and_an_elapsed_deadline_times_out() {
    let rig = HostRig::new("h57");
    let rt = runtime();
    let clock = WallClock;

    rt.block_on(GatewayConnectDataplane::probe(
        &rig.dataplane,
        Instant::now() + Duration::from_secs(5),
        &clock,
    ))
    .expect("the sentinel connect is denied and its NoBackend receipt consumed");
    assert_eq!(live_counts(&rig), (0, 0), "a passing probe leaves no residue");

    let elapsed = Instant::now();
    std::thread::sleep(Duration::from_millis(20));
    let outcome = rt.block_on(GatewayConnectDataplane::probe(&rig.dataplane, elapsed, &clock));
    assert_eq!(outcome, Err(GatewayConnectDataplaneError::ProbeTimeout));
    assert_eq!(live_counts(&rig), (0, 0), "a timed-out probe leaves no residue");
}

/// S-PIG-71 (instance half). After the dataplane instance is re-created on the
/// same pin directory (a serve restart), no old receipt, intent or applied
/// identity survives; a registered socket is `NoBackend` until the ordinary
/// update path re-programs the Service, then `Selected` again with the
/// allocation's identity.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER selection-receipt-and-applied-identity"]
fn after_an_instance_restart_old_identities_and_receipts_are_gone_until_reprogrammed() {
    let rig = HostRig::new("h71");
    let rt = runtime();

    let listener = TcpListener::bind(backend_target(71)).expect("bind the backend address");
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(71, 1, true)]))
        .expect("programmed");
    let (socket, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    drop(rt.block_on(dial(socket, frontend_target(1), Duration::from_secs(5))));
    let old_id = match rig.dataplane.take_receipt(&intent).expect("receipt") {
        GatewaySelectionReceipt::Selected { backend_id, .. } => backend_id,
        other @ GatewaySelectionReceipt::NoBackend { .. } => {
            panic!("a programmed Service is Selected, got {other:?}")
        }
    };
    assert_eq!(rig.dataplane.selected_backend_identity(old_id), Ok(alloc(1)));
    // A second registered socket whose receipt is still pending at the restart.
    let (pending_socket, pending) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    drop(rt.block_on(dial(pending_socket, frontend_target(1), Duration::from_secs(5))));
    assert_eq!(rig.dataplane.live_receipts().map(|receipts| receipts.len()), Ok(1));
    drop(listener);

    let rig = rig.recreate();

    assert_eq!(
        rig.dataplane.selected_backend_identity(old_id),
        Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id: old_id }),
        "no applied identity survives the restart",
    );
    assert_eq!(rig.dataplane.live_intent_count(), Ok(0), "no intent survives the restart");
    assert_eq!(
        rig.dataplane.live_receipts().map(|receipts| receipts.len()),
        Ok(0),
        "no receipt survives the restart"
    );
    assert!(
        rig.dataplane.take_receipt(&pending).is_err(),
        "the receipt pending at the restart is gone",
    );

    // Before re-hydration the Service has no outer entry: NoBackend.
    let (socket, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    drop(rt.block_on(dial(socket, frontend_target(1), Duration::from_secs(5))));
    assert!(
        matches!(
            rig.dataplane.take_receipt(&intent).expect("receipt"),
            GatewaySelectionReceipt::NoBackend { .. }
        ),
        "an unprogrammed Service is NoBackend",
    );
    rig.dataplane.cleanup(&intent).expect("cleanup");

    // The ordinary update path (the hydrator's) re-programs it.
    let listener = TcpListener::bind(backend_target(71)).expect("rebind the backend address");
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(71, 1, true)]))
        .expect("re-programmed");
    let (socket, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    drop(rt.block_on(dial(socket, frontend_target(1), Duration::from_secs(5))));
    match rig.dataplane.take_receipt(&intent).expect("receipt") {
        GatewaySelectionReceipt::Selected { backend_id, .. } => {
            assert_eq!(rig.dataplane.selected_backend_identity(backend_id), Ok(alloc(1)));
        }
        other @ GatewaySelectionReceipt::NoBackend { .. } => {
            panic!("a re-programmed Service is Selected, got {other:?}")
        }
    }
    rig.dataplane.cleanup(&intent).expect("cleanup");
    drop(listener);
    rt.block_on(rig.dataplane.update_service(frontend(1), Vec::new())).expect("emptied");
}

/// S-PIG-72. Updates racing `selected_backend_identity` never return a missing
/// identity for an id a `Selected` receipt named: 2 000 rounds, each racing one
/// production `update_service` of a fresh backend against registered dials whose
/// `Selected` ids are read by four concurrent readers. Bounded at 60 s; the seed
/// of the interleaving jitter and the rounds run are printed.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER selection-receipt-and-applied-identity"]
fn updates_racing_identity_reads_never_return_a_missing_identity_for_a_selected_id() {
    const ROUNDS: u16 = 2_000;
    const SEED: u64 = 0x9e37_79b9_7f4a_7c15;
    const BOUND: Duration = Duration::from_secs(60);

    let rig = HostRig::new("h72");
    let started = Instant::now();
    let selected_seen = AtomicU32::new(0);
    let violations = AtomicU32::new(0);
    let mut jitter = SEED;
    let mut rounds_run: u16 = 0;
    eprintln!("S-PIG-72 seed={SEED:#x} rounds={ROUNDS}");

    for round in 1..=ROUNDS {
        if started.elapsed() > BOUND {
            break;
        }
        jitter =
            jitter.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let spin = (jitter >> 60) as u32;
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..spin {
                    std::thread::yield_now();
                }
                runtime()
                    .block_on(
                        rig.dataplane.update_service(frontend(1), vec![backend(round, 1, true)]),
                    )
                    .expect("update_service");
                done.store(true, Ordering::Release);
            });
            let rt = runtime();
            while !done.load(Ordering::Acquire) {
                let (socket, intent) = rig
                    .dataplane
                    .open_upstream(service_key(1))
                    .expect("open_upstream")
                    .into_parts();
                drop(rt.block_on(dial(socket, frontend_target(1), Duration::from_millis(200))));
                if let Ok(GatewaySelectionReceipt::Selected { backend_id, .. }) =
                    rig.dataplane.take_receipt(&intent)
                {
                    selected_seen.fetch_add(1, Ordering::Relaxed);
                    std::thread::scope(|readers| {
                        for _ in 0..4 {
                            readers.spawn(|| {
                                if rig.dataplane.selected_backend_identity(backend_id).is_err() {
                                    violations.fetch_add(1, Ordering::Relaxed);
                                }
                            });
                        }
                    });
                }
                rig.dataplane.cleanup(&intent).expect("cleanup");
            }
        });
        rounds_run = round;
    }

    eprintln!(
        "S-PIG-72 rounds_run={rounds_run} selected={}",
        selected_seen.load(Ordering::Relaxed)
    );
    assert_eq!(rounds_run, ROUNDS, "all rounds complete within {BOUND:?}");
    assert!(
        selected_seen.load(Ordering::Relaxed) > 0,
        "the race must have produced at least one Selected receipt",
    );
    assert_eq!(
        violations.load(Ordering::Relaxed),
        0,
        "a Selected id's identity read returned a missing identity",
    );

    runtime().block_on(rig.dataplane.update_service(frontend(1), Vec::new())).expect("emptied");
    assert_eq!(rig.dataplane.live_intent_count(), Ok(0));
    assert_eq!(rig.dataplane.live_receipts().map(|receipts| receipts.len()), Ok(0));
}

/// S-PIG-124. The hook's rewrite as the kernel shows it. A registered socket's
/// `connect(2)` to the Service frontend `198.18.250.1:8080` (port bytes
/// `0x1F90`) completes with `getpeername` equal to the programmed backend
/// `127.1.x.y:9000` (`0x2328`): the port is written low-16 network order (a
/// host-order or high-half write lands elsewhere and the connect is refused).
/// The intent is registered for the kernel's own `SO_COOKIE` of the socket.
/// Receipt, residue and routing oracles are S-PIG-63's. This test reads the
/// destination the kernel reports after the rewrite; it does not read
/// `user_ip4`/`user_port` inside the program.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-selection-hook-rewrite"]
fn a_registered_connect_lands_on_the_programmed_backend_with_the_port_in_network_order() {
    let rig = HostRig::new("h124");
    let rt = runtime();

    let target = backend_target(124);
    let _backend_listener = TcpListener::bind(target).expect("bind the backend address");
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(124, 1, true)]))
        .expect("the Service is programmed through the production update path");

    let (socket, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    assert_eq!(
        intent.socket_cookie.get().get(),
        socket_option::<u64>(socket.as_raw_fd(), libc::SO_COOKIE),
        "the intent is registered for the kernel's cookie of this socket",
    );

    let stream = rt
        .block_on(dial(socket, frontend_target(1), Duration::from_secs(5)))
        .expect("the rewritten dial completes");
    assert_eq!(
        stream.peer_addr().expect("getpeername"),
        SocketAddr::V4(target),
        "the kernel shows the programmed backend, port 9000 in network order",
    );
    drop(stream);

    rig.dataplane.cleanup(&intent).expect("cleanup");
    assert_eq!(live_counts(&rig), (0, 0), "no intent and no receipt remain");
}

/// S-PIG-125 (b). A socket whose intent was removed by `cleanup` is a
/// non-registered cookie and is not selected. Positive control: a second
/// registered socket to the same programmed Service lands on the backend. The
/// unregistered dial does not connect, is not denied by the hook (not
/// `PermissionDenied`), and creates no receipt.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER gateway-selection-hook-unselected-sockets"]
fn a_socket_whose_intent_was_cleaned_up_is_not_selected_and_leaves_no_receipt() {
    let rig = HostRig::new("h125b");
    let rt = runtime();

    let _backend_listener = TcpListener::bind(backend_target(125)).expect("bind the backend");
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(125, 1, true)]))
        .expect("the Service is programmed");

    // Positive control: a registered socket lands on the programmed backend.
    let (control, control_intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    let stream = rt
        .block_on(dial(control, frontend_target(1), Duration::from_secs(5)))
        .expect("a registered socket to the programmed Service reaches the backend");
    assert_eq!(stream.peer_addr().expect("peer"), SocketAddr::V4(backend_target(125)));
    drop(stream);
    rig.dataplane.take_receipt(&control_intent).expect("the control's receipt");
    rig.dataplane.cleanup(&control_intent).expect("cleanup");

    // The unregistered cookie.
    let (socket, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    rig.dataplane.cleanup(&intent).expect("cleanup before the dial");
    assert_eq!(live_counts(&rig), (0, 0), "the intent is gone before the dial");

    let outcome = rt.block_on(async {
        tokio::time::timeout(
            Duration::from_millis(500),
            socket.connect(SocketAddr::V4(frontend_target(1))),
        )
        .await
    });
    match outcome {
        Ok(Ok(_)) => panic!("a non-registered cookie is not rewritten, the dial must not connect"),
        Ok(Err(error)) => assert_ne!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied,
            "the kernel's own failure, not a hook denial: {error}",
        ),
        Err(_elapsed) => {}
    }
    assert_eq!(live_counts(&rig), (0, 0), "no receipt was created");
    assert!(rig.dataplane.take_receipt(&intent).is_err(), "there is no receipt to take");
}

/// S-PIG-125 (c). While a registered intent is outstanding, a plain unregistered
/// socket dialing the programmed frontend is not rewritten (it does not connect)
/// and creates no receipt: the outstanding intent is untouched.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER gateway-selection-hook-unselected-sockets"]
fn a_plain_socket_dialing_the_frontend_is_not_rewritten_while_an_intent_is_outstanding() {
    let rig = HostRig::new("h125c");
    let rt = runtime();

    let _backend_listener = TcpListener::bind(backend_target(1250)).expect("bind the backend");
    rt.block_on(rig.dataplane.update_service(frontend(1), vec![backend(1250, 1, true)]))
        .expect("the Service is programmed");

    let (_registered, intent) =
        rig.dataplane.open_upstream(service_key(1)).expect("open_upstream").into_parts();
    assert_eq!(live_counts(&rig), (1, 0), "one registered intent is outstanding");

    let plain = tokio::net::TcpSocket::new_v4().expect("a plain socket");
    let outcome = rt.block_on(async {
        tokio::time::timeout(
            Duration::from_millis(500),
            plain.connect(SocketAddr::V4(frontend_target(1))),
        )
        .await
    });
    assert!(
        !matches!(outcome, Ok(Ok(_))),
        "an unregistered socket is not rewritten to the backend, it must not connect",
    );
    assert_eq!(live_counts(&rig), (1, 0), "the plain socket created no receipt");

    rig.dataplane.cleanup(&intent).expect("cleanup");
    assert_eq!(live_counts(&rig), (0, 0));
}
