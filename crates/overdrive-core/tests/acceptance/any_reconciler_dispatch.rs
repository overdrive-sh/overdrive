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
//!   - `delete match arm (Self::JobLifecycle(r), … JobLifecycle …)`
//!     — same shape: production dispatches to
//!     `JobLifecycle::reconcile` and returns
//!     `AnyReconcilerView::JobLifecycle(_)`. Under deletion the
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

use overdrive_core::aggregate::Node;
use overdrive_core::id::{JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, JobLifecycle, JobLifecycleState,
    NoopHeartbeat, TickContext,
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
    let tick = TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) };

    let (actions, view) =
        any.reconcile(&AnyState::Unit, &AnyState::Unit, &AnyReconcilerView::Unit, &tick);

    assert_eq!(actions, vec![Action::Noop], "NoopHeartbeat must emit exactly one Noop");
    assert!(
        matches!(view, AnyReconcilerView::Unit),
        "NoopHeartbeat dispatch must return AnyReconcilerView::Unit; got {view:?}",
    );
}

// -------------------------------------------------------------------
// L975 — JobLifecycle dispatch arm
// -------------------------------------------------------------------

#[test]
fn dispatch_routes_job_lifecycle_triple_to_job_lifecycle_view() {
    // Construct the canonical `JobLifecycle`, wrap it in
    // `AnyReconciler::JobLifecycle`, and dispatch with the
    // JobLifecycle triple. Production: routes to
    // `JobLifecycle::reconcile` → returns `AnyReconcilerView::
    // JobLifecycle(_)`. Mutant (delete arm): wildcard panic.
    let any = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
    let now = Instant::now();
    let tick = TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) };

    // Empty desired/actual JobLifecycle states — no allocs, no nodes,
    // no job. The reconciler emits no actions, returns its default
    // view. We don't care about the action shape for this test —
    // only the *variant* of the returned view, which is uniquely
    // produced by the JobLifecycle dispatch arm.
    let mut nodes = BTreeMap::new();
    let local = Node {
        id: NodeId::new("local").expect("valid NodeId"),
        region: Region::new("local").expect("valid Region"),
        capacity: Resources { cpu_milli: 1_000, memory_bytes: 1024 * 1024 * 1024 },
    };
    nodes.insert(local.id.clone(), local);
    let _ = JobId::new("payments").expect("valid JobId");

    let desired = JobLifecycleState {
        job: None,
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: None,
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
    };
    let view =
        AnyReconcilerView::JobLifecycle(overdrive_core::reconciler::JobLifecycleView::default());

    let (_actions, returned_view) = any.reconcile(
        &AnyState::JobLifecycle(desired),
        &AnyState::JobLifecycle(actual),
        &view,
        &tick,
    );

    assert!(
        matches!(returned_view, AnyReconcilerView::JobLifecycle(_)),
        "JobLifecycle dispatch must return AnyReconcilerView::JobLifecycle; got {returned_view:?}",
    );
}
