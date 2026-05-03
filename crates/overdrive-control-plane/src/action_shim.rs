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

use overdrive_core::TransitionReason;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{AllocationHandle, Driver, DriverError, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::transition_reason::TerminalCondition;
use tokio::sync::broadcast;

use crate::api::{AllocStateWire, TransitionSource};

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = false;

// ---------------------------------------------------------------------------
// LifecycleEvent — broadcast-channel payload for slice 02 streaming
// ---------------------------------------------------------------------------

/// Internal broadcast-channel payload emitted by the action shim after
/// every successful `AllocStatusRow` write.
///
/// Per architecture.md §10 (cli-submit-vs-deploy-and-alloc-status DESIGN):
/// `LifecycleEvent` is a wire-shape projection of the row write event.
/// It does NOT carry the raw `AllocStatusRow` directly — a trybuild
/// compile-fail fixture (architecture.md §8) pins this invariant. The
/// fields are typed projections (`AllocStateWire` for from/to,
/// `TransitionReason` for the cause-class, `TransitionSource` for who
/// produced the row).
///
/// `LifecycleEvent` is the broadcast payload, NOT the wire type. The
/// streaming `SubmitEvent::LifecycleTransition` is constructed FROM a
/// `LifecycleEvent` in slice 02 step 02-02 / 02-03. For that reason
/// `LifecycleEvent` derives only `Debug + Clone` — NOT
/// `Serialize`/`Deserialize`/`ToSchema`. That property is what the
/// trybuild fixture defends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    /// Allocation this transition concerns.
    pub alloc_id: AllocationId,
    /// Job the allocation belongs to.
    pub job_id: JobId,
    /// Wire-shape state the alloc was in before this transition. The
    /// shim does not currently track this on every write (it's the
    /// downstream consumer's job to compute `from` from prior state);
    /// in step 02-01 this carries the row's *new* state in both `from`
    /// and `to` for the row-write events the shim emits without prior
    /// context. Step 02-03's streaming handler refines this against
    /// per-alloc prior-state tracking when it lands.
    pub from: AllocStateWire,
    /// Wire-shape state the alloc moved to.
    pub to: AllocStateWire,
    /// Structured cause-class for this transition.
    pub reason: TransitionReason,
    /// Verbatim driver text the cause-class payload does not capture
    /// (e.g. raw `errno`-decorated message). Audit trail per
    /// architecture.md §10.
    pub detail: Option<String>,
    /// Who/what produced the row write — `Reconciler` or
    /// `Driver(DriverType)` per ADR-0033 §1.
    pub source: TransitionSource,
    /// Logical-timestamp string (counter@writer) for this transition.
    pub at: String,
    /// Reconciler-decided terminal claim per ADR-0037 §4. Carries the
    /// SAME value the action shim wrote onto `AllocStatusRow.terminal`
    /// in the same dispatch call frame. Drift between the two surfaces
    /// is structurally impossible — both are populated from the
    /// originating `Action.terminal` field at one source site.
    /// `None` means "this transition is not terminal" (e.g. a
    /// Pending → Running success, a mid-budget Failed transition, an
    /// exit-observer-emitted exit event whose terminal classification
    /// is the reconciler's job to make on a subsequent tick).
    pub terminal: Option<TerminalCondition>,
}

// ---------------------------------------------------------------------------
// Classifier — DriverError::StartRejected.reason text → TransitionReason
// ---------------------------------------------------------------------------

/// Classify a `DriverError::StartRejected.reason` text into a typed
/// cause-class `TransitionReason` variant per ADR-0032 §4 Amendment
/// 2026-04-30.
///
/// Prefix-match table (order matters — specific before generic):
///
/// | Prefix shape | Variant |
/// |---|---|
/// | `spawn <path>: No such file or directory (os error 2)` | `ExecBinaryNotFound { path }` |
/// | `spawn <path>: Permission denied (os error 13)`        | `ExecPermissionDenied { path }` |
/// | `spawn <path>: Exec format error (os error 8)`         | `ExecBinaryInvalid { path, kind: "exec_format_error" }` |
/// | `cgroup setup failed: <kind>: <source>`                | `CgroupSetupFailed { kind, source }` |
/// | (anything else)                                        | `DriverInternalError { detail }` |
///
/// `_driver` and `_command` are accepted for forward-compatibility —
/// future phases may use the driver kind or the configured command to
/// disambiguate ambiguous prefix matches. Phase 1's prefix table is
/// `ExecDriver`-shaped only and the parameters are unused.
#[must_use]
pub(crate) fn classify_driver_failure(
    text: &str,
    _driver: DriverType,
    _command: &str,
) -> TransitionReason {
    // `spawn <path>: No such file or directory (os error 2)`
    if let Some(rest) = text.strip_prefix("spawn ") {
        if let Some((path, tail)) = split_once_after_path(rest) {
            if tail.starts_with("No such file or directory") {
                return TransitionReason::ExecBinaryNotFound { path: path.to_owned() };
            }
            if tail.starts_with("Permission denied") {
                return TransitionReason::ExecPermissionDenied { path: path.to_owned() };
            }
            if tail.starts_with("Exec format error") {
                return TransitionReason::ExecBinaryInvalid {
                    path: path.to_owned(),
                    kind: "exec_format_error".to_owned(),
                };
            }
        }
    }

    // `cgroup setup failed: <kind>: <source>`
    if let Some(rest) = text.strip_prefix("cgroup setup failed: ") {
        if let Some(idx) = rest.find(": ") {
            let kind = &rest[..idx];
            let source = &rest[idx + 2..];
            return TransitionReason::CgroupSetupFailed {
                kind: kind.to_owned(),
                source: source.to_owned(),
            };
        }
    }

    // Unclassified — fall through to internal error with the verbatim text.
    TransitionReason::DriverInternalError { detail: text.to_owned() }
}

/// Helper: given a string of the form `<path>: <tail>`, split into
/// `(path, tail)` on the first `: `. Returns `None` if no separator is
/// found.
fn split_once_after_path(s: &str) -> Option<(&str, &str)> {
    let idx = s.find(": ")?;
    Some((&s[..idx], &s[idx + 2..]))
}

// ---------------------------------------------------------------------------
// dispatch — single async I/O boundary, with broadcast emit
// ---------------------------------------------------------------------------

/// Build an `AllocStatusRow` for a state transition driven by the shim.
/// Used by every variant that writes observation: `StartAllocation`,
/// `RestartAllocation`, `StopAllocation`, and `FinalizeFailed` all funnel
/// through this helper so the row shape is constructed in exactly one
/// place. Pure over its inputs — does not touch the observation store.
///
/// Per ADR-0032 §3 (Amendment 2026-04-30) the row carries
/// `reason: Option<TransitionReason>` and `detail: Option<String>`
/// for cause-class attribution.
///
/// Per ADR-0037 §4 the row carries `terminal: Option<TerminalCondition>`
/// — the reconciler-emitted classification of *why* an allocation
/// reached a terminal lifecycle state. The dispatch arm passes the
/// `Action.terminal` value through here so the row's durable surface
/// and the broadcasted `LifecycleEvent.terminal` derived in
/// `build_lifecycle_event` BOTH come from the same Action-derived value
/// — drift between the two surfaces is structurally impossible.
//
// 8 row fields are intentional — the row is the durable wire shape and
// adding indirection would add noise without simplifying the call sites;
// ADR-0032 §3 + ADR-0037 §4 both grew this list deliberately.
#[allow(clippy::too_many_arguments)]
fn build_alloc_status_row(
    alloc_id: AllocationId,
    job_id: JobId,
    node_id: NodeId,
    state: AllocState,
    tick: &TickContext,
    reason: Option<TransitionReason>,
    detail: Option<String>,
    terminal: Option<TerminalCondition>,
) -> AllocStatusRow {
    let writer = node_id.clone();
    AllocStatusRow {
        alloc_id,
        job_id,
        node_id,
        state,
        updated_at: timestamp_for(tick, writer),
        reason,
        detail,
        terminal,
    }
}

/// Build a `LifecycleEvent` for the broadcast channel from a freshly
/// written `AllocStatusRow`. The wire-shape projection is mechanical —
/// `state → AllocStateWire`, `LogicalTimestamp → String`. `prior_state`
/// carries the actual allocation state before this transition; `from` is
/// set to `prior_state` so the event correctly reflects the transition
/// direction. Each call site reads the prior obs row and passes it here;
/// `StartAllocation` defaults to `Pending` for first-seen allocs.
///
/// Per ADR-0037 §4: the event's `terminal` field is byte-equal to the
/// row's `terminal` field — both are populated from the originating
/// `Action.terminal` value in the same dispatch call frame, so drift is
/// structurally impossible.
fn build_lifecycle_event(
    row: &AllocStatusRow,
    prior_state: AllocStateWire,
    source: TransitionSource,
) -> LifecycleEvent {
    let to_wire: AllocStateWire = row.state.into();
    LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        job_id: row.job_id.clone(),
        from: prior_state,
        to: to_wire,
        reason: row
            .reason
            .clone()
            .unwrap_or(TransitionReason::DriverInternalError { detail: String::new() }),
        detail: row.detail.clone(),
        source,
        at: format_logical_timestamp(&row.updated_at),
        terminal: row.terminal.clone(),
    }
}

/// Render a `LogicalTimestamp` as `counter@writer` for the wire/event
/// surface. Phase 1 keeps it stringly-typed because the CLI renders it
/// verbatim and never round-trips through arithmetic.
fn format_logical_timestamp(ts: &LogicalTimestamp) -> String {
    format!("{}@{}", ts.counter, ts.writer.as_str())
}

/// Emit a `LifecycleEvent` on the broadcast channel. Per
/// architecture.md §10: broadcast-send error is logged and discarded —
/// the row was already committed, the snapshot will see it, and a
/// missing event signals a missing subscriber (not a missed write).
/// Per-variant error isolation is preserved: a broadcast send failure
/// does not abort subsequent action dispatch.
fn emit_event(bus: &broadcast::Sender<LifecycleEvent>, event: LifecycleEvent) {
    if let Err(err) = bus.send(event) {
        // No subscribers is the normal Phase 1 case (the streaming
        // handler in 02-03 may not be active yet); demote to debug so
        // the no-subscriber path does not spam the log.
        tracing::debug!(
            target: "overdrive::action_shim",
            err = %err,
            "lifecycle event broadcast send returned error (no subscribers?); ignored",
        );
    }
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
/// - A driver `StartRejected` writes a `Failed` [`AllocStatusRow`]
///   (ADR-0032 §5: distinguishes "operator stopped" from "driver
///   could not start") and returns `Ok(())` — the failure is
///   *recorded*, not surfaced as [`ShimError`].
/// - [`ShimError`] is reserved for failures the shim cannot resolve
///   into an observation row (e.g. observation store itself broken).
///
/// Per architecture.md §10: every successful `obs.write(row)` is
/// followed by `bus.send(event)` against the broadcast channel. The
/// send error is logged and discarded; a failed send does not abort
/// subsequent action dispatch (per-variant error isolation).
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
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
) -> Result<(), ShimError> {
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        let result = dispatch_single(action, driver, obs, bus, tick).await;
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
#[allow(clippy::too_many_lines)]
async fn dispatch_single(
    action: Action,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
) -> Result<(), ShimError> {
    match action {
        // No-op (Action::Noop), Phase 3 workflow start, and the Phase 3
        // HttpCall placeholder are all "no dispatch needed" — the
        // action is observation-only or deferred.
        Action::Noop | Action::StartWorkflow { .. } | Action::HttpCall { .. } => Ok(()),
        // FinalizeFailed: the reconciler has decided this allocation
        // has reached a terminal failure (e.g. restart budget
        // exhausted). Per ADR-0037 §4 the shim threads the
        // `Action.terminal` value onto BOTH `AllocStatusRow.terminal`
        // (durable surface, written via `obs.write`) AND
        // `LifecycleEvent.terminal` (broadcast surface, emitted via
        // `bus.send`) in the same call frame — both surfaces come
        // from the same source value, so drift is structurally
        // impossible.
        //
        // The row is written with `state: Failed` (per ADR-0032 §5
        // distinguishes "operator stopped" → Terminated from
        // "driver could not start / budget exhausted" → Failed). The
        // reason carries the `RestartBudgetExhausted` summary so
        // existing wire consumers (snapshot's `last_transition.reason`)
        // see a coherent cause-class explanation alongside the
        // structured `terminal` field.
        Action::FinalizeFailed { alloc_id, terminal } => {
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                // No prior row — nothing to finalize against. This is
                // structurally rare (the JobLifecycle only emits
                // FinalizeFailed against a known-failed alloc) but
                // we tolerate it as a no-op so a level-triggered
                // re-enqueue against a torn-down alloc does not
                // surface as a ShimError.
                return Ok(());
            };
            let prior_state: AllocStateWire = prior_row.state.into();
            // Surface the terminal reason on the row's cause-class
            // field for wire compatibility. `RestartBudgetExhausted`
            // attaches the attempts count and a brief cause summary;
            // when `terminal` is something else (forward-compat for
            // future variants) we fall back to a generic
            // DriverInternalError detail derived from the prior row.
            let reason = match &terminal {
                Some(TerminalCondition::BackoffExhausted { attempts }) => {
                    Some(TransitionReason::RestartBudgetExhausted {
                        attempts: *attempts,
                        last_cause_summary: prior_row
                            .reason
                            .as_ref()
                            .map_or_else(|| "unknown".to_owned(), |r| format!("{r:?}")),
                    })
                }
                _ => prior_row.reason.clone(),
            };
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.job_id,
                prior_row.node_id,
                AllocState::Failed,
                tick,
                reason,
                None,
                terminal,
            );
            obs.write(ObservationRow::AllocStatus(row.clone())).await?;
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
            Ok(())
        }
        // Start: spawn the allocation via the driver and write a
        // Running AllocStatusRow on success. On StartRejected, write
        // a `Failed` row recording the typed cause-class
        // (ADR-0032 §5 + §4 Amendment).
        Action::StartAllocation { alloc_id, job_id, node_id, spec } => {
            // Read prior obs row before the driver call so we capture
            // the allocation's state before this transition. For first-
            // seen allocs (no prior row) default to Pending — consistent
            // with how existing tests model the initial transition.
            let prior_state: AllocStateWire = find_prior_alloc_row(obs, &alloc_id)
                .await?
                .map_or(AllocStateWire::Pending, |r| r.state.into());

            let driver_kind = driver.r#type();
            // Per ADR-0032 §4 Amendment 2026-04-30: classify the
            // driver's `StartRejected.reason` text into a typed
            // cause-class `TransitionReason` variant. State on
            // failure is `Failed` (not `Terminated`) — distinguishes
            // operator-stop from driver-could-not-start.
            let (state, reason, detail, source): (
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
            ) = match driver.start(&spec).await {
                Ok(_handle) => (
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                ),
                Err(DriverError::StartRejected { reason: reason_text, driver: drv }) => {
                    let cause = classify_driver_failure(&reason_text, drv, &spec.command);
                    (
                        AllocState::Failed,
                        Some(cause),
                        Some(reason_text),
                        TransitionSource::Driver(drv),
                    )
                }
                Err(other) => return Err(ShimError::Driver(other)),
            };
            // Per ADR-0037 §4: StartAllocation is never a terminal
            // claim — JobLifecycle emits FinalizeFailed on a separate
            // tick when restart budget is exhausted, and the row that
            // gets the BackoffExhausted terminal is written by that
            // arm. A successful start or a single mid-budget failed
            // start carries `terminal: None`.
            let row = build_alloc_status_row(
                alloc_id, job_id, node_id, state, tick, reason, detail, None,
            );
            obs.write(ObservationRow::AllocStatus(row.clone())).await?;
            emit_event(bus, build_lifecycle_event(&row, prior_state, source));
            Ok(())
        }
        // Restart: stop-then-start, reusing the same alloc id. Per
        // ADR-0023 §2 Restart is semantically `stop + start` against
        // the prior alloc. Per ADR-0031 §5 the action carries a
        // fully-populated `AllocationSpec` constructed in the
        // reconciler from the live `Job`; the shim reads it straight
        // off the action. `find_prior_alloc_row` is still needed to
        // recover `(job_id, node_id)` for the `AllocStatusRow` write.
        Action::RestartAllocation { alloc_id, spec } => {
            // Stop half — Phase 1 uses an empty AllocationHandle (no
            // pid tracking yet); the driver's `stop` is best-effort
            // and `NotFound` is silently absorbed (the alloc may have
            // already terminated on a prior failed start).
            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            let _ = driver.stop(&handle).await;

            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Err(ShimError::HandleMissing { alloc_id });
            };
            // Extract prior_state before prior_row moves into build_alloc_status_row.
            let prior_state: AllocStateWire = prior_row.state.into();

            let driver_kind = driver.r#type();
            // Failed restart — same cause-class classification path
            // as StartAllocation. Per ADR-0032 §5: state is `Failed`
            // on driver `StartRejected`.
            let (state, reason, detail, source): (
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
            ) = match driver.start(&spec).await {
                Ok(_handle) => (
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                ),
                Err(DriverError::StartRejected { reason: reason_text, driver: drv }) => {
                    let cause = classify_driver_failure(&reason_text, drv, &spec.command);
                    (
                        AllocState::Failed,
                        Some(cause),
                        Some(reason_text),
                        TransitionSource::Driver(drv),
                    )
                }
                Err(other) => return Err(ShimError::Driver(other)),
            };
            // Per ADR-0037 §4: RestartAllocation is never a terminal
            // claim. Same rationale as StartAllocation — restart is a
            // mid-budget recovery attempt; only `FinalizeFailed`
            // carries the BackoffExhausted terminal.
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.job_id,
                prior_row.node_id,
                state,
                tick,
                reason,
                detail,
                None,
            );
            obs.write(ObservationRow::AllocStatus(row.clone())).await?;
            emit_event(bus, build_lifecycle_event(&row, prior_state, source));
            Ok(())
        }
        // Stop: best-effort driver stop, then write a Terminated row
        // for the alloc. Per ADR-0023 §2 the stop path is best-effort
        // — if the driver no longer tracks the alloc (NotFound), the
        // shim still records Terminated so the next tick's hydrate
        // sees the alloc gone. Per-variant error isolation: a Stop
        // failure does NOT abort dispatch of subsequent actions.
        // Per ADR-0037 §4: the `terminal` field on the action carries
        // the reconciler's typed terminal claim. The shim threads it
        // onto BOTH `AllocStatusRow.terminal` (durable surface) AND
        // `LifecycleEvent.terminal` (broadcast surface) from the SAME
        // dispatch call frame — drift between the two is structurally
        // impossible because both are populated from the same
        // `terminal` value at the same source site.
        Action::StopAllocation { alloc_id, terminal } => {
            // Look up prior obs row to recover (job_id, node_id) for
            // the Terminated row we will write. If the alloc has no
            // obs row at all (e.g. the reconciler emitted Stop
            // without ever having seen the alloc Running) there is
            // nothing to write — return Ok.
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Ok(());
            };
            // Extract prior_state before prior_row moves into build_alloc_status_row.
            let prior_state: AllocStateWire = prior_row.state.into();

            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            // Driver stop is best-effort — NotFound and other
            // failures are absorbed; the Terminated row records the
            // outcome regardless. This mirrors the Restart variant's
            // stop-half pattern.
            let _ = driver.stop(&handle).await;
            // The `reason` field carries the cause-class summary on
            // the row; the `terminal` field is the reconciler's
            // typed terminal claim and is the source of truth for
            // *who* initiated the stop (Operator vs Reconciler).
            // Phase 1 surfaces the legacy `Stopped { by: Reconciler }`
            // reason here for backwards compatibility on the wire-side
            // `last_transition.reason`; the operator-attribution lands
            // exclusively on `terminal`.
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.job_id,
                prior_row.node_id,
                AllocState::Terminated,
                tick,
                Some(TransitionReason::Stopped {
                    by: overdrive_core::transition_reason::StoppedBy::Reconciler,
                }),
                None,
                terminal,
            );
            obs.write(ObservationRow::AllocStatus(row.clone())).await?;
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
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

/// Look up the LWW-winner observation row for `alloc_id`, used by the
/// Restart and Stop variants to recover `(job_id, node_id)` for the
/// Terminated row they write. Returns `Ok(None)` when no row exists —
/// callers decide whether that is an error (Restart) or a no-op (Stop).
async fn find_prior_alloc_row(
    obs: &dyn ObservationStore,
    alloc_id: &AllocationId,
) -> Result<Option<AllocStatusRow>, ShimError> {
    Ok(obs.alloc_status_row(alloc_id).await?)
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
