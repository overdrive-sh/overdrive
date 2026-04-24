//! Integration tests for `GET /v1/allocs` and `GET /v1/nodes` — step 03-03.
//!
//! Proves the Phase 1 observation-read handlers return HONEST empty
//! row arrays from the `SimObservationStore` without fabrication, AND
//! that a canary row injected through the public `ObservationStore`
//! trait surface is visible on the next read.
//!
//! The canary-injection tests are the Fixture-Theater defence (quality
//! framework Pattern 8): the handler is proven to actually consult the
//! observation store rather than unconditionally returning a hardcoded
//! empty array. If a future refactor replaces the read with `return
//! vec![]`, the canary tests flip red.
//!
//! Tier 3 — real axum server, real rustls handshake, real reqwest
//! client, real `SimObservationStore` wiring. Gated by the
//! `integration-tests` feature at the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{AllocStatusResponse, NodeList};
use overdrive_control_plane::observation_wiring::wire_single_node_observation;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — shared in shape with `submit_round_trip.rs` /
// `describe_round_trip.rs`. Any drift should be extracted into a shared
// helper module under `tests/integration/` — kept duplicated for now so
// each scenario is self-contained.
// -----------------------------------------------------------------------

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

fn read_ca_from_trust_triple(data_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = data_dir.join(".overdrive").join("config");
    let yaml = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));
    let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("parse trust triple YAML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(|c| c.get("local"))
        .and_then(|c| c.get("ca"))
        .and_then(|v| v.as_str())
        .expect("contexts.local.ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

/// Spawn a server and return a handle to the observation store the
/// handlers are reading from. The canary-injection tests write into
/// THIS store via the public `ObservationStore::write` trait method
/// and then assert the handler surfaces the row on the next GET.
async fn spawn_server_with_obs_handle()
-> (ServerHandle, SocketAddr, TempDir, String, Arc<dyn ObservationStore>) {
    let tmp = TempDir::new().expect("tempdir");
    let obs: Arc<dyn ObservationStore> =
        Arc::from(wire_single_node_observation(tmp.path()).expect("wire obs store"));
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: tmp.path().to_path_buf(),
    };
    let handle = run_server_with_obs(config, Arc::clone(&obs)).await.expect("run_server_with_obs");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(tmp.path());
    (handle, bound, tmp, ca_pem, obs)
}

fn canary_alloc_row() -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new("canary-alloc-03-03").expect("valid canary alloc id"),
        job_id: JobId::new("canary-job").expect("valid canary job id"),
        node_id: NodeId::new("control-plane-0").expect("valid canary node id"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("control-plane-0").expect("writer node id"),
        },
    }
}

fn canary_node_health_row() -> NodeHealthRow {
    NodeHealthRow {
        node_id: NodeId::new("canary-node-03-03").expect("valid canary node id"),
        region: Region::new("us-east-1").expect("valid region"),
        last_heartbeat: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("control-plane-0").expect("writer node id"),
        },
    }
}

// -----------------------------------------------------------------------
// AC (a): empty store → 200 with explicit `{"rows": []}` for /v1/allocs
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_allocs_returns_empty_rows_array_on_fresh_store() {
    let (handle, bound, _tmp, ca_pem, _obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    let url = format!("https://localhost:{}/v1/allocs", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/allocs");

    assert_eq!(resp.status(), reqwest::StatusCode::OK, "fresh-store GET must be HTTP 200");
    let body: AllocStatusResponse = resp.json().await.expect("decode AllocStatusResponse");
    assert!(body.rows.is_empty(), "fresh store must yield zero rows; got {}", body.rows.len());

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (b): empty store → 200 with explicit `{"rows": []}` for /v1/nodes
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_nodes_returns_empty_rows_array_on_fresh_store() {
    let (handle, bound, _tmp, ca_pem, _obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    let url = format!("https://localhost:{}/v1/nodes", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/nodes");

    assert_eq!(resp.status(), reqwest::StatusCode::OK, "fresh-store GET must be HTTP 200");
    let body: NodeList = resp.json().await.expect("decode NodeList");
    assert!(body.rows.is_empty(), "fresh store must yield zero rows; got {}", body.rows.len());

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (c): Fixture-Theater defence — canary row injected via the public
// `ObservationStore::write(ObservationRow)` trait is visible in the
// next handler GET. If the handler short-circuits with a hardcoded
// `vec![]`, this test flips red.
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_allocs_returns_injected_canary_alloc_status_row() {
    let (handle, bound, _tmp, ca_pem, obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    // Inject a sentinel row through the same public trait surface any
    // reconciler / node-agent would use. No test-only hook.
    obs.write(ObservationRow::AllocStatus(canary_alloc_row()))
        .await
        .expect("inject canary alloc row via ObservationStore::write");

    let url = format!("https://localhost:{}/v1/allocs", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/allocs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let body: AllocStatusResponse = resp.json().await.expect("decode AllocStatusResponse");
    assert_eq!(
        body.rows.len(),
        1,
        "handler must surface the canary alloc row; got {} rows",
        body.rows.len(),
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn get_v1_nodes_returns_injected_canary_node_health_row() {
    let (handle, bound, _tmp, ca_pem, obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    obs.write(ObservationRow::NodeHealth(canary_node_health_row()))
        .await
        .expect("inject canary node_health row via ObservationStore::write");

    let url = format!("https://localhost:{}/v1/nodes", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/nodes");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let body: NodeList = resp.json().await.expect("decode NodeList");
    assert_eq!(
        body.rows.len(),
        1,
        "handler must surface the canary node_health row; got {} rows",
        body.rows.len(),
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (e): Honest-empty-state (K7) — response JSON must contain the
// explicit `"rows":[]` field, not an omitted-rows body. A handler that
// returns `{}` would pass the deserialise-to-default path silently; the
// CLI's "(0 allocations)" rendering depends on the field being present.
// -----------------------------------------------------------------------

#[tokio::test]
async fn response_body_field_rows_is_explicit_empty_array_not_omitted() {
    let (handle, bound, _tmp, ca_pem, _obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    let allocs_url = format!("https://localhost:{}/v1/allocs", bound.port());
    let allocs_raw = client
        .get(&allocs_url)
        .send()
        .await
        .expect("GET /v1/allocs")
        .text()
        .await
        .expect("read body");
    assert!(
        allocs_raw.contains("\"rows\""),
        "allocs response must carry explicit `rows` field; got {allocs_raw:?}",
    );
    assert!(
        allocs_raw.contains("[]"),
        "allocs response must serialise empty array `[]`; got {allocs_raw:?}",
    );

    let nodes_url = format!("https://localhost:{}/v1/nodes", bound.port());
    let nodes_raw = client
        .get(&nodes_url)
        .send()
        .await
        .expect("GET /v1/nodes")
        .text()
        .await
        .expect("read body");
    assert!(
        nodes_raw.contains("\"rows\""),
        "nodes response must carry explicit `rows` field; got {nodes_raw:?}",
    );
    assert!(
        nodes_raw.contains("[]"),
        "nodes response must serialise empty array `[]`; got {nodes_raw:?}",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
