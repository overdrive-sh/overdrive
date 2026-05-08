//! Integration-test entrypoint for the `overdrive-bpf` crate.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating" —
//! integration tests live under `tests/integration/<scenario>.rs` and
//! are wired through this single entrypoint. The whole binary is gated
//! behind the `integration-tests` feature; per-scenario modules inherit
//! the gate without repeating the cfg attribute.
//!
//! Submodules MUST be declared inside an inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory.
//!
//! These tests are Tier 2 BPF unit tests per `.claude/rules/testing.md`
//! § "Tier 2 — BPF Unit Tests". Each test loads the BPF object from
//! `target/bpf/overdrive_bpf.o` (produced by
//! `cargo xtask bpf-build`), drives `BPF_PROG_TEST_RUN` via aya's
//! userspace API, and asserts on the returned verdict and observable
//! BPF map state.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

mod integration {
    // Shared helpers; consumed by sibling scenario modules below.
    mod bpf_artifact;

    mod xdp_pass_test_run;
    // phase-2-xdp-service-map DISTILL — RED scaffolds per
    // `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
    // S-2.2-04, S-2.2-05, S-2.2-08, S-2.2-16, S-2.2-17, S-2.2-19,
    // S-2.2-20, S-2.2-21. Bodies panic until DELIVER fills them
    // per the carpaccio slice plan.
    mod sanity_prologue_drops;
    mod xdp_service_map_lookup;
    // Slice 09 (ADR-0045 — bpf_redirect_neigh datapath pivot).
    // Step 09-01: forward-path FIB+L2-rewrite+redirect_neigh contract.
    mod xdp_service_map_redirect_neigh;
    // Step 09-02: reverse-path xdp_reverse_nat_lookup at backend-veth
    // ingress (REVERSE_NAT lookup + L3+L4 rewrite + FIB + L2 +
    // bpf_redirect).
    mod xdp_reverse_nat_redirect_neigh;
}
