//! TcpAccept, guest side (INB-1; feature-delta § *Guest adaptation contract*
//! "TcpAccept, guest side", § *Per-kind total orders* TcpAccept steps 1–6,
//! EVMOD-PAIR, EVMOD-ATTACH-END, TCPACCEPT-END, DG-5-E, U-1, U-5) and the
//! guest half of R5-28 and R5-59 (b).
//!
//! The guest attaches (V_g, C) once C's connect completed and before it writes
//! `Paired`, in one dispatch (EVMOD-PAIR); V_g is half A, C half B ("Sides and
//! pairs").

use std::io::ErrorKind;

use nix::libc;
use overdrive_core::guest_transport::forwarder::PairSide;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowRefusal, GUEST_INBOUND_CONNECT_TIMEOUT,
};
use overdrive_sim::adapters::{SimFarEnd, SimShutdown};

use super::super::GuestAcceptPhase;
use super::super::sim::{FarEndEvent, SimConnectTarget, SimGuestEffect};
use super::super::test_support::ACCEPT_PATH_CASES;
use super::*;

/// INB-1 steps 2–4, the pinned order: the owner reads the 32-byte request,
/// compares the launch token, checks the end of V_g (TCPACCEPT-END), creates C
/// bound to the request's peer (`gateway:0`) and connects it to
/// `workload_addr:listen_port`; on `Connected` it attaches (V_g, C) and then
/// writes exactly one `Paired`. `accepted[h]` goes `Accepted → Paired { pair }`
/// and `flows[h]` holds the same pair.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_dial_with_the_guests_token_attaches_at_connect_and_then_pairs() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.io.clear_effects();
    g.accept_until_connecting(h, 8080);
    assert_eq!(g.obs().accepted.get(&h), Some(&GuestAcceptPhase::Accepted));
    let pending = g.io.pending_connects();
    assert!(
        pending.iter().any(|(_, t)| *t
            == SimConnectTarget::App {
                client: accept_client(),
                destination: SocketAddrV4::new(WL, 8080),
                flow: Some(h),
            }),
        "C is bound to gateway:0 and connects to workload_addr:listen_port: {pending:?}"
    );
    assert!(g.io.outbox().is_empty(), "no reply before C connected");

    g.io.complete_app_connect(h, Ok(()));
    g.settle();
    let effects = g.io.effects();
    let read =
        position(&effects, |e| matches!(e, SimGuestEffect::ReadInbound { flow, .. } if *flow == h))
            .expect("the request is read");
    let ended =
        position(&effects, |e| matches!(e, SimGuestEffect::InboundEnded { ended: false, .. }))
            .expect("the end of V_g is checked after the read");
    let connect = position(&effects, |e| matches!(e, SimGuestEffect::StartConnectApp { .. }))
        .expect("C is connected");
    let attach = position(&effects, |e| {
        matches!(e, SimGuestEffect::AttachInbound { accepted: Some(_), .. })
    })
    .expect("the pair is attached");
    let paired = position(&effects, |e| matches!(e, SimGuestEffect::WriteControl { message: ControlMessage::Paired { flow }, .. } if *flow == h))
        .expect("Paired is written");
    assert!(read < ended && ended < connect && connect < attach && attach < paired, "{effects:?}");
    let pair = g.pair(h);
    assert_eq!(g.obs().accepted.get(&h), Some(&GuestAcceptPhase::Paired { pair }));
    assert_eq!(g.obs().flows.get(&h), Some(&pair));
    assert_eq!(g.io.vsock_side(h), Some(PairSide::A), "V_g is half A of a TcpAccept pair");
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Paired { flow: h }], "exactly one reply");
    assert_eq!(
        effects.iter().filter(|e| matches!(e, SimGuestEffect::ReadInbound { .. })).count(),
        1,
        "the request is the only read on V_g (no payload in guest userspace)"
    );
}

/// A paired dial forwards both ways through the guest pair: the host's bytes
/// reach the application, the application's bytes reach the host, and the
/// guest owner reads no payload.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_paired_dial_forwards_both_ways_through_the_guest_pair() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_flow(h, 8080);
    g.vsock(h).peer_send(b"GET /");
    g.outer(h).peer_send(b"200 OK");
    g.run_and_settle();
    assert_eq!(
        g.outer(h).peer_received(),
        b"GET /".to_vec(),
        "the application received the host's bytes"
    );
    assert_eq!(
        g.vsock(h).peer_received(),
        b"200 OK".to_vec(),
        "the host received the application's bytes"
    );
    assert!(g.obs().flows.contains_key(&h), "the flow stays paired");
}

/// ATTR-TOKEN: a request carrying another launch's token closes V_g with no
/// message, creates no C and attaches nothing.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_dial_with_another_launchs_token_closes_v_g_silently() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.io.host_open_accept(accept_request(h, 8080, other_launch()));
    g.io.release_inbound_accept(h);
    g.settle();
    g.io.release_inbound_request(h);
    g.settle();
    assert!(g.obs().accepted.is_empty());
    assert!(g.io.vsock_ended().contains(&h), "V_g is closed");
    assert!(!g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::StartConnectApp { .. })));
    assert!(g.io.outbox().is_empty(), "no message");
}

/// TCPACCEPT-END (P-38): the host closed V_h after writing the request, so V_g
/// reads the request and then reports its end; the owner's check after the
/// request read finds it: the flow ends with no C and no message.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_v_g_the_host_closed_after_the_request_ends_with_no_c_and_no_message() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.io.host_open_accept(accept_request(h, 8080, launch()));
    g.io.release_inbound_accept(h);
    g.settle();
    g.io.release_inbound_request(h);
    g.io.host_vsock_end(h);
    g.settle();
    assert!(
        g.io.effects()
            .iter()
            .any(|e| matches!(e, SimGuestEffect::InboundEnded { ended: true, .. }))
    );
    assert!(
        !g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::StartConnectApp { .. })),
        "no C"
    );
    assert!(g.obs().accepted.is_empty());
    assert!(g.io.outbox().is_empty(), "no message");
}

/// INB-1: a refused application connect aborts C and V_g and answers
/// `Refused { ApplicationRefused { errno } }` with the connect's errno
/// (`ECONNREFUSED` when nothing listens), once.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_refused_application_connect_answers_refused_with_its_errno() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.complete_app_connect(h, Err(ErrorKind::ConnectionRefused));
    g.settle();
    assert_eq!(
        g.io.take_outbox(),
        vec![ControlMessage::Refused {
            flow: h,
            refusal: FlowRefusal::ApplicationRefused { errno: errno(libc::ECONNREFUSED) },
        }]
    );
    assert!(g.obs().accepted.is_empty());
    assert!(g.io.vsock_ended().contains(&h), "V_g is aborted");
    assert_eq!(g.io.forwarder_inventory().map(|i| i.attached), Some(0), "nothing was attached");
}

/// INB-1: C not connected by `GUEST_INBOUND_CONNECT_TIMEOUT` (1.5 s) — nothing
/// is sent before it; at it the owner aborts C and V_g and answers
/// `TimedOut { errno: ETIMEDOUT }` once; a later completion of C's connect is
/// discarded (U-1).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_application_connect_past_the_inbound_timeout_answers_timed_out_once() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    let start = g.now();
    g.accept_until_connecting(h, 8080);
    g.advance_to(start + GUEST_INBOUND_CONNECT_TIMEOUT.saturating_sub(Duration::from_millis(1)));
    assert!(g.io.outbox().is_empty(), "nothing before the timeout");
    g.advance_to(start + GUEST_INBOUND_CONNECT_TIMEOUT);
    assert_eq!(
        g.io.take_outbox(),
        vec![ControlMessage::TimedOut { flow: h, errno: errno(libc::ETIMEDOUT) }]
    );
    assert!(g.obs().accepted.is_empty());
    assert_eq!(
        g.io.guest_socket_observation().far_end.get(&h),
        Some(&FarEndEvent::Reset),
        "C aborted"
    );
    g.advance(GUEST_INBOUND_CONNECT_TIMEOUT * 4);
    assert!(g.io.outbox().is_empty(), "exactly one reply");
}

/// DG-5-E: an accepted, unattached V_g the host closes (`ReadHangup`) while C
/// connects ends the flow in that dispatch with no message: C is aborted; a
/// later `Connected` for C finds the flow closed and attaches nothing (U-1).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_accepted_v_g_closed_by_the_host_ends_silently_and_cannot_pair() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    let c =
        g.io.pending_connects()
            .into_iter()
            .find(|(_, t)| matches!(t, SimConnectTarget::App { .. }))
            .map(|(id, _)| id)
            .expect("C is connecting");
    g.io.host_vsock_end(h);
    g.step();
    assert!(g.obs().accepted.is_empty(), "the flow ends in the dispatch that receives the hangup");
    assert!(g.io.outbox().is_empty(), "no message");
    assert!(
        g.io.effects()
            .iter()
            .any(|e| matches!(e, SimGuestEffect::Abort { socket, .. } if *socket == c)),
        "C is aborted"
    );
    assert!(g.io.pending_connects().is_empty());
    assert_eq!(g.io.forwarder_inventory().map(|i| i.attached), Some(0));
}

/// DG-5-E: an error on an accepted, unattached V_g aborts the flow and sends
/// `Abort { flow }` once.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_error_on_an_accepted_v_g_aborts_with_abort() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.flow_vsock_error(h, ErrorKind::ConnectionReset);
    g.settle();
    assert!(g.obs().accepted.is_empty());
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: h }]);
}

/// K-A4 + U-5: a V_g the host closes while it is still in the guest's accept
/// queue never reaches the guest owner; the host's `Abort` for it — a flow the
/// guest does not hold — is discarded with no reply (R5-28, guest half).
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn a_queued_v_g_closed_by_the_host_never_reaches_the_guest_and_its_abort_is_discarded() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.io.host_open_accept(accept_request(h, 8080, launch()));
    g.io.host_vsock_end(h);
    let before = g.obs();
    g.io.host_control(ControlMessage::Abort { flow: h });
    g.settle();
    assert!(g.io.guest_socket_observation().reset_in_queue.contains(&h));
    assert_eq!(g.obs(), before, "nothing changes");
    assert!(g.io.outbox().is_empty(), "no reply");
}

/// `Abort { flow }` for a paired dial aborts its pair (`abort_forward`): the
/// forwarder resets C and shuts V_g; the owner closes both on the `Detached`
/// record; no reply.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn abort_of_a_paired_dial_aborts_its_pair_and_resets_the_application() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    let pair = g.accept_flow(h, 8080);
    g.io.take_outbox();
    g.io.host_control(ControlMessage::Abort { flow: h });
    g.settle();
    assert!(g.io.effects().contains(&SimGuestEffect::AbortForward { id: pair }));
    g.run_and_settle();
    assert!(g.obs().accepted.is_empty() && g.obs().flows.is_empty());
    assert_eq!(g.outer(h).peer_end(), Some(SimFarEnd::Reset), "the application is reset");
    assert_eq!(g.vsock(h).sent_shutdown(), Some(SimShutdown::Both), "V_g shut both ways");
    assert!(g.io.outbox().is_empty(), "no reply to an Abort");
}

/// EVMOD-ATTACH-END, guest TcpAccept before `Paired`, C ended: the
/// application reset after accepting, so the attach is refused `HalfEnded`;
/// the owner writes `Refused { ApplicationRefused { errno: end_errno } }`,
/// closes C (linger) and V_g, counts `forwarder_attach_ended`, and the flow
/// ends.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_application_reset_before_the_attach_is_refused_and_answered_with_its_errno() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    let c = g.io.complete_app_connect(h, Ok(()));
    // the application resets between accepting and the owner's dispatch
    g.io.kernel_socket(c).peer_reset();
    g.settle();
    let obs = g.obs();
    assert!(obs.accepted.is_empty() && obs.flows.is_empty());
    assert_eq!(obs.forwarder_attach_ended, 1);
    assert_eq!(
        g.io.take_outbox(),
        vec![ControlMessage::Refused {
            flow: h,
            refusal: FlowRefusal::ApplicationRefused { errno: errno(libc::ECONNRESET) },
        }]
    );
    assert!(g.io.vsock_ended().contains(&h));
    assert_eq!(g.io.forwarder_inventory().map(|i| (i.attached, i.live_pairs)), Some((0, 0)));
}

/// EVMOD-ATTACH-END, guest TcpAccept before `Paired`, V_g ended: the host's
/// dial ended between the request read and the attach (V_g in `CLOSING`), so
/// the attach is refused `HalfEnded`; the owner closes C (linger) and V_g,
/// sends nothing (a reply would be late, U-5), and counts
/// `forwarder_attach_ended`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_v_g_ended_before_the_attach_is_refused_and_ends_with_no_message() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.complete_app_connect(h, Ok(()));
    // the host's close of V_h reaches V_g after the owner's TCPACCEPT-END check
    g.vsock(h).peer_reset();
    g.settle();
    let obs = g.obs();
    assert!(obs.accepted.is_empty());
    assert_eq!(obs.forwarder_attach_ended, 1);
    assert!(g.io.outbox().is_empty(), "nothing is sent");
    assert_eq!(
        g.io.guest_socket_observation().far_end.get(&h),
        Some(&FarEndEvent::Reset),
        "C closed with linger"
    );
}

/// U-5 for every control message the guest can receive for a dial it no
/// longer holds: a `Paired`, `Refused` or `Abort` for a closed host-origin flow
/// is discarded without reply.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn control_messages_for_closed_dials_are_discarded_without_reply() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.complete_app_connect(h, Err(ErrorKind::ConnectionRefused));
    g.settle();
    g.io.take_outbox();
    let before = g.obs();
    for message in [
        ControlMessage::Abort { flow: h },
        ControlMessage::Paired { flow: h },
        ControlMessage::Refused { flow: h, refusal: FlowRefusal::Policy },
    ] {
        g.io.host_control(message);
        g.settle();
        assert_eq!(g.obs(), before, "{message:?}");
    }
    assert!(g.io.outbox().is_empty());
}

/// U-1: a C whose connect completes after the session's loss sends nothing and
/// attaches nothing (the flow was aborted by the loss, DG-5-F).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_connect_completing_after_session_loss_sends_nothing() {
    let mut g = Guest::new();
    g.open_session();
    let h = host_flow(1);
    g.accept_until_connecting(h, 8080);
    g.io.end_session(super::super::sim::SimSessionEnd::Eof);
    g.settle();
    assert!(
        g.io.pending_connects().iter().all(|(_, t)| !matches!(t, SimConnectTarget::App { .. })),
        "C aborted"
    );
    assert!(g.obs().accepted.is_empty());
    assert_eq!(g.io.forwarder_inventory().map(|i| i.attached), Some(0));
}

/// R5-59 (b), the sim half: the accept-path outcome per connection, by its
/// first record — a connection with its `TcpAccept` request buffered is
/// served (the application accepts it and bytes round-trip), one closed by
/// the host before any byte (no request) yields no application connection. The
/// real-kernel half (`guest_flow::real_kernel`) drives the same table through
/// `KernelGuestOwnerIo` and compares per connection.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn accept_path_outcomes_by_first_record() {
    for case in ACCEPT_PATH_CASES {
        let mut g = Guest::new();
        g.open_session();
        let h = host_flow(case.seq);
        if case.request_buffered {
            g.io.host_open_accept(accept_request(h, case.port, launch()));
        } else {
            g.io.host_open_accept_without_request(h);
        }
        g.io.release_inbound_accept(h);
        g.settle();
        g.io.release_inbound_request(h);
        g.settle();
        if case.request_buffered {
            g.io.complete_app_connect(h, Ok(()));
            g.settle();
            g.vsock(h).peer_send(case.payload);
            g.run_and_settle();
            assert_eq!(g.outer(h).peer_received(), case.payload.to_vec(), "{case:?}: served");
        } else {
            assert!(g.io.vsock_ended().contains(&h), "{case:?}: V_g closed");
            assert!(
                !g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::StartConnectApp { .. })),
                "{case:?}: no application connection"
            );
        }
    }
}
