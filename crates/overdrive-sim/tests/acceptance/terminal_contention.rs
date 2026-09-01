//! Acceptance pin for the registered terminal-contention invariant.

#![allow(clippy::doc_markdown)]

use overdrive_sim::Harness;
use overdrive_sim::harness::InvariantStatus;
use overdrive_sim::invariants::Invariant;

const SEED: u64 = 424_242;
const NAME: &str = "terminal-contention-converges";

/// CONTRACT_SHAPE: bounded-change.
///
/// The evaluator owns the production Stop/exit-observer race, evidence
/// checker, and deletion-sensitive negative control. This acceptance test
/// pins only catalogue registration, fixed-seed execution, and the exact
/// reproduction name; the focused action-shim table separately covers the
/// zero/one/two-proposal and typed-error complements.
#[test]
fn registered_terminal_contention_invariant_passes_the_fixed_seed() {
    let report = Harness::new()
        .only(Invariant::TerminalContentionConverges)
        .run(SEED)
        .expect("fixed-seed terminal-contention harness composes");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, NAME);
    assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
    assert!(
        report.failures.is_empty(),
        "fixed-seed failure: {:?}; reproduce with `cargo dst --seed {SEED} --only {NAME}`",
        report.failures
    );
}
