//! The guest UDP slot pool and datagram associations (feature-delta § *Guest
//! adaptation contract* intake model and "UDP slot release", § *Per-kind total
//! orders* "Datagram association", EVMOD-DGRAM, U-2, SLOT-ABORT, DG-5-A; R5-14
//! and R5-27, guest side).
//!
//! An association is one vsock STREAM (kind 2) whose pair (S, V_g) the owner
//! attaches on `Paired` (S, the UDP slot socket, half A; V_g half B); until
//! then datagrams wait in S's own receive queue. Every end of an association
//! aborts the pair (or closes V_g before `Paired`), discards S's receive queue
//! without reading payload, and disconnects S; a slot release then returns the
//! slot, which is never re-associated.

use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardOutcome, PairMode, PairSide,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, FLOW_PAIRING_DEADLINE, FlowRefusal, FlowRequest,
};
use overdrive_core::guest_transport::{GUEST_DATAGRAM_SLOT_IDLE, GUEST_OWNER_AUDIT_PERIOD};
use overdrive_sim::adapters::{SimForwarderDevice, SimShutdown, SimSocket};

use super::super::sim::SimGuestEffect;
use super::*;

/// The effects of the end-of-association order on `slot`, in log order.
fn end_order(
    effects: &[SimGuestEffect],
    slot: SocketAddrV4,
    pair: Option<ForwardId>,
) -> Vec<&'static str> {
    effects
        .iter()
        .filter_map(|e| match e {
            SimGuestEffect::AbortForward { id } if Some(*id) == pair => Some("abort"),
            SimGuestEffect::EndAssociation { slot: s } if *s == slot => Some("discard+disconnect"),
            SimGuestEffect::ReleaseSlot { slot: s } if *s == slot => Some("release"),
            _ => None,
        })
        .collect()
}

/// Datagram happy path: the first datagram claims a slot (the slot socket
/// queues it and becomes readable); its readiness makes the owner connect V_g
/// and write one `DatagramStream` request (guest origin, the original
/// destination, its token); on `Paired` it attaches (S, V_g), S half A; the
/// attach-time run forwards the queued datagram as one frame, and later
/// datagrams go straight through.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_first_datagram_claims_a_slot_associates_attaches_and_forwards() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).expect("a free slot");
    g.settle();
    let kernel = g.io.guest_socket_observation();
    assert_eq!(
        kernel.parked.get(&s),
        Some(&std::iter::once((cookie(1), dest(1))).collect()),
        "the real slot queue carries the claiming socket and destination"
    );
    assert!(kernel.readiness.contains(&s), "the claimed slot holds one readiness event");
    assert!(g.obs().slots[&s].owner.is_none(), "the owner has not received SlotReady yet");
    assert!(
        g.obs().slots[&s].association.is_none(),
        "no association before readiness is dispatched"
    );
    let a = g.associate(s);
    let requests = g.io.requests();
    assert_eq!(
        requests,
        vec![FlowRequest::DatagramStream { flow: a, destination: dest(1), launch: launch() }],
        "one request"
    );
    g.pair_slot(a);
    let attached = g.obs().slots[&s].attached.expect("attached on Paired");
    assert_eq!(g.io.vsock_side(a), Some(PairSide::B));
    g.io.run_forwarder();
    let vsock = g.io.slot_vsock(s).expect("the association's V_g");
    assert!(!vsock.peer_received().is_empty(), "the queued datagram is forwarded as a frame");
    g.io.app_send(cookie(1), dest(1));
    g.io.run_forwarder();
    assert!(
        g.io.forwarder_pair_state(attached).is_some_and(|p| p.a_to_b.moved >= 2),
        "the second datagram went straight through"
    );
    assert!(
        g.io.guest_socket_observation().readiness.is_empty(),
        "an attached slot gives no readiness"
    );
}

/// Slot identity: one slot per (application socket, original destination):
/// two sockets claim distinct slots, and one socket sending to N destinations
/// holds N slots (R5-14).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn slots_are_held_per_socket_and_destination() {
    let mut g = Guest::new();
    g.open_session();
    let a = g.io.app_send(cookie(1), dest(1)).unwrap();
    let b = g.io.app_send(cookie(2), dest(1)).unwrap();
    let c = g.io.app_send(cookie(1), dest(2)).unwrap();
    let again = g.io.app_send(cookie(1), dest(1)).unwrap();
    for s in [a, b, c] {
        g.io.release_slot_ready(s);
    }
    g.settle();
    assert_eq!(again, a, "the same (socket, destination) reuses its slot");
    let owned = owned_slots(&g.obs());
    assert_eq!(owned, [a, b, c].into_iter().collect(), "three distinct slots");
}

/// R5-14: a socket that sent to N destinations frees N slots when it closes:
/// on its `sock_release` report the owner releases every slot it owns, each in
/// the release order — abort the pair (attached) or close V_g (pending), then
/// consume an attached pair's end record, discard S's queue and disconnect,
/// then return the slot. The host sees each association end (its V_h reads
/// `0x3`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_socket_close_releases_every_slot_it_owns_in_the_release_order() {
    let mut g = Guest::new();
    g.open_session();
    let s1 = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a1 = g.associate(s1);
    g.pair_slot(a1);
    let p1 = g.obs().slots[&s1].attached;
    let s2 = g.io.app_send(cookie(1), dest(2)).unwrap();
    g.associate(s2);
    g.io.clear_effects();

    g.io.app_close_socket(cookie(1), true);
    g.io.release_socket_report(cookie(1));
    g.settle();
    let obs = g.obs();
    assert_eq!(
        (obs.slots[&s1].owner, obs.slots[&s1].association, obs.slots[&s1].attached),
        (Some((cookie(1), dest(1))), Some(a1), p1),
        "the attached slot remains resident until its end record is consumed"
    );
    assert!(obs.slots[&s2].owner.is_none(), "the pending slot is released immediately");
    let effects = g.io.effects();
    assert_eq!(end_order(&effects, s1, p1), ["abort"]);
    assert_eq!(end_order(&effects, s2, None), ["discard+disconnect", "release"]);
    assert!(
        g.io.vsock_ended().contains(
            &g.io
                .requests()
                .iter()
                .find_map(|r| match r {
                    FlowRequest::DatagramStream { flow, destination, .. }
                        if *destination == dest(2) =>
                        Some(*flow),
                    _ => None,
                })
                .unwrap()
        ),
        "the pending association's V_g is closed"
    );
    g.run_and_settle();
    let after_record = g.obs();
    assert!(
        after_record.slots[&s1].owner.is_none()
            && after_record.slots[&s1].association.is_none()
            && after_record.slots[&s1].attached.is_none(),
        "the dispatch after consuming the pair's end record releases the attached slot"
    );
    let effects = g.io.effects();
    assert_eq!(end_order(&effects, s1, p1), ["abort", "discard+disconnect", "release"]);
    assert_eq!(end_order(&effects, s2, None), ["discard+disconnect", "release"]);
}

/// K-B2 / R5-14: a lost `sock_release` report is recovered by the
/// level-triggered audit — the next audit tick's `live_cookies` finds the
/// owner's cookie gone and releases its slot.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_lost_release_report_is_recovered_by_the_audit_tick() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    g.settle();
    g.io.app_close_socket(cookie(1), false);
    g.advance(GUEST_OWNER_AUDIT_PERIOD);
    assert!(g.obs().slots[&s].owner.is_none(), "the audit tick released the orphan");
    assert!(g.io.effects().contains(&SimGuestEffect::LiveCookies));
}

/// DG-5-A / R5-14: an associated slot whose counters (both directions, the
/// pair's `moved_ab` / `moved_ba`) do not change for `GUEST_DATAGRAM_SLOT_IDLE`
/// is aborted by the first audit tick at or after its idle deadline (no timer
/// of its own), then released after its end record and the following owner
/// dispatch; a slot that keeps moving is not.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_idle_slot_is_released_by_the_audit_tick_after_the_idle_period() {
    let mut g = Guest::new();
    g.open_session();
    let idle = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a = g.associate(idle);
    g.pair_slot(a);
    let idle_pair = g.obs().slots[&idle].attached;
    let busy = g.io.app_send(cookie(2), dest(2)).unwrap();
    let b = g.associate(busy);
    g.pair_slot(b);
    g.io.run_forwarder();
    let ticks = GUEST_DATAGRAM_SLOT_IDLE.as_secs() / GUEST_OWNER_AUDIT_PERIOD.as_secs();
    for _ in 0..ticks - 1 {
        g.io.app_send(cookie(2), dest(2));
        g.io.run_forwarder();
        g.advance(GUEST_OWNER_AUDIT_PERIOD);
    }
    assert!(g.obs().slots[&idle].owner.is_some(), "not before the idle period");
    for _ in 0..3 {
        g.io.app_send(cookie(2), dest(2));
        g.io.run_forwarder();
        g.advance(GUEST_OWNER_AUDIT_PERIOD);
    }
    assert_eq!(
        (
            g.obs().slots[&idle].owner,
            g.obs().slots[&idle].association,
            g.obs().slots[&idle].attached,
        ),
        (Some((cookie(1), dest(1))), Some(a), idle_pair),
        "the first due audit aborts the pair but retains the slot until its end record"
    );
    assert_eq!(end_order(&g.io.effects(), idle, idle_pair), ["abort"]);
    assert!(g.obs().slots[&busy].owner.is_some(), "a moving slot stays");
    g.run_and_settle();
    assert!(
        g.obs().slots[&idle].owner.is_none(),
        "the post-record dispatch releases the idle slot"
    );
    assert_eq!(
        end_order(&g.io.effects(), idle, idle_pair),
        ["abort", "discard+disconnect", "release"]
    );
    assert!(g.obs().slots[&busy].owner.is_some(), "a moving slot still stays");
}

/// R5-14 / EVMOD-DGRAM: a slot released with a burst queued in its slot socket
/// (200 datagrams before `Paired`) has its queue discarded without reading
/// payload, starts no further association, and its next owner — another
/// socket to another destination — receives none of the old datagrams: its
/// association's first frame carries the new owner's datagram only.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_released_burst_is_discarded_and_the_next_owner_gets_none_of_it() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    g.associate(s);
    for _ in 0..199 {
        g.io.app_send(cookie(1), dest(1));
    }
    g.io.app_close_socket(cookie(1), true);
    g.io.release_socket_report(cookie(1));
    g.settle();
    let state = g.io.slot_state(s);
    assert_eq!(state.discarded, 200, "the whole queue discarded, unread");
    assert!(state.owner.is_none());
    assert!(!g.io.guest_socket_observation().parked.contains_key(&s));
    assert!(g.io.guest_socket_observation().readiness.is_empty(), "no further association");

    // the recycled slot's next owner
    let s2 = g.io.app_send(cookie(2), dest(2)).unwrap();
    assert_eq!(s2, s, "the slot is recycled");
    let a2 = g.associate(s2);
    g.pair_slot(a2);
    g.io.run_forwarder();
    let state = g.io.forwarder_pair_state(g.obs().slots[&s2].attached.unwrap()).unwrap();
    assert_eq!(state.a_to_b.moved, 1, "one datagram's payload: none of the previous owner's 200");
}

/// U-2 / R5-27: an association refused (`Policy`) discards S's queue without
/// reading payload, keeps the slot owned, and is not retried until the
/// application sends again; the next send starts exactly one new association.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_refused_association_discards_its_queue_and_retries_only_on_a_new_datagram() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a = g.associate(s);
    g.io.app_send(cookie(1), dest(1));
    g.io.host_control(ControlMessage::Refused { flow: a, refusal: FlowRefusal::Policy });
    g.settle();
    let obs = g.obs();
    assert_eq!(obs.slots[&s].owner, Some((cookie(1), dest(1))), "the slot is kept");
    assert!(obs.slots[&s].association.is_none());
    assert_eq!(g.io.slot_state(s).discarded, 2);
    assert!(
        g.io.guest_socket_observation().readiness.is_empty(),
        "no retry without a new datagram"
    );
    for _ in 0..3 {
        g.advance(GUEST_OWNER_AUDIT_PERIOD);
    }
    assert_eq!(g.io.requests().len(), 1, "still one request");
    g.io.app_send(cookie(1), dest(1));
    let a2 = g.associate(s);
    assert_ne!(a2, a, "a new association id");
    assert_eq!(g.io.requests().len(), 2);
}

/// EVMOD-ATTACH-END / SLOT-ATTACH-REFUSAL-SHAPE: V_g ends after the host's
/// `Paired` is in flight but before the guest forwarder attaches `(S, V_g)`.
/// The owner receives V_g back from the typed datagram refusal, counts and
/// closes it, then discards and disconnects resident S without releasing its
/// owner. The refusing guest context creates no pair or end record and sends
/// no control message; V_g's close follows the already-attached host pair's
/// ordinary one-record path. A later datagram starts a fresh association.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_v_g_ending_during_paired_refuses_the_attach_and_the_slot_reassociates() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).expect("a free resident slot");
    let first = g.associate(s);

    // The host has already attached V_h to its outer half before it sends
    // Paired. This context observes the normal end record produced when the
    // guest's close of V_g reaches V_h as 0x3.
    let host_device = SimForwarderDevice::new();
    let host = host_device.open();
    let (v_h, host_outer) = (SimSocket::vsock_stream(), SimSocket::tcp());
    let host_pair = host.attach(&v_h, &host_outer, PairMode::Stream).expect("host pair");

    let v_g = g.io.slot_vsock(s).expect("the pending association's V_g");
    v_g.peer_reset();
    let before = g.obs();
    g.io.clear_effects();
    g.io.host_control(ControlMessage::Paired { flow: first });
    g.settle();

    let after = g.obs();
    assert_eq!(
        after.forwarder_attach_ended,
        before.forwarder_attach_ended + 1,
        "the V_g HalfEnded refusal is counted once"
    );
    assert_eq!(
        after.slots[&s].owner,
        Some((cookie(1), dest(1))),
        "resident S keeps its application/destination owner"
    );
    assert!(
        after.slots[&s].association.is_none() && after.slots[&s].attached.is_none(),
        "the refused association and guest pair are absent"
    );
    let slot_state = g.io.slot_state(s);
    assert_eq!(slot_state.discarded, 1, "S's queued datagram is discarded");
    assert_eq!(slot_state.ended, 1, "the association end order runs once");
    assert!(!slot_state.connected, "resident S is disconnected");
    assert!(g.io.vsock_ended().contains(&first), "the returned V_g is closed");
    let effects = g.io.effects();
    let refused = position(
        &effects,
        |effect| matches!(effect, SimGuestEffect::AttachSlot { slot, accepted: None } if *slot == s),
    )
    .expect("the datagram attach returns its typed refusal");
    let close = position(
        &effects,
        |effect| matches!(effect, SimGuestEffect::Close { flow: Some(flow), .. } if *flow == first),
    )
    .expect("the exact returned V_g is closed");
    let end = position(
        &effects,
        |effect| matches!(effect, SimGuestEffect::EndAssociation { slot } if *slot == s),
    )
    .expect("resident S runs its end-of-association order");
    assert!(refused < close && close < end, "return V_g, close it, then end resident S");
    assert!(
        effects.iter().all(|effect| !matches!(effect, SimGuestEffect::AbortForward { .. })),
        "no guest pair id exists to abort"
    );
    assert_eq!(
        g.io.forwarder_inventory().map(|inventory| (inventory.attached, inventory.live_pairs)),
        Some((0, 0)),
        "the refusing guest forwarder allocates no pair id"
    );
    assert!(
        g.io.forwarder_contexts().iter().all(|context| context.queued_ends.is_empty()),
        "the refusing guest forwarder queues no end record"
    );
    assert!(g.io.outbox().is_empty(), "the guest writes no control message");

    v_h.peer_shutdown(SimShutdown::Both);
    host.run_until_idle();
    let host_end = host.take_end().expect("host take_end").expect("one host end record");
    assert_eq!(host_end.id, host_pair);
    assert_eq!(
        host_end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert!(host.take_end().expect("host take_end").is_none(), "exactly one host record");

    assert_eq!(
        g.io.app_send(cookie(1), dest(1)),
        Some(s),
        "the next datagram reuses the retained resident slot"
    );
    let second = g.associate(s);
    assert_ne!(second, first, "the later datagram starts a fresh association");
    assert_eq!(g.obs().slots[&s].association, Some(second));
}

/// EVMOD-DGRAM: the host's `Abort` for a live association aborts its pair,
/// discards S's queue, disconnects S and keeps the slot owned; the next
/// datagram re-associates.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_abort_of_a_live_association_aborts_discards_disconnects_and_keeps_the_slot() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a = g.associate(s);
    g.pair_slot(a);
    let pair = g.obs().slots[&s].attached;
    g.io.clear_effects();
    g.io.host_control(ControlMessage::Abort { flow: a });
    g.settle();
    assert_eq!(end_order(&g.io.effects(), s, pair), ["abort"]);
    let obs = g.obs();
    assert_eq!(obs.slots[&s].owner, Some((cookie(1), dest(1))));
    assert_eq!(
        (obs.slots[&s].association, obs.slots[&s].attached),
        (Some(a), pair),
        "the association remains resident until the pair's end record is consumed"
    );
    assert!(g.io.slot_state(s).connected, "S stays connected until the deferred owner close");
    g.run_and_settle();
    assert_eq!(
        end_order(&g.io.effects(), s, pair),
        ["abort", "discard+disconnect"],
        "the slot is kept after its record-driven association end"
    );
    let obs = g.obs();
    assert_eq!(obs.slots[&s].owner, Some((cookie(1), dest(1))));
    assert!(obs.slots[&s].association.is_none() && obs.slots[&s].attached.is_none());
    assert!(!g.io.slot_state(s).connected, "S disconnects after its record is consumed");
    g.io.app_send(cookie(1), dest(1));
    g.associate(s);
}

/// The host's abort reaching the guest pair as a peer abort (`0x3` with a
/// direction open) ends the association through its end record the same way
/// — discard, disconnect, slot kept — and the host's later `Abort` for it is
/// discarded.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_peer_abort_record_ends_the_association_and_a_late_abort_is_discarded() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a = g.associate(s);
    g.pair_slot(a);
    g.io.slot_vsock(s).unwrap().peer_shutdown(SimShutdown::Both);
    g.run_and_settle();
    assert!(g.obs().slots[&s].association.is_none());
    assert_eq!(g.obs().slots[&s].owner, Some((cookie(1), dest(1))));
    assert_eq!(g.io.slot_state(s).ended, 1);
    let before = g.obs();
    g.io.host_control(ControlMessage::Abort { flow: a });
    g.settle();
    assert_eq!(g.obs(), before);
    assert_eq!(g.io.slot_state(s).ended, 1, "the end order ran once");
}

/// `FLOW_PAIRING_DEADLINE`: the guest ends an unpaired association itself —
/// closes V_g, writes `Abort`, discards and disconnects S, keeps the slot; the
/// host's late `Paired` is discarded without a second `Abort` (U-5).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_pairing_deadline_aborts_and_a_late_paired_is_discarded() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    let started = g.now();
    let a = g.associate(s);
    g.advance_to(started + FLOW_PAIRING_DEADLINE);
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: a }]);
    let obs = g.obs();
    assert!(obs.slots[&s].association.is_none());
    assert!(obs.slots[&s].owner.is_some(), "the slot is kept");
    g.io.host_control(ControlMessage::Paired { flow: a });
    g.settle();
    assert!(g.io.outbox().is_empty(), "no second Abort");
    assert!(g.obs().slots[&s].attached.is_none());
}

/// SLOT-ABORT: the socket closes and its slot is released before the host's
/// `Paired` arrives; the late `Paired` is answered with `Abort` and attached
/// nowhere.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_late_paired_for_a_released_slot_is_answered_with_abort() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    let a = g.associate(s);
    g.io.app_close_socket(cookie(1), true);
    g.io.release_socket_report(cookie(1));
    g.settle();
    g.io.host_control(ControlMessage::Paired { flow: a });
    g.settle();
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: a }]);
    assert!(g.obs().slots.values().all(|x| x.attached.is_none()));
    assert_eq!(g.io.forwarder_inventory().map(|i| i.attached), Some(0));
}

/// R5-27 (guest side): a destination refused every time — each later send
/// causes at most one association attempt per pairing round trip, also under
/// a 500-send burst: while an attempt is pending no further request is
/// written; each `Refused` discards the queue; only the next datagram after
/// it starts the next attempt.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_refused_destination_gets_at_most_one_attempt_per_round_trip_under_a_burst() {
    let mut g = Guest::new();
    g.open_session();
    let s = g.io.app_send(cookie(1), dest(1)).unwrap();
    for round in 1..=3 {
        let a = g.associate(s);
        for _ in 0..500 {
            g.io.app_send(cookie(1), dest(1));
        }
        g.settle();
        assert_eq!(g.io.requests().len(), round, "round {round}: one attempt while it is pending");
        g.io.host_control(ControlMessage::Refused { flow: a, refusal: FlowRefusal::Policy });
        g.settle();
        assert!(
            !g.io.guest_socket_observation().parked.contains_key(&s),
            "round {round}: the queue is discarded"
        );
        assert!(
            g.io.guest_socket_observation().readiness.is_empty(),
            "round {round}: no retry without a new datagram"
        );
        g.io.app_send(cookie(1), dest(1));
    }
    assert_eq!(g.obs().slots[&s].owner, Some((cookie(1), dest(1))), "the slot is kept throughout");
}
