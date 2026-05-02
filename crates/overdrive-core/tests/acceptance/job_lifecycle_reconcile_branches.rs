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

use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, RESTART_BACKOFF_CEILING,
    RESTART_BACKOFF_DURATION, Reconciler, TickContext,
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
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
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
        reason: None,
        detail: None,
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

// -------------------------------------------------------------------
// Write-side regression — RestartAllocation branch must materialise
// `next_view.next_attempt_at`
// -------------------------------------------------------------------
//
// Companion to the L1146 read-side gate tests above. The existing
// tests hand-seed `view.next_attempt_at` and assert the gate fires
// correctly given a populated deadline; they cannot detect that
// `reconcile`'s emission branch never *writes* a deadline back into
// `next_view`. Without these write-side assertions the field is
// inert from one tick to the next, every Terminated alloc re-emits
// `RestartAllocation` immediately, and `RESTART_BACKOFF_CEILING = 5`
// is exhausted in ~500 ms instead of the spec'd
// `5 × RESTART_BACKOFF_DURATION` window.
//
// See `docs/feature/fix-restart-backoff-deadline-not-written/deliver/rca.md`.
//
// These three tests reference `RESTART_BACKOFF_DURATION` directly.
// The constant lands in step 01-02; until then the import is
// unresolved and the file fails to compile. The compile failure IS
// the RED state per `.claude/rules/testing.md` § "RED scaffolds and
// intentionally-failing commits".

/// Single-tick: a fresh Terminated alloc with no prior view entries
/// must emit `RestartAllocation` AND populate
/// `next_view.next_attempt_at[<alloc_id>]` with `tick.now +
/// RESTART_BACKOFF_DURATION`. Restart count goes from 0 to 1.
///
/// This is the load-bearing assertion. Against current `main` the
/// `next_attempt_at` map remains empty after the call — the bug.
#[test]
fn fresh_failure_writes_deadline_into_next_view() {
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
    // view is empty — fresh failure, no prior restart bookkeeping.
    let view = JobLifecycleView::default();
    let now = Instant::now();
    let tick = fresh_tick(now);

    let r = JobLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    // RestartAllocation emitted for the failed alloc.
    assert_eq!(actions.len(), 1, "fresh failure must emit one RestartAllocation; got {actions:?}");
    match &actions[0] {
        Action::RestartAllocation { alloc_id, .. } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
        }
        other => panic!("expected RestartAllocation, got {other:?}"),
    }

    // Restart count incremented from 0 to 1.
    assert_eq!(
        next_view.restart_counts.get(&aid("alloc-payments-0")).copied(),
        Some(1),
        "restart count must be incremented to 1 on first failure",
    );

    // Deadline written: tick.now + RESTART_BACKOFF_DURATION. This is
    // the assertion that catches the dead-code bug.
    assert_eq!(
        next_view.next_attempt_at.get(&aid("alloc-payments-0")).copied(),
        Some(now + RESTART_BACKOFF_DURATION),
        "next_attempt_at must be populated with tick.now + RESTART_BACKOFF_DURATION; \
         empty map indicates the deadline was never written",
    );
}

/// Two-tick chain: tick 1 emits a restart and writes a deadline;
/// tick 2 advances `tick.now` by less than `RESTART_BACKOFF_DURATION`
/// and asserts the gate fires — empty actions, count NOT bumped,
/// deadline NOT advanced. This is the regression evidence: against
/// current `main`, tick 2 re-emits `RestartAllocation` because
/// `view.next_attempt_at` was never populated by tick 1.
#[test]
fn subsequent_tick_within_backoff_window_emits_nothing() {
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

    // Tick 1: fresh failure. Capture next_view as the input to tick 2.
    let view_1 = JobLifecycleView::default();
    let now_1 = Instant::now();
    let tick_1 = fresh_tick(now_1);

    let r = JobLifecycle::canonical();
    let (_actions_1, next_view_1) = r.reconcile(&desired, &actual, &view_1, &tick_1);

    // Tick 2: advance now by less than RESTART_BACKOFF_DURATION. The
    // gate must fire (now < deadline) and emit nothing. The view fed
    // in IS the next_view from tick 1 — this is what makes the test
    // a true regression for the "deadline never written" bug.
    let now_2 = now_1 + Duration::from_millis(500);
    let tick_2 = fresh_tick(now_2);

    let (actions_2, next_view_2) = r.reconcile(&desired, &actual, &next_view_1, &tick_2);

    assert!(
        actions_2.is_empty(),
        "tick 2 within backoff window (now < deadline) must emit nothing; got {actions_2:?}",
    );

    // Count NOT bumped during a gated tick — the alloc was never
    // restarted on this tick.
    assert_eq!(
        next_view_2.restart_counts.get(&aid("alloc-payments-0")).copied(),
        next_view_1.restart_counts.get(&aid("alloc-payments-0")).copied(),
        "restart count must not advance on a gated tick",
    );

    // Deadline NOT advanced — the same deadline survives a gated
    // tick. (Advancing the deadline on a gated tick would let
    // failures slip past the ceiling indefinitely.)
    assert_eq!(
        next_view_2.next_attempt_at.get(&aid("alloc-payments-0")).copied(),
        next_view_1.next_attempt_at.get(&aid("alloc-payments-0")).copied(),
        "next_attempt_at must not advance on a gated tick",
    );
}

/// Two-tick chain: tick 1 emits a restart and writes a deadline;
/// tick 2 advances `tick.now` past `RESTART_BACKOFF_DURATION` and
/// asserts another restart fires, count is bumped, AND the deadline
/// rolls forward to `new tick.now + RESTART_BACKOFF_DURATION` (NOT
/// the previous deadline + `RESTART_BACKOFF_DURATION` — the spec
/// pins the new window to the current tick).
#[test]
fn tick_after_backoff_elapsed_emits_restart_and_advances_deadline() {
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

    // Tick 1: fresh failure.
    let view_1 = JobLifecycleView::default();
    let now_1 = Instant::now();
    let tick_1 = fresh_tick(now_1);

    let r = JobLifecycle::canonical();
    let (_actions_1, next_view_1) = r.reconcile(&desired, &actual, &view_1, &tick_1);

    // Tick 2: advance now strictly past RESTART_BACKOFF_DURATION.
    // Gate elapsed → another restart must fire, deadline rolls
    // forward to the new tick's now + window.
    let now_2 = now_1 + RESTART_BACKOFF_DURATION + Duration::from_millis(1);
    let tick_2 = TickContext { now: now_2, tick: 1, deadline: now_2 + Duration::from_secs(1) };

    let (actions_2, next_view_2) = r.reconcile(&desired, &actual, &next_view_1, &tick_2);

    assert_eq!(
        actions_2.len(),
        1,
        "tick 2 after backoff elapsed must emit one RestartAllocation; got {actions_2:?}",
    );
    assert!(
        matches!(actions_2[0], Action::RestartAllocation { .. }),
        "first action must be RestartAllocation; got {:?}",
        actions_2[0],
    );

    // Count bumped by exactly 1.
    let count_1 = next_view_1.restart_counts.get(&aid("alloc-payments-0")).copied().unwrap_or(0);
    let count_2 = next_view_2.restart_counts.get(&aid("alloc-payments-0")).copied().unwrap_or(0);
    assert_eq!(count_2, count_1 + 1, "restart count must advance by exactly 1 on a non-gated tick");

    // Deadline rolls forward to the *new* tick's now + window — NOT
    // the old deadline + window. This pins the spec semantics:
    // backoff window resets relative to the current tick on each
    // restart attempt.
    assert_eq!(
        next_view_2.next_attempt_at.get(&aid("alloc-payments-0")).copied(),
        Some(now_2 + RESTART_BACKOFF_DURATION),
        "deadline must roll forward to new tick.now + RESTART_BACKOFF_DURATION",
    );
}

// -------------------------------------------------------------------
// fix-stop-branch-backoff-pending — Stop branch must clear
// `next_attempt_at` when there are no Running allocs to stop
// -------------------------------------------------------------------
//
// Companion to the DST acceptance test at
// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`
// — that test pins the user-visible symptom (broker drains after stop);
// THIS test pins the pure-function contract that the symptom flows
// from. See `docs/feature/fix-stop-branch-backoff-pending/deliver/rca.md`
// §"Root cause" for the architectural argument: the §18 *Level-triggered
// inside the reconciler* contract says "actual ≠ desired" is signalled
// by EITHER non-empty actions OR transitional view state (a backoff
// deadline pending). The Stop branch's `view.clone()` pass-through
// violates the second clause for the Failed-mid-backoff intersection.
//
// Pre-fix (current `main`): `reconciler.rs:1019-1027` returns
// `(stop_actions, view.clone())`. With no Running allocs, `stop_actions`
// is empty BUT `view.next_attempt_at` is unchanged — still names the
// Failed alloc — so `view_has_backoff_pending` returns true and the
// runtime self-re-enqueues every tick.
//
// Post-fix (the GREEN edit at step 01-02): the Stop branch clears
// `next_attempt_at` when `stop_actions.is_empty()`. This test pins the
// expected post-fix `next_view`.
//
// The `#[ignore]` attribute keeps this test out of the lefthook
// nextest-affected pre-commit pass between the RED scaffold commit
// and the GREEN fix commit. Step 01-02 removes the `#[ignore]` in the
// same commit as the production edit; the test transitions skipped →
// executed-and-passing.

/// RED — a Stop branch with `desired_to_stop = true`, a Failed
/// alloc (no Running allocs to stop), and a populated
/// `view.next_attempt_at` must return `(actions: empty,
/// next_view: next_attempt_at cleared)`. The cleared deadline is the
/// load-bearing assertion: it is what `view_has_backoff_pending`
/// reads in the runtime to decide whether to self-re-enqueue.
///
/// Pre-fix: `next_view.next_attempt_at` still contains the alloc
/// entry, the runtime treats this as "transitional state" and
/// re-enqueues every tick until `restart_counts` reaches the
/// `RESTART_BACKOFF_CEILING`.
#[test]
fn stop_branch_clears_next_attempt_at_when_no_running_allocs() {
    // desired_to_stop = true, job is present, the only alloc is
    // Failed (NOT Running). The Stop branch fires but `stop_actions`
    // is empty — there is nothing to stop. The view carries a
    // populated `next_attempt_at` from a prior restart-with-backoff
    // tick (this is the Failed-mid-backoff intersection from the RCA).
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Failed),
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
        allocations,
    };

    // Seed the view: restart_counts < CEILING (so
    // view_has_backoff_pending would return true pre-fix);
    // next_attempt_at carries a non-empty deadline for the alloc.
    let now = Instant::now();
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), 1);
    let mut next_attempt_at = BTreeMap::new();
    next_attempt_at.insert(aid("alloc-payments-0"), now + RESTART_BACKOFF_DURATION);
    let view = JobLifecycleView { restart_counts, next_attempt_at };
    let tick = fresh_tick(now);

    let r = JobLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    // Sanity: no actions emitted (no Running allocs to stop).
    assert!(
        actions.is_empty(),
        "stop intent + no Running allocs must emit no actions; got {actions:?}",
    );

    // The load-bearing assertion: the Stop branch must clear
    // `next_attempt_at` to signal "stop is complete; no pending
    // work" to the runtime's `view_has_backoff_pending` predicate.
    //
    // Pre-fix (`view.clone()` pass-through): `next_attempt_at`
    // still contains the alloc → predicate returns true →
    // self-re-enqueue every tick → broker spins for ~5 s until
    // `restart_counts` reaches the ceiling.
    //
    // Post-fix (clear `next_attempt_at` when `stop_actions.is_empty()`):
    // predicate returns false on the first post-stop tick → broker
    // drains and stays empty.
    assert!(
        next_view.next_attempt_at.is_empty(),
        "Stop branch must clear `next_attempt_at` when there are no \
         Running allocs to stop, otherwise `view_has_backoff_pending` \
         keeps `has_work = true` and the broker self-re-enqueues every \
         tick until `restart_counts` reaches RESTART_BACKOFF_CEILING. \
         Got next_attempt_at = {:?} (expected empty)",
        next_view.next_attempt_at,
    );
}
