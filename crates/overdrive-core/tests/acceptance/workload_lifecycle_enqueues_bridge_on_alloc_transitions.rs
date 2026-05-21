//! UI-06 acceptance — `WorkloadLifecycle::reconcile` dual-emits
//! `Action::EnqueueEvaluation { reconciler: "backend-discovery-bridge",
//! target: "job/<workload_id>" }` alongside every alloc-mutating
//! action (`StartAllocation` / `RestartAllocation` / `StopAllocation`
//! / `FinalizeFailed`) so the `BackendDiscoveryBridge` re-ticks
//! against the new alloc state on the next convergence cycle.
//!
//! Closes the F1 gap surfaced by
//! `docs/feature/backend-discovery-bridge-service-reachability/deliver/audit-reconciler-handoff-topology.md`.
//!
//! Pre-UI-06 the only enqueue site for the bridge was the exit
//! observer (`exit_observer.rs:253-256`) which fires only on workload
//! exit. For long-lived Service workloads the bridge never ticked
//! after Pending → Running, no `ServiceBackendRow` was written, and
//! the entire downstream hydrator → dataplane chain was structurally
//! unreachable. This test pins the dual-emission at the reconciler's
//! surface — the load-bearing structural property the F1 fix
//! introduces.
//!
//! Mirrors the UI-05 pattern test (`bridge_emits_enqueue_evaluation_
//! for_hydrator.rs`): port-to-port at the domain function (the pure
//! `Reconciler::reconcile` is its own driving port — calling it
//! directly IS port-to-port testing per the project's testing
//! discipline).

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconciler::{
    Action, RESTART_BACKOFF_CEILING, Reconciler, TargetResource, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::TerminalCondition;

// -------------------------------------------------------------------
// fixtures (mirror workload_lifecycle_reconcile_branches.rs shape)
// -------------------------------------------------------------------

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn jid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn local_region() -> Region {
    Region::new("local").expect("valid Region")
}
fn make_node(id: &str) -> Node {
    Node {
        id: nid(id),
        region: local_region(),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    }
}
fn make_job(id: &str) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}
fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = make_node(node_id);
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}
fn alloc_with_state(
    alloc_id: &str,
    workload_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(node_id) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
    }
}
fn fresh_tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// Helper: assert that `actions` contains exactly one
/// `Action::EnqueueEvaluation` routed at the bridge for the given
/// workload, and return a reference to its inner fields.
fn assert_single_bridge_enqueue<'a>(
    actions: &'a [Action],
    workload_id: &WorkloadId,
) -> (&'a overdrive_core::reconciler::ReconcilerName, &'a TargetResource) {
    let mut found: Option<(&overdrive_core::reconciler::ReconcilerName, &TargetResource)> = None;
    let mut count = 0;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action {
            if reconciler.as_str() == "backend-discovery-bridge" {
                count += 1;
                found = Some((reconciler, target));
            }
        }
    }
    assert_eq!(
        count, 1,
        "UI-06: WorkloadLifecycle MUST emit exactly one EnqueueEvaluation routed at \
         'backend-discovery-bridge' per tick that mutates the alloc set; got {count} in {actions:?}",
    );
    let (reconciler, target) = found.expect("count==1 checked above");
    assert_eq!(
        target.as_str(),
        &format!("job/{workload_id}"),
        "UI-06: bridge enqueue target MUST be 'job/<workload_id>' (mirrors exit_observer)"
    );
    (reconciler, target)
}

// -------------------------------------------------------------------
// StartAllocation branch — fresh Service workload, no allocs yet.
// -------------------------------------------------------------------

#[test]
fn start_allocation_branch_dual_emits_bridge_enqueue() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Exactly two actions: one StartAllocation + one EnqueueEvaluation.
    assert_eq!(actions.len(), 2, "expected StartAllocation + EnqueueEvaluation; got {actions:?}");
    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "first action must be StartAllocation; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
}

// -------------------------------------------------------------------
// StopAllocation branch (operator stop) — desired_to_stop AND job set,
// a Running alloc present.
// -------------------------------------------------------------------

#[test]
fn stop_allocation_branch_dual_emits_bridge_enqueue() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 2, "expected StopAllocation + EnqueueEvaluation; got {actions:?}");
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation among actions; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
}

// -------------------------------------------------------------------
// GC branch — job=None, Running orphan alloc — emits SystemGc-stamped
// StopAllocation. Still dual-emits the bridge enqueue.
// -------------------------------------------------------------------

#[test]
fn gc_stop_branch_dual_emits_bridge_enqueue() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: None,
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: None,
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 2, "expected StopAllocation + EnqueueEvaluation; got {actions:?}");
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
}

// -------------------------------------------------------------------
// FinalizeFailed branch (backoff exhausted) — Failed alloc whose
// restart_counts has hit the ceiling.
// -------------------------------------------------------------------

#[test]
fn finalize_failed_branch_dual_emits_bridge_enqueue() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    let failed_row = AllocStatusRow {
        state: AllocState::Failed,
        ..alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Failed)
    };
    allocations.insert(aid("alloc-payments-0"), failed_row);

    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };

    // View at the ceiling: restart_counts hits RESTART_BACKOFF_CEILING,
    // so the reconciler emits FinalizeFailed (not RestartAllocation).
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(aid("alloc-payments-0"), RESTART_BACKOFF_CEILING);

    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 2, "expected FinalizeFailed + EnqueueEvaluation; got {actions:?}");
    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                ..
            }
        )),
        "expected FinalizeFailed with BackoffExhausted terminal; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
}

// -------------------------------------------------------------------
// Converged tick — Running alloc already present, reconciler emits
// NOTHING. The bridge enqueue MUST NOT fire on a noop tick (broker
// would churn empty re-enqueues).
// -------------------------------------------------------------------

#[test]
fn converged_tick_emits_no_bridge_enqueue() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Bridge enqueue paired ONLY with alloc-mutating actions; converged
    // tick produces zero actions including zero enqueue.
    assert!(
        actions.is_empty(),
        "converged tick must emit zero actions (no spurious bridge enqueue); got {actions:?}"
    );
}
