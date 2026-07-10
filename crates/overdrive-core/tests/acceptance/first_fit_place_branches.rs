//! Reconciler-adoption branch-coverage for `WorkloadLifecycle::reconcile`'s
//! placement path.
//!
//! Per ADR-0074 the reconciler no longer inlines placement — its Run
//! branch calls the single SSOT `overdrive_core::scheduler::schedule`
//! and maps `Ok(node_id)` → `Action::StartAllocation` /
//! `Err(PlacementError::{NoCapacity, NoHealthyNode})` → no-action
//! (behaviour-preserving adoption). These tests drive that adoption
//! through the reconciler's public `reconcile` driving port; they pin
//! that the reconciler translates a placement decision into the right
//! action shape.
//!
//! The pure-function boundary contract of `schedule` / `free_capacity`
//! itself (exact-fit inequalities, `NoCapacity{needed, max_free}`
//! fields, empty-set `NoHealthyNode`, determinism, per-component
//! `max_free`, the `&&`→`||` running-alloc filter) is covered directly
//! at the pure-function port by the migrated scheduler suite
//! (`scheduler_first_fit_happy_path`, `scheduler_capacity_accounting`,
//! `scheduler_empty_node_set`, `scheduler_determinism`,
//! `scheduler_free_capacity_strict_inequality`). To avoid shipping two
//! copies of the same boundary tests (ADR-0074 §5 dedup), the pure
//! exact-cpu / exact-memory fit boundaries formerly re-proven here at
//! the reconciler port are folded out — the scheduler suite owns them.
//! What remains here is the reconciler-specific translation the
//! scheduler port cannot observe:
//!
//!   - a fitting placement → `Action::StartAllocation { node_id }`;
//!   - a placement miss → no action emitted;
//!   - a Pending alloc on the target node does not reserve capacity,
//!     so the reconciler still passes the alloc set through and places
//!     (integration of the reconciler's alloc-set projection with the
//!     scheduler's `Running`-only reservation filter).

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{
    Action, Reconciler, TickContext, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

// -------------------------------------------------------------------
// fixtures
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

fn make_node(id: &str, capacity: Resources) -> Node {
    Node { id: nid(id), region: Region::new("local").expect("valid Region"), capacity }
}

fn make_job_with_resources(id: &str, resources: Resources) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources,
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}

fn alloc_with_state_on(
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
        kind: overdrive_core::aggregate::WorkloadKind::Service,
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

/// Canonical `fresh_tick` signature (uniform across every acceptance
/// suite per step 03-01): callers pass both `now` (monotonic) and
/// `now_unix` (wall-clock) explicitly. Tests that do not exercise the
/// wall-clock domain pass
/// `UnixInstant::from_unix_duration(Duration::from_secs(0))`.
fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// Drive the reconciler's placement path with the given `nodes`,
/// `job`, and `current_allocs`, returning the emitted actions. The
/// reconciler enters its Run branch (no Running alloc for this job
/// → `crate::scheduler::schedule` runs).
fn placement_actions(
    nodes: BTreeMap<NodeId, Node>,
    job: Job,
    current_allocs: BTreeMap<AllocationId, AllocStatusRow>,
) -> Vec<Action> {
    let wid = job.id.clone();
    let desired = WorkloadLifecycleState {
        workload_id: wid.clone(),
        job: Some(job.clone()),
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: wid,
        job: Some(job),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations: current_allocs,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}

// -------------------------------------------------------------------
// fitting placement → StartAllocation with the chosen node_id
// -------------------------------------------------------------------

#[test]
fn placement_returns_node_when_capacity_fits() {
    // Single node with abundant capacity, modest job. Production:
    // `crate::scheduler::schedule` returns `Ok(local)` → reconciler
    // emits `StartAllocation { node_id: local, … }`. Asserting on the
    // action's `node_id` proves the Ok→StartAllocation adoption.
    let mut nodes = BTreeMap::new();
    let local =
        make_node("local", Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 });
    nodes.insert(local.id.clone(), local);
    let job = make_job_with_resources(
        "payments",
        Resources { cpu_milli: 500, memory_bytes: 1024 * 1024 * 1024 },
    );

    let actions = placement_actions(nodes, job, BTreeMap::new());

    assert_eq!(
        actions.len(),
        4,
        "expected StartAllocation + EnqueueEvaluation(bridge) per UI-06 + \
         EnqueueEvaluation(service-lifecycle) per GAP-9 + \
         EnqueueEvaluation(svid-lifecycle) per ADR-0067 D5b; got {actions:?}",
    );
    match &actions[0] {
        Action::StartAllocation { node_id, .. } => {
            assert_eq!(node_id.as_str(), "local", "must place on the only fitting node");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// reconciler miss → no action (cpu fits, memory exhausted)
// -------------------------------------------------------------------

#[test]
fn placement_returns_none_when_one_resource_fits_other_does_not() {
    // Single node with cpu fits, memory exhausted. Production
    // (`&&`): false → None. Mutant (`||`): true → Some(local) →
    // emits StartAllocation. Asserting empty actions kills the
    // mutant.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        nid("local"),
        make_node("local", Resources { cpu_milli: 4_000, memory_bytes: 1024 }),
    );
    // Job needs more memory than the node has.
    let job = make_job_with_resources(
        "memhog",
        Resources { cpu_milli: 1_000, memory_bytes: 4 * 1024 * 1024 * 1024 },
    );

    let actions = placement_actions(nodes, job, BTreeMap::new());

    assert!(
        actions.is_empty(),
        "memory-exhausted node must not be selected even though cpu fits; got {actions:?}",
    );
}

// -------------------------------------------------------------------
// reconciler + scheduler integration — Running-only reservation filter
// -------------------------------------------------------------------
//
// `scheduler::free_capacity`'s `alloc.node_id == node.id && alloc.state
// == AllocState::Running` filter drives the count of
// "running-on-this-node", which subtracts from the node's declared
// capacity to produce `free`. This test proves the reconciler correctly
// passes its alloc-set projection through to the scheduler so that a
// non-`Running` (Pending) alloc on the target node does NOT reserve
// capacity and placement still succeeds.

// Note on the `free_capacity` filter's node_id / state `==` → `!=`
// mutations — these are NOT killable from the reconciler-level driving
// port at Phase 1. The reconciler short-circuits to "already converged"
// the moment ANY alloc is in state Running, so the placement path is
// never reached when there is a Running alloc anywhere. The closest
// port that IS reachable — `overdrive_core::scheduler::{schedule,
// free_capacity}` — exposes the same logic publicly; its acceptance
// tests in `tests/acceptance/scheduler_free_capacity_strict_inequality.rs`
// pin the same boundary conditions at the pure-function port. Phase 2+
// will add multi-replica scheduling, at which point the reservation
// filter becomes reachable from the reconciler with Running allocs in
// the input — at that point these mutants become killable here too.

#[test]
fn placement_excludes_non_running_allocs_on_same_node() {
    // Setup distinguishes `&&` vs `||`:
    //   - one node "local" with capacity (1500 mCPU, 2 GiB).
    //   - job needing (1000 mCPU, 1 GiB).
    //   - one allocation: same node ("local"), state = PENDING.
    //
    // Pending is chosen because the reconciler's Run branch only
    // matches Terminated/Draining as "failed_alloc" (restart
    // branch); Running short-circuits "already converged"; Pending
    // and Suspended fall through to placement, hitting
    // `crate::scheduler::free_capacity` — the reservation filter
    // under test (driven through the reconciler port).
    //
    // Production (`&&`): node_id == local AND state == Running →
    // local matches but state is Pending → 0 matches → free =
    // (1500, 2 GiB) → fits → StartAllocation emitted on "local".
    //
    // Mutant `||`: node_id == local OR state == Running → first
    // clause true → 1 match → reserves (1000, 1 GiB) → free =
    // (500, 1 GiB) → cpu < needed → None → empty actions.
    //
    // Asserting StartAllocation is emitted kills the `||` mutant.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        nid("local"),
        make_node("local", Resources { cpu_milli: 1_500, memory_bytes: 2 * 1024 * 1024 * 1024 }),
    );
    let job = make_job_with_resources(
        "payments",
        Resources { cpu_milli: 1_000, memory_bytes: 1024 * 1024 * 1024 },
    );
    let mut allocs = BTreeMap::new();
    allocs.insert(
        aid("alloc-pending-0"),
        alloc_with_state_on("alloc-pending-0", "other", "local", AllocState::Pending),
    );

    let actions = placement_actions(nodes, job, allocs);

    assert_eq!(
        actions.len(),
        4,
        "Pending alloc must NOT reserve capacity; placement must succeed (StartAllocation + EnqueueEvaluation(bridge) per UI-06 + EnqueueEvaluation(service-lifecycle) per GAP-9 + EnqueueEvaluation(svid-lifecycle) per ADR-0067 D5b); got {actions:?}",
    );
    match &actions[0] {
        Action::StartAllocation { node_id, .. } => {
            assert_eq!(node_id.as_str(), "local");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}
