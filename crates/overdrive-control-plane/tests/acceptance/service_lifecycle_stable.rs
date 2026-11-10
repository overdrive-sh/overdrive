//! Tier 1 acceptance — `ServiceLifecycleReconciler` Stable /
//! EarlyExit / StartupProbeFailed emission per ADR-0055.
//!
//! Slice 01 (US-01 walking skeleton). Pure-sync reconcile body
//! exercised port-to-port through `Reconciler::reconcile`.
//!
//! Per ADR-0055 § 3 + DDD-5: `ServiceLifecycleView` carries inputs
//! only. Stable predicate is recomputed every tick. These tests
//! pin the predicate behaviour, NOT the View shape.
//!
//! Per `.claude/rules/development.md` § "Reconciler I/O":
//! `reconcile` is pure sync `(desired, actual, view, tick) →
//! (Vec<Action>, View)`. No `.await` in test bodies.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::redundant_clone,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::field_reassign_with_default
)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

/// Tick context with deterministic synthetic wall-clock.
fn tick_at(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid alloc id")
}

fn fact_running_with_pass(alloc_id: AllocationId, started_at_unix_ms: u64) -> ServiceAllocFact {
    ServiceAllocFact {
        stable_committed: false,
        alloc_id,
        phase: overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
            overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                spiffe: overdrive_core::SpiffeId::new(
                    "spiffe://overdrive.local/workload/svc/alloc/x",
                )
                .expect("valid spiffe"),
                ip: std::net::Ipv4Addr::LOCALHOST,
            },
        ),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        latest_startup_probe: Some(ProbeStatus::Pass),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn fact_failed_after_start(alloc_id: AllocationId, started_at_unix_ms: u64) -> ServiceAllocFact {
    ServiceAllocFact {
        stable_committed: false,
        alloc_id,
        phase: overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning(
            overdrive_reconcilers::service_lifecycle::NotRunningState::new(AllocState::Failed)
                .expect("not Running"),
        ),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        latest_startup_probe: None,
        latest_startup_probe_observed_at: None,
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn state_with(facts: Vec<ServiceAllocFact>) -> ServiceLifecycleState {
    let mut state = ServiceLifecycleState::default();
    for fact in facts {
        let alloc_id = fact.alloc_id.clone();
        let startup = fact.latest_startup_probe.clone();
        state.allocs.insert(alloc_id.clone(), fact);
        if let Some(status) = startup {
            put_startup_row(&mut state, &alloc_id, status, 1, 1, 1);
        }
    }
    state
}

fn put_startup_row(
    state: &mut ServiceLifecycleState,
    alloc_id: &AllocationId,
    status: ProbeStatus,
    run_seq: u64,
    same_status_runs: u32,
    last_observed_at_unix_ms: u64,
) {
    let probe_idx = ProbeIdx::new(0);
    let fact = state.allocs.get_mut(alloc_id).expect("fixture allocation");
    fact.latest_startup_probe = Some(status.clone());
    fact.latest_startup_probe_observed_at =
        Some(UnixInstant::from_unix_duration(Duration::from_millis(last_observed_at_unix_ms)));
    fact.probe_rows.insert(
        (ProbeRole::Startup, probe_idx),
        ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx,
            role: ProbeRole::Startup,
            status,
            last_observed_at_unix_ms,
            inferred: false,
            run_seq: ProbeRunSeq::new(run_seq).expect("fixture run sequence is non-zero"),
            same_status_runs: SameStatusRuns::new(same_status_runs)
                .expect("fixture streak is non-zero"),
        },
    );
}

/// S-SHCP-RECON-01 (US-01 / K1 / DDD-7 AND-of-all) — Service alloc
/// has Running status row AND startup probe #0 has Pass row →
/// reconciler emits `Action::FinalizeFailed { terminal:
/// Some(Stable { settled_in_ms, witness }) }` exactly once AND
/// keeps the allocation in the observed watch until the Stable row commits.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_running_alloc_with_pass_startup_probe_when_reconcile_then_emits_stable_once() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    let actual = state_with(vec![fact_running_with_pass(alloc_id.clone(), 1000)]);
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();
    let tick = tick_at(1500);

    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "exactly one Stable action expected");
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id: emitted,
            terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
        } => {
            assert_eq!(emitted, &alloc_id);
            assert_eq!(*settled_in_ms, 500, "settled_in_ms = now_unix - started_at");
            assert_eq!(witness.probe_idx, 0);
            assert_eq!(witness.role, "startup");
            assert_eq!(witness.mechanic_summary, "tcp 127.0.0.1:8080");
            assert!(witness.inferred);
        }
        other => panic!("expected Stable action, got {other:?}"),
    }
    assert!(next_view.observed.contains(&alloc_id), "the allocation remains observed");
}

/// S-SHCP-RECON-02 (B1/R2-2) — a committed Stable row suppresses a second
/// announcement, independently of the persisted view.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_stable_already_announced_when_reconcile_then_emits_no_actions() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    let mut committed = fact_running_with_pass(alloc_id.clone(), 1000);
    committed.stable_committed = true;
    let actual = state_with(vec![committed]);
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();
    let tick = tick_at(2000);

    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert!(actions.is_empty(), "a committed Stable must NOT re-emit; got {actions:?}");
    assert_eq!(next_view, view, "the committed row needs no view marker");
}

/// S-SHCP-RECON-05 as amended by D6/DG-12: a Failed row after Stable
/// produces no ServiceLifecycle action and closes its terminal watch.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_alloc_exits_after_stable_when_reconcile_then_does_not_emit_early_exit() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    // The failed row itself closes this allocation's terminal watch.
    let view = ServiceLifecycleView::default();
    // Now the alloc dies
    let actual = state_with(vec![fact_failed_after_start(alloc_id.clone(), 1000)]);
    let desired = actual.clone();
    let tick = tick_at(1500);

    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert!(actions.is_empty(), "a terminal allocation emits no action; got {actions:?}");
    assert!(next_view.terminal_announced.contains(&alloc_id), "the terminal watch closes");
}

/// S-SHCP-RECON-03 (US-01 / K1 sad path) — startup probe never
/// passes within `startup_deadline` AND attempts >= max_attempts
/// → reconciler emits `ServiceFailed { reason: StartupProbeFailed
/// { probe_idx, last_fail, attempts } }`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_startup_probe_exhausts_attempts_when_reconcile_then_emits_startup_stop() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("never-binds-0");
    let mut fact = fact_running_with_pass(alloc_id.clone(), 1000);
    fact.latest_startup_probe =
        Some(ProbeStatus::Fail { last_fail_reason: "connection refused".to_string() });
    fact.latest_startup_probe_observed_at =
        Some(UnixInstant::from_unix_duration(Duration::from_millis(1200)));
    fact.max_attempts = 3;
    fact.startup_deadline = Duration::from_millis(100);
    let mut actual = state_with(vec![fact]);
    put_startup_row(
        &mut actual,
        &alloc_id,
        ProbeStatus::Fail { last_fail_reason: "connection refused".to_string() },
        3,
        3,
        1_200,
    );
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();
    // 200ms after start — past 100ms deadline
    let tick = tick_at(1200);

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::StopAllocation {
            alloc_id: stopped,
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        } => assert_eq!(stopped, &alloc_id),
        other => panic!("expected StopAllocation {{ Stopped {{ StartupProbe }} }}, got {other:?}"),
    }
}
