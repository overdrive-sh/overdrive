//! `EvaluationBroker` with cancelable-eval-set semantics per whitepaper §18.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Keyed on `(ReconcilerName, TargetResource)` — a second submit at the
//! same key moves the prior evaluation into the cancelable set. The
//! reaper empties the cancelable set in bulk on a fixed tick cadence.
//! The storm-proofing guarantee from ADR-0013 is this broker's reason
//! for existing.

use overdrive_core::reconciler::{ReconcilerName, TargetResource};

use crate::error::ControlPlaneError;

/// Per-reconciler broker counters rendered by `cluster status`.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Copy, Default)]
pub struct BrokerCounters {
    pub queued: u64,
    pub cancelled: u64,
    pub dispatched: u64,
}

/// The cancelable-eval-set evaluation broker.
///
/// SCAFFOLD: true
pub struct EvaluationBroker {
    // Field shape lands with Slice 4.
}

/// One evaluation through the broker.
///
/// SCAFFOLD: true
pub struct Evaluation {
    pub reconciler: ReconcilerName,
    pub target: TargetResource,
}

impl EvaluationBroker {
    /// Construct a fresh, empty broker.
    ///
    /// SCAFFOLD: true
    pub fn new() -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Submit an evaluation. If a pending evaluation exists at the same
    /// key, it is moved to the cancelable set and the cancelled counter
    /// increments.
    ///
    /// SCAFFOLD: true
    pub fn submit(&mut self, _eval: Evaluation) -> Result<(), ControlPlaneError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Empty the pending queue into the runtime's dispatch path. The
    /// dispatched counter increments per drained evaluation.
    ///
    /// SCAFFOLD: true
    pub fn drain_pending(&mut self) -> Vec<Evaluation> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Empty the cancelable set in bulk.
    ///
    /// SCAFFOLD: true
    pub fn reap_cancelable(&mut self) -> usize {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Current counter snapshot.
    ///
    /// SCAFFOLD: true
    pub fn counters(&self) -> BrokerCounters {
        panic!("Not yet implemented -- RED scaffold")
    }
}
