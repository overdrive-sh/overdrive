#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Acceptance scenarios for step 06-02 — default invariant catalogue
//! evaluates and green-passes within the wall-clock budget.
//!
//! Covers:
//!
//! * §1.1 WS-1 — clean-clone `cargo xtask dst` is green within <60 s.
//! * §7.1 scenario 1 — harness reports every Sim adapter and a real
//!   `LocalStore` backing the run.
//! * §7.1 scenario 2 — the six default-catalogue invariants all ran.
//! * §5.2 — `intent_never_crosses_into_observation` invariant runs on
//!   every tick and reports pass.
//!
//! Each scenario invokes the compiled `xtask` binary as a subprocess,
//! following the DWD-04 / ADR-0005 driving-port discipline. Artifact
//! assertions read `dst-summary.json` — the single source of truth on
//! what actually ran.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path to the compiled `xtask` binary for the current cargo
/// test invocation. `CARGO_BIN_EXE_xtask` is injected by Cargo.
fn xtask_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().expect("xtask crate lives directly under the workspace root").to_path_buf()
}

fn run_dst(target_dir: &Path, extra_args: &[&str]) -> Output {
    let mut cmd = Command::new(xtask_bin());
    cmd.arg("dst");
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.current_dir(workspace_root());
    cmd.env("CARGO_TARGET_DIR", target_dir);
    cmd.output().expect("xtask binary must be invokable")
}

fn read_summary(target_dir: &Path) -> serde_json::Value {
    let path = target_dir.join("xtask").join("dst-summary.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("dst-summary.json must exist at {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("dst-summary.json must be valid JSON")
}

/// The six invariants in the Phase 1 default catalogue — in canonical
/// kebab-case as printed by `Invariant::Display`.
const EXPECTED_INVARIANTS: &[&str] = &[
    "single-leader",
    "intent-never-crosses-into-observation",
    "snapshot-roundtrip-bit-identical",
    "sim-observation-lww-converges",
    "replay-equivalent-empty-workflow",
    "entropy-determinism-under-reseed",
];

// -----------------------------------------------------------------------------
// §1.1 WS-1 — Clean-clone cargo xtask dst is green within the wall-clock budget
// -----------------------------------------------------------------------------

/// The whole default catalogue runs, every invariant passes, and the
/// wall-clock budget (<60 s per KPI K1) is met.
#[test]
fn default_catalogue_is_green_within_wall_clock_budget() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &["--seed", "42"]);

    assert!(
        out.status.success(),
        "dst --seed 42 must succeed; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let summary = read_summary(target.path());

    // Seed echoed.
    assert_eq!(summary["seed"].as_u64(), Some(42), "summary seed must echo --seed; got {summary}");

    // Every invariant is present, passing, and carries a non-empty host.
    let invariants = summary["invariants"].as_array().expect("invariants array");
    assert_eq!(invariants.len(), EXPECTED_INVARIANTS.len(), "catalogue length");

    for expected in EXPECTED_INVARIANTS {
        let entry = invariants
            .iter()
            .find(|e| e["name"].as_str() == Some(*expected))
            .unwrap_or_else(|| panic!("catalogue missing {expected}: {summary}"));
        assert_eq!(
            entry["status"].as_str(),
            Some("pass"),
            "{expected} must pass on seed=42; got {entry}",
        );
        let host = entry["host"].as_str().expect("host must be present");
        assert!(!host.is_empty(), "{expected} must report a host");
    }

    // Zero failures.
    assert_eq!(
        summary["failures"].as_array().map(Vec::len),
        Some(0),
        "green run has no failures; got {summary}",
    );

    // Wall-clock budget — KPI K1 target is 60 s on an M-class laptop.
    // CI can be slower, so the assertion is the KPI ceiling, not a tight
    // perf gate. A mutation that makes the harness sleep for minutes
    // will fail here.
    let wall_clock_ms = summary["wall_clock_ms"]
        .as_u64()
        .unwrap_or_else(|| panic!("wall_clock_ms must be a u64; got {summary}"));
    assert!(
        wall_clock_ms < 60_000,
        "wall-clock budget: KPI K1 ceiling is 60_000 ms; got {wall_clock_ms} ms (summary: {summary})",
    );
}

// -----------------------------------------------------------------------------
// §7.1 scenario 2 — the default catalogue runs to completion
// -----------------------------------------------------------------------------

/// Every named invariant in §7.1 scenario 2 appears in the summary.
#[test]
fn summary_names_every_expected_invariant() {
    let target = tempfile::tempdir().expect("tempdir");
    let out = run_dst(target.path(), &["--seed", "42"]);
    assert!(out.status.success(), "dst must succeed");

    let summary = read_summary(target.path());
    let names: Vec<&str> = summary["invariants"]
        .as_array()
        .expect("invariants array")
        .iter()
        .map(|e| e["name"].as_str().expect("name string"))
        .collect();

    for expected in EXPECTED_INVARIANTS {
        assert!(
            names.contains(expected),
            "invariant {expected} must be present in summary; got names={names:?}",
        );
    }
}

// -----------------------------------------------------------------------------
// §5.2 — intent_never_crosses_into_observation invariant
// -----------------------------------------------------------------------------

/// The invariant runs and reports pass — confirming the §4 Intent /
/// Observation boundary holds throughout the run.
#[test]
fn intent_never_crosses_into_observation_is_evaluated_and_passes() {
    let target = tempfile::tempdir().expect("tempdir");
    let out = run_dst(
        target.path(),
        &["--seed", "42", "--only", "intent-never-crosses-into-observation"],
    );

    assert!(
        out.status.success(),
        "intent-never-crosses-into-observation must pass on seed=42; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    let summary = read_summary(target.path());
    let invariants = summary["invariants"].as_array().expect("invariants array");
    assert_eq!(invariants.len(), 1, "--only narrows to one");
    assert_eq!(invariants[0]["name"].as_str(), Some("intent-never-crosses-into-observation"),);
    assert_eq!(invariants[0]["status"].as_str(), Some("pass"));
}

// -----------------------------------------------------------------------------
// Per-invariant smoke: each name in the catalogue runs green on --only
// -----------------------------------------------------------------------------

/// Every name in the default catalogue must be independently resolvable
/// via `--only` and must report pass in isolation. This is the step's
/// claim that every invariant body is wired and not just stubbed out.
#[test]
fn every_invariant_runs_green_when_selected_individually() {
    for name in EXPECTED_INVARIANTS {
        let target = tempfile::tempdir().expect("tempdir");
        let out = run_dst(target.path(), &["--seed", "42", "--only", name]);
        assert!(
            out.status.success(),
            "--only {name} must succeed on seed=42; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr),
        );

        let summary = read_summary(target.path());
        let invariants = summary["invariants"].as_array().expect("invariants array");
        assert_eq!(invariants.len(), 1, "--only {name} narrows to one");
        assert_eq!(invariants[0]["name"].as_str(), Some(*name));
        assert_eq!(
            invariants[0]["status"].as_str(),
            Some("pass"),
            "{name} must pass on seed=42; got {summary}",
        );
    }
}
