//! RED acceptance scaffolds pinning lifecycle gate ownership for VM Service
//! probe observations. These are regression extensions over reused
//! reconcilers, not authority to add VM-specific lifecycle state.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::id::{NodeId, ServiceId, ServiceVip};
use overdrive_core::observation::ProbeStatus;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, SpiffeId};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-service-vm-e08-0").expect("static allocation ID is valid")
}

fn tick(seconds: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(seconds)),
        tick: seconds,
        deadline: Instant::now() + Duration::from_secs(1),
    }
}

fn running_fact(
    alloc_id: AllocationId,
    startup: ProbeStatus,
    max_attempts: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id,
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(10))),
        exit_code: None,
        latest_startup_probe: Some(startup),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        max_attempts,
        startup_deadline: Duration::from_secs(1),
        mechanic_summary: "tcp 0.0.0.0:18081".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: SpiffeId::new("spiffe://overdrive.local/workload/service-vm-e08/alloc/0")
            .expect("static backend SPIFFE ID is valid"),
        backend_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 18_081),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
    }
}

fn state_for(fact: ServiceAllocFact) -> ServiceLifecycleState {
    let alloc_id = fact.alloc_id.clone();
    ServiceLifecycleState { allocs: BTreeMap::from([(alloc_id, fact)]), ..Default::default() }
}

fn service_dataplane() -> ServiceDataplaneIdentity {
    ServiceDataplaneIdentity {
        service_id: ServiceId::new(42).expect("valid service id"),
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 42))).expect("valid vip"),
        writer: NodeId::new("svm-0096").expect("valid node id"),
    }
}

/// S-SVM-18 — Beacon/driver start commits Running before probe registration.
/// A closed guest startup port leaves Running intact and only the existing
/// startup-attempt/deadline path can produce `StartupProbeFailed`.
/// CONTRACT_SHAPE: unbounded-preservation.
/// Outcome anchor: DISCUSS Elevator Pitch.
#[test]
fn vm_startup_failure_leaves_running_owned_by_beacon_and_fails_only_startup() {
    let alloc_id = alloc_id();
    let actual = state_for(running_fact(
        alloc_id.clone(),
        ProbeStatus::Fail { last_fail_reason: "guest TCP port 18081 refused".to_string() },
        1,
    ));

    let (actions, next_view) = ServiceLifecycleReconciler::new().reconcile(
        &actual,
        &actual,
        &ServiceLifecycleView::default(),
        &tick(11),
    );

    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert_eq!(actions.len(), 1, "startup failure must emit only its existing terminal action");
    assert!(matches!(
        &actions[0],
        Action::FinalizeFailed {
            alloc_id: action_alloc_id,
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed {
                    probe_idx: 0,
                    last_fail,
                    attempts: 1,
                },
            }),
        } if action_alloc_id == &alloc_id && last_fail == "guest TCP port 18081 refused"
    ));
    assert!(next_view.terminal_announced.contains(&alloc_id));
    assert!(!next_view.stable_announced.contains(&alloc_id));
}

/// ADR-0096 — a deciding StartupProbeFailed writes the existing unhealthy
/// backend row before its existing terminal, while a non-terminal no-readiness
/// allocation remains eligible and a late observation cannot revive the
/// terminal allocation.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failure_vetoes_backend_eligibility_before_terminal_publication() {
    let alloc_id = alloc_id();
    let mut actual = state_for(running_fact(
        alloc_id.clone(),
        ProbeStatus::Fail { last_fail_reason: "guest TCP port 18081 refused".to_string() },
        1,
    ));
    actual.service_dataplane = Some(service_dataplane());

    let (actions, next_view) = ServiceLifecycleReconciler::new().reconcile(
        &actual,
        &actual,
        &ServiceLifecycleView::default(),
        &tick(11),
    );

    assert!(matches!(
        &actions[..],
        [
            Action::WriteServiceBackendRow { row, .. },
            Action::FinalizeFailed {
                alloc_id: action_alloc_id,
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::StartupProbeFailed { .. },
                }),
            },
        ] if row.backends.len() == 1
            && row.backends[0].alloc == actual.allocs[&alloc_id].backend_spiffe
            && !row.backends[0].healthy
            && action_alloc_id == &alloc_id
    ));
    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert!(next_view.terminal_announced.contains(&alloc_id));
    assert!(actions.iter().all(|action| !matches!(action, Action::RestartAllocation { .. })));

    let (late_actions, _) =
        ServiceLifecycleReconciler::new().reconcile(&actual, &actual, &next_view, &tick(12));
    assert!(
        late_actions.is_empty(),
        "the persisted unhealthy row remains the current backend projection; a late tick must not re-enable it"
    );

    let fresh_alloc = AllocationId::new("alloc-service-vm-e08-1").expect("valid allocation ID");
    let mut fresh = state_for(running_fact(
        fresh_alloc.clone(),
        ProbeStatus::Fail { last_fail_reason: "first observation".to_string() },
        2,
    ));
    fresh.service_dataplane = Some(service_dataplane());
    let (fresh_actions, _) = ServiceLifecycleReconciler::new().reconcile(
        &fresh,
        &fresh,
        &ServiceLifecycleView::default(),
        &tick(11),
    );
    assert!(matches!(
        &fresh_actions[..],
        [Action::WriteServiceBackendRow { row, .. }]
            if row.backends.len() == 1 && row.backends[0].healthy
    ));
}

/// S-SVM-19 — a VM startup Pass makes `ServiceLifecycle` emit Stable with the
/// existing witness/timing shape. It does not write readiness health or make
/// a restart decision.
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch.
#[test]
fn vm_startup_pass_changes_only_service_stable() {
    let alloc_id = alloc_id();
    let actual = state_for(running_fact(alloc_id.clone(), ProbeStatus::Pass, 30));

    let (actions, next_view) = ServiceLifecycleReconciler::new().reconcile(
        &actual,
        &actual,
        &ServiceLifecycleView::default(),
        &tick(15),
    );

    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert_eq!(actions.len(), 1, "startup pass must emit only Stable");
    assert!(matches!(
        &actions[0],
        Action::FinalizeFailed {
            alloc_id: action_alloc_id,
            terminal: Some(TerminalCondition::Stable { settled_in_ms: 5_000, witness }),
        } if action_alloc_id == &alloc_id
            && witness.probe_idx == 0
            && witness.role == "startup"
            && witness.mechanic_summary == "tcp 0.0.0.0:18081"
            && !witness.inferred
    ));
    assert!(next_view.stable_announced.contains(&alloc_id));
    assert!(next_view.terminal_announced.is_empty());
    assert!(next_view.readiness_consecutive_successes.is_empty());
    assert!(next_view.liveness_consecutive_failures.is_empty());
}

/// S-SVM-20 — readiness 204 -> 503 -> 204 flips only
/// `Backend.healthy` true -> false -> true at the existing thresholds; the
/// allocation remains Running and Stable and no RestartAllocation is emitted.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_readiness_flaps_only_backend_eligibility_and_recovers() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-20 / readiness owns backend health)");
}

/// S-SVM-21A — the liveness threshold makes `ServiceLifecycle` emit only the
/// existing liveness `StopAllocation`; success before threshold resets the
/// counter.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_liveness_threshold_emits_only_the_existing_liveness_stop() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-21A / liveness stop owner)");
}

/// S-SVM-21B — after the liveness-stopped row is observed,
/// `WorkloadLifecycle` alone chooses restart versus final failure under the
/// unified budget.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn workload_lifecycle_alone_decides_restart_after_liveness_stop() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-21B / restart owner)");
}
