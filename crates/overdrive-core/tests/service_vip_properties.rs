//! Property tests for the canonical [`ServiceVip`] newtype + the
//! `ServiceVipAllocator` memo-hit behaviour.
//!
//! Step 01-02 of the service-vip-allocator feature (ADR-0049). Pins
//! the **single canonical** `ServiceVip` declaration in
//! `overdrive_core::id::ServiceVip` — the prior duplicate at
//! `overdrive_core::aggregate::workload_spec::ServiceVip` is deleted
//! in this step. Per DWD-03 — unit-level default lane, no
//! `integration-tests` feature gate; pure in-memory, no I/O.
//!
//! Scope:
//!
//! - **S-VIP-P01** — `ServiceVip::from_str(&ServiceVip::new(a).to_string())
//!   == Ok(ServiceVip(a))` for all valid IPv4 via proptest. Same for
//!   serde JSON roundtrip (serialize → deserialize → equal).
//!
//! The companion S-VIP-12 (allocator memo-hit) scenario lives in
//! `crates/overdrive-dataplane/tests/allocator_properties.rs` —
//! the allocator type cannot be exercised from `overdrive-core`
//! without a forbidden upward dependency on `overdrive-dataplane`.

#![allow(clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use overdrive_core::id::ServiceVip;
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators — every IPv4 address is a valid ServiceVip today.
// -----------------------------------------------------------------------------

fn valid_ipv4() -> impl Strategy<Value = Ipv4Addr> {
    any::<u32>().prop_map(Ipv4Addr::from)
}

// -----------------------------------------------------------------------------
// S-VIP-P01 — Display / FromStr round-trip + serde JSON round-trip.
// -----------------------------------------------------------------------------

proptest! {
    /// S-VIP-P01: `ServiceVip::from_str(&ServiceVip::new(a).to_string())
    /// == Ok(ServiceVip(a))` for every valid IPv4.
    #[test]
    fn service_vip_newtype_display_from_str_round_trip(addr in valid_ipv4()) {
        let original = ServiceVip::new(IpAddr::V4(addr))
            .expect("IPv4 is always a valid ServiceVip");
        let rendered = original.to_string();
        let reparsed = ServiceVip::from_str(&rendered)
            .expect("canonical Display must re-parse");
        prop_assert_eq!(reparsed, original);
    }

    /// S-VIP-P01 (serde leg): serialize → deserialize → equal, for
    /// every valid IPv4. The JSON form is the canonical Display form
    /// surrounded by quotes (matches `#[serde(into = "String",
    /// try_from = "String")]`).
    #[test]
    fn service_vip_newtype_serde_json_round_trip(addr in valid_ipv4()) {
        let original = ServiceVip::new(IpAddr::V4(addr))
            .expect("IPv4 is always a valid ServiceVip");
        let json = serde_json::to_string(&original).expect("serialises");
        let back: ServiceVip = serde_json::from_str(&json)
            .expect("deserialises through canonical String form");
        prop_assert_eq!(back, original);
    }
}
