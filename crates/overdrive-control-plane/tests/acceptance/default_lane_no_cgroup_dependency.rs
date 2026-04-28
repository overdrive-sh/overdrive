//! Step 03-01 / Slice 4 scenario 4.8 —
//! `default_lane_no_cgroup_dependency`.
//!
//! Per ADR-0028, the default unit-test lane (`cargo nextest run
//! --workspace`) MUST stay green on macOS / Windows / Linux without
//! cgroup v2 delegation. This test pins that property: a `ServerConfig`
//! with `allow_no_cgroups: true` boots `run_server` end-to-end on any
//! host, with the trust triple written and the listener bound — proof
//! that no default-lane test depends on cgroup v2 being present.
//!
//! Default-lane (in-memory). Same shape as `job_stop_idempotent.rs`
//! — `allow_no_cgroups: true` is the load-bearing field.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::time::Duration;

use overdrive_control_plane::{ServerConfig, run_server};
use tempfile::TempDir;

#[tokio::test]
async fn boots_with_allow_no_cgroups_on_any_host() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // The contract: this flag bypasses the cgroup pre-flight so the
        // default lane runs uniformly on every host.
        allow_no_cgroups: true,
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        ..Default::default()
    };
    let handle = run_server(config).await.expect("run_server with allow_no_cgroups");

    let bound = handle.local_addr().await.expect("listener must bind");
    assert!(bound.port() > 0, "ephemeral port must be assigned");

    // The trust triple must be on disk so the CLI can find it.
    let triple = operator_config_dir.join(".overdrive").join("config");
    assert!(
        triple.exists(),
        "trust triple must be written to {} after a successful boot",
        triple.display()
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
