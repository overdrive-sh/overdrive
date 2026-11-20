//! `generation_replacement_pending` — the one definition of `restart_pending`
//! (ADR-0326, node-local persistent volume, GH #348; scenario family
//! S-BIR-PENDING).
//!
//! CONTRACT_SHAPE: pure-function. Owner path:
//! `overdrive_reconcilers::generation_replacement_pending(view, desired_generation, current)`
//! with `current` selected by `overdrive_reconcilers::current_alloc` over the
//! observed rows. Oracle: the function's boolean result against an independent
//! restatement of the contract ("true iff the current row exists, is not a
//! finished Job row, and `view.issued_generation[row]` (absent reads 0) is
//! below the generation"), and against the action shim's own fence,
//! `allocation_attempt_transition(..) == NoChange`, over the same generated
//! rows. Tier: Tier 1, default lane (pure, no I/O).
//!
//! Status of every test in this file: PENDING (proven RED). The pinned
//! production scaffold panics with "RED scaffold", so each body calls the
//! function through `pending`, which catches that unwind and re-panics with
//! this file's own named assertion; `#[should_panic(expected = ..)]` names that
//! assertion, never the scaffold text. DELIVER removes the markers when it
//! implements the function.
//!
//! Mutation targets killed (`workload_lifecycle.rs`, the helper):
//! - the `<` (an `<=` or `>` mutant): the equal-generation and
//!   below/above examples, and the property whose entries and generation
//!   share a range so equality is generated;
//! - the finished-Job exception (removed or widened to every kind): the Job
//!   examples, the Service-with-terminal example and the shim-fence agreement
//!   property.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Duration;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::workload_lifecycle::{
    AllocationAttemptEvent, AllocationAttemptTransition, allocation_attempt_transition,
};
use overdrive_reconcilers::{WorkloadLifecycleView, current_alloc, generation_replacement_pending};
use proptest::prelude::*;

const WORKLOAD: &str = "payments";

/// The assertion every test of this file names when the function does not yet
/// return the contract's answer.
const CONTRACT: &str = "generation_replacement_pending must be true iff the current row exists, \
     is not a finished Job row, and its issued_generation entry (absent reads 0) is below the \
     desired generation";

fn aid(suffix: &str) -> AllocationId {
    AllocationId::new(&format!("alloc-{WORKLOAD}-{suffix}")).expect("valid allocation id")
}

fn nid() -> NodeId {
    NodeId::new("local").expect("valid node id")
}

fn row(
    suffix: &str,
    kind: WorkloadKind,
    state: AllocState,
    terminal: Option<TerminalCondition>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(suffix),
        workload_id: WorkloadId::new(WORKLOAD).expect("valid workload id"),
        node_id: nid(),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: nid() },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(23),
            signal: None,
            stderr_tail: None,
        }),
        detail: None,
        terminal,
        stderr_tail: None,
        kind,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(10))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn failed_service(suffix: &str) -> AllocStatusRow {
    row(suffix, WorkloadKind::Service, AllocState::Failed, None)
}

fn view_with(entries: &[(&str, u64)]) -> WorkloadLifecycleView {
    let mut view = WorkloadLifecycleView::default();
    for (suffix, generation) in entries {
        view.issued_generation.insert(aid(suffix), *generation);
    }
    view
}

/// The payload of the production scaffold's panic, matched exactly: only this
/// panic is the pending state; any other panic is a real failure and propagates.
const SCAFFOLD_PANIC: &str =
    "RED scaffold: generation_replacement_pending is not implemented (ADR-0326)";

fn panic_text(payload: &(dyn Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

/// Call the function over `rows` the way the reconciler does: `current` is
/// `current_alloc` over the observed rows. The scaffold's unwind (exactly its
/// literal payload) is re-raised as this file's named assertion; any other
/// panic is resumed unchanged.
fn pending(rows: &[AllocStatusRow], view: &WorkloadLifecycleView, generation: u64) -> bool {
    let refs: Vec<&AllocStatusRow> = rows.iter().collect();
    let current = current_alloc(&refs);
    match catch_unwind(AssertUnwindSafe(|| {
        generation_replacement_pending(view, generation, current)
    })) {
        Ok(answer) => answer,
        Err(payload) => {
            let is_scaffold = panic_text(&*payload) == Some(SCAFFOLD_PANIC);
            assert!(!is_scaffold, "{CONTRACT}");
            resume_unwind(payload)
        }
    }
}

// ---------------------------------------------------------------------------
// Named examples
// ---------------------------------------------------------------------------

/// With no observed row nothing is pending: an issued id with no row is
/// invisible, so a first placement is never "a replacement".
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn no_row_means_no_generation_is_pending() {
    let view = view_with(&[("0", 0)]);
    assert!(
        !pending(&[], &view, 3),
        "no current row: the generation is not pending, whatever the View holds"
    );
}

/// The current row's entry is below the committed generation: pending.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_current_row_issued_for_an_older_generation_is_pending() {
    let view = view_with(&[("0", 1)]);
    assert!(pending(&[failed_service("0")], &view, 2), "entry 1 < generation 2 is pending");
}

/// Mutation target: the `<`. An entry equal to the generation is handled.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_current_row_issued_for_the_committed_generation_is_handled() {
    let view = view_with(&[("0", 2)]);
    assert!(!pending(&[failed_service("0")], &view, 2), "entry 2 == generation 2 is not pending");
}

/// An entry above the generation (the generation never moves backwards, but the
/// comparison must not read as "not equal") is not pending.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_current_row_issued_for_a_newer_generation_is_not_pending() {
    let view = view_with(&[("0", 5)]);
    assert!(!pending(&[failed_service("0")], &view, 2), "entry 5 > generation 2 is not pending");
}

/// An absent entry reads 0: pending once the generation is above 0, not before.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn an_absent_entry_reads_zero() {
    let view = WorkloadLifecycleView::default();
    assert!(pending(&[failed_service("0")], &view, 1), "absent entry (0) < generation 1");
    assert!(!pending(&[failed_service("0")], &view, 0), "absent entry (0) == generation 0");
}

/// The issued id's row is absent: the View names a newer id the observation does
/// not show, `current_alloc` is still the older row, so the generation is still
/// pending (the relaunch after an abandoned launch).
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn an_issued_id_without_a_row_leaves_the_generation_pending() {
    let view = view_with(&[("0", 1), ("1", 2)]);
    assert!(
        pending(&[failed_service("0")], &view, 2),
        "the issued successor has no row: the older row is current and still below the generation"
    );
}

/// Once the issued id has a row, that row is current and its entry equals the
/// generation: handled.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn the_successors_row_ends_the_pending_generation() {
    let view = view_with(&[("0", 1), ("1", 2)]);
    let rows = [failed_service("0"), row("1", WorkloadKind::Service, AllocState::Running, None)];
    assert!(!pending(&rows, &view, 2), "the successor's row is current and serves generation 2");
}

/// The current row is the numerically highest suffix (`-10` over `-2`, which a
/// lexical maximum would get wrong).
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn the_numeric_maximum_suffix_selects_the_current_row() {
    let rows = [failed_service("2"), failed_service("10")];
    // `-10` serves the generation, `-2` does not: numeric current => handled.
    let handled = view_with(&[("2", 0), ("10", 3)]);
    assert!(!pending(&rows, &handled, 3), "`-10` is current (numeric max) and serves generation 3");
    // The converse: `-10` is stale, `-2` is fresh: numeric current => pending.
    let stale = view_with(&[("2", 3), ("10", 0)]);
    assert!(pending(&rows, &stale, 3), "`-10` is current (numeric max) and is below generation 3");
}

/// A row whose id has no parseable attempt suffix is never the current row.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_row_without_an_attempt_suffix_is_never_current() {
    let rows = [failed_service("0"), failed_service("orphan")];
    let view = view_with(&[("0", 3), ("orphan", 0)]);
    assert!(
        !pending(&rows, &view, 3),
        "`-orphan` has no attempt index: `-0` (generation 3) is current, nothing is pending"
    );
}

/// H1, mutation target (the finished-Job exception): a Job whose current row
/// completed is never given a replacement by a restart, even below the
/// generation.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_completed_job_is_never_pending() {
    let rows = [row(
        "0",
        WorkloadKind::Job,
        AllocState::Terminated,
        Some(TerminalCondition::Completed { exit_code: 0 }),
    )];
    assert!(!pending(&rows, &view_with(&[("0", 0)]), 4), "a completed Job: not pending");
}

/// H1: a failed Job.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_failed_job_is_never_pending() {
    let rows = [row(
        "0",
        WorkloadKind::Job,
        AllocState::Failed,
        Some(TerminalCondition::Failed { exit_code: Some(1) }),
    )];
    assert!(!pending(&rows, &view_with(&[("0", 0)]), 4), "a failed Job: not pending");
}

/// H1: an operator-stopped Job whose row carries `Stopped { by: Operator }` as
/// its terminal (the stop's `StopAllocation` carries it).
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn an_operator_stopped_job_with_a_terminal_is_never_pending() {
    let rows = [row(
        "0",
        WorkloadKind::Job,
        AllocState::Terminated,
        Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
    )];
    assert!(!pending(&rows, &view_with(&[("0", 0)]), 4), "an operator-stopped Job: not pending");
}

/// A Job row that carries only a `reason` and no terminal is replaced exactly
/// as on `main`: pending below the generation.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_job_row_without_a_terminal_is_pending_below_the_generation() {
    let mut job = row("0", WorkloadKind::Job, AllocState::Terminated, None);
    job.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
    assert!(pending(&[job], &view_with(&[("0", 0)]), 4), "a Job row with no terminal is replaced");
}

/// The exception is the Job kind only: a Service row carrying a terminal
/// (`BackoffExhausted`, an operator stop) is still replaced by a restart.
#[test]
#[should_panic(
    expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation"
)]
fn a_service_row_with_a_terminal_is_pending_below_the_generation() {
    for terminal in [
        TerminalCondition::BackoffExhausted { attempts: 5 },
        TerminalCondition::Stopped { by: StoppedBy::Operator },
    ] {
        let rows =
            [row("0", WorkloadKind::Service, AllocState::Terminated, Some(terminal.clone()))];
        assert!(
            pending(&rows, &view_with(&[("0", 0)]), 4),
            "a Service row with terminal {terminal:?} is replaced by a restart"
        );
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct GeneratedRow {
    suffix: u32,
    kind: WorkloadKind,
    state: AllocState,
    terminal: Option<TerminalCondition>,
    entry: Option<u64>,
}

fn kind_strategy() -> impl Strategy<Value = WorkloadKind> {
    prop_oneof![Just(WorkloadKind::Service), Just(WorkloadKind::Job)]
}

fn state_strategy() -> impl Strategy<Value = AllocState> {
    prop_oneof![
        Just(AllocState::Pending),
        Just(AllocState::Running),
        Just(AllocState::Draining),
        Just(AllocState::Terminated),
        Just(AllocState::Failed),
    ]
}

fn terminal_strategy() -> impl Strategy<Value = Option<TerminalCondition>> {
    prop_oneof![
        3 => Just(None),
        1 => Just(Some(TerminalCondition::Completed { exit_code: 0 })),
        1 => Just(Some(TerminalCondition::Failed { exit_code: Some(1) })),
        1 => Just(Some(TerminalCondition::Stopped { by: StoppedBy::Operator })),
        1 => Just(Some(TerminalCondition::BackoffExhausted { attempts: 5 })),
    ]
}

/// Entries and the desired generation share the range `0..=4`, so equality,
/// below and above are all generated.
fn rows_strategy() -> impl Strategy<Value = Vec<GeneratedRow>> {
    prop::collection::btree_map(
        0_u32..=12,
        (kind_strategy(), state_strategy(), terminal_strategy(), prop::option::of(0_u64..=4)),
        0..=6,
    )
    .prop_map(|rows| {
        rows.into_iter()
            .map(|(suffix, (kind, state, terminal, entry))| GeneratedRow {
                suffix,
                kind,
                state,
                terminal,
                entry,
            })
            .collect()
    })
}

fn materialise(generated: &[GeneratedRow]) -> (Vec<AllocStatusRow>, WorkloadLifecycleView) {
    let mut view = WorkloadLifecycleView::default();
    let mut rows = Vec::with_capacity(generated.len());
    for g in generated {
        let suffix = g.suffix.to_string();
        if let Some(entry) = g.entry {
            view.issued_generation.insert(aid(&suffix), entry);
        }
        rows.push(row(&suffix, g.kind, g.state, g.terminal.clone()));
    }
    (rows, view)
}

/// The contract restated independently: the highest numeric suffix is current.
fn expected(generated: &[GeneratedRow], generation: u64) -> bool {
    let Some(current) = generated.iter().max_by_key(|g| g.suffix) else { return false };
    let finished_job = current.kind == WorkloadKind::Job && current.terminal.is_some();
    !finished_job && current.entry.unwrap_or(0) < generation
}

proptest! {
    // PENDING cases fail by design today; do not persist their seeds into the repo. Drop this
    // setting together with the `should_panic` markers at the activation step.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// For every generated `(rows, view, generation)` the function equals the
    /// contract. Kills: the `<`, the finished-Job exception, the absent-reads-0
    /// default, the current-row selection.
    #[test]
    #[should_panic(expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation")]
    fn pending_is_true_iff_the_current_row_is_below_the_generation_and_not_a_finished_job(
        generated in rows_strategy(),
        generation in 0_u64..=4,
    ) {
        let (rows, view) = materialise(&generated);
        let want = expected(&generated, generation);
        let got = pending(&rows, &view, generation);
        prop_assert_eq!(got, want, "{}", CONTRACT);
    }

    /// The two fences agree on a finished Job: over the same generated rows, the
    /// generation is pending exactly when the current row's entry is below it
    /// AND `allocation_attempt_transition` would `Apply` (not `NoChange`). A
    /// finished-Job row is never pending, and the shim's late-event fence and the
    /// reconciler's predicate cannot disagree on it.
    #[test]
    #[should_panic(expected = "generation_replacement_pending must be true iff the current row exists, is not a finished Job row, and its issued_generation entry (absent reads 0) is below the desired generation")]
    fn pending_agrees_with_the_shims_finished_job_fence(
        generated in rows_strategy(),
        generation in 0_u64..=4,
    ) {
        let (rows, view) = materialise(&generated);
        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        let current = current_alloc(&refs);
        let got = pending(&rows, &view, generation);
        let want = current.is_some_and(|row| {
            let below = view.issued_generation.get(&row.alloc_id).copied().unwrap_or(0) < generation;
            let applies = [
                AllocationAttemptEvent::Ready,
                AllocationAttemptEvent::Dispatch,
                AllocationAttemptEvent::Finalize,
            ]
            .into_iter()
            .all(|event| allocation_attempt_transition(row, event) == AllocationAttemptTransition::Apply);
            below && applies
        });
        prop_assert_eq!(
            got,
            want,
            "generation_replacement_pending must be true iff the current row exists, is not a \
             finished Job row (allocation_attempt_transition == Apply), and is below the generation"
        );
    }
}
