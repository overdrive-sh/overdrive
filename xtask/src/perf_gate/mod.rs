//! Tier 4 perf-regression gate — `xdp-perf`.
//!
//! Wraps `xdp-bench`, compares against baselines under
//! `perf-baseline/main/xdp-perf-<mode>.txt`, fails the build on
//! regression. Splits into a pure decision fn (`evaluate`) that takes
//! baselines + candidates + policy and returns a structured outcome,
//! plus a shell-side wrapper in `xtask/src/main.rs` that handles the
//! I/O.
//!
//! The pure fn lives here so `xtask/tests/perf_gate_self_test.rs` can
//! call it directly without `xdp-bench` installed — running on macOS
//! without Lima.
//!
//! # See also
//!
//! The companion `verifier-regress` gate lives in
//! `crates/overdrive-dataplane` and is invoked via `cargo
//! verifier-regress`. It moved out of xtask because it must load the
//! BPF object via aya (which xtask cannot depend on per the xtask-
//! purity rule); this gate stays in xtask because `xdp-bench` is a
//! pure subprocess.

pub mod xdp_perf;
pub mod xdp_perf_setup;
