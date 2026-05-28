//! ADR-0037 §4 — `WorkloadLifecycle::reconcile` stamps a typed
//! `TerminalCondition` on the lifecycle-concluding `Action` variants.
//!
//! Per `docs/feature/reconciler-memory-redb/deliver/roadmap.json` step 02-01:
//!
//! - `Action::StopAllocation` and `Action::FinalizeFailed` carry
//!   `terminal: Option<TerminalCondition>`. The reconciler is the
//!   *single source* of every terminal claim — emission sites outside
//!   a reconciler tick (the action-shim heartbeat, the exit observer)
//!   emit `terminal: None`.
//! - When `view.restart_counts` for a Failed/Terminated alloc reaches
//!   `RESTART_BACKOFF_CEILING`, the reconciler emits
//!   `Action::FinalizeFailed { terminal: Some(BackoffExhausted { attempts }) }`.
//! - When `desired.desired_to_stop` is set against a Running alloc,
//!   the emitted `Action::StopAllocation` carries
//!   `terminal: Some(Stopped { by: Operator })` — the by-source is
//!   already known from the desired state.
//! - Every other transition (Pending → Running, Running → Failed with
//!   budget remaining) emits `terminal: None`.
//!
//! `RESTART_BACKOFF_CEILING` is hardcoded WorkloadLifecycle-internal policy
//! per AC#5 — NOT exported as a property-test input. The proptest
//! sweeps `(restart_counts, last_failure_seen_at, desired_to_stop)`
//! against the *fixed* internal ceiling and asserts terminal is
//! deterministic over those inputs.

#![allow(clippy::expect_used)]
// Doc-comment references symbol-shaped tokens (`tick.now`, `tick.now_unix`,
// action names) in plain prose — backticking every occurrence costs more
// readability than it buys.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{
    Action, RESTART_BACKOFF_CEILING, Reconciler, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use proptest::prelude::*;

// -------------------------------------------------------------------
// Fixtures (mirror the shape used in
// `tests/acceptance/workload_lifecycle_reconcile_branches.rs`)
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
        // GAP-1 subsidiary: None on Pending (no Running yet); fixed
        // wall-clock on Running-or-later states. Value arbitrary;
        // this test does not assert on it directly.
        started_at_unix_ms: match state {
            AllocState::Pending => None,
            _ => Some(1_700_000_000_000),
        },
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = make_node(node_id);
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

fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

// -------------------------------------------------------------------
// Scenario tests (AC #2, #3, #4)
// -------------------------------------------------------------------

/// AC#2 — at the deciding tick (`attempts >= RESTART_BACKOFF_CEILING`),
/// the reconciler stamps `Some(BackoffExhausted { attempts })` on the
/// emitted `Action::FinalizeFailed`. Pre-CEILING the same restart
/// branch emits `Action::RestartAllocation` with `terminal: None`.
#[test]
fn workload_lifecycle_stamps_backoff_exhausted_terminal_when_attempts_reach_ceiling() {
    // attempts == ceiling: the reconciler must emit the synthetic
    // FinalizeFailed action carrying terminal Some(BackoffExhausted).
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Failed),
    );
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), RESTART_BACKOFF_CEILING);
    let view = WorkloadLifecycleView {
        restart_counts,
        last_failure_seen_at: BTreeMap::new(),
        released_for_terminal: ::std::collections::BTreeSet::new(),
    };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "at-ceiling must emit FinalizeFailed + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    match &actions[0] {
        Action::FinalizeFailed { terminal, alloc_id } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
            assert_eq!(
                *terminal,
                Some(TerminalCondition::BackoffExhausted { attempts: RESTART_BACKOFF_CEILING }),
                "BackoffExhausted must carry the consumed attempts count",
            );
        }
        other => panic!("expected FinalizeFailed at ceiling, got {other:?}"),
    }
}

/// AC#3 — when an operator-issued stop is in scope (`desired_to_stop`)
/// the emitted `StopAllocation` carries
/// `Some(Stopped { by: StoppedBy::Operator })`. The convergence-to-Stopped
/// follow-through is exercised by the `terminal == Some(...)` assertion
/// on the action — the action shim writes that value onto the row in
/// step 02-02.
#[test]
fn workload_lifecycle_stamps_stopped_terminal_when_operator_stop_converges() {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "stop branch with one Running alloc emits StopAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    match &actions[0] {
        Action::StopAllocation { alloc_id, terminal } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
            assert_eq!(
                *terminal,
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
                "operator-stop StopAllocation must stamp Stopped {{ by: Operator }}",
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }
}

/// AC#4a — Pending → Running (fresh-schedule) emits `StartAllocation`
/// with no terminal claim.
#[test]
fn workload_lifecycle_emits_no_terminal_for_pending_to_running() {
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: empty_alloc_map(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "fresh schedule must emit StartAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    match &actions[0] {
        // StartAllocation is non-terminal by construction — it does
        // not carry a `terminal` field. The mere fact the variant
        // does not include a terminal field IS the structural claim
        // "Pending → Running is never a terminal moment".
        Action::StartAllocation { .. } => {}
        other => panic!("expected StartAllocation, got {other:?}"),
    }
    // Belt-and-braces: scan all actions for any terminal claim.
    for a in &actions {
        assert!(action_terminal(a).is_none(), "fresh schedule must not stamp any terminal: {a:?}");
    }
}

/// AC#4b — Failed-with-budget-remaining: the restart branch fires and
/// emits `RestartAllocation`. By construction `RestartAllocation` is
/// never a terminal moment (the reconciler is going to try again).
#[test]
fn workload_lifecycle_emits_no_terminal_when_failed_with_budget_remaining() {
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Failed),
    );
    let nodes = one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("payments"),
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::default(),
        service_spec_digest: None,
    };
    let mut restart_counts = BTreeMap::new();
    // Budget remaining: attempts < ceiling.
    restart_counts.insert(aid("alloc-payments-0"), 0);
    let view = WorkloadLifecycleView {
        restart_counts,
        last_failure_seen_at: BTreeMap::new(),
        released_for_terminal: ::std::collections::BTreeSet::new(),
    };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        2,
        "Failed-with-budget must emit RestartAllocation + bridge EnqueueEvaluation per UI-06; got {actions:?}"
    );
    match &actions[0] {
        Action::RestartAllocation { .. } => {}
        other => panic!("expected RestartAllocation, got {other:?}"),
    }
    for a in &actions {
        assert!(
            action_terminal(a).is_none(),
            "non-terminal restart must not stamp terminal: {a:?}"
        );
    }
}

// -------------------------------------------------------------------
// Property test (AC #5)
// -------------------------------------------------------------------

// AC#5 — `WorkloadLifecycleTerminalIsPureFunctionOfViewInputs`.
//
// The reconcile's terminal-decision logic depends ONLY on:
// - `view.restart_counts` (max value consumed)
// - `view.last_failure_seen_at`
// - `desired_state.desired_to_stop` (operator signal)
//
// `RESTART_BACKOFF_CEILING` is hardcoded WorkloadLifecycle-internal policy
// (NOT a property-test input); the test sweeps multiple `(attempts,
// last_failure_seen_at, desired_to_stop)` triples against the fixed
// ceiling and asserts terminal is deterministic for fixed inputs.
// Default workspace proptest budget (1024 cases) is fine for a pure
// function over a small input domain — the inner reconcile call is
// in-process and allocation-free under `Bump`-style scratch.
proptest! {
    #[test]
    fn workload_lifecycle_terminal_is_pure_function_of_view_inputs(
        // Sweep across the ceiling boundary plus headroom either side
        // so both the BackoffExhausted (>=) and the budget-remaining
        // (<) branches are exercised.
        attempts in 0_u32..=(RESTART_BACKOFF_CEILING + 3),
        // A wall-clock seen-at sample; affects whether the backoff
        // window has elapsed (and thus whether RestartAllocation
        // actually fires pre-ceiling). Bounded to avoid u64 overflow.
        seen_at_secs in 0_u64..=10_000,
        // Operator stop signal — exercises the Stop branch.
        desired_to_stop: bool,
        // Alloc state — exercises Running (Stop branch ignores
        // non-Running) vs Failed (restart branch).
        state_choice in 0_u8..=2_u8,
    ) {
        // Map the discrete state choice to a concrete AllocState.
        let alloc_state = match state_choice {
            0 => AllocState::Running,
            1 => AllocState::Failed,
            _ => AllocState::Terminated,
        };

        let nodes = one_node_map("local");
        let allocations = one_alloc_map(
            "alloc-payments-0",
            alloc_with_state("alloc-payments-0", "payments", "local", alloc_state),
        );
        let desired = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop,
            nodes: nodes.clone(),
            allocations: BTreeMap::new(),
                    workload_kind: WorkloadKind::default(),
                    service_spec_digest: None,
        };
        let actual = WorkloadLifecycleState {
            workload_id: jid("payments"),
            job: Some(make_job("payments")),
            desired_to_stop: false,
            nodes,
            allocations,
            workload_kind: WorkloadKind::default(),
            service_spec_digest: None,
        };
        let mut restart_counts = BTreeMap::new();
        restart_counts.insert(aid("alloc-payments-0"), attempts);
        let mut last_failure_seen_at = BTreeMap::new();
        last_failure_seen_at.insert(
            aid("alloc-payments-0"),
            UnixInstant::from_unix_duration(Duration::from_secs(seen_at_secs)),
        );
        let view = WorkloadLifecycleView { restart_counts, last_failure_seen_at, released_for_terminal: ::std::collections::BTreeSet::new() };

        // Use a tick well past any seen_at + backoff so the deadline
        // gate never blocks the restart branch — ensures the
        // ceiling-boundary check is exercised regardless of seen_at.
        let now = Instant::now();
        let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(seen_at_secs + 60));
        let tick_a = fresh_tick(now, now_unix);
        let tick_b = fresh_tick(now, now_unix);

        let r = WorkloadLifecycle::canonical();
        let (actions_a, _va) = r.reconcile(&desired, &actual, &view, &tick_a);
        let (actions_b, _vb) = r.reconcile(&desired, &actual, &view, &tick_b);

        // Twin-invocation purity: identical inputs must yield identical
        // terminal claims on every emitted action.
        let terminals_a: Vec<Option<TerminalCondition>> =
            actions_a.iter().map(action_terminal).collect();
        let terminals_b: Vec<Option<TerminalCondition>> =
            actions_b.iter().map(action_terminal).collect();
        prop_assert_eq!(
            &terminals_a,
            &terminals_b,
            "twin-invocation must produce identical terminal claims",
        );

        // Cross-validate the stamping spec against expected shapes:
        //
        // - Operator stop on a Running alloc → StopAllocation with
        //   Some(Stopped { by: Operator }).
        // - Failed/Terminated + attempts >= ceiling → FinalizeFailed
        //   with Some(BackoffExhausted { attempts }).
        // - Otherwise no action carries a terminal claim.
        let any_terminal = terminals_a.iter().any(Option::is_some);

        let operator_stop_active =
            desired_to_stop && matches!(alloc_state, AllocState::Running);
        let backoff_exhausted_branch = !desired_to_stop
            && matches!(alloc_state, AllocState::Failed | AllocState::Terminated)
            && attempts >= RESTART_BACKOFF_CEILING;

        if operator_stop_active {
            // Exactly one StopAllocation with the operator stamp.
            prop_assert!(
                actions_a.iter().any(|a| matches!(
                    a,
                    Action::StopAllocation {
                        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
                        ..
                    },
                )),
                "operator stop on Running alloc must stamp Stopped(Operator); got {:?}",
                actions_a,
            );
        } else if backoff_exhausted_branch {
            // Exactly one FinalizeFailed with the consumed attempts.
            prop_assert!(
                actions_a.iter().any(|a| matches!(
                    a,
                    Action::FinalizeFailed {
                        terminal: Some(TerminalCondition::BackoffExhausted { attempts: a_count }),
                        ..
                    } if *a_count == attempts,
                )),
                "Failed/Terminated at attempts={} >= ceiling must stamp \
                 BackoffExhausted({}); got {:?}",
                attempts,
                attempts,
                actions_a,
            );
        } else {
            // No terminal claim on any emitted action.
            prop_assert!(
                !any_terminal,
                "non-terminal scenario emitted unexpected terminal claim; \
                 attempts={}, desired_to_stop={}, state={:?}, actions={:?}",
                attempts,
                desired_to_stop,
                alloc_state,
                actions_a,
            );
        }
    }
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Project `terminal` off any Action variant that carries one.
/// Variants that do not carry terminal (Noop, HttpCall, StartWorkflow,
/// StartAllocation, RestartAllocation) return `None` — i.e. they make
/// no terminal claim by construction.
fn action_terminal(action: &Action) -> Option<TerminalCondition> {
    match action {
        Action::StopAllocation { terminal, .. } | Action::FinalizeFailed { terminal, .. } => {
            terminal.clone()
        }
        Action::Noop
        | Action::HttpCall { .. }
        | Action::StartWorkflow { .. }
        | Action::StartAllocation { .. }
        | Action::RestartAllocation { .. }
        // phase-2-xdp-service-map (US-08; ADR-0042): the hydrator's
        // typed Action makes no terminal claim per architecture.md
        // § 7 *Failure surface* — service hydration failures land
        // in the `service_hydration_results` observation row, not on
        // `TerminalCondition`, preserving ADR-0037's "every terminal
        // claim has a single typed source" invariant.
        | Action::DataplaneUpdateService { .. }
        // service-vip-allocator step 03-01: ReleaseServiceVip carries
        // no terminal claim — Service VIP release is a follow-on
        // dispatch triggered BY an already-terminal observation row,
        // not a new terminal claim. The reconciler that emits Release
        // is also the writer of the terminal claim via StopAllocation /
        // FinalizeFailed above.
        | Action::ReleaseServiceVip { .. }
        // backend-discovery-bridge-service-reachability step 01-01:
        // WriteServiceBackendRow carries no terminal claim — the
        // bridge writes an observation row tracking workload-side
        // backend membership, orthogonal to the alloc-lifecycle
        // terminal claim that lives on WorkloadLifecycle's
        // StopAllocation / FinalizeFailed.
        | Action::WriteServiceBackendRow { .. }
        // backend-discovery-bridge-service-reachability UI-05:
        // EnqueueEvaluation is a control-plane re-enqueue (carries
        // (reconciler, target) for cross-reconciler handoff). It
        // makes no terminal claim by construction — the emitting
        // reconciler is the source of terminal claims, not the
        // broker dispatch surface.
        | Action::EnqueueEvaluation { .. }
        // ADR-0053 — RegisterLocalBackend / DeregisterLocalBackend
        // are same-host LB primitives consumed by the
        // cgroup_sock_addr program. They make no terminal claim
        // — same rationale as DataplaneUpdateService above.
        | Action::RegisterLocalBackend { .. }
        | Action::DeregisterLocalBackend { .. } => None,
    }
}
