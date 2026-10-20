//! A flow's end at the guest (DG-5-D, EVMOD-REC, EVMOD-END, CLEAN-FULL-END,
//! END-IDEMPOTENT; feature-delta § *Failure projection*): the guest owner
//! takes no drain step — the guest forwarder carries each direction's end to
//! its target after the last accepted byte and ends the pair — and handles
//! each pair's one end record in one dispatch: the flow leaves its table, a
//! completed TCP half's linger is cleared, both halves are closed. An
//! `Abort { flow }` for a paired flow aborts its pair; one for a flow the guest
//! does not hold is discarded (R5-28, guest half).

use overdrive_core::guest_transport::GUEST_OWNER_AUDIT_PERIOD;
use overdrive_core::guest_transport::forwarder::{AbortCause, ForwardOutcome, PairSide};
use overdrive_core::guest_transport::wire::ControlMessage;
use overdrive_sim::adapters::{SimFarEnd, SimShutdown};

use super::super::sim::SimGuestEffect;
use super::*;

/// A paired TcpConnect flow (I half A, V_g half B) on a live session.
fn paired(g: &mut Guest, n: u16) -> FlowId {
    g.open_session();
    let f = g.connect_flow(cookie(u64::from(n)), dest(n));
    g.io.clear_effects();
    f
}

/// CLEAN-FULL-END: the application half-closes after its bytes; the
/// forwarder carries the bytes then a `0x2` on V_g (the owner takes no step);
/// the host's bytes and `0x2` reach the application as data then EOF; the pair
/// completes only after both directions ended cleanly, and on its `Completed`
/// record the owner — in that one dispatch — removes the flow, clears I's
/// linger and closes both halves. The application reads every byte, then EOF.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_clean_end_both_ways_completes_and_the_owner_closes_on_the_record() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    let pair = g.pair(f);
    g.outer(f).peer_send(b"abc");
    g.outer(f).peer_shutdown(SimShutdown::Send);
    g.io.run_forwarder();
    assert_eq!(g.vsock(f).peer_received(), b"abc".to_vec());
    assert_eq!(g.vsock(f).sent_shutdown(), Some(SimShutdown::Send), "SHUT_WR after the last byte");
    assert!(g.obs().flows.contains_key(&f), "one direction open: the pair is live");
    assert!(
        !g.io.effects().iter().any(|e| matches!(
            e,
            SimGuestEffect::ClearLinger { .. } | SimGuestEffect::Close { .. }
        )),
        "the owner takes no drain step"
    );

    g.vsock(f).peer_send(b"xy");
    g.vsock(f).peer_shutdown(SimShutdown::Send);
    g.io.run_forwarder();
    let ends = g.io.release_forward_ends();
    assert_eq!(ends.len(), 1, "one record per pair");
    assert_eq!(ends[0].id, pair);
    assert_eq!(ends[0].outcome, ForwardOutcome::Completed);
    g.step();
    assert!(
        !g.obs().flows.contains_key(&f),
        "the flow leaves in the dispatch that reads its record"
    );
    let effects = g.io.effects();
    let clear = position(&effects, |e| matches!(e, SimGuestEffect::ClearLinger { .. }))
        .expect("linger cleared");
    let closes = effects
        .iter()
        .filter(|e| matches!(e, SimGuestEffect::Close { flow: Some(x), .. } if *x == f))
        .count();
    assert_eq!(closes, 2, "both halves closed");
    assert!(clear < position(&effects, |e| matches!(e, SimGuestEffect::Close { .. })).unwrap());
    assert_eq!(g.outer(f).peer_received(), b"xy".to_vec());
    assert_eq!(g.outer(f).peer_end(), Some(SimFarEnd::Eof));
    assert!(g.io.outbox().is_empty(), "no control message carries a flow's end (R2-END)");
}

/// EVMOD-END: the host's `0x3` while a direction is open is an abort: the
/// forwarder resets I and shuts V_g, the record is `Aborted { PeerAbort }`;
/// the owner removes the flow and closes both halves (no linger clearing) and
/// sends nothing (the host already ended it). The application reads a reset.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_peer_abort_record_closes_the_flow_and_the_application_reads_a_reset() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.vsock(f).peer_shutdown(SimShutdown::Both);
    g.io.run_forwarder();
    let ends = g.io.release_forward_ends();
    assert!(
        matches!(
            ends.as_slice(),
            [e] if matches!(e.outcome, ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::B } })
        ),
        "{ends:?}"
    );
    g.settle();
    assert!(!g.obs().flows.contains_key(&f));
    assert!(!g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::ClearLinger { .. })));
    assert_eq!(g.outer(f).peer_end(), Some(SimFarEnd::Reset));
    assert!(g.io.outbox().is_empty());
}

/// A half-close then the peer's close is clean (CLEAN-FULL-END): the host's
/// `0x2`, then its `0x3` after the guest's own direction ended, completes the
/// pair — the `0x3` arrives after this side shut its send.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_half_close_then_the_peers_close_after_both_ends_is_clean() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.vsock(f).peer_shutdown(SimShutdown::Send);
    g.outer(f).peer_shutdown(SimShutdown::Send);
    g.io.run_forwarder();
    g.vsock(f).peer_shutdown(SimShutdown::Both);
    g.io.run_forwarder();
    let ends = g.io.release_forward_ends();
    assert!(matches!(ends.as_slice(), [e] if e.outcome == ForwardOutcome::Completed), "{ends:?}");
    g.settle();
    assert_eq!(g.outer(f).peer_end(), Some(SimFarEnd::Eof));
}

/// EVMOD-END, TCP halves: the application resets while the pair is live
/// (`sk_error_report` in the reset's own step): the record is
/// `Aborted { SocketError { side: A, errno: ECONNRESET } }`; the owner closes
/// both halves; the host's V_g is shut both ways.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn an_application_reset_while_paired_is_a_socket_error_record() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.io.app_reset(f);
    let ends = g.io.release_forward_ends();
    assert!(
        matches!(
            ends.as_slice(),
            [e] if matches!(e.outcome, ForwardOutcome::Aborted { cause: AbortCause::SocketError { side: PairSide::A, errno: x } } if x == errno(nix::libc::ECONNRESET))
        ),
        "{ends:?}"
    );
    g.settle();
    assert!(!g.obs().flows.contains_key(&f));
    assert_eq!(g.vsock(f).sent_shutdown(), Some(SimShutdown::Both));
}

/// The application half-closes and then resets (FIN then RST, the half in
/// `CLOSE_WAIT`'s mirror on the guest side): the reset still aborts the pair —
/// a FIN then a reset is never a clean end (`ResetNeverEndsClean`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_fin_then_a_reset_from_the_application_aborts_the_pair() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.outer(f).peer_shutdown(SimShutdown::Send);
    g.io.run_forwarder();
    g.io.app_reset(f);
    g.io.run_forwarder();
    let ends = g.io.release_forward_ends();
    assert!(
        matches!(ends.as_slice(), [e] if matches!(e.outcome, ForwardOutcome::Aborted { .. })),
        "{ends:?}"
    );
    g.settle();
    assert!(!g.obs().flows.contains_key(&f));
}

/// `Abort { flow }` for a paired flow aborts its pair (`abort_forward`): the
/// `Detached` record follows; the owner closes both halves; the application
/// is reset; no reply.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn an_abort_message_for_a_paired_flow_aborts_its_pair() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    let pair = g.pair(f);
    g.io.host_control(ControlMessage::Abort { flow: f });
    g.settle();
    assert!(g.io.effects().contains(&SimGuestEffect::AbortForward { id: pair }));
    g.run_and_settle();
    assert!(!g.obs().flows.contains_key(&f));
    assert_eq!(g.outer(f).peer_end(), Some(SimFarEnd::Reset));
    assert!(g.io.outbox().is_empty());
}

/// R5-28, guest half: an `Abort` for a flow the guest does not hold — never
/// opened, or already ended — is discarded with no reply and changes nothing.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 5"]
fn an_abort_for_a_flow_the_guest_does_not_hold_is_discarded() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.vsock(f).peer_shutdown(SimShutdown::Both);
    g.run_and_settle();
    let before = g.obs();
    for flow in [f, unknown_guest_flow(), host_flow(9)] {
        g.io.host_control(ControlMessage::Abort { flow });
        g.settle();
        assert_eq!(g.obs(), before, "{flow:?}");
    }
    assert!(g.io.outbox().is_empty());
    assert!(
        !g.io.effects().iter().any(|e| matches!(e, SimGuestEffect::AbortForward { .. })),
        "nothing to abort"
    );
}

/// ADR-0183 / DG-5-D: a slow reader is backpressure, not a deadline. The host
/// stops reading; the application keeps writing; an hour of audit ticks
/// passes; the flow stays paired, nothing is aborted, and once the host reads
/// again every byte arrives in order.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_slow_reader_holds_the_flow_without_any_deadline() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.vsock(f).peer_stop_reading();
    let payload: Vec<u8> = (0..=255u8).cycle().take(64 * 1024).collect();
    g.outer(f).peer_send(&payload);
    g.io.run_forwarder();
    for _ in 0..3_600 {
        g.advance(GUEST_OWNER_AUDIT_PERIOD);
    }
    assert!(g.obs().flows.contains_key(&f), "no drain or idle deadline ends a TCP flow");
    assert!(
        !g.io.effects().iter().any(|e| matches!(
            e,
            SimGuestEffect::AbortForward { .. } | SimGuestEffect::Abort { .. }
        ))
    );
    g.vsock(f).peer_resume_reading();
    g.io.run_forwarder();
    assert_eq!(g.vsock(f).peer_received(), payload);
}

/// END-IDEMPOTENT: a trailing reset after the pair completed and the owner
/// closed it changes nothing: no second record, no owner step acts on it.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_trailing_reset_after_completion_is_inert() {
    let mut g = Guest::new();
    let f = paired(&mut g, 1);
    g.outer(f).peer_shutdown(SimShutdown::Send);
    g.vsock(f).peer_shutdown(SimShutdown::Send);
    g.run_and_settle();
    let before = g.obs();
    let inventory = g.io.forwarder_inventory();
    g.vsock(f).peer_reset();
    g.run_and_settle();
    assert_eq!(g.obs(), before);
    assert_eq!(g.io.forwarder_inventory(), inventory, "no further record");
    let state = g.io.forwarder_pair_state(g.pair(f));
    assert_eq!(state, None, "the pair is gone");
}
