//! Green-build + import-rewrite gate for step 02-02 (S2 of ADR-0086).
//!
//! This is a **compile/presence** gate, not a runtime-behaviour test. The
//! whole point of S2 is a pure mechanical crate-extraction with NO runtime
//! behaviour change: the 8 reconciler impls, the 3 dispatch enums, and the
//! `service_lifecycle` module move OUT of `overdrive-core` INTO
//! `overdrive-reconcilers`, and every importer across the workspace is
//! rewired. The behaviour-preservation proof is the FULL workspace test run
//! passing unchanged; this file only pins the extraction's structural
//! post-conditions so a regression (a symbol left in core, a dispatch enum
//! not re-exported) fails a fast compile.
//!
//! Post-conditions asserted (all COMPILE-TIME — the test body is trivial):
//!
//! 1. The 3 dispatch enums resolve at `overdrive_reconcilers::` (moved IN).
//! 2. Representative reconciler impls + per-reconciler `State`/`View` types
//!    and pure helpers resolve at `overdrive_reconcilers::` (moved IN).
//! 3. `service_lifecycle` resolves at `overdrive_reconcilers::service_lifecycle`.
//! 4. The reconciler CONTRACT still resolves at `overdrive_core::reconcilers::`
//!    (stayed in core — contract-in-core; no core->reconcilers back-edge).

// (1) The three dispatch enums moved OUT of core INTO overdrive-reconcilers.
use overdrive_reconcilers::{AnyReconciler, AnyReconcilerView, AnyState};

// (2) A representative slice of the moved impls + State/View + pure helpers.
use overdrive_reconcilers::{
    BackendDiscoveryBridge, NoopHeartbeat, ServiceMapHydrator, SvidLifecycle, VmReclamation,
    WorkflowLifecycle, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
    backoff_for_attempt, classify_backend_address, plan_reclamation,
};

// (3) The `service_lifecycle` module moved wholesale.
use overdrive_reconcilers::service_lifecycle::{
    ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

// (4) The CONTRACT stayed in core — no core->reconcilers back-edge. These
//     paths MUST keep resolving at `overdrive_core::reconcilers::`.
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, ResyncSchedule,
    ResyncScope, TargetResource, TickContext, resolve_scope,
};

/// The extraction compiled and every moved/stayed symbol resolves at its
/// designed home. A trivial runtime assertion keeps nextest reporting a PASS;
/// the load-bearing signal is that this file COMPILES — an unresolved import
/// (a symbol left in the wrong crate) is a hard compile error above.
///
/// Each imported name is touched with a bare-`_` binding so a symbol that
/// silently stopped resolving cannot hide behind an `unused_imports` allow.
#[test]
fn moved_symbols_resolve_in_reconcilers_crate_contract_stays_in_core() {
    // Moved-IN types (enums + impls + State/View + service_lifecycle) resolve
    // at `overdrive_reconcilers::`.
    let _: Option<AnyReconciler> = None;
    let _: Option<AnyState> = None;
    let _: Option<AnyReconcilerView> = None;
    let _: Option<NoopHeartbeat> = None;
    let _: Option<WorkloadLifecycle> = None;
    let _: Option<WorkflowLifecycle> = None;
    let _: Option<ServiceMapHydrator> = None;
    let _: Option<BackendDiscoveryBridge> = None;
    let _: Option<SvidLifecycle> = None;
    let _: Option<VmReclamation> = None;
    let _: Option<WorkloadLifecycleState> = None;
    let _: Option<WorkloadLifecycleView> = None;
    let _: Option<ServiceLifecycleReconciler> = None;
    let _: Option<ServiceLifecycleState> = None;
    let _: Option<ServiceLifecycleView> = None;

    // Moved-IN pure helpers resolve at `overdrive_reconcilers::`.
    let _ = backoff_for_attempt;
    let _ = classify_backend_address;
    let _ = plan_reclamation;

    // CONTRACT stayed in core — resolves at `overdrive_core::reconcilers::`.
    let _: Option<Action> = None;
    let _: Option<TickContext> = None;
    let _: Option<ReconcilerName> = None;
    let _: Option<TargetResource> = None;
    let _: Option<ResyncSchedule> = None;
    let _: Option<ResyncScope> = None;
    let _: Option<HydrationContext<'static>> = None;
    let _: Option<HydrateError> = None;
    let _ = resolve_scope;

    // `Reconciler` is NOT dyn-compatible (it carries `const NAME` + associated
    // types), so we exercise the trait import through the associated const on a
    // concrete impl — enough to force name resolution of the core trait.
    let reconciler_contract_name: &str = <NoopHeartbeat as Reconciler>::NAME;
    assert_eq!(reconciler_contract_name, "noop-heartbeat");
}
