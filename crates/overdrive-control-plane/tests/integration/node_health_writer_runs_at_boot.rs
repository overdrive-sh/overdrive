//! Regression test — step 01-01 of `fix-orphaned-node-health-writer`.
//!
//! Pins the missing boot-time `node_health` write per ADR-0025 § 3
//! step 5 (amended by ADR-0029 — writer relocated to worker-subsystem
//! startup). `run_server_with_obs_and_driver` wires the
//! `ObservationStore` but never invokes `overdrive_worker::
//! write_node_health_row`; `GET /v1/nodes` on a healthy single-node
//! deployment returns `[]` instead of one row.
//!
//! This test boots the server through the SAME entry point the CLI
//! uses (`run_server` → `run_server_with_obs_and_driver`) and asserts
//! the observation store carries exactly one `NodeHealthRow` after
//! startup. It MUST FAIL today (zero rows in the store, `assertion
//! left: 0, right: 1`) and pass after step 01-02 lands the
//! `start_local_node` helper + call site in
//! `run_server_with_obs_and_driver`.
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

use overdrive_control_plane::observation_wiring::wire_single_node_observation;
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::driver::SimDriver;
use tempfile::TempDir;

/// Boot the server through the public `run_server_with_obs_and_driver`
/// driving port and assert the `ObservationStore` carries exactly one
/// `NodeHealthRow` after startup completes.
///
/// Fails today: the boot path never calls
/// `overdrive_worker::write_node_health_row`. The expected failure
/// shape is `assertion left: 0, right: 1` (zero rows in the store) —
/// NOT a compile error, NOT a panic from elsewhere. Step 01-02 wires
/// the writer and turns this test green.
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
        ..Default::default()
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
