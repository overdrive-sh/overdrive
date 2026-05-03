//! Branch-coverage tests for the `first_fit_place` and
//! `node_free_capacity` helpers in `overdrive-core::reconciler`.
//!
//! These helpers are private; we drive them through their only
//! public caller: `JobLifecycle::reconcile`'s Run branch (placement
//! emits `Action::StartAllocation` with the chosen `node_id`, or
//! emits nothing when no node fits). The decisions covered:
//!
//!   - L1204 `first_fit_place -> Option<NodeId>` body→`None` —
//!     a happy-path placement test where production returns
//!     `Some(local)` would emit `StartAllocation { node_id }`.
//!     Under `body→None` reconcile emits no action. Asserting on
//!     `Action::StartAllocation` presence and the `node_id` value
//!     kills the mutant.
//!
//!   - L1206 `free.cpu_milli >= job.resources.cpu_milli` (`>=` →
//!     `<`) — boundary at exact-fit on cpu. Under `>=`: matches,
//!     placement succeeds. Under `<`: no match, returns None.
//!
//!   - L1207 `free.memory_bytes >= job.resources.memory_bytes`
//!     (`>=` → `<`) — same boundary on memory.
//!
//!   - L1207 `&& -> ||` — two-node case where ONE node fits cpu,
//!     OTHER fits memory; production returns None, mutant returns
//!     the first node.
//!
//!   - L1226 `alloc.node_id == node.id && alloc.state ==
//!     AllocState::Running` (`&&` → `||`, `==` → `!=` ×2) — drives
//!     the running-on-this-node count in `node_free_capacity`. We
//!     stage allocs whose join under production produces exactly
//!     N=1 reservation, but under any of the three mutations
//!     differs, so the resulting `free` capacity differs and the
//!     placement decision flips.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, Reconciler, TickContext,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

// -------------------------------------------------------------------
// fixtures
// -------------------------------------------------------------------

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn jid(s: &str) -> JobId {
    JobId::new(s).expect("valid JobId")
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
    job_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(node_id) },
        reason: None,
        detail: None,
    }
}

fn fresh_tick(now: Instant) -> TickContext {
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// Drive the reconciler's placement path with the given `nodes`,
/// `job`, and `current_allocs`, returning the emitted actions. The
/// reconciler enters its Run branch (no Running alloc for this job
/// → `first_fit_place` runs).
fn placement_actions(
    nodes: BTreeMap<NodeId, Node>,
    job: Job,
    current_allocs: BTreeMap<AllocationId, AllocStatusRow>,
) -> Vec<Action> {
    let desired = JobLifecycleState {
        job: Some(job.clone()),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(job),
        desired_to_stop: false,
        nodes,
        allocations: current_allocs,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}

// -------------------------------------------------------------------
// L1204 — `first_fit_place -> Option<NodeId>` body→None
// -------------------------------------------------------------------

#[test]
fn placement_returns_node_when_capacity_fits() {
    // Single node with abundant capacity, modest job. Production:
    // `first_fit_place` returns `Some(local)` → reconciler emits
    // `StartAllocation { node_id: local, … }`. Mutant (`body→None`):
    // emits no action. Asserting on the action's `node_id` kills the
    // mutant.
    let mut nodes = BTreeMap::new();
    let local =
        make_node("local", Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 });
    nodes.insert(local.id.clone(), local);
    let job = make_job_with_resources(
        "payments",
        Resources { cpu_milli: 500, memory_bytes: 1024 * 1024 * 1024 },
    );

    let actions = placement_actions(nodes, job, BTreeMap::new());

    assert_eq!(actions.len(), 1, "expected one StartAllocation; got {actions:?}");
    match &actions[0] {
        Action::StartAllocation { node_id, .. } => {
            assert_eq!(node_id.as_str(), "local", "must place on the only fitting node");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// L1206 — `free.cpu_milli >= job.resources.cpu_milli` (`>=` → `<`)
// -------------------------------------------------------------------

#[test]
fn placement_succeeds_at_exact_cpu_fit_with_memory_excess() {
    // free.cpu == needed.cpu; free.mem > needed.mem.
    // Under `>=`: 1000 >= 1000 → true; 4 GiB >= 1 GiB → true →
    // Some(local). Under `<`: 1000 < 1000 → false → None.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        nid("local"),
        make_node("local", Resources { cpu_milli: 1_000, memory_bytes: 4 * 1024 * 1024 * 1024 }),
    );
    let job = make_job_with_resources(
        "payments",
        Resources { cpu_milli: 1_000, memory_bytes: 1024 * 1024 * 1024 },
    );

    let actions = placement_actions(nodes, job, BTreeMap::new());

    assert_eq!(actions.len(), 1, "exact-fit on cpu must place; got {actions:?}");
    assert!(matches!(actions[0], Action::StartAllocation { .. }));
}

// -------------------------------------------------------------------
// L1207 — `free.memory_bytes >= job.resources.memory_bytes` (`>=` → `<`)
// -------------------------------------------------------------------

#[test]
fn placement_succeeds_at_exact_memory_fit_with_cpu_excess() {
    // free.mem == needed.mem; free.cpu > needed.cpu.
    // Under `>=`: 1 GiB >= 1 GiB → true → Some(local). Under `<`:
    // 1 GiB < 1 GiB → false → None.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        nid("local"),
        make_node("local", Resources { cpu_milli: 4_000, memory_bytes: 1024 * 1024 * 1024 }),
    );
    let job = make_job_with_resources(
        "payments",
        Resources { cpu_milli: 500, memory_bytes: 1024 * 1024 * 1024 },
    );

    let actions = placement_actions(nodes, job, BTreeMap::new());

    assert_eq!(actions.len(), 1, "exact-fit on memory must place; got {actions:?}");
    assert!(matches!(actions[0], Action::StartAllocation { .. }));
}

// -------------------------------------------------------------------
// L1207 — `&&` -> `||` between cpu and memory checks
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
// L1226 — `alloc.node_id == node.id && alloc.state == AllocState::Running`
// -------------------------------------------------------------------
//
// Three mutations on this single line:
//
//   - L1226:43 `alloc.node_id == node.id` -> `!=`
//   - L1226:54 `&&` -> `||`
//   - L1226:69 `alloc.state == AllocState::Running` -> `!=`
//
// All three drive the count of "running-on-this-node" in
// `node_free_capacity`, which then subtracts from the node's
// declared capacity to produce `free`. We construct fixtures whose
// resulting `free` capacity flips the placement decision under each
// mutation.

// Note on `node_free_capacity` mutations (L1226:43 `==` → `!=` on
// node_id; L1226:69 `==` → `!=` on state) — these are NOT killable
// from the reconciler-level driving port at Phase 1. The reconciler
// short-circuits to "already converged" the moment ANY alloc is in
// state Running (line 1120 check), so `first_fit_place` /
// `node_free_capacity` are never reached when there is a Running
// alloc anywhere. The closest sibling helper that IS reachable —
// `overdrive_scheduler::free_capacity` — exposes the same logic
// publicly; its acceptance tests in
// `overdrive-scheduler/tests/acceptance/free_capacity_strict_inequality.rs`
// pin the same boundary conditions for the public API. Phase 2+ will
// add multi-replica scheduling, at which point `node_free_capacity`
// becomes reachable from the reconciler with Running allocs in the
// input — at that point these mutants become killable here too.

#[test]
fn node_free_capacity_excludes_non_running_allocs_on_same_node() {
    // Setup distinguishes `&&` vs `||`:
    //   - one node "local" with capacity (1500 mCPU, 2 GiB).
    //   - job needing (1000 mCPU, 1 GiB).
    //   - one allocation: same node ("local"), state = PENDING.
    //
    // Pending is chosen because the reconciler's Run branch only
    // matches Terminated/Draining as "failed_alloc" (restart
    // branch); Running short-circuits "already converged"; Pending
    // and Suspended fall through to placement, hitting
    // `node_free_capacity` — exactly the helper under test.
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
        1,
        "Pending alloc must NOT reserve capacity; placement must succeed; got {actions:?}",
    );
    match &actions[0] {
        Action::StartAllocation { node_id, .. } => {
            assert_eq!(node_id.as_str(), "local");
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}
