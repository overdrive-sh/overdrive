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
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Node, WorkloadKind};
use overdrive_core::id::{NodeId, Region, WorkloadId};
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, NoopHeartbeat, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState,
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
    };
    let actual = WorkloadLifecycleState {
        workload_id: WorkloadId::new("test").expect("valid WorkloadId"),
        job: None,
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let view = AnyReconcilerView::WorkloadLifecycle(
        overdrive_core::reconciler::WorkloadLifecycleView::default(),
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
    let any = AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical());
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
