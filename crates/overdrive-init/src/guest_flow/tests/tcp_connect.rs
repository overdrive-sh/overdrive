//! TcpConnect, guest half (§ *Per-kind total orders* steps 1–2 and 7; § *Flow
//! end* "Outer-leg EOF before Paired"): the captured connect opens V_g, which
//! is installed toward I before the request is written once; `Paired` routes
//! the parking cell; no parking cell → local reset; `Refused` and an
//! application reset before `Paired` end the flow with reset semantics; an
//! early application EOF is a half-close carried out at `Paired`.

use overdrive_core::guest_transport::wire::{ControlMessage, FlowOrigin, FlowRefusal, FlowRequest};

use super::super::HalfCloseAnnounce;
use super::super::sim::{FarEndEvent, SimAppEnd, SimAppTarget, SimGuestEffect, sim_cookie};
use super::{Guest, dest, launch, position};
use crate::guest_flow::HostChannel;

/// Steps 2 and 7: re-arm I, connect V_g to (2, 1240), install V_g → I, write
/// the TcpConnect request (guest origin, the original destination, the
/// guest's token) exactly once; on `Paired`, route the cell → V_g.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_captured_connect_opens_v_g_writes_the_request_once_and_routes_on_paired() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let cookie = sim_cookie(21);
    let destination = dest(1);
    g.io.app_connect(cookie, destination, true);
    g.run_released();
    assert_eq!(g.connects(HostChannel::FlowStream), 1, "V_g connects to 1240");
    g.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
    g.run_released();

    let f = g.last_request_flow();
    assert_eq!(f.origin(), FlowOrigin::Guest);
    let e = g.io.effects();
    let requests: Vec<&FlowRequest> = e
        .iter()
        .filter_map(|x| match x {
            SimGuestEffect::WriteRequest { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(requests, vec![&FlowRequest::TcpConnect { flow: f, destination, launch: launch() }]);
    let rearm =
        position(&e, |x| matches!(x, SimGuestEffect::RearmCaptured { .. })).expect("I re-armed");
    let connect = position(&e, |x| {
        matches!(x, SimGuestEffect::ConnectHost { channel: HostChannel::FlowStream, .. })
    })
    .expect("V_g connect");
    let install =
        position(&e, |x| matches!(x, SimGuestEffect::InstallOutbound { .. })).expect("installed");
    let write =
        position(&e, |x| matches!(x, SimGuestEffect::WriteRequest { .. })).expect("request");
    assert!(rearm < connect && connect < install && install < write, "{e:?}");
    assert_eq!(g.obs().awaiting_paired.get(&f), Some(&false));
    assert!(g.io.take_outbox().is_empty(), "the request travels on V_g, never the session");

    g.io.host_control(ControlMessage::Paired { flow: f });
    g.run_released();
    let obs = g.obs();
    assert!(!obs.awaiting_paired.contains_key(&f), "awaiting_paired and flows are disjoint");
    assert!(obs.flows.contains_key(&f));
    assert!(
        position(
            &g.io.effects(),
            |x| matches!(x, SimGuestEffect::RouteOutbound { flow: Some(fl), .. } if *fl == f)
        )
        .is_some()
    );
}

/// Step 1: no free parking cell (`parked = false`) → the owner resets I
/// locally; no V_g, no flow id, no message.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_capture_without_a_parking_cell_is_reset_locally() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let before = g.obs();
    g.io.app_connect(sim_cookie(22), dest(2), false);
    g.run_released();
    let e = g.io.effects();
    assert!(
        position(&e, |x| matches!(x, SimGuestEffect::Abort { .. })).is_some(),
        "I reset (linger)"
    );
    assert_eq!(g.connects(HostChannel::FlowStream), 0, "no V_g");
    assert!(position(&e, |x| matches!(x, SimGuestEffect::WriteRequest { .. })).is_none());
    assert!(g.io.take_outbox().is_empty());
    assert_eq!(g.obs(), before);
}

/// D20: `Refused` for a TcpConnect closes it with reset semantics — the
/// application sees a reset, not EOF; no reply.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_refused_tcp_connect_resets_the_application() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let f = g.capture_flow(sim_cookie(23), dest(3));
    g.io.host_control(ControlMessage::Refused {
        flow: f,
        refusal: FlowRefusal::DestinationUnreachable,
    });
    g.run_released();
    assert!(!g.obs().awaiting_paired.contains_key(&f));
    assert_eq!(g.io.guest_socket_observation().far_end.get(&f), Some(&FarEndEvent::Reset));
    assert!(g.io.take_outbox().is_empty(), "Refused is not answered");
}

/// A TcpConnect opener has no pairing deadline (D15-R3); an error or reset on
/// I before `Paired` aborts the flow and sends `Abort`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn an_application_reset_before_paired_aborts_and_sends_abort() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let cookie = sim_cookie(24);
    let f = g.capture_flow(cookie, dest(4));
    // No pairing deadline: the flow waits across FLOW_PAIRING_DEADLINE.
    g.advance(overdrive_core::guest_transport::wire::FLOW_PAIRING_DEADLINE * 2);
    assert_eq!(g.obs().awaiting_paired.get(&f), Some(&false), "still waiting for Paired");
    g.io.app_end(SimAppTarget::Captured(cookie), SimAppEnd::Reset);
    g.run_released();
    assert!(!g.obs().awaiting_paired.contains_key(&f));
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: f }]);
}

/// § *Flow end*: an application that writes and half-closes before `Paired`
/// is a half-close, never an abort: the EOF is recorded at once, and at
/// `Paired` the outer-leg-first path runs (drain, then `HalfClose`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn an_early_application_eof_is_carried_out_at_paired() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let cookie = sim_cookie(25);
    let f = g.capture_flow(cookie, dest(5));
    g.io.app_end(SimAppTarget::Captured(cookie), SimAppEnd::HalfClose);
    g.run_released();
    assert_eq!(g.obs().awaiting_paired.get(&f), Some(&true), "EOF recorded when observed");
    assert!(g.io.take_outbox().is_empty(), "no Abort, no HalfClose before Paired");

    g.io.host_control(ControlMessage::Paired { flow: f });
    g.run_released();
    let end = g.obs().flows[&f];
    assert!(end.outer_eof);
    assert_eq!(end.announce, HalfCloseAnnounce::Pending);
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::HalfClose { flow: f }]);
    assert!(
        !g.io.guest_socket_observation().vsock_ended.contains(&f),
        "V_g not shut before HalfCloseAck"
    );
}
