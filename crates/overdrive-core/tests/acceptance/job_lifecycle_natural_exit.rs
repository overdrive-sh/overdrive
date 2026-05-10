//! ADR-0037 Amendment 2026-05-10 / ADR-0047 §1 — `JobLifecycle::reconcile`
//! branches on workload kind for natural-exit terminals.
//!
//! Per `docs/feature/workload-kind-discriminator/deliver/roadmap.json`
//! step 02-04:
//!
//! - On a Job-kind alloc whose terminal observation row arrives with a
//!   clean exit (`state: Terminated`, `reason: Stopped { by: Process }`),
//!   the reconciler emits `Action::FinalizeFailed` carrying
//!   `Some(TerminalCondition::Completed { exit_code: 0 })`.
//! - On a Job-kind alloc whose terminal observation row arrives with a
//!   crash (`state: Failed`, `reason: DriverInternalError { detail: "exit_code=N" }`),
//!   the reconciler emits `Action::FinalizeFailed` carrying
//!   `Some(TerminalCondition::Failed { exit_code: N })`.
//! - Service-kind preserves its existing semantics: a Failed alloc with
//!   restart budget remaining flows through the `RestartAllocation`
//!   branch; only when budget is exhausted does it emit
//!   `FinalizeFailed { BackoffExhausted }`. The natural-exit branch
//!   does NOT fire for Service kind.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, Reconciler, TickContext,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

// -------------------------------------------------------------------
// Fixtures (mirror `job_lifecycle_terminal_decision.rs`)
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

fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// Construct a terminal alloc row representing the shape the
/// `ExitObserver` writes today (`AllocState::Terminated` +
/// `TransitionReason::Stopped { by: Process }`) for a clean exit.
fn alloc_clean_exit(alloc_id: &str, job_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(node_id),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Process }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
    }
}

/// Construct a terminal alloc row representing the shape the
/// `ExitObserver` writes today (`AllocState::Failed` +
/// `TransitionReason::DriverInternalError { detail: "exit_code=N" }`) for
/// a crash with non-zero exit code.
fn alloc_crashed_with_exit(
    alloc_id: &str,
    job_id: &str,
    node_id: &str,
    exit_code: i32,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(node_id),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: Some(TransitionReason::DriverInternalError {
            detail: format!("exit_code={exit_code}"),
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
    }
}

// -------------------------------------------------------------------
// Job-kind natural-exit emission (the canonical AC for 02-04)
// -------------------------------------------------------------------

/// Pending → Running → Terminal exit 0 (clean exit) under Job kind:
/// the reconciler emits `Action::FinalizeFailed` carrying
/// `Some(TerminalCondition::Completed { exit_code: 0 })`. This is the
/// canonical AC for step 02-04.
#[test]
fn job_lifecycle_natural_exit_emits_typed_terminal_unit_completed() {
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations
        .insert(aid("alloc-payments-0"), alloc_clean_exit("alloc-payments-0", "payments", "local"));

    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Job,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        1,
        "Job-kind natural clean exit must emit exactly one FinalizeFailed action; got {actions:?}"
    );
    match &actions[0] {
        Action::FinalizeFailed { alloc_id, terminal } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
            assert_eq!(
                *terminal,
                Some(TerminalCondition::Completed { exit_code: 0 }),
                "Job-kind clean exit must stamp Completed {{ exit_code: 0 }}",
            );
        }
        other => panic!("expected FinalizeFailed for Job-kind clean exit, got {other:?}"),
    }
}

/// Pending → Running → Terminal exit N (non-zero) under Job kind: the
/// reconciler emits `Action::FinalizeFailed` carrying
/// `Some(TerminalCondition::Failed { exit_code: N })`.
#[test]
fn job_lifecycle_natural_exit_emits_typed_terminal_unit_failed() {
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        alloc_crashed_with_exit("alloc-payments-0", "payments", "local", 1),
    );

    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Job,
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        1,
        "Job-kind natural failed exit must emit exactly one FinalizeFailed action; got {actions:?}"
    );
    match &actions[0] {
        Action::FinalizeFailed { alloc_id, terminal } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
            assert_eq!(
                *terminal,
                Some(TerminalCondition::Failed { exit_code: 1 }),
                "Job-kind exit_code=1 must stamp Failed {{ exit_code: 1 }}",
            );
        }
        other => panic!("expected FinalizeFailed for Job-kind crash, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// Service-kind regression guard — existing semantics preserved
// -------------------------------------------------------------------

/// Service-kind preserves its existing semantics: a Failed alloc with
/// restart budget remaining flows through the `RestartAllocation`
/// branch (NOT the natural-exit branch). This test pins the
/// no-regression invariant of step 02-04.
#[test]
fn service_kind_failed_alloc_preserves_restart_branch() {
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations
        .insert(aid("alloc-svc-0"), alloc_crashed_with_exit("alloc-svc-0", "svc", "local", 1));

    let desired = JobLifecycleState {
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
    };
    let actual = JobLifecycleState {
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
    };
    // Budget remaining: attempts == 0 < ceiling.
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-svc-0"), 0);
    let view = JobLifecycleView { restart_counts, last_failure_seen_at: BTreeMap::new() };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions.len(),
        1,
        "Service-kind Failed-with-budget must emit one RestartAllocation; got {actions:?}"
    );
    match &actions[0] {
        Action::RestartAllocation { .. } => {}
        other => panic!("Service-kind regression: expected RestartAllocation, got {other:?}"),
    }
}
