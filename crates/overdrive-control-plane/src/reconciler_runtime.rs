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
//! Phase 1 shape: the runtime owns a `BTreeMap<ReconcilerName,
//! AnyReconciler>` keyed by the canonical name, plus an
//! `EvaluationBroker` behind `&self`. The `BTreeMap` choice — over
//! `HashMap` — is deliberate: registry iteration must be deterministic
//! across runtime constructions because [`Self::registered`] is
//! consumed by the operator-facing `cluster status` JSON output, and
//! `HashMap`'s `RandomState` hasher would put per-process-randomised
//! key order on the wire (see ADR-0013 §8 storm-proofing rationale and
//! the project-wide ordered-collection-as-nondeterminism rule in
//! `.claude/rules/development.md`). Registration eagerly derives the
//! per-reconciler libSQL path via
//! [`crate::libsql_provisioner::provision_db_path`] — the DB itself is
//! opened lazily by callers that need it (Phase 3+). Provisioning the
//! path at register time surfaces invalid `data_dir`s (permission
//! denied, traversal attempt) at registration rather than deferred
//! until first use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{IntentKey, Job, Node};
use overdrive_core::id::{JobId, NodeId};
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, JobLifecycleState, JobLifecycleView,
    LibsqlHandle, ReconcilerName, TargetResource, TickContext,
};
use overdrive_core::traits::intent_store::IntentStore;

use crate::AppState;
use crate::action_shim;
use crate::error::ControlPlaneError;
use crate::eval_broker::{Evaluation, EvaluationBroker};
use crate::libsql_provisioner::provision_db_path;

/// Registry + broker + libSQL path owner.
pub struct ReconcilerRuntime {
    /// Canonicalised data directory under which per-reconciler libSQL
    /// files live at `<data_dir>/reconcilers/<name>/memory.db`.
    data_dir: PathBuf,
    /// Registry keyed on canonical reconciler name. Duplicate
    /// registration is rejected with `ControlPlaneError::Conflict`.
    reconcilers: BTreeMap<ReconcilerName, AnyReconciler>,
    /// Cancelable-eval-set evaluation broker per ADR-0013 §8.
    ///
    /// Wrapped in [`parking_lot::Mutex`] per
    /// `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2):
    /// `submit_job` / `stop_job` (handler path) and the spawn loop in
    /// [`crate::run_server_with_obs_and_driver`] both call broker
    /// methods that need `&mut self` (`submit`, `drain_pending`).
    /// Since `state.runtime` is `Arc<ReconcilerRuntime>`, neither
    /// caller has unique ownership; a sync mutex is the smallest
    /// adapter. Per `.claude/rules/development.md` § Concurrency &
    /// async — `parking_lot` over `std::sync` because the critical
    /// sections are straight-line and panic-free; no `.await` is
    /// ever held across the lock (broker methods are sync; the
    /// spawn loop drains into a local `Vec<Evaluation>` and drops
    /// the guard before per-eval `.await`).
    broker: parking_lot::Mutex<EvaluationBroker>,
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
        Ok(Self {
            data_dir: canon,
            reconcilers: BTreeMap::new(),
            broker: parking_lot::Mutex::new(EvaluationBroker::new()),
        })
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

    /// Registered reconciler names in canonical (Ord) order —
    /// deterministic across runtime constructions given the same
    /// registration sequence.
    #[must_use]
    pub fn registered(&self) -> Vec<ReconcilerName> {
        self.reconcilers.keys().cloned().collect()
    }

    /// Borrow the evaluation broker through the per-runtime mutex.
    ///
    /// Returns a [`parking_lot::MutexGuard`] which derefs to
    /// `&EvaluationBroker` AND `&mut EvaluationBroker` so both reads
    /// (`counters`) and writes (`submit`, `drain_pending`) work
    /// uniformly through the same accessor. Callers MUST drop the
    /// guard before any `.await` per the no-locks-across-await rule
    /// in `.claude/rules/development.md` § Concurrency & async; the
    /// spawn loop in [`crate::run_server_with_obs_and_driver`] drains
    /// into a local `Vec<Evaluation>` and drops the guard before
    /// dispatching.
    pub fn broker(&self) -> parking_lot::MutexGuard<'_, EvaluationBroker> {
        self.broker.lock()
    }

    /// Iterate the registered reconcilers. Used by the ADR-0017
    /// `reconciler_is_pure` invariant to twin-invocation-check every
    /// reconciler in the registry from a single harness entry point.
    pub fn reconcilers_iter(&self) -> impl Iterator<Item = &AnyReconciler> {
        self.reconcilers.values()
    }

    /// Look up a reconciler by canonical name. O(log N) keyed lookup
    /// over the underlying `BTreeMap`. Used by the per-tick dispatch
    /// path in [`run_convergence_tick`] — each drained Evaluation
    /// names exactly one reconciler (ADR-0013 §8 / whitepaper §18),
    /// so dispatch is a keyed lookup, not a registry scan.
    #[must_use]
    pub fn get(&self, name: &ReconcilerName) -> Option<&AnyReconciler> {
        self.reconcilers.get(name)
    }
}

// ---------------------------------------------------------------------------
// phase-1-first-workload — slice 3 (US-03) — runtime convergence tick loop
//
// Per ADR-0023 + whitepaper §18: the runtime owns the `.await` on
// hydrate, the diff-and-persist of returned views, and the dispatch
// of emitted actions. Each tick: hydrate_desired → hydrate_actual →
// reconcile → action_shim::dispatch.
// ---------------------------------------------------------------------------

/// Default tick cadence — how often the runtime ticks the broker in
/// production. Per ADR-0023 + .claude/rules/development.md.
pub const DEFAULT_TICK_CADENCE: Duration = Duration::from_millis(100);

/// Drive ONE convergence tick against `target` for the reconciler
/// named in `reconciler_name`.
///
/// The reconciler is looked up via [`ReconcilerRuntime::get`] (O(log N)
/// keyed lookup over the `BTreeMap` registry); if
/// not registered, the function logs a structured warning and returns
/// Ok cleanly (the reconciler may have been deregistered between
/// submit and drain — Phase 2+ concern, defensively handled).
///
/// Returns `Err(ShimError)` only when the action shim cannot resolve
/// a dispatched action into an observation row (the shim itself is
/// the boundary the runtime expects to keep healthy). Other errors
/// (hydrate, libSQL) are surfaced through the same channel for now;
/// Phase 2+ refines the error taxonomy.
///
/// Spawned by [`crate::run_server_with_obs_and_driver`] as a tokio
/// task that drains the [`crate::eval_broker::EvaluationBroker`] each
/// tick (`config.tick_cadence`, default [`DEFAULT_TICK_CADENCE`]) and
/// dispatches one call per pending [`crate::eval_broker::Evaluation`].
/// Each drained Evaluation runs exactly one reconciler — the one it
/// names. Tests call this directly per-tick to drive the tick loop
/// deterministically without booting the full server.
///
/// Self-re-enqueue: when `reconcile` returns at least one
/// non-`Action::Noop` action (i.e. desired ≠ actual, the cluster has
/// not converged yet), this function re-submits under the same
/// `(reconciler_name, target)` key the inbound Evaluation carried —
/// the broker collapses redundant submits at the same key per
/// ADR-0013 §8 / whitepaper §18. Without this, the reconciler runs
/// once after submit, the broker drains empty, and convergence stalls.
///
/// Shutdown ordering: [`crate::ServerHandle::shutdown`] cancels the
/// convergence task FIRST (via `convergence_shutdown` token), awaits
/// its join, THEN triggers axum graceful shutdown. The reverse
/// ordering risks reconciler tasks holding `Arc<dyn Driver>` while
/// axum-shutting-down state is accessed.
///
/// # Errors
///
/// Returns [`ConvergenceError`] when hydrate, reconcile, or dispatch
/// fail in a way the runtime cannot represent as observation.
pub async fn run_convergence_tick(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    now: Instant,
    tick_n: u64,
    deadline: Instant,
) -> Result<(), ConvergenceError> {
    // Look up the named reconciler from the registered set. The
    // Evaluation's `reconciler` field is the broker's key half and
    // is now the dispatch target. Each drained Evaluation runs
    // exactly one reconciler — the one it names. O(log N) keyed
    // lookup over the BTreeMap registry — not a linear scan.
    let Some(reconciler) = state.runtime.get(reconciler_name) else {
        tracing::warn!(
            target: "overdrive::reconciler",
            reconciler = %reconciler_name,
            target = %target.as_str(),
            "convergence tick: reconciler not registered; skipping"
        );
        return Ok(());
    };

    // Construct the per-tick TickContext. The wall-clock `now_unix`
    // snapshot is taken from the SAME injected `Clock` the spawn loop
    // sourced `now` from (`state.clock`), once per tick — never
    // `SystemTime::now()` (dst-lint enforces). Reconcilers that need a
    // persistable deadline (e.g. JobLifecycleView's
    // `last_failure_seen_at` per issue #141) read `tick.now_unix`;
    // in-process deadline arithmetic continues to use `tick.now`.
    let now_unix = UnixInstant::from_clock(&*state.clock);
    let tick = TickContext { now, now_unix, tick: tick_n, deadline };

    // Hydrate desired (intent-side) and actual (observation-side).
    let desired = hydrate_desired(reconciler, target, state).await?;
    let actual = hydrate_actual(reconciler, target, state).await?;

    // Hydrate the typed View — currently carried in
    // `AppState::view_cache` rather than libSQL. On first tick the
    // cache is empty; `cached_view_or_default` returns a fresh
    // `default()` view. We still call `reconciler.hydrate` to stay
    // on-contract with ADR-0013 §2 (hydrate is the ONLY async read
    // seam) — today's `hydrate` impls return a default view that we
    // discard in favour of the cached value.
    //
    // TODO(#139): wire `LibsqlHandle` for real per ADR-0013 §2b and
    // use the returned view directly; drop the discard-and-cache
    // shape below.
    let db = LibsqlHandle::default_phase1();
    let _ = reconciler.hydrate(target, &db).await.map_err(ConvergenceError::Hydrate)?;
    let view = cached_view_or_default(reconciler, target, state);

    // Pure reconcile.
    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    // Capture `has_work` BEFORE dispatch — `action_shim::dispatch`
    // consumes `actions: Vec<Action>` by value, so checking
    // `actions.is_empty()` after the call would not compile. The
    // self-re-enqueue gate (`has_work`) is what makes the
    // level-triggered §18 half work: the next tick re-evaluates
    // only when the cluster has not yet converged.
    //
    // `Action::Noop` is the documented "nothing to do this tick"
    // sentinel (see `core/reconciler.rs` `Action::Noop` variant)
    // and `action_shim::dispatch` already treats it as a no-op
    // (see `action_shim.rs`). The §18 re-enqueue gate must honor
    // that documented semantic — an all-Noop actions vec is
    // semantically empty, so it must NOT trip a self-re-enqueue
    // (otherwise a converged target with a heartbeat reconciler
    // self-re-enqueues forever).
    //
    // Backoff-pending fix (§18 level-triggered, S-WS-02 path):
    // when `reconcile` returns no actions because a Failed alloc
    // is mid-backoff (`tick.now_unix < view.last_failure_seen_at[alloc]
    // + backoff_for_attempt(restart_counts[alloc])` and
    // `restart_counts[alloc] < RESTART_BACKOFF_CEILING`), the cluster
    // has NOT converged — actual still has a Failed alloc that the
    // reconciler intends to restart once the deadline elapses. Without
    // re-enqueueing, the broker drains empty, the convergence loop
    // sleeps forever, and the deadline never gets re-evaluated. The
    // `view_has_backoff_pending` predicate inspects `next_view` to
    // detect this transitional state and treats it as has_work=true,
    // preserving the level-triggered semantics whitepaper §18 promises.
    let backoff_pending = view_has_backoff_pending(&next_view);
    let has_work = actions.iter().any(|a| !matches!(a, Action::Noop)) || backoff_pending;

    // Persist the next-view back into the in-memory cache.
    // TODO(#139): replace with libSQL diff-and-persist per ADR-0013 §2b.
    store_cached_view(reconciler, target, state, next_view);

    // Dispatch through the action shim — this is where `.await`
    // is permitted. Per-action error isolation lives in the shim.
    // The shim emits a `LifecycleEvent` on `state.lifecycle_events`
    // after every successful `obs.write` per architecture.md §10.
    action_shim::dispatch(
        actions,
        state.driver.as_ref(),
        state.obs.as_ref(),
        state.lifecycle_events.as_ref(),
        &tick,
    )
    .await
    .map_err(ConvergenceError::Shim)?;

    // Cooperative yield — every action_shim::dispatch path on the
    // single-node SimObservationStore returns Ready synchronously
    // (in-memory writes, no real I/O). Without an explicit yield
    // here, a tight `for tick in 0..N { run_convergence_tick(...).await }`
    // test loop never lets peer `tokio::spawn` tasks (e.g. the
    // `SimDriver` exit-event emit task and the `exit_observer`
    // subsystem reading from the driver's mpsc receiver) progress
    // between ticks. Per `fix-exec-driver-exit-watcher` Step 01-02
    // RCA §Bug 1: the exit-observer DST must observe events between
    // convergence ticks, which requires the test thread to actually
    // yield control once per tick. The production convergence loop
    // (`lib.rs::run_server_with_obs_and_driver`) already calls
    // `yield_now` between ticks for the same reason; this preserves
    // the same semantics for callers that drive `run_convergence_tick`
    // synchronously.
    tokio::task::yield_now().await;

    // Self-re-enqueue per whitepaper §18 *Level-triggered inside
    // the reconciler*: if `reconcile` emitted at least one action,
    // desired ≠ actual on this tick — re-submit so the next drain
    // re-evaluates. The broker collapses duplicates by
    // `(reconciler, target)` so a flapping target produces one
    // pending evaluation, not N.
    if has_work {
        state
            .runtime
            .broker()
            .submit(Evaluation { reconciler: reconciler_name.clone(), target: target.clone() });
    }
    Ok(())
}

/// Cache key string form for the per-target view cache. The cache map
/// is keyed on `(reconciler_name_string, target_string)` so it can
/// be type-erased across reconciler kinds.
fn cache_key(reconciler: &AnyReconciler, target: &TargetResource) -> (String, String) {
    (reconciler.name().to_string(), target.to_string())
}

/// Return the cached `AnyReconcilerView` for `(reconciler, target)`,
/// or a fresh default if the cache is empty.
fn cached_view_or_default(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> AnyReconcilerView {
    let key = cache_key(reconciler, target);
    // Mutex poisoning is unreachable: every critical section in this
    // module is straight-line and panic-free under the workspace's
    // `expect_used` discipline. `allow` rather than reach for
    // `parking_lot` for one call site.
    #[allow(clippy::expect_used)]
    let cache = state.view_cache.lock().expect("view_cache mutex");
    match (reconciler, cache.get(&key)) {
        (AnyReconciler::NoopHeartbeat(_), _) => AnyReconcilerView::Unit,
        (AnyReconciler::JobLifecycle(_), Some(crate::CachedView::JobLifecycle(v))) => {
            AnyReconcilerView::JobLifecycle(v.clone())
        }
        (AnyReconciler::JobLifecycle(_), _) => {
            AnyReconcilerView::JobLifecycle(JobLifecycleView::default())
        }
    }
}

/// Persist the returned `next_view` back to the per-target cache.
fn store_cached_view(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
    next_view: AnyReconcilerView,
) {
    let key = cache_key(reconciler, target);
    // See `cached_view_or_default` — same Mutex, same rationale.
    #[allow(clippy::expect_used)]
    let mut cache = state.view_cache.lock().expect("view_cache mutex");
    let cached = match next_view {
        AnyReconcilerView::Unit => crate::CachedView::Unit,
        AnyReconcilerView::JobLifecycle(v) => crate::CachedView::JobLifecycle(v),
    };
    cache.insert(key, cached);
}

/// Pure predicate over `next_view`: does the `JobLifecycle` reconciler
/// have transitional state still to converge?
///
/// "Transitional" = the view records a `last_failure_seen_at`
/// observation timestamp for at least one alloc whose `restart_counts`
/// is below `RESTART_BACKOFF_CEILING`. A non-empty
/// `last_failure_seen_at` AFTER the reconciler has already declined to
/// emit further actions on this tick means the reconciler is
/// mid-backoff — the next tick (after the per-alloc backoff window
/// elapses) WILL emit a Restart action, so the runtime MUST re-enqueue
/// or the broker drains empty and the convergence loop sleeps without
/// ever re-evaluating the deadline.
///
/// Returns `false` for `Unit` views and for `JobLifecycle` views whose
/// allocs have all reached the backoff ceiling (terminal-failed) or
/// whose `last_failure_seen_at` is empty (no pending restart). The
/// latter covers the converged-Running case (no Failed alloc → no
/// observation timestamp recorded) and the never-failed case alike.
///
/// This is the §18 *Level-triggered inside the reconciler* counterpart
/// to the action-emitted gate above: actions emitted is one signal of
/// "actual ≠ desired"; an outstanding backoff observation is the other.
/// Without this predicate, `reconcile` returning empty actions during
/// backoff would silently drop the eval and leave the runtime stuck.
fn view_has_backoff_pending(next_view: &AnyReconcilerView) -> bool {
    match next_view {
        AnyReconcilerView::Unit => false,
        AnyReconcilerView::JobLifecycle(view) => {
            view.last_failure_seen_at.iter().any(|(alloc, _)| {
                view.restart_counts.get(alloc).copied().unwrap_or(0)
                    < overdrive_core::reconciler::RESTART_BACKOFF_CEILING
            })
        }
    }
}

/// Hydrate the `desired` cluster-state projection for `reconciler`
/// against the `AppState`'s `IntentStore`.
///
/// Per ADR-0021 the runtime owns hydrate-desired; for `NoopHeartbeat`
/// this is `AnyState::Unit`, for `JobLifecycle` it constructs a
/// `JobLifecycleState` from the `IntentStore`.
async fn hydrate_desired(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        AnyReconciler::NoopHeartbeat(_) => Ok(AnyState::Unit),
        AnyReconciler::JobLifecycle(_) => {
            let job_id = job_id_from_target(target)?;
            let job = read_job(state, &job_id).await?;
            // ADR-0027: also read the stop intent. If present →
            // desired_to_stop = true. The reconciler's Stop branch
            // fires only when the spec is also Some (a stop intent
            // for an absent job is a no-op).
            let desired_to_stop = stop_intent_present(state, &job_id).await?;

            let nodes = baseline_nodes_phase1();
            // `desired.allocations` is unused by the JobLifecycle
            // reconciler — it inspects `actual.allocations`.
            let s = JobLifecycleState { job, desired_to_stop, nodes, allocations: BTreeMap::new() };
            Ok(AnyState::JobLifecycle(s))
        }
    }
}

/// Read a `Job` from the `IntentStore` at the canonical `jobs/<id>` key,
/// rkyv-decoding the archived bytes. Returns `Ok(None)` when the key is
/// absent. Errors map to `ConvergenceError::IntentRead`.
async fn read_job(state: &AppState, job_id: &JobId) -> Result<Option<Job>, ConvergenceError> {
    let key = IntentKey::for_job(job_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let Some(b) = bytes else { return Ok(None) };
    let archived = rkyv::access::<rkyv::Archived<Job>, rkyv::rancor::Error>(b.as_ref())
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let job = rkyv::deserialize::<Job, rkyv::rancor::Error>(archived)
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    Ok(Some(job))
}

/// Probe the canonical `jobs/<id>:stop` key; presence is the signal.
async fn stop_intent_present(state: &AppState, job_id: &JobId) -> Result<bool, ConvergenceError> {
    let stop_key = IntentKey::for_job_stop(job_id);
    let stop_bytes = state
        .store
        .get(stop_key.as_bytes())
        .await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    Ok(stop_bytes.is_some())
}

/// Hydrate the `actual` cluster-state projection for `reconciler`
/// against the `AppState`'s `ObservationStore`.
async fn hydrate_actual(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        AnyReconciler::NoopHeartbeat(_) => Ok(AnyState::Unit),
        AnyReconciler::JobLifecycle(_) => {
            let job_id = job_id_from_target(target)?;
            let rows = state
                .obs
                .alloc_status_rows()
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut allocations = BTreeMap::new();
            for row in rows.into_iter().filter(|r| r.job_id == job_id) {
                allocations.insert(row.alloc_id.clone(), row);
            }
            let nodes = baseline_nodes_phase1();
            // `actual.job` is unused — the reconciler reads desired.job.
            // `actual.desired_to_stop` is also unused (only the desired
            // side carries it); set false unconditionally.
            let s = JobLifecycleState { job: None, desired_to_stop: false, nodes, allocations };
            Ok(AnyState::JobLifecycle(s))
        }
    }
}

/// Phase 1 single-node baseline. Returns one `local` node with
/// abundant capacity. Phase 2+ replaces this with a real
/// node-registration handler reading from intent + observation.
fn baseline_nodes_phase1() -> BTreeMap<NodeId, Node> {
    use overdrive_core::aggregate::NodeSpecInput;
    let mut nodes = BTreeMap::new();
    #[allow(clippy::expect_used)]
    let node = Node::new(NodeSpecInput {
        id: "local".to_string(),
        region: "local".to_string(),
        cpu_milli: 4_000,
        memory_bytes: 8 * 1024 * 1024 * 1024,
    })
    .expect("baseline 'local' node spec is valid");
    nodes.insert(node.id.clone(), node);
    nodes
}

/// Extract a `JobId` from a `TargetResource` of shape `job/<id>`.
fn job_id_from_target(target: &TargetResource) -> Result<JobId, ConvergenceError> {
    let raw = target.as_str();
    let id_part =
        raw.strip_prefix("job/").ok_or_else(|| ConvergenceError::TargetShape(raw.to_string()))?;
    JobId::new(id_part).map_err(|e| ConvergenceError::TargetShape(e.to_string()))
}

/// Errors from [`run_convergence_tick`].
#[derive(Debug, thiserror::Error)]
pub enum ConvergenceError {
    /// Hydrate (libSQL read) failed.
    #[error("hydrate failed: {0}")]
    Hydrate(overdrive_core::reconciler::HydrateError),
    /// `IntentStore` read failed.
    #[error("intent read failed: {0}")]
    IntentRead(String),
    /// `ObservationStore` read failed.
    #[error("observation read failed: {0}")]
    ObservationRead(String),
    /// Target resource did not match the expected `job/<id>` shape.
    #[error("invalid target resource: {0}")]
    TargetShape(String),
    /// Action shim returned an error.
    #[error("shim failure: {0}")]
    Shim(crate::action_shim::ShimError),
}

// ---------------------------------------------------------------------------
// Unit tests — pure-logic helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Pin every numeric field of `baseline_nodes_phase1`'s
    /// hardcoded local node. Kills the `*` mutation on
    /// `8 * 1024 * 1024 * 1024` (would yield 8 + ... = 1073741832
    /// or 8 / ... = 0 etc). The exact 8 GiB value (`8 * 1024^3`)
    /// distinguishes every variant.
    #[test]
    fn baseline_nodes_phase1_pins_local_node_capacity() {
        let nodes = baseline_nodes_phase1();
        assert_eq!(nodes.len(), 1, "phase 1 baseline must have exactly one node");

        let local_id = NodeId::new("local").expect("valid NodeId");
        let local = nodes.get(&local_id).expect("local node must be present");
        assert_eq!(local.capacity.cpu_milli, 4_000, "cpu must be exactly 4000 mCPU");
        assert_eq!(
            local.capacity.memory_bytes,
            8_u64 * 1024 * 1024 * 1024,
            "memory must be exactly 8 GiB",
        );
        // Belt-and-braces: pin the exact byte count so no `*`
        // mutation that happens to yield a similar shape escapes.
        assert_eq!(
            local.capacity.memory_bytes, 8_589_934_592_u64,
            "memory must be exactly 8 GiB = 8589934592 bytes",
        );
    }
}
