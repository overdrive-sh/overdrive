//! Tier 1 acceptance — `ProbeIdx` newtype + `ProbeDescriptor`
//! serde roundtrip.
//!
//! Slice 01 of service-health-check-probes. Per
//! `.claude/rules/development.md` § "Newtype completeness", every
//! newtype must round-trip through its `Display` / `FromStr` /
//! `Serialize` / `Deserialize` pair bit-equivalent.
//!
//! Per `.claude/rules/testing.md` § "Property-based testing
//! (proptest)" → "Mandatory call sites" → "Newtype roundtrip", this
//! roundtrip is mandatory.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// ProbeIdx — newtype serde roundtrip.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// S-SHCP-IDX-01 — `ProbeIdx` round-trips bit-equivalent through
    /// `serde_json::to_string` / `from_str` for every u32.
    #[test]
    fn probe_idx_serde_roundtrip_bit_equivalent(value in any::<u32>()) {
        let original = ProbeIdx::new(value);
        let json = serde_json::to_string(&original).expect("ProbeIdx serializes");
        let parsed: ProbeIdx = serde_json::from_str(&json).expect("ProbeIdx deserializes");
        prop_assert_eq!(parsed, original);
        prop_assert_eq!(parsed.get(), value);
    }
}

// ---------------------------------------------------------------------------
// ProbeDescriptor — full aggregate serde roundtrip.
// ---------------------------------------------------------------------------

fn arb_probe_role() -> impl Strategy<Value = ProbeRole> {
    prop_oneof![Just(ProbeRole::Startup), Just(ProbeRole::Readiness), Just(ProbeRole::Liveness),]
}

fn arb_mechanic() -> impl Strategy<Value = ProbeMechanic> {
    prop_oneof![
        ("[a-zA-Z0-9._-]{1,40}", 1u16..=65535)
            .prop_map(|(host, port)| ProbeMechanic::Tcp { host, port }),
        ("/[a-zA-Z0-9_./-]{0,60}", 1u16..=65535, proptest::option::of("[a-zA-Z0-9._-]{1,40}"),)
            .prop_map(|(path, port, host)| ProbeMechanic::Http { path, port, host }),
        proptest::collection::vec("[a-zA-Z0-9_./-]{1,30}", 1..=4)
            .prop_map(|command| ProbeMechanic::Exec { command }),
    ]
}

fn arb_probe_descriptor() -> impl Strategy<Value = ProbeDescriptor> {
    (
        arb_probe_role(),
        arb_mechanic(),
        1u32..=60,
        1u32..=60,
        1u32..=300,
        proptest::option::of(1u32..=10),
        proptest::option::of(1u32..=10),
        any::<bool>(),
    )
        .prop_map(
            |(
                role,
                mechanic,
                timeout_seconds,
                interval_seconds,
                max_attempts,
                failure_threshold,
                success_threshold,
                inferred,
            )| {
                ProbeDescriptor {
                    role,
                    mechanic,
                    timeout_seconds,
                    interval_seconds,
                    max_attempts,
                    failure_threshold,
                    success_threshold,
                    inferred,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-SHCP-DESC-01 — `ProbeDescriptor` round-trips bit-equivalent
    /// through serde JSON. Covers every mechanic + every role + every
    /// threshold combination the generator spans.
    #[test]
    fn probe_descriptor_serde_roundtrip_bit_equivalent(
        descriptor in arb_probe_descriptor(),
    ) {
        let json = serde_json::to_string(&descriptor).expect("ProbeDescriptor serializes");
        let parsed: ProbeDescriptor =
            serde_json::from_str(&json).expect("ProbeDescriptor deserializes");
        prop_assert_eq!(parsed, descriptor);
    }
}
