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

/// Further scope narrowing layered on top of `Mode`.
///
/// Orthogonal to `Mode` — a diff-scoped run can be narrowed to a
/// single file; a workspace run can be narrowed to one package. These
/// are pass-throughs to `cargo-mutants`' own `--file`, `--package`,
/// and `--features` flags, plus one wrapper-level convenience
/// (`test_whole_workspace`) that encodes the default this project
/// wants when `--package` is set.
///
/// Defaults mirror `Default::default()`: empty file/package/feature
/// lists, and `test_whole_workspace = false` (tests run only against
/// the selected package when `packages` is non-empty).
///
/// Feature handling is a flat pass-through. Callers that want
/// integration-gated tests to participate must pass `--features
/// integration-tests` explicitly (CI does this; see
/// `.claude/rules/testing.md` §"Integration vs unit gating"). The
/// workspace-wide convention requires every member to declare
/// `integration-tests = []`, so a bare `integration-tests` resolves
/// uniformly under cargo-mutants v27's per-package scoping — no
/// per-package qualifier games here.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// `--file <GLOB>` (repeatable). Paths or globs passed verbatim to
    /// cargo-mutants.
    pub files: Vec<PathBuf>,
    /// `--package <CRATE>` (repeatable). When non-empty and
    /// `test_whole_workspace` is false, `--test-workspace=false` is
    /// also passed — mutation reruns only the selected packages'
    /// tests instead of the full workspace suite.
    pub packages: Vec<String>,
    /// `--features <LIST>` entries, joined with commas and passed
    /// through to cargo-mutants verbatim.
    pub features: Vec<String>,
    /// Force `--test-workspace=true` even when `packages` is
    /// non-empty. Rare escape hatch for mutations whose kill signal
    /// only fires in another crate's tests.
    pub test_whole_workspace: bool,
}

/// Hard gate thresholds (percentage of mutations caught).
///
/// Kept as associated constants so a test can assert them and so a
/// code review that changes the number shows up as a diff on the
/// constant, not hidden inside a comparison.
pub const DIFF_KILL_RATE_FLOOR: f64 = 80.0;
pub const WORKSPACE_ABSOLUTE_FLOOR: f64 = 60.0;
pub const WORKSPACE_DRIFT_WARN_PP: f64 = -2.0;

pub fn run(mode: &Mode, scope: &Scope) -> Result<()> {
    which_cargo_mutants()?;
    // We pass `--test-tool=nextest` below, so cargo-nextest must also
    // be on PATH. Fail fast with an install hint rather than letting
    // cargo-mutants report a cryptic subprocess error per mutation.
    which_cargo_nextest()?;

    // `cargo-mutants --output <DIR>` *creates* a `mutants.out/` subdir
    // within <DIR> (see `cargo mutants --help`). So we pass the xtask
    // target dir as `--output`, and outcomes.json lands at
    // `target/xtask/mutants.out/outcomes.json`.
    let output_parent = xtask_target_dir();
    let out_dir = output_parent.join("mutants.out");
    let summary_path = output_parent.join("mutants-summary.json");
    std::fs::create_dir_all(&output_parent)
        .wrap_err_with(|| format!("create_dir_all({})", output_parent.display()))?;

    // Clear any pre-existing wrapper / cargo-mutants verdict files
    // upfront. If the current run gets killed mid-invocation, a stale
    // verdict from the prior run must not remain on disk looking
    // authoritative — readers polling the files during an in-progress
    // run would otherwise act on verdicts the current run never
    // produced. See [`clear_stale_summary`] for which files are
    // cleared and why both `mutants-summary.json` AND
    // `mutants.out/outcomes.json` are at risk.
    clear_stale_summary(&output_parent)?;

    // 1. Run cargo-mutants. Exit status is partially trusted — a
    //    non-zero exit happens both for missed mutants (our gate
    //    handles it) and for genuine subprocess crashes. We
    //    distinguish them via the report file: a clean run always
    //    writes `outcomes.json`, *unless* cargo-mutants
    //    short-circuited at filter time ("INFO No mutants to filter")
    //    — in which case it exits 0 and writes nothing. That third
    //    state is treated as a vacuously-passing zero-mutant run.
    let status = invoke_cargo_mutants(mode, scope, &output_parent)?;

    // 2. Parse outcomes.json — or, if absent AND the subprocess
    //    exited cleanly, treat as a zero-mutant short-circuit.
    let outcomes_path = out_dir.join("outcomes.json");
    let Some(report) = read_outcomes_or_short_circuit(&outcomes_path, status)? else {
        // cargo-mutants exited 0 but wrote nothing — the
        // "INFO No mutants to filter" path. Vacuously pass:
        // emit a synthetic zero-mutant report, write the
        // summary, print one line, exit 0.
        return finalise_zero_mutant_run(&summary_path, mode);
    };

    // 3. Evaluate the mode-specific gate. The subprocess exit status
    //    is a load-bearing input: cargo-mutants exits non-zero in
    //    *two* failure modes — missed mutants (the gate handles it)
    //    and "unmutated baseline failed before any mutation ran" (the
    //    gate must NOT silently pass it). The discriminator is
    //    `total_mutants == 0 && !status.success()` — if mutants were
    //    generated, the gate covers missed-mutant exits; if zero
    //    mutants ran AND the subprocess crashed, the only explanation
    //    is a baseline failure, so refuse to pass.
    let baseline_succeeded = status.success();
    let gate = match mode {
        Mode::Diff { .. } => evaluate_diff_gate(&report, baseline_succeeded),
        Mode::Workspace { baseline_path } => {
            evaluate_workspace_gate(&report, baseline_path, baseline_succeeded)?
        }
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
        .is_ok_and(|s| s.success());
    if !found {
        bail!(
            "`cargo-mutants` not found on PATH. Install it with:\n\
             cargo install cargo-mutants\n\
             (or, on CI, via `taiki-e/install-action` with `tool: cargo-mutants`)"
        );
    }
    Ok(())
}

fn which_cargo_nextest() -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg("command -v cargo-nextest")
        .status()
        .is_ok_and(|s| s.success());
    if !found {
        bail!(
            "`cargo-nextest` not found on PATH. xtask mutants runs \
             cargo-mutants with `--test-tool=nextest` per \
             `.config/nextest.toml`. Install it with:\n\
             cargo install cargo-nextest --locked\n\
             (or, on CI, via `taiki-e/install-action` with `tool: cargo-nextest`)"
        );
    }
    Ok(())
}

fn invoke_cargo_mutants(
    mode: &Mode,
    scope: &Scope,
    output_parent: &Path,
) -> Result<std::process::ExitStatus> {
    // Materialise the diff file first (only for Diff mode). The pure
    // arg-assembly below takes the file path, not the git ref, so
    // tests can exercise it without hitting git.
    let diff_path = match mode {
        Mode::Diff { base } => {
            let path = xtask_target_dir().join("mutants.diff");
            let bytes = git_diff_against(base).wrap_err_with(|| {
                format!("git diff {base} failed — is `{base}` fetched in this clone?")
            })?;
            std::fs::write(&path, &bytes).wrap_err_with(|| format!("write {}", path.display()))?;
            eprintln!(
                "xtask mutants: wrote {} ({} bytes) for --in-diff",
                path.display(),
                bytes.len()
            );
            Some(path)
        }
        Mode::Workspace { .. } => None,
    };

    let args = build_cargo_mutants_args(mode, scope, output_parent, diff_path.as_deref());

    let mut cmd = Command::new(cargo());
    cmd.args(&args);
    // Select the `mutants` nextest profile (`.config/nextest.toml`) so
    // the per-mutant test run drops trybuild binaries — they don't
    // contribute to kill-rate signal and dominate wall-clock. nextest
    // reads NEXTEST_PROFILE as the equivalent of `--profile`; passing
    // it via env is the only way that works with cargo-mutants, which
    // does not forward arbitrary nextest flags.
    cmd.env("NEXTEST_PROFILE", "mutants");
    eprintln!("xtask mutants: running {} (NEXTEST_PROFILE=mutants)", format_cmd(&cmd));
    let status = cmd.status().wrap_err("spawn cargo-mutants")?;
    // Don't bail on non-zero — cargo-mutants returns non-zero for any
    // missed mutant, which is exactly the signal we want to measure
    // ourselves. We only fail if `outcomes.json` was not written (the
    // parse step surfaces that).
    Ok(status)
}

fn build_cargo_mutants_args(
    mode: &Mode,
    scope: &Scope,
    output_parent: &Path,
    diff_path: Option<&Path>,
) -> Vec<std::ffi::OsString> {
    use std::ffi::OsString;

    let mut args: Vec<OsString> = vec![
        "mutants".into(),
        // `--output <DIR>` points at the *parent* directory cargo-mutants
        // will drop `mutants.out/` into. Passing the mutants.out path
        // itself double-nests — see the doc comment on `run`.
        "--output".into(),
        output_parent.into(),
        // Match the project's primary test runner (`.config/nextest.toml`).
        // Doctests are not re-run per mutation; nextest skips them and
        // that is the documented mutation-testing behaviour.
        "--test-tool=nextest".into(),
    ];

    match mode {
        Mode::Diff { .. } => {
            let path = diff_path.expect("Mode::Diff must carry a materialised diff path");
            args.push("--in-diff".into());
            args.push(path.into());
        }
        Mode::Workspace { .. } => {
            args.push("--workspace".into());
        }
    }

    // Scope.files — `--file <GLOB>` (repeatable).
    for file in &scope.files {
        args.push("--file".into());
        args.push(file.into());
    }

    // Scope.packages — `--package <CRATE>` (repeatable).
    for pkg in &scope.packages {
        args.push("--package".into());
        args.push(pkg.into());
    }

    // Package-scoped runs default to rerunning only that package's
    // tests per mutation. This is the single largest wall-clock win
    // on a large workspace — the full suite may be hundreds of tests
    // whereas one crate's suite is dozens.
    if !scope.packages.is_empty() && !scope.test_whole_workspace {
        args.push("--test-workspace=false".into());
    }

    // Feature list — flat pass-through. The workspace-wide convention
    // is that every member declares `integration-tests = []` (see
    // `.claude/rules/testing.md` §"Integration vs unit gating" and the
    // enforcement test below), so a bare `--features integration-tests`
    // resolves uniformly under cargo-mutants v27's per-package scoping.
    // CI is responsible for passing `--features integration-tests`
    // explicitly when it wants integration-gated tests to participate.
    if !scope.features.is_empty() {
        args.push("--features".into());
        args.push(scope.features.join(",").into());
    }

    args
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

/// Discriminate between three post-subprocess states:
///   1. outcomes.json present  → parse and return Some(report).
///   2. outcomes.json absent + exit 0 → short-circuit (filter
///      intersection empty); return None and let the caller emit
///      a synthetic zero-mutant report.
///   3. outcomes.json absent + non-zero exit → genuine crash; bail
///      with the original "may have crashed" error.
///
/// The exit-status check is the only thing distinguishing (2) from
/// (3); cargo-mutants does not expose a structured "I had nothing
/// to do" signal.
fn read_outcomes_or_short_circuit(
    path: &Path,
    status: std::process::ExitStatus,
) -> Result<Option<RawReport>> {
    if path.is_file() {
        return Ok(Some(parse_outcomes(path)?));
    }
    if status.success() {
        // Filter intersection produced zero mutants. cargo-mutants
        // logged "INFO No mutants to filter" and returned 0 without
        // creating mutants.out/. Vacuously pass.
        eprintln!(
            "xtask mutants: cargo-mutants produced no report (filter \
             intersection is empty) — treating as zero-mutant vacuous pass"
        );
        return Ok(None);
    }
    bail!(
        "no outcomes.json at {} — cargo-mutants exited {} without producing \
         a report (subprocess likely crashed)",
        path.display(),
        status.code().map_or_else(|| "?".into(), |c| c.to_string()),
    )
}

/// Emit a `mutants-summary.json` representing a vacuously-passing
/// zero-mutant run, print a one-line report, and return Ok(()) so
/// the wrapper exits 0. Mirrors `write_summary` + `print_report`
/// for the case where there is no `RawReport` to populate them
/// from.
fn finalise_zero_mutant_run(summary_path: &Path, mode: &Mode) -> Result<()> {
    let synthetic = RawReport {
        total_mutants: 0,
        caught: 0,
        missed: 0,
        timeout: 0,
        unviable: 0,
        success: 0,
        cargo_mutants_version: String::new(),
    };
    // Workspace mode never short-circuits to zero mutants in
    // practice (no --in-diff filter), but model it consistently:
    // the diff-gate logic already returns Pass on total_mutants=0,
    // so both arms collapse to the same call. This branch is only
    // reachable when cargo-mutants exited 0 (the "INFO No mutants
    // to filter" path), so `baseline_succeeded = true` is correct
    // by construction — the new baseline-failure guard is a no-op
    // here and the vacuous-pass arm fires.
    let gate = match mode {
        Mode::Diff { .. } | Mode::Workspace { .. } => evaluate_diff_gate(&synthetic, true),
    };
    write_summary(summary_path, &synthetic, &gate, mode)
        .wrap_err_with(|| format!("write {}", summary_path.display()))?;
    println!(
        "mutants: mode={} total=0 — no mutants in scope (filter intersection empty); vacuous pass",
        gate.mode_label
    );
    println!("mutants: PASS");
    Ok(())
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

fn evaluate_diff_gate(report: &RawReport, baseline_succeeded: bool) -> Gate {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);
    let status = if report.total_mutants == 0 && !baseline_succeeded {
        // cargo-mutants exited non-zero AND zero mutants were
        // generated — the unmutated baseline failed before any
        // mutation could run (`ERROR cargo test failed in an
        // unmutated tree, so no mutants were tested`). cargo-mutants
        // still writes outcomes.json with all counters zeroed; the
        // earlier `total_mutants == 0` arm would otherwise read that
        // as "vacuous pass" and the wrapper would exit 0 against a
        // failing test suite. Refuse to pass: there is no quality
        // signal, and a falsely-green run masks the underlying
        // breakage.
        GateStatus::Fail {
            reason: "unmutated baseline failed: cargo-mutants exited non-zero \
                     with zero mutants tested — see target/xtask/mutants.out/log/* \
                     for the failing test names"
                .to_owned(),
        }
    } else if report.total_mutants == 0 {
        // No mutants generated from the diff AND the subprocess
        // exited cleanly — e.g. the PR only touched excluded paths
        // or comments. Truly vacuous: pass.
        GateStatus::Pass
    } else if report.caught == 0 && report.missed == 0 {
        // Mutants WERE generated but none were evaluated — every one
        // was unviable (rustc rejected the mutated source) or timed
        // out. cargo-mutants logs this as
        // `WARN No mutants were viable`. The gate has no quality
        // signal and must not pass: a future repeat of this state
        // would silently mask a broken mutation lane.
        GateStatus::Fail {
            reason: format!(
                "no quality signal: total={total} unviable={unviable} timeout={timeout} \
                 — see target/xtask/mutants.out/log/* for rustc diagnostics",
                total = report.total_mutants,
                unviable = report.unviable,
                timeout = report.timeout,
            ),
        }
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

fn evaluate_workspace_gate(
    report: &RawReport,
    baseline_path: &Path,
    baseline_succeeded: bool,
) -> Result<Gate> {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);

    // Mirror the diff-gate baseline-failure guard: cargo-mutants
    // exited non-zero with zero mutants tested ⇒ the unmutated
    // baseline failed and there is no quality signal to gate on.
    // Refuse to pass before consulting the stored baseline (otherwise
    // we'd seed `mutants-baseline/main/kill_rate.txt` with a fake
    // 100% from a broken run).
    if report.total_mutants == 0 && !baseline_succeeded {
        return Ok(Gate {
            mode_label: "workspace".to_owned(),
            kill_rate_pct,
            baseline_pct: None,
            drift_pp: None,
            status: GateStatus::Fail {
                reason: "unmutated baseline failed: cargo-mutants exited non-zero \
                         with zero mutants tested — see target/xtask/mutants.out/log/* \
                         for the failing test names"
                    .to_owned(),
            },
        });
    }

    // Symmetric guard with `evaluate_diff_gate`: mutants WERE
    // generated but none were evaluated — every one unviable or
    // timed out. No quality signal; must fail before consulting the
    // baseline.
    if report.total_mutants > 0 && report.caught == 0 && report.missed == 0 {
        return Ok(Gate {
            mode_label: "workspace".to_owned(),
            kill_rate_pct,
            baseline_pct: None,
            drift_pp: None,
            status: GateStatus::Fail {
                reason: format!(
                    "no quality signal: total={total} unviable={unviable} timeout={timeout} \
                     — see target/xtask/mutants.out/log/* for rustc diagnostics",
                    total = report.total_mutants,
                    unviable = report.unviable,
                    timeout = report.timeout,
                ),
            },
        });
    }

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

/// Remove pre-existing wrapper / cargo-mutants verdict files so an
/// in-progress run cannot leave a stale verdict visible on disk. Two
/// files are at risk and BOTH must be cleared upfront:
///
/// * `<output_parent>/mutants-summary.json` — the wrapper's own
///   structured summary.
/// * `<output_parent>/mutants.out/outcomes.json` — cargo-mutants'
///   per-run report.
///
/// Why both. cargo-mutants writes `outcomes.json` ONLY when it actually
/// runs mutations; if the resolved filter intersection is empty it
/// logs `INFO No mutants to filter`, exits 0, and writes nothing. Our
/// post-subprocess [`read_outcomes_or_short_circuit`] then trusts a
/// pre-existing `outcomes.json` on disk as the current run's report —
/// even though the current run never produced it. A prior run with a
/// different `.cargo/mutants.toml` `exclude_re` (or a different scope,
/// or a different `--in-diff` baseline) leaves its verdict behind, and
/// the next run that short-circuits silently inherits it. The fix
/// mirrors the existing summary-clearing semantic: the only valid
/// `outcomes.json` on disk is "the one written by the run that just
/// finished."
///
/// Clearing only `mutants-summary.json` was the *original* shape of
/// this function; a wrapper-driven mutation run on
/// `crates/overdrive-cli/src/commands/job.rs` (the
/// `fix-job-submit-body-decode-variant` PR) surfaced the asymmetry —
/// a freshly-suppressed `Default::default` `exclude_re` correctly
/// short-circuited cargo-mutants, but the wrapper still reported
/// `total=3 unviable=3` from a prior pre-edit run's `outcomes.json`.
///
/// Noop on either file when absent (fresh checkout, or the prior run
/// cleaned up). Any other I/O error propagates with context.
fn clear_stale_summary(output_parent: &Path) -> Result<()> {
    let summary_path = output_parent.join("mutants-summary.json");
    let outcomes_path = output_parent.join("mutants.out").join("outcomes.json");
    remove_file_if_present(&summary_path)
        .wrap_err_with(|| format!("remove stale summary {}", summary_path.display()))?;
    remove_file_if_present(&outcomes_path)
        .wrap_err_with(|| format!("remove stale outcomes {}", outcomes_path.display()))?;
    Ok(())
}

/// Idempotent file removal: `Ok(())` if removed or absent, error
/// otherwise. Factored out so [`clear_stale_summary`] reads as two
/// declarative removals rather than two repeated `match` blocks.
fn remove_file_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
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
        let gate = evaluate_diff_gate(&report(8, 2, 0, 0), true);
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn diff_gate_fails_just_below_floor() {
        // 7 / (7 + 2) = 77.7…% — below 80%. Must fail, and the reason
        // must mention the actual numerator/denominator so the
        // developer can find the missed mutations.
        let gate = evaluate_diff_gate(&report(7, 2, 0, 0), true);
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail, got {other:?}"),
        };
        assert!(reason.contains("77.8%") || reason.contains("77.7%"), "got: {reason}");
        assert!(reason.contains("caught=7"), "got: {reason}");
        assert!(reason.contains("missed=2"), "got: {reason}");
    }

    #[test]
    fn diff_gate_fails_when_unmutated_baseline_failed() {
        // Reproduction of the GHA passing-on-test-failures bug:
        // cargo-mutants runs the unmutated baseline first, the
        // baseline test suite fails, no mutants are tested, and
        // outcomes.json lands with all counters at zero. Before the
        // fix this fell through to the `total_mutants == 0 ⇒ Pass`
        // arm and the wrapper exited 0 against a broken test suite.
        // The non-zero subprocess exit is the discriminator: if the
        // baseline had succeeded we would have reached `mutants tested
        // = 0` because the diff genuinely had no mutable lines, in
        // which case cargo-mutants exits 0 and the
        // `read_outcomes_or_short_circuit` short-circuit fires —
        // that path goes through `finalise_zero_mutant_run`, not
        // here.
        let zero_mutants = RawReport {
            total_mutants: 0,
            caught: 0,
            missed: 0,
            timeout: 0,
            unviable: 0,
            success: 0,
            cargo_mutants_version: "27.0.0".to_owned(),
        };
        let gate = evaluate_diff_gate(&zero_mutants, false);
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail on baseline failure, got {other:?}"),
        };
        assert!(reason.contains("unmutated baseline failed"), "got: {reason}");
        assert!(reason.contains("mutants.out/log"), "got: {reason}");
    }

    #[test]
    fn workspace_gate_fails_when_unmutated_baseline_failed() {
        // Same shape as the diff variant: a workspace nightly whose
        // baseline test suite fails must NOT seed the stored
        // baseline file with a fake 100% — that would silently
        // ratchet the next run's bar to "anything passes" until
        // someone manually corrects the file.
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        // No prior baseline on disk — this is the scenario where
        // the bug would seed a bogus 100.0%.
        let zero_mutants = RawReport {
            total_mutants: 0,
            caught: 0,
            missed: 0,
            timeout: 0,
            unviable: 0,
            success: 0,
            cargo_mutants_version: "27.0.0".to_owned(),
        };
        let gate = evaluate_workspace_gate(&zero_mutants, &baseline, false).unwrap();
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail on baseline failure, got {other:?}"),
        };
        assert!(reason.contains("unmutated baseline failed"), "got: {reason}");
        // Baseline file MUST NOT have been seeded — a failed run
        // cannot be allowed to write a fake kill-rate floor.
        assert!(!baseline.exists(), "baseline file must not be seeded on baseline failure");
    }

    #[test]
    fn diff_gate_passes_when_no_mutants_in_scope() {
        // PR touched only excluded paths or comments — cargo-mutants
        // generated zero mutants. Truly vacuous: must pass.
        let no_mutants = RawReport {
            total_mutants: 0,
            caught: 0,
            missed: 0,
            timeout: 0,
            unviable: 0,
            success: 0,
            cargo_mutants_version: "27.0.0".to_owned(),
        };
        let gate = evaluate_diff_gate(&no_mutants, true);
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn diff_gate_fails_when_every_mutant_unviable() {
        // cargo-mutants generated mutants but every single one was
        // rejected by rustc — the WARN
        // `No mutants were viable: perhaps there is a problem with
        // building in a scratch directory` case. The gate has no
        // quality signal and must FAIL: silently passing here
        // would mask any future cause of all-unviable runs (broken
        // workspace lints, scratch-dir issue, build.rs path bug,
        // mutation operator producing rustc-rejected code).
        let gate = evaluate_diff_gate(&report(0, 0, 250, 0), true);
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail on all-unviable, got {other:?}"),
        };
        assert!(reason.contains("no quality signal"), "got: {reason}");
        assert!(reason.contains("unviable=250"), "got: {reason}");
        assert!(reason.contains("mutants.out/log"), "got: {reason}");
    }

    #[test]
    fn diff_gate_fails_when_every_mutant_timed_out() {
        // Symmetric to all-unviable: every mutant exceeded the test
        // budget. Same loss of quality signal; must FAIL with the
        // same reason shape.
        let gate = evaluate_diff_gate(&report(0, 0, 0, 250), true);
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail on all-timeout, got {other:?}"),
        };
        assert!(reason.contains("no quality signal"), "got: {reason}");
        assert!(reason.contains("timeout=250"), "got: {reason}");
    }

    #[test]
    fn workspace_gate_fails_when_every_mutant_unviable() {
        // Same predicate as diff: a fully-unviable workspace run is
        // not a baseline-relative regression — it is a quality-signal
        // failure that must short-circuit before the baseline read.
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        std::fs::write(&baseline, "85.0\n").unwrap();
        let gate = evaluate_workspace_gate(&report(0, 0, 250, 0), &baseline, true).unwrap();
        let reason = match gate.status {
            GateStatus::Fail { reason } => reason,
            other => panic!("expected Fail on all-unviable, got {other:?}"),
        };
        assert!(reason.contains("no quality signal"), "got: {reason}");
        assert!(reason.contains("unviable=250"), "got: {reason}");
    }

    #[test]
    fn workspace_gate_fails_below_absolute_floor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        std::fs::write(&baseline, "50.0\n").unwrap();
        // 5 / (5 + 6) ≈ 45.5% — below the 60% absolute floor.
        let gate = evaluate_workspace_gate(&report(5, 6, 0, 0), &baseline, true).unwrap();
        assert!(matches!(gate.status, GateStatus::Fail { .. }));
    }

    #[test]
    fn workspace_gate_warns_on_drift_above_floor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        // Baseline 85%; current 80% (10 caught / 12.5 denom? Use 8/10
        // exactly for determinism). Drift = 80 - 85 = -5pp ≤ -2pp.
        std::fs::write(&baseline, "85.0\n").unwrap();
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline, true).unwrap();
        assert!(matches!(gate.status, GateStatus::Warn { .. }));
    }

    #[test]
    fn workspace_gate_passes_on_improvement() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("kill_rate.txt");
        std::fs::write(&baseline, "70.0\n").unwrap();
        // 8/(8+2) = 80% — up from 70%.
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline, true).unwrap();
        assert!(matches!(gate.status, GateStatus::Pass));
    }

    #[test]
    fn workspace_gate_seeds_missing_baseline_and_passes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let baseline = tmp.path().join("nested").join("kill_rate.txt");
        assert!(!baseline.exists());
        let gate = evaluate_workspace_gate(&report(8, 2, 0, 0), &baseline, true).unwrap();
        assert!(matches!(gate.status, GateStatus::Pass));
        let seeded = std::fs::read_to_string(&baseline).unwrap();
        assert_eq!(seeded.trim(), "80.0", "seeded baseline must be rounded");
    }

    #[test]
    fn summary_round_trips_to_json_with_expected_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("summary.json");
        let report = report(8, 2, 3, 0);
        let gate = evaluate_diff_gate(&report, true);
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
        let gate = evaluate_diff_gate(&report, true);
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
    fn read_outcomes_returns_none_when_file_absent_and_exit_zero() {
        // Simulate cargo-mutants short-circuiting on "No mutants to filter":
        // outcomes.json absent, exit 0 ⇒ Ok(None).
        let tmp = tempfile::tempdir().expect("tempdir");
        let absent = tmp.path().join("outcomes.json");
        let zero = std::process::Command::new("true").status().expect("spawn true");
        let result = read_outcomes_or_short_circuit(&absent, zero).expect("must not bail");
        assert!(result.is_none(), "absent file + clean exit ⇒ None");
    }

    #[test]
    fn read_outcomes_bails_when_file_absent_and_exit_nonzero() {
        // Crash case: outcomes.json absent AND subprocess returned
        // non-zero. Must bail.
        let tmp = tempfile::tempdir().expect("tempdir");
        let absent = tmp.path().join("outcomes.json");
        let nonzero = std::process::Command::new("false").status().expect("spawn false");
        let err = read_outcomes_or_short_circuit(&absent, nonzero)
            .expect_err("must bail on absent file + non-zero exit");
        let msg = format!("{err:#}");
        assert!(msg.contains("no outcomes.json"), "got: {msg}");
        assert!(msg.contains("subprocess likely crashed"), "got: {msg}");
    }

    #[test]
    fn read_outcomes_returns_some_when_file_present() {
        // Happy path: file present ⇒ parse and return Some(report).
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("outcomes.json");
        std::fs::write(
            &path,
            r#"{"outcomes":[],"total_mutants":1,"caught":1,"missed":0,
                "timeout":0,"unviable":0,"success":1,
                "cargo_mutants_version":"27.0.0"}"#,
        )
        .unwrap();
        let zero = std::process::Command::new("true").status().expect("spawn true");
        let report = read_outcomes_or_short_circuit(&path, zero)
            .expect("must parse")
            .expect("Some when file present");
        assert_eq!(report.caught, 1);
    }

    #[test]
    fn finalise_zero_mutant_run_writes_pass_summary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let summary = tmp.path().join("mutants-summary.json");
        finalise_zero_mutant_run(&summary, &Mode::Diff { base: "origin/main".into() })
            .expect("must succeed");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&summary).unwrap()).unwrap();
        assert_eq!(v["status"].as_str(), Some("pass"));
        assert_eq!(v["total_mutants"].as_u64(), Some(0));
        assert_eq!(v["caught"].as_u64(), Some(0));
        assert_eq!(v["missed"].as_u64(), Some(0));
        // 100.0 because kill_rate_percent(0, 0) == 100.0 by the
        // existing vacuous-pass convention.
        assert!((v["kill_rate_pct"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert_eq!(v["base_ref"].as_str(), Some("origin/main"));
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

    // ------------------------------------------------------------------
    // build_cargo_mutants_args — pure argv assembly.
    // ------------------------------------------------------------------

    /// Convert the argv to a whitespace-joined string so assertions
    /// read as the flag shape a human would recognise.
    fn argv_str(args: &[std::ffi::OsString]) -> String {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn argv_diff_mode_carries_in_diff_file_path() {
        let mode = Mode::Diff { base: "origin/main".into() };
        let diff = Path::new("/tmp/mutants.diff");
        let out = Path::new("/tmp/xtask");
        let args = build_cargo_mutants_args(&mode, &Scope::default(), out, Some(diff));
        let joined = argv_str(&args);
        assert!(
            joined.starts_with("mutants --output /tmp/xtask --test-tool=nextest"),
            "got: {joined}"
        );
        assert!(joined.contains("--in-diff /tmp/mutants.diff"), "got: {joined}");
        assert!(!joined.contains("--workspace"), "got: {joined}");
    }

    #[test]
    fn argv_workspace_mode_uses_workspace_flag_not_in_diff() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let out = Path::new("/tmp/xtask");
        let args = build_cargo_mutants_args(&mode, &Scope::default(), out, None);
        let joined = argv_str(&args);
        assert!(joined.contains("--workspace"), "got: {joined}");
        assert!(!joined.contains("--in-diff"), "got: {joined}");
    }

    #[test]
    fn argv_includes_repeated_file_flags_in_order() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            files: vec![
                PathBuf::from("crates/a/src/lib.rs"),
                PathBuf::from("crates/b/src/handlers.rs"),
            ],
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        // Two --file flags, in the declared order.
        let a_pos = joined.find("--file crates/a/src/lib.rs").expect("first --file");
        let b_pos = joined.find("--file crates/b/src/handlers.rs").expect("second --file");
        assert!(a_pos < b_pos, "got: {joined}");
    }

    #[test]
    fn argv_package_scope_adds_test_workspace_false_by_default() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope { packages: vec!["overdrive-control-plane".into()], ..Scope::default() };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--package overdrive-control-plane"), "got: {joined}");
        assert!(joined.contains("--test-workspace=false"), "got: {joined}");
    }

    #[test]
    fn argv_test_whole_workspace_override_suppresses_test_workspace_false() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            packages: vec!["overdrive-core".into()],
            test_whole_workspace: true,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--package overdrive-core"), "got: {joined}");
        assert!(!joined.contains("--test-workspace=false"), "got: {joined}");
    }

    #[test]
    fn argv_emits_features_joined_with_commas() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope { features: vec!["foo".into(), "bar".into()], ..Scope::default() };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--features foo,bar"), "got: {joined}");
    }

    #[test]
    fn argv_no_features_flag_when_list_empty() {
        // With no user features, no --features flag should appear at
        // all. Diff mode and workspace mode behave identically here —
        // the wrapper does not auto-add anything; CI is responsible
        // for passing `--features integration-tests` explicitly.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope::default();
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(!joined.contains("--features"), "got: {joined}");
    }

    #[test]
    fn argv_features_pass_through_verbatim() {
        // Confirms the flat pass-through: whatever the caller puts in
        // `Scope.features` lands in `--features`, comma-joined, no
        // wrapper-side rewriting. Replaces the prior auto-add tests.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            features: vec!["integration-tests".into(), "extra-feature".into()],
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--features integration-tests,extra-feature"), "got: {joined}");
    }

    #[test]
    fn argv_output_flag_always_points_at_given_parent() {
        // Guard against a silent default drift that could re-introduce
        // the old `mutants.out/mutants.out/` double-nest bug.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let args =
            build_cargo_mutants_args(&mode, &Scope::default(), Path::new("/tmp/custom"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--output /tmp/custom"), "got: {joined}");
    }

    #[test]
    fn argv_always_pins_nextest_test_tool() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let args =
            build_cargo_mutants_args(&mode, &Scope::default(), Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--test-tool=nextest"), "got: {joined}");
    }

    // ---- clear_stale_summary ---------------------------------------------

    #[test]
    fn clear_stale_summary_removes_existing_file() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let summary = dir.path().join("mutants-summary.json");
        std::fs::write(&summary, r#"{"status":"pass"}"#).expect("seed stale summary");
        assert!(summary.exists(), "precondition — stale summary exists");

        clear_stale_summary(dir.path()).expect("clear must succeed when file present");

        assert!(!summary.exists(), "stale summary must be removed");
    }

    #[test]
    fn clear_stale_summary_is_noop_when_file_absent() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let summary = dir.path().join("mutants-summary.json");
        assert!(!summary.exists(), "precondition — summary absent");

        clear_stale_summary(dir.path()).expect("clear must succeed when file absent");

        assert!(!summary.exists(), "file absent remains absent");
    }

    /// Regression test for the stale-`outcomes.json` bug surfaced on
    /// `fix-job-submit-body-decode-variant`. cargo-mutants writes
    /// `outcomes.json` ONLY when it actually runs mutations; on
    /// short-circuit ("INFO No mutants to filter") it writes nothing
    /// and exits 0, leaving any prior run's `outcomes.json` on disk.
    /// `read_outcomes_or_short_circuit` then trusted the stale file
    /// as the current run's report — silently importing a verdict the
    /// current run never produced (e.g. a 3-mutant `unviable` outcome
    /// from a prior run, after the operator added an `exclude_re`
    /// entry that should have made the next run vacuously pass).
    /// `clear_stale_summary` now wipes both files upfront.
    #[test]
    fn clear_stale_summary_removes_stale_outcomes_json_too() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let summary = dir.path().join("mutants-summary.json");
        let mutants_out = dir.path().join("mutants.out");
        let outcomes = mutants_out.join("outcomes.json");
        std::fs::create_dir_all(&mutants_out).expect("create mutants.out");
        std::fs::write(&summary, r#"{"status":"pass"}"#).expect("seed stale summary");
        std::fs::write(&outcomes, r#"{"outcomes":[]}"#).expect("seed stale outcomes");
        assert!(summary.exists() && outcomes.exists(), "precondition — both stale");

        clear_stale_summary(dir.path()).expect("clear must succeed");

        assert!(!summary.exists(), "stale summary must be removed");
        assert!(!outcomes.exists(), "stale outcomes.json must be removed");
        // The mutants.out/ directory itself MAY remain (cargo-mutants
        // recreates it as needed); only the verdict file is the
        // authority-leaking artefact.
    }

    #[test]
    fn clear_stale_summary_handles_missing_mutants_out_directory() {
        // The fresh-checkout case: nothing on disk yet. Both files
        // absent must not error.
        let dir = tempfile::tempdir().expect("tmpdir");
        assert!(!dir.path().join("mutants-summary.json").exists());
        assert!(!dir.path().join("mutants.out").exists());

        clear_stale_summary(dir.path()).expect("must succeed on fresh checkout");
    }

    // ---- workspace convention enforcement ----------------------------------

    /// Workspace-wide convention: every member declares
    /// `integration-tests = []` in its `[features]` block, even crates
    /// with no integration tests of their own (no-op declaration).
    ///
    /// Why: cargo-mutants v27.0.0 scopes per-mutant test invocations
    /// to `--package <owning-crate>` and inherits the workspace
    /// `--features` list. A bare `--features integration-tests` on the
    /// CLI breaks the moment cargo-mutants scopes to a non-declaring
    /// package — cargo refuses with `error: the package 'X' does not
    /// contain this feature: integration-tests` and every mutant under
    /// that package is marked unviable, collapsing the kill-rate signal
    /// to zero. The convention makes the bare feature universally
    /// valid; this test enforces the convention so a newly-added crate
    /// cannot silently re-introduce the failure mode.
    ///
    /// See `.claude/rules/testing.md` §"Integration vs unit gating"
    /// and the prior incident on PR #132 (April 2026).
    #[test]
    fn every_workspace_member_declares_integration_tests_feature() {
        // `cargo metadata` is the source of truth for workspace
        // members — same data Cargo uses when resolving builds, no
        // hand-rolled TOML parsing of our own. xtask already depends
        // on `cargo_metadata` for unrelated subcommands.
        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("CARGO_MANIFEST_DIR has a parent (workspace root)")
                    .join("Cargo.toml"),
            )
            .no_deps()
            .exec()
            .expect("cargo metadata succeeds");

        let mut missing = Vec::new();
        for pkg_id in &metadata.workspace_members {
            let pkg = &metadata[pkg_id];
            if !pkg.features.contains_key("integration-tests") {
                missing.push(pkg.name.clone());
            }
        }

        assert!(
            missing.is_empty(),
            "workspace convention violation — every member must declare \
             `integration-tests = []` in its `[features]` block (no-op \
             for crates without integration tests). See \
             `.claude/rules/testing.md` §\"Integration vs unit gating\". \
             Missing in: {missing:?}"
        );
    }
}
