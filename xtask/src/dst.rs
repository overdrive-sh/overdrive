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
