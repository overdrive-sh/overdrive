//! Proptest — mandatory newtype round-trip call site.
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites*:
//!
//! > **Newtype roundtrip.** Every newtype's `Display` / `FromStr` / serde
//! > must round-trip bit-equivalent for every valid input, and every
//! > invalid input must be rejected by `FromStr` with a structured
//! > `ParseError`.
//!
//! This file covers `JobId`, `NodeId`, and `AllocationId` — the three
//! identifiers under step 01-01. The extended identifier set (US-02)
//! lands in a later step with its own proptest module.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{AllocationId, IdParseError, JobId, NodeId};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators — only valid inputs.
//
// The label newtypes accept DNS-1123-label-like strings:
//   * lowercase ASCII letters, digits, `-`, `_`, `.`
//   * first and last char must be alphanumeric
//   * non-empty, ≤ 253 chars
//
// We build the generator as (first_alnum, middle_body, last_alnum) so
// every drawn string satisfies the start-and-end-alnum rule by
// construction. The middle body draws from the full allowed class.
//
// `FromStr` is case-insensitive — uppercase inputs are lowercased in
// canonical form. We therefore draw mixed-case inputs to exercise that
// leg of the contract: the round-trip must still be lossless because
// `Display` emits the canonical (lowercase) form.
// -----------------------------------------------------------------------------

const ALNUM_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const INTERIOR_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.";

/// A single alphanumeric character (upper- or lowercase letter, or digit).
fn alnum_char() -> impl Strategy<Value = char> {
    proptest::sample::select(ALNUM_CHARS.chars().collect::<Vec<_>>())
}

/// One character from the allowed interior class: alphanumeric (any case),
/// `-`, `_`, `.`.
fn interior_char() -> impl Strategy<Value = char> {
    proptest::sample::select(INTERIOR_CHARS.chars().collect::<Vec<_>>())
}

/// A valid label-newtype input with length in `1..=32`.
///
/// Keeping the upper bound small (vs the 253-char ceiling) keeps
/// shrinking fast; the length ceiling is covered by a dedicated
/// boundary test in `id.rs`.
///
/// The validator requires both the first AND last character to be
/// alphanumeric. We encode this in the generator by producing:
///
///   * the 1-char case: a single alphanumeric
///   * the ≥ 2-char case: alnum + interior-body + alnum
///
/// so no drawn string can end with `-`, `_`, or `.`.
fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        // Single-character case.
        alnum_char().prop_map(|c| c.to_string()),
        // Two-or-more case: alnum, optional interior body, terminal alnum.
        (alnum_char(), prop::collection::vec(interior_char(), 0..=30), alnum_char(),).prop_map(
            |(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }
        ),
    ]
}

// -----------------------------------------------------------------------------
// Round-trip properties — Display / FromStr / serde must compose to
// identity for every valid input.
//
// The default CI budget is PROPTEST_CASES=1024 per `.claude/rules/testing.md`;
// each proptest below implicitly inherits that via the env.
// -----------------------------------------------------------------------------

proptest! {
    /// JobId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 1 under the property budget.
    #[test]
    fn job_id_display_from_str_round_trip(raw in valid_label()) {
        let original = JobId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = JobId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// NodeId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 2 under the property budget.
    #[test]
    fn node_id_display_from_str_round_trip(raw in valid_label()) {
        let original = NodeId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = NodeId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// AllocationId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 3 under the property budget.
    #[test]
    fn allocation_id_display_from_str_round_trip(raw in valid_label()) {
        let original = AllocationId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = AllocationId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// serde JSON output equals the Display form surrounded by quotes
    /// for every valid JobId.
    ///
    /// Covers §2.1 scenario 4 (the JSON-byte-equivalence leg).
    #[test]
    fn job_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = JobId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: JobId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }

    #[test]
    fn node_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = NodeId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: NodeId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }

    #[test]
    fn allocation_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = AllocationId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: AllocationId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }
}

// -----------------------------------------------------------------------------
// Invalid-input rejection — the other half of the mandatory call site.
//
// Every invalid input must be rejected by `FromStr` with a structured
// `ParseError`. We cover the three canonical rejection shapes:
//   * empty string          -> Empty
//   * forbidden character   -> InvalidChar
//   * leading non-alnum     -> InvalidFormat
//
// Length-ceiling rejection is covered by a dedicated `#[test]` in
// `id.rs`; the generator here stays under the ceiling so the property
// stays focused.
// -----------------------------------------------------------------------------

/// A character NOT in the label allowed class.
fn forbidden_char() -> impl Strategy<Value = char> {
    // Pick from a hand-curated set of clearly-invalid printable
    // characters. Keeps shrinking tight and avoids the generator-vs-
    // validator tautology trap.
    prop_oneof![
        Just(' '),
        Just('!'),
        Just('@'),
        Just('#'),
        Just('/'),
        Just('\\'),
        Just('*'),
        Just(':'),
    ]
}

proptest! {
    /// An input containing a forbidden character must be rejected with
    /// a structured `InvalidChar` variant.
    #[test]
    fn job_id_rejects_forbidden_character(
        prefix in valid_label(),
        bad in forbidden_char(),
    ) {
        let mut raw = prefix;
        raw.push(bad);
        raw.push('x'); // keep last char valid so only the forbidden one trips
        let err = JobId::from_str(&raw).expect_err("forbidden char must reject");
        let is_invalid_char = matches!(err, IdParseError::InvalidChar { .. });
        prop_assert!(is_invalid_char, "expected InvalidChar variant");
    }
}

#[test]
fn job_id_rejects_empty_input_with_structured_error() {
    // Empty is a single case, not a property.
    let err = JobId::from_str("").expect_err("empty must reject");
    assert!(matches!(err, IdParseError::Empty { .. }));
}

#[test]
fn node_id_rejects_leading_hyphen_with_structured_error() {
    // Leading-non-alnum is a structural boundary case, not a property.
    let err = NodeId::from_str("-leading-hyphen").expect_err("leading hyphen must reject");
    assert!(matches!(err, IdParseError::InvalidFormat { .. }));
}
