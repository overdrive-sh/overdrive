//! Integration tests for `overdrive_cli::commands::cluster::status` and
//! `overdrive_cli::commands::node::list` — step 05-03.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handlers directly
//! (NO subprocess). The handlers stand up a real in-process control-plane
//! server via `commands::serve::run(...)` (from step 05-02), then call
//! the cluster-status and node-list handlers which in turn go through
//! the `ApiClient` from step 05-01.
//!
//! Acceptance coverage:
//!   (a) `cluster::status` against in-process server returns typed
//!       `ClusterStatusOutput` carrying `mode`, `region`, the reconciler
//!       registry (includes `noop-heartbeat`), and typed broker
//!       counters (per ADR-0020 the four-field shape; the
//!       `commit_index` field is dropped).
//!   (b) `node::list` against in-process server returns an honest empty
//!       rows vector + an empty-state message naming
//!       `phase-1-first-workload`.
//!   (c) `cluster::status` with no server returns `CliError::Transport`
//!       whose Display names the endpoint so the operator can act on it.

use std::net::SocketAddr;
use std::path::Path;

use overdrive_cli::commands::cluster::{ClusterStatusOutput, StatusArgs};
use overdrive_cli::commands::node::{ListArgs, NodeListOutput};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::CliError;
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0` and
/// return the handle and the `TempDir` backing both directories. The
/// `TempDir` is returned so the caller can keep it alive for the
/// duration of the test — dropping it deletes the config.
///
/// `data_dir` and `config_dir` are SEPARATE subdirectories of the
/// tempdir (`data` and `conf` respectively) per
/// `fix-cli-cannot-reach-control-plane` Step 01-02 (RCA §WHY 4C):
/// the redb + libSQL storage root MUST stay decoupled from the
/// operator-config base.
async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    // Per ADR-0034 the in-binary cgroup escape hatch is gone; on
    // macOS the pre-flight is a `#[cfg(target_os = "linux")]` no-op,
    // and on Linux this test runs via `cargo xtask lima run --`
    // against the bundled VM (root + delegated cgroups).
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    (handle, tmp)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<config_dir>/.overdrive/config` — given the tempdir root from
/// [`spawn_server`].
fn config_path(tmp: &Path) -> std::path::PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

// -------------------------------------------------------------------
// (a) cluster::status returns typed output with reconciler registry
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_against_in_process_server_returns_typed_output_with_reconciler_registry() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let args = StatusArgs { config_path: cfg };
    let output: ClusterStatusOutput =
        overdrive_cli::commands::cluster::status(args).await.expect("cluster::status");

    assert_eq!(output.mode, "single", "Phase 1 control plane mode must be `single`");
    assert_eq!(output.region, "local", "Phase 1 region must be `local`");
    assert!(
        output.reconcilers.contains(&"noop-heartbeat".to_string()),
        "reconciler registry must contain `noop-heartbeat`; got {:?}",
        output.reconcilers,
    );

    // broker counters are typed u64 fields — on a fresh server they
    // are all zero, but the structural assertion (values are u64 and
    // start at zero) is the important part.
    assert_eq!(output.broker.queued, 0_u64, "fresh broker must report queued=0");
    assert_eq!(output.broker.cancelled, 0_u64, "fresh broker must report cancelled=0");
    assert_eq!(output.broker.dispatched, 0_u64, "fresh broker must report dispatched=0");

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (b) node::list returns empty rows and phase-1-first-workload message
// -------------------------------------------------------------------

#[tokio::test]
async fn node_list_against_in_process_server_returns_empty_rows_with_phase_1_first_workload_message()
 {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let args = ListArgs { config_path: cfg };
    let output: NodeListOutput =
        overdrive_cli::commands::node::list(args).await.expect("node::list");

    assert!(output.rows.is_empty(), "fresh store must report zero node rows");
    assert!(
        output.empty_state_message.contains("phase-1-first-workload"),
        "empty-state message must reference `phase-1-first-workload`; got: {}",
        output.empty_state_message,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (c) cluster::status with no server returns CliError::Transport
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_with_no_server_returns_transport_error() {
    // Spawn-and-shutdown to get a valid trust-triple file on disk, then
    // point the handler at the (now-closed) endpoint via the on-disk
    // config.
    let (handle, tmp) = spawn_server().await;
    let port = handle.endpoint().port().expect("endpoint port");
    handle.shutdown().await.expect("clean shutdown");
    let cfg = config_path(tmp.path());

    let args = StatusArgs { config_path: cfg };
    let err = overdrive_cli::commands::cluster::status(args)
        .await
        .expect_err("no server → cluster::status must fail");

    match &err {
        CliError::Transport { endpoint: ep, .. } => {
            assert!(
                ep.contains(&port.to_string()),
                "Transport.endpoint must name the endpoint; got {ep}",
            );
        }
        other => panic!("expected CliError::Transport, got {other:?}"),
    }
}
