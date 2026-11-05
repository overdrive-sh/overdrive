//! `WorkloadLifecycle::reconcile` at the restart ceiling — which give-up
//! terminal the final attempt's row carries.
//!
//! Feature `service-allocation-readiness-and-ending`:
//!
//! - D6 + B3 (S-15, S-45): a pre-startup crash loop (Service, declared startup
//!   probe, `Failed` + a guest-crash reason, startup not passed) gives up as
//!   `ServiceFailed { EarlyExit { exit_code } }`; `exit_code` is `None` for
//!   `VmOutOfMemory`.
//! - D7 (S-23): a startup-probe stop (`Terminated` + `Stopped { StartupProbe }`)
//!   gives up as `ServiceFailed { StartupProbeFailed { 0, last_fail,
//!   restart_counts } }`; `last_fail` is the total mapping over
//!   `current_startup_probe`.
//!
//! Port-to-port: the pure driving port `WorkloadLifecycle::reconcile`. The
//! oracle is the returned `Vec<Action>` and the returned `View`.
//!
//! The kind axis: a `Job` has no probe surface (ADR-0054 section 3), so the
//! Job x {liveness stop, startup stop} cells are not reachable in production and
//! are not generated; a Job crash is asserted to never carry a `ServiceFailed`
//! terminal.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::doc_markdown, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::guest_network::GuestAttachmentOccupancy;
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::GuestAttachmentObservation;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use proptest::prelude::*;

const WORKLOAD: &str = "payments";
const FINAL_ALLOC: &str = "alloc-payments-5";
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

/// The final attempt's row, as the production owners write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalRow {
    /// `Failed` + `WorkloadCrashedImmediately { exit_code: Some(1) }`.
    CrashWithExitCode,
    /// `Failed` + `WorkloadCrashedImmediately { exit_code: None }` (signal).
    CrashBySignal,
    /// `Failed` + `VmOutOfMemory` (B3).
    OutOfMemory,
    /// `Failed` + a start rejection (`DriverInternalError`).
    StartRejected,
    /// `Failed` + the mTLS fail-closed supersession.
    MtlsFailClosed,
    /// `Terminated` + `Stopped { Process }` (a clean exit).
    CleanExit,
    /// `Terminated` + `Stopped { LivenessProbe }`.
    LivenessStop,
    /// `Terminated` + `Stopped { StartupProbe }`.
    StartupStop,
    /// `Terminated` + `Stopped { PlatformReclaimed }`.
    PlatformReclaimed,
    /// `Terminated` + `Stopped { Operator }`.
    OperatorStop,
    /// `Terminated` + `Stopped { SystemGc }`.
    SystemGcStop,
}

fn final_row(kind: FinalRow) -> AllocStatusRow {
    let (state, reason, terminal) = match kind {
        FinalRow::CrashWithExitCode => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
            None,
        ),
        FinalRow::CrashBySignal => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: None,
                signal: Some(9),
                stderr_tail: None,
            }),
            None,
        ),
        FinalRow::OutOfMemory => (
            AllocState::Failed,
            Some(TransitionReason::VmOutOfMemory { limit_bytes: 134_217_728, oom_kill_count: 1 }),
            None,
        ),
        FinalRow::StartRejected => (
            AllocState::Failed,
            Some(TransitionReason::DriverInternalError { detail: "start rejected".to_owned() }),
            None,
        ),
        FinalRow::MtlsFailClosed => (
            AllocState::Failed,
            Some(TransitionReason::MtlsInterceptInstallFailed {
                stage: "inbound_tproxy".to_owned(),
                detail: "install failed".to_owned(),
            }),
            None,
        ),
        FinalRow::CleanExit => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Process }),
            Some(TerminalCondition::Stopped { by: StoppedBy::Process }),
        ),
        FinalRow::LivenessStop => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Reconciler }),
            Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        ),
        FinalRow::StartupStop => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Reconciler }),
            Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        ),
        FinalRow::PlatformReclaimed => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
            None,
        ),
        FinalRow::OperatorStop => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        ),
        FinalRow::SystemGcStop => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::SystemGc }),
            Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }),
        ),
    };
    row_with(state, reason, terminal)
}

fn row_with(
    state: AllocState,
    reason: Option<TransitionReason>,
    terminal: Option<TerminalCondition>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(FINAL_ALLOC),
        workload_id: jid(WORKLOAD),
        node_id: nid(NODE),
        state,
        updated_at: LogicalTimestamp { counter: 9, writer: nid(NODE) },
        reason,
        detail: None,
        terminal,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn job() -> Job {
    Job {
        id: jid(WORKLOAD),
        replicas: NonZeroU32::new(1).expect("non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Vm(Vm {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    }
}

fn startup_descriptor() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

/// The ONE place a `WorkloadLifecycleState` literal is built in this file.
fn state(
    kind: WorkloadKind,
    declared: bool,
    row: Option<AllocStatusRow>,
    current_startup_probe: Option<ProbeStatus>,
) -> WorkloadLifecycleState {
    let mut allocations = BTreeMap::new();
    if let Some(row) = row {
        allocations.insert(row.alloc_id.clone(), row);
    }
    WorkloadLifecycleState {
        workload_id: jid(WORKLOAD),
        job: Some(job()),
        desired_to_stop: false,
        generation: 0,
        nodes: BTreeMap::from([(
            nid(NODE),
            Node {
                id: nid(NODE),
                region: Region::new("local").expect("region"),
                capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
            },
        )]),
        allocations,
        workload_kind: kind,
        service_spec_digest: None,
        probe_descriptors: if declared { vec![startup_descriptor()] } else { Vec::new() },
        service_ports: Vec::new(),
        guest_attachments: GuestAttachmentObservation {
            occupancy: GuestAttachmentOccupancy { held: 0, retiring: 0 },
            leases: BTreeMap::new(),
        },
        current_startup_probe,
    }
}

fn desired_of(kind: WorkloadKind, declared: bool) -> WorkloadLifecycleState {
    state(kind, declared, None, None)
}

fn view_at(attempts: u32) -> WorkloadLifecycleView {
    WorkloadLifecycleView {
        restart_counts: BTreeMap::from([(aid(FINAL_ALLOC), attempts)]),
        ..WorkloadLifecycleView::default()
    }
}

fn tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        // Far past any backoff window.
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(10_000)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn finalised(actions: &[Action]) -> Vec<Option<TerminalCondition>> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FinalizeFailed { terminal, .. } => Some(terminal.clone()),
            _ => None,
        })
        .collect()
}

fn restarts(actions: &[Action]) -> usize {
    actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count()
}

/// What the ceiling decision must produce for a Service.
#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Give(TerminalCondition),
    Restart,
    Nothing,
}

fn probe_last_fail(probe: Option<&ProbeStatus>) -> String {
    match probe {
        Some(ProbeStatus::Fail { last_fail_reason }) => last_fail_reason.clone(),
        _ => String::new(),
    }
}

fn expected(declared: bool, row: FinalRow, probe: Option<&ProbeStatus>) -> Expect {
    let attempts = RESTART_BACKOFF_CEILING;
    let passed = matches!(probe, Some(ProbeStatus::Pass));
    let early = |exit_code: Option<i32>| {
        if declared && !passed {
            Expect::Give(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::EarlyExit { exit_code },
            })
        } else {
            Expect::Give(TerminalCondition::BackoffExhausted { attempts })
        }
    };
    match row {
        FinalRow::CrashWithExitCode => early(Some(1)),
        FinalRow::CrashBySignal | FinalRow::OutOfMemory => early(None),
        FinalRow::StartRejected | FinalRow::MtlsFailClosed | FinalRow::CleanExit => {
            Expect::Give(TerminalCondition::BackoffExhausted { attempts })
        }
        FinalRow::LivenessStop => Expect::Give(TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts },
        }),
        FinalRow::StartupStop => Expect::Give(TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: probe_last_fail(probe),
                attempts,
            },
        }),
        FinalRow::PlatformReclaimed => Expect::Restart,
        FinalRow::OperatorStop | FinalRow::SystemGcStop => Expect::Nothing,
    }
}

fn any_row() -> impl Strategy<Value = FinalRow> {
    prop_oneof![
        Just(FinalRow::CrashWithExitCode),
        Just(FinalRow::CrashBySignal),
        Just(FinalRow::OutOfMemory),
        Just(FinalRow::StartRejected),
        Just(FinalRow::MtlsFailClosed),
        Just(FinalRow::CleanExit),
        Just(FinalRow::LivenessStop),
        Just(FinalRow::StartupStop),
        Just(FinalRow::PlatformReclaimed),
        Just(FinalRow::OperatorStop),
        Just(FinalRow::SystemGcStop),
    ]
}

fn any_probe() -> impl Strategy<Value = Option<ProbeStatus>> {
    prop_oneof![
        Just(None),
        Just(Some(ProbeStatus::Pass)),
        Just(Some(ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() })),
    ]
}

fn check_service_ceiling(
    declared: bool,
    row: FinalRow,
    probe: Option<&ProbeStatus>,
) -> Result<(), TestCaseError> {
    // An opt-out Service declares no startup probe, so no startup stop is ever
    // authored for it: that cell is unreachable in production.
    prop_assume!(declared || row != FinalRow::StartupStop);
    let desired = desired_of(WorkloadKind::Service, declared);
    let actual = state(WorkloadKind::Service, declared, Some(final_row(row)), probe.cloned());
    let view = view_at(RESTART_BACKOFF_CEILING);
    let r = WorkloadLifecycle::canonical();

    let (actions, next) = r.reconcile(&desired, &actual, &view, &tick());

    match expected(declared, row, probe) {
        Expect::Give(terminal) => {
            prop_assert_eq!(
                finalised(&actions),
                vec![Some(terminal)],
                "row {:?} declared {} probe {:?}: {:?}",
                row,
                declared,
                probe,
                actions
            );
            prop_assert_eq!(restarts(&actions), 0);
            prop_assert_eq!(&next, &view, "the view is unchanged at the give-up");
        }
        Expect::Restart => {
            prop_assert_eq!(restarts(&actions), 1, "{:?}", actions);
            prop_assert!(finalised(&actions).is_empty());
            // R-1: the successor's count equals the predecessor's.
            let successor_counts: Vec<u32> = next
                .restart_counts
                .iter()
                .filter(|(id, _)| id.as_str() != FINAL_ALLOC)
                .map(|(_, n)| *n)
                .collect();
            prop_assert_eq!(successor_counts, vec![RESTART_BACKOFF_CEILING]);
        }
        Expect::Nothing => {
            prop_assert_eq!(restarts(&actions), 0, "{:?}", actions);
            prop_assert!(finalised(&actions).is_empty(), "{:?}", actions);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-15 + S-23 + S-45 (D6, D7, B3). The ceiling terminal over
    /// declared startup probe x final row x `current_startup_probe`, for a
    /// Service: `EarlyExit { exit_code }` iff declared, `Failed` + a guest-crash
    /// reason, not passed (`None` exit code for out-of-memory and signals);
    /// `LivenessProbeFailed` for the liveness stop; `StartupProbeFailed { 0,
    /// last_fail, restart_counts }` for the startup stop (`last_fail` is the
    /// total mapping, the empty string for `Some(Pass)` and `None`); otherwise
    /// `BackoffExhausted`; a platform-reclaimed row restarts with the
    /// predecessor's count and no terminal; operator and system-gc stops are
    /// neither restarted nor finalised; the returned view equals the input
    /// view. The cases are pairwise exclusive by construction (one `Expect`
    /// per input).
    ///
    /// Mutation target: each condition of the pre-startup-crash predicate
    /// removed or negated in turn, always choosing `BackoffExhausted`, the
    /// order and each case of the selection, the `current_startup_probe`
    /// population, the `VmOutOfMemory` arm.
    #[test]
    #[ignore = "pending DELIVER: D6 D7 B3 the restart authority names the give-up cause"]
    fn ceiling_terminal_names_the_final_attempts_cause(
        declared in any::<bool>(),
        row in any_row(),
        probe in any_probe(),
    ) {
        check_service_ceiling(declared, row, probe.as_ref())?;
    }

    /// S-15 control. The cells whose ceiling decision does not change in this
    /// feature (every cell except a pre-startup crash and a startup stop) keep
    /// their current terminal. Passes at HEAD.
    #[test]
    fn ceiling_terminal_of_unclassified_rows_is_unchanged(
        declared in any::<bool>(),
        row in prop_oneof![
            Just(FinalRow::StartRejected), Just(FinalRow::MtlsFailClosed),
            Just(FinalRow::CleanExit), Just(FinalRow::LivenessStop),
            Just(FinalRow::PlatformReclaimed), Just(FinalRow::OperatorStop),
            Just(FinalRow::SystemGcStop)
        ],
        probe in any_probe(),
    ) {
        check_service_ceiling(declared, row, probe.as_ref())?;
    }

    /// S-15 (Job kind). A Job crash at the ceiling never carries a
    /// `ServiceFailed` terminal.
    #[test]
    fn job_crash_at_the_ceiling_never_names_a_service_failure(
        row in prop_oneof![
            Just(FinalRow::CrashWithExitCode), Just(FinalRow::CrashBySignal),
            Just(FinalRow::OutOfMemory), Just(FinalRow::StartRejected)
        ],
        probe in any_probe(),
    ) {
        let desired = desired_of(WorkloadKind::Job, false);
        let actual = state(WorkloadKind::Job, false, Some(final_row(row)), probe);
        let (actions, _) = WorkloadLifecycle::canonical()
            .reconcile(&desired, &actual, &view_at(RESTART_BACKOFF_CEILING), &tick());
        for terminal in finalised(&actions) {
            prop_assert!(
                !matches!(terminal, Some(TerminalCondition::ServiceFailed { .. })),
                "a Job never gives up as a ServiceFailed: {:?}", actions
            );
        }
    }

    /// S-15 / S-23 (below the ceiling). The restart decision is a function of
    /// the same inputs as at HEAD: below the ceiling a restartable row yields
    /// `RestartAllocation` identically with and without the classification
    /// input, and no terminal. Passes at HEAD (the input is ignored).
    #[test]
    fn restart_below_the_ceiling_ignores_the_classification_inputs(
        declared in any::<bool>(),
        row in prop_oneof![
            Just(FinalRow::CrashWithExitCode), Just(FinalRow::OutOfMemory),
            Just(FinalRow::StartupStop), Just(FinalRow::LivenessStop), Just(FinalRow::CleanExit)
        ],
        probe in any_probe(),
        attempts in 0..RESTART_BACKOFF_CEILING,
    ) {
        let desired = desired_of(WorkloadKind::Service, declared);
        let view = view_at(attempts);
        let r = WorkloadLifecycle::canonical();
        let with = state(WorkloadKind::Service, declared, Some(final_row(row)), probe);
        let without = state(WorkloadKind::Service, declared, Some(final_row(row)), None);
        let (a, va) = r.reconcile(&desired, &with, &view, &tick());
        let (b, vb) = r.reconcile(&desired, &without, &view, &tick());
        prop_assert_eq!(restarts(&a), 1, "{:?}", a);
        prop_assert!(finalised(&a).is_empty());
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(&va, &vb);
    }
}

/// S-15 / S-23 (idempotency). A row already carrying a give-up terminal emits
/// nothing: not `BackoffExhausted`, not `LivenessProbeFailed`, and, after D6 and
/// D7, not `EarlyExit` and not `StartupProbeFailed`.
#[test]
#[ignore = "pending DELIVER: D6 D7 the idempotency guard also matches EarlyExit and StartupProbeFailed"]
fn an_already_finalised_row_is_not_finalised_again() {
    let finalised_terminals = [
        TerminalCondition::BackoffExhausted { attempts: RESTART_BACKOFF_CEILING },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed {
                probe_idx: 0,
                attempts: RESTART_BACKOFF_CEILING,
            },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: "connection refused".to_owned(),
                attempts: RESTART_BACKOFF_CEILING,
            },
        },
    ];
    for terminal in finalised_terminals {
        let mut row = final_row(FinalRow::CrashWithExitCode);
        row.terminal = Some(terminal.clone());
        let desired = desired_of(WorkloadKind::Service, true);
        let actual = state(WorkloadKind::Service, true, Some(row), None);
        let view = view_at(RESTART_BACKOFF_CEILING);
        let (actions, next) =
            WorkloadLifecycle::canonical().reconcile(&desired, &actual, &view, &tick());
        assert!(
            actions.is_empty(),
            "a row carrying {terminal:?} must emit nothing; got {actions:?}"
        );
        assert_eq!(next, view);
    }
}
