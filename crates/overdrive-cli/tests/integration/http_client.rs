//! Integration tests for `overdrive_cli::http_client::ApiClient` — the
//! hand-rolled reqwest client per ADR-0014.
//!
//! These tests are Tier 3 (real reqwest + real TLS + real in-process
//! control-plane server) per `.claude/rules/testing.md`. They stand up
//! the server via `overdrive_control_plane::run_server` on an ephemeral
//! port (NOT a subprocess per `crates/overdrive-cli/CLAUDE.md`), point
//! an `ApiClient` loaded from the on-disk trust triple at the server,
//! and exercise the five endpoint methods plus the failure modes.
//!
//! Acceptance coverage:
//!   (a) `from_config` loads + pins CA + builds client
//!   (b) `cluster_status` against real server returns Ok
//!   (c) `submit_job` + `describe_job` round-trip through HTTP
//!   (d) `cluster_status` with no server → typed `CliError::Transport`
//!       with actionable (non-leaky) Display
//!   (e) `submit_job` with invalid spec → `CliError::HttpStatus { 400,
//!       ErrorBody { error: "validation", field: Some("replicas"), .. } }`
//!   (f) `from_config` with malformed base64 CA → `CliError::ConfigLoad`
//!       with non-raw cause

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use overdrive_cli::http_client::{ApiClient, CliError};
use overdrive_control_plane::api::{JobDescription, SubmitJobRequest};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::JobSpecInput;
use tempfile::TempDir;

/// Spawn a server on an ephemeral port and return (handle, bound addr,
/// tempdir kept alive, path to the trust-triple config). The trust
/// triple is written by `run_server` to `<data_dir>/.overdrive/config`.
async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: tmp.path().to_path_buf(),
    };
    let handle: ServerHandle = run_server(config).await.expect("run_server");
    let bound: SocketAddr = handle.local_addr().await.expect("bound addr");
    let config_path = tmp.path().join(".overdrive").join("config");
    (handle, bound, tmp, config_path)
}

/// Build an `ApiClient` from the on-disk trust triple, overriding the
/// recorded endpoint so it points at the actual ephemeral port the
/// server bound. `run_server` writes the endpoint as
/// `https://127.0.0.1:0` (the configured bind, NOT the resolved port —
/// that's fine: tests supply the live port explicitly).
fn build_client_for(config_path: &Path, bound: SocketAddr) -> ApiClient {
    let override_endpoint = format!("https://localhost:{}", bound.port());
    ApiClient::from_config_with_endpoint(config_path, Some(&override_endpoint))
        .expect("build ApiClient from trust triple")
}

// -------------------------------------------------------------------
// (a) from_config loads the trust triple and builds a client
// -------------------------------------------------------------------

#[tokio::test]
async fn from_config_loads_trust_triple_and_builds_client() {
    let (handle, bound, _tmp, config_path) = spawn_server().await;

    // No endpoint override — consume whatever `run_server` wrote.
    let client = ApiClient::from_config(&config_path).expect("build ApiClient from on-disk config");

    // The recorded endpoint should be a parseable URL, non-empty.
    assert!(!client.base_url().as_str().is_empty(), "base URL must not be empty");
    assert_eq!(client.base_url().scheme(), "https", "must be HTTPS per ADR-0010");

    // Prove the client is not merely "constructed" — it can hit the
    // server when pointed at the real bound port. This kills a
    // mutation where `from_config` returns a client that trusts the
    // wrong CA (TLS handshake would fail).
    //
    // Uses `node_list` as the proof-of-life endpoint: `/v1/nodes` is
    // the real observation-read handler wired in step 03-03 and
    // renders `{"rows":[]}` on a fresh store. The `/v1/cluster/info`
    // route is still stubbed until step 03-05 wires the real handler,
    // so we cannot decode it as `ClusterStatus` from this test.
    let live = build_client_for(&config_path, bound);
    let nodes = live.node_list().await.expect("live node_list");
    assert!(nodes.rows.is_empty(), "fresh store must report zero node rows");

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (b) alloc_status + node_list against in-process server return Ok
// -------------------------------------------------------------------
//
// `/v1/cluster/info` is still stubbed until step 03-05 wires the real
// `cluster_status` handler; exercising it against the stub would force
// this test to assert on `{}` rather than on a real `ClusterStatus`
// shape, which would turn into a contradicting test the moment 03-05
// lands. We instead pin the two observation-read endpoints
// (`/v1/allocs`, `/v1/nodes`) that ARE wired to real handlers as of
// step 03-03 — those return `{"rows":[]}` on a fresh store and
// decode deterministically into their typed responses.

#[tokio::test]
async fn observation_reads_against_in_process_server_return_ok() {
    let (handle, bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path, bound);

    let allocs = client.alloc_status().await.expect("alloc_status");
    assert!(allocs.rows.is_empty(), "fresh store must report zero alloc rows");

    let nodes = client.node_list().await.expect("node_list");
    assert!(nodes.rows.is_empty(), "fresh store must report zero node rows");

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (c) submit_job + describe_job round-trip
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_job_then_describe_round_trips_via_http_client() {
    let (handle, bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path, bound);

    let spec = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        cpu_milli: 500,
        memory_bytes: 536_870_912,
    };

    let submit_resp =
        client.submit_job(SubmitJobRequest { spec: spec.clone() }).await.expect("submit_job");
    assert!(!submit_resp.job_id.is_empty(), "job_id must not be empty");
    assert!(submit_resp.commit_index > 0, "commit_index must be > 0");

    let description: JobDescription =
        client.describe_job(&submit_resp.job_id).await.expect("describe_job");
    assert_eq!(description.spec, spec, "round-tripped spec must match submitted spec");
    assert_eq!(description.commit_index, submit_resp.commit_index);
    assert!(!description.spec_digest.is_empty(), "spec_digest must not be empty");

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (d) no server → CliError::Transport with actionable, non-leaky Display
// -------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_with_no_server_returns_transport_error_with_actionable_message() {
    // Spawn a server to write a valid trust triple, then immediately
    // shut it down. `from_config` succeeds (file on disk is valid);
    // the subsequent `cluster_status` call fails because nothing is
    // listening on the chosen port.
    let (handle, bound, _tmp, config_path) = spawn_server().await;
    handle.shutdown(Duration::from_secs(1)).await;

    // Point the client at the now-closed port. `from_config_with_endpoint`
    // does NOT attempt to connect — it only loads the trust material.
    let client = build_client_for(&config_path, bound);

    let err = client.cluster_status().await.expect_err("no server → transport error");

    // Variant-level assertion: must be Transport.
    match &err {
        CliError::Transport { endpoint, .. } => {
            assert!(
                endpoint.contains(&bound.port().to_string()),
                "Transport.endpoint must name the endpoint; got {endpoint}",
            );
        }
        other => panic!("expected CliError::Transport, got {other:?}"),
    }

    // Display-level assertion: message must be actionable (names the
    // endpoint) and must NOT leak low-level transport internals.
    let rendered = format!("{err}");
    assert!(
        rendered.contains("127.0.0.1") || rendered.contains("localhost"),
        "Display must name the endpoint so operators can act on the error; got: {rendered}"
    );
    assert!(
        !rendered.contains("ECONNREFUSED"),
        "Display must not leak raw `ECONNREFUSED` token; got: {rendered}",
    );
    assert!(
        !rendered.contains("reqwest::Error"),
        "Display must not leak reqwest::Error Debug format; got: {rendered}",
    );
}

// -------------------------------------------------------------------
// (e) invalid spec → HTTP 400 mapped to CliError::HttpStatus
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_with_invalid_spec_returns_http_status_400_with_error_body() {
    let (handle, bound, _tmp, config_path) = spawn_server().await;
    let client = build_client_for(&config_path, bound);

    // `replicas = 0` fails `Job::from_spec` at the NonZeroU32 gate;
    // server returns 400 with ErrorBody { error: "validation",
    // field: Some("replicas"), .. } per ADR-0015.
    let bad = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 0,
        cpu_milli: 500,
        memory_bytes: 536_870_912,
    };

    let err = client.submit_job(SubmitJobRequest { spec: bad }).await.expect_err("bad spec");

    match &err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(*status, 400, "must map HTTP 400 to HttpStatus with status 400");
            assert_eq!(body.error, "validation", "error kind must be 'validation'");
            assert_eq!(
                body.field.as_deref(),
                Some("replicas"),
                "field must name the offending spec field; got {:?}",
                body.field,
            );
            assert!(
                body.message.contains("replicas"),
                "message must name the offending field; got {:?}",
                body.message,
            );
        }
        other => panic!("expected CliError::HttpStatus, got {other:?}"),
    }

    handle.shutdown(Duration::from_secs(2)).await;
}

// -------------------------------------------------------------------
// (f) malformed base64 CA → CliError::ConfigLoad
// -------------------------------------------------------------------

#[tokio::test]
async fn from_config_with_malformed_base64_ca_returns_config_load_error() {
    let tmp = TempDir::new().expect("tempdir");
    let overdrive_dir = tmp.path().join(".overdrive");
    std::fs::create_dir_all(&overdrive_dir).expect("create .overdrive dir");
    let config_path = overdrive_dir.join("config");

    // Write a Talos-shape YAML whose `ca` field is deliberately
    // corrupt base64 — contains invalid characters that no base64
    // decoder will accept.
    let corrupt_yaml = "context: local
contexts:
  local:
    endpoint: https://127.0.0.1:7001
    ca: not-base64!!@#$%
    crt: dmFsaWQgZW5vdWdo
    key: dmFsaWQgZW5vdWdo
";
    std::fs::write(&config_path, corrupt_yaml).expect("write corrupt config");

    let err = ApiClient::from_config(&config_path).expect_err("malformed base64 must fail");

    match &err {
        CliError::ConfigLoad { path, .. } => {
            assert!(
                path.contains(config_path.file_name().unwrap().to_str().unwrap())
                    || path.contains(".overdrive"),
                "ConfigLoad.path must name the config file; got {path}",
            );
        }
        other => panic!("expected CliError::ConfigLoad, got {other:?}"),
    }

    // Display must be actionable (names the path) and not leak decoder
    // Debug format.
    let rendered = format!("{err}");
    assert!(
        rendered.contains(".overdrive") || rendered.contains("config"),
        "Display must name the config path; got: {rendered}",
    );
    assert!(
        !rendered.contains("DecodeError"),
        "Display must not leak base64::DecodeError Debug format; got: {rendered}",
    );
}
