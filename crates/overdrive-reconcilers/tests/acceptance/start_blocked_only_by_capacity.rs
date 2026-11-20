//! `start_blocked_only_by_capacity` — the one predicate the reconciler's rules
//! and `overdrive workload describe` share for "this workload waits for
//! guest-network capacity" (ADR-0327, node-local persistent volume, GH #348;
//! scenario family S-WAIT-PRED).
//!
//! CONTRACT_SHAPE: pure-function (the table), plus three unbounded-preservation
//! agreement properties against `WorkloadLifecycle::reconcile` ((a) blocked means
//! no start, (b) a freed slot starts a blocked workload, (c) a start a freed slot
//! releases is a blocked workload) and a non-vacuity guard over a fixed ensemble.
//!
//! Owner path: `overdrive_reconcilers::start_blocked_only_by_capacity(kind,
//! run_desired, rows, view, desired_generation, observation)` for the table;
//! `WorkloadLifecycle::reconcile` (through `Reconciler::reconcile`) for the three
//! agreement properties. Oracle: the predicate's boolean per class of the
//! current row; for the properties, the presence or absence of exactly one
//! `StartAllocation` / `RestartAllocation` in the reconciler's returned
//! actions. Tier: Tier 1, default lane (pure, no I/O).
//!
//! Status of every test in this file but one: PENDING (proven RED); the
//! exception is the non-vacuity guard, which judges the generator by the
//! reconciler alone and passes today. The pinned
//! production scaffold panics with the literal payload `SCAFFOLD_PANIC`; each
//! body calls the predicate through `verdict` / `predicate`, which catch exactly
//! that payload and yield `None` (any other panic is resumed), and the test then
//! fails on its own named assertion ("start_blocked_only_by_capacity must
//! classify ... as <verdict>", "... must agree with the reconciler ...").
//! `#[should_panic(expected = ..)]` names that assertion, never the scaffold text. DELIVER removes the markers when it
//! implements the predicate.
//!
//! Mutation targets killed (the predicate's step 6 and its inputs):
//! - each arm of step 6: `a_job_with_a_natural_exit_row_never_waits`,
//!   `an_operator_stopped_current_row_waits_only_once_a_generation_is_pending`,
//!   `a_system_gc_row_waits`, the crashed / ceiling / `BackoffExhausted` /
//!   liveness / reclaimed tests (the restartable arm and its attempts input),
//!   `a_pending_state_current_row_never_waits` (the "anything else" arm);
//! - the lease test: `a_leased_current_row_is_reclaim_progress_not_a_wait`,
//!   `a_lease_on_a_historical_row_does_not_make_the_current_row_leased`;
//! - the finished-Job exception: `a_finished_job_never_waits_even_with_the_generation_bumped`;
//! - the `at_cap` comparison: every table runs the pool at `cap - 1`, `cap`
//!   and `cap + 1`, and `retiring_leases_count_toward_held`;
//! - the shared decision with the reconciler: the three agreement properties.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::any::Any;
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::guest_network::{GuestAttachmentOccupancy, MAX_GUEST_NETWORK_ATTACHMENTS};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::GuestAttachmentObservation;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::guest_attachment_view::GuestAttachmentLease;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
    start_blocked_only_by_capacity,
};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

const WORKLOAD: &str = "payments";

/// The payload of the production scaffold's panic, matched exactly: only this
/// panic is the pending state; any other panic is a real failure and propagates.
const SCAFFOLD_PANIC: &str =
    "RED scaffold: start_blocked_only_by_capacity is not implemented (ADR-0327)";

const CAP: u32 = MAX_GUEST_NETWORK_ATTACHMENTS;

fn aid(suffix: u32) -> AllocationId {
    AllocationId::new(&format!("alloc-{WORKLOAD}-{suffix}")).expect("valid allocation id")
}

fn wid() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("valid workload id")
}

fn nid() -> NodeId {
    NodeId::new("local").expect("valid node id")
}

fn panic_text(payload: &(dyn Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

/// `Some(answer)`, or `None` while the production scaffold (exactly its literal
/// panic payload) stands. Any other panic is resumed unchanged.
fn scaffold_none<T>(outcome: Result<T, Box<dyn Any + Send>>) -> Option<T> {
    match outcome {
        Ok(answer) => Some(answer),
        Err(payload) => {
            if panic_text(&*payload) == Some(SCAFFOLD_PANIC) {
                None
            } else {
                resume_unwind(payload)
            }
        }
    }
}

const fn instant(seconds: u64) -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(seconds))
}

// ---------------------------------------------------------------------------
// Rows by class
// ---------------------------------------------------------------------------

/// The classes of an allocation row the predicate distinguishes. Each is a
/// state production authors (a driver exit, the action shim's stop terminal,
/// the finalise path, platform reclamation); none is seeded to manufacture a
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// `Failed`, crash reason, no terminal.
    Crashed,
    /// `Terminated`, `Stopped { PlatformReclaimed }`, no terminal.
    Reclaimed,
    /// `Terminated`, `Stopped { Operator }` reason and terminal.
    OperatorStopped,
    /// `Terminated`, `Stopped { SystemGc }` reason and terminal.
    SystemGc,
    /// `Terminated`, `Stopped { LivenessProbe }` reason and terminal.
    LivenessKilled,
    /// `Terminated`, `Stopped { StartupProbe }` reason and terminal.
    StartupKilled,
    /// `Failed`, terminal `BackoffExhausted`.
    BackoffExhausted,
    /// `Failed`, terminal `ServiceFailed { EarlyExit }`.
    ServiceFailed,
    /// `Terminated`, terminal `Completed` (Job).
    Completed,
    /// `Failed`, terminal `Failed` (Job).
    JobFailed,
    Running,
    Pending,
    Draining,
}

fn row(suffix: u32, kind: WorkloadKind, class: Class) -> AllocStatusRow {
    let (state, reason, terminal) = match class {
        Class::Crashed => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(23),
                signal: None,
                stderr_tail: None,
            }),
            None,
        ),
        Class::Reclaimed => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
            None,
        ),
        Class::OperatorStopped => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        ),
        Class::SystemGc => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::SystemGc }),
            Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }),
        ),
        Class::LivenessKilled => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::LivenessProbe }),
            Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        ),
        Class::StartupKilled => (
            AllocState::Terminated,
            Some(TransitionReason::Stopped { by: StoppedBy::StartupProbe }),
            Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        ),
        Class::BackoffExhausted => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(23),
                signal: None,
                stderr_tail: None,
            }),
            Some(TerminalCondition::BackoffExhausted { attempts: RESTART_BACKOFF_CEILING }),
        ),
        Class::ServiceFailed => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
            Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
            }),
        ),
        Class::Completed => {
            (AllocState::Terminated, None, Some(TerminalCondition::Completed { exit_code: 0 }))
        }
        Class::JobFailed => (
            AllocState::Failed,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
            Some(TerminalCondition::Failed { exit_code: Some(1) }),
        ),
        Class::Running => (AllocState::Running, None, None),
        Class::Pending => (AllocState::Pending, None, None),
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
// The predicate, called the way `describe` does
// ---------------------------------------------------------------------------

/// One workload's inputs to the predicate, with the generation either handled
/// (`issued == generation`) or pending (`generation` one above `issued`).
struct Scenario {
    class: &'static str,
    kind: WorkloadKind,
    run_desired: bool,
    rows: Vec<AllocStatusRow>,
    /// `view.restart_counts` per allocation suffix.
    attempts: Vec<(u32, u32)>,
    /// Allocation suffixes holding a guest-network lease.
    leases: Vec<(u32, GuestAttachmentLease)>,
}

/// The generation an issued id serves; a pending generation is one above it.
const ISSUED: u64 = 1;

impl Scenario {
    fn view(&self) -> WorkloadLifecycleView {
        let mut view = WorkloadLifecycleView::default();
        for row in &self.rows {
            view.issued_generation.insert(row.alloc_id.clone(), ISSUED);
        }
        for (suffix, attempts) in &self.attempts {
            view.restart_counts.insert(aid(*suffix), *attempts);
        }
        view
    }

    fn observation(&self, held: u32, retiring: u32) -> GuestAttachmentObservation {
        GuestAttachmentObservation {
            occupancy: GuestAttachmentOccupancy { held, retiring },
            leases: self.leases.iter().map(|(suffix, lease)| (aid(*suffix), *lease)).collect(),
        }
    }

    /// The predicate's answer, or `None` while the production scaffold panics.
    fn verdict(&self, generation_pending: bool, held: u32, retiring: u32) -> Option<bool> {
        let view = self.view();
        let refs: Vec<&AllocStatusRow> = self.rows.iter().collect();
        let generation = if generation_pending { ISSUED + 1 } else { ISSUED };
        let observation = self.observation(held, retiring);
        scaffold_none(catch_unwind(AssertUnwindSafe(|| {
            start_blocked_only_by_capacity(
                self.kind,
                self.run_desired,
                &refs,
                &view,
                generation,
                &observation,
            )
        })))
    }

    /// Assert the predicate at the pool levels `cap`, `cap - 1`, `cap + 1`, with
    /// and without a pending generation (the at-cap, no-generation-pending case
    /// first, so a failure names the headline verdict). `blocked` gives the verdict AT the cap
    /// for (`generation_pending`); below the cap the verdict is always false.
    fn expect(&self, blocked: impl Fn(bool) -> bool) {
        for generation_pending in [false, true] {
            for held in [CAP, CAP - 1, CAP + 1] {
                let want = held >= CAP && blocked(generation_pending);
                assert_eq!(
                    self.verdict(generation_pending, held, 0),
                    Some(want),
                    "start_blocked_only_by_capacity must classify {} (held {held} of {CAP}, \
                     generation pending: {generation_pending}) as {want}",
                    self.class,
                );
            }
        }
    }
}

fn service(class: &'static str, current: Class) -> Scenario {
    Scenario {
        class,
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: vec![row(0, WorkloadKind::Service, current)],
        attempts: vec![(0, 1)],
        leases: Vec::new(),
    }
}

fn job(class: &'static str, current: Class) -> Scenario {
    Scenario {
        class,
        kind: WorkloadKind::Job,
        run_desired: true,
        rows: vec![row(0, WorkloadKind::Job, current)],
        attempts: vec![(0, 1)],
        leases: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// No row yet (a first placement or a first restart's fresh start): waiting
/// whenever the pool is at the cap, pending or not.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a workload with no row (held 16384 of 16384, generation pending: false) as true"
)]
fn a_workload_with_no_row_waits_at_the_cap() {
    let scenario = Scenario {
        class: "a workload with no row",
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: Vec::new(),
        attempts: Vec::new(),
        leases: Vec::new(),
    };
    scenario.expect(|_| true);
}

/// A crashed row under budget: the restart is delayed only by capacity.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a crashed row under the restart budget (held 16384 of 16384, generation pending: false) as true"
)]
fn a_crashed_row_under_budget_waits_at_the_cap() {
    service("a crashed row under the restart budget", Class::Crashed).expect(|_| true);
}

/// A row at the restart ceiling is blocked for good; a pending generation
/// reopens it (a generation replacement is uncharged).
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a crashed row at the restart ceiling (held 16384 of 16384, generation pending: false) as false"
)]
fn a_row_at_the_ceiling_waits_only_once_a_generation_is_pending() {
    let mut scenario = service("a crashed row at the restart ceiling", Class::Crashed);
    scenario.attempts = vec![(0, RESTART_BACKOFF_CEILING)];
    scenario.expect(|generation_pending| generation_pending);
}

/// `BackoffExhausted` is blocked for good; a pending generation reopens it.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a BackoffExhausted row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_backoff_exhausted_row_waits_only_once_a_generation_is_pending() {
    let mut scenario = service("a BackoffExhausted row", Class::BackoffExhausted);
    scenario.attempts = vec![(0, RESTART_BACKOFF_CEILING)];
    scenario.expect(|generation_pending| generation_pending);
}

/// The other terminal the ceiling produces (`ServiceFailed`) behaves the same.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a ServiceFailed row at the ceiling (held 16384 of 16384, generation pending: false) as false"
)]
fn a_service_failed_row_waits_only_once_a_generation_is_pending() {
    let mut scenario = service("a ServiceFailed row at the ceiling", Class::ServiceFailed);
    scenario.attempts = vec![(0, RESTART_BACKOFF_CEILING)];
    scenario.expect(|generation_pending| generation_pending);
}

/// An operator-stopped row (the veto): blocked for good unless a generation is
/// pending (a `workload restart` after the stop).
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify an operator-stopped row (held 16384 of 16384, generation pending: false) as false"
)]
fn an_operator_stopped_current_row_waits_only_once_a_generation_is_pending() {
    service("an operator-stopped row", Class::OperatorStopped)
        .expect(|generation_pending| generation_pending);
}

/// `Stopped { SystemGc }` is neither vetoed nor restartable: it falls through to
/// a fresh placement, which the cap delays.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a system-GC row (held 16384 of 16384, generation pending: false) as true"
)]
fn a_system_gc_row_waits() {
    service("a system-GC row", Class::SystemGc).expect(|_| true);
}

/// `Stopped { LivenessProbe }` consumes restart budget like a crash: under
/// budget it waits.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a liveness-killed row under budget (held 16384 of 16384, generation pending: false) as true"
)]
fn a_liveness_killed_row_under_budget_waits() {
    service("a liveness-killed row under budget", Class::LivenessKilled).expect(|_| true);
}

/// `Stopped { StartupProbe }` consumes restart budget like a crash: under
/// budget it waits.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a startup-probe-killed row under budget (held 16384 of 16384, generation pending: false) as true"
)]
fn a_startup_probe_killed_row_under_budget_waits() {
    service("a startup-probe-killed row under budget", Class::StartupKilled).expect(|_| true);
}

/// A startup-probe-killed row at the restart ceiling is blocked for good; a
/// pending generation reopens it.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a startup-probe-killed row at the ceiling (held 16384 of 16384, generation pending: false) as false"
)]
fn a_startup_probe_killed_row_at_the_ceiling_waits_only_once_a_generation_is_pending() {
    let mut scenario = service("a startup-probe-killed row at the ceiling", Class::StartupKilled);
    scenario.attempts = vec![(0, RESTART_BACKOFF_CEILING)];
    scenario.expect(|generation_pending| generation_pending);
}

/// A platform-reclaimed row is exempt from the ceiling: it waits even at it.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a platform-reclaimed row at the ceiling (held 16384 of 16384, generation pending: false) as true"
)]
fn a_platform_reclaimed_row_waits_even_at_the_ceiling() {
    let mut scenario = service("a platform-reclaimed row at the ceiling", Class::Reclaimed);
    scenario.attempts = vec![(0, RESTART_BACKOFF_CEILING)];
    scenario.expect(|_| true);
}

/// H1: a Job that completed or failed, or was operator-stopped (its row carries
/// the terminal), never waits, and the generation bumped changes nothing: a
/// `restart` of a finished Job is a no-op.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a finished Job (held 16384 of 16384, generation pending: false) as false"
)]
fn a_finished_job_never_waits_even_with_the_generation_bumped() {
    for class in [Class::Completed, Class::JobFailed, Class::OperatorStopped] {
        job("a finished Job", class).expect(|_| false);
    }
}

/// The Job natural-exit arm scans every active row: a Job whose crashed row has
/// no terminal yet (the reconciler finalises it, it never restarts) does not wait.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a Job with a natural-exit row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_job_with_a_natural_exit_row_never_waits() {
    job("a Job with a natural-exit row", Class::Crashed).expect(|_| false);
}

/// A Job with no row yet (a first placement) waits at the cap, like a Service.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a Job with no row (held 16384 of 16384, generation pending: false) as true"
)]
fn a_job_with_no_row_waits_at_the_cap() {
    let scenario = Scenario {
        class: "a Job with no row",
        kind: WorkloadKind::Job,
        run_desired: true,
        rows: Vec::new(),
        attempts: Vec::new(),
        leases: Vec::new(),
    };
    scenario.expect(|_| true);
}

/// A Job whose current row was platform-reclaimed (no terminal, the intent still
/// stands) is not a natural exit: it waits at the cap.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a Job whose current row was platform-reclaimed (held 16384 of 16384, generation pending: false) as true"
)]
fn a_job_whose_current_row_was_platform_reclaimed_waits() {
    job("a Job whose current row was platform-reclaimed", Class::Reclaimed).expect(|_| true);
}

/// A Job whose current row is `Stopped { SystemGc }` is neither vetoed nor
/// restartable: it falls through to a fresh placement, which the cap delays.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a Job whose current row is system-GC (held 16384 of 16384, generation pending: false) as true"
)]
fn a_job_whose_current_row_is_system_gc_waits() {
    job("a Job whose current row is system-GC", Class::SystemGc).expect(|_| true);
}

/// The Job natural-exit arm scans every active row, not only the current one: a
/// historical `Completed` row beside a reclaimed current row means the Job
/// finished, so it never waits.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a historical Completed Job row beside a reclaimed current row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_historical_completed_job_row_beside_a_reclaimed_current_row_never_waits() {
    let scenario = Scenario {
        class: "a historical Completed Job row beside a reclaimed current row",
        kind: WorkloadKind::Job,
        run_desired: true,
        rows: vec![
            row(0, WorkloadKind::Job, Class::Completed),
            row(1, WorkloadKind::Job, Class::Reclaimed),
        ],
        attempts: vec![(0, 0), (1, 1)],
        leases: Vec::new(),
    };
    scenario.expect(|_| false);
}

/// A leased current row at the cap is the reconciler's row-neutral reclaim
/// pass (shown as `network_cleanup_pending`): progress, not a wait.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a leased current row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_leased_current_row_is_reclaim_progress_not_a_wait() {
    for lease in [GuestAttachmentLease::Admitted, GuestAttachmentLease::Retiring] {
        let mut scenario = service("a leased current row", Class::Crashed);
        scenario.leases = vec![(0, lease)];
        scenario.expect(|_| false);
    }
}

/// A lease on a historical row does not make the current row leased: the
/// current row's own lease is the test.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a crashed current row beside a leased historical row (held 16384 of 16384, generation pending: false) as true"
)]
fn a_lease_on_a_historical_row_does_not_make_the_current_row_leased() {
    let scenario = Scenario {
        class: "a crashed current row beside a leased historical row",
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: vec![
            row(0, WorkloadKind::Service, Class::Crashed),
            row(1, WorkloadKind::Service, Class::Crashed),
        ],
        attempts: vec![(0, 1), (1, 2)],
        leases: vec![(0, GuestAttachmentLease::Admitted)],
    };
    scenario.expect(|_| true);
}

/// A stopped workload (the stop sentinel is present) never waits.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a stopped workload (held 16384 of 16384, generation pending: false) as false"
)]
fn a_stopped_workload_never_waits() {
    let mut scenario = service("a stopped workload", Class::Crashed);
    scenario.run_desired = false;
    scenario.expect(|_| false);
}

/// A `Running` current row is not waiting for anything.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a running workload (held 16384 of 16384, generation pending: false) as false"
)]
fn a_running_workload_never_waits() {
    service("a running workload", Class::Running).expect(|_| false);
}

/// Any active `Running` row, historical or not, means the workload is running.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a running row beside a newer crashed row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_running_historical_row_beside_a_newer_crashed_row_never_waits() {
    let scenario = Scenario {
        class: "a running row beside a newer crashed row",
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: vec![
            row(0, WorkloadKind::Service, Class::Running),
            row(1, WorkloadKind::Service, Class::Crashed),
        ],
        attempts: vec![(0, 0), (1, 1)],
        leases: Vec::new(),
    };
    scenario.expect(|_| false);
}

/// A `Draining` current row is being torn down; the reconciler leaves it alone.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a draining current row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_draining_current_row_never_waits() {
    service("a draining current row", Class::Draining).expect(|_| false);
}

/// The Draining test reads the current row only: a `Draining` historical row
/// beside a newer crashed current row does NOT block the wait.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a draining historical row beside a newer crashed row (held 16384 of 16384, generation pending: false) as true"
)]
fn a_draining_historical_row_beside_a_newer_crashed_row_waits() {
    let scenario = Scenario {
        class: "a draining historical row beside a newer crashed row",
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: vec![
            row(0, WorkloadKind::Service, Class::Draining),
            row(1, WorkloadKind::Service, Class::Crashed),
        ],
        attempts: vec![(0, 0), (1, 1)],
        leases: Vec::new(),
    };
    scenario.expect(|_| true);
}

/// The "anything else" arm: a `Pending` current row (a launch the reconciler
/// does not restart) is not a wait.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a pending-state current row (held 16384 of 16384, generation pending: false) as false"
)]
fn a_pending_state_current_row_never_waits() {
    service("a pending-state current row", Class::Pending).expect(|_| false);
}

/// A Schedule never waits: `describe` rejects it and it has no start rule here.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must classify a Schedule (held 16384 of 16384, generation pending: false) as false"
)]
fn a_schedule_never_waits() {
    let scenario = Scenario {
        class: "a Schedule",
        kind: WorkloadKind::Schedule,
        run_desired: true,
        rows: Vec::new(),
        attempts: Vec::new(),
        leases: Vec::new(),
    };
    scenario.expect(|_| false);
}

/// Retiring leases still occupy the cap (ADR-0133): `held` is compared whole,
/// `retiring` is not subtracted from it.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must count retiring leases toward held (held 16384, retiring 0)"
)]
fn retiring_leases_count_toward_held() {
    let scenario = Scenario {
        class: "a workload with no row",
        kind: WorkloadKind::Service,
        run_desired: true,
        rows: Vec::new(),
        attempts: Vec::new(),
        leases: Vec::new(),
    };
    for retiring in [0, 1, CAP] {
        assert_eq!(
            scenario.verdict(false, CAP, retiring),
            Some(true),
            "start_blocked_only_by_capacity must count retiring leases toward held \
             (held {CAP}, retiring {retiring})"
        );
    }
}

// ---------------------------------------------------------------------------
// Agreement with the reconciler
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct RowSpec {
    class: Class,
    lease: Option<GuestAttachmentLease>,
    issued: Option<u64>,
    attempts: Option<u32>,
}

fn class_strategy(kind: WorkloadKind) -> BoxedStrategy<Class> {
    match kind {
        WorkloadKind::Job => prop_oneof![
            Just(Class::Crashed),
            Just(Class::Reclaimed),
            Just(Class::OperatorStopped),
            Just(Class::SystemGc),
            Just(Class::Completed),
            Just(Class::JobFailed),
            Just(Class::Running),
            Just(Class::Pending),
            Just(Class::Draining),
        ]
        .boxed(),
        _ => prop_oneof![
            Just(Class::Crashed),
            Just(Class::Reclaimed),
            Just(Class::OperatorStopped),
            Just(Class::SystemGc),
            Just(Class::LivenessKilled),
            Just(Class::StartupKilled),
            Just(Class::BackoffExhausted),
            Just(Class::ServiceFailed),
            Just(Class::Running),
            Just(Class::Pending),
            Just(Class::Draining),
        ]
        .boxed(),
    }
}

fn row_spec_strategy(kind: WorkloadKind) -> impl Strategy<Value = RowSpec> {
    (
        class_strategy(kind),
        prop::option::weighted(
            0.2,
            prop_oneof![Just(GuestAttachmentLease::Admitted), Just(GuestAttachmentLease::Retiring)],
        ),
        prop::option::of(0_u64..=3),
        prop::option::of(0_u32..=RESTART_BACKOFF_CEILING + 1),
    )
        .prop_map(|(class, lease, issued, attempts)| RowSpec {
            class,
            lease,
            issued,
            attempts,
        })
}

#[derive(Debug, Clone)]
struct Generated {
    kind: WorkloadKind,
    stop: bool,
    rows: BTreeMap<u32, RowSpec>,
    generation: u64,
}

fn generated_strategy() -> impl Strategy<Value = Generated> {
    prop_oneof![
        Just(WorkloadKind::Service),
        Just(WorkloadKind::Service),
        Just(WorkloadKind::Job),
        Just(WorkloadKind::Schedule),
    ]
    .prop_flat_map(|kind| {
        (
            Just(kind),
            any::<bool>(),
            prop::collection::btree_map(0_u32..=6, row_spec_strategy(kind), 0..=4),
            0_u64..=4,
        )
    })
    .prop_map(|(kind, stop, rows, generation)| Generated { kind, stop, rows, generation })
}

impl Generated {
    fn run_desired(&self) -> bool {
        // A Schedule has no `Job` shape (hydration projects `None`), so it never runs here.
        self.kind != WorkloadKind::Schedule && !self.stop
    }

    fn rows(&self) -> Vec<AllocStatusRow> {
        self.rows.iter().map(|(suffix, spec)| row(*suffix, self.kind, spec.class)).collect()
    }

    fn view(&self) -> WorkloadLifecycleView {
        let mut view = WorkloadLifecycleView::default();
        for (suffix, spec) in &self.rows {
            if let Some(issued) = spec.issued {
                view.issued_generation.insert(aid(*suffix), issued);
            }
            if let Some(attempts) = spec.attempts {
                view.restart_counts.insert(aid(*suffix), attempts);
            }
            // The failure was seen long before the tick: the backoff window has elapsed.
            view.last_failure_seen_at.insert(aid(*suffix), instant(1));
        }
        view
    }

    /// The current row's spec: the highest suffix (`current_alloc`).
    fn current_spec(&self) -> Option<&RowSpec> {
        self.rows.values().next_back()
    }

    /// Whether the current row holds a lease (the reconciler's row-neutral
    /// reclaim pass, not a wait) or is `Pending` (a launch it does not restart).
    fn current_is_leased_or_pending(&self) -> bool {
        self.current_spec().is_some_and(|spec| spec.lease.is_some() || spec.class == Class::Pending)
    }

    fn observation(&self, held: u32) -> GuestAttachmentObservation {
        GuestAttachmentObservation {
            occupancy: GuestAttachmentOccupancy { held, retiring: 0 },
            leases: self
                .rows
                .iter()
                .filter_map(|(suffix, spec)| spec.lease.map(|lease| (aid(*suffix), lease)))
                .collect(),
        }
    }
}

fn node() -> Node {
    // Ample: the reconciler's own CPU and memory check never blocks.
    Node {
        id: nid(),
        region: Region::new("local").expect("valid region"),
        capacity: Resources { cpu_milli: 64_000, memory_bytes: 256 * 1024 * 1024 * 1024 },
    }
}

fn workload_job() -> Job {
    Job {
        id: wid(),
        replicas: NonZeroU32::new(1).expect("one replica"),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: WorkloadDriver::Vm(Vm {
            command: "/sbin/workload".to_owned(),
            args: Vec::new(),
            kernel: "/srv/vm/kernel".to_owned(),
            rootfs: "/srv/vm/rootfs.ext4".to_owned(),
        }),
    }
}

/// `(desired, actual)` the way hydration builds them: `desired` carries the
/// intent and generation, `actual` the observed rows and the pool observation.
fn states(
    generated: &Generated,
    observation: &GuestAttachmentObservation,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let nodes = BTreeMap::from([(nid(), node())]);
    let job = (generated.kind != WorkloadKind::Schedule).then(workload_job);
    let allocations = generated
        .rows()
        .into_iter()
        .map(|row| (row.alloc_id.clone(), row))
        .collect::<BTreeMap<_, _>>();
    let desired = WorkloadLifecycleState {
        current_startup_probe: None,
        workload_id: wid(),
        job: job.clone(),
        desired_to_stop: generated.stop,
        generation: generated.generation,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: generated.kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        declared_listeners: Vec::new(),
        guest_attachments: observation.clone(),
    };
    let actual = WorkloadLifecycleState {
        current_startup_probe: None,
        workload_id: wid(),
        job,
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: generated.kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        declared_listeners: Vec::new(),
        guest_attachments: observation.clone(),
    };
    (desired, actual)
}

fn start_actions(actions: &[Action]) -> usize {
    actions
        .iter()
        .filter(|a| matches!(a, Action::StartAllocation { .. } | Action::RestartAllocation { .. }))
        .count()
}

fn reconcile_start_actions(
    generated: &Generated,
    observation: &GuestAttachmentObservation,
) -> usize {
    let (desired, actual) = states(generated, observation);
    let now = Instant::now();
    let tick = TickContext {
        now,
        // Far past `last_failure_seen_at + backoff`: no backoff window is open.
        now_unix: instant(100_000),
        tick: 1,
        deadline: now + Duration::from_secs(1),
    };
    let (actions, _) =
        WorkloadLifecycle::canonical().reconcile(&desired, &actual, &generated.view(), &tick);
    start_actions(&actions)
}

fn predicate(generated: &Generated, observation: &GuestAttachmentObservation) -> Option<bool> {
    let rows = generated.rows();
    let refs: Vec<&AllocStatusRow> = rows.iter().collect();
    let view = generated.view();
    scaffold_none(catch_unwind(AssertUnwindSafe(|| {
        start_blocked_only_by_capacity(
            generated.kind,
            generated.run_desired(),
            &refs,
            &view,
            generated.generation,
            observation,
        )
    })))
}

proptest! {
    // PENDING cases fail by design today; do not persist their seeds into the repo. Drop this
    // setting together with the `should_panic` markers at the activation step.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// (a) Whenever the predicate says "blocked only by capacity", the
    /// reconciler emits no `StartAllocation` / `RestartAllocation` at that
    /// observation: the line `describe` shows is never contradicted by a start.
    #[test]
    #[should_panic(
        expected = "start_blocked_only_by_capacity must agree with the reconciler: blocked means no start (the predicate returned no verdict)"
    )]
    fn a_blocked_workload_is_never_started(
        generated in generated_strategy(),
        held in prop_oneof![Just(CAP - 1), Just(CAP), Just(CAP + 1)],
    ) {
        let observation = generated.observation(held);
        let blocked = predicate(&generated, &observation);
        prop_assert!(
            blocked.is_some(),
            "start_blocked_only_by_capacity must agree with the reconciler: blocked means no start \
             (the predicate returned no verdict)"
        );
        if blocked == Some(true) {
            prop_assert_eq!(
                reconcile_start_actions(&generated, &observation),
                0,
                "start_blocked_only_by_capacity must agree with the reconciler: blocked means no start"
            );
        }
    }

    /// (b) Whenever the predicate over the at-cap observation says "blocked",
    /// the same state with the pool lowered to `cap - 1` and the backoff window
    /// elapsed makes the reconciler emit exactly one start: capacity is the only
    /// thing in the way.
    #[test]
    #[should_panic(
        expected = "start_blocked_only_by_capacity must agree with the reconciler: a freed slot starts the workload (the predicate returned no verdict)"
    )]
    fn freeing_one_slot_starts_a_blocked_workload(
        generated in generated_strategy(),
        held in prop_oneof![Just(CAP), Just(CAP + 1)],
    ) {
        let at_cap = generated.observation(held);
        let blocked = predicate(&generated, &at_cap);
        prop_assert!(
            blocked.is_some(),
            "start_blocked_only_by_capacity must agree with the reconciler: a freed slot starts \
             the workload (the predicate returned no verdict)"
        );
        if blocked == Some(true) {
            let lowered = generated.observation(CAP - 1);
            prop_assert_eq!(
                reconcile_start_actions(&generated, &lowered),
                1,
                "start_blocked_only_by_capacity must agree with the reconciler: a freed slot starts \
                 the workload"
            );
        }
    }

    /// (c) The converse of (a) and (b): whenever the reconciler emits no start at
    /// the at-cap observation and exactly one at `cap - 1`, capacity is the only
    /// thing in the way, so the predicate over the at-cap observation is true.
    /// A leased current row (the row-neutral reclaim pass, shown as
    /// `network_cleanup_pending`) and a `Pending` current row (a launch the
    /// reconciler does not restart) are outside the claim.
    #[test]
    #[should_panic(
        expected = "start_blocked_only_by_capacity must agree with the reconciler: a withheld start that a freed slot releases is blocked (the predicate returned no verdict)"
    )]
    fn a_start_a_freed_slot_releases_is_a_blocked_workload(
        generated in generated_strategy(),
        held in prop_oneof![Just(CAP), Just(CAP + 1)],
    ) {
        let at_cap = generated.observation(held);
        let lowered = generated.observation(CAP - 1);
        let released = reconcile_start_actions(&generated, &at_cap) == 0
            && reconcile_start_actions(&generated, &lowered) == 1
            && !generated.current_is_leased_or_pending();
        if released {
            let blocked = predicate(&generated, &at_cap);
            prop_assert!(
                blocked.is_some(),
                "start_blocked_only_by_capacity must agree with the reconciler: a withheld start \
                 that a freed slot releases is blocked (the predicate returned no verdict)"
            );
            prop_assert_eq!(
                blocked,
                Some(true),
                "start_blocked_only_by_capacity must agree with the reconciler: a withheld start \
                 that a freed slot releases is blocked"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Non-vacuity: the generator reaches every branch the properties quantify over
// ---------------------------------------------------------------------------

/// Cases in the fixed ensemble.
const ENSEMBLE: usize = 512;

/// A fixed ensemble drawn from the properties' own generator with proptest's
/// deterministic runner: the same cases on every run.
fn ensemble() -> Vec<(Generated, u32)> {
    let mut runner = TestRunner::deterministic();
    let strategy = (generated_strategy(), prop_oneof![Just(CAP - 1), Just(CAP), Just(CAP + 1)]);
    (0..ENSEMBLE)
        .map(|_| {
            strategy
                .new_tree(&mut runner)
                .expect("the agreement generator yields a value")
                .current()
        })
        .collect()
}

/// GUARD (passes today, judged by the reconciler alone): the generator emits
/// starts, withholds them at the cap, and reaches the population property (c)
/// quantifies over, so (a), (b) and (c) are not vacuous.
#[test]
fn the_agreement_generator_reaches_starts_withheld_starts_and_released_starts() {
    let (mut started_below_cap, mut withheld_at_cap, mut released_by_a_freed_slot, mut eligible) =
        (0_u32, 0_u32, 0_u32, 0_u32);
    for (generated, held) in ensemble() {
        let starts = reconcile_start_actions(&generated, &generated.observation(held));
        if held < CAP && starts >= 1 {
            started_below_cap += 1;
        }
        if held >= CAP {
            let lowered = reconcile_start_actions(&generated, &generated.observation(CAP - 1));
            if starts == 0 {
                withheld_at_cap += 1;
            }
            if starts == 0 && lowered == 1 {
                released_by_a_freed_slot += 1;
                if !generated.current_is_leased_or_pending() {
                    eligible += 1;
                }
            }
        }
    }
    assert!(started_below_cap >= 5, "the generator must emit starts below the cap");
    assert!(withheld_at_cap >= 5, "the generator must reach at-cap cases with no start");
    assert!(
        released_by_a_freed_slot >= 5,
        "the generator must reach cases where only a freed slot releases the start"
    );
    assert!(
        eligible >= 5,
        "the generator must reach released starts outside the leased and Pending exclusions"
    );
}

/// PENDING: over the same ensemble the predicate answers both ways, so the
/// `blocked == Some(true)` branches of (a) and (b) are reached and the verdict
/// is not constant.
#[test]
#[should_panic(
    expected = "start_blocked_only_by_capacity must be reached with both verdicts by the agreement generator (the predicate returned no verdict)"
)]
fn the_agreement_generator_reaches_both_predicate_verdicts() {
    let (mut blocked, mut free) = (0_u32, 0_u32);
    for (generated, held) in ensemble() {
        let verdict = predicate(&generated, &generated.observation(held)).expect(
            "start_blocked_only_by_capacity must be reached with both verdicts by the agreement \
             generator (the predicate returned no verdict)",
        );
        if verdict {
            blocked += 1;
        } else {
            free += 1;
        }
    }
    assert!(blocked >= 5, "the generator must reach cases the predicate calls blocked");
    assert!(free >= 5, "the generator must reach cases the predicate calls not blocked");
}
