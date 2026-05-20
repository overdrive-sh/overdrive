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
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{IntentKey, Job, Node, WorkloadKind};
use overdrive_core::id::{AllocationId, ContentHash, NodeId, WorkloadId};
#[cfg(any(test, feature = "integration-tests"))]
use overdrive_core::reconciler::ServiceMapHydrator;
use overdrive_core::reconciler::backend_discovery_bridge::BackendDiscoveryBridgeView;
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, Reconciler, ReconcilerName,
    ServiceMapHydratorState, ServiceMapHydratorView, TargetResource, TickContext,
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::intent_store::IntentStore;
use parking_lot::Mutex;

use crate::AppState;
use crate::action_shim;
use crate::error::ControlPlaneError;
use crate::view_store::{ViewStore, ViewStoreExt};
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};

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
    /// `WorkloadLifecycle` carries `View = WorkloadLifecycleView`; the map
    /// holds per-target persisted views.
    WorkloadLifecycle(BTreeMap<TargetResource, WorkloadLifecycleView>),
    /// `ServiceMapHydrator` carries `View = ServiceMapHydratorView`;
    /// the map holds per-target persisted views per ADR-0035 §5.
    /// Phase 2 (Slice 08; ASR-2.2-04).
    ServiceMapHydrator(BTreeMap<TargetResource, ServiceMapHydratorView>),
    /// `BackendDiscoveryBridge` carries `View =
    /// BackendDiscoveryBridgeView`; the map holds per-target persisted
    /// views per ADR-0035 §5. Phase 2.2
    /// (`backend-discovery-bridge-service-reachability` step 01-01).
    BackendDiscoveryBridge(BTreeMap<TargetResource, BackendDiscoveryBridgeView>),
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
    /// `submit_workload` / `stop_workload` (handler path) and the spawn loop in
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
            AnyReconciler::WorkloadLifecycle(_) => {
                let loaded: BTreeMap<TargetResource, WorkloadLifecycleView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::WorkloadLifecycle(loaded)
            }
            AnyReconciler::ServiceMapHydrator(_) => {
                let loaded: BTreeMap<TargetResource, ServiceMapHydratorView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::ServiceMapHydrator(loaded)
            }
            // backend-discovery-bridge-service-reachability step 01-01 —
            // bulk-load the persisted `BackendDiscoveryBridgeView` map.
            // Shape mirrors `ServiceMapHydrator` exactly; the production
            // hydrate / persist paths land in step 01-03.
            AnyReconciler::BackendDiscoveryBridge(_) => {
                let loaded: BTreeMap<TargetResource, BackendDiscoveryBridgeView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::BackendDiscoveryBridge(loaded)
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

    /// Read the current in-memory `WorkloadLifecycleView` for `target`. Returns
    /// `WorkloadLifecycleView::default()` when the reconciler is not
    /// registered, when the target has no persisted row, or when the
    /// registered reconciler is not `WorkloadLifecycle`. The default fall-back
    /// matches the legacy `view_cache` accessor's contract — fresh-job
    /// callers (`handlers::describe_workload`, the streaming submit's
    /// terminal-event detection) see an empty view rather than a missing
    /// one.
    #[must_use]
    pub fn view_for_workload_lifecycle(&self, target: &TargetResource) -> WorkloadLifecycleView {
        let Some(entry) = self.reconcilers.get(&workload_lifecycle_canonical_name()) else {
            return WorkloadLifecycleView::default();
        };
        match &*entry.views.lock() {
            AnyViewMap::WorkloadLifecycle(map) => map.get(target).cloned().unwrap_or_default(),
            AnyViewMap::Unit
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_) => WorkloadLifecycleView::default(),
        }
    }

    /// Restart-budget snapshot for a single allocation within the
    /// `WorkloadLifecycle` view. Returns `(attempt_index, will_restart)`
    /// where `attempt_index` is 1-indexed (first attempt = 1) and
    /// `will_restart` is true when the reconciler's budget has not been
    /// exhausted.
    ///
    /// Falls back to `(1, true)` when the view is empty (fresh job,
    /// reconciler not yet registered) — conservative: first attempt,
    /// budget assumed available.
    #[must_use]
    pub fn restart_status_for_alloc(
        &self,
        target: &TargetResource,
        alloc_id: &AllocationId,
    ) -> (u32, bool) {
        let view = self.view_for_workload_lifecycle(target);
        let attempts = view.restart_counts.get(alloc_id).copied().unwrap_or(0);
        let attempt_index = attempts.saturating_add(1);
        let will_restart = attempt_index < overdrive_core::reconciler::RESTART_BACKOFF_CEILING;
        (attempt_index, will_restart)
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
            AnyViewMap::WorkloadLifecycle(map) => {
                AnyReconcilerView::WorkloadLifecycle(map.get(target).cloned().unwrap_or_default())
            }
            AnyViewMap::ServiceMapHydrator(map) => {
                AnyReconcilerView::ServiceMapHydrator(map.get(target).cloned().unwrap_or_default())
            }
            // backend-discovery-bridge-service-reachability step 01-01 —
            // shape mirrors the ServiceMapHydrator arm exactly. Returns
            // the persisted view for `target`, or `default()` when no
            // row exists (fresh target before the bridge has written).
            AnyViewMap::BackendDiscoveryBridge(map) => AnyReconcilerView::BackendDiscoveryBridge(
                map.get(target).cloned().unwrap_or_default(),
            ),
        })
    }

    /// Persist `next_view` through the `ViewStore` and, on success,
    /// install it into the in-memory map. The fsync-then-memory
    /// ordering is load-bearing per ADR-0035 §5 step 7→8 — a crash
    /// between the `.await` returning Ok and the in-memory insert
    /// leaves the persisted view as the source of truth, which the
    /// next boot's `bulk_load` recovers.
    ///
    /// **Eq-diff skip** (additive extension per ADR-0035 §1, May
    /// 2026): when `next_view` is `Eq`-equal to the current
    /// in-memory value, this function returns `Ok(())` WITHOUT
    /// calling `write_through` and WITHOUT touching the in-memory
    /// map. The motivation is to elide the per-tick fsync on no-op
    /// ticks (a converged target whose reconciler emits `Noop` and
    /// an unchanged view). Equality is defined by `PartialEq` /
    /// `Eq` on `Self::View`, which the `Reconciler` trait now
    /// requires; the comparison is against the same in-memory value
    /// the runtime would have handed the reconciler as `view`, so a
    /// reconciler returning its input unchanged trivially satisfies
    /// the gate. The fsync-then-memory ordering for the non-equal
    /// branch is independently pinned by the
    /// `WriteThroughOrdering` invariant.
    ///
    /// Returns `Err(ControlPlaneError::Internal)` when the underlying
    /// `write_through` fails (e.g. fsync injection in tests, real
    /// fsync error in production). On error the in-memory map is
    /// unchanged — verifiable via the `WriteThroughOrdering` invariant.
    #[allow(
        clippy::too_many_lines,
        reason = "per-variant Eq-diff + fsync-then-memory block is the same \
                  fixed shape repeated once per reconciler kind; extracting \
                  would require a higher-rank generic helper without changing \
                  the per-arm logic. Refactored alongside the bridge's GREEN \
                  body in step 01-03."
    )]
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
                // row to round-trip. The Eq-diff skip would be a
                // tautology here (`() == ()` always), so the dedicated
                // arm acts as the skip already.
                Ok(())
            }
            AnyReconcilerView::WorkloadLifecycle(view) => {
                // Eq-diff skip — compare `next_view` against the
                // current in-memory value (or `default()` when no
                // row exists for this target, matching the runtime's
                // `view` hydration in `run_convergence_tick`). When
                // equal: skip the fsync AND the in-memory insert,
                // both no-ops by definition. The lock is held only
                // for the duration of the `.cloned()` read; no
                // `.await` is held across it per
                // `.claude/rules/development.md` § Concurrency & async.
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::WorkloadLifecycle(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_) => WorkloadLifecycleView::default(),
                    }
                };
                if current == view {
                    // No-op tick: reconciler returned its input
                    // unchanged. Elide the fsync and the in-memory
                    // insert — both are by-definition no-ops.
                    return Ok(());
                }

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
                    if let AnyViewMap::WorkloadLifecycle(map) = &mut *guard {
                        map.insert(target.clone(), view);
                    }
                }
                Ok(())
            }
            AnyReconcilerView::ServiceMapHydrator(view) => {
                // Eq-diff skip — same shape as WorkloadLifecycle arm above.
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::ServiceMapHydrator(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::BackendDiscoveryBridge(_) => {
                            ServiceMapHydratorView::default()
                        }
                    }
                };
                if current == view {
                    return Ok(());
                }

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
                // STEP 8 — in-memory update AFTER fsync OK.
                {
                    let mut guard = entry.views.lock();
                    if let AnyViewMap::ServiceMapHydrator(map) = &mut *guard {
                        map.insert(target.clone(), view);
                    }
                }
                Ok(())
            }
            // backend-discovery-bridge-service-reachability step 01-01 —
            // Eq-diff skip + fsync-then-memory write-through, mirrors
            // the ServiceMapHydrator arm above. The bridge's reconcile
            // body (lands 01-02) returns a `BackendDiscoveryBridgeView`
            // every tick; this arm persists it.
            AnyReconcilerView::BackendDiscoveryBridge(view) => {
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::BackendDiscoveryBridge(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_) => {
                            BackendDiscoveryBridgeView::default()
                        }
                    }
                };
                if current == view {
                    return Ok(());
                }

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
                // STEP 8 — in-memory update AFTER fsync OK.
                {
                    let mut guard = entry.views.lock();
                    if let AnyViewMap::BackendDiscoveryBridge(map) = &mut *guard {
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

    /// Snapshot of the in-memory `WorkloadLifecycleView` map for `name`.
    /// Returns `None` when the reconciler is not registered or is not
    /// the `WorkloadLifecycle` variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn loaded_workload_lifecycle_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, WorkloadLifecycleView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::WorkloadLifecycle(map) => Some(map.clone()),
            AnyViewMap::Unit
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_) => None,
        }
    }

    /// Drive the runtime's persist-view path directly with a typed
    /// `WorkloadLifecycleView`. Used by the `WriteThroughOrdering`
    /// integration test to assert the runtime obeys the fsync-first
    /// ordering without spinning up a full tick. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: WorkloadLifecycleView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::WorkloadLifecycle(next)).await
    }

    /// Seed the in-memory view for `(job-lifecycle, target)` directly,
    /// bypassing the `ViewStore`. Used by acceptance tests that need
    /// to bootstrap a specific `WorkloadLifecycleView` shape (e.g.
    /// Failed-mid-backoff) without driving the full reconcile cycle to
    /// produce it. **Test-only.**
    ///
    /// Returns silently when the reconciler is not registered or is
    /// not the `WorkloadLifecycle` variant — same fall-back contract as
    /// [`Self::view_for_workload_lifecycle`].
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_workload_lifecycle_view_for_test(
        &self,
        target: &TargetResource,
        view: WorkloadLifecycleView,
    ) {
        let Some(entry) = self.reconcilers.get(&workload_lifecycle_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::WorkloadLifecycle(map) = &mut *guard {
            map.insert(target.clone(), view);
        }
    }

    /// Drop the in-memory view for `(job-lifecycle, target)` directly.
    /// Pairs with [`Self::seed_workload_lifecycle_view_for_test`] for the
    /// "simulate process restart" test pattern in
    /// `runtime_convergence_loop.rs`. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn drop_workload_lifecycle_view_for_test(&self, target: &TargetResource) {
        let Some(entry) = self.reconcilers.get(&workload_lifecycle_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::WorkloadLifecycle(map) = &mut *guard {
            map.remove(target);
        }
    }

    /// Snapshot of the in-memory `ServiceMapHydratorView` map for `name`.
    /// Returns `None` when the reconciler is not registered or is not
    /// the `ServiceMapHydrator` variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn loaded_service_map_hydrator_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, ServiceMapHydratorView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::ServiceMapHydrator(map) => Some(map.clone()),
            AnyViewMap::Unit
            | AnyViewMap::WorkloadLifecycle(_)
            | AnyViewMap::BackendDiscoveryBridge(_) => None,
        }
    }

    /// Drive the runtime's persist-view path directly with a typed
    /// `ServiceMapHydratorView`. Mirrors
    /// [`Self::apply_next_view_for_test`] for the ServiceMapHydrator
    /// variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_service_map_hydrator_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: ServiceMapHydratorView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::ServiceMapHydrator(next)).await
    }

    /// Seed the in-memory view for `(service-map-hydrator, target)`
    /// directly, bypassing the `ViewStore`. Mirrors
    /// [`Self::seed_workload_lifecycle_view_for_test`] for the
    /// ServiceMapHydrator variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_service_map_hydrator_view_for_test(
        &self,
        target: &TargetResource,
        view: ServiceMapHydratorView,
    ) {
        let Some(entry) = self.reconcilers.get(&service_map_hydrator_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::ServiceMapHydrator(map) = &mut *guard {
            map.insert(target.clone(), view);
        }
    }
}

/// Build the canonical [`ReconcilerName`] for the [`WorkloadLifecycle`]
/// reconciler from its trait const [`WorkloadLifecycle::NAME`].
///
/// The const is the single compile-time anchor for the name string —
/// see the `refactor-reconciler-static-name` RCA. `ReconcilerName::new`
/// validates against `^[a-z][a-z0-9-]{0,62}$`; the literal
/// `"job-lifecycle"` declared on `<WorkloadLifecycle as Reconciler>::NAME`
/// is verified-valid at construction time by every `WorkloadLifecycle::canonical()`
/// call site (`unwrap` or `expect` would be equivalent at runtime —
/// the literal cannot fail validation as long as the trait const and
/// the validator's grammar agree).
#[allow(clippy::expect_used)]
fn workload_lifecycle_canonical_name() -> ReconcilerName {
    ReconcilerName::new(<WorkloadLifecycle as Reconciler>::NAME)
        .expect("WorkloadLifecycle::NAME is a valid ReconcilerName by construction")
}

#[cfg(any(test, feature = "integration-tests"))]
#[allow(clippy::expect_used)]
fn service_map_hydrator_canonical_name() -> ReconcilerName {
    ReconcilerName::new(<ServiceMapHydrator as Reconciler>::NAME)
        .expect("ServiceMapHydrator::NAME is a valid ReconcilerName by construction")
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
/// task that drains the [`overdrive_core::eval_broker::EvaluationBroker`] each
/// tick (`config.tick_cadence`, default [`DEFAULT_TICK_CADENCE`]) and
/// dispatches one call per pending [`overdrive_core::eval_broker::Evaluation`].
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
    // persistable deadline (e.g. WorkloadLifecycleView's
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
    // The streaming subscriber (`crate::streaming::check_terminal`)
    // does NOT read the view — per ADR-0037 §4 it projects
    // `event.terminal` directly from the `LifecycleEvent` the action
    // shim broadcasts. View consistency is therefore not a constraint
    // on this ordering; durability is the sole load-bearing reason.
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
        state.dataplane.as_ref(),
        state.lifecycle_events.as_ref(),
        &tick,
        &state.node_id,
        std::sync::Arc::clone(&state.allocator),
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

/// Pure predicate over `next_view`: does the `WorkloadLifecycle` reconciler
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
/// Returns `false` for `Unit` views and for `WorkloadLifecycle` views whose
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
        // Both `Unit` (NoopHeartbeat) and `ServiceMapHydrator` carry no
        // backoff-pending signal at this layer. The hydrator's per-
        // service typed `RetryMemory` is not wired into the
        // convergence-tick loop today; when the production hydrate path
        // lands (GH #160), the corresponding "any service has retry
        // memory recorded" predicate ships alongside.
        AnyReconcilerView::Unit
        | AnyReconcilerView::ServiceMapHydrator(_)
        // backend-discovery-bridge-service-reachability step 01-01 —
        // the bridge's view carries dedup-fingerprint memory (per
        // ADR-0035 / Persist inputs); it does not carry a
        // backoff-pending signal, so this arm returns false. A future
        // bridge-side retry policy would extend this match.
        | AnyReconcilerView::BackendDiscoveryBridge(_) => false,
        AnyReconcilerView::WorkloadLifecycle(view) => {
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
/// this is `AnyState::Unit`, for `WorkloadLifecycle` it constructs a
/// `WorkloadLifecycleState` from the `IntentStore`.
async fn hydrate_desired(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        AnyReconciler::NoopHeartbeat(_) => Ok(AnyState::Unit),
        AnyReconciler::WorkloadLifecycle(_) => {
            let workload_id = workload_id_from_target(target)?;
            let (job, intent_digest) = read_job(state, &workload_id).await?;
            // ADR-0027: also read the stop intent. If present →
            // desired_to_stop = true. The reconciler's Stop branch
            // fires only when the spec is also Some (a stop intent
            // for an absent job is a no-op).
            let desired_to_stop = stop_intent_present(state, &workload_id).await?;

            let nodes = baseline_nodes_phase1();
            // `desired.allocations` is unused by the WorkloadLifecycle
            // reconciler — it inspects `actual.allocations`.
            // ADR-0037 Amendment 2026-05-10 / ADR-0047 §1: read the
            // persisted workload-kind discriminator at
            // `IntentKey::for_workload_kind` (written by `submit_workload` in
            // slice 02-06). Absent / unparseable bytes default to
            // `WorkloadKind::default()` (Service) per
            // `from_discriminator_byte` forward-compat — preserves
            // the kind-agnostic Service shape for legacy submits that
            // predate the discriminator persistence.
            let workload_kind = read_workload_kind(state, &workload_id).await?;
            let service_spec_digest =
                if workload_kind == WorkloadKind::Service { intent_digest } else { None };
            let s = WorkloadLifecycleState {
                workload_id: workload_id.clone(),
                job,
                desired_to_stop,
                nodes,
                allocations: BTreeMap::new(),
                workload_kind,
                service_spec_digest,
            };
            Ok(AnyState::WorkloadLifecycle(s))
        }
        AnyReconciler::ServiceMapHydrator(_) => {
            let service_id = service_id_from_target(target)?;
            let rows = state
                .obs
                .service_backends_rows(&service_id)
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut desired = BTreeMap::new();
            for row in rows {
                // Wrap the row's wire-shape `Ipv4Addr` into `ServiceVip`
                // at the read boundary per architecture.md § 8.
                let vip = overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(row.vip))
                    .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
                let fp = overdrive_core::dataplane::fingerprint::fingerprint(&vip, &row.backends);
                desired.insert(
                    row.service_id,
                    overdrive_core::reconciler::ServiceDesired {
                        vip,
                        backends: row.backends,
                        fingerprint: fp,
                    },
                );
            }
            Ok(AnyState::ServiceMapHydrator(ServiceMapHydratorState {
                desired,
                actual: BTreeMap::new(),
            }))
        }
        // backend-discovery-bridge-service-reachability step 01-03 —
        // GREEN. Per architecture.md § 4.5 / ADR-0049 § 5a. The body
        // of the per-Service projection lives in
        // `hydrate_bridge_desired_listeners` so the outer match-arm
        // stays within `clippy::too_many_lines`.
        AnyReconciler::BackendDiscoveryBridge(_) => {
            let workload_id = workload_id_from_target(target)?;
            let listeners = hydrate_bridge_desired_listeners(state, &workload_id).await?;
            let s =
                overdrive_core::reconciler::backend_discovery_bridge::BackendDiscoveryBridgeState {
                    desired:
                        overdrive_core::reconciler::backend_discovery_bridge::ServiceListenerSet {
                            workload_id: workload_id.clone(),
                            listeners,
                        },
                    actual: overdrive_core::reconciler::backend_discovery_bridge::RunningAllocSet {
                        workload_id,
                        running: std::collections::BTreeSet::new(),
                    },
                };
            Ok(AnyState::BackendDiscoveryBridge(s))
        }
    }
}

/// Test-only public wrapper for [`hydrate_desired`]. Used by
/// acceptance tests (GH #160) to exercise the production hydrate
/// path without going through the full `run_convergence_tick` loop.
#[doc(hidden)]
pub async fn hydrate_desired_for_test(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    hydrate_desired(reconciler, target, state).await
}

/// Test-only public wrapper for [`hydrate_actual`]. Mirrors
/// [`hydrate_desired_for_test`] for the actual-side projection so
/// hydrate-boundary unit tests can exercise the production path
/// directly. Used by `backend-discovery-bridge-service-reachability`
/// step 01-03 inline tests and by future per-reconciler hydrate
/// acceptance tests.
#[doc(hidden)]
pub async fn hydrate_actual_for_test(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    hydrate_actual(reconciler, target, state).await
}

/// Project the per-`ServiceId` `ProjectedListener` map for the
/// `BackendDiscoveryBridge` desired-side hydration arm.
///
/// Reads `WorkloadIntent` at `IntentKey::for_workload(&workload_id)`
/// and dispatches per ADR-0050 § 2:
///
/// * `WorkloadIntent::Service(ServiceV1)` — computes `spec_digest`,
///   consults `state.allocator.lock().await.get(&digest)` for the
///   allocator-issued VIP, projects each listener through
///   `ServiceId::derive(&vip, port, "service-map")` per ADR-0052 § 1.
/// * `WorkloadIntent::Job(_)` / `Schedule(_)` — returns empty map
///   (S-BDB-08; only Service has listeners).
///
/// Phase 1 invariant (ADR-0049 § 4): the allocator memo is populated
/// synchronously at admission, so the `get` is always `Some(_)` for
/// a persisted Service intent. A `None` here is a structural bug —
/// emit `bridge.allocator_memo_absent` debug event and return an
/// empty map (defers convergence to the next tick).
///
/// Lock discipline (`.claude/rules/development.md` § "Concurrency &
/// async"): the allocator guard is acquired, the synchronous
/// `get(&digest)` is consulted, and the guard is dropped BEFORE any
/// further `.await` so the rest of the hydrate path does not hold a
/// lock across an `.await`.
async fn hydrate_bridge_desired_listeners(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<
    BTreeMap<
        overdrive_core::id::ServiceId,
        overdrive_core::reconciler::backend_discovery_bridge::ProjectedListener,
    >,
    ConvergenceError,
> {
    let key = IntentKey::for_workload(workload_id);
    let Some(bytes) = state
        .store
        .get(key.as_bytes())
        .await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?
    else {
        // Intent absent — empty desired. Next tick retries after submit.
        return Ok(BTreeMap::new());
    };
    let intent = overdrive_core::aggregate::WorkloadIntent::from_store_bytes(
        bytes.as_ref(),
        &state.intent_redb_path,
        Some(key.as_str()),
    )
    .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let service_v1 = match &intent {
        overdrive_core::aggregate::WorkloadIntent::Service(s) => s,
        // Only Service workloads have listeners per ADR-0050 § 2.
        overdrive_core::aggregate::WorkloadIntent::Job(_)
        | overdrive_core::aggregate::WorkloadIntent::Schedule(_) => {
            return Ok(BTreeMap::new());
        }
    };
    let spec_digest_hash =
        intent.spec_digest().map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
    // Acquire allocator guard; sync `get()`; drop BEFORE the
    // function returns (no `.await` follows the drop in this fn,
    // but the contract is "guard never crosses `.await`").
    let assigned_vip_opt = {
        let guard = state.allocator.lock().await;
        let vip = guard.get(&digest_bytes);
        drop(guard);
        vip
    };
    let Some(assigned_vip) = assigned_vip_opt else {
        // Phase 1 structural invariant violation — see ADR-0049 § 4.
        tracing::debug!(
            name: "bridge.allocator_memo_absent",
            workload_id = %workload_id,
            spec_digest = %spec_digest_hash,
            "VIP allocator memo absent for Service intent; deferring tick",
        );
        return Ok(BTreeMap::new());
    };
    let mut listeners = BTreeMap::new();
    for listener in &service_v1.listeners {
        let service_id =
            overdrive_core::id::ServiceId::derive(&assigned_vip, listener.port, "service-map");
        listeners.insert(
            service_id,
            overdrive_core::reconciler::backend_discovery_bridge::ProjectedListener {
                vip: assigned_vip,
                port: listener.port,
                protocol: listener.protocol,
            },
        );
    }
    Ok(listeners)
}

/// Read a workload from the `IntentStore` at the canonical
/// `workloads/<id>` key (per ADR-0050 OQ-5 single-cut migration),
/// rkyv-decoding the `WorkloadIntentEnvelope` archived bytes, and
/// project it onto a kind-agnostic [`Job`] shape consumed by the
/// downstream reconciler.
///
/// Returns `Ok((None, None))` when the key is absent. Errors map to
/// `ConvergenceError::IntentRead`.
///
/// Returns a kind-agnostic `Job` projection for both `Job` and
/// `Service` variants — `ServiceV1` carries an identical
/// `(id, replicas, resources, driver)` envelope (its only extra field
/// `listeners` is consumed elsewhere via `ServiceV1`-typed reads, not
/// through this projection), so Service workloads pick up their
/// driver + resource envelope identically and feed into the existing
/// `Some(job) => …` allocation-emission arm at
/// `crates/overdrive-core/src/reconciler.rs::WorkloadLifecycle::reconcile`.
/// The persisted `WorkloadKind` discriminator continues to flow
/// separately via `desired.workload_kind` (sourced from
/// [`read_workload_kind`]) and is threaded onto every emitted
/// `Action::StartAllocation` / `Action::RestartAllocation` so the
/// action shim and observation rows correctly record `kind: Service`
/// for Service-derived allocs.
///
/// The second element is the `WorkloadIntent`'s content-addressed
/// `spec_digest` (SHA-256 over the rkyv-archived payload). Returned
/// only for `Service` intents — Job and Schedule workloads do not
/// allocate VIPs (ADR-0049), so their digest is not surfaced.
async fn read_job(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<(Option<Job>, Option<ContentHash>), ConvergenceError> {
    let key = IntentKey::for_workload(workload_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let Some(b) = bytes else { return Ok((None, None)) };
    let intent = overdrive_core::aggregate::WorkloadIntent::from_store_bytes(
        b.as_ref(),
        &state.intent_redb_path,
        Some(key.as_str()),
    )
    .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    match &intent {
        overdrive_core::aggregate::WorkloadIntent::Job(job) => Ok((Some(job.clone()), None)),
        overdrive_core::aggregate::WorkloadIntent::Service(svc) => {
            // Project Service onto a kind-agnostic Job shape. JobV1
            // and ServiceV1 are field-for-field equivalent over
            // (id, replicas, resources, driver) — the reconciler's
            // `Some(job) =>` arm reads only these four fields, so the
            // projection is lossless from its perspective. Service-
            // only fields (listeners) are consumed elsewhere via
            // ServiceV1-typed reads. The `WorkloadKind::Service`
            // discriminator is threaded separately via
            // `desired.workload_kind` so emitted actions and rows
            // correctly record their Service origin.
            let job = Job {
                id: svc.id.clone(),
                replicas: svc.replicas,
                resources: svc.resources,
                driver: svc.driver.clone(),
            };
            let digest =
                intent.spec_digest().map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
            Ok((Some(job), Some(digest)))
        }
        overdrive_core::aggregate::WorkloadIntent::Schedule(_) => Ok((None, None)),
    }
}

/// Read the persisted workload-kind discriminator at
/// `IntentKey::for_workload_kind`. Absent or unparseable bytes default to
/// `WorkloadKind::default()` (Service) per ADR-0047 §1 forward-compat
/// — legacy submits that predate slice 02-06's discriminator
/// persistence still hydrate as Service-shape (kind-agnostic).
async fn read_workload_kind(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<WorkloadKind, ConvergenceError> {
    let key = IntentKey::for_workload_kind(workload_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    Ok(bytes
        .as_ref()
        .and_then(|b| b.first().copied())
        .map_or_else(WorkloadKind::default, WorkloadKind::from_discriminator_byte))
}

/// Probe the canonical `workloads/<id>/stop` key; presence is the
/// signal. Per ADR-0050 OQ-5 single-cut migration.
async fn stop_intent_present(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<bool, ConvergenceError> {
    let stop_key = IntentKey::for_workload_stop(workload_id);
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
        AnyReconciler::WorkloadLifecycle(_) => {
            let workload_id = workload_id_from_target(target)?;
            let rows = state
                .obs
                .alloc_status_rows()
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut allocations = BTreeMap::new();
            for row in rows.into_iter().filter(|r| r.workload_id == workload_id) {
                allocations.insert(row.alloc_id.clone(), row);
            }
            let nodes = baseline_nodes_phase1();
            // `actual.job` is unused — the reconciler reads desired.job.
            // `actual.desired_to_stop` is also unused (only the desired
            // side carries it); set false unconditionally.
            // ADR-0037 Amendment 2026-05-10 / ADR-0047 §1: read the
            // persisted workload-kind discriminator at
            // `IntentKey::for_workload_kind` so the `actual` side carries
            // the same kind as `desired`. Only the `desired` side
            // drives `reconcile`'s kind-branching logic today, but
            // populating both with the same value keeps the field
            // semantically uniform across the State pair so future
            // `actual`-side branching has a non-default value to work
            // with.
            let workload_kind = read_workload_kind(state, &workload_id).await?;
            let (_, intent_digest) = read_job(state, &workload_id).await?;
            let service_spec_digest =
                if workload_kind == WorkloadKind::Service { intent_digest } else { None };
            let s = WorkloadLifecycleState {
                workload_id,
                job: None,
                desired_to_stop: false,
                nodes,
                allocations,
                workload_kind,
                service_spec_digest,
            };
            Ok(AnyState::WorkloadLifecycle(s))
        }
        AnyReconciler::ServiceMapHydrator(_) => {
            // 08-02 hydrate-actual reads from
            // `service_hydration_results` (the table 08-01 added).
            // GH #160 covers the upstream `service_backends` table for
            // `desired`; `actual` is wire-shape-complete today. Project
            // rows into `BTreeMap<ServiceId, ServiceHydrationStatus>` —
            // the latest LWW winner per `(service_id, fingerprint)` is
            // already filtered by the trait's
            // `service_hydration_results_rows` LWW contract.
            let service_id = service_id_from_target(target)?;
            let rows = state
                .obs
                .service_hydration_results_rows(&service_id)
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut actual = BTreeMap::new();
            // Multiple rows for the same `service_id` are keyed by
            // distinct `fingerprint`s under LWW — the most-recently-
            // written status for THIS service is the row whose
            // `updated_at` dominates. The trait already filters to LWW
            // winners, so all returned rows are tip-of-history; the
            // hydrator wants the most-recent one for the convergence
            // check. `LogicalTimestamp::dominates` is the single
            // comparator the §4 LWW invariant exposes; iterate the rows
            // and retain the dominator.
            let mut latest: Option<
                overdrive_core::traits::observation_store::ServiceHydrationResultRow,
            > = None;
            for row in rows {
                let take = match latest.as_ref() {
                    None => true,
                    Some(current) => row.updated_at.dominates(&current.updated_at),
                };
                if take {
                    latest = Some(row);
                }
            }
            if let Some(row) = latest {
                actual.insert(row.service_id, row.status);
            }
            Ok(AnyState::ServiceMapHydrator(ServiceMapHydratorState {
                desired: BTreeMap::new(),
                actual,
            }))
        }
        // backend-discovery-bridge-service-reachability step 01-03 —
        // GREEN. Per architecture.md § 4.5: read `alloc_status_rows`
        // (the trait surface exposed by `ObservationStore`), filter
        // to `workload_id == this workload` AND `state == Running`,
        // collect alloc-ids into a `BTreeSet<AllocationId>`.
        //
        // The trait does not expose a `_for_workload` variant today —
        // it returns the full per-store row set. Filtering at the
        // hydrate boundary is the same pattern `WorkloadLifecycle`
        // uses (see the arm a few hundred lines above); Phase 2.2
        // single-node row counts are bounded by the local node's
        // allocations.
        AnyReconciler::BackendDiscoveryBridge(_) => {
            let workload_id = workload_id_from_target(target)?;
            let rows = state
                .obs
                .alloc_status_rows()
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let running: std::collections::BTreeSet<AllocationId> = rows
                .into_iter()
                .filter(|r| {
                    r.workload_id == workload_id
                        && r.state == overdrive_core::traits::observation_store::AllocState::Running
                })
                .map(|r| r.alloc_id)
                .collect();
            let s =
                overdrive_core::reconciler::backend_discovery_bridge::BackendDiscoveryBridgeState {
                    desired:
                        overdrive_core::reconciler::backend_discovery_bridge::ServiceListenerSet {
                            workload_id: workload_id.clone(),
                            listeners: BTreeMap::new(),
                        },
                    actual: overdrive_core::reconciler::backend_discovery_bridge::RunningAllocSet {
                        workload_id,
                        running,
                    },
                };
            Ok(AnyState::BackendDiscoveryBridge(s))
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

/// Extract a `WorkloadId` from a `TargetResource` of shape `job/<id>`.
fn workload_id_from_target(target: &TargetResource) -> Result<WorkloadId, ConvergenceError> {
    let raw = target.as_str();
    let id_part =
        raw.strip_prefix("job/").ok_or_else(|| ConvergenceError::TargetShape(raw.to_string()))?;
    WorkloadId::new(id_part).map_err(|e| ConvergenceError::TargetShape(e.to_string()))
}

/// Extract a `ServiceId` from a `TargetResource` of shape `service/<id>`.
/// Mirrors `workload_id_from_target` for the hydrator. Phase 2 (Slice 08).
fn service_id_from_target(
    target: &TargetResource,
) -> Result<overdrive_core::id::ServiceId, ConvergenceError> {
    let raw = target.as_str();
    let id_part = raw
        .strip_prefix("service/")
        .ok_or_else(|| ConvergenceError::TargetShape(raw.to_string()))?;
    overdrive_core::id::ServiceId::from_str(id_part)
        .map_err(|e| ConvergenceError::TargetShape(e.to_string()))
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

    /// Boundary test for `restart_status_for_alloc` at the
    /// `RESTART_BACKOFF_CEILING`. Catches the `< vs <=` mutation:
    /// at exactly ceiling attempts, `will_restart` must be false.
    #[tokio::test]
    async fn restart_status_flips_at_ceiling_boundary() {
        use overdrive_core::id::AllocationId;
        use overdrive_core::reconciler::{
            RESTART_BACKOFF_CEILING, TargetResource, WorkloadLifecycleView,
        };

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(crate::workload_lifecycle()).await.expect("register");

        let target = TargetResource::new("job/payments").expect("target");
        let alloc = AllocationId::new("payments-0").expect("alloc id");

        // attempts = CEILING - 2 → attempt_index = CEILING - 1 → below ceiling → will_restart
        let mut below = WorkloadLifecycleView::default();
        below.restart_counts.insert(alloc.clone(), RESTART_BACKOFF_CEILING - 2);
        runtime.seed_workload_lifecycle_view_for_test(&target, below);
        let (idx, restart) = runtime.restart_status_for_alloc(&target, &alloc);
        assert_eq!(idx, RESTART_BACKOFF_CEILING - 1);
        assert!(restart, "one below ceiling must still restart");

        // attempts = CEILING - 1 → attempt_index = CEILING → AT ceiling → must NOT restart
        let mut at = WorkloadLifecycleView::default();
        at.restart_counts.insert(alloc.clone(), RESTART_BACKOFF_CEILING - 1);
        runtime.seed_workload_lifecycle_view_for_test(&target, at);
        let (idx, restart) = runtime.restart_status_for_alloc(&target, &alloc);
        assert_eq!(idx, RESTART_BACKOFF_CEILING);
        assert!(!restart, "at ceiling must NOT restart — catches < vs <= mutation");
    }

    // -----------------------------------------------------------------
    // backend-discovery-bridge-service-reachability step 01-03 —
    // hydrate_desired / hydrate_actual arms for
    // `AnyReconciler::BackendDiscoveryBridge`.
    //
    // Per architecture.md § 4.5 the runtime owns hydration end-to-end
    // (ADR-0036). These tests close the 01-01 RED scaffolds at the
    // hydrate boundary and act as unit-level proxies for the DST
    // scenarios that close in 01-05:
    //   * S-BDB-02 — Service intent → listener projection (happy path)
    //   * S-BDB-08 — Job / Schedule intents skipped (no listeners)
    //   * S-BDB-10 — multi-listener projection (one entry per port)
    //   * S-BDB-16 — host_ipv4 plumbed at runtime boundary (covered
    //                indirectly: hydrate emits the State the bridge
    //                reconcile body crosses with its own host_ipv4)
    // -----------------------------------------------------------------

    mod backend_discovery_bridge_hydrate {
        use super::*;
        use std::net::Ipv4Addr;
        use std::num::NonZeroU16;
        use std::sync::Arc;

        use overdrive_core::aggregate::{
            DriverInput, ExecInput, ResourcesInput, ServiceV1, WorkloadIntent, WorkloadKind,
        };
        use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
        use overdrive_core::dataplane::backend_key::Proto;
        use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
        use overdrive_core::reconciler::backend_discovery_bridge::BackendDiscoveryBridge;
        use overdrive_core::reconciler::{AnyReconciler, AnyState, TargetResource};
        use overdrive_core::traits::driver::{Driver, DriverType};
        use overdrive_core::traits::intent_store::IntentStore;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
        };
        use overdrive_sim::adapters::clock::SimClock;
        use overdrive_sim::adapters::dataplane::SimDataplane;
        use overdrive_sim::adapters::driver::SimDriver;
        use overdrive_sim::adapters::observation_store::SimObservationStore;
        use overdrive_store_local::LocalIntentStore;
        use tempfile::TempDir;

        // -------------------------------------------------------------
        // Fixtures
        // -------------------------------------------------------------

        const WORKLOAD: &str = "payments";

        fn workload_id() -> WorkloadId {
            WorkloadId::new(WORKLOAD).expect("valid WorkloadId")
        }

        fn target() -> TargetResource {
            TargetResource::new(&format!("job/{WORKLOAD}")).expect("valid target")
        }

        fn writer_node() -> NodeId {
            NodeId::new("writer-1").expect("valid NodeId")
        }

        fn bridge_reconciler() -> AnyReconciler {
            AnyReconciler::BackendDiscoveryBridge(BackendDiscoveryBridge::new(
                Ipv4Addr::new(10, 0, 0, 5),
                writer_node(),
            ))
        }

        fn service_intent(ports: &[u16]) -> WorkloadIntent {
            let listeners: Vec<ListenerInput> = ports
                .iter()
                .map(|p| ListenerInput { port: *p, protocol: "tcp".to_string() })
                .collect();
            let svc = ServiceV1::from_submit(ServiceSpecInput {
                id: WORKLOAD.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/serve".to_string(),
                    args: vec![],
                }),
                listeners,
            })
            .expect("valid service spec");
            WorkloadIntent::Service(svc)
        }

        async fn build_state(tmp: &TempDir, intent: Option<WorkloadIntent>) -> AppState {
            let runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
                .expect("runtime new");
            let store_path = tmp.path().join("intent.redb");
            let store =
                Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
            let obs: Arc<dyn ObservationStore> =
                Arc::new(SimObservationStore::single_peer(writer_node(), 0));

            // Persist the intent (and its kind discriminator) BEFORE
            // building AppState — `state.allocator.allocate` reads
            // from `store` indirectly via spec_digest and we'd race
            // ourselves otherwise. Persist via the byte-level store
            // surface, mirroring `submit_workload` handler shape.
            if let Some(intent_val) = intent {
                let workload_id = match &intent_val {
                    WorkloadIntent::Service(s) => s.id.clone(),
                    WorkloadIntent::Job(j) => j.id.clone(),
                    WorkloadIntent::Schedule(s) => s.id.clone(),
                };
                let key = overdrive_core::aggregate::IntentKey::for_workload(&workload_id);
                let archived = intent_val.archive_for_store().expect("rkyv archive");
                store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
                let kind_key =
                    overdrive_core::aggregate::IntentKey::for_workload_kind(&workload_id);
                let kind = match &intent_val {
                    WorkloadIntent::Job(_) => WorkloadKind::Job,
                    WorkloadIntent::Service(_) => WorkloadKind::Service,
                    WorkloadIntent::Schedule(_) => WorkloadKind::Schedule,
                };
                store
                    .put(kind_key.as_bytes(), &[kind.discriminator_byte()])
                    .await
                    .expect("put kind");
            }

            let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
            let allocator =
                crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
            AppState::new(
                store,
                store_path,
                obs,
                Arc::new(runtime),
                driver,
                Arc::new(SimClock::new()),
                Arc::new(SimDataplane::new()),
                writer_node(),
                allocator,
                std::net::Ipv4Addr::LOCALHOST,
            )
        }

        /// Allocate a VIP via the production allocator path so the
        /// memo is populated for the given Service intent's digest.
        /// Mirrors the handler's `state.allocator.allocate()` call
        /// site (`handlers.rs` § "Service-arm VIP allocation").
        async fn allocate_vip(state: &AppState, intent: &WorkloadIntent) -> ServiceVip {
            let digest = intent.spec_digest().expect("spec_digest");
            let bytes: [u8; 32] = *digest.as_bytes();
            let mut guard = state.allocator.lock().await;
            let vip = guard.allocate(bytes).await.expect("allocate vip");
            drop(guard);
            vip
        }

        async fn write_alloc_status(
            state: &AppState,
            alloc: &str,
            alloc_state: AllocState,
            counter: u64,
        ) {
            let row = AllocStatusRow {
                alloc_id: AllocationId::new(alloc).expect("alloc id"),
                workload_id: workload_id(),
                node_id: NodeId::new("local").expect("node id"),
                state: alloc_state,
                updated_at: LogicalTimestamp { counter, writer: writer_node() },
                reason: None,
                detail: None,
                terminal: None,
                stderr_tail: None,
                kind: WorkloadKind::Service,
                listeners: vec![],
            };
            state.obs.write(ObservationRow::AllocStatus(row)).await.expect("write alloc row");
        }

        // -------------------------------------------------------------
        // Tests (5 — within budget: 5 distinct behaviours x 2 = 10)
        // -------------------------------------------------------------

        /// S-BDB-10 unit-level proxy: an N-listener Service produces
        /// exactly N (ServiceId, ProjectedListener) entries, each
        /// keyed by `ServiceId::derive(&assigned_vip, port,
        /// "service-map")` and carrying the allocator-issued VIP.
        #[tokio::test]
        async fn hydrate_desired_service_projects_listeners_with_allocator_vip() {
            let tmp = TempDir::new().expect("tmpdir");
            let intent = service_intent(&[8080, 8443]);
            let state = build_state(&tmp, Some(intent.clone())).await;
            let assigned_vip = allocate_vip(&state, &intent).await;

            let result = crate::reconciler_runtime::hydrate_desired_for_test(
                &bridge_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_desired ok");

            let AnyState::BackendDiscoveryBridge(s) = result else {
                panic!("expected AnyState::BackendDiscoveryBridge variant");
            };
            assert_eq!(s.desired.workload_id, workload_id());
            assert_eq!(s.desired.listeners.len(), 2, "two listeners → two entries");

            let port_8080 = NonZeroU16::new(8080).expect("nz");
            let port_8443 = NonZeroU16::new(8443).expect("nz");
            let sid_8080 = ServiceId::derive(&assigned_vip, port_8080, "service-map");
            let sid_8443 = ServiceId::derive(&assigned_vip, port_8443, "service-map");

            let pl_8080 = s.desired.listeners.get(&sid_8080).expect("8080 entry");
            assert_eq!(pl_8080.vip, assigned_vip, "vip from allocator memo");
            assert_eq!(pl_8080.port, port_8080);
            assert_eq!(pl_8080.protocol, Proto::Tcp);

            let pl_8443 = s.desired.listeners.get(&sid_8443).expect("8443 entry");
            assert_eq!(pl_8443.vip, assigned_vip);
            assert_eq!(pl_8443.port, port_8443);

            // The `actual` side comes from hydrate_actual; hydrate_desired
            // leaves it empty (the runtime stitches per ADR-0036).
            assert!(s.actual.running.is_empty(), "hydrate_desired leaves actual empty");
        }

        /// S-BDB-08 unit-level proxy: a `Job` intent has no listeners
        /// per ADR-0050 § 2 — hydrate_desired returns an empty
        /// listener map.
        #[tokio::test]
        async fn hydrate_desired_job_returns_empty_listeners() {
            use overdrive_core::aggregate::{JobSpecInput, JobV1};

            let tmp = TempDir::new().expect("tmpdir");
            let job = JobV1::from_submit(JobSpecInput {
                id: WORKLOAD.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/run".to_string(),
                    args: vec![],
                }),
            })
            .expect("valid job");
            let intent = WorkloadIntent::Job(job);
            let state = build_state(&tmp, Some(intent)).await;

            let result = crate::reconciler_runtime::hydrate_desired_for_test(
                &bridge_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_desired ok");

            let AnyState::BackendDiscoveryBridge(s) = result else {
                panic!("expected BackendDiscoveryBridge variant");
            };
            assert!(
                s.desired.listeners.is_empty(),
                "Job intent must project to empty listener map per ADR-0050 § 2",
            );
        }

        /// S-BDB-08 unit-level proxy: a `Schedule` intent also has no
        /// listeners — same hydrate skip as Job.
        ///
        /// Note: `ScheduleV1::from_submit` is itself a RED scaffold
        /// (lands in a future slice per ADR-0051 OQ-5). The test
        /// constructs `ScheduleV1` directly via struct literal —
        /// the wire-arm validator is not under test here, only the
        /// hydrate path's `Schedule(_)` arm.
        #[tokio::test]
        async fn hydrate_desired_schedule_returns_empty_listeners() {
            use overdrive_core::aggregate::{CronExpr, JobSpecInput, JobV1, ScheduleV1};

            let tmp = TempDir::new().expect("tmpdir");
            let inner_job = JobV1::from_submit(JobSpecInput {
                id: WORKLOAD.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/run".to_string(),
                    args: vec![],
                }),
            })
            .expect("valid job");
            let sched = ScheduleV1 {
                id: workload_id(),
                job: inner_job,
                cron_expr: CronExpr::new("* * * * *").expect("valid cron"),
            };
            let intent = WorkloadIntent::Schedule(sched);
            let state = build_state(&tmp, Some(intent)).await;

            let result = crate::reconciler_runtime::hydrate_desired_for_test(
                &bridge_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_desired ok");

            let AnyState::BackendDiscoveryBridge(s) = result else {
                panic!("expected BackendDiscoveryBridge variant");
            };
            assert!(
                s.desired.listeners.is_empty(),
                "Schedule intent must project to empty listener map per ADR-0050 § 2",
            );
        }

        /// Phase 1 invariant violation path (ADR-0049 § 4): if a
        /// Service intent is persisted WITHOUT a matching allocator
        /// memo, hydrate emits `bridge.allocator_memo_absent` and
        /// returns empty desired (deferring convergence to the next
        /// tick). The handler invariant guarantees the memo exists
        /// in production; this test exercises the structural defense.
        #[tokio::test]
        async fn hydrate_desired_allocator_memo_absent_returns_empty_and_logs_debug() {
            let tmp = TempDir::new().expect("tmpdir");
            let intent = service_intent(&[8080]);
            // Deliberately DO NOT call `allocate_vip` — the memo is
            // empty for this digest.
            let state = build_state(&tmp, Some(intent)).await;

            let result = crate::reconciler_runtime::hydrate_desired_for_test(
                &bridge_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_desired ok");

            let AnyState::BackendDiscoveryBridge(s) = result else {
                panic!("expected BackendDiscoveryBridge variant");
            };
            assert!(
                s.desired.listeners.is_empty(),
                "absent allocator memo must yield empty desired (defers to next tick)",
            );
        }

        /// S-BDB-02 unit-level proxy: hydrate_actual filters rows to
        /// `state == Running` only. Pending / Failed / Terminated
        /// rows are dropped.
        #[tokio::test]
        async fn hydrate_actual_filters_to_running_only() {
            let tmp = TempDir::new().expect("tmpdir");
            let state = build_state(&tmp, None).await;

            // Mix of states — only Running should survive.
            write_alloc_status(&state, "payments-0", AllocState::Running, 1).await;
            write_alloc_status(&state, "payments-1", AllocState::Pending, 2).await;
            write_alloc_status(&state, "payments-2", AllocState::Running, 3).await;
            write_alloc_status(&state, "payments-3", AllocState::Failed, 4).await;
            write_alloc_status(&state, "payments-4", AllocState::Terminated, 5).await;

            let result = crate::reconciler_runtime::hydrate_actual_for_test(
                &bridge_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_actual ok");

            let AnyState::BackendDiscoveryBridge(s) = result else {
                panic!("expected BackendDiscoveryBridge variant");
            };
            assert_eq!(s.actual.running.len(), 2, "only Running rows must pass the filter");
            assert!(s.actual.running.contains(&AllocationId::new("payments-0").expect("alloc id")));
            assert!(s.actual.running.contains(&AllocationId::new("payments-2").expect("alloc id")));
            assert_eq!(s.actual.workload_id, workload_id());
        }
    }
}
