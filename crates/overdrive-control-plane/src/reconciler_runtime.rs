//! `ReconcilerRuntime` — composes `AnyReconciler` enum-dispatched
//! reconcilers, the `EvaluationBroker`, and per-primitive libSQL path
//! provisioning.
//!
//! Per ADR-0013 (amended 2026-04-24), the trait's pre-hydration +
//! `TickContext` shape broke object safety, so the runtime registers
//! `AnyReconciler` (enum-dispatched) rather than `Box<dyn Reconciler>`.
//!
//! Per ADR-0013, the runtime lives in this crate (NOT in `overdrive-core`),
//! because it pulls in `libsql` and wiring-layer concerns. Core stays
//! port-only.
//!
//! Phase 1 shape: the runtime owns a `HashMap<ReconcilerName,
//! AnyReconciler>` keyed by the canonical name, plus an
//! `EvaluationBroker` behind `&self`. Registration eagerly derives the
//! per-reconciler libSQL path via
//! [`crate::libsql_provisioner::provision_db_path`] — the DB itself is
//! opened lazily by callers that need it (Phase 3+). Provisioning the
//! path at register time surfaces invalid `data_dir`s (permission
//! denied, traversal attempt) at registration rather than deferred
//! until first use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use overdrive_core::reconciler::{AnyReconciler, ReconcilerName};

use crate::error::ControlPlaneError;
use crate::eval_broker::EvaluationBroker;
use crate::libsql_provisioner::provision_db_path;

/// Registry + broker + libSQL path owner.
pub struct ReconcilerRuntime {
    /// Canonicalised data directory under which per-reconciler libSQL
    /// files live at `<data_dir>/reconcilers/<name>/memory.db`.
    data_dir: PathBuf,
    /// Registry keyed on canonical reconciler name. Duplicate
    /// registration is rejected with `ControlPlaneError::Conflict`.
    reconcilers: HashMap<ReconcilerName, AnyReconciler>,
    /// Cancelable-eval-set evaluation broker per ADR-0013 §8.
    broker: EvaluationBroker,
}

impl ReconcilerRuntime {
    /// Construct a new runtime rooted at `data_dir`. Creates the
    /// directory if absent (so `canonicalize` has a real target) and
    /// canonicalises it once per ADR-0013 §5 so subsequent
    /// `provision_db_path` calls operate on the fully-resolved path.
    ///
    /// # Errors
    ///
    /// Returns [`ControlPlaneError::Internal`] if the directory cannot
    /// be created or canonicalised.
    pub fn new(data_dir: &Path) -> Result<Self, ControlPlaneError> {
        std::fs::create_dir_all(data_dir).map_err(|e| {
            ControlPlaneError::internal(
                format!("ReconcilerRuntime::new: create_dir_all {} failed", data_dir.display()),
                e,
            )
        })?;
        let canon = std::fs::canonicalize(data_dir).map_err(|e| {
            ControlPlaneError::internal(
                format!("ReconcilerRuntime::new: canonicalize {} failed", data_dir.display()),
                e,
            )
        })?;
        Ok(Self { data_dir: canon, reconcilers: HashMap::new(), broker: EvaluationBroker::new() })
    }

    /// Register a reconciler. Derives its libSQL path under
    /// `<data_dir>/reconcilers/<name>/memory.db` (path derivation only —
    /// the DB is not opened here) and inserts it into the registry.
    ///
    /// # Errors
    ///
    /// * [`ControlPlaneError::Conflict`] if a reconciler with the same
    ///   name is already registered. The second registration is
    ///   rejected cleanly — the registry is left unchanged.
    /// * [`ControlPlaneError::Internal`] if path provisioning fails
    ///   (permission denied, traversal rejected, etc.).
    pub fn register(&mut self, reconciler: AnyReconciler) -> Result<(), ControlPlaneError> {
        let name = reconciler.name().clone();
        if self.reconcilers.contains_key(&name) {
            return Err(ControlPlaneError::Conflict {
                message: format!("reconciler {name} already registered"),
            });
        }
        // Path derivation only — surfaces permission / traversal errors
        // at register time rather than deferring to first DB open.
        let _path = provision_db_path(&self.data_dir, &name)?;
        self.reconcilers.insert(name, reconciler);
        Ok(())
    }

    /// Registered reconciler names. Order is unspecified (`HashMap`) but
    /// stable within a single runtime lifetime given the same
    /// registration sequence — callers that need deterministic order
    /// should sort.
    #[must_use]
    pub fn registered(&self) -> Vec<ReconcilerName> {
        self.reconcilers.keys().cloned().collect()
    }

    /// Borrow the evaluation broker.
    #[must_use]
    pub const fn broker(&self) -> &EvaluationBroker {
        &self.broker
    }

    /// Iterate the registered reconcilers. Used by the ADR-0017
    /// `reconciler_is_pure` invariant to twin-invocation-check every
    /// reconciler in the registry from a single harness entry point.
    pub fn reconcilers_iter(&self) -> impl Iterator<Item = &AnyReconciler> {
        self.reconcilers.values()
    }
}
