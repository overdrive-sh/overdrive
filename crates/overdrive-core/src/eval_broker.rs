//! `EvaluationBroker` with cancelable-eval-set semantics per whitepaper §18.
//!
//! Keyed on `(ReconcilerName, TargetResource)` — a second submit at the
//! same key moves the prior evaluation into the cancelable set (LWW).
//! The reaper empties the cancelable set in bulk on a fixed tick
//! cadence. The storm-proofing guarantee from ADR-0013 §8 is this
//! broker's reason for existing: 60 000 redundant evaluations from a
//! single flap collapse to one dispatch per distinct target.
//!
//! Phase 1 is single-threaded — the broker is owned by the runtime
//! event loop and mutated through `&mut self`. No `Arc`, no `Mutex`,
//! no `async`. The HA Phase 2 path wraps this struct behind the
//! runtime's actor surface without changing the broker's own
//! contract.
//!
//! By construction this module contains no clock / transport / entropy
//! access; the acceptance test
//! `eval_broker_does_not_import_clock_transport_entropy` enforces that
//! structurally.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::reconcilers::{ReconcilerName, TargetResource};

/// Per-broker counter snapshot rendered by `cluster status` and the
/// ADR-0017 storm-proofing invariant.
///
/// `queued` is the current pending size (a snapshot); `cancelled` and
/// `dispatched` are monotonically increasing accumulators across the
/// broker's lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BrokerCounters {
    /// Number of evaluations currently pending dispatch — equal to the
    /// number of distinct `(ReconcilerName, TargetResource)` keys in
    /// the pending map at the moment the snapshot was taken.
    pub queued: u64,
    /// Cumulative count of evaluations that were superseded at their
    /// key and moved to the cancelable vec. Not reset by `drain_pending`
    /// or `reap_cancelable` — reset only by constructing a new broker.
    pub cancelled: u64,
    /// Cumulative count of evaluations that have been drained into the
    /// dispatch path. Increments by `drained.len()` per `drain_pending`.
    pub dispatched: u64,
}

/// One evaluation routed through the broker.
///
/// Equality / hashing is delegated to the embedded identifiers so the
/// broker's key-collapse logic operates on canonical name + target
/// rather than on the `Evaluation` value as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub reconciler: ReconcilerName,
    pub target: TargetResource,
}

/// The cancelable-eval-set evaluation broker.
#[derive(Debug, Default)]
pub struct EvaluationBroker {
    /// Current pending evaluations, keyed on
    /// `(ReconcilerName, TargetResource)`. A second submit at the same
    /// key evicts the prior value into `cancelable`.
    pending: BTreeMap<(ReconcilerName, TargetResource), PendingEvaluation>,
    /// Evaluations that were superseded at their key, awaiting bulk
    /// reap by the runtime reaper tick.
    cancelable: Vec<Evaluation>,
    /// Accumulator counters. `queued` in the snapshot is computed from
    /// `pending.len()` at `counters()` call time; the struct field
    /// tracks only the accumulators.
    cancelled: u64,
    dispatched: u64,
    next_fifo: u64,
}

#[derive(Debug)]
struct PendingEvaluation {
    evaluation: Evaluation,
    first_pending_at: Instant,
    fifo: u64,
}

impl EvaluationBroker {
    /// Construct a fresh, empty broker. All counters start at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit an evaluation. If an evaluation is already pending at the
    /// same `(ReconcilerName, TargetResource)` key, the prior value is
    /// moved to the cancelable vec (LWW) and `cancelled` is incremented
    /// by one. A first submit at a fresh key simply populates `pending`.
    pub fn submit(&mut self, eval: Evaluation, now: Instant) {
        let key = (eval.reconciler.clone(), eval.target.clone());
        if let Some(prev) = self.pending.get_mut(&key) {
            self.cancelable.push(prev.evaluation.clone());
            prev.evaluation = eval;
            self.cancelled = self.cancelled.saturating_add(1);
            return;
        }
        let fifo = self.next_fifo;
        self.next_fifo = self.next_fifo.saturating_add(1);
        self.pending
            .insert(key, PendingEvaluation { evaluation: eval, first_pending_at: now, fifo });
    }

    /// Empty up to `limit` eligible pending evaluations into the runtime's
    /// dispatch path, oldest first. A target can appear at most once in one
    /// admission result; blocked targets remain pending for a later turn.
    pub fn drain_pending(
        &mut self,
        limit: usize,
        blocked_targets: &BTreeSet<TargetResource>,
        now: Instant,
    ) -> Vec<(Evaluation, Duration)> {
        if limit == 0 || self.pending.is_empty() {
            return Vec::new();
        }

        let mut candidates: Vec<_> =
            self.pending.iter().map(|(key, value)| (value.fifo, key.clone())).collect();
        candidates.sort_by_key(|(fifo, _)| *fifo);

        let mut blocked = blocked_targets.clone();
        let mut drained = Vec::new();
        for (_, key) in candidates {
            if drained.len() == limit || blocked.contains(&key.1) {
                continue;
            }
            let Some(value) = self.pending.remove(&key) else {
                continue;
            };
            let queued_for = now.saturating_duration_since(value.first_pending_at);
            blocked.insert(value.evaluation.target.clone());
            drained.push((value.evaluation, queued_for));
        }
        self.dispatched = self.dispatched.saturating_add(drained.len() as u64);
        drained
    }

    /// Empty the cancelable vec in bulk. Returns the number of
    /// evaluations reclaimed. Counters are not adjusted — `cancelled`
    /// has already been bumped at submit time; this only reclaims the
    /// storage.
    pub fn reap_cancelable(&mut self) -> usize {
        let n = self.cancelable.len();
        self.cancelable.clear();
        n
    }

    /// Current counter snapshot. `queued` is taken from `pending.len()`
    /// at call time; `cancelled` / `dispatched` are the broker's own
    /// accumulators.
    #[must_use]
    pub fn counters(&self) -> BrokerCounters {
        BrokerCounters {
            queued: self.pending.len() as u64,
            cancelled: self.cancelled,
            dispatched: self.dispatched,
        }
    }
}

// ---------------------------------------------------------------------------
// S-266-19 (GH #266, ADR-0084 §4) — resync-on-resync same-key collapse. A
// resync submitted through `broker.submit` (C-A1) at a key that already has a
// pending eval MUST coalesce through the LWW key-collapse to ≤1 pending at
// that key, bumping `cancelled` by exactly one. This is the no-resync-storm
// guarantee Open Question 5 rests on, exercised at the `(R, node/n)` resync
// key. Co-located so the `-p overdrive-core --file eval_broker.rs` mutation
// run has a same-crate killer for the LWW collapse (the
// `if let Some(prev) = pending.insert(..)` + `cancelled += 1`).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod resync_collapse_tests {
    use std::time::Instant;

    use proptest::prelude::*;

    use super::{Evaluation, EvaluationBroker};
    use crate::reconcilers::{ReconcilerName, TargetResource};

    /// A resync evaluation at the canonical `(R, node/n)` key.
    fn resync_eval(node_raw: &str) -> Evaluation {
        Evaluation {
            reconciler: ReconcilerName::new("cadence-r").expect("valid ReconcilerName"),
            target: TargetResource::new(&format!("node/{node_raw}"))
                .expect("valid node TargetResource"),
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    /// A prior eval already pending at `(R, node/n)`; a redundant same-key
    /// resync submit while it is still pending collapses to EXACTLY one
    /// pending and bumps `cancelled` by EXACTLY one — never fewer (missed
    /// collapse) and never more (double-count).
    #[test]
    fn redundant_same_key_resync_collapses_to_one_pending_and_bumps_cancelled_by_one() {
        let mut broker = EvaluationBroker::new();
        let eval = resync_eval("n");

        broker.submit(eval.clone(), Instant::now());
        let before = broker.counters();
        assert_eq!(before.queued, 1, "prior resync is pending at (R, node/n)");
        assert_eq!(before.cancelled, 0, "no collapse yet");

        broker.submit(eval, Instant::now());
        let after = broker.counters();
        assert_eq!(after.queued, 1, "≤1 pending at (R, node/n) after redundant resync");
        assert_eq!(after.cancelled, 1, "cancelled bumps by exactly one");
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        /// For m ≥ 1 redundant same-key resync submits at `(R, node/n)`:
        /// `pending` holds EXACTLY one entry at that key (assert_always ≤1)
        /// and `cancelled == m - 1`. Kills a dropped/skipped LWW collapse
        /// (would leave >1 pending or under-count `cancelled`) and a
        /// double-count (would over-count `cancelled`).
        #[test]
        fn same_key_resync_burst_keeps_at_most_one_pending(
            node_raw in "[a-z][a-z0-9]{0,8}",
            m in 1u64..=64,
        ) {
            let mut broker = EvaluationBroker::new();
            for _ in 0..m {
                broker.submit(resync_eval(&node_raw), Instant::now());
                // assert_always: never more than one pending at the resync key.
                prop_assert_eq!(broker.counters().queued, 1);
            }
            let counters = broker.counters();
            prop_assert_eq!(counters.queued, 1, "exactly one pending at (R, node/n)");
            prop_assert_eq!(counters.cancelled, m - 1, "cancelled == submits - 1");
        }
    }
}

// vm-lifecycle-latency: source-local policy properties transition with the
// exact timestamped signatures in ADR-0102. No test-side compatibility API.
#[cfg(test)]
#[allow(clippy::doc_markdown, reason = "exact per-test contract declarations")]
mod bounded_admission_contract {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, Instant};

    use proptest::prelude::*;

    use super::{BrokerCounters, Evaluation, EvaluationBroker};
    use crate::reconcilers::{ReconcilerName, TargetResource};

    #[derive(Clone, Debug)]
    enum TraceOp {
        Submit { reconciler: u8, target: u8, at_ms: u16 },
        Drain { limit: u8, blocked: u16, now_ms: u16 },
        Reap,
    }

    fn trace_op() -> impl Strategy<Value = TraceOp> {
        prop_oneof![
            6 => (0u8..4, 0u8..12, 0u16..400).prop_map(
                |(reconciler, target, at_ms)| TraceOp::Submit { reconciler, target, at_ms }
            ),
            3 => (0u8..=12, any::<u16>(), 0u16..400).prop_map(
                |(limit, blocked, now_ms)| TraceOp::Drain { limit, blocked, now_ms }
            ),
            1 => Just(TraceOp::Reap),
        ]
    }

    fn evaluation(reconciler: u8, target: u8) -> Evaluation {
        let family = match target % 3 {
            0 => "workload",
            1 => "service",
            _ => "node",
        };
        Evaluation {
            reconciler: ReconcilerName::new(&format!("r{reconciler}"))
                .expect("generated reconciler name is valid"),
            target: TargetResource::new(&format!("{family}/t{target}"))
                .expect("generated target is valid"),
        }
    }

    #[derive(Clone, Debug)]
    struct ReferencePending {
        evaluation: Evaluation,
        submitted_at_ms: u16,
        fifo: u64,
    }

    #[derive(Default)]
    struct ReferenceBroker {
        pending: BTreeMap<(ReconcilerName, TargetResource), ReferencePending>,
        cancelable: Vec<Evaluation>,
        cancelled: u64,
        dispatched: u64,
        next_fifo: u64,
    }

    impl ReferenceBroker {
        fn submit(&mut self, value: Evaluation, submitted_at_ms: u16) {
            let key = (value.reconciler.clone(), value.target.clone());
            if let Some(prior) = self.pending.get_mut(&key) {
                self.cancelable.push(prior.evaluation.clone());
                prior.evaluation = value;
                self.cancelled = self.cancelled.saturating_add(1);
                return;
            }
            let fifo = self.next_fifo;
            self.next_fifo = self.next_fifo.saturating_add(1);
            self.pending.insert(key, ReferencePending { evaluation: value, submitted_at_ms, fifo });
        }

        fn drain(
            &mut self,
            limit: usize,
            blocked: &BTreeSet<TargetResource>,
            now_ms: u16,
        ) -> Vec<(Evaluation, Duration)> {
            let mut candidates: Vec<_> =
                self.pending.iter().map(|(key, value)| (value.fifo, key.clone())).collect();
            candidates.sort_by_key(|(fifo, _)| *fifo);
            let mut unavailable = blocked.clone();
            let mut selected = Vec::new();
            for (_, key) in candidates {
                if selected.len() == limit || unavailable.contains(&key.1) {
                    continue;
                }
                let value = self.pending.remove(&key).expect("candidate remains pending");
                unavailable.insert(value.evaluation.target.clone());
                let queued_ms = now_ms.saturating_sub(value.submitted_at_ms);
                selected.push((value.evaluation, Duration::from_millis(u64::from(queued_ms))));
            }
            self.dispatched = self.dispatched.saturating_add(selected.len() as u64);
            selected
        }

        fn reap(&mut self) -> usize {
            let count = self.cancelable.len();
            self.cancelable.clear();
            count
        }

        fn counters(&self) -> BrokerCounters {
            BrokerCounters {
                queued: self.pending.len() as u64,
                cancelled: self.cancelled,
                dispatched: self.dispatched,
            }
        }
    }

    fn blocked_targets(mask: u16) -> BTreeSet<TargetResource> {
        (0u8..12)
            .filter(|target| mask & (1 << target) != 0)
            .map(|target| evaluation(0, target).target)
            .collect()
    }

    fn elapsed(base: Instant, at_ms: u16) -> Instant {
        base + Duration::from_millis(u64::from(at_ms))
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        /// S-VLL-05a. Generate submit/replace/drain/reap traces; compare complete
        /// returned evaluations and residence durations, pending/cancelled/dispatched
        /// counters and reap result against an independent ordered reference model.
        /// Domains: 0..12 slots, all/none/some blocked targets across workload/service/
        /// node keys, cross-reconciler same targets, clock before/equal/after submit.
        /// First-submit FIFO age survives replacement; zero/all-blocked drains have
        /// zero delta; earliest eligible wins; only actual admissions increment
        /// dispatched; cancelable reaping preserves lifetime counters.
        #[test]
        fn bounded_admission_preserves_fifo_age_and_complete_counter_delta(
            trace in proptest::collection::vec(trace_op(), 1..96),
        ) {
            let base = Instant::now();
            // Mandatory named boundaries are exercised on every generated
            // case rather than left to random sampling.  The three masks are
            // respectively none/some/all blocked; enqueue at 200ms and drain
            // before/equal/after it pins saturating residence time.
            for limit in [0usize, 1, 7, 8, 9] {
                for (mask, now_ms) in [(0u16, 100u16), (0b0101_0101_0101, 200), (0x0fff, 300)] {
                    let mut boundary_actual = EvaluationBroker::new();
                    let mut boundary_expected = ReferenceBroker::default();
                    for target in 0u8..12 {
                        let value = evaluation(target % 4, target);
                        boundary_actual.submit(value.clone(), elapsed(base, 200));
                        boundary_expected.submit(value, 200);
                    }
                    let blocked = blocked_targets(mask);
                    prop_assert_eq!(
                        boundary_actual.drain_pending(limit, &blocked, elapsed(base, now_ms)),
                        boundary_expected.drain(limit, &blocked, now_ms),
                    );
                    prop_assert_eq!(boundary_actual.counters(), boundary_expected.counters());
                }
            }

            let mut actual = EvaluationBroker::new();
            let mut expected = ReferenceBroker::default();
            for operation in trace {
                match operation {
                    TraceOp::Submit { reconciler, target, at_ms } => {
                        let value = evaluation(reconciler, target);
                        actual.submit(value.clone(), elapsed(base, at_ms));
                        expected.submit(value, at_ms);
                    }
                    TraceOp::Drain { limit, blocked, now_ms } => {
                        let blocked = blocked_targets(blocked);
                        let actual_drain = actual.drain_pending(
                            usize::from(limit),
                            &blocked,
                            elapsed(base, now_ms),
                        );
                        let expected_drain = expected.drain(
                            usize::from(limit),
                            &blocked,
                            now_ms,
                        );
                        prop_assert_eq!(actual_drain, expected_drain);
                    }
                    TraceOp::Reap => {
                        prop_assert_eq!(actual.reap_cancelable(), expected.reap());
                    }
                }
                prop_assert_eq!(actual.counters(), expected.counters());
            }
        }
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        /// S-VLL-05b. Repeatedly enqueue a hot key around independently arriving
        /// keys; releasing its lease and resubmitting puts new work at the tail,
        /// while replacement of a still-pending key retains its original age.
        /// Compare complete drained order, including cross-reconciler same-target
        /// exclusion within one admission batch; no lexicographic-ID bias.
        #[test]
        fn hot_target_requeue_cannot_starve_older_eligible_work(
            older_count in 1u8..=8,
            hot_replacements in 1u8..=16,
            initial_ms in 20u16..200,
        ) {
            let base = Instant::now();
            let mut broker = EvaluationBroker::new();
            // The oldest key sorts after every independently arriving key;
            // a key-ordered drain therefore fails this FIFO oracle.
            let hot = evaluation(3, 11);
            broker.submit(hot.clone(), elapsed(base, initial_ms));
            for ordinal in 0..older_count {
                let target = older_count - ordinal - 1;
                broker.submit(
                    evaluation(0, target),
                    elapsed(base, initial_ms.saturating_add(u16::from(ordinal) + 1)),
                );
            }
            for replacement in 0..hot_replacements {
                broker.submit(
                    hot.clone(),
                    elapsed(base, initial_ms.saturating_add(50 + u16::from(replacement))),
                );
            }

            let first = broker.drain_pending(1, &BTreeSet::new(), elapsed(base, initial_ms + 300));
            prop_assert_eq!(first.len(), 1);
            prop_assert_eq!(&first[0].0, &hot);
            prop_assert_eq!(first[0].1, Duration::from_millis(300));

            // The active hot target is unavailable across reconciler names.
            let hot_other_reconciler = evaluation(2, 11);
            broker.submit(hot_other_reconciler.clone(), elapsed(base, initial_ms + 301));
            broker.submit(hot.clone(), elapsed(base, initial_ms + 302));
            let blocked = BTreeSet::from([hot.target.clone()]);
            let older = broker.drain_pending(
                usize::from(older_count),
                &blocked,
                elapsed(base, initial_ms + 400),
            );
            let expected_older: Vec<_> = (0..older_count)
                .map(|ordinal| evaluation(0, older_count - ordinal - 1))
                .collect();
            prop_assert_eq!(
                older.iter().map(|(value, _)| value.clone()).collect::<Vec<_>>(),
                expected_older
            );

            // Releasing the lease admits only one same-target evaluation in
            // this batch; the other remains pending for the next lease.
            let one_hot = broker.drain_pending(8, &BTreeSet::new(), elapsed(base, initial_ms + 500));
            prop_assert_eq!(one_hot.len(), 1);
            prop_assert!(one_hot[0].0 == hot || one_hot[0].0 == hot_other_reconciler);
            prop_assert_eq!(broker.counters().queued, 1);
        }
    }
}
