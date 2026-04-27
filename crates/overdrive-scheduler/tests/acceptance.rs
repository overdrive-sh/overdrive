//! Acceptance-test entrypoint for `overdrive-scheduler`.
//!
//! Per ADR-0005 and `.claude/rules/testing.md`, default-lane acceptance
//! tests live under `tests/acceptance/<scenario>.rs` and are wired
//! into a single Cargo integration-test binary by this entrypoint.
//!
//! These tests run on `cargo nextest run -p overdrive-scheduler` —
//! NO `--features integration-tests` flag required. The scheduler is
//! a pure function; its tests are pure-Rust and stay in the default
//! lane.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

// SCAFFOLD: true — phase-1-first-workload DISTILL. The per-scenario
// modules below all panic with the RED-scaffold message until DELIVER
// implements the body.
mod acceptance {
    mod first_fit_happy_path;
    mod determinism;
    mod capacity_accounting;
    mod empty_node_set;
}
