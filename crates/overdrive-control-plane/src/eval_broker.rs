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
//! structurally. The reaper *cadence* (N = 16 ticks per ADR-0013 §8)
//! lives in the runtime assembly at step 04-04 — this module delivers
//! only the `reap_cancelable()` primitive the cadence will drive.

use std::collections::BTreeMap;

use overdrive_core::reconciler::{ReconcilerName, TargetResource};

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
