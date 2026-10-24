//! TcpConnect, guest side (feature-delta § *Per-kind total orders* TcpConnect,
//! § *Guest adaptation contract* intake model, EVMOD-PAIR, EVMOD-ATTACH-END,
//! DG-5-E, D20).
//!
//! The guest attaches (I, V_g) on `Paired` (I half A, V_g half B, "Sides and
//! pairs"); until then the intake child is unattached and observed only for
//! its error, and the application's early bytes and FIN wait in its own
//! receive queue.

use overdrive_core::guest_transport::GUEST_TCP_PENDING_MAX;
use overdrive_core::guest_transport::forwarder::{PairSide, SourceEnd};
use overdrive_core::guest_transport::wire::{ControlMessage, FlowRefusal};
use overdrive_sim::adapters::{SimFarEnd, SimShutdown};

use super::super::HostChannel;
use super::super::sim::{FarEndEvent, SimGuestEffect};
use super::*;

/// Steps 2 and 7: a captured `connect()` makes the owner connect V_g to the
/// host stream port and write the TcpConnect request — guest origin, the original
/// destination, the guest's own launch token — exactly once; the flow is in
/// `awaiting_paired` and nothing is attached; on `Paired` the owner attaches
/// (I, V_g) and the flow moves to `flows`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_captured_connect_writes_the_request_once_and_attaches_on_paired() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.capture_flow(cookie(7), dest(1));
    let requests = g.io.requests();
    assert_eq!(requests.len(), 1, "one request");
    assert_eq!(
        requests[0],
        overdrive_core::guest_transport::wire::FlowRequest::TcpConnect {
            flow: f,
            destination: dest(1),
            launch: launch(),
        }
    );
    assert_eq!(f.origin(), overdrive_core::guest_transport::wire::FlowOrigin::Guest);
    assert!(g.obs().awaiting_paired.contains(&f));
    assert!(!g.obs().flows.contains_key(&f));
    assert!(
        !g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::AttachOutbound { .. })),
        "nothing is attached before Paired"
    );

    g.io.host_control(ControlMessage::Paired { flow: f });
    g.settle();
    let pair = g.pair(f);
    assert_eq!(g.obs().flows.get(&f), Some(&pair));
    assert!(!g.obs().awaiting_paired.contains(&f));
    assert_eq!(g.io.vsock_side(f), Some(PairSide::B), "V_g is half B of a TcpConnect pair");
    assert!(g.io.outbox().is_empty(), "the guest replies nothing to Paired");
}

/// EVMOD-PAIR, CLOSE_WAIT accepted: the application writes and half-closes
/// before `Paired`; the bytes and the FIN wait in I's queue. On `Paired` the
/// attach accepts I in `CLOSE_WAIT`, the attach-time run carries the bytes
/// and then a `0x2` on V_g; once the host's direction ends cleanly too the
/// pair completes, and on its record the owner clears I's linger and closes
/// both halves: the application reads EOF, never a reset.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_application_write_and_fin_before_paired_attach_in_close_wait_and_complete() {
    let mut g = Guest::new();
    g.open_session();
    let child = g.io.app_connect(cookie(7), dest(1));
    let i = g.io.kernel_socket(child);
    i.peer_send(b"early");
    i.peer_shutdown(SimShutdown::Send);
    g.settle();
    g.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
    g.settle();
    let f = g.last_request_flow();
    g.io.host_control(ControlMessage::Paired { flow: f });
    g.settle();
    let pair = g.pair(f);
    g.io.run_forwarder();
    assert_eq!(g.vsock(f).peer_received(), b"early".to_vec(), "the early bytes are carried out");
    assert_eq!(
        g.vsock(f).sent_shutdown(),
        Some(SimShutdown::Send),
        "then a 0x2 after the last byte"
    );
    let state = g.io.forwarder_pair_state(pair).expect("the pair is live");
    assert_eq!(state.a_to_b.source_end, Some(SourceEnd::Clean));
    assert!(state.a_to_b.target_shut);

    g.vsock(f).peer_shutdown(SimShutdown::Send);
    g.run_and_settle();
    assert!(!g.obs().flows.contains_key(&f), "the flow leaves on its Completed record");
    let effects = g.io.effects();
    let clear = position(&effects, |e| matches!(e, SimGuestEffect::ClearLinger { .. }))
        .expect("the TCP half's linger is cleared");
    let close =
        position(&effects, |e| matches!(e, SimGuestEffect::Close { flow: Some(x), .. } if *x == f))
            .expect("the halves are closed");
    assert!(clear < close, "linger cleared before the close");
    assert_eq!(g.outer(f).peer_end(), Some(SimFarEnd::Eof), "the application reads EOF");
}

/// EVMOD-ATTACH-END, guest TcpConnect on `Paired`, I ended: the intake child
/// reset after the owner's last check and before the attach, so the attach is
/// refused `HalfEnded`; the owner closes I with linger (a reset) and V_g (the
/// host's attached pair sees `0x3` and aborts), sends no message, and counts
/// `forwarder_attach_ended`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_intake_child_reset_before_paired_is_refused_half_ended_and_the_flow_aborts() {
    let mut g = Guest::new();
    g.open_session();
    let child = g.io.app_connect(cookie(7), dest(1));
    g.settle();
    g.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
    g.settle();
    let f = g.last_request_flow();
    // the reset lands after the owner's check (the race of "Owner response to
    // an ended half at attach", row I): no seam hangup reaches the owner
    g.io.kernel_socket(child).peer_reset();
    g.io.host_control(ControlMessage::Paired { flow: f });
    g.settle();
    let obs = g.obs();
    assert!(!obs.flows.contains_key(&f) && !obs.awaiting_paired.contains(&f));
    assert_eq!(obs.forwarder_attach_ended, 1, "counted once");
    assert!(g.io.outbox().is_empty(), "no message: the host already attached");
    assert!(g.io.vsock_ended().contains(&f), "V_g closed");
    assert_eq!(g.io.forwarder_inventory().map(|i| (i.attached, i.live_pairs)), Some((0, 0)));
}

/// EVMOD-ATTACH-END, guest TcpConnect on `Paired`, V_g ended (`HalfEnded`):
/// the host aborted after its `Paired` left (V_g reset before the guest's
/// attach); the owner closes I with linger and V_g and counts it.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_v_g_reset_before_paired_is_refused_half_ended_and_i_is_reset() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.capture_flow(cookie(7), dest(1));
    g.vsock(f).peer_reset();
    g.io.host_control(ControlMessage::Paired { flow: f });
    g.settle();
    assert_eq!(g.obs().forwarder_attach_ended, 1);
    assert!(!g.obs().awaiting_paired.contains(&f) && !g.obs().flows.contains_key(&f));
    assert_eq!(
        g.io.guest_socket_observation().far_end.get(&f),
        Some(&FarEndEvent::Reset),
        "I reset"
    );
}

/// D25 exhaustion: with `GUEST_TCP_PENDING_MAX` TcpConnect flows pending, the
/// next captured connection is reset at once — no V_g, no flow id, no message
/// — and the pending flows are unaffected.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_capture_past_the_pending_maximum_is_reset_locally() {
    let mut g = Guest::new();
    g.open_session();
    let max = usize::try_from(GUEST_TCP_PENDING_MAX).expect("fits usize");
    for n in 0..max {
        g.io.app_connect(cookie(u64::try_from(n).unwrap() + 1), dest(1));
        g.settle();
    }
    assert_eq!(g.connects(HostChannel::FlowStream), max, "one V_g per pending flow");
    let extra = g.io.app_connect(cookie(1_000_000), dest(1));
    g.settle();
    assert_eq!(g.connects(HostChannel::FlowStream), max, "no V_g for the extra capture");
    assert!(
        g.io.effects()
            .iter()
            .any(|e| matches!(e, SimGuestEffect::Abort { socket, .. } if *socket == extra)),
        "the extra intake child is reset"
    );
    assert!(g.io.outbox().is_empty());
}

/// D20: `Refused` for a TcpConnect ends it with reset semantics — I aborted
/// (the application sees a reset, not EOF), V_g closed — and no reply.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_refused_tcp_connect_resets_the_application() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.capture_flow(cookie(7), dest(1));
    g.io.host_control(ControlMessage::Refused { flow: f, refusal: FlowRefusal::Policy });
    g.settle();
    assert!(!g.obs().awaiting_paired.contains(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    assert!(g.io.vsock_ended().contains(&f));
    assert!(g.io.outbox().is_empty(), "no reply");
}

/// An application reset of the pending intake child (`Hangup { Error }`, the
/// only event an unattached child reports) aborts the flow and sends
/// `Abort { flow }`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_application_reset_before_paired_aborts_and_sends_abort() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.capture_flow(cookie(7), dest(1));
    g.io.app_reset(f);
    g.settle();
    assert!(!g.obs().awaiting_paired.contains(&f));
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: f }]);
    assert!(g.io.vsock_ended().contains(&f));
}

/// DG-5-E (TcpConnect): the host closes V_h while the guest's V_g awaits
/// `Paired`: the flow ends with no message and I is reset.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_hosts_close_of_an_unattached_v_g_resets_i_with_no_message() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.capture_flow(cookie(7), dest(1));
    g.io.host_vsock_end(f);
    g.settle();
    assert!(!g.obs().awaiting_paired.contains(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    assert!(g.io.outbox().is_empty(), "no message");
}

/// A paired TcpConnect's pair carries both directions; the owner reads no
/// payload and holds no forwarding state of its own: the forwarder's
/// `pair_state` is the per-direction state (EVMOD-PORT).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_paired_tcp_connect_forwards_both_ways() {
    let mut g = Guest::new();
    g.open_session();
    let f = g.connect_flow(cookie(7), dest(1));
    g.outer(f).peer_send(b"req");
    g.vsock(f).peer_send(b"resp");
    g.run_and_settle();
    assert_eq!(g.vsock(f).peer_received(), b"req".to_vec());
    assert_eq!(g.outer(f).peer_received(), b"resp".to_vec());
    let state = g.io.forwarder_pair_state(g.pair(f)).expect("live");
    assert_eq!((state.a_to_b.moved, state.b_to_a.moved), (3, 4));
    assert!(g.io.outbox().is_empty());
}
