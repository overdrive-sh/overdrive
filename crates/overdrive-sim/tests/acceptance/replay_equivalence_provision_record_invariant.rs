//! Slice 01 / US-WP-4 AC1/AC2/AC3 — replay-equivalence is a named DST
//! invariant on the CI critical path, green from a seed; slice-01 AC3.
//!
//! Scenario S-WP-01-09. **K4 — the load-bearing KPI (O5).** The
//! `replay_equivalence_provision_record` `SimInvariant` (a named enum
//! variant graduating `ReplayEquivalentEmptyWorkflow`,
//! `invariants/mod.rs:136`, no inline string literal — house convention)
//! drives an uninterrupted run, a crash-injected run, and a resumed run,
//! asserting the resumed trajectory is byte-identical to the
//! uninterrupted one (`assert_replay_equivalent!`) AND
//! `assert_eventually!(is_terminal)` within the declared step budget
//! (bounded progress). ADR-0066 §6, ADR-0064 §3/§6.
//!
//! Cross-scenario consistency (journey steps 3↔4): the run prints a seed
//! and reproduces bit-for-bit on a second run — the `trust-the-sim`
//! discipline.
//!
//! # Port-to-port
//!
//! The driving port is the DST harness (`Harness::only(...).run(seed)`) —
//! the same surface `cargo dst --only <NAME>` drives. The observable
//! outcome is asserted at the `RunReport` boundary: the named invariant is
//! present, green, and the verdict is bit-stable across a second run at
//! the same seed.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;

use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;

/// The canonical name is a NAMED enum variant (no inline string literal —
/// US-WP-4 AC1) and round-trips losslessly through `FromStr → Display`.
#[test]
fn replay_equivalence_provision_record_is_a_named_catalogue_variant() {
    let by_name = Invariant::from_str("replay-equivalence-provision-record")
        .expect("the graduated variant resolves by its canonical kebab name");
    assert_eq!(
        by_name,
        Invariant::ReplayEquivalenceProvisionRecord,
        "the canonical name maps to the named variant — no inline literal"
    );
    assert_eq!(
        by_name.to_string(),
        "replay-equivalence-provision-record",
        "Display round-trips the canonical name"
    );
    // The placeholder it graduated is gone from the catalogue.
    assert!(
        Invariant::ALL.contains(&Invariant::ReplayEquivalenceProvisionRecord),
        "the graduated variant is on the default `cargo dst` critical path"
    );
}

#[test]
fn replay_equivalence_provision_record_is_a_named_invariant_green_and_seed_reproducible() {
    const SEED: u64 = 0x5eed_c0de;

    // Drive the named invariant through the harness — the same surface
    // `cargo dst --only replay-equivalence-provision-record` drives.
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
        "K4 — replay-equivalence is GREEN: {:?}",
        result_a.cause
    );

    // Seed-reproducible (the `trust-the-sim` discipline): a second run at
    // the SAME seed produces a bit-identical verdict for this invariant.
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

/// Step 01-06 / D6 (ADR-0064 §6) — the EXTENDED `replay_equivalence_provision
/// _record` invariant pins (b) `Started` at command-index 0 with a FULL
/// command-sequence equality between the resumed and uninterrupted runs, AND
/// (c) the `SignalSeen` notification is resolved by `SignalKey` OFF the
/// positional command walk (never consumed as a command). This is the
/// structural regression guard that would have caught the trap: a dropped
/// `Started` write, or a `SignalSeen` entering the command walk, fails the
/// invariant.
///
/// # Port-to-port
///
/// The driving port is the DST harness (`Harness::only(...).run(seed)`) — the
/// same surface `cargo dst --only replay-equivalence-provision-record`
/// drives. The observable outcome at the `RunReport` boundary is that the
/// extended invariant (now carrying the (b)+(c) guard) is GREEN and
/// seed-reproducible. A regression that dropped `Started` at command-index 0
/// or walked the `SignalSeen` as a command flips this invariant to `Fail` at
/// the same boundary.
#[test]
fn replay_equivalence_started_at_index_0_and_notification_not_consumed_as_command() {
    const SEED: u64 = 0x5eed_0d06;

    // Drive the EXTENDED invariant through the harness driving port.
    let report = Harness::new()
        .only(Invariant::ReplayEquivalenceProvisionRecord)
        .run(SEED)
        .expect("harness composes");
    let result = report
        .invariants
        .iter()
        .find(|r| r.name == "replay-equivalence-provision-record")
        .expect("the extended invariant ran (it is on the critical path)");

    // The (b) Started-at-0 full-command-sequence equality AND the (c)
    // notification-not-as-command cursor-advance guard both hold — the
    // invariant is GREEN. (A dropped `Started` write fails (b); a `SignalSeen`
    // walked as a command fails (c) — either flips this to `Fail` with a cause
    // naming the trap.)
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "D6 — Started-at-0 + notification-not-as-command guard is GREEN: {:?}",
        result.cause
    );

    // Seed-reproducible: a second run at the SAME seed reproduces the verdict
    // bit-for-bit — the `trust-the-sim` discipline (`cargo dst --seed`).
    let report_again = Harness::new()
        .only(Invariant::ReplayEquivalenceProvisionRecord)
        .run(SEED)
        .expect("harness composes (second run)");
    let result_again = report_again
        .invariants
        .iter()
        .find(|r| r.name == "replay-equivalence-provision-record")
        .expect("the extended invariant ran on the second pass");
    assert_eq!(
        result, result_again,
        "the same seed reproduces the extended-guard verdict bit-for-bit (seed {SEED:#x})"
    );
}
