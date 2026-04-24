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
/// and `--features` flags, plus two wrapper-level conveniences
/// (`test_whole_workspace`, `auto_integration_tests`) that encode the
/// defaults this project wants.
///
/// Defaults mirror `Default::default()`: empty file/package/feature
/// lists, `test_whole_workspace = false` (tests run only against the
/// selected package when `packages` is non-empty), and
/// `auto_integration_tests = true` (this repo's acceptance tests
/// live behind `#[cfg(feature = "integration-tests")]` per
/// `.claude/rules/testing.md` §"Integration vs unit gating" —
/// without that feature, the tests that would catch mutations do not
/// compile and the kill rate is artificially low).
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
    /// `--features <LIST>` entries. Merged with `integration-tests`
    /// when `auto_integration_tests` applies (see below).
    pub features: Vec<String>,
    /// Force `--test-workspace=true` even when `packages` is
    /// non-empty. Rare escape hatch for mutations whose kill signal
    /// only fires in another crate's tests.
    pub test_whole_workspace: bool,
    /// Auto-add `integration-tests` to the effective feature list
    /// when at least one entry in `packages` declares that feature
    /// in its Cargo.toml. Default true; the CLI exposes
    /// `--no-integration-tests` as the opt-out.
    pub auto_integration_tests: bool,
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

    // Clear any pre-existing `mutants-summary.json` upfront. If the
    // current run gets killed mid-invocation, a stale summary from the
    // prior run must not remain on disk looking authoritative —
    // readers polling the file during an in-progress run would
    // otherwise act on a verdict the current run never produced.
    clear_stale_summary(&summary_path)?;

    // 1. Run cargo-mutants. Exit status is intentionally ignored — a
    //    non-zero exit from cargo-mutants happens on any missed mutant,
    //    which we handle via our own gate below. We only care that the
    //    subprocess produced `outcomes.json`.
    let _ = invoke_cargo_mutants(mode, scope, &output_parent)?;

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
    eprintln!("xtask mutants: running {}", format_cmd(&cmd));
    let status = cmd.status().wrap_err("spawn cargo-mutants")?;
    // Don't bail on non-zero — cargo-mutants returns non-zero for any
    // missed mutant, which is exactly the signal we want to measure
    // ourselves. We only fail if `outcomes.json` was not written (the
    // parse step surfaces that).
    Ok(status)
}

/// Assemble the flag list for `cargo mutants`. Pure: given mode/scope
/// and a pre-materialised diff path, returns the argv. No subprocess,
/// no filesystem writes.
///
/// Every flag is documented alongside its case so a reviewer can see
/// which `Mode` or `Scope` field produced it without cross-referencing
/// the cargo-mutants man page.
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

    // Effective feature list.
    let mut features: Vec<String> = scope.features.clone();
    if scope.auto_integration_tests {
        if scope.packages.is_empty() {
            // Diff-scoped or workspace-scoped run with no explicit
            // `--package`. The old behaviour skipped integration-tests
            // auto-enable here, which silently dropped every
            // acceptance test gated behind `#[cfg(feature =
            // "integration-tests")]` from the mutation lane —
            // measurably lowering kill rate (the phase-1-control-plane-
            // core diff-scoped run without this fix finished at 72.9%;
            // with it, the integration-gated tests participate and
            // catch the per-crate missed mutations). We use cargo's
            // per-package feature syntax (`<pkg>/integration-tests`)
            // so crates that do NOT declare the feature never see an
            // unknown-feature build error — enablement is narrowed to
            // the crates that actually declare it.
            if let Ok(declaring_crates) = workspace_crates_with_integration_tests_feature() {
                for pkg in declaring_crates {
                    let qualified = format!("{pkg}/integration-tests");
                    if !features.iter().any(|f| f == &qualified) {
                        features.push(qualified);
                    }
                }
            }
        } else {
            // Package-scoped: enable the bare `integration-tests`
            // feature if at least one scoped package declares it.
            // Cargo's scoping rules mean the bare feature name applies
            // to the packages under test.
            let any_declares = scope
                .packages
                .iter()
                .any(|p| crate_declares_integration_tests_feature(p).unwrap_or(false));
            if any_declares && !features.iter().any(|f| f == "integration-tests") {
                features.push("integration-tests".into());
            }
        }
    }
    if !features.is_empty() {
        args.push("--features".into());
        args.push(features.join(",").into());
    }

    args
}

fn crate_declares_integration_tests_feature(pkg: &str) -> Result<bool> {
    // Resolve against the workspace root, not the current directory:
    // `cargo xtask mutants` is normally invoked from the workspace
    // root so `crates/<pkg>` resolves, but `cargo nextest` runs tests
    // with cwd = the package being tested (xtask/), where those paths
    // don't resolve. `CARGO_MANIFEST_DIR` is xtask's dir; its parent
    // is the workspace root in this repo's layout.
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| color_eyre::eyre::eyre!("CARGO_MANIFEST_DIR has no parent"))?
        .to_path_buf();
    let candidates = [
        workspace_root.join("crates").join(pkg).join("Cargo.toml"),
        workspace_root.join(pkg).join("Cargo.toml"),
    ];
    let manifest = candidates.iter().find(|p| p.is_file());
    let Some(manifest) = manifest else {
        return Ok(false);
    };
    let raw = std::fs::read_to_string(manifest)
        .wrap_err_with(|| format!("read {}", manifest.display()))?;
    Ok(manifest_declares_integration_tests_feature(&raw))
}

/// Enumerate workspace member crates that declare the
/// `integration-tests` feature in their `Cargo.toml`.
///
/// Used by `build_cargo_mutants_args` in diff-scoped mode: without a
/// `--package` scope, the existing per-package auto-enable has no
/// targets to walk, so a diff-scoped run otherwise invisibly strips
/// integration-gated tests from the mutation lane. Reading the root
/// workspace manifest and enumerating its `members` list is the
/// narrowest fix — no crate is tested against a feature it does not
/// declare, and cargo-mutants' per-package `<pkg>/integration-tests`
/// feature syntax lets us enable each one independently.
///
/// Only members under `crates/` are returned. `xtask/` also declares
/// the feature for its own acceptance tests, but those tests exercise
/// xtask subcommands — they do not kill mutations in the platform
/// crates, and enabling the feature on every per-mutation build adds
/// wall-clock without raising kill rate. `xtask` is already excluded
/// from `cargo mutants` via `.cargo/mutants.toml`'s Rule 6 glob, so
/// skipping it here keeps the two filters consistent.
///
/// # Errors
///
/// Returns the I/O error wrapped in context if the workspace
/// `Cargo.toml` cannot be read. A workspace Cargo.toml without a
/// `members` list or without any integration-tests-declaring member
/// yields `Ok(vec![])`.
fn workspace_crates_with_integration_tests_feature() -> Result<Vec<String>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| color_eyre::eyre::eyre!("CARGO_MANIFEST_DIR has no parent"))?
        .to_path_buf();
    let root_manifest = workspace_root.join("Cargo.toml");
    let raw = std::fs::read_to_string(&root_manifest)
        .wrap_err_with(|| format!("read {}", root_manifest.display()))?;

    let members = parse_workspace_members(&raw);

    let mut declared = Vec::new();
    for member_path in members {
        // Skip members outside `crates/` — see the doc comment above
        // for why `xtask/` is deliberately not auto-enabled.
        if !member_path.starts_with("crates/") {
            continue;
        }
        // Member paths are workspace-relative (e.g. `crates/overdrive-cli`).
        // The crate name is the last path component — matches Cargo's
        // default of `package.name == last path component` across every
        // member in this workspace.
        let Some(crate_name) =
            member_path.rsplit('/').next().filter(|s| !s.is_empty()).map(str::to_owned)
        else {
            continue;
        };
        if crate_declares_integration_tests_feature(&crate_name).unwrap_or(false) {
            declared.push(crate_name);
        }
    }
    declared.sort();
    Ok(declared)
}

/// Pure text scan of the root `Cargo.toml` — returns the entries under
/// `[workspace] members = [ ... ]`, in source order, with surrounding
/// whitespace and quotes stripped.
///
/// Tolerates single-line `members = ["a", "b"]` and multi-line forms.
/// Comment lines (`#`) and entries mentioning `.` in leading position
/// (defensive — workspace members shouldn't be `.` but any stray entry
/// would be meaningless in the per-crate feature-discovery pass) are
/// silently dropped.
fn parse_workspace_members(manifest: &str) -> Vec<String> {
    let mut in_workspace = false;
    let mut in_members = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim();

        // Section boundary — reset the members-array state if we cross
        // into another section.
        if trimmed.starts_with('[') {
            in_workspace = trimmed.starts_with("[workspace]");
            in_members = false;
            continue;
        }

        if !in_workspace {
            continue;
        }

        // Enter members array. Handles both `members = [` (start on this
        // line) and inline `members = ["a", "b"]`.
        if let Some(after) = trimmed.strip_prefix("members") {
            let after = after.trim_start().trim_start_matches('=').trim_start();
            if let Some(inside) = after.strip_prefix('[') {
                in_members = true;
                // `inside` drops the opening `[` so a single-line form
                // lands in the element-parsing path below.
                for entry in inside.split(',') {
                    let e = entry.trim().trim_end_matches(']').trim();
                    if let Some(name) = strip_string_literal(e) {
                        out.push(name.to_owned());
                    }
                }
                if after.contains(']') {
                    in_members = false;
                }
            }
            continue;
        }

        if in_members {
            for entry in trimmed.split(',') {
                let e = entry.trim().trim_end_matches(']').trim();
                if let Some(name) = strip_string_literal(e) {
                    out.push(name.to_owned());
                }
            }
            if trimmed.contains(']') {
                in_members = false;
            }
        }
    }
    out
}

/// Strip surrounding `"..."` or `'...'` from a TOML string literal,
/// rejecting bare identifiers and comments.
fn strip_string_literal(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('#') {
        return None;
    }
    let (first, last) = (s.chars().next()?, s.chars().last()?);
    if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
        let inner = &s[1..s.len() - 1];
        if inner.is_empty() { None } else { Some(inner) }
    } else {
        None
    }
}

/// Pure text scan of a Cargo.toml body — extracted so the
/// prefix-collision edge case (`integration-tests-slow = []` must not
/// match) can be exercised without touching the filesystem.
///
/// `line.starts_with("integration-tests")` after `[features]` and
/// before the next `[section]` header is sufficient — TOML arrays may
/// span lines but the feature declaration itself always starts with
/// the key on its own line.
fn manifest_declares_integration_tests_feature(manifest: &str) -> bool {
    let mut in_features = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed.starts_with("[features]");
            continue;
        }
        if in_features && trimmed.starts_with("integration-tests") {
            // Guard against `integration-tests-something = ...` by
            // requiring either `=`, whitespace, or `.` after the key.
            let after = trimmed.trim_start_matches("integration-tests");
            if after.chars().next().is_none_or(|c| c == '=' || c.is_whitespace() || c == '.') {
                return true;
            }
        }
    }
    false
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

/// Remove any pre-existing `mutants-summary.json` so an in-progress
/// run cannot leave a stale summary visible on disk. A prior run that
/// was killed mid-invocation — SIGKILL, crash, CI-runner timeout —
/// leaves its summary behind; a reader checking the file during the
/// next run sees that stale verdict and may act on it. The authoritative
/// verdict is "the summary written by the run that just finished" — so
/// the contract is: no summary on disk until the current run has
/// parsed cargo-mutants' outcomes and written its own.
///
/// Noop when the file is absent (fresh checkout, or the prior run
/// cleaned up). Any other I/O error propagates with context.
fn clear_stale_summary(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).wrap_err_with(|| format!("remove stale summary {}", path.display())),
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
    fn argv_no_features_flag_when_list_empty_and_auto_off() {
        // With auto_integration_tests=false and no user features,
        // no --features flag should appear at all. Diff mode and
        // workspace mode behave identically here.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope { auto_integration_tests: false, ..Scope::default() };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(!joined.contains("--features"), "got: {joined}");
    }

    #[test]
    fn argv_workspace_mode_empty_packages_auto_adds_qualified_features() {
        // Workspace mode (nightly full-corpus run) with empty packages
        // and auto_integration_tests=true now enables the feature for
        // every workspace crate that declares it — same rationale as
        // diff-scoped: the full corpus should see integration-gated
        // tests participate. Behaviour change vs. the pre-04-10 code,
        // which skipped the auto-add whenever packages was empty.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope { auto_integration_tests: true, ..Scope::default() };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(joined.contains("--features"), "expected --features; got: {joined}");
        assert!(
            joined.contains("overdrive-control-plane/integration-tests"),
            "expected qualified integration-tests feature; got: {joined}",
        );
    }

    #[test]
    fn argv_auto_integration_tests_adds_feature_when_package_declares_it() {
        // Run from the workspace root so `crates/<pkg>/Cargo.toml`
        // resolves. This test is coupled to the repo: when the
        // feature is removed from overdrive-control-plane the
        // assertion will flip, which is the correct signal.
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            packages: vec!["overdrive-control-plane".into()],
            auto_integration_tests: true,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(
            joined.contains("--features integration-tests"),
            "overdrive-control-plane declares the feature; auto-add \
             should have fired. got: {joined}"
        );
    }

    #[test]
    fn argv_auto_integration_tests_noop_when_package_does_not_declare_it() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            packages: vec!["overdrive-core".into()],
            auto_integration_tests: true,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(
            !joined.contains("integration-tests"),
            "overdrive-core does not declare the feature; auto-add \
             should not have fired. got: {joined}"
        );
    }

    #[test]
    fn argv_auto_integration_tests_does_not_duplicate_when_user_already_passed_it() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            packages: vec!["overdrive-control-plane".into()],
            features: vec!["integration-tests".into()],
            auto_integration_tests: true,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        // Exactly one integration-tests token, no duplicated comma
        // form like "integration-tests,integration-tests".
        assert_eq!(joined.matches("integration-tests").count(), 1, "got: {joined}");
    }

    #[test]
    fn argv_no_integration_tests_opt_out_respected() {
        let mode = Mode::Workspace { baseline_path: PathBuf::from("/ignored") };
        let scope = Scope {
            packages: vec!["overdrive-control-plane".into()],
            auto_integration_tests: false,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(&mode, &scope, Path::new("/tmp/xtask"), None);
        let joined = argv_str(&args);
        assert!(!joined.contains("integration-tests"), "got: {joined}");
    }

    #[test]
    fn crate_declares_integration_tests_feature_true_for_control_plane() {
        // Coupled to repo state — when the project convention reaches
        // every crate this test is the reminder to update its peers.
        let declared = crate_declares_integration_tests_feature("overdrive-control-plane")
            .expect("Cargo.toml readable");
        assert!(declared, "overdrive-control-plane declares `integration-tests`");
    }

    #[test]
    fn crate_declares_integration_tests_feature_false_for_core() {
        let declared = crate_declares_integration_tests_feature("overdrive-core")
            .expect("Cargo.toml readable");
        assert!(!declared, "overdrive-core does NOT declare `integration-tests`");
    }

    #[test]
    fn crate_declares_integration_tests_feature_handles_xtask_outside_crates_dir() {
        // xtask lives at `<root>/xtask/Cargo.toml`, not
        // `<root>/crates/xtask/Cargo.toml`. The fallback path must
        // locate it.
        let declared =
            crate_declares_integration_tests_feature("xtask").expect("Cargo.toml readable");
        assert!(declared, "xtask/Cargo.toml declares `integration-tests`");
    }

    #[test]
    fn crate_declares_integration_tests_feature_false_for_unknown_crate() {
        let declared = crate_declares_integration_tests_feature("no-such-crate-xyz")
            .expect("missing Cargo.toml yields Ok(false)");
        assert!(!declared);
    }

    #[test]
    fn crate_declares_integration_tests_feature_rejects_prefix_collisions() {
        // The scanner must not match a hypothetical feature like
        // `integration-tests-slow = []`. Exercise the pure string-level
        // helper directly so the test doesn't depend on filesystem or
        // cwd state.
        let manifest = "[package]\nname = \"fake\"\n\n\
                    [features]\nintegration-tests-slow = []\n";
        assert!(!manifest_declares_integration_tests_feature(manifest));

        // And a positive control so the test documents both sides.
        let manifest_ok = "[package]\nname = \"fake\"\n\n\
                       [features]\nintegration-tests = []\n";
        assert!(manifest_declares_integration_tests_feature(manifest_ok));

        // Feature outside a `[features]` table must not count — e.g.
        // a dependency named `integration-tests`.
        let not_under_features = "[package]\nname = \"fake\"\n\n\
                              [dependencies]\nintegration-tests = \"1\"\n";
        assert!(!manifest_declares_integration_tests_feature(not_under_features));
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
        let path = dir.path().join("mutants-summary.json");
        std::fs::write(&path, r#"{"status":"pass"}"#).expect("seed stale summary");
        assert!(path.exists(), "precondition — stale summary exists");

        clear_stale_summary(&path).expect("clear must succeed when file present");

        assert!(!path.exists(), "stale summary must be removed");
    }

    #[test]
    fn clear_stale_summary_is_noop_when_file_absent() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("mutants-summary.json");
        assert!(!path.exists(), "precondition — summary absent");

        clear_stale_summary(&path).expect("clear must succeed when file absent");

        assert!(!path.exists(), "file absent remains absent");
    }

    // ---- workspace member / feature discovery -------------------------------

    #[test]
    fn parse_workspace_members_handles_multiline_form() {
        let manifest = r#"
[workspace]
resolver = "3"
members = [
    "crates/overdrive-core",
    "crates/overdrive-cli",
    # a comment line
    "xtask",
]

[workspace.package]
edition = "2024"
"#;
        let members = parse_workspace_members(manifest);
        assert_eq!(
            members,
            vec![
                "crates/overdrive-core".to_owned(),
                "crates/overdrive-cli".to_owned(),
                "xtask".to_owned()
            ],
        );
    }

    #[test]
    fn parse_workspace_members_handles_single_line_form() {
        let manifest = r#"[workspace]
members = ["crates/a", "crates/b"]
resolver = "3"
"#;
        let members = parse_workspace_members(manifest);
        assert_eq!(members, vec!["crates/a".to_owned(), "crates/b".to_owned()]);
    }

    #[test]
    fn parse_workspace_members_ignores_other_sections_array_named_members() {
        let manifest = r#"
[features]
members = [
    "this is not a workspace members array",
]

[workspace]
members = ["xtask"]
"#;
        let members = parse_workspace_members(manifest);
        assert_eq!(members, vec!["xtask".to_owned()]);
    }

    #[test]
    fn workspace_crates_with_integration_tests_feature_finds_declaring_crates() {
        // Runs against the real repo's Cargo.toml via
        // `CARGO_MANIFEST_DIR`. Asserts the set of crates that declare
        // the feature — stable enough to keep in CI, and exactly the
        // crates the diff-scoped auto-enable needs to target.
        let found = workspace_crates_with_integration_tests_feature()
            .expect("workspace manifest must be readable");
        // Known-at-time-of-writing set. If a new crate declares the
        // feature (or an existing one drops it), this test fails
        // loudly — update the list and the commit message.
        assert_eq!(
            found,
            vec![
                "overdrive-cli".to_owned(),
                "overdrive-control-plane".to_owned(),
                "overdrive-store-local".to_owned(),
            ],
        );
    }

    // ---- build_cargo_mutants_args: diff-scoped integration-tests auto ------

    #[test]
    fn argv_diff_mode_auto_adds_qualified_integration_tests_features() {
        // Diff mode with auto_integration_tests=true and empty packages
        // must append `<pkg>/integration-tests` for each workspace
        // crate that declares the feature — no bare
        // `integration-tests` (which would fail to build against
        // crates that do not declare it).
        let mode = Mode::Diff { base: "origin/main".into() };
        let scope = Scope { auto_integration_tests: true, ..Scope::default() };
        let args = build_cargo_mutants_args(
            &mode,
            &scope,
            Path::new("/tmp/xtask"),
            Some(Path::new("/tmp/mutants.diff")),
        );
        let joined = argv_str(&args);

        assert!(joined.contains("--features"), "expected --features flag; got: {joined}");
        // Each declaring crate from the current workspace must appear
        // in the joined feature list.
        for pkg in ["overdrive-cli", "overdrive-control-plane", "overdrive-store-local"] {
            let qualified = format!("{pkg}/integration-tests");
            assert!(
                joined.contains(&qualified),
                "expected per-package feature `{qualified}` in argv; got: {joined}",
            );
        }
        // Must NOT contain the bare feature name alone — cargo would
        // reject it against crates that do not declare the feature.
        // Every `integration-tests` occurrence must be preceded by `/`
        // (the qualified `<pkg>/integration-tests` form).
        let mut remaining = joined.as_str();
        while let Some(idx) = remaining.find("integration-tests") {
            assert!(idx > 0, "`integration-tests` at argv start is bare; got: {joined}");
            let preceding = remaining.as_bytes()[idx - 1];
            assert_eq!(
                preceding as char, '/',
                "bare `integration-tests` must not appear (preceding char: {:?}); got: {joined}",
                preceding as char,
            );
            remaining = &remaining[idx + "integration-tests".len()..];
        }
    }

    #[test]
    fn argv_diff_mode_opt_out_suppresses_auto_integration_tests() {
        let mode = Mode::Diff { base: "origin/main".into() };
        let scope = Scope { auto_integration_tests: false, ..Scope::default() };
        let args = build_cargo_mutants_args(
            &mode,
            &scope,
            Path::new("/tmp/xtask"),
            Some(Path::new("/tmp/mutants.diff")),
        );
        let joined = argv_str(&args);
        assert!(
            !joined.contains("integration-tests"),
            "opt-out must suppress all integration-tests feature entries; got: {joined}",
        );
    }

    #[test]
    fn argv_package_scope_still_uses_bare_integration_tests_feature_name() {
        // Package-scoped runs keep the prior bare-feature behaviour
        // because cargo applies bare features only to the packages
        // under test — scoping is implicit.
        let mode = Mode::Diff { base: "origin/main".into() };
        let scope = Scope {
            packages: vec!["overdrive-control-plane".to_owned()],
            auto_integration_tests: true,
            ..Scope::default()
        };
        let args = build_cargo_mutants_args(
            &mode,
            &scope,
            Path::new("/tmp/xtask"),
            Some(Path::new("/tmp/mutants.diff")),
        );
        let joined = argv_str(&args);
        assert!(
            joined.contains(",integration-tests") || joined.ends_with("integration-tests"),
            "package-scoped run must pass bare `integration-tests`; got: {joined}",
        );
        // And must NOT also add the qualified form — cargo would
        // enable the feature twice, harmless but noisy.
        assert!(
            !joined.contains("overdrive-control-plane/integration-tests"),
            "package-scoped run must not duplicate as `<pkg>/integration-tests`; got: {joined}",
        );
    }
}
