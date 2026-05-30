//! Tier 1 acceptance — liveness probe → `RestartAllocation`.
//!
//! Slice 05 (US-05). Step 03-02.
//!
//! KPI K3: liveness probe N consecutive fails (N =
//! `failure_threshold`) → `Action::RestartAllocation { reason:
//! LivenessExhausted { probe_idx, consecutive_failures, threshold } }`
//! emitted within 1 reconciler tick; recovery (one Pass below
//! threshold) resets the counter and emits no restart; once the shared
//! `RESTART_BACKOFF_CEILING` budget is spent the liveness branch
//! finalises `ServiceFailed { LivenessProbeFailed }`.
//!
//! These acceptance scenarios drive the real
//! `ServiceLifecycleReconciler::reconcile` through its driving-port
//! signature and assert on the emitted `Action`s + the next-View
//! counter slot (the port-exposed observable surface). The exhaustive
//! property coverage of the predicate universe lives co-located with
//! the reconcile logic in
//! `overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`
//! (per the 03-01 cross-crate mutation-killing lesson); these are the
//! representative end-to-end demonstrations of the three named
//! scenarios.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::too_many_lines)]
#![allow(
    clippy::doc_markdown,
    reason = "acceptance-test docs name bare API identifiers (RestartAllocation, BackoffExhausted) in prose; backticking every one is noise in test-doc context"
)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeStatus};
use overdrive_core::reconcilers::{
    Action, RESTART_BACKOFF_CEILING, Reconciler, RestartReason, TickContext,
};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn restart_spec() -> overdrive_core::traits::driver::AllocationSpec {
    overdrive_core::traits::driver::AllocationSpec {
        alloc: aid("alloc-x"),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        command: "/bin/svc".to_string(),
        args: vec![],
        resources: overdrive_core::traits::driver::Resources {
            cpu_milli: 100,
            memory_bytes: 64 * 1024 * 1024,
        },
        probe_descriptors: vec![],
    }
}

fn liveness_fact(
    alloc_id: &str,
    latest_liveness_probe: Option<ProbeStatus>,
    failure_threshold: u32,
    restart_count: u32,
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
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe,
        has_liveness_probe: true,
        liveness_failure_threshold: failure_threshold,
        restart_count,
        restart_spec: restart_spec(),
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    allocs.insert(f.alloc_id.clone(), f);
    ServiceLifecycleState { allocs, service_dataplane: None }
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
    restart_count: u32,
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
            restart_count,
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

/// S-SHCP-RECON-09 (US-05 / K3) — three consecutive liveness fails on a
/// Running Service alloc (threshold 3, budget remaining) emit EXACTLY
/// one `Action::RestartAllocation` with `reason: LivenessExhausted {
/// probe_idx: 0, consecutive_failures: 3, threshold: 3 }` within one
/// tick.
#[test]
fn three_consecutive_liveness_fails_emits_restart_allocation_liveness_exhausted() {
    let (actions, _view) = run_consecutive_fails("svc-live-0", 3, 0, 3);

    let restarts: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(restarts.len(), 1, "exactly one RestartAllocation at threshold; got {actions:?}");
    match restarts[0] {
        Action::RestartAllocation { reason: Some(r), kind, .. } => {
            assert_eq!(
                r,
                &RestartReason::LivenessExhausted {
                    probe_idx: 0,
                    consecutive_failures: 3,
                    threshold: 3,
                },
            );
            assert_eq!(*kind, overdrive_core::aggregate::WorkloadKind::Service);
        }
        other => panic!("expected RestartAllocation(LivenessExhausted), got {other:?}"),
    }
}

/// S-SHCP-RECON-10 (US-05 — recovery resets counter) — liveness fails
/// twice (below the threshold of 3) then passes → the next-View
/// consecutive-failure counter resets to 0 and zero RestartAllocation
/// is ever emitted.
#[test]
fn liveness_fail_fail_pass_resets_counter_and_emits_no_restart() {
    let recon = ServiceLifecycleReconciler::new();
    let mut view = ServiceLifecycleView::default();
    let mut all_restarts = 0usize;

    // Fail, Fail (below threshold 3), then Pass.
    let observations = [
        Some(ProbeStatus::Fail { last_fail_reason: "1".to_string() }),
        Some(ProbeStatus::Fail { last_fail_reason: "2".to_string() }),
        Some(ProbeStatus::Pass),
    ];
    for obs in observations {
        let fact = liveness_fact("svc-rec-0", obs, 3, 0);
        let (actions, next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick(),
        );
        all_restarts +=
            actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
        view = next;
    }

    let key = (aid("svc-rec-0"), ProbeIdx::new(0));
    assert_eq!(
        view.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
        0,
        "a Pass below threshold resets the counter to 0",
    );
    assert_eq!(all_restarts, 0, "fail/fail/pass below threshold emits no RestartAllocation");
}

/// S-SHCP-RECON-11 (US-05 — restart budget exhaustion) — with the
/// shared `RESTART_BACKOFF_CEILING` already consumed, a fresh liveness
/// threshold breach finalises `Failed { ServiceFailed {
/// LivenessProbeFailed { probe_idx: 0, attempts } } }` so operators
/// can distinguish liveness-driven backoff from crash-loop backoff,
/// and emits no further RestartAllocation.
#[test]
fn liveness_retrigger_at_ceiling_emits_backoff_exhausted() {
    let (actions, _view) = run_consecutive_fails("svc-ceil-0", 3, RESTART_BACKOFF_CEILING, 3);

    let restarts = actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
    assert_eq!(restarts, 0, "at ceiling, no further RestartAllocation; got {actions:?}");

    let service_failed = actions.iter().any(|a| {
        matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::LivenessProbeFailed { .. },
                }),
                ..
            }
        )
    });
    assert!(
        service_failed,
        "ceiling re-trigger finalises ServiceFailed(LivenessProbeFailed); got {actions:?}"
    );
}
