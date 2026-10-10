//! TcpAccept, guest half (§ *Per-kind total orders* steps 3–5; the guest rows
//! of `owner_flows` `flows_ok`; P2-S10 guest half): own-token check
//! (ATTR-TOKEN), TCPACCEPT-END (P-38), K-A4 teardown in the accept queue,
//! `ApplicationNotListening`, U-1 (a connect completing after its flow closed
//! sends nothing), U-5 discards, and `Abort` receipt with reset semantics.

use std::io::ErrorKind;
use std::net::SocketAddrV4;

use overdrive_core::guest_transport::wire::{ControlMessage, FlowRefusal};

use super::super::GuestAcceptPhase;
use super::super::sim::{FarEndEvent, SimGuestEffect, SimSessionEnd};
use super::{
    CLIENT, Guest, WL, accept_request, host_flow, launch, other_launch, position,
    unknown_guest_flow,
};

/// Steps 3–5 in order: accept V_g, read the request, `inbound_ended` (false),
/// install V_g, connect C (bound to the client, to `workload_addr:port`), then
/// on its completion route V_g → C and send `Paired`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn tcp_accept_with_the_guests_token_pairs_in_the_pinned_order() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let f = host_flow(1);
    g.io.host_open_accept(accept_request(f, 8080, launch()));
    assert!(g.io.guest_socket_observation().queued_inbound.contains(&f));
    assert!(g.obs().accepted.is_empty(), "a queued V_g is not the guest's yet");

    g.io.release_inbound_accept(f);
    g.run_released();
    g.io.release_inbound_request(f);
    g.run_released();
    assert_eq!(g.obs().accepted.get(&f), Some(&GuestAcceptPhase::Accepted));
    assert!(g.io.take_outbox().is_empty(), "no message before the application connect completes");

    g.io.complete_app_connect(f, Ok(()));
    g.run_released();
    let obs = g.obs();
    assert_eq!(obs.accepted.get(&f), Some(&GuestAcceptPhase::Routed));
    assert!(obs.flows.contains_key(&f), "a routed flow is a paired flow");
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Paired { flow: f }]);

    let e = g.io.effects();
    let read =
        position(&e, |x| matches!(x, SimGuestEffect::ReadInbound { .. })).expect("request read");
    let ended = position(&e, |x| matches!(x, SimGuestEffect::InboundEnded { ended: false, .. }))
        .expect("end checked");
    let install =
        position(&e, |x| matches!(x, SimGuestEffect::InstallInbound { .. })).expect("installed");
    let connect = position(&e, |x| {
        matches!(x, SimGuestEffect::StartConnectApp { client, destination, .. }
            if *client == CLIENT && *destination == SocketAddrV4::new(WL, 8080))
    })
    .expect("C connects to workload_addr:listen_port, bound to the client");
    let route = position(&e, |x| matches!(x, SimGuestEffect::RouteInbound { .. })).expect("routed");
    let paired = position(&e, |x| {
        matches!(x, SimGuestEffect::WriteControl { message: ControlMessage::Paired { .. }, .. })
    })
    .expect("Paired written");
    assert!(
        read < ended && ended < install && install < connect && connect < route && route < paired,
        "{e:?}"
    );
}

/// ATTR-TOKEN: a request carrying another launch's token closes V_g with no
/// message, no install and no application connect.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn tcp_accept_with_another_launchs_token_closes_v_g_silently() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let before = g.obs();
    let f = host_flow(2);
    g.io.host_open_accept(accept_request(f, 8080, other_launch()));
    g.io.release_inbound_accept(f);
    g.run_released();
    g.io.release_inbound_request(f);
    g.run_released();
    let e = g.io.effects();
    assert!(position(&e, |x| matches!(x, SimGuestEffect::InstallInbound { .. })).is_none());
    assert!(position(&e, |x| matches!(x, SimGuestEffect::StartConnectApp { .. })).is_none());
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::Abort { .. } | SimGuestEffect::Close { .. }))
            .is_some(),
        "V_g closed"
    );
    assert!(g.io.take_outbox().is_empty(), "no control message");
    assert_eq!(g.obs(), before, "the guest holds nothing of the stale flow");
}

/// Step 4: C's connect refused → `Refused(ApplicationNotListening)`; the flow
/// leaves `accepted`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn refused_application_connect_answers_application_not_listening() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let f = host_flow(3);
    g.io.host_open_accept(accept_request(f, 9999, launch()));
    g.io.release_inbound_accept(f);
    g.run_released();
    g.io.release_inbound_request(f);
    g.run_released();
    g.io.complete_app_connect(f, Err(ErrorKind::ConnectionRefused));
    g.run_released();
    assert_eq!(
        g.io.take_outbox(),
        vec![ControlMessage::Refused { flow: f, refusal: FlowRefusal::ApplicationNotListening }]
    );
    let obs = g.obs();
    assert!(!obs.accepted.contains_key(&f) && !obs.flows.contains_key(&f));
}

/// TCPACCEPT-END (P-38): a V_g whose host side closed after the request is
/// read delivers the request, then reports its end; the flow ends there — no
/// C, nothing sent (K-A4).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_v_g_the_host_closed_after_the_request_ends_with_no_c_and_no_message() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let f = host_flow(4);
    g.io.host_open_accept(accept_request(f, 8080, launch()));
    g.io.release_inbound_accept(f);
    g.run_released();
    g.io.host_vsock_end(f); // the host closes V_h after writing the request
    g.io.release_inbound_request(f);
    g.run_released();
    let e = g.io.effects();
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::InboundEnded { ended: true, .. })).is_some()
    );
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::InstallInbound { .. })).is_none(),
        "no install"
    );
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::StartConnectApp { .. })).is_none(),
        "no C"
    );
    assert!(g.io.take_outbox().is_empty(), "nothing sent");
    assert!(!g.obs().accepted.contains_key(&f));
}

/// K-A4 / P2-S10: the host's close of V_h while V_g still waits in the guest
/// accept queue tears it down there; an `Abort` for that flow — one the guest
/// does not hold — is discarded with no reply (U-5). The guest's tables are
/// unchanged.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_queued_v_g_closed_by_the_host_never_reaches_the_guest_and_its_abort_is_discarded() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let before = g.obs();
    let f = host_flow(5);
    g.io.host_open_accept(accept_request(f, 8080, launch()));
    g.io.host_vsock_end(f);
    let sim = g.io.guest_socket_observation();
    assert!(sim.reset_in_queue.contains(&f) && !sim.queued_inbound.contains(&f));
    g.io.host_control(ControlMessage::Abort { flow: f });
    g.run_released();
    assert!(g.io.take_outbox().is_empty(), "no reply to a discarded Abort");
    assert_eq!(g.obs(), before);
}

/// U-5: `Paired`, `Refused`, `Abort`, `HalfClose`, `HalfCloseAck` naming a
/// flow the guest never opened or already closed are discarded with no reply.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn control_messages_for_unknown_or_closed_flows_are_discarded_without_reply() {
    let mut g = Guest::new();
    g.open_session();
    let closed = host_flow(6);
    g.accept_flow(closed, 8080);
    g.io.host_control(ControlMessage::Abort { flow: closed });
    g.run_released();
    let _ = g.io.take_outbox();
    let before = g.obs();
    for flow in [unknown_guest_flow(), host_flow(999), closed] {
        for message in [
            ControlMessage::Paired { flow },
            ControlMessage::Refused { flow, refusal: FlowRefusal::Policy },
            ControlMessage::Abort { flow },
            ControlMessage::HalfClose { flow },
            ControlMessage::HalfCloseAck { flow },
        ] {
            g.io.host_control(message);
            g.run_released();
            assert!(g.io.take_outbox().is_empty(), "no reply to {message:?}");
            assert_eq!(g.obs(), before, "{message:?} changed nothing");
        }
    }
}

/// D20: an `Abort` for a routed flow closes it with reset semantics: the
/// application leg is reset, never EOF; no reply.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn abort_of_a_routed_flow_resets_the_application_leg() {
    let mut g = Guest::new();
    g.open_session();
    let f = host_flow(7);
    g.accept_flow(f, 8080);
    let _ = g.io.take_outbox();
    g.io.host_control(ControlMessage::Abort { flow: f });
    g.run_released();
    let obs = g.obs();
    assert!(!obs.flows.contains_key(&f) && !obs.accepted.contains_key(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    assert!(g.io.take_outbox().is_empty(), "Abort is not answered");
    let e = g.io.effects();
    let removed =
        position(&e, |x| matches!(x, SimGuestEffect::RemovePair { .. })).expect("pair removed");
    let aborted =
        position(&e, |x| matches!(x, SimGuestEffect::Abort { flow: Some(fl), .. } if *fl == f))
            .expect("sockets aborted");
    assert!(removed < aborted, "remove_pair, then abort every socket");
}

/// U-1, step 5: when the session is lost while C is connecting, the flow is
/// already Closed when the connect completes: C is closed and nothing is sent.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_connect_completing_after_session_loss_sends_nothing_and_closes_c() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let f = host_flow(8);
    g.io.host_open_accept(accept_request(f, 8080, launch()));
    g.io.release_inbound_accept(f);
    g.run_released();
    g.io.release_inbound_request(f);
    g.run_released();
    g.io.end_session(SimSessionEnd::Reset);
    g.run_released();
    assert!(!g.obs().accepted.contains_key(&f), "session loss aborts every flow");
    if g.io
        .pending_connects()
        .iter()
        .any(|(_, t)| matches!(t, super::super::sim::SimConnectTarget::App { .. }))
    {
        g.io.complete_app_connect(f, Ok(()));
        g.run_released();
    }
    let e = g.io.effects();
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::RouteInbound { .. })).is_none(),
        "never routed"
    );
    assert!(
        position(&e, |x| matches!(
            x,
            SimGuestEffect::WriteControl { message: ControlMessage::Paired { .. }, .. }
        ))
        .is_none()
    );
    assert_ne!(
        g.io.guest_socket_observation().far_end.get(&f),
        Some(&FarEndEvent::Eof),
        "C never closed cleanly"
    );
}
