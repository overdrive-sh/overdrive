//! Slice 03 / US-WP-5 AC4 — replay-equivalence holds across a signal wait
//! and an emit, seeded and reproducible; slice-03 AC4.
//!
//! Scenario S-WP-03-05. K4 (O5). The `replay-equivalence-provision-record`
//! invariant (the SAME named variant — extended, not a new family, per the
//! slice brief) now also drives a `ctx.wait_for_signal → ctx.emit_action →
//! terminal` shape across a crash-resume: replaying the journal across the
//! signal wait + emit produces a bit-identical trajectory, green on the CI
//! critical path, reproducing bit-for-bit from the printed seed.
//! ADR-0064 §6.
//!
//! # Port-to-port
//!
//! The driving port is the DST harness (`Harness::only(...).run(seed)`) —
//! the same surface `cargo dst --only replay-equivalence-provision-record`
//! drives. The observable outcome is asserted at the `RunReport` boundary:
//! the named invariant is green, and the verdict is bit-stable across a
//! second run at the same seed.
//!
//! # Falsifiability
//!
//! The invariant's signal+emit extension drives a crash WHILE blocked on
//! the signal and a crash AFTER the emit; if the re-block-on-resume or the
//! idempotent-emit replay were broken, the extended invariant would FAIL
//! and this test would red.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;

#[test]
fn replay_equivalence_holds_across_a_signal_wait_and_an_emit_seeded_and_reproducible() {
    const SEED: u64 = 0x5eed_0302;

    // Drive the named invariant through the harness — the SAME surface
    // `cargo dst --only replay-equivalence-provision-record` drives. The
    // invariant now also exercises the slice-03 signal-wait + emit
    // crash-resume shape (extended in-place, not a new family).
    let report_a = Harness::new()
        .only(Invariant::ReplayEquivalenceProvisionRecord)
        .run(SEED)
        .expect("harness composes");
    let result_a = report_a
        .invariants
        .iter()
        .find(|r| r.name == "replay-equivalence-provision-record")
        .expect("the named invariant ran (it is on the critical path)");
    assert_eq!(
        result_a.status,
        InvariantStatus::Pass,
        "K4 — replay-equivalence across signal+emit is GREEN: {:?}",
        result_a.cause
    );

    // Seed-reproducible (`trust-the-sim`): a second run at the SAME seed
    // produces a bit-identical verdict for this invariant.
    let report_b = Harness::new()
        .only(Invariant::ReplayEquivalenceProvisionRecord)
        .run(SEED)
        .expect("harness composes (second run)");
    let result_b = report_b
        .invariants
        .iter()
        .find(|r| r.name == "replay-equivalence-provision-record")
        .expect("the named invariant ran on the second pass");
    assert_eq!(
        result_a, result_b,
        "the same seed reproduces the invariant verdict bit-for-bit (seed {SEED:#x})"
    );
}
