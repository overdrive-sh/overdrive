//! Branch-coverage tests for `AnyReconciler::reconcile` dispatch.
//!
//! Mutations covered:
//!
//!   - `delete match arm (Self::NoopHeartbeat(r), AnyState::Unit,
//!     AnyState::Unit, AnyReconcilerView::Unit)` — under
//!     production, this arm dispatches to `NoopHeartbeat::reconcile`
//!     and returns `(vec![Action::Noop], AnyReconcilerView::Unit)`.
//!     Under the `delete` mutation, the arm vanishes and the
//!     dispatch falls through to the wildcard `_ => panic!`. The
//!     test catches the panic-via-non-Noop-result asymmetry: a Unit
//!     dispatch that does NOT panic and returns the expected tuple
//!     can only have come from the production arm.
//!
//!   - `delete match arm (Self::WorkloadLifecycle(r), … WorkloadLifecycle …)`
//!     — same shape: production dispatches to
//!     `WorkloadLifecycle::reconcile` and returns
//!     `AnyReconcilerView::WorkloadLifecycle(_)`. Under deletion the
//!     wildcard panics. The test asserts the variant of the
//!     returned `AnyReconcilerView`, which can only come from the
//!     production arm.
//!
//! Both tests use `std::panic::catch_unwind` to detect the panic
//! shape — a panic during dispatch fails the assertion, the
//! production arm returns cleanly with the expected variant.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Node, WorkloadKind};
use overdrive_core::id::{NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
};
use overdrive_core::reconcilers::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, NoopHeartbeat, Reconciler,
    ServiceMapHydrator, ServiceMapHydratorState, ServiceMapHydratorView, TickContext,
    WorkloadLifecycle, WorkloadLifecycleState,
};
use overdrive_core::traits::driver::Resources;

// -------------------------------------------------------------------
// L956 — NoopHeartbeat dispatch arm
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_noop_heartbeat_unit_triple_to_noop_action() {
    // Construct the canonical `NoopHeartbeat`, wrap it in
    // `AnyReconciler::NoopHeartbeat`, and dispatch with the Unit
    // triple. Production: returns `(vec![Action::Noop],
    // AnyReconcilerView::Unit)`. Mutant (delete arm): falls into
    // wildcard `panic!`.
    let any = AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical());
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let (actions, view) =
        any.reconcile(&AnyState::Unit, &AnyState::Unit, &AnyReconcilerView::Unit, &tick);

    assert_eq!(actions, vec![Action::Noop], "NoopHeartbeat must emit exactly one Noop");
    assert!(
        matches!(view, AnyReconcilerView::Unit),
        "NoopHeartbeat dispatch must return AnyReconcilerView::Unit; got {view:?}",
    );
}

// -------------------------------------------------------------------
// L975 — WorkloadLifecycle dispatch arm
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_job_lifecycle_triple_to_job_lifecycle_view() {
    // Construct the canonical `WorkloadLifecycle`, wrap it in
    // `AnyReconciler::WorkloadLifecycle`, and dispatch with the
    // WorkloadLifecycle triple. Production: routes to
    // `WorkloadLifecycle::reconcile` → returns `AnyReconcilerView::
    // WorkloadLifecycle(_)`. Mutant (delete arm): wildcard panic.
    let any = AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical());
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // Empty desired/actual WorkloadLifecycle states — no allocs, no nodes,
    // no job. The reconciler emits no actions, returns its default
    // view. We don't care about the action shape for this test —
    // only the *variant* of the returned view, which is uniquely
    // produced by the WorkloadLifecycle dispatch arm.
    let mut nodes = BTreeMap::new();
    let local = Node {
        id: NodeId::new("local").expect("valid NodeId"),
        region: Region::new("local").expect("valid Region"),
        capacity: Resources { cpu_milli: 1_000, memory_bytes: 1024 * 1024 * 1024 },
    };
    nodes.insert(local.id.clone(), local);

    let desired = WorkloadLifecycleState {
        workload_id: WorkloadId::new("test").expect("valid WorkloadId"),
        job: None,
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: WorkloadId::new("test").expect("valid WorkloadId"),
        job: None,
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = AnyReconcilerView::WorkloadLifecycle(
        overdrive_core::reconcilers::WorkloadLifecycleView::default(),
    );

    let (_actions, returned_view) = any.reconcile(
        &AnyState::WorkloadLifecycle(desired),
        &AnyState::WorkloadLifecycle(actual),
        &view,
        &tick,
    );

    assert!(
        matches!(returned_view, AnyReconcilerView::WorkloadLifecycle(_)),
        "WorkloadLifecycle dispatch must return AnyReconcilerView::WorkloadLifecycle; got {returned_view:?}",
    );
}

// -------------------------------------------------------------------
// L965 — ServiceMapHydrator dispatch arm (Phase 2 / ASR-2.2-04)
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_service_map_hydrator_triple_to_hydrator_view() {
    // Construct the canonical `ServiceMapHydrator`, wrap it in
    // `AnyReconciler::ServiceMapHydrator`, and dispatch with the
    // ServiceMapHydrator triple. Production: routes to
    // `ServiceMapHydrator::reconcile` → returns
    // `AnyReconcilerView::ServiceMapHydrator(_)`. Mutant (delete arm):
    // wildcard panic. Closes the missed mutation flagged at line
    // 965 by the QUALITY_GATE wave's mutation run.
    let any = AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        std::net::Ipv4Addr::UNSPECIFIED,
    ));
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // Empty desired/actual hydrator states — no services. The
    // reconciler returns no actions; only the variant of the
    // returned view discriminates the dispatch arm. The mutation
    // (`delete match arm`) makes this fall through to the wildcard
    // `_ => panic!`, which `catch_unwind` would reveal.
    let desired = ServiceMapHydratorState::default();
    let actual = ServiceMapHydratorState::default();
    let view = AnyReconcilerView::ServiceMapHydrator(ServiceMapHydratorView::default());

    let (_actions, returned_view) = any.reconcile(
        &AnyState::ServiceMapHydrator(desired),
        &AnyState::ServiceMapHydrator(actual),
        &view,
        &tick,
    );

    assert!(
        matches!(returned_view, AnyReconcilerView::ServiceMapHydrator(_)),
        "ServiceMapHydrator dispatch must return AnyReconcilerView::ServiceMapHydrator; \
         got {returned_view:?}",
    );
    // Suppress unused-import warning for `Action` in this test —
    // the import remains used by the NoopHeartbeat dispatch test
    // above, retained here as a no-op assertion to make the
    // dependency obvious.
    let _ = Action::Noop;
}

// -------------------------------------------------------------------
// L1260 — BackendDiscoveryBridge dispatch arm
// (backend-discovery-bridge-service-reachability step 01-02)
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_backend_discovery_bridge_triple_to_bridge_view() {
    // Construct a `BackendDiscoveryBridge`, wrap it in
    // `AnyReconciler::BackendDiscoveryBridge`, and dispatch with the
    // matching state + view triple. Production: routes to
    // `BackendDiscoveryBridge::reconcile` → returns
    // `AnyReconcilerView::BackendDiscoveryBridge(_)`. Mutant (delete
    // match arm at reconciler.rs:1260): wildcard `_ => panic!` fires
    // instead, so the dispatch panics. The variant-of-returned-view
    // assertion below is uniquely produced by the BackendDiscoveryBridge
    // arm — any other arm (or the panic wildcard) would not return
    // `AnyReconcilerView::BackendDiscoveryBridge(_)`.
    //
    // Phase 17 mutation gap: until this test, the `delete match arm`
    // mutation for the BackendDiscoveryBridge dispatch arm was MISSED
    // by the default-lane suite — every existing dispatch test covers
    // one of the other three arms (NoopHeartbeat, WorkloadLifecycle,
    // ServiceMapHydrator) only.
    let writer = NodeId::new("writer-1").expect("valid NodeId");
    let any = AnyReconciler::BackendDiscoveryBridge(BackendDiscoveryBridge::new(
        Ipv4Addr::new(10, 0, 0, 5),
        writer,
    ));
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // Empty bridge state — no listeners, no Running allocs. The
    // bridge's reconcile body returns an empty action list and the
    // unchanged view; the variant of the returned view is the
    // discriminating signal for this test.
    let workload_id = WorkloadId::new("payments").expect("valid WorkloadId");
    let desired = BackendDiscoveryBridgeState::empty_for_workload(workload_id.clone());
    let actual = BackendDiscoveryBridgeState::empty_for_workload(workload_id);
    let view = AnyReconcilerView::BackendDiscoveryBridge(BackendDiscoveryBridgeView::default());

    let (_actions, returned_view) = any.reconcile(
        &AnyState::BackendDiscoveryBridge(desired),
        &AnyState::BackendDiscoveryBridge(actual),
        &view,
        &tick,
    );

    assert!(
        matches!(returned_view, AnyReconcilerView::BackendDiscoveryBridge(_)),
        "BackendDiscoveryBridge dispatch must return \
         AnyReconcilerView::BackendDiscoveryBridge; got {returned_view:?}",
    );
}

// -------------------------------------------------------------------
// L727 — ServiceLifecycle dispatch arm
// (service-health-check-probes step 01-03b mutation tightening)
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_service_lifecycle_triple_to_service_lifecycle_view() {
    // Construct a `ServiceLifecycleReconciler`, wrap it in
    // `AnyReconciler::ServiceLifecycle`, and dispatch with the matching
    // state + view triple. Production: routes to
    // `ServiceLifecycleReconciler::reconcile` → returns
    // `AnyReconcilerView::ServiceLifecycle(_)`. Mutant
    // (`delete match arm` at reconciler.rs:727): wildcard `_ => panic!`
    // fires; the variant-of-returned-view assertion below is uniquely
    // produced by the ServiceLifecycle dispatch arm.
    //
    // Per service-health-check-probes step 01-03b mutation report:
    // until this test, the ServiceLifecycle arm was the one dispatch
    // arm whose `delete match arm` mutant was MISSED in the
    // `--diff origin/main` scope.
    use overdrive_core::service_lifecycle::{
        ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
    };

    let any = AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new());
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // Empty service-lifecycle state — no allocs. The reconciler returns
    // no actions; only the variant of the returned view discriminates
    // the dispatch arm. We additionally assert oracle equivalence: the
    // (actions, view) tuple from `AnyReconciler::reconcile` must match
    // the direct `ServiceLifecycleReconciler::reconcile` call on the
    // same inputs.
    let desired = ServiceLifecycleState::default();
    let actual = ServiceLifecycleState::default();
    let view_inner = ServiceLifecycleView::default();
    let view = AnyReconcilerView::ServiceLifecycle(view_inner.clone());

    let (actions_via_any, returned_view) = any.reconcile(
        &AnyState::ServiceLifecycle(desired.clone()),
        &AnyState::ServiceLifecycle(actual.clone()),
        &view,
        &tick,
    );

    assert!(
        matches!(returned_view, AnyReconcilerView::ServiceLifecycle(_)),
        "ServiceLifecycle dispatch must return AnyReconcilerView::ServiceLifecycle; \
         got {returned_view:?}",
    );

    // Oracle: direct reconcile call. Same inputs => same output.
    let r = ServiceLifecycleReconciler::new();
    let (actions_direct, view_direct) = r.reconcile(&desired, &actual, &view_inner, &tick);
    assert_eq!(
        actions_via_any, actions_direct,
        "AnyReconciler::reconcile must emit the same actions as the direct call"
    );
    let AnyReconcilerView::ServiceLifecycle(unwrapped) = returned_view else {
        panic!("must be ServiceLifecycle variant")
    };
    assert_eq!(
        unwrapped, view_direct,
        "AnyReconciler::reconcile must return the same view as the direct call"
    );
}
