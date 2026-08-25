//! Tier 1 acceptance — liveness probe → liveness TERMINATE (ADR-0087 D2).
//!
//! Single restart authority (ADR-0087): `ServiceLifecycle` is demoted to
//! a liveness DETECTOR. On a Running Service alloc whose liveness probe
//! reaches its consecutive-failure `failure_threshold` it emits EXACTLY
//! one `Action::StopAllocation { terminal: Stopped { by: LivenessProbe } }`
//! — it reads NO restart budget and emits neither `RestartAllocation`
//! nor `FinalizeFailed`. Recovery (one Pass below threshold) resets the
//! counter and emits nothing. The restart-vs-finalize decision (and the
//! `ServiceFailed { LivenessProbeFailed }` exhaustion terminal) is
//! `WorkloadLifecycle`'s (the sole restart authority) — covered in
//! `overdrive-core/tests/acceptance/workload_lifecycle_restart.rs`.
//!
//! These scenarios drive the real `ServiceLifecycleReconciler::reconcile`
//! through its driving-port signature and assert on the emitted
//! `Action`s + the next-View counter slot (the port-exposed observable
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
use overdrive_core::observation::{ProbeIdx, ProbeStatus};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

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
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe,
        has_liveness_probe: true,
        liveness_failure_threshold: failure_threshold,
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    allocs.insert(f.alloc_id.clone(), f);
    ServiceLifecycleState { allocs, service_dataplane: None, prior_backend_row_at: None }
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
/// threading the View so the counter accumulates the way it does in
/// production (one increment per tick). Returns the actions from the
/// LAST tick and the final next-View.
fn run_consecutive_fails(
    alloc_id: &str,
    failure_threshold: u32,
    n: u32,
) -> (Vec<Action>, ServiceLifecycleView) {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let mut last_actions = Vec::new();
    for _ in 0..n {
        let fact = liveness_fact(
            alloc_id,
            Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
            failure_threshold,
        );
        let (actions, next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick(),
        );
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
/// threshold of 3) then passes → the next-View consecutive-failure
/// counter resets to 0 and zero StopAllocation is ever emitted.
#[test]
fn liveness_fail_fail_pass_resets_counter_and_emits_no_terminate() {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let mut all_stops = 0usize;

    let observations = [
        Some(ProbeStatus::Fail { last_fail_reason: "1".to_string() }),
        Some(ProbeStatus::Fail { last_fail_reason: "2".to_string() }),
        Some(ProbeStatus::Pass),
    ];
    for obs in observations {
        let fact = liveness_fact("svc-rec-0", obs, 3);
        let (actions, next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick(),
        );
        all_stops +=
            actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count();
        view = next;
    }

    let key = (aid("svc-rec-0"), ProbeIdx::new(0));
    assert_eq!(
        view.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
        0,
        "a Pass below threshold resets the counter to 0",
    );
    assert_eq!(all_stops, 0, "fail/fail/pass below threshold emits no StopAllocation");
}

/// S-ROH-A-09 (ADR-0087 D2, counter reset-on-emit retained) —
/// CONTRACT_SHAPE: bounded-change (emit-only). After ServiceLifecycle
/// emits the liveness terminate at threshold, the consecutive-failure
/// counter is reset. On the IMMEDIATE next Fail tick — the row still
/// Running because the shim's stop is in flight — the counter climbs
/// back from 0 to 1 (below threshold), so NO second StopAllocation is
/// emitted for the same alloc. This is the reset-on-emit half of the
/// double-terminate guard.
#[test]
fn liveness_terminate_idempotent_on_next_tick_while_stop_in_flight() {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let threshold: u32 = 3;

    // Drive to threshold — the threshold-th tick emits exactly one stop.
    for i in 1..=threshold {
        let fact = liveness_fact(
            "svc-inflight-0",
            Some(ProbeStatus::Fail { last_fail_reason: format!("fail-{i}") }),
            threshold,
        );
        let (actions, next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick(),
        );
        let stops =
            actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count();
        if i == threshold {
            assert_eq!(stops, 1, "threshold tick emits exactly one StopAllocation; got {actions:?}");
        } else {
            assert_eq!(stops, 0, "below-threshold tick emits no StopAllocation; got {actions:?}");
        }
        view = next;
    }

    // Reset-on-emit: the counter was cleared at the emit tick.
    let key = (aid("svc-inflight-0"), ProbeIdx::new(0));
    assert_eq!(
        view.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
        0,
        "the counter is reset to 0 on emit (ADR-0087 D2)",
    );

    // The row is STILL Running (the shim's stop is in flight). One more
    // Fail tick must NOT re-emit — the reset counter climbs 0 -> 1, below
    // threshold.
    let fact = liveness_fact(
        "svc-inflight-0",
        Some(ProbeStatus::Fail { last_fail_reason: "fail-inflight".to_string() }),
        threshold,
    );
    let (actions, next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact),
        &view,
        &tick(),
    );
    assert!(
        !actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "no second StopAllocation while the stop is in flight; got {actions:?}",
    );
    assert_eq!(
        next.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
        1,
        "the reset counter climbs 0 -> 1 on the next Fail (still below threshold)",
    );
}

/// S-ROH-A-09 (ADR-0087 D2, `state == Running` guard) — CONTRACT_SHAPE:
/// bounded-change (emit-only). Once the row leaves Running (the shim's
/// stop landed), the liveness predicate is false BY CONSTRUCTION even
/// with a threshold-crossing counter: `triggered = state == Running &&
/// consecutive_failures >= threshold`. No terminate is emitted.
#[test]
fn liveness_predicate_false_once_row_leaves_running() {
    let recon = ServiceLifecycleReconciler::new();
    let threshold: u32 = 3;
    let key = (aid("svc-terminated-0"), ProbeIdx::new(0));

    // Seed the counter one below threshold, so a single Fail this tick
    // would cross it — proving the ONLY thing suppressing the emit is the
    // non-Running state.
    let mut view = ServiceLifecycleView::default();
    view.liveness_consecutive_failures.insert(key, threshold - 1);

    let mut fact = liveness_fact(
        "svc-terminated-0",
        Some(ProbeStatus::Fail { last_fail_reason: "post-stop fail".to_string() }),
        threshold,
    );
    fact.state = AllocState::Terminated;

    let (actions, _next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact),
        &view,
        &tick(),
    );

    assert!(
        !actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "a non-Running row emits no liveness terminate regardless of counter; got {actions:?}",
    );
}

/// S-ROH-A-09 (ADR-0087 D2, clean-counter-after-restart) —
/// CONTRACT_SHAPE: bounded-change (emit-only). A fresh Running alloc
/// (distinct id, no prior counter entry — the post-restart shape) starts
/// its consecutive-failure streak from zero: a single Fail lands the
/// counter at 1, below a threshold > 1, and emits no terminate.
#[test]
fn fresh_alloc_starts_with_clean_liveness_counter() {
    let recon = ServiceLifecycleReconciler::new();
    let view = ServiceLifecycleView::default(); // no entry for this alloc
    let threshold: u32 = 3;

    let fact = liveness_fact(
        "svc-fresh-0",
        Some(ProbeStatus::Fail { last_fail_reason: "first fail".to_string() }),
        threshold,
    );
    let (actions, next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact),
        &view,
        &tick(),
    );

    assert!(
        !actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "a fresh alloc's first Fail is below threshold — no terminate; got {actions:?}",
    );
    let key = (aid("svc-fresh-0"), ProbeIdx::new(0));
    assert_eq!(
        next.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
        1,
        "the fresh streak starts at 1 after one Fail",
    );
}
