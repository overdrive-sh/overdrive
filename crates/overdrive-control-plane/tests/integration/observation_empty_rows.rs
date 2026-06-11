//! Integration tests for `GET /v1/nodes` — step 03-03.
//!
//! Proves the Phase 1 observation-read handler returns HONEST empty
//! row arrays from the `SimObservationStore` without fabrication, AND
//! that a canary row injected through the public `ObservationStore`
//! trait surface is visible on the next read.
//!
//! The canary-injection test is the Fixture-Theater defence (quality
//! framework Pattern 8): the handler is proven to actually consult the
//! observation store rather than unconditionally returning a hardcoded
//! empty array. If a future refactor replaces the read with `return
//! vec![]`, the canary test flips red.
//!
//! `/v1/allocs` coverage moved to `acceptance::alloc_status_snapshot`
//! (S-AS-01, S-AS-07, S-AS-09): `?job=<id>` is the canonical shape; a
//! bare GET returns HTTP 400 with `field = Some("job")`. The legacy
//! bare-GET integration tests were dropped per the single-cut greenfield
//! rule ([C9]).
//!
//! Tier 3 — real axum server, real rustls handshake, real reqwest
//! client, real `SimObservationStore` wiring. Gated by the
//! `integration-tests` feature at the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::NodeList;
use overdrive_control_plane::observation_wiring::wire_single_node_observation;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::id::{NodeId, Region};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::driver::SimDriver;
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

fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));
    // ADR-0019 canonical TOML shape: `current-context = "local"` +
    // `[[contexts]]` array-of-tables, each entry carrying `name`,
    // `endpoint`, and the base64-PEM trust triple.
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|c| c.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
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
    // `data_dir` and `operator_config_dir` are SEPARATE subdirectories
    // of the tempdir per `fix-cli-cannot-reach-control-plane` Step
    // 01-02 (RCA §WHY 4C). The observation wiring opens its libSQL
    // database under `data_dir`; the trust triple goes under
    // `operator_config_dir`.
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let obs: Arc<dyn ObservationStore> =
        Arc::from(wire_single_node_observation(&data_dir).expect("wire obs store"));
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        dataplane_override: Some(std::sync::Arc::new(
            overdrive_sim::adapters::dataplane::SimDataplane::new(),
        )),
        // ADR-0061 § 1 (step 01-03): the default `ServerConfig.dataplane`
        // is now the veth-named single-node shape, whose `client_iface`
        // (`ovd-veth-cli`) does not exist in the test VM. This fixture
        // injects `SimDataplane` (no XDP attach) but still resolves
        // `host_ipv4` from `client_iface` at boot, so it names `lo` via
        // the shared SSOT helper.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK so `boot_ca`'s
        // KEK-resolve probe succeeds with no kernel-keyring / env dependency.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let handle = run_server_with_obs_and_driver(config, Arc::clone(&obs), driver)
        .await
        .expect("run_server_with_obs_and_driver");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem, obs)
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
// AC (b): fresh-boot store → 200 with EXACTLY ONE boot-time node_health
// row per ADR-0025 § 3 step 5 (writer wired by step 01-02 of
// `fix-orphaned-node-health-writer`). Prior to that wiring the boot
// produced an empty `node_health` table and this test asserted
// `body.rows.is_empty()`; that assertion was a contract on the
// orphaned-writer bug, not on real behaviour. The structural invariant
// now: a healthy single-node Phase 1 boot ALWAYS surfaces the local
// node's row.
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_nodes_returns_boot_time_node_health_row_on_fresh_store() {
    let (handle, bound, _tmp, ca_pem, _obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

    let url = format!("https://localhost:{}/v1/nodes", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/nodes");

    assert_eq!(resp.status(), reqwest::StatusCode::OK, "fresh-store GET must be HTTP 200");
    let body: NodeList = resp.json().await.expect("decode NodeList");
    assert_eq!(
        body.rows.len(),
        1,
        "ADR-0025 step 5: healthy single-node boot must surface exactly one \
         node_health row (the boot-time writer's output); got {} rows",
        body.rows.len(),
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (c): Fixture-Theater defence — canary row injected via the public
// `ObservationStore::write(ObservationRow)` trait is visible in the
// next handler GET. If the handler short-circuits with a hardcoded
// `vec![]`, this test flips red.
// -----------------------------------------------------------------------

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
    // Two rows: the boot-time writer's row (per ADR-0025 step 5) plus
    // the canary injected by this test. The handler must surface BOTH
    // — a hardcoded short-circuit return would surface neither.
    assert_eq!(
        body.rows.len(),
        2,
        "handler must surface the boot-time row PLUS the canary node_health \
         row; got {} rows",
        body.rows.len(),
    );
    assert!(
        body.rows.iter().any(|r| r.node_id == "canary-node-03-03"),
        "handler must include the injected canary node_health row in the \
         response; got rows: {:?}",
        body.rows,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (e): Honest-rows-field (K7-adjacent) — `/v1/nodes` response JSON
// must contain the explicit `"rows"` field, not an omitted-rows body.
// A handler that returns `{}` would pass the deserialise-to-default
// path silently; CLI rendering depends on the field being present.
//
// Post-`fix-orphaned-node-health-writer` (step 01-02 of that feature):
// boot now writes one row per ADR-0025 step 5, so the body shape is
// `{"rows":[{...}]}` rather than `{"rows":[]}`. The `[]` substring
// assertion that originally lived here was a contract on the
// orphaned-writer bug; it's been replaced with the array-shape
// assertion that holds against any non-empty `rows` payload.
//
// `/v1/allocs` honest-empty-state coverage moved to
// `acceptance::alloc_status_snapshot::s_as_09_*` — the post-cleanup
// shape is `?job=<id>` + 404 on unknown / 400 on missing query, not a
// no-query empty body.
// -----------------------------------------------------------------------

#[tokio::test]
async fn response_body_nodes_field_rows_is_explicit_array_not_omitted() {
    let (handle, bound, _tmp, ca_pem, _obs) = spawn_server_with_obs_handle().await;
    let client = client_trusting(&ca_pem);

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
    // The boot-time writer (per ADR-0025 step 5) produces one row, so
    // the serialised body must include an opening `[` for the rows
    // array. The shape `"rows":[…]` is the structural assertion that
    // a `{}` deserialise-to-default body would fail.
    assert!(
        nodes_raw.contains("\"rows\":["),
        "nodes response must serialise `rows` as a JSON array; got {nodes_raw:?}",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
