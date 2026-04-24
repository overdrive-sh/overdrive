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
//!       `ClusterStatusOutput` carrying `mode`, `region`, `commit_index`,
//!       the reconciler registry (includes `noop-heartbeat`), and typed
//!       broker counters.
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
/// return the handle, the resolved bound address, and the `TempDir`
/// backing the data directory. The `TempDir` is returned so the caller
/// can keep it alive for the duration of the test — dropping it
/// deletes the config.
async fn spawn_server() -> (ServeHandle, SocketAddr, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    let port = handle.endpoint().port().expect("endpoint port");
    let bound: SocketAddr = format!("127.0.0.1:{port}").parse().expect("parse bound addr");
    (handle, bound, tmp)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<data_dir>/.overdrive/config`.
fn config_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(".overdrive").join("config")
}

/// Rewrite the `endpoint` field in the on-disk trust-triple TOML so it
/// names the real ephemeral port the server bound to. The operator
/// config is the sole source of the endpoint (no `--endpoint` override),
/// so tests mutate the on-disk config to point at the live server.
fn rewrite_config_endpoint(config_path: &Path, new_endpoint: &str) {
    let original = std::fs::read_to_string(config_path).expect("read existing trust-triple config");
    let mut doc: toml::Value = toml::from_str(&original).expect("parse existing config toml");
    let contexts =
        doc.get_mut("contexts").and_then(|c| c.as_array_mut()).expect("contexts array present");
    for ctx in contexts.iter_mut() {
        if let Some(tbl) = ctx.as_table_mut() {
            tbl.insert("endpoint".to_owned(), toml::Value::String(new_endpoint.to_owned()));
        }
    }
    let rewritten = toml::to_string(&doc).expect("reserialise config toml");
    std::fs::write(config_path, rewritten).expect("write rewritten config");
}

/// Point the operator config for `data_dir` at the ephemeral port the
/// running server bound to, returning the path to the written config.
fn point_config_at(data_dir: &Path, port: u16) -> std::path::PathBuf {
    let cfg = config_path(data_dir);
    rewrite_config_endpoint(&cfg, &format!("https://localhost:{port}"));
    cfg
}

// -------------------------------------------------------------------
// (a) cluster::status returns typed output with reconciler registry
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_against_in_process_server_returns_typed_output_with_reconciler_registry() {
    let (handle, bound, tmp) = spawn_server().await;
    let cfg = point_config_at(tmp.path(), bound.port());

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
    let (handle, bound, tmp) = spawn_server().await;
    let cfg = point_config_at(tmp.path(), bound.port());

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
    let (handle, bound, tmp) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg = point_config_at(tmp.path(), bound.port());

    let args = StatusArgs { config_path: cfg };
    let err = overdrive_cli::commands::cluster::status(args)
        .await
        .expect_err("no server → cluster::status must fail");

    match &err {
        CliError::Transport { endpoint: ep, .. } => {
            assert!(
                ep.contains(&bound.port().to_string()),
                "Transport.endpoint must name the endpoint; got {ep}",
            );
        }
        other => panic!("expected CliError::Transport, got {other:?}"),
    }
}
