//! Tier 3 — single-UDP-listener forward+reverse e2e (WALKING SKELETON)
//! (udp-service-support US-04; ADR-0060 § Enforcement Tier 3; K1).
//!
//! **RED scaffolds** (`#[should_panic(expected = "RED scaffold")]`).
//! GREEN: DELIVER follows the `reverse_nat_e2e` / `service_map_forward`
//! Tier-3 shape (real `EbpfDataplane` + `overdrive-testing`
//! `ThreeIfaceTopology` netns/veth fixtures), drops `#[should_panic]`,
//! and asserts on the observable kernel side-effects.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-04-A (WALKING SKELETON / driving adapter): `overdrive deploy
//!   dns-resolver.toml` (real subprocess; CLI verb `Deploy`, NOT `job
//!   submit`) -> exit 0 + "Accepted." stdout + `bpftool map dump
//!   REVERSE_NAT_MAP` shows (backend_ip,5353,udp)->vip + a wire capture
//!   on the client veth shows the reply sourced from the VIP.
//! - S-04-B: three datagrams each independently source-rewritten to VIP.
//! - S-04-C: a missing-backend response (no reply) is distinguished from
//!   a wrong-source response (reply with backend source) — only the
//!   latter is the #163 defect.
//!
//! Assertion discipline (`.claude/rules/testing.md` § "Assertion rules"):
//! assert on OBSERVABLE kernel side-effects (the REVERSE_NAT_MAP dump,
//! the AF_PACKET/tcpdump wire capture source address) — NEVER on "the
//! program took branch X". Tier 3 (layer 4+) — example-only per
//! Mandate 11; traditional assertions per Mandate 8.
//!
//! Gated behind `integration-tests`; runs via `cargo xtask lima run --`.
//! Linux-only (real veth + bpffs + kernel). The whole `tests/integration`
//! binary is gated in `tests/integration.rs`.
//!
//! NOTE on the driving-adapter (subprocess `overdrive deploy`) half:
//! this dataplane-crate test owns the WIRE half (REVERSE_NAT_MAP + wire
//! capture). The subprocess `overdrive deploy` exit-code + "Accepted."
//! assertion lands alongside the existing `exec_spec_walking_skeleton`
//! precedent in `overdrive-control-plane` / `overdrive-cli` integration
//! tests; S-04-A's driving-adapter clause is satisfied by that companion
//! subprocess test. See feature-delta DISTILL [REF] § Driving adapter
//! coverage.

// S-04-A (WALKING SKELETON) — real UDP round-trip carries the VIP source.
#[test]
#[should_panic(expected = "RED scaffold")]
fn single_udp_listener_round_trip_carries_vip_source() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04-A / walking skeleton: deploy a \
         single-udp-listener service; REVERSE_NAT_MAP shows (ip,5353,udp)->vip and \
         the wire capture shows the reply sourced from the VIP)"
    );
}

// S-04-B — every UDP reply is independently source-rewritten to the VIP.
#[test]
#[should_panic(expected = "RED scaffold")]
fn every_udp_reply_independently_source_rewritten() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04-B / three datagrams: all three \
         replies captured with the VIP as source)"
    );
}

// S-04-C — boundary: a missing-backend response (no reply) is NOT
// misreported as a source-rewrite failure.
#[test]
#[should_panic(expected = "RED scaffold")]
fn missing_backend_response_distinguished_from_wrong_source() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04-C / backend not bound: no reply \
         captured; test reports 'no response', NOT a source-rewrite failure)"
    );
}
