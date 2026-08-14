//! `VmReclamation`'s pure diff — `microvm-driver-cloud-hypervisor` (GH #42),
//! cross-cutting (SD-1, `reconcilers.md` Bar 2).
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § "Cross-cutting — `VmReclamation` reconciler" (S-VM-31, S-VM-32,
//! S-VM-92) and ADR-0083 §D7, `brief.md` §§105a.3-105a.4. All three are
//! `@property` scenarios and mandatory mutation targets.
//!
//! Per CLAUDE.md § "Implement to the design": `plan_reclamation` takes NO
//! port parameter (pure, no I/O) and `SupervisionSet` has EXACTLY two
//! inhabitants (`Unavailable` as `Default`, `Observed(set)`).
//!
//! S-VM-32's "Platform Reclamation" ending class is structurally
//! UNREACHABLE today: `StoppedBy::PlatformReclaimed` (ADR-0081 D5) has not
//! landed — its addition is bundled by ADR-0081's own "Narrows ADR-0078
//! §D1" section with a same-commit ADR-0078 amendment + an
//! `observation_store.rs` docstring correction, which lands with
//! `execute_reclaim_allocation` (the first real producer of the
//! disposition), not with this reconciler skeleton (step 02-01). The
//! totality/disjointness property below is proven over the two classes
//! representable today (`is_intentional_stop`, `is_workload_failure`) plus
//! the structurally-`false` `is_platform_reclamation` placeholder — see
//! `overdrive_core::reconcilers::vm_reclamation`'s module docs for the full
//! reasoning.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use overdrive_core::AllocationId;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{NodeId, WorkloadId};
use overdrive_core::reconcilers::{
    Action, SupervisionSet, VmAllocFacts, VmReclamationState, is_intentional_stop,
    is_platform_reclamation, is_workload_failure, plan_reclamation,
};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::traits::vm_host_state::ScopeFacts;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// S-VM-31 — plan_reclamation fixtures
// ---------------------------------------------------------------------------

/// The six rows of `brief.md` §105a.4's decision table.
#[derive(Debug, Clone, Copy)]
enum Row {
    NonTerminalAuthorised,
    NonTerminalHeld,
    Terminal,
    UnknownVmExclusiveAuthorised,
    UnknownVmExclusiveHeld,
    CgroupScopeOnly,
}

fn arb_row() -> impl Strategy<Value = Row> {
    prop_oneof![
        Just(Row::NonTerminalAuthorised),
        Just(Row::NonTerminalHeld),
        Just(Row::Terminal),
        Just(Row::UnknownVmExclusiveAuthorised),
        Just(Row::UnknownVmExclusiveHeld),
        Just(Row::CgroupScopeOnly),
    ]
}

/// Allocation ids drawn from `[offset, offset + 100_000)` — callers pass
/// disjoint offsets so two generated ids can never collide, deterministically
/// (not merely with high probability).
fn arb_alloc_id(offset: u32) -> impl Strategy<Value = AllocationId> {
    (0u32..100_000).prop_map(move |n| {
        AllocationId::new(&format!("vm-plan-{}", n + offset)).expect("valid AllocationId")
    })
}

fn arb_workload_id() -> impl Strategy<Value = WorkloadId> {
    (0u32..100_000).prop_map(|n| WorkloadId::new(&format!("wl-{n}")).expect("valid WorkloadId"))
}

/// Build the `(desired, actual, expected)` triple for one decision-table
/// row, keyed on `alloc` — the case under test — layered with a SECOND,
/// constant "noise" allocation (a supervised, non-terminal VM, row 2's own
/// shape) that must contribute NOTHING to the output regardless of which
/// row `alloc` exercises. Proves the six-row table AND the aggregation
/// over multiple allocations simultaneously.
fn build_case(
    row: Row,
    alloc: AllocationId,
    workload_id: WorkloadId,
    noise: AllocationId,
) -> (VmReclamationState, VmReclamationState, Vec<Action>) {
    let mut desired = VmReclamationState::default();
    let mut actual = VmReclamationState::default();

    desired
        .allocations
        .insert(noise.clone(), VmAllocFacts { workload_id: workload_id.clone(), terminal: false });
    actual.host.scopes.insert(noise.clone(), ScopeFacts::default());
    let mut held = BTreeSet::new();
    held.insert(noise);

    let expected = match row {
        Row::NonTerminalAuthorised => {
            desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: false });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            vec![Action::ReclaimAllocation { alloc_id: alloc }]
        }
        Row::NonTerminalHeld => {
            desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: false });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            held.insert(alloc);
            Vec::new()
        }
        Row::Terminal => {
            desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: true });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            actual.host.run_dirs.insert(alloc.clone());
            // Exempt — even a HELD supervision must not suppress the
            // disposal; the exemption never consults the predicate.
            held.insert(alloc.clone());
            vec![Action::DiscardStrandedArtifacts { alloc_id: alloc }]
        }
        Row::UnknownVmExclusiveAuthorised => {
            actual.host.run_dirs.insert(alloc.clone());
            vec![Action::DiscardStrandedArtifacts { alloc_id: alloc }]
        }
        Row::UnknownVmExclusiveHeld => {
            actual.host.clones.insert(alloc.clone(), PathBuf::from("/staging/noise.img"));
            held.insert(alloc);
            Vec::new()
        }
        Row::CgroupScopeOnly => {
            actual.host.scopes.insert(alloc, ScopeFacts::default());
            Vec::new()
        }
    };

    actual.supervision = SupervisionSet::Observed(held);
    (desired, actual, expected)
}

// ---------------------------------------------------------------------------
// S-VM-32 — Ending Class fixtures
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn terminal_row(
    state: AllocState,
    terminal: Option<TerminalCondition>,
    reason: Option<TransitionReason>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new("vm-ending-class-0").expect("valid AllocationId"),
        workload_id: WorkloadId::new("wl-ending-class").expect("valid WorkloadId"),
        node_id: NodeId::new("local").expect("valid NodeId"),
        state,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("local").expect("valid NodeId"),
        },
        reason,
        detail: None,
        terminal,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn arb_stopped_by() -> impl Strategy<Value = StoppedBy> {
    prop_oneof![
        Just(StoppedBy::Operator),
        Just(StoppedBy::Reconciler),
        Just(StoppedBy::Process),
        Just(StoppedBy::SystemGc),
    ]
}

fn arb_terminal_condition() -> impl Strategy<Value = Option<TerminalCondition>> {
    prop_oneof![
        Just(None),
        arb_stopped_by().prop_map(|by| Some(TerminalCondition::Stopped { by })),
        any::<i32>().prop_map(|c| Some(TerminalCondition::Completed { exit_code: c })),
        prop::option::of(any::<i32>())
            .prop_map(|c| Some(TerminalCondition::Failed { exit_code: c })),
        (1u32..10).prop_map(|a| Some(TerminalCondition::BackoffExhausted { attempts: a })),
    ]
}

fn arb_transition_reason() -> impl Strategy<Value = Option<TransitionReason>> {
    prop_oneof![
        Just(None),
        arb_stopped_by().prop_map(|by| Some(TransitionReason::Stopped { by })),
        any::<String>().prop_map(|d| Some(TransitionReason::DriverInternalError { detail: d })),
    ]
}

fn arb_terminal_alloc_state() -> impl Strategy<Value = AllocState> {
    prop_oneof![Just(AllocState::Terminated), Just(AllocState::Failed)]
}

proptest! {
    /// S-VM-31 — `plan_reclamation(desired, actual) -> Vec<Action>` is
    /// pure and matches the design's six-row decision table exactly for
    /// every generated `(VmReclamationState, VmReclamationState)` pair.
    #[test]
    fn plan_reclamation_is_pure_and_matches_the_decision_table(
        row in arb_row(),
        alloc in arb_alloc_id(0),
        noise in arb_alloc_id(1_000_000),
        workload_id in arb_workload_id(),
    ) {
        let (desired, actual, expected) = build_case(row, alloc, workload_id, noise);

        let actions_first = plan_reclamation(&desired, &actual);
        prop_assert_eq!(
            &actions_first, &expected,
            "row {:?} did not match the six-row decision table", row
        );

        // PURITY / no I/O: a second call over the SAME (unmutated,
        // `&`-borrowed) inputs is byte-identical — "the observe pass
        // wrote something" is structurally unrepresentable (the function
        // takes no port), and determinism is this claim's behavioural
        // witness.
        let actions_second = plan_reclamation(&desired, &actual);
        prop_assert_eq!(actions_first, actions_second, "plan_reclamation must be deterministic");
    }

    /// S-VM-32 — every terminal `AllocStatusRow` the design's
    /// classification can produce belongs to EXACTLY ONE of the three
    /// Ending Classes (Intentional Stop, Workload Failure, Platform
    /// Reclamation).
    #[test]
    fn ending_class_is_total_and_disjoint_over_terminal_rows(
        state in arb_terminal_alloc_state(),
        terminal in arb_terminal_condition(),
        reason in arb_transition_reason(),
    ) {
        let row = terminal_row(state, terminal, reason);
        prop_assert!(row.state.is_terminal(), "fixture precondition: state must be terminal");

        let classes =
            [is_intentional_stop(&row), is_workload_failure(&row), is_platform_reclamation(&row)];
        let true_count = classes.iter().filter(|c| **c).count();
        prop_assert_eq!(
            true_count, 1,
            "terminal row must belong to EXACTLY ONE Ending Class, got {} for {:?}", true_count, row
        );
    }

    /// S-VM-92 — `SupervisionSet::reclamation_authorised` is the ONE
    /// kill-authorising predicate; `Unavailable` always returns `false`,
    /// never "unsupervised".
    #[test]
    fn supervision_set_unavailable_never_authorises_reclamation(
        alloc in arb_alloc_id(0),
        other_held in prop::collection::btree_set(arb_alloc_id(2_000_000), 0..5),
        alloc_is_member in any::<bool>(),
    ) {
        prop_assert!(
            !SupervisionSet::Unavailable.reclamation_authorised(&alloc),
            "Unavailable must NEVER authorise reclamation"
        );

        let mut held_ids = other_held;
        if alloc_is_member {
            held_ids.insert(alloc.clone());
        }
        let observed = SupervisionSet::Observed(held_ids.clone());
        prop_assert_eq!(
            observed.reclamation_authorised(&alloc),
            !held_ids.contains(&alloc),
            "Observed(held) must authorise iff alloc is NOT a member of held"
        );
    }
}
