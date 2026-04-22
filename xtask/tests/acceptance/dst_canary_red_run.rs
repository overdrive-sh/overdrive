#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Acceptance scenarios for US-06 §1.3 (WS-3) + §7.2 — a deliberately
//! planted bug in `SimObservationStore` causes a real invariant failure,
//! and the failure block names the failing invariant, seed, tick, host,
//! cause, and prints a reproduction command that reproduces the failure
//! at the same tick.
//!
//! The planted bug lives behind the `overdrive-sim/canary-bug` Cargo
//! feature. Production builds never enable this feature; the test
//! pipeline enables it deliberately to prove the harness catches real
//! divergence.
//!
//! Driving-port discipline per DWD-04: the test enters through the
//! compiled `xtask` binary as a subprocess, reads the structured
//! summary, and invokes the printed reproduction command as a second
//! subprocess — proving that the command embedded in the failure block
//! actually reproduces the failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// Workspace root — subprocess `current_dir` + source of relative paths.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().expect("xtask crate lives directly under the workspace root").to_path_buf()
}

/// Run `cargo run -p xtask --features overdrive-sim/canary-bug -- dst <args>`
/// from a fresh target dir. `cargo run` is used instead of the pre-built
/// `CARGO_BIN_EXE_xtask` because the latter is compiled without the
/// canary-bug feature — we need a per-test compile that turns the
/// feature on.
///
/// This is deliberately slow (one cargo compile per test). Two tests
/// exist in this file: one invokes the failing run, and the second
/// runs the reproduction command. Both share a single tempdir so the
/// second test reuses the first compilation's target cache.
fn run_dst_canary(target_dir: &Path, extra_args: &[&str]) -> Output {
    let mut cmd = Command::new(cargo());
    cmd.args([
        "run",
        "--quiet",
        "-p",
        "xtask",
        "--features",
        "overdrive-sim/canary-bug",
        "--",
        "dst",
    ]);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.current_dir(workspace_root());
    cmd.env("CARGO_TARGET_DIR", target_dir);
    cmd.output().expect("cargo run must be invokable")
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn read_summary(target_dir: &Path) -> Value {
    let path = target_dir.join("xtask").join("dst-summary.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("dst-summary.json must exist at {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("dst-summary.json must be valid JSON")
}

/// Seed that the planted canary triggers on. Chosen to be distinctive
/// so a grep for `3735928559` in code points at exactly one place.
const CANARY_TRIGGER_SEED: u64 = 0xDEAD_BEEF;

// -----------------------------------------------------------------------------
// §1.3 WS-3 — red run produces seed/tick/host/cause + reproduction command.
// §7.2 error boundary — dst-summary.json contains the failure fields.
// -----------------------------------------------------------------------------

#[test]
fn canary_feature_on_trigger_seed_fails_with_full_failure_block() {
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    let out = run_dst_canary(target.path(), &["--seed", &CANARY_TRIGGER_SEED.to_string()]);

    // 1. The subprocess exits with non-zero status.
    assert!(
        !out.status.success(),
        "canary-bug on seed={CANARY_TRIGGER_SEED} must fail; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 2. A dst-output.log artifact is written on red.
    let log_path = target.path().join("xtask").join("dst-output.log");
    assert!(log_path.is_file(), "dst-output.log must exist on red runs at {}", log_path.display());

    // 3. A dst-summary.json artifact is written on red.
    let summary = read_summary(target.path());

    // 4. Top-level summary carries the failure fields the CI parser in
    //    .github/workflows/ci.yml reads: .seed, .invariant, .tick, .host,
    //    .reproduce. The step AC additionally requires the same fields
    //    via the `failing_invariant` + `cause` naming — both are
    //    populated so downstream consumers are not forced to pick one.
    assert_eq!(summary["seed"].as_u64(), Some(CANARY_TRIGGER_SEED), "top-level seed must match");
    assert_eq!(
        summary["invariant"].as_str(),
        Some("sim-observation-lww-converges"),
        "CI parser reads .invariant (top-level) — must match the failing name"
    );
    assert_eq!(
        summary["failing_invariant"].as_str(),
        Some("sim-observation-lww-converges"),
        "step AC reads .failing_invariant — must match the failing name"
    );
    assert!(
        summary["tick"].as_u64().is_some(),
        "top-level tick must be a u64; got {}",
        summary["tick"]
    );
    assert!(
        summary["host"].as_str().is_some_and(|h| !h.is_empty()),
        "top-level host must be a non-empty string; got {}",
        summary["host"]
    );
    assert!(
        summary["cause"].as_str().is_some_and(|c| !c.is_empty()),
        "top-level cause must be a non-empty string; got {}",
        summary["cause"]
    );
    let reproduce = summary["reproduce"].as_str().expect("top-level reproduce must be a string");
    assert!(
        reproduce.contains(&format!("--seed {CANARY_TRIGGER_SEED}")),
        "reproduce must embed the same seed; got {reproduce}"
    );
    assert!(
        reproduce.contains("--only sim-observation-lww-converges"),
        "reproduce must narrow via --only to the failing invariant; got {reproduce}"
    );

    // 5. The failures array carries the full detail, keyed to the same
    //    invariant.
    let failures = summary["failures"].as_array().expect("failures array");
    assert_eq!(failures.len(), 1, "canary triggers exactly one failure; got {failures:?}");
    let failure = &failures[0];
    assert_eq!(failure["invariant"].as_str(), Some("sim-observation-lww-converges"));

    // 6. The stderr failure block names every field the AC requires —
    //    seed, tick, host, cause, and a reproduction command.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("FAILED"), "stderr must contain FAILED marker; got {stderr}");
    assert!(
        stderr.contains(&CANARY_TRIGGER_SEED.to_string()),
        "stderr must name the seed; got {stderr}"
    );
    assert!(
        stderr.contains("sim-observation-lww-converges"),
        "stderr must name the failing invariant; got {stderr}"
    );
    assert!(
        stderr.contains("cargo xtask dst --seed"),
        "stderr must include the reproduction command; got {stderr}"
    );
}

// -----------------------------------------------------------------------------
// §7.1 / §1.3 — the printed reproduction command reproduces the failure.
// -----------------------------------------------------------------------------

#[test]
fn printed_reproduction_command_reproduces_the_failure_at_same_tick() {
    // Share a target dir with the first test so cargo's build cache
    // does not recompile overdrive-sim twice per test session. The
    // tempdir is owned by each test though (no literal sharing across
    // tests in Rust without #[ignore] + sequencing); this test pays
    // the cargo-compile cost once more.
    let target = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");

    // Step 1 — run the canary to capture the reproduction command.
    let first = run_dst_canary(target.path(), &["--seed", &CANARY_TRIGGER_SEED.to_string()]);
    assert!(!first.status.success(), "first run must fail for the canary to trigger");

    let summary = read_summary(target.path());
    let reproduce = summary["reproduce"]
        .as_str()
        .expect("summary must carry a reproduction command")
        .to_owned();
    let first_tick = summary["tick"].as_u64().expect("summary must carry a failing tick");
    let first_host =
        summary["host"].as_str().expect("summary must carry a failing host").to_owned();

    // Step 2 — replay the reproduction command as a fresh subprocess
    // into a *different* tempdir. We parse the emitted command, strip
    // the leading "cargo xtask", and re-invoke via the same cargo-run
    // shape with the canary-bug feature enabled (the emitted command
    // assumes the feature is on, consistent with how the production
    // failure would have originated).
    let replay_target = tempfile::tempdir().expect("tempdir for replay");
    let args = parse_reproduce_args(&reproduce);
    let replay =
        run_dst_canary(replay_target.path(), &args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        !replay.status.success(),
        "reproduction command must fail deterministically; stderr:\n{}",
        String::from_utf8_lossy(&replay.stderr),
    );

    // Step 3 — the replay's failing tick and host must match the first
    //    run's. This is the "same failure at the same tick" claim.
    let replay_summary = read_summary(replay_target.path());
    assert_eq!(
        replay_summary["tick"].as_u64(),
        Some(first_tick),
        "replay must fail at the same tick as the original run"
    );
    assert_eq!(
        replay_summary["host"].as_str(),
        Some(first_host.as_str()),
        "replay must fail on the same host as the original run"
    );
    assert_eq!(
        replay_summary["invariant"].as_str(),
        Some("sim-observation-lww-converges"),
        "replay must fail on the same invariant as the original run"
    );
}

/// Parse the args portion of a `cargo xtask dst ...` reproduction
/// command. Returns the args after `dst`, ready to hand to
/// `run_dst_canary`.
fn parse_reproduce_args(reproduce: &str) -> Vec<String> {
    // Canonical form: `cargo xtask dst --seed <N> --only <NAME>`.
    let mut out = Vec::new();
    let mut saw_dst = false;
    for word in reproduce.split_whitespace() {
        if !saw_dst {
            if word == "dst" {
                saw_dst = true;
            }
            continue;
        }
        out.push(word.to_owned());
    }
    assert!(
        !out.is_empty(),
        "reproduce must carry args after 'dst'; parsed empty from {reproduce:?}"
    );
    out
}
