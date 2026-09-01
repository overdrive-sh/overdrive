//! Acceptance pin for the registered VM provision-failure cleanup invariant.
//!
//! The evaluator itself drives the production action shim and carries the
//! negative-control teeth check. This test pins the public `cargo dst --only`
//! catalogue surface so the property cannot silently become an unregistered
//! helper.

#![allow(clippy::doc_markdown)]

use overdrive_sim::{Harness, Invariant, InvariantStatus};

/// The fixed seed reproduces through
/// `cargo dst --seed 424242 --only vm-provision-failure-cleans-network-and-reuses-slot`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn registered_invariant_converges_for_the_fixed_seed() {
    let report = Harness::new()
        .only(Invariant::VmProvisionFailureCleansNetworkAndReusesSlot)
        .run(424_242)
        .expect("the single-node LocalIntentStore + SimObservationStore harness composes");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "vm-provision-failure-cleans-network-and-reuses-slot");
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "fixed-seed failure: {:?}; reproduce with `cargo dst --seed 424242 --only vm-provision-failure-cleans-network-and-reuses-slot`",
        report.invariants[0].cause
    );
    assert!(report.is_green());
}
