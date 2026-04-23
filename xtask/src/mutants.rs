#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `cargo xtask mutants` command logic.
//!
//! Two modes, both gated on kill-rate per `.claude/rules/testing.md`
//! §"Mutation testing (cargo-mutants)":
//!
//! * `--diff <base>` — per-PR, diff-scoped. Gate: kill rate ≥ 80%.
//! * `--workspace` — nightly, full corpus. Gate: kill rate ≥ 60%
//!   (absolute floor, hard fail); drift ≤ -2 pp vs.
//!   `mutants-baseline/main/kill_rate.txt` is a soft-warn
//!   (annotation, not exit-1).
//!
//! Both modes:
//!
//! * Write `target/xtask/mutants-summary.json` — structured summary the
//!   CI workflow parses for `GITHUB_STEP_SUMMARY` annotations (same
//!   shape idea as `dst-summary.json`, ADR-0006).
//! * Exit status mirrors the gate — zero iff the gate passed.
//!
//! The xtask crate is a binary boundary per ADR-0003; subprocess I/O,
//! `rand`, and wall-clock reads are permitted here (the dst-lint gate
//! only scans `crate_class = "core"` crates).
//!
//! # Kill-rate definition
//!
//! Kill rate is `caught / (caught + missed)` — the denominator excludes
//! `Unviable` (rustc rejected the mutated source) and `Timeout` (test
//! run exceeded wall-clock budget). This matches cargo-mutants' own
//! recommended interpretation and the intent of the rules doc
//! ("Missed mutations are reviewed per-PR"). Unviable and timeout
//! counters are still reported in the summary so regressions in either
//! surface independently — they are not freebies, per testing.md, but
//! they are also not test-quality failures.
//!
//! This deviates from the previous inline bash + Python gate, which
//! computed `caught / len(outcomes)` over a field (`outcome`) that
//! cargo-mutants does not emit — the real field name is `summary`. The
//! old gate was silently dividing by the wrong denominator against a
//! counter that was always zero. The new math is the one the rules
//! actually describe.

use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr, bail};
use serde::{Deserialize, Serialize};

/// Invocation mode for `cargo xtask mutants`.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Per-PR diff-scoped run. `base` is a git ref (e.g. "origin/main")
    /// that we materialise to a diff file before invoking cargo-mutants
    /// (cargo-mutants' `--in-diff` takes a *file path*, not a git ref —
    /// see <https://mutants.rs/in-diff.html>).
    Diff { base: String },
    /// Full-workspace run. Compared against a stored baseline path.
    Workspace { baseline_path: PathBuf },
}

/// Hard gate thresholds (percentage of mutations caught).
///
/// Kept as associated constants so a test can assert them and so a
/// code review that changes the number shows up as a diff on the
/// constant, not hidden inside a comparison.
pub const DIFF_KILL_RATE_FLOOR: f64 = 80.0;
pub const WORKSPACE_ABSOLUTE_FLOOR: f64 = 60.0;
pub const WORKSPACE_DRIFT_WARN_PP: f64 = -2.0;

/// Entry point called from `main.rs`.
pub fn run(mode: &Mode) -> Result<()> {
    which_cargo_mutants()?;

    let out_dir = xtask_target_dir().join("mutants.out");
    let summary_path = xtask_target_dir().join("mutants-summary.json");
    std::fs::create_dir_all(xtask_target_dir())
        .wrap_err_with(|| format!("create_dir_all({})", xtask_target_dir().display()))?;

    // 1. Run cargo-mutants. Exit status is intentionally ignored — a
    //    non-zero exit from cargo-mutants happens on any missed mutant,
    //    which we handle via our own gate below. We only care that the
    //    subprocess produced `outcomes.json`.
    let _ = invoke_cargo_mutants(mode, &out_dir)?;

    // 2. Parse outcomes.json.
    let outcomes_path = out_dir.join("outcomes.json");
    let report = parse_outcomes(&outcomes_path).wrap_err_with(|| {
        format!(
            "parse cargo-mutants outcomes at {} — cargo-mutants may have crashed \
             before writing its report",
            outcomes_path.display()
        )
    })?;

    // 3. Evaluate the mode-specific gate.
    let gate = match mode {
        Mode::Diff { .. } => evaluate_diff_gate(&report),
        Mode::Workspace { baseline_path } => evaluate_workspace_gate(&report, baseline_path)?,
    };

    // 4. Write the structured summary and human-readable lines.
    write_summary(&summary_path, &report, &gate, mode)
        .wrap_err_with(|| format!("write {}", summary_path.display()))?;
    print_report(&report, &gate);

    // 5. Exit status mirrors the gate. `Pass` and `Warn` both exit zero —
    //    a `Warn` only emits an annotation (e.g. nightly drift) and does
    //    not block the run.
    match &gate.status {
        GateStatus::Pass | GateStatus::Warn { .. } => Ok(()),
        GateStatus::Fail { reason } => bail!("{reason}"),
    }
}

fn which_cargo_mutants() -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg("command -v cargo-mutants")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !found {
        bail!(
            "`cargo-mutants` not found on PATH. Install it with:\n\
             cargo install cargo-mutants\n\
             (or, on CI, via `taiki-e/install-action` with `tool: cargo-mutants`)"
        );
    }
    Ok(())
}

/// Run `cargo mutants` with the flags appropriate to `mode`. The diff
/// file (if any) is written under the xtask target dir.
fn invoke_cargo_mutants(mode: &Mode, out_dir: &Path) -> Result<std::process::ExitStatus> {
    let mut cmd = Command::new(cargo());
    cmd.arg("mutants").arg("--output").arg(out_dir);

    match mode {
        Mode::Diff { base } => {
            let diff_path = xtask_target_dir().join("mutants.diff");
            let diff_bytes = git_diff_against(base).wrap_err_with(|| {
                format!("git diff {base} failed — is `{base}` fetched in this clone?")
            })?;
            std::fs::write(&diff_path, &diff_bytes)
                .wrap_err_with(|| format!("write {}", diff_path.display()))?;
            eprintln!(
                "xtask mutants: wrote {} ({} bytes) for --in-diff",
                diff_path.display(),
                diff_bytes.len()
            );
            cmd.arg("--in-diff").arg(&diff_path);
        }
        Mode::Workspace { .. } => {
            cmd.arg("--workspace");
        }
    }

    eprintln!("xtask mutants: running {}", format_cmd(&cmd));
    let status = cmd.status().wrap_err("spawn cargo-mutants")?;
    // Don't bail on non-zero — cargo-mutants returns non-zero for any
    // missed mutant, which is exactly the signal we want to measure
    // ourselves. We only fail if `outcomes.json` was not written (the
    // parse step surfaces that).
    Ok(status)
}

fn git_diff_against(base: &str) -> Result<Vec<u8>> {
    let out = Command::new("git").args(["diff", base]).output().wrap_err("spawn git diff")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        bail!("git diff {base} exited non-zero: {stderr}");
    }
    Ok(out.stdout)
}

// ---------------------------------------------------------------------------
// outcomes.json parsing — raw shape emitted by cargo-mutants.
//
// Only fields we actually use are deserialized. `#[serde(default)]` on
// the counters is deliberate: a baseline-only run (no mutants in diff)
// may omit them.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RawReport {
    #[serde(default)]
    total_mutants: u64,
    #[serde(default)]
    caught: u64,
    #[serde(default)]
    missed: u64,
    #[serde(default)]
    timeout: u64,
    #[serde(default)]
    unviable: u64,
    #[serde(default)]
    success: u64,
    #[serde(default)]
    cargo_mutants_version: String,
}

fn parse_outcomes(path: &Path) -> Result<RawReport> {
    if !path.is_file() {
        bail!("no outcomes.json at {} — cargo-mutants did not produce a report", path.display());
    }
    let raw = std::fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
    let report: RawReport =
        serde_json::from_str(&raw).wrap_err_with(|| format!("parse {}", path.display()))?;
    Ok(report)
}

/// Kill rate (caught / (caught + missed)) in percent. Returns 100.0 if
/// there are no mutations to evaluate (vacuously passing) — the caller
/// decides whether that counts as "pass" or "nothing to do."
fn kill_rate_percent(caught: u64, missed: u64) -> f64 {
    let denom = caught.saturating_add(missed);
    if denom == 0 {
        100.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        {
            (caught as f64 / denom as f64) * 100.0
        }
    }
}

// ---------------------------------------------------------------------------
// Gate evaluation.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Gate {
    mode_label: String,
    kill_rate_pct: f64,
    baseline_pct: Option<f64>,
    drift_pp: Option<f64>,
    status: GateStatus,
}

#[derive(Debug, Clone)]
enum GateStatus {
    /// Gate passed with no notes.
    Pass,
    /// Gate passed but with a warning (e.g. workspace drift annotation).
    Warn { reason: String },
    /// Gate failed. Non-zero exit.
    Fail { reason: String },
}

fn evaluate_diff_gate(report: &RawReport) -> Gate {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);
    let status = if report.caught == 0 && report.missed == 0 {
        // No gate-relevant mutations generated from the diff — e.g. the
        // PR only touched excluded paths or comments. Vacuously pass.
        GateStatus::Pass
    } else if kill_rate_pct + f64::EPSILON < DIFF_KILL_RATE_FLOOR {
        GateStatus::Fail {
            reason: format!(
                "mutants kill rate {kill_rate_pct:.1}% < {DIFF_KILL_RATE_FLOOR:.1}% threshold \
                 (caught={caught} missed={missed})",
                caught = report.caught,
                missed = report.missed,
            ),
        }
    } else {
        GateStatus::Pass
    };

    Gate {
        mode_label: "diff".to_owned(),
        kill_rate_pct,
        baseline_pct: None,
        drift_pp: None,
        status,
    }
}

fn evaluate_workspace_gate(report: &RawReport, baseline_path: &Path) -> Result<Gate> {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);

    // Read or seed the baseline.
    let (baseline_pct, drift_pp, status) = if baseline_path.is_file() {
        let raw = std::fs::read_to_string(baseline_path)
            .wrap_err_with(|| format!("read {}", baseline_path.display()))?;
        let baseline: f64 = raw.trim().parse().wrap_err_with(|| {
            format!(
                "parse baseline kill rate from {} — expected a single float \
                     (e.g. `75.0`), got {raw:?}",
                baseline_path.display()
            )
        })?;
        let drift = kill_rate_pct - baseline;

        let status = if kill_rate_pct + f64::EPSILON < WORKSPACE_ABSOLUTE_FLOOR {
            GateStatus::Fail {
                reason: format!(
                    "critical mutants regression: {kill_rate_pct:.1}% < \
                     {WORKSPACE_ABSOLUTE_FLOOR:.1}% absolute floor"
                ),
            }
        } else if drift + f64::EPSILON < WORKSPACE_DRIFT_WARN_PP {
            GateStatus::Warn {
                reason: format!(
                    "mutants drift {drift:+.1}pp below baseline {baseline:.1}% \
                     (current={kill_rate_pct:.1}%)"
                ),
            }
        } else {
            GateStatus::Pass
        };

        (Some(baseline), Some(drift), status)
    } else {
        // No baseline yet — seed it and pass. Matches the prior
        // nightly.yml behaviour: the file is written to the local
        // checkout; the operator commits it as a follow-up PR.
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("create_dir_all({})", parent.display()))?;
        }
        std::fs::write(baseline_path, format!("{kill_rate_pct:.1}\n"))
            .wrap_err_with(|| format!("seed {}", baseline_path.display()))?;
        eprintln!(
            "xtask mutants: no baseline at {}; seeded at {kill_rate_pct:.1}%",
            baseline_path.display()
        );
        (None, None, GateStatus::Pass)
    };

    Ok(Gate { mode_label: "workspace".to_owned(), kill_rate_pct, baseline_pct, drift_pp, status })
}

// ---------------------------------------------------------------------------
// Summary artifact — the shape CI parses via `jq`.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Summary {
    mode: String,
    cargo_mutants_version: String,
    total_mutants: u64,
    caught: u64,
    missed: u64,
    timeout: u64,
    unviable: u64,
    baseline_success: u64,
    kill_rate_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drift_pp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_path: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

fn write_summary(path: &Path, report: &RawReport, gate: &Gate, mode: &Mode) -> Result<()> {
    let (status, reason) = match &gate.status {
        GateStatus::Pass => ("pass", None),
        GateStatus::Warn { reason } => ("warn", Some(reason.clone())),
        GateStatus::Fail { reason } => ("fail", Some(reason.clone())),
    };

    let (base_ref, baseline_path) = match mode {
        Mode::Diff { base } => (Some(base.clone()), None),
        Mode::Workspace { baseline_path } => (None, Some(baseline_path.display().to_string())),
    };

    let summary = Summary {
        mode: gate.mode_label.clone(),
        cargo_mutants_version: report.cargo_mutants_version.clone(),
        total_mutants: report.total_mutants,
        caught: report.caught,
        missed: report.missed,
        timeout: report.timeout,
        unviable: report.unviable,
        baseline_success: report.success,
        kill_rate_pct: round_pct(gate.kill_rate_pct),
        baseline_pct: gate.baseline_pct.map(round_pct),
        drift_pp: gate.drift_pp.map(round_pct),
        base_ref,
        baseline_path,
        status: status.to_owned(),
        reason,
    };

    let serialised =
        serde_json::to_string_pretty(&summary).wrap_err("serialise mutants-summary.json")?;
    std::fs::write(path, serialised)?;
    Ok(())
}

/// Round to one decimal place so the summary stays stable across minor
/// floating-point jitter. Matches `mutants-baseline/main/kill_rate.txt`.
fn round_pct(pct: f64) -> f64 {
    (pct * 10.0).round() / 10.0
}

fn print_report(report: &RawReport, gate: &Gate) {
    println!(
        "mutants: mode={} total={} caught={} missed={} timeout={} unviable={} \
         kill_rate={:.1}%",
        gate.mode_label,
        report.total_mutants,
        report.caught,
        report.missed,
        report.timeout,
        report.unviable,
        gate.kill_rate_pct,
    );
    if let (Some(base), Some(drift)) = (gate.baseline_pct, gate.drift_pp) {
        println!("mutants: baseline={base:.1}% drift={drift:+.1}pp");
    }
    match &gate.status {
        GateStatus::Pass => println!("mutants: PASS"),
        GateStatus::Warn { reason } => {
            println!("mutants: WARN — {reason}");
            // Also emit a GitHub annotation so nightly soft-fails show
            // up in the job summary UI.
            println!("::warning title=mutants drift::{reason}");
        }
        GateStatus::Fail { reason } => {
            eprintln!("mutants: FAIL — {reason}");
            eprintln!("::error title=mutants kill rate::{reason}");
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers (mirror dst.rs shape).
// ---------------------------------------------------------------------------

fn xtask_target_dir() -> PathBuf {
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    target.join("xtask")
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn format_cmd(cmd: &Command) -> String {
    use std::fmt::Write as _;
    let mut out = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        let _ = write!(out, " {}", arg.to_string_lossy());
    }
    out
}

// ---------------------------------------------------------------------------
// Tests.
//
// These cover the gate logic in isolation — the one place bugs can
// silently lower the bar. Subprocess-level integration (actually
// invoking `cargo mutants`) is covered by the CI workflows themselves.
// ---------------------------------------------------------------------------
#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn report(caught: u64, missed: u64, unviable: u64, timeout: u64) -> RawReport {
        RawReport {
            total_mutants: caught + missed + unviable + timeout + 1,
            caught,
            missed,
            timeout,
            unviable,
            success: 1,
            cargo_mutants_version: "27.0.0".to_owned(),
        }
    }

    #[test]
    fn kill_rate_excludes_unviable_and_timeout() {
        // caught + missed is the denominator; unviable/timeout do not
        // dilute the rate. 4/(4+1) = 80%.
        assert!((kill_rate_percent(4, 1) - 80.0).abs() < 1e-9);
    }

    #[test]
    fn kill_rate_is_vacuously_100_when_no_mutations_evaluated() {
        assert!((kill_rate_percent(0, 0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn diff_gate_passes_at_exact_floor() {
        // 8 / (8 + 2) = 80.0% — exactly at the floor. Must pass.
        let gate = evaluate_diff_gate(&report(8, 2, 0, 0));
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn diff_gate_fails_just_below_floor() {
        // 7 / (7 + 2) = 77.7…% — below 80%. Must fail, and the reason
        // must mention the actual numerator/denominator so the
        // developer can find the missed mutations.
        let gate = evaluate_diff_gate(&report(7, 2, 0, 0));
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail, got {other:?}"),
        };
        assert!(reason.contains("77.8%") || reason.contains("77.7%"), "got: {reason}");
        assert!(reason.contains("caught=7"), "got: {reason}");
        assert!(reason.contains("missed=2"), "got: {reason}");
    }

    #[test]
    fn diff_gate_passes_when_diff_generated_no_mutations() {
        // PR touched only excluded paths — caught=missed=0. Must pass
        // (vacuously). Gate reason must not claim a kill-rate failure.
        let gate = evaluate_diff_gate(&report(0, 0, 5, 0));
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn workspace_gate_fails_below_absolute_floor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        std::fs::write(&baseline, "50.0\n").unwrap();
        // 5 / (5 + 6) ≈ 45.5% — below the 60% absolute floor.
        let gate = evaluate_workspace_gate(&report(5, 6, 0, 0), &baseline).unwrap();
        assert!(matches!(gate.status, GateStatus::Fail { .. }));
    }

    #[test]
    fn workspace_gate_warns_on_drift_above_floor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        // Baseline 85%; current 80% (10 caught / 12.5 denom? Use 8/10
        // exactly for determinism). Drift = 80 - 85 = -5pp ≤ -2pp.
        std::fs::write(&baseline, "85.0\n").unwrap();
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline).unwrap();
        assert!(matches!(gate.status, GateStatus::Warn { .. }));
    }

    #[test]
    fn workspace_gate_passes_on_improvement() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        std::fs::write(&baseline, "70.0\n").unwrap();
        // 8/(8+2) = 80% — up from 70%.
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline).unwrap();
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn workspace_gate_seeds_missing_baseline_and_passes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("nested").join("kill_rate.txt");
        assert!(!baseline.exists());
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline).unwrap();
        assert!(matches!(gate.status, GateStatus::Pass));
        let seeded = std::fs::read_to_string(&baseline).unwrap();
        assert_eq!(seeded.trim(), "80.0", "seeded baseline must be rounded");
    }

    #[test]
    fn summary_round_trips_to_json_with_expected_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("summary.json");
        let report = report(8, 2, 3, 0);
        let gate = evaluate_diff_gate(&report);
        write_summary(&path, &report, &gate, &Mode::Diff { base: "origin/main".into() })
            .expect("write_summary");

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mode"].as_str(), Some("diff"));
        assert_eq!(v["caught"].as_u64(), Some(8));
        assert_eq!(v["missed"].as_u64(), Some(2));
        assert_eq!(v["unviable"].as_u64(), Some(3));
        assert_eq!(v["status"].as_str(), Some("pass"));
        assert!((v["kill_rate_pct"].as_f64().unwrap() - 80.0).abs() < 1e-6);
        assert_eq!(v["base_ref"].as_str(), Some("origin/main"));
        // drift fields are absent in diff mode.
        assert!(v.get("drift_pp").is_none() || v["drift_pp"].is_null());
    }

    #[test]
    fn summary_records_fail_status_and_reason() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("summary.json");
        let report = report(7, 3, 0, 0); // 70% — below the 80% floor
        let gate = evaluate_diff_gate(&report);
        write_summary(&path, &report, &gate, &Mode::Diff { base: "origin/main".into() })
            .expect("write_summary");

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["status"].as_str(), Some("fail"));
        let reason = v["reason"].as_str().expect("reason on fail");
        assert!(reason.contains("70.0%"), "reason should quote kill rate; got: {reason}");
    }

    #[test]
    fn round_pct_stable_on_whole_numbers() {
        assert!((round_pct(80.0) - 80.0).abs() < 1e-9);
        assert!((round_pct(80.04) - 80.0).abs() < 1e-9);
        assert!((round_pct(80.05) - 80.1).abs() < 1e-9);
    }

    #[test]
    fn raw_report_deserialises_from_cargo_mutants_output() {
        // The real shape cargo-mutants emits — verified against an
        // actual outcomes.json from this repo. Defensive against the
        // cargo-mutants schema drifting.
        let raw = r#"{
            "outcomes": [],
            "total_mutants": 260,
            "caught": 149,
            "missed": 38,
            "timeout": 0,
            "unviable": 73,
            "success": 0,
            "start_time": "2026-04-23T00:00:00Z",
            "end_time": "2026-04-23T00:05:00Z",
            "cargo_mutants_version": "27.0.0"
        }"#;
        let report: RawReport = serde_json::from_str(raw).unwrap();
        assert_eq!(report.caught, 149);
        assert_eq!(report.missed, 38);
        assert_eq!(report.unviable, 73);
        assert_eq!(report.cargo_mutants_version, "27.0.0");
        // Kill rate on this real run: 149 / (149 + 38) ≈ 79.7% → just
        // below the 80% diff gate. This is the actual project state
        // as of the outcomes.json used to write this test.
        let kr = kill_rate_percent(report.caught, report.missed);
        assert!(kr > 79.0 && kr < 80.0, "expected ~79.7%, got {kr}");
    }

    #[test]
    fn kill_rate_floor_constants_match_testing_rules() {
        // Guards against a drive-by edit silently lowering the bar.
        // The rules doc is .claude/rules/testing.md §Mutation testing.
        assert!(
            (DIFF_KILL_RATE_FLOOR - 80.0).abs() < 1e-9,
            "diff floor must match .claude/rules/testing.md (≥80%)"
        );
        assert!(
            (WORKSPACE_ABSOLUTE_FLOOR - 60.0).abs() < 1e-9,
            "workspace floor must match .claude/rules/testing.md (≥60%)"
        );
        assert!(
            (WORKSPACE_DRIFT_WARN_PP - (-2.0)).abs() < 1e-9,
            "drift warn must match .claude/rules/testing.md (-2pp)"
        );
    }
}
