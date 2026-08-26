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

pub mod reconcilers;
pub mod service_lifecycle;

// Flat re-exports so importers spell `overdrive_reconcilers::<Symbol>` for the
// moved impls / enums / State / View / helpers (mirrors the ergonomic top-level
// access the impls had inside `overdrive_core::reconcilers` before the move).
pub use reconcilers::{
    AnyReconciler, AnyReconcilerView, AnyState, BackendAddressRejection, BackendDiscoveryBridge,
    BackendDiscoveryBridgeState, BackendDiscoveryBridgeView, NoopHeartbeat, RESTART_BACKOFF_CEILING,
    RESTART_BACKOFF_DURATION, RetryMemory, RunningAlloc, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, SupervisionSet, SvidLifecycle,
    SvidLifecycleState, SvidLifecycleView, VmAllocFacts, VmReclamation, VmReclamationState,
    VmReclamationView, WorkflowInstanceState, WorkflowLifecycle, WorkflowLifecycleState,
    WorkflowLifecycleView, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
    backoff_for_attempt, classify_backend_address, plan_reclamation, project_probe_descriptors,
    project_service_listen_ports,
};
