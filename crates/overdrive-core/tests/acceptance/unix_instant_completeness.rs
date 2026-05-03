//! Acceptance scenarios for issue #141 step 01-02 — `UnixInstant`
//! newtype completeness contract (`Display` / `FromStr` / Serde) plus
//! the mandatory proptest roundtrips required by
//! `.claude/rules/testing.md` § "Property-based testing (proptest) —
//! Mandatory call sites".
//!
//! Port-to-port at domain scope: the newtype's public signature IS its
//! driving port. The `Display` / `FromStr` / `Serialize` /
//! `Deserialize` impls are the canonical-form surface persisted across
//! every `IntentStore` boundary the type crosses (libSQL hydrate
//! paths, Raft log entries, cross-process audit rows).
//!
//! The canonical Display form is `<seconds>.<nanos>` with **exactly 9
//! nanos digits, zero-padded** — `1700000000.000000123`,
//! `1700000000.000000000`, `0.000000001`. `FromStr` accepts any
//! decimal form (`1700000000`, `1700000000.0`,
//! `1700000000.000000123`) and normalises to the 9-digit canonical
//! form on parse, so `UnixInstant::from_str(&u.to_string()) == Ok(u)`
//! holds for every valid `u`.
//!
//! Per the mutation-testing § "Mandatory targets" rule, every
//! `ParseError` accept/reject branch must have at least one test that
//! flips on the mutation. Each variant is covered explicitly below.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::wall_clock::ParseError;
use proptest::prelude::*;
use rkyv::rancor;

// -----------------------------------------------------------------------------
// Display canonical form — exact-string assertions on the two
// load-bearing fixtures called out in the AC.
// -----------------------------------------------------------------------------

#[test]
fn display_emits_nine_digit_zero_padded_nanos_for_subsecond_value() {
    // Given a `UnixInstant` 1_700_000_000 s + 123 ns past the epoch.
    let u = UnixInstant::from_unix_duration(Duration::new(1_700_000_000, 123));

    // When rendered via Display.
    let rendered = u.to_string();

    // Then the canonical form is `<secs>.<9-digit-nanos>` with leading
    // zeros — every nanos digit count below 9 is zero-padded.
    assert_eq!(rendered, "1700000000.000000123");
}

#[test]
fn display_emits_nine_zero_digits_when_nanos_are_zero() {
    // Given an integer-second `UnixInstant`.
    let u = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));

    // Then the canonical form still carries the 9 zero-pad nanos
    // digits — Display NEVER drops the fractional part. This is what
    // makes the round-trip property total.
    assert_eq!(u.to_string(), "1700000000.000000000");
}

#[test]
fn display_emits_zero_seconds_canonical_form() {
    // Boundary case — the epoch instant. Without this the mutation
    // `secs == 0 ? "" : format!(...)` would not be killed.
    let u = UnixInstant::from_unix_duration(Duration::ZERO);
    assert_eq!(u.to_string(), "0.000000000");
}

// -----------------------------------------------------------------------------
// FromStr equivalence — three lexically-distinct decimal forms ALL
// parse to the same `UnixInstant`. This pins the normalisation
// contract: `FromStr` is total over the decimal representations a
// human or a peer system might produce.
// -----------------------------------------------------------------------------

#[test]
fn from_str_normalises_all_decimal_forms_to_the_same_value() {
    let bare: UnixInstant = "1700000000".parse().expect("bare integer parses");
    let short_frac: UnixInstant = "1700000000.0".parse().expect("short fractional parses");
    let full_frac: UnixInstant = "1700000000.000000000".parse().expect("full 9-digit parses");

    assert_eq!(bare, short_frac);
    assert_eq!(short_frac, full_frac);

    // And the canonical render of any of them is the 9-digit form.
    assert_eq!(bare.to_string(), "1700000000.000000000");
}

#[test]
fn from_str_pads_short_fractional_to_full_nanos() {
    // `.5` is 500_000_000 ns. Pinning this catches a mutation that
    // appends zeros on the wrong side (left-pad vs right-pad).
    let parsed: UnixInstant = "10.5".parse().expect("short fractional parses");
    assert_eq!(parsed, UnixInstant::from_unix_duration(Duration::new(10, 500_000_000)));
    assert_eq!(parsed.to_string(), "10.500000000");
}

#[test]
fn from_str_accepts_full_nine_digit_fractional() {
    // `.000000123` -> 123 ns. Mirrors the Display fixture above.
    let parsed: UnixInstant = "1700000000.000000123".parse().expect("9-digit fractional parses");
    assert_eq!(parsed, UnixInstant::from_unix_duration(Duration::new(1_700_000_000, 123)));
}

// -----------------------------------------------------------------------------
// FromStr error coverage — every `ParseError` variant has a dedicated
// fixture so a mutation that collapses the variants (e.g. always
// returning `Empty`) is killed. The proptest below additionally walks
// every variant under property pressure.
// -----------------------------------------------------------------------------

#[test]
fn from_str_rejects_empty_string_with_empty_variant() {
    let err = "".parse::<UnixInstant>().expect_err("empty must reject");
    assert!(matches!(err, ParseError::Empty), "expected Empty, got {err:?}");
}

#[test]
fn from_str_rejects_non_numeric_input_with_malformed_variant() {
    let err = "abc".parse::<UnixInstant>().expect_err("non-numeric must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_double_decimal_point_with_malformed_variant() {
    let err = "1.2.3".parse::<UnixInstant>().expect_err("double-decimal must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_leading_whitespace_with_malformed_variant() {
    let err = " 1".parse::<UnixInstant>().expect_err("leading whitespace must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_trailing_whitespace_with_malformed_variant() {
    let err = "1 ".parse::<UnixInstant>().expect_err("trailing whitespace must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_more_than_nine_fractional_digits_with_malformed_variant() {
    // 10 fractional digits is one over the canonical width. Accepting
    // this would make Display non-injective (two distinct `UnixInstant`
    // values would have the same canonical form), so FromStr must
    // reject it. Pinning the boundary kills the off-by-one mutation.
    let err = "1.0000000001".parse::<UnixInstant>().expect_err("10-digit fractional must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_negative_seconds_with_malformed_variant() {
    // `Duration` does not represent negative spans; FromStr must
    // reject the leading sign rather than silently parse to a wrap.
    let err = "-1".parse::<UnixInstant>().expect_err("negative must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

#[test]
fn from_str_rejects_empty_fractional_after_dot_with_malformed_variant() {
    // `1.` is not a valid decimal — the dot demands at least one digit.
    let err = "1.".parse::<UnixInstant>().expect_err("empty fractional must reject");
    assert!(matches!(err, ParseError::MalformedDecimal), "expected MalformedDecimal, got {err:?}");
}

// -----------------------------------------------------------------------------
// serde JSON shape — the wire form is a JSON STRING (not a number,
// not an object) whose contents equal the Display canonical form.
// This matches the workspace convention every other newtype follows.
// -----------------------------------------------------------------------------

#[test]
fn serde_json_serialises_to_quoted_canonical_form() {
    let u = UnixInstant::from_unix_duration(Duration::new(1_700_000_000, 123));

    let json = serde_json::to_string(&u).expect("serialises");

    assert_eq!(json, "\"1700000000.000000123\"");

    // And the round-trip yields the same value.
    let back: UnixInstant = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, u);
}

#[test]
fn serde_json_rejects_non_string_inputs() {
    // A bare JSON number (not a quoted string) MUST NOT deserialise —
    // mixing the two would create two equally-valid wire forms and
    // silently break content-hash determinism for any record carrying
    // a `UnixInstant` field.
    let err = serde_json::from_str::<UnixInstant>("1700000000.000000000")
        .expect_err("number must reject");
    // `serde_json` returns a generic Error; we just need to confirm it
    // is an error, not a successful parse to some default value.
    let _ = err;
}

// -----------------------------------------------------------------------------
// Proptest — Display/FromStr roundtrip.
//
// The mandatory call site per `.claude/rules/testing.md`:
//
//   > Newtype roundtrip. Every newtype's Display / FromStr / serde
//   > must round-trip bit-equivalent for every valid input, and every
//   > invalid input must be rejected by FromStr with a structured
//   > ParseError.
//
// We bound the seconds component well below `u64::MAX` so downstream
// arithmetic (`UnixInstant + Duration`) does not overflow under
// composed property tests in later steps. 0..=10^12 seconds covers
// roughly 31 700 years past the epoch — well past any real persisted
// deadline.
// -----------------------------------------------------------------------------

/// A `Duration` strategy bounded to a safe sub-`u64::MAX` range. Pins
/// seconds to `0..=10^12` and nanos to the full `0..1_000_000_000`
/// range so every nanos digit in the Display form is exercised.
fn arb_unix_duration() -> impl Strategy<Value = Duration> {
    (0_u64..=1_000_000_000_000_u64, 0_u32..1_000_000_000_u32)
        .prop_map(|(secs, nanos)| Duration::new(secs, nanos))
}

proptest! {
    /// Display → FromStr roundtrip is total and lossless over every
    /// valid `UnixInstant`. Pinned by the mandatory-call-site rule.
    #[test]
    fn display_from_str_round_trip(d in arb_unix_duration()) {
        let original = UnixInstant::from_unix_duration(d);
        let rendered = original.to_string();
        let reparsed = UnixInstant::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);

        // Canonical width invariant — every Display output has the
        // shape `<secs>.<exactly-9-digits>`. Pinning this kills the
        // mutation that drops the zero-pad on `nanos == 0`.
        let dot_idx = rendered.find('.').expect("Display always emits a dot");
        let frac = &rendered[dot_idx + 1..];
        prop_assert_eq!(frac.len(), 9);
        prop_assert!(frac.chars().all(|c| c.is_ascii_digit()));
    }
}

// -----------------------------------------------------------------------------
// Proptest — FromStr structurally rejects malformed inputs across
// every `ParseError` variant. Each strategy below produces inputs
// that MUST trip the matching variant.
// -----------------------------------------------------------------------------

proptest! {
    /// Any input containing a non-digit, non-decimal-point character
    /// produces `MalformedDecimal`. Empty-input rejection is pinned
    /// by the dedicated `#[test]` above (a property of width zero).
    #[test]
    fn from_str_rejects_non_digit(s in "[a-zA-Z]{1,8}") {
        let err = s.parse::<UnixInstant>().expect_err("non-digit rejects");
        prop_assert!(matches!(err, ParseError::MalformedDecimal));
    }

    /// More than 9 fractional digits produces `MalformedDecimal`.
    #[test]
    fn from_str_rejects_overlong_fractional(
        secs in 0_u64..=1_000_u64,
        frac_len in 10_usize..=20_usize,
    ) {
        let frac = "1".repeat(frac_len);
        let s = format!("{secs}.{frac}");
        let err = s.parse::<UnixInstant>().expect_err("> 9 fractional digits rejects");
        prop_assert!(matches!(err, ParseError::MalformedDecimal));
    }
}

// -----------------------------------------------------------------------------
// Proptest — rkyv archive → access → deserialise → equal roundtrip.
//
// Mandatory call site per `.claude/rules/testing.md`:
//
//   > rkyv roundtrip. Archive → access → deserialise → equal-to-original
//   > for every durable type crossing the IntentStore boundary.
//
// `UnixInstant` crosses the IntentStore boundary as a field of
// `JobLifecycleView` in step 02-02 (#139); that wiring depends on
// this property holding for every valid value.
// -----------------------------------------------------------------------------

proptest! {
    #[test]
    fn rkyv_archive_access_deserialise_round_trip(d in arb_unix_duration()) {
        let original = UnixInstant::from_unix_duration(d);

        // Archive.
        let bytes = rkyv::to_bytes::<rancor::Error>(&original)
            .expect("rkyv archival of UnixInstant succeeds");

        // Access.
        let archived = rkyv::access::<rkyv::Archived<UnixInstant>, rancor::Error>(&bytes)
            .expect("rkyv access of archived UnixInstant succeeds");

        // Deserialise.
        let back: UnixInstant = rkyv::deserialize::<UnixInstant, rancor::Error>(archived)
            .expect("rkyv deserialise of UnixInstant succeeds");

        // Equal-to-original.
        prop_assert_eq!(back, original);

        // And archival is itself deterministic — two runs produce
        // identical bytes. This is the leg that makes content-hashing
        // a record carrying a `UnixInstant` field stable.
        let bytes_again = rkyv::to_bytes::<rancor::Error>(&original)
            .expect("second rkyv archival succeeds");
        prop_assert_eq!(bytes.as_ref(), bytes_again.as_ref());
    }
}

// -----------------------------------------------------------------------------
// Proptest — serde_json roundtrip equivalence.
// -----------------------------------------------------------------------------

proptest! {
    #[test]
    fn serde_json_round_trip_is_lossless(d in arb_unix_duration()) {
        let original = UnixInstant::from_unix_duration(d);
        let json = serde_json::to_string(&original).expect("serialises");

        // Wire form is a quoted Display string.
        let expected = format!("\"{original}\"");
        prop_assert_eq!(&json, &expected);

        // And the round-trip yields the same value.
        let back: UnixInstant = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }
}
