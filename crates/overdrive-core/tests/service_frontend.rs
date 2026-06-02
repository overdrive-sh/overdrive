//! `ServiceFrontend` newtype — DISTILL acceptance scaffolds
//! (udp-service-support US-01 / S-01-A, S-01-B; ADR-0060, DESIGN D1a/D1b/D2).
//!
//! **RED scaffolds.** Bodies drive the `ServiceFrontend::new` /
//! `vip_v4()` production scaffold, whose `todo!("RED scaffold: …")`
//! panics carry the phrase "RED scaffold". Each test is annotated
//! `#[should_panic(expected = "RED scaffold")]` per
//! `.claude/rules/testing.md` § "RED scaffolds" — GREEN at the bar so
//! sibling commits do not need `--no-verify`. DELIVER drops the
//! `#[should_panic]` and writes the real assertions in the same commit
//! that lands US-01.
//!
//! Scenario SSOT: `docs/feature/udp-service-support/distill/test-scenarios.md`
//! S-01-A (IPv4 round-trips), S-01-B (IPv6 rejected).
//!
//! Tier 1 (in-memory, layer 1). PBT-full is permitted here (Mandate 9);
//! the GREEN versions of S-01-A/B SHOULD use `proptest` over IPv4 octets
//! × `NonZeroU16` × `{Tcp, Udp}` (S-01-A) and IPv6 segments (S-01-B).
//! The scaffolds below pin one canonical `@example` each; DELIVER
//! generalises to proptest.
//!
//! NOTE (D2): `ServiceFrontend` has NO `Display`/`FromStr` (no serde),
//! so there is **no** newtype string-roundtrip property — the roundtrip
//! is over the typed accessors (`vip_v4`/`port`/`proto`) only. Do not
//! add a string-roundtrip proptest.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceVip;

/// S-01-A — `ServiceFrontend::new` accepts an IPv4 VIP and the typed
/// accessors round-trip the supplied `(vip, port, proto)`.
///
/// GREEN: replace `#[should_panic]` with a `proptest!` over IPv4 octets
/// × `NonZeroU16` × `{Tcp, Udp}`; assert `new(..)` is `Ok`, and
/// `vip_v4()/port()/proto()` equal the inputs. Canonical `@example`:
/// `10.96.0.10`, port 5353, `Udp`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn ipv4_vip_constructs_and_accessors_round_trip() {
    let ipv4 = Ipv4Addr::new(10, 96, 0, 10);
    let vip = ServiceVip::new(IpAddr::V4(ipv4)).expect("valid IPv4 ServiceVip");
    let port = NonZeroU16::new(5353).expect("non-zero port");

    // Drives the production `new` scaffold: panics with "RED scaffold".
    let frontend =
        ServiceFrontend::new(vip, port, Proto::Udp).expect("S-01-A: IPv4 VIP must construct");

    // GREEN assertions (unreachable until the scaffold lands):
    assert_eq!(frontend.vip_v4(), ipv4, "vip_v4() must round-trip the IPv4");
    assert_eq!(frontend.port(), port, "port() must round-trip");
    assert_eq!(frontend.proto(), Proto::Udp, "proto() must round-trip");
}

/// S-01-B — `ServiceFrontend::new` REJECTS an IPv6 VIP (negative arm).
///
/// GREEN: replace `#[should_panic]` with a `proptest!` over IPv6
/// segments asserting `new(..)` is `Err`. Canonical `@example`: `::1`.
/// Per ADR-0060 / D1a the rejection is structured (`ParseError`) and is
/// surfaced as the existing operator-visible `Failed` row at the
/// action-shim (see S-01-F for the operator-visible-site scenario).
#[test]
#[should_panic(expected = "RED scaffold")]
fn ipv6_vip_is_rejected() {
    let vip = ServiceVip::new(IpAddr::V6(Ipv6Addr::LOCALHOST)).expect("valid IPv6 ServiceVip");
    let port = NonZeroU16::new(5353).expect("non-zero port");

    // Drives the production `new` scaffold: panics with "RED scaffold"
    // today. GREEN: this returns Err and the assertion below fires.
    let result = ServiceFrontend::new(vip, port, Proto::Udp);
    assert!(result.is_err(), "S-01-B: an IPv6 VIP must be rejected by new()");
}
