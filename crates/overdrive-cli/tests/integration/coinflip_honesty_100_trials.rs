//! S-02-09 — KPI K1 Tier-3 honesty test (100-trial Lima/ExecDriver
//! gate).
//!
//! Per slice 02 step 02-07 of `workload-kind-discriminator`: 100 trials
//! of `overdrive job submit examples/coinflip.toml` against the real
//! `ExecDriver` in Lima must show ≥99 trials where the CLI process
//! exit code equals the workload's kernel-observed exit code AND every
//! trial's terminal verdict line names the same exit code as the
//! kernel observed AND no trial's output contains the historical
//! false-positive substrings `"is running with"` or `"(took live)"`
//! (anti-scenario S-02-05 at the real-ExecDriver boundary).
//!
//! This test is the **load-bearing observability KPI** for the bug
//! under audit (RCA: B+C+D conjunction, see slice 02 distill notes).
//! Every prior slice's structural correctness is meaningless if K1
//! does not pass at this scale.
//!
//! ## Mechanics
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*: the test calls
//! `overdrive_cli::commands::job::submit_streaming` directly as a
//! Rust async function. No `Command::spawn`.
//!
//! The `examples/coinflip.toml` workload picks a pseudo-random branch
//! (`SUCCESS` or `ERROR`) per run via bash's `$RANDOM`. Trial-to-trial
//! variance in expected exit codes is normal — the K1 contract is
//! "for whatever exit code the workload's bash script chooses, the
//! CLI process exit equals it AND the terminal verdict line names it
//! correctly." Mid-distribution sanity check (informational, not
//! gated): both branches should be observed across 100 trials.
//!
//! Each trial uses a fresh job ID (`coinflip-NNN`) so the server's
//! `IntentStore` idempotency layer treats each trial as a distinct
//! submission. The `[exec]` body and `[resources]` block come straight
//! from `examples/coinflip.toml`.
//!
//! ## Telemetry
//!
//! The test writes a per-trial CSV to
//! `target/k1-telemetry/02-07-coinflip-100trials.csv` so the DELIVER
//! reviewer can audit the 100-trial distribution. Format:
//! `trial,cli_exit,kernel_exit,verdict_line` (one row per trial,
//! `verdict_line` is the parsed "exit code N" snippet, never the full
//! summary — the full summary is asserted on inline).
//!
//! ## Linux + Lima gate
//!
//! `#![cfg(target_os = "linux")]` ensures the macOS host compiles the
//! file under `--no-run` per `.claude/rules/testing.md` but does not
//! attempt to run it; the runtime path is `cargo xtask lima run --
//! cargo nextest run -p overdrive-cli --features integration-tests
//! -E 'test(coinflip_honesty)'`.

#![cfg(target_os = "linux")]

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use serial_test::serial;
use tempfile::TempDir;

/// Number of trials. Contractual per AC step 02-07; do NOT lower
/// to mask flakiness. K1 below threshold means the code is wrong,
/// not the test.
const TRIAL_COUNT: usize = 100;

/// Honesty threshold. Contractual per AC step 02-07 / KPI K1. Do NOT
/// lower to mask flakiness.
const HONESTY_THRESHOLD: usize = 99;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
    )
    .await
    .expect("serve::run");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

/// Per-trial coinflip TOML. Body comes from `examples/coinflip.toml`
/// (the AC-named SSOT) except `id = "coinflip-<NNN>"` per trial —
/// defeats `IntentStore` idempotency (each trial is structurally a
/// distinct submit).
///
/// Per fix-exit-observer-running-gate (Solution 1'): the action-shim's
/// `obs.write(Running)` is now structurally happens-before the
/// watcher's `ExitEvent` emission, so sub-millisecond-exit workloads
/// no longer race. No fixture-side workaround required.
fn coinflip_spec_for_trial(trial: usize) -> String {
    format!(
        r#"
[job]
id = "coinflip-{trial:03}"

[exec]
command = "/bin/bash"
args = [
    "-c",
    "if (( RANDOM % 2 )); then echo SUCCESS; exit 0; else echo ERROR >&2; exit 1; fi",
]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
    )
}

/// Parse the "exit code N" snippet from a streaming summary. Returns
/// `Some(N)` on a clean match, `None` if the summary does not contain
/// the canonical phrase. The streaming layer renders both
/// `JobSubmitEvent::Succeeded` and `JobSubmitEvent::Failed` with an
/// "exit code N" segment per slice 02 step 02-04 / 02-05.
fn parse_kernel_exit_from_summary(summary: &str) -> Option<i32> {
    // Find "exit code " and take digits up to the next non-digit char.
    let needle = "exit code ";
    let idx = summary.find(needle)?;
    let tail = &summary[idx + needle.len()..];
    let digit_end = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
    if digit_end == 0 {
        return None;
    }
    tail[..digit_end].parse::<i32>().ok()
}

/// Telemetry artifact path. `target/k1-telemetry/...` is gitignored
/// (target/ is workspace-wide gitignored). The DELIVER reviewer reads
/// this file to audit the 100-trial distribution.
///
/// Resolution: `CARGO_MANIFEST_DIR` is set at compile time to the
/// crate root (`crates/overdrive-cli`); the workspace root is its
/// great-grandparent (`crates/overdrive-cli` → `crates` →
/// `<workspace>`). The artifact lands at
/// `<workspace>/target/k1-telemetry/02-07-coinflip-100trials.csv`,
/// which is the deterministic path the AC names. Using
/// `CARGO_TARGET_TMPDIR` here would be brittle — nextest does not
/// always populate it, and the fallback to `std::env::temp_dir()`
/// silently ships the artifact to `/tmp/...` (or `/k1-telemetry/`
/// after `pop()`-walking past `/tmp`), where the DELIVER reviewer
/// cannot find it.
fn telemetry_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|crates| crates.parent())
        .expect("CARGO_MANIFEST_DIR points two levels under workspace root");
    let dir = workspace_root.join("target").join("k1-telemetry");
    std::fs::create_dir_all(&dir).expect("create telemetry dir");
    dir.join("02-07-coinflip-100trials.csv")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn s_02_09_k1_honesty_100_trials() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // Per-trial telemetry rows.
    let mut rows: Vec<(usize, i32, Option<i32>, String)> = Vec::with_capacity(TRIAL_COUNT);
    let mut honest = 0usize;
    let mut anti_scenario_violations: Vec<(usize, String)> = Vec::new();
    let mut missing_exit_code: Vec<(usize, String)> = Vec::new();

    for trial in 0..TRIAL_COUNT {
        let body = coinflip_spec_for_trial(trial);
        let spec_name = format!("coinflip-{trial:03}.toml");
        let spec_path = write_toml(tmp.path(), &spec_name, &body);

        let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: cfg.clone(),
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "S-02-09 trial {trial}: submit_streaming MUST complete end-to-end against \
                 the real ExecDriver — a failure here means the streaming pipeline broke \
                 at the real-ExecDriver boundary, not a workload-exit edge case. err={err:?}"
            )
        });

        // Anti-scenario S-02-05 at the real-ExecDriver boundary —
        // every trial's summary MUST NOT contain the historical
        // false-positive substrings. A single violation across 100
        // trials is a hard-fail because the structural fix from
        // 02-01 (no `ConvergedRunning` on `JobSubmitEvent`) is
        // supposed to make these substrings unreachable for Job kind
        // by construction.
        if output.summary.contains("is running with") || output.summary.contains("(took live)") {
            anti_scenario_violations.push((trial, output.summary.clone()));
        }

        let kernel_exit = parse_kernel_exit_from_summary(&output.summary);
        if kernel_exit.is_none() {
            missing_exit_code.push((trial, output.summary.clone()));
        }

        // K1: cli_exit_code == kernel_observed_exit_code.
        if Some(output.exit_code) == kernel_exit {
            honest += 1;
        }

        // Capture the "exit code N" verdict snippet for telemetry.
        // Renders `"exit code N"` if parsed, else `"<missing>"`.
        let verdict_snippet =
            kernel_exit.map_or_else(|| "<missing>".to_owned(), |code| format!("exit code {code}"));
        rows.push((trial, output.exit_code, kernel_exit, verdict_snippet));
    }

    // Cleanly shut the server down before failing assertions — keeps
    // the cgroup `AllocCleanup` Drop guard happy on the path where the
    // test panics. (`drop(handle)` would also work; explicit shutdown
    // is the project convention per `exec_spec_walking_skeleton.rs`.)
    handle.shutdown().await.expect("clean shutdown");

    // ── Telemetry write ────────────────────────────────────────────
    // Per AC: "the test records the per-trial (cli_exit, kernel_exit)
    // tuple to a deterministic artifact path so the DELIVER reviewer
    // can audit the 100-trial distribution."
    let telemetry = telemetry_path();
    let mut csv = String::with_capacity(TRIAL_COUNT * 32);
    csv.push_str("trial,cli_exit,kernel_exit,verdict_snippet\n");
    for (trial, cli_exit, kernel_exit, snippet) in &rows {
        let kernel = kernel_exit.map_or_else(|| "missing".to_owned(), |c| c.to_string());
        let _ = writeln!(csv, "{trial},{cli_exit},{kernel},{snippet}");
    }
    let _ = writeln!(
        csv,
        "# honest={honest}/{TRIAL_COUNT},threshold={HONESTY_THRESHOLD},anti_scenario_violations={},missing_exit_code={}",
        anti_scenario_violations.len(),
        missing_exit_code.len(),
    );
    std::fs::write(&telemetry, csv).expect("write telemetry CSV");

    // ── Assertions ─────────────────────────────────────────────────
    // 1. Anti-scenario S-02-05 at the real-ExecDriver boundary — ZERO
    //    occurrences across all 100 trials. The structural fix from
    //    02-01 makes these substrings unreachable for Job kind by
    //    construction; a violation here is a hard-fail diagnosable to
    //    a code regression, not a flaky observer race.
    assert!(
        anti_scenario_violations.is_empty(),
        "S-02-09 anti-scenario violation — Job-kind summary MUST NOT contain \
         'is running with' or '(took live)' on any trial. The structural fix \
         from step 02-01 is supposed to make this unreachable for Job kind. \
         Violations: {anti_scenario_violations:#?}"
    );

    // 2. Every trial's verdict line MUST name a parseable "exit code N"
    //    snippet. A summary missing this is a renderer regression — the
    //    streaming layer's projection of `JobSubmitEvent::Succeeded` /
    //    `JobSubmitEvent::Failed` is supposed to always produce this
    //    snippet.
    assert!(
        missing_exit_code.is_empty(),
        "S-02-09: every trial's summary MUST contain a parseable 'exit code N' \
         snippet. Missing on trials: {missing_exit_code:#?}"
    );

    // 3. KPI K1 — ≥ 99/100 trials must have cli_exit_code ==
    //    kernel_observed_exit_code. Below threshold = code is wrong;
    //    do NOT lower the threshold to mask flakiness.
    assert!(
        honest >= HONESTY_THRESHOLD,
        "S-02-09 KPI K1 honesty contract violated: only {honest}/{TRIAL_COUNT} trials \
         had cli_exit_code == kernel_observed_exit_code (threshold = {HONESTY_THRESHOLD}). \
         Telemetry written to: {telemetry:?}. Diagnose root cause; do NOT lower threshold. \
         Likely culprits: (a) streaming-wire close-vs-CLI-process-exit ordering race, \
         (b) CLI defaulting to exit 0 on transport-close-without-terminal where it should \
         exit non-zero, (c) ExitObserver writing wrong exit_code under signal/timing edge \
         cases. See AC step 02-07 implementation notes."
    );
}
