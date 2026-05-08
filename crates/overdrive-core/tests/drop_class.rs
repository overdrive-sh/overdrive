//! `DropClass` newtype completeness — Slice 06 / S-2.2-23.
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites*:
//!
//! > **Newtype roundtrip.** Every newtype's `Display` / `FromStr` / serde
//! > must round-trip bit-equivalent for every valid input, and every
//! > invalid input must be rejected by `FromStr` with a structured
//! > `ParseError`.
//!
//! Plus the slot-mapping completeness assertion that S-2.2-23 names —
//! every variant maps to a unique kernel-side slot in `0..VARIANT_COUNT`
//! and `VARIANT_COUNT == 6` (Q7=B, ADR-0040 D8). Adding a variant
//! without bumping `VARIANT_COUNT` is caught at compile time by the
//! const-assert in `dataplane/drop_class.rs`; the runtime mapping
//! completeness is the harness here.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::collections::BTreeSet;
use std::str::FromStr;

use overdrive_core::dataplane::DropClass;
use proptest::prelude::*;

/// Every variant the kernel-side `DROP_COUNTER` `PERCPU_ARRAY` indexes.
/// Lockstep with the `#[repr(u32)]` discriminants in
/// `crates/overdrive-core/src/dataplane/drop_class.rs`. Iteration order
/// is deterministic — the test below asserts on it.
const ALL_VARIANTS: [DropClass; 6] = [
    DropClass::MalformedHeader,
    DropClass::UnknownVip,
    DropClass::NoHealthyBackend,
    DropClass::SanityPrologue,
    DropClass::ReverseNatMiss,
    DropClass::OversizePacket,
];

/// S-2.2-23 — `drop_class_slot_mapping_completeness`.
///
/// Every variant maps to a unique slot index in `0..VARIANT_COUNT`,
/// and the variant set covers `0..VARIANT_COUNT` exhaustively. Slot 0
/// is `MalformedHeader` (the most common drop class — sanity-prologue
/// header rejects), slot 5 is `OversizePacket` (the highest variant).
/// `VARIANT_COUNT == 6` is the kernel-side `DROP_COUNTER` map size.
#[test]
fn drop_class_slot_mapping_completeness() {
    assert_eq!(DropClass::VARIANT_COUNT, 6, "Q7=B locks 6 slots; bump ADR before changing");
    assert_eq!(ALL_VARIANTS.len(), DropClass::VARIANT_COUNT as usize);

    // Every variant maps to a unique slot.
    let slots: BTreeSet<u32> = ALL_VARIANTS.iter().map(|v| v.as_index()).collect();
    assert_eq!(slots.len(), ALL_VARIANTS.len(), "slot indices must be unique");

    // Slot range is exactly 0..VARIANT_COUNT — no gaps, no overflow.
    let expected: BTreeSet<u32> = (0..DropClass::VARIANT_COUNT).collect();
    assert_eq!(slots, expected, "slot indices must cover 0..VARIANT_COUNT exhaustively");

    // Discriminants stable per architecture.md § 6 / ADR-0040 D8.
    assert_eq!(DropClass::MalformedHeader.as_index(), 0);
    assert_eq!(DropClass::UnknownVip.as_index(), 1);
    assert_eq!(DropClass::NoHealthyBackend.as_index(), 2);
    assert_eq!(DropClass::SanityPrologue.as_index(), 3);
    assert_eq!(DropClass::ReverseNatMiss.as_index(), 4);
    assert_eq!(DropClass::OversizePacket.as_index(), 5);
}

/// Display emits canonical kebab-case per architecture.md § 6.
#[test]
fn drop_class_display_emits_canonical_kebab_case() {
    assert_eq!(DropClass::MalformedHeader.to_string(), "malformed-header");
    assert_eq!(DropClass::UnknownVip.to_string(), "unknown-vip");
    assert_eq!(DropClass::NoHealthyBackend.to_string(), "no-healthy-backend");
    assert_eq!(DropClass::SanityPrologue.to_string(), "sanity-prologue");
    assert_eq!(DropClass::ReverseNatMiss.to_string(), "reverse-nat-miss");
    assert_eq!(DropClass::OversizePacket.to_string(), "oversize-packet");
}

/// `FromStr` is case-insensitive on the kebab-case canonical token —
/// matches the `Newtype completeness` rule in
/// `.claude/rules/development.md` for human-typed identifiers.
#[test]
fn drop_class_from_str_case_insensitive() {
    for variant in ALL_VARIANTS {
        let canonical = variant.to_string();
        let upper = canonical.to_uppercase();
        let mixed: String = canonical
            .chars()
            .enumerate()
            .map(|(i, c)| if i % 2 == 0 { c.to_ascii_uppercase() } else { c })
            .collect();
        assert_eq!(DropClass::from_str(&canonical).expect("canonical parses"), variant);
        assert_eq!(DropClass::from_str(&upper).expect("uppercase parses"), variant);
        assert_eq!(DropClass::from_str(&mixed).expect("mixed-case parses"), variant);
    }
}

/// Unknown / malformed inputs return a structured `ParseError`, never
/// panic. Per `.claude/rules/development.md` § Newtype completeness.
#[test]
fn drop_class_from_str_rejects_unknown() {
    for invalid in [
        "", // empty
        "no-such-class",
        "malformed_header", // underscore, not kebab
        "MalformedHeader",  // PascalCase, not kebab
        "drop",
        "🔥",
    ] {
        assert!(DropClass::from_str(invalid).is_err(), "expected error for input {invalid:?}");
    }
}

proptest! {
    /// Every variant round-trips losslessly through Display → FromStr.
    #[test]
    fn drop_class_display_from_str_round_trip(idx in 0_usize..ALL_VARIANTS.len()) {
        let original = ALL_VARIANTS[idx];
        let rendered = original.to_string();
        let reparsed = DropClass::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// Every variant round-trips losslessly through serde JSON. Per
    /// `.claude/rules/development.md` § Newtype completeness — serde
    /// must agree with Display/FromStr.
    #[test]
    fn drop_class_serde_round_trip(idx in 0_usize..ALL_VARIANTS.len()) {
        let original = ALL_VARIANTS[idx];
        let json = serde_json::to_string(&original).expect("serialises");
        let expected = format!("\"{original}\"");
        prop_assert_eq!(&json, &expected);
        let back: DropClass = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }

    /// Per-CPU aggregator sums correctly under arbitrary per-CPU
    /// values. Models the userspace `DropCounterHandle::read(class)`
    /// surface: input is `Vec<u64>` (one entry per online CPU);
    /// output is the bit-exact sum. Tests the `aggregate_per_cpu`
    /// helper (which is a thin wrapper around `iter().sum()` so the
    /// proptest catches integer-width and overflow regressions).
    #[test]
    fn per_cpu_aggregator_sums_bit_exact(
        per_cpu_values in proptest::collection::vec(0_u64..1_000_000, 1..=128)
    ) {
        let expected: u64 = per_cpu_values.iter().sum();
        let actual = overdrive_core::dataplane::drop_class::aggregate_per_cpu(&per_cpu_values);
        prop_assert_eq!(actual, expected);
    }
}
