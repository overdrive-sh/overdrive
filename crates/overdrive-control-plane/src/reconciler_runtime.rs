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
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::{
    Action, Reconciler, ReconcilerName, TargetResource, TickContext,
};
use overdrive_core::traits::observation_store::{
    ConflictRoute, LogicalTimestamp, ObservationWrite, ReconcileConflictRow,
};
#[cfg(any(test, feature = "integration-tests"))]
use overdrive_reconcilers::ServiceMapHydrator;
use overdrive_reconcilers::backend_discovery_bridge::BackendDiscoveryBridgeView;
use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;
use overdrive_reconcilers::{
    AnyReconciler, AnyReconcilerView, AnyState, ServiceMapHydratorView, SvidLifecycleView,
    WorkflowLifecycleView, WorkloadLifecycle, WorkloadLifecycleView,
};
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
    /// `WorkflowLifecycle` carries `View = WorkflowLifecycleView` (Phase 1
    /// empty); the map holds per-target persisted views per ADR-0035 §5 /
    /// ADR-0064 §5.
    #[expect(
        clippy::zero_sized_map_values,
        reason = "WorkflowLifecycleView is intentionally Phase-1-empty (ADR-0064 §5 — the \
                  re-emit decision is pure over `actual`; there is no input to persist yet). \
                  The per-target map shape mirrors every other reconciler kind so the runtime \
                  dispatch stays uniform; the View gains a field (and this expect self-removes) \
                  when a retry/budget policy lands per `development.md` Persist-inputs rule."
    )]
    WorkflowLifecycle(BTreeMap<TargetResource, WorkflowLifecycleView>),
    /// `ServiceMapHydrator` carries `View = ServiceMapHydratorView`;
    /// the map holds per-target persisted views per ADR-0035 §5.
    /// Phase 2 (Slice 08; ASR-2.2-04).
    ServiceMapHydrator(BTreeMap<TargetResource, ServiceMapHydratorView>),
    /// `BackendDiscoveryBridge` carries `View =
    /// BackendDiscoveryBridgeView`; the map holds per-target persisted
    /// views per ADR-0035 §5. Phase 2.2
    /// (`backend-discovery-bridge-service-reachability` step 01-01).
    #[expect(
        clippy::zero_sized_map_values,
        reason = "BackendDiscoveryBridgeView is deliberately field-less since ADR-0079 § D3 — \
                  the bridge converges by diffing against the `service_backends` rows it \
                  manages, so it holds no per-tick memory. The per-target map shape mirrors \
                  every other reconciler kind so the runtime dispatch stays uniform (§ D3 \
                  rejects `type View = ()` for that reason); the View gains a field (and this \
                  expect self-removes) if a bridge-side retry/backoff policy ever lands. \
                  Same precedent as AnyViewMap::WorkflowLifecycle above."
    )]
    BackendDiscoveryBridge(BTreeMap<TargetResource, BackendDiscoveryBridgeView>),
    /// `ServiceLifecycle` carries `View = ServiceLifecycleView`;
    /// the map holds per-target persisted views per ADR-0035 §5 /
    /// ADR-0055. Service-health-check-probes step 01-03b (dispatch
    /// wiring); the runtime-registration call site lands in a
    /// later slice.
    ServiceLifecycle(BTreeMap<TargetResource, ServiceLifecycleView>),
    /// `SvidLifecycle` carries `View = SvidLifecycleView` — retry memory
    /// (`retry: BTreeMap<AllocationId, IssueRetry>`) per ADR-0067 D8, so a
    /// failed `IssueSvid` backs off instead of re-firing every tick. The map
    /// holds per-target persisted views per ADR-0035 §5.
    SvidLifecycle(BTreeMap<TargetResource, SvidLifecycleView>),
    /// `VmReclamation` carries `View = VmReclamationView`; FIELD-LESS per
    /// the ADR-0079 precedent (`brief.md` §105a.1 — SD-1's Bar-2
    /// reconciler, ADR-0083 §D7, GH #42, step 02-01). The per-target map
    /// shape mirrors every other reconciler kind so the runtime dispatch
    /// stays uniform; the View gains a field (and this expect
    /// self-removes) if a retry/backoff policy ever lands.
    #[expect(
        clippy::zero_sized_map_values,
        reason = "VmReclamationView is deliberately field-less (brief.md §105a.1, ADR-0079 \
                  precedent) -- nothing this reconciler emits is ever consulted, so retry falls \
                  out of the runtime's has_work self-re-enqueue. Same precedent as \
                  AnyViewMap::BackendDiscoveryBridge above."
    )]
    VmReclamation(BTreeMap<TargetResource, overdrive_reconcilers::VmReclamationView>),
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
    #[allow(
        clippy::too_many_lines,
        reason = "per-variant typed bulk-load is the same fixed shape repeated once per \
                  reconciler kind; extracting would require a higher-rank generic helper \
                  without changing the per-arm logic. Same precedent as persist_view above."
    )]
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
            AnyReconciler::WorkflowLifecycle(_) => {
                #[expect(
                    clippy::zero_sized_map_values,
                    reason = "WorkflowLifecycleView is intentionally Phase-1-empty (ADR-0064 §5); \
                              self-removes when the View gains a field. See AnyViewMap::WorkflowLifecycle."
                )]
                let loaded: BTreeMap<TargetResource, WorkflowLifecycleView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::WorkflowLifecycle(loaded)
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
                #[expect(
                    clippy::zero_sized_map_values,
                    reason = "BackendDiscoveryBridgeView is deliberately field-less (ADR-0079 \
                              § D3); self-removes when the View gains a field. See \
                              AnyViewMap::BackendDiscoveryBridge."
                )]
                let loaded: BTreeMap<TargetResource, BackendDiscoveryBridgeView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::BackendDiscoveryBridge(loaded)
            }
            // service-health-check-probes step 01-03b — bulk-load the
            // persisted `ServiceLifecycleView` map. Shape mirrors
            // `WorkloadLifecycle` exactly; the registration call site
            // is wired in a later slice.
            AnyReconciler::ServiceLifecycle(_) => {
                let loaded: BTreeMap<TargetResource, ServiceLifecycleView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::ServiceLifecycle(loaded)
            }
            // workload-identity-manager — bulk-load the persisted
            // `SvidLifecycleView` retry-memory map (ADR-0067 D8); shape mirrors
            // `WorkflowLifecycle` exactly.
            AnyReconciler::SvidLifecycle(_) => {
                let loaded: BTreeMap<TargetResource, SvidLifecycleView> =
                    self.view_store.bulk_load(static_name).await.map_err(|e| {
                        ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                            reconciler: name.clone(),
                            source: e,
                        })
                    })?;
                AnyViewMap::SvidLifecycle(loaded)
            }
            // microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7,
            // GH #42) — bulk-load the persisted `VmReclamationView` map.
            // Field-less (ADR-0079 precedent); shape mirrors
            // `BackendDiscoveryBridge` exactly.
            AnyReconciler::VmReclamation(_) => {
                #[expect(
                    clippy::zero_sized_map_values,
                    reason = "VmReclamationView is deliberately field-less (brief.md §105a.1); \
                              self-removes when the View gains a field. See \
                              AnyViewMap::VmReclamation."
                )]
                let loaded: BTreeMap<
                    TargetResource,
                    overdrive_reconcilers::VmReclamationView,
                > = self.view_store.bulk_load(static_name).await.map_err(|e| {
                    ControlPlaneError::from(crate::error::ViewStoreBootError::BulkLoad {
                        reconciler: name.clone(),
                        source: e,
                    })
                })?;
                AnyViewMap::VmReclamation(loaded)
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

    /// Borrow the broker's mutex directly (rather than the
    /// `MutexGuard`). Lets callers pass the lock by reference into a
    /// dispatch path that takes a brief lock-grab-submit-release per
    /// `Action::EnqueueEvaluation` without holding the guard across
    /// `.await` per `.claude/rules/development.md` § Concurrency &
    /// async.
    ///
    /// Used by [`action_shim::dispatch`] so the cross-reconciler
    /// handoff variant can re-enqueue downstream reconcilers
    /// directly. See UI-05 (the
    /// `backend-discovery-bridge-service-reachability` step 02-04
    /// architectural remediation) for the rationale.
    #[must_use]
    pub fn broker_mutex(&self) -> &parking_lot::Mutex<EvaluationBroker> {
        &self.broker
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
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_)
            | AnyViewMap::ServiceLifecycle(_)
            | AnyViewMap::SvidLifecycle(_)
            | AnyViewMap::VmReclamation(_) => WorkloadLifecycleView::default(),
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
        let will_restart = attempt_index < overdrive_reconcilers::RESTART_BACKOFF_CEILING;
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
            AnyViewMap::WorkflowLifecycle(map) => {
                AnyReconcilerView::WorkflowLifecycle(map.get(target).cloned().unwrap_or_default())
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
            // service-health-check-probes step 01-03b — same shape as
            // the WorkloadLifecycle / ServiceMapHydrator arms.
            AnyViewMap::ServiceLifecycle(map) => {
                AnyReconcilerView::ServiceLifecycle(map.get(target).cloned().unwrap_or_default())
            }
            // workload-identity-manager step 01-04 — same shape as the
            // WorkflowLifecycle arm (Slice-01 empty view; ADR-0067 D8).
            AnyViewMap::SvidLifecycle(map) => {
                AnyReconcilerView::SvidLifecycle(map.get(target).cloned().unwrap_or_default())
            }
            // microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7,
            // GH #42) — same shape as the WorkflowLifecycle arm
            // (field-less view; brief.md §105a.1).
            AnyViewMap::VmReclamation(map) => {
                AnyReconcilerView::VmReclamation(map.get(target).cloned().unwrap_or_default())
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
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::SvidLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => WorkloadLifecycleView::default(),
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
            AnyReconcilerView::WorkflowLifecycle(view) => {
                // Eq-diff skip — same shape as the WorkloadLifecycle arm.
                // The Phase 1 `WorkflowLifecycleView` is empty, so the
                // current-vs-next comparison is always equal and this arm
                // elides the fsync on every tick. The arm is kept full
                // (not collapsed to `Ok(())`) so a future non-empty view
                // persists through the same fsync-then-memory ordering.
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::WorkflowLifecycle(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::SvidLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => WorkflowLifecycleView::default(),
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
                    if let AnyViewMap::WorkflowLifecycle(map) = &mut *guard {
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
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::SvidLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => ServiceMapHydratorView::default(),
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
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::SvidLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => BackendDiscoveryBridgeView::default(),
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
            // service-health-check-probes step 01-03b — Eq-diff skip
            // + fsync-then-memory write-through, mirrors the
            // BackendDiscoveryBridge arm above. ADR-0055 / ADR-0035 §5.
            AnyReconcilerView::ServiceLifecycle(view) => {
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::ServiceLifecycle(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::SvidLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => ServiceLifecycleView::default(),
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
                    if let AnyViewMap::ServiceLifecycle(map) = &mut *guard {
                        map.insert(target.clone(), view);
                    }
                }
                Ok(())
            }
            // workload-identity-manager step 01-04 — Eq-diff skip +
            // fsync-then-memory write-through, mirrors the WorkflowLifecycle
            // arm above. The Slice-01 `SvidLifecycleView` is empty
            // (ADR-0067 D8), so the current-vs-next comparison is always
            // equal and this arm elides the fsync every tick; the arm is
            // kept full so the retry-memory view (03-01) persists through
            // the same ordering.
            AnyReconcilerView::SvidLifecycle(view) => {
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::SvidLifecycle(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::VmReclamation(_) => SvidLifecycleView::default(),
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
                    if let AnyViewMap::SvidLifecycle(map) = &mut *guard {
                        map.insert(target.clone(), view);
                    }
                }
                Ok(())
            }
            // microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7,
            // GH #42) — Eq-diff skip + fsync-then-memory write-through,
            // mirrors the WorkflowLifecycle arm above. `VmReclamationView`
            // is field-less (brief.md §105a.1), so the current-vs-next
            // comparison is always equal and this arm elides the fsync
            // every tick; kept full so a future non-empty view persists
            // through the same ordering.
            AnyReconcilerView::VmReclamation(view) => {
                let current = {
                    let guard = entry.views.lock();
                    match &*guard {
                        AnyViewMap::VmReclamation(map) => {
                            map.get(target).cloned().unwrap_or_default()
                        }
                        AnyViewMap::Unit
                        | AnyViewMap::WorkflowLifecycle(_)
                        | AnyViewMap::WorkloadLifecycle(_)
                        | AnyViewMap::ServiceMapHydrator(_)
                        | AnyViewMap::BackendDiscoveryBridge(_)
                        | AnyViewMap::ServiceLifecycle(_)
                        | AnyViewMap::SvidLifecycle(_) => {
                            overdrive_reconcilers::VmReclamationView::default()
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
                    if let AnyViewMap::VmReclamation(map) = &mut *guard {
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
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_)
            | AnyViewMap::ServiceLifecycle(_)
            | AnyViewMap::SvidLifecycle(_)
            | AnyViewMap::VmReclamation(_) => None,
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

    /// Seed the in-memory view for `(workload-lifecycle, target)` directly,
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

    /// Drop the in-memory view for `(workload-lifecycle, target)` directly.
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
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::WorkloadLifecycle(_)
            | AnyViewMap::BackendDiscoveryBridge(_)
            | AnyViewMap::ServiceLifecycle(_)
            | AnyViewMap::SvidLifecycle(_)
            | AnyViewMap::VmReclamation(_) => None,
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

    /// Snapshot of the in-memory `BackendDiscoveryBridgeView` map for
    /// `name`. Mirrors [`Self::loaded_service_map_hydrator_views_for_test`]
    /// for the BackendDiscoveryBridge variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[expect(
        clippy::zero_sized_map_values,
        reason = "BackendDiscoveryBridgeView is deliberately field-less (ADR-0079 § D3); \
                  self-removes when the View gains a field. See \
                  AnyViewMap::BackendDiscoveryBridge."
    )]
    pub fn loaded_backend_discovery_bridge_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, BackendDiscoveryBridgeView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::BackendDiscoveryBridge(map) => Some(map.clone()),
            AnyViewMap::Unit
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::WorkloadLifecycle(_)
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::ServiceLifecycle(_)
            | AnyViewMap::SvidLifecycle(_)
            | AnyViewMap::VmReclamation(_) => None,
        }
    }

    /// Drive the runtime's persist-view path with a typed
    /// `BackendDiscoveryBridgeView`. Mirrors
    /// [`Self::apply_next_service_map_hydrator_view_for_test`] for
    /// the BackendDiscoveryBridge variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_backend_discovery_bridge_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: BackendDiscoveryBridgeView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::BackendDiscoveryBridge(next)).await
    }

    /// Seed the in-memory view for `(backend-discovery-bridge, target)`
    /// directly, bypassing the `ViewStore`. Mirrors
    /// [`Self::seed_service_map_hydrator_view_for_test`] for the
    /// BackendDiscoveryBridge variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_backend_discovery_bridge_view_for_test(
        &self,
        target: &TargetResource,
        view: BackendDiscoveryBridgeView,
    ) {
        let Some(entry) = self.reconcilers.get(&backend_discovery_bridge_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::BackendDiscoveryBridge(map) = &mut *guard {
            map.insert(target.clone(), view);
        }
    }

    /// Snapshot of the in-memory `ServiceLifecycleView` map for
    /// `name`. Mirrors the BackendDiscoveryBridge variant for the
    /// ServiceLifecycle reconciler. **Test-only.** Per
    /// service-health-check-probes step 01-03b mutation-tightening
    /// pass — exposes the in-memory state so the Eq-diff write-skip
    /// gate can be asserted directly.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn loaded_service_lifecycle_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, ServiceLifecycleView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::ServiceLifecycle(map) => Some(map.clone()),
            AnyViewMap::Unit
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::WorkloadLifecycle(_)
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_)
            | AnyViewMap::SvidLifecycle(_)
            | AnyViewMap::VmReclamation(_) => None,
        }
    }

    /// Drive the runtime's persist-view path with a typed
    /// `ServiceLifecycleView`. Mirrors the BackendDiscoveryBridge
    /// variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_service_lifecycle_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: ServiceLifecycleView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::ServiceLifecycle(next)).await
    }

    /// Seed the in-memory view for `(service-lifecycle, target)`
    /// directly, bypassing the `ViewStore`. Mirrors the
    /// BackendDiscoveryBridge variant. **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_service_lifecycle_view_for_test(
        &self,
        target: &TargetResource,
        view: ServiceLifecycleView,
    ) {
        let Some(entry) = self.reconcilers.get(&service_lifecycle_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::ServiceLifecycle(map) = &mut *guard {
            map.insert(target.clone(), view);
        }
    }

    /// Snapshot of the in-memory `SvidLifecycleView` map for `name`.
    /// Mirrors the ServiceLifecycle variant for the SvidLifecycle
    /// reconciler (workload-identity-manager). **Test-only.** Exposes
    /// the in-memory retry-memory state so the Eq-diff write-skip gate
    /// (`persist_view`'s `SvidLifecycle` arm `if current == view`) can
    /// be asserted directly — the kill site for the missed `==`→`!=`
    /// mutant on that arm.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn loaded_svid_lifecycle_views_for_test(
        &self,
        name: &ReconcilerName,
    ) -> Option<BTreeMap<TargetResource, SvidLifecycleView>> {
        let entry = self.reconcilers.get(name)?;
        match &*entry.views.lock() {
            AnyViewMap::SvidLifecycle(map) => Some(map.clone()),
            AnyViewMap::Unit
            | AnyViewMap::WorkflowLifecycle(_)
            | AnyViewMap::WorkloadLifecycle(_)
            | AnyViewMap::ServiceMapHydrator(_)
            | AnyViewMap::BackendDiscoveryBridge(_)
            | AnyViewMap::ServiceLifecycle(_)
            | AnyViewMap::VmReclamation(_) => None,
        }
    }

    /// Drive the runtime's persist-view path with a typed
    /// `SvidLifecycleView`. Mirrors the ServiceLifecycle variant.
    /// **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn apply_next_svid_lifecycle_view_for_test(
        &self,
        name: &ReconcilerName,
        target: &TargetResource,
        next: SvidLifecycleView,
    ) -> Result<(), ControlPlaneError> {
        self.persist_view(name, target, AnyReconcilerView::SvidLifecycle(next)).await
    }

    /// Seed the in-memory view for `(svid-lifecycle, target)` directly,
    /// bypassing the `ViewStore`. Mirrors the ServiceLifecycle variant.
    /// **Test-only.**
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn seed_svid_lifecycle_view_for_test(
        &self,
        target: &TargetResource,
        view: SvidLifecycleView,
    ) {
        let Some(entry) = self.reconcilers.get(&svid_lifecycle_canonical_name()) else {
            return;
        };
        let mut guard = entry.views.lock();
        if let AnyViewMap::SvidLifecycle(map) = &mut *guard {
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
/// `"workload-lifecycle"` declared on `<WorkloadLifecycle as Reconciler>::NAME`
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

#[cfg(any(test, feature = "integration-tests"))]
#[allow(clippy::expect_used)]
fn backend_discovery_bridge_canonical_name() -> ReconcilerName {
    ReconcilerName::new(
        <overdrive_reconcilers::backend_discovery_bridge::BackendDiscoveryBridge
            as Reconciler>::NAME,
    )
    .expect("BackendDiscoveryBridge::NAME is a valid ReconcilerName by construction")
}

#[cfg(any(test, feature = "integration-tests"))]
#[allow(clippy::expect_used)]
fn service_lifecycle_canonical_name() -> ReconcilerName {
    ReconcilerName::new(
        <overdrive_reconcilers::service_lifecycle::ServiceLifecycleReconciler as Reconciler>::NAME,
    )
    .expect("ServiceLifecycleReconciler::NAME is a valid ReconcilerName by construction")
}

#[cfg(any(test, feature = "integration-tests"))]
#[allow(clippy::expect_used)]
fn svid_lifecycle_canonical_name() -> ReconcilerName {
    ReconcilerName::new(<overdrive_reconcilers::svid_lifecycle::SvidLifecycle as Reconciler>::NAME)
        .expect("SvidLifecycle::NAME is a valid ReconcilerName by construction")
}

/// Map the dispatch-boundary [`action_shim::validate::WriteRoute`] onto
/// the core-side [`ConflictRoute`] the observation row records. The two
/// enums are intentionally separate (`WriteRoute` lives at the dispatch
/// boundary; `ConflictRoute` is the core-side data mirror — an
/// `overdrive-core → overdrive-control-plane` dep would invert the
/// crate layering). Fix C, RCA `fix-mixed-backend-dispatch-spin`.
const fn write_route_to_conflict_route(route: action_shim::validate::WriteRoute) -> ConflictRoute {
    match route {
        action_shim::validate::WriteRoute::Xdp => ConflictRoute::Xdp,
        action_shim::validate::WriteRoute::Cgroup => ConflictRoute::Cgroup,
    }
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
/// Surface a reconcile-output invariant violation on both operator
/// channels, then return so the caller can self-heal.
///
/// Surface-then-continue (`.claude/rules/reconcilers.md` self-heal
/// posture; RCA `fix-mixed-backend-dispatch-spin` § Fix C). On a genuine
/// same-slot conflict the violation is surfaced on TWO channels — the
/// Kubernetes Events model: a machine-queryable control signal distinct
/// from a best-effort human signal. The caller then skips dispatch this
/// tick, persists the View, and retries next tick. NO stop /
/// early-return: the appliance OS has no operator shell, so the system
/// must self-heal.
///
/// Infallible by construction: **every** failure inside is logged and
/// swallowed, because this function is itself the error-reporting path.
/// A failure to report must not abort the tick that is reporting.
async fn surface_reconcile_conflict(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    tick: &TickContext,
    violation: &action_shim::validate::ReconcilerOutputViolation,
) {
    // Channel 1 (machine-queryable control signal): a durable
    // `reconcile_conflict` observation row keyed on the conflicting
    // `(service_id, vip, port, proto)` slot. Operators query it via
    // `ObservationStore::reconcile_conflict_rows`. Best-effort write —
    // a write failure must NOT abort the tick (the tracing signal below
    // still fires and convergence retries), so we log + drop the error
    // rather than propagate.
    let action_shim::validate::ReconcilerOutputViolation::ConflictingServiceWrites {
        service_id,
        vip,
        vip_port,
        proto,
        first_route,
        second_route,
    } = violation;
    let (service_id, vip, vip_port, proto, first_route, second_route) =
        (*service_id, *vip, *vip_port, *proto, *first_route, *second_route);
    // `vip_port` is `Some(_)` for every surviving conflict class in
    // Phase 1 (same-route same-slot carries the shared port); the
    // `Option` exists only to avoid churning the variant if a future
    // port-less conflict class lands. Fall back to 0 if ever `None`.
    let port = vip_port.unwrap_or(0);
    // ADR-0077 § D2 site 8: the LWW counter derives from the prior row
    // at this `(service_id, vip, port, proto)` key, never from the tick
    // alone. The conflict write is already best-effort (a write failure
    // is logged and convergence continues, because the tracing event
    // below is the primary signal), so a READ failure must not abort the
    // conflict signal either — but it must not be silently absorbed
    // (`.claude/rules/development.md` § "Errors"): log the cause and
    // proceed with `None`.
    let prior_updated_at: Option<LogicalTimestamp> =
        match state.obs.reconcile_conflict_rows(&service_id).await {
            Ok(rows) => rows
                .into_iter()
                .find(|r| r.vip == vip && r.port == port && r.proto == proto)
                .map(|r| r.updated_at),
            Err(err) => {
                tracing::warn!(
                    target: "overdrive::reconciler",
                    name = "reconciler.output.conflict_prior_read_failed",
                    reconciler = %reconciler_name,
                    target = %target.as_str(),
                    error = %err,
                    "could not read the prior reconcile_conflict row; stamping without a \
                     prior floor — this write may lose the LWW merge. The tracing signal \
                     above is unaffected."
                );
                None
            }
        };
    let conflict_row = ReconcileConflictRow {
        service_id,
        vip,
        port,
        proto,
        first_route: write_route_to_conflict_route(first_route),
        second_route: write_route_to_conflict_route(second_route),
        // LWW timestamp matching the action-shim convention
        // (prior-derived counter, tick as floor) — see
        // `ServiceHydrationResultRowV1::updated_at`.
        updated_at: LogicalTimestamp::dominating(
            tick.tick,
            state.node_id.clone(),
            prior_updated_at.as_ref(),
        ),
    };
    if let Err(err) = state.obs.write(ObservationWrite::ReconcileConflict(conflict_row)).await {
        tracing::warn!(
            target: "overdrive::reconciler",
            name = "reconciler.output.conflict_row_write_failed",
            reconciler = %reconciler_name,
            target = %target.as_str(),
            error = %err,
            "failed to write reconcile_conflict observation row; the tracing \
             signal still fired and convergence will retry next tick"
        );
    }
    // Channel 2 (supplemental human signal): the structured tracing
    // event. KEPT alongside the observation row, never replaced.
    tracing::error!(
        target: "overdrive::reconciler",
        name = "reconciler.output.invariant_violation",
        reconciler = %reconciler_name,
        target = %target.as_str(),
        tick = tick.tick,
        violation = ?violation,
        "reconciler emitted conflicting Actions in one tick; skipping dispatch"
    );
}

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
    run_convergence_tick_inner(state, reconciler_name, target, now, tick_n, deadline, None).await
}

/// Integration-test form of [`run_convergence_tick`] that drives the same
/// registered reconciler, hydration, ViewStore, validation, dispatch, and
/// re-enqueue path while replacing only the privileged host-network adapter.
///
/// # Errors
///
/// Returns the same errors as [`run_convergence_tick`].
#[doc(hidden)]
#[cfg(any(test, feature = "integration-tests"))]
pub async fn run_convergence_tick_with_network_provisioner_for_test(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    now: Instant,
    tick_n: u64,
    deadline: Instant,
    network_provisioner: &dyn action_shim::WorkloadNetworkProvisioner,
) -> Result<(), ConvergenceError> {
    run_convergence_tick_inner(
        state,
        reconciler_name,
        target,
        now,
        tick_n,
        deadline,
        Some(network_provisioner),
    )
    .await
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the allocator/listener_facts MutexGuards are lent into the HydrationContext \
              borrow-bundle and must outlive both hydrate_* .await calls; the scoped block \
              already releases them at the minimal hydration window"
)]
async fn run_convergence_tick_inner(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    now: Instant,
    tick_n: u64,
    deadline: Instant,
    network_provisioner: Option<&dyn action_shim::WorkloadNetworkProvisioner>,
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

    // Hydrate desired (intent-side) and actual (observation-side) through the
    // port-driven `AnyReconciler::hydrate_*` dispatch (ADR-0086 S3): lock the two
    // mutex-guarded read-ports for the hydration window, build the borrow-bundle
    // `HydrationContext`, run both hydrate calls, and drop the guards before
    // `reconcile` / action dispatch (no lock outlives the hydration window).
    // The scoped block IS the minimal hydration window: the two guards are lent
    // into the `HydrationContext` borrow-bundle and released at the block end,
    // before `reconcile` / dispatch. rust-1.95.0's `significant_drop_tightening`
    // cannot see the borrow that forces the guards to outlive both hydrate calls
    // — suppressed at the fn level (`#[expect]` on the fn signature).
    let (desired, actual) = {
        let allocator = state.allocator.lock().await;
        let listener_facts = state.listener_facts.lock().await;
        let ctx = build_hydration_context(state, &allocator, &listener_facts);
        let desired = reconciler.hydrate_desired(&ctx, target).await?;
        let actual = reconciler.hydrate_actual(&ctx, target).await?;
        (desired, actual)
    };

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

    // Reconcile-output invariant validator — closes the inter-Action
    // conflict gap that Phase 16 D11 surfaced. Sum-type-interior
    // modelling on the `Action` enum is insufficient: the enum admits
    // valid actions whose Vec-level composition is a bug (two writes
    // to the same service-LB VIP in one tick produce non-deterministic
    // dataplane post-state). On violation, fail-safe: skip dispatch
    // this tick, persist View as normal (reconciler memory is
    // independent of dispatch success — skipping the View update
    // would re-trigger the same broken reconcile next tick), log a
    // structured `reconciler.output.invariant_violation` event for
    // operators. Convergence retries on the next tick; once the
    // reconciler is fixed, normal dispatch resumes. The control-plane
    // does NOT panic on a buggy reconciler.
    //
    // Capture the dispatch outcome instead of `?`-propagating it inline: a
    // recoverable shim error (e.g. a transient `IssueSvid` issuance failure)
    // MUST still fall through to the `yield_now` + `if has_work` self-re-enqueue
    // below before it returns. Early-`?` here skipped the re-enqueue, so the
    // FIRST failed tick — which has already persisted its retry-bearing View
    // (above) — stalled forever: the broker drained empty and the persisted
    // retry memory never re-drove (`view_has_backoff_pending` only re-enqueues
    // once a tick actually runs). The error is still propagated (returned last,
    // unchanged) so `lib.rs` logs it; the self-heal is the re-enqueue. The
    // invariant-conflict branch directly below already self-heals by NOT
    // early-returning — this matches that posture for the dispatch path.
    let dispatch_outcome: Result<(), ConvergenceError> =
        if let Err(violation) = action_shim::validate::validate_reconcile_output(&actions) {
            surface_reconcile_conflict(state, reconciler_name, target, &tick, &violation).await;
            // The validate-violation path is itself a self-heal: skip dispatch,
            // keep the persisted View, retry next tick. It contributes no dispatch
            // error to propagate.
            Ok(())
        } else {
            // Dispatch through the action shim — this is where `.await`
            // is permitted. Per-action error isolation lives in the shim.
            // The shim emits a `LifecycleEvent` on `state.lifecycle_events`
            // after every successful `obs.write` per architecture.md §10.
            //
            // ADR-0064 §5 — the WorkflowEngine is now composed into AppState
            // (step 01-08), so the shim receives the REAL engine, replacing
            // the 01-05/01-06 `None` placeholder. `dispatch_with_workflow_intent`
            // is the AppState-aware path that ALSO persists workflow-instance
            // desired-intent for every `Action::StartWorkflow` BEFORE handing
            // the actions to the engine off the shim — so the workflow-lifecycle
            // reconciler's `hydrate_desired` can read the instance back on the
            // next tick (and re-emit on restart).
            // NOTE: no `?` here — the outcome is captured into `dispatch_outcome`
            // and returned at the END of the function, AFTER the self-re-enqueue
            // below. A recoverable shim error must still re-enqueue (self-heal) so
            // the persisted retry memory actually re-drives on a later tick.
            match network_provisioner {
                #[cfg(any(test, feature = "integration-tests"))]
                Some(network_provisioner) => {
                    action_shim::dispatch_with_workflow_intent_and_network_provisioner_for_test(
                        actions,
                        state,
                        &tick,
                        network_provisioner,
                    )
                    .await
                    .map_err(ConvergenceError::Shim)
                }
                #[cfg(not(any(test, feature = "integration-tests")))]
                Some(_) => action_shim::dispatch_with_workflow_intent(actions, state, &tick)
                    .await
                    .map_err(ConvergenceError::Shim),
                None => action_shim::dispatch_with_workflow_intent(actions, state, &tick)
                    .await
                    .map_err(ConvergenceError::Shim),
            }
        };

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
    // Return the (still-propagated) dispatch outcome LAST — after the
    // self-re-enqueue above ran on ALL paths. On a recoverable shim error this
    // is `Err(ConvergenceError::Shim(_))`, which `lib.rs` logs; the re-enqueue
    // is what lets the persisted retry memory re-drive next tick.
    dispatch_outcome
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
        // The bridge's view is field-less since ADR-0079 § D3 — it
        // converges by diffing against the `service_backends` row it
        // manages, so it holds no memory at all and certainly no
        // backoff-pending signal. This arm MUST stay `false`: retry
        // after a dropped write is carried by `has_work`, which is true
        // on exactly the ticks that emitted a write. A converged tick
        // emits nothing, `has_work` is false, and the broker correctly
        // drains — returning `true` here would busy-loop. A future
        // bridge-side retry policy would extend this match.
        | AnyReconcilerView::BackendDiscoveryBridge(_)
        // The workflow-lifecycle view is Phase-1 empty (ADR-0064 §5) and
        // carries no backoff-pending signal; the §18 re-enqueue for a
        // running-no-task instance is driven by the action-emitted gate
        // (the reconciler returns a `StartWorkflow`), not this predicate.
        | AnyReconcilerView::WorkflowLifecycle(_)
        // microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7, GH
        // #42) — the VmReclamation view is field-less (brief.md §105a.1):
        // retry falls out of the runtime's has_work self-re-enqueue, no
        // View-carried backoff-pending signal at all.
        | AnyReconcilerView::VmReclamation(_) => false,
        // The svid-lifecycle view carries per-allocation issue-retry
        // memory (ADR-0067 D8). A `retry` entry is written on EVERY
        // `IssueSvid` emit — the record-on-emit / `bump_if_dispatched`
        // shape in `SvidLifecycle::reconcile` (`attempts += 1`,
        // `last_failure_seen_at = tick.now_unix`) — so a non-empty `retry`
        // does NOT exclusively mean "a recorded FAILED attempt mid-backoff".
        // It can equally be the transient artifact of an as-yet-unconfirmed
        // SUCCESSFUL first issue: the entry persists from the emit tick until
        // the confirming tick observes the alloc held and clear-on-success
        // removes it (`reconcile`'s `running ∧ held` branch). The predicate
        // INTENTIONALLY keeps the reconciler enqueued in BOTH cases —
        // failing-and-backing-off, and emitted-but-not-yet-confirmed-held.
        //
        // Division of labour with the §18 action-emitted gate (`has_work`):
        // on a tick that EMITS `IssueSvid` (first issue, restart recovery, OR
        // a near-expiry rotate — ADR-0067 rev 7: rotation now bumps `retry` on
        // emit too), the re-tick is ALREADY driven by `has_work` (an
        // `IssueSvid` is non-`Noop`), so this predicate firing too is
        // redundant-but-harmless — the broker collapses duplicate
        // `(reconciler, target)` submits. This predicate is the SOLE
        // re-enqueue driver only on a SUPPRESSED tick: a `running ∧ ¬held`
        // alloc inside its first-issue backoff window — or, rev 7, a `running ∧
        // held(near-expiry)` alloc mid-rotation-backoff — emits a bare `Noop`,
        // `has_work` is false, and without this arm the broker drains empty and
        // the reconciler is never re-ticked at the deadline.
        // That suppressed-tick path is the one pinned by
        // `svid_lifecycle_reenqueues_while_issue_backoff_pending`.
        //
        // The bump is LOAD-BEARING, not incidental: removing it would let a
        // FAILED issue re-fire every tick with no backoff. Do not "simplify"
        // it away — it is pinned by
        // `running_alloc_without_held_svid_emits_issue_svid` and
        // `first_issue_unheld_never_issued_alloc_issues_and_records_one_attempt`
        // (both assert `attempts == 1` after a first emit).
        //
        // Unlike `WorkloadLifecycle`, the svid reconciler has NO terminal
        // backoff ceiling — a failed issue retries indefinitely (there is no
        // `attempts >= CEILING` give-up in `SvidLifecycle::reconcile`), so
        // EVERY non-empty `retry` entry is outstanding work. The reconcile
        // body's `retain` GCs entries for non-Running allocs and its
        // clear-on-success removes entries for held allocs, so a non-empty map
        // means a still-running alloc has a recorded attempt not yet
        // confirmed-held — exactly the keep-enqueued condition. Derivable from
        // `next_view` alone, as the contract requires.
        AnyReconcilerView::SvidLifecycle(view) => !view.retry.is_empty(),
        // GAP-9 Shape B — keep the service-lifecycle reconciler alive
        // across cadences while any observed alloc is mid-startup-window.
        //
        // During the active startup window the reconciler emits ZERO
        // actions (Running, no Pass yet, deadline not elapsed), so the
        // §18 *action-emitted* self-re-enqueue gate (`has_work`) is
        // false and the broker would drain empty after the FIRST tick —
        // leaving the reconciler never re-ticked and its Stable /
        // EarlyExit / StartupProbeFailed branches structurally
        // unreachable in production (the GAP-9 defect).
        //
        // The predicate is true IFF the view records an observed alloc
        // that has NOT yet reached a terminal (`stable_announced` ∪
        // `terminal_announced`). It flips to false the instant the alloc
        // reaches ANY terminal — Stable OR ServiceFailed — so a
        // terminal alloc does NOT keep the runtime spinning (the
        // busy-loop GAP-9's fix must avoid). The decision is derivable
        // from `next_view` alone, as `view_has_backoff_pending`
        // requires.
        AnyReconcilerView::ServiceLifecycle(view) => view.has_alloc_mid_startup_window(),
        AnyReconcilerView::WorkloadLifecycle(view) => {
            view.last_failure_seen_at.iter().any(|(alloc, _)| {
                view.restart_counts.get(alloc).copied().unwrap_or(0)
                    < overdrive_reconcilers::RESTART_BACKOFF_CEILING
            })
        }
    }
}

/// Build the per-tick [`HydrationContext`](overdrive_core::reconcilers::HydrationContext)
/// borrow-bundle from `AppState` (ADR-0086 D5/S3).
///
/// The two `tokio::sync::Mutex`-guarded read-ports (`ServiceVipView` over the
/// allocator, `ListenerFacts` over the fact store) are locked by the CALLER and
/// their guards lent in as `&PersistentServiceVipAllocator` / `&ListenerFactStore`
/// so the borrow bundle can name them as `&dyn` trait objects; the caller drops
/// the guards once the two hydrate calls return (no lock outlives the tick's
/// hydration window). The remaining handles (`IntentStore`, `ObservationStore`,
/// `DriverRegistry`, `VmHostState`, `WorkflowLiveSet`, `HeldSvidView`) are lent
/// straight off the `Arc`-held `AppState` fields. This is the single site that
/// projects `AppState` onto the read-port surface the moved `hydrate_*` bodies
/// consume — no `hydrate_*` body reaches an `AppState` field directly (ADR-0086
/// D5 S1 invariant).
pub(crate) fn build_hydration_context<'a>(
    state: &'a AppState,
    allocator: &'a overdrive_dataplane::allocators::PersistentServiceVipAllocator,
    listener_facts: &'a crate::listener_facts::ListenerFactStore,
) -> overdrive_core::reconcilers::HydrationContext<'a> {
    overdrive_core::reconcilers::HydrationContext {
        intent_store: state.store.as_ref(),
        observation_store: state.obs.as_ref(),
        drivers: state.drivers.as_ref(),
        vm_host_state: state.vm_host_state.as_ref(),
        listener_facts,
        service_vip_view: allocator,
        workflow_live_set: state.workflow_engine.as_ref(),
        held_svid_view: state.identity.as_ref(),
        node_id: &state.node_id,
        host_ipv4: state.host_ipv4,
        intent_redb_path: &state.intent_redb_path,
    }
}

/// Test-only public wrapper for the port-driven hydrate-desired dispatch
/// ([`AnyReconciler::hydrate_desired`], ADR-0086 D1). Used by acceptance tests
/// (GH #160) to exercise the production hydrate path without going through the
/// full `run_convergence_tick` loop. Post-S3 (step 02-04) this drives the moved
/// per-reconciler `hydrate_desired` through the injected read-ports — the same
/// dispatch the tick loop uses — so the 02-03 characterization golden asserts
/// port-driven == pre-move golden.
#[doc(hidden)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "guards lent into HydrationContext must outlive the hydrate .await"
)]
pub async fn hydrate_desired_for_test(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    let allocator = state.allocator.lock().await;
    let listener_facts = state.listener_facts.lock().await;
    let ctx = build_hydration_context(state, &allocator, &listener_facts);
    Ok(reconciler.hydrate_desired(&ctx, target).await?)
}

/// Test-only public wrapper for the port-driven hydrate-actual dispatch
/// ([`AnyReconciler::hydrate_actual`]). Mirrors [`hydrate_desired_for_test`].
#[doc(hidden)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "guards lent into HydrationContext must outlive the hydrate .await"
)]
pub async fn hydrate_actual_for_test(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    let allocator = state.allocator.lock().await;
    let listener_facts = state.listener_facts.lock().await;
    let ctx = build_hydration_context(state, &allocator, &listener_facts);
    Ok(reconciler.hydrate_actual(&ctx, target).await?)
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
    /// A persisted workflow-instance intent failed to decode through the
    /// `WorkflowStart` rkyv-envelope codec. Intent is the load-bearing SSOT
    /// (ADR-0048 §3 asymmetry): an undecodable intent REFUSES — it is NOT
    /// log-and-skipped like an observation row. The reconcile tick surfaces
    /// this and the runtime escalates it to `health.startup.refused` +
    /// non-zero exit (ADR-0065 §5).
    #[error("workflow-instance intent decode failed: {0}")]
    IntentDecode(String),
    /// Target resource did not match the expected `workload/<id>` shape.
    #[error("invalid target resource: {0}")]
    TargetShape(String),
    /// A per-reconciler `hydrate_desired` / `hydrate_actual` (ADR-0086 D1)
    /// failed at the hydration boundary. The runtime consumes `ConvergenceError`
    /// on the tick path; the moved hydrate bodies return the core
    /// [`HydrateError`](overdrive_core::reconcilers::HydrateError) and this
    /// `#[from]` variant converts at the call site (ADR-0086 D3).
    #[error("hydration failed: {0}")]
    Hydrate(#[from] overdrive_core::reconcilers::HydrateError),
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

    /// Boundary test for `restart_status_for_alloc` at the
    /// `RESTART_BACKOFF_CEILING`. Catches the `< vs <=` mutation:
    /// at exactly ceiling attempts, `will_restart` must be false.
    #[tokio::test]
    async fn restart_status_flips_at_ceiling_boundary() {
        use overdrive_core::id::AllocationId;
        use overdrive_core::reconcilers::TargetResource;
        use overdrive_reconcilers::{RESTART_BACKOFF_CEILING, WorkloadLifecycleView};

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(crate::workload_lifecycle()).await.expect("register");

        let target = TargetResource::new("workload/payments").expect("target");
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
            DriverInput, ExecInput, ResourcesInput, ServiceV2, WorkloadIntent, WorkloadKind,
        };
        use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
        use overdrive_core::dataplane::backend_key::Proto;
        use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
        use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
        use overdrive_core::reconcilers::TargetResource;
        use overdrive_core::traits::driver::{Driver, DriverType};
        use overdrive_core::traits::intent_store::IntentStore;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
        };
        use overdrive_reconcilers::backend_discovery_bridge::BackendDiscoveryBridge;
        use overdrive_reconcilers::service_lifecycle::ServiceLifecycleReconciler;
        use overdrive_reconcilers::workload_lifecycle::WorkloadLifecycle;
        use overdrive_reconcilers::{AnyReconciler, AnyState};
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
            TargetResource::new(&format!("workload/{WORKLOAD}")).expect("valid target")
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
            let svc = ServiceV2::from_submit(ServiceSpecInput {
                id: WORKLOAD.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/serve".to_string(),
                    args: vec![],
                }),
                listeners,
                startup_probes: vec![],
                readiness_probes: vec![],
                liveness_probes: vec![],
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
            let listener_facts = crate::test_empty_listener_facts();
            AppState::new(
                store,
                store_path,
                obs,
                Arc::new(runtime),
                driver,
                Arc::new(SimClock::new()),
                Arc::new(SimDataplane::new()),
                Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                    overdrive_sim::adapters::entropy::SimEntropy::new(0),
                ))),
                Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
                writer_node(),
                allocator,
                listener_facts,
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
            write_alloc_status_with_addr(state, alloc, alloc_state, counter, None).await;
        }

        /// Variant carrying an explicit per-alloc canonical `workload_addr`
        /// (AllocStatusRowV2 additive field, GH #241). The `None`-default
        /// `write_alloc_status` delegates here; the bridge-population
        /// mutation-gate test passes `Some(addr)` to assert the
        /// `hydrate_actual` read threads the V2 row's `workload_addr` into
        /// the `RunningAllocSet.running` map value (Obligation #2a).
        async fn write_alloc_status_with_addr(
            state: &AppState,
            alloc: &str,
            alloc_state: AllocState,
            counter: u64,
            workload_addr: Option<std::net::Ipv4Addr>,
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
                // GAP-1 subsidiary: None on Pending; fixed wall-clock otherwise.
                started_at: match alloc_state {
                    AllocState::Pending => None,
                    _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
                },
                // Per-alloc canonical workload address (AllocStatusRowV2 additive field, GH #241).
                workload_addr,
                last_terminated: None,
                restart_count: 0,
            };
            state
                .obs
                .write_alloc_lifecycle(
                    row,
                    overdrive_core::traits::observation_store::TransitionSource::Reconciler,
                )
                .await
                .expect("write alloc row");
        }

        // -------------------------------------------------------------
        // Tests (5 — within budget: 5 distinct behaviours x 2 = 10)
        // -------------------------------------------------------------

        /// S-BDB-10 unit-level proxy: an N-listener Service produces
        /// exactly N (ServiceId, ProjectedListener) entries, each
        /// keyed by `ServiceId::derive(&assigned_vip, port, protocol,
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
            let sid_8080 = ServiceId::derive(&assigned_vip, port_8080, Proto::Tcp, "service-map");
            let sid_8443 = ServiceId::derive(&assigned_vip, port_8443, Proto::Tcp, "service-map");

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
            use overdrive_core::aggregate::{JobSpecInput, JobV2};

            let tmp = TempDir::new().expect("tmpdir");
            let job = JobV2::from_submit(JobSpecInput {
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
        /// Note: `ScheduleV2::from_submit` is itself a RED scaffold
        /// (lands in a future slice per ADR-0051 OQ-5). The test
        /// constructs `ScheduleV2` directly via struct literal —
        /// the wire-arm validator is not under test here, only the
        /// hydrate path's `Schedule(_)` arm.
        #[tokio::test]
        async fn hydrate_desired_schedule_returns_empty_listeners() {
            use overdrive_core::aggregate::{CronExpr, JobSpecInput, JobV2, ScheduleV2};

            let tmp = TempDir::new().expect("tmpdir");
            let inner_job = JobV2::from_submit(JobSpecInput {
                id: WORKLOAD.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/run".to_string(),
                    args: vec![],
                }),
            })
            .expect("valid job");
            let sched = ScheduleV2 {
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
            assert!(
                s.actual.running.contains_key(&AllocationId::new("payments-0").expect("alloc id"))
            );
            assert!(
                s.actual.running.contains_key(&AllocationId::new("payments-2").expect("alloc id"))
            );
            assert_eq!(s.actual.workload_id, workload_id());
        }

        /// Obligation #2a (GH #241) — `hydrate_actual` threads each Running
        /// V2 row's per-alloc `workload_addr` into the `RunningAllocSet.running`
        /// map VALUE. Mutation-gate for the population read
        /// (`.map(|r| (r.alloc_id, r.workload_addr))`): a mesh alloc carries
        /// `Some(10.99.0.6)`; a host-netns alloc carries `None`. A mutant that
        /// drops the read (`-> None`) or swaps the field is killed by the
        /// `Some` assertion below.
        #[tokio::test]
        async fn hydrate_actual_populates_per_alloc_workload_addr() {
            let tmp = TempDir::new().expect("tmpdir");
            let state = build_state(&tmp, None).await;

            let mesh_addr = std::net::Ipv4Addr::new(10, 99, 0, 6);
            // Mesh (Path-A) alloc — carries the canonical workload_addr.
            write_alloc_status_with_addr(
                &state,
                "payments-mesh",
                AllocState::Running,
                1,
                Some(mesh_addr),
            )
            .await;
            // Host-netns alloc — no canonical workload address.
            write_alloc_status_with_addr(&state, "payments-host", AllocState::Running, 2, None)
                .await;

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
            assert_eq!(
                s.actual.running.get(&AllocationId::new("payments-mesh").expect("alloc id")),
                Some(&Some(mesh_addr)),
                "mesh alloc must carry Some(workload_addr) read verbatim from the V2 row",
            );
            assert_eq!(
                s.actual.running.get(&AllocationId::new("payments-host").expect("alloc id")),
                Some(&None),
                "host-netns alloc must carry None (no canonical workload address)",
            );
        }

        // -------------------------------------------------------------
        // Mutation-gate killing tests (step 01-03f-2 Part B)
        // -------------------------------------------------------------

        fn workload_lifecycle_reconciler() -> AnyReconciler {
            AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical())
        }

        fn service_lifecycle_reconciler() -> AnyReconciler {
            AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new())
        }

        /// Kills `reconciler_runtime.rs:1759 == → !=` in `hydrate_actual`:
        /// `workload_kind == WorkloadKind::Service` gates whether
        /// `service_spec_digest` is populated from the persisted intent
        /// digest or forced to `None`. For a persisted Service intent the
        /// digest MUST be `Some(_)`; the `!=` mutant flips it to `None`.
        #[tokio::test]
        async fn hydrate_actual_service_kind_populates_service_spec_digest() {
            let tmp = TempDir::new().expect("tmpdir");
            let intent = service_intent(&[8080]);
            let state = build_state(&tmp, Some(intent)).await;

            let result = crate::reconciler_runtime::hydrate_actual_for_test(
                &workload_lifecycle_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_actual ok");

            let AnyState::WorkloadLifecycle(s) = result else {
                panic!("expected AnyState::WorkloadLifecycle variant");
            };
            assert_eq!(
                s.workload_kind,
                WorkloadKind::Service,
                "persisted Service intent must hydrate kind == Service"
            );
            assert!(
                s.service_spec_digest.is_some(),
                "Service-kind workload MUST carry the intent spec_digest \
                 (kills == → != mutant at reconciler_runtime.rs:1759); got None"
            );
        }

        /// Write a single startup-role probe-result row for `alloc`.
        async fn write_probe(
            state: &AppState,
            alloc: &str,
            role: ProbeRole,
            probe_idx: u32,
            status: ProbeStatus,
            last_observed_at_unix_ms: u64,
        ) {
            let row = ProbeResultRow {
                alloc_id: AllocationId::new(alloc).expect("alloc id"),
                probe_idx: ProbeIdx::new(probe_idx),
                role,
                status,
                last_observed_at_unix_ms,
                inferred: false,
            };
            state.obs.write_probe_result(row).await.expect("write probe row");
        }

        /// Kills `reconciler_runtime.rs:1937 && → ||` in
        /// `hydrate_service_alloc_facts`: the per-alloc LWW probe
        /// projection filters `role == Startup && probe_idx == 0`.
        ///
        /// The SimObservationStore LWW index is keyed on
        /// `(alloc_id, probe_idx)`, so the two rows MUST carry distinct
        /// `probe_idx` values to coexist. The discriminating row is
        /// `Startup / idx 1 / Fail` at a LATER timestamp: it satisfies
        /// exactly ONE clause of the filter (`role == Startup`, but
        /// `probe_idx != 0`). Under the correct `&&` it is excluded and
        /// only the `Startup / idx 0 / Pass` row survives →
        /// `Some(Pass)`. Under the `||` mutant the idx-1 Fail row is
        /// wrongly admitted (role clause alone suffices) and, being
        /// later, wins `max_by_key(last_observed_at)` → `Some(Fail)`.
        #[tokio::test]
        async fn hydrate_service_alloc_facts_probe_filter_requires_both_role_and_idx() {
            let tmp = TempDir::new().expect("tmpdir");
            let intent = service_intent(&[8080]);
            let state = build_state(&tmp, Some(intent)).await;

            write_alloc_status(&state, "payments-0", AllocState::Running, 1).await;
            // Matching row: Startup / idx 0 / Pass at t=100 (both clauses).
            write_probe(&state, "payments-0", ProbeRole::Startup, 0, ProbeStatus::Pass, 100).await;
            // Discriminating row: Startup / idx 1 / Fail at LATER t=200.
            // `role == Startup` true but `probe_idx == 0` false — under
            // `&&` excluded; under `||` admitted (and winning by ts).
            // Distinct probe_idx keeps it from colliding with the idx-0
            // row under the store's `(alloc_id, probe_idx)` PK.
            write_probe(
                &state,
                "payments-0",
                ProbeRole::Startup,
                1,
                ProbeStatus::Fail { last_fail_reason: "mutant-bait".to_string() },
                200,
            )
            .await;

            let result = crate::reconciler_runtime::hydrate_actual_for_test(
                &service_lifecycle_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_actual ok");

            let AnyState::ServiceLifecycle(s) = result else {
                panic!("expected AnyState::ServiceLifecycle variant");
            };
            let fact = s
                .allocs
                .get(&AllocationId::new("payments-0").expect("alloc id"))
                .expect("payments-0 fact present");
            assert_eq!(
                fact.latest_startup_probe,
                Some(ProbeStatus::Pass),
                "only the Startup/idx-0 Pass row may project as latest_startup_probe; the \
                 later Startup/idx-1 Fail row must be excluded because BOTH role AND probe_idx \
                 must match (kills && → || mutant at reconciler_runtime.rs:1937); got {:?}",
                fact.latest_startup_probe
            );
        }

        /// T-SL-ADDR-1 (ADR-0079 § D8) —
        /// `service_lifecycle_advertises_workload_addr_not_host_ipv4`.
        ///
        /// `hydrate_service_alloc_facts` builds `ServiceAllocFact.backend_addr`
        /// from the alloc-status row's canonical per-workload
        /// `workload_addr`, falling back to `state.host_ipv4` — an
        /// expression byte-identical to the `BackendDiscoveryBridge`'s
        /// (`backend_discovery_bridge.rs`). Before § D8 this site used
        /// `state.host_ipv4` unconditionally, which is the PRE-MESH
        /// addressing model: the two writers of the shared
        /// `service_backends` row then disagreed on `addr` for every
        /// production mesh alloc.
        ///
        /// Mirrors the bridge-side shape of
        /// [`Self::hydrate_actual_populates_per_alloc_workload_addr`] with
        /// the same `Some` / `None` pair. **The `None` half is the
        /// regression guard that keeps the two writers' fallbacks
        /// identical** — without it a future edit to one expression
        /// silently diverges from the other. It is also what kills the
        /// `unwrap_or` mutant: dropping the fallback and always taking
        /// `workload_addr` fails the `None` assertion, while dropping the
        /// field read and always taking `host_ipv4` fails the `Some` one.
        #[tokio::test]
        async fn service_lifecycle_advertises_workload_addr_not_host_ipv4() {
            let tmp = TempDir::new().expect("tmpdir");
            let intent = service_intent(&[8080]);
            let state = build_state(&tmp, Some(intent)).await;

            let mesh_addr = std::net::Ipv4Addr::new(10, 99, 0, 6);
            // Mesh (Path-A) alloc — carries the canonical workload_addr.
            write_alloc_status_with_addr(
                &state,
                "payments-mesh",
                AllocState::Running,
                1,
                Some(mesh_addr),
            )
            .await;
            // Host-netns alloc — no canonical workload address.
            write_alloc_status_with_addr(&state, "payments-host", AllocState::Running, 2, None)
                .await;

            let result = crate::reconciler_runtime::hydrate_actual_for_test(
                &service_lifecycle_reconciler(),
                &target(),
                &state,
            )
            .await
            .expect("hydrate_actual ok");

            let AnyState::ServiceLifecycle(s) = result else {
                panic!("expected AnyState::ServiceLifecycle variant");
            };

            let mesh = s
                .allocs
                .get(&AllocationId::new("payments-mesh").expect("alloc id"))
                .expect("payments-mesh fact present");
            assert_eq!(
                mesh.backend_addr,
                std::net::SocketAddr::new(std::net::IpAddr::V4(mesh_addr), 8080),
                "a mesh alloc MUST advertise its canonical workload_addr:port, not \
                 host_ipv4:port — the pre-mesh model ADR-0079 § D8 migrates off",
            );

            let host = s
                .allocs
                .get(&AllocationId::new("payments-host").expect("alloc id"))
                .expect("payments-host fact present");
            assert_eq!(
                host.backend_addr,
                std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    8080
                ),
                "a host-netns alloc (workload_addr == None) MUST fall back to \
                 host_ipv4:port — byte-identical to the bridge's fallback, which is \
                 what keeps the two writers in agreement on BOTH branches",
            );
        }
    }

    // -----------------------------------------------------------------
    // ADR-0080 § D7 item 1 — the hydrate-boundary regression guard, and
    // the end-to-end proof that `Backend.healthy` tracks OBSERVATION.
    //
    // Before ADR-0080 `Backend.healthy` was a constant function of
    // INTENT, never of observation:
    //
    //   no readiness probe  -> `true`, always
    //   >=1 readiness probe -> `false`, PERMANENTLY
    //
    // and `false` is consumed fail-closed at three live seams
    // (`mtls_resolve_adapter::classify_by_addr` -> `MeshUnreachable`,
    // `first_healthy_backend_for` -> no frontend re-key, and
    // `dns_responder/name_index.rs` -> the name is WITHHELD ->
    // NXDOMAIN). Net operator-visible behaviour: declaring a readiness
    // probe made a Service permanently unreachable.
    //
    // Mechanism 1 (this module's target): `ProbeRunner::start_alloc`
    // assigned `probe_idx` from the flat position in the concatenated
    // `startup ++ readiness ++ liveness` TRANSPORT vector, while every
    // consumer filtered on PER-ROLE index 0. Readiness probe 0
    // therefore landed at flat index `startup_probes.len()` and was
    // never read — for the dominant production shape, since ADR-0058's
    // inference fills `startup_probes` unless the operator explicitly
    // opts out.
    //
    // WHY THE PRE-EXISTING TESTS DID NOT CATCH IT: every readiness test
    // constructed `ServiceAllocFact` BY HAND, setting
    // `latest_readiness_probe: Some(..)` alongside
    // `startup_probes_empty: false` — a combination the production
    // hydrate could not produce. Those stay valid as reconcile-branch
    // unit tests (production can produce that state now), but they can
    // never catch a producer/consumer index disagreement, because they
    // skip the producer.
    //
    // This module therefore builds NO `ServiceAllocFact`. It writes
    // `ProbeResultRow`s into a REAL redb `LocalObservationStore` and
    // enters through the production `hydrate_actual` and the production
    // `ServiceLifecycleReconciler::reconcile`, asserting on the
    // `healthy` flag of the emitted `Action::WriteServiceBackendRow` —
    // the driven-port boundary the dataplane and the withhold seams
    // consume.
    //
    // Homed in-src rather than under `tests/integration/` (where
    // ADR-0080 § D7 item 1 sketches it) because `AnyReconciler` /
    // `AnyState` are `pub(crate)`; reaching `hydrate_actual_for_test`
    // from an external test crate would require widening the crate's
    // public API, which the design does not sanction.
    // -----------------------------------------------------------------
    mod readiness_gates_backend_healthy {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use overdrive_core::UnixInstant;
        use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
        use overdrive_core::aggregate::{
            DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent,
            WorkloadKind,
        };
        use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
        use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
        use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
        use overdrive_core::reconcilers::{Action, Reconciler, TargetResource, TickContext};
        use overdrive_core::traits::driver::DriverType;
        use overdrive_core::traits::intent_store::IntentStore;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
        };
        use overdrive_reconcilers::service_lifecycle::{
            ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
        };
        use overdrive_reconcilers::{AnyReconciler, AnyState};
        use overdrive_sim::adapters::clock::SimClock;
        use overdrive_sim::adapters::dataplane::SimDataplane;
        use overdrive_sim::adapters::driver::SimDriver;
        use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
        use tempfile::TempDir;

        use crate::AppState;
        use crate::reconciler_runtime::{ReconcilerRuntime, hydrate_actual_for_test};

        const WORKLOAD: &str = "readiness-gate-svc";
        const ALLOC: &str = "readiness-gate-svc-0";

        fn workload_id() -> WorkloadId {
            WorkloadId::new(WORKLOAD).expect("valid workload id")
        }

        fn writer_node() -> NodeId {
            NodeId::new("writer-1").expect("valid NodeId")
        }

        fn alloc_id() -> AllocationId {
            AllocationId::new(ALLOC).expect("valid alloc id")
        }

        fn target() -> TargetResource {
            TargetResource::new(&format!("workload/{WORKLOAD}")).expect("valid target")
        }

        /// The production-typical Service shape: ONE startup probe AND
        /// ONE readiness probe.
        ///
        /// The startup probe is load-bearing for this guard. It is what
        /// makes `startup_probes.len() == 1`, which under the
        /// pre-ADR-0080 flat indexing put readiness probe 0 at flat
        /// index 1 — invisible to a consumer filtering on
        /// `probe_idx == 0`. A fixture with an empty startup array
        /// would pass even against the broken producer, which is
        /// exactly why the defect survived. ADR-0058's inference gives
        /// real Services a startup probe unless the operator opts out,
        /// so this is the shape that matters.
        fn service_intent_with_startup_and_readiness() -> WorkloadIntent {
            let tcp_probe = |role: ProbeRole| ProbeDescriptor {
                // BOTH probes sit at per-role index 0 — that is the
                // point: they are DISTINCT probes that share an index
                // and are separated by `role`, in the descriptor
                // (§ D1) and in the durable key (§ D2) alike.
                idx: ProbeIdx::new(0),
                role,
                mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port: 8080 },
                timeout_seconds: 5,
                interval_seconds: 2,
                max_attempts: 30,
                failure_threshold: None,
                success_threshold: if role == ProbeRole::Readiness { Some(1) } else { None },
                inferred: false,
            };

            let svc = ServiceV2::from_submit(ServiceSpecInput {
                id: WORKLOAD.to_owned(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/serve".to_owned(),
                    args: vec![],
                }),
                listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
                startup_probes: vec![tcp_probe(ProbeRole::Startup)],
                readiness_probes: vec![tcp_probe(ProbeRole::Readiness)],
                liveness_probes: vec![],
            })
            .expect("canonical Service spec is valid");
            WorkloadIntent::Service(svc)
        }

        /// Build an `AppState` whose observation store is a REAL redb
        /// [`LocalObservationStore`] — not the in-memory sim adapter.
        /// The composite probe-result key this guard depends on is
        /// byte-encoded only in the redb backend, so the real store is
        /// what makes the assertion meaningful.
        async fn build_state(tmp: &TempDir, intent: &WorkloadIntent) -> AppState {
            let runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
                .expect("runtime new");
            let store_path = tmp.path().join("intent.redb");
            let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
            let obs: Arc<dyn ObservationStore> = Arc::new(
                LocalObservationStore::open(tmp.path().join("observation.redb"))
                    .expect("open observation store"),
            );

            let key = IntentKey::for_workload(&workload_id());
            let archived = intent.archive_for_store().expect("rkyv archive");
            store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
            let kind_key = IntentKey::for_workload_kind(&workload_id());
            store
                .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
                .await
                .expect("put kind");

            let allocator =
                crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
            AppState::new(
                store,
                store_path,
                obs,
                Arc::new(runtime),
                Arc::new(SimDriver::new(DriverType::Exec)),
                Arc::new(SimClock::new()),
                Arc::new(SimDataplane::new()),
                Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                    overdrive_sim::adapters::entropy::SimEntropy::new(0),
                ))),
                Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
                writer_node(),
                allocator,
                crate::test_empty_listener_facts(),
                std::net::Ipv4Addr::LOCALHOST,
            )
        }

        /// Allocate the Service's VIP through the production allocator
        /// path — without it `service_dataplane_identity` returns
        /// `None` and the readiness branch is a no-op (no row to write).
        async fn allocate_vip(state: &AppState, intent: &WorkloadIntent) {
            let digest = intent.spec_digest().expect("spec_digest");
            let bytes: [u8; 32] = *digest.as_bytes();
            let mut guard = state.allocator.lock().await;
            guard.allocate(bytes).await.expect("allocate vip");
            drop(guard);
        }

        async fn write_running_alloc(state: &AppState) {
            let row = AllocStatusRow {
                alloc_id: alloc_id(),
                workload_id: workload_id(),
                node_id: NodeId::new("local").expect("node id"),
                state: AllocState::Running,
                updated_at: LogicalTimestamp { counter: 1, writer: writer_node() },
                reason: None,
                detail: None,
                terminal: None,
                stderr_tail: None,
                kind: WorkloadKind::Service,
                listeners: Vec::new(),
                started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(
                    1_700_000_000,
                ))),
                workload_addr: None,
                last_terminated: None,
                restart_count: 0,
            };
            state
                .obs
                .write_alloc_lifecycle(
                    row,
                    overdrive_core::traits::observation_store::TransitionSource::Reconciler,
                )
                .await
                .expect("write Running alloc row");
        }

        /// Write one probe observation exactly as `ProbeRunner`'s
        /// `supervised_probe_loop` does — through `write_probe_result`,
        /// at the descriptor's per-role `idx`.
        async fn write_probe(
            state: &AppState,
            role: ProbeRole,
            status: ProbeStatus,
            observed_at: u64,
        ) {
            state
                .obs
                .write_probe_result(ProbeResultRow {
                    alloc_id: alloc_id(),
                    probe_idx: ProbeIdx::new(0),
                    role,
                    status,
                    last_observed_at_unix_ms: observed_at,
                    inferred: false,
                })
                .await
                .expect("write probe result");
        }

        fn tick_at(counter: u64) -> TickContext {
            let now = Instant::now();
            TickContext {
                now,
                now_unix: UnixInstant::from_unix_duration(Duration::from_secs(
                    1_700_000_000 + counter,
                )),
                tick: counter,
                deadline: now + Duration::from_secs(1),
            }
        }

        /// Hydrate through the production path and return the
        /// `ServiceLifecycleState` the reconciler consumes.
        async fn hydrate(state: &AppState) -> ServiceLifecycleState {
            let reconciler = AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new());
            let hydrated = hydrate_actual_for_test(&reconciler, &target(), state)
                .await
                .expect("hydrate_actual must succeed");
            let AnyState::ServiceLifecycle(s) = hydrated else {
                panic!("expected AnyState::ServiceLifecycle");
            };
            s
        }

        /// Reconcile through the production reconciler and extract the
        /// `healthy` flag of the single backend on the emitted
        /// `WriteServiceBackendRow` — the driven-port boundary the
        /// dataplane and the DNS / mesh withhold seams consume.
        fn reconcile_backend_healthy(
            actual: &ServiceLifecycleState,
            view: &ServiceLifecycleView,
            tick: &TickContext,
        ) -> (Option<bool>, ServiceLifecycleView) {
            let reconciler = ServiceLifecycleReconciler::new();
            // `ServiceLifecycleReconciler::reconcile` ignores its
            // `desired` parameter (`_desired`), so passing `actual`
            // twice is the honest call shape here.
            let (actions, next_view) = reconciler.reconcile(actual, actual, view, tick);
            let healthy = actions.iter().find_map(|a| match a {
                Action::WriteServiceBackendRow { row, .. } => {
                    Some(row.backends.first().expect("one Running backend").healthy)
                }
                _ => None,
            });
            (healthy, next_view)
        }

        /// **The guard.** `Backend.healthy` is a function of the
        /// readiness OBSERVATION, driven end-to-end through the
        /// production path.
        ///
        /// 1. A Service declaring one startup AND one readiness probe,
        ///    with an allocated VIP and one `Running` alloc.
        /// 2. `startup/0 = Pass`, `readiness/0 = Fail` written through
        ///    the real store -> hydrate -> the fact carries
        ///    `latest_readiness_probe = Some(Fail)` (§ D7 item 1: this
        ///    is `None` under the broken producer, because readiness
        ///    landed at flat index 1) -> reconcile emits
        ///    `healthy: false`.
        /// 3. `readiness/0 = Pass` at a later timestamp -> hydrate ->
        ///    reconcile emits `healthy: true`.
        ///
        /// Step 3 is what proves the flag TRACKS observation rather
        /// than merely being derivable as `false`: pre-fix `healthy`
        /// was permanently `false` for any Service with a readiness
        /// probe, so a recovery could never flip it back.
        #[tokio::test]
        async fn readiness_observation_drives_backend_healthy_false_then_true() {
            let tmp = TempDir::new().expect("tempdir");
            let intent = service_intent_with_startup_and_readiness();
            let state = build_state(&tmp, &intent).await;
            allocate_vip(&state, &intent).await;
            write_running_alloc(&state).await;

            // ---- Tick 1: startup Pass, readiness FAIL.
            write_probe(&state, ProbeRole::Startup, ProbeStatus::Pass, 1_000).await;
            write_probe(
                &state,
                ProbeRole::Readiness,
                ProbeStatus::Fail { last_fail_reason: "backend still warming".to_owned() },
                1_000,
            )
            .await;

            let actual = hydrate(&state).await;
            let fact =
                actual.allocs.get(&alloc_id()).expect("the Running alloc must hydrate to a fact");

            // § D7 item 1 — the direct hydrate-boundary assertion.
            // Under the pre-ADR-0080 producer this is `None`: the
            // readiness descriptor was written at flat index 1 (because
            // `startup_probes.len() == 1`) while the consumer filters
            // on per-role index 0.
            assert_eq!(
                fact.latest_readiness_probe,
                Some(ProbeStatus::Fail { last_fail_reason: "backend still warming".to_owned() }),
                "the readiness observation MUST reach the hydrated fact. `None` here means \
                 the producer and the consumer disagree about what `probe_idx` indexes — the \
                 defect ADR-0080 § D1 fixes by carrying the parser-assigned per-role index \
                 verbatim into ProbeRunner::start_alloc",
            );
            assert!(fact.has_readiness_probe, "the spec declares a readiness probe");
            assert!(
                !fact.startup_probes_empty,
                "the fixture declares a startup probe — the production-typical shape under \
                 ADR-0058 inference, and the shape the flat index broke",
            );

            let (healthy_on_fail, view_after_fail) =
                reconcile_backend_healthy(&actual, &ServiceLifecycleView::default(), &tick_at(1));
            assert_eq!(
                healthy_on_fail,
                Some(false),
                "a readiness Fail must gate traffic off: healthy=false. The flag is consumed \
                 fail-closed — MeshUnreachable on dial, and the name withheld from DNS",
            );

            // ---- Tick 2: readiness recovers.
            write_probe(&state, ProbeRole::Readiness, ProbeStatus::Pass, 2_000).await;

            let actual = hydrate(&state).await;
            let fact = actual
                .allocs
                .get(&alloc_id())
                .expect("the Running alloc must still hydrate to a fact");
            assert_eq!(
                fact.latest_readiness_probe,
                Some(ProbeStatus::Pass),
                "the later Pass wins LWW at (alloc, Readiness, 0)",
            );

            let (healthy_on_pass, _) =
                reconcile_backend_healthy(&actual, &view_after_fail, &tick_at(2));
            assert_eq!(
                healthy_on_pass,
                Some(true),
                "a readiness Pass at or above the success threshold must admit traffic: \
                 healthy=true. Pre-ADR-0080 `healthy` was PERMANENTLY false for any Service \
                 declaring a readiness probe, so recovery was unrepresentable",
            );
        }

        /// The startup probe's own observation is unaffected by the
        /// readiness row that shares its `probe_idx` — the two are
        /// separated by `role` in the descriptor and in the durable key
        /// alike.
        ///
        /// Consumer-side companion to the store-level § A2 guard: it
        /// fails if `role` is dropped from the key (the LATER readiness
        /// Fail would clobber the startup row and surface as
        /// `latest_startup_probe`), and it fails if the per-role index
        /// is reverted to a flat one (startup would still match by
        /// accident of ordering, but readiness would go `None`).
        #[tokio::test]
        async fn startup_and_readiness_observations_at_index_zero_do_not_shadow_each_other() {
            let tmp = TempDir::new().expect("tempdir");
            let intent = service_intent_with_startup_and_readiness();
            let state = build_state(&tmp, &intent).await;
            allocate_vip(&state, &intent).await;
            write_running_alloc(&state).await;

            // Startup Pass FIRST, readiness Fail LATER — so a key that
            // omits `role` would let the readiness Fail win LWW and be
            // surfaced as the startup status.
            write_probe(&state, ProbeRole::Startup, ProbeStatus::Pass, 1_000).await;
            write_probe(
                &state,
                ProbeRole::Readiness,
                ProbeStatus::Fail { last_fail_reason: "not ready".to_owned() },
                9_000,
            )
            .await;

            let actual = hydrate(&state).await;
            let fact =
                actual.allocs.get(&alloc_id()).expect("the Running alloc must hydrate to a fact");

            assert_eq!(
                fact.latest_startup_probe,
                Some(ProbeStatus::Pass),
                "the startup observation is untouched by a LATER readiness write at the same \
                 probe_idx; surfacing the readiness Fail here means `role` left the durable key",
            );
            assert_eq!(
                fact.latest_readiness_probe,
                Some(ProbeStatus::Fail { last_fail_reason: "not ready".to_owned() }),
                "and the readiness observation survives alongside it",
            );
        }
    }

    // -----------------------------------------------------------------
    // workflow-lifecycle hydrate boundary — regression guard against
    // the redundant double-scan of the `workflows/` intent prefix.
    //
    // `WorkflowLifecycle::reconcile` reads ONLY `actual` (the merged
    // desired+actual projection); its `desired` parameter is `_desired`
    // (unused). Meanwhile `hydrate_actual` → `hydrate_workflow_actual_
    // instances` already starts from the intent SSOT scan as its base.
    // The `hydrate_desired` arm must therefore NOT scan a second time —
    // it returns an empty `WorkflowLifecycleState`. This module pins
    // that contract so the discarded second scan cannot be reintroduced.
    // -----------------------------------------------------------------
    mod workflow_lifecycle_hydrate {
        use std::sync::Arc;

        use overdrive_core::aggregate::IntentKey;
        use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
        use overdrive_core::reconcilers::TargetResource;
        use overdrive_core::traits::driver::{Driver, DriverType};
        use overdrive_core::traits::intent_store::IntentStore;
        use overdrive_core::traits::observation_store::ObservationStore;
        use overdrive_core::workflow::{WorkflowName, WorkflowStart};
        use overdrive_reconcilers::AnyState;
        use overdrive_sim::adapters::clock::SimClock;
        use overdrive_sim::adapters::dataplane::SimDataplane;
        use overdrive_sim::adapters::driver::SimDriver;
        use overdrive_sim::adapters::observation_store::SimObservationStore;
        use overdrive_store_local::LocalIntentStore;
        use tempfile::TempDir;

        use crate::AppState;
        use crate::reconciler_runtime::{
            ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
        };

        fn writer_node() -> NodeId {
            NodeId::new("writer-1").expect("valid NodeId")
        }

        fn wf_target() -> TargetResource {
            TargetResource::new("workflow/all").expect("valid target")
        }

        fn provision_spec() -> WorkflowStart {
            WorkflowStart {
                name: WorkflowName::new("provision-record").expect("valid workflow name"),
                input: Vec::new(),
            }
        }

        fn correlation_for(spec: &WorkflowStart) -> CorrelationKey {
            CorrelationKey::derive(
                "wf-provision-0001",
                &ContentHash::of(spec.name.as_str().as_bytes()),
                "start-workflow",
            )
        }

        /// Build an `AppState` over a real (tempdir) `LocalIntentStore`
        /// and persist one workflow-instance desired-intent row at
        /// `workflows/<correlation>` — the exact key/value shape the
        /// production `persist_workflow_intents` writes for a committed
        /// `Action::StartWorkflow`.
        async fn build_state_with_workflow_intent(
            tmp: &TempDir,
            spec: &WorkflowStart,
            correlation: &CorrelationKey,
        ) -> AppState {
            let runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
                .expect("runtime new");
            let store_path = tmp.path().join("intent.redb");
            let store =
                Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
            let obs: Arc<dyn ObservationStore> =
                Arc::new(SimObservationStore::single_peer(writer_node(), 0));

            // Mirror `persist_workflow_intents`: key =
            // `IntentKey::for_workflow_instance(correlation)`, value =
            // the FULL `WorkflowStart` spec via the co-located codec.
            let key = IntentKey::for_workflow_instance(correlation);
            let archived = spec.archive_for_store().expect("archive WorkflowStart");
            store.put(key.as_bytes(), archived.as_ref()).await.expect("put workflow intent");

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
                Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                    overdrive_sim::adapters::entropy::SimEntropy::new(0),
                ))),
                Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
                writer_node(),
                allocator,
                crate::test_empty_listener_facts(),
                std::net::Ipv4Addr::LOCALHOST,
            )
        }

        /// Regression: `hydrate_desired` for the workflow-lifecycle
        /// reconciler must NOT scan the `workflows/` prefix — it returns
        /// an empty `WorkflowLifecycleState`. The same prefix is scanned
        /// by `hydrate_actual` (whose `WorkflowInstanceState` the pure
        /// reconcile body actually reads), so a desired-side scan is a
        /// redundant second read whose result is discarded by
        /// `reconcile(_desired, actual, ...)`.
        ///
        /// The companion `hydrate_actual` assertion makes this
        /// non-vacuous: it proves the intent row IS persisted and IS
        /// readable, so "desired is empty" reflects the new contract, not
        /// a missing fixture.
        #[tokio::test]
        async fn hydrate_desired_does_not_rescan_workflow_intent() {
            let tmp = TempDir::new().expect("tmpdir");
            let spec = provision_spec();
            let correlation = correlation_for(&spec);
            let state = build_state_with_workflow_intent(&tmp, &spec, &correlation).await;
            let reconciler = crate::workflow_lifecycle();

            // hydrate_actual scans the intent prefix as its base and
            // surfaces the persisted instance — running-in-intent with no
            // live engine task and no terminal row (the empty-registry
            // default engine holds no live tasks).
            let actual = hydrate_actual_for_test(&reconciler, &wf_target(), &state)
                .await
                .expect("hydrate_actual ok");
            let AnyState::WorkflowLifecycle(actual_state) = actual else {
                panic!("expected WorkflowLifecycle actual state");
            };
            let instance = actual_state
                .instances
                .get(&correlation)
                .expect("hydrate_actual must surface the persisted workflow instance");
            assert!(
                instance.running_in_intent,
                "the persisted workflow intent marks the instance running-in-intent"
            );

            // hydrate_desired must NOT scan again — the desired side is
            // empty by design (the merged projection lives in `actual`).
            let desired = hydrate_desired_for_test(&reconciler, &wf_target(), &state)
                .await
                .expect("hydrate_desired ok");
            let AnyState::WorkflowLifecycle(desired_state) = desired else {
                panic!("expected WorkflowLifecycle desired state");
            };
            assert!(
                desired_state.instances.is_empty(),
                "hydrate_desired for the workflow-lifecycle reconciler must NOT re-scan the \
                 `workflows/` intent prefix — reconcile reads only `actual`, so the desired side \
                 returns an empty WorkflowLifecycleState; got {} instance(s)",
                desired_state.instances.len()
            );
        }
    }

    // -----------------------------------------------------------------
    // persist_view eq-diff skip — the WorkflowLifecycle arm elides the
    // fsync `write_through` when the next view equals the current one.
    // The Phase 1 `WorkflowLifecycleView` is an empty struct, so the
    // comparison is ALWAYS equal and existing tests cannot distinguish
    // the `==` guard from its `!=` mutant (both leave the empty view
    // observably identical). The only behavioural effect the guard
    // controls is whether the durable fsync fires — observed here via a
    // call-counting spy `ViewStore`.
    // -----------------------------------------------------------------
    mod workflow_view_persist_elision {
        use std::collections::BTreeMap;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use async_trait::async_trait;
        use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
        use overdrive_reconcilers::{AnyReconcilerView, WorkflowLifecycleView};
        use tempfile::TempDir;

        use crate::reconciler_runtime::ReconcilerRuntime;
        use crate::view_store::{ProbeError, Result as ViewStoreResult, ViewStore};

        /// Spy `ViewStore` that counts `write_through_bytes` (fsync) calls.
        /// Storage is a no-op — the test observes only whether the durable
        /// write fired, which is the sole behavioural effect the eq-diff
        /// skip in `persist_view`'s WorkflowLifecycle arm controls.
        #[derive(Default)]
        struct CountingViewStore {
            write_through_calls: AtomicUsize,
        }

        #[async_trait]
        impl ViewStore for CountingViewStore {
            async fn bulk_load_bytes(
                &self,
                _reconciler: &'static str,
            ) -> ViewStoreResult<BTreeMap<TargetResource, Vec<u8>>> {
                Ok(BTreeMap::new())
            }

            async fn write_through_bytes(
                &self,
                _reconciler: &'static str,
                _target: &TargetResource,
                _cbor: &[u8],
            ) -> ViewStoreResult<()> {
                self.write_through_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            async fn delete(
                &self,
                _reconciler: &'static str,
                _target: &TargetResource,
            ) -> ViewStoreResult<()> {
                Ok(())
            }

            async fn probe(&self) -> std::result::Result<(), ProbeError> {
                Ok(())
            }
        }

        /// Kills `reconciler_runtime.rs:607 == → !=` in `persist_view`'s
        /// WorkflowLifecycle arm. The Phase 1 `WorkflowLifecycleView` is an
        /// empty struct, so a freshly-hydrated `current` (default) always
        /// equals the `next_view` the runtime persists — the eq-diff skip
        /// MUST fire and elide the fsync `write_through` on every tick (the
        /// optimization the arm's doc-comment promises). Under the correct
        /// `==` the spy records ZERO write_through calls; the `!=` mutant
        /// inverts the guard so the early return never fires and the fsync
        /// runs (count == 1), failing this assertion.
        #[tokio::test]
        async fn persist_view_elides_fsync_for_unchanged_workflow_view() {
            let tmp = TempDir::new().expect("tmpdir");
            let spy = Arc::new(CountingViewStore::default());
            let mut runtime =
                ReconcilerRuntime::new(tmp.path(), Arc::clone(&spy) as Arc<dyn ViewStore>)
                    .expect("runtime::new");
            runtime
                .register(crate::workflow_lifecycle())
                .await
                .expect("register workflow-lifecycle");

            let name = ReconcilerName::new("workflow-lifecycle").expect("valid reconciler name");
            let target = TargetResource::new("workflow/all").expect("valid target");

            // The persisted view equals the freshly-hydrated default (empty
            // struct) — the eq-diff skip must fire and elide the fsync.
            runtime
                .persist_view(
                    &name,
                    &target,
                    AnyReconcilerView::WorkflowLifecycle(WorkflowLifecycleView::default()),
                )
                .await
                .expect("persist_view ok");

            assert_eq!(
                spy.write_through_calls.load(Ordering::SeqCst),
                0,
                "persisting an unchanged (empty) WorkflowLifecycleView must elide the fsync \
                 write_through (eq-diff skip at persist_view's WorkflowLifecycle arm); a non-zero \
                 count means the `current == view` guard was inverted (kills the == → != mutant)"
            );
        }
    }
}
