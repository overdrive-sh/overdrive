//! `ServiceFrontend` newtype — acceptance + PBT
//! (udp-service-support US-01 / S-01-A, S-01-B; ADR-0060, DESIGN D1a/D1b/D2).
//!
//! Scenario SSOT: `docs/feature/udp-service-support/distill/test-scenarios.md`
//! S-01-A (IPv4 round-trips), S-01-B (IPv6 rejected).
//!
//! Tier 1 (in-memory, layer 1). `proptest! full` (criteria 1-2): the
//! property is over IPv4 octets × `NonZeroU16` × `{Tcp, Udp}` (S-01-A)
//! and IPv6 segments (S-01-B).
//!
//! NOTE (D2): `ServiceFrontend` has NO `Display`/`FromStr` (no serde),
//! so there is **no** newtype string-roundtrip property — the roundtrip
//! is over the typed accessors (`vip_v4`/`port`/`proto`) only.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceVip;
use proptest::prelude::*;

/// Strategy over the two L4 protocols `ServiceFrontend` admits.
fn proto_strategy() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

proptest! {
    /// S-01-A — `ServiceFrontend::new` accepts any IPv4 VIP and the typed
    /// accessors round-trip the supplied `(vip, port, proto)` exactly.
    #[test]
    fn ipv4_vip_constructs_and_accessors_round_trip(
        octets in any::<[u8; 4]>(),
        port_raw in 1u16..=u16::MAX,
        proto in proto_strategy(),
    ) {
        let ipv4 = Ipv4Addr::from(octets);
        let vip = ServiceVip::new(IpAddr::V4(ipv4)).expect("valid IPv4 ServiceVip");
        let port = NonZeroU16::new(port_raw).expect("non-zero port");

        let frontend =
            ServiceFrontend::new(vip, port, proto).expect("S-01-A: IPv4 VIP must construct");

        prop_assert_eq!(frontend.vip_v4(), ipv4, "vip_v4() must round-trip the IPv4");
        prop_assert_eq!(frontend.port(), port, "port() must round-trip");
        prop_assert_eq!(frontend.proto(), proto, "proto() must round-trip");
        prop_assert_eq!(frontend.vip(), vip, "vip() must round-trip the ServiceVip");
    }

    /// S-01-B — `ServiceFrontend::new` REJECTS any IPv6 VIP (negative arm).
    /// Per ADR-0060 / D1a the rejection is structured (`IdParseError`) and
    /// is surfaced as the existing operator-visible `Failed` row at the
    /// action-shim (see `service_frontend_ipv6_rejected` for the
    /// operator-visible-site scenario).
    #[test]
    fn ipv6_vip_is_rejected(
        segments in any::<[u16; 8]>(),
        port_raw in 1u16..=u16::MAX,
        proto in proto_strategy(),
    ) {
        let ipv6 = Ipv6Addr::from(segments);
        let vip = ServiceVip::new(IpAddr::V6(ipv6)).expect("valid IPv6 ServiceVip");
        let port = NonZeroU16::new(port_raw).expect("non-zero port");

        let result = ServiceFrontend::new(vip, port, proto);
        prop_assert!(result.is_err(), "S-01-B: an IPv6 VIP must be rejected by new()");
    }
}
