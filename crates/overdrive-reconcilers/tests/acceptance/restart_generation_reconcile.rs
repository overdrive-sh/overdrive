//! Restart generation at `WorkloadLifecycle::reconcile` (ADR-0335, ADR-0337,
//! ADR-0338; node-local persistent volume, GH #348; scenario family
//! S-BIR-GENERATION).
//!
//! A generation is handled once the allocation issued for it has a row that ran.
//! The View records what was issued, never that a launch happened, so a
//! launch that ends with no row (superseded, or lost with the control plane)
//! is relaunched by the next evaluation, once, and a relaunch of a generation
//! replacement charges nothing.
//!
//! CONTRACT_SHAPE: unbounded-preservation (proptest over the unbounded
//! domain: restart counts, generations, predecessor classes) driven through
//! the real `Reconciler::reconcile` of `WorkloadLifecycle`. Every multi-step
//! scenario chains the evaluations the way the runtime does: the second
//! evaluation reads the `next_view` the first returned, so the View is
//! authored by the reconciler, never seeded with a state production would not
//! author. The only injected fault is the lost launch, i.e. the allocation row
//! the dispatch would have written is simply absent from the second evaluation's
//! observation (a superseded launch, a control-plane crash and a node shutdown
//! all leave exactly that). Oracle: the returned actions and the returned View.
//! Tier: Tier 1, default lane (pure, no I/O).
//!
//! Scenarios:
//! - a lost launch is relaunched once for the pending generation
//!   (`a_lost_launch_is_relaunched_once_for_the_pending_generation`), and an
//!   abandoned replacement is relaunched uncharged
//!   (`an_abandoned_replacement_relaunch_charges_nothing`);
//! - a row present for the issued id issues nothing
//!   (`an_evaluation_with_the_issued_ids_row_present_issues_nothing`); N bumps
//!   before any placement coalesce into one placement and an evaluation after
//!   the successor's row exists does not re-place
//!   (`bumps_before_the_placement_coalesce_into_one_placement`,
//!   `an_evaluation_after_the_successors_row_exists_does_not_replace`);
//! - a crash-restart relaunch assigns the same count the abandoned issue assigned
//!   (`a_relaunched_crash_restart_is_charged_once_not_twice`);
//! - a finished Job is never given a new allocation, an operator-stopped Job
//!   included (`a_restart_never_gives_a_completed_or_failed_job_a_new_allocation`,
//!   `a_restart_never_gives_an_operator_stopped_job_a_new_allocation`), whichever
//!   writer recorded the stop, so the exit observer's terminal-less row too (H3,
//!   `a_restart_never_gives_an_exit_observer_stopped_job_a_new_allocation`); the same
//!   row for a Service is replaced
//!   (`an_exit_observer_stopped_service_is_replaced_by_a_restart`);
//! - a first placement records the committed generation
//!   (`a_first_placement_records_the_committed_generation`).
//!
//! Mutation targets killed (`workload_lifecycle.rs`):
//! - the helper's `<` (an `<=` mutant leaves a handled generation pending and
//!   makes `an_evaluation_with_the_issued_ids_row_present_issues_nothing` emit a
//!   stop; a `>` mutant breaks every pending test above): the present-row test
//!   and the lost-launch test;
//! - `desired.generation` written into `issued_generation` at both sites: the
//!   restart arm by `a_lost_launch_is_relaunched_once_for_the_pending_generation`
//!   (the new id's entry), the placement arm by
//!   `a_first_placement_records_the_committed_generation`;
//! - the carry of `restart_counts` (and `last_failure_seen_at`) for a generation
//!   replacement: `an_abandoned_replacement_relaunch_charges_nothing` and the
//!   lost-launch test, against the crash-restart charge of
//!   `a_relaunched_crash_restart_is_charged_once_not_twice`.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::guest_network::GuestAttachmentOccupancy;
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::GuestAttachmentObservation;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use proptest::prelude::*;

const WORKLOAD: &str = "payments";

fn aid(suffix: u32) -> AllocationId {
    AllocationId::new(&format!("alloc-{WORKLOAD}-{suffix}")).expect("valid allocation id")
}

fn wid() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("valid workload id")
}

fn nid() -> NodeId {
    NodeId::new("local").expect("valid node id")
}

const fn instant(seconds: u64) -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(seconds))
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// `Failed`, crash reason, no terminal.
    Crashed,
    /// `Terminated`, `Stopped { Operator }` reason and terminal.
    OperatorStopped,
    /// `Terminated`, `Stopped { Operator }` reason only (no terminal): the exit observer's
    /// row, which stands when both of the stop path's write proposals are rejected under
    /// contention (the action shim's double rejection in the `StopAllocation` arm of
    /// `action_shim/mod.rs`, then the exit observer's `terminal: None` write in
    /// `worker/exit_observer.rs`).
    OperatorStoppedReasonOnly,
    /// `Terminated`, terminal `Completed`.
    Completed,
    /// `Failed`, terminal `Failed`.
    JobFailed,
    Running,
    Draining,
}

fn row(suffix: u32, kind: WorkloadKind, class: Class) -> AllocStatusRow {
    let crash = || {
        Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(23),
            signal: None,
            stderr_tail: None,
        })
    };
    let (state, reason, terminal) = match class {
        Class::Crashed => (AllocState::Failed, crash(), None),
        Class::OperatorStopped => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        ),
        Class::OperatorStoppedReasonOnly => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
            None,
        ),
        Class::Completed => {
            (AllocState::Terminated, None, Some(TerminalCondition::Completed { exit_code: 0 }))
        }
        Class::JobFailed => {
            (AllocState::Failed, crash(), Some(TerminalCondition::Failed { exit_code: Some(1) }))
        }
        Class::Running => (AllocState::Running, None, None),
        Class::Draining => (AllocState::Draining, None, None),
    };
    AllocStatusRow {
        alloc_id: aid(suffix),
        workload_id: wid(),
        node_id: nid(),
        state,
        updated_at: LogicalTimestamp { counter: u64::from(suffix) + 1, writer: nid() },
        reason,
        detail: None,
        terminal,
        stderr_tail: None,
        kind,
        listeners: Vec::new(),
        started_at: Some(instant(10)),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

// ---------------------------------------------------------------------------
// One evaluation through the real reconciler
// ---------------------------------------------------------------------------

fn workload_job() -> Job {
    Job {
        id: wid(),
        replicas: NonZeroU32::new(1).expect("one replica"),
        resources: Resources {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(64 * 1024 * 1024),
        },
        driver: WorkloadDriver::Vm(Vm {
            volume: None,
            command: "/sbin/workload".to_owned(),
            args: Vec::new(),
            kernel: "/srv/vm/kernel".to_owned(),
            rootfs: "/srv/vm/rootfs.ext4".to_owned(),
        }),
    }
}

fn node() -> Node {
    Node {
        id: nid(),
        region: Region::new("local").expect("valid region"),
        capacity: Resources {
            cpu: CpuQuantity::from_millis(64_000),
            memory: ByteSize::new(256 * 1024 * 1024 * 1024),
        },
    }
}

struct Evaluation {
    actions: Vec<Action>,
    view: WorkloadLifecycleView,
}

impl Evaluation {
    /// The allocation ids the evaluation issued: the successor of each
    /// `StartAllocation` / `RestartAllocation` (`spec.alloc`).
    fn issued(&self) -> Vec<AllocationId> {
        self.actions
            .iter()
            .filter_map(|action| match action {
                Action::StartAllocation { spec, .. } | Action::RestartAllocation { spec, .. } => {
                    Some(spec.alloc.clone())
                }
                _ => None,
            })
            .collect()
    }
}

fn evaluate(
    kind: WorkloadKind,
    rows: &[AllocStatusRow],
    view: &WorkloadLifecycleView,
    generation: u64,
    seconds: u64,
) -> Evaluation {
    let nodes = BTreeMap::from([(nid(), node())]);
    let observation = GuestAttachmentObservation {
        occupancy: GuestAttachmentOccupancy { held: 0, retiring: 0 },
        leases: BTreeMap::new(),
    };
    let desired = WorkloadLifecycleState {
        current_startup_probe: None,
        workload_id: wid(),
        job: Some(workload_job()),
        desired_to_stop: false,
        generation,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        declared_listeners: Vec::new(),
        guest_attachments: observation.clone(),
    };
    let actual = WorkloadLifecycleState {
        current_startup_probe: None,
        workload_id: wid(),
        job: Some(workload_job()),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations: rows.iter().cloned().map(|row| (row.alloc_id.clone(), row)).collect(),
        workload_kind: kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        declared_listeners: Vec::new(),
        guest_attachments: observation,
    };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: instant(seconds),
        tick: seconds,
        deadline: now + Duration::from_secs(1),
    };
    let (actions, view) = WorkloadLifecycle::canonical().reconcile(&desired, &actual, view, &tick);
    Evaluation { actions, view }
}

/// The View of a predecessor `alloc-payments-0` the reconciler issued for
/// `issued_for` having failed `attempts` times, failure seen at second 1.
fn predecessor_view(attempts: u32, issued_for: u64, seen: bool) -> WorkloadLifecycleView {
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(aid(0), attempts);
    view.issued_generation.insert(aid(0), issued_for);
    if seen {
        view.last_failure_seen_at.insert(aid(0), instant(1));
    }
    view
}

// ---------------------------------------------------------------------------
// (i) A lost launch is relaunched once for the pending generation
// ---------------------------------------------------------------------------

/// What the relaunch of a lost generation replacement did, in one comparable value.
#[derive(Debug, PartialEq, Eq)]
struct Relaunch {
    /// Ids issued by the first evaluation.
    first_issued: Vec<AllocationId>,
    /// Ids issued by the second evaluation (the launch left no row).
    second_issued: Vec<AllocationId>,
    /// `issued_generation` of the second id.
    entry: Option<u64>,
    /// `restart_counts` of the second id.
    count: Option<u32>,
    /// `last_failure_seen_at` of the second id.
    seen: Option<UnixInstant>,
    /// The abandoned id's entries stay as the reservation.
    reservation_kept: bool,
}

fn relaunch_after_a_lost_launch(
    class: Class,
    attempts: u32,
    previous_generation: u64,
    generation: u64,
) -> Relaunch {
    let rows = [row(0, WorkloadKind::Service, class)];
    let seen = class == Class::Crashed;
    let initial = predecessor_view(attempts, previous_generation, seen);
    let first = evaluate(WorkloadKind::Service, &rows, &initial, generation, 100);
    // The launch of the first id ends with no row: the rows are unchanged.
    let second = evaluate(WorkloadKind::Service, &rows, &first.view, generation, 200);
    let second_id = aid(2);
    Relaunch {
        first_issued: first.issued(),
        second_issued: second.issued(),
        entry: second.view.issued_generation.get(&second_id).copied(),
        count: second.view.restart_counts.get(&second_id).copied(),
        seen: second.view.last_failure_seen_at.get(&second_id).copied(),
        reservation_kept: second.view.issued_generation.get(&aid(1)) == Some(&generation)
            && second.view.restart_counts.contains_key(&aid(1)),
    }
}

proptest! {
    /// With the issued id's row ABSENT (the View holds the first evaluation's
    /// issued entry, the rows show only the older predecessor) a second
    /// evaluation with the generation still pending issues exactly one new id,
    /// whose `issued_generation` entry is `desired.generation`; a generation
    /// replacement carries the predecessor's `restart_counts` and
    /// `last_failure_seen_at` unchanged, and the abandoned id's entries stay as
    /// the reservation.
    #[test]
    fn a_lost_launch_is_relaunched_once_for_the_pending_generation(
        class in prop_oneof![Just(Class::Crashed), Just(Class::OperatorStopped)],
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
    ) {
        let generation = previous_generation + bumps;
        let observed = relaunch_after_a_lost_launch(class, attempts, previous_generation, generation);
        let expected = Relaunch {
            first_issued: vec![aid(1)],
            second_issued: vec![aid(2)],
            entry: Some(generation),
            count: Some(attempts),
            seen: (class == Class::Crashed).then(|| instant(1)),
            reservation_kept: true,
        };
        prop_assert_eq!(
            observed,
            expected,
            "a second evaluation with the issued id's row absent must issue exactly one new id \
             for the pending generation, record desired.generation for it, and carry the \
             predecessor's restart count and failure time unchanged"
        );
    }

    /// The carry itself, on a crashed predecessor with budget spent: the
    /// relaunch of an abandoned generation replacement charges nothing, whatever
    /// the count (the ceiling does not apply to a generation replacement).
    #[test]
    fn an_abandoned_replacement_relaunch_charges_nothing(
        attempts in 1_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
    ) {
        let generation = previous_generation + bumps;
        let observed =
            relaunch_after_a_lost_launch(Class::Crashed, attempts, previous_generation, generation);
        prop_assert_eq!(
            (observed.count, observed.seen),
            (Some(attempts), Some(instant(1))),
            "an abandoned generation replacement must be relaunched with the predecessor's \
             restart count and failure time"
        );
    }
}

// ---------------------------------------------------------------------------
// (ii) With the issued id's row present, an evaluation issues nothing
// ---------------------------------------------------------------------------

proptest! {
    /// The issued id's row exists and serves the committed generation:
    /// the evaluation emits nothing and gains no View entry. Kills the helper's
    /// `<` (an `<=` mutant reads the equal entry as pending and stops the
    /// `Running` instance).
    #[test]
    fn an_evaluation_with_the_issued_ids_row_present_issues_nothing(
        predecessor in prop_oneof![Just(Class::Crashed), Just(Class::OperatorStopped)],
        successor in prop_oneof![Just(Class::Running), Just(Class::Draining)],
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
    ) {
        let generation = previous_generation + bumps;
        let rows = [row(0, WorkloadKind::Service, predecessor)];
        let initial = predecessor_view(attempts, previous_generation, predecessor == Class::Crashed);
        let first = evaluate(WorkloadKind::Service, &rows, &initial, generation, 100);
        prop_assert_eq!(first.issued().len(), 1, "the first evaluation issues the replacement");
        // The dispatch wrote the successor's row.
        let rows = [
            row(0, WorkloadKind::Service, predecessor),
            row(1, WorkloadKind::Service, successor),
        ];
        let second = evaluate(WorkloadKind::Service, &rows, &first.view, generation, 200);
        prop_assert!(
            second.issued().is_empty(),
            "an evaluation with the issued id's row present must issue nothing, got {:?}",
            second.actions
        );
        prop_assert_eq!(
            second.actions.iter().filter(|a| !matches!(a, Action::EnqueueEvaluation { .. })).count(),
            0,
            "an evaluation with the issued id's row present must emit no allocation action"
        );
        prop_assert_eq!(
            (second.view.issued_generation.keys().collect::<Vec<_>>(),
             second.view.restart_counts.keys().collect::<Vec<_>>()),
            (first.view.issued_generation.keys().collect::<Vec<_>>(),
             first.view.restart_counts.keys().collect::<Vec<_>>()),
            "an evaluation with the issued id's row present must gain no View entry"
        );
    }
}

// ---------------------------------------------------------------------------
// (iii) Coalescing
// ---------------------------------------------------------------------------

proptest! {
    /// S-BIR-COALESCE-PLACE. N operator `restart` commits
    /// before any placement leave the generation N above the predecessor's
    /// entry; one evaluation places one successor, which serves the final
    /// generation, never one placement per bump.
    #[test]
    fn bumps_before_the_placement_coalesce_into_one_placement(
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=5,
    ) {
        let generation = previous_generation + bumps;
        let rows = [row(0, WorkloadKind::Service, Class::OperatorStopped)];
        let initial = predecessor_view(attempts, previous_generation, false);
        let evaluation = evaluate(WorkloadKind::Service, &rows, &initial, generation, 100);
        prop_assert_eq!(
            evaluation.issued(),
            vec![aid(1)],
            "N bumps before the placement must coalesce into exactly one placement"
        );
        prop_assert_eq!(
            evaluation.view.issued_generation.get(&aid(1)).copied(),
            Some(generation),
            "the one placement serves the final committed generation"
        );
    }

    /// S-BIR-COALESCE-NO-REPLAY. An evaluation after the
    /// successor's row exists does not place again for the same bumps.
    #[test]
    fn an_evaluation_after_the_successors_row_exists_does_not_replace(
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=5,
        evaluations in 1_usize..=4,
    ) {
        let generation = previous_generation + bumps;
        let predecessor = row(0, WorkloadKind::Service, Class::OperatorStopped);
        let initial = predecessor_view(attempts, previous_generation, false);
        let placed = evaluate(WorkloadKind::Service, std::slice::from_ref(&predecessor), &initial, generation, 100);
        prop_assert_eq!(placed.issued(), vec![aid(1)], "the bumps coalesce into one placement");
        let rows = [predecessor, row(1, WorkloadKind::Service, Class::Running)];
        let mut view = placed.view;
        for step in 0..evaluations {
            let evaluation = evaluate(WorkloadKind::Service, &rows, &view, generation, 200 + step as u64);
            prop_assert!(
                evaluation.issued().is_empty(),
                "an evaluation after the successor's row exists must not place again, got {:?}",
                evaluation.actions
            );
            view = evaluation.view;
        }
    }
}

// ---------------------------------------------------------------------------
// (iv) Budget: a crash-restart relaunch is charged once
// ---------------------------------------------------------------------------

proptest! {
    /// A crash-restart successor whose launch left no row is
    /// relaunched by the next evaluation with the SAME restart count the
    /// abandoned issue assigned (the predecessor's plus one, computed from the
    /// still-current predecessor), so one crash is charged once, not twice. The
    /// predecessor's own count is untouched. Contrasts with the uncharged
    /// generation replacement above.
    #[test]
    fn a_relaunched_crash_restart_is_charged_once_not_twice(
        attempts in 0_u32..RESTART_BACKOFF_CEILING,
        generation in 0_u64..=3,
    ) {
        let rows = [row(0, WorkloadKind::Service, Class::Crashed)];
        // The predecessor was issued for the committed generation: no generation is pending.
        let initial = predecessor_view(attempts, generation, true);
        let first = evaluate(WorkloadKind::Service, &rows, &initial, generation, 100);
        let second = evaluate(WorkloadKind::Service, &rows, &first.view, generation, 200);
        prop_assert_eq!(
            (first.issued(), second.issued()),
            (vec![aid(1)], vec![aid(2)]),
            "each evaluation with the predecessor still current issues one crash-restart successor"
        );
        prop_assert_eq!(
            (
                first.view.restart_counts.get(&aid(1)).copied(),
                second.view.restart_counts.get(&aid(2)).copied(),
                second.view.restart_counts.get(&aid(0)).copied(),
            ),
            (Some(attempts + 1), Some(attempts + 1), Some(attempts)),
            "a relaunched crash-restart must be charged once: the same count the abandoned issue \
             assigned, with the predecessor's count unchanged"
        );
    }
}

// ---------------------------------------------------------------------------
// The placement arm records the committed generation
// ---------------------------------------------------------------------------

proptest! {
    /// A first placement (no row at all) records the
    /// committed generation it serves, so the generation is not pending against
    /// its own successor. Kills the placement arm's `desired.generation` write.
    #[test]
    fn a_first_placement_records_the_committed_generation(
        generation in 0_u64..=6,
        kind in prop_oneof![Just(WorkloadKind::Service), Just(WorkloadKind::Job)],
    ) {
        let evaluation = evaluate(kind, &[], &WorkloadLifecycleView::default(), generation, 100);
        prop_assert_eq!(evaluation.issued(), vec![aid(0)], "a first placement issues one id");
        prop_assert_eq!(
            (
                evaluation.view.issued_generation.get(&aid(0)).copied(),
                evaluation.view.restart_counts.get(&aid(0)).copied(),
            ),
            (Some(generation), Some(0)),
            "a first placement must record the committed generation it serves and a zero count"
        );
    }
}

// ---------------------------------------------------------------------------
// (v) A finished Job is never given a new allocation by a restart (H1)
// ---------------------------------------------------------------------------

/// What many evaluations did to a Job whose current row is fixed: the number of
/// allocation ids issued and whether the View gained any entry.
#[derive(Debug, PartialEq, Eq)]
struct JobTrajectory {
    issued: usize,
    gained_issued_entry: bool,
    gained_count_entry: bool,
}

fn job_trajectory(
    class: Class,
    attempts: u32,
    previous_generation: u64,
    bumps: u64,
    ticks: u64,
) -> JobTrajectory {
    let generation = previous_generation + bumps;
    let rows = [row(0, WorkloadKind::Job, class)];
    let initial = predecessor_view(attempts, previous_generation, false);
    let mut view = initial.clone();
    let mut issued = 0;
    for tick in 0..ticks {
        let evaluation = evaluate(WorkloadKind::Job, &rows, &view, generation, 100 + tick);
        issued += evaluation.issued().len();
        view = evaluation.view;
    }
    JobTrajectory {
        issued,
        gained_issued_entry: view.issued_generation != initial.issued_generation,
        gained_count_entry: view.restart_counts != initial.restart_counts,
    }
}

const NO_NEW_ALLOCATION: JobTrajectory =
    JobTrajectory { issued: 0, gained_issued_entry: false, gained_count_entry: false };

proptest! {
    /// Across many ticks, with the
    /// generation bumped, no allocation action is emitted, no id is issued, and
    /// `issued_generation` and `restart_counts` gain no entry. Kills the finished-
    /// Job exception of the helper once the reconciler decides with it.
    #[test]
    fn a_restart_never_gives_a_completed_or_failed_job_a_new_allocation(
        class in prop_oneof![Just(Class::Completed), Just(Class::JobFailed)],
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
        ticks in 1_u64..=8,
    ) {
        prop_assert_eq!(
            job_trajectory(class, attempts, previous_generation, bumps, ticks),
            NO_NEW_ALLOCATION,
            "a Job whose current row carries a terminal must never be given a new allocation \
             by a restart"
        );
    }

    /// An operator-stopped Job whose row carries `Stopped { by: Operator }`
    /// as its terminal is a finished Job: `workload restart` bumps the generation
    /// and the reconciler issues nothing (the action shim would return at its
    /// fence with no successor, leaving the generation never handled).
    #[test]
    fn a_restart_never_gives_an_operator_stopped_job_a_new_allocation(
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
        ticks in 1_u64..=8,
    ) {
        prop_assert_eq!(
            job_trajectory(Class::OperatorStopped, attempts, previous_generation, bumps, ticks),
            NO_NEW_ALLOCATION,
            "an operator-stopped Job whose row carries a terminal must never be given a new \
             allocation by a restart"
        );
    }

    /// H3: a Job whose newest row that ran is the exit observer's operator-stop row
    /// (`Terminated`, reason `Stopped { Operator }`, no terminal) is a finished Job. The
    /// production path that produces the shape: the action shim's `StopAllocation` arm
    /// proposes the terminal row twice and both proposals are rejected under contention
    /// (`action_shim/mod.rs`), so the exit observer's row (`terminal: None`,
    /// `worker/exit_observer.rs`) stands. Driven the way `overdrive workload restart` does:
    /// the generation is bumped and the stop sentinel is cleared (the reconciler's desired
    /// state has `desired_to_stop: false`). No `StartAllocation` / `RestartAllocation`, no id
    /// issued, no View entry gained.
    #[test]
    fn a_restart_never_gives_an_exit_observer_stopped_job_a_new_allocation(
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
        ticks in 1_u64..=8,
    ) {
        prop_assert_eq!(
            job_trajectory(
                Class::OperatorStoppedReasonOnly,
                attempts,
                previous_generation,
                bumps,
                ticks,
            ),
            NO_NEW_ALLOCATION,
            "an operator-stopped Job row recorded by the exit observer (no terminal) must never \
             be given a new allocation by a restart"
        );
    }

    /// H3 applies to Jobs only: the same row for a Service is replaced by a restart, one
    /// uncharged replacement serving the generation.
    #[test]
    fn an_exit_observer_stopped_service_is_replaced_by_a_restart(
        attempts in 0_u32..=RESTART_BACKOFF_CEILING,
        previous_generation in 0_u64..=2,
        bumps in 1_u64..=3,
    ) {
        let generation = previous_generation + bumps;
        let rows = [row(0, WorkloadKind::Service, Class::OperatorStoppedReasonOnly)];
        let initial = predecessor_view(attempts, previous_generation, false);
        let evaluation = evaluate(WorkloadKind::Service, &rows, &initial, generation, 100);
        prop_assert_eq!(
            (
                evaluation.issued(),
                evaluation.view.issued_generation.get(&aid(1)).copied(),
                evaluation.view.restart_counts.get(&aid(1)).copied(),
            ),
            (vec![aid(1)], Some(generation), Some(attempts)),
            "an operator-stopped Service row with no terminal must be replaced once, uncharged, \
             for the generation"
        );
    }
}
