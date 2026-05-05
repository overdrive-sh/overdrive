//! Proptest — `ServiceVip` newtype roundtrip (S-2.2-04 sibling).
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites — Newtype
//! roundtrip*: every newtype's `Display` / `FromStr` / serde must
//! round-trip bit-equivalent for every valid input, and every invalid
//! input must be rejected by `FromStr` with a structured `ParseError`.
//! Per `development.md` § Newtype completeness — case-insensitive parse
//! for human-typed inputs (IPv6 hex digits), lowercase canonical form.
//!
//! `ServiceVip` wraps [`std::net::IpAddr`] per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! Construction validates the underlying address (IPv4 today; IPv6
//! deferred to GH #155); the userspace control-plane newtype is
//! distinct from the wire-shape `vip: Ipv4Addr` that
//! `service_backends` rows continue to carry.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use overdrive_core::id::{IdParseError, ServiceVip};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators — every IPv4 address is a valid ServiceVip today.
// -----------------------------------------------------------------------------

fn valid_ipv4() -> impl Strategy<Value = Ipv4Addr> {
    any::<u32>().prop_map(Ipv4Addr::from)
}

// -----------------------------------------------------------------------------
// Round-trip properties — Display / FromStr / serde compose to identity.
// -----------------------------------------------------------------------------

proptest! {
    /// `ServiceVip::from(IpAddr)` round-trips through Display → FromStr.
    #[test]
    fn service_vip_display_from_str_round_trip(addr in valid_ipv4()) {
        let original =
            ServiceVip::new(IpAddr::V4(addr)).expect("IPv4 is always a valid ServiceVip");
        let rendered = original.to_string();
        let reparsed =
            ServiceVip::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// `ServiceVip` round-trips through serde JSON. The JSON form is the
    /// canonical Display form surrounded by quotes.
    #[test]
    fn service_vip_serde_round_trip(addr in valid_ipv4()) {
        let original =
            ServiceVip::new(IpAddr::V4(addr)).expect("IPv4 is always a valid ServiceVip");
        let json = serde_json::to_string(&original).expect("serialises");
        let expected = format!("\"{original}\"");
        prop_assert_eq!(&json, &expected);
        let back: ServiceVip = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, original);
    }

    /// `ServiceVip` round-trips through rkyv archive → access →
    /// deserialise. The mandatory rkyv round-trip per
    /// `.claude/rules/testing.md` *Mandatory call sites — rkyv roundtrip*.
    #[test]
    fn service_vip_rkyv_round_trip(addr in valid_ipv4()) {
        let original =
            ServiceVip::new(IpAddr::V4(addr)).expect("IPv4 is always a valid ServiceVip");
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&original)
            .expect("rkyv archive of ServiceVip is infallible");
        let restored: ServiceVip =
            rkyv::from_bytes::<ServiceVip, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialise round-trips");
        prop_assert_eq!(restored, original);
    }
}

// -----------------------------------------------------------------------------
// Invalid-input rejection — structural errors must surface as
// structured `IdParseError` variants, not panics.
// -----------------------------------------------------------------------------

#[test]
fn service_vip_rejects_empty_input() {
    let err = ServiceVip::from_str("").expect_err("empty must reject");
    assert!(matches!(err, IdParseError::Empty { .. }), "expected Empty variant, got {err:?}");
}

#[test]
fn service_vip_rejects_garbage_string() {
    let err = ServiceVip::from_str("not-an-ip").expect_err("non-IP must reject");
    assert!(
        matches!(err, IdParseError::InvalidFormat { .. }),
        "expected InvalidFormat variant, got {err:?}"
    );
}

// -----------------------------------------------------------------------------
// Display canonical form is lowercase for IPv6 hex digits — humans pasting
// uppercased IPv6 strings ("::FFFF") still parse, but the canonical
// Display always emits lowercase. IPv4 has no case axis, so this is
// covered by a structural test rather than a property.
// -----------------------------------------------------------------------------

#[test]
fn service_vip_canonical_form_is_lossless_for_ipv4() {
    let addr = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid IPv4");
    assert_eq!(addr.to_string(), "10.0.0.1");
}
