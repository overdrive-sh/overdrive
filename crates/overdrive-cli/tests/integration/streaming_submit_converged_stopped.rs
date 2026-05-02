//! Regression — `SubmitEvent::ConvergedStopped` must terminate the
//! streaming consumer with exit code 0.
//!
//! RCA: `docs/feature/fix-converged-stopped-cli-arm/deliver/rca.md`.
//! Operator-visible bug: `overdrive job submit --stream` exited 2 with
//! `BodyDecode` ("streaming submit response closed without
//! ConvergedRunning or ConvergedFailed") when a concurrent
//! `overdrive job stop` reached the workload before `Running`. Root
//! cause: the streaming consumer's match fell through to `_ =>` for
//! `SubmitEvent::ConvergedStopped`, treating a *present-day terminal
//! event* as a forward-compat future variant.
//!
//! Test shape: spawn a real in-process control plane, submit a real
//! `/bin/sleep` spec via `submit_streaming`, concurrently issue a
//! `stop` from a sibling task, and assert the streaming consumer
//! returns `Ok` with `exit_code == 0`. The assertion that closes the
//! bug is `exit_code == 0` — current code returns
//! `Err(CliError::BodyDecode { ... })` which `.expect()` would unwrap
//! into a panic.
//!
//! Linux-gated — production `ExecDriver` requires real
//! `tokio::process::Command::spawn`. macOS dev runs via
//! `cargo xtask lima run --` per `crates/overdrive-cli/CLAUDE.md`.

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

/// Long-running `/bin/sleep` so the streaming submit observes a
/// `Running` transition before the concurrent `stop` lands. The
/// `ConvergedStopped` event the regression targets fires when the
/// reconciler converges the alloc to `Terminated` after the operator
/// stop intent.
const fn sleep_spec_toml() -> &'static str {
    r#"
id = "stoppable"
replicas = 1

[resources]
cpu_milli = 100
memory_bytes = 67108864

[exec]
command = "/bin/sleep"
args = ["300"]
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_submit_observes_converged_stopped_exit_0_on_concurrent_stop() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let spec_path = write_toml(server_tmp.path(), "stoppable.toml", sleep_spec_toml());

    // Drive the streaming submit on the test's task; spawn a sibling
    // task that issues `stop` after a short delay (long enough for the
    // streaming consumer to observe `Accepted` and at least one
    // `LifecycleTransition`, but not so long that `ConvergedRunning`
    // races the stop). The streaming consumer must observe
    // `ConvergedStopped` and return Ok with exit code 0.
    let submit_cfg = server_cfg.clone();
    let stop_cfg = server_cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });

    // Brief delay so the submit is in flight on the streaming bus
    // before the stop lands. The test does NOT need to be tight here
    // — `ConvergedStopped` fires whether stop arrives pre- or
    // post-`Running`; the regression is "any path that ends in
    // ConvergedStopped must yield exit 0", not a specific timing.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "stoppable".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle.await.expect("submit task must not panic").expect(
        "submit_streaming must return Ok on ConvergedStopped — \
             current code returns Err(BodyDecode) because the \
             streaming consumer falls through to the catch-all arm",
    );

    // The load-bearing assertion: ConvergedStopped maps to exit 0.
    assert_eq!(
        output.exit_code, 0,
        "ConvergedStopped must map to exit code 0; got {} for output {output:?}",
        output.exit_code,
    );

    // Terminal-reason / streaming-reason / streaming-error are all
    // None on the clean-stop path — these fields exist for the
    // ConvergedFailed surface only.
    assert!(
        output.terminal_reason.is_none(),
        "ConvergedStopped must not carry a terminal_reason; got {:?}",
        output.terminal_reason,
    );

    // Summary mentions the job id and the initiator label. Stop
    // intent flowing through the operator's CLI lands the
    // `StoppedBy::Operator` variant on the streaming bus today
    // (per the StopAllocation reconciler path); the renderer
    // produces a one-line summary naming the initiator.
    assert!(
        output.summary.contains("stoppable"),
        "summary must mention the job id; got: {}",
        output.summary,
    );

    // job_id / next_command keep the same shape as the other terminal
    // arms — same operator hint surface.
    assert_eq!(output.job_id, "stoppable");
    assert_eq!(output.next_command, "overdrive alloc status --job stoppable");

    handle.shutdown().await.expect("clean shutdown");
}
