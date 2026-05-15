//! Regression — Job-kind streaming submit observing operator `stop`
//! terminates with the `JobSubmitEvent::Stopped` arm and CLI exit 130
//! (SIGINT-equivalent). Post-ADR-0051 migration of the original
//! `SubmitEvent::ConvergedStopped` regression target.
//!
//! Original RCA: `docs/feature/fix-converged-stopped-cli-arm/deliver/rca.md`.
//! Operator-visible bug: `overdrive job submit --stream` exited 2 with
//! `BodyDecode` ("streaming submit response closed without
//! `ConvergedRunning` or `ConvergedFailed`) when a concurrent
//! `overdrive job stop` reached the workload before `Running`. Root
//! cause: the streaming consumer's match fell through to `_ =>` for
//! the terminal stop event, treating a present-day terminal event as
//! a forward-compat future variant.
//!
//! ADR-0051 migration note: pre-migration this test used a flat
//! `JobSpecInput` TOML which the server (with the `workload_kind: None
//! → Service` coercion default) routed through the Service-arm
//! `SubmitEvent` lane, where the bug fixed by the RCA above lived.
//! ADR-0051 OQ-8 deleted the coercion default; routing through the
//! Service lane now requires an explicit `[service]` section
//! (covered by 02-03c). This test migrates to `[job]` so it routes
//! via `submit_streaming_job` → `consume_stream_job`; the regression
//! shape is preserved (terminal stop event must not fall through to
//! the catch-all arm) at the Job-arm `JobSubmitEvent::Stopped`
//! handler, asserted via CLI exit 130 + `stopped by operator` summary
//! line.
//!
//! Test shape: spawn a real in-process control plane, submit a real
//! `[job]` + `/bin/sleep 300` spec via `submit_streaming`, wait for
//! the alloc to reach `Running`, concurrently issue a `stop` from a
//! sibling task, and assert the streaming consumer returns `Ok` with
//! `exit_code == 130` and the Job-kind stop summary.
//!
//! Linux-gated — production `ExecDriver` requires real
//! `tokio::process::Command::spawn`. macOS dev runs via
//! `cargo xtask lima run --` per `crates/overdrive-cli/CLAUDE.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_cli::commands::alloc::StatusArgs as AllocStatusArgs;
use overdrive_cli::commands::job::{StopArgs, SubmitArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::AllocStateWire;
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

/// Poll the alloc snapshot until at least one allocation for `job_id`
/// reaches `Running`. Mirrors the helper in `job_kind_streaming.rs` —
/// each integration test file is its own crate root so the helper
/// cannot be shared without an explicit module structure; inlining is
/// the simpler shape until a shared helper module lands.
async fn wait_for_alloc_running(config_path: &Path, job_id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = overdrive_cli::commands::alloc::status_snapshot(AllocStatusArgs {
            job: job_id.to_owned(),
            config_path: config_path.to_owned(),
        })
        .await;
        if let Ok(resp) = snapshot
            && resp.rows.iter().any(|r| matches!(r.state, AllocStateWire::Running))
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "alloc for job {job_id} did not reach Running within 10s",
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// `[job]` TOML — the kind-discriminator triggers `WorkloadSpec::Job`
/// per ADR-0047. Long-running `/bin/sleep` so the streaming submit
/// observes a `Running` transition before the concurrent `stop` lands.
/// The `JobSubmitEvent::Stopped` event the regression targets fires
/// when the reconciler converges the alloc to `Terminated` after the
/// operator stop intent.
const fn stoppable_job_spec_toml() -> &'static str {
    r#"
[job]
id = "stoppable"

[exec]
command = "/bin/sleep"
args = ["300"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_submit_observes_job_stopped_exit_130_on_concurrent_stop() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let spec_path = write_toml(server_tmp.path(), "stoppable.toml", stoppable_job_spec_toml());

    // Drive the streaming submit on a sibling task; this test's task
    // waits for the alloc to reach `Running` before issuing `stop`
    // (replaces the legacy 150ms sleep race — same shape as the
    // `wait_for_alloc_running` pattern in `job_kind_streaming.rs`).
    let submit_cfg = server_cfg.clone();
    let stop_cfg = server_cfg.clone();
    let wait_cfg = server_cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });

    wait_for_alloc_running(&wait_cfg, "stoppable").await;

    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "stoppable".to_owned(),
        config_path: stop_cfg,
    })
    .await
    .expect("stop must succeed");

    let output = submit_handle
        .await
        .expect("submit task must not panic")
        .expect("submit_streaming must return Ok on JobSubmitEvent::Stopped");

    // The load-bearing assertion: operator-stopped Job maps to CLI
    // exit 130 (SIGINT-equivalent) per `consume_stream_job`'s
    // `JobSubmitEvent::Stopped` handler. The original
    // `SubmitEvent::ConvergedStopped` arm mapped to exit 0; the
    // Job-kind redesign per ADR-0047 §3 [D7] differentiates
    // operator-stopped (130) from clean-exit (0) so the operator's
    // signal is honest about WHY the workload ended.
    assert_eq!(
        output.exit_code, 130,
        "operator-stopped Job-kind must map to CLI exit code 130; got {} for output {output:?}",
        output.exit_code,
    );

    // Job-kind terminal events do not carry the streaming/terminal
    // reason fields — those exist on the legacy Service-arm
    // `SubmitEvent::ConvergedFailed` path only.
    assert!(
        output.terminal_reason.is_none(),
        "Job-kind Stopped must not carry terminal_reason; got {:?}",
        output.terminal_reason,
    );

    // Summary mentions the job id, the Job vocabulary, and names
    // the initiator. `format_job_stopped_summary` is the SSOT —
    // form: `Job '<name>' stopped by <initiator>. (took ..., attempts ...)`.
    assert!(
        output.summary.contains("Job 'stoppable'"),
        "summary must contain Job 'stoppable'; got: {}",
        output.summary,
    );
    assert!(
        output.summary.contains("stopped by operator"),
        "summary must contain `stopped by operator`; got: {}",
        output.summary,
    );

    // Anti-scenario — Service vocabulary must be structurally
    // unreachable on the Job-kind streaming path.
    assert!(
        !output.summary.contains("Service 'stoppable'"),
        "Job-kind summary must NOT contain Service vocabulary; got: {}",
        output.summary,
    );

    // workload_id / next_command keep the same shape as the other terminal
    // arms — same operator hint surface.
    assert_eq!(output.workload_id, "stoppable");
    assert_eq!(output.next_command, "overdrive alloc status --job stoppable");

    handle.shutdown().await.expect("clean shutdown");
}
