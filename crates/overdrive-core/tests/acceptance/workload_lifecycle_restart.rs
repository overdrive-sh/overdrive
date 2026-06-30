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
use overdrive_core::reconcilers::{
    Action, Reconciler, TickContext, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
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
    }
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
    assert!(
        !actions.iter().any(|a| matches!(a, Action::ReleaseServiceVip { .. })),
        "no Action withdraws intent — the workloads/payments intent is retained; got {actions:?}",
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
        stop_allocations(&actions).is_empty(),
        "R5: NO second StopAllocation for the draining alloc — the prior stop is in flight; \
         got {actions:?}",
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
