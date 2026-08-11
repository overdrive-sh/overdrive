//! RED scaffolds for `VmReclamation`'s pure diff — `microvm-driver-cloud-hypervisor`
//! (GH #42), cross-cutting (SD-1, `reconcilers.md` Bar 2).
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § "Cross-cutting — `VmReclamation` reconciler" (S-VM-31, S-VM-32,
//! S-VM-92) and ADR-0083 §D7, `brief.md` §§105a.3-105a.4. All three are
//! `@property` scenarios and mandatory mutation targets — the design pins
//! the exact signatures; DELIVER converts each scaffold below into a
//! `proptest!` block (Tier 1, default lane, PBT-full per Mandate 9).
//!
//! Per `.claude/rules/testing.md` § "RED scaffolds": placeholder bodies
//! only. Per CLAUDE.md § "Implement to the design": `plan_reclamation`
//! takes NO port parameter (pure, no I/O) and `SupervisionSet` has
//! EXACTLY two inhabitants (`Unavailable` as `Default`, `Observed(set)`)
//! — a crafter must not add a third variant, a disposition parameter, or
//! a regime field to either.

#![allow(clippy::missing_panics_doc)]

/// S-VM-31 — `plan_reclamation(desired, actual) -> Vec<Action>` is pure
/// and matches the design's six-row decision table exactly for every
/// generated `(VmReclamationState, VmReclamationState)` pair.
#[test]
#[should_panic(expected = "RED scaffold")]
fn plan_reclamation_is_pure_and_matches_the_decision_table() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-31 / plan_reclamation \
         takes NO port parameter -- \"the observe pass wrote something\" is \
         structurally unrepresentable -- and its Vec<Action> output matches \
         brief.md §105a.4's six-row table for every generated state pair -- \
         mandatory mutation target)"
    );
}

/// S-VM-32 — every terminal `AllocStatusRow` the design's classification
/// can produce belongs to EXACTLY ONE of the three Ending Classes
/// (Intentional Stop, Workload Failure, Platform Reclamation).
#[test]
#[should_panic(expected = "RED scaffold")]
fn ending_class_is_total_and_disjoint_over_terminal_rows() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-32 / totality + \
         disjointness proptest -- ADR-0081 D1, named P1 in brief.md §105a.10 \
         -- never zero classes, never two, for any terminal row shape)"
    );
}

/// S-VM-92 — `SupervisionSet::reclamation_authorised` is the ONE
/// kill-authorising predicate; `Unavailable` always returns `false`,
/// never "unsupervised".
#[test]
#[should_panic(expected = "RED scaffold")]
fn supervision_set_unavailable_never_authorises_reclamation() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-92 / for any \
         SupervisionSet and AllocationId: Unavailable -> false always; \
         Observed(held) -> true iff alloc NOT IN held -- brief.md §105a.3, \
         mandatory mutation target)"
    );
}
