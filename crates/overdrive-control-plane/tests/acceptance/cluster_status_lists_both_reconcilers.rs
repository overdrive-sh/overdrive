//! Step 02-02 / Slice 3A.2 scenario 3.14 — `cluster status` enumerates
//! both `noop-heartbeat` AND `job-lifecycle` reconcilers from the
//! runtime registry.
//!
//! Default-lane (in-process server fixture). The test enters via the
//! `cluster_status` axum handler against an `AppState` whose runtime
//! has both reconcilers registered, and asserts the rendered
//! `ClusterStatus.reconcilers` Vec contains both names in canonical
//! Ord order (`BTreeMap` iteration).
//!
//! This is the walking-skeleton driver for 02-02: until the runtime
//! boot path actually registers `JobLifecycle`, the test fails because
//! the rendered list contains only `noop-heartbeat`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use overdrive_control_plane::api::ClusterStatus;
use overdrive_control_plane::handlers::cluster_status;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

async fn build_app_state(tmp: &TempDir) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn overdrive_core::traits::driver::Driver> =
        Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver, Arc::new(SimClock::new()))
}

#[tokio::test]
async fn cluster_status_renders_job_lifecycle_alongside_noop_heartbeat() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp).await;

    let Json(body): Json<ClusterStatus> =
        cluster_status(State(state)).await.expect("cluster_status handler ok");

    // BTreeMap iteration → canonical Ord order — `job-lifecycle` < `noop-heartbeat`
    // alphabetically.
    assert_eq!(
        body.reconcilers,
        vec!["job-lifecycle".to_string(), "noop-heartbeat".to_string()],
        "cluster status must enumerate both registered reconcilers in canonical order"
    );
}
