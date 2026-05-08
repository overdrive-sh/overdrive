//! Proptest — `BackendKey` newtype roundtrip + STRICT validation
//! (S-2.2-15..18 / Slice 05).
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites — Newtype
//! roundtrip*: every newtype's `Display` / `FromStr` / serde must
//! round-trip bit-equivalent for every valid input, and every invalid
//! input must be rejected by `FromStr` with a structured `ParseError`.
//!
//! `BackendKey` is the kernel-side key for `REVERSE_NAT_MAP` per
//! `docs/feature/phase-2-xdp-service-map/discuss/user-stories.md`
//! AC #1 (US-05; line 793) — a triple `(ip, port, proto)`. Stored
//! host-order; the kernel-side egress program converts at the read
//! boundary per architecture.md § 11. Userspace stores host-order
//! without flipping — the same lockstep contract `ServiceMapHandle`
//! and `BackendMapHandle` carry.
//!
//! Wire form:
//!
//! - `Display` emits the canonical `"<ip>:<port>/<proto>"` form
//!   (e.g. `10.0.0.1:8080/tcp`). `FromStr` parses the same shape;
//!   case-insensitive on the `proto` token (matches `ServiceVip`
//!   IPv6 hex casing precedent).
//! - `Serialize` / `Deserialize` use a structured form rather than
//!   transparent — the tuple's three fields are too distinct to
//!   collapse into a numeric, and a string form preserves audit-log
//!   readability while staying canonical for content-hashing.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::net::Ipv4Addr;
use std::str::FromStr;

use overdrive_core::dataplane::backend_key::{BackendKey, ParseError, Proto};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators.
// -----------------------------------------------------------------------------

fn arb_proto() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

fn arb_ipv4() -> impl Strategy<Value = Ipv4Addr> {
    any::<u32>().prop_map(Ipv4Addr::from)
}

fn arb_backend_key() -> impl Strategy<Value = BackendKey> {
    (arb_ipv4(), any::<u16>(), arb_proto())
        .prop_map(|(ip, port, proto)| BackendKey::new(ip, port, proto))
}

// -----------------------------------------------------------------------------
// Round-trip properties.
// -----------------------------------------------------------------------------

proptest! {
    /// `BackendKey` round-trips through `Display → FromStr` for every
    /// `(ip, port, proto)` triple.
    #[test]
    fn backend_key_display_from_str_round_trip(key in arb_backend_key()) {
        let rendered = key.to_string();
        let reparsed = BackendKey::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, key);
    }

    /// `BackendKey` round-trips through serde JSON. JSON form mirrors
    /// the canonical `Display` output — see module docs.
    #[test]
    fn backend_key_serde_round_trip(key in arb_backend_key()) {
        let json = serde_json::to_string(&key).expect("serialises");
        let back: BackendKey = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, key);
    }

    /// `BackendKey` round-trips through rkyv archive → access →
    /// deserialise. The mandatory rkyv round-trip per
    /// `.claude/rules/testing.md` *Mandatory call sites — rkyv roundtrip*.
    #[test]
    fn backend_key_rkyv_round_trip(key in arb_backend_key()) {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&key)
            .expect("rkyv archive of BackendKey is infallible");
        let restored: BackendKey =
            rkyv::from_bytes::<BackendKey, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialise round-trips");
        prop_assert_eq!(restored, key);
    }

    /// Case-insensitive proto token — `TCP`, `tcp`, `Tcp` all parse
    /// identically. Matches the `ServiceVip` IPv6 hex casing precedent.
    #[test]
    fn backend_key_proto_is_case_insensitive(
        ip in arb_ipv4(),
        port in any::<u16>(),
        proto in arb_proto()
    ) {
        let key = BackendKey::new(ip, port, proto);
        let lower = key.to_string();
        let upper = lower.to_ascii_uppercase();
        let from_lower = BackendKey::from_str(&lower).expect("lower parses");
        let from_upper = BackendKey::from_str(&upper).expect("upper parses");
        prop_assert_eq!(from_lower, key);
        prop_assert_eq!(from_upper, key);
    }
}

// -----------------------------------------------------------------------------
// Hand-picked rejection cases.
// -----------------------------------------------------------------------------

#[test]
fn from_str_rejects_empty_input() {
    let err = BackendKey::from_str("").expect_err("empty must reject");
    assert!(matches!(err, ParseError::Empty), "expected Empty variant, got {err:?}");
}

#[test]
fn from_str_rejects_missing_proto_separator() {
    // No '/' separator means we cannot distinguish proto from address.
    let err = BackendKey::from_str("10.0.0.1:8080").expect_err("missing '/' must reject");
    assert!(matches!(err, ParseError::Malformed(_)), "expected Malformed, got {err:?}");
}

#[test]
fn from_str_rejects_missing_port_separator() {
    let err = BackendKey::from_str("10.0.0.1/tcp").expect_err("missing ':' must reject");
    assert!(matches!(err, ParseError::Malformed(_)), "expected Malformed, got {err:?}");
}

#[test]
fn from_str_rejects_unknown_proto() {
    let err = BackendKey::from_str("10.0.0.1:8080/sctp").expect_err("unknown proto must reject");
    assert!(matches!(err, ParseError::UnknownProto(_)), "expected UnknownProto, got {err:?}");
}

#[test]
fn from_str_rejects_non_ipv4_address() {
    let err = BackendKey::from_str("not-an-ip:8080/tcp").expect_err("non-IPv4 must reject");
    assert!(
        matches!(err, ParseError::Malformed(_)),
        "expected Malformed for bad IPv4, got {err:?}"
    );
}

#[test]
fn from_str_rejects_port_overflow() {
    let err = BackendKey::from_str("10.0.0.1:65536/tcp").expect_err("port overflow must reject");
    assert!(matches!(err, ParseError::Malformed(_)), "expected Malformed, got {err:?}");
}

#[test]
fn proto_to_u8_matches_iana_assignments() {
    // RFC 1700 / IANA — TCP=6, UDP=17.
    assert_eq!(Proto::Tcp.as_u8(), 6);
    assert_eq!(Proto::Udp.as_u8(), 17);
}

#[test]
fn proto_from_u8_round_trips_for_known_values() {
    assert_eq!(Proto::try_from(6u8).expect("TCP=6"), Proto::Tcp);
    assert_eq!(Proto::try_from(17u8).expect("UDP=17"), Proto::Udp);
    assert!(Proto::try_from(99u8).is_err(), "unknown proto must reject");
}

#[test]
fn display_canonical_form_matches_spec() {
    // The canonical Display form emitted by BackendKey.
    let key = BackendKey::new(Ipv4Addr::new(10, 0, 0, 1), 8080, Proto::Tcp);
    assert_eq!(key.to_string(), "10.0.0.1:8080/tcp");
}
