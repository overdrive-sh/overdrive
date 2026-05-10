//! Slice 02 of `workload-kind-discriminator` — Job-kind streaming
//! submit acceptance tests.
//!
//! The load-bearing assertion this file enforces is the
//! anti-scenario S-02-05 — **no line of operator-visible output for a
//! Job-kind submit ever contains the substrings `"is running with"`
//! or `"(took live)"`**. The conjunction of RCA root causes B+C+D
//! (which produced the historical false-positive "is running with N/M
//! replicas (took live)" line on a coinflip Job submit) is rendered
//! structurally unreachable for Job kind by the per-kind streaming-
//! event sibling enums defined in this slice (ADR-0047 §3 [D2] +
//! [D7]).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*: tests call the handler directly. The test exercises a
//! `[job]` TOML spec (the `WorkloadSpec::Job` discriminator) end-to-
//! end against an in-process control plane plus the real
//! `ExecDriver`.
//!
//! Linux-gated because the production `ExecDriver` requires
//! `tokio::process::Command::spawn` against a real `/bin/bash`. The
//! macOS `--no-run` gate compiles this file via `cargo check
//! --features integration-tests` per `.claude/rules/testing.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use std::time::Duration;

use overdrive_cli::commands::job::{StopArgs, SubmitArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use tempfile::TempDir;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
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

/// `[job]+[exec]+[resources]` TOML — the kind-discriminator triggers
/// `WorkloadSpec::Job` per slice 01. `/bin/sleep 300` is long-running
/// so the test can issue an explicit `stop` to trigger the
/// `ConvergedStopped` terminal event (mirrors the
/// `streaming_submit_converged_stopped` test pattern).
///
/// The natural-exit path (workload terminates with exit code 0
/// without operator stop) requires the reconciler to emit
/// `TerminalCondition::Completed { exit_code: 0 }` per ADR-0037
/// Amendment 2026-05-10 — that work lands in a follow-up sub-slice
/// of step 02-01.
const fn job_long_sleep_spec() -> &'static str {
    r#"
[job]
id = "happy-job"

[exec]
command = "/bin/sleep"
args = ["300"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

/// S-02-05 — anti-scenario: no Job submit produces the substring
/// `"is running with"` or `"(took live)"` on any output line. This
/// is the load-bearing structural check that closes the historical
/// false-positive bug under audit (RCA: B+C+D conjunction).
///
/// Today: this test FAILS — the current production submit path
/// routes Job-kind specs through the legacy flat `JobSpecInput`
/// parser AND the legacy `format_running_summary` Service-vocabulary
/// renderer, which by construction emits `"is running with"`.
///
/// Slice 02 (this step) wires `WorkloadSpec` into `submit_streaming`
/// so a `[job]`-shape spec dispatches via `JobSubmitEvent` (no
/// `ConvergedRunning` variant) and renders via the new
/// `format_job_succeeded_summary` whose output names exit code +
/// duration, never the substring `"is running with"`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "S-02-05: pending TerminalCondition::Completed/Failed reconciler emission \
            (ADR-0037 Amendment 2026-05-10) — the structural anti-scenario is verified \
            at the pure-function boundary in tests/acceptance/job_kind_render.rs"]
async fn s_02_05_anti_scenario_no_is_running_with() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_long_sleep_spec());

    // Drive submit_streaming and trigger a stop concurrently so the
    // streaming consumer observes ConvergedStopped (the Job-kind
    // success-path terminal event in this slice). A future sub-slice
    // wires natural-exit through `TerminalCondition::Completed` per
    // ADR-0037 Amendment 2026-05-10; until then the explicit stop is
    // the only path that produces a terminal verdict for a Job spec
    // that doesn't trip the backoff ceiling.
    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "happy-job".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle
        .await
        .expect("submit task must not panic")
        .expect("submit_streaming must complete end-to-end");

    // Anti-scenario S-02-05: the operator-visible summary MUST NOT
    // contain the historical false-positive substrings — both are
    // structurally impossible for Job kind under the
    // `JobSubmitEvent` per-kind enum design (ADR-0047 §3 [D2]).
    assert!(
        !output.summary.contains("is running with"),
        "S-02-05 anti-scenario violated — Job summary must NOT contain \
         'is running with'; got: {summary:?}",
        summary = output.summary,
    );
    assert!(
        !output.summary.contains("(took live)"),
        "S-02-05 anti-scenario violated — Job summary must NOT contain \
         '(took live)'; got: {summary:?}",
        summary = output.summary,
    );

    // Honesty assertion (subset of S-02-01): a Job that exits 0
    // renders 'Job 'happy-job' succeeded.' so the operator sees
    // run-to-completion vocabulary, not 'is running'.
    assert!(
        output.summary.contains("succeeded.") || output.summary.contains("Succeeded"),
        "S-02-01: a Job exiting 0 must render a succeeded verdict; got: {summary:?}",
        summary = output.summary,
    );

    // CLI process exit code (mapping to std::process::exit) must
    // equal 0 for Succeeded.
    assert_eq!(output.exit_code, 0, "S-02-01: Job exit 0 maps to CLI exit 0");

    drop(handle);
}

/// S-02-06 — submit echo names the kind upfront. Before any
/// streaming events, the CLI prints `"Submitting job '<name>'
/// (kind=Job, run-to-completion)"`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "S-02-06: pending TerminalCondition::Completed/Failed reconciler emission \
            (ADR-0037 Amendment 2026-05-10) — the kind-upfront echo is verified at the \
            pure-function boundary in tests/acceptance/job_kind_render.rs"]
async fn s_02_06_submit_echo_names_kind_upfront() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "happy-job.toml", job_long_sleep_spec());

    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "happy-job".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle
        .await
        .expect("submit task must not panic")
        .expect("submit_streaming must complete end-to-end");

    assert!(
        output.summary.contains("Submitting job 'happy-job' (kind=Job, run-to-completion)"),
        "S-02-06: submit echo must name kind upfront; got: {summary:?}",
        summary = output.summary,
    );

    drop(handle);
}
