//! The guest's control session (feature-delta § *Control session*, DG-5
//! failure classes, DG-5-A, DG-5-F, D-UP-8, K-A5) and the closed fatal set of
//! the guest owner.
//!
//! K-A5 (findings-ka5.md, WORKS on 7.0.0-29): the guest owner detects the
//! host's end of the session by EOF / `EPOLLRDHUP`; a reset close is
//! indistinguishable from a graceful one, so no test here asserts *how* the
//! host closed — only that the guest observed the loss and took the loss path.

use std::io::ErrorKind;

use overdrive_core::guest_transport::forwarder::ForwarderInventory;
use overdrive_core::guest_transport::wire::{
    CONTROL_RECONNECT_BACKOFF, ControlMessage, ControlOpen, FlowRefusal,
};
use overdrive_core::guest_transport::{GUEST_OWNER_AUDIT_PERIOD, GUEST_UDP_SLOTS};

use super::super::sim::{FarEndEvent, SimGuestEffect, SimGuestOp, SimSessionEnd};
use super::super::{GuestListener, GuestOwnerFatal, HostChannel};
use super::*;

/// Plain data (real now): the three fatal variants render their pinned texts.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guest_owner_fatal_renders_its_pinned_display_texts() {
    let wait = GuestOwnerFatal::EventWait { source: std::io::Error::other("boom") };
    assert_eq!(wait.to_string(), "waiting for guest transport events failed: boom");
    let listener = GuestOwnerFatal::ListenerFailed {
        listener: GuestListener::Inbound,
        kind: ErrorKind::Other,
    };
    assert_eq!(listener.to_string(), "guest listener Inbound failed: Other");
    let forwarder = GuestOwnerFatal::Forwarder {
        source: overdrive_core::guest_transport::forwarder::ForwarderError::AbiMismatch {
            expected: 1,
            found: 2,
        },
    };
    assert_eq!(
        forwarder.to_string(),
        "the guest forwarder failed: forwarder ABI 2 is not the expected 1"
    );
}

/// QPROJ-5 edge cases and the first dispatch (D-UP-7): before the session,
/// `session = false`, every table is empty except `slots`, which lists every
/// slot unowned, and `forwarder_attach_ended = 0`; `next_deadline()` is
/// `Some(now)` after `new`. The first dispatch starts the control connect; on
/// `Connected` the guest's first write is the `ControlOpen` carrying its own
/// launch token (ATTR-TOKEN). A repeated observation with no owner step is
/// equal.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn session_opens_with_a_control_open_carrying_the_guests_own_token() {
    let mut g = Guest::new();
    assert_eq!(g.owner.next_deadline(), Some(g.now()), "new arms the session-open deadline at now");
    let before = g.obs();
    assert!(!before.session);
    assert!(
        before.accepted.is_empty() && before.flows.is_empty() && before.awaiting_paired.is_empty()
    );
    assert_eq!(before.slots.len(), usize::from(GUEST_UDP_SLOTS));
    assert!(
        before
            .slots
            .values()
            .all(|s| s.owner.is_none() && s.association.is_none() && s.attached.is_none())
    );
    assert_eq!(before.forwarder_attach_ended, 0);
    assert_eq!(g.obs(), before, "a repeated observation with no owner step is equal");

    g.start();
    assert_eq!(
        g.connects(HostChannel::Control),
        1,
        "the first dispatch starts the control connect"
    );
    assert!(
        g.obs().session,
        "the session is held from the dispatch that starts its connect (D-UP-8)"
    );
    g.io.complete_host_connect(HostChannel::Control, Ok(()));
    g.settle();
    let opens = g.io.control_opens();
    assert_eq!(opens.len(), 1, "exactly one ControlOpen");
    assert_eq!(ControlOpen::decode(&opens[0]), Ok(ControlOpen { launch: launch() }));
    let effects = g.io.effects();
    let open = position(&effects, |e| matches!(e, SimGuestEffect::WriteControlOpen { .. }))
        .expect("the ControlOpen is written");
    let first_read = position(&effects, |e| matches!(e, SimGuestEffect::ReadHost { .. }))
        .expect("the session is read");
    assert!(open < first_read, "the ControlOpen is the first write, before the first read");
    assert!(g.obs().session);
    assert_eq!(g.io.forwarder_inventory(), Some(ForwarderInventory::default()), "nothing attached");
}

/// DG-5-A: the audit tick is periodic at `GUEST_OWNER_AUDIT_PERIOD`, armed by
/// the first dispatch, fixed rate: each next tick at the previous tick's
/// deadline plus the period; a tick handled late arms the next from its own
/// deadline, so ticks do not accumulate. `next_deadline` is a pure read.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_audit_tick_is_fixed_rate_from_the_first_dispatch() {
    let mut g = Guest::new();
    let t0 = g.now();
    g.open_session();
    assert_eq!(g.owner.next_deadline(), Some(t0 + GUEST_OWNER_AUDIT_PERIOD));
    assert_eq!(g.owner.next_deadline(), g.owner.next_deadline(), "a pure read");
    g.advance_to(t0 + GUEST_OWNER_AUDIT_PERIOD);
    assert_eq!(g.owner.next_deadline(), Some(t0 + GUEST_OWNER_AUDIT_PERIOD * 2));
    // a tick handled 3.5 periods late arms the next from its own deadline
    g.clock.advance(GUEST_OWNER_AUDIT_PERIOD * 7 / 2);
    g.step();
    assert_eq!(g.owner.next_deadline(), Some(t0 + GUEST_OWNER_AUDIT_PERIOD * 3));
    g.settle();
    assert!(
        g.owner.next_deadline().is_some_and(|d| d > g.now()),
        "the overdue ticks are dispatched one per deadline, then none is due"
    );
}

/// DG-5-F: the session is lost (EOF, K-A5) while the guest holds a paired
/// TcpAccept flow, an accepted one whose C is connecting, a paired TcpConnect
/// flow, an unpaired TcpConnect flow and an attached datagram association. The
/// one dispatch that observes the loss aborts every pair (`abort_forward`),
/// aborts every unattached socket and ends every association, and no control
/// connect precedes those aborts; every table is empty after it and every
/// application far end reads a reset, never EOF. The reconnect is started
/// `CONTROL_RECONNECT_BACKOFF` after the loss.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn session_loss_aborts_every_flow_in_the_observing_dispatch_before_any_reconnect() {
    let mut g = Guest::new();
    g.open_session();
    let (h1, h2) = (host_flow(1), host_flow(2));
    let p1 = g.accept_flow(h1, 8080);
    g.accept_until_connecting(h2, 8081);
    let f1 = g.connect_flow(cookie(7), dest(1));
    let p2 = g.pair(f1);
    let f2 = g.capture_flow(cookie(8), dest(2));
    let s = g.io.app_send(cookie(9), dest(3)).expect("a free slot");
    let a = g.associate(s);
    g.pair_slot(a);
    let p3 = g.obs().slots[&s].attached.expect("the association is attached");
    g.io.clear_effects();

    let lost_at = g.now();
    g.io.end_session(SimSessionEnd::Eof);
    g.step();

    let obs = g.obs();
    assert!(!obs.session, "the loss is observed");
    assert!(obs.accepted.is_empty() && obs.flows.is_empty() && obs.awaiting_paired.is_empty());
    assert!(obs.slots.values().all(|x| x.association.is_none() && x.attached.is_none()));
    assert_eq!(obs.slots[&s].owner, Some((cookie(9), dest(3))), "the slot stays owned (U-2)");
    let effects = g.io.effects();
    for pair in [p1, p2, p3] {
        assert!(
            effects.contains(&SimGuestEffect::AbortForward { id: pair }),
            "{pair:?} is aborted in the observing dispatch"
        );
    }
    assert!(effects.contains(&SimGuestEffect::EndAssociation { slot: s }));
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            SimGuestEffect::ConnectHost { channel: HostChannel::Control, .. }
        )),
        "no reconnect precedes the aborts (DG-5-F)"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, SimGuestEffect::WriteControl { .. })),
        "nothing is written on a lost session"
    );

    g.run_and_settle();
    let far = g.io.guest_socket_observation().far_end;
    for flow in [h1, h2, f1, f2] {
        assert_eq!(
            far.get(&flow),
            Some(&FarEndEvent::Reset),
            "{flow:?}'s application reads a reset"
        );
    }
    assert_eq!(g.obs(), obs, "the aborts' Detached records change nothing (the flows are ended)");

    assert_eq!(
        g.owner.next_deadline().map(|d| d <= lost_at + CONTROL_RECONNECT_BACKOFF),
        Some(true)
    );
    g.advance_to(lost_at + CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 1, "the reconnect starts at the backoff");
}

/// K-A5: the host's end of the session is observed whichever shape the guest
/// sees — EOF on the pending session read, or `EPOLLRDHUP` on the session
/// socket with no read pending — and both take the loss path. No test path
/// relies on `ECONNRESET` / `EPOLLERR` for the host's close.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_hosts_end_of_the_session_is_observed_as_eof_or_read_hangup() {
    for how in ["eof-on-read", "read-hangup"] {
        let mut g = Guest::new();
        g.open_session();
        let h = host_flow(1);
        g.accept_flow(h, 8080);
        if how == "read-hangup" {
            // a host message completes the session read, and the host closes
            // before the owner re-arms it: no read is pending at the close, so
            // the owner sees the session socket's POLLRDHUP
            g.io.host_control(ControlMessage::Abort { flow: unknown_guest_flow() });
        }
        g.io.end_session(SimSessionEnd::Eof);
        g.settle();
        assert!(!g.obs().session, "{how}: the loss is observed");
        assert!(g.obs().accepted.is_empty(), "{how}: the session's flows are aborted");
    }
}

/// DG-5 boundary scenario: the host owner restarts (`serve` killed and
/// started) → the guest stays up; every flow vsock receives `0x3`, which the
/// guest forwarder classifies as an abort with a direction open, and every
/// flow ends with a reset (never a FIN); a V_g still queued is torn down there;
/// the session is re-opened `CONTROL_RECONNECT_BACKOFF` after the loss, retried
/// at that backoff while the host is down, and re-established once the host
/// listens again; no `GuestOwnerFatal` at any step.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn host_owner_restart_resets_flows_and_reopens_the_session_without_fatal() {
    let mut g = Guest::new();
    g.open_session();
    let (h1, h2) = (host_flow(1), host_flow(2));
    g.accept_flow(h1, 8080);
    let f = g.connect_flow(cookie(7), dest(1));
    g.io.host_open_accept(accept_request(h2, 8080, launch()));

    let lost_at = g.now();
    g.io.host_owner_dies();
    g.settle();
    g.run_and_settle();

    let obs = g.obs();
    assert!(!obs.session && obs.accepted.is_empty() && obs.flows.is_empty());
    let sockets = g.io.guest_socket_observation();
    assert!(
        sockets.reset_in_queue.contains(&h2),
        "the queued V_g is torn down in the queue (K-A4)"
    );
    for flow in [h1, f] {
        assert_eq!(
            sockets.far_end.get(&flow),
            Some(&FarEndEvent::Reset),
            "{flow:?} ends with a reset"
        );
    }

    // the host is down: each reconnect is refused and retried at the backoff
    let mut at = lost_at;
    for _ in 0..3 {
        at += CONTROL_RECONNECT_BACKOFF;
        g.advance_to(at);
        assert_eq!(g.connects(HostChannel::Control), 1, "one reconnect per backoff");
        g.io.complete_host_connect(HostChannel::Control, Err(ErrorKind::ConnectionRefused));
        g.settle();
        assert!(!g.obs().session);
    }
    // the host listens again
    g.advance_to(at + CONTROL_RECONNECT_BACKOFF);
    g.io.complete_host_connect(HostChannel::Control, Ok(()));
    g.settle();
    assert!(g.obs().session);
    assert_eq!(g.io.control_opens().len(), 2, "one ControlOpen per established session");
}

/// DG-5 failure classes (H-4): every control-session loss shape — EOF or an
/// error on the session read, a failed session write, a refused or
/// unreachable control connect, a synchronous `start_connect_host` error — is
/// not fatal: the owner ends the session and re-opens it
/// `CONTROL_RECONNECT_BACKOFF` later.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn every_session_loss_shape_is_recovered_not_fatal() {
    // the session read ends: EOF, or an error
    for how in [SimSessionEnd::Eof, SimSessionEnd::Reset] {
        let mut g = Guest::new();
        g.open_session();
        g.io.end_session(how);
        g.settle();
        assert!(!g.obs().session, "{how:?}");
        g.advance(CONTROL_RECONNECT_BACKOFF);
        assert_eq!(g.connects(HostChannel::Control), 1, "{how:?}: re-opened after the backoff");
    }
    // a failed session write (the guest's Paired for a dial)
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.fail_next(SimGuestOp::WriteHost, ErrorKind::BrokenPipe);
    g.io.complete_app_connect(h, Ok(()));
    g.settle();
    assert!(!g.obs().session, "a failed session write is the session's loss");
    assert!(g.obs().accepted.is_empty(), "the session's flows are aborted");
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 1);
    // a refused / unreachable control connect, and a synchronous connect error
    for kind in [
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::TimedOut,
        ErrorKind::Other,
    ] {
        let mut g = Guest::new();
        g.start();
        g.io.complete_host_connect(HostChannel::Control, Err(kind));
        g.settle();
        assert!(!g.obs().session, "{kind:?}");
        g.advance(CONTROL_RECONNECT_BACKOFF);
        assert_eq!(g.connects(HostChannel::Control), 1, "{kind:?}: retried at the backoff");
    }
    let mut g = Guest::new();
    g.io.fail_next(SimGuestOp::StartConnectHost, ErrorKind::AddrNotAvailable);
    g.start();
    assert!(!g.obs().session);
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(g.connects(HostChannel::Control), 1, "a synchronous connect error is retried too");
}

/// D-UP-8: the guest owner holds at most one control-session connection. While
/// it holds one it never starts another; after it observes the loss it starts
/// the next only in a later dispatch, at the backoff.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_holds_one_session_and_reopens_only_after_observing_its_loss() {
    let mut g = Guest::new();
    g.open_session();
    for _ in 0..5 {
        g.advance(CONTROL_RECONNECT_BACKOFF);
    }
    let connects = |g: &Guest| {
        g.io.effects()
            .iter()
            .filter(|e| {
                matches!(e, SimGuestEffect::ConnectHost { channel: HostChannel::Control, .. })
            })
            .count()
    };
    assert_eq!(connects(&g), 1, "no second connect while the session is held");
    g.io.host_end_session_deferred(SimSessionEnd::Eof);
    g.advance(CONTROL_RECONNECT_BACKOFF * 4);
    assert!(g.obs().session, "an unobserved loss leaves the session held");
    assert_eq!(connects(&g), 1, "no reconnect before the loss is observed");
    g.io.release_session_end();
    g.step();
    assert!(!g.obs().session);
    assert_eq!(connects(&g), 1, "not in the dispatch that observes the loss");
    g.advance(CONTROL_RECONNECT_BACKOFF);
    assert_eq!(connects(&g), 2, "the next connect at the backoff");
}

/// U-5 on the session: a message naming a flow the guest does not hold
/// (`Paired`, `Refused`, `Abort`) is discarded with no reply, and the session
/// and every table stay unchanged.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_message_for_an_unknown_flow_leaves_the_session_and_every_table_unchanged() {
    let mut g = Guest::new();
    g.open_session();
    let before = g.obs();
    let unknown = unknown_guest_flow();
    for message in [
        ControlMessage::Paired { flow: unknown },
        ControlMessage::Refused { flow: unknown, refusal: FlowRefusal::Policy },
        ControlMessage::Abort { flow: unknown },
        ControlMessage::Abort { flow: host_flow(77) },
    ] {
        g.io.host_control(message);
        g.settle();
        assert_eq!(g.obs(), before, "{message:?} changes nothing");
    }
    assert!(g.io.outbox().is_empty(), "no reply");
    assert_eq!(g.io.forwarder_inventory(), Some(ForwarderInventory::default()));
}

/// DG-5: the owner's fatal set is closed — a failed readiness wait
/// (`EventWait`) and a failed setup listener (`ListenerFailed`, either
/// listener) end `run`; nothing per-flow or per-session does. (A guest
/// forwarder context `Io` is the third, `Forwarder`; the sim context never
/// fails, so it is not driven here.)
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn only_event_wait_and_listener_failures_are_fatal() {
    let mut g = Guest::new();
    g.open_session();
    g.io.fail_event_wait(ErrorKind::Interrupted);
    assert!(matches!(g.owner.dispatch_next(), Err(GuestOwnerFatal::EventWait { .. })));
    for listener in [GuestListener::Inbound, GuestListener::TcpIntake] {
        let mut g = Guest::new();
        g.open_session();
        g.io.listener_failed(listener, ErrorKind::Other);
        assert!(
            matches!(g.owner.dispatch_next(), Err(GuestOwnerFatal::ListenerFailed { listener: l, .. }) if l == listener),
            "{listener:?}"
        );
    }
    // a per-flow failure returns Ok
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.complete_app_connect(h, Err(ErrorKind::ConnectionRefused));
    assert!(g.owner.dispatch_next().is_ok(), "a refused application connect is per-flow");
}
