//! Step 02-02 / Slice 3A.2 scenario 3.2 — `ReconcilerIsPure` invariant
//! holds for the `WorkloadLifecycle` reconciler.
//!
//! The §18 purity contract requires `reconcile(desired, actual, view,
//! tick)` to produce bit-identical `(Vec<Action>, NextView)` tuples on
//! twin invocation with identical inputs. ADR-0017's
//! `ReconcilerIsPure` invariant is the runtime witness.
//!
//! This scenario exercises the contract specifically against
//! `WorkloadLifecycle` — the first reconciler with a non-trivial `State`
//! and `View`. Twin invocation is performed directly (the harness's
//! `evaluate_reconciler_is_pure` evaluator currently constructs an
//! `AnyState::Unit` for every reconciler; we test `WorkloadLifecycle`
//! against its own typed state directly here).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, Job, JobSpecInput, Node, NodeSpecInput, ResourcesInput, WorkloadKind,
};
use overdrive_core::reconciler::{
    AnyReconciler, AnyReconcilerView, AnyState, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState, WorkloadLifecycleView,
};

/// Canonical `fresh_tick` signature (uniform across every acceptance
/// suite per step 03-01): callers pass both `now` (monotonic) and
/// `now_unix` (wall-clock) explicitly. Tests that do not exercise the
/// wall-clock domain pass
/// `UnixInstant::from_unix_duration(Duration::from_secs(0))`.
fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

fn payments_job() -> Job {
    Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("valid Job spec")
}

fn node_alpha() -> Node {
    Node::new(NodeSpecInput {
        id: "node-alpha".to_string(),
        region: "local".to_string(),
        cpu_milli: 4000,
        memory_bytes: 8 * 1024 * 1024 * 1024,
    })
    .expect("valid Node spec")
}

fn happy_path_state() -> WorkloadLifecycleState {
    let mut nodes = BTreeMap::new();
    let n = node_alpha();
    nodes.insert(n.id.clone(), n);
    WorkloadLifecycleState {
        job: Some(payments_job()),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
    }
}

const fn empty_view() -> WorkloadLifecycleView {
    WorkloadLifecycleView { restart_counts: BTreeMap::new(), last_failure_seen_at: BTreeMap::new() }
}

#[test]
fn workload_lifecycle_satisfies_reconciler_is_pure_invariant() {
    // Construct AnyReconciler::WorkloadLifecycle and twin-invoke through
    // the AnyReconciler dispatch layer.
    let reconciler = AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical());
    let desired_inner = happy_path_state();
    let actual_inner = WorkloadLifecycleState {
        job: desired_inner.job.clone(),
        desired_to_stop: false,
        nodes: desired_inner.nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
    };
    let desired = AnyState::WorkloadLifecycle(desired_inner);
    let actual = AnyState::WorkloadLifecycle(actual_inner);
    let view = AnyReconcilerView::WorkloadLifecycle(empty_view());
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    // Twin invocation per ADR-0013 §2 / §2c — single TickContext shared
    // across both calls.
    let (actions_a, view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions_a, actions_b,
        "ReconcilerIsPure: WorkloadLifecycle twin invocations must produce bit-identical actions; \
         got first={actions_a:?}, second={actions_b:?}"
    );
    assert_eq!(
        view_a, view_b,
        "ReconcilerIsPure: WorkloadLifecycle twin invocations must produce bit-identical NextView; \
         got first={view_a:?}, second={view_b:?}"
    );
}

#[test]
fn workload_lifecycle_run_emits_start_allocation_when_no_running_alloc() {
    // Sanity check: when desired says "Run" (job present) and actual
    // shows no Running alloc, WorkloadLifecycle must emit a StartAllocation.
    // This is the happy-path Slice 3 acceptance: the first reconciler
    // exercises the scheduler and emits a Start.
    use overdrive_core::reconciler::Action;

    let reconciler = AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical());
    let desired_inner = happy_path_state();
    let actual_inner = WorkloadLifecycleState {
        job: desired_inner.job.clone(),
        desired_to_stop: false,
        nodes: desired_inner.nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
    };
    let desired = AnyState::WorkloadLifecycle(desired_inner);
    let actual = AnyState::WorkloadLifecycle(actual_inner);
    let view = AnyReconcilerView::WorkloadLifecycle(empty_view());
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "exactly one StartAllocation expected; got {actions:?}");
    let action = actions.first().expect("one action present");
    match action {
        Action::StartAllocation { workload_id, node_id, .. } => {
            assert_eq!(workload_id.to_string(), "payments");
            assert_eq!(node_id.to_string(), "node-alpha");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}
