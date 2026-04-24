//! Walking-skeleton gate for phase-1-control-plane-core — step 05-05.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handlers directly
//! (NO subprocess, NO `Command::new(env!("CARGO_BIN_EXE_overdrive"))`).
//! The full WS-1 sequence:
//!
//!   1. `cluster::init` writes a trust triple to a tempdir.
//!   2. `serve::run` spawns an in-process axum+rustls server on an
//!      ephemeral port.
//!   3. `job::submit` POSTs a `payments.toml` to the live server via
//!      reqwest.
//!   4. `alloc::status` GETs the job description + alloc rows and
//!      returns an `AllocStatusOutput` whose `spec_digest` is
//!      BYTE-IDENTICAL to a locally-computed
//!      `ContentHash::of(rkyv::to_bytes(&Job::from_spec(...)))`.
//!   5. `serve_handle.shutdown()` drains in-flight connections.
//!
//! THIS TEST IS THE WALKING-SKELETON GATE — flipping it GREEN marks the
//! entire feature walking-skeleton as complete per DWD-05.
//!
//! Acceptance coverage:
//!   (a) full WS-1 end-to-end via direct handler calls with
//!       byte-identical spec digest
//!   (b) unknown job → `CliError::HttpStatus { status: 404, .. }` with
//!       actionable message naming the job id
//!   (c) trust-triple config remains on disk after serve shutdown

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::alloc::{AllocStatusOutput, StatusArgs};
use overdrive_cli::commands::cluster::InitArgs;
use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::CliError;
use overdrive_core::aggregate::{Job, JobSpecInput};
use overdrive_core::id::ContentHash;
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0`. Returns
/// `(handle, tmp)`; the `TempDir` lives for the test duration.
async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    (handle, tmp)
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".overdrive").join("config")
}

const fn payments_toml_spec_str() -> &'static str {
    r#"
id = "payments"
replicas = 3
cpu_milli = 500
memory_bytes = 536870912
"#
}

fn write_payments_toml(dir: &Path) -> PathBuf {
    let path = dir.join("payments.toml");
    std::fs::write(&path, payments_toml_spec_str()).expect("write payments.toml");
    path
}

/// Locally compute the canonical `spec_digest` using the same primitives
/// the server uses in `handlers::describe_job`:
///   SHA-256 of `rkyv::to_bytes::<rancor::Error>(&Job::from_spec(spec))`.
/// Any drift between this and the server-side computation is a bug.
fn local_spec_digest(spec_toml: &str) -> String {
    let parsed: JobSpecInput = toml::from_str(spec_toml).expect("parse TOML");
    let job = Job::from_spec(parsed).expect("Job::from_spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    ContentHash::of(archived.as_ref()).to_string()
}

// -------------------------------------------------------------------
// (a) WALKING-SKELETON GATE — full end-to-end via direct handler calls
// -------------------------------------------------------------------

#[tokio::test]
async fn walking_skeleton_e2e_round_trips_byte_identical_spec_digest_via_direct_handler_calls() {
    // Phase 0: cluster init.
    let tmp = TempDir::new().expect("tempdir");
    let init_output = overdrive_cli::commands::cluster::init(InitArgs {
        config_dir: Some(tmp.path().to_path_buf()),
        force: false,
    })
    .await
    .expect("cluster::init");
    assert!(
        init_output.config_path.exists(),
        "trust triple must exist on disk after init: {}",
        init_output.config_path.display()
    );

    // Phase 1: serve — in-process axum+rustls on ephemeral port.
    // `run_server` writes the resolved-port trust triple to disk, so
    // `from_config` picks up the live endpoint without further help.
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Phase 2: write the job spec, then submit via handler.
    let spec_path = write_payments_toml(tmp.path());
    let submit_output = overdrive_cli::commands::job::submit(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect("job::submit");
    assert_eq!(submit_output.job_id, "payments");
    assert_eq!(submit_output.intent_key, "jobs/payments");
    assert!(
        submit_output.commit_index >= 1,
        "commit_index must be >= 1 after successful submit; got {}",
        submit_output.commit_index,
    );

    // Phase 3: alloc status — returns digest byte-identical to local compute.
    let status_output: AllocStatusOutput = overdrive_cli::commands::alloc::status(StatusArgs {
        job: "payments".to_string(),
        config_path: server_cfg,
    })
    .await
    .expect("alloc::status");

    assert_eq!(status_output.job_id, "payments", "status output must echo job id");
    assert!(
        status_output.commit_index >= 1,
        "alloc status commit_index must be >= 1; got {}",
        status_output.commit_index,
    );
    assert_eq!(
        status_output.allocations_total, 0,
        "phase-1 allocations_total must be 0 (scheduler ships in phase-1-first-workload)",
    );
    assert!(
        status_output.empty_state_message.contains("phase-1-first-workload"),
        "empty_state_message must reference phase-1-first-workload; got: {}",
        status_output.empty_state_message,
    );

    // THE WALKING-SKELETON ASSERTION: spec_digest byte-identical to local.
    let expected_digest = local_spec_digest(payments_toml_spec_str());
    assert_eq!(
        status_output.spec_digest, expected_digest,
        "WS-1: spec_digest returned via alloc::status MUST be byte-identical to \
         ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed))); this proves \
         the whole cluster init → serve → submit → describe round-trip preserves \
         canonical rkyv bytes (ADR-0002 + ADR-0011). Mismatch indicates a \
         client-side second canonicalisation, a server-side re-archival, or a \
         serde-JSON-driven digest recomputation somewhere in the pipeline.",
    );

    // Phase 4: clean shutdown; cluster init config persists on disk.
    handle.shutdown().await.expect("clean shutdown");
    assert!(
        init_output.config_path.exists(),
        "cluster init config must survive serve shutdown: {}",
        init_output.config_path.display(),
    );
}

// -------------------------------------------------------------------
// (b) alloc::status for unknown job → typed 404 with actionable message
// -------------------------------------------------------------------

#[tokio::test]
async fn alloc_status_for_unknown_job_returns_typed_http_status_404_with_actionable_message() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let err = overdrive_cli::commands::alloc::status(StatusArgs {
        job: "mystery".to_string(),
        config_path: server_cfg,
    })
    .await
    .expect_err("alloc::status must fail for an unknown job");

    match &err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 404_u16, "expected HTTP 404 for unknown job; got {}", *status);
            assert_eq!(body.error, "not_found", "error class must be `not_found`");
            // Message must name the offending job id so the operator can act.
            assert!(
                body.message.contains("mystery") || body.message.contains("jobs/mystery"),
                "ErrorBody.message must name `mystery`; got: {}",
                body.message,
            );
        }
        other => panic!(
            "expected CliError::HttpStatus {{ status: 404, .. }} for unknown job, got {other:?}"
        ),
    }

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (c) config file remains on disk after serve shutdown
// -------------------------------------------------------------------

#[tokio::test]
async fn config_file_remains_on_disk_after_serve_shutdown() {
    let (handle, server_tmp) = spawn_server().await;
    let cfg_path = config_path(server_tmp.path());
    assert!(
        cfg_path.exists(),
        "trust-triple config must be written by serve::run at {}",
        cfg_path.display()
    );
    handle.shutdown().await.expect("clean shutdown");
    assert!(
        cfg_path.exists(),
        "trust-triple config must persist after serve shutdown at {}",
        cfg_path.display()
    );
}
