//! Acceptance pin for ADR-0089 §7's same-ID restart invariant.

#![allow(clippy::doc_markdown)]

use overdrive_sim::{Harness, Invariant, InvariantStatus};

/// CONTRACT_SHAPE: bounded-change.
///
/// Reproduce with `cargo dst --seed 424242 --only
/// same-id-restart-removes-prior-protection-before-replacement-provision`.
#[test]
fn same_id_restart_removes_prior_protection_before_replacement_provision() {
    let report = Harness::new()
        .only(Invariant::SameIdRestartRemovesPriorProtectionBeforeReplacementProvision)
        .run(424_242)
        .expect("the single-node action-shim fixture composes");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(
        report.invariants[0].name,
        "same-id-restart-removes-prior-protection-before-replacement-provision"
    );
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "{:?}",
        report.invariants[0].cause
    );
    assert!(report.is_green());
}
