//! S-ROH-A-06 (PRIMARY) — ADR-0087 single-restart-authority end-to-end
//! trajectory: liveness-Fail → `ServiceLifecycle` `StopAllocation` →
//! shim writes `Stopped { by: LivenessProbe }` → `WorkloadLifecycle`
//! `RestartAllocation` (restart_counts++) → … → exhaust →
//! `ServiceFailed { LivenessProbeFailed }`.
//!
//! This composes BOTH reconcilers' pure `reconcile` driving ports across
//! simulated cadences, threading the observed `AllocStatusRow` and the
//! `WorkloadLifecycleView.restart_counts` budget exactly as the runtime
//! + action-shim would (the shim's `StopAllocation` writes the
//! action-supplied `terminal` verbatim; its `RestartAllocation` brings
//! the alloc back to Running). The load-bearing property this pins is
//! the ADR-0087 crux: the liveness cause travels ONLY on the shared
//! observed row's terminal — `ServiceLifecycle` reads no restart budget,
//! `WorkloadLifecycle` owns the single budget spanning the whole loop —
//! and the exhaustion terminal is the cause-aware
//! `ServiceFailed { LivenessProbeFailed }`, NOT `BackoffExhausted`.
//! CONTRACT_SHAPE: bounded-change (emit-only reconcile decisions +
//! View evolution observed across the trajectory).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::too_many_lines)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    reason = "acceptance-test docs name bare API identifiers and arrow-continued prose"
)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::observation::ProbeStatus;
use overdrive_core::reconcilers::{
    Action, RESTART_BACKOFF_CEILING, Reconciler, TickContext, WorkloadLifecycle,
    WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};

const ALLOC: &str = "alloc-svc-0";
const WORKLOAD: &str = "svc";
const NODE: &str = "local";

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn jid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn tick_at(now_unix_secs: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_unix_secs)),
        tick: now_unix_secs,
        deadline: now + Duration::from_secs(1),
    }
}

// ---- ServiceLifecycle side ----

fn liveness_running_fact() -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: aid(ALLOC),
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
            "spiffe://overdrive.local/workload/svc/alloc/0",
        )
        .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        // threshold 1 → a single Fail this tick trips the terminate.
        liveness_failure_threshold: 1,
    }
}

fn service_state(fact: ServiceAllocFact) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    allocs.insert(fact.alloc_id.clone(), fact);
    ServiceLifecycleState { allocs, service_dataplane: None, prior_backend_row_at: None }
}

// ---- WorkloadLifecycle side ----

fn make_job() -> Job {
    Job {
        id: jid(WORKLOAD),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
    }
}

fn one_node_map() -> BTreeMap<NodeId, Node> {
    let mut m = BTreeMap::new();
    m.insert(
        nid(NODE),
        Node {
            id: nid(NODE),
            region: Region::new("local").expect("valid Region"),
            capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
        },
    );
    m
}

/// A Terminated alloc row carrying the liveness terminal the shim writes
/// after a `ServiceLifecycle` liveness `StopAllocation`: `reason =
/// Stopped { by: Reconciler }` (shim hardcode), cause on `terminal`.
fn terminated_by_liveness(counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(ALLOC),
        workload_id: jid(WORKLOAD),
        node_id: nid(NODE),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter, writer: nid(NODE) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Reconciler }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn workload_states(row: AllocStatusRow) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let base = |allocs: BTreeMap<AllocationId, AllocStatusRow>| WorkloadLifecycleState {
        workload_id: jid(WORKLOAD),
        job: Some(make_job()),
        desired_to_stop: false,
        generation: 1,
        nodes: one_node_map(),
        allocations: allocs,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let mut actual_allocs = BTreeMap::new();
    actual_allocs.insert(row.alloc_id.clone(), row);
    // observed_generation == desired.generation is set on the View; the
    // desired/actual states both carry generation 1.
    (base(BTreeMap::new()), base(actual_allocs))
}

/// S-ROH-A-06 — the full liveness restart-loop trajectory converges to
/// `ServiceFailed { LivenessProbeFailed }` under one unified restart
/// budget, cross-read-free.
#[test]
fn liveness_restart_loop_trajectory_exhausts_to_service_failed() {
    let sl = ServiceLifecycleReconciler::new();
    let wl = WorkloadLifecycle::canonical();

    // Single shared budget lives here, threaded across every cadence.
    let mut wl_view =
        WorkloadLifecycleView { observed_generation: 1, ..WorkloadLifecycleView::default() };
    // ServiceLifecycle's private counter View (never carries budget).
    let sl_view = ServiceLifecycleView::default();

    // Cycles 0..CEILING each restart; the CEILING-th cycle finalises.
    for cycle in 0..=RESTART_BACKOFF_CEILING {
        let now_secs = 100 + u64::from(cycle) * 10; // advance well past the 1s backoff window

        // --- ServiceLifecycle: liveness threshold → StopAllocation ---
        let (sl_actions, _sl_next) = sl.reconcile(
            &ServiceLifecycleState::default(),
            &service_state(liveness_running_fact()),
            &sl_view,
            &tick_at(now_secs),
        );
        let sl_stops: Vec<_> =
            sl_actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).collect();
        assert_eq!(
            sl_stops.len(),
            1,
            "cycle {cycle}: ServiceLifecycle emits exactly one StopAllocation; got {sl_actions:?}",
        );
        assert!(
            matches!(
                sl_stops[0],
                Action::StopAllocation {
                    terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
                    ..
                }
            ),
            "cycle {cycle}: the terminate carries Stopped {{ by: LivenessProbe }}; got {sl_actions:?}",
        );
        assert!(
            !sl_actions
                .iter()
                .any(|a| matches!(a, Action::RestartAllocation { .. } | Action::FinalizeFailed { .. })),
            "cycle {cycle}: ServiceLifecycle reads no budget and makes no restart/finalize decision",
        );

        // --- shim: write the Terminated row (cause on `terminal`) ---
        let row = terminated_by_liveness(2 + u64::from(cycle));
        let (desired, actual) = workload_states(row);

        // --- WorkloadLifecycle: sole restart authority ---
        let (wl_actions, wl_next) = wl.reconcile(&desired, &actual, &wl_view, &tick_at(now_secs));
        wl_view = wl_next;

        if cycle < RESTART_BACKOFF_CEILING {
            let restarts: Vec<_> =
                wl_actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
            assert_eq!(
                restarts.len(),
                1,
                "cycle {cycle}: below ceiling WorkloadLifecycle restarts under its single budget; got {wl_actions:?}",
            );
            assert_eq!(
                wl_view.restart_counts.get(&aid(ALLOC)).copied(),
                Some(cycle + 1),
                "cycle {cycle}: the single restart budget increments by one per liveness restart",
            );
            assert!(
                !wl_actions.iter().any(|a| matches!(a, Action::FinalizeFailed { .. })),
                "cycle {cycle}: below ceiling nothing is finalised; got {wl_actions:?}",
            );
            // shim RestartAllocation brings the alloc back to Running for
            // the next cadence — modelled by the fresh Running fact the
            // next ServiceLifecycle tick presents.
        } else {
            // Ceiling: cause-aware exhaustion terminal, NOT BackoffExhausted.
            let terminal = wl_actions.iter().find_map(|a| match a {
                Action::FinalizeFailed { terminal: Some(t), .. } => Some(t),
                _ => None,
            });
            assert_eq!(
                terminal,
                Some(&TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::LivenessProbeFailed {
                        probe_idx: 0,
                        attempts: RESTART_BACKOFF_CEILING,
                    },
                }),
                "ceiling: the liveness loop finalises ServiceFailed(LivenessProbeFailed), \
                 NOT BackoffExhausted; got {wl_actions:?}",
            );
            assert!(
                !wl_actions.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
                "ceiling: no further RestartAllocation; got {wl_actions:?}",
            );
        }
    }

    // The unified budget was drawn down entirely by liveness kills.
    assert_eq!(
        wl_view.restart_counts.get(&aid(ALLOC)).copied(),
        Some(RESTART_BACKOFF_CEILING),
        "the whole liveness loop drew ONE budget up to RESTART_BACKOFF_CEILING",
    );
}
