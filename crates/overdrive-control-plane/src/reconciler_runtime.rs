//! `ReconcilerRuntime` — composes the `Reconciler` trait, the
//! `EvaluationBroker`, and per-primitive libSQL provisioning.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0013, the runtime lives in this crate (NOT in `overdrive-core`),
//! because it pulls in `libsql` and wiring-layer concerns. Core stays
//! port-only.

use std::path::PathBuf;

use overdrive_core::reconciler::{Reconciler, ReconcilerName};

use crate::error::ControlPlaneError;
use crate::eval_broker::EvaluationBroker;

/// Registry + broker + libSQL pool owner.
///
/// SCAFFOLD: true
pub struct ReconcilerRuntime {
    // Field shape lands with Slice 4.
}

impl ReconcilerRuntime {
    /// Construct a new runtime rooted at `data_dir`. Canonicalises the
    /// path once per ADR-0013 §5.
    ///
    /// SCAFFOLD: true
    pub fn new(_data_dir: PathBuf) -> Result<Self, ControlPlaneError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Register a reconciler. Provisions its libSQL DB path under
    /// `<data_dir>/reconcilers/<name>/memory.db` and adds it to the
    /// registry.
    ///
    /// SCAFFOLD: true
    pub fn register(&mut self, _reconciler: Box<dyn Reconciler>) -> Result<(), ControlPlaneError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// The registry — names of currently-registered reconcilers.
    ///
    /// SCAFFOLD: true
    pub fn registered(&self) -> Vec<ReconcilerName> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Borrow the evaluation broker.
    ///
    /// SCAFFOLD: true
    pub fn broker(&self) -> &EvaluationBroker {
        panic!("Not yet implemented -- RED scaffold")
    }
}
