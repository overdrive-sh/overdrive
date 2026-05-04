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
//! `target/xtask/bpf-objects/overdrive_bpf.o` (produced by
//! `cargo xtask bpf-build`), drives `BPF_PROG_TEST_RUN` via aya's
//! userspace API, and asserts on the returned verdict and observable
//! BPF map state.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

mod integration {
    mod xdp_pass_test_run;
}
