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

mod acceptance {
    // Shared generators / fixtures used across the per-scenario modules.
    // Per the RED→GREEN crafter discipline, the scenarios share a single
    // proptest strategy file rather than duplicating valid_label / arb_job
    // boilerplate (the same pattern overdrive-core uses in
    // tests/acceptance/aggregate_roundtrip.rs).
    mod common;

    mod capacity_accounting;
    mod determinism;
    mod empty_node_set;
    mod first_fit_happy_path;
    mod free_capacity_strict_inequality;
}
