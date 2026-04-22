#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Acceptance scenarios for US-06 §1.2 (WS-2) — `cargo xtask dst --seed S`
//! produces a bit-identical trajectory across two runs.
//!
//! Also covers §7.3 — the twin-run self-test proptest that exercises the
//! bit-identical property across 16 seeds drawn from the DST seed
//! generator (the step 06-03 acceptance criterion).
//!
//! Driving-port discipline per DWD-04: the test enters the subprocess
//! boundary for the per-run comparison; the proptest uses the
//! `overdrive-sim` library surface directly so the 16-seed loop runs in
//! tens of milliseconds rather than minutes.
//!
//! Equality excludes `wall_clock_ms` — that is real wall-clock time and
//! varies across runs. Every other field (`seed`, `invariants` array
//! shape and contents, `failures` array, `git_sha`, `toolchain`) must be
//! byte-identical under a stable toolchain.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use proptest::prelude::*;
use serde_json::Value;

/// Absolute path to the compiled `xtask` binary.
fn xtask_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

/// Workspace root — needed so subprocess `current_dir` is stable.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().expect("xtask crate lives directly under the workspace root").to_path_buf()
}

/// Run `xtask dst <args>` with a fresh tempdir for `CARGO_TARGET_DIR` so
/// two back-to-back runs do not see each other's artifacts.
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

fn read_summary(target_dir: &Path) -> Value {
    let path = target_dir.join("xtask").join("dst-summary.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("dst-summary.json must exist at {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("dst-summary.json must be valid JSON")
}

/// Remove non-deterministic fields from a summary so two runs with the
/// same seed compare byte-identical. `wall_clock_ms` is real wall-clock
/// time; everything else is pinned by the seed.
fn strip_nondeterministic(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("wall_clock_ms");
    }
    v
}

// -----------------------------------------------------------------------------
// §1.2 WS-2 — Same seed reproduces bit-identical trajectory across two runs.
// -----------------------------------------------------------------------------

#[test]
fn two_subprocess_runs_with_same_seed_produce_bit_identical_trajectory() {
    let target_a = tempfile::tempdir().expect("tempdir a");
    let target_b = tempfile::tempdir().expect("tempdir b");

    // Both runs use the exact same seed — the determinism claim.
    let out_a = run_dst(target_a.path(), &["--seed", "42"]);
    let out_b = run_dst(target_b.path(), &["--seed", "42"]);

    assert!(
        out_a.status.success(),
        "run a must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out_a.stderr),
    );
    assert!(
        out_b.status.success(),
        "run b must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out_b.stderr),
    );

    // First line of stdout — the seed — must match across both runs.
    let stdout_a = String::from_utf8(out_a.stdout).expect("valid utf8 stdout a");
    let stdout_b = String::from_utf8(out_b.stdout).expect("valid utf8 stdout b");
    let line_a = stdout_a.lines().next().expect("run a must produce stdout");
    let line_b = stdout_b.lines().next().expect("run b must produce stdout");
    assert_eq!(line_a, line_b, "first-line seed must match across two same-seed runs");
    assert!(line_a.contains("42"), "first line must contain the seed 42; got {line_a:?}");

    // The structured summary — stripped of wall_clock_ms — must match.
    let summary_a = strip_nondeterministic(read_summary(target_a.path()));
    let summary_b = strip_nondeterministic(read_summary(target_b.path()));
    assert_eq!(
        summary_a, summary_b,
        "dst-summary.json (minus wall_clock_ms) must match across two same-seed runs"
    );

    // Double-check the two pieces of the AC in isolation so a
    // regression on one doesn't hide behind the other:
    //   - Ordered list of invariant names matches.
    //   - Per-invariant tick numbers match.
    let invariants_a = summary_a["invariants"].as_array().expect("invariants array a");
    let invariants_b = summary_b["invariants"].as_array().expect("invariants array b");
    assert_eq!(
        invariants_a.len(),
        invariants_b.len(),
        "same seed must produce the same invariant count"
    );
    for (a, b) in invariants_a.iter().zip(invariants_b.iter()) {
        assert_eq!(a["name"], b["name"], "invariant names must match in order");
        assert_eq!(a["tick"], b["tick"], "per-invariant tick numbers must match");
        assert_eq!(a["host"], b["host"], "per-invariant host must match");
        assert_eq!(a["status"], b["status"], "per-invariant status must match");
    }
}

// -----------------------------------------------------------------------------
// §7.3 — Twin-run identity holds for any seed drawn from the DST seed
//       generator (proptest over 16 seeds).
//
// This proptest runs the harness *in-process* (via the library surface)
// rather than via subprocess — 16 seeds × two subprocess spawns would
// dominate wall-clock on laptops. The in-process path proves the same
// determinism claim at the library boundary where `cargo xtask dst`
// consumes it.
// -----------------------------------------------------------------------------

/// Canonical representation of a `RunReport` with non-deterministic
/// fields stripped. Two `RunReport`s produced by same-seed runs must be
/// canonically equal.
#[derive(Debug, PartialEq, Eq)]
struct Canonical {
    seed: u64,
    invariants: Vec<(String, String, u64, String, Option<String>)>,
    failures: Vec<(String, u64, String, String)>,
}

impl Canonical {
    fn from_report(r: &overdrive_sim::RunReport) -> Self {
        Self {
            seed: r.seed,
            invariants: r
                .invariants
                .iter()
                .map(|i| {
                    (
                        i.name.clone(),
                        i.status.as_str().to_owned(),
                        i.tick,
                        i.host.clone(),
                        i.cause.clone(),
                    )
                })
                .collect(),
            failures: r
                .failures
                .iter()
                .map(|f| (f.invariant.clone(), f.tick, f.host.clone(), f.cause.clone()))
                .collect(),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        // 16 cases per the AC — "proptest over 16 seeds."
        cases: 16,
        // Keep default source-of-randomness for the seed generator —
        // each property case generates one seed for the inner twin-run.
        .. ProptestConfig::default()
    })]

    #[test]
    fn twin_run_identity_holds_for_any_seed(seed in any::<u64>()) {
        // Two independent runs of the harness with the same seed.
        // Fresh `Harness::new()` instances — no shared state between them.
        let a = overdrive_sim::Harness::new()
            .run(seed)
            .expect("harness must compose for run a");
        let b = overdrive_sim::Harness::new()
            .run(seed)
            .expect("harness must compose for run b");

        // Canonical equality — wall_clock is stripped because it is real
        // wall-clock time and varies across runs.
        prop_assert_eq!(
            Canonical::from_report(&a),
            Canonical::from_report(&b),
            "two runs with seed={} must produce bit-identical reports",
            seed
        );
    }
}
