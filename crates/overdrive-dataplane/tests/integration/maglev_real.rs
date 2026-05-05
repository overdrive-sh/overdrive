//! ASR-2.2-02 Tier 3 confirm — Maglev disruption bound on real
//! veth (sibling of the Tier 1 DST proptest in
//! `crates/overdrive-sim/tests/integration/maglev_churn.rs`).
//!
//! Tags: `@US-04` `@K4` `@slice-04` `@ASR-2.2-02`
//! `@real-io @adapter-integration` `@pending`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// Tier 3 confirm of ASR-2.2-02. The Tier 1 DST proptest is the
/// primary surface; this test runs once on real veth at lower
/// `xdp-trafficgen` rate to confirm the bound holds against real
/// kernel verifier + real packet flow.
#[test]
#[ignore = "RED scaffold ASR-2.2-02 Tier 3 — DELIVER fills the body per Slice 04"]
fn maglev_disruption_bound_holds_on_real_veth() {
    panic!(
        "Not yet implemented -- RED scaffold: ASR-2.2-02 Tier 3 confirm — \
         100 backends, remove one, assert ≤ 2 % total flow shift on real veth"
    );
}
