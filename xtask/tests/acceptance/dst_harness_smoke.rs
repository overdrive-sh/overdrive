#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Acceptance scenarios for US-06 §7.1 — `cargo xtask dst` harness
//! smoke tests.
//!
//! Each scenario invokes the compiled `xtask` binary as a subprocess
//! (driving-port discipline per DWD-04 / ADR-0005). The test asserts on
//! the observable outputs specified by ADR-0006: first-line seed,
//! `target/xtask/dst-output.log`, `target/xtask/dst-summary.json`, and
//! the `--only` filter.
//!
//! Covers scenarios 1, 4, 6 from §7.1 (the invariant-enum round-trip
//! from scenario 3 is in `crates/overdrive-sim/tests/invariant_roundtrip.rs`
//! because it is a property on the enum itself, not a subprocess test).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path to the compiled `xtask` binary for the current cargo
/// test invocation. `CARGO_BIN_EXE_xtask` is injected by Cargo when the
/// crate declares a `[[bin]]` of that name.
fn xtask_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

/// The workspace root — needed so `cargo xtask dst` writes its artifacts
/// to a predictable location under the per-run Cargo target dir.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().expect("xtask crate lives directly under the workspace root").to_path_buf()
}

/// Run `xtask dst <args>` from the workspace root and capture the output.
///
/// `CARGO_TARGET_DIR` is overridden to a per-test tempdir so scenarios
/// cannot accidentally see artifacts from each other — each subprocess
/// writes to its own `dst-output.log` / `dst-summary.json`.
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

/// Parse the `dst-summary.json` produced by an `xtask dst` run. Returns
/// the parsed `serde_json::Value` for assertion-friendly access.
fn read_summary(target_dir: &Path) -> serde_json::Value {
    let path = target_dir.join("xtask").join("dst-summary.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("dst-summary.json must exist at {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("dst-summary.json must be valid JSON")
}

/// The blessed invariant catalogue for DST runs. Keep in sync with
/// `Invariant::ALL` in `overdrive-sim` and with `EXPECTED_INVARIANTS`
/// in `dst_clean_clone_green.rs`. The first six are the walking-
/// skeleton catalogue; the next three landed in slice 4 alongside the
/// reconciler-primitive runtime (ADR-0013 §9); `broker-drain-order-is-
/// deterministic` landed with the eval-broker drain-determinism fix
/// (commit `bc95506`).
const EXPECTED_INVARIANTS: &[&str] = &[
    "single-leader",
    "intent-never-crosses-into-observation",
    "snapshot-roundtrip-bit-identical",
    "sim-observation-lww-converges",
    "replay-equivalent-empty-workflow",
    "entropy-determinism-under-reseed",
    "at-least-one-reconciler-registered",
    "duplicate-evaluations-collapse",
    "broker-drain-order-is-deterministic",
    "reconciler-is-pure",
    "intent-store-returns-caller-bytes",
];

// -----------------------------------------------------------------------------
// §7.1 scenario 1 — "The DST harness composes real LocalIntentStore with every Sim
// adapter" — end-to-end smoke: exits 0, artifacts exist, seed present.
// -----------------------------------------------------------------------------

#[test]
fn dst_with_fixed_seed_exits_zero_and_writes_artifacts() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &["--seed", "42"]);

    // The subprocess exits with status zero.
    assert!(
        out.status.success(),
        "dst must succeed on seed=42; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // Artifact 1: dst-output.log exists under target/xtask/.
    let log_path = target.path().join("xtask").join("dst-output.log");
    assert!(log_path.is_file(), "dst-output.log must be written at {}", log_path.display());

    // Artifact 2: dst-summary.json exists and is well-formed.
    let summary = read_summary(target.path());

    // The summary carries the seed we passed in.
    assert_eq!(summary["seed"].as_u64(), Some(42), "summary seed must echo --seed; got {summary}");

    // A non-empty invariants array (the catalogue ran to completion).
    // Named-set containment — pairs with the length check so both
    // silent shrinkage (missing name) and silent drift (new name we
    // didn't bless here) fail the test.
    let invariants = summary["invariants"].as_array().expect("invariants must be an array");
    assert_eq!(
        invariants.len(),
        EXPECTED_INVARIANTS.len(),
        "default catalogue size must match the blessed set; got {invariants:?}"
    );
    for expected in EXPECTED_INVARIANTS {
        assert!(
            invariants.iter().any(|e| e["name"].as_str() == Some(*expected)),
            "catalogue missing {expected}; got {invariants:?}"
        );
    }

    // Every entry has a canonical kebab-case name and status="pass" in
    // Phase 1 — this pins both the stub evaluator and the canonical
    // form on the wire. 06-02 will keep the same shape but may flip
    // some status values.
    for entry in invariants {
        let name = entry["name"].as_str().expect("invariant name must be string");
        assert!(!name.is_empty(), "invariant name must not be empty; got {entry}");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "invariant name must be kebab-case lowercase; got {name}"
        );
        assert_eq!(
            entry["status"].as_str(),
            Some("pass"),
            "Phase 1 stub evaluators must report pass; got {entry}"
        );
    }

    // The failures array is empty on a green run.
    assert_eq!(
        summary["failures"].as_array().map(Vec::len),
        Some(0),
        "green run must have empty failures; got {summary}"
    );

    // wall_clock_ms is present.
    assert!(
        summary["wall_clock_ms"].is_u64(),
        "summary must carry wall_clock_ms as a u64; got {summary}"
    );

    // git_sha and toolchain are captured (ADR-0006 failure-artifact
    // schema) — assert non-empty so a mutation that returns "" is
    // caught.
    let git_sha = summary["git_sha"].as_str().expect("git_sha must be present");
    assert!(!git_sha.is_empty(), "git_sha must be non-empty; got {summary}");
    let toolchain = summary["toolchain"].as_str().expect("toolchain must be present");
    assert!(
        toolchain.contains("rustc") || toolchain == "unknown",
        "toolchain must look like rustc output or 'unknown'; got {toolchain}"
    );
}

// -----------------------------------------------------------------------------
// §7.1 scenario 6 — "The seed is printed on the first line of every run"
// -----------------------------------------------------------------------------

#[test]
fn first_line_of_stdout_names_the_seed() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &["--seed", "7"]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let first_line = stdout.lines().next().expect("xtask dst must produce stdout");

    // First line format per ADR-0006: `dst: seed = <u64>` — but the
    // roadmap's scenario acceptance criterion is looser: "first line
    // names the seed." We match either shape as long as the seed appears
    // and the line starts with `seed:` or `dst: seed =`.
    assert!(
        first_line.starts_with("seed: ") || first_line.starts_with("dst: seed ="),
        "first line must start with 'seed: ' or 'dst: seed ='; got {first_line:?}"
    );
    assert!(first_line.contains('7'), "first line must include the seed (7); got {first_line:?}");
}

// Without --seed, the xtask generates a fresh seed from OS entropy and
// still prints it on line 1. Covered here because the scenario phrasing
// is "the first line of output names the seed used for THIS run" — a
// default run must also satisfy it.
#[test]
fn first_line_of_stdout_names_the_seed_when_random() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &[]);

    assert!(
        out.status.success(),
        "dst must succeed without --seed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let first_line = stdout.lines().next().expect("xtask dst must produce stdout");

    assert!(
        first_line.starts_with("seed: ") || first_line.starts_with("dst: seed ="),
        "first line must name the seed; got {first_line:?}"
    );

    // Parse the seed off line 1 and assert the JSON echoes the same.
    let seed_in_line: u64 = first_line
        .trim_start_matches("seed: ")
        .trim_start_matches("dst: seed = ")
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("first line must end with a numeric seed; got {first_line:?}"));

    let summary = read_summary(target.path());
    assert_eq!(
        summary["seed"].as_u64(),
        Some(seed_in_line),
        "the seed on stdout line 1 must match summary.seed"
    );
}

// Two back-to-back default runs must produce *different* seeds (with
// overwhelming probability) — catches a mutation that replaces
// `fresh_seed` with a constant. A constant-seed fresh_seed would make
// both runs show the same number on line 1; real OS entropy will not.
#[test]
fn two_default_runs_produce_different_seeds() {
    let target_a = tempfile::tempdir().expect("tempdir a");
    let target_b = tempfile::tempdir().expect("tempdir b");

    let seed_a = extract_first_line_seed(&run_dst(target_a.path(), &[]));
    let seed_b = extract_first_line_seed(&run_dst(target_b.path(), &[]));

    assert_ne!(
        seed_a, seed_b,
        "two default runs must not use the same seed (probability 2^-64 otherwise)"
    );
}

fn extract_first_line_seed(out: &Output) -> u64 {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first_line = stdout.lines().next().expect("dst must produce stdout");
    first_line
        .trim_start_matches("seed: ")
        .trim_start_matches("dst: seed = ")
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("first line must end with a numeric seed; got {first_line:?}"))
}

// -----------------------------------------------------------------------------
// §7.1 scenario 4 — "Passing --only narrows a run to a single named invariant"
// -----------------------------------------------------------------------------

#[test]
fn only_narrows_run_to_one_invariant() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &["--seed", "42", "--only", "single-leader"]);

    assert!(
        out.status.success(),
        "dst --only single-leader must succeed on seed=42; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    let summary = read_summary(target.path());
    let invariants = summary["invariants"].as_array().expect("invariants array");
    assert_eq!(
        invariants.len(),
        1,
        "--only must narrow to exactly one invariant; got {invariants:?}"
    );
    assert_eq!(
        invariants[0]["name"].as_str(),
        Some("single-leader"),
        "the only invariant must be the one we requested; got {invariants:?}"
    );
}

#[test]
fn only_with_unknown_invariant_exits_non_zero_and_reports_error() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst(target.path(), &["--seed", "42", "--only", "not-a-real-invariant"]);

    assert!(!out.status.success(), "--only with an unknown name must exit non-zero");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not-a-real-invariant"),
        "stderr must name the rejected input; got {stderr}"
    );
}
