//! S-2.2-24, S-2.2-25 — `cargo xtask verifier-regress` /
//! `cargo xtask xdp-perf` self-tests.
//!
//! Tags: `@US-07` `@K7` `@slice-07` `@kpi` `@pending`.
//! Tier: Tier 4 (xtask self-test — proves the gate logic itself).
//!
//! Per `outcome-kpis.md` K7 measurement plan: the xtask subcommand
//! must return non-zero on a synthetic >5% regression input and
//! zero on a synthetic 2% input. This self-test pair locks both
//! shapes.

#![allow(clippy::missing_panics_doc)]

/// S-2.2-24 — `verifier-regress` returns non-zero on synthetic
/// 12 % growth.
#[test]
#[ignore = "RED scaffold S-2.2-24 — DELIVER fills the body per Slice 07"]
fn verifier_regress_returns_nonzero_on_synthetic_twelve_percent_growth() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-24 — \
         synthetic baseline 5000; candidate 5600 (12% growth); \
         cargo xtask verifier-regress returns non-zero exit code; \
         output names program, both counts, threshold"
    );
}

/// S-2.2-25 — `xdp-perf` returns non-zero on synthetic 6 % pps
/// regression.
#[test]
#[ignore = "RED scaffold S-2.2-25 — DELIVER fills the body per Slice 07"]
fn xdp_perf_returns_nonzero_on_synthetic_six_percent_pps_regression() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-25 — \
         synthetic baseline 5.0 Mpps; candidate 4.7 Mpps (6% regression); \
         cargo xtask xdp-perf returns non-zero exit code; \
         output reports both numbers and 5% relative-delta threshold"
    );
}
