//! Step 01-02 (GH #266, ADR-0084 §4, Piece A) — the `spawn_convergence_loop`
//! next-wake table drives resync via `broker.submit` once per period.
//!
//! These are Tier-1, default-lane tests: they drive the pure cadence
//! decision helpers (`build_cadence_table` / `arm_next_wake` /
//! `due_resync_evaluations`) the convergence loop composes, and feed the
//! produced evaluations into a REAL [`EvaluationBroker`] to get the
//! "in-broker count == k" teeth (C-A1). No `Clock`, no `run_server`, no
//! real infrastructure — logical `UnixInstant` values stand in for the
//! `SimClock` the loop reads (`SimClock` advances `now`/`unix_now` in
//! lockstep, so the helper's `now: UnixInstant` is exactly what the loop
//! passes). The walking-skeleton production boot of the loop is step 02-02.
//!
//! Scenarios (docs/feature/reconciler-framework-improvements): S-266-02
//! (k periods → exactly k routed submits), S-266-03 (once-per-period, never
//! per-tick — the `<=`/`<` and `+= period` mutation killers), S-266-04
//! (default `None` → no cadence entry, zero resync-origin submits), and
//! S-266-05 (distinct periods fire independently + the structural no-hardcode
//! scan). S-266-19 (broker resync-key collapse) is co-located in
//! `overdrive-core/src/eval_broker.rs`.

use std::collections::BTreeMap;
use std::time::Duration;

use overdrive_control_plane::{arm_next_wake, build_cadence_table, due_resync_evaluations};
use overdrive_core::UnixInstant;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{ReconcilerName, ResyncSchedule, ResyncScope, TargetResource};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Small helpers — these tests are the only caller site.
// ---------------------------------------------------------------------------

fn rname(raw: &str) -> ReconcilerName {
    ReconcilerName::new(raw).expect("valid ReconcilerName")
}

fn node(raw: &str) -> NodeId {
    NodeId::new(raw).expect("valid NodeId")
}

/// The single target `LocalNode` resolves to for node `n` — mirrors the
/// core `resolve_scope(LocalNode, n) = [ node/<n> ]` (01-01).
fn node_target(n: &NodeId) -> TargetResource {
    TargetResource::new(&format!("node/{}", n.as_str())).expect("valid node TargetResource")
}

/// An arbitrary, deterministic epoch base — the absolute value is
/// irrelevant (the cadence decision is relative arithmetic), so a fixed
/// `UnixInstant` stands in for a `SimClock` reading with zero
/// non-determinism.
const fn base() -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(1_000_000))
}

const fn local_schedule(period: Duration) -> ResyncSchedule {
    ResyncSchedule { period, scope: ResyncScope::LocalNode }
}

/// Valid `NodeId` strategy: leading lowercase letter, alphanumeric tail —
/// always a valid label, so `node/<id>` is always a canonical target.
fn arb_node_id() -> impl Strategy<Value = NodeId> {
    "[a-z][a-z0-9]{0,8}".prop_map(|raw| node(&raw))
}

// ---------------------------------------------------------------------------
// S-266-04 — the default `resync_schedule() = None` builds NO cadence entry,
// and an empty cadence table yields ZERO resync-origin submits over many
// periods (SD-6: None ⟺ host-backed ⟺ resync-only-off).
// ---------------------------------------------------------------------------

#[test]
fn s_266_04_default_none_reconciler_builds_no_cadence_entry() {
    // `noop_heartbeat` is a real production reconciler that returns the
    // default `resync_schedule() = None`. It must contribute no entry.
    let noop = overdrive_control_plane::noop_heartbeat();
    let table = build_cadence_table(std::iter::once(&noop));
    assert!(
        table.is_empty(),
        "a reconciler returning the default None resync_schedule builds no cadence entry",
    );
}

#[test]
fn s_266_04_empty_cadence_table_never_submits_over_many_periods() {
    let n = node("nzero");
    let schedules: BTreeMap<ReconcilerName, ResyncSchedule> = BTreeMap::new();
    let mut next_wake = arm_next_wake(&schedules, base());
    // arm over an empty table => empty next-wake table.
    assert!(next_wake.is_empty(), "no schedules => no next-wake entries");

    // assert_always: no (R, *) submit whose origin is the cadence path.
    for step in 0..1_000u32 {
        let now = base() + Duration::from_secs(u64::from(step));
        let evals = due_resync_evaluations(&schedules, &mut next_wake, now, &n);
        assert!(evals.is_empty(), "empty cadence table must never emit a resync submit");
    }
}

// ---------------------------------------------------------------------------
// S-266-02 — R declares Some { period P, LocalNode }; loop owns NodeId n.
// After the clock advances k whole periods with no row changes, the broker
// receives EXACTLY k submits of (R, node/n) — one per period, each routed
// through `broker.submit` (C-A1). In-broker count teeth: a side-channel
// bypass makes dispatched < k; a per-tick re-arm makes dispatched > k; only
// routing every resync through `broker.submit` exactly once per period
// yields k. proptest over P, k, n.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn s_266_02_k_periods_yield_exactly_k_broker_routed_submits(
        period_secs in 1u64..=120,
        k in 1u32..=8,
        n in arb_node_id(),
    ) {
        let r = rname("cadence-r");
        let period = Duration::from_secs(period_secs);
        let mut schedules = BTreeMap::new();
        schedules.insert(r.clone(), local_schedule(period));

        let t0 = base();
        let mut next_wake = arm_next_wake(&schedules, t0); // { r: t0 + period }
        let mut broker = EvaluationBroker::new();
        let expected_target = node_target(&n);

        // Mirror the real loop: each period-boundary tick runs the cadence
        // submit phase (route through broker.submit) THEN drains + dispatches.
        for i in 1..=k {
            let now = t0 + period * i;
            let evals = due_resync_evaluations(&schedules, &mut next_wake, now, &n);
            for e in &evals {
                prop_assert_eq!(&e.reconciler, &r);
                prop_assert_eq!(&e.target, &expected_target);
                broker.submit(e.clone());
            }
            let drained = broker.drain_pending();
            prop_assert_eq!(drained.len(), 1, "exactly one resync survives per period");
            prop_assert_eq!(&drained[0].reconciler, &r);
            prop_assert_eq!(&drained[0].target, &expected_target);
        }

        // In-broker count (C-A1 teeth): exactly k routed through the broker.
        let counters = broker.counters();
        prop_assert_eq!(counters.dispatched, u64::from(k), "exactly k submits reached the broker");
        prop_assert_eq!(counters.cancelled, 0, "no same-key collapse across drained periods");
    }
}

// ---------------------------------------------------------------------------
// S-266-03 — R armed at t0 with period P (next_wake = t0+P): the clock at
// t0+P−ε submits NOTHING; at t0+P submits EXACTLY one; within the next period
// (t0+P+ε) submits NOTHING; at t0+2P one more. Never one-per-tick within a
// period. Mutation targets: the `next_wake <= now` decision (swap `<=`↔`<`)
// and the `next_wake += period` re-arm (drop it) → both caught. C-A2 /
// no-storm boundary. proptest over P, n.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn s_266_03_fires_once_per_period_never_per_tick(
        period_secs in 3u64..=3_600,
        n in arb_node_id(),
    ) {
        let r = rname("cadence-r");
        let period = Duration::from_secs(period_secs);
        let eps = Duration::from_secs(1); // sub-period (period >= 3 > 1)
        let mut schedules = BTreeMap::new();
        schedules.insert(r.clone(), local_schedule(period));

        let t0 = base();
        let mut next_wake = arm_next_wake(&schedules, t0); // { r: t0+P }
        let expected_target = node_target(&n);

        // Sub-period tick sequence across two periods. `UnixInstant` supports
        // `+ Duration` only, so a "P − ε" instant is `t0 + (P − ε)`.
        // `UnixInstant` supports `+ Duration` only; a "P − ε" instant is
        // `t0 + (P − ε)`. `checked_sub` per clippy::unchecked_time_subtraction
        // (period >= 3s > eps = 1s, so the subtraction never underflows).
        let before_p = period.checked_sub(eps).expect("period > eps");
        let before_2p = (period * 2).checked_sub(eps).expect("2*period > eps");
        let ticks: [(UnixInstant, usize); 5] = [
            (t0 + before_p, 0),       // before the boundary  -> no fire
            (t0 + period, 1),         // exact boundary       -> one (kills `<=`->`<`)
            (t0 + (period + eps), 0), // within next period   -> no fire (kills drop-re-arm)
            (t0 + before_2p, 0),      // still within period  -> no fire
            (t0 + period * 2, 1),     // next boundary        -> one
        ];

        for (now, want) in ticks {
            let evals = due_resync_evaluations(&schedules, &mut next_wake, now, &n);
            prop_assert_eq!(evals.len(), want, "fire count at this tick");
            for e in &evals {
                prop_assert_eq!(&e.reconciler, &r);
                prop_assert_eq!(&e.target, &expected_target);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// S-266-05 — X = Some { 10s, LocalNode }, Y = Some { 30s, LocalNode }, local
// NodeId n. Over 60s with no row changes X receives 6 submits of (X, node/n)
// and Y receives 2 of (Y, node/n), each driven purely from its declaration.
// ---------------------------------------------------------------------------

#[test]
fn s_266_05_distinct_periods_fire_independently_over_60s() {
    let n = node("nfive");
    let x = rname("cadence-x");
    let y = rname("cadence-y");
    let mut schedules = BTreeMap::new();
    schedules.insert(x.clone(), local_schedule(Duration::from_secs(10)));
    schedules.insert(y.clone(), local_schedule(Duration::from_secs(30)));

    let t0 = base();
    let mut next_wake = arm_next_wake(&schedules, t0); // { x: t0+10, y: t0+30 }
    let mut broker = EvaluationBroker::new();
    let target = node_target(&n);

    let mut x_count = 0u64;
    let mut y_count = 0u64;

    // Fine 1s ticking across [t0+1s, t0+60s].
    for sec in 1..=60u32 {
        let now = t0 + Duration::from_secs(u64::from(sec));
        for e in due_resync_evaluations(&schedules, &mut next_wake, now, &n) {
            broker.submit(e);
        }
        for e in broker.drain_pending() {
            assert_eq!(e.target, target, "every resync fires against node/n");
            if e.reconciler == x {
                x_count += 1;
            } else if e.reconciler == y {
                y_count += 1;
            } else {
                panic!("unexpected reconciler in cadence stream: {}", e.reconciler);
            }
        }
    }

    assert_eq!(x_count, 6, "X (10s period) fires 6 times over 60s");
    assert_eq!(y_count, 2, "Y (30s period) fires 2 times over 60s");
}

// ---------------------------------------------------------------------------
// S-266-05 companion (structural, CM-style) — the convergence loop + cadence
// machinery names NO reconciler and NO cadence constant. Mirrors
// `eval_broker_does_not_import_clock_transport_entropy`: a source scan is a
// belt-and-braces check that a refactor cannot reintroduce the incoming
// `VM_RECLAMATION_SWEEP_INTERVAL` + `node/<id>` hardcode the ADR pre-empts.
// ---------------------------------------------------------------------------

#[test]
fn loop_names_no_reconciler_or_cadence_constant() {
    let src = include_str!("../../src/lib.rs");

    let start = src
        .find("[cadence-loop-region-start]")
        .expect("cadence-loop region start sentinel present");
    let end =
        src.find("[cadence-loop-region-end]").expect("cadence-loop region end sentinel present");
    assert!(start < end, "region sentinels are ordered");
    let region = &src[start..end];

    // The loop resolves scope generically — it names no specific reconciler.
    for name in [
        "noop-heartbeat",
        "workload-lifecycle",
        "workflow-lifecycle",
        "service-map-hydrator",
        "backend-discovery-bridge",
        "service-lifecycle",
        "svid-lifecycle",
    ] {
        assert!(!region.contains(name), "cadence-loop region must not name reconciler `{name}`");
    }

    // No hardcoded cadence constant and no hardcoded target scheme — the
    // loop carries only the generic table + the core `resolve_scope`.
    assert!(
        !region.contains("VM_RECLAMATION"),
        "cadence-loop region must not carry a cadence constant",
    );
    assert!(
        !region.contains("node/"),
        "cadence-loop region must not hardcode a target scheme (that is resolve_scope's job)",
    );

    // Whole-file belt-and-braces: the pre-empted hardcode never reaches lib.rs.
    assert!(
        !src.contains("VM_RECLAMATION_SWEEP_INTERVAL"),
        "no cadence/target hardcode reaches the convergence loop",
    );
}
