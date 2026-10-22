//! EVMOD-PORT — `SimFlowForwarder` honours the `FlowForwarder` port contract
//! (GH #295, the in-kernel event-driven forwarder).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § "[REF]
//! Driven port — `FlowForwarder` (EVMOD-PORT)" — its preconditions,
//! postconditions, edge cases and observable invariants — with § "End
//! classification", § "Pair end", § "Datagram mode" and § "Control
//! interface" (accepted states, EVMOD-ATTACH-END, WATCH-ABORT). The sim is
//! the named arbiter of the forwarder's kernel facts every owner driver and
//! seeded-sim scenario reads; these cells pin that its facts are the port's
//! contract, so an owner test over the sim is evidence about the owner, not
//! about a lenient double. The same shapes run against the real module
//! through `KernelFlowForwarder` in the module's Tier-3 suite (V-EV-END,
//! V-EV-CTL, V-EV-PAIR, V-EV-DGRAM; P-69 to P-72) — the arbiter's pairing.
//!
//! Driving port: the `FlowForwarder` methods of `SimFlowForwarder`; the
//! stimulus is the sim sockets' scripted far sides and kernel facts
//! (`SimSocket::peer_*`, `set_sk_err`, `report_error`, …), and the harness
//! drives the forwarder's runs explicitly (`run_until_idle`). Layer 2,
//! in-memory: `proptest` for the inventory invariant (Mandate 9).
//!
//! RED until DELIVER step 4 implements the sim (decision B-4), and the core
//! codec of step 2 (the datagram cells frame with `DatagramFrame`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;

use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome, ForwarderError,
    PairMode, PairSide, SourceEnd,
};
use overdrive_core::guest_transport::wire::{
    DatagramFrame, KernelErrno, MAX_GUEST_DATAGRAM_FRAME, MAX_GUEST_DATAGRAM_PAYLOAD,
};
use overdrive_sim::adapters::flow_forwarder::{
    SimContextState, SimFarEnd, SimFlowForwarder, SimForwarderDevice, SimLocalRead, SimShutdown,
    SimSocket, SimSocketKind, SimSocketSetup, SimVsockState,
};
use proptest::prelude::*;

/// `EPIPE` (32): a TCP reset of a half in `CLOSE_WAIT`.
fn epipe() -> KernelErrno {
    KernelErrno::new(32).expect("EPIPE is non-zero")
}

/// One open context on a fresh device.
fn context() -> (SimForwarderDevice, SimFlowForwarder) {
    let device = SimForwarderDevice::new();
    let forwarder = device.open();
    (device, forwarder)
}

/// A host TcpConnect-shaped stream pair: half A the vsock, half B the TCP
/// destination.
fn stream_pair(forwarder: &SimFlowForwarder) -> (ForwardId, SimSocket, SimSocket) {
    let (vsock, tcp) = (SimSocket::vsock_stream(), SimSocket::tcp());
    let id = forwarder.attach(&vsock, &tcp, PairMode::Stream).expect("an admitted pair attaches");
    (id, vsock, tcp)
}

fn take_all(forwarder: &SimFlowForwarder) -> Vec<ForwardEnd> {
    let mut ends = Vec::new();
    while let Some(end) = forwarder.take_end().expect("take_end") {
        ends.push(end);
    }
    ends
}

fn frame(payload: &[u8]) -> Vec<u8> {
    DatagramFrame::new(payload).expect("within the bound").encode()
}

// ===========================================================================
// Stream pairs — forwarding, clean ends, aborts
// ===========================================================================

/// Bytes cross both ways byte-exact; a clean end of each source is carried
/// to its target after the last byte; the pair completes once both
/// directions ended cleanly, with one record carrying the bytes each target
/// accepted, and leaves the inventory.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_stream_pair_forwards_both_ways_and_completes_on_two_clean_ends() {
    let (_device, forwarder) = context();
    let (id, vsock, tcp) = stream_pair(&forwarder);

    vsock.peer_send(b"GET / HTTP/1.1\r\n\r\n");
    tcp.peer_send(b"HTTP/1.1 200 OK\r\n\r\n");
    forwarder.run_until_idle();
    assert_eq!(tcp.peer_received(), b"GET / HTTP/1.1\r\n\r\n");
    assert_eq!(vsock.peer_received(), b"HTTP/1.1 200 OK\r\n\r\n");
    assert_eq!(forwarder.take_end().expect("take_end"), None, "a live pair has no record");

    vsock.peer_shutdown(SimShutdown::Send);
    forwarder.run_until_idle();
    let state = forwarder.pair_state(id).expect("pair_state").expect("still live");
    assert_eq!(state.a_to_b.source_end, Some(SourceEnd::Clean));
    assert!(state.a_to_b.target_shut, "the clean end reached the target");
    assert_eq!(state.a_to_b.moved, 18);
    assert_eq!(state.b_to_a.source_end, None);
    assert_eq!(tcp.peer_end(), Some(SimFarEnd::Eof));

    tcp.peer_shutdown(SimShutdown::Send);
    forwarder.run_until_idle();
    assert_eq!(
        take_all(&forwarder),
        vec![ForwardEnd { id, outcome: ForwardOutcome::Completed, moved_ab: 18, moved_ba: 19 }]
    );
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Send), "the vsock target got 0x2");
    assert_eq!(forwarder.pair_state(id).expect("pair_state"), None, "an ended pair has no state");
    let inventory = forwarder.inventory().expect("inventory");
    assert_eq!((inventory.live_pairs, inventory.completed, inventory.queued_ends), (0, 1, 0));
    assert!(!vsock.is_hooked() && !tcp.is_hooked(), "both halves unhooked at the end");
}

/// EVMOD-END: the vsock peer's `0x3` with a direction open aborts the pair
/// (`PeerAbort` on the vsock half), resets the TCP far end, and forwards no
/// byte past the abort.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_vsock_peer_abort_with_a_direction_open_resets_the_tcp_far_end() {
    let (_device, forwarder) = context();
    let (id, vsock, tcp) = stream_pair(&forwarder);
    vsock.peer_send(b"partial");
    forwarder.run_until_idle();
    vsock.peer_shutdown(SimShutdown::Both);
    vsock.peer_send(b"after");
    forwarder.run_until_idle();

    let ends = take_all(&forwarder);
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].id, id);
    assert_eq!(
        ends[0].outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert_eq!(tcp.peer_received(), b"partial", "no byte past the abort");
    assert_eq!(tcp.peer_end(), Some(SimFarEnd::Reset));
    assert_eq!(vsock.recorded_class(), Some(SourceEnd::Abort));
}

/// A TCP reset after the destination's FIN, with the other direction still
/// open, aborts the pair through `sk_error_report` with `EPIPE` (the half
/// was in `CLOSE_WAIT`) — the end-of-stream the forwarder already read does
/// not make it clean (hazard 16).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_tcp_reset_after_its_fin_aborts_the_pair_with_the_reset_errno() {
    let (_device, forwarder) = context();
    let (_id, vsock, tcp) = stream_pair(&forwarder);
    tcp.peer_shutdown(SimShutdown::Send);
    forwarder.run_until_idle();
    tcp.peer_reset();
    forwarder.run_until_idle();

    let ends = take_all(&forwarder);
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0].outcome,
        ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: PairSide::B, errno: epipe() }
        }
    );
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Both), "the vsock target was shut 0x3");
}

/// CLEAN-FULL-END / END-IDEMPOTENT: once both directions ended cleanly the
/// pair has completed; the vsock peer's later `0x3` (it released the flow)
/// and a trailing reset change nothing and queue no second record.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ends_after_a_clean_full_end_change_nothing() {
    let (_device, forwarder) = context();
    let (id, vsock, tcp) = stream_pair(&forwarder);
    vsock.peer_shutdown(SimShutdown::Send);
    tcp.peer_shutdown(SimShutdown::Send);
    forwarder.run_until_idle();
    assert_eq!(
        take_all(&forwarder).iter().map(|e| (e.id, e.outcome)).collect::<Vec<_>>(),
        [(id, ForwardOutcome::Completed)]
    );

    vsock.peer_shutdown(SimShutdown::Both);
    tcp.peer_reset();
    forwarder.run_until_idle();
    assert_eq!(forwarder.take_end().expect("take_end"), None, "at most one record per id");
    assert_eq!(tcp.peer_end(), Some(SimFarEnd::Eof), "the far end saw the clean end, no reset");
}

/// A target whose reader stops reading stops the sender: the source keeps
/// what the target did not accept; when the reader resumes, the rest
/// arrives in order.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_target_that_does_not_read_stops_the_sender_until_it_reads_again() {
    let (_device, forwarder) = context();
    let (_id, vsock, tcp) = stream_pair(&forwarder);
    let payload: Vec<u8> =
        (0..4 * 1024 * 1024).map(|i| u8::try_from(i % 251).expect("< 251")).collect();

    tcp.peer_stop_reading();
    vsock.peer_send(&payload);
    forwarder.run_until_idle();
    assert!(
        tcp.peer_received().len() < payload.len(),
        "the stalled target accepted only its buffer"
    );
    assert!(vsock.queued() > 0, "the source keeps the remainder");

    tcp.peer_resume_reading();
    forwarder.run_until_idle();
    assert_eq!(tcp.peer_received(), payload, "every byte, in order, once the reader reads");
    assert_eq!(vsock.queued(), 0);
}

// ===========================================================================
// Edge cases of the port
// ===========================================================================

/// An unknown id, and an id of another context, are `AlreadyEnded` for
/// `abort` and `None` for `pair_state` — and the other context's pair is
/// not touched.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ids_unknown_to_a_context_are_already_ended_and_touch_nothing() {
    let device = SimForwarderDevice::new();
    let (first, second) = (device.open(), device.open());
    let (theirs, _vsock, tcp) = stream_pair(&second);

    let unknown = ForwardId::new(999_999).expect("non-zero");
    assert_eq!(first.abort(unknown).expect("abort"), AbortResult::AlreadyEnded);
    assert_eq!(first.pair_state(unknown).expect("pair_state"), None);
    assert_eq!(first.abort(theirs).expect("abort"), AbortResult::AlreadyEnded);
    assert_eq!(first.pair_state(theirs).expect("pair_state"), None);

    assert!(second.pair_state(theirs).expect("pair_state").is_some(), "still live in its context");
    assert_eq!(second.take_end().expect("take_end"), None);
    assert_eq!(tcp.peer_end(), None);
}

/// `take_end` returns each id's record once: every aborted id appears
/// exactly once, oldest first, and then the queue is empty; an id already
/// ended is `AlreadyEnded` on a second abort.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn every_record_is_taken_once_and_a_second_abort_is_already_ended() {
    let (_device, forwarder) = context();
    let ids: Vec<ForwardId> = (0..5).map(|_| stream_pair(&forwarder).0).collect();
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 5, "ids are never reused");

    for id in &ids {
        assert_eq!(forwarder.abort(*id).expect("abort"), AbortResult::Aborted);
    }
    let ends = take_all(&forwarder);
    assert_eq!(ends.iter().map(|e| e.id).collect::<Vec<_>>(), ids, "oldest first, once each");
    assert!(
        ends.iter().all(|e| e.outcome == ForwardOutcome::Aborted { cause: AbortCause::Detached })
    );
    assert_eq!(forwarder.take_end().expect("take_end"), None);
    assert_eq!(forwarder.abort(ids[0]).expect("abort"), AbortResult::AlreadyEnded);
}

/// `abort_all` aborts every live id of the context — pairs and watches —
/// returns how many, and queues each record before it returns.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn abort_all_queues_one_detached_record_per_live_id() {
    let (_device, forwarder) = context();
    let (pair, _v, _t) = stream_pair(&forwarder);
    let watched = SimSocket::vsock_stream();
    let watch = forwarder.watch(&watched).expect("watch");

    assert_eq!(forwarder.abort_all().expect("abort_all"), 2);
    assert_eq!(forwarder.inventory().expect("inventory").queued_ends, 2);
    let ids: BTreeSet<ForwardId> = take_all(&forwarder).iter().map(|e| e.id).collect();
    assert_eq!(ids, BTreeSet::from([pair, watch]));
    assert_eq!(forwarder.abort_all().expect("abort_all"), 0, "nothing left to abort");
}

/// The context's release aborts every live pair and watch — the TCP far end
/// is reset, the vsock halves and the watched vsock are shut `0x3` — leaves
/// every socket unhooked, queues no record, and leaves another context
/// alone.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_context_release_aborts_everything_it_holds_and_queues_no_record() {
    let device = SimForwarderDevice::new();
    let (released, kept) = (device.open(), device.open());
    let (_pair, vsock, tcp) = stream_pair(&released);
    let watched = SimSocket::vsock_stream();
    released.watch(&watched).expect("watch");
    let (kept_pair, _kv, kept_tcp) = stream_pair(&kept);

    released.release();

    assert_eq!(tcp.peer_end(), Some(SimFarEnd::Reset));
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Both));
    assert_eq!(watched.sent_shutdown(), Some(SimShutdown::Both), "release shuts a watched vsock");
    assert!(!vsock.is_hooked() && !tcp.is_hooked() && !watched.is_hooked());
    assert_eq!(
        device.contexts(),
        vec![
            SimContextState { open: false, live_pairs: 0, live_watches: 0, queued_ends: vec![] },
            SimContextState { open: true, live_pairs: 1, live_watches: 0, queued_ends: vec![] },
        ]
    );
    assert!(kept.pair_state(kept_pair).expect("pair_state").is_some());
    assert_eq!(kept.take_end().expect("take_end"), None, "no record reaches another context");
    assert_eq!(kept_tcp.peer_end(), None);
}

// ===========================================================================
// EVMOD-ATTACH-END and the owner-defect refusals
// ===========================================================================

/// A half that has ended is refused `HalfEnded` with its own cause — a TCP
/// half reset before attach (`ECONNRESET`, or `EPIPE` in `CLOSE_WAIT`), a
/// vsock half in `CLOSING`, a vsock half whose `sk_err` the sweep set — and
/// the refusal changes nothing: the other half can still be attached.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_ended_half_is_refused_half_ended_and_nothing_changes() {
    let (_device, forwarder) = context();

    let (vsock, reset_tcp) = (SimSocket::vsock_stream(), SimSocket::tcp());
    reset_tcp.peer_reset();
    assert!(matches!(
        forwarder.attach(&vsock, &reset_tcp, PairMode::Stream),
        Err(ForwarderError::HalfEnded { side: PairSide::B, errno }) if errno == KernelErrno::ECONNRESET
    ));
    assert!(!vsock.is_hooked(), "a refused attach hooks neither half");
    forwarder.attach(&vsock, &SimSocket::tcp(), PairMode::Stream).expect("the vsock is still free");

    let fin_then_reset = SimSocket::tcp();
    fin_then_reset.peer_shutdown(SimShutdown::Send);
    fin_then_reset.peer_reset();
    assert!(matches!(
        forwarder.attach(&SimSocket::vsock_stream(), &fin_then_reset, PairMode::Stream),
        Err(ForwarderError::HalfEnded { side: PairSide::B, errno }) if errno == epipe()
    ));

    let closing = SimSocket::vsock_stream();
    closing.set_vsock_state(SimVsockState::Closing);
    assert!(matches!(
        forwarder.attach(&closing, &SimSocket::tcp(), PairMode::Stream),
        Err(ForwarderError::HalfEnded { side: PairSide::A, errno }) if errno == KernelErrno::ECONNRESET
    ));

    let swept = SimSocket::vsock_stream();
    swept.set_sk_err(KernelErrno::ECONNRESET);
    assert!(matches!(
        forwarder.watch(&swept),
        Err(ForwarderError::HalfEnded { side: PairSide::A, errno }) if errno == KernelErrno::ECONNRESET
    ));
    let inventory = forwarder.inventory().expect("inventory");
    assert_eq!((inventory.live_pairs, inventory.live_watches, inventory.queued_ends), (1, 0, 0));
}

/// Ends present at attach that leave the half connected are admitted and
/// carried out by the first run: a TCP half in `CLOSE_WAIT` with bytes
/// queued forwards them then shuts the target; a vsock half whose peer sent
/// `0x3` with bytes still queued aborts in the first run after no byte.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ends_present_at_attach_are_carried_out_by_the_first_run() {
    let (_device, forwarder) = context();

    let (vsock, write_and_fin) = (SimSocket::vsock_stream(), SimSocket::tcp());
    write_and_fin.peer_send(b"last words");
    write_and_fin.peer_shutdown(SimShutdown::Send);
    let id =
        forwarder.attach(&vsock, &write_and_fin, PairMode::Stream).expect("CLOSE_WAIT is admitted");
    forwarder.run_until_idle();
    assert_eq!(vsock.peer_received(), b"last words");
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Send));
    let state = forwarder.pair_state(id).expect("pair_state").expect("still live");
    assert_eq!(state.b_to_a.source_end, Some(SourceEnd::Clean));

    let (aborted, tcp) = (SimSocket::vsock_stream(), SimSocket::tcp());
    aborted.peer_send(b"queued");
    aborted.peer_shutdown(SimShutdown::Both);
    let doomed =
        forwarder.attach(&aborted, &tcp, PairMode::Stream).expect("ESTABLISHED with peer bits");
    forwarder.run_until_idle();
    let end = take_all(&forwarder).into_iter().find(|e| e.id == doomed).expect("its record");
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert_eq!(tcp.peer_received(), b"", "after no byte");
    assert_eq!(tcp.peer_end(), Some(SimFarEnd::Reset));
}

/// The owner-defect refusals are typed and change nothing: two halves of
/// one kind, an unconnected half, a half whose own send this side shut, a
/// TCP half with a ULP, and a half already hooked by another context.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn owner_defects_are_refused_with_their_typed_cause() {
    let device = SimForwarderDevice::new();
    let (forwarder, other) = (device.open(), device.open());

    assert!(matches!(
        forwarder.attach(&SimSocket::tcp(), &SimSocket::tcp(), PairMode::Stream),
        Err(ForwarderError::UnsupportedSocket { .. })
    ));
    assert!(matches!(
        forwarder.attach(&SimSocket::vsock_stream(), &SimSocket::udp(), PairMode::Stream),
        Err(ForwarderError::UnsupportedSocket { side: PairSide::B })
    ));
    let unconnected = SimSocket::new(SimSocketKind::Tcp, SimSocketSetup::Unconnected);
    assert!(matches!(
        forwarder.attach(&SimSocket::vsock_stream(), &unconnected, PairMode::Stream),
        Err(ForwarderError::NotConnected { side: PairSide::B })
    ));
    let listening = SimSocket::new(SimSocketKind::VsockStream, SimSocketSetup::Listening);
    assert!(matches!(
        forwarder.watch(&listening),
        Err(ForwarderError::NotConnected { side: PairSide::A })
    ));
    let self_shut = SimSocket::vsock_stream();
    self_shut.shutdown_send();
    assert!(matches!(
        forwarder.attach(&self_shut, &SimSocket::tcp(), PairMode::Stream),
        Err(ForwarderError::NotConnected { side: PairSide::A })
    ));
    let tls = SimSocket::tcp();
    tls.set_ulp();
    assert!(matches!(
        forwarder.attach(&SimSocket::vsock_stream(), &tls, PairMode::Stream),
        Err(ForwarderError::UpperLayerProtocol { side: PairSide::B })
    ));
    let (_id, held_vsock, _t) = stream_pair(&other);
    assert!(matches!(
        forwarder.attach(&held_vsock, &SimSocket::tcp(), PairMode::Stream),
        Err(ForwarderError::AlreadyHooked { side: PairSide::A })
    ));
    assert!(matches!(
        forwarder.watch(&held_vsock),
        Err(ForwarderError::AlreadyHooked { side: PairSide::A })
    ));

    let inventory = forwarder.inventory().expect("inventory");
    assert_eq!((inventory.live_pairs, inventory.live_watches, inventory.attached), (0, 0, 0));
}

// ===========================================================================
// Watches (EVMOD-STREAM-END, WATCH-ABORT)
// ===========================================================================

/// A watch on a vsock whose guest direction already ended queues its record
/// at once: `0x2` → `GuestHalfClose`, `0x3` → `Aborted { PeerAbort }`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_watch_on_an_already_ended_vsock_queues_its_record_at_once() {
    let (_device, forwarder) = context();
    for (how, outcome) in [
        (SimShutdown::Send, ForwardOutcome::GuestHalfClose),
        (
            SimShutdown::Both,
            ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } },
        ),
    ] {
        let vsock = SimSocket::vsock_stream();
        vsock.peer_send(b"reply");
        vsock.peer_shutdown(how);
        let id = forwarder.watch(&vsock).expect("an ESTABLISHED vsock with peer bits is admitted");
        assert_eq!(
            forwarder.take_end().expect("take_end"),
            Some(ForwardEnd { id, outcome, moved_ab: 0, moved_ba: 0 }),
            "{how:?}"
        );
        assert_eq!(
            vsock.local_read(64),
            SimLocalRead::Bytes(b"reply".to_vec()),
            "the holder's bytes stay"
        );
    }
}

/// The watch's record is queued before the holder can read the end: on the
/// locked receive path (`0x2` while watched) and on the orphan sweep, where
/// the end is readable after `sk_err` is set and before `sk_error_report`
/// runs — the record is there once the report has run.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_watch_record_is_queued_by_the_callback_that_classifies_the_end() {
    let (_device, forwarder) = context();

    let half_closed = SimSocket::vsock_stream();
    let id = forwarder.watch(&half_closed).expect("watch");
    half_closed.peer_shutdown(SimShutdown::Send);
    assert_eq!(
        forwarder.take_end().expect("take_end"),
        Some(ForwardEnd { id, outcome: ForwardOutcome::GuestHalfClose, moved_ab: 0, moved_ba: 0 })
    );
    assert_eq!(half_closed.local_read(8), SimLocalRead::Eof);

    let swept = SimSocket::vsock_stream();
    let swept_id = forwarder.watch(&swept).expect("watch");
    swept.set_sk_err(KernelErrno::ECONNRESET);
    assert_eq!(swept.local_read(8), SimLocalRead::Err(KernelErrno::ECONNRESET), "readable first");
    assert_eq!(forwarder.take_end().expect("take_end"), None, "the record is in flight");
    swept.report_error();
    assert_eq!(
        forwarder.take_end().expect("take_end").map(|e| (e.id, e.outcome)),
        Some((
            swept_id,
            ForwardOutcome::Aborted {
                cause: AbortCause::SocketError {
                    side: PairSide::A,
                    errno: KernelErrno::ECONNRESET
                }
            }
        ))
    );
}

/// WATCH-ABORT: `abort` of a watch queues its `Detached` record, unhooks it,
/// and changes nothing on the socket — no shutdown sent, the state, `sk_err`
/// and the holder's own send direction as they were, the holder's queued
/// bytes still readable, the far end untouched.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn aborting_a_watch_changes_no_socket_fact() {
    let (_device, forwarder) = context();
    let vsock = SimSocket::vsock_stream();
    vsock.peer_send(b"unread");
    let id = forwarder.watch(&vsock).expect("watch");

    assert_eq!(forwarder.abort(id).expect("abort"), AbortResult::Aborted);
    assert_eq!(
        forwarder.take_end().expect("take_end"),
        Some(ForwardEnd {
            id,
            outcome: ForwardOutcome::Aborted { cause: AbortCause::Detached },
            moved_ab: 0,
            moved_ba: 0
        })
    );
    assert!(!vsock.is_hooked());
    assert_eq!(vsock.sent_shutdown(), None, "nothing sent on the vsock");
    assert_eq!(vsock.vsock_state(), SimVsockState::Established);
    assert_eq!(vsock.sk_err(), None);
    assert!(!vsock.own_send_shut());
    assert_eq!(vsock.peer_end(), None);
    assert_eq!(vsock.local_read(64), SimLocalRead::Bytes(b"unread".to_vec()));
    assert_eq!(forwarder.inventory().expect("inventory").live_watches, 0);
}

// ===========================================================================
// Datagram mode (EVMOD-DGRAM)
// ===========================================================================

/// Datagram-mode attach refuses a vsock half whose buffer is below the
/// largest frame (`BufferTooSmall`, naming the vsock half) and admits one at
/// exactly `MAX_GUEST_DATAGRAM_FRAME`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_datagram_vsock_half_below_the_largest_frame_is_buffer_too_small() {
    let (_device, forwarder) = context();
    let small = SimSocket::vsock_stream();
    small.set_buffer_size(MAX_GUEST_DATAGRAM_FRAME - 1);
    assert!(matches!(
        forwarder.attach(&SimSocket::udp(), &small, PairMode::Datagram { udp: PairSide::A }),
        Err(ForwarderError::BufferTooSmall { side: PairSide::B })
    ));
    assert!(!small.is_hooked());

    let exact = SimSocket::vsock_stream();
    exact.set_buffer_size(MAX_GUEST_DATAGRAM_FRAME);
    forwarder
        .attach(&exact, &SimSocket::udp(), PairMode::Datagram { udp: PairSide::B })
        .expect("a buffer of exactly the largest frame is admitted");
}

/// R5-51: datagrams from the UDP half are framed onto the vsock in order, an
/// empty one included; frames from the vsock become one datagram each on
/// the UDP half, empties included; a datagram above the bound is consumed
/// and counted `dgram_oversize`, never framed.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn datagrams_cross_in_order_with_empties_and_oversize_is_counted() {
    let (_device, forwarder) = context();
    let (udp, vsock) = (SimSocket::udp(), SimSocket::vsock_stream());
    let id =
        forwarder.attach(&udp, &vsock, PairMode::Datagram { udp: PairSide::A }).expect("attach");

    let oversize = vec![0u8; usize::try_from(MAX_GUEST_DATAGRAM_PAYLOAD).expect("fits") + 1];
    for datagram in [&b"one"[..], &b""[..], &oversize[..], &b"two"[..]] {
        udp.peer_send(datagram);
    }
    vsock.peer_send(&[frame(b"").as_slice(), frame(b"three").as_slice()].concat());
    forwarder.run_until_idle();

    assert_eq!(vsock.peer_received(), [frame(b"one"), frame(b""), frame(b"two")].concat());
    assert_eq!(udp.peer_datagrams(), vec![Vec::new(), b"three".to_vec()]);
    let inventory = forwarder.inventory().expect("inventory");
    assert_eq!(inventory.dgram_oversize, 1);
    let state = forwarder.pair_state(id).expect("pair_state").expect("live");
    assert_eq!(state.a_to_b.moved, 6, "payload bytes, not frame bytes");
}

/// A frame with a bad magic from the vsock half aborts the association
/// `FrameInvalid` on the vsock half; nothing of it reaches the UDP side.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_bad_frame_from_the_vsock_aborts_the_association_frame_invalid() {
    let (_device, forwarder) = context();
    let (udp, vsock) = (SimSocket::udp(), SimSocket::vsock_stream());
    let id =
        forwarder.attach(&udp, &vsock, PairMode::Datagram { udp: PairSide::A }).expect("attach");
    vsock.peer_send(b"XVD1\x00\x00\x00\x02hi");
    forwarder.run_until_idle();

    assert_eq!(
        take_all(&forwarder).iter().map(|e| (e.id, e.outcome)).collect::<Vec<_>>(),
        [(id, ForwardOutcome::Aborted { cause: AbortCause::FrameInvalid { side: PairSide::B } })]
    );
    assert!(udp.peer_datagrams().is_empty());
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Both));
}

// ===========================================================================
// Observable invariant — the inventory counts what is hooked
// ===========================================================================

#[derive(Debug, Clone)]
enum Op {
    AttachPair,
    Watch,
    Abort(usize),
    TakeEnd,
    AbortAll,
}

fn an_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::AttachPair),
        Just(Op::Watch),
        (0usize..8).prop_map(Op::Abort),
        Just(Op::TakeEnd),
        Just(Op::AbortAll),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// For every sequence of attaches, watches, aborts and record reads:
    /// `live_pairs + live_watches` equals the ids made minus the records
    /// queued or read, `queued_ends` equals the records queued and not yet
    /// read, and no id's record is ever read twice.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn the_inventory_counts_every_hooked_id_and_every_unread_record(
        ops in prop::collection::vec(an_op(), 0..=24),
    ) {
        let (_device, forwarder) = context();
        let mut made: Vec<ForwardId> = Vec::new();
        let mut ended = 0usize;
        let mut read: BTreeSet<ForwardId> = BTreeSet::new();
        #[allow(clippy::collection_is_never_read, reason = "the sockets are held so their pairs stay live")]
        let mut sockets = Vec::new();
        for op in ops {
            match op {
                Op::AttachPair => {
                    let (id, vsock, tcp) = stream_pair(&forwarder);
                    made.push(id);
                    sockets.push((vsock, tcp));
                }
                Op::Watch => {
                    let vsock = SimSocket::vsock_stream();
                    made.push(forwarder.watch(&vsock).expect("watch"));
                    sockets.push((vsock, SimSocket::tcp()));
                }
                Op::Abort(k) => {
                    if let Some(id) = made.get(k)
                        && forwarder.abort(*id).expect("abort") == AbortResult::Aborted
                    {
                        ended += 1;
                    }
                }
                Op::TakeEnd => {
                    if let Some(end) = forwarder.take_end().expect("take_end") {
                        prop_assert!(read.insert(end.id), "record of {:?} read twice", end.id);
                    }
                }
                Op::AbortAll => {
                    ended += usize::try_from(forwarder.abort_all().expect("abort_all")).expect("fits");
                }
            }
            let inventory = forwarder.inventory().expect("inventory");
            let live = usize::try_from(inventory.live_pairs + inventory.live_watches).expect("fits");
            prop_assert_eq!(live, made.len() - ended);
            prop_assert_eq!(usize::try_from(inventory.queued_ends).expect("fits"), ended - read.len());
        }
    }
}
