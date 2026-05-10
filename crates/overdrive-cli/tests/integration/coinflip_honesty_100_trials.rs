//! S-02-09 — KPI K1 Tier-3 honesty test.
//!
//! Per slice 02 of `workload-kind-discriminator`: 100 trials of
//! `overdrive job submit examples/coinflip.toml` against the real
//! `ExecDriver` in Lima must show ≥99 trials where the CLI process
//! exit code equals the workload's kernel-observed exit code AND every
//! trial's terminal verdict line names the same exit code as the
//! kernel observed.
//!
//! Linux-gated; per CLAUDE.md, the runner must route through Lima.
//!
//! NOTE: This test is currently scaffolded as `#[ignore]` pending the
//! full Job-kind streaming dispatch through the reconciler typed
//! `TerminalCondition::Completed { exit_code }` / `Failed { exit_code }`
//! emission path (reconciler ADR-0037 Amendment 2026-05-10), which
//! lands across multiple sub-slices of step 02-01.
//!
//! When the reconciler typed terminals land, drop `#[ignore]` and run
//! via:
//!
//! ```text
//! cargo xtask lima run -- cargo nextest run -p overdrive-cli \
//!   --features integration-tests -E 'test(coinflip_honesty)'
//! ```

#![cfg(target_os = "linux")]

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "S-02-09: K1 honesty 100-trial Lima/ExecDriver test pending typed Completed/Failed terminals \
            (ADR-0037 Amendment 2026-05-10)"]
async fn s_02_09_k1_honesty_100_trials() {
    // Scaffold — see file rustdoc.
}
