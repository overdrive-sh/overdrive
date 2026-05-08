//! Tier 4 perf-regression gates.
//!
//! Two gates land here, sharing the same parse → evaluate → render
//! shape:
//!
//! - [`verifier_regress`] — `cargo xtask verifier-regress` body. Runs
//!   `veristat` against compiled BPF programs, parses the output,
//!   compares against per-program baselines under
//!   `perf-baseline/main/verifier-budget/`, fails the build on
//!   regression. Wired at step 07-01.
//! - `xdp_perf` — `cargo xtask xdp-perf` body. Runs `xdp-bench`,
//!   compares against baselines under `perf-baseline/main/xdp-perf/`.
//!   Wired at step 07-02 (NOT this step).
//!
//! Each submodule splits into a *pure decision fn* (`evaluate`) that
//! takes baselines + candidates + policy and returns a structured
//! outcome, plus a *shell-side wrapper* in `xtask/src/main.rs` that
//! handles the I/O (find files, run subprocess, render).
//!
//! The pure fn lives here so `xtask/tests/perf_gate_self_test.rs` can
//! call it directly without veristat installed — running on macOS
//! without Lima.

pub mod verifier_regress;
pub mod xdp_perf;
pub mod xdp_perf_setup;
