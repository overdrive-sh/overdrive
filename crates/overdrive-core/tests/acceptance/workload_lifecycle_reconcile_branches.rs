//! Branch-coverage tests for `WorkloadLifecycle::reconcile`.
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
//! Tests enter through the driving port (`WorkloadLifecycle::reconcile`
//! via the `Reconciler` trait) and assert observable outcomes
//! (returned `Vec<Action>` shape and `next_view` deltas). No internal
//! state is peeked.

#![allow(clippy::expect_used)]
// Test-doc references mention symbol-shaped tokens (`pre-issue-141`,
// `tick_2.now_unix`, action / state names) in plain prose where
// backticking every occurrence costs more readability than it buys.
#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{
    Action, RESTART_BACKOFF_CEILING, RESTART_BACKOFF_DURATION, Reconciler, TickContext,
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

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
    // desired_to_stop = true, job = None — the Stop branch is
    // SKIPPED because `&&` requires both clauses true; control falls
    // through to the GC arm (#148, ADR-0037 Amendment 2026-05-14)
    // which emits one StopAllocation per Running orphan stamped with
    // `Stopped { by: SystemGc }`.
    //
    // Mutation discrimination on the original `&&` clause is
    // preserved by the terminal-by-source discriminator:
    //   - Under `&&` (correct): Stop branch skipped → GC arm fires →
    //     emits StopAllocation { terminal: Some(Stopped { by: SystemGc }) }.
    //   - Under `||` (mutation): Stop branch fires (`true || false`
    //     evaluates true even with `job: None`) → emits
    //     StopAllocation { terminal: Some(Stopped { by: Operator }) }.
    // The action count alone is identical (1 in both); the
    // `terminal` field's by-source distinguishes them.
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: jid("absent"),
        job: None,
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("absent"),
        job: None,
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // The GC arm fires (job is None) and emits one StopAllocation
    // for the orphan Running row. The terminal MUST carry SystemGc,
    // not Operator — that is what distinguishes `&&` (correct,
    // skipped Stop branch → GC arm) from `||` (mutation, Stop branch
    // fires with Operator terminal).
    assert_eq!(
        actions.len(),
        2,
        "GC arm must emit StopAllocation for the orphan Running row + bridge \
         EnqueueEvaluation per UI-06; got {actions:?}",
    );
    match &actions[0] {
        Action::StopAllocation { terminal, .. } => {
            assert_eq!(
                terminal,
                &Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }),
                "with `&&` the GC arm fires and stamps SystemGc; with `||` the Stop \
                 branch would fire and stamp Operator. Got terminal = {terminal:?}",
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: allocs,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Under `==`: exactly one StopAllocation, naming the Running alloc.
    assert_eq!(
        actions.len(),
        2,
        "must emit StopAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    match &actions[0] {
        Action::StopAllocation { alloc_id, .. } => {
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: empty_alloc_map(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "must emit StartAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    assert!(
        matches!(
            actions[0],
            Action::StartAllocation { kind: overdrive_core::aggregate::WorkloadKind::Service, .. }
        ),
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
        2,
        "attempts={attempts_when_below_ceiling} (< ceiling) must emit RestartAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}",
    );
    assert!(
        matches!(
            actions[0],
            Action::RestartAllocation {
                kind: overdrive_core::aggregate::WorkloadKind::Service,
                ..
            }
        ),
        "first action must be RestartAllocation; got {:?}",
        actions[0],
    );
}

#[test]
fn restart_suppressed_at_exact_ceiling() {
    // attempts == ceiling: under `>=`, true → restart suppressed and
    // a `FinalizeFailed` synthetic action carrying the typed
    // `BackoffExhausted` terminal claim is emitted (per ADR-0037 §4
    // — the reconciler is the single source of every terminal claim;
    // this is the "synthetic Failed-row action shape" reference).
    // Under `<`, false → RestartAllocation emitted instead. The two
    // are observable.
    let actions = run_with_failed_alloc_and_attempts(RESTART_BACKOFF_CEILING);
    assert_eq!(
        actions.len(),
        2,
        "attempts == ceiling must emit FinalizeFailed + bridge EnqueueEvaluation per UI-06; got {actions:?}",
    );
    assert!(
        matches!(actions[0], Action::FinalizeFailed { .. }),
        "first action must be FinalizeFailed; got {:?}",
        actions[0],
    );
}

#[test]
fn restart_suppressed_above_ceiling() {
    // attempts == ceiling + 1: under `>=`, true → restart suppressed
    // and FinalizeFailed emitted (mirrors `==ceiling`). Under `<`,
    // false → RestartAllocation. (This third sample is redundant
    // with the at-ceiling test for distinguishing `>=` vs `<` — it
    // is included as the explicit `=ceiling+1` boundary called out
    // in the agenda.)
    let actions = run_with_failed_alloc_and_attempts(RESTART_BACKOFF_CEILING + 1);
    assert_eq!(
        actions.len(),
        2,
        "attempts > ceiling must emit FinalizeFailed + bridge EnqueueEvaluation per UI-06; got {actions:?}",
    );
    assert!(
        matches!(actions[0], Action::FinalizeFailed { .. }),
        "first action must be FinalizeFailed; got {:?}",
        actions[0],
    );
}

fn run_with_failed_alloc_and_attempts(attempts: u32) -> Vec<Action> {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated),
    );
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), attempts);
    let view = WorkloadLifecycleView {
        restart_counts,
        last_failure_seen_at: BTreeMap::new(),
        released_for_terminal: ::std::collections::BTreeSet::new(),
    };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
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
        2,
        "now_unix == seen_at + backoff must emit RestartAllocation + bridge \
         EnqueueEvaluation per UI-06 (backoff elapsed); got {actions:?}",
    );
    assert!(
        matches!(
            actions[0],
            Action::RestartAllocation {
                kind: overdrive_core::aggregate::WorkloadKind::Service,
                ..
            }
        ),
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
        2,
        "now_unix > seen_at + backoff must emit RestartAllocation + bridge \
         EnqueueEvaluation per UI-06 (backoff elapsed); got {actions:?}",
    );
}

fn run_with_failed_alloc_and_seen_at(now_unix: UnixInstant, seen_at: UnixInstant) -> Vec<Action> {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated),
    );
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(aid("alloc-payments-0"), seen_at);
    // attempts=0 → ceiling check passes AND backoff_for_attempt(0)
    // = RESTART_BACKOFF_DURATION; backoff window is the gating
    // decision under test.
    let view = WorkloadLifecycleView {
        restart_counts: BTreeMap::new(),
        last_failure_seen_at,
        released_for_terminal: ::std::collections::BTreeSet::new(),
    };
    let tick = fresh_tick(Instant::now(), now_unix);

    let r = WorkloadLifecycle::canonical();
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    // view is empty — fresh failure, no prior restart bookkeeping.
    let view = WorkloadLifecycleView::default();
    let now = Instant::now();
    let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick = tick_at_unix(now, now_unix, 0);

    let r = WorkloadLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    // RestartAllocation emitted for the failed alloc.
    assert_eq!(
        actions.len(),
        2,
        "fresh failure must emit RestartAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };

    // Tick 1: fresh failure. Capture next_view as the input to tick 2.
    let view_1 = WorkloadLifecycleView::default();
    let now_1 = Instant::now();
    let now_unix_1 = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick_1 = tick_at_unix(now_1, now_unix_1, 0);

    let r = WorkloadLifecycle::canonical();
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };

    // Tick 1: fresh failure.
    let view_1 = WorkloadLifecycleView::default();
    let now_1 = Instant::now();
    let now_unix_1 = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let tick_1 = tick_at_unix(now_1, now_unix_1, 0);

    let r = WorkloadLifecycle::canonical();
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
        2,
        "tick 2 after backoff elapsed must emit one RestartAllocation + bridge EnqueueEvaluation per UI-06; got {actions_2:?}",
    );
    assert!(
        matches!(
            actions_2[0],
            Action::RestartAllocation {
                kind: overdrive_core::aggregate::WorkloadKind::Service,
                ..
            }
        ),
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
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
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
    let view = WorkloadLifecycleView {
        restart_counts,
        last_failure_seen_at,
        released_for_terminal: ::std::collections::BTreeSet::new(),
    };
    let tick = tick_at_unix(now, now_unix, 0);

    let r = WorkloadLifecycle::canonical();
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

// -------------------------------------------------------------------
// GH #149 — `is_operator_stopped` must check `row.terminal` (ADR-0037 §4)
// -------------------------------------------------------------------
//
// The action shim writes operator attribution to `row.terminal`, NOT
// `row.reason`. `is_operator_stopped` previously only matched on
// `row.reason`, so action-shim-produced Stop rows were invisible to
// the operator-stop guard — the Run branch would emit a fresh
// `StartAllocation` for an operator-stopped allocation, undoing the
// operator's stop intent.

/// Regression for GH #149: an alloc whose `terminal` field carries
/// `Stopped { by: Operator }` (action-shim shape per ADR-0037 §4)
/// must be recognised by `is_operator_stopped` even when `reason`
/// carries `Stopped { by: Reconciler }` (the action shim's hard-coded
/// reason). The Run branch must return empty actions — no
/// `StartAllocation` for an operator-stopped allocation.
#[test]
fn run_branch_blocked_when_alloc_has_terminal_operator_stop() {
    let nodes = one_node_map("local");

    // Build an AllocStatusRow mimicking the action-shim output:
    //   reason:   Stopped { by: Reconciler }  — action shim hard-codes this
    //   terminal: Stopped { by: Operator }    — threaded from the action
    let mut row = alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated);
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Reconciler });
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });

    let allocations = one_alloc_map("alloc-payments-0", row);

    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "Run branch must emit nothing for an operator-stopped alloc \
         (terminal: Stopped {{ by: Operator }}); got {actions:?}",
    );
}

// -------------------------------------------------------------------
// GH #148 / ADR-0037 Amendment 2026-05-14 — GC arm: `desired.job ==
// None` with stale Running allocs emits one
// `Action::StopAllocation { terminal: Some(Stopped { by: SystemGc }) }`
// per orphan Running row.
// -------------------------------------------------------------------
//
// Mechanism (per `docs/feature/workload-gc-absent-stale-allocs/design/
// architecture.md` § 4 Option A): the WorkloadLifecycle reconciler's
// `None` arm — previously a no-op pass-through — is the GC branch.
// When `desired.job == None` (hard-delete, multi-node drain,
// crash-recovery surgery) the arm withdraws any non-terminal
// allocations by stamping a system-GC terminal claim. Structural
// mirror of the operator-Stop branch (`reconciler.rs:1180-1205`):
// iterate Running rows, emit one StopAllocation per row, clear
// `last_failure_seen_at` when no work remains.
//
// Filter shape: the GC arm filters `state == Running` exactly like
// the Stop branch. A `Pending` row has no driver-side runtime to
// stop; a `Draining` row is already being torn down by the worker
// (architecture.md § 8 Open Q3).
//
// Kind agnosticism: the arm branches on `desired.job.is_none()`, NOT
// on workload kind. Tests parametrise over `WorkloadKind` ∈ { Service,
// Job, Schedule } to confirm the body does not branch on kind
// (architecture.md § 8 Open Q2).
//
// Mutation-killability:
//   - `Running` → `Terminated` in the filter: test (a) fails (zero
//     stops emitted instead of N).
//   - `is_empty()` → `is_empty().not()` in view-cleanup: tests (b)+(c)
//     fail (`last_failure_seen_at` not cleared when steady-state).
//   - `StoppedBy::SystemGc` → `StoppedBy::Operator`: test (a) fails
//     (terminal mismatch on the stop action).

/// All `WorkloadKind` variants. Used to parametrise the GC-arm tests
/// over kinds; the GC arm body MUST NOT branch on kind. If a future
/// kind is added, this slice is extended in the same commit and every
/// test below picks it up automatically.
const ALL_WORKLOAD_KINDS: &[WorkloadKind] =
    &[WorkloadKind::Service, WorkloadKind::Job, WorkloadKind::Schedule];

/// (a) `desired.job == None` AND `actual.allocations` contains N>=1
/// `Running` rows produces N `Action::StopAllocation` whose
/// `terminal == Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })`,
/// one per row, indexed by `alloc_id` (asserted on the multiset, not
/// list order).
#[test]
fn absent_workload_with_running_rows_emits_system_gc_stops() {
    for kind in ALL_WORKLOAD_KINDS {
        // Three Running rows: makes the multiset assertion non-trivial.
        let nodes = one_node_map("local");
        let mut allocs = BTreeMap::new();
        for i in 0..3 {
            let aid_str = format!("alloc-payments-{i}");
            let mut row = alloc_with_state(&aid_str, "payments", "local", AllocState::Running);
            row.kind = *kind;
            allocs.insert(aid(&aid_str), row);
        }

        let desired = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes,
            allocations: allocs,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        // Multiset assertion: collect (alloc_id, terminal) pairs from
        // emitted actions, compare to expected — order-independent.
        assert_eq!(
            actions.len(),
            4,
            "kind={kind:?}: expected one StopAllocation per Running row + bridge EnqueueEvaluation per UI-06; got {actions:?}",
        );
        let expected_terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc });
        let mut emitted: Vec<(String, Option<TerminalCondition>)> = actions
            .iter()
            .filter_map(|a| match a {
                Action::StopAllocation { alloc_id, terminal } => {
                    Some((alloc_id.as_str().to_owned(), terminal.clone()))
                }
                Action::EnqueueEvaluation { .. } => None, // Per UI-06: bridge re-enqueue paired with the Stop actions
                other => panic!(
                    "kind={kind:?}: expected StopAllocation or EnqueueEvaluation, got {other:?}"
                ),
            })
            .collect();
        emitted.sort_by(|l, r| l.0.cmp(&r.0));
        let expected: Vec<(String, Option<TerminalCondition>)> =
            (0..3).map(|i| (format!("alloc-payments-{i}"), expected_terminal.clone())).collect();
        assert_eq!(
            emitted, expected,
            "kind={kind:?}: emitted (alloc_id, terminal) multiset must match \
             one StopAllocation per Running row stamped with SystemGc",
        );
    }
}

/// (b) `desired.job == None` AND `actual.allocations` is empty
/// produces zero actions AND `next_view.last_failure_seen_at` is
/// cleared. The cleared map is the load-bearing assertion that the
/// GC arm signals "no pending work" to `view_has_backoff_pending`,
/// matching the Stop branch's view-cleanup shape.
#[test]
fn absent_workload_with_no_allocs_clears_view_backoff() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let desired = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes,
            allocations: empty_alloc_map(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };

        // Seed the view: a stale `last_failure_seen_at` carried over
        // from a prior tick (e.g. the workload had a Failed alloc
        // mid-backoff before the intent was hard-deleted). Without
        // the clear, `view_has_backoff_pending` keeps the broker
        // re-enqueueing this target indefinitely.
        let now = Instant::now();
        let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
        let mut restart_counts = BTreeMap::new();
        restart_counts.insert(aid("alloc-payments-0"), 1);
        let mut last_failure_seen_at = BTreeMap::new();
        last_failure_seen_at.insert(aid("alloc-payments-0"), now_unix);
        let view = WorkloadLifecycleView {
            restart_counts,
            last_failure_seen_at,
            released_for_terminal: BTreeSet::new(),
        };
        let tick = tick_at_unix(now, now_unix, 0);

        let r = WorkloadLifecycle::canonical();
        let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

        assert!(
            actions.is_empty(),
            "kind={kind:?}: absent workload with no allocs must emit no actions; \
             got {actions:?}",
        );
        assert!(
            next_view.last_failure_seen_at.is_empty(),
            "kind={kind:?}: GC arm must clear last_failure_seen_at when no work \
             remains, otherwise view_has_backoff_pending self-re-enqueues. \
             Got last_failure_seen_at = {:?}",
            next_view.last_failure_seen_at,
        );
    }
}

/// (c) `desired.job == None` AND every row already terminal
/// (`Terminated`/`Failed`) produces zero actions AND
/// `next_view.last_failure_seen_at` is cleared. This is the
/// idempotency / steady-state contract: re-ticking after the GC arm
/// has converged emits no further actions.
#[test]
fn absent_workload_with_only_terminal_allocs_is_idempotent() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut allocs = BTreeMap::new();
        let mut terminated =
            alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated);
        terminated.kind = *kind;
        allocs.insert(aid("alloc-payments-0"), terminated);
        let mut failed =
            alloc_with_state("alloc-payments-1", "payments", "local", AllocState::Failed);
        failed.kind = *kind;
        allocs.insert(aid("alloc-payments-1"), failed);

        let desired = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes,
            allocations: allocs,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };

        let now = Instant::now();
        let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
        let mut last_failure_seen_at = BTreeMap::new();
        last_failure_seen_at.insert(aid("alloc-payments-1"), now_unix);
        let view = WorkloadLifecycleView {
            restart_counts: BTreeMap::new(),
            last_failure_seen_at,
            released_for_terminal: BTreeSet::new(),
        };
        let tick = tick_at_unix(now, now_unix, 0);

        let r = WorkloadLifecycle::canonical();
        let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

        assert!(
            actions.is_empty(),
            "kind={kind:?}: only-terminal rows must emit zero actions; got {actions:?}",
        );
        assert!(
            next_view.last_failure_seen_at.is_empty(),
            "kind={kind:?}: GC arm must clear last_failure_seen_at on steady-state \
             tick. Got last_failure_seen_at = {:?}",
            next_view.last_failure_seen_at,
        );
    }
}

/// (d) Mixed states: orphan workload with `[Pending, Running, Draining,
/// Terminated]` rows emits exactly ONE `StopAllocation` (for the Running
/// row) — zero for the others. Pins the filter-shape decision
/// (architecture.md § 8 Open Q3): only Running rows are stopped,
/// matching the operator-Stop branch's filter.
#[test]
fn absent_workload_mixed_states_only_stops_running_rows() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut allocs = BTreeMap::new();
        for (i, state) in
            [AllocState::Pending, AllocState::Running, AllocState::Draining, AllocState::Terminated]
                .iter()
                .enumerate()
        {
            let aid_str = format!("alloc-payments-{i}");
            let mut row = alloc_with_state(&aid_str, "payments", "local", *state);
            row.kind = *kind;
            allocs.insert(aid(&aid_str), row);
        }

        let desired = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("absent"),
            job: None,
            desired_to_stop: false,
            nodes,
            allocations: allocs,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        // Exactly ONE StopAllocation, naming the Running alloc
        // (alloc-payments-1 — index 1 in the state list above).
        assert_eq!(
            actions.len(),
            2,
            "kind={kind:?}: mixed states must emit exactly one StopAllocation \
             (the Running row) + bridge EnqueueEvaluation per UI-06; got {actions:?}",
        );
        match &actions[0] {
            Action::StopAllocation { alloc_id, terminal } => {
                assert_eq!(
                    alloc_id.as_str(),
                    "alloc-payments-1",
                    "kind={kind:?}: must stop the Running row, not Pending/Draining/Terminated",
                );
                assert_eq!(
                    terminal,
                    &Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }),
                    "kind={kind:?}: GC stop must carry SystemGc terminal",
                );
            }
            other => panic!("kind={kind:?}: expected StopAllocation, got {other:?}"),
        }
    }
}

// -------------------------------------------------------------------
// Step 01-04 — `is_intentionally_stopped` Run-branch filter
// -------------------------------------------------------------------
//
// These four tests parametrise over `WorkloadKind ∈ {Service, Job,
// Schedule}` and pin the symmetric `Operator OR SystemGc` semantics
// of the Run-branch's intentional-stop class. The asymmetry against
// `is_operator_stopped` is load-bearing: Operator-stop short-circuits
// the entire Run branch (operator's intent overrides re-submit);
// SystemGc-stop is filtered out of `active_allocs_vec` so that
// resubmit lands a fresh placement (the operator's new intent IS the
// override).
//
// Mutation-killability targets:
//   - A mutant defining `is_intentionally_stopped` as
//     `is_operator_stopped(row)` (forgets the SystemGc arm) fails (e).
//   - A mutant flipping the Operator vs SystemGc precedence in (g)
//     fails (g).
//   - A mutant broadening the filter to all-terminal (allowing Failed
//     rows to be filtered out of restart candidacy) fails (h).
//   - The fresh-id derivation in (e) — distinct alloc_id from the
//     SystemGc-stopped row — guards against `mint_alloc_id`
//     regressing to a workload-id-only deterministic form.

/// Build an `AllocStatusRow` already in the SystemGc-Terminated
/// shape (state=Terminated, terminal=Some(Stopped { by: SystemGc })).
/// Pure helper to keep the four tests below readable.
fn alloc_system_gc_stopped(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    let mut row = alloc_with_state(alloc_id, workload_id, node_id, AllocState::Terminated);
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc });
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::SystemGc });
    row
}

/// Build an `AllocStatusRow` already in the Operator-Terminated shape
/// (state=Terminated, terminal=Some(Stopped { by: Operator })).
fn alloc_operator_stopped(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    let mut row = alloc_with_state(alloc_id, workload_id, node_id, AllocState::Terminated);
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
    row
}

/// (e) Run-branch with intent present + one SystemGc-Terminated row +
/// zero Running rows → emits exactly one fresh `Action::StartAllocation`
/// with a NEW alloc_id (distinct from the SystemGc-stopped row's
/// alloc_id). This is the architecture.md § 5 promise: a resubmit
/// lands a fresh allocation.
#[test]
fn run_branch_with_system_gc_row_only_places_fresh_alloc() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut row = alloc_system_gc_stopped("alloc-payments-0", "payments", "local");
        row.kind = *kind;
        let allocations = one_alloc_map("alloc-payments-0", row);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        assert_eq!(
            actions.len(),
            2,
            "kind={kind:?}: SystemGc-Terminated row + intent present must emit \
             exactly one fresh placement + bridge EnqueueEvaluation per UI-06; got {actions:?}",
        );
        match &actions[0] {
            Action::StartAllocation { alloc_id, .. } => {
                assert_ne!(
                    alloc_id.as_str(),
                    "alloc-payments-0",
                    "kind={kind:?}: fresh placement MUST mint a NEW alloc_id distinct \
                     from the SystemGc-stopped row's alloc_id; reusing the prior id \
                     would let the action shim's LWW write overwrite the SystemGc \
                     terminal stamp on the obs row, violating \
                     resubmit.preserves_prior_gc_terminal (architecture.md § 7)",
                );
            }
            other => {
                panic!("kind={kind:?}: expected StartAllocation (fresh placement), got {other:?}")
            }
        }
    }
}

/// (f) Run-branch with intent present + one Operator-Terminated row →
/// emits zero actions. Preserves the existing operator-stop short-
/// circuit behaviour — regression guard against the new helper
/// accidentally widening the Run-branch short-circuit beyond its
/// narrower `is_operator_stopped` predicate.
#[test]
fn run_branch_with_operator_stopped_row_short_circuits_to_zero_actions() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut row = alloc_operator_stopped("alloc-payments-0", "payments", "local");
        row.kind = *kind;
        let allocations = one_alloc_map("alloc-payments-0", row);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        assert!(
            actions.is_empty(),
            "kind={kind:?}: Operator-Terminated row + intent present must short-circuit \
             to zero actions (operator stop overrides re-submit); got {actions:?}",
        );
    }
}

/// (g) Run-branch with intent present + one SystemGc-Terminated row +
/// one Operator-Terminated row → emits zero actions (operator-stop
/// short-circuit takes precedence; without the Operator row, the
/// SystemGc-only case would have placed fresh per test (e)).
/// Documents the asymmetry: Operator stop is the more-specific
/// override.
#[test]
fn run_branch_operator_stop_takes_precedence_over_system_gc() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut allocs = BTreeMap::new();
        let mut sys_gc = alloc_system_gc_stopped("alloc-payments-0", "payments", "local");
        sys_gc.kind = *kind;
        allocs.insert(aid("alloc-payments-0"), sys_gc);
        let mut op_stop = alloc_operator_stopped("alloc-payments-1", "payments", "local");
        op_stop.kind = *kind;
        allocs.insert(aid("alloc-payments-1"), op_stop);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations: allocs,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        assert!(
            actions.is_empty(),
            "kind={kind:?}: Operator-Terminated row coexisting with SystemGc-Terminated \
             row must short-circuit to zero actions — operator stop overrides re-submit \
             AND overrides the SystemGc-only fresh-placement path; got {actions:?}",
        );
    }
}

/// (h) Run-branch with intent present + one SystemGc-Terminated row +
/// one Failed row → emits a `RestartAllocation` for the Failed row
/// (the SystemGc row is filtered out of restart candidacy; the Failed
/// row drives the restart). Documents the asymmetry between
/// intentional-stop (Operator/SystemGc) and natural-failure (Failed).
#[test]
fn run_branch_system_gc_row_excluded_failed_row_drives_restart() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut allocs = BTreeMap::new();
        let mut sys_gc = alloc_system_gc_stopped("alloc-payments-0", "payments", "local");
        sys_gc.kind = *kind;
        allocs.insert(aid("alloc-payments-0"), sys_gc);
        let mut failed =
            alloc_with_state("alloc-payments-1", "payments", "local", AllocState::Failed);
        failed.kind = *kind;
        allocs.insert(aid("alloc-payments-1"), failed);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations: allocs,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        // Job-kind hits the Job-natural-exit handler before reaching
        // the restart branch — for a Job kind, a Failed row is a
        // natural-exit and emits FinalizeFailed. Service/Schedule kinds
        // skip the Job-natural-exit branch and emit RestartAllocation.
        // Both shapes prove the asymmetry: the SystemGc row is NEVER
        // the action target (no Stop or other action emitted against
        // alloc-payments-0).
        assert_eq!(
            actions.len(),
            2,
            "kind={kind:?}: SystemGc row + Failed row must emit exactly one action \
             (against the Failed row, not the SystemGc row) + bridge EnqueueEvaluation per UI-06; got {actions:?}",
        );
        let action_alloc_id = match &actions[0] {
            Action::RestartAllocation { alloc_id, .. }
            | Action::FinalizeFailed { alloc_id, .. } => alloc_id.as_str(),
            other => panic!(
                "kind={kind:?}: expected RestartAllocation or FinalizeFailed against \
                 Failed row, got {other:?}"
            ),
        };
        assert_eq!(
            action_alloc_id, "alloc-payments-1",
            "kind={kind:?}: action MUST target the Failed row (`alloc-payments-1`), \
             NOT the SystemGc-stopped row (`alloc-payments-0`). The SystemGc row is \
             filtered out of restart/natural-exit candidacy by the \
             `active_allocs_vec` filter; the Failed row drives the action.",
        );
    }
}

// -------------------------------------------------------------------
// Step 01-05: kill 3 missed mutants on `is_intentionally_stopped` and
// `is_natural_exit` (crates/overdrive-core/src/reconciler.rs lines 1713,
// 1745, 1746). Per CLAUDE.md "missed mutations are actionable, not
// aspirational"; user-approved option 1 on 2026-05-14 (add tests).
//
//   M1) line 1713 — `||` -> `&&` between the `terminal` arm and the
//       `reason` arm of `is_intentionally_stopped`. Existing tests (e)
//       through (h) construct rows with the intentional-stop marker in
//       BOTH `terminal` and `reason` (the canonical post-step-01-02
//       shape — see `alloc_system_gc_stopped` / `alloc_operator_stopped`
//       helpers above), so neither arm differentiates the mutation —
//       a `&&` mutation that requires both arms to fire still passes
//       because both arms DO fire. Test (i) below exercises the
//       `reason`-only path (terminal=None, reason=Some(Stopped{...}))
//       so that under `&&` the row would NOT be filtered → enters
//       natural-exit branch / placement path differently → assertion
//       fails → mutant caught.
//
//   M2) line 1745 — `is_natural_exit -> bool` body replaced with
//       `true`. `active_allocs_vec` filters intentional-stop rows
//       BEFORE the natural-exit check, so for filtered input the
//       helper's `!is_intentionally_stopped(row)` is always true; the
//       only way to differentiate the always-true mutant is to feed a
//       non-Terminated/non-Failed row through `active_allocs_vec`
//       (Pending or Draining) and assert it does NOT trigger the
//       natural-exit FinalizeFailed branch.
//
//   M3) line 1746 — `&&` -> `||` between `terminal_state` and
//       `!is_intentionally_stopped(row)`. Same survival reason as M2:
//       under `||` a Pending row (terminal_state == false) still
//       triggers natural-exit because `!is_intentionally_stopped` is
//       true. Test (j) catches both M2 and M3.
// -------------------------------------------------------------------

/// (i.SystemGc) Run-branch with intent present + one row whose
/// intentional-stop marker (`StoppedBy::SystemGc`) lives in `reason`
/// ONLY (`terminal == None`) → row is filtered out of
/// `active_allocs_vec` and a fresh placement happens. The
/// SystemGc-via-reason row is NEVER the action target.
///
/// Mutation discrimination on the original `||` clause at line 1713
/// (between the `terminal` and `reason` arms of `is_intentionally_
/// stopped`):
///   - Under `||` (correct): EITHER arm matching is sufficient — the
///     `reason: Some(Stopped { by: SystemGc })` arm fires, the row is
///     filtered out of `active_allocs_vec`, fresh placement emits
///     `StartAllocation` with a NEW alloc_id. Test passes.
///   - Under `&&` (mutation): BOTH arms must match — `terminal: None`
///     fails the `terminal` arm, the row is NOT filtered, it enters
///     the natural-exit branch (Job kind) → emits `FinalizeFailed`
///     against the stale row; or for Service/Schedule kinds it
///     enters `is_restartable` (Terminated state + the corrupted
///     `is_intentionally_stopped == false` predicate makes the row
///     restartable) → emits `RestartAllocation` against the stale
///     row. Either action shape fails the "no action targets the
///     stale row" assertion → mutant caught.
///
/// Parametrised over `WorkloadKind` so the path under EVERY kind
/// guards the mutation (the `||` clause is upstream of every kind
/// branch).
///
/// (Cannot be parametrised over `StoppedBy` because Operator has a
/// stricter Run-branch short-circuit that fires regardless of which
/// arm of `is_intentionally_stopped` matches — the Operator case is
/// covered by `intentional_stop_via_reason_only_operator_short_circuits`
/// below.)
#[test]
fn intentional_stop_marker_in_reason_only_filters_row() {
    for kind in ALL_WORKLOAD_KINDS {
        // Construct a row with the intentional-stop marker in
        // `reason` ONLY. `terminal` stays `None` (the discriminator
        // for the `||` arm in `is_intentionally_stopped`). State is
        // Terminated so the row IS in a terminal lifecycle state —
        // distinguishing the `||`/`&&` mutation, not a state-machine
        // question.
        let nodes = one_node_map("local");
        let mut row =
            alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated);
        row.kind = *kind;
        row.terminal = None;
        row.reason = Some(TransitionReason::Stopped { by: StoppedBy::SystemGc });
        let allocations = one_alloc_map("alloc-payments-0", row);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        // Load-bearing assertion: the stale SystemGc-via-reason row
        // is NEVER the target of any emitted action. Under the `&&`
        // mutation (M1) the row is NOT filtered → enters
        // natural-exit (Job kind) and emits FinalizeFailed against
        // alloc-payments-0 → assertion fails → mutant caught. For
        // Service/Schedule kinds the same row would be picked up by
        // the `is_restartable` branch (Terminated + reason-only stop
        // is restartable under `&&`, since the helper incorrectly
        // returns false) and emit RestartAllocation against
        // alloc-payments-0 — same assertion catches it.
        for action in &actions {
            let target_id = match action {
                Action::StopAllocation { alloc_id, .. }
                | Action::RestartAllocation { alloc_id, .. }
                | Action::FinalizeFailed { alloc_id, .. }
                | Action::StartAllocation { alloc_id, .. } => Some(alloc_id.as_str()),
                _ => None,
            };
            assert_ne!(
                target_id,
                Some("alloc-payments-0"),
                "kind={kind:?}: the stale SystemGc-via-reason row (terminal=None, \
                 reason=Some(Stopped{{by: SystemGc}})) MUST be filtered out of \
                 `active_allocs_vec` by the `||` arm in `is_intentionally_stopped` \
                 (reconciler.rs:1713). Under the `&&` mutation the row is NOT \
                 filtered → enters natural-exit / restart-budget paths and becomes \
                 the action target. Got action: {action:?}",
            );
        }

        // Mirrors test (e): intent + one filtered intentional-stop
        // row → exactly one fresh `Action::StartAllocation` with a
        // NEW alloc_id (architecture.md § 5 promise). Same assertion
        // shape across kinds — the placement path is kind-agnostic.
        assert_eq!(
            actions.len(),
            2,
            "kind={kind:?}: filtered SystemGc-via-reason row + intent present must \
             emit exactly one fresh placement + bridge EnqueueEvaluation per UI-06; got {actions:?}",
        );
        match &actions[0] {
            Action::StartAllocation { alloc_id, .. } => {
                assert_ne!(
                    alloc_id.as_str(),
                    "alloc-payments-0",
                    "kind={kind:?}: fresh placement MUST mint a NEW alloc_id \
                     distinct from the filtered SystemGc-via-reason row's id",
                );
            }
            other => {
                panic!("kind={kind:?}: expected StartAllocation (fresh placement), got {other:?}")
            }
        }
    }
}

/// (i.Operator) Companion to `intentional_stop_marker_in_reason_only_
/// filters_row`: an Operator-via-reason-only row triggers the Run-
/// branch's stricter `is_operator_stopped` short-circuit (line 1323),
/// emitting zero actions. The Operator short-circuit ALSO uses the
/// `||` clause between `terminal` and `reason` arms (reconciler.rs
/// lines 1660-1673), so this test guards the equivalent mutation
/// surface on `is_operator_stopped` — not directly named in the
/// step's M1/M2/M3 mutant list, but the same code shape.
///
/// Under the `||` clause (correct): the `reason` arm fires, the
/// short-circuit triggers, zero actions. Under `&&` (mutation): the
/// short-circuit does NOT fire (terminal=None fails the terminal
/// arm), control falls through to the natural-exit / restart paths
/// and emits an action against the stale row.
#[test]
fn intentional_stop_via_reason_only_operator_short_circuits() {
    for kind in ALL_WORKLOAD_KINDS {
        let nodes = one_node_map("local");
        let mut row =
            alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated);
        row.kind = *kind;
        row.terminal = None;
        row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
        let allocations = one_alloc_map("alloc-payments-0", row);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: *kind,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        assert!(
            actions.is_empty(),
            "kind={kind:?}: Operator-via-reason-only Terminated row + intent \
             present must short-circuit to zero actions via `is_operator_stopped` \
             (reconciler.rs:1323). The `||` clause in `is_operator_stopped` \
             (lines 1660-1673) makes `reason: Some(Stopped{{by: Operator}})` with \
             `terminal: None` sufficient to trigger the short-circuit; under a `&&` \
             mutation the row would NOT short-circuit and would emit an action. \
             Got: {actions:?}",
        );
    }
}

/// (j) Job-kind Run-branch with intent present + one row whose
/// `state: Pending` (alloc placed but not yet Running, no terminal /
/// reason markers) → the Pending row is in `active_allocs_vec` (it is
/// NOT intentional-stop) but is NOT a natural-exit candidate (Pending
/// is not a terminal state). The Run branch must NOT emit
/// `Action::FinalizeFailed` against the Pending row.
///
/// Mutation discrimination on `is_natural_exit` (lines 1744-1747):
///   - Under correct `terminal_state && !is_intentionally_stopped`
///     (line 1746): a Pending row has `terminal_state == false`
///     (Pending is not Terminated/Failed), so `is_natural_exit`
///     returns false. The Run branch falls through to the
///     restart-budget branch which only matches restartable terminal
///     states, then to fresh placement. No FinalizeFailed.
///   - Under the `is_natural_exit -> true` mutation (M2, line 1745):
///     the Pending row is classified as natural-exit → emits
///     FinalizeFailed against a non-terminal row.
///   - Under the `&&` -> `||` mutation (M3, line 1746): a Pending row
///     has `terminal_state == false` but
///     `!is_intentionally_stopped == true`, so `false || true == true`
///     → also classified as natural-exit → also emits FinalizeFailed.
///
/// Both M2 and M3 fail the same assertion: a Pending row triggers
/// FinalizeFailed under the mutation but not under the correct
/// implementation.
///
/// Mirrors with `state: Draining` (the other non-terminal active
/// state) so both lifecycle-intermediate states are guarded.
#[test]
fn pending_row_does_not_trigger_natural_exit_finalize() {
    // Pending and Draining are the two non-terminal states that flow
    // through `active_allocs_vec` (filtered only by intentional-stop).
    // Both must NOT classify as natural-exit. Job kind is the
    // load-bearing case (the natural-exit branch is gated on
    // `WorkloadKind::Job` at reconciler.rs:1345).
    for state in [AllocState::Pending, AllocState::Draining] {
        let nodes = one_node_map("local");
        let mut row = alloc_with_state("alloc-payments-0", "payments", "local", state);
        row.kind = WorkloadKind::Job;
        // Explicit: no intentional-stop markers. The row is purely
        // mid-lifecycle.
        row.terminal = None;
        row.reason = None;
        let allocations = one_alloc_map("alloc-payments-0", row);

        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
            workload_kind: WorkloadKind::Job,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: WorkloadKind::Job,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();
        let tick =
            fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

        let r = WorkloadLifecycle::canonical();
        let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

        // Load-bearing assertion: NO `Action::FinalizeFailed` is
        // emitted against the Pending/Draining row. Under either
        // mutation (M2: `is_natural_exit -> true`; M3: `&&` -> `||`)
        // the Pending row would be classified as natural-exit and
        // emit FinalizeFailed → assertion fails → both mutants caught.
        for action in &actions {
            if let Action::FinalizeFailed { alloc_id, .. } = action {
                panic!(
                    "state={state:?}: Pending/Draining row MUST NOT trigger \
                     natural-exit FinalizeFailed. The natural-exit branch is gated on \
                     `terminal_state` (Terminated || Failed) AND \
                     `!is_intentionally_stopped` — a Pending/Draining row fails the \
                     `terminal_state` clause and `is_natural_exit` MUST return false. \
                     Under the M2 (always-true body) or M3 (&& -> ||) mutation, the \
                     Pending row is incorrectly classified as natural-exit. \
                     Got FinalizeFailed against alloc_id={alloc_id:?}",
                );
            }
        }
    }
}
