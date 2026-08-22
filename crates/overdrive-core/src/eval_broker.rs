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

use std::collections::BTreeMap;

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
    pending: BTreeMap<(ReconcilerName, TargetResource), Evaluation>,
    /// Evaluations that were superseded at their key, awaiting bulk
    /// reap by the runtime reaper tick.
    cancelable: Vec<Evaluation>,
    /// Accumulator counters. `queued` in the snapshot is computed from
    /// `pending.len()` at `counters()` call time; the struct field
    /// tracks only the accumulators.
    cancelled: u64,
    dispatched: u64,
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
    pub fn submit(&mut self, eval: Evaluation) {
        let key = (eval.reconciler.clone(), eval.target.clone());
        if let Some(prev) = self.pending.insert(key, eval) {
            self.cancelable.push(prev);
            self.cancelled = self.cancelled.saturating_add(1);
        }
    }

    /// Empty the pending map into the runtime's dispatch path.
    /// `dispatched` increments by the number of drained evaluations;
    /// the cancelable vec is untouched.
    pub fn drain_pending(&mut self) -> Vec<Evaluation> {
        // `BTreeMap::drain` is nightly-only on stable Rust; `mem::take` +
        // `into_values` is the equivalent pattern that yields entries in
        // ascending key order — exactly the determinism property this
        // method exists to guarantee.
        let drained: Vec<Evaluation> = std::mem::take(&mut self.pending).into_values().collect();
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
// S-266-19 (GH #266, ADR-0081 §4) — resync-on-resync same-key collapse. A
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

    /// A prior eval already pending at `(R, node/n)`; a redundant same-key
    /// resync submit while it is still pending collapses to EXACTLY one
    /// pending and bumps `cancelled` by EXACTLY one — never fewer (missed
    /// collapse) and never more (double-count).
    #[test]
    fn redundant_same_key_resync_collapses_to_one_pending_and_bumps_cancelled_by_one() {
        let mut broker = EvaluationBroker::new();
        let eval = resync_eval("n");

        broker.submit(eval.clone());
        let before = broker.counters();
        assert_eq!(before.queued, 1, "prior resync is pending at (R, node/n)");
        assert_eq!(before.cancelled, 0, "no collapse yet");

        broker.submit(eval);
        let after = broker.counters();
        assert_eq!(after.queued, 1, "≤1 pending at (R, node/n) after redundant resync");
        assert_eq!(after.cancelled, 1, "cancelled bumps by exactly one");
    }

    proptest! {
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
                broker.submit(resync_eval(&node_raw));
                // assert_always: never more than one pending at the resync key.
                prop_assert_eq!(broker.counters().queued, 1);
            }
            let counters = broker.counters();
            prop_assert_eq!(counters.queued, 1, "exactly one pending at (R, node/n)");
            prop_assert_eq!(counters.cancelled, m - 1, "cancelled == submits - 1");
        }
    }
}
