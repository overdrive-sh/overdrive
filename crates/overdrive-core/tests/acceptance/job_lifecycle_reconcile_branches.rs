//! Branch-coverage tests for `JobLifecycle::reconcile`.
//!
//! Each test pins a single decision point in the reconciler against a
//! mutation that would otherwise pass the test suite silently. The
//! decision points covered:
//!
//!   - L1093 `desired.desired_to_stop && desired.job.is_some()` —
//!     two tests where ONE clause is true but the other false; under
//!     `&&` the Stop branch is skipped (no `StopAllocation` actions),
//!     under `||` the Stop branch fires and emits actions for any
//!     Running alloc. Assertions distinguish the two.
//!
//!   - L1097 `r.state == AllocState::Running` (Stop-branch filter) —
//!     the Stop branch only stops Running allocs; Pending/Terminated
//!     are skipped. Under `==` → `!=` the filter inverts: Running
//!     would be skipped and Pending/Terminated stopped instead.
//!
//!   - L1120 `r.state == AllocState::Running` (Run-branch
//!     "already-converged" probe) — under `==` → `!=` the reconciler
//!     would treat a Running alloc as not-yet-running and emit a
//!     fresh `StartAllocation`, or treat a Terminated alloc as already
//!     running and emit nothing.
//!
//!   - L1140 `attempts >= RESTART_BACKOFF_CEILING` — three
//!     parametrized samples at `ceiling-1`, `=ceiling`, `=ceiling+1`.
//!     Under `>=` → `<` the ceiling check inverts and a
//!     restart-exhausted alloc would re-emit `RestartAllocation`.
//!
//!   - L1146 `tick.now < *deadline` — three samples at `now <
//!     deadline`, `now == deadline`, `now > deadline`. Distinguishes
//!     `<` vs `==` / `>` / `<=`.
//!
//! Tests enter through the driving port (`JobLifecycle::reconcile`
//! via the `Reconciler` trait) and assert observable outcomes
//! (returned `Vec<Action>` shape and `next_view` deltas). No internal
//! state is peeked.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, Node};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, RESTART_BACKOFF_CEILING, Reconciler,
    TickContext,
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

fn local_region() -> Region {
    Region::new("local").expect("valid Region")
}

fn make_node(id: &str, capacity: Resources) -> Node {
    Node { id: nid(id), region: local_region(), capacity }
}

fn make_job(id: &str) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
    }
}

fn alloc_with_state(
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
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n =
        make_node(node_id, Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 });
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

fn one_alloc_map(alloc_id: &str, row: AllocStatusRow) -> BTreeMap<AllocationId, AllocStatusRow> {
    let mut m = BTreeMap::new();
    m.insert(aid(alloc_id), row);
    m
}

const fn empty_alloc_map() -> BTreeMap<AllocationId, AllocStatusRow> {
    BTreeMap::new()
}

fn fresh_tick(now: Instant) -> TickContext {
    TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) }
}

// -------------------------------------------------------------------
// L1093 — `desired.desired_to_stop && desired.job.is_some()` (Stop branch)
// -------------------------------------------------------------------
//
// Mutation: `&&` -> `||`.
//
// Scenarios where one clause is true but the other false. Under `&&`
// the Stop branch must be SKIPPED — falling through to the Run /
// Absent branches. Under `||` the Stop branch would fire incorrectly.

#[test]
fn stop_branch_skipped_when_stop_intent_set_but_no_job() {
    // desired_to_stop = true, job = None — the Run branch's `Some`
    // arm cannot fire (no job), the Absent arm runs and emits no
    // actions. Under `||` the Stop branch fires, attempting to
    // collect StopAllocation rows — but there are no Running allocs
    // so the action vector is also empty. To distinguish we need a
    // Running alloc present that the Stop branch WOULD stop.
    //
    // Stop intent is meaningful only when paired with a job (per
    // ADR-0027 §IntentKey::for_job_stop) — but we feed the
    // reconciler a Running alloc anyway. Under `&&`: no job → Absent
    // arm → empty actions. Under `||`: Stop branch sees Running →
    // emits StopAllocation. The two are observable.
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = JobLifecycleState {
        job: None,
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState { job: None, desired_to_stop: false, nodes, allocations };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "stop intent without a job spec must not emit StopAllocation; got {actions:?}",
    );
}

#[test]
fn stop_branch_skipped_when_job_present_but_no_stop_intent() {
    // desired_to_stop = false, job = Some — the Run branch fires.
    // With a Running alloc already present the Run branch's
    // "already-converged" probe (L1120) hits and emits no actions.
    // Under `||` the Stop branch fires AND emits StopAllocation
    // because `false || true == true`. The two are observable.
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "no stop intent + already-Running alloc must emit nothing; got {actions:?}",
    );
}

// -------------------------------------------------------------------
// L1097 — `r.state == AllocState::Running` (Stop-branch filter)
// -------------------------------------------------------------------
//
// Mutation: `==` -> `!=`.
//
// Stop branch must STOP only Running allocs. A Pending or Terminated
// alloc is NOT stopped. Under `==` → `!=` the filter inverts.

#[test]
fn stop_branch_emits_one_stop_per_running_alloc_only() {
    // Two allocs: one Running, one Terminated. Under `==`: exactly
    // one StopAllocation. Under `!=`: one StopAllocation for the
    // Terminated alloc, none for Running — different alloc id. The
    // emitted alloc_id is observable.
    let nodes = one_node_map("local");
    let mut allocs = BTreeMap::new();
    allocs.insert(
        aid("alloc-payments-0"),
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    allocs.insert(
        aid("alloc-payments-1"),
        alloc_with_state("alloc-payments-1", "payments", "local", AllocState::Terminated),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: allocs,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Under `==`: exactly one StopAllocation, naming the Running alloc.
    assert_eq!(actions.len(), 1, "must emit exactly one StopAllocation; got {actions:?}");
    match &actions[0] {
        Action::StopAllocation { alloc_id } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-0",
                "must stop the RUNNING alloc, not the Terminated one",
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// L1120 — `r.state == AllocState::Running` (Run-branch already-converged probe)
// -------------------------------------------------------------------
//
// Mutation: `==` -> `!=`.
//
// When a Running alloc exists, the reconciler must emit nothing
// (already converged). Under `==` → `!=` the probe inverts: a
// Terminated alloc would be misread as Running (already converged →
// no restart), and a Running alloc would be misread as not-running
// (emits a fresh StartAllocation, polluting the cluster).

#[test]
fn run_branch_emits_nothing_when_an_alloc_is_already_running() {
    // Single Running alloc, attempts=0 (so the restart branch is not
    // applicable anyway). Under `==`: empty actions. Under `!=`: the
    // probe misses the Running alloc → falls through to placement,
    // emits StartAllocation.
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(actions.is_empty(), "Running alloc present → must emit nothing; got {actions:?}");
}

#[test]
fn run_branch_starts_fresh_alloc_when_no_running_no_failed() {
    // No allocs at all → must emit StartAllocation (Pending →
    // Running). Distinguishes `==` from `!=` because under `!=` the
    // probe matches *every* state-discriminator, but with an empty
    // map nothing matches — so this test alone wouldn't kill the
    // mutant. Pair it with the `run_branch_emits_nothing_…` test
    // above: under `!=` that test fails (it would emit
    // StartAllocation against a Running alloc).
    let nodes = one_node_map("local");
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: empty_alloc_map(),
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "must emit one StartAllocation; got {actions:?}");
    assert!(
        matches!(actions[0], Action::StartAllocation { .. }),
        "first action must be StartAllocation; got {:?}",
        actions[0],
    );
}

// -------------------------------------------------------------------
// L1140 — `attempts >= RESTART_BACKOFF_CEILING` (backoff exhaustion)
// -------------------------------------------------------------------
//
// Mutation: `>=` -> `<`. Three parametrized samples.

#[test]
fn restart_emitted_when_attempts_below_ceiling() {
    // attempts = ceiling - 1: under `>=` (production), the check
    // returns false → restart is emitted. Under `<` (mutant), the
    // check returns true → backoff exhausted → emit nothing.
    let attempts_when_below_ceiling = RESTART_BACKOFF_CEILING - 1;
    let actions = run_with_failed_alloc_and_attempts(attempts_when_below_ceiling);
    assert_eq!(
        actions.len(),
        1,
        "attempts={attempts_when_below_ceiling} (< ceiling) must emit RestartAllocation; got {actions:?}",
    );
    assert!(
        matches!(actions[0], Action::RestartAllocation { .. }),
        "first action must be RestartAllocation; got {:?}",
        actions[0],
    );
}

#[test]
fn restart_suppressed_at_exact_ceiling() {
    // attempts == ceiling: under `>=`, true → suppressed (no
    // actions). Under `<`, false → restart emitted. The two are
    // observable.
    let actions = run_with_failed_alloc_and_attempts(RESTART_BACKOFF_CEILING);
    assert!(
        actions.is_empty(),
        "attempts == ceiling must suppress RestartAllocation; got {actions:?}",
    );
}

#[test]
fn restart_suppressed_above_ceiling() {
    // attempts == ceiling + 1: under `>=`, true → suppressed. Under
    // `<`, false → restart emitted. (This third sample is redundant
    // with the at-ceiling test for distinguishing `>=` vs `<` — it
    // is included as the explicit `=ceiling+1` boundary called out
    // in the agenda.)
    let actions = run_with_failed_alloc_and_attempts(RESTART_BACKOFF_CEILING + 1);
    assert!(
        actions.is_empty(),
        "attempts > ceiling must suppress RestartAllocation; got {actions:?}",
    );
}

fn run_with_failed_alloc_and_attempts(attempts: u32) -> Vec<Action> {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
    };
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), attempts);
    let view = JobLifecycleView { restart_counts, next_attempt_at: BTreeMap::new() };
    let tick = fresh_tick(Instant::now());

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}

// -------------------------------------------------------------------
// L1146 — `tick.now < *deadline` (backoff window check)
// -------------------------------------------------------------------
//
// Mutations: `<` -> `==`, `<` -> `>`, `<` -> `<=`.
//
// Three samples at `now < deadline`, `now == deadline`, `now >
// deadline`. Each sample distinguishes a different mutant.

#[test]
fn restart_suppressed_when_now_strictly_before_deadline() {
    // now < deadline: under `<` (production), backoff active →
    // suppressed. Under `>`: false → restart emitted. Under `==`:
    // false (now != deadline) → restart emitted. Under `<=`: true
    // (same as `<`) → suppressed (NOT distinguishable here).
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);
    let actions = run_with_failed_alloc_and_deadline(now, deadline);
    assert!(
        actions.is_empty(),
        "now < deadline must suppress RestartAllocation (backoff active); got {actions:?}",
    );
}

#[test]
fn restart_emitted_when_now_equals_deadline() {
    // now == deadline: under `<` (production), false → restart
    // emitted. Under `==` (mutant): true → suppressed. Under `<=`:
    // true → suppressed. Under `>`: false → emitted (same as
    // production for this sample). Distinguishes `<` from `==` and
    // `<=`.
    let now = Instant::now();
    let deadline = now;
    let actions = run_with_failed_alloc_and_deadline(now, deadline);
    assert_eq!(
        actions.len(),
        1,
        "now == deadline must emit RestartAllocation (backoff elapsed); got {actions:?}",
    );
    assert!(
        matches!(actions[0], Action::RestartAllocation { .. }),
        "first action must be RestartAllocation; got {:?}",
        actions[0],
    );
}

#[test]
fn restart_emitted_when_now_strictly_after_deadline() {
    // now > deadline: under `<` (production), false → emitted.
    // Under `==`: false → emitted. Under `>`: true → suppressed —
    // distinguishes `<` from `>`. Under `<=`: false → emitted.
    let deadline = Instant::now();
    let now = deadline + Duration::from_secs(60);
    let actions = run_with_failed_alloc_and_deadline(now, deadline);
    assert_eq!(
        actions.len(),
        1,
        "now > deadline must emit RestartAllocation (backoff elapsed); got {actions:?}",
    );
}

fn run_with_failed_alloc_and_deadline(now: Instant, deadline: Instant) -> Vec<Action> {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
    };
    let mut next_attempt_at = BTreeMap::new();
    next_attempt_at.insert(aid("alloc-payments-0"), deadline);
    // attempts=0 → ceiling check passes; backoff window is the
    // gating decision under test.
    let view = JobLifecycleView { restart_counts: BTreeMap::new(), next_attempt_at };
    let tick = TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) };

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}
