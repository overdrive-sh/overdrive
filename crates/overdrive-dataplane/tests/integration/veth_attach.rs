//! S-2.2-01..03 — Real-iface XDP attach.
//!
//! Tags: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the Gherkin specification of each scenario.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-01 — Real veth pair attach with packet count assertion.
/// Starting scenario for DELIVER (NOT `@pending`).
#[test]
fn xdp_attaches_to_real_veth_and_packet_counter_increments() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-01 — \
         create veth0/veth1, attach xdp_pass, push 100 frames, \
         assert PACKET_COUNTER reads 100"
    );
}

/// S-2.2-02 — Native attach failure logs structured fallback warning.
#[test]
#[ignore = "RED scaffold S-2.2-02 — DELIVER fills the body per Slice 01"]
fn native_attach_failure_logs_fallback_warning() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-02 — \
         native attach failure on driver lacking native XDP support \
         logs xdp.attach.fallback_generic and falls back to XDP_SKB"
    );
}

/// S-2.2-03 — Missing iface produces typed `IfaceNotFound` error.
#[test]
#[ignore = "RED scaffold S-2.2-03 — DELIVER fills the body per Slice 01"]
fn missing_iface_returns_typed_iface_not_found_error() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-03 — \
         loader returns DataplaneError::IfaceNotFound on missing \
         interface; no XDP program is loaded"
    );
}
