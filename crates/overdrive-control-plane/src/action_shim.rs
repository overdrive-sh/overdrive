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

use overdrive_core::id::NodeId;
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{AllocationHandle, Driver, DriverError};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = false;

/// Dispatch a reconciler's emitted `Vec<Action>` against the active
/// driver and observation store. Called by the runtime's tick loop
/// after every `reconcile` call.
///
/// Per ADR-0023 §2:
/// - Takes `&dyn Driver` and `&dyn ObservationStore` (NOT Arc; the
///   caller holds the Arcs).
/// - Each [`Action`] variant gets its own match arm; the compiler
///   enforces exhaustiveness across the [`Action`] enum.
/// - A driver `StartRejected` writes a `Failed` (Terminated)
///   [`AllocStatusRow`] and returns `Ok(())` — the failure is *recorded*,
///   not surfaced as [`ShimError`].
/// - [`ShimError`] is reserved for failures the shim cannot resolve
///   into an observation row (e.g. observation store itself broken).
///
/// Per-variant error isolation: a failed `StartAllocation` does NOT
/// abort dispatch of subsequent actions. Each variant is processed
/// independently; if multiple actions fail, the first [`ShimError`]
/// surfaces.
///
/// # Errors
///
/// Returns [`ShimError::Driver`] only when the underlying error is not
/// representable as an [`AllocStatusRow`]. Returns
/// [`ShimError::Observation`] when the observation store rejects the
/// write itself.
///
/// # Panics
///
/// `Action::StopAllocation` arm is panic-bodied — landing in 02-04.
pub async fn dispatch(
    actions: Vec<Action>,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    tick: &TickContext,
) -> Result<(), ShimError> {
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        let result = dispatch_single(action, driver, obs, tick).await;
        if let Err(err) = result {
            // Per-variant error isolation: record only the first error
            // and continue draining the rest of the actions.
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }

    first_error.map_or(Ok(()), Err)
}

/// Dispatch a single action. Each variant is independent; the caller
/// loops over a `Vec<Action>` and aggregates errors.
async fn dispatch_single(
    action: Action,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    tick: &TickContext,
) -> Result<(), ShimError> {
    match action {
        // No-op (Action::Noop), Phase 3 workflow start, and the Phase 3
        // HttpCall placeholder are all "no dispatch needed at 02-02"
        // — the action is observation-only or deferred.
        Action::Noop | Action::StartWorkflow { .. } | Action::HttpCall { .. } => Ok(()),
        // Start: spawn the allocation via the driver and write a
        // Running AllocStatusRow on success. On StartRejected, write
        // a Terminated row recording the failure (per ADR-0023 §2).
        Action::StartAllocation { alloc_id, job_id, node_id, spec } => {
            let writer_node = node_id.clone();
            match driver.start(&spec).await {
                Ok(_handle) => {
                    let row = AllocStatusRow {
                        alloc_id: alloc_id.clone(),
                        job_id,
                        node_id,
                        state: AllocState::Running,
                        updated_at: timestamp_for(tick, writer_node),
                    };
                    obs.write(ObservationRow::AllocStatus(row)).await?;
                    Ok(())
                }
                Err(DriverError::StartRejected { reason: _, .. }) => {
                    // Record failure as a Terminated row. Phase 1
                    // does not yet model a `Failed` AllocState
                    // variant — Terminated is the closest match.
                    let row = AllocStatusRow {
                        alloc_id: alloc_id.clone(),
                        job_id,
                        node_id,
                        state: AllocState::Terminated,
                        updated_at: timestamp_for(tick, writer_node),
                    };
                    obs.write(ObservationRow::AllocStatus(row)).await?;
                    Ok(())
                }
                Err(other) => Err(ShimError::Driver(other)),
            }
        }
        // Restart: stop-then-start with a fresh alloc id. For 02-02
        // we treat it as a fresh start through the same Start path —
        // proper stop+start sequencing lands in 02-04 alongside the
        // StopAllocation wiring.
        Action::RestartAllocation { alloc_id } => {
            // For 02-02 we cannot synthesise a fresh AllocationSpec
            // without more context (the JobLifecycle reconciler emits
            // RestartAllocation only in scenarios its current Run
            // branch does not produce — the convergence path lives
            // in 02-03). Stop the prior allocation if its handle is
            // still tracked and write a Terminated marker.
            let handle = AllocationHandle { alloc: alloc_id, pid: None };
            match driver.stop(&handle).await {
                Ok(()) | Err(DriverError::NotFound { .. }) => Ok(()),
                Err(other) => Err(ShimError::Driver(other)),
            }
        }
        // Stop: panic-bodied per the 02-02 scope. The convergence
        // path that emits Action::StopAllocation lands in 02-04.
        Action::StopAllocation { .. } => {
            panic!("Not yet implemented -- 02-04 RED scaffold")
        }
    }
}

/// Build a `LogicalTimestamp` from the current tick. The shim writes
/// every observation row with `(counter = tick.tick + 1, writer = node_id)`
/// so two writes for the same alloc on different ticks are correctly
/// ordered under LWW.
const fn timestamp_for(tick: &TickContext, writer: NodeId) -> LogicalTimestamp {
    LogicalTimestamp { counter: tick.tick.saturating_add(1), writer }
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
