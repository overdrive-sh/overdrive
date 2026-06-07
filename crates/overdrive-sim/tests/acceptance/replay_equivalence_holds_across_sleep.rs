//! Slice 02 / AC3 — replay-equivalence holds across the sleep, seeded and
//! reproducible.
//!
//! Scenario S-WP-02-04. K4 (O5). The `replay_equivalence_*` invariant
//! extended over the 3-await `ctx.run → ctx.sleep → ctx.run` shape:
//! replaying the journal across the sleep produces a bit-identical
//! trajectory, green on the CI critical path, reproducing bit-for-bit
//! from the printed seed. ADR-0064 §6.
//!
//! # Port-to-port
//!
//! The driving port is the DST harness (`Harness::only(...).run(seed)`) —
//! the same surface `cargo dst --only replay-equivalence-provision-record`
//! drives. The observable outcome is asserted at the `RunReport` boundary:
//! the named invariant — now driving the `ctx.run → ctx.sleep → ctx.run`
//! shape in addition to the slice-01 `ctx.run → terminal` shape — is
//! present, green, and the verdict is bit-stable across a second run at the
//! same seed. No new invariant family is minted (the verbatim 02-02
//! constraint): the EXISTING `ReplayEquivalenceProvisionRecord` evaluator
//! is extended to exercise the sleep shape.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;

#[test]
fn replay_equivalence_holds_across_a_durable_sleep_seeded_and_reproducible() {
    const SEED: u64 = 0x5eed_510e;

    // Drive the named invariant — the SAME `ReplayEquivalenceProvisionRecord`
    // catalogue variant the slice-01 critical path drives, now extended to
    // exercise the `ctx.run → ctx.sleep → ctx.run` shape across a crash that
    // spans the sleep window (the verbatim 02-02 constraint: extend the
    // existing invariant, do not add a new family).
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
        "K4 — replay-equivalence holds across the durable sleep: {:?}",
        result_a.cause
    );

    // Seed-reproducible (the `trust-the-sim` discipline): a second run at
    // the SAME seed produces a bit-identical verdict for this invariant —
    // the sleep shape drives logical time through `SimClock`, so the
    // trajectory is reproducible bit-for-bit.
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
        "the same seed reproduces the invariant verdict bit-for-bit across the sleep (seed {SEED:#x})"
    );
}
