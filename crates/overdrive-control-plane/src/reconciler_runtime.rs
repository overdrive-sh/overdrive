//! `ReconcilerRuntime` — runtime-owned reconciler registry per ADR-0035 §5.
//!
//! Composes `AnyReconciler` enum-dispatched reconcilers, the
//! `EvaluationBroker`, and the runtime-owned
//! [`crate::view_store::ViewStore`] for per-reconciler `View` memory.
//!
//! Per ADR-0035 §5 the runtime owns:
//!
//! 1. The `Arc<dyn ViewStore>` port (mandatory constructor parameter
//!    per `.claude/rules/development.md` § Port-trait dependencies).
//! 2. An in-memory `BTreeMap<TargetResource, View>` per reconciler
//!    kind, bulk-loaded at register time and served from RAM on every
//!    tick. The map IS the steady-state read SSOT.
//! 3. The probe → `bulk_load` handshake at register: a probe failure
//!    surfaces as `ControlPlaneError::Internal` and prevents the
//!    reconciler from being added to the registry; the composition
//!    root (`overdrive-cli::commands::serve`) translates the failure
//!    into `health.startup.refused` + non-zero exit.
//!
//! Per ADR-0036 the runtime owns hydration of all three of intent,
//! observation, and view. Reconcilers see a typed `&Self::View` per
//! tick; they never see the `ViewStore` port.
//!
//! Phase 1 shape: the runtime owns a `BTreeMap<ReconcilerName,
//! AnyReconciler>` keyed by the canonical name, plus per-kind in-memory
//! view maps stashed alongside each registered reconciler, plus an
//! `EvaluationBroker` behind `&self`. The `BTreeMap` choice — over
//! `HashMap` — is deliberate: registry iteration must be deterministic
//! across runtime constructions because [`Self::registered`] is
//! consumed by the operator-facing `cluster status` JSON output, and
//! `HashMap`'s `RandomState` hasher would put per-process-randomised
//! key order on the wire (see ADR-0013 §8 storm-proofing rationale and
//! the project-wide ordered-collection-as-nondeterminism rule in
//! `.claude/rules/development.md`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{IntentKey, Job, Node};
use overdrive_core::id::{JobId, NodeId};
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, JobLifecycle, JobLifecycleState,
    JobLifecycleView, Reconciler, ReconcilerName, TargetResource, TickContext,
};
use overdrive_core::traits::intent_store::IntentStore;
use parking_lot::Mutex;

use crate::AppState;
use crate::action_shim;
use crate::error::ControlPlaneError;
use crate::eval_broker::{Evaluation, EvaluationBroker};
use crate::view_store::{ViewStore, ViewStoreExt};

/// Per-reconciler-kind in-memory view map. Mirrors the `AnyReconciler`
/// enum's variant set so the runtime can dispatch typed `View` reads
/// and writes without an `Any`-shaped registry.
///
/// Per ADR-0035 §5 the map IS the steady-state read SSOT. The
/// `BTreeMap<TargetResource, V>` choice over `HashMap` keeps DST
/// replay deterministic
/// (`.claude/rules/development.md` § "Ordered-collection choice").
#[derive(Debug, Default)]
enum AnyViewMap {
    /// `NoopHeartbeat` carries `View = ()`; the per-target map exists
    /// for shape symmetry but never holds anything beyond the implicit
    /// `default()` when a target is read.
    #[default]
    Unit,
    /// `JobLifecycle` carries `View = JobLifecycleView`; the map
    /// holds per-target persisted views.
    JobLifecycle(BTreeMap<TargetResource, JobLifecycleView>),
}

/// Registry entry — pairs an `AnyReconciler` with its typed in-memory
/// view map. Stored under [`ReconcilerRuntime::reconcilers`].
struct RegistryEntry {
    reconciler: AnyReconciler,
    /// In-memory view map. Wrapped in `Mutex` so per-tick reads/writes
    /// can mutate it through the shared `&self` accessor pattern the
    /// convergence-loop spawn uses (`Arc<ReconcilerRuntime>`). Per
    /// `.claude/rules/development.md` § Concurrency & async — no
    /// `.await` is held across this lock; the tick loop reads the
    /// view by value (`.cloned()`), drops the guard, calls the sync
    /// `reconcile` function, then re-acquires the lock to install the
    /// `next_view` after the (`.await`'d) `write_through` returns Ok.
    views: Mutex<AnyViewMap>,
}

/// Registry + broker + view-store owner.
pub struct ReconcilerRuntime {
    /// Runtime-owned `ViewStore` port. The mandatory constructor
    /// parameter per `.claude/rules/development.md` § Port-trait
    /// dependencies. Production wires `RedbViewStore` from the
    /// composition root; DST tests wire `SimViewStore`.
    view_store: Arc<dyn ViewStore>,
    /// Registry keyed on canonical reconciler name. Duplicate
    /// registration is rejected with `ControlPlaneError::Conflict`.
    reconcilers: BTreeMap<ReconcilerName, RegistryEntry>,
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
    /// Construct a new runtime rooted at `data_dir` against the
    /// supplied `view_store`. Creates the directory if absent (so
    /// `canonicalize` has a real target) and canonicalises it once per
    /// ADR-0035 §5.
    ///
    /// Per `.claude/rules/development.md` § Port-trait dependencies the
    /// `view_store` parameter is mandatory — there is no builder
    /// override or in-constructor default. Production wires
    /// `RedbViewStore::open(data_dir)?`; DST tests wire `SimViewStore`.
    ///
    /// # Errors
    ///
    /// Returns [`ControlPlaneError::Internal`] if the directory cannot
    /// be created or canonicalised. Probe failures are deferred to
    /// [`Self::register`] — the constructor itself does no I/O against
    /// the supplied `view_store`.
    pub fn new(data_dir: &Path, view_store: Arc<dyn ViewStore>) -> Result<Self, ControlPlaneError> {
        std::fs::create_dir_all(data_dir).map_err(|e| {
            ControlPlaneError::internal(
                format!("ReconcilerRuntime::new: create_dir_all {} failed", data_dir.display()),
                e,
            )
        })?;
        // Canonicalise to surface bad data_dirs (permission denied,
        // bad symlink) at construction time. The result is discarded:
        // the `RedbViewStore` (production) and `SimViewStore` (tests)
        // resolve their own paths against the supplied `view_store`,
        // so the runtime no longer needs to hold a copy.
        let _canon = std::fs::canonicalize(data_dir).map_err(|e| {
            ControlPlaneError::internal(
                format!("ReconcilerRuntime::new: canonicalize {} failed", data_dir.display()),
                e,
            )
        })?;
        Ok(Self {
            view_store,
            reconcilers: BTreeMap::new(),
            broker: parking_lot::Mutex::new(EvaluationBroker::new()),
        })
    }

    /// Register a reconciler. Performs the ADR-0035 §5 boot handshake:
    ///
    /// 1. `view_store.probe().await` — Earned-Trust validation that
    ///    the underlying store can write/fsync/read/delete. Probe
    ///    failure short-circuits register; the composition root
    ///    translates the resulting `Internal` error into
    ///    `health.startup.refused` and exits non-zero.
    /// 2. `view_store.bulk_load::<R::View>(name).await` — pre-load
    ///    every persisted `(target, view)` row into the runtime's
    ///    in-memory map. The map is the steady-state read SSOT
    ///    thereafter; subsequent ticks consult it without an `.await`.
    /// 3. Insert the registry entry alongside the typed view map.
    ///
    /// Per ADR-0036 the runtime owns hydration end-to-end — reconcilers
    /// never see the `ViewStore` port.
    ///
    /// # Errors
    ///
    /// * [`ControlPlaneError::Conflict`] if a reconciler with the same
    ///   name is already registered. The second registration is
    ///   rejected cleanly — the registry is left unchanged.
    /// * [`ControlPlaneError::Internal`] if the probe fails or the
    ///   bulk-load round-trip fails (CBOR decode error, underlying I/O
    ///   error). Both are hard boot failures — the composition root
    ///   refuses to come up.
    pub async fn register(&mut self, reconciler: AnyReconciler) -> Result<(), ControlPlaneError> {
        let name = reconciler.name().clone();
        if self.reconcilers.contains_key(&name) {
            return Err(ControlPlaneError::Conflict {
                message: format!("reconciler {name} already registered"),
            });
        }

        // Step 1 — Earned-Trust probe. Composition-root invariant:
        // every reconciler's `register` call probes before bulk-loading
        // anything. Probe failure prevents this reconciler from
        // entering the registry. The probe is per-call (not per-runtime)
        // so a transient probe failure on the FIRST register call
        // doesn't poison the runtime — the composition root retries by
        // restarting the binary; mid-process probe failure during a
        // late `register` still surfaces with the same shape.
        self.view_store.probe().await.map_err(|e| {
            ControlPlaneError::from(crate::error::ViewStoreBootError::Probe {
                reconciler: name.clone(),
                source: e,
            })
        })?;

        // Step 2 — typed bulk-load. The per-variant dispatch picks the
        // right `View` type and constructs the matching `AnyViewMap`
        // variant.
        //
        // `static_name()` projects the inner reconciler's
        // `Self::NAME` const — a `&'static str` aliased to the
        // binary's data segment — and is the only shape the
        // post-`refactor-reconciler-static-name` `ViewStore` accepts.
        // Going through `name.as_str()` would produce a `&str`
        // borrowed from the `ReconcilerName`'s `String`, which is
        // non-`'static` and rejected at compile time.
        let static_name = reconciler.static_name();
        let views = match &reconciler {
            AnyReconciler::NoopHeartbeat(_) => AnyViewMap::Unit,
            AnyReconciler::JobLifecycle(_) => {
                let loaded: BTreeMap<TargetResource, JobLifecycleView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::JobLifecycle(loaded)
            }
        };

        // Step 3 — install the registry entry.
        self.reconcilers.insert(name, RegistryEntry { reconciler, views: Mutex::new(views) });
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
        self.reconcilers.values().map(|e| &e.reconciler)
    }

    /// Look up a reconciler by canonical name. O(log N) keyed lookup
    /// over the underlying `BTreeMap`. Used by the per-tick dispatch
    /// path in [`run_convergence_tick`] — each drained Evaluation
    /// names exactly one reconciler (ADR-0013 §8 / whitepaper §18),
    /// so dispatch is a keyed lookup, not a registry scan.
    #[must_use]
    pub fn get(&self, name: &ReconcilerName) -> Option<&AnyReconciler> {
        self.reconcilers.get(name).map(|e| &e.reconciler)
    }

    /// Read the current in-memory `JobLifecycleView` for `target`. Returns
    /// `JobLifecycleView::default()` when the reconciler is not
    /// registered, when the target has no persisted row, or when the
    /// registered reconciler is not `JobLifecycle`. The default fall-back
    /// matches the legacy `view_cache` accessor's contract — fresh-job
    /// callers (`handlers::describe_job`, the streaming submit's
    /// terminal-event detection) see an empty view rather than a missing
    /// one.
    #[must_use]
    pub fn view_for_job_lifecycle(&self, target: &TargetResource) -> JobLifecycleView {
        let Some(entry) = self.reconcilers.get(&job_lifecycle_canonical_name()) else {
            return JobLifecycleView::default();
        };
        match &*entry.views.lock() {
            AnyViewMap::JobLifecycle(map) => map.get(target).cloned().unwrap_or_default(),
            AnyViewMap::Unit => JobLifecycleView::default(),
        }
    }

    /// Look up the in-memory view for `(reconciler, target)` against
    /// the runtime-owned map. Returns `None` when the reconciler is
    /// not registered; otherwise returns the bulk-loaded view (or a
    /// fresh `default()` when no persisted row exists for this
    /// target). The returned `AnyReconcilerView` is a clone — callers
    /// (the tick loop) drop the lock before invoking `reconcile`.
    fn get_view(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
    ) -> Option<AnyReconcilerView> {
        let entry = self.reconcilers.get(name)?;
        let guard = entry.views.lock();
        Some(match &*guard {
            AnyViewMap::Unit => AnyReconcilerView::Unit,
            AnyViewMap::JobLifecycle(map) => {
                AnyReconcilerView::JobLifecycle(map.get(target).cloned().unwrap_or_default())
            }
        })
    }

    /// Persist `next_view` through the `ViewStore` and, on success,
    /// install it into the in-memory map. The fsync-then-memory
    /// ordering is load-bearing per ADR-0035 §5 step 7→8 — a crash
    /// between the `.await` returning Ok and the in-memory insert
    /// leaves the persisted view as the source of truth, which the
    /// next boot's `bulk_load` recovers.
    ///
    /// Returns `Err(ControlPlaneError::Internal)` when the underlying
    /// `write_through` fails (e.g. fsync injection in tests, real
    /// fsync error in production). On error the in-memory map is
    /// unchanged — verifiable via the `WriteThroughOrdering` invariant.
    async fn persist_view(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next_view: AnyReconcilerView,
    ) -> Result<(), ControlPlaneError> {
        let Some(entry) = self.reconcilers.get(name) else {
            return Err(ControlPlaneError::internal(
                format!("ReconcilerRuntime::persist_view: unknown reconciler {name}"),
                "no registry entry",
            ));
        };
        // Recover the `&'static str` canonical name from the registry
        // entry's inner `AnyReconciler`. Required for the post-
        // `refactor-reconciler-static-name` `ViewStore` byte surface,
        // whose `reconciler` parameter is typed `&'static str`.
        let static_name = entry.reconciler.static_name();
        match next_view {
            AnyReconcilerView::Unit => {
                // Unit views carry no data; nothing to persist or
                // install in-memory. Returning Ok matches the
                // ViewStore's semantic: there is no `(target, ())`
                // row to round-trip.
                Ok(())
            }
            AnyReconcilerView::JobLifecycle(view) => {
                // STEP 7 — durable write-through with fsync.
                self.view_store
                    .write_through(static_name, target, &view)
                    .await
                    .map_err(|e| {
                        ControlPlaneError::internal(
                            format!(
                                "ReconcilerRuntime::persist_view({name}, {target}): write_through failed"
                            ),
                            e,
                        )
                    })?;
                // STEP 8 — in-memory update AFTER fsync OK. The lock
                // is taken here, not earlier — the `.await` above
                // must NOT be held across the lock per
                // `.claude/rules/development.md` § Concurrency & async.
                {
                    let mut guard = entry.views.lock();
                    if let AnyViewMap::JobLifecycle(map) = &mut *guard {
                        map.insert(target.clone(), view);
                    }
                }
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------------
    // Test-only accessors — exposed under `cfg(any(test, feature =
    // "integration-tests"))` so the integration test in
    // `tests/integration/reconciler_runtime_view_store.rs` can assert
    // on the in-memory view map shape without going through a tick.
    // ---------------------------------------------------------------

    /// Test-only convenience: construct a runtime against an in-memory
    /// `RedbViewStore` rooted at `data_dir`. Equivalent to
    /// `ReconcilerRuntime::new(data_dir, Arc::new(RedbViewStore::open(
    /// data_dir)))`. **Test-only.** Production code in
    /// `overdrive-cli::commands::serve` calls [`Self::new`] with the
    /// same wiring; this helper exists so existing acceptance /
    /// integration tests that need a runtime+store pair don't have to
    /// repeat the two-line construction at every call site.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`] — `data_dir` create / canonicalize. Also
    /// returns `ControlPlaneError::Internal` when the redb file cannot
    /// be opened (e.g. concurrent open in the same process).
    #[doc(hidden)]
    pub fn new_with_redb_view_store_for_test(data_dir: &Path) -> Result<Self, ControlPlaneError> {
        let store: Arc<dyn ViewStore> =
            Arc::new(crate::view_store::redb::RedbViewStore::open(data_dir).map_err(|e| {
                ControlPlaneError::from(crate::error::ViewStoreBootError::Open {
                    path: crate::view_store::redb::RedbViewStore::resolve_path(data_dir),
                    source: e,
                })
            })?);
        Self::new(data_dir, store)
    }

    /// Snapshot of the in-memory `JobLifecycleView` map for `name`.
    /// Returns `None` when the reconciler is not registered or is not
    /// the `JobLifecycle` variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn loaded_job_lifecycle_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, JobLifecycleView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::JobLifecycle(map) => Some(map.clone()),
            AnyViewMap::Unit => None,
        }
    }

    /// Drive the runtime's persist-view path directly with a typed
    /// `JobLifecycleView`. Used by the `WriteThroughOrdering`
    /// integration test to assert the runtime obeys the fsync-first
    /// ordering without spinning up a full tick. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: JobLifecycleView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::JobLifecycle(next)).await
    }

    /// Seed the in-memory view for `(job-lifecycle, target)` directly,
    /// bypassing the `ViewStore`. Used by acceptance tests that need
    /// to bootstrap a specific `JobLifecycleView` shape (e.g.
    /// Failed-mid-backoff) without driving the full reconcile cycle to
    /// produce it. **Test-only.**
    ///
    /// Returns silently when the reconciler is not registered or is
    /// not the `JobLifecycle` variant — same fall-back contract as
    /// [`Self::view_for_job_lifecycle`].
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_job_lifecycle_view_for_test(
        &self,
        target: &TargetResource,
        view: JobLifecycleView,
    ) {
        let Some(entry) = self.reconcilers.get(&job_lifecycle_canonical_name()) else { return };
        let mut guard = entry.views.lock();
        if let AnyViewMap::JobLifecycle(map) = &mut *guard {
            map.insert(target.clone(), view);
        }
    }

    /// Drop the in-memory view for `(job-lifecycle, target)` directly.
    /// Pairs with [`Self::seed_job_lifecycle_view_for_test`] for the
    /// "simulate process restart" test pattern in
    /// `runtime_convergence_loop.rs`. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn drop_job_lifecycle_view_for_test(&self, target: &TargetResource) {
        let Some(entry) = self.reconcilers.get(&job_lifecycle_canonical_name()) else { return };
        let mut guard = entry.views.lock();
        if let AnyViewMap::JobLifecycle(map) = &mut *guard {
            map.remove(target);
        }
    }
}

/// Build the canonical [`ReconcilerName`] for the [`JobLifecycle`]
/// reconciler from its trait const [`JobLifecycle::NAME`].
///
/// The const is the single compile-time anchor for the name string —
/// see the `refactor-reconciler-static-name` RCA. `ReconcilerName::new`
/// validates against `^[a-z][a-z0-9-]{0,62}$`; the literal
/// `"job-lifecycle"` declared on `<JobLifecycle as Reconciler>::NAME`
/// is verified-valid at construction time by every `JobLifecycle::canonical()`
/// call site (`unwrap` or `expect` would be equivalent at runtime —
/// the literal cannot fail validation as long as the trait const and
/// the validator's grammar agree).
#[allow(clippy::expect_used)]
fn job_lifecycle_canonical_name() -> ReconcilerName {
    ReconcilerName::new(<JobLifecycle as Reconciler>::NAME)
        .expect("JobLifecycle::NAME is a valid ReconcilerName by construction")
}

// ---------------------------------------------------------------------------
// phase-1-first-workload — slice 3 (US-03) — runtime convergence tick loop
//
// Per ADR-0035 §5 + whitepaper §18: the runtime owns the `.await` on
// hydrate (intent + observation), the diff-and-persist of returned
// views via the ViewStore, and the dispatch of emitted actions. Each
// tick: hydrate_desired → hydrate_actual → get_view → reconcile →
// dispatch → persist_view (fsync first) → in-memory install.
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
/// Returns `Err(ConvergenceError)` only when an action shim or
/// view-persist call fails. The fsync-then-memory ordering on the
/// view-persist path is load-bearing per ADR-0035 §5 step 7→8.
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
/// # Errors
///
/// Returns [`ConvergenceError`] when hydrate, reconcile-dispatch, or
/// view-persist fail in a way the runtime cannot represent as observation.
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

    // Hydrate the typed View from the runtime's in-memory map. Per
    // ADR-0035 §5 the map IS the steady-state read SSOT; the
    // `bulk_load` ran once at register time, every tick reads from
    // RAM. A target with no persisted row reads as `default()`.
    let view = state.runtime.get_view(reconciler_name, target).unwrap_or(AnyReconcilerView::Unit);

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
    // Backoff-pending fix (§18 level-triggered, S-WS-02 path): see
    // `view_has_backoff_pending` for the predicate body — when a
    // Failed alloc is mid-backoff the reconciler emits no actions
    // BUT actual still has a Failed alloc, so the runtime must
    // re-enqueue or the broker drains empty and the convergence
    // loop sleeps forever.
    let backoff_pending = view_has_backoff_pending(&next_view);
    let has_work = actions.iter().any(|a| !matches!(a, Action::Noop)) || backoff_pending;

    // Persist next_view through the runtime-owned ViewStore BEFORE
    // dispatching the action. ADR-0035 §5 step 7→8 ordering: fsync
    // first via `write_through`, then install into the in-memory map.
    // On crash between the two, the next boot's `bulk_load` recovers
    // the persisted value (which is the intended source of truth).
    //
    // The persist-before-dispatch ordering also matters for the
    // streaming submit handler (`crate::streaming`): the action shim's
    // dispatch fires a `LifecycleEvent` on `state.lifecycle_events`,
    // and the streaming subscriber's `check_terminal` reads
    // `view_for_job_lifecycle` to decide whether the alloc has hit
    // `BackoffExhausted` (`restart_counts >= RESTART_BACKOFF_CEILING`).
    // The view MUST reflect the just-computed `next_view` before the
    // event is broadcast, otherwise the subscriber sees stale
    // `restart_counts` and the streaming cap fires before
    // `BackoffExhausted` is detected. Pre-ADR-0035 the equivalent
    // `store_cached_view` call sat in this same slot for the same
    // reason.
    state
        .runtime
        .persist_view(reconciler_name, target, next_view)
        .await
        .map_err(ConvergenceError::ViewPersist)?;

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
    /// `ViewStore::write_through` failed (fsync error, decode error,
    /// underlying I/O error). Per ADR-0035 §5 step 7→8 the in-memory
    /// map is unchanged when this fires.
    #[error("view persist failed: {0}")]
    ViewPersist(crate::error::ControlPlaneError),
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
