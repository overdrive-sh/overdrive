//! Step 02-02 / Slice 3A.2 scenario 3.2 — `ReconcilerIsPure` invariant
//! holds for the `JobLifecycle` reconciler.
//!
//! The §18 purity contract requires `reconcile(desired, actual, view,
//! tick)` to produce bit-identical `(Vec<Action>, NextView)` tuples on
//! twin invocation with identical inputs. ADR-0017's
//! `ReconcilerIsPure` invariant is the runtime witness.
//!
//! This scenario exercises the contract specifically against
//! `JobLifecycle` — the first reconciler with a non-trivial `State`
//! and `View`. Twin invocation is performed directly (the harness's
//! `evaluate_reconciler_is_pure` evaluator currently constructs an
//! `AnyState::Unit` for every reconciler; we test `JobLifecycle`
//! against its own typed state directly here).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, JobSpecInput, Node, NodeSpecInput};
use overdrive_core::reconciler::{
    AnyReconciler, AnyReconcilerView, AnyState, JobLifecycle, JobLifecycleState, JobLifecycleView,
    TickContext,
};

fn fresh_tick() -> TickContext {
    let now = Instant::now();
    TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) }
}

fn payments_job() -> Job {
    Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        cpu_milli: 500,
        memory_bytes: 256 * 1024 * 1024,
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

fn happy_path_state() -> JobLifecycleState {
    let mut nodes = BTreeMap::new();
    let n = node_alpha();
    nodes.insert(n.id.clone(), n);
    JobLifecycleState { job: Some(payments_job()), nodes, allocations: BTreeMap::new() }
}

const fn empty_view() -> JobLifecycleView {
    JobLifecycleView { restart_counts: BTreeMap::new(), next_attempt_at: BTreeMap::new() }
}

#[test]
fn job_lifecycle_satisfies_reconciler_is_pure_invariant() {
    // Construct AnyReconciler::JobLifecycle and twin-invoke through
    // the AnyReconciler dispatch layer.
    let reconciler = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
    let desired_inner = happy_path_state();
    let actual_inner = JobLifecycleState {
        job: desired_inner.job.clone(),
        nodes: desired_inner.nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let desired = AnyState::JobLifecycle(desired_inner);
    let actual = AnyState::JobLifecycle(actual_inner);
    let view = AnyReconcilerView::JobLifecycle(empty_view());
    let tick = fresh_tick();

    // Twin invocation per ADR-0013 §2 / §2c — single TickContext shared
    // across both calls.
    let (actions_a, view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions_a, actions_b,
        "ReconcilerIsPure: JobLifecycle twin invocations must produce bit-identical actions; \
         got first={actions_a:?}, second={actions_b:?}"
    );
    assert_eq!(
        view_a, view_b,
        "ReconcilerIsPure: JobLifecycle twin invocations must produce bit-identical NextView; \
         got first={view_a:?}, second={view_b:?}"
    );
}

#[test]
fn job_lifecycle_run_emits_start_allocation_when_no_running_alloc() {
    // Sanity check: when desired says "Run" (job present) and actual
    // shows no Running alloc, JobLifecycle must emit a StartAllocation.
    // This is the happy-path Slice 3 acceptance: the first reconciler
    // exercises the scheduler and emits a Start.
    use overdrive_core::reconciler::Action;

    let reconciler = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
    let desired_inner = happy_path_state();
    let actual_inner = JobLifecycleState {
        job: desired_inner.job.clone(),
        nodes: desired_inner.nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let desired = AnyState::JobLifecycle(desired_inner);
    let actual = AnyState::JobLifecycle(actual_inner);
    let view = AnyReconcilerView::JobLifecycle(empty_view());
    let tick = fresh_tick();

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "exactly one StartAllocation expected; got {actions:?}");
    let action = actions.first().expect("one action present");
    match action {
        Action::StartAllocation { job_id, node_id, .. } => {
            assert_eq!(job_id.to_string(), "payments");
            assert_eq!(node_id.to_string(), "node-alpha");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}
