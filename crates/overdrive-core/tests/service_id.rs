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

use std::num::NonZeroU16;
use std::str::FromStr;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{IdParseError, ServiceId, ServiceVip};
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

// -----------------------------------------------------------------------------
// `ServiceId::derive` — proto axis distinctness (Model A proto-widening,
// ADR-0040 companion revision 2026-06-03 / ADR-0052 § 1).
//
// `derive` gains an L4-protocol axis so two listeners on the same
// `(vip, port)` but different protocol (the canonical CoreDNS
// `tcp/53` + `udp/53` case) derive DISTINCT `ServiceId`s instead of
// colliding. These tests pin that the proto byte materially shifts the
// hash — they kill the "proto dropped from the hash pre-image" mutation
// on `id.rs::derive`.
// -----------------------------------------------------------------------------

fn vip(addr: &str) -> ServiceVip {
    ServiceVip::new(addr.parse().expect("valid ip")).expect("valid vip")
}

/// The reported bug, as a regression assertion: `ServiceId::derive`
/// over the SAME `(vip, port, purpose)` but DISTINCT protocols must
/// produce DISTINCT ids. Before the proto-widening this collapsed to a
/// single id (last-writer-wins in `ListenerFactStore`). `CoreDNS`
/// `tcp/53` + `udp/53` is the canonical motivating case.
#[test]
fn derive_distinguishes_tcp_from_udp_on_same_vip_and_port() {
    let v = vip("10.96.0.53");
    let port = NonZeroU16::new(53).expect("non-zero port");

    let tcp = ServiceId::derive(&v, port, Proto::Tcp, "service-map");
    let udp = ServiceId::derive(&v, port, Proto::Udp, "service-map");

    assert_ne!(
        tcp, udp,
        "CoreDNS tcp/53 and udp/53 must derive DISTINCT ServiceIds — \
         the proto axis is part of the identity (ADR-0040 companion / ADR-0052 § 1)"
    );
}

proptest! {
    /// Property: for ANY `(vip, port, purpose)`, swapping the protocol
    /// from TCP to UDP changes the derived `ServiceId`. The proto byte
    /// is a load-bearing input to the hash pre-image — a `derive` that
    /// dropped it would make these two ids equal for every input.
    #[test]
    fn derive_proto_axis_changes_id_for_any_vip_port_purpose(
        octets in any::<[u8; 4]>(),
        raw_port in 1u16..=u16::MAX,
        purpose in "[a-z][a-z0-9-]{0,15}",
    ) {
        let v = ServiceVip::new(std::net::Ipv4Addr::from(octets).into())
            .expect("any IPv4 is a valid ServiceVip");
        let port = NonZeroU16::new(raw_port).expect("1..=u16::MAX is non-zero");

        let tcp = ServiceId::derive(&v, port, Proto::Tcp, &purpose);
        let udp = ServiceId::derive(&v, port, Proto::Udp, &purpose);

        prop_assert_ne!(
            tcp, udp,
            "proto axis must change the derived id for ({}, {}, {:?})",
            v, raw_port, purpose
        );
    }

    /// Property: `derive` is deterministic AND total over its inputs —
    /// the same 4-tuple always yields the same id (no hidden state, no
    /// nondeterminism). Guards the hash-determinism contract per
    /// `.claude/rules/development.md` § "Hashing requires deterministic
    /// serialization".
    #[test]
    fn derive_is_deterministic_per_proto(
        octets in any::<[u8; 4]>(),
        raw_port in 1u16..=u16::MAX,
        proto_is_tcp in any::<bool>(),
    ) {
        let v = ServiceVip::new(std::net::Ipv4Addr::from(octets).into())
            .expect("any IPv4 is a valid ServiceVip");
        let port = NonZeroU16::new(raw_port).expect("1..=u16::MAX is non-zero");
        let proto = if proto_is_tcp { Proto::Tcp } else { Proto::Udp };

        let first = ServiceId::derive(&v, port, proto, "service-map");
        let second = ServiceId::derive(&v, port, proto, "service-map");

        prop_assert_eq!(first, second, "derive must be deterministic for a fixed 4-tuple");
    }
}
