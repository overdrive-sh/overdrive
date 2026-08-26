//! backend-instance-replacement step 01-02 (ADR-0073 § 5) —
//! `WorkloadLifecycle::reconcile` generation gate + current-instance-scoped
//! veto + level-triggered coalescing + the R1-crash regression net.
//!
//! Translated from `docs/feature/backend-instance-replacement/distill/
//! test-scenarios.md` (the S-BIR-* GIVEN/WHEN/THEN SSOT). Every scenario
//! here drives the PURE `reconcile()` (the `Reconciler` driving port) over
//! a constructed `(desired, actual, view, tick)` and asserts ONLY on the
//! returned `(Vec<Action>, NextView)` tuple — never a private View field.
//! The lone exception is S-BIR-CURRENT-ALLOC, an explicitly-sanctioned
//! `@property` proptest on the pure `current_alloc` helper (the roadmap
//! pins it as the pure-fn complement to the reconciler-boundary
//! S-BIR-REGRESSION-NUMERIC).
//!
//! The load-bearing contract (ADR-0073 § 5, item 5):
//!
//! - `restart_pending = view.observed_generation < desired.generation`.
//! - The veto is current-instance-scoped:
//!   `!restart_pending && current_alloc(&allocs).is_some_and(is_operator_stopped)`
//!   — NOT `allocs.iter().any(is_operator_stopped)`.
//! - The placement tick stamps `observed_generation = desired.generation`
//!   (NOT `observed + 1`); the stop tick (R2) and the draining tick (R5)
//!   leave `observed_generation` unchanged.

#![allow(clippy::expect_used)]
// Doc comments reference symbol-shaped tokens in plain prose.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use proptest::prelude::*;

// -------------------------------------------------------------------
// Fixtures (mirror workload_lifecycle_natural_exit.rs / _terminal_decision.rs)
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
        driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = make_node(node_id);
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// A Running alloc row.
fn alloc_running(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(node_id) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A Terminated-by-Operator alloc row (the operator-stop terminal the
/// action shim writes per ADR-0037 §4: `terminal: Stopped { by: Operator }`).
fn alloc_operator_stopped(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
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

/// A Draining alloc row (a stop emitted but not yet Terminated — R5).
fn alloc_draining(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Draining,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A crashed alloc row — `Failed` with a crash reason (NOT
/// `Stopped { by: Operator }`). This is the fresh instance that reached
/// Running then crashed (R1-crash).
fn alloc_crashed(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 3, writer: nid(node_id) },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(1),
            signal: None,
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A liveness-terminated alloc row — `Terminated` with `terminal =
/// Stopped { by: LivenessProbe }` (the row the action-shim writes after a
/// `ServiceLifecycle` liveness `StopAllocation`: `reason = Stopped { by:
/// Reconciler }` hardcoded by the shim, cause travelling on `terminal`
/// per ADR-0087 D3). `WorkloadLifecycle` sees this as restartable and
/// restarts it under its single budget.
fn alloc_liveness_terminated(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 3, writer: nid(node_id) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Reconciler }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A platform-reclaimed alloc row — `Terminated` with `reason = Stopped
/// { by: PlatformReclaimed }` and no `terminal` (the reclamation
/// executor's write shape, brief §105a.5). `is_platform_reclaimed` reads
/// `true` for it, exempting the ceiling CHECK.
fn alloc_platform_reclaimed(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 3, writer: nid(node_id) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A `WorkloadLifecycleView` whose `restart_counts` for `alloc_id` is
/// pre-set to `n` (observed_generation defaults to `desired` in the
/// tests below so `restart_pending` is false and the crash-restart
/// branch is reached).
fn view_with_restart_counts(observed: u64, alloc_id: &str, n: u32) -> WorkloadLifecycleView {
    let mut view = WorkloadLifecycleView { observed_generation: observed, ..Default::default() };
    view.restart_counts.insert(aid(alloc_id), n);
    view
}

fn alloc_map(rows: Vec<AllocStatusRow>) -> BTreeMap<AllocationId, AllocStatusRow> {
    let mut m = BTreeMap::new();
    for r in rows {
        m.insert(r.alloc_id.clone(), r);
    }
    m
}

/// Build a `(desired, actual)` pair for workload `wid` at the given
/// generations, with the given actual alloc rows. `desired.allocations`
/// is empty (the reconciler inspects `actual.allocations`); `desired`
/// carries the desired-run `generation`.
fn states(
    wid: &str,
    desired_generation: u64,
    actual_rows: Vec<AllocStatusRow>,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: jid(wid),
        job: Some(make_job(wid)),
        desired_to_stop: false,
        generation: desired_generation,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid(wid),
        job: Some(make_job(wid)),
        desired_to_stop: false,
        generation: desired_generation,
        nodes,
        allocations: alloc_map(actual_rows),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    (desired, actual)
}

fn view_with_observed(observed: u64) -> WorkloadLifecycleView {
    WorkloadLifecycleView { observed_generation: observed, ..Default::default() }
}

fn run(
    desired: &WorkloadLifecycleState,
    actual: &WorkloadLifecycleState,
    view: &WorkloadLifecycleView,
) -> (Vec<Action>, WorkloadLifecycleView) {
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));
    WorkloadLifecycle::canonical().reconcile(desired, actual, view, &tick)
}

/// Count the `StartAllocation` actions in an action set.
fn start_allocations(actions: &[Action]) -> Vec<&Action> {
    actions.iter().filter(|a| matches!(a, Action::StartAllocation { .. })).collect()
}

/// Count the `StopAllocation` actions in an action set.
fn stop_allocations(actions: &[Action]) -> Vec<&Action> {
    actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).collect()
}

// ===================================================================
// S-BIR-RESTART-STOPPED (R4) — stopped-origin restart places a fresh
// instance, intent retained, observed stamped to desired.
// ===================================================================

#[test]
fn s_bir_restart_stopped_places_fresh_instance_and_stamps() {
    // Given: payments-0 Terminated{Operator}, desired.generation=1,
    // observed=0 (restart_pending).
    let (desired, actual) = states(
        "payments",
        1,
        vec![alloc_operator_stopped("alloc-payments-0", "payments", "local")],
    );
    let view = view_with_observed(0);

    let (actions, next) = run(&desired, &actual, &view);

    let starts = start_allocations(&actions);
    assert_eq!(
        starts.len(),
        1,
        "stopped-origin restart (R4) must place exactly one fresh instance; got {actions:?}",
    );
    match starts[0] {
        Action::StartAllocation { alloc_id, .. } => {
            assert_ne!(
                alloc_id.as_str(),
                "alloc-payments-0",
                "the fresh instance must be a NEW AllocationId (A1 != A2), got {alloc_id:?}",
            );
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-1",
                "mint_alloc_id(attempt = allocs_vec.len() = 1) mints payments-1",
            );
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
    assert_eq!(
        next.observed_generation, 1,
        "the placement tick must stamp observed_generation = desired.generation (1)",
    );
    // Intent-retained contract, pinned across EVERY withdrawal/teardown
    // shape (review-01-02 nitpick): a stopped-origin restart places the
    // fresh instance and withdraws NOTHING. The full action set for this
    // R4 fixture is exactly one `StartAllocation` plus the three
    // documented Service-kind dual-emit enqueues (backend-discovery-bridge
    // + service-lifecycle + svid-lifecycle) — no StopAllocation (the prior
    // instance is already Terminated), no ReleaseServiceVip (intent
    // declared), no FinalizeFailed (this is a placement, not a give-up).
    assert!(
        !actions.iter().any(|a| matches!(
            a,
            Action::StopAllocation { .. }
                | Action::ReleaseServiceVip { .. }
                | Action::FinalizeFailed { .. }
        )),
        "no Action withdraws or tears down intent — the workloads/payments intent is retained \
         (no Stop / Release / Finalize); got {actions:?}",
    );
    assert_eq!(
        stop_allocations(&actions).len(),
        0,
        "the Terminated prior instance is not re-stopped; got {actions:?}",
    );
}

// ===================================================================
// S-BIR-RESTART-RUNNING-STOP (R2) — running-origin restart stops the
// current instance, does NOT stamp observed.
// ===================================================================

#[test]
fn s_bir_restart_running_stop_emits_one_stop_no_stamp() {
    let (desired, actual) =
        states("coinflip", 1, vec![alloc_running("alloc-coinflip-0", "coinflip", "local")]);
    let view = view_with_observed(0);

    let (actions, next) = run(&desired, &actual, &view);

    let stops = stop_allocations(&actions);
    assert_eq!(
        stops.len(),
        1,
        "running-origin restart (R2) must emit exactly one StopAllocation; got {actions:?}",
    );
    match stops[0] {
        Action::StopAllocation { alloc_id, terminal } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-coinflip-0",
                "stop targets the current Running instance"
            );
            assert_eq!(
                *terminal,
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
                "the R2 stop is terminal Stopped {{ by: Operator }}",
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }
    assert!(
        start_allocations(&actions).is_empty(),
        "R2 places nothing this tick — the fresh instance comes after the old one Terminates",
    );
    assert_eq!(
        next.observed_generation, 0,
        "observed_generation MUST be UNCHANGED on the stop tick (R2) — stamping here re-arms \
         the veto before the fresh instance exists, stranding the workload Terminated",
    );
}

// ===================================================================
// S-BIR-RESTART-RUNNING-PLACE (R3) — once the old instance is
// Terminated, place the fresh one and stamp.
// ===================================================================

#[test]
fn s_bir_restart_running_place_places_fresh_and_stamps() {
    // Given: coinflip-0 already stopped (Terminated{Operator}), no Running.
    let (desired, actual) = states(
        "coinflip",
        1,
        vec![alloc_operator_stopped("alloc-coinflip-0", "coinflip", "local")],
    );
    let view = view_with_observed(0);

    let (actions, next) = run(&desired, &actual, &view);

    let starts = start_allocations(&actions);
    assert_eq!(starts.len(), 1, "R3 places exactly one fresh instance; got {actions:?}");
    match starts[0] {
        Action::StartAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-coinflip-1",
                "the fresh coinflip-1 (A1 != A2, new /30)",
            );
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
    assert_eq!(
        next.observed_generation, 1,
        "the placement tick (R3) is the only tick that stamps observed_generation = desired",
    );
}

// ===================================================================
// S-BIR-STOP-ONCE (R5) — no duplicate stop while the old instance drains.
// ===================================================================

#[test]
fn s_bir_stop_once_no_duplicate_stop_while_draining() {
    // Given: coinflip-0 already draining (R2 stop emitted on a prior tick),
    // still restart_pending (observed 0 < desired 1).
    let (desired, actual) =
        states("coinflip", 1, vec![alloc_draining("alloc-coinflip-0", "coinflip", "local")]);
    let view = view_with_observed(0);

    let (actions, next) = run(&desired, &actual, &view);

    assert!(
        actions.is_empty(),
        "R5: the draining old instance is left alone during the replacement — no second \
         StopAllocation (the prior stop is in flight) AND no spurious RestartAllocation that \
         fights the teardown; got {actions:?}",
    );
    assert_eq!(
        next.observed_generation, 0,
        "observed_generation is still unstamped while the old instance drains",
    );
}

// ===================================================================
// S-BIR-COALESCE-PLACE (DDD-10) — two pre-placement restarts place ONE
// instance for the latest generation, stamp observed = desired (=2).
// ===================================================================

#[test]
fn s_bir_coalesce_place_one_instance_stamps_to_latest_generation() {
    // Given: stopped-origin payments, observed=0, two restarts advanced
    // desired to 2 before any placement.
    let (desired, actual) = states(
        "payments",
        2,
        vec![alloc_operator_stopped("alloc-payments-0", "payments", "local")],
    );
    let view = view_with_observed(0);

    let (actions, next) = run(&desired, &actual, &view);

    assert_eq!(
        start_allocations(&actions).len(),
        1,
        "two pre-placement restarts coalesce into exactly ONE placement; got {actions:?}",
    );
    assert_eq!(
        next.observed_generation, 2,
        "stamp is observed = desired (= 2), NOT observed + 1 — the level-triggered coalesce",
    );
}

// ===================================================================
// S-BIR-COALESCE-NO-REPLAY (DDD-10) — after observed == desired, a
// follow-up reconcile emits no second instance, generation never reverses.
// ===================================================================

#[test]
fn s_bir_coalesce_no_replay_after_stamp() {
    // Given: coalesced placement already stamped observed == desired (=2),
    // payments-1 placed (Running).
    let (desired, actual) =
        states("payments", 2, vec![alloc_running("alloc-payments-1", "payments", "local")]);
    let view = view_with_observed(2);

    let (actions, next) = run(&desired, &actual, &view);

    assert!(
        start_allocations(&actions).is_empty(),
        "restart_pending is false (observed == desired) — no further StartAllocation; got {actions:?}",
    );
    assert_eq!(
        next.observed_generation, 2,
        "the generation never goes backwards — observed_generation stays 2",
    );
}

// ===================================================================
// S-BIR-SEQUENTIAL (DDD-10) — a restart after the prior placement
// re-enters the cycle (re-stops the current instance, no stamp).
// ===================================================================

#[test]
fn s_bir_sequential_restart_reenters_cycle() {
    // Given: prior restart placed payments-1 (Running) and stamped
    // observed=1; a second restart advanced desired to 2 (observed 1 < 2).
    let (desired, actual) =
        states("payments", 2, vec![alloc_running("alloc-payments-1", "payments", "local")]);
    let view = view_with_observed(1);

    let (actions, next) = run(&desired, &actual, &view);

    let stops = stop_allocations(&actions);
    assert_eq!(
        stops.len(),
        1,
        "restart_pending (1 < 2) re-enters the cycle: re-stop the current payments-1; got {actions:?}",
    );
    match stops[0] {
        Action::StopAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-1",
                "the re-entry stops the CURRENT instance"
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }
    assert_eq!(
        next.observed_generation, 1,
        "no stamp on this re-entry stop tick (the fresh payments-2 has not been placed)",
    );
}

// ===================================================================
// S-BIR-REGRESSION-STOPPED (R1-crash, DDD-13) — a fresh instance that
// crashed after a stopped-origin restart is crash-restarted, not wedged
// on the stale superseded payments-0/Operator row.
// ===================================================================

#[test]
fn s_bir_regression_stopped_crash_restarts_not_wedged() {
    // Given: payments-1 CRASHED (Failed, crash reason), superseded
    // payments-0 Terminated{Operator} retained, observed == desired
    // (restart_pending false).
    let (desired, actual) = states(
        "payments",
        1,
        vec![
            alloc_operator_stopped("alloc-payments-0", "payments", "local"),
            alloc_crashed("alloc-payments-1", "payments", "local"),
        ],
    );
    let view = view_with_observed(1);

    let (actions, _next) = run(&desired, &actual, &view);

    assert!(
        !actions.is_empty(),
        "the crashed fresh instance MUST converge (crash-restart), NOT wedge on the stale \
         payments-0/Operator row — the buggy any(...) veto returned an empty action set here",
    );
    let restarts: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(
        restarts.len(),
        1,
        "exactly one crash-restart for the current instance; got {actions:?}"
    );
    match restarts[0] {
        Action::RestartAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-1",
                "crash-restart targets the CURRENT (crashed) instance, not the superseded payments-0",
            );
        }
        other => panic!("expected RestartAllocation, got {other:?}"),
    }
}

// ===================================================================
// S-BIR-REGRESSION-NUMERIC (DDD-13, HIGH-3) — the numeric-vs-lexical
// 'current instance' invariant proven AT THE DRIVING PORT (reconcile()),
// double-digit alloc history.
// ===================================================================

#[test]
fn s_bir_regression_numeric_crash_restarts_numeric_max_not_lexical() {
    // Given: crashed CURRENT payments-10 (numeric max) alongside a
    // lexically-later-but-numerically-earlier superseded payments-2
    // Terminated{Operator}, observed == desired (restart_pending false).
    //
    // LEXICALLY, "alloc-payments-2" > "alloc-payments-10" (BTreeMap order),
    // so a lexical-max current_alloc would pick payments-2/Operator, fire
    // the veto, and wedge the crashed payments-10. The numeric-max helper
    // picks payments-10 (the genuine current instance) and crash-restarts it.
    let (desired, actual) = states(
        "payments",
        1,
        vec![
            alloc_operator_stopped("alloc-payments-2", "payments", "local"),
            alloc_crashed("alloc-payments-10", "payments", "local"),
        ],
    );
    let view = view_with_observed(1);

    let (actions, _next) = run(&desired, &actual, &view);

    let restarts: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(
        restarts.len(),
        1,
        "the numeric-max current instance (payments-10) must be crash-restarted, NOT wedged by \
         the lexically-later payments-2/Operator row; got {actions:?}",
    );
    match restarts[0] {
        Action::RestartAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-10",
                "current_alloc selects the NUMERIC max suffix (10), not the lexical max (2)",
            );
        }
        other => panic!("expected RestartAllocation for payments-10, got {other:?}"),
    }
}

// ===================================================================
// S-BIR-REGRESSION-RUNNING (R1-crash, DDD-13) — running-origin variant
// of the crash-restart-not-wedged regression.
// ===================================================================

#[test]
fn s_bir_regression_running_crash_restarts_not_wedged() {
    // Given: running-origin restart cycled coinflip-0 -> fresh coinflip-1
    // reached Running then CRASHED; superseded coinflip-0 Terminated{Operator}
    // retained, restart_pending false.
    let (desired, actual) = states(
        "coinflip",
        1,
        vec![
            alloc_operator_stopped("alloc-coinflip-0", "coinflip", "local"),
            alloc_crashed("alloc-coinflip-1", "coinflip", "local"),
        ],
    );
    let view = view_with_observed(1);

    let (actions, _next) = run(&desired, &actual, &view);

    let restarts: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(
        restarts.len(),
        1,
        "the crashed coinflip-1 must crash-restart, not wedge on the superseded coinflip-0/Operator; \
         got {actions:?}",
    );
    match restarts[0] {
        Action::RestartAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-coinflip-1",
                "crash-restart targets the current instance"
            );
        }
        other => panic!("expected RestartAllocation, got {other:?}"),
    }
}

// ===================================================================
// S-BIR-BUG3-PRESERVED (DDD-7) — the scoped veto STILL fires when the
// CURRENT instance is the operator-stopped one and generations are equal.
// ===================================================================

#[test]
fn s_bir_bug3_preserved_same_spec_deploy_does_not_resurrect() {
    // Given: payments-0 Terminated{Operator} is the CURRENT instance,
    // same-spec deploy did NOT bump (observed == desired, restart_pending false).
    let (desired, actual) = states(
        "payments",
        0,
        vec![alloc_operator_stopped("alloc-payments-0", "payments", "local")],
    );
    let view = view_with_observed(0);

    let (actions, _next) = run(&desired, &actual, &view);

    assert!(
        start_allocations(&actions).is_empty(),
        "the current-instance-scoped veto must FIRE on a CURRENT operator-stop with equal \
         generations — a re-deploy must NOT resurrect an operator-stopped workload; got {actions:?}",
    );
    // Stronger: the veto returns an empty action set entirely (no bridge /
    // svid enqueues either, since no alloc-mutating action fired).
    assert!(
        actions.is_empty(),
        "Bug-3: the scoped veto returns (Vec::new(), view) — the workload stays stopped; got {actions:?}",
    );
}

// ===================================================================
// S-BIR-CURRENT-ALLOC (DDD-13, @property) — the pure current_alloc helper
// picks the numerically-highest suffix, not the lexical max.
//
// `current_alloc` is private to the reconciler module; its observable
// behavior at the driving port is proven by S-BIR-REGRESSION-NUMERIC
// (reconcile() picks payments-10 over the lexical payments-2). This
// proptest is the pure-fn complement the roadmap sanctions — it drives
// the SAME numeric-vs-lexical selection through reconcile()'s observable
// outcome over a generated alloc history, since the helper itself is not
// a public port.
// ===================================================================

// ===================================================================
// ADR-0087 single restart authority — WorkloadLifecycle owns crash AND
// liveness restart under one budget. CONTRACT_SHAPE: bounded-change
// (emit-only reconcile decisions observed at the driving port).
// ===================================================================

/// S-ROH-A-02 — a `Stopped { by: LivenessProbe }` Terminated row with
/// budget remaining is restartable, and WorkloadLifecycle restarts it at
/// its SINGLE existing increment site (crash and liveness share ONE
/// `restart_counts` pool). `RestartAllocation` carries no cause field —
/// the cause is the prior row's observable `Stopped { by: LivenessProbe }`
/// terminal (ADR-0087 D4).
#[test]
fn s_roh_a_02_liveness_terminated_restarts_under_single_budget() {
    let (desired, actual) = states(
        "payments",
        1,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    // Two prior restarts already consumed; observed == desired so the
    // crash-restart branch (not the generation-replace path) is reached.
    let view = view_with_restart_counts(1, "alloc-payments-0", 2);

    let (actions, next) = run(&desired, &actual, &view);

    let restarts: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(
        restarts.len(),
        1,
        "a liveness-terminated row below ceiling is restartable — exactly one RestartAllocation; \
         got {actions:?}",
    );
    match restarts[0] {
        Action::RestartAllocation { alloc_id, .. } => {
            assert_eq!(
                alloc_id.as_str(),
                "alloc-payments-0",
                "restart targets the liveness-killed alloc"
            );
        }
        other => panic!("expected RestartAllocation, got {other:?}"),
    }
    assert_eq!(
        next.restart_counts.get(&aid("alloc-payments-0")).copied(),
        Some(3),
        "the single restart_counts site increments by exactly one (2 -> 3) — crash and liveness \
         share ONE budget",
    );
    assert!(
        !actions.iter().any(|a| matches!(a, Action::FinalizeFailed { .. })),
        "below ceiling the liveness restart never finalises; got {actions:?}",
    );
}

/// S-ROH-A-03 (mutation-gate target) — at the ceiling a liveness loop
/// finalises `ServiceFailed { LivenessProbeFailed { probe_idx: 0,
/// attempts } }`, NOT `BackoffExhausted`; a crash loop on an
/// identically-shaped alloc at the same ceiling finalises
/// `BackoffExhausted { attempts }`. The two are distinguished on the
/// same alloc shape by `is_liveness_killed` (ADR-0087 D4, Hard
/// Constraint 1).
#[test]
fn s_roh_a_03_ceiling_terminal_is_cause_aware_liveness_vs_crash() {
    // Liveness loop at ceiling.
    let (desired_l, actual_l) = states(
        "payments",
        1,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    let view_l = view_with_restart_counts(1, "alloc-payments-0", RESTART_BACKOFF_CEILING);
    let (actions_l, _n) = run(&desired_l, &actual_l, &view_l);

    let liveness_terminal = actions_l.iter().find_map(|a| match a {
        Action::FinalizeFailed { terminal: Some(t), .. } => Some(t),
        _ => None,
    });
    assert_eq!(
        liveness_terminal,
        Some(&TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed {
                probe_idx: 0,
                attempts: RESTART_BACKOFF_CEILING,
            },
        }),
        "a liveness loop at ceiling finalises ServiceFailed(LivenessProbeFailed), NOT \
         BackoffExhausted; got {actions_l:?}",
    );
    assert!(
        !actions_l.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                ..
            }
        )),
        "the liveness loop must never flatten to BackoffExhausted; got {actions_l:?}",
    );

    // Crash loop at ceiling — identical alloc shape, non-liveness terminal.
    let (desired_c, actual_c) =
        states("payments", 1, vec![alloc_crashed("alloc-payments-0", "payments", "local")]);
    let view_c = view_with_restart_counts(1, "alloc-payments-0", RESTART_BACKOFF_CEILING);
    let (actions_c, _n) = run(&desired_c, &actual_c, &view_c);

    let crash_terminal = actions_c.iter().find_map(|a| match a {
        Action::FinalizeFailed { terminal: Some(t), .. } => Some(t),
        _ => None,
    });
    assert_eq!(
        crash_terminal,
        Some(&TerminalCondition::BackoffExhausted { attempts: RESTART_BACKOFF_CEILING }),
        "a crash loop at the same ceiling finalises BackoffExhausted; got {actions_c:?}",
    );
}

/// S-ROH-A-07 (locked per OQ-1) — the stamped `LivenessProbeFailed.attempts`
/// reads the restart-budget consumed (= CEILING), NOT the liveness
/// consecutive-failure streak. Parallels `BackoffExhausted { attempts }`.
#[test]
fn s_roh_a_07_liveness_terminal_attempts_is_restart_budget_consumed() {
    let (desired, actual) = states(
        "payments",
        1,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    let view = view_with_restart_counts(1, "alloc-payments-0", RESTART_BACKOFF_CEILING);
    let (actions, _n) = run(&desired, &actual, &view);

    match actions.iter().find_map(|a| match a {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::LivenessProbeFailed { attempts, .. },
                }),
            ..
        } => Some(*attempts),
        _ => None,
    }) {
        Some(attempts) => assert_eq!(
            attempts, RESTART_BACKOFF_CEILING,
            "attempts is the restart-budget count consumed (= CEILING), not a liveness streak",
        ),
        None => panic!("expected a ServiceFailed(LivenessProbeFailed) terminal; got {actions:?}"),
    }
}

/// S-ROH-A-10 — the ceiling idempotency guard covers BOTH terminal
/// kinds: a row already carrying `ServiceFailed { LivenessProbeFailed }`
/// re-emits NO FinalizeFailed (extends the prior BackoffExhausted-only
/// guard).
#[test]
fn s_roh_a_10_exhaustion_idempotent_across_both_terminal_kinds() {
    let mut row = alloc_liveness_terminated("alloc-payments-0", "payments", "local");
    row.terminal = Some(TerminalCondition::ServiceFailed {
        reason: ServiceFailureReason::LivenessProbeFailed {
            probe_idx: 0,
            attempts: RESTART_BACKOFF_CEILING,
        },
    });
    let (desired, actual) = states("payments", 1, vec![row]);
    let view = view_with_restart_counts(1, "alloc-payments-0", RESTART_BACKOFF_CEILING);

    let (actions, _n) = run(&desired, &actual, &view);

    assert!(
        !actions.iter().any(|a| matches!(a, Action::FinalizeFailed { .. })),
        "an already-finalised LivenessProbeFailed row re-emits no FinalizeFailed; got {actions:?}",
    );
}

/// S-ROH-A-04 — a liveness kill CONSUMES restart budget (crash-class:
/// `by_reclaims_platform(LivenessProbe) == false`) and exhausts at the
/// ceiling; by contrast a platform reclamation is EXEMPT from the
/// ceiling check (`is_platform_reclaimed`) and is re-driven — restarted,
/// not finalised — even at the ceiling.
#[test]
fn s_roh_a_04_liveness_consumes_budget_platform_reclaim_is_exempt() {
    // Liveness at ceiling → finalises (budget consumed).
    let (liveness_desired, liveness_actual) = states(
        "payments",
        1,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    let view = view_with_restart_counts(1, "alloc-payments-0", RESTART_BACKOFF_CEILING);
    let (liveness_out, _n) = run(&liveness_desired, &liveness_actual, &view);
    assert!(
        liveness_out.iter().any(|a| matches!(a, Action::FinalizeFailed { .. })),
        "a liveness kill at ceiling exhausts the budget (crash-class); got {liveness_out:?}",
    );
    assert!(
        !liveness_out.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
        "at ceiling a liveness kill does not restart; got {liveness_out:?}",
    );

    // Platform reclamation at the same ceiling → exempt, re-driven.
    let (reclaim_desired, reclaim_actual) = states(
        "payments",
        1,
        vec![alloc_platform_reclaimed("alloc-payments-0", "payments", "local")],
    );
    let (reclaim_out, _n) = run(&reclaim_desired, &reclaim_actual, &view);
    assert!(
        reclaim_out.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
        "platform reclamation is EXEMPT from the ceiling — it re-drives, not finalises; got {reclaim_out:?}",
    );
    assert!(
        !reclaim_out.iter().any(|a| matches!(a, Action::FinalizeFailed { .. })),
        "a platform reclaim never exhausts the budget; got {reclaim_out:?}",
    );
}

/// S-ROH-A-05 — the intentional-stop discriminator is NOT widened by the
/// new variant: an Operator or SystemGc terminal is NEVER restarted,
/// while a LivenessProbe terminal always IS. Proven at the reconcile
/// port (the predicates are module-private).
#[test]
fn s_roh_a_05_intentional_stop_discriminator_unchanged() {
    let view = view_with_observed(0);

    // Operator stop on the current instance, equal generations → veto
    // (never restarted).
    let (operator_desired, operator_actual) = states(
        "payments",
        0,
        vec![alloc_operator_stopped("alloc-payments-0", "payments", "local")],
    );
    let (operator_out, _n) = run(&operator_desired, &operator_actual, &view);
    assert!(
        !operator_out.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
        "an Operator-stopped alloc is never restarted; got {operator_out:?}",
    );

    // LivenessProbe terminal → restartable (restarted).
    let (killed_desired, killed_actual) = states(
        "payments",
        0,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    let (killed_out, _n) = run(&killed_desired, &killed_actual, &view);
    assert!(
        killed_out.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
        "a LivenessProbe-terminated alloc IS restartable (not an intentional stop); got {killed_out:?}",
    );
}

/// S-ROH-A-08 — budget unification: a crash and a liveness kill on the
/// SAME alloc draw the SAME `restart_counts` pool through the single
/// increment site (the kubelet single-RESTARTS shape). A crash restart
/// bumps the counter, and a subsequent liveness restart bumps the SAME
/// counter — one budget, not two.
#[test]
fn s_roh_a_08_crash_and_liveness_share_one_budget() {
    // Crash restart at counts=2 → counts=3.
    let (crash_desired, crash_actual) =
        states("payments", 1, vec![alloc_crashed("alloc-payments-0", "payments", "local")]);
    let view = view_with_restart_counts(1, "alloc-payments-0", 2);
    let (_crash_out, after_crash) = run(&crash_desired, &crash_actual, &view);
    assert_eq!(
        after_crash.restart_counts.get(&aid("alloc-payments-0")).copied(),
        Some(3),
        "a crash restart increments the shared budget (2 -> 3)",
    );

    // A liveness kill on the same alloc continues the SAME pool → 3 -> 4.
    // Advance the clock past the 1s backoff window the crash restart
    // stamped (last_failure_seen_at) so the restart-emission path is
    // reached — the point under test is the shared counter, not backoff.
    let (liveness_desired, liveness_actual) = states(
        "payments",
        1,
        vec![alloc_liveness_terminated("alloc-payments-0", "payments", "local")],
    );
    let later_tick =
        fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(10)));
    let (_liveness_out, after_liveness) = WorkloadLifecycle::canonical().reconcile(
        &liveness_desired,
        &liveness_actual,
        &after_crash,
        &later_tick,
    );
    assert_eq!(
        after_liveness.restart_counts.get(&aid("alloc-payments-0")).copied(),
        Some(4),
        "a liveness restart draws the SAME budget the crash did (3 -> 4) — one pool, not two",
    );
}

proptest! {
    /// Over an alloc history whose attempt indices span single- and
    /// double-digit suffixes with the CURRENT (numeric-max) instance
    /// crashed and an arbitrary superseded operator-stopped row, the
    /// reconciler crash-restarts the numeric-max instance — never the
    /// lexical max. This is the @property form of the numeric-current
    /// invariant, observed through the reconcile() driving port (the
    /// `current_alloc` helper is module-private).
    #[test]
    fn s_bir_current_alloc_numeric_max_is_crash_restarted(
        // The numeric-max (current) suffix — always >= 10 so it is
        // lexically SMALLER than a single-digit superseded suffix,
        // making the numeric-vs-lexical distinction falsifiable.
        current_suffix in 10_u32..=99,
        // A superseded operator-stopped suffix, single-digit (so it
        // sorts lexically AFTER the double-digit current suffix).
        superseded_suffix in 0_u32..=9,
    ) {
        let current = format!("alloc-payments-{current_suffix}");
        let superseded = format!("alloc-payments-{superseded_suffix}");
        let (desired, actual) = states(
            "payments",
            1,
            vec![
                alloc_operator_stopped(&superseded, "payments", "local"),
                alloc_crashed(&current, "payments", "local"),
            ],
        );
        let view = view_with_observed(1);

        let (actions, _next) = run(&desired, &actual, &view);

        let restarts: Vec<&Action> =
            actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
        prop_assert_eq!(
            restarts.len(),
            1,
            "the numeric-max current instance must be crash-restarted (not wedged by the \
             lexically-later superseded operator-stop); actions={:?}",
            actions
        );
        if let Action::RestartAllocation { alloc_id, .. } = restarts[0] {
            prop_assert_eq!(
                alloc_id.as_str(),
                current.as_str(),
                "current_alloc selects the NUMERIC max suffix, not the lexical max",
            );
        }
    }
}
