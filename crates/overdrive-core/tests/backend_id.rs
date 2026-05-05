//! Proptest — `BackendId` newtype roundtrip (S-2.2-09 sibling).
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites — Newtype
//! roundtrip*: every newtype's `Display` / `FromStr` / serde must
//! round-trip bit-equivalent for every valid input, and every invalid
//! input must be rejected by `FromStr` with a structured `ParseError`.
//!
//! `BackendId` wraps a `u32` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! It is a stable monotonic backend identifier shared across services,
//! used as the key for `BACKEND_MAP`. Display emits the decimal `u32`;
//! `FromStr` parses decimal `u32`. There is no case axis for a numeric
//! identifier, so the case-insensitivity rule from `development.md`
//! § Newtype completeness does not apply (matches the precedent of
//! `ServiceId` / `MaglevTableSize`).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{BackendId, IdParseError};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Round-trip properties.
// -----------------------------------------------------------------------------

proptest! {
    /// `BackendId` round-trips through Display → FromStr for any `u32`.
    #[test]
    fn backend_id_display_from_str_round_trip(value in any::<u32>()) {
        let original = BackendId::new(value).expect("any u32 is a valid BackendId");
        let rendered = original.to_string();
        let reparsed = BackendId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
        prop_assert_eq!(reparsed.get(), value);
    }

    /// `BackendId` round-trips through serde JSON. JSON form is the
    /// numeric `u32` literal (transparent serde representation —
    /// matches the `ServiceId` precedent of content-derived numeric
    /// IDs surfacing as bare integers).
    #[test]
    fn backend_id_serde_round_trip(value in any::<u32>()) {
        let original = BackendId::new(value).expect("any u32 is a valid BackendId");
        let json = serde_json::to_string(&original).expect("serialises");
        let expected = value.to_string();
        prop_assert_eq!(&json, &expected);
        let back: BackendId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }

    /// `BackendId` round-trips through rkyv archive → access →
    /// deserialise. The mandatory rkyv round-trip per
    /// `.claude/rules/testing.md` *Mandatory call sites — rkyv roundtrip*.
    #[test]
    fn backend_id_rkyv_round_trip(value in any::<u32>()) {
        let original = BackendId::new(value).expect("any u32 is a valid BackendId");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&original)
            .expect("rkyv archive of BackendId is infallible");
        let restored: BackendId =
            rkyv::from_bytes::<BackendId, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialise round-trips");
        prop_assert_eq!(restored, original);
    }
}

// -----------------------------------------------------------------------------
// Invalid-input rejection — non-numeric input must surface as a
// structured `IdParseError`, not a `ParseIntError` / panic.
// -----------------------------------------------------------------------------

#[test]
fn backend_id_rejects_empty_input() {
    let err = BackendId::from_str("").expect_err("empty must reject");
    assert!(matches!(err, IdParseError::Empty { .. }), "expected Empty variant, got {err:?}");
}

#[test]
fn backend_id_rejects_non_numeric_string() {
    let err = BackendId::from_str("not-a-number").expect_err("non-numeric must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}

#[test]
fn backend_id_rejects_negative_number() {
    let err = BackendId::from_str("-1").expect_err("negative must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}

#[test]
fn backend_id_rejects_overflow() {
    // u64::MAX rendered as decimal — far beyond u32 range.
    let err = BackendId::from_str(&u64::MAX.to_string()).expect_err("u32 overflow must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}
