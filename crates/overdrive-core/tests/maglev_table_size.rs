//! Proptest — `MaglevTableSize` newtype roundtrip + STRICT validation
//! (S-2.2-12 newtype completeness portion).
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites — Newtype
//! roundtrip*: every newtype's `Display` / `FromStr` / serde must
//! round-trip bit-equivalent for every valid input, and every invalid
//! input must be rejected by `FromStr` / `TryFrom<u32>` with a
//! structured `ParseError`.
//!
//! `MaglevTableSize` wraps a `u32` constrained to Cilium's prime list per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! The list is `{ 251, 509, 1_021, 2_039, 4_093, 8_191, 16_381, 32_749,
//! 65_521, 131_071 }` (research § 5.2). Every value outside this set —
//! whether composite (17, 100) or prime-but-not-listed (13, 17,
//! `1_000_000_007`) — must reject with a structured
//! [`ParseError::NotInPrimeList`] variant.
//!
//! There is no case axis for a numeric identifier, so the
//! case-insensitivity rule from `development.md` § Newtype completeness
//! does not apply (matches the precedent of `BackendId` / `ServiceId`).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::dataplane::maglev_table_size::{
    ALLOWED_PRIMES, DEFAULT_M, MaglevTableSize, ParseError,
};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Constants surface — every prime in the Cilium list is accepted; the
// `DEFAULT` constant matches `DEFAULT_M` and is in the prime list.
// -----------------------------------------------------------------------------

#[test]
fn default_constant_matches_design_locked_value() {
    // Architecture.md § 6 locks `DEFAULT = 16_381` (Q5=A / Q6=A).
    assert_eq!(MaglevTableSize::DEFAULT.get(), 16_381);
    assert_eq!(DEFAULT_M, 16_381);
    assert_eq!(MaglevTableSize::default(), MaglevTableSize::DEFAULT);
}

#[test]
fn every_prime_in_allowed_list_is_accepted() {
    for &prime in &ALLOWED_PRIMES {
        let m = MaglevTableSize::new(prime)
            .unwrap_or_else(|err| panic!("prime {prime} must be accepted, got {err:?}"));
        assert_eq!(m.get(), prime);
    }
}

// -----------------------------------------------------------------------------
// Round-trip properties.
// -----------------------------------------------------------------------------

/// Generator picking uniformly from the Cilium prime list — every
/// `MaglevTableSize` valid by construction.
fn arb_prime() -> impl Strategy<Value = u32> {
    proptest::sample::select(ALLOWED_PRIMES.to_vec())
}

proptest! {
    /// `MaglevTableSize` round-trips through `Display → FromStr` for
    /// every prime in the Cilium list.
    #[test]
    fn maglev_display_from_str_round_trip(value in arb_prime()) {
        let original = MaglevTableSize::new(value).expect("prime must construct");
        let rendered = original.to_string();
        let reparsed = MaglevTableSize::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
        prop_assert_eq!(reparsed.get(), value);
    }

    /// `MaglevTableSize` round-trips through serde JSON. JSON form is
    /// the numeric `u32` literal (`#[serde(try_from = "u32", into = "u32")]`
    /// on the type per architecture.md § 6).
    #[test]
    fn maglev_serde_round_trip(value in arb_prime()) {
        let original = MaglevTableSize::new(value).expect("prime must construct");
        let json = serde_json::to_string(&original).expect("serialises");
        let expected = value.to_string();
        prop_assert_eq!(&json, &expected);
        let back: MaglevTableSize = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }

    /// `MaglevTableSize` round-trips through rkyv archive → access →
    /// deserialise. The mandatory rkyv round-trip per
    /// `.claude/rules/testing.md` *Mandatory call sites — rkyv roundtrip*.
    #[test]
    fn maglev_rkyv_round_trip(value in arb_prime()) {
        let original = MaglevTableSize::new(value).expect("prime must construct");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&original)
            .expect("rkyv archive of MaglevTableSize is infallible");
        let restored: MaglevTableSize =
            rkyv::from_bytes::<MaglevTableSize, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialise round-trips");
        prop_assert_eq!(restored, original);
    }

    /// Every `u32` not in `ALLOWED_PRIMES` must reject through
    /// `MaglevTableSize::new` with a structured
    /// [`ParseError::NotInPrimeList`] variant — never a panic, never a
    /// silent accept.
    #[test]
    fn maglev_rejects_non_prime_u32(value in any::<u32>()
        .prop_filter("must not be in the Cilium prime list",
                     |v| !ALLOWED_PRIMES.contains(v)))
    {
        let err = MaglevTableSize::new(value).expect_err("non-prime must reject");
        prop_assert!(
            matches!(err, ParseError::NotInPrimeList { value: v, .. } if v == value),
            "expected NotInPrimeList {{ value: {value}, .. }}, got {err:?}"
        );
    }

    /// Serde `Deserialize` must validate via `TryFrom<u32>` per the
    /// `#[serde(try_from = "u32")]` attribute. A wire payload carrying
    /// a non-prime u32 must be rejected at the deserialisation boundary,
    /// never silently accepted (architecture.md § 6).
    #[test]
    fn maglev_serde_rejects_non_prime(value in any::<u32>()
        .prop_filter("must not be in the Cilium prime list",
                     |v| !ALLOWED_PRIMES.contains(v)))
    {
        let json = value.to_string();
        let result: Result<MaglevTableSize, _> = serde_json::from_str(&json);
        prop_assert!(result.is_err(),
                     "serde must reject non-prime {value}, but accepted it");
    }
}

// -----------------------------------------------------------------------------
// Hand-picked rejection cases — the AC's blocking examples.
// -----------------------------------------------------------------------------

#[test]
fn rejects_seventeen_composite_with_structured_error() {
    // 17 is composite — listed explicitly in AC #2.
    let err = MaglevTableSize::new(17).expect_err("17 (composite) must reject");
    let ParseError::NotInPrimeList { value, allowed } = err.clone() else {
        panic!("expected NotInPrimeList, got {err:?}");
    };
    assert_eq!(value, 17);
    assert_eq!(allowed, ALLOWED_PRIMES);
}

#[test]
fn rejects_thirteen_prime_but_not_listed_with_structured_error() {
    // 13 is prime but NOT in the Cilium list — listed explicitly in AC #2.
    // This guards against the "any prime is fine" misimplementation.
    let err = MaglevTableSize::new(13).expect_err("13 (prime, not listed) must reject");
    let ParseError::NotInPrimeList { value, allowed } = err.clone() else {
        panic!("expected NotInPrimeList, got {err:?}");
    };
    assert_eq!(value, 13);
    assert_eq!(allowed, ALLOWED_PRIMES);
}

#[test]
fn from_str_rejects_empty_input() {
    let err = MaglevTableSize::from_str("").expect_err("empty must reject");
    assert!(matches!(err, ParseError::Malformed(_)), "expected Malformed variant, got {err:?}");
}

#[test]
fn from_str_rejects_non_numeric_string() {
    let err = MaglevTableSize::from_str("not-a-number").expect_err("non-numeric must reject");
    assert!(matches!(err, ParseError::Malformed(_)), "expected Malformed variant, got {err:?}");
}

#[test]
fn from_str_rejects_non_prime_value() {
    // FromStr parses 17 successfully as a u32, then validates — must
    // surface NotInPrimeList, not Malformed.
    let err = MaglevTableSize::from_str("17").expect_err("17 must reject");
    assert!(
        matches!(err, ParseError::NotInPrimeList { value: 17, .. }),
        "expected NotInPrimeList {{ value: 17, .. }}, got {err:?}"
    );
}

#[test]
fn try_from_u32_rejects_non_prime() {
    let err = MaglevTableSize::try_from(100u32).expect_err("100 (non-prime) must reject");
    assert!(
        matches!(err, ParseError::NotInPrimeList { value: 100, .. }),
        "expected NotInPrimeList {{ value: 100, .. }}, got {err:?}"
    );
}

#[test]
fn from_into_u32_round_trips_for_default() {
    let m = MaglevTableSize::DEFAULT;
    let raw: u32 = m.into();
    assert_eq!(raw, DEFAULT_M);
    let back = MaglevTableSize::try_from(raw).expect("DEFAULT_M must round-trip");
    assert_eq!(back, m);
}
