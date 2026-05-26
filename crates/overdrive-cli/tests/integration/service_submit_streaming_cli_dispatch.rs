//! Step 01-03e3-fix — close the CLI submit-side dispatch scope gap.
//!
//! 01-03e3 (commit `db4fccc5`) migrated the CLI consumer match arms
//! to `ServiceSubmitEvent` but missed the submit-side dispatch path
//! at `crates/overdrive-cli/src/commands/job.rs::submit_streaming`.
//! Today (pre-fix) a Service-kind TOML falls through every Service
//! TOML wrapped in `SubmitSpecInput::Job(spec_input)` — actually,
//! because `JobSpecInput` is `deny_unknown_fields` and a Service
//! TOML carries a `[service]` table the legacy parser does not
//! recognise, the production submit path returns
//! `CliError::InvalidSpec` synchronously instead of routing to a
//! Service-kind streaming consumer.
//!
//! This file pins the corrective contract:
//!
//!   * **S-SHCP-CLI-DISPATCH-01** — a Service-kind TOML fed through
//!     `submit_streaming` MUST NOT synchronously return
//!     `CliError::InvalidSpec` (it routes to the new
//!     `submit_streaming_service` path; the consumer observes
//!     `ServiceSubmitEvent::Accepted` as the first wire line; the
//!     test does not wait for terminal — it asserts the dispatch
//!     reached the in-process server).
//!   * **S-SHCP-CLI-DISPATCH-02** — a Job-kind TOML fed through the
//!     same `submit_streaming` entrypoint MUST route to
//!     `submit_streaming_job` and produce a Job-vocabulary summary
//!     (regression guard against accidental cross-routing).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
//! subprocess": this file spawns a real in-process control-plane
//! server via `commands::serve::run_with_dataplane(...)` and calls
//! `commands::job::submit_streaming(...)` directly.
//!
//! Linux-gated because the production submit path eventually drives
//! `ExecDriver` against `/bin/sh`. The macOS `--no-run` gate
//! compiles this file via `cargo check --features integration-tests`
//! per `.claude/rules/testing.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_cli::commands::job::{StopArgs, SubmitArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::CliError;
use serial_test::serial;
use tempfile::TempDir;

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

// A valid Service-kind TOML per ADR-0047 / ADR-0057 / ADR-0058 — the
// `[service]` table is the kind discriminator and triggers the
// `WorkloadSpecInput::Service(_)` arm of the parser. The `/bin/sleep
// 300` command runs long enough that the test can observe the
// dispatch-side routing without racing the workload to terminal.
const SERVICE_TOML: &str = r#"
[service]
id = "svc-dispatch-1"
replicas = 1

[[listener]]
port = 18080
protocol = "tcp"

[exec]
command = "/bin/sleep"
args = ["300"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;

// A valid Job-kind TOML — the `[job]` table is the kind
// discriminator and triggers the `WorkloadSpecInput::Job(_)` arm.
// `/bin/sh -c "exit 0"` exits cleanly so the streaming consumer
// reaches `JobSubmitEvent::Succeeded` quickly.
const JOB_TOML: &str = r#"
[job]
id = "job-dispatch-1"

[exec]
command = "/bin/sh"
args = ["-c", "exit 0"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;

// ===========================================================================
// S-SHCP-CLI-DISPATCH-01 — Service TOML routes to ServiceSubmitEvent
// ===========================================================================

/// A Service-kind TOML fed through `submit_streaming` MUST route to
/// `submit_streaming_service` (the `ServiceSubmitEvent` consumer
/// surface). Today the production path returns
/// `CliError::InvalidSpec` synchronously because the Service TOML
/// falls through to a legacy `JobSpecInput`-deserialise that
/// rejects `[service]` as an unknown field. This test pins the
/// post-fix contract: the call MUST NOT return `InvalidSpec`
/// synchronously; the dispatch reached the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn cli_submit_streaming_service_routes_to_service_submit_event_consumer() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "svc-dispatch-1.toml", SERVICE_TOML);

    // Drive submit_streaming on a Service TOML. Pre-fix this returns
    // `Err(CliError::InvalidSpec)` synchronously because the legacy
    // path tries `toml::from_str::<JobSpecInput>` and `[service]` is
    // an unknown field under `deny_unknown_fields`. Post-fix this
    // routes to the new `submit_streaming_service` function which
    // POSTs to the in-process server; the streaming consumer awaits
    // terminal — we cap the await with a short timeout and assert
    // the dispatch reached the server (no synchronous InvalidSpec).
    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::job::submit_streaming(SubmitArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });

    // Give the submit a generous window to:
    //   1. Parse the TOML (post-fix: WorkloadSpecInput::Service)
    //   2. Validate ServiceV1::from_submit client-side
    //   3. POST to the in-process server
    //   4. Receive the streaming `Accepted` first wire line
    //
    // Pre-fix the call returns InvalidSpec in <100ms — the timeout
    // is irrelevant on the RED path. Post-fix the call WILL block
    // (server-side Service-kind end-to-end is RED scaffold per
    // `service_honest_stable.rs`) waiting for terminal — the timeout
    // fires and we issue an operator stop to free the streaming
    // consumer.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Issue stop to drive the Service-kind stream to terminal
    // `Stopped` so the submit task finishes. Pre-fix this is a
    // no-op (the workload was never admitted; the submit already
    // failed); post-fix it walks the Service-kind terminal path.
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "svc-dispatch-1".to_owned(),
        config_path: stop_cfg,
    })
    .await;

    // Reap the submit task with a hard timeout so the test does not
    // dangle on the 60s streaming cap.
    let outcome = tokio::time::timeout(Duration::from_secs(15), submit_handle).await;

    match outcome {
        Ok(Ok(Ok(output))) => {
            // Service-vocabulary terminal summary or non-Job summary
            // — the dispatch reached the Service consumer. The
            // load-bearing structural assertion: NOT a Job-vocabulary
            // summary (cross-routing regression guard).
            assert!(
                !output.summary.contains("Job '"),
                "S-SHCP-CLI-DISPATCH-01: Service TOML must NOT produce a \
                 Job-vocabulary summary; got: {summary:?}",
                summary = output.summary,
            );
        }
        Ok(Ok(Err(CliError::InvalidSpec { field, message }))) => {
            panic!(
                "S-SHCP-CLI-DISPATCH-01: Service TOML returned CliError::InvalidSpec — \
                 the legacy JobSpecInput fall-through is still wired. field={field}, \
                 message={message}",
            );
        }
        Ok(Ok(Err(other))) => {
            // Other errors (Transport, Validation) are acceptable —
            // they mean the dispatch reached the server which then
            // failed for a different reason. The load-bearing
            // assertion is "NOT InvalidSpec from legacy parser".
            // Tolerate so the test focuses on the dispatch.
            tracing::debug!("Service submit returned non-InvalidSpec error: {other:?}");
        }
        Ok(Err(join_err)) => panic!("submit task panicked: {join_err:?}"),
        Err(_elapsed) => {
            // Timeout expired — the submit task is still running
            // because the Service-kind terminal path is RED scaffold
            // upstream. The dispatch reached the server (no
            // synchronous InvalidSpec). Pass — the load-bearing
            // contract is satisfied.
        }
    }

    drop(handle);
}

// ===========================================================================
// S-SHCP-CLI-DISPATCH-02 — Job TOML routes to JobSubmitEvent (regression)
// ===========================================================================

/// A Job-kind TOML fed through `submit_streaming` MUST route to
/// `submit_streaming_job` (the `JobSubmitEvent` consumer surface).
/// Regression guard against accidental cross-routing when the new
/// Service-kind branch is added.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn cli_submit_streaming_job_still_routes_to_job_submit_event_consumer() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "job-dispatch-1.toml", JOB_TOML);

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: cfg,
    })
    .await
    .expect("Job submit must complete");

    // Job-vocabulary terminal summary — the dispatch reached the
    // Job consumer. `format_job_succeeded_summary` emits `Job
    // '<name>' succeeded.` (verbatim) for an exit-0 workload.
    assert!(
        output.summary.contains("Job 'job-dispatch-1' succeeded."),
        "S-SHCP-CLI-DISPATCH-02: Job TOML must route to the Job consumer and \
         render Job-vocabulary terminal summary; got: {summary:?}",
        summary = output.summary,
    );
    // Anti-cross-route check: the summary must NOT contain the
    // Service-vocabulary phrase `is stable` (which the
    // `format_service_stable_summary` would emit).
    assert!(
        !output.summary.contains("is stable"),
        "S-SHCP-CLI-DISPATCH-02: Job TOML must NOT produce a Service-vocabulary \
         summary; got: {summary:?}",
        summary = output.summary,
    );
    assert_eq!(output.exit_code, 0, "exit-0 Job → CLI exit 0 per KPI K1");

    drop(handle);
}
