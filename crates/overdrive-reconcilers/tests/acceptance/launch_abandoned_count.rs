//! A launch that never ran does not count (ADR-0355 sections 1 and 2, RB-13; node-local
//! persistent volume, GH #348).
//!
//! The restart count of a new launch is derived from the **count basis**, the newest row that
//! ran (`last_ran_alloc`): a crash restart carries the basis row's count plus one, a generation
//! replacement the basis row's count unchanged, a fresh placement (no row ran) zero. A launch
//! that ended before it ran (an abandoned row: `Terminated` with `LaunchSuperseded` or
//! `LaunchLostWithControlPlane`) stays in history and is still the workload's current
//! allocation, but no derivation reads it: not the count, not the backoff gate, not the ceiling
//! decision, not the `FinalizeFailed` target, not a Job's natural-exit test.
//!
//! CONTRACT_SHAPE: unbounded-preservation (the deletion-equivalence property over generated
//! chains) plus bounded-change examples, driven through the real `Reconciler::reconcile` of
//! `WorkloadLifecycle` over hand-built `AllocStatusRow` chains. Multi-launch scenarios chain
//! the evaluations the way the runtime does: each evaluation reads the `next_view` the previous
//! one returned, so the View is authored by the reconciler; the only injected fact is the row
//! the dispatch would have written (an abandoned launch ends as one abandoned row). Oracle: the
//! returned actions and the returned View. Tier: Tier 1, default lane (pure, no I/O).
//!
//! Classification. The count scenarios, the generation-replacement relaunch, the Job scenario,
//! the `last_ran_alloc` units, the backend-membership guard and the deletion-equivalence
//! property pass against the reconciler. The pending-generation agreement clause is a separate
//! property and its stamp-on-emit example (an operator-stopped basis followed by an abandoned
//! replacement issued under the committed generation) still PENDING (proven RED,
//! `#[should_panic]` on the assertion in their own bodies): the function is a scaffold that
//! panics until the generation rule is implemented.
//!
//! Mutation targets (`workload_lifecycle.rs`): the `!is_launch_abandoned` filter of
//! `last_ran_alloc` (`last_ran_skips_abandoned_rows...`), every read of the count basis at the
//! derivation sites (the deletion-equivalence property and the three count scenarios), and
//! `&& !is_launch_abandoned(row)` in `is_natural_exit` (the Job test).

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::cast_possible_truncation
)]

use std::any::Any;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_core::guest_network::GuestAttachmentOccupancy;
use overdrive_core::id::{AllocationId, NodeId, Region, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::GuestAttachmentObservation;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    StoppedBy, TerminalCondition, TransitionReason, is_launch_abandoned,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::ServiceAllocPhase;
use overdrive_reconcilers::workload_lifecycle::last_ran_alloc;
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
    current_alloc, generation_replacement_pending,
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
    /// `Failed`, crash reason, no terminal: a launch that ran and crashed.
    Crashed,
    /// `Terminated`, `Stopped { PlatformReclaimed }` reason, no terminal: a launch that ran and
    /// was reclaimed by the platform.
    PlatformReclaimed,
    /// `Terminated`, `Stopped { Operator }` reason and terminal.
    OperatorStopped,
    /// `Terminated`, terminal `Completed`.
    Completed,
    /// `Failed`, terminal `Failed`.
    JobFailed,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Abandon {
    Superseded,
    LostWithControlPlane,
}

impl Abandon {
    const BOTH: [Self; 2] = [Self::Superseded, Self::LostWithControlPlane];

    const fn reason(self) -> TransitionReason {
        match self {
            Self::Superseded => TransitionReason::LaunchSuperseded,
            Self::LostWithControlPlane => TransitionReason::LaunchLostWithControlPlane,
        }
    }
}

fn base(suffix: u32, kind: WorkloadKind) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(suffix),
        workload_id: wid(),
        node_id: nid(),
        state: AllocState::Pending,
        updated_at: LogicalTimestamp { counter: u64::from(suffix) + 1, writer: nid() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A launch that ran.
fn ran(suffix: u32, kind: WorkloadKind, class: Class) -> AllocStatusRow {
    let mut row = base(suffix, kind);
    row.started_at = Some(instant(10));
    match class {
        Class::Crashed => {
            row.state = AllocState::Failed;
            row.reason = Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(23),
                signal: None,
                stderr_tail: None,
            });
        }
        Class::PlatformReclaimed => {
            row.state = AllocState::Terminated;
            row.reason = Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed });
        }
        Class::OperatorStopped => {
            row.state = AllocState::Terminated;
            row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
            row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
        }
        Class::Completed => {
            row.state = AllocState::Terminated;
            row.terminal = Some(TerminalCondition::Completed { exit_code: 0 });
        }
        Class::JobFailed => {
            row.state = AllocState::Failed;
            row.terminal = Some(TerminalCondition::Failed { exit_code: Some(1) });
        }
        Class::Running => row.state = AllocState::Running,
    }
    row
}

/// A launch that ended before it ran: `Terminated`, no terminal claim, never started, the
/// `Pending` row's address carried (ADR-0355 section 1).
fn abandoned(suffix: u32, kind: WorkloadKind, why: Abandon) -> AllocStatusRow {
    let mut row = base(suffix, kind);
    row.state = AllocState::Terminated;
    row.reason = Some(why.reason());
    row.workload_addr = Some(Ipv4Addr::new(10, 99, 128, 2));
    row
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
    /// The allocation ids the evaluation issued (`spec.alloc` of each start or restart).
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

    fn finalised(&self) -> Vec<(AllocationId, Option<TerminalCondition>)> {
        self.actions
            .iter()
            .filter_map(|action| match action {
                Action::FinalizeFailed { alloc_id, terminal } => {
                    Some((alloc_id.clone(), terminal.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn count_of(&self, id: &AllocationId) -> Option<u32> {
        self.view.restart_counts.get(id).copied()
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

/// The View of a crashed predecessor `alloc-payments-0` the reconciler issued for generation 0,
/// having failed `attempts` times, failure seen at second 1.
fn crashed_predecessor_view(attempts: u32) -> WorkloadLifecycleView {
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(aid(0), attempts);
    view.issued_generation.insert(aid(0), 0);
    view.last_failure_seen_at.insert(aid(0), instant(1));
    view
}

// ---------------------------------------------------------------------------
// (1) A crash restart abandoned by an operator restart forgives the crash
// ---------------------------------------------------------------------------

/// The crash row #0 (count 0) is restarted as #1 (count 1); #1 ends before it runs; the
/// operator restarts; the replacement #2 carries the count of the launch that ran, 0.
fn operator_restart_after_an_abandoned_crash_restart(why: Abandon) {
    let mut rows = vec![ran(0, WorkloadKind::Service, Class::Crashed)];
    let first = evaluate(WorkloadKind::Service, &rows, &crashed_predecessor_view(0), 0, 100);
    assert_eq!(first.issued(), vec![aid(1)], "setup: the crash is restarted as one new launch");
    rows.push(abandoned(1, WorkloadKind::Service, why));
    // The operator's restart commits generation 1.
    let second = evaluate(WorkloadKind::Service, &rows, &first.view, 1, 200);
    assert_eq!(
        second.issued(),
        vec![aid(2)],
        "setup: the committed generation relaunches over the abandoned row as one new launch"
    );
    assert_eq!(
        second.count_of(&aid(2)),
        Some(0),
        "an operator restart over a crash restart that never ran must carry the count of the \
         launch that ran (0), forgiving the crash, not the abandoned launch's count"
    );
}

/// RB-13 count test 1, `LaunchSuperseded`. Contract: the abandoned row is not the count basis.
/// Fault: the crash restart's launch is superseded, then the operator restarts.
#[test]
fn a_superseded_crash_restart_abandoned_by_an_operator_restart_forgives_the_crash() {
    operator_restart_after_an_abandoned_crash_restart(Abandon::Superseded);
}

/// RB-13 count test 1, `LaunchLostWithControlPlane`: the same, for a launch lost at boot.
#[test]
fn a_crash_restart_lost_with_the_control_plane_then_restarted_by_the_operator_forgives_the_crash() {
    operator_restart_after_an_abandoned_crash_restart(Abandon::LostWithControlPlane);
}

// ---------------------------------------------------------------------------
// (2) A lost launch with no operator action charges the crash once
// ---------------------------------------------------------------------------

/// A crash row #0 followed by `abandoned` consecutive abandoned relaunches (one evaluation
/// each, the abandoned row appended after each, no operator action): the next relaunch's count
/// is the crash's charge, once.
fn relaunch_after_consecutive_abandoned_launches(
    abandoned_rows: u32,
) -> (Evaluation, AllocationId) {
    let mut rows = vec![ran(0, WorkloadKind::Service, Class::Crashed)];
    let mut view = crashed_predecessor_view(0);
    let mut seconds = 100;
    let mut last = None;
    for index in 1..=abandoned_rows + 1 {
        let evaluation = evaluate(WorkloadKind::Service, &rows, &view, 0, seconds);
        assert_eq!(
            evaluation.issued(),
            vec![aid(index)],
            "setup: evaluation {index} relaunches the workload as one new launch"
        );
        view = evaluation.view.clone();
        if index <= abandoned_rows {
            let why = Abandon::BOTH[(index % 2) as usize];
            rows.push(abandoned(index, WorkloadKind::Service, why));
        }
        seconds += 100;
        last = Some((evaluation, aid(index)));
    }
    last.expect("at least one evaluation ran")
}

/// RB-13 count test 2: one abandoned row. The relaunch carries the basis count plus one (1),
/// the charge the crash already carries, not 2.
#[test]
fn a_lost_launch_with_no_operator_action_charges_the_crash_once() {
    let (evaluation, id) = relaunch_after_consecutive_abandoned_launches(1);
    assert_eq!(
        evaluation.count_of(&id),
        Some(1),
        "a lost launch with no operator action must charge the crash exactly once: the relaunch \
         carries the crashed launch's count plus one (1)"
    );
}

/// RB-13 count test 2, two consecutive abandoned rows: count 1, not 2 and not 3.
#[test]
fn two_consecutive_abandoned_launches_charge_the_crash_once() {
    let (evaluation, id) = relaunch_after_consecutive_abandoned_launches(2);
    assert_eq!(
        evaluation.count_of(&id),
        Some(1),
        "two consecutive abandoned launches must charge the crash exactly once: the relaunch \
         carries the crashed launch's count plus one (1)"
    );
}

/// RB-13, reconciler half of the restart-status agreement (the accessor half is
/// `restart_status_of_an_abandoned_current_row_reads_the_attempts_of_the_launch_that_ran`):
/// the crash `#0` failed at `CEILING - 1`, its crash restart `#1` was abandoned carrying
/// `CEILING`. The abandoned launch is not counted, so the budget is not spent: the reconciler
/// places a successor, carrying the failed launch's count plus one (`CEILING`).
#[test]
fn a_crash_restart_abandoned_at_the_ceiling_is_restarted_again() {
    for why in Abandon::BOTH {
        let rows = [
            ran(0, WorkloadKind::Service, Class::Crashed),
            abandoned(1, WorkloadKind::Service, why),
        ];
        let mut view = crashed_predecessor_view(RESTART_BACKOFF_CEILING - 1);
        view.restart_counts.insert(aid(1), RESTART_BACKOFF_CEILING);
        view.issued_generation.insert(aid(1), 0);
        let evaluation = evaluate(WorkloadKind::Service, &rows, &view, 0, 1_000_000);
        assert!(
            evaluation.finalised().is_empty(),
            "an abandoned launch at the ceiling must not exhaust the budget ({why:?}), got {:?}",
            evaluation.finalised()
        );
        assert_eq!(
            evaluation.issued(),
            vec![aid(2)],
            "the crash restart abandoned at the ceiling must be restarted again ({why:?})"
        );
        assert_eq!(
            evaluation.count_of(&aid(2)),
            Some(RESTART_BACKOFF_CEILING),
            "the relaunch carries the failed launch's count plus one ({why:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// (3) A launch abandoned before anything ran carries 0; a replacement stays a replacement
// ---------------------------------------------------------------------------

/// RB-13 count test 3, first part: the first launch of a workload is abandoned (a shutdown or a
/// lost launch); no row ever ran, so the relaunch is a fresh placement carrying 0.
#[test]
fn a_launch_abandoned_before_anything_ran_is_relaunched_with_count_zero() {
    for why in Abandon::BOTH {
        let first = evaluate(WorkloadKind::Service, &[], &WorkloadLifecycleView::default(), 0, 100);
        assert_eq!(first.issued(), vec![aid(0)], "setup: the first placement issues one launch");
        let rows = [abandoned(0, WorkloadKind::Service, why)];
        let second = evaluate(WorkloadKind::Service, &rows, &first.view, 0, 200);
        assert_eq!(
            second.issued(),
            vec![aid(1)],
            "setup: the abandoned first launch is relaunched as one new launch"
        );
        assert_eq!(
            second.count_of(&aid(1)),
            Some(0),
            "a launch abandoned before anything ran must be relaunched as a fresh placement \
             carrying count 0 ({why:?})"
        );
    }
}

/// RB-13 count test 3, second part: the replacement of an operator-stopped row (count `c`) is
/// abandoned by node shutdown. The workload relaunches as a generation replacement still
/// pending against the stopped row, carrying the stopped row's count `c`, not `c + 1`.
#[test]
fn a_shutdown_abandoned_replacement_is_relaunched_as_a_generation_replacement_with_the_stopped_rows_count()
 {
    for why in Abandon::BOTH {
        for stopped_count in 0..RESTART_BACKOFF_CEILING {
            let mut rows = vec![ran(0, WorkloadKind::Service, Class::OperatorStopped)];
            let mut view = WorkloadLifecycleView::default();
            view.restart_counts.insert(aid(0), stopped_count);
            view.issued_generation.insert(aid(0), 0);
            // The operator's restart commits generation 1: the replacement is issued.
            let first = evaluate(WorkloadKind::Service, &rows, &view, 1, 100);
            assert_eq!(
                first.issued(),
                vec![aid(1)],
                "setup: the restart commit issues one replacement"
            );
            // Its launch ends before it runs (node shutdown, or lost with the control plane).
            rows.push(abandoned(1, WorkloadKind::Service, why));
            let second = evaluate(WorkloadKind::Service, &rows, &first.view, 1, 200);
            assert_eq!(
                second.issued(),
                vec![aid(2)],
                "setup: the abandoned replacement is relaunched as one new launch"
            );
            assert_eq!(
                second.count_of(&aid(2)),
                Some(stopped_count),
                "a shutdown-abandoned replacement must be relaunched as a generation replacement \
                 carrying the stopped row's count ({stopped_count}, {why:?})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A Job whose launch never ran is not finalised
// ---------------------------------------------------------------------------

/// `is_natural_exit` excludes an abandoned row: a Job's launch that ended before it ran is
/// relaunched, never finalised `Failed { exit_code: Some(0) }` as if the workload had exited.
#[test]
fn a_job_whose_launch_never_ran_is_relaunched_and_not_finalised() {
    for why in Abandon::BOTH {
        let rows = [abandoned(0, WorkloadKind::Job, why)];
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(aid(0), 0);
        view.issued_generation.insert(aid(0), 0);
        let evaluation = evaluate(WorkloadKind::Job, &rows, &view, 0, 100);
        assert!(
            evaluation.finalised().is_empty(),
            "a Job whose launch never ran must not be finalised Failed {{ exit_code: Some(0) }} \
             ({why:?}), got {:?}",
            evaluation.finalised()
        );
        assert_eq!(
            evaluation.issued(),
            vec![aid(1)],
            "a Job whose launch never ran must be relaunched exactly once ({why:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// last_ran_alloc units (GUARDs)
// ---------------------------------------------------------------------------

fn refs(rows: &[AllocStatusRow]) -> Vec<&AllocStatusRow> {
    rows.iter().collect()
}

/// GUARD: the newest row that ran is returned even when abandoned rows sit above it.
#[test]
fn last_ran_skips_abandoned_rows_and_returns_the_newest_row_that_ran() {
    let rows = [
        ran(0, WorkloadKind::Service, Class::Crashed),
        ran(1, WorkloadKind::Service, Class::Crashed),
        abandoned(2, WorkloadKind::Service, Abandon::Superseded),
        abandoned(3, WorkloadKind::Service, Abandon::LostWithControlPlane),
    ];
    let all = refs(&rows);
    assert_eq!(last_ran_alloc(&all).map(|row| row.alloc_id.clone()), Some(aid(1)));
    assert_eq!(
        current_alloc(&all).map(|row| row.alloc_id.clone()),
        Some(aid(3)),
        "current_alloc is unchanged: the newest row, abandoned or not"
    );
}

/// GUARD: with no row that ran (no rows, or every row abandoned) there is no basis.
#[test]
fn last_ran_is_none_when_no_row_ran() {
    assert!(last_ran_alloc(&[]).is_none(), "no rows: no basis");
    let rows = [
        abandoned(0, WorkloadKind::Service, Abandon::Superseded),
        abandoned(1, WorkloadKind::Service, Abandon::LostWithControlPlane),
    ];
    assert!(last_ran_alloc(&refs(&rows)).is_none(), "every row abandoned: no basis");
}

/// GUARD: an abandoned row is skipped, a failed or reclaimed one is not, and the numeric
/// attempt suffix (not the lexical id order) picks the newest.
#[test]
fn last_ran_orders_by_numeric_attempt_suffix() {
    let rows = [
        ran(2, WorkloadKind::Service, Class::Crashed),
        ran(10, WorkloadKind::Service, Class::PlatformReclaimed),
        abandoned(11, WorkloadKind::Service, Abandon::Superseded),
    ];
    assert_eq!(
        last_ran_alloc(&refs(&rows)).map(|row| row.alloc_id.clone()),
        Some(aid(10)),
        "alloc-payments-10 is newer than alloc-payments-2 although it sorts before it"
    );
}

fn slot_rows(slots: &[bool]) -> Vec<AllocStatusRow> {
    slots
        .iter()
        .enumerate()
        .map(|(index, is_abandoned)| {
            let suffix = index as u32;
            if *is_abandoned {
                abandoned(suffix, WorkloadKind::Service, Abandon::BOTH[index % 2])
            } else {
                ran(suffix, WorkloadKind::Service, Class::Crashed)
            }
        })
        .collect()
}

proptest! {
    /// GUARD: `last_ran_alloc` is `current_alloc` over the rows that are not abandoned, never
    /// an abandoned row, and `None` exactly when every row is abandoned.
    #[test]
    fn last_ran_is_current_alloc_over_the_rows_that_are_not_abandoned(
        slots in prop::collection::vec(any::<bool>(), 0..8),
    ) {
        let rows = slot_rows(&slots);
        let all = refs(&rows);
        let kept: Vec<&AllocStatusRow> =
            rows.iter().filter(|row| !is_launch_abandoned(row)).collect();
        let basis = last_ran_alloc(&all);
        prop_assert_eq!(
            basis.map(|row| row.alloc_id.clone()),
            current_alloc(&kept).map(|row| row.alloc_id.clone())
        );
        prop_assert!(basis.is_none_or(|row| !is_launch_abandoned(row)));
        prop_assert_eq!(basis.is_none(), slots.iter().all(|is_abandoned| *is_abandoned));
    }
}

// ---------------------------------------------------------------------------
// Backend membership (GUARD)
// ---------------------------------------------------------------------------

/// GUARD: a Service allocation whose row is `Pending` or an abandoned launch carries the
/// address it was handed but is never a backend candidate: it projects `NotRunning`.
#[test]
fn a_service_allocation_that_never_ran_is_not_a_backend_candidate() {
    let spiffe = SpiffeId::new("spiffe://overdrive.local/workload/payments").expect("spiffe id");
    let mut pending = base(0, WorkloadKind::Service);
    pending.workload_addr = Some(Ipv4Addr::new(10, 99, 128, 2));
    let rows = [
        pending,
        abandoned(1, WorkloadKind::Service, Abandon::Superseded),
        abandoned(2, WorkloadKind::Service, Abandon::LostWithControlPlane),
    ];
    for row in &rows {
        assert!(row.workload_addr.is_some(), "setup: the row carries an address");
        let phase = ServiceAllocPhase::from_row(row, spiffe.clone()).expect("a non-Running row");
        assert!(
            matches!(phase, ServiceAllocPhase::NotRunning(_)),
            "a {:?} row that never ran must not be a backend candidate, got {phase:?}",
            row.state
        );
    }
}

// ---------------------------------------------------------------------------
// The deletion-equivalence property (restart and FinalizeFailed clauses)
// ---------------------------------------------------------------------------

/// One launch in a generated chain, with the View entries the reconciler holds for its id.
#[derive(Debug, Clone)]
enum Slot {
    Ran { class: Class, count: u32, seen: Option<u64>, issued: u64 },
    Abandoned { why: Abandon, count: u32, seen: Option<u64>, issued: u64 },
}

/// A generated chain: the workload kind, its launches oldest first, and the committed generation.
#[derive(Debug, Clone)]
struct Chain {
    kind: WorkloadKind,
    slots: Vec<Slot>,
    generation: u64,
}

fn arb_slot() -> impl Strategy<Value = Slot> {
    let seen = prop::option::of(995_u64..=1000);
    prop_oneof![
        (
            prop_oneof![
                Just(Class::Crashed),
                Just(Class::PlatformReclaimed),
                Just(Class::OperatorStopped),
                Just(Class::Running),
                Just(Class::Completed),
                Just(Class::JobFailed),
            ],
            0..=RESTART_BACKOFF_CEILING,
            seen.clone(),
            0_u64..=2,
        )
            .prop_map(|(class, count, seen, issued)| Slot::Ran {
                class,
                count,
                seen,
                issued
            }),
        (
            prop_oneof![Just(Abandon::Superseded), Just(Abandon::LostWithControlPlane)],
            0..=RESTART_BACKOFF_CEILING + 2,
            seen,
            0_u64..=2,
        )
            .prop_map(|(why, count, seen, issued)| Slot::Abandoned {
                why,
                count,
                seen,
                issued
            }),
    ]
}

/// A chain a workload can actually produce. A launch is only replaced once its predecessor has
/// ended, so a `Running` row, and a finished Job's row (final, a restart issues no id), is only
/// ever the newest row; and only a Job has a finished row. The committed generation never falls
/// below the generation any launch was issued for (it only grows), and a launch issued after an
/// operator stop was issued for a later generation than the stopped row (only a restart or a
/// resume lifts the stop, and each commits a new generation).
fn arb_chain() -> impl Strategy<Value = Chain> {
    (any::<bool>(), prop::collection::vec(arb_slot(), 1..=6), 0_u64..=2).prop_map(
        |(is_job, mut slots, headroom)| {
            let kind = if is_job { WorkloadKind::Job } else { WorkloadKind::Service };
            let newest = slots.len() - 1;
            let mut issued_for = 0_u64;
            let mut after_operator_stop = false;
            for (index, slot) in slots.iter_mut().enumerate() {
                let (Slot::Ran { issued, .. } | Slot::Abandoned { issued, .. }) = slot;
                let bump = if after_operator_stop { (*issued).max(1) } else { *issued % 2 };
                issued_for += bump;
                *issued = issued_for;
                if let Slot::Ran { class, .. } = slot {
                    let final_class =
                        matches!(class, Class::Running | Class::Completed | Class::JobFailed);
                    let job_only = matches!(class, Class::Completed | Class::JobFailed);
                    if (final_class && index != newest) || (job_only && !is_job) {
                        *class = Class::Crashed;
                    }
                    after_operator_stop = *class == Class::OperatorStopped;
                } else {
                    after_operator_stop = false;
                }
            }
            // An operator-stopped basis followed by abandoned launches issued under the committed
            // generation itself is the pending-generation agreement clause below (the read of
            // "is a replacement still pending against the stopped row" is that clause's subject);
            // here the operator restarts once more after the abandoned launch.
            let basis = slots.iter().rposition(|slot| matches!(slot, Slot::Ran { .. }));
            let stopped_then_abandoned = basis.is_some_and(|index| {
                index < newest
                    && matches!(slots[index], Slot::Ran { class: Class::OperatorStopped, .. })
            });
            let headroom = if stopped_then_abandoned { headroom.max(1) } else { headroom };
            Chain { kind, slots, generation: issued_for + headroom }
        },
    )
}

/// The evaluation instant; generated failure times sit inside and just outside the backoff window.
const NOW: u64 = 1000;

/// The full chain, the chain with the abandoned rows deleted, and the View both are read with.
fn build_chain(chain: &Chain) -> (Vec<AllocStatusRow>, Vec<AllocStatusRow>, WorkloadLifecycleView) {
    let mut view = WorkloadLifecycleView::default();
    let mut full = Vec::with_capacity(chain.slots.len());
    for (index, slot) in chain.slots.iter().enumerate() {
        let suffix = index as u32;
        let (row, count, seen, issued) = match slot {
            Slot::Ran { class, count, seen, issued } => {
                (ran(suffix, chain.kind, *class), count, seen, issued)
            }
            Slot::Abandoned { why, count, seen, issued } => {
                (abandoned(suffix, chain.kind, *why), count, seen, issued)
            }
        };
        full.push(row);
        view.restart_counts.insert(aid(suffix), *count);
        view.issued_generation.insert(aid(suffix), *issued);
        if let Some(seen) = seen {
            view.last_failure_seen_at.insert(aid(suffix), instant(*seen));
        }
    }
    let deleted: Vec<AllocStatusRow> =
        full.iter().filter(|row| !is_launch_abandoned(row)).cloned().collect();
    (full, deleted, view)
}

/// What an evaluation decided about the next launch: the count and the failure time it carries,
/// or the exhausted budget's finalisation, or nothing. Ids differ across the two chains and are
/// not compared for a launch; a finalisation names the basis row in both.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Quiet,
    Launch { count: Option<u32>, last_failure_seen_at: Option<UnixInstant> },
    Finalise { alloc: AllocationId, terminal: Option<TerminalCondition> },
}

fn decision(evaluation: &Evaluation) -> Decision {
    if let Some((alloc, terminal)) = evaluation.finalised().into_iter().next() {
        return Decision::Finalise { alloc, terminal };
    }
    evaluation.issued().first().map_or(Decision::Quiet, |id| Decision::Launch {
        count: evaluation.count_of(id),
        last_failure_seen_at: evaluation.view.last_failure_seen_at.get(id).copied(),
    })
}

proptest! {
    /// Deleting the abandoned rows from any chain changes no later launch's count, charge or
    /// exhaustion: the same count and failure time are carried, the same budget exhaustion is
    /// finalised naming the same row, and the backoff gate holds or opens identically, whatever
    /// View entries the abandoned ids carry. Chains mix launches that ran (crashed,
    /// platform-reclaimed, operator-stopped, running, or a finished Job) with abandoned launches
    /// of both reasons, in any order a workload can produce, for a Service or a Job, under any
    /// committed generation, so the pending-generation replacement and the operator-stop veto are
    /// reached. The oracle is differential: the full chain against the chain with the abandoned
    /// rows deleted. The seed is printed on failure by proptest.
    #[test]
    fn deleting_abandoned_rows_changes_no_later_launchs_count_charge_or_exhaustion(
        chain in arb_chain(),
    ) {
        let generation = chain.generation;
        let (full, deleted, view) = build_chain(&chain);
        let with_abandoned = decision(&evaluate(chain.kind, &full, &view, generation, NOW));
        let without_abandoned = decision(&evaluate(chain.kind, &deleted, &view, generation, NOW));
        prop_assert_eq!(
            with_abandoned,
            without_abandoned,
            "deleting the abandoned rows from a chain must not change the next launch's count, \
             charge or exhaustion (generation {}, chain {:?})",
            generation,
            chain
        );
    }
}

// ---------------------------------------------------------------------------
// The generation_replacement_pending agreement clause (separate property)
// ---------------------------------------------------------------------------

/// The payload of the production scaffold's panic, matched exactly: only this panic is the
/// pending state; any other panic is a real failure and propagates.
const SCAFFOLD_PANIC: &str =
    "RED scaffold: generation_replacement_pending is not implemented (ADR-0335)";

fn panic_text(payload: &(dyn Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

const AGREEMENT: &str = "generation_replacement_pending must read the generation from the newest \
     row that ran: an abandoned row is invisible to it, so deleting abandoned rows from a chain \
     changes no verdict";

/// The function's answer; the scaffold's unwind (exactly its literal payload) is re-raised as
/// this file's named assertion, any other panic is resumed unchanged.
fn pending_verdict(
    view: &WorkloadLifecycleView,
    generation: u64,
    ran: Option<&AllocStatusRow>,
) -> bool {
    match catch_unwind(AssertUnwindSafe(|| generation_replacement_pending(view, generation, ran))) {
        Ok(answer) => answer,
        Err(payload) => {
            let is_scaffold = panic_text(&*payload) == Some(SCAFFOLD_PANIC);
            assert!(!is_scaffold, "{AGREEMENT}");
            resume_unwind(payload)
        }
    }
}

/// One launch of an agreement chain: `Some` is a launch that ran (kind and class), `None` an
/// abandoned launch of the given reason; the last field is its `issued_generation` entry.
type AgreementLaunch = (Option<(WorkloadKind, Class)>, Abandon, u64);

fn arb_agreement_chain() -> impl Strategy<Value = Vec<AgreementLaunch>> {
    // Each element is one launch: `Some` is a launch that ran (kind and class), `None` an
    // abandoned launch of the given reason; the third field is its `issued_generation` entry.
    prop::collection::vec(
        (
            prop::option::of((
                prop_oneof![Just(WorkloadKind::Service), Just(WorkloadKind::Job)],
                prop_oneof![
                    Just(Class::Crashed),
                    Just(Class::OperatorStopped),
                    Just(Class::Completed),
                    Just(Class::JobFailed),
                    Just(Class::Running),
                ],
            )),
            prop_oneof![Just(Abandon::Superseded), Just(Abandon::LostWithControlPlane)],
            0_u64..=3,
        ),
        1..=6,
    )
}

/// The stamp-on-emit case the main deletion-equivalence property keeps out of its chains: an
/// operator-stopped basis followed by an abandoned replacement issued under the committed
/// generation itself. The old rule stamps the abandoned launch's entry on emit and reads the
/// generation as handled; the abandoned row ran nothing, so the stopped basis (issued under an
/// earlier generation) is still the row the generation is read from and it is pending. The same
/// holds when the chain has the abandoned row deleted.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must read the generation from the newest row that ran"
)]
fn an_abandoned_replacement_issued_under_the_committed_generation_leaves_a_stopped_basis_pending() {
    for why in Abandon::BOTH {
        let full = [
            ran(0, WorkloadKind::Service, Class::OperatorStopped),
            abandoned(1, WorkloadKind::Service, why),
        ];
        let deleted = [full[0].clone()];
        let mut view = WorkloadLifecycleView::default();
        view.issued_generation.insert(aid(0), 1);
        view.issued_generation.insert(aid(1), 2);
        let over_full = pending_verdict(&view, 2, last_ran_alloc(&refs(&full)));
        let over_deleted = pending_verdict(&view, 2, current_alloc(&refs(&deleted)));
        assert!(
            over_full && over_deleted,
            "{AGREEMENT}: the stopped basis (entry 1) is below generation 2 although the abandoned \
             replacement carries entry 2 ({why:?}); over the full chain {over_full}, over the chain \
             without the abandoned row {over_deleted}"
        );
    }
}

proptest! {
    // The test is PENDING and fails by design today: do not persist its seeds into the repo.
    // Drop this setting together with the `should_panic` marker at the activation step.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Over any chain, any View entries (abandoned ids included) and any committed generation,
    /// `generation_replacement_pending` read through `last_ran_alloc` over the full chain agrees
    /// with the same function read through `current_alloc` over the chain with the abandoned
    /// rows deleted, and with an independent restatement of the contract: true iff the newest
    /// row that ran exists, is not a finished Job row, and its `issued_generation` entry
    /// (absent reads 0) is below the generation. PENDING: the function is a scaffold today.
    #[test]
    #[should_panic(
        expected = "generation_replacement_pending must read the generation from the newest row that ran"
    )]
    fn the_pending_generation_verdict_agrees_over_a_chain_and_the_chain_without_abandoned_rows(
        chain in arb_agreement_chain(),
        generation in 0_u64..=4,
    ) {
        let mut view = WorkloadLifecycleView::default();
        let full: Vec<AllocStatusRow> = chain
            .iter()
            .enumerate()
            .map(|(index, (launch, why, issued))| {
                let suffix = index as u32;
                view.issued_generation.insert(aid(suffix), *issued);
                launch.map_or_else(
                    || abandoned(suffix, WorkloadKind::Service, *why),
                    |(kind, class)| ran(suffix, kind, class),
                )
            })
            .collect();
        let deleted: Vec<AllocStatusRow> =
            full.iter().filter(|row| !is_launch_abandoned(row)).cloned().collect();
        let basis = last_ran_alloc(&refs(&full));
        let basis_after_deletion = current_alloc(&refs(&deleted));
        prop_assert_eq!(
            basis.map(|row| row.alloc_id.clone()),
            basis_after_deletion.map(|row| row.alloc_id.clone())
        );
        let expected = basis.is_some_and(|row| {
            !(row.kind == WorkloadKind::Job && row.terminal.is_some())
                && view.issued_generation.get(&row.alloc_id).copied().unwrap_or(0) < generation
        });
        let over_full = pending_verdict(&view, generation, basis);
        let over_deleted = pending_verdict(&view, generation, basis_after_deletion);
        prop_assert_eq!(over_full, over_deleted, "{}", AGREEMENT);
        prop_assert_eq!(over_full, expected, "{}", AGREEMENT);
    }
}
