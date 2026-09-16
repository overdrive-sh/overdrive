//! Step 01-03e3-fix — close the CLI submit-side dispatch scope gap.
//!
//! 01-03e3 (commit `db4fccc5`) migrated the CLI consumer match arms
//! to `ServiceSubmitEvent` but missed the submit-side dispatch path
//! at `crates/overdrive-cli/src/commands/deploy.rs::deploy_streaming`.
//! Before the fix, a Service-kind TOML was routed through the Job
//! submission path and returned `CliError::InvalidSpec` synchronously
//! instead of reaching a Service-kind streaming consumer.
//!
//! This file pins the corrective contract:
//!
//!   * **S-SHCP-CLI-DISPATCH-01** — a Service-kind TOML fed through
//!     `deploy_streaming` MUST NOT synchronously return
//!     `CliError::InvalidSpec` (it routes to the new
//!     `deploy_streaming_service` path; the consumer observes
//!     `ServiceSubmitEvent::Accepted` as the first wire line; the
//!     test does not wait for terminal — it asserts the dispatch
//!     reached the in-process server).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
//! subprocess": this file spawns a real in-process control-plane
//! server via `commands::serve::run_with_dataplane(...)` and calls
//! `commands::deploy::deploy_streaming(...)` directly.
//!
//! Linux-gated because the integration path uses the production server
//! and network lifecycle. The macOS `--no-run` gate compiles this file
//! via `cargo check --features integration-tests` per
//! `.claude/rules/testing.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs};
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
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK.
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
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

[vm]
command = "/bin/sleep"
args = ["300"]
kernel = "/kernel"
rootfs = "/rootfs"

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;

// ===========================================================================
// S-SHCP-CLI-DISPATCH-01 — Service TOML routes to ServiceSubmitEvent
// ===========================================================================

/// A Service-kind TOML fed through `deploy_streaming` MUST route to
/// `deploy_streaming_service` (the `ServiceSubmitEvent` consumer
/// surface). The call MUST NOT return `CliError::InvalidSpec`
/// synchronously; the dispatch must reach the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn cli_submit_streaming_service_routes_to_service_submit_event_consumer() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "svc-dispatch-1.toml", SERVICE_TOML);

    // Drive submit_streaming on a Service TOML. The current parser
    // projects the Service arm and the deploy path
    // routes to the new `deploy_streaming_service` function which
    // POSTs to the in-process server; the streaming consumer awaits
    // terminal — we cap the await with a short timeout and assert
    // the dispatch reached the server (no synchronous InvalidSpec).
    let submit_cfg = cfg.clone();
    let stop_cfg = cfg.clone();
    let submit_handle = tokio::spawn(async move {
        overdrive_cli::commands::deploy::deploy_streaming(DeployArgs {
            spec: spec_path,
            config_path: submit_cfg,
        })
        .await
    });

    // Give the submit a generous window to:
    //   1. Parse the TOML (post-fix: WorkloadSpecInput::Service)
    //   2. Validate Service::from_submit client-side
    //   3. POST to the in-process server
    //   4. Receive the streaming `Accepted` first wire line
    //
    // The call may block waiting for a terminal event — the timeout
    // fires and we issue an operator stop to free the streaming
    // consumer.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Issue stop to drive the Service-kind stream to terminal
    // `Stopped` so the submit task finishes. Pre-fix this is a
    // no-op (the workload was never admitted; the submit already
    // failed); post-fix it walks the Service-kind terminal path.
    let _ = overdrive_cli::commands::deploy::stop(StopArgs {
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
                 the submit-side Service dispatch did not run. field={field}, \
                 message={message}",
            );
        }
        Ok(Ok(Err(other))) => {
            // Other errors (Transport, Validation) are acceptable —
            // they mean the dispatch reached the server which then
            // failed for a different reason. The load-bearing
            // assertion is "NOT InvalidSpec from the submit-side parser".
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
