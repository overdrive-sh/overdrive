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

use std::sync::Arc;

use overdrive_core::TransitionReason;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::{AllocationHandle, Driver, DriverError, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_dataplane::allocators::{PersistentAllocatorError, PersistentServiceVipAllocator};
use tokio::sync::broadcast;

use crate::api::{AllocStateWire, TransitionSource};
use crate::journal::WorkflowId;
use crate::workflow_runtime::WorkflowEngine;

/// Per-arm dispatch for `Action::DataplaneUpdateService`. See
/// module docstring of [`dataplane_update_service`] for the
/// failure-surface contract per architecture.md § 7.
pub mod dataplane_update_service;

/// Per-arm dispatch for `Action::ReleaseServiceVip` per ADR-0049
/// (amended 2026-05-15). See module docstring of
/// [`release_service_vip`] for the lock discipline + idempotency
/// contract (service-vip-allocator step 03-02).
pub mod release_service_vip;

/// Per-arm dispatch for `Action::WriteServiceBackendRow` per
/// `docs/feature/backend-discovery-bridge-service-reachability/
/// design/architecture.md` § 4.4. The wrapper writes the row to the
/// ObservationStore; the bridge's next tick observes its own write
/// via the dedup fingerprint in [`BackendDiscoveryBridgeView`].
///
/// [`BackendDiscoveryBridgeView`]:
///     overdrive_core::reconcilers::backend_discovery_bridge::BackendDiscoveryBridgeView
pub mod write_service_backend_row;

/// Per-arm dispatch for `Action::EnqueueEvaluation` per UI-05 (the
/// `backend-discovery-bridge-service-reachability` step 02-04
/// architectural remediation). The wrapper submits an
/// `Evaluation { reconciler, target }` to the runtime's
/// [`EvaluationBroker`] so the named downstream reconciler ticks
/// against `target` on the next convergence cycle.
pub mod enqueue_evaluation;

/// Per-arm dispatch for `Action::RegisterLocalBackend` per ADR-0053
/// § 3. Invokes `Dataplane::register_local_backend` so the
/// cgroup_sock_addr program rewrites subsequent
/// `connect(vip:vip_port)` calls to the resolved backend address.
pub mod register_local_backend;

/// Per-arm dispatch for `Action::DeregisterLocalBackend` per ADR-0053
/// § 3. Invokes `Dataplane::deregister_local_backend` to remove the
/// LOCAL_BACKEND_MAP entry. Idempotent per the ADR-0053 § 2
/// trait contract.
pub mod deregister_local_backend;

/// Reconcile-output invariant validator — rejects post-`reconcile`
/// `Vec<Action>` returns that contain two or more write-actions
/// targeting the same service-LB VIP (see the module docstring on
/// [`validate`] for the full conflict taxonomy and fail-safe
/// semantics).
pub mod validate;

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
/// per-kind streaming event (`JobSubmitEvent` / `ServiceSubmitEvent`)
/// is constructed FROM a `LifecycleEvent` by the streaming loop. For
/// that reason `LifecycleEvent` derives only `Debug + Clone` — NOT
/// `Serialize`/`Deserialize`/`ToSchema`. That property is what the
/// trybuild fixture defends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    /// Allocation this transition concerns.
    pub alloc_id: AllocationId,
    /// Job the allocation belongs to.
    pub workload_id: WorkloadId,
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
    if let Some(rest) = text.strip_prefix("spawn ")
        && let Some((path, tail)) = split_once_after_path(rest)
    {
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

    // `cgroup setup failed: <kind>: <source>`
    if let Some(rest) = text.strip_prefix("cgroup setup failed: ")
        && let Some(idx) = rest.find(": ")
    {
        let kind = &rest[..idx];
        let source = &rest[idx + 2..];
        return TransitionReason::CgroupSetupFailed {
            kind: kind.to_owned(),
            source: source.to_owned(),
        };
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
// 9 row fields are intentional — the row is the durable wire shape and
// adding indirection would add noise without simplifying the call sites;
// ADR-0032 §3 + ADR-0037 §4 + slice 02-06 (stderr_tail propagation)
// each grew this list deliberately.
#[allow(clippy::too_many_arguments)]
fn build_alloc_status_row(
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: NodeId,
    state: AllocState,
    tick: &TickContext,
    reason: Option<TransitionReason>,
    detail: Option<String>,
    terminal: Option<TerminalCondition>,
    stderr_tail: Option<String>,
    kind: overdrive_core::aggregate::WorkloadKind,
    // Per the subsidiary fix to GAP-1: wall-clock at the
    // Pending → Running transition. Captured ONCE when this row
    // records a `state == AllocState::Running` for the first time;
    // preserved verbatim by every subsequent arm by reading the
    // prior row and forwarding the value. Typed `UnixInstant` —
    // unit + origin are encoded in the type. See the
    // `AllocStatusRowV1::started_at` docstring on the
    // input-vs-derived discipline.
    started_at: Option<overdrive_core::UnixInstant>,
) -> AllocStatusRow {
    let writer = node_id.clone();
    AllocStatusRow {
        alloc_id,
        workload_id,
        node_id,
        state,
        updated_at: timestamp_for(tick, writer),
        reason,
        detail,
        terminal,
        // Per ADR-0033 Amendment 2026-05-10 / slice 02-05 the
        // observation row's `stderr_tail` field carries the workload's
        // stderr verbatim (the `ExitObserver` boundary populates it
        // for crashed exits). Per slice 02-06 the action shim's
        // `FinalizeFailed` arm propagates this forward so the typed
        // terminal row inherits the prior attempt's stderr — without
        // this, the streaming layer's terminal `Failed` projection
        // sees `stderr_tail: None` even when the workload wrote to
        // stderr before exiting. Other arms still pass `None` (no
        // stderr was observed at those write sites).
        stderr_tail,
        kind,
        listeners: Vec::new(),
        started_at,
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
        workload_id: row.workload_id.clone(),
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
#[allow(
    clippy::too_many_arguments,
    reason = "Action-shim ports (Driver, ObservationStore, Dataplane, LifecycleEvent bus, ServiceVipAllocator) are required at dispatch per .claude/rules/development.md § Port-trait dependencies; bundling into a struct would make individual deps optional and defeat the explicit-injection invariant."
)]
pub async fn dispatch(
    actions: Vec<Action>,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
) -> Result<(), ShimError> {
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        let result = dispatch_single(
            action,
            driver,
            obs,
            dataplane,
            bus,
            tick,
            writer_node,
            &allocator,
            broker,
            workflow_engine,
        )
        .await;
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

/// Pre-flight: persist workflow-instance desired-intent for every
/// `Action::StartWorkflow` in `actions`, with **per-action isolation**.
///
/// For each action:
/// - `StartWorkflow`: attempt
///   `put(IntentKey::for_workflow_instance(correlation), spec.name bytes)`.
///   - `Ok`  → the action is pushed into the returned `dispatchable` set;
///     its desired-intent is now durable, so it is safe to drive the
///     engine for it.
///   - `Err` → the failure is recorded into `first_error` (only if no
///     earlier error exists, mirroring [`dispatch`]'s first-error
///     aggregation), and this `StartWorkflow` is **DROPPED** from the
///     batch. RATIONALE (load-bearing, ADR-0064 §5): dispatching
///     `engine.start` for a `StartWorkflow` whose intent did NOT persist
///     would leave a running instance that is NOT re-emittable on restart
///     — the exact invariant the intent-persist-before-engine-start
///     ordering protects. So drop it; the level-triggered
///     `WorkflowLifecycle` reconciler re-emits it next tick.
/// - any other action: always pushed into `dispatchable` unchanged.
///
/// Returns `(dispatchable, first_error)`. A failed intent write for one
/// `StartWorkflow` therefore does NOT discard the rest of the tick's
/// batch — every surviving action (including already-persisted earlier
/// `StartWorkflow`s and all non-workflow actions) still reaches
/// [`dispatch`]. The first intent error is surfaced to the caller.
pub(crate) async fn persist_workflow_intents(
    store: &dyn overdrive_core::traits::intent_store::IntentStore,
    actions: Vec<Action>,
) -> (Vec<Action>, Option<ShimError>) {
    use overdrive_core::aggregate::IntentKey;

    let mut dispatchable: Vec<Action> = Vec::with_capacity(actions.len());
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        match &action {
            Action::StartWorkflow { start, correlation } => {
                let key = IntentKey::for_workflow_instance(correlation);
                // Persist the FULL `WorkflowStart` spec (name + opaque CBOR
                // input) via the co-located rkyv-envelope codec — NOT the bare
                // name bytes (the #217 bug). Per `development.md` § "Persist
                // inputs, not derived state" the persisted intent is the inputs;
                // `from_store_bytes` rehydrates the whole spec on every tick so
                // a restart re-emits with the original input intact (ADR-0048
                // §4b, ADR-0065 §5).
                //
                // A serialiser failure is unreachable for a valid payload, but
                // it is handled the SAME way as a failed `put`: record the first
                // error and DROP this StartWorkflow — starting an instance whose
                // intent did not persist would leave it non-re-emittable, the
                // exact invariant the intent-persist-before-engine-start
                // ordering protects.
                let archived = match start.archive_for_store() {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        if first_error.is_none() {
                            first_error =
                                Some(ShimError::WorkflowIntent { message: err.to_string() });
                        }
                        continue;
                    }
                };
                match store.put(key.as_bytes(), archived.as_ref()).await {
                    // Intent durable — engine-start may proceed for this
                    // StartWorkflow.
                    Ok(()) => dispatchable.push(action),
                    // Intent write failed — DROP this StartWorkflow (it is
                    // not re-emittable if started without durable intent),
                    // record the first such error, and let the rest of the
                    // batch survive.
                    Err(err) => {
                        if first_error.is_none() {
                            first_error =
                                Some(ShimError::WorkflowIntent { message: err.to_string() });
                        }
                    }
                }
            }
            // Non-workflow actions carry no pre-flight intent; always
            // dispatch them.
            _ => dispatchable.push(action),
        }
    }

    (dispatchable, first_error)
}

/// AppState-aware dispatch that persists workflow-instance desired-intent
/// for every `Action::StartWorkflow` BEFORE handing the surviving actions
/// to [`dispatch`], threaded the real engine from `state.workflow_engine`
/// (ADR-0064 §5).
///
/// This is the production commit point for a reconciler-emitted
/// `StartWorkflow`: a committed action both (1) persists the instance's
/// desired-intent (`workflows/<correlation>` → the workflow spec inputs,
/// per `development.md` § "Persist inputs, not derived state") so the
/// `WorkflowLifecycle` reconciler's `hydrate_desired` can read it back on
/// every tick (and re-emit on restart), AND (2) drives the engine off the
/// shim. Intent persistence is FIRST so a crash between the two leaves the
/// instance re-emittable (the level-triggered reconciler re-drives it).
///
/// Per-action isolation holds **at the pre-flight stage too**: the
/// intent-persist loop ([`persist_workflow_intents`]) does NOT early-return
/// on the first failed `put`. A failed intent write drops only its own
/// `StartWorkflow` (which would not be re-emittable if started without
/// durable intent) and records the first such error; every other action in
/// the batch — already-persisted earlier `StartWorkflow`s and all
/// non-workflow actions — still flows into [`dispatch`], which applies its
/// own first-error aggregation over the survivors. The pre-flight error
/// (chronologically first) wins over the dispatch result on failure.
///
/// Mirrors the `StartAllocation → workload-intent → WorkloadLifecycle`
/// precedent for `StartWorkflow → workflow-intent → WorkflowLifecycle`.
///
/// # Errors
///
/// - [`ShimError::WorkflowIntent`] — persisting a workflow-instance intent
///   failed (the first such failure; the offending `StartWorkflow` is
///   dropped, the rest of the batch still dispatches).
/// - Any error [`dispatch`] surfaces (driver / observation / dataplane /
///   workflow-engine failure), with per-action isolation.
pub async fn dispatch_with_workflow_intent(
    actions: Vec<Action>,
    state: &crate::AppState,
    tick: &TickContext,
) -> Result<(), ShimError> {
    let (dispatchable, preflight_err) =
        persist_workflow_intents(state.store.as_ref(), actions).await;

    let dispatch_result = dispatch(
        dispatchable,
        state.driver.as_ref(),
        state.obs.as_ref(),
        state.dataplane.as_ref(),
        state.lifecycle_events.as_ref(),
        tick,
        &state.node_id,
        std::sync::Arc::clone(&state.allocator),
        state.runtime.broker_mutex(),
        Some(state.workflow_engine.as_ref()),
    )
    .await;

    // Pre-flight error wins (it is chronologically first); otherwise the
    // dispatch result (which itself carries dispatch()'s own first_error
    // aggregation over the surviving actions).
    preflight_err.map_or(dispatch_result, Err)
}

/// Dispatch a single action. Each variant is independent; the caller
/// loops over a `Vec<Action>` and aggregates errors.
#[allow(clippy::too_many_lines)]
#[allow(
    clippy::too_many_arguments,
    reason = "See dispatch() docstring — port-trait dependencies are required at call site, not optional via a builder."
)]
async fn dispatch_single(
    action: Action,
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: &Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
) -> Result<(), ShimError> {
    match action {
        // No-op (Action::Noop) and the Phase 3 HttpCall placeholder are
        // "no dispatch needed" — observation-only or deferred. (Per
        // ADR-0064 §5 `StartWorkflow` is NO LONGER in this no-op group —
        // it has its own arm below that hands the instance to the
        // WorkflowEngine off the shim.)
        Action::Noop | Action::HttpCall { .. } => Ok(()),
        // StartWorkflow: hand the instance to the WorkflowEngine off the
        // shim — exactly as Action::StartAllocation -> Driver::start
        // (ADR-0064 §5, DDD-5, the RATIFY-flagged engine↔reconciler
        // boundary). The engine spawns the author's `async fn run` as a
        // tracked async task and journals its terminal; it is NOT run as
        // a per-tick reconcile loop. The emitting workflow-lifecycle
        // reconciler stays pure-sync.
        //
        // When no engine is wired (the production reconciler-runtime path
        // until 01-06's full boot composition lands), the start is a
        // no-op for this tick — the level-triggered reconciler re-emits
        // it once the engine is composed. This mirrors the
        // StartAllocation arm's tolerance of a level-triggered re-enqueue.
        Action::StartWorkflow { start, correlation } => {
            let Some(engine) = workflow_engine else {
                return Ok(());
            };
            // Derive the per-instance journal id deterministically from the
            // action's correlation (ADR-0064 §5): the SAME instance always
            // resolves to the SAME `WorkflowId`, so a crash-resume
            // re-emit re-targets the same journal (the engine's
            // `load_journal` then RESUMES rather than cold-starts).
            let workflow_id = WorkflowId::for_correlation(&correlation);
            engine
                .start(&start, &correlation, &workflow_id)
                .await
                .map_err(|err| ShimError::WorkflowEngine { message: err.to_string() })
        }
        // FinalizeFailed: the reconciler has decided this allocation
        // has reached a terminal lifecycle moment. Per ADR-0037 §4 the
        // shim threads the `Action.terminal` value onto BOTH
        // `AllocStatusRow.terminal` (durable surface, written via
        // `obs.write`) AND `LifecycleEvent.terminal` (broadcast surface,
        // emitted via `bus.send`) in the same call frame — both
        // surfaces come from the same source value, so drift is
        // structurally impossible.
        //
        // The reconciler emits `FinalizeFailed` for several distinct
        // typed terminal claims, all flowing through the same arm
        // here unchanged:
        // - `BackoffExhausted { attempts }` — restart budget exceeded
        //   for a Service-shape workload (per existing ADR-0037 §4).
        // - `Completed { exit_code: 0 }` — Job-kind workload exited
        //   cleanly (per ADR-0037 Amendment 2026-05-10 / ADR-0047 §1,
        //   landed in slice 02-04).
        // - `Failed { exit_code: N }` — Job-kind workload exited with
        //   non-zero status (per ADR-0037 Amendment 2026-05-10).
        //
        // The row is written with `state: Failed` (per ADR-0032 §5
        // distinguishes "operator stopped" → Terminated from
        // "driver could not start / budget exhausted / Job exit" →
        // Failed). The `reason` field propagates the prior row's typed
        // leaf cause unchanged (e.g. `ExecBinaryNotFound { path }`);
        // the typed terminal claim lives on the orthogonal `terminal`
        // field per ADR-0037 §4. Synthesising a derived `reason` here
        // (e.g. `RestartBudgetExhausted { attempts, last_cause_summary }`)
        // would duplicate `attempts` (already on `terminal`) and
        // stringify the typed leaf cause into `last_cause_summary` —
        // both violations of `.claude/rules/development.md`
        // § "Persist inputs, not derived state". Wire consumers wanting
        // the "we gave up after N" / "exited cleanly" / "exited with N"
        // framing render it from `terminal` directly. The streaming
        // dispatcher's `workload_event_from_terminal` projection maps
        // each `TerminalCondition` to its `JobSubmitEvent`
        // (`Completed → Succeeded`, `Failed → Failed`,
        // `BackoffExhausted → Failed`, `Stopped → Stopped`).
        Action::FinalizeFailed { alloc_id, terminal } => {
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                // No prior row — nothing to finalize against. This is
                // structurally rare (the WorkloadLifecycle only emits
                // FinalizeFailed against a known-failed alloc) but
                // we tolerate it as a no-op so a level-triggered
                // re-enqueue against a torn-down alloc does not
                // surface as a ShimError.
                return Ok(());
            };
            let prior_state: AllocStateWire = prior_row.state.into();
            // Per slice 02-06: propagate the prior row's `stderr_tail`
            // forward onto the typed terminal row so the streaming
            // layer's `JobSubmitEvent::Failed` projection can render
            // the workload's stderr verbatim. The exit observer (per
            // slice 02-05 / ADR-0033 Amendment 2026-05-10) populates
            // `stderr_tail` on the per-attempt failure row; without
            // this propagation, the FinalizeFailed write would
            // overwrite that row with `stderr_tail: None`, breaking
            // S-02-02's stderr-tail rendering assertion.
            let prior_stderr_tail = prior_row.stderr_tail.clone();
            // FinalizeFailed is a terminal claim — preserve the prior
            // row's `started_at` verbatim. If the prior row never
            // reached Running (Pending only), `started_at` is `None`
            // and stays `None` here. Same forward-carry pattern as
            // `stderr_tail` / `detail` / `kind`.
            let prior_started_at = prior_row.started_at;
            // GAP-9 — a `Stable` terminal is a SUCCESS claim, not a
            // failure: the Service alloc has passed its startup probes
            // and is healthily serving. It MUST remain `Running` so the
            // BackendDiscoveryBridge (which renders backends from the
            // `state == Running` set) keeps the backend registered.
            // Every other `TerminalCondition` (ServiceFailed /
            // BackoffExhausted / Completed / Stopped …) is a genuine
            // terminal and lands `Failed`.
            //
            // Pre-GAP-9 the `service-lifecycle` reconciler never ran in
            // production, so `FinalizeFailed { Stable }` was never
            // emitted and this arm only ever saw real failures — the
            // unconditional `AllocState::Failed` was latently wrong but
            // unreachable. GAP-9 makes the Stable path live, surfacing
            // the bug as a walking-skeleton backend-drop; this guard
            // closes it. The terminal CLAIM (`terminal`) is still
            // written verbatim onto the row + lifecycle event, so the
            // streaming layer's `ServiceSubmitEvent::Stable` projection
            // (which reads `event.terminal`, not the state) is unchanged.
            let finalized_state = if matches!(terminal, Some(TerminalCondition::Stable { .. })) {
                prior_row.state
            } else {
                AllocState::Failed
            };
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.workload_id,
                prior_row.node_id,
                finalized_state,
                tick,
                prior_row.reason.clone(),
                // Propagate the prior row's verbatim driver text. The
                // last failed Start/RestartAllocation populates `detail`
                // with the `DriverError::StartRejected.reason_text`
                // (per the StartAllocation arm above); the streaming
                // surface's failed-terminal rendering reads this
                // through `event.detail`. Hardcoding `None` here would
                // drop the operator-visible cause text on the
                // budget-exhausted terminal, even though the prior
                // attempt rows carry it.
                prior_row.detail.clone(),
                terminal,
                prior_stderr_tail,
                prior_row.kind,
                prior_started_at,
            );
            obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
            // Service-health-check-probes step 01-03d / ADR-0054 § 2:
            // FinalizeFailed is a terminal claim (BackoffExhausted /
            // Completed / Failed) — fire the terminal lifecycle hook
            // so any probe supervisor spawned earlier in the alloc's
            // lifetime is cleaned up. Default no-op when no
            // ProbeRunner is wired.
            driver.on_alloc_terminal(&row.alloc_id);
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
            Ok(())
        }
        // Start: spawn the allocation via the driver and write a
        // Running AllocStatusRow on success. On StartRejected, write
        // a `Failed` row recording the typed cause-class
        // (ADR-0032 §5 + §4 Amendment).
        Action::StartAllocation { alloc_id, workload_id, node_id, spec, kind } => {
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
            let (handle_opt, state, reason, detail, source): (
                Option<AllocationHandle>,
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
            ) = match driver.start(&spec).await {
                Ok(handle) => (
                    Some(handle),
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                ),
                Err(DriverError::StartRejected { reason: reason_text, driver: drv }) => {
                    let cause = classify_driver_failure(&reason_text, drv, &spec.command);
                    (
                        None,
                        AllocState::Failed,
                        Some(cause),
                        Some(reason_text),
                        TransitionSource::Driver(drv),
                    )
                }
                Err(other) => return Err(ShimError::Driver(other)),
            };
            // Per ADR-0037 §4: StartAllocation is never a terminal
            // claim — WorkloadLifecycle emits FinalizeFailed on a separate
            // tick when restart budget is exhausted, and the row that
            // gets the BackoffExhausted terminal is written by that
            // arm. A successful start or a single mid-budget failed
            // start carries `terminal: None`.
            //
            // Subsidiary GAP-1 fix: capture the wall-clock at the
            // Pending → Running transition. On a successful start
            // (`state == AllocState::Running`) the row carries
            // `Some(tick.now_unix)` — the same `Clock` port DST
            // already controls. On a failed start
            // (`state == AllocState::Failed`) the alloc never
            // reached Running and there is no "started at"
            // wall-clock; the row carries `None`. The reconciler's
            // EarlyExit / StartupProbeFailed / Stable gates branch
            // on `None` explicitly (no silent-zero collapse).
            let started_at = if state == AllocState::Running { Some(tick.now_unix) } else { None };
            let row = build_alloc_status_row(
                alloc_id,
                workload_id,
                node_id,
                state,
                tick,
                reason,
                detail,
                None,
                None,
                kind,
                started_at,
            );
            // Fires the Running-confirmed gate exposed by Driver::start.
            // Required for liveness — the watcher parks on this gate
            // before emitting ExitEvent. The two firing sites
            // (post-Running-Ok and post-degraded-escalation) are jointly
            // load-bearing; missing either leaks the watcher. Per RCA
            // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
            // (Solution 1'). Standard convention: fire the gate
            // IMMEDIATELY after `obs.write` resolves Ok, BEFORE the
            // lifecycle-event emit.
            //
            // Failed-row branch (state == Failed, handle_opt == None) does
            // NOT fire the gate per AC2 — the alloc never reached Running
            // and no watcher exists for a never-spawned alloc. The
            // driver's `release_for_exit_emission` is idempotent for
            // unknown allocs anyway; the explicit None-check here makes
            // the AC contract structurally readable at the call site.
            obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
            if state == AllocState::Running {
                if let Some(handle) = &handle_opt {
                    driver.release_for_exit_emission(handle);
                }
                // Service-health-check-probes step 01-03d / ADR-0054
                // § 2: fire the lifecycle hook so the driver can
                // dispatch to its configured `ProbeRunner`. Default
                // no-op for SimDriver and any driver wired without
                // a probe runner.
                driver.on_alloc_running(&spec);
            }
            emit_event(bus, build_lifecycle_event(&row, prior_state, source));
            Ok(())
        }
        // Restart: stop-then-start, reusing the same alloc id. Per
        // ADR-0023 §2 Restart is semantically `stop + start` against
        // the prior alloc. Per ADR-0031 §5 the action carries a
        // fully-populated `AllocationSpec` constructed in the
        // reconciler from the live `Job`; the shim reads it straight
        // off the action. `find_prior_alloc_row` is still needed to
        // recover `(workload_id, node_id)` for the `AllocStatusRow` write.
        // `reason` (Some(LivenessExhausted) for service-lifecycle
        // liveness restarts; None for the WorkloadLifecycle crash loop)
        // is ignored here: per ADR-0023 §2 / ADR-0037 §4 a restart is
        // semantically `stop + start` regardless of cause, and
        // RestartAllocation never carries a terminal claim. The cause
        // surfaces to operators through the reconciler's own
        // observation/render path, not the shim's stop+start.
        Action::RestartAllocation { alloc_id, spec, kind, reason: _ } => {
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
            let (handle_opt, state, reason, detail, source): (
                Option<AllocationHandle>,
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
            ) = match driver.start(&spec).await {
                Ok(handle) => (
                    Some(handle),
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                ),
                Err(DriverError::StartRejected { reason: reason_text, driver: drv }) => {
                    let cause = classify_driver_failure(&reason_text, drv, &spec.command);
                    (
                        None,
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
            //
            // Per ADR-0047 §1 / step 02-02 [D4]: kind comes from the
            // emitting action (sourced by the reconciler from the
            // hydrated `WorkloadLifecycleState.workload_kind`), NOT from
            // the prior row. The action's kind is the authoritative
            // value at every restart write.
            //
            // Subsidiary GAP-1 fix: a restart is a fresh process spawn
            // (`stop + start` per ADR-0023 §2) — capture a fresh
            // wall-clock for the new Pending → Running transition.
            // The reconciler's startup-probe / EarlyExit gates measure
            // elapsed since THIS process reached Running, not since
            // the prior (now-stopped) process did. On a failed restart
            // (`state == AllocState::Failed`) no new Running state was
            // reached; carry `None` forward — and a Phase-1
            // restart-rejected row that does not observe Running is
            // semantically equivalent to "never started."
            let started_at = if state == AllocState::Running {
                Some(tick.now_unix)
            } else {
                // Restart was rejected — never observed Running on
                // this attempt. Preserve the prior row's value (if
                // any) so a downstream FinalizeFailed terminal still
                // carries the prior generation's "started at" if it
                // ever reached Running.
                prior_row.started_at
            };
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.workload_id,
                prior_row.node_id,
                state,
                tick,
                reason,
                detail,
                None,
                None,
                kind,
                started_at,
            );
            // Fires the Running-confirmed gate exposed by Driver::start.
            // Required for liveness — the watcher parks on this gate
            // before emitting ExitEvent. The two firing sites
            // (post-Running-Ok and post-degraded-escalation) are jointly
            // load-bearing; missing either leaks the watcher. Per RCA
            // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
            // (Solution 1'). Symmetric with the StartAllocation arm
            // above. Failed-row branch (state == Failed, handle_opt ==
            // None) does NOT fire — restart-rejected reuses the prior
            // alloc id, but the new watcher was never spawned, so no
            // gate is awaited.
            obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
            // mutants::skip — Running gate exercised by exit_observer_running_gate integration test; dispatch_single requires full Driver+ObservationStore wiring
            if state == AllocState::Running {
                if let Some(handle) = &handle_opt {
                    driver.release_for_exit_emission(handle);
                }
                // Service-health-check-probes step 01-03d / ADR-0054
                // § 2: symmetric with the StartAllocation arm above.
                driver.on_alloc_running(&spec);
            }
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
            // Look up prior obs row to recover (workload_id, node_id) for
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
            // Subsidiary GAP-1 fix: StopAllocation is a terminal
            // operator-initiated stop — preserve the prior row's
            // `started_at` verbatim so downstream consumers
            // (e.g. settled-in / uptime renderers) still see when
            // the alloc reached Running. If it never reached Running
            // (Pending → Stopped), the prior value is `None` and
            // stays `None`.
            let prior_started_at = prior_row.started_at;
            let row = build_alloc_status_row(
                alloc_id,
                prior_row.workload_id,
                prior_row.node_id,
                AllocState::Terminated,
                tick,
                Some(TransitionReason::Stopped {
                    by: overdrive_core::transition_reason::StoppedBy::Reconciler,
                }),
                None,
                terminal,
                None,
                prior_row.kind,
                prior_started_at,
            );
            obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
            // Service-health-check-probes step 01-03d / ADR-0054 § 2:
            // fire the terminal lifecycle hook so the driver can
            // cancel every per-probe task spawned under this
            // alloc's supervisor. Default no-op for drivers wired
            // without a `ProbeRunner`. We use `row.alloc_id` rather
            // than the moved `alloc_id` binding because the latter
            // was consumed by `build_alloc_status_row` above.
            driver.on_alloc_terminal(&row.alloc_id);
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
            Ok(())
        }
        // phase-2-xdp-service-map Slice 08 (US-08; ASR-2.2-04) —
        // The shim invokes `Dataplane::update_service(...)` via the
        // canonical per-arm dispatch fn at
        // `dataplane_update_service::dispatch`, which writes the
        // outcome row to `service_hydration_results` per
        // architecture.md § 7 *Failure surface*. A
        // `Dataplane::update_service` failure does NOT surface as
        // `ShimError` — it lands as a `Failed` observation row and
        // dispatch returns `Ok(DispatchOutcome::Failed)`. Only an
        // ObservationStore write failure surfaces as
        // `ShimError::Observation`.
        action @ Action::DataplaneUpdateService { .. } => {
            dataplane_update_service::dispatch(&action, dataplane, obs, tick, writer_node)
                .await
                .map_err(|e| match e {
                    dataplane_update_service::ServiceHydrationDispatchError::ObservationWrite {
                        source,
                    } => ShimError::Observation(source),
                    dataplane_update_service::ServiceHydrationDispatchError::Ipv6Unsupported {
                        ..
                    } => {
                        unreachable!(
                            "Ipv6Unsupported is handled inside dispatch — it writes \
                             a Failed row and returns Ok(DispatchOutcome::Failed)"
                        )
                    }
                })?;
            Ok(())
        }
        // service-vip-allocator step 03-02 — real dispatch arm per
        // ADR-0049 (amended 2026-05-15). Threads the digest +
        // correlation into the per-arm `release_service_vip::dispatch`
        // which owns the `tokio::sync::Mutex` guard + the
        // `PersistentServiceVipAllocator::release` call (memo +
        // IntentStore allocator_entries row removal in
        // fsync-then-memory order). On Ok, the released VIP returns to
        // the pool for reallocation on the next `allocate(&fresh)`. On
        // Err, the typed `PersistentAllocatorError` surfaces via
        // `ShimError::AllocatorRelease { #[from] source }` so callers
        // can `matches!` on the structured cause without re-parsing
        // `Display` (per `.claude/rules/development.md` § "Never
        // flatten a typed error to `Internal(String)`").
        Action::ReleaseServiceVip { spec_digest, correlation } => {
            release_service_vip::dispatch(&spec_digest, &correlation, allocator).await
        }
        // backend-discovery-bridge-service-reachability step 01-04 —
        // GREEN. The per-arm dispatch wrapper in
        // `crates/overdrive-control-plane/src/action_shim/
        // write_service_backend_row.rs` writes the row via
        // `ObservationStore::write(ObservationRow::ServiceBackend(row))`.
        // No correlation-driven follow-up at the shim level — the
        // bridge's next tick reads the row stream (transitively
        // through the runtime's hydrate path) and observes its own
        // write via the dedup fingerprint in
        // `BackendDiscoveryBridgeView::last_written_fingerprint`. An
        // `ObservationStore::write` failure surfaces as
        // `ShimError::Observation` via the typed `#[from]` variant
        // per `.claude/rules/development.md` § Errors / pass-through.
        action @ Action::WriteServiceBackendRow { .. } => {
            write_service_backend_row::dispatch(&action, obs).await.map_err(ShimError::from)
        }
        // backend-discovery-bridge-service-reachability UI-05 —
        // cross-reconciler handoff at the action boundary. The
        // wrapper takes a brief lock-grab-submit-release on the
        // broker mutex; per `.claude/rules/development.md`
        // § Concurrency & async the guard is dropped before any
        // subsequent `.await` (the wrapper is a sync function and
        // the per-action loop awaits between iterations).
        action @ Action::EnqueueEvaluation { .. } => {
            let mut guard = broker.lock();
            enqueue_evaluation::dispatch(&action, &mut guard);
            drop(guard);
            Ok(())
        }
        // ADR-0053 § 3 — same-host backend delivery via
        // cgroup_sock_addr. The hydrator's classifier emits this
        // variant for every backend whose IP matches `host_ipv4`
        // (Phase 1 single-node: every Running alloc). The shim
        // invokes `Dataplane::register_local_backend` which writes
        // the LOCAL_BACKEND_MAP entry the cgroup_connect4_service
        // program reads on every connect(2). No observation row
        // dispatch — the cgroup hook is not an HTTP-call surface.
        action @ Action::RegisterLocalBackend { .. } => {
            register_local_backend::dispatch(&action, dataplane).await.map_err(ShimError::from)?;
            Ok(())
        }
        action @ Action::DeregisterLocalBackend { .. } => {
            deregister_local_backend::dispatch(&action, dataplane)
                .await
                .map_err(ShimError::from)?;
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
/// Restart and Stop variants to recover `(workload_id, node_id)` for the
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
    /// The [`PersistentServiceVipAllocator::release`] call failed —
    /// typically a byte-level `IntentStore::delete` rejection (disk
    /// full, file corruption, redb internal error). Pass-through
    /// `#[from]` per `.claude/rules/development.md` § Errors so the
    /// typed cause is preserved end-to-end. Service-vip-allocator
    /// step 03-02 / ADR-0049.
    #[error("release_service_vip failed: {source}")]
    AllocatorRelease {
        /// The underlying typed error from the allocator.
        #[from]
        source: PersistentAllocatorError,
    },
    /// `register_local_backend` shim dispatch failed (ADR-0053 § 3).
    /// Pass-through `#[from]` preserves the typed
    /// `DataplaneError::LocalBackendInsert` cause.
    #[error("register_local_backend dispatch failed")]
    RegisterLocalBackend(#[from] register_local_backend::RegisterLocalBackendDispatchError),
    /// `deregister_local_backend` shim dispatch failed (ADR-0053 § 3).
    #[error("deregister_local_backend dispatch failed")]
    DeregisterLocalBackend(#[from] deregister_local_backend::DeregisterLocalBackendDispatchError),
    /// The `WorkflowEngine::start` dispatch failed (ADR-0064 §5) — the
    /// engine could not resolve the workflow kind or load its journal.
    /// The shim surfaces this as a typed `ShimError` rather than swallow
    /// it, mirroring the StartAllocation driver-failure surface.
    #[error("workflow engine start failed: {message}")]
    WorkflowEngine {
        /// Cause string from the engine's typed `WorkflowEngineError`.
        message: String,
    },

    /// Persisting a workflow-instance desired-intent on
    /// `Action::StartWorkflow` commit failed (ADR-0064 §5). The intent
    /// write is the `hydrate_desired` SSOT the workflow-lifecycle
    /// reconciler reads back; a failure here means the instance would not
    /// be re-emittable on restart, so it is surfaced rather than dropped.
    #[error("workflow intent persistence failed: {message}")]
    WorkflowIntent {
        /// Cause string from the `IntentStore` error.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    //! Pre-flight per-action isolation regression (ADR-0064 §5).
    //!
    //! Drives the pure-async helper [`persist_workflow_intents`] directly
    //! — it IS the driving port for the intent-persist pre-flight stage.
    //! The observable universe is `(dispatchable, first_error)` plus the
    //! intent store's persisted bytes (read back via `get`). No real
    //! redb / FS — a fault-injecting in-memory `IntentStore` keeps this in
    //! the default lane.

    // Test-double constructors + `.expect()` on infallible test reads are
    // idiomatic in test code; the const-fn / expect-used lints add ceremony
    // with no test value (mirrors the file-level allow on the sibling
    // `tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs`).
    #![allow(clippy::expect_used, clippy::missing_const_for_fn)]

    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use bytes::Bytes;
    use futures::Stream;
    use overdrive_core::aggregate::IntentKey;
    use overdrive_core::id::{ContentHash, CorrelationKey};
    use overdrive_core::traits::intent_store::{
        IntentStore, IntentStoreError, PutOutcome, StateSnapshot, TxnOp, TxnOutcome,
    };
    use overdrive_core::workflow::{WorkflowName, WorkflowStart};

    use super::{Action, ShimError, persist_workflow_intents};

    /// In-memory `IntentStore` that fails `put` for one configured
    /// "poison" key and otherwise stores the bytes. `get` reflects what
    /// actually persisted so the test can assert which intents landed.
    /// Ordered map per `.claude/rules/development.md` § "Ordered-collection
    /// choice".
    struct FaultInjectingIntentStore {
        stored: parking_lot::Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
        poison_key: Vec<u8>,
    }

    impl FaultInjectingIntentStore {
        fn with_poison(poison_key: Vec<u8>) -> Self {
            Self { stored: parking_lot::Mutex::new(BTreeMap::new()), poison_key }
        }
    }

    #[async_trait]
    impl IntentStore for FaultInjectingIntentStore {
        async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
            Ok(self.stored.lock().get(key).map(|v| Bytes::copy_from_slice(v)))
        }

        async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError> {
            if key == self.poison_key.as_slice() {
                return Err(IntentStoreError::Busy);
            }
            self.stored.lock().insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        async fn put_if_absent(
            &self,
            _key: &[u8],
            _value: &[u8],
        ) -> Result<PutOutcome, IntentStoreError> {
            Ok(PutOutcome::Inserted)
        }

        async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError> {
            self.stored.lock().remove(key);
            Ok(())
        }

        async fn txn(&self, _ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
            Ok(TxnOutcome::Committed)
        }

        async fn watch(
            &self,
            _prefix: &[u8],
        ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError>
        {
            Ok(Box::new(futures::stream::empty()))
        }

        async fn scan_prefix(
            &self,
            prefix: &[u8],
        ) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError> {
            Ok(self
                .stored
                .lock()
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| (Bytes::copy_from_slice(k), Bytes::copy_from_slice(v)))
                .collect())
        }

        async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
            Ok(StateSnapshot::from_parts(0, Vec::new(), Vec::new()))
        }

        async fn bootstrap_from(&self, _snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
            Ok(())
        }
    }

    fn start_workflow(slug: &str) -> (Action, CorrelationKey, WorkflowStart) {
        let spec = WorkflowStart {
            name: WorkflowName::new("provision-record").expect("valid kebab name"),
            input: Vec::new(),
        };
        // Correlation is derived from the workflow-KIND identity (the spec
        // name) — unrelated to the persisted intent VALUE, which is now the
        // full `archive_for_store` envelope (the #217 fix above), never the
        // name bytes.
        let kind_name = spec.name.as_str();
        let kind_digest = ContentHash::of(kind_name.as_bytes());
        let correlation = CorrelationKey::derive(slug, &kind_digest, "start-workflow");
        let action =
            Action::StartWorkflow { start: spec.clone(), correlation: correlation.clone() };
        (action, correlation, spec)
    }

    /// One failed intent write (for B) must NOT discard the rest of the
    /// batch: A, C, and the interleaved non-workflow action survive into
    /// `dispatchable`; B is dropped; the first error surfaces; and exactly
    /// A's and C's intents persist — never B's. This is the per-action
    /// isolation the pre-flight `?` early-return previously bypassed.
    #[tokio::test]
    async fn preflight_isolation_one_failed_intent_does_not_drop_the_batch() {
        let (action_a, corr_a, _) = start_workflow("wf-a-0001");
        let (action_b, corr_b, _) = start_workflow("wf-b-0002");
        let (action_c, corr_c, _) = start_workflow("wf-c-0003");

        let key_a = IntentKey::for_workflow_instance(&corr_a).as_bytes().to_vec();
        let key_b = IntentKey::for_workflow_instance(&corr_b).as_bytes().to_vec();
        let key_c = IntentKey::for_workflow_instance(&corr_c).as_bytes().to_vec();

        // Fail ONLY B's intent write.
        let store = FaultInjectingIntentStore::with_poison(key_b.clone());

        // Interleave a non-workflow action so the test also pins that
        // non-StartWorkflow actions always survive.
        let actions = vec![action_a.clone(), action_b, Action::Noop, action_c.clone()];

        let (dispatchable, first_error) = persist_workflow_intents(&store, actions).await;

        // 1. The first error is a WorkflowIntent failure.
        assert!(
            matches!(first_error, Some(ShimError::WorkflowIntent { .. })),
            "B's failed intent write must surface as ShimError::WorkflowIntent; \
             got {first_error:?}"
        );

        // 2. dispatchable contains A, C, and the Noop — and NOT B.
        assert!(
            dispatchable.contains(&action_a),
            "A (intent persisted) must survive into dispatchable; got {dispatchable:?}"
        );
        assert!(
            dispatchable.contains(&action_c),
            "C (intent persisted) must survive into dispatchable; got {dispatchable:?}"
        );
        assert!(
            dispatchable.contains(&Action::Noop),
            "the non-workflow action must always survive into dispatchable; \
             got {dispatchable:?}"
        );
        assert!(
            !dispatchable.iter().any(|a| matches!(
                a,
                Action::StartWorkflow { correlation, .. } if *correlation == corr_b
            )),
            "B (intent write failed) must be DROPPED from dispatchable — starting it \
             would leave a non-re-emittable instance; got {dispatchable:?}"
        );

        // 3. The store persisted A's and C's intents, and NOT B's.
        assert!(store.get(&key_a).await.expect("get a").is_some(), "A's intent must be persisted");
        assert!(store.get(&key_c).await.expect("get c").is_some(), "C's intent must be persisted");
        assert!(
            store.get(&key_b).await.expect("get b").is_none(),
            "B's intent must NOT be persisted (its put failed)"
        );
    }
}
