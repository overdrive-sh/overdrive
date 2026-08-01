//! T-A + T-B (ADR-0078 § D6) — the postconditions of
//! [`CrashFacts::advance`], the single sanctioned producer of
//! `AllocStatusRow.last_terminated` and `.restart_count`, and the
//! snapshot fidelity of the `LastTerminated` it mints.
//!
//! WHY-NEW-FILE: crates/overdrive-core/tests/acceptance/crash_facts_advance.rs
//!   CLOSEST-EXISTING: crates/overdrive-core/tests/acceptance/logical_timestamp_dominating.rs
//!   EXTENSION-COST: that file's module doc, filename and mutation table
//!     all declare it the surface for `LogicalTimestamp::dominating` —
//!     the LWW *stamp* constructor. Folding a second constructor's
//!     property suite in would make the filename false and blur the two
//!     mutation tables, which name disjoint mutants (`max`/`min` and
//!     `saturating_add` arity there; match-arm deletion, boolean flip
//!     and `+1`/`+0` here).
//!   PARALLEL-RATIONALE: different function under test on a different
//!     type (`CrashFacts::advance` over a whole `AllocStatusRow`, not
//!     `LogicalTimestamp::dominating` over a stamp), and a different
//!     generated space (`(prior_row, next_state)` including the terminal
//!     predicate, vs. `(tick_floor, writer, prior_stamp)`).
//!
//! # The contract under test (ADR-0078 § D1)
//!
//! An `AllocStatusRow` is merged last-write-wins on one key per
//! allocation, full-row writes only. A crash-then-restart therefore ends
//! at `Running` and the crash is **unobservable** on the durable surface
//! — the LWW-Register discards intermediate values by construction.
//! `advance` closes that by snapshotting the superseded terminal into a
//! depth-1 `last_terminated` and bumping a monotone `restart_count`, both
//! of which survive every subsequent merge because they ride the row.
//!
//! # Mutation surface
//!
//! `advance` is a comparison-and-arithmetic function over a match, so it
//! carries the canonical `+1`/`+0`, match-arm-deletion and boolean-flip
//! surface and a 100 % kill obligation (`.claude/rules/testing.md`
//! § "Mandatory targets"). Each mutant and the case that flips it:
//!
//! | Mutation | Killed by |
//! |---|---|
//! | `restart_count.saturating_add(1)` → `+ 0` | [`recovery_from_a_crash_snapshots_and_increments`] and the proptest's increment leg |
//! | drop the `prior_terminal` guard on the increment | [`advance_forwards_on_a_non_terminal_prior`] (a `Running` prior must not bump) |
//! | drop the `!next_state.is_terminal()` guard on the snapshot | [`terminal_to_terminal_forwards_it_does_not_snapshot`] |
//! | swap `Running` for any other `next_state` on the increment | [`recovery_to_a_non_running_state_snapshots_without_incrementing`] |
//! | replace the `prior == None` arm with a forward-carry | [`advance_without_a_prior_is_the_zero_value`] |
//! | drop or swap any field in `from_superseded` | [`snapshot_is_field_for_field_verbatim`] (T-B) |
//! | `AllocState::is_terminal` returning a wider/narrower set | [`is_terminal_is_exactly_terminated_and_failed`] |

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, CrashFacts, LastTerminated, LogicalTimestamp,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use proptest::prelude::*;

/// Every `AllocState`, so the generated `(prior.state, next_state)` space
/// covers the terminal predicate on both sides of the transition.
const ALL_STATES: &[AllocState] = &[
    AllocState::Pending,
    AllocState::Running,
    AllocState::Draining,
    AllocState::Suspended,
    AllocState::Terminated,
    AllocState::Failed,
];

fn node(name: &str) -> NodeId {
    NodeId::new(name).expect("valid node id")
}

fn ts(counter: u64) -> LogicalTimestamp {
    LogicalTimestamp { counter, writer: node("local") }
}

/// A row carrying DISTINCT, non-default values in every field
/// `LastTerminated` snapshots, so a mutant that drops or swaps one of
/// them is observable rather than masked by a shared `None`.
fn row(state: AllocState) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new("alloc-recovery-0").expect("valid alloc id"),
        workload_id: WorkloadId::new("recovery").expect("valid workload id"),
        node_id: node("local"),
        state,
        updated_at: ts(2),
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: Some("Segmentation fault".to_owned()),
        }),
        detail: Some("killed by SIGKILL".to_owned()),
        terminal: Some(TerminalCondition::Failed { exit_code: Some(137) }),
        stderr_tail: Some("Segmentation fault".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: Some(std::net::Ipv4Addr::new(10, 99, 0, 6)),
        last_terminated: None,
        restart_count: 0,
    }
}

/// A `LastTerminated` describing some OTHER, earlier terminal — used to
/// prove the forward-carry arms return the prior's value verbatim rather
/// than re-minting one.
fn earlier_terminal() -> LastTerminated {
    LastTerminated {
        state: AllocState::Terminated,
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Process }),
        detail: Some("an earlier, unrelated terminal".to_owned()),
        terminal: Some(TerminalCondition::Completed { exit_code: 0 }),
        stderr_tail: None,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_600_000_000))),
        terminated_at: ts(1),
    }
}

// ---------------------------------------------------------------------------
// T-A — the postconditions, property-based over (prior, next_state)
// ---------------------------------------------------------------------------

prop_compose! {
    fn arb_state()(idx in 0..ALL_STATES.len()) -> AllocState {
        ALL_STATES[idx]
    }
}

prop_compose! {
    /// `restart_count` is bounded BELOW `u32::MAX`: the contract states
    /// explicitly that a saturated counter clamps and the strict
    /// increment does not hold there. Excluding it is the stated
    /// precondition, not a convenience — the saturation edge is pinned
    /// deterministically by
    /// [`saturated_restart_count_clamps_as_documented`].
    ///
    /// `last_terminated` is generated BOTH absent and present so the
    /// forward-carry arms are exercised against a non-`None` value; a
    /// generator that only ever produced `None` would let a mutant
    /// replacing the forward-carry with a literal `None` survive.
    fn arb_prior()(
        state in arb_state(),
        restart_count in 0..u32::MAX,
        carries_earlier in any::<bool>(),
    ) -> AllocStatusRow {
        let mut r = row(state);
        r.restart_count = restart_count;
        r.last_terminated = carries_earlier.then(earlier_terminal);
        r
    }
}

proptest! {
    /// Monotonicity, verbatim from § D1: for every `p` and every `s`,
    /// `p.restart_count <= advance(Some(p), s).restart_count
    ///  <= p.restart_count + 1` — never more than one increment per write.
    #[test]
    fn restart_count_is_monotone_and_bumps_by_at_most_one(
        prior in arb_prior(),
        next_state in arb_state(),
    ) {
        let out = CrashFacts::advance(Some(&prior), next_state);
        prop_assert!(
            out.restart_count >= prior.restart_count,
            "restart_count must never decrease ({} -> {})",
            prior.restart_count,
            out.restart_count,
        );
        prop_assert!(
            out.restart_count <= prior.restart_count + 1,
            "restart_count must bump by at most one ({} -> {})",
            prior.restart_count,
            out.restart_count,
        );
    }

    /// The increment fires IFF `p.state.is_terminal() && next_state == Running`.
    /// Stated as an `iff`, so both a missing increment and a spurious one
    /// fail.
    #[test]
    fn restart_count_increments_exactly_on_a_terminal_to_running_transition(
        prior in arb_prior(),
        next_state in arb_state(),
    ) {
        let out = CrashFacts::advance(Some(&prior), next_state);
        let expected_bump =
            prior.state.is_terminal() && matches!(next_state, AllocState::Running);
        prop_assert_eq!(
            out.restart_count,
            if expected_bump { prior.restart_count + 1 } else { prior.restart_count },
            "prior {:?} -> {:?}: increment must fire iff terminal -> Running (expected bump: {})",
            prior.state,
            next_state,
            expected_bump,
        );
    }

    /// `last_terminated` is a SNAPSHOT of the prior iff the transition is
    /// `terminal -> non-terminal`; in EVERY other case it is the prior's
    /// own value, forwarded verbatim.
    #[test]
    fn last_terminated_snapshots_only_on_a_recovery_and_otherwise_forwards(
        prior in arb_prior(),
        next_state in arb_state(),
    ) {
        let out = CrashFacts::advance(Some(&prior), next_state);
        if prior.state.is_terminal() && !next_state.is_terminal() {
            let snapshot = out
                .last_terminated
                .as_ref()
                .expect("a terminal -> non-terminal transition must snapshot");
            prop_assert_eq!(snapshot.state, prior.state);
            prop_assert_eq!(&snapshot.reason, &prior.reason);
            prop_assert_eq!(&snapshot.terminated_at, &prior.updated_at);
        } else {
            prop_assert_eq!(
                &out.last_terminated,
                &prior.last_terminated,
                "prior {:?} -> {:?} must forward the prior's last_terminated verbatim",
                prior.state,
                next_state,
            );
        }
    }

    /// The `prior == None` arm: a genuinely-first write at the key has no
    /// crash history to carry and no restart to count.
    #[test]
    fn advance_without_a_prior_is_the_zero_value(next_state in arb_state()) {
        let out = CrashFacts::advance(None, next_state);
        prop_assert_eq!(
            out,
            CrashFacts { last_terminated: None, restart_count: 0 },
            "no prior at the key: nothing observed, nothing counted",
        );
    }
}

// ---------------------------------------------------------------------------
// T-B — snapshot fidelity: the falsifiable form of § D1's membership rule
// ---------------------------------------------------------------------------

/// Every one of the seven `LastTerminated` fields equals the
/// corresponding field of the superseded row, byte-for-byte. This kills a
/// mutant that drops or swaps a field in `from_superseded` — which the
/// T-A properties cannot, because they assert on three fields only.
///
/// The membership rule (§ D1) is also asserted structurally: the snapshot
/// carries EXACTLY the fields a successor write overwrites. Fields every
/// writer forward-carries anyway (`alloc_id`, `workload_id`, `node_id`,
/// `kind`, `listeners`, `workload_addr`) are absent from the type, so the
/// only way to check "nothing extra" is that the struct literal below
/// compiles with exactly these seven fields.
#[test]
fn snapshot_is_field_for_field_verbatim() {
    let prior = row(AllocState::Failed);
    let out = CrashFacts::advance(Some(&prior), AllocState::Running);
    let snapshot = out.last_terminated.expect("a recovery must snapshot");

    assert_eq!(
        snapshot,
        LastTerminated {
            state: prior.state,
            reason: prior.reason.clone(),
            detail: prior.detail.clone(),
            terminal: prior.terminal.clone(),
            stderr_tail: prior.stderr_tail.clone(),
            started_at: prior.started_at,
            terminated_at: prior.updated_at,
        },
        "every LastTerminated field must be the superseded row's field verbatim",
    );
}

// ---------------------------------------------------------------------------
// The named edge cases (§ D1), pinned deterministically so the mutants the
// proptest kills only probabilistically are killed on every run.
// ---------------------------------------------------------------------------

/// THE crash-observability shape: `Failed -> Running` snapshots the crash
/// AND counts the restart. This is the transition the whole ADR exists to
/// make durably observable.
#[test]
fn recovery_from_a_crash_snapshots_and_increments() {
    let prior = row(AllocState::Failed);
    let out = CrashFacts::advance(Some(&prior), AllocState::Running);

    assert_eq!(out.restart_count, 1, "the restart is counted");
    let snapshot = out.last_terminated.expect("the crash is snapshotted");
    assert_eq!(snapshot.state, AllocState::Failed, "the SIGKILL was classified Failed");
    assert!(
        matches!(snapshot.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "the typed cause-class rides the snapshot verbatim: {:?}",
        snapshot.reason,
    );
}

/// § D1 edge case 1 — the `FinalizeFailed` shape. `terminal -> terminal`
/// FORWARDS; it does not snapshot. Snapshotting there would put the row's
/// own `reason` / `detail` / `stderr_tail` / `started_at` / `terminal` on
/// the row TWICE, which is the two-sources-of-truth duplication § D1
/// rejects.
#[test]
fn terminal_to_terminal_forwards_it_does_not_snapshot() {
    let mut prior = row(AllocState::Failed);
    prior.last_terminated = Some(earlier_terminal());
    prior.restart_count = 4;

    let out = CrashFacts::advance(Some(&prior), AllocState::Failed);

    assert_eq!(out.restart_count, 4, "a re-stamped terminal is not a restart");
    assert_eq!(
        out.last_terminated,
        Some(earlier_terminal()),
        "the prior's own snapshot is forwarded, NOT replaced by a self-snapshot",
    );
}

/// § D1 edge case 2 — a driver-REJECTED restart (`Failed -> Failed`)
/// forwards and does not increment. Nothing restarted.
#[test]
fn driver_rejected_restart_neither_snapshots_nor_increments() {
    let mut prior = row(AllocState::Failed);
    prior.restart_count = 2;
    let out = CrashFacts::advance(Some(&prior), AllocState::Failed);

    assert_eq!(out.restart_count, 2, "a rejected restart is not an observed restart");
    assert_eq!(out.last_terminated, None, "the prior carried none; none is forwarded");
}

/// § D1 edge case 3 — the mTLS fail-closed supersession
/// (`Running -> Failed` within one dispatch). The prior is `Running`, so
/// no snapshot and no increment.
#[test]
fn advance_forwards_on_a_non_terminal_prior() {
    let mut prior = row(AllocState::Running);
    prior.restart_count = 7;
    prior.last_terminated = Some(earlier_terminal());

    for next_state in ALL_STATES {
        let out = CrashFacts::advance(Some(&prior), *next_state);
        assert_eq!(
            out.restart_count, 7,
            "a non-terminal prior never increments (next_state {next_state:?})",
        );
        assert_eq!(
            out.last_terminated,
            Some(earlier_terminal()),
            "a non-terminal prior always forwards (next_state {next_state:?})",
        );
    }
}

/// § D1 edge case 4 — re-dispatching against an already-`Running` row is
/// idempotent on both fields.
#[test]
fn redispatch_against_a_running_row_is_idempotent() {
    let mut prior = row(AllocState::Running);
    prior.restart_count = 3;
    prior.last_terminated = Some(earlier_terminal());

    let once = CrashFacts::advance(Some(&prior), AllocState::Running);
    // Apply the facts back onto the row, exactly as `build_alloc_status_row`
    // does, then advance a second time against that row.
    let mut applied = prior;
    applied.last_terminated = once.last_terminated.clone();
    applied.restart_count = once.restart_count;
    let twice = CrashFacts::advance(Some(&applied), AllocState::Running);

    assert_eq!(once, twice, "re-dispatching against a Running row changes nothing");
    assert_eq!(once.restart_count, 3);
}

/// The snapshot arm is gated on `!next_state.is_terminal()`, NOT on
/// `next_state == Running` — a `terminal -> Pending` recovery snapshots
/// but does not count a restart (nothing reached Running). Pins the two
/// guards as INDEPENDENT, killing a mutant that collapses them into one.
#[test]
fn recovery_to_a_non_running_state_snapshots_without_incrementing() {
    for next_state in [AllocState::Pending, AllocState::Draining, AllocState::Suspended] {
        let prior = row(AllocState::Terminated);
        let out = CrashFacts::advance(Some(&prior), next_state);

        assert!(
            out.last_terminated.is_some(),
            "terminal -> {next_state:?} is a recovery: it snapshots",
        );
        assert_eq!(
            out.restart_count, 0,
            "terminal -> {next_state:?} never reached Running: no restart counted",
        );
    }
}

/// § D1 edge case 5 — an operator-stopped `Terminated` prior counts like
/// any other terminal. `advance` deliberately does NOT consult an
/// intentional-stop discriminator; the exclusion happens upstream
/// (`is_restartable` never emits a `RestartAllocation` for one). Pinned so
/// the documented deliberate omission is falsifiable rather than folklore.
#[test]
fn an_operator_stopped_terminal_counts_like_any_other() {
    let mut prior = row(AllocState::Terminated);
    prior.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
    prior.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });

    let out = CrashFacts::advance(Some(&prior), AllocState::Running);

    assert_eq!(out.restart_count, 1, "advance does not special-case an intentional stop");
    assert_eq!(out.last_terminated.as_ref().map(|lt| lt.state), Some(AllocState::Terminated),);
}

/// § D1 edge case 6 — `p.restart_count == u32::MAX` clamps; the strict
/// increment does not hold. Pinned so the documented limit is falsifiable.
#[test]
fn saturated_restart_count_clamps_as_documented() {
    let mut prior = row(AllocState::Failed);
    prior.restart_count = u32::MAX;

    let out = CrashFacts::advance(Some(&prior), AllocState::Running);

    assert_eq!(out.restart_count, u32::MAX, "saturating_add clamps at the ceiling");
    assert!(
        out.last_terminated.is_some(),
        "the snapshot half is unaffected by the counter saturating",
    );
}

/// The observable invariant (§ D1): across the whole life of one
/// allocation key, `restart_count` is non-decreasing and `last_terminated`
/// describes the MOST RECENT terminal the allocation survived —
/// independently of how many intermediate LWW values were discarded.
///
/// Drives two full crash cycles at the pure-function level. This is the
/// unit-level sibling of T-F (the integration-level two-cycle test); the
/// two are complementary, not redundant — this one pins the function,
/// T-F pins that every production call site forward-carries correctly.
#[test]
fn two_crash_cycles_count_two_and_describe_the_second_terminal() {
    // Cycle 1: Running -> Failed (crash 1) -> Running (recovery 1).
    let mut current = row(AllocState::Running);
    current.last_terminated = None;
    current.restart_count = 0;

    let mut crash_one = current.clone();
    crash_one.state = AllocState::Failed;
    crash_one.updated_at = ts(2);
    crash_one.detail = Some("crash one".to_owned());
    let facts = CrashFacts::advance(Some(&current), AllocState::Failed);
    crash_one.last_terminated = facts.last_terminated;
    crash_one.restart_count = facts.restart_count;

    let mut recovery_one = crash_one.clone();
    recovery_one.state = AllocState::Running;
    recovery_one.updated_at = ts(3);
    let facts = CrashFacts::advance(Some(&crash_one), AllocState::Running);
    recovery_one.last_terminated = facts.last_terminated;
    recovery_one.restart_count = facts.restart_count;

    assert_eq!(recovery_one.restart_count, 1, "one crash survived");
    assert_eq!(
        recovery_one.last_terminated.as_ref().and_then(|lt| lt.detail.as_deref()),
        Some("crash one"),
    );

    // Cycle 2: Failed (crash 2) -> Running (recovery 2).
    let mut crash_two = recovery_one.clone();
    crash_two.state = AllocState::Failed;
    crash_two.updated_at = ts(4);
    crash_two.detail = Some("crash two".to_owned());
    let facts = CrashFacts::advance(Some(&recovery_one), AllocState::Failed);
    crash_two.last_terminated = facts.last_terminated;
    crash_two.restart_count = facts.restart_count;

    // The crash row itself still describes the PREVIOUS terminal — the
    // invariant "`last_terminated` never describes the row that carries it".
    assert_eq!(
        crash_two.last_terminated.as_ref().and_then(|lt| lt.detail.as_deref()),
        Some("crash one"),
        "the crash row forwards; it does not self-describe",
    );

    let mut recovery_two = crash_two.clone();
    recovery_two.state = AllocState::Running;
    recovery_two.updated_at = ts(5);
    let facts = CrashFacts::advance(Some(&crash_two), AllocState::Running);
    recovery_two.last_terminated = facts.last_terminated;
    recovery_two.restart_count = facts.restart_count;

    assert_eq!(recovery_two.restart_count, 2, "two crashes survived");
    assert_eq!(
        recovery_two.last_terminated.as_ref().and_then(|lt| lt.detail.as_deref()),
        Some("crash two"),
        "the depth-1 snapshot describes the SECOND terminal, not the first",
    );
    assert_eq!(
        recovery_two.last_terminated.as_ref().map(|lt| lt.terminated_at.counter),
        Some(4),
        "and it identifies exactly WHICH durable observation it summarises",
    );
}

/// `AllocState::is_terminal` is the single terminal-detection site
/// consulted by `advance` and by `WorkloadLifecycle::is_natural_exit`.
/// Pinned exhaustively so a widened or narrowed set fails here rather
/// than silently changing both consumers.
///
/// `Draining` is deliberately NOT terminal — it is
/// transient-and-restartable. `is_restartable`'s wider set
/// (`Terminated | Draining | Failed`) is a DIFFERENT predicate and is not
/// this method.
#[test]
fn is_terminal_is_exactly_terminated_and_failed() {
    assert!(AllocState::Terminated.is_terminal());
    assert!(AllocState::Failed.is_terminal());
    assert!(!AllocState::Pending.is_terminal());
    assert!(!AllocState::Running.is_terminal());
    assert!(!AllocState::Draining.is_terminal(), "Draining is transient-and-restartable");
    assert!(!AllocState::Suspended.is_terminal());
}
