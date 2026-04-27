//! Action shim — the single async I/O boundary in the convergence
//! loop. Per ADR-0023.
//!
//! The shim consumes `Vec<Action>` emitted by the reconciler runtime
//! (after `reconcile` returns), dispatches allocation-management
//! actions to `&dyn Driver`, and writes resulting `AllocStatusRow`s
//! to `&dyn ObservationStore`. All `.await` points in the
//! post-reconcile pipeline live here — `reconcile` itself is
//! synchronous + pure per ADR-0013.
//!
//! # Module path
//!
//! Per ADR-0023 §1, the canonical module path is
//! `overdrive_control_plane::reconciler_runtime::action_shim`. The
//! existing `reconciler_runtime` is currently a single .rs file;
//! during DELIVER's first refactor pass, it becomes a directory and
//! this module is re-exported from inside it. For the RED-scaffold
//! moment, the shim lives at the crate root as `action_shim` and is
//! re-exported under the canonical path via `pub mod` in lib.rs.
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 3 (US-03).
//! Wave: DISTILL. SCAFFOLD: true — `dispatch` panics; DELIVER
//! implements the per-action match per ADR-0023 §2.

use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{Driver, DriverError};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = true;

/// Dispatch a reconciler's emitted `Vec<Action>` against the active
/// driver and observation store. Called by the runtime's tick loop
/// after every `reconcile` call.
///
/// Per ADR-0023 §2:
/// - Takes `&dyn Driver` and `&dyn ObservationStore` (NOT Arc; the
///   caller holds the Arcs).
/// - Each `Action` variant gets its own match arm; the compiler
///   enforces exhaustiveness across the (now five-variant) Action
///   enum.
/// - A driver `SpawnFailed` writes a `Failed` AllocStatusRow and
///   returns `Ok(())` — the failure is *recorded*, not surfaced as
///   ShimError.
/// - `ShimError` is reserved for failures the shim cannot resolve
///   into an observation row (e.g. observation store itself broken).
///
/// # Errors
///
/// Returns `ShimError::Driver` only when the underlying error is not
/// representable as an `AllocStatusRow`. Returns `ShimError::Observation`
/// when the observation store rejects the write itself.
///
/// # Panics
///
/// RED scaffold.
pub async fn dispatch(
    _actions: Vec<Action>,
    _driver: &dyn Driver,
    _obs: &dyn ObservationStore,
    _tick: &TickContext,
) -> Result<(), ShimError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Errors from [`dispatch`] that cannot be resolved into an
/// observation row. Per ADR-0023 §3.
#[derive(Debug, thiserror::Error)]
pub enum ShimError {
    /// A driver failure that did not fit the SpawnFailed shape (i.e.
    /// the shim cannot record it as `state: Failed`).
    #[error("driver failure")]
    Driver(#[from] DriverError),
    /// The observation store itself rejected the write.
    #[error("observation write failure")]
    Observation(#[from] ObservationStoreError),
    /// The shim could not look up an `AllocationHandle` for the
    /// requested `alloc_id` — typically when a Stop / Restart action
    /// arrives for an alloc the driver no longer tracks.
    #[error("alloc handle missing for {alloc_id}")]
    HandleMissing {
        /// The allocation whose handle is missing.
        alloc_id: overdrive_core::id::AllocationId,
    },
}
