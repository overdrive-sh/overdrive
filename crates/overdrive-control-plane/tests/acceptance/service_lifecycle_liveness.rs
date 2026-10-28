//! Tier 1 acceptance — liveness probe → liveness TERMINATE (ADR-0087 D2).
//!
//! Single restart authority (ADR-0087): `ServiceLifecycle` is demoted to
//! a liveness DETECTOR. On a Running Service alloc whose liveness probe
//! carries a producer-recorded failure streak at `failure_threshold` it emits
//! EXACTLY one `Action::StopAllocation { terminal: Stopped { by: LivenessProbe } }`
//! — it reads NO restart budget and emits neither `RestartAllocation`
//! nor `FinalizeFailed`. Recovery (one Pass below threshold) starts a new
//! producer streak and emits nothing. The restart-vs-finalize decision (and the
//! `ServiceFailed { LivenessProbeFailed }` exhaustion terminal) is
//! `WorkloadLifecycle`'s (the sole restart authority) — covered in
//! `overdrive-core/tests/acceptance/workload_lifecycle_restart.rs`.
//!
//! These scenarios drive the real `ServiceLifecycleReconciler::reconcile`
//! through its driving-port signature and assert on the emitted
//! `Action`s + the next-View act-once slot (the port-exposed observable
//! surface). The exhaustive property coverage of the predicate universe
//! lives co-located with the reconcile logic in
//! `overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::too_many_lines)]
#![allow(
    clippy::doc_markdown,
    reason = "acceptance-test docs name bare API identifiers (StopAllocation, LivenessProbe) in prose; backticking every one is noise in test-doc context"
)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn liveness_fact(
    alloc_id: &str,
    latest_liveness_probe: Option<ProbeStatus>,
    failure_threshold: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: aid(alloc_id),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: None,
        latest_startup_probe_observed_at: None,
        max_attempts: u32::MAX,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new(
            "spiffe://overdrive.local/workload/svc/alloc/x",
        )
        .expect("valid spiffe"),
        backend_ip: std::net::Ipv4Addr::LOCALHOST,
        latest_liveness_probe,
        has_liveness_probe: true,
        liveness_failure_threshold: failure_threshold,
        probe_rows: BTreeMap::new(),
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let alloc_id = f.alloc_id.clone();
    let liveness = f.latest_liveness_probe.clone();
    let mut state = ServiceLifecycleState {
        allocs: BTreeMap::from([(alloc_id.clone(), f)]),
        ..ServiceLifecycleState::default()
    };
    if let Some(status) = liveness {
        put_liveness_row(&mut state, &alloc_id, status, 1, 1);
    }
    state
}

fn put_liveness_row(
    state: &mut ServiceLifecycleState,
    alloc_id: &AllocationId,
    status: ProbeStatus,
    run_seq: u64,
    same_status_runs: u32,
) {
    let probe_idx = ProbeIdx::new(0);
    state.allocs.get_mut(alloc_id).expect("fixture allocation").probe_rows.insert(
        (ProbeRole::Liveness, probe_idx),
        ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx,
            role: ProbeRole::Liveness,
            status,
            last_observed_at_unix_ms: run_seq,
            inferred: false,
            run_seq: ProbeRunSeq::new(run_seq).expect("fixture run sequence is non-zero"),
            same_status_runs: SameStatusRuns::new(same_status_runs)
                .expect("fixture streak is non-zero"),
        },
    );
}

fn tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(10)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// Drive the reconciler across `n` consecutive liveness Fail ticks,
/// threading the View while each input row carries the producer streak.
/// Returns the actions from the
/// LAST tick and the final next-View.
fn run_consecutive_fails(
    alloc_id: &str,
    failure_threshold: u32,
    n: u32,
) -> (Vec<Action>, ServiceLifecycleView) {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let mut last_actions = Vec::new();
    for run in 1..=n {
        let fact = liveness_fact(
            alloc_id,
            Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
            failure_threshold,
        );
        let alloc_id = fact.alloc_id.clone();
        let mut actual = one_alloc_state(fact);
        put_liveness_row(
            &mut actual,
            &alloc_id,
            ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() },
            u64::from(run),
            run,
        );
        let (actions, next) =
            recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick());
        view = next;
        last_actions = actions;
    }
    (last_actions, view)
}

/// S-ROH-A-01 (ADR-0087 D2 / K3) — three consecutive liveness fails on a
/// Running Service alloc (threshold 3) emit EXACTLY one
/// `Action::StopAllocation { terminal: Stopped { by: LivenessProbe } }`
/// within one tick — and NO RestartAllocation, NO FinalizeFailed (no
/// budget read, no restart-vs-finalize decision).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn three_consecutive_liveness_fails_emits_stop_allocation_liveness_probe() {
    let (actions, _view) = run_consecutive_fails("svc-live-0", 3, 3);

    let stops: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).collect();
    assert_eq!(stops.len(), 1, "exactly one StopAllocation at threshold; got {actions:?}");
    match stops[0] {
        Action::StopAllocation { alloc_id, terminal } => {
            assert_eq!(alloc_id.as_str(), "svc-live-0", "stop targets the liveness-failing alloc");
            assert_eq!(
                *terminal,
                Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
                "the liveness terminate carries Stopped {{ by: LivenessProbe }}",
            );
        }
        other => panic!("expected StopAllocation(LivenessProbe), got {other:?}"),
    }
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::RestartAllocation { .. } | Action::FinalizeFailed { .. })),
        "ServiceLifecycle makes no restart-vs-finalize decision on the liveness path; got {actions:?}",
    );
}

/// S-SHCP-RECON-10 (retained) — liveness fails twice (below the
/// threshold of 3) then passes → the producer row starts a new streak and
/// zero StopAllocation is ever emitted.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn liveness_fail_fail_pass_emits_no_terminate() {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let mut all_stops = 0usize;

    let observations = [
        (ProbeStatus::Fail { last_fail_reason: "1".to_string() }, 1),
        (ProbeStatus::Fail { last_fail_reason: "2".to_string() }, 2),
        (ProbeStatus::Pass, 1),
    ];
    for (index, (status, streak)) in observations.into_iter().enumerate() {
        let fact = liveness_fact("svc-rec-0", Some(status.clone()), 3);
        let alloc_id = fact.alloc_id.clone();
        let mut actual = one_alloc_state(fact);
        put_liveness_row(
            &mut actual,
            &alloc_id,
            status,
            u64::try_from(index + 1).expect("small fixture index"),
            streak,
        );
        let (actions, next) =
            recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick());
        all_stops += actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count();
        view = next;
    }

    assert!(view.liveness_last_acted_run.is_empty(), "no row crossed the threshold");
    assert_eq!(all_stops, 0, "fail/fail/pass below threshold emits no StopAllocation");
}

/// S-ROH-A-09 (ADR-0191 D-1) — CONTRACT_SHAPE: bounded-change
/// (emit-only). ServiceLifecycle acts once on a crossed row, does not act
/// again when that row is re-read, and retries on the next crossed row if
/// the allocation is still Running because the prior stop failed.
#[test]
fn liveness_terminate_is_once_per_row_and_retries_on_the_next_crossed_row() {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let threshold: u32 = 3;

    let mut threshold_state = None;
    // Drive to threshold — the threshold-th producer row emits exactly one stop.
    for i in 1..=threshold {
        let fact = liveness_fact(
            "svc-inflight-0",
            Some(ProbeStatus::Fail { last_fail_reason: format!("fail-{i}") }),
            threshold,
        );
        let alloc_id = fact.alloc_id.clone();
        let mut actual = one_alloc_state(fact);
        put_liveness_row(
            &mut actual,
            &alloc_id,
            ProbeStatus::Fail { last_fail_reason: format!("fail-{i}") },
            u64::from(i),
            i,
        );
        let (actions, next) =
            recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick());
        let stops = actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count();
        if i == threshold {
            assert_eq!(
                stops, 1,
                "threshold tick emits exactly one StopAllocation; got {actions:?}"
            );
        } else {
            assert_eq!(stops, 0, "below-threshold tick emits no StopAllocation; got {actions:?}");
        }
        if i == threshold {
            threshold_state = Some(actual);
        }
        view = next;
    }

    // Act-once: the threshold row's run sequence is recorded.
    let key = (aid("svc-inflight-0"), ProbeIdx::new(0));
    assert_eq!(
        view.liveness_last_acted_run.get(&key),
        Some(&ProbeRunSeq::new(3).expect("non-zero run sequence")),
        "the row acted on is recorded",
    );

    // Re-read the same row while the stop is in flight: no duplicate stop.
    let threshold_state = threshold_state.expect("threshold state recorded");
    let (same_row_actions, same_row_view) =
        recon.reconcile(&ServiceLifecycleState::default(), &threshold_state, &view, &tick());
    assert!(
        !same_row_actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "the same crossed row is acted on only once; got {same_row_actions:?}",
    );

    // A newer crossed row retries because the allocation is still Running.
    let fact = liveness_fact(
        "svc-inflight-0",
        Some(ProbeStatus::Fail { last_fail_reason: "fail-inflight".to_string() }),
        threshold,
    );
    let alloc_id = fact.alloc_id.clone();
    let mut next_row_state = one_alloc_state(fact);
    put_liveness_row(
        &mut next_row_state,
        &alloc_id,
        ProbeStatus::Fail { last_fail_reason: "fail-inflight".to_string() },
        4,
        4,
    );
    let (actions, next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &next_row_state,
        &same_row_view,
        &tick(),
    );
    assert_eq!(
        actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count(),
        1,
        "a failed stop is retried on the next crossed row; got {actions:?}",
    );
    assert_eq!(
        next.liveness_last_acted_run.get(&key),
        Some(&ProbeRunSeq::new(4).expect("non-zero run sequence")),
        "the retry row becomes the new act-once input",
    );
}

/// S-ROH-A-09 (ADR-0087 D2, `state == Running` guard) — CONTRACT_SHAPE:
/// bounded-change (emit-only). Once the row leaves Running (the shim's
/// stop landed), the liveness predicate is false BY CONSTRUCTION even
/// with a threshold-crossing producer row. No terminate is emitted.
#[test]
fn liveness_predicate_false_once_row_leaves_running() {
    let recon = ServiceLifecycleReconciler::new();
    let threshold: u32 = 3;
    let mut fact = liveness_fact(
        "svc-terminated-0",
        Some(ProbeStatus::Fail { last_fail_reason: "post-stop fail".to_string() }),
        threshold,
    );
    fact.state = AllocState::Terminated;
    let alloc_id = fact.alloc_id.clone();
    let mut actual = one_alloc_state(fact);
    put_liveness_row(
        &mut actual,
        &alloc_id,
        ProbeStatus::Fail { last_fail_reason: "post-stop fail".to_string() },
        u64::from(threshold),
        threshold,
    );

    let (actions, _next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick(),
    );

    assert!(
        !actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "a non-Running row emits no liveness terminate regardless of probe streak; got {actions:?}",
    );
}

/// S-ROH-A-09 (ADR-0087 D2, clean-counter-after-restart) —
/// CONTRACT_SHAPE: bounded-change (emit-only). A fresh Running alloc
/// (distinct id, no prior act-once entry — the post-restart shape) receives
/// a first-run streak of one, below a threshold > 1, and emits no terminate.
#[test]
fn fresh_alloc_starts_with_first_run_liveness_streak() {
    let recon = ServiceLifecycleReconciler::new();
    let view = ServiceLifecycleView::default(); // no entry for this alloc
    let threshold: u32 = 3;

    let fact = liveness_fact(
        "svc-fresh-0",
        Some(ProbeStatus::Fail { last_fail_reason: "first fail".to_string() }),
        threshold,
    );
    let actual = one_alloc_state(fact);
    let (actions, next) =
        recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick());

    assert!(
        !actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "a fresh alloc's first Fail is below threshold — no terminate; got {actions:?}",
    );
    assert!(next.liveness_last_acted_run.is_empty(), "a below-threshold row is not acted on");
}
