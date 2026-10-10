//! HALFCLOSE-SIGNAL, guest side (§ *Flow end*; P2-S12 guest half; the guest
//! side of `owner_flows` `flows_half_guest`): the flow vsock is shut for
//! writing only after `HalfCloseAck`; a peer `HalfClose` is acked at once and
//! the outer leg shut after the vsock EOF; an unannounced vsock end aborts and
//! resets the application (never EOF); a drain that does not complete within
//! `FLOW_DRAIN_DEADLINE` aborts; a clean close clears linger before closing.

use overdrive_core::guest_transport::wire::FlowId;
use overdrive_core::guest_transport::wire::{ControlMessage, FLOW_DRAIN_DEADLINE};

use super::super::sim::{FarEndEvent, SimAppEnd, SimAppTarget, SimGuestEffect};
use super::super::{FlowEndObservation, GuestFlowDirection, HalfCloseAnnounce};
use super::{Guest, host_flow, position};

fn routed(seq: u32) -> (Guest, FlowId) {
    let mut g = Guest::new();
    g.open_session();
    let f = host_flow(seq);
    g.accept_flow(f, 8080);
    let _ = g.io.take_outbox();
    g.io.clear_effects();
    (g, f)
}

const OPEN: FlowEndObservation = FlowEndObservation {
    outer_eof: false,
    announce: HalfCloseAnnounce::None,
    peer_announced: false,
    outer_shut: false,
};

/// Outer leg first: `POLLRDHUP` on C → drained toward the vsock → `HalfClose`
/// (Pending) → only on `HalfCloseAck` `shutdown_write` on V_g (Done).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn the_flow_vsock_is_shut_for_writing_only_after_half_close_ack() {
    let (mut g, f) = routed(1);
    assert_eq!(g.obs().flows[&f], OPEN);
    g.io.app_end(SimAppTarget::Flow(f), SimAppEnd::HalfClose);
    g.run_released();
    assert_eq!(
        g.obs().flows[&f],
        FlowEndObservation { outer_eof: true, announce: HalfCloseAnnounce::Pending, ..OPEN }
    );
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::HalfClose { flow: f }]);
    let e = g.io.effects();
    assert!(
        position(&e, |x| matches!(
            x,
            SimGuestEffect::Drained { direction: GuestFlowDirection::TowardHost, .. }
        ))
        .is_some()
    );
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::ShutdownWrite { vsock: true, .. })).is_none()
    );
    assert!(!g.io.guest_socket_observation().vsock_ended.contains(&f));

    g.io.host_control(ControlMessage::HalfCloseAck { flow: f });
    g.run_released();
    assert_eq!(g.obs().flows[&f].announce, HalfCloseAnnounce::Done);
    let sim = g.io.guest_socket_observation();
    assert!(sim.vsock_ended.contains(&f) && sim.vsock_ends_in_flight.contains(&f));
    assert!(
        !sim.far_end.contains_key(&f),
        "the application leg is untouched by its own half-close"
    );
}

/// Flow vsock: a peer `HalfClose` is answered with `HalfCloseAck` at once;
/// the vsock EOF of the announced direction is drained toward C, then C is
/// shut for writing — the application reads EOF.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_peer_half_close_is_acked_and_the_outer_leg_shut_after_the_vsock_eof() {
    let (mut g, f) = routed(2);
    g.io.host_control(ControlMessage::HalfClose { flow: f });
    g.run_released();
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::HalfCloseAck { flow: f }]);
    assert!(g.obs().flows[&f].peer_announced);
    g.io.host_vsock_end(f);
    g.run_released();
    assert!(g.obs().flows[&f].outer_shut);
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Eof));
}

/// An unannounced vsock end is the peer owner's loss: abort at once — the
/// application is reset before any FIN, whenever the session loss arrives.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn an_unannounced_vsock_end_aborts_and_resets_the_application() {
    let (mut g, f) = routed(3);
    g.io.host_vsock_end(f);
    g.run_released();
    assert!(!g.obs().flows.contains_key(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    let e = g.io.effects();
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::ShutdownWrite { vsock: false, .. })).is_none(),
        "never a FIN"
    );
}

/// Outer leg first, but the direction never drains: at `FLOW_DRAIN_DEADLINE`
/// the flow is aborted (`DrainTimeout`), the application reset, `Abort` sent.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_drain_that_misses_the_deadline_aborts_the_flow() {
    let (mut g, f) = routed(4);
    g.io.set_drained(f, GuestFlowDirection::TowardHost, false);
    g.io.app_end(SimAppTarget::Flow(f), SimAppEnd::HalfClose);
    g.run_released();
    assert!(g.io.take_outbox().is_empty(), "no HalfClose before the drain");
    g.advance(FLOW_DRAIN_DEADLINE);
    assert!(!g.obs().flows.contains_key(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: f }]);
}

/// Clean close: both directions half-closed through the handshake and drained
/// → `remove_pair`, `clear_linger` on the outer socket, then close — the
/// application sees EOF, never a reset.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_clean_close_clears_linger_before_closing() {
    let (mut g, f) = routed(5);
    g.io.app_end(SimAppTarget::Flow(f), SimAppEnd::HalfClose);
    g.run_released();
    g.io.host_control(ControlMessage::HalfCloseAck { flow: f });
    g.run_released();
    g.io.host_control(ControlMessage::HalfClose { flow: f });
    g.run_released();
    g.io.host_vsock_end(f);
    g.run_released();
    assert!(!g.obs().flows.contains_key(&f), "closed");
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Eof));
    let e = g.io.effects();
    let removed =
        position(&e, |x| matches!(x, SimGuestEffect::RemovePair { .. })).expect("pair removed");
    let cleared =
        position(&e, |x| matches!(x, SimGuestEffect::ClearLinger { .. })).expect("linger cleared");
    let closed = position(&e, |x| matches!(x, SimGuestEffect::Close { .. })).expect("closed");
    assert!(removed < closed && cleared < closed, "{e:?}");
    assert!(position(&e, |x| matches!(x, SimGuestEffect::Abort { .. })).is_none());
}
