//! Proptest — `ServiceId` newtype roundtrip (S-2.2-04 sibling).
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites — Newtype
//! roundtrip*: every newtype's `Display` / `FromStr` / serde must
//! round-trip bit-equivalent for every valid input, and every invalid
//! input must be rejected by `FromStr` with a structured `ParseError`.
//!
//! `ServiceId` wraps a `u64` content hash per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! The `u64` is content-hashed from a `(VIP, port, scope)` tuple
//! upstream; the newtype itself is opaque — it carries the value, not
//! the derivation. Display emits the decimal `u64`; `FromStr` parses
//! decimal `u64`. There is no case axis for a numeric identifier, so
//! the case-insensitivity rule from `development.md` § Newtype
//! completeness does not apply (matches the precedent of `BackendId`,
//! `MaglevTableSize`).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{IdParseError, ServiceId};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Round-trip properties.
// -----------------------------------------------------------------------------

proptest! {
    /// `ServiceId` round-trips through Display → FromStr for any `u64`.
    #[test]
    fn service_id_display_from_str_round_trip(value in any::<u64>()) {
        let original = ServiceId::new(value).expect("any u64 is a valid ServiceId");
        let rendered = original.to_string();
        let reparsed = ServiceId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
        prop_assert_eq!(reparsed.get(), value);
    }

    /// `ServiceId` round-trips through serde JSON. JSON form is the
    /// numeric `u64` literal (transparent serde representation —
    /// matches the `BackendSetFingerprint` type-alias precedent of
    /// content-derived numeric IDs surfacing as bare `u64`).
    #[test]
    fn service_id_serde_round_trip(value in any::<u64>()) {
        let original = ServiceId::new(value).expect("any u64 is a valid ServiceId");
        let json = serde_json::to_string(&original).expect("serialises");
        let expected = value.to_string();
        prop_assert_eq!(&json, &expected);
        let back: ServiceId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }

    /// `ServiceId` round-trips through rkyv archive → access →
    /// deserialise. The mandatory rkyv round-trip per
    /// `.claude/rules/testing.md` *Mandatory call sites — rkyv roundtrip*.
    #[test]
    fn service_id_rkyv_round_trip(value in any::<u64>()) {
        let original = ServiceId::new(value).expect("any u64 is a valid ServiceId");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&original)
            .expect("rkyv archive of ServiceId is infallible");
        let restored: ServiceId =
            rkyv::from_bytes::<ServiceId, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialise round-trips");
        prop_assert_eq!(restored, original);
    }
}

// -----------------------------------------------------------------------------
// Invalid-input rejection — non-numeric input must surface as a
// structured `IdParseError`, not a `ParseIntError` / panic.
// -----------------------------------------------------------------------------

#[test]
fn service_id_rejects_empty_input() {
    let err = ServiceId::from_str("").expect_err("empty must reject");
    assert!(matches!(err, IdParseError::Empty { .. }), "expected Empty variant, got {err:?}");
}

#[test]
fn service_id_rejects_non_numeric_string() {
    let err = ServiceId::from_str("not-a-number").expect_err("non-numeric must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}

#[test]
fn service_id_rejects_negative_number() {
    let err = ServiceId::from_str("-1").expect_err("negative must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}

#[test]
fn service_id_rejects_overflow() {
    // u128::MAX rendered as decimal — far beyond u64 range.
    let err = ServiceId::from_str(&u128::MAX.to_string()).expect_err("u64 overflow must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}
