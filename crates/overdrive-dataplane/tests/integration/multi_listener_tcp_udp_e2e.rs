//! Tier 3 — multi-listener (TCP + UDP) forward+reverse e2e
//! (udp-service-support US-05; ADR-0060; K4).
//!
//! **RED scaffolds** (`#[should_panic(expected = "RED scaffold")]`).
//! GREEN: DELIVER follows the `reverse_nat_e2e` Tier-3 shape with a
//! two-listener service through the real CLI→control-plane→reconciler→
//! EbpfDataplane chain, drops `#[should_panic]`, and asserts both
//! protocols' captures show the VIP source.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-05-A: `overdrive deploy edge.toml` (tcp/8080 + udp/8081) -> the
//!   hydrator emits one update_service per listener; two captures (tcp,
//!   udp) both show the VIP source.
//! - S-05-B: each listener's reverse path is independently VIP-sourced.
//! - S-05-C: re-submitting with an added udp/8082 listener installs the
//!   new path and preserves the existing two.
//!
//! US-05 depends on US-01 + US-02 + US-04; it is the lowest-urgency
//! slice (single-listener UDP — US-04 — already delivers the core value).
//!
//! Assertion discipline: observable kernel side-effects only (two wire
//! captures' source addresses; the per-listener REVERSE_NAT entries).
//! Tier 3 (layer 4+) — example-only per Mandate 11. Gated behind
//! `integration-tests`; runs via `cargo xtask lima run --`. Linux-only.

// S-05-A — a two-listener service installs both protocols' paths.
#[test]
#[should_panic(expected = "RED scaffold")]
fn two_listener_service_installs_both_protocol_paths() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05-A / deploy edge.toml tcp/8080 + \
         udp/8081: hydrator emits one update_service per listener; both tcp and udp \
         captures show the VIP source)"
    );
}

// S-05-B — each listener's reverse path is independently VIP-sourced.
#[test]
#[should_panic(expected = "RED scaffold")]
fn each_listener_reverse_path_independently_vip_sourced() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05-B / both the tcp reply and the \
         udp reply are captured with the VIP as source)"
    );
}

// S-05-C — adding a listener on re-submit converges without breaking
// existing paths.
#[test]
#[should_panic(expected = "RED scaffold")]
fn adding_listener_on_resubmit_converges_without_breaking_existing() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05-C / re-submit edge.toml with a \
         third udp/8082 listener: new path works, existing two still work)"
    );
}
