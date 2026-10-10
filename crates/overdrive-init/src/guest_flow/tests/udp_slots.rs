//! P3 guest-side UDP slots (D25, CELL-RECREATE, U-2, SLOT-ABORT, U-5; the
//! guest rows of `udp_slots`): slot identity (P3-S2), idle release and the
//! lost-report audit (P3-S4), cell recreation, generation dispatch and
//! quarantine (P3-S5), keep-slot after `Refused` / `Abort` / pairing deadline
//! (P3-S6), late `Paired` for a released slot answered with `Abort` (P3-S9
//! guest half), and exhaustion leaving existing associations unaffected
//! (P3-S10). The guest kernel (capture program, slot map, cells) is the sim's;
//! the owner's decisions are observed through QPROJ-5 and the sim's QPROJ-7
//! surface.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::time::Duration;

use overdrive_core::guest_transport::wire::{ControlMessage, FLOW_PAIRING_DEADLINE, FlowRefusal};
use overdrive_core::guest_transport::{GUEST_DATAGRAM_SLOT_IDLE, GUEST_UDP_SLOTS};

use super::super::sim::{SimGuestEffect, sim_cookie, sim_generation};
use super::{Guest, dest, owned_slots, position};
use crate::guest_flow::HostChannel;

fn associations_started(g: &Guest) -> usize {
    g.io.effects().iter().filter(|e| matches!(e, SimGuestEffect::WriteSlotRequests { .. })).count()
}

/// P3-S2: one socket sending first datagrams to N distinct destinations holds
/// exactly N slots, each owned by a distinct (socket, destination); a repeat
/// send to an owned destination claims nothing and starts no association.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn one_slot_per_socket_destination_pair() {
    for n in [0u16, 1, 2, GUEST_UDP_SLOTS] {
        let mut g = Guest::new();
        g.open_session();
        let cookie = sim_cookie(42);
        for i in 0..n {
            let s = g.io.app_send(cookie, dest(i)).expect("a free slot");
            g.io.release_slot_ready(s, sim_generation(0));
        }
        g.run_released();
        let obs = g.obs();
        let owned = owned_slots(&obs);
        assert_eq!(owned.len(), usize::from(n), "N = {n}");
        let tags: BTreeSet<_> = owned.iter().map(|s| obs.slots[s].owner).collect();
        assert_eq!(
            tags.len(),
            usize::from(n),
            "every owned slot has a distinct (socket, destination)"
        );
        assert!(tags.iter().all(|t| t.is_some_and(|(k, _)| k == cookie)));
        assert_eq!(g.connects(HostChannel::FlowStream), usize::from(n), "one association per slot");

        if n > 0 {
            let s = g.io.app_send(cookie, dest(0)).expect("owned destination");
            assert_eq!(obs.slots[&s].owner, Some((cookie, dest(0))));
            g.io.release_slot_ready(s, sim_generation(0));
            g.run_released();
            assert_eq!(owned_slots(&g.obs()), owned, "no new slot on a repeat send");
            assert_eq!(
                g.connects(HostChannel::FlowStream),
                usize::from(n),
                "no second association"
            );
        }
    }
}

/// P3-S4: an association idle in both directions for
/// `GUEST_DATAGRAM_SLOT_IDLE` ends in the end-of-association order and its
/// slot returns to the pool (the host observes the legs' end); a slot with
/// traffic in the window is kept.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn an_idle_association_is_released_and_a_busy_one_kept() {
    let mut g = Guest::new();
    g.open_session();
    let (idle_k, busy_k) = (sim_cookie(7), sim_cookie(8));
    let idle = g.io.app_send(idle_k, dest(1)).expect("slot");
    let idle_flow = g.associate(idle);
    g.pair_slot(idle_flow);
    let busy = g.io.app_send(busy_k, dest(2)).expect("slot");
    let busy_flow = g.associate(busy);
    g.pair_slot(busy_flow);
    let _ = g.io.take_outbox();

    let step = Duration::from_secs(30);
    for _ in 0..10 {
        g.io.host_datagram(busy);
        g.io.app_send(busy_k, dest(2)).expect("owned");
        g.advance(step);
    }
    assert!(step * 10 >= GUEST_DATAGRAM_SLOT_IDLE * 2);
    let obs = g.obs();
    assert_eq!(obs.slots[&idle].owner, None, "the idle slot is released");
    assert_eq!(obs.slots[&idle].association, None);
    assert!(!obs.slots[&idle].routed);
    assert!(g.io.slot_state(idle).in_pool);
    assert!(
        g.io.guest_socket_observation().legs_closed_unobserved.contains(&idle_flow),
        "the host observes the end"
    );

    assert_eq!(obs.slots[&busy].owner, Some((busy_k, dest(2))), "the busy slot is kept");
    assert_eq!(obs.slots[&busy].association, Some(busy_flow));
    assert!(obs.slots[&busy].routed);
}

/// P3-S4 / K-B2: a socket released with its `sock_release` report lost is found
/// by the level-triggered audit (its cookie no longer exists); every slot of
/// it is released — not only one — even while host traffic keeps them busy.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_lost_release_report_is_recovered_by_the_audit() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(9);
    let a = g.io.app_send(k, dest(1)).expect("slot");
    let fa = g.associate(a);
    g.pair_slot(fa);
    let b = g.io.app_send(k, dest(2)).expect("slot");
    let fb = g.associate(b);
    g.pair_slot(fb);
    assert_eq!(owned_slots(&g.obs()), BTreeSet::from([a, b]));

    g.io.app_close_socket(k, false); // the report is lost
    assert!(g.io.guest_socket_observation().release_reports.is_empty());
    for _ in 0..4 {
        // host traffic keeps both slots out of the idle path: only the audit frees them
        g.io.host_datagram(a);
        g.io.host_datagram(b);
        g.advance(Duration::from_secs(30));
        if owned_slots(&g.obs()).is_empty() {
            break;
        }
    }
    assert!(owned_slots(&g.obs()).is_empty(), "both slots of the released socket reclaimed");
    assert!(g.io.slot_state(a).in_pool && g.io.slot_state(b).in_pool);
    let ended = g.io.guest_socket_observation().legs_closed_unobserved;
    assert!(ended.contains(&fa) && ended.contains(&fb));
}

/// Release through a delivered report: every slot of the socket ends and
/// returns to the pool.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_release_report_releases_every_slot_of_the_socket() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(10);
    let other = sim_cookie(11);
    let slots: Vec<_> = (0..3).map(|i| g.io.app_send(k, dest(i)).expect("slot")).collect();
    for s in &slots {
        let _ = g.associate(*s);
    }
    let kept = g.io.app_send(other, dest(9)).expect("slot");
    let kept_flow = g.associate(kept);
    g.io.app_close_socket(k, true);
    g.io.release_socket_report(k);
    g.run_released();
    let obs = g.obs();
    for s in &slots {
        assert_eq!(obs.slots[s].owner, None, "{s} released");
        assert!(g.io.slot_state(*s).in_pool);
    }
    assert_eq!(
        obs.slots[&kept].association,
        Some(kept_flow),
        "another socket's slot is unaffected"
    );
}

/// P3-S5 (CELL-RECREATE): ending an association (here by `Refused`) recreates
/// both cells — the parked burst is discarded, the generation advances — and
/// a readiness event registered for the earlier generation never starts an
/// association; a current-generation event (a new datagram) does.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn recreated_cells_discard_parked_frames_and_stale_generation_events() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(12);
    let s = g.io.app_send(k, dest(1)).expect("slot");
    let f = g.associate(s);
    for _ in 0..200 {
        g.io.app_send(k, dest(1)).expect("owned slot");
    }
    let parked = g.io.guest_socket_observation();
    assert_eq!(parked.parked.get(&s), Some(&BTreeSet::from([(k, dest(1))])));
    assert!(parked.readiness.contains(&(s, sim_generation(0))), "the burst re-armed generation 0");
    let started = associations_started(&g);

    g.io.host_control(ControlMessage::Refused { flow: f, refusal: FlowRefusal::Policy });
    g.run_released();
    let obs = g.obs();
    assert_eq!(obs.slots[&s].generation, sim_generation(1), "cells recreated");
    assert_eq!(obs.slots[&s].association, None);
    assert_eq!(obs.slots[&s].owner, Some((k, dest(1))), "U-2: the slot stays owned");
    let sim = g.io.guest_socket_observation();
    assert!(!sim.parked.contains_key(&s), "parked frames discarded");
    assert!(sim.legs_closed_unobserved.contains(&f), "T_g and Q_g closed");

    // The stale (generation 0) event is dispatched: discarded.
    g.io.release_slot_ready(s, sim_generation(0));
    g.run_released();
    assert_eq!(associations_started(&g), started, "no association from an earlier generation");
    assert_eq!(g.connects(HostChannel::FlowStream), 0);
    assert_eq!(g.obs().slots[&s].association, None);

    // A new datagram registers generation 1 and starts exactly one association.
    g.io.app_send(k, dest(1)).expect("owned");
    let _ = g.associate(s);
    assert_eq!(associations_started(&g), started + 1);
}

/// P3-S5: a slot whose new cell cannot be created is quarantined — no
/// association, detached from its (socket, destination), not in the pool;
/// the next send to that destination takes a free slot; the audit's retry
/// returns it to the pool.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_cell_creation_failure_quarantines_the_slot_until_the_audit_retry() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(13);
    let s = g.io.app_send(k, dest(1)).expect("slot");
    let f = g.associate(s);
    g.io.fail_cell_creation(s);
    g.io.host_control(ControlMessage::Abort { flow: f });
    g.run_released();
    let obs = g.obs();
    assert_eq!(obs.slots[&s].owner, None, "detached from its owner");
    assert_eq!(obs.slots[&s].association, None);
    let state = g.io.slot_state(s);
    assert!(state.quarantined && !state.in_pool);
    let next = g.io.app_send(k, dest(1)).expect("a free slot");
    assert_ne!(next, s, "the next send takes another slot");

    g.io.fail_next(super::super::sim::SimGuestOp::RetryQuarantined, ErrorKind::OutOfMemory);
    g.advance(Duration::from_secs(30));
    for _ in 0..4 {
        if g.io.slot_state(s).in_pool {
            break;
        }
        g.advance(Duration::from_secs(30));
    }
    assert!(g.io.slot_state(s).in_pool, "a later audit retry succeeds");
    let retries =
        g.io.effects()
            .iter()
            .filter(|e| matches!(e, SimGuestEffect::RetryQuarantined { slot } if *slot == s))
            .count();
    assert!(retries >= 2, "the failed retry was retried at a later audit tick");
}

/// P3-S6 (U-2): a destination refused every attempt costs at most one pairing
/// attempt per application send — a parked burst never starts an association
/// after `Refused`; the slot stays owned.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_refused_burst_keeps_the_slot_and_starts_one_association_per_send() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(14);
    let s = g.io.app_send(k, dest(1)).expect("slot");
    for _ in 0..499 {
        g.io.app_send(k, dest(1)).expect("owned");
    }
    for round in 0..3 {
        let f = g.associate(s);
        g.io.host_control(ControlMessage::Refused { flow: f, refusal: FlowRefusal::HostInternal });
        g.run_released();
        assert_eq!(g.obs().slots[&s].owner, Some((k, dest(1))), "round {round}: slot kept");
        assert!(
            !g.io.guest_socket_observation().parked.contains_key(&s),
            "round {round}: backlog discarded"
        );
        assert!(
            g.io.guest_socket_observation()
                .readiness
                .iter()
                .all(|(sl, generation)| *sl != s || *generation < g.obs().slots[&s].generation),
            "round {round}: no current-generation readiness without a new datagram"
        );
        assert_eq!(associations_started(&g), round + 1);
        g.io.app_send(k, dest(1)).expect("the application sends again");
    }
}

/// P3-S6: a connected socket's association aborted after `Paired`
/// re-associates on its next datagram; with no new datagram it does not.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn an_aborted_paired_association_reassociates_only_on_the_next_datagram() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(15);
    let s = g.io.app_send(k, dest(1)).expect("slot");
    let f = g.associate(s);
    g.pair_slot(f);
    assert!(g.obs().slots[&s].routed);
    g.io.host_control(ControlMessage::Abort { flow: f });
    g.run_released();
    assert_eq!(g.obs().slots[&s].association, None);
    assert_eq!(g.obs().slots[&s].owner, Some((k, dest(1))));
    assert_eq!(g.connects(HostChannel::FlowStream), 0, "no self-retry");
    assert!(
        g.io.take_outbox().iter().all(|m| *m != ControlMessage::Abort { flow: f }),
        "Abort is not answered"
    );

    g.io.app_send(k, dest(1)).expect("next datagram");
    let f2 = g.associate(s);
    assert_ne!(f2, f, "a new association with a new flow id");
}

/// P3-S6 / U-2: the guest opener's `FLOW_PAIRING_DEADLINE` ends an unpaired
/// association: `Abort` sent, frames discarded, slot kept.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn the_pairing_deadline_aborts_the_association_and_keeps_the_slot() {
    let mut g = Guest::new();
    g.open_session();
    let _ = g.io.take_outbox();
    let k = sim_cookie(16);
    let s = g.io.app_send(k, dest(1)).expect("slot");
    let f = g.associate(s);
    g.advance(FLOW_PAIRING_DEADLINE);
    assert!(g.io.take_outbox().contains(&ControlMessage::Abort { flow: f }));
    let obs = g.obs();
    assert_eq!(obs.slots[&s].association, None);
    assert_eq!(obs.slots[&s].owner, Some((k, dest(1))));
    assert!(!g.io.guest_socket_observation().parked.contains_key(&s));
}

/// P3-S9 guest half (SLOT-ABORT): a late `Paired` for a released slot's
/// association is answered with `Abort` and routes nothing; a `Paired` for a
/// live association of the same socket is routed; a late `Refused` / `Abort`
/// for the released association is discarded with no reply (U-5).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn a_late_paired_for_a_released_slot_is_answered_with_abort() {
    let mut g = Guest::new();
    g.open_session();
    let k = sim_cookie(17);
    let released = g.io.app_send(k, dest(1)).expect("slot");
    let late = g.associate(released);
    let live_k = sim_cookie(18);
    let live = g.io.app_send(live_k, dest(1)).expect("slot");
    let live_flow = g.associate(live);
    g.io.app_close_socket(k, true);
    g.io.release_socket_report(k);
    g.run_released();
    assert_eq!(g.obs().slots[&released].owner, None);
    let _ = g.io.take_outbox();
    g.io.clear_effects();

    g.io.host_control(ControlMessage::Paired { flow: late });
    g.run_released();
    assert_eq!(g.io.take_outbox(), vec![ControlMessage::Abort { flow: late }]);
    assert!(
        position(&g.io.effects(), |e| matches!(e, SimGuestEffect::RouteSlot { .. })).is_none(),
        "routes nothing"
    );

    for message in [
        ControlMessage::Refused { flow: late, refusal: FlowRefusal::Policy },
        ControlMessage::Abort { flow: late },
    ] {
        g.io.host_control(message);
        g.run_released();
        assert!(g.io.take_outbox().is_empty(), "{message:?} discarded, no reply");
    }

    g.pair_slot(live_flow);
    assert!(g.obs().slots[&live].routed, "a live association is routed normally");
    assert!(
        position(
            &g.io.effects(),
            |e| matches!(e, SimGuestEffect::RouteSlot { slot } if *slot == live)
        )
        .is_some()
    );
}

/// P3-S10 (D25 exhaustion, host part): with every slot owned the capture
/// program refuses a new destination (`EPERM`); existing associations are
/// unaffected; once the owner releases a socket's slot, a new destination
/// claims it.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn exhaustion_refuses_a_new_destination_and_leaves_existing_associations() {
    let mut g = Guest::new();
    g.open_session();
    let a = sim_cookie(30);
    let b = sim_cookie(31);
    let first = g.io.app_send(a, dest(0)).expect("slot");
    let fa = g.associate(first);
    for i in 1..GUEST_UDP_SLOTS {
        g.io.app_send(b, dest(i)).expect("slot");
    }
    let before = g.obs();
    assert_eq!(
        g.io.app_send(b, dest(60_000)),
        Err(ErrorKind::PermissionDenied),
        "the 1,025th destination"
    );
    assert_eq!(g.io.exhausted_sends(), 1);
    assert_eq!(g.obs(), before, "existing associations unaffected");
    assert_eq!(before.slots[&first].association, Some(fa));

    g.io.app_close_socket(a, true);
    g.io.release_socket_report(a);
    g.run_released();
    assert_eq!(g.io.app_send(b, dest(60_000)), Ok(first), "the released slot is claimable again");
}
