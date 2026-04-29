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
//! this module is re-exported from inside it. For Phase 1 the shim
//! lives at the crate root as `action_shim` and is re-exported under
//! the canonical path via `pub mod` in lib.rs.

use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{AllocationHandle, AllocationSpec, Driver, DriverError};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = false;

/// Build an `AllocStatusRow` for a state transition driven by the shim.
/// Used by every variant that writes observation: `StartAllocation`,
/// `RestartAllocation`, and `StopAllocation` all funnel through this
/// helper so the row shape is constructed in exactly one place. Pure
/// over its inputs — does not touch the observation store.
fn build_alloc_status_row(
    alloc_id: AllocationId,
    job_id: JobId,
    node_id: NodeId,
    state: AllocState,
    tick: &TickContext,
) -> AllocStatusRow {
    let writer = node_id.clone();
    AllocStatusRow { alloc_id, job_id, node_id, state, updated_at: timestamp_for(tick, writer) }
}

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
        // Phase 1 does not yet model a `Failed` AllocState variant —
        // Terminated is the closest match.
        Action::StartAllocation { alloc_id, job_id, node_id, spec } => {
            let state = match driver.start(&spec).await {
                Ok(_handle) => AllocState::Running,
                Err(DriverError::StartRejected { reason: _, .. }) => AllocState::Terminated,
                Err(other) => return Err(ShimError::Driver(other)),
            };
            let row = build_alloc_status_row(alloc_id, job_id, node_id, state, tick);
            obs.write(ObservationRow::AllocStatus(row)).await?;
            Ok(())
        }
        // Restart: stop-then-start, reusing the same alloc id. Per
        // ADR-0023 §2 Restart is semantically `stop + start` against
        // the prior alloc. Phase 1 single-mode reuses the deterministic
        // alloc id derived by `JobLifecycle::reconcile`'s
        // `mint_alloc_id(job_id)` (the same alloc id flows through
        // every restart cycle). The action shim looks up the alloc
        // metadata in observation to reconstruct the spec for the
        // start half — for 02-03 the spec is rebuilt from the existing
        // `AllocStatusRow.job_id` plus a Phase-1 baseline image and
        // resource envelope derived from the original Job intent.
        Action::RestartAllocation { alloc_id } => {
            // Stop half — Phase 1 uses an empty AllocationHandle (no
            // pid tracking yet); the driver's `stop` is best-effort
            // and `NotFound` is silently absorbed (the alloc may have
            // already terminated on a prior failed start).
            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            let _ = driver.stop(&handle).await;

            // Start half — look up the prior alloc row to recover the
            // job_id and node_id; reconstruct the spec from a Phase-1
            // baseline (`/bin/sleep`, default resources). This keeps
            // the restart path observable without threading the full
            // Job aggregate through the action.
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Err(ShimError::HandleMissing { alloc_id });
            };

            let spec = build_phase1_restart_spec(&alloc_id, &prior_row.job_id);
            // Failed restart — record as Terminated so the next tick's
            // hydrate sees the prior failure and can decide whether to
            // back off or exhaust.
            let state = match driver.start(&spec).await {
                Ok(_handle) => AllocState::Running,
                Err(DriverError::StartRejected { .. }) => AllocState::Terminated,
                Err(other) => return Err(ShimError::Driver(other)),
            };
            let row =
                build_alloc_status_row(alloc_id, prior_row.job_id, prior_row.node_id, state, tick);
            obs.write(ObservationRow::AllocStatus(row)).await?;
            Ok(())
        }
        // Stop: best-effort driver stop, then write a Terminated row
        // for the alloc. Per ADR-0023 §2 the stop path is best-effort
        // — if the driver no longer tracks the alloc (NotFound), the
        // shim still records Terminated so the next tick's hydrate
        // sees the alloc gone. Per-variant error isolation: a Stop
        // failure does NOT abort dispatch of subsequent actions.
        Action::StopAllocation { alloc_id } => {
            // Look up prior obs row to recover (job_id, node_id) for
            // the Terminated row we will write. If the alloc has no
            // obs row at all (e.g. the reconciler emitted Stop
            // without ever having seen the alloc Running) there is
            // nothing to write — return Ok.
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Ok(());
            };

            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            // Driver stop is best-effort — NotFound and other
            // failures are absorbed; the Terminated row records the
            // outcome regardless. This mirrors the Restart variant's
            // stop-half pattern.
            let _ = driver.stop(&handle).await;
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.job_id,
                prior_row.node_id,
                AllocState::Terminated,
                tick,
            );
            obs.write(ObservationRow::AllocStatus(row)).await?;
            Ok(())
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

/// Look up the most recent observation row for `alloc_id`, used by the
/// Restart and Stop variants to recover `(job_id, node_id)` for the
/// Terminated row they write. Returns `Ok(None)` when no row exists —
/// callers decide whether that is an error (Restart) or a no-op (Stop).
async fn find_prior_alloc_row(
    obs: &dyn ObservationStore,
    alloc_id: &AllocationId,
) -> Result<Option<AllocStatusRow>, ShimError> {
    Ok(obs.alloc_status_rows().await?.into_iter().find(|r| &r.alloc_id == alloc_id))
}

/// Build the Phase-1 baseline `AllocationSpec` used to reconstruct a
/// Restart's spawn from observation alone. Phase 2+ threads the full
/// Job aggregate through the action so this helper goes away.
fn build_phase1_restart_spec(alloc_id: &AllocationId, job_id: &JobId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id.clone(),
        identity: build_identity(job_id, alloc_id),
        image: "/bin/sleep".to_string(),
        resources: default_restart_resources(),
    }
}

/// Reconstruct the SPIFFE identity for a restart's fresh
/// `AllocationSpec`. Mirrors the derivation in
/// `overdrive_core::reconciler::mint_identity` — the `JobLifecycle`
/// reconciler is the source of truth for the canonical form, but the
/// shim cannot reach a private function in core. The two formulae are
/// pinned by an acceptance test.
fn build_identity(
    job_id: &overdrive_core::id::JobId,
    alloc_id: &overdrive_core::id::AllocationId,
) -> overdrive_core::SpiffeId {
    let raw =
        format!("spiffe://overdrive.local/job/{}/alloc/{}", job_id.as_str(), alloc_id.as_str());
    #[allow(clippy::expect_used)]
    overdrive_core::SpiffeId::new(&raw).expect("derived SpiffeId is valid")
}

/// Phase 1 baseline resources used when reconstructing a Restart's
/// `AllocationSpec`. The original Job intent's resource envelope is
/// the right long-term source — Phase 2+ threads the Job aggregate
/// through the action — but for 02-03 a baseline is sufficient: the
/// `ExecDriver` currently ignores `resources` (cgroup pre-flight is
/// out-of-scope until 03-01).
const fn default_restart_resources() -> overdrive_core::traits::driver::Resources {
    overdrive_core::traits::driver::Resources { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 }
}

/// Errors from [`dispatch`] that cannot be resolved into an
/// observation row. Per ADR-0023 §3.
#[derive(Debug, thiserror::Error)]
pub enum ShimError {
    /// A driver failure that did not fit the `SpawnFailed` shape (i.e.
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

// ---------------------------------------------------------------------------
// Unit tests for the private const helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::default_restart_resources;

    /// Pin every numeric field of `default_restart_resources` to its
    /// exact production value. Kills the four mutations on the
    /// `cpu_milli: 100, memory_bytes: 256 * 1024 * 1024` literal:
    ///
    ///   - `*` -> `+` at position 83 (between 256 and the first
    ///     1024) — would yield `256 + 1024 * 1024 = 1048832`.
    ///   - `*` -> `/` at position 83 — would yield `256 / 1024 *
    ///     1024 = 0`.
    ///   - `*` -> `+` at position 90 (between 1024 and 1024) — would
    ///     yield `256 * (1024 + 1024) = 524288`.
    ///   - `*` -> `/` at position 90 — would yield `256 * (1024 /
    ///     1024) = 256`.
    ///
    /// Pinning the exact production value `268435456` (= 256 MiB)
    /// rejects every mutant because their values differ.
    #[test]
    fn default_restart_resources_pins_exact_values() {
        let r = default_restart_resources();
        assert_eq!(r.cpu_milli, 100, "cpu_milli must be exactly 100");
        assert_eq!(
            r.memory_bytes,
            256 * 1024 * 1024,
            "memory_bytes must be exactly 256 MiB = 268435456",
        );
        // Belt-and-braces: pin the absolute byte count too, so a
        // mutation that happens to yield the right SHAPE but the
        // wrong VALUE is still caught.
        assert_eq!(r.memory_bytes, 268_435_456_u64);
    }
}
