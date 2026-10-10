//! The guest's control session (D18a) and the owner's failure classes (DG-5,
//! H-4): session open with the guest's own token, ListenState full state then
//! changes (D23), session loss never fatal with reconnect at
//! `CONTROL_RECONNECT_BACKOFF`, the host-owner-restart boundary scenario, and
//! `GuestOwnerFatal` only for `EventWait` / `ListenerFailed`.

use std::collections::BTreeSet;
use std::io::{self, ErrorKind};

use overdrive_core::guest_transport::GUEST_UDP_SLOTS;
use overdrive_core::guest_transport::wire::{
    CONTROL_RECONNECT_BACKOFF, ControlMessage, ControlOpen,
};

use super::super::sim::{FarEndEvent, SimGuestEffect, SimGuestOp, SimSessionEnd};
use super::super::{GuestListener, GuestOwnerFatal, HostChannel};
use super::{Guest, dest, host_flow, launch, listen_port};

/// Plain data (real now): the two fatal variants render their pinned texts.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guest_owner_fatal_renders_its_pinned_display_texts() {
    let wait = GuestOwnerFatal::EventWait { source: io::Error::other("boom") };
    assert_eq!(wait.to_string(), "waiting for guest transport events failed: boom");
    let listener = GuestOwnerFatal::ListenerFailed {
        listener: GuestListener::Inbound,
        kind: ErrorKind::OutOfMemory,
    };
    assert_eq!(listener.to_string(), "guest listener Inbound failed: OutOfMemory");
    let intake = GuestOwnerFatal::ListenerFailed {
        listener: GuestListener::TcpIntake,
        kind: ErrorKind::Other,
    };
    assert_eq!(intake.to_string(), "guest listener TcpIntake failed: Other");
}

/// QPROJ-5 edge case: before the session, `session = false`, every table is
/// empty except `slots`, which lists every slot unowned at generation 0. The
/// first dispatch starts the control connect; on `Connected` the guest's first
/// write is the `ControlOpen` with its own launch token (ATTR-TOKEN).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn session_opens_with_a_control_open_carrying_the_guests_own_token() {
    let mut g = Guest::new();
    let before = g.obs();
    assert!(!before.session);
    assert!(
        before.accepted.is_empty() && before.flows.is_empty() && before.awaiting_paired.is_empty()
    );
    assert_eq!(before.slots.len(), usize::from(GUEST_UDP_SLOTS));
    assert!(
        before.slots.values().all(|s| s.owner.is_none() && s.association.is_none() && !s.routed)
    );

    g.step();
    assert_eq!(
        g.connects(HostChannel::Control),
        1,
        "the first dispatch starts the control connect"
    );
    assert!(!g.obs().session, "not live before Connected");

    g.io.complete_host_connect(HostChannel::Control, Ok(()));
    g.run_released();
    let after = g.obs();
    assert!(after.session);
    assert_eq!(g.io.control_opens(), vec![ControlOpen { launch: launch() }.encode()]);
    let effects = g.io.effects();
    let open = super::position(&effects, |e| matches!(e, SimGuestEffect::WriteControlOpen { .. }));
    let first_write = super::position(&effects, |e| {
        matches!(e, SimGuestEffect::WriteControlOpen { .. } | SimGuestEffect::WriteControl { .. })
    });
    assert_eq!(open, first_write, "the ControlOpen is the first write on the session");
    assert!(g.io.take_outbox().is_empty(), "empty listener map: no ListenState");
    assert_eq!(after.slots, before.slots, "opening the session touches no slot");
    // Repeated observation with no owner step is equal.
    assert_eq!(g.obs(), after);
}

/// D23: a session opens with every port not listening; the guest then sends
/// `ListenState{true}` for every port in its listener map, and one message per
/// change on each wake.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn listen_state_reports_the_full_map_then_each_change() {
    let mut g = Guest::new();
    g.io.set_listener_map(BTreeSet::from([listen_port(8080), listen_port(9090)]));
    g.open_session();
    super::assert_same_messages(
        &g.io.take_outbox(),
        &[
            ControlMessage::ListenState { port: listen_port(8080), listening: true },
            ControlMessage::ListenState { port: listen_port(9090), listening: true },
        ],
    );

    g.io.set_listener_map(BTreeSet::from([listen_port(8080), listen_port(7070)]));
    g.io.listener_map_wake();
    g.run_released();
    // Only the changes, never the unchanged 8080.
    super::assert_same_messages(
        &g.io.take_outbox(),
        &[
            ControlMessage::ListenState { port: listen_port(9090), listening: false },
            ControlMessage::ListenState { port: listen_port(7070), listening: true },
        ],
    );
}

/// DG-5 boundary scenario: the host owner restarts (`serve` killed and started)
/// → the guest stays up, every flow ends with reset (HALFCLOSE-SIGNAL: never a
/// FIN), the session is re-opened `CONTROL_RECONNECT_BACKOFF` after the loss,
/// retried at that backoff while the host is down, and re-established within
/// one backoff after the host listens again; no `GuestOwnerFatal` at any step.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn host_owner_restart_resets_flows_and_reopens_the_session_without_fatal() {
    let mut g = Guest::new();
    g.open_session();
    let accepted = host_flow(1);
    g.accept_flow(accepted, 8080);
    let opened = g.connect_flow(super::super::sim::sim_cookie(77), dest(1));
    let _ = g.io.take_outbox();
    let before = g.obs();
    assert!(before.flows.contains_key(&accepted) && before.flows.contains_key(&opened));

    // The host owner dies: every flow vsock ends unannounced, then the session resets.
    g.io.host_owner_dies();
    g.run_released(); // every step `expect`s no fatal
    let lost = g.obs();
    assert!(!lost.session);
    assert!(lost.flows.is_empty() && lost.accepted.is_empty() && lost.awaiting_paired.is_empty());
    let sim = g.io.guest_socket_observation();
    assert_eq!(
        sim.far_end.get(&accepted),
        Some(&FarEndEvent::Reset),
        "the application is reset, never EOF"
    );
    assert_eq!(sim.far_end.get(&opened), Some(&FarEndEvent::Reset));
    assert!(sim.session_ended);
    assert_eq!(g.connects(HostChannel::Control), 0, "no reconnect before the backoff");

    // Not yet: just short of the backoff.
    g.advance(
        CONTROL_RECONNECT_BACKOFF
            .checked_sub(std::time::Duration::from_millis(1))
            .expect("backoff exceeds 1 ms"),
    );
    assert_eq!(g.connects(HostChannel::Control), 0);
    // One backoff after the loss: a new connect; the host is still down.
    g.advance(std::time::Duration::from_millis(1));
    assert_eq!(g.connects(HostChannel::Control), 1);
    g.io.complete_host_connect(HostChannel::Control, Err(ErrorKind::ConnectionRefused));
    g.run_released();
    assert!(!g.obs().session);

    // The host listens again; the next attempt comes within one backoff.
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 1, "retried at the backoff");
    g.io.complete_host_connect(HostChannel::Control, Ok(()));
    g.run_released();
    assert!(g.obs().session, "re-established");
    assert_eq!(g.io.control_opens().len(), 2, "a new ControlOpen on the new session");
    assert!(g.obs().flows.is_empty(), "no flow is resurrected on the new session");
}

/// DG-5 H-4: every control-session loss shape — EOF or error on the session
/// read, a failed session write, a `Hangup` on the session socket, a refused
/// or synchronously failing control connect — is not fatal: the owner ends the
/// session and re-opens it `CONTROL_RECONNECT_BACKOFF` later.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn every_session_loss_shape_is_recovered_not_fatal() {
    type Loss = fn(&mut Guest);
    let losses: Vec<(&str, Loss)> = vec![
        ("read EOF", |g| g.io.end_session(SimSessionEnd::Eof)),
        ("read reset", |g| g.io.end_session(SimSessionEnd::Reset)),
        ("write fails", |g| {
            g.io.fail_next(SimGuestOp::WriteHost, ErrorKind::BrokenPipe);
            g.io.set_listener_map(BTreeSet::from([listen_port(8080)]));
            g.io.listener_map_wake();
        }),
    ];
    for (name, loss) in losses {
        let mut g = Guest::new();
        g.open_session();
        loss(&mut g);
        g.run_released();
        assert!(!g.obs().session, "{name}: session ended");
        g.advance(CONTROL_RECONNECT_BACKOFF);
        assert_eq!(g.connects(HostChannel::Control), 1, "{name}: reconnect at the backoff");
    }

    // Refused control connect, then a synchronous connect error: retried at the backoff.
    let mut g = Guest::new();
    g.step();
    g.io.complete_host_connect(HostChannel::Control, Err(ErrorKind::ConnectionRefused));
    g.run_released();
    assert!(!g.obs().session);
    g.io.fail_next(SimGuestOp::StartConnectHost, ErrorKind::NetworkUnreachable);
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 0, "the synchronous failure started nothing");
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 1, "retried again one backoff later");
    let restarts = g
        .io
        .effects()
        .iter()
        .filter(|e| matches!(e, SimGuestEffect::ConnectHost { channel: HostChannel::Control, .. }))
        .count();
    assert_eq!(
        restarts, 2,
        "the refused attempt and the successful start (the failed start issued nothing)"
    );
}

/// DG-5: only a failed readiness wait and a failed setup listener are fatal.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn only_event_wait_and_listener_failures_are_fatal() {
    let mut g = Guest::new();
    g.open_session();
    g.io.fail_event_wait(ErrorKind::Interrupted);
    match g.owner.dispatch_next() {
        Err(GuestOwnerFatal::EventWait { source }) => {
            assert_eq!(source.kind(), ErrorKind::Interrupted);
        }
        other => panic!("expected EventWait, got {other:?}"),
    }

    for listener in [GuestListener::Inbound, GuestListener::TcpIntake] {
        let mut g = Guest::new();
        g.open_session();
        g.io.listener_failed(listener, ErrorKind::OutOfMemory);
        match g.owner.dispatch_next() {
            Err(GuestOwnerFatal::ListenerFailed { listener: l, kind }) => {
                assert_eq!((l, kind), (listener, ErrorKind::OutOfMemory));
            }
            other => panic!("expected ListenerFailed, got {other:?}"),
        }
    }
}
