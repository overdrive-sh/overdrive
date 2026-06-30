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
use overdrive_core::id::{AllocationId, ContentHash, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{
    Action, RESTART_BACKOFF_CEILING, Reconciler, TargetResource, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

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
        // GAP-1 subsidiary: None on Pending; fixed wall-clock otherwise.
        started_at: match state {
            AllocState::Pending => None,
            _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        },
        // Host-netns acceptance fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
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
) -> (&'a overdrive_core::reconcilers::ReconcilerName, &'a TargetResource) {
    let mut found: Option<(&overdrive_core::reconcilers::ReconcilerName, &TargetResource)> = None;
    let mut count = 0;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action
            && reconciler.as_str() == "backend-discovery-bridge"
        {
            count += 1;
            found = Some((reconciler, target));
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

/// GAP-9 helper — assert that `actions` contains exactly one
/// `Action::EnqueueEvaluation` routed at `service-lifecycle` for the
/// given workload, keyed `job/<workload_id>`. Pins the Shape C dual-emit.
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
        &format!("job/{workload_id}"),
        "GAP-9: service-lifecycle enqueue target MUST be 'job/<workload_id>'"
    );
}

/// ADR-0067 D5b helper — assert that `actions` contains exactly one
/// `Action::EnqueueEvaluation` routed at `svid-lifecycle` for the given
/// workload, keyed `job/<workload_id>`. The svid-lifecycle enqueue is
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
        &format!("job/{workload_id}"),
        "ADR-0067 D5b: svid-lifecycle enqueue target MUST be 'job/<workload_id>'"
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

    // GAP-9 + ADR-0067 D5b: Service-kind StartAllocation now emits FOUR
    // actions — StartAllocation + bridge EnqueueEvaluation +
    // service-lifecycle EnqueueEvaluation + svid-lifecycle EnqueueEvaluation.
    assert_eq!(
        actions.len(),
        4,
        "expected StartAllocation + bridge enqueue + service-lifecycle enqueue \
         + svid-lifecycle enqueue; got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "first action must be StartAllocation; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
    assert_single_service_enqueue(&actions, &workload_id);
    assert_single_svid_enqueue(&actions, &workload_id);
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

    // GAP-9: StopAllocation is a terminal-REMOVAL transition — NOT an
    // alloc-starting one — so it dual-emits the bridge enqueue (the
    // bridge cares about removals) but NOT the service-lifecycle
    // enqueue (no new startup window). ADR-0067 D5b: it DOES emit the
    // svid-lifecycle enqueue (a removal must drop the held leaf). 3 actions.
    assert_eq!(
        actions.len(),
        3,
        "expected StopAllocation + bridge enqueue + svid-lifecycle enqueue \
         (no service enqueue on Stop); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation among actions; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
    assert_no_service_enqueue(&actions);
    assert_single_svid_enqueue(&actions, &workload_id);
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

    // GAP-9: GC StopAllocation is a removal — bridge enqueue only, no
    // service-lifecycle enqueue (no new startup window). ADR-0067 D5b
    // adds the svid-lifecycle enqueue (a removal drops the held leaf). 3.
    assert_eq!(
        actions.len(),
        3,
        "expected StopAllocation + bridge enqueue + svid-lifecycle enqueue \
         (no service enqueue on GC stop); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
    assert_no_service_enqueue(&actions);
    assert_single_svid_enqueue(&actions, &workload_id);
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

    // View at the ceiling: restart_counts hits RESTART_BACKOFF_CEILING,
    // so the reconciler emits FinalizeFailed (not RestartAllocation).
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(aid("alloc-payments-0"), RESTART_BACKOFF_CEILING);

    let tick = fresh_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // GAP-9: FinalizeFailed is a terminal-removal transition — bridge
    // enqueue only, no service-lifecycle enqueue. ADR-0067 D5b adds the
    // svid-lifecycle enqueue (a removal drops the held leaf). 3 actions.
    assert_eq!(
        actions.len(),
        3,
        "expected FinalizeFailed + bridge enqueue + svid-lifecycle enqueue \
         (no service enqueue on Finalize); got {actions:?}"
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
    assert_single_bridge_enqueue(&actions, &workload_id);
    assert_no_service_enqueue(&actions);
    assert_single_svid_enqueue(&actions, &workload_id);
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

    // Bridge enqueue paired ONLY with alloc-mutating actions; converged
    // tick produces zero actions including zero enqueue.
    assert!(
        actions.is_empty(),
        "converged tick must emit zero actions (no spurious bridge enqueue); got {actions:?}"
    );
}

// -------------------------------------------------------------------
// Non-alloc-mutating action classifier — `is_alloc_mutating_action`
// (reconciler.rs:1517) decides whether a given tick should append the
// bridge enqueue. The four cases above (Start / Stop / GC / Finalize)
// all return `true` from the predicate. This test pins the inverse
// case: a tick whose only emitted action is `ReleaseServiceVip` —
// which is observationally non-mutating to the bridge's alloc set —
// MUST NOT trigger a spurious bridge enqueue.
//
// Mutation kill: `replace body with true` at reconciler.rs:1517 would
// make every non-empty action vector trigger the bridge enqueue.
// Without this test, the four positive-case assertions above can't
// distinguish the real predicate from the always-true mutant, because
// every case they cover happens to contain an alloc-mutating action
// already. The Service-VIP release path is the only Phase-1 emission
// site that produces a non-empty action vector whose contents are
// strictly outside the alloc-mutating set, so it is the unique killer
// for this mutation.
// -------------------------------------------------------------------

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
        // GAP-1 subsidiary: Terminated state was Running first.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns acceptance fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    }
}

#[test]
fn release_service_vip_only_tick_emits_no_bridge_enqueue() {
    // Build a Service workload whose spec intent is WITHDRAWN
    // (`desired.job = None` — logical-workload deletion, the trigger the
    // withhold-not-release gate keys on per ADR-0049 amendment
    // 2026-06-28) AND carries a populated `service_spec_digest`
    // (the hydrator-supplied input the release-emission path
    // requires). With `desired.job = None`, `reconcile_inner` enters the
    // Absent/GC branch, which emits a `StopAllocation` only for Running
    // allocs — and the sole alloc here is Terminated, so the GC branch
    // emits NOTHING. The wrapper then appends the
    // `Action::ReleaseServiceVip` via `service_vip_release_emission`
    // (intent withdrawn ⇒ release fires).
    //
    // Net `actions` after the wrapper: `[ReleaseServiceVip]` — one
    // entry, NOT in the alloc-mutating set. With the real
    // predicate, `actions.iter().any(is_alloc_mutating_action)`
    // returns false and no bridge enqueue is appended. With the
    // mutated predicate (`body → true`), `any(...)` returns true
    // (because the vec is non-empty) and a spurious bridge enqueue
    // would be appended. The assertion below catches the spurious
    // emission.
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
        // Intent withdrawn — the deletion trigger the new release gate
        // keys on. A still-declared Service would RETAIN its VIP.
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

    // Exactly one action: the release. Total length pins both the
    // presence of the release AND the absence of any other emission
    // (including the would-be-spurious bridge enqueue under the
    // always-true mutant).
    assert_eq!(
        actions.len(),
        1,
        "Service intent-withdrawal tick must emit exactly one action (ReleaseServiceVip); \
         a bridge EnqueueEvaluation here would be spurious since no alloc-set mutation \
         occurred (the sole alloc is Terminated, so the GC branch emits no StopAllocation); \
         got {actions:?}"
    );
    assert!(
        matches!(actions[0], Action::ReleaseServiceVip { .. }),
        "the single emitted action must be ReleaseServiceVip; got {actions:?}"
    );
    // Belt-and-braces: explicitly assert that NO EnqueueEvaluation
    // routed at the bridge appears, regardless of total count.
    let bridge_enqueues = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::EnqueueEvaluation { reconciler, .. }
                    if reconciler.as_str() == "backend-discovery-bridge"
            )
        })
        .count();
    assert_eq!(
        bridge_enqueues, 0,
        "non-alloc-mutating tick (ReleaseServiceVip only) must emit ZERO bridge enqueues; \
         got {bridge_enqueues} in {actions:?} — `is_alloc_mutating_action` may have been \
         mutated to always return true"
    );
    // GAP-9: a non-alloc-mutating tick must ALSO emit zero
    // service-lifecycle enqueues (the Shape C dual-emit is gated on the
    // same `is_alloc_mutating_action` predicate as the bridge enqueue).
    assert_no_service_enqueue(&actions);
}

// -------------------------------------------------------------------
// GAP-9 — Job-kind StartAllocation MUST NOT emit a service-lifecycle
// enqueue (the bridge enqueue still fires — it is kind-agnostic).
// Pins the `desired.workload_kind == Service` gate on the Shape C
// dual-emit: mutating the gate to always-true would spuriously
// enqueue the service-lifecycle reconciler for every Job workload,
// hydrating an empty Service state and churning the broker.
// -------------------------------------------------------------------

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

    // Job-kind StartAllocation: StartAllocation + bridge enqueue +
    // svid-lifecycle enqueue (both are kind-agnostic — ADR-0067 D5b
    // svid-lifecycle is UNGATED by kind). NO service-lifecycle enqueue
    // (service-lifecycle IS Service-gated). 3 actions.
    assert_eq!(
        actions.len(),
        3,
        "Job-kind StartAllocation: StartAllocation + bridge enqueue + svid-lifecycle enqueue \
         (no service-lifecycle); got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation; got {actions:?}"
    );
    assert_single_bridge_enqueue(&actions, &workload_id);
    assert_no_service_enqueue(&actions);
    assert_single_svid_enqueue(&actions, &workload_id);
}
