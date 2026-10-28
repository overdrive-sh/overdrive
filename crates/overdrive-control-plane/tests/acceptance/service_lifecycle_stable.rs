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
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
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
        alloc_id,
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code: None,
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
        backend_spiffe: overdrive_core::SpiffeId::new(
            "spiffe://overdrive.local/workload/svc/alloc/x",
        )
        .expect("valid spiffe"),
        backend_ip: std::net::Ipv4Addr::LOCALHOST,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn fact_failed_within_deadline(
    alloc_id: AllocationId,
    started_at_unix_ms: u64,
    exit_code: i32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id,
        state: AllocState::Failed,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code: Some(exit_code),
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
        backend_spiffe: overdrive_core::SpiffeId::new(
            "spiffe://overdrive.local/workload/svc/alloc/x",
        )
        .expect("valid spiffe"),
        backend_ip: std::net::Ipv4Addr::LOCALHOST,
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
/// inserts the alloc into next-View's `stable_announced`.
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
    assert!(
        next_view.stable_announced.contains(&alloc_id),
        "next-View must include alloc in stable_announced"
    );
}

/// S-SHCP-RECON-02 (US-01 / DDD-6 dedup) — once Stable announced
/// for an alloc, a second reconcile tick with unchanged inputs
/// emits zero Stable actions. View's `stable_announced` BTreeSet
/// is the dedup guard.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_stable_already_announced_when_reconcile_then_emits_no_actions() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    let actual = state_with(vec![fact_running_with_pass(alloc_id.clone(), 1000)]);
    let desired = actual.clone();
    let mut view = ServiceLifecycleView::default();
    view.stable_announced.insert(alloc_id.clone());
    let tick = tick_at(2000);

    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert!(actions.is_empty(), "Stable must NOT re-emit once announced; got {actions:?}");
    assert!(next_view.stable_announced.contains(&alloc_id));
}

/// S-SHCP-RECON-04 (US-08 / K1 — closes RCA-A) — alloc Failed
/// terminal row arrives within startup_deadline AND no Pass probe
/// result yet → reconciler emits `Action::FinalizeFailed { terminal:
/// Some(ServiceFailed { reason: EarlyExit { exit_code } }) }`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_alloc_exits_within_deadline_no_pass_probe_when_reconcile_then_emits_failed_early_exit() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("coinflip-0");
    let actual = state_with(vec![fact_failed_within_deadline(alloc_id.clone(), 1000, 1)]);
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();
    // 30ms after start — well within 60s deadline
    let tick = tick_at(1030);

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { exit_code },
                }),
            ..
        } => {
            assert_eq!(*exit_code, Some(1));
        }
        other => panic!("expected EarlyExit ServiceFailed, got {other:?}"),
    }
}

/// S-SHCP-RECON-05 (US-08 AC — exit after Stable is NOT EarlyExit)
/// — alloc Failed row arrives AFTER Stable announced →
/// reconciler does NOT emit EarlyExit; dedup applies.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_alloc_exits_after_stable_when_reconcile_then_does_not_emit_early_exit() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    // Stable already announced previously
    let mut view = ServiceLifecycleView::default();
    view.stable_announced.insert(alloc_id.clone());
    // Now the alloc dies
    let actual = state_with(vec![fact_failed_within_deadline(alloc_id.clone(), 1000, 1)]);
    let desired = actual.clone();
    let tick = tick_at(1500);

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert!(actions.is_empty(), "exit after Stable must NOT emit EarlyExit; got {actions:?}");
}

/// S-SHCP-RECON-06 (US-08 AC — exit 0 within deadline is still
/// EarlyExit) — alloc exits with code 0 within startup_deadline →
/// reconciler emits `ServiceFailed { reason: EarlyExit { exit_code: 0 } }`
/// (Service kind expects long-lived; exit 0 is failure).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_alloc_exits_zero_within_deadline_when_reconcile_then_emits_failed_early_exit_zero() {
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc("payments-0");
    let actual = state_with(vec![fact_failed_within_deadline(alloc_id.clone(), 1000, 0)]);
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();
    let tick = tick_at(1100);

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { exit_code },
                }),
            ..
        } => {
            assert_eq!(*exit_code, Some(0), "exit 0 within deadline is still EarlyExit");
        }
        other => panic!("expected EarlyExit(0) ServiceFailed, got {other:?}"),
    }
}

/// S-SHCP-RECON-03 (US-01 / K1 sad path) — startup probe never
/// passes within `startup_deadline` AND attempts >= max_attempts
/// → reconciler emits `ServiceFailed { reason: StartupProbeFailed
/// { probe_idx, last_fail, attempts } }`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn given_startup_probe_exhausts_attempts_when_reconcile_then_emits_failed_startup_probe_failed() {
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
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason:
                        ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts },
                }),
            ..
        } => {
            assert_eq!(*probe_idx, 0);
            assert_eq!(last_fail, "connection refused");
            assert_eq!(*attempts, 3);
        }
        other => panic!("expected StartupProbeFailed ServiceFailed, got {other:?}"),
    }
}
