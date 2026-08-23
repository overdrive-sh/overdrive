//! S-266-01 walking skeleton (ADR-0081 §5, GH #266) — the PRODUCTION
//! composition entry `run_server_with_obs_and_driver` spawns
//! `spawn_interest_router` as part of its boot, wired with a Sim observation
//! store + `SimClock` + `SimDriver`.
//!
//! Vertical-slice teeth (CLAUDE.md § "Build vertical slices through
//! production entry points"): the test hand-calls NO spawn fn and
//! hand-assembles NO router — it boots the real production entry and asserts
//! the router task is LIVE after the entry returns (a structural/boot check
//! via `ServerHandle::interest_router_running`). If the `spawn_interest_router`
//! wiring is deleted from `run_server_with_obs_and_driver`, this goes RED.
//!
//! Tier 3 — real axum boot + `LocalIntentStore` on a tempdir + `SimClock` /
//! `SimDriver` / `SimDataplane`. Gated by `integration-tests` at the
//! `tests/integration.rs` entrypoint; runs under Lima per
//! `.claude/rules/testing.md` § "Running tests — Lima VM". A `--no-run` gate
//! proves nothing — this MUST actually boot the fixture.

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tempfile::TempDir;

/// GIVEN the runtime booted through its production composition entry
/// `run_server_with_obs_and_driver` (Sim obs + `SimClock` + `SimDriver`) —
/// WHEN the entry returns its `ServerHandle` —
/// THEN the entry spawned `spawn_interest_router` and the router task is live
/// (`interest_router_running()` is `true`), driven by the production boot and
/// not by any hand-installed spawn.
#[tokio::test]
async fn production_boot_spawns_the_interest_router() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");

    let clock = Arc::new(SimClock::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir,
        tick_cadence: Duration::from_millis(100),
        clock: clock.clone(),
        // Hermetic in-process boot KEK so `boot_ca`'s KEK-resolve probe
        // succeeds with no kernel-keyring / env dependency.
        kek: Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        node: overdrive_worker::NodeConfig::default(),
        vip_range: overdrive_dataplane::allocators::VipRange::default(),
        // ADR-0061 § 1 — `lo`/`lo` shape so the boot `host_ipv4` resolution
        // succeeds in the test VM; `SimDataplane` (below) skips XDP attach.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        dataplane_pin_dir: None,
        dataplane_cgroup_attach_path: None,
        // Inject `SimDataplane` per architecture.md § 4.7 — the SUT here is
        // the interest-router spawn wiring, not the dataplane attach path.
        dataplane_override: Some(Arc::new(
            overdrive_sim::adapters::dataplane::SimDataplane::new(),
        )),
        dataplane_probe_fault: None,
        mtls_probe_fault: None,
        dns_probe_fault: None,
        mtls_identity_override: None,
    };

    let handle = run_server_with_obs_and_driver(config, Arc::clone(&obs), Arc::clone(&driver))
        .await
        .expect("server boot");

    // The vertical-slice assertion: the PRODUCTION entry spawned the router.
    // No spawn fn was hand-called, no router hand-assembled — the boot did it.
    assert!(
        handle.interest_router_running(),
        "run_server_with_obs_and_driver MUST spawn spawn_interest_router as part of its \
         composition (ADR-0081 §5); the router task is not live after the entry returned. \
         If this fails, the production spawn wiring is missing — the mechanism is dead code.",
    );

    handle.shutdown(Duration::from_secs(1)).await;
}
