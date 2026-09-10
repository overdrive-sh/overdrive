//! Step 04-02 — `EvaluationBroker` collapses duplicate evaluations at
//! the same `(ReconcilerName, TargetResource)` key into a cancelable
//! set bounded by the reaper.
//!
//! Per ADR-0013 §8 and whitepaper §18, a second submit at an
//! already-pending key moves the prior evaluation into the cancelable
//! vec (LWW) and increments the `cancelled` counter. `drain_pending`
//! empties pending into the runtime's dispatch path; `reap_cancelable`
//! empties the cancelable vec in bulk.
//!
//! This step delivers the data-structure primitive. The in-runtime
//! reaper tick cadence (N=16) lands at 04-04 when the runtime
//! assembly comes together.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Small helpers — kept inline; these tests are the only caller site.
// ---------------------------------------------------------------------------

fn rname(raw: &str) -> ReconcilerName {
    ReconcilerName::new(raw).expect("valid ReconcilerName")
}

fn tres(raw: &str) -> TargetResource {
    TargetResource::new(raw).expect("valid TargetResource")
}

fn eval_for(r: &str, t: &str) -> Evaluation {
    Evaluation { reconciler: rname(r), target: tres(t) }
}

// ---------------------------------------------------------------------------
// (a) Fresh broker starts with all counters at zero and empty pending.
// ---------------------------------------------------------------------------

#[test]
fn new_broker_has_zero_counters_and_empty_pending() {
    let broker = EvaluationBroker::new();
    let counters = broker.counters();
    assert_eq!(counters.queued, 0, "new broker queued must be 0");
    assert_eq!(counters.cancelled, 0, "new broker cancelled must be 0");
    assert_eq!(counters.dispatched, 0, "new broker dispatched must be 0");
}

// ---------------------------------------------------------------------------
// (b) A first submit populates pending and raises queued to 1.
// ---------------------------------------------------------------------------

#[test]
fn single_submit_increments_queued_and_populates_pending() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    let counters = broker.counters();
    assert_eq!(counters.queued, 1, "after 1 submit queued must be 1");
    assert_eq!(counters.cancelled, 0, "no duplicate yet -> cancelled stays 0");
    assert_eq!(counters.dispatched, 0, "nothing drained -> dispatched stays 0");
}

// ---------------------------------------------------------------------------
// (c) Duplicate submit at the same key moves prior to cancelable; the
//     pending entry is replaced (LWW), queued still reflects 1.
// ---------------------------------------------------------------------------

#[test]
fn duplicate_submit_at_same_key_collapses_to_one_pending_with_one_cancelled() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    let counters = broker.counters();
    assert_eq!(counters.queued, 1, "still exactly one pending at the key");
    assert_eq!(counters.cancelled, 1, "prior moved to cancelable");
    assert_eq!(counters.dispatched, 0, "nothing drained yet");
}

// ---------------------------------------------------------------------------
// (d) Third submit at the same key yields two cancelled; pending stays 1.
// ---------------------------------------------------------------------------

#[test]
fn triple_submit_at_same_key_yields_two_cancelled() {
    let mut broker = EvaluationBroker::new();
    for _ in 0..3 {
        broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    }
    let counters = broker.counters();
    assert_eq!(counters.queued, 1);
    assert_eq!(counters.cancelled, 2);
    assert_eq!(counters.dispatched, 0);
}

// ---------------------------------------------------------------------------
// (e) Submits at distinct keys do not collapse -- each occupies pending.
// ---------------------------------------------------------------------------

#[test]
fn submits_at_distinct_keys_dont_collapse() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    broker.submit(eval_for("noop-heartbeat", "workload/frontend"), std::time::Instant::now());
    let counters = broker.counters();
    assert_eq!(counters.queued, 2, "two distinct targets -> two pending");
    assert_eq!(counters.cancelled, 0, "no duplicate key -> cancelled stays 0");
}

#[test]
fn submits_with_same_target_different_reconciler_dont_collapse() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    broker.submit(eval_for("cert-rotator", "workload/payments"), std::time::Instant::now());
    let counters = broker.counters();
    assert_eq!(counters.queued, 2, "distinct reconciler dimension -> two pending");
    assert_eq!(counters.cancelled, 0);
}

// ---------------------------------------------------------------------------
// (f) drain_pending returns surviving evaluations and empties pending;
//     dispatched increments by drained count.
// ---------------------------------------------------------------------------

#[test]
fn drain_pending_returns_surviving_evaluations_and_empties_pending() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    let drained = broker.drain_pending(
        usize::MAX,
        &std::collections::BTreeSet::new(),
        std::time::Instant::now(),
    );
    assert_eq!(drained.len(), 1, "one surviving evaluation after collapse");
    let counters = broker.counters();
    assert_eq!(counters.queued, 0, "pending must be empty after drain");
    assert_eq!(counters.dispatched, 1, "dispatched tracks drained count");
    assert_eq!(counters.cancelled, 1, "cancelled counter is not reset by drain");
}

#[test]
fn drain_on_empty_broker_returns_empty_vec() {
    let mut broker = EvaluationBroker::new();
    let drained = broker.drain_pending(
        usize::MAX,
        &std::collections::BTreeSet::new(),
        std::time::Instant::now(),
    );
    assert!(drained.is_empty());
    assert_eq!(broker.counters().dispatched, 0);
}

// ---------------------------------------------------------------------------
// (g) reap_cancelable returns count reclaimed and empties cancelable vec.
// ---------------------------------------------------------------------------

#[test]
fn reap_cancelable_returns_reclaimed_count_and_empties_cancelable() {
    let mut broker = EvaluationBroker::new();
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    let reaped = broker.reap_cancelable();
    assert_eq!(reaped, 1, "exactly one evaluation was cancelled -> one reaped");
    // Reaping again yields zero; the vec has been emptied.
    assert_eq!(broker.reap_cancelable(), 0);
}

#[test]
fn reap_on_empty_cancelable_returns_zero() {
    let mut broker = EvaluationBroker::new();
    assert_eq!(broker.reap_cancelable(), 0);
}

// ---------------------------------------------------------------------------
// (h) cancelable.len() bounded by submissions-at-same-key between reaps.
//     Exercised via reap's return value (the only observable length).
// ---------------------------------------------------------------------------

#[test]
fn cancelable_growth_matches_duplicate_submits_since_last_reap() {
    let mut broker = EvaluationBroker::new();
    // Five submits at one key => four cancelled (first still pending).
    for _ in 0..5 {
        broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    }
    assert_eq!(broker.reap_cancelable(), 4);
    // Three more at the same key; one still pending, so two more cancelled.
    for _ in 0..3 {
        broker.submit(eval_for("noop-heartbeat", "workload/payments"), std::time::Instant::now());
    }
    // Only the THREE new submits contribute; the previous pending
    // evaluation (still in pending from the first batch) is the one
    // replaced by the first of these three, so two of the three new ones
    // get cancelled and one stays pending.
    assert_eq!(broker.reap_cancelable(), 3);
}

// ---------------------------------------------------------------------------
// (i) Proptest -- arbitrary interleave of submits across a small key
//     space, followed by target-exclusive admission rounds and reap,
//     satisfies the broker identity:
//       eventually dispatched == distinct_keys_seen
//       cancelled  == total_submits - distinct_keys_seen
//       cancelable.len() == 0
//
// Case count uses proptest's default (1024); the broker is pure and
// single-threaded so the run is still fast.
// ---------------------------------------------------------------------------

fn arb_key_index() -> impl Strategy<Value = usize> {
    // 1..=5 distinct (r, t) keys -> index into a fixed table.
    0usize..5
}

fn arb_submit_count() -> impl Strategy<Value = usize> {
    // 1..=50 submissions per trial.
    1usize..=50
}

proptest! {
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn arbitrary_interleave_satisfies_invariants(
        n in arb_submit_count(),
        indices in proptest::collection::vec(arb_key_index(), 1..=50),
    ) {
        // Trial-sized slice of indices.
        let ops: Vec<usize> = indices.into_iter().take(n).collect();
        if ops.is_empty() { return Ok(()); }

        // Fixed five-key table -- the proptest index picks one.
        let keys: [(&str, &str); 5] = [
            ("noop-heartbeat", "workload/payments"),
            ("noop-heartbeat", "workload/frontend"),
            ("cert-rotator",   "workload/payments"),
            ("scheduler",      "node/n-001"),
            ("right-sizer",    "alloc/a-42"),
        ];

        let submitted_at = Instant::now();
        let admitted_at = submitted_at + Duration::from_millis(1);
        let mut broker = EvaluationBroker::new();
        let mut key_counts: HashMap<(String, String), usize> = HashMap::new();

        for idx in &ops {
            let (r, t) = keys[*idx];
            *key_counts.entry((r.to_string(), t.to_string())).or_insert(0) += 1;
            broker.submit(eval_for(r, t), submitted_at);
        }

        let total: usize = ops.len();
        let distinct: usize = key_counts.len();
        let distinct_targets: usize = key_counts
            .keys()
            .map(|(_, target)| target)
            .collect::<HashSet<_>>()
            .len();

        // After all submits, pre-drain: cancelled == total - distinct,
        // queued == distinct, dispatched == 0.
        let c = broker.counters();
        prop_assert_eq!(c.queued, distinct as u64, "queued must equal distinct keys");
        prop_assert_eq!(c.cancelled, (total - distinct) as u64, "cancelled == total - distinct");
        prop_assert_eq!(c.dispatched, 0);

        // One admission result contains at most one evaluation per target,
        // even when distinct reconciler names have pending work for that
        // target. With no externally blocked targets and an unlimited limit,
        // it admits exactly one representative for every pending target.
        let first = broker.drain_pending(
            usize::MAX,
            &std::collections::BTreeSet::new(),
            admitted_at,
        );
        let first_targets: HashSet<_> =
            first.iter().map(|(evaluation, _)| evaluation.target.clone()).collect();
        prop_assert_eq!(first.len(), distinct_targets, "one admission per distinct target");
        prop_assert_eq!(first_targets.len(), first.len(), "targets are exclusive within a batch");
        prop_assert_eq!(broker.counters().queued, (distinct - distinct_targets) as u64);

        // Releasing those target leases permits every deferred distinct key
        // to be admitted in a later round; same-target exclusion does not
        // discard or coalesce a different reconciler's pending evaluation.
        let mut admitted_keys: HashSet<(String, String)> = first
            .iter()
            .map(|(evaluation, _)| {
                (evaluation.reconciler.as_str().to_owned(), evaluation.target.as_str().to_owned())
            })
            .collect();
        for _ in 0..distinct {
            if broker.counters().queued == 0 {
                break;
            }
            let batch = broker.drain_pending(
                usize::MAX,
                &std::collections::BTreeSet::new(),
                admitted_at,
            );
            prop_assert!(!batch.is_empty(), "an eligible pending key makes progress");
            let batch_targets: HashSet<_> =
                batch.iter().map(|(evaluation, _)| evaluation.target.clone()).collect();
            prop_assert_eq!(batch_targets.len(), batch.len(), "targets remain exclusive per batch");
            for (evaluation, _) in batch {
                prop_assert!(admitted_keys.insert((
                    evaluation.reconciler.as_str().to_owned(),
                    evaluation.target.as_str().to_owned(),
                )), "a pending key is admitted exactly once");
            }
        }
        let c = broker.counters();
        prop_assert_eq!(c.queued, 0, "finite eligible pending work is eventually admitted");
        prop_assert_eq!(c.dispatched, distinct as u64);
        prop_assert_eq!(admitted_keys.len(), distinct);

        // Reap: returns cancelled count; cancelable empties.
        let reaped = broker.reap_cancelable();
        prop_assert_eq!(reaped, total - distinct, "reap returns prior cancelled count");
        prop_assert_eq!(broker.reap_cancelable(), 0, "second reap is zero -- vec emptied");
    }

    // ---------------------------------------------------------------------
    // (j) Oracle: after every submit, pending is structurally keyed --
    // this is enforced by HashMap but the proptest is a sanity check that
    // the broker is not double-inserting elsewhere (e.g. a hypothetical
    // second pending container).
    // ---------------------------------------------------------------------
    #[test]
    fn duplicate_evaluations_collapse_invariant_holds_after_every_submit(
        indices in proptest::collection::vec(arb_key_index(), 1..=50),
    ) {
        let keys: [(&str, &str); 5] = [
            ("noop-heartbeat", "workload/payments"),
            ("noop-heartbeat", "workload/frontend"),
            ("cert-rotator",   "workload/payments"),
            ("scheduler",      "node/n-001"),
            ("right-sizer",    "alloc/a-42"),
        ];
        let mut broker = EvaluationBroker::new();
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for idx in indices {
            let (r, t) = keys[idx];
            seen.insert((r.to_string(), t.to_string()));
            broker.submit(eval_for(r, t), std::time::Instant::now());
            // queued always equals the count of distinct keys seen so far.
            prop_assert_eq!(broker.counters().queued, seen.len() as u64);
        }
    }
}

// ---------------------------------------------------------------------------
// (k) Defence-in-depth: ADR-0102 requires std::time's value types as
// explicit inputs, while the broker policy must not read a clock or pull
// rng / tokio::net onto its compile path. dst-lint enforces this at the
// crate level; this grep is a file-local belt-and-braces check.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn eval_broker_does_not_import_clock_transport_entropy() {
    let src = include_str!("../../../overdrive-core/src/eval_broker.rs");
    let production = src.split_once("#[cfg(test)]").map_or(src, |(production, _)| production);
    assert!(
        production.contains("use std::time::{Duration, Instant};"),
        "ADR-0102 requires std::time Instant/Duration as explicit broker inputs"
    );
    assert!(!production.contains("SystemTime"), "eval_broker policy must not use SystemTime");
    assert!(!production.contains("Instant::now"), "eval_broker policy must not read the clock");
    assert!(!production.contains("rand::"), "eval_broker policy must not import rand");
    assert!(!production.contains("tokio::net"), "eval_broker policy must not import tokio::net");
    assert!(!production.contains("std::net::"), "eval_broker policy must not import std::net");
}

// ---------------------------------------------------------------------------
// (l) Regression for fix-eval-broker-drain-determinism, transitioned to
// ADR-0102. Identical submission order and explicit timestamps must produce
// identical FIFO admissions and residence durations. A second reconciler
// pending for an already-admitted target stays pending until the next round.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn drain_pending_is_deterministic_across_two_brokers() {
    // Canonical five-key fixture lifted from
    // `arbitrary_interleave_satisfies_invariants` above. Each key is
    // submitted exactly once, in a fixed order, into both brokers.
    let keys: [(&str, &str); 5] = [
        ("noop-heartbeat", "workload/payments"),
        ("noop-heartbeat", "workload/frontend"),
        ("cert-rotator", "workload/payments"),
        ("scheduler", "node/n-001"),
        ("right-sizer", "alloc/a-42"),
    ];

    let submitted_at = Instant::now();
    let admitted_at = submitted_at + Duration::from_secs(1);
    let expected_first = vec![
        (eval_for("noop-heartbeat", "workload/payments"), Duration::from_secs(1)),
        (eval_for("noop-heartbeat", "workload/frontend"), Duration::from_secs(1)),
        (eval_for("scheduler", "node/n-001"), Duration::from_secs(1)),
        (eval_for("right-sizer", "alloc/a-42"), Duration::from_secs(1)),
    ];
    let expected_deferred =
        vec![(eval_for("cert-rotator", "workload/payments"), Duration::from_secs(1))];

    for iteration in 0..16 {
        let mut a = EvaluationBroker::new();
        let mut b = EvaluationBroker::new();
        for (r, t) in &keys {
            a.submit(eval_for(r, t), submitted_at);
            b.submit(eval_for(r, t), submitted_at);
        }
        let drained_a =
            a.drain_pending(usize::MAX, &std::collections::BTreeSet::new(), admitted_at);
        let drained_b =
            b.drain_pending(usize::MAX, &std::collections::BTreeSet::new(), admitted_at);
        assert_eq!(
            drained_a, drained_b,
            "broker drain order must be deterministic across instances (iteration {iteration})",
        );
        assert_eq!(drained_a, expected_first, "FIFO order, residence age, and target exclusion");
        assert_eq!(a.counters().queued, 1, "same-target work remains pending");
        assert_eq!(b.counters().queued, 1, "same-target work remains pending");

        let deferred_a =
            a.drain_pending(usize::MAX, &std::collections::BTreeSet::new(), admitted_at);
        let deferred_b =
            b.drain_pending(usize::MAX, &std::collections::BTreeSet::new(), admitted_at);
        assert_eq!(deferred_a, deferred_b, "deferred round is deterministic");
        assert_eq!(deferred_a, expected_deferred, "the deferred same-target key is not lost");
    }
}
