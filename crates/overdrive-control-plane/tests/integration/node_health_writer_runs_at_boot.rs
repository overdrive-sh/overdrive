//! Regression test — step 01-01 of `fix-orphaned-node-health-writer`.
//!
//! Pins the boot-time `node_health` write per ADR-0025 § 3 step 5
//! (amended by ADR-0029 — writer relocated to worker-subsystem
//! startup). Before the fix, `run_server_with_obs_and_driver` wired
//! the `ObservationStore` but never invoked `overdrive_worker::
//! write_node_health_row`; `GET /v1/nodes` on a healthy single-node
//! deployment returned `[]` instead of one row.
//!
//! This test boots the server through the SAME entry point the CLI
//! uses (`run_server` → `run_server_with_obs_and_driver`) and asserts
//! the observation store carries exactly one `NodeHealthRow` after
//! startup, written by `start_local_node`.
//!
//! Port-to-port principle: if a future refactor deletes the
//! `start_local_node` call, this test flips red — the test enters
//! through the production boot path's driving port (the public
//! `run_server_with_obs_and_driver` API) and asserts at the
//! `ObservationStore` driven-port boundary.
//!
//! Tier 3 — real axum server, real rustls handshake, real
//! `SimObservationStore`. Gated by the `integration-tests` feature at
//! the `tests/integration.rs` entrypoint.
//!
//! See `docs/feature/fix-orphaned-node-health-writer/deliver/rca.md`
//! for the full root-cause analysis.

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::NodeList;
use overdrive_control_plane::observation_wiring::wire_single_node_observation;
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::driver::SimDriver;
use tempfile::TempDir;

/// Boot the server through the public `run_server_with_obs_and_driver`
/// driving port and assert the `ObservationStore` carries exactly one
/// `NodeHealthRow` after startup completes.
#[tokio::test]
async fn boot_writes_exactly_one_node_health_row_to_observation_store() {
    let tmp = TempDir::new().expect("tempdir");
    // `data_dir` + `operator_config_dir` are separate subdirectories
    // per `fix-cli-cannot-reach-control-plane` step 01-02 (RCA §WHY 4C).
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");

    // Retain the obs handle so we can read `node_health_rows()` after
    // boot; this is precisely why `run_server_with_obs_and_driver`
    // exists as a split entry point (see its docstring in
    // `crates/overdrive-control-plane/src/lib.rs`).
    let obs: Arc<dyn ObservationStore> =
        Arc::from(wire_single_node_observation(&data_dir).expect("wire obs store"));

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir,
        dataplane_override: Some(Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new())),
        // ADR-0061 § 1 (step 01-03): default `dataplane` is now veth-named
        // (`ovd-veth-cli` absent in the test VM); name `lo` so the boot
        // `host_ipv4` resolution succeeds. SimDataplane skips XDP attach.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK so `boot_ca`'s
        // KEK-resolve probe succeeds with no kernel-keyring / env dependency.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let handle = run_server_with_obs_and_driver(config, Arc::clone(&obs), driver)
        .await
        .expect("run_server_with_obs_and_driver");

    // Read directly from the obs handle the server holds. The
    // expected post-boot state (ADR-0025 § 3 step 5): exactly one
    // `NodeHealthRow` written by `overdrive_worker::
    // write_node_health_row` via the `start_local_node` helper.
    let rows = obs.node_health_rows().await.expect("read node_health_rows");

    assert_eq!(
        rows.len(),
        1,
        "ADR-0025 step 5: boot must write exactly one node_health row \
         (single-node Phase 1); got {} rows. If this assertion reads \
         `left: 0, right: 1`, the boot path is skipping the writer — \
         see docs/feature/fix-orphaned-node-health-writer/deliver/rca.md.",
        rows.len(),
    );

    // Sanity check on the row shape — `last_heartbeat` must not be
    // the default `LogicalTimestamp` (counter=0, writer=epoch). A
    // row written from the real clock carries a non-default
    // timestamp; the default value would suggest the writer was
    // called with an uninitialised clock.
    let row = &rows[0];
    assert_ne!(
        row.last_heartbeat.counter, 0,
        "node_health row must carry a non-default LogicalTimestamp.counter; got {:?}",
        row.last_heartbeat,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

/// Operator-visible KPI for the same ADR-0025 step 5 contract: after
/// boot, `GET /v1/nodes` returns a single-element array whose row
/// matches the configured node. Whereas the prior test reads through
/// the `ObservationStore` driving port directly, THIS test exercises
/// the full HTTPS handler chain — rustls handshake, axum routing, the
/// `node_list` handler, the `NodeRowBody` serde projection — proving
/// the writer's output reaches the operator surface.
///
/// Gated by `--features integration-tests` at the
/// `tests/integration.rs` entrypoint; runs under Lima per
/// `.claude/rules/testing.md` § "Running tests — Lima VM".
#[tokio::test]
async fn boot_writes_node_health_row_visible_via_get_v1_nodes() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let tmp = TempDir::new().expect("tempdir");
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
        dataplane_override: Some(Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new())),
        // ADR-0061 § 1 (step 01-03): default `dataplane` is now veth-named
        // (`ovd-veth-cli` absent in the test VM); name `lo` so the boot
        // `host_ipv4` resolution succeeds. SimDataplane skips XDP attach.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK so `boot_ca`'s
        // KEK-resolve probe succeeds with no kernel-keyring / env dependency.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let handle = run_server_with_obs_and_driver(config, Arc::clone(&obs), driver)
        .await
        .expect("run_server_with_obs_and_driver");

    // Read the trust triple to build a CA-pinned reqwest client —
    // same shape as `tests/integration/observation_empty_rows.rs`.
    let bound = handle.local_addr().await.expect("bound addr");
    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path).expect("read trust triple");
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
    let ca_pem =
        String::from_utf8(BASE64.decode(ca_b64).expect("base64 decode ca")).expect("ca PEM UTF-8");
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    let client = reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client");

    let url = format!("https://localhost:{}/v1/nodes", bound.port());
    let resp = client.get(&url).send().await.expect("GET /v1/nodes");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET /v1/nodes expected 200");
    let body: NodeList = resp.json().await.expect("decode NodeList");
    assert_eq!(
        body.rows.len(),
        1,
        "ADR-0025 step 5 KPI: single-node boot must surface exactly one row \
         via GET /v1/nodes; got {} rows",
        body.rows.len(),
    );
    // `node_id` is the resolved hostname (the default `NodeConfig`
    // has no override, so the writer's hostname fallback fires) and
    // `region` is the default `"local"`. The exact hostname depends
    // on the test runner, but the row must carry SOME non-empty value
    // and the region must be the configured default.
    let row = &body.rows[0];
    assert!(!row.node_id.is_empty(), "node_id must be non-empty");
    assert_eq!(row.region, "local", "region must match the default NodeConfig");

    handle.shutdown(Duration::from_secs(2)).await;
}
