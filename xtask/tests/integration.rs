//! Integration-test entrypoint for the `xtask` crate.
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

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

mod integration {
    mod crate_class_metadata;
    mod dev_setup_bpf_linker;
    /// phase-2-xdp-service-map Slice 07 (US-07; S-2.2-24, S-2.2-25)
    /// — perf-gate self-test. Synthetic input proves the gate logic
    /// itself returns non-zero on >5% regression. RED scaffolds.
    mod perf_gate_self_test;
}
