//! `overdrive-reconcilers` — first-party reconciler impls + enum dispatch.
//!
//! Extracted out of `overdrive-core` per ADR-0086 (reconcilers own their
//! hydration). The reconciler CONTRACT — the [`Reconciler`] trait, [`Action`],
//! [`TickContext`], [`ReconcilerName`], [`TargetResource`], the resync/interest
//! vocabulary, `HydrationContext` / `HydrateError`, and the 4 read-port traits
//! — STAYS in [`overdrive_core::reconcilers`]. This crate holds the 8 impls,
//! the three dispatch enums ([`AnyReconciler`] / [`AnyState`] /
//! [`AnyReconcilerView`]), the per-reconciler `State`/`View` projections, the
//! `service_lifecycle` module, and the pure helpers. It depends only DOWN on
//! `overdrive-core` — the edge that breaks the Cargo cycle (ADR-0086 D3/D4).
//!
//! `crate_class = "adapter-host"` (ADR-0086 D3): the impls carry impure `async`
//! hydration (real store reads through injected ports, landing in step 02-04),
//! so the crate is off the dst-lint whole-crate `core` scan.
//!
//! The impl modules and the three dispatch enums live directly at the crate
//! root (flat `src/` layout — no `reconcilers/` subdirectory, which would be
//! redundant against the crate name; every symbol resolves at
//! `overdrive_reconcilers::…`). The contract vocabulary is re-imported
//! (privately) from `overdrive_core::reconcilers` below so the impl submodules
//! keep referencing it via `super::` unchanged.
//!
//! [`Reconciler`]: overdrive_core::reconcilers::Reconciler
//! [`Action`]: overdrive_core::reconcilers::Action
//! [`TickContext`]: overdrive_core::reconcilers::TickContext
//! [`ReconcilerName`]: overdrive_core::reconcilers::ReconcilerName
//! [`TargetResource`]: overdrive_core::reconcilers::TargetResource

// `expect`/`unwrap` are the standard idiom in this crate's inline test modules;
// production reconciler code keeps them warned. Mirrors the `overdrive-core`
// crate-level posture the impls carried before the step 02-02 extraction.
#![cfg_attr(not(test), warn(clippy::expect_used, clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
// The reconciler impls relocated verbatim from `overdrive-core` (step 02-02),
// where they lived under this same crate-level suppression. Their mature
// docstrings carry long first paragraphs and un-backticked prose items; the
// mechanical move does not rewrite them. `expect` (not `allow`) so the
// suppression self-removes if the docs are ever tightened.
#![expect(
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    reason = "reconciler impls relocated from overdrive-core; same suppression they carried there"
)]

// Contract vocabulary — DEFINED in `overdrive-core`, re-imported here so the
// impl submodules resolve `super::{Action, Reconciler, …}` unchanged and the
// dispatch enums' public signatures name the core types. NOT re-exported from
// this crate's root (contract-in-core; importers spell `overdrive_core::…`).
use overdrive_core::id::{NodeId, ServiceId, WorkloadId};
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, ResyncSchedule,
    ResyncScope, TargetResource, TickContext, resolve_scope,
};
use overdrive_core::traits::observation_store::ObservationRowKind;

pub mod backend_discovery_bridge;
pub mod noop_heartbeat;
pub mod service_lifecycle;
pub mod service_map_hydrator;
pub mod svid_lifecycle;
pub mod vm_reclamation;
pub mod workflow_lifecycle;
pub mod workload_lifecycle;

// Flat re-exports so importers spell `overdrive_reconcilers::<Symbol>` for the
// moved impls / State / View / helpers (mirrors the ergonomic top-level access
// the impls had inside `overdrive_core::reconcilers` before the move).
pub use backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
};
pub use noop_heartbeat::NoopHeartbeat;
pub use service_map_hydrator::{
    BackendAddressRejection, RetryMemory, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, classify_backend_address,
};
pub use svid_lifecycle::{RunningAlloc, SvidLifecycle, SvidLifecycleState, SvidLifecycleView};
pub use vm_reclamation::{
    SupervisionSet, VmAllocFacts, VmReclamation, VmReclamationState, VmReclamationView,
    plan_reclamation,
};
pub use workflow_lifecycle::{
    WorkflowInstanceState, WorkflowLifecycle, WorkflowLifecycleState, WorkflowLifecycleView,
};
pub use workload_lifecycle::{
    RESTART_BACKOFF_CEILING, RESTART_BACKOFF_DURATION, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView, backoff_for_attempt, project_probe_descriptors,
    project_service_listen_ports,
};

// `ServiceLifecycleReconciler` lives in `crate::service_lifecycle` (a sibling
// module) for the cycle-breaking reasons documented at that module's header.
// Re-import here so the dispatch enums can reference it without forcing every
// dispatcher to spell the full path.
use crate::service_lifecycle::{
    ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

// `NodeId` / `resolve_scope` / `ResyncScope` / `TargetResource` are pulled in
// above for the impl submodules' `super::` references (test modules resolve
// scope-target derivations against them). Reference them here so a future
// refactor that drops an impl's use does not silently orphan the re-import.
const _: fn(ResyncScope, &NodeId) -> Vec<TargetResource> = resolve_scope;

// ---------------------------------------------------------------------------
// AnyState enum — per-reconciler typed `desired`/`actual` projection
// ---------------------------------------------------------------------------

/// Sum of every `desired`/`actual` shape consumed by a registered reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyState {
    /// `State = ()` variant for Phase 1 reconcilers that do not
    /// dereference their projection (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's typed projection — see
    /// [`WorkloadLifecycleState`].
    WorkloadLifecycle(WorkloadLifecycleState),
    /// `WorkflowLifecycle` reconciler's typed projection — see
    /// [`WorkflowLifecycleState`] (ADR-0064 §5).
    WorkflowLifecycle(WorkflowLifecycleState),
    /// `ServiceMapHydrator` reconciler's typed projection — see
    /// [`ServiceMapHydratorState`].
    ServiceMapHydrator(ServiceMapHydratorState),
    /// `BackendDiscoveryBridge` reconciler's typed projection — see
    /// [`backend_discovery_bridge::BackendDiscoveryBridgeState`].
    BackendDiscoveryBridge(BackendDiscoveryBridgeState),
    /// `ServiceLifecycle` reconciler's typed projection — see
    /// [`crate::service_lifecycle::ServiceLifecycleState`]. Per
    /// ADR-0055; landed by the `service-health-check-probes` feature.
    ServiceLifecycle(ServiceLifecycleState),
    /// `SvidLifecycle` reconciler's typed projection — see
    /// [`SvidLifecycleState`] (ADR-0067 D1: `desired = running allocs`,
    /// `actual = the IdentityMgr held set`).
    SvidLifecycle(SvidLifecycleState),
    /// `VmReclamation` reconciler's typed projection — see
    /// [`vm_reclamation::VmReclamationState`] (SD-1's Bar-2 reconciler,
    /// ADR-0083 §D7 / `brief.md` §105a).
    VmReclamation(vm_reclamation::VmReclamationState),
}

// ---------------------------------------------------------------------------
// AnyReconciler — enum-dispatch replacement for Box<dyn Reconciler>
// ---------------------------------------------------------------------------

/// Enum-dispatched wrapper over every first-party reconciler kind.
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// First real (non-proof-of-life) reconciler.
    WorkloadLifecycle(WorkloadLifecycle),
    /// The workflow-lifecycle reconciler — manages WHICH workflow
    /// instances exist; re-emits `StartWorkflow` on restart (ADR-0064 §5).
    WorkflowLifecycle(WorkflowLifecycle),
    /// Phase 2 — `service-map-hydrator`.
    ServiceMapHydrator(ServiceMapHydrator),
    /// Phase 2.2 — `backend-discovery-bridge`.
    BackendDiscoveryBridge(BackendDiscoveryBridge),
    /// Service-health-check-probes — `service-lifecycle` per
    /// ADR-0055. See [`crate::service_lifecycle::ServiceLifecycleReconciler`].
    ServiceLifecycle(ServiceLifecycleReconciler),
    /// Workload-identity-manager — `svid-lifecycle` per ADR-0067 D1.
    /// Converges `desired = running allocs` against `actual = held set`,
    /// emitting `IssueSvid` / `DropSvid`. See [`SvidLifecycle`].
    SvidLifecycle(SvidLifecycle),
    /// SD-1's Bar-2 reconciler — `vm-reclamation` per ADR-0083 §D7 /
    /// `brief.md` §105a. Converges `desired = VM allocations` against
    /// `actual = observed host state + supervision`, emitting
    /// `ReclaimAllocation` / `DiscardStrandedArtifacts`. See
    /// [`vm_reclamation::VmReclamation`].
    VmReclamation(vm_reclamation::VmReclamation),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            Self::WorkloadLifecycle(r) => r.name(),
            Self::WorkflowLifecycle(r) => r.name(),
            Self::ServiceMapHydrator(r) => r.name(),
            Self::BackendDiscoveryBridge(r) => r.name(),
            Self::ServiceLifecycle(r) => r.name(),
            Self::SvidLifecycle(r) => r.name(),
            Self::VmReclamation(r) => r.name(),
        }
    }

    /// Canonical name as the inner reconciler's `Self::NAME` const —
    /// a `&'static str` aliased to the binary's data segment.
    #[must_use]
    pub const fn static_name(&self) -> &'static str {
        match self {
            Self::NoopHeartbeat(_) => <NoopHeartbeat as Reconciler>::NAME,
            Self::WorkloadLifecycle(_) => <WorkloadLifecycle as Reconciler>::NAME,
            Self::WorkflowLifecycle(_) => <WorkflowLifecycle as Reconciler>::NAME,
            Self::ServiceMapHydrator(_) => <ServiceMapHydrator as Reconciler>::NAME,
            Self::BackendDiscoveryBridge(_) => <BackendDiscoveryBridge as Reconciler>::NAME,
            Self::ServiceLifecycle(_) => <ServiceLifecycleReconciler as Reconciler>::NAME,
            Self::SvidLifecycle(_) => <SvidLifecycle as Reconciler>::NAME,
            Self::VmReclamation(_) => <vm_reclamation::VmReclamation as Reconciler>::NAME,
        }
    }

    /// Declarative resync cadence of the inner reconciler — forwards to
    /// [`Reconciler::resync_schedule`] across all variants, exactly like
    /// [`AnyReconciler::name`]. Adds no `AnyState` / `AnyReconcilerView`
    /// / reconcile-dispatch change (ADR-0084 §3).
    #[must_use]
    pub fn resync_schedule(&self) -> Option<ResyncSchedule> {
        match self {
            Self::NoopHeartbeat(r) => r.resync_schedule(),
            Self::WorkloadLifecycle(r) => r.resync_schedule(),
            Self::WorkflowLifecycle(r) => r.resync_schedule(),
            Self::ServiceMapHydrator(r) => r.resync_schedule(),
            Self::BackendDiscoveryBridge(r) => r.resync_schedule(),
            Self::ServiceLifecycle(r) => r.resync_schedule(),
            Self::SvidLifecycle(r) => r.resync_schedule(),
            Self::VmReclamation(r) => r.resync_schedule(),
        }
    }

    /// Declarative event-interests of the inner reconciler — forwards to
    /// [`Reconciler::interests`] across all variants, exactly like
    /// [`AnyReconciler::name`] / [`AnyReconciler::resync_schedule`]. Adds no
    /// `AnyState` / `AnyReconcilerView` / reconcile-dispatch change
    /// (ADR-0084 §3).
    #[must_use]
    pub fn interests(&self) -> &'static [ObservationRowKind] {
        match self {
            Self::NoopHeartbeat(r) => r.interests(),
            Self::WorkloadLifecycle(r) => r.interests(),
            Self::WorkflowLifecycle(r) => r.interests(),
            Self::ServiceMapHydrator(r) => r.interests(),
            Self::BackendDiscoveryBridge(r) => r.interests(),
            Self::ServiceLifecycle(r) => r.interests(),
            Self::SvidLifecycle(r) => r.interests(),
            Self::VmReclamation(r) => r.interests(),
        }
    }

    /// Pure compute phase — dispatches to the inner reconciler's
    /// `reconcile`.
    #[must_use]
    pub fn reconcile(
        &self,
        desired: &AnyState,
        actual: &AnyState,
        view: &AnyReconcilerView,
        tick: &TickContext,
    ) -> (Vec<Action>, AnyReconcilerView) {
        match (self, desired, actual, view) {
            (Self::NoopHeartbeat(r), AnyState::Unit, AnyState::Unit, AnyReconcilerView::Unit) => {
                let (actions, ()) = r.reconcile(&(), &(), &(), tick);
                (actions, AnyReconcilerView::Unit)
            }
            (
                Self::WorkloadLifecycle(r),
                AnyState::WorkloadLifecycle(desired),
                AnyState::WorkloadLifecycle(actual),
                AnyReconcilerView::WorkloadLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::WorkloadLifecycle(next_view))
            }
            (
                Self::WorkflowLifecycle(r),
                AnyState::WorkflowLifecycle(desired),
                AnyState::WorkflowLifecycle(actual),
                AnyReconcilerView::WorkflowLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::WorkflowLifecycle(next_view))
            }
            (
                Self::ServiceMapHydrator(r),
                AnyState::ServiceMapHydrator(desired),
                AnyState::ServiceMapHydrator(actual),
                AnyReconcilerView::ServiceMapHydrator(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::ServiceMapHydrator(next_view))
            }
            (
                Self::BackendDiscoveryBridge(r),
                AnyState::BackendDiscoveryBridge(desired),
                AnyState::BackendDiscoveryBridge(actual),
                AnyReconcilerView::BackendDiscoveryBridge(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::BackendDiscoveryBridge(next_view))
            }
            (
                Self::ServiceLifecycle(r),
                AnyState::ServiceLifecycle(desired),
                AnyState::ServiceLifecycle(actual),
                AnyReconcilerView::ServiceLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::ServiceLifecycle(next_view))
            }
            (
                Self::SvidLifecycle(r),
                AnyState::SvidLifecycle(desired),
                AnyState::SvidLifecycle(actual),
                AnyReconcilerView::SvidLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::SvidLifecycle(next_view))
            }
            (
                Self::VmReclamation(r),
                AnyState::VmReclamation(desired),
                AnyState::VmReclamation(actual),
                AnyReconcilerView::VmReclamation(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::VmReclamation(next_view))
            }
            _ => {
                panic!(
                    "AnyReconciler::reconcile dispatch mismatch — \
                    runtime supplied incompatible (reconciler, state, view) triple"
                )
            }
        }
    }

    /// Impure hydrate-desired dispatch (ADR-0086 D1). Forwards to the inner
    /// reconciler's [`Reconciler::hydrate_desired`] and wraps the concrete
    /// `Self::State` into the MATCHING [`AnyState`] variant at the boundary —
    /// one arm per variant, mirroring [`AnyReconciler::reconcile`]'s wrapping of
    /// `Self::View` into [`AnyReconcilerView`]. Reads every fact through the
    /// injected [`HydrationContext`] read-ports; the pure `reconcile` is
    /// untouched. Errors surface as the core [`HydrateError`]; the runtime maps
    /// them to `ConvergenceError` via `#[from]` at the call site.
    pub async fn hydrate_desired(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<AnyState, HydrateError> {
        match self {
            Self::NoopHeartbeat(r) => {
                r.hydrate_desired(ctx, target).await?;
                Ok(AnyState::Unit)
            }
            Self::WorkloadLifecycle(r) => {
                Ok(AnyState::WorkloadLifecycle(r.hydrate_desired(ctx, target).await?))
            }
            Self::WorkflowLifecycle(r) => {
                Ok(AnyState::WorkflowLifecycle(r.hydrate_desired(ctx, target).await?))
            }
            Self::ServiceMapHydrator(r) => {
                Ok(AnyState::ServiceMapHydrator(r.hydrate_desired(ctx, target).await?))
            }
            Self::BackendDiscoveryBridge(r) => {
                Ok(AnyState::BackendDiscoveryBridge(r.hydrate_desired(ctx, target).await?))
            }
            Self::ServiceLifecycle(r) => {
                Ok(AnyState::ServiceLifecycle(r.hydrate_desired(ctx, target).await?))
            }
            Self::SvidLifecycle(r) => {
                Ok(AnyState::SvidLifecycle(r.hydrate_desired(ctx, target).await?))
            }
            Self::VmReclamation(r) => {
                Ok(AnyState::VmReclamation(r.hydrate_desired(ctx, target).await?))
            }
        }
    }

    /// Impure hydrate-actual dispatch (ADR-0086 D1) — the mirror of
    /// [`AnyReconciler::hydrate_desired`]. Forwards to
    /// [`Reconciler::hydrate_actual`] and wraps into the matching [`AnyState`]
    /// variant.
    pub async fn hydrate_actual(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<AnyState, HydrateError> {
        match self {
            Self::NoopHeartbeat(r) => {
                r.hydrate_actual(ctx, target).await?;
                Ok(AnyState::Unit)
            }
            Self::WorkloadLifecycle(r) => {
                Ok(AnyState::WorkloadLifecycle(r.hydrate_actual(ctx, target).await?))
            }
            Self::WorkflowLifecycle(r) => {
                Ok(AnyState::WorkflowLifecycle(r.hydrate_actual(ctx, target).await?))
            }
            Self::ServiceMapHydrator(r) => {
                Ok(AnyState::ServiceMapHydrator(r.hydrate_actual(ctx, target).await?))
            }
            Self::BackendDiscoveryBridge(r) => {
                Ok(AnyState::BackendDiscoveryBridge(r.hydrate_actual(ctx, target).await?))
            }
            Self::ServiceLifecycle(r) => {
                Ok(AnyState::ServiceLifecycle(r.hydrate_actual(ctx, target).await?))
            }
            Self::SvidLifecycle(r) => {
                Ok(AnyState::SvidLifecycle(r.hydrate_actual(ctx, target).await?))
            }
            Self::VmReclamation(r) => {
                Ok(AnyState::VmReclamation(r.hydrate_actual(ctx, target).await?))
            }
        }
    }
}

/// Extract a [`WorkloadId`] from a `TargetResource` of shape `workload/<id>`.
///
/// The shared target parser for the workload-keyed hydrate arms
/// (`WorkloadLifecycle`, `SvidLifecycle`, `BackendDiscoveryBridge`,
/// `ServiceLifecycle`). Moved off the pre-move central
/// `reconciler_runtime::workload_id_from_target` (ADR-0086 S3); returns the core
/// [`HydrateError::TargetShape`] instead of the control-plane `ConvergenceError`.
pub(crate) fn workload_id_from_target(target: &TargetResource) -> Result<WorkloadId, HydrateError> {
    let raw = target.as_str();
    let id_part =
        raw.strip_prefix("workload/").ok_or_else(|| HydrateError::TargetShape(raw.to_string()))?;
    WorkloadId::new(id_part).map_err(|e| HydrateError::TargetShape(e.to_string()))
}

/// Extract a [`ServiceId`] from a `TargetResource` of shape `service/<id>`.
///
/// Mirrors [`workload_id_from_target`] for the `ServiceMapHydrator` arm. Moved
/// off the pre-move central `reconciler_runtime::service_id_from_target`.
pub(crate) fn service_id_from_target(target: &TargetResource) -> Result<ServiceId, HydrateError> {
    use std::str::FromStr;
    let raw = target.as_str();
    let id_part =
        raw.strip_prefix("service/").ok_or_else(|| HydrateError::TargetShape(raw.to_string()))?;
    ServiceId::from_str(id_part).map_err(|e| HydrateError::TargetShape(e.to_string()))
}

/// Sum of every per-reconciler `View` shape held by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyReconcilerView {
    /// The `View = ()` variant used by Phase 1 reconcilers
    /// (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's view.
    WorkloadLifecycle(WorkloadLifecycleView),
    /// `WorkflowLifecycle` reconciler's view (Phase 1: empty — the
    /// re-emit decision is pure over `actual`). ADR-0064 §5.
    WorkflowLifecycle(WorkflowLifecycleView),
    /// `ServiceMapHydrator` reconciler's view.
    ServiceMapHydrator(ServiceMapHydratorView),
    /// `BackendDiscoveryBridge` reconciler's view.
    BackendDiscoveryBridge(BackendDiscoveryBridgeView),
    /// `ServiceLifecycle` reconciler's view per ADR-0055 § 3 / DDD-5.
    /// Carries inputs only (counters / once-only Stable-announcement
    /// set) — derived state (`Stable` predicate, deadlines) is
    /// recomputed every tick.
    ServiceLifecycle(ServiceLifecycleView),
    /// `SvidLifecycle` reconciler's view (Slice 01: empty — the issue/drop
    /// decision is pure over `desired`/`actual`; retry memory lands in
    /// 03-01). ADR-0067 D8.
    SvidLifecycle(SvidLifecycleView),
    /// `VmReclamation` reconciler's view — FIELD-LESS per the ADR-0079
    /// precedent (`brief.md` §105a.1): nothing this reconciler emitted is
    /// ever consulted, so retry falls out of the runtime's `has_work`
    /// self-re-enqueue.
    VmReclamation(vm_reclamation::VmReclamationView),
}
