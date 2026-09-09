//! RED acceptance scaffolds pinning lifecycle gate ownership for VM Service
//! probe observations. These are regression extensions over reused
//! reconcilers, not authority to add VM-specific lifecycle state.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Exec, Job, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{NodeId, ServiceId, ServiceVip};
use overdrive_core::observation::ProbeStatus;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ServiceBackendRow,
};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, SpiffeId, WorkloadId};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use overdrive_reconcilers::workload_lifecycle::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
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
        backend_ip: Ipv4Addr::LOCALHOST,
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
        port: std::num::NonZeroU16::new(18_081).expect("listener port"),
        protocol: overdrive_core::dataplane::backend_key::Proto::Tcp,
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
    actual.service_dataplane =
        BTreeMap::from([(ServiceId::new(42).expect("service id"), service_dataplane())]);

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
            Action::EnqueueEvaluation { reconciler, target },
            Action::FinalizeFailed {
                alloc_id: action_alloc_id,
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::StartupProbeFailed { .. },
                }),
            },
        ] if reconciler.as_str() == "service-map-hydrator"
            && target.as_str() == "service/42"
            && row.backends.len() == 1
            && row.backends[0].alloc == actual.allocs[&alloc_id].backend_spiffe
            && !row.backends[0].healthy
            && action_alloc_id == &alloc_id
    ));
    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert!(next_view.terminal_announced.contains(&alloc_id));
    assert!(actions.iter().all(|action| !matches!(action, Action::RestartAllocation { .. })));

    for action in &actions {
        if let Action::WriteServiceBackendRow { row, .. } = action {
            actual.observed_backend_rows.insert(row.service_id, row.clone());
        }
    }
    let (late_actions, _) =
        ServiceLifecycleReconciler::new().reconcile(&actual, &actual, &next_view, &tick(12));
    assert!(
        late_actions.is_empty(),
        "the persisted unhealthy row remains the current backend projection; a late tick must not re-enable it"
    );

    let fresh_alloc = AllocationId::new("alloc-service-vm-e08-1").expect("valid allocation ID");
    let mut fresh = state_for(running_fact(
        fresh_alloc,
        ProbeStatus::Fail { last_fail_reason: "first observation".to_string() },
        2,
    ));
    fresh.service_dataplane =
        BTreeMap::from([(ServiceId::new(42).expect("service id"), service_dataplane())]);
    let (fresh_actions, _) = ServiceLifecycleReconciler::new().reconcile(
        &fresh,
        &fresh,
        &ServiceLifecycleView::default(),
        &tick(11),
    );
    assert!(matches!(
        &fresh_actions[..],
        [Action::WriteServiceBackendRow { row, .. }, Action::EnqueueEvaluation { reconciler, target }]
            if row.backends.len() == 1 && row.backends[0].healthy
                && reconciler.as_str() == "service-map-hydrator" && target.as_str() == "service/42"
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
fn vm_readiness_flaps_only_backend_eligibility_and_recovers() {
    let alloc_id = alloc_id();
    let service_id = ServiceId::new(42).expect("service id");
    let mut actual = state_for(running_fact(alloc_id.clone(), ProbeStatus::Pass, 30));
    actual.service_dataplane = BTreeMap::from([(service_id, service_dataplane())]);
    actual.allocs.get_mut(&alloc_id).expect("allocation fact").latest_readiness_probe =
        Some(ProbeStatus::Pass);
    actual.allocs.get_mut(&alloc_id).expect("allocation fact").has_readiness_probe = true;
    actual.allocs.get_mut(&alloc_id).expect("allocation fact").readiness_success_threshold = 1;

    let reconciler = ServiceLifecycleReconciler::new();
    assert_eq!(
        reconciler.interests(),
        &[
            overdrive_core::traits::observation_store::ObservationRowKind::AllocStatus,
            overdrive_core::traits::observation_store::ObservationRowKind::ProbeResult,
        ],
        "ServiceLifecycle wakes from allocation and accepted probe-result observations",
    );
    let (initial_actions, stable_view) =
        reconciler.reconcile(&actual, &actual, &ServiceLifecycleView::default(), &tick(15));

    let initial_row = service_backend_row(&initial_actions);
    assert!(initial_row.backends[0].healthy, "a readiness Pass makes the Running backend eligible");
    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert!(stable_view.stable_announced.contains(&alloc_id));
    assert!(!has_restart(&initial_actions));
    assert_eq!(initial_row.backends.len(), 1);
    assert_eq!(initial_row.service_id, service_id);
    assert_eq!(initial_row.vip, Ipv4Addr::new(10, 96, 0, 42));
    actual.observed_backend_rows.insert(service_id, initial_row.clone());

    actual.allocs.get_mut(&alloc_id).expect("allocation fact").latest_readiness_probe =
        Some(ProbeStatus::Fail { last_fail_reason: "HTTP status 503".to_string() });
    let (withdraw_actions, unready_view) =
        reconciler.reconcile(&actual, &actual, &stable_view, &tick(16));

    let withdrawn_row = service_backend_row(&withdraw_actions);
    assert!(!withdrawn_row.backends[0].healthy, "readiness Fail withdraws eligibility");
    assert_eq!(withdrawn_row.backends[0].alloc, initial_row.backends[0].alloc);
    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert!(unready_view.stable_announced.contains(&alloc_id));
    assert!(unready_view.readiness_consecutive_successes.is_empty());
    assert!(!has_restart(&withdraw_actions));
    assert!(!withdraw_actions.iter().any(|action| matches!(
        action,
        Action::FinalizeFailed { .. } | Action::StopAllocation { .. }
    )));
    actual.observed_backend_rows.insert(service_id, withdrawn_row.clone());

    actual.allocs.get_mut(&alloc_id).expect("allocation fact").latest_readiness_probe =
        Some(ProbeStatus::Pass);
    let (restore_actions, ready_view) =
        reconciler.reconcile(&actual, &actual, &unready_view, &tick(17));

    let restored_row = service_backend_row(&restore_actions);
    assert!(restored_row.backends[0].healthy, "readiness Pass restores eligibility");
    assert_eq!(restored_row.backends[0].alloc, initial_row.backends[0].alloc);
    assert_eq!(actual.allocs[&alloc_id].state, AllocState::Running);
    assert!(ready_view.stable_announced.contains(&alloc_id));
    assert_eq!(ready_view.readiness_consecutive_successes.len(), 1);
    assert!(!has_restart(&restore_actions));
    assert!(!restore_actions.iter().any(|action| matches!(
        action,
        Action::FinalizeFailed { .. } | Action::StopAllocation { .. }
    )));
}

fn service_backend_row(actions: &[Action]) -> &ServiceBackendRow {
    actions
        .iter()
        .find_map(|action| match action {
            Action::WriteServiceBackendRow { row, .. } => Some(row),
            _ => None,
        })
        .expect("readiness transition writes the complete service backend row")
}

fn has_restart(actions: &[Action]) -> bool {
    actions.iter().any(|action| matches!(action, Action::RestartAllocation { .. }))
}

fn service_job(workload_id: &WorkloadId) -> Job {
    Job {
        id: workload_id.clone(),
        replicas: NonZeroU32::new(1).expect("one replica is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
    }
}

fn liveness_stopped_row(alloc_id: &AllocationId, workload_id: &WorkloadId) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: NodeId::new("local").expect("static node id is valid"),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("local").expect("static node id is valid"),
        },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Reconciler }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(10))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn workload_states(
    row: AllocStatusRow,
) -> (WorkloadLifecycleState, WorkloadLifecycleState, AllocationId) {
    let workload_id = row.workload_id.clone();
    let alloc_id = row.alloc_id.clone();
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(service_job(&workload_id)),
        desired_to_stop: false,
        generation: 0,
        nodes: BTreeMap::new(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: None,
        desired_to_stop: false,
        generation: 0,
        nodes: BTreeMap::new(),
        allocations: BTreeMap::from([(alloc_id.clone(), row)]),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    (desired, actual, alloc_id)
}

/// S-SVM-21A — the liveness threshold makes `ServiceLifecycle` emit only the
/// existing liveness `StopAllocation`; success before threshold resets the
/// counter.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn vm_liveness_threshold_emits_only_the_existing_liveness_stop() {
    let alloc_id = alloc_id();
    let reconciler = ServiceLifecycleReconciler::new();
    let mut actual = state_for(running_fact(alloc_id.clone(), ProbeStatus::Pass, 30));
    actual.allocs.get_mut(&alloc_id).expect("allocation fact").has_liveness_probe = true;
    let (stable_actions, mut view) =
        reconciler.reconcile(&actual, &actual, &ServiceLifecycleView::default(), &tick(15));
    assert!(stable_actions.iter().any(|action| matches!(
        action,
        Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }), .. }
    )));

    let mut observed = actual.clone();
    observed.allocs.get_mut(&alloc_id).expect("allocation fact").latest_liveness_probe =
        Some(ProbeStatus::Fail { last_fail_reason: "HTTP status 503".to_string() });
    let (first_fail, first_fail_view) =
        reconciler.reconcile(&observed, &observed, &view, &tick(16));
    assert!(first_fail.is_empty(), "one liveness failure is below the threshold");
    view = first_fail_view;

    let (second_fail, second_fail_view) =
        reconciler.reconcile(&observed, &observed, &view, &tick(17));
    assert!(second_fail.is_empty(), "two liveness failures are below the threshold");
    view = second_fail_view;

    observed.allocs.get_mut(&alloc_id).expect("allocation fact").latest_liveness_probe =
        Some(ProbeStatus::Pass);
    let (recovered, recovered_view) = reconciler.reconcile(&observed, &observed, &view, &tick(18));
    assert!(recovered.is_empty(), "a liveness Pass resets without stopping the allocation");
    assert!(recovered_view.liveness_consecutive_failures.is_empty());
    view = recovered_view;

    observed.allocs.get_mut(&alloc_id).expect("allocation fact").latest_liveness_probe =
        Some(ProbeStatus::Fail { last_fail_reason: "HTTP status 503".to_string() });
    let (new_first_fail, new_first_fail_view) =
        reconciler.reconcile(&observed, &observed, &view, &tick(19));
    assert!(new_first_fail.is_empty(), "the recovered streak starts at one");
    view = new_first_fail_view;
    let (new_second_fail, new_second_fail_view) =
        reconciler.reconcile(&observed, &observed, &view, &tick(20));
    assert!(new_second_fail.is_empty(), "the recovered streak is still below threshold");
    view = new_second_fail_view;

    let (threshold_actions, threshold_view) =
        reconciler.reconcile(&observed, &observed, &view, &tick(21));
    assert_eq!(threshold_actions.len(), 1, "threshold emits only one existing liveness stop");
    assert!(matches!(
        &threshold_actions[0],
        Action::StopAllocation {
            alloc_id: action_alloc_id,
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        } if action_alloc_id == &alloc_id
    ));
    assert!(threshold_view.liveness_consecutive_failures.is_empty());
    assert_eq!(observed.allocs[&alloc_id].state, AllocState::Running);
    assert!(!has_restart(&threshold_actions));
    assert!(
        !threshold_actions.iter().any(|action| matches!(action, Action::FinalizeFailed { .. }))
    );
}

/// S-SVM-21B — after the liveness-stopped row is observed,
/// `WorkloadLifecycle` alone chooses restart versus final failure under the
/// unified budget.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn workload_lifecycle_alone_decides_restart_after_liveness_stop() {
    let workload_id = WorkloadId::new("service-vm-liveness-restart").expect("valid workload id");
    let alloc_id =
        AllocationId::new("alloc-service-vm-liveness-restart-0").expect("valid allocation id");
    let row = liveness_stopped_row(&alloc_id, &workload_id);
    let (desired, actual, alloc_id) = workload_states(row);
    let reconciler = WorkloadLifecycle::canonical();
    let (restart_actions, restart_view) =
        reconciler.reconcile(&desired, &actual, &WorkloadLifecycleView::default(), &tick(30));
    assert!(restart_actions.iter().any(|action| matches!(
        action,
        Action::RestartAllocation {
            alloc_id: action_alloc_id,
            kind: WorkloadKind::Service,
            ..
        } if action_alloc_id == &alloc_id
    )));
    assert!(!restart_actions.iter().any(|action| matches!(
        action,
        Action::StopAllocation { .. } | Action::FinalizeFailed { .. }
    )));
    assert_eq!(restart_view.restart_counts.get(&alloc_id), Some(&1));
    assert!(restart_actions.iter().any(|action| matches!(
        action,
        Action::EnqueueEvaluation { reconciler, target }
            if reconciler.as_str() == "service-lifecycle"
                && target.as_str() == "workload/service-vm-liveness-restart"
    )));

    let mut exhausted_view = WorkloadLifecycleView::default();
    exhausted_view.restart_counts.insert(alloc_id.clone(), RESTART_BACKOFF_CEILING);
    let (final_actions, final_view) =
        reconciler.reconcile(&desired, &actual, &exhausted_view, &tick(31));
    assert!(final_actions.iter().any(|action| matches!(
        action,
        Action::FinalizeFailed {
            alloc_id: action_alloc_id,
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts },
            }),
        } if action_alloc_id == &alloc_id && *attempts == RESTART_BACKOFF_CEILING
    )));
    assert!(!final_actions.iter().any(|action| matches!(action, Action::RestartAllocation { .. })));
    assert_eq!(final_view, exhausted_view);
}
