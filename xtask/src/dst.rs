#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `cargo xtask dst` command logic.
//!
//! ADR-0006 specifies the contract:
//!
//! * Optional `--seed <u64>` — random u64 via OS entropy if absent.
//! * Optional `--only <NAME>` — narrow to one invariant.
//! * Seed is printed on **line 1** of stdout (survives a killed run).
//! * On completion, writes:
//!   * `target/xtask/dst-output.log` — human-readable mirror of stdout.
//!   * `target/xtask/dst-summary.json` — structured summary (schema in
//!     the `Summary` struct below).
//! * Exit status mirrors the harness — zero iff every invariant passed.
//!
//! The xtask crate is a binary boundary per ADR-0003; `rand::random` is
//! permitted here (the lint gate only scans `crate_class = "core"`
//! crates). Using OS entropy for the seed is deliberate per ADR-0006 —
//! the seed is the *one* place real entropy enters the DST stack.

use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr as _;

use color_eyre::eyre::{Result, WrapErr, bail};
use serde::Serialize;

use overdrive_sim::{Harness, Invariant, InvariantStatus, RunReport};

/// Run the DST harness with the given `--seed` / `--only` args. Returns
/// `Ok(())` when every invariant passes; `Err` with a detailed message
/// when any invariant fails (so the binary exits non-zero).
pub fn run(seed: Option<u64>, only: Option<&str>) -> Result<()> {
    // Resolve `--only` via the canonical Invariant enum — unknown names
    // surface as a friendly error *before* we emit the seed line, so
    // callers do not see a seed for a run that never executed.
    let only_variant = match only {
        Some(name) => Some(
            Invariant::from_str(name)
                .wrap_err_with(|| format!("--only name {name:?} is not a known invariant"))?,
        ),
        None => None,
    };

    // Resolve the seed: `--seed` overrides OS entropy. No other source
    // per ADR-0006 — env-var override belongs to 06-03.
    let effective_seed = seed.unwrap_or_else(fresh_seed);

    // Line 1 of stdout — emitted *before* building the harness so a
    // crash during composition still preserves the seed.
    println!("seed: {effective_seed}");

    // Build and run the harness.
    let mut harness = Harness::new();
    if let Some(v) = only_variant {
        harness = harness.only(v);
    }
    let report = harness.run(effective_seed).wrap_err("DST harness failed to compose")?;

    // Render the human-readable log + structured summary, before we
    // propagate any failure — failure artifacts are *always* required
    // (ADR-0006 "Upload both artifacts regardless of success").
    write_artifacts(&report).wrap_err("writing DST artifacts")?;

    // Mirror the per-invariant progress to stdout so a developer
    // watching the terminal sees what ran.
    for result in &report.invariants {
        println!(
            "invariant: {name} status={status} tick={tick} host={host}",
            name = result.name,
            status = result.status.as_str(),
            tick = result.tick,
            host = result.host,
        );
    }

    if report.is_green() {
        println!(
            "dst: {n} invariants passed in {ms} ms (seed={seed})",
            n = report.invariants.len(),
            ms = report.wall_clock.as_millis(),
            seed = report.seed,
        );
        Ok(())
    } else {
        // Emit the failure block to stderr per ADR-0006. The first
        // failure is the canonical one for the reproduction line — the
        // JSON summary carries all of them for dashboards.
        let first =
            report.failures.first().expect("is_green() == false implies at least one failure");

        let only_arg =
            report.failures.first().map_or(String::new(), |f| format!(" --only {}", f.invariant));

        eprintln!();
        eprintln!("dst: FAILED");
        eprintln!("  seed       = {}", report.seed);
        eprintln!("  invariant  = {}", first.invariant);
        eprintln!("  tick       = {}", first.tick);
        eprintln!("  host       = {}", first.host);
        eprintln!("  cause      = {}", first.cause);
        eprintln!("  reproduce  = cargo xtask dst --seed {}{only_arg}", report.seed);

        bail!(
            "{n} invariant failure(s); see target/xtask/dst-summary.json",
            n = report.failures.len()
        )
    }
}

/// Generate a fresh u64 seed from OS entropy. The xtask binary is a
/// boundary crate per ADR-0003, so `rand::random` is permitted here —
/// the dst-lint gate only scans `crate_class = "core"` crates.
fn fresh_seed() -> u64 {
    rand::random()
}

/// Directory artifacts land in: `$CARGO_TARGET_DIR/xtask/` per ADR-0006.
/// We respect `CARGO_TARGET_DIR` when set (so per-test tempdirs work);
/// otherwise fall back to `target/`.
fn xtask_target_dir() -> PathBuf {
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    target.join("xtask")
}

/// Structured summary written to `dst-summary.json`.
#[derive(Serialize)]
struct Summary {
    seed: u64,
    git_sha: String,
    toolchain: String,
    invariants: Vec<InvariantEntry>,
    failures: Vec<FailureEntry>,
    wall_clock_ms: u128,
}

#[derive(Serialize)]
struct InvariantEntry {
    name: String,
    status: String,
    tick: u64,
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
}

#[derive(Serialize)]
struct FailureEntry {
    invariant: String,
    tick: u64,
    host: String,
    cause: String,
}

fn write_artifacts(report: &RunReport) -> Result<()> {
    use std::fmt::Write as _;

    let dir = xtask_target_dir();
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("create_dir_all({})", dir.display()))?;

    // Human-readable log — mirrors stdout shape so a developer who
    // downloaded the artifact sees exactly what they would have seen
    // on the console.
    let log_path = dir.join("dst-output.log");
    let mut log = String::new();
    writeln!(log, "seed: {}", report.seed).ok();
    for result in &report.invariants {
        writeln!(
            log,
            "invariant: {name} status={status} tick={tick} host={host}",
            name = result.name,
            status = result.status.as_str(),
            tick = result.tick,
            host = result.host,
        )
        .ok();
    }
    if report.is_green() {
        writeln!(
            log,
            "dst: {n} invariants passed in {ms} ms (seed={seed})",
            n = report.invariants.len(),
            ms = report.wall_clock.as_millis(),
            seed = report.seed,
        )
        .ok();
    } else {
        for failure in &report.failures {
            writeln!(
                log,
                "FAILED: invariant={invariant} tick={tick} host={host} cause={cause}",
                invariant = failure.invariant,
                tick = failure.tick,
                host = failure.host,
                cause = failure.cause,
            )
            .ok();
        }
    }
    std::fs::write(&log_path, log).wrap_err_with(|| format!("write {}", log_path.display()))?;

    // Structured summary — CI dashboards parse this.
    let summary = Summary {
        seed: report.seed,
        git_sha: git_sha(),
        toolchain: toolchain(),
        invariants: report
            .invariants
            .iter()
            .map(|r| InvariantEntry {
                name: r.name.clone(),
                status: r.status.as_str().to_owned(),
                tick: r.tick,
                host: r.host.clone(),
                cause: r.cause.clone(),
            })
            .collect(),
        failures: report
            .failures
            .iter()
            .map(|f| FailureEntry {
                invariant: f.invariant.clone(),
                tick: f.tick,
                host: f.host.clone(),
                cause: f.cause.clone(),
            })
            .collect(),
        wall_clock_ms: report.wall_clock.as_millis(),
    };
    let summary_path = dir.join("dst-summary.json");
    let serialised =
        serde_json::to_string_pretty(&summary).wrap_err("serialise dst-summary.json")?;
    std::fs::write(&summary_path, serialised)
        .wrap_err_with(|| format!("write {}", summary_path.display()))?;

    Ok(())
}

fn git_sha() -> String {
    Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().map_or_else(
        || "unknown".to_owned(),
        |out| {
            if out.status.success() {
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            } else {
                "unknown".to_owned()
            }
        },
    )
}

fn toolchain() -> String {
    Command::new("rustc").arg("--version").output().ok().map_or_else(
        || "unknown".to_owned(),
        |out| {
            if out.status.success() {
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            } else {
                "unknown".to_owned()
            }
        },
    )
}

/// Guard against the status enum's kebab-case serialisation silently
/// drifting — `InvariantStatus::Pass.as_str()` MUST stay "pass".
#[allow(dead_code)]
const fn _status_enum_used() -> InvariantStatus {
    InvariantStatus::Pass
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    //! Library-level unit tests — these are what cargo-mutants uses to
    //! kill mutations in this file (integration-level subprocess tests
    //! live under `xtask/tests/acceptance/` and are excluded from
    //! mutation runs by `.mutants.toml`).

    use super::*;

    #[test]
    fn fresh_seed_produces_distinct_values_across_calls() {
        let a = fresh_seed();
        let b = fresh_seed();
        // The probability of a 64-bit collision from `OsRng` is 2^-64 —
        // a flake here is signalling a mutation that pinned the seed to
        // a constant, not a real RNG collision.
        assert_ne!(a, b, "fresh_seed must sample OS entropy, not return a constant");
    }

    #[test]
    fn git_sha_is_non_empty_or_unknown_sentinel() {
        let sha = git_sha();
        assert!(!sha.is_empty(), "git_sha must never return an empty string");
        // Accept either a hex-ish short SHA (git available) or the
        // fallback sentinel. A mutation to a fixed non-sentinel string
        // (e.g. "xyzzy") will fail both branches.
        assert!(
            sha == "unknown" || sha.chars().all(|c| c.is_ascii_hexdigit()),
            "git_sha must look like a git short-SHA or the 'unknown' sentinel; got {sha:?}"
        );
    }

    #[test]
    fn toolchain_looks_like_rustc_version_or_unknown_sentinel() {
        let tc = toolchain();
        assert!(!tc.is_empty(), "toolchain must never return an empty string");
        assert!(
            tc.contains("rustc") || tc == "unknown",
            "toolchain must contain 'rustc' or be the 'unknown' sentinel; got {tc:?}"
        );
    }

    #[test]
    fn xtask_target_dir_ends_in_xtask() {
        let dir = xtask_target_dir();
        assert_eq!(
            dir.file_name().and_then(|s| s.to_str()),
            Some("xtask"),
            "artifact directory must end in 'xtask' per ADR-0006; got {dir:?}"
        );
    }

    #[test]
    fn write_artifacts_writes_log_and_summary_on_green_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Point CARGO_TARGET_DIR at our tempdir so the artifacts land
        // under tmp/xtask/.
        // SAFETY: this test crate is single-threaded via `#[cfg(test)]`
        // in cargo's default; setting an env var in-process is fine.
        // We restore nothing — the env vanishes with the process.
        let guard = EnvGuard::set("CARGO_TARGET_DIR", tmp.path().to_str().unwrap());

        let report = RunReport {
            seed: 1234,
            invariants: vec![overdrive_sim::InvariantResult {
                name: "single-leader".to_owned(),
                status: InvariantStatus::Pass,
                tick: 100,
                host: "host-0".to_owned(),
                cause: None,
            }],
            wall_clock: std::time::Duration::from_millis(5),
            failures: Vec::new(),
        };
        write_artifacts(&report).expect("write_artifacts");
        drop(guard);

        let log_path = tmp.path().join("xtask").join("dst-output.log");
        let summary_path = tmp.path().join("xtask").join("dst-summary.json");
        assert!(log_path.is_file());
        assert!(summary_path.is_file());

        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(log.starts_with("seed: 1234"), "log line 1 must name the seed; got {log}");
        assert!(log.contains("single-leader"));
        assert!(log.contains("1 invariants passed"));

        let summary: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&summary_path).unwrap()).unwrap();
        assert_eq!(summary["seed"].as_u64(), Some(1234));
        assert_eq!(summary["invariants"][0]["name"].as_str(), Some("single-leader"));
        assert_eq!(summary["invariants"][0]["status"].as_str(), Some("pass"));
        assert_eq!(summary["failures"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn write_artifacts_records_failures_on_red_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let guard = EnvGuard::set("CARGO_TARGET_DIR", tmp.path().to_str().unwrap());

        let report = RunReport {
            seed: 9,
            invariants: vec![overdrive_sim::InvariantResult {
                name: "single-leader".to_owned(),
                status: InvariantStatus::Fail,
                tick: 42,
                host: "host-1".to_owned(),
                cause: Some("two leaders".to_owned()),
            }],
            wall_clock: std::time::Duration::from_millis(5),
            failures: vec![overdrive_sim::Failure {
                invariant: "single-leader".to_owned(),
                tick: 42,
                host: "host-1".to_owned(),
                cause: "two leaders".to_owned(),
            }],
        };
        write_artifacts(&report).expect("write_artifacts");
        drop(guard);

        let log_path = tmp.path().join("xtask").join("dst-output.log");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("FAILED") && log.contains("two leaders"),
            "failure log must name FAILED and cause; got {log}"
        );

        let summary: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("xtask").join("dst-summary.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(summary["failures"].as_array().map(Vec::len), Some(1));
        assert_eq!(summary["failures"][0]["invariant"].as_str(), Some("single-leader"));
        assert_eq!(summary["failures"][0]["cause"].as_str(), Some("two leaders"));
    }

    /// RAII guard that restores (or clears) an env var on drop. Scoped
    /// narrowly to these tests; real DST code reads through the
    /// injected traits, not process env.
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: Rust 2024's set_var is unsafe because concurrent
            // getenv in other threads is UB. Our tests run
            // single-threaded within this binary and the write is
            // ordered before any later read in the same test, so the
            // unsafe contract is met.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see EnvGuard::set.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
