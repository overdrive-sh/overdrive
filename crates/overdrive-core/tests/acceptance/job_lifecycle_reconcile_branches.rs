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
// Test-doc references mention symbol-shaped tokens (`pre-issue-141`,
// `tick_2.now_unix`, action / state names) in plain prose where
// backticking every occurrence costs more readability than it buys.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
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
        terminal: None,
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

/// Canonical `fresh_tick` signature (uniform across every acceptance
/// suite per step 03-01): callers pass both `now` (monotonic) and
/// `now_unix` (wall-clock) explicitly. Tests that do not exercise the
/// wall-clock domain pass
/// `UnixInstant::from_unix_duration(Duration::from_secs(0))`.
fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// Build a `TickContext` at `(now, now_unix)` with an explicit tick
/// counter. Both `now` and `now_unix` advance in lockstep across
/// two-tick test chains so backoff arithmetic is consistent across the
/// monotonic and wall-clock domains. Same argument shape as
/// `fresh_tick`; `tick_at_unix` adds the explicit `tick` counter for
/// multi-tick test chains where the counter must advance.
fn tick_at_unix(now: Instant, now_unix: UnixInstant, tick: u64) -> TickContext {
    TickContext { now, now_unix, tick, deadline: now + Duration::from_secs(1) }
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
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

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
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

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
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

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
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

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
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

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
    let view = JobLifecycleView { restart_counts, last_failure_seen_at: BTreeMap::new() };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}

// -------------------------------------------------------------------
// Read-site: `tick.now_unix < seen_at + backoff_for_attempt(attempts)`
// (backoff window check, recomputed from persisted inputs)
// -------------------------------------------------------------------
//
// Mutations: `<` -> `==`, `<` -> `>`, `<` -> `<=`.
//
// Three samples at `now_unix < deadline`, `now_unix == deadline`,
// `now_unix > deadline` (where `deadline = seen_at +
// backoff_for_attempt(attempts)`). Each sample distinguishes a
// different mutant. Per issue #141 the deadline is recomputed on every
// tick from the persisted inputs (`last_failure_seen_at`,
// `restart_counts`) — the helper drives this by seeding seen_at and
// letting `backoff_for_attempt(0)` produce the backoff span.

#[test]
fn restart_suppressed_when_now_strictly_before_deadline() {
    // now_unix < seen_at + backoff: under `<` (production), backoff
    // active → suppressed. Under `>`: false → restart emitted. Under
    // `==`: false → restart emitted. Under `<=`: true → suppressed
    // (NOT distinguishable here).
    let seen_at = UnixInstant::from_unix_duration(Duration::from_secs(1_000));
    let now_unix = seen_at;
    let actions = run_with_failed_alloc_and_seen_at(now_unix, seen_at);
    assert!(
        actions.is_empty(),
        "now_unix < seen_at + backoff must suppress RestartAllocation \
         (backoff active); got {actions:?}",
    );
}

#[test]
fn restart_emitted_when_now_equals_deadline() {
    // now_unix == seen_at + backoff: under `<` (production), false →
    // restart emitted. Under `==` (mutant): true → suppressed. Under
    // `<=`: true → suppressed. Under `>`: false → emitted.
    // Distinguishes `<` from `==` and `<=`.
    let seen_at = UnixInstant::from_unix_duration(Duration::from_secs(1_000));
    // attempts=0 in the helper, so backoff = backoff_for_attempt(0) =
    // RESTART_BACKOFF_DURATION.
    let now_unix = seen_at + RESTART_BACKOFF_DURATION;
    let actions = run_with_failed_alloc_and_seen_at(now_unix, seen_at);
    assert_eq!(
        actions.len(),
        1,
        "now_unix == seen_at + backoff must emit RestartAllocation \
         (backoff elapsed); got {actions:?}",
    );
    assert!(
        matches!(actions[0], Action::RestartAllocation { .. }),
        "first action must be RestartAllocation; got {:?}",
        actions[0],
    );
}

#[test]
fn restart_emitted_when_now_strictly_after_deadline() {
    // now_unix > seen_at + backoff: under `<` (production), false →
    // emitted. Under `==`: false → emitted. Under `>`: true →
    // suppressed — distinguishes `<` from `>`. Under `<=`: false →
    // emitted.
    let seen_at = UnixInstant::from_unix_duration(Duration::from_secs(1_000));
    let now_unix = seen_at + RESTART_BACKOFF_DURATION + Duration::from_secs(60);
    let actions = run_with_failed_alloc_and_seen_at(now_unix, seen_at);
    assert_eq!(
        actions.len(),
        1,
        "now_unix > seen_at + backoff must emit RestartAllocation \
         (backoff elapsed); got {actions:?}",
    );
}

fn run_with_failed_alloc_and_seen_at(now_unix: UnixInstant, seen_at: UnixInstant) -> Vec<Action> {
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
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(aid("alloc-payments-0"), seen_at);
    // attempts=0 → ceiling check passes AND backoff_for_attempt(0)
    // = RESTART_BACKOFF_DURATION; backoff window is the gating
    // decision under test.
    let view = JobLifecycleView { restart_counts: BTreeMap::new(), last_failure_seen_at };
    let tick = fresh_tick(Instant::now(), now_unix);

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);
    actions
}

// -------------------------------------------------------------------
// Write-side regression — RestartAllocation branch must materialise
// `next_view.last_failure_seen_at`
// -------------------------------------------------------------------
//
// Companion to the read-site gate tests above. The existing
// tests hand-seed `view.last_failure_seen_at` and assert the gate fires
// correctly given a populated observation timestamp; they cannot
// detect that `reconcile`'s emission branch never *writes* a
// timestamp back into `next_view`. Without these write-side
// assertions the field is inert from one tick to the next, every
// Terminated alloc re-emits `RestartAllocation` immediately, and
// `RESTART_BACKOFF_CEILING = 5` is exhausted in ~500 ms instead of
// the spec'd `5 × RESTART_BACKOFF_DURATION` window.
//
// See `docs/feature/fix-restart-backoff-deadline-not-written/deliver/rca.md`.
// Per issue #141: the persisted value is the failure observation
// timestamp (`tick.now_unix`) — NOT a precomputed deadline.

/// Single-tick: a fresh Terminated alloc with no prior view entries
/// must emit `RestartAllocation` AND populate
/// `next_view.last_failure_seen_at[<alloc_id>]` with `tick.now_unix`
/// (the observation timestamp, NOT `tick.now_unix +
/// RESTART_BACKOFF_DURATION`). Restart count goes from 0 to 1.
#[test]
fn fresh_failure_writes_seen_at_into_next_view() {
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
    let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick = tick_at_unix(now, now_unix, 0);

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

    // Observation timestamp written: tick.now_unix (the *input*, NOT a
    // precomputed deadline). Per issue #141 — persist inputs, recompute
    // deadlines on read.
    assert_eq!(
        next_view.last_failure_seen_at.get(&aid("alloc-payments-0")).copied(),
        Some(now_unix),
        "last_failure_seen_at must be populated with tick.now_unix \
         (the observation timestamp, NOT a precomputed deadline)",
    );
}

/// Two-tick chain: tick 1 emits a restart and writes a seen_at;
/// tick 2 advances `tick.now_unix` by less than
/// `RESTART_BACKOFF_DURATION` and asserts the gate fires — empty
/// actions, count NOT bumped, seen_at NOT advanced. This is the
/// regression evidence: against the pre-issue-141 `main`, tick 2
/// re-emits `RestartAllocation` because `view.last_failure_seen_at`
/// was never populated by tick 1.
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
    let now_unix_1 = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick_1 = tick_at_unix(now_1, now_unix_1, 0);

    let r = JobLifecycle::canonical();
    let (_actions_1, next_view_1) = r.reconcile(&desired, &actual, &view_1, &tick_1);

    // Tick 2: advance now_unix by less than RESTART_BACKOFF_DURATION.
    // The gate must fire (`now_unix < seen_at + backoff`) and emit
    // nothing. The view fed in IS the next_view from tick 1.
    let now_2 = now_1 + Duration::from_millis(500);
    let now_unix_2 = now_unix_1 + Duration::from_millis(500);
    let tick_2 = tick_at_unix(now_2, now_unix_2, 1);

    let (actions_2, next_view_2) = r.reconcile(&desired, &actual, &next_view_1, &tick_2);

    assert!(
        actions_2.is_empty(),
        "tick 2 within backoff window (now_unix < seen_at + backoff) must emit nothing; \
         got {actions_2:?}",
    );

    // Count NOT bumped during a gated tick — the alloc was never
    // restarted on this tick.
    assert_eq!(
        next_view_2.restart_counts.get(&aid("alloc-payments-0")).copied(),
        next_view_1.restart_counts.get(&aid("alloc-payments-0")).copied(),
        "restart count must not advance on a gated tick",
    );

    // seen_at NOT advanced — the same observation timestamp survives a
    // gated tick. (Advancing it on a gated tick would let failures
    // slip past the ceiling indefinitely.)
    assert_eq!(
        next_view_2.last_failure_seen_at.get(&aid("alloc-payments-0")).copied(),
        next_view_1.last_failure_seen_at.get(&aid("alloc-payments-0")).copied(),
        "last_failure_seen_at must not advance on a gated tick",
    );
}

/// Two-tick chain: tick 1 emits a restart and writes a seen_at; tick
/// 2 advances `tick.now_unix` past `RESTART_BACKOFF_DURATION` and
/// asserts another restart fires, count is bumped, AND the seen_at
/// rolls forward to `tick_2.now_unix` (the new observation
/// timestamp, NOT the previous seen_at + window). This pins the spec
/// semantics: each restart attempt records a fresh failure
/// observation.
#[test]
fn tick_after_backoff_elapsed_emits_restart_and_advances_seen_at() {
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
    let now_unix_1 = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick_1 = tick_at_unix(now_1, now_unix_1, 0);

    let r = JobLifecycle::canonical();
    let (_actions_1, next_view_1) = r.reconcile(&desired, &actual, &view_1, &tick_1);

    // Tick 2: advance now_unix strictly past RESTART_BACKOFF_DURATION.
    // Gate elapsed → another restart must fire, seen_at rolls forward
    // to the new tick's now_unix.
    let now_2 = now_1 + RESTART_BACKOFF_DURATION + Duration::from_millis(1);
    let now_unix_2 = now_unix_1 + RESTART_BACKOFF_DURATION + Duration::from_millis(1);
    let tick_2 = tick_at_unix(now_2, now_unix_2, 1);

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

    // seen_at rolls forward to tick_2.now_unix — NOT the old seen_at +
    // window. This pins the spec semantics: each restart attempt
    // records a fresh failure observation, and the deadline is
    // recomputed from it on subsequent reads.
    assert_eq!(
        next_view_2.last_failure_seen_at.get(&aid("alloc-payments-0")).copied(),
        Some(now_unix_2),
        "deadline must roll forward to new tick.now + RESTART_BACKOFF_DURATION",
    );
}

// -------------------------------------------------------------------
// fix-stop-branch-backoff-pending — Stop branch must clear
// `last_failure_seen_at` when there are no Running allocs to stop
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
// Pre-fix (pre-stop-branch-backoff-pending): the Stop branch returned
// `(stop_actions, view.clone())`. With no Running allocs, `stop_actions`
// is empty BUT `view.last_failure_seen_at` is unchanged — still names
// the Failed alloc — so `view_has_backoff_pending` returns true and
// the runtime self-re-enqueues every tick.
//
// Post-fix (the Stop branch clears `last_failure_seen_at` when
// `stop_actions.is_empty()`): predicate returns false on the first
// post-stop tick → broker drains and stays empty.

/// A Stop branch with `desired_to_stop = true`, a Failed alloc (no
/// Running allocs to stop), and a populated `view.last_failure_seen_at`
/// must return `(actions: empty, next_view: last_failure_seen_at
/// cleared)`. The cleared map is the load-bearing assertion: it is
/// what `view_has_backoff_pending` reads in the runtime to decide
/// whether to self-re-enqueue.
#[test]
fn stop_branch_clears_last_failure_seen_at_when_no_running_allocs() {
    // desired_to_stop = true, job is present, the only alloc is
    // Failed (NOT Running). The Stop branch fires but `stop_actions`
    // is empty — there is nothing to stop. The view carries a
    // populated `last_failure_seen_at` from a prior restart-with-backoff
    // tick (the Failed-mid-backoff intersection).
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
    // last_failure_seen_at carries a non-empty seen_at for the alloc.
    let now = Instant::now();
    let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), 1);
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(aid("alloc-payments-0"), now_unix);
    let view = JobLifecycleView { restart_counts, last_failure_seen_at };
    let tick = tick_at_unix(now, now_unix, 0);

    let r = JobLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    // Sanity: no actions emitted (no Running allocs to stop).
    assert!(
        actions.is_empty(),
        "stop intent + no Running allocs must emit no actions; got {actions:?}",
    );

    // The load-bearing assertion: the Stop branch must clear
    // `last_failure_seen_at` to signal "stop is complete; no pending
    // work" to the runtime's `view_has_backoff_pending` predicate.
    assert!(
        next_view.last_failure_seen_at.is_empty(),
        "Stop branch must clear `last_failure_seen_at` when there are no \
         Running allocs to stop, otherwise `view_has_backoff_pending` \
         keeps `has_work = true` and the broker self-re-enqueues every \
         tick until `restart_counts` reaches RESTART_BACKOFF_CEILING. \
         Got last_failure_seen_at = {:?} (expected empty)",
        next_view.last_failure_seen_at,
    );
}
