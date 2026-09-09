//! Constructor/runtime registration control for the authoritative Service
//! backend publisher and its asynchronous ServiceMapHydrator consumer.
//! This is a focused registry boundary check, not a claim to execute server
//! boot. The retained boot-composition and walking-skeleton tests exercise
//! production server composition; BE10 exercises owner-to-consumer convergence.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{
    noop_heartbeat, service_lifecycle, service_map_hydrator, workload_lifecycle,
};
use std::net::Ipv4Addr;
use tempfile::TempDir;

/// The canonical publisher and consumer constructors register successfully
/// in the production relative order, so their enqueue names resolve.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn service_backend_publisher_and_hydrator_register_with_canonical_names() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
        .expect("ReconcilerRuntime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
    runtime
        .register(service_map_hydrator(Ipv4Addr::LOCALHOST))
        .await
        .expect("register service-map-hydrator");
    runtime.register(service_lifecycle()).await.expect("register service-lifecycle");

    let registered_names: Vec<String> =
        runtime.registered().into_iter().map(|n| n.as_str().to_owned()).collect();
    for expected in ["service-lifecycle", "service-map-hydrator"] {
        assert!(
            registered_names.iter().any(|n| n == expected),
            "canonical owner/consumer name {expected} must resolve: {registered_names:?}"
        );
    }
}
