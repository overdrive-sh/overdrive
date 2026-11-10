//! `ServiceLifecycleReconciler::reconcile` — the startup-probe stop contract.
//!
//! Feature `service-allocation-readiness-and-ending`:
//!
//! - D5 + D7 (S-21): a startup-probe failure stops a live allocation; a
//!   `Failed` or `Terminated` row yields no action; never a `FinalizeFailed`.
//! - D8 (S-29): the stop is re-emitted once per new written startup row; a
//!   `Pass` ends the retry; `Stable` leaves `terminal_announced`.
//! - D12 (S-46): backend eligibility is a function of the committed `Stable`
//!   and the readiness rule only; no per-allocation veto.
//!
//! Port-to-port: the pure driving port `ServiceLifecycleReconciler::reconcile`;
//! the oracle is the returned `Vec<Action>` and the returned `View`.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::doc_markdown, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use proptest::prelude::*;

const ALLOC: &str = "svc-0";
const STARTED_AT_MS: u64 = 1_000;
const DEADLINE: Duration = Duration::from_secs(60);
const MAX_ATTEMPTS: u32 = 3;

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn fail() -> ProbeStatus {
    ProbeStatus::Fail { last_fail_reason: "connection refused".to_string() }
}

fn tick_at_ms(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// The ONE place a `ServiceAllocFact` literal is built in this file.
fn fact(state: AllocState, started: bool, startup_probes_empty: bool) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: aid(ALLOC),
        phase: overdrive_reconcilers::service_lifecycle::NotRunningState::new(state).map_or_else(
            || {
                overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
                    overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                        spiffe: overdrive_core::SpiffeId::new(
                            "spiffe://overdrive.local/workload/svc/alloc/svc-0",
                        )
                        .expect("valid spiffe"),
                        ip: Ipv4Addr::new(192, 168, 1, 10),
                    },
                )
            },
            overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning,
        ),
        started_at: started
            .then(|| UnixInstant::from_unix_duration(Duration::from_millis(STARTED_AT_MS))),
        latest_startup_probe: None,
        latest_startup_probe_observed_at: None,
        max_attempts: MAX_ATTEMPTS,
        startup_deadline: DEADLINE,
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
        stable_committed: false,
    }
}

fn put_row(
    fact: &mut ServiceAllocFact,
    role: ProbeRole,
    status: ProbeStatus,
    run_seq: u64,
    streak: u32,
) {
    match role {
        ProbeRole::Startup => {
            fact.latest_startup_probe = Some(status.clone());
            fact.latest_startup_probe_observed_at =
                Some(UnixInstant::from_unix_duration(Duration::from_millis(run_seq)));
        }
        ProbeRole::Readiness => fact.latest_readiness_probe = Some(status.clone()),
        ProbeRole::Liveness => fact.latest_liveness_probe = Some(status.clone()),
    }
    let probe_idx = ProbeIdx::new(0);
    fact.probe_rows.insert(
        (role, probe_idx),
        ProbeResultRow {
            alloc_id: fact.alloc_id.clone(),
            probe_idx,
            role,
            status,
            last_observed_at_unix_ms: run_seq,
            inferred: false,
            run_seq: ProbeRunSeq::new(run_seq).expect("non-zero run sequence"),
            same_status_runs: SameStatusRuns::new(streak).expect("non-zero streak"),
        },
    );
}

fn state_of(fact: ServiceAllocFact, with_listener: bool) -> ServiceLifecycleState {
    let mut state = ServiceLifecycleState::default();
    if with_listener {
        state.service_dataplane.insert(
            ServiceId::new(7).expect("service id"),
            ServiceDataplaneIdentity {
                port: NonZeroU16::new(8080).expect("port"),
                protocol: Proto::Tcp,
                vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 9))).expect("vip"),
                writer: NodeId::new("node-r").expect("node id"),
            },
        );
    }
    state.allocs.insert(fact.alloc_id.clone(), fact);
    state
}

const fn is_startup_stop(action: &Action) -> bool {
    matches!(
        action,
        Action::StopAllocation {
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
            ..
        }
    )
}

fn startup_stops(actions: &[Action]) -> usize {
    actions.iter().filter(|a| is_startup_stop(a)).count()
}

fn failure_finalisations(actions: &[Action]) -> usize {
    actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::ServiceFailed { .. }),
                    ..
                }
            )
        })
        .count()
}

fn backend_rows_healthy(actions: &[Action]) -> Vec<bool> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row.backends.clone()),
            _ => None,
        })
        .flatten()
        .map(|b| b.healthy)
        .collect()
}

// =====================================================================
// S-21 — D5 guard + D7 stop
// =====================================================================

#[derive(Debug, Clone, Copy)]
enum History {
    NoRow,
    /// The latest startup row is a `Fail`. The history `Pass, Fail, Fail`
    /// is this case: the gate reads the LATEST row (R-2).
    LatestFail,
    LatestPass,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-21 (D5, D7). State x streak x elapsed x latest startup row x
    /// `started_at` x listener. A `Failed` or `Terminated` fact yields no
    /// finalisation, stop or restart (only the ordinary backend-row write); otherwise a stop iff all three gates hold, and then exactly
    /// `StopAllocation { Stopped { StartupProbe } }`, first in the returned
    /// list, never a `FinalizeFailed`; the allocation joins
    /// `terminal_announced`; a second evaluation with the same startup row
    /// emits no stop.
    ///
    /// Mutation target: the action swap, the D5 guard, each gate.
    #[test]
    /// CONTRACT_SHAPE: bounded-change.
    fn startup_probe_failure_stops_only_a_live_allocation(
        state in prop_oneof![
            Just(AllocState::Pending), Just(AllocState::Running), Just(AllocState::Failed),
            Just(AllocState::Terminated), Just(AllocState::Draining)
        ],
        history in prop_oneof![
            Just(History::NoRow), Just(History::LatestFail), Just(History::LatestPass)
        ],
        streak_at_max in any::<bool>(),
        past_deadline in any::<bool>(),
        started in any::<bool>(),
        with_listener in any::<bool>(),
    ) {
        // Hydrate invariant: a `Running` allocation carries `started_at`.
        prop_assume!(started || state != AllocState::Running);
        let streak = if streak_at_max { MAX_ATTEMPTS } else { MAX_ATTEMPTS - 1 };
        let mut f = fact(state, started, false);
        match history {
            History::NoRow => {}
            History::LatestFail => put_row(&mut f, ProbeRole::Startup, fail(), 4, streak),
            History::LatestPass => put_row(&mut f, ProbeRole::Startup, ProbeStatus::Pass, 4, streak),
        }
        let actual = state_of(f, with_listener);
        let now_ms = STARTED_AT_MS + if past_deadline { 60_000 } else { 59_999 };
        let tick = tick_at_ms(now_ms);
        let view = ServiceLifecycleView::default();
        let r = ServiceLifecycleReconciler::new();

        let (actions, next) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

        let terminal_row = matches!(state, AllocState::Failed | AllocState::Terminated);
        let expect_stop = !terminal_row
            && started
            && matches!(history, History::LatestFail)
            && streak_at_max
            && past_deadline;

        if terminal_row {
            // No ending action of any kind; the ordinary backend-row projection
            // (a withdrawal write) and the re-enqueue are not actions about the terminal row.
            prop_assert!(
                actions.iter().all(|a| matches!(
                    a,
                    Action::WriteServiceBackendRow { .. } | Action::EnqueueEvaluation { .. }
                )),
                "a terminal row yields no finalisation or stop; got {actions:?}"
            );
        }
        prop_assert_eq!(startup_stops(&actions), usize::from(expect_stop), "actions: {:?}", actions);
        prop_assert_eq!(failure_finalisations(&actions), 0, "never a FinalizeFailed failure: {:?}", actions);
        if expect_stop {
            prop_assert!(is_startup_stop(&actions[0]), "nothing precedes the stop: {:?}", actions);
            prop_assert!(next.terminal_announced.contains(&aid(ALLOC)));
            // No backend-row write accompanies the stop on account of it: any
            // row written is the ordinary projection and marks it not healthy.
            prop_assert!(backend_rows_healthy(&actions).iter().all(|h| !h));
            let (again, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &next, &tick);
            prop_assert_eq!(startup_stops(&again), 0, "same startup row, no second stop: {:?}", again);
        }
    }
}

// =====================================================================
// S-29 — D8 emission rule
// =====================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-29 (D8). Emit iff the guard and gates hold AND the latest startup
    /// row's `run_seq` exceeds the recorded `startup_last_acted_run`; the
    /// same `run_seq` never emits twice; a `Running` fact already in
    /// `terminal_announced` re-emits on a newer row; a non-`Running` fact in
    /// `terminal_announced` is skipped; a `Pass` ends the retry and the fact
    /// reaches the Stable branch and leaves `terminal_announced`; the stop
    /// records the row's `run_seq`; the backend row marks the allocation not
    /// `healthy` while the stop is pending.
    ///
    /// Mutation target: the `run_seq > recorded` comparison, the `Pass`
    /// exemption, the top-of-loop dedup for `Running` facts.
    #[test]
    /// CONTRACT_SHAPE: bounded-change.
    #[ignore = "pending DELIVER: D8 startup stop re-emitted once per new startup probe row"]
    fn startup_stop_is_emitted_once_per_new_startup_row(
        running in any::<bool>(),
        announced in any::<bool>(),
        latest_pass in any::<bool>(),
        run_seq in 1u64..=6,
        recorded in proptest::option::of(1u64..=6),
    ) {
        let state = if running { AllocState::Running } else { AllocState::Pending };
        let mut f = fact(state, true, false);
        let status = if latest_pass { ProbeStatus::Pass } else { fail() };
        put_row(&mut f, ProbeRole::Startup, status, run_seq, MAX_ATTEMPTS);
        let actual = state_of(f, true);
        let mut view = ServiceLifecycleView::default();
        if announced {
            view.terminal_announced.insert(aid(ALLOC));
        }
        if let Some(seq) = recorded {
            view.startup_last_acted_run.insert(
                (aid(ALLOC), ProbeIdx::new(0)),
                ProbeRunSeq::new(seq).expect("non-zero"),
            );
        }
        let tick = tick_at_ms(STARTED_AT_MS + 60_000);
        let r = ServiceLifecycleReconciler::new();

        let (actions, next) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

        let newer = recorded.is_none_or(|seq| run_seq > seq);
        let expect_stop = !latest_pass && newer && (running || !announced);
        prop_assert_eq!(startup_stops(&actions), usize::from(expect_stop), "actions: {:?}", actions);
        prop_assert_eq!(failure_finalisations(&actions), 0);
        let key = (aid(ALLOC), ProbeIdx::new(0));
        let recorded_after = next.startup_last_acted_run.get(&key).map(|s| s.get());
        if expect_stop {
            prop_assert_eq!(recorded_after, Some(run_seq));
            prop_assert!(next.terminal_announced.contains(&aid(ALLOC)));
            prop_assert!(backend_rows_healthy(&actions).iter().all(|h| !h));
            // The same row never emits twice.
            let (again, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &next, &tick);
            prop_assert_eq!(startup_stops(&again), 0);
        } else {
            prop_assert_eq!(recorded_after, recorded);
        }
        prop_assert_eq!(&next.liveness_last_acted_run, &view.liveness_last_acted_run);
        if running && latest_pass {
            let stables = actions
                .iter()
                .filter(|a| matches!(a, Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::Stable { .. }), ..
                }))
                .count();
            prop_assert_eq!(stables, 1, "a Pass reaches the Stable branch even for an announced id: {:?}", actions);
            prop_assert!(!next.terminal_announced.contains(&aid(ALLOC)), "Stable leaves terminal_announced");
        }
    }
}

// =====================================================================
// S-46 — D12 eligibility predicate
// =====================================================================

#[derive(Debug, Clone, Copy)]
enum Readiness {
    NotDeclared,
    PassAtThreshold,
    PassBelowThreshold,
    PassAboveThreshold,
    Fail,
    NoRow,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// S-46 (D12). Backend `healthy` is true iff (opt-out OR the allocation's
    /// own `Stable` is committed) AND the readiness rule holds. Neither the
    /// latest startup row, the allocation's own `terminal_announced`
    /// membership, nor an unrelated allocation's membership changes it.
    ///
    /// Mutation target: the `stable_committed` test (removed, negated, or
    /// read from the startup row), the opt-out exemption, the readiness rule.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER: D12 backend eligibility keys on the committed Stable"]
    fn startup_probed_allocation_is_a_backend_only_after_its_stable_is_committed(
        opt_out in any::<bool>(),
        stable_committed in any::<bool>(),
        own_announced in any::<bool>(),
        unrelated_announced in any::<bool>(),
        readiness in prop_oneof![
            Just(Readiness::NotDeclared), Just(Readiness::PassAtThreshold),
            Just(Readiness::PassBelowThreshold), Just(Readiness::PassAboveThreshold),
            Just(Readiness::Fail), Just(Readiness::NoRow)
        ],
        startup in prop_oneof![Just(None), Just(Some(true)), Just(Some(false))],
    ) {
        const THRESHOLD: u32 = 3;
        let mut f = fact(AllocState::Running, true, opt_out);
        match startup {
            None => {}
            // Streak 1 of max 3 and elapsed 1 s: no startup gate trips.
            Some(true) => put_row(&mut f, ProbeRole::Startup, ProbeStatus::Pass, 2, 1),
            Some(false) => put_row(&mut f, ProbeRole::Startup, fail(), 2, 1),
        }
        f.stable_committed = stable_committed;
        f.has_readiness_probe = !matches!(readiness, Readiness::NotDeclared);
        f.readiness_success_threshold = THRESHOLD;
        match readiness {
            Readiness::NotDeclared | Readiness::NoRow => {}
            Readiness::PassAtThreshold => put_row(&mut f, ProbeRole::Readiness, ProbeStatus::Pass, 9, THRESHOLD),
            Readiness::PassBelowThreshold => put_row(&mut f, ProbeRole::Readiness, ProbeStatus::Pass, 9, THRESHOLD - 1),
            Readiness::PassAboveThreshold => put_row(&mut f, ProbeRole::Readiness, ProbeStatus::Pass, 9, THRESHOLD + 1),
            Readiness::Fail => put_row(&mut f, ProbeRole::Readiness, fail(), 9, 1),
        }
        let actual = state_of(f, true);
        let mut view = ServiceLifecycleView::default();
        if own_announced {
            view.terminal_announced.insert(aid(ALLOC));
        }
        if unrelated_announced {
            view.terminal_announced.insert(aid("other-0"));
        }
        let r = ServiceLifecycleReconciler::new();

        let (actions, next) = r.reconcile(
            &ServiceLifecycleState::default(),
            &actual,
            &view,
            &tick_at_ms(STARTED_AT_MS + 1_000),
        );

        let readiness_ok = matches!(
            readiness,
            Readiness::NotDeclared | Readiness::PassAtThreshold | Readiness::PassAboveThreshold
        );
        let expected = (opt_out || stable_committed) && readiness_ok;
        let rows = backend_rows_healthy(&actions);
        prop_assert_eq!(rows, vec![expected], "actions: {:?}", actions);
        if unrelated_announced {
            prop_assert!(next.terminal_announced.contains(&aid("other-0")));
        }
    }
}
