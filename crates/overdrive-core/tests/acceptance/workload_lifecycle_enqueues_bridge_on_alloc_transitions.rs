//! Allocation mutation handoffs after ADR-0101 D5: Service workloads wake
//! ServiceLifecycle for Start, Stop, GC Stop and FinalizeFailed; SVID wakes
//! remain kind-independent. Job starts, converged ticks and VIP-only release
//! retain their negative ServiceLifecycle controls. BE11/BE12 prove the
//! corresponding dispatch path, including same-ID restart, compositionally.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, ContentHash, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TargetResource, TickContext};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};

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
        started_at: match state {
            AllocState::Pending => None,
            _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        },
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
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

/// GAP-9 helper — assert that `actions` contains exactly one
/// `Action::EnqueueEvaluation` routed at `service-lifecycle` for the
/// given workload, keyed `workload/<workload_id>`. Pins the Shape C dual-emit.
fn assert_single_service_enqueue(actions: &[Action], workload_id: &WorkloadId) {
    let mut count = 0;
    let mut found_target: Option<&TargetResource> = None;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action
            && reconciler.as_str() == "service-lifecycle"
        {
            count += 1;
            found_target = Some(target);
        }
    }
    assert_eq!(
        count, 1,
        "GAP-9: a Service-kind alloc-mutating tick MUST emit exactly one EnqueueEvaluation \
         routed at 'service-lifecycle'; got {count} in {actions:?}",
    );
    let target = found_target.expect("count==1 checked above");
    assert_eq!(
        target.as_str(),
        &format!("workload/{workload_id}"),
        "GAP-9: service-lifecycle enqueue target MUST be 'workload/<workload_id>'"
    );
}

/// ADR-0067 D5b helper — assert that `actions` contains exactly one
/// `Action::EnqueueEvaluation` routed at `svid-lifecycle` for the given
/// workload, keyed `workload/<workload_id>`. The svid-lifecycle enqueue is
/// UNGATED by workload kind (identity is needed by every running alloc).
fn assert_single_svid_enqueue(actions: &[Action], workload_id: &WorkloadId) {
    let mut count = 0;
    let mut found_target: Option<&TargetResource> = None;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action
            && reconciler.as_str() == "svid-lifecycle"
        {
            count += 1;
            found_target = Some(target);
        }
    }
    assert_eq!(
        count, 1,
        "ADR-0067 D5b: an alloc-mutating tick MUST emit exactly one EnqueueEvaluation \
         routed at 'svid-lifecycle'; got {count} in {actions:?}",
    );
    let target = found_target.expect("count==1 checked above");
    assert_eq!(
        target.as_str(),
        &format!("workload/{workload_id}"),
        "ADR-0067 D5b: svid-lifecycle enqueue target MUST be 'workload/<workload_id>'"
    );
}

/// GAP-9 helper — assert NO `service-lifecycle` enqueue appears.
fn assert_no_service_enqueue(actions: &[Action]) {
    let count = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::EnqueueEvaluation { reconciler, .. }
                    if reconciler.as_str() == "service-lifecycle"
            )
        })
        .count();
    assert_eq!(
        count, 0,
        "GAP-9: this tick must emit ZERO service-lifecycle enqueues; got {count} in {actions:?}",
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn start_allocation_branch_emits_service_and_svid_enqueues() {
    let workload_id = jid("payments");
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        3,
        "expected StartAllocation + service-lifecycle enqueue \
         + svid-lifecycle enqueue; got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "first action must be StartAllocation; got {actions:?}"
    );
    assert_single_service_enqueue(&actions, &workload_id);
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stop_allocation_branch_emits_service_and_svid_enqueues() {
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
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        3,
        "expected StopAllocation + service-lifecycle enqueue + svid-lifecycle enqueue \
         (Service removal wakes projection); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation among actions; got {actions:?}"
    );
    assert_single_service_enqueue(&actions, &workload_id);
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn gc_stop_branch_emits_service_and_svid_enqueues() {
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
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: None,
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        3,
        "expected StopAllocation + service-lifecycle enqueue + svid-lifecycle enqueue \
         (Service removal wakes projection); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation; got {actions:?}"
    );
    assert_single_service_enqueue(&actions, &workload_id);
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn finalize_failed_branch_emits_service_and_svid_enqueues() {
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
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };

    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(aid("alloc-payments-0"), RESTART_BACKOFF_CEILING);

    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        3,
        "expected FinalizeFailed + service-lifecycle enqueue + svid-lifecycle enqueue \
         (Service removal wakes projection); got {actions:?}"
    );
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
    assert_single_service_enqueue(&actions, &workload_id);
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn converged_tick_emits_no_lifecycle_enqueue() {
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
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: Some(make_job("payments")),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "converged tick must emit zero actions (no spurious lifecycle enqueue); got {actions:?}"
    );
}

fn fake_spec_digest() -> ContentHash {
    ContentHash::of(b"is-alloc-mutating-action-fixture-digest")
}

/// Construct an alloc row in the canonical "terminal-Operator-stopped
/// Service alloc" shape — `state: Terminated` with a
/// `terminal: Some(Stopped { by: Operator })` claim — used by the
/// `service_vip_release_emission` path. Mirrors the shape used by
/// `workload_lifecycle_release_service_vip.rs::alloc_terminal_operator_stopped`.
fn terminal_operator_stopped_alloc(
    alloc_id: &str,
    workload_id: &str,
    node_id: &str,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn release_service_vip_only_tick_emits_no_lifecycle_enqueue() {
    let workload_id = jid("payments");
    let digest = fake_spec_digest();
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        terminal_operator_stopped_alloc("alloc-payments-0", "payments", "local"),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: None,
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: Some(digest),
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: None,
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: Some(digest),
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        1,
        "Service intent-withdrawal tick must emit exactly one action (ReleaseServiceVip); \
         a lifecycle EnqueueEvaluation here would be spurious since no alloc-set mutation \
         occurred (the sole alloc is Terminated, so the GC branch emits no StopAllocation); \
         got {actions:?}"
    );
    assert!(
        matches!(actions[0], Action::ReleaseServiceVip { .. }),
        "the single emitted action must be ReleaseServiceVip; got {actions:?}"
    );
    assert_no_service_enqueue(&actions);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn job_kind_start_allocation_emits_no_service_enqueue() {
    let workload_id = jid("batch");
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("batch")),
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(make_job("batch")),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "Job-kind StartAllocation: StartAllocation + svid-lifecycle enqueue \
         (no service-lifecycle); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation; got {actions:?}"
    );
    assert_no_service_enqueue(&actions);
    assert_single_svid_enqueue(&actions, &workload_id);
}
