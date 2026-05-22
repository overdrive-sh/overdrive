//! UI-05 acceptance — the production boot registers BOTH the
//! `backend-discovery-bridge` AND the `service-map-hydrator`
//! reconcilers against the runtime. Prior to UI-05 the hydrator
//! was missing — `architecture.md` § 4.7 / § 6 claimed it was
//! `// existing` but no `runtime.register` call site existed.
//!
//! This test pins the registration property without rebooting the
//! whole HTTPS / control-plane stack — it constructs the same
//! reconcilers the production boot constructs and asserts they
//! land in the runtime's registered set with their canonical names.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{
    backend_discovery_bridge, noop_heartbeat, service_map_hydrator, workload_lifecycle,
};
use overdrive_core::id::NodeId;
use std::net::Ipv4Addr;
use tempfile::TempDir;

/// GIVEN a fresh `ReconcilerRuntime` configured the way the
/// production boot configures it (noop-heartbeat,
/// workload-lifecycle, backend-discovery-bridge,
/// service-map-hydrator registered in order) —
/// WHEN we query `runtime.registered()` —
/// THEN both `backend-discovery-bridge` AND `service-map-hydrator`
/// appear in the registered set.
///
/// The structural defense: an `Action::EnqueueEvaluation { reconciler:
/// "service-map-hydrator", .. }` emitted by the bridge resolves
/// against a registered reconciler at broker-drain time. Without
/// this registration, the broker would still accept the submit
/// (it doesn't validate against the registry) but the
/// drain-side dispatch would skip the eval — the production gap
/// UI-05 closes.
#[tokio::test]
async fn production_boot_registers_both_bridge_and_hydrator() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
        .expect("ReconcilerRuntime::new");

    // Mirror the production boot order from
    // `run_server_with_obs_and_driver` — noop, workload-lifecycle,
    // backend-discovery-bridge, service-map-hydrator.
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");
    let host_ipv4 = Ipv4Addr::LOCALHOST;
    let node_id = NodeId::new("local").expect("valid NodeId");
    runtime
        .register(backend_discovery_bridge(host_ipv4, node_id))
        .await
        .expect("register backend-discovery-bridge");
    runtime.register(service_map_hydrator(host_ipv4)).await.expect("register service-map-hydrator");

    let registered_names: Vec<String> =
        runtime.registered().into_iter().map(|n| n.as_str().to_owned()).collect();

    assert!(
        registered_names.iter().any(|n| n == "backend-discovery-bridge"),
        "production boot MUST register backend-discovery-bridge; got: {registered_names:?}"
    );
    assert!(
        registered_names.iter().any(|n| n == "service-map-hydrator"),
        "UI-05: production boot MUST register service-map-hydrator (was missing pre-UI-05); \
         got: {registered_names:?}"
    );
}
