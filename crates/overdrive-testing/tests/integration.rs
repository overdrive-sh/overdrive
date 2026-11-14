//! Integration-test entrypoint for `overdrive-testing`: the Metal scenarios of
//! the placement attribute and the shared-guest owner. Gated behind
//! `integration-tests` and `kvm-tests` (the scenarios boot real guests on the
//! native Metal host; no off-Metal lane enables `kvm-tests`); the inline `mod integration` shifts the module lookup
//! base into `integration/` (testing.md § "Integration vs unit gating").

#![cfg(all(feature = "integration-tests", feature = "kvm-tests", target_os = "linux"))]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unwrap_used,
    clippy::manual_assert,
    clippy::similar_names,
    reason = "scenario drivers assert their own preconditions and name protocol terms"
)]

mod integration {
    mod scenario_support;

    mod lane_placement;
    mod placement_off_metal;
    mod shared_guest_fixtures;
    mod shared_guest_scenarios;
    mod shared_owner_lifetime;
}
