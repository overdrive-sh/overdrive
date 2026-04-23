#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Property tests for the [`Invariant`] enum (`crates/overdrive-sim/src/invariants.rs`).
//!
//! Covers §7.1 scenario 3 from `docs/feature/phase-1-foundation/distill/test-scenarios.md`:
//!
//! > Every invariant name printed by the harness round-trips through the
//! > invariant enum FromStr. Display → FromStr is lossless; FromStr is
//! > case-insensitive.
//!
//! The enum is also the single source of truth for the `--only <NAME>`
//! filter on `cargo xtask dst`. An invariant name accepted by the
//! harness that the enum refused to parse would decouple the CLI from
//! its own summary format — this test closes that loop.

use std::str::FromStr;

use proptest::prelude::*;

use overdrive_sim::invariants::Invariant;

/// Every variant the enum exposes. Keep this list synchronised with
/// the enum itself — adding a variant without adding an entry here
/// means the round-trip property silently stops covering it.
const ALL_VARIANTS: &[Invariant] = &[
    Invariant::SingleLeader,
    Invariant::IntentNeverCrossesIntoObservation,
    Invariant::SnapshotRoundtripBitIdentical,
    Invariant::SimObservationLwwConverges,
    Invariant::ReplayEquivalentEmptyWorkflow,
    Invariant::EntropyDeterminismUnderReseed,
];

fn variant_strategy() -> impl Strategy<Value = Invariant> {
    // `prop_oneof!` with `Just(...)` is the idiomatic way to generate
    // one of an enumerable set.
    prop_oneof![
        Just(Invariant::SingleLeader),
        Just(Invariant::IntentNeverCrossesIntoObservation),
        Just(Invariant::SnapshotRoundtripBitIdentical),
        Just(Invariant::SimObservationLwwConverges),
        Just(Invariant::ReplayEquivalentEmptyWorkflow),
        Just(Invariant::EntropyDeterminismUnderReseed),
    ]
}

// -----------------------------------------------------------------------------
// Canonical form assertions — pin the kebab-case spelling for every variant.
// A mutation that silently relabels a variant (e.g. SingleLeader →
// "single_leader") is caught here.
// -----------------------------------------------------------------------------

#[test]
fn display_is_kebab_case_lowercase() {
    assert_eq!(Invariant::SingleLeader.to_string(), "single-leader");
    assert_eq!(
        Invariant::IntentNeverCrossesIntoObservation.to_string(),
        "intent-never-crosses-into-observation"
    );
    assert_eq!(
        Invariant::SnapshotRoundtripBitIdentical.to_string(),
        "snapshot-roundtrip-bit-identical"
    );
    assert_eq!(Invariant::SimObservationLwwConverges.to_string(), "sim-observation-lww-converges");
    assert_eq!(
        Invariant::ReplayEquivalentEmptyWorkflow.to_string(),
        "replay-equivalent-empty-workflow"
    );
    assert_eq!(
        Invariant::EntropyDeterminismUnderReseed.to_string(),
        "entropy-determinism-under-reseed"
    );
}

#[test]
fn from_str_accepts_canonical_forms() {
    for v in ALL_VARIANTS {
        let canonical = v.to_string();
        let parsed: Invariant = canonical.parse().expect("canonical form must parse");
        assert_eq!(parsed, *v, "{canonical} round-trips to {v:?}");
    }
}

#[test]
fn from_str_is_case_insensitive() {
    assert_eq!(Invariant::from_str("SINGLE-LEADER").unwrap(), Invariant::SingleLeader);
    assert_eq!(Invariant::from_str("Single-Leader").unwrap(), Invariant::SingleLeader);
    assert_eq!(
        Invariant::from_str("INTENT-NEVER-CROSSES-INTO-OBSERVATION").unwrap(),
        Invariant::IntentNeverCrossesIntoObservation
    );
}

#[test]
fn from_str_rejects_unknown_names_with_the_raw_input_in_the_error() {
    let err = Invariant::from_str("not-a-real-invariant").expect_err("unknown name must error");
    assert_eq!(err.raw, "not-a-real-invariant");
}

#[test]
fn from_str_rejects_empty_string() {
    let err = Invariant::from_str("").expect_err("empty name must error");
    assert_eq!(err.raw, "");
}

// -----------------------------------------------------------------------------
// Property: lossless round-trip for every variant.
// -----------------------------------------------------------------------------

proptest! {
    #[test]
    fn display_fromstr_is_lossless(v in variant_strategy()) {
        let rendered = v.to_string();
        let parsed: Invariant = rendered.parse().expect("Display output must parse");
        prop_assert_eq!(parsed, v);
    }

    /// FromStr ignores ASCII case — any case-mangled spelling of a
    /// canonical name resolves to the same variant.
    #[test]
    fn from_str_ignores_ascii_case(
        v in variant_strategy(),
        mask in proptest::collection::vec(any::<bool>(), 0..64),
    ) {
        let canonical = v.to_string();
        let mangled: String = canonical
            .chars()
            .enumerate()
            .map(|(i, c)| {
                // Flip the case of the i-th alphabetic character when
                // mask[i] is true. Non-alpha chars (the hyphens) are
                // untouched so the shape of the input remains valid.
                let flip = mask.get(i).copied().unwrap_or(false);
                if flip && c.is_ascii_alphabetic() {
                    if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }
                } else {
                    c
                }
            })
            .collect();
        let parsed: Invariant = mangled.parse().expect("case-mangled form must parse");
        prop_assert_eq!(parsed, v);
    }
}
