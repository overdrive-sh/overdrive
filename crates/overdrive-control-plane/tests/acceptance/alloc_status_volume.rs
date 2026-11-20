//! `GET /v1/allocs?job=<id>` carries the workload's declared volume (GH #348, ADR-0310): the
//! handler-layer half of the operator's `workload describe` Volume section.
//!
//! The handler projects the declaration from the persisted workload intent
//! (`AllocStatusResponse.volume`, pinned as `Option<VolumeBody { size: String }>`); the
//! `guest disk: virtio serial ovd-volume` text is static in the CLI render and not part of the
//! response (`overdrive-cli/tests/acceptance/workload_describe_volume.rs`). The test reads the
//! response through its wire form, so it names only the pinned field by its serialised name.
//! The workload reaches the intent through the production wire shape and the validating
//! constructor, as `submit_workload` admits it; nothing is seeded that production does not
//! write, so it fails today where the wire spec refuses `vm.volume`.
//!
//! Default-lane: in-process axum handler call over a `LocalIntentStore`, no real I/O.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::{IntentKey, Service, WorkloadIntent, WorkloadKind};
use overdrive_core::api::submit::ServiceSpecInput;
use overdrive_core::id::{NodeId, WorkloadId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_testing::placement;
use serde_json::json;
use tempfile::TempDir;

placement!(host_pure);

const WORKLOAD: &str = "dev-box";

fn node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
    )
}

/// Persists a one-replica Service whose VM declares `[vm.volume] size = "40GiB"`, through the
/// production wire shape and `Service::from_submit`.
async fn install_volume_service(state: &AppState) {
    let spec: ServiceSpecInput = serde_json::from_value(json!({
        "id": WORKLOAD,
        "replicas": 1,
        "resources": {"cpu_milli": 500, "memory_bytes": 134_217_728u64},
        "vm": {
            "command": "/sbin/init", "args": [], "kernel": "/kernel", "rootfs": "/rootfs",
            "volume": {"size": "40GiB"}
        },
        "listeners": [{"port": 8080, "protocol": "tcp"}],
        "startup_probes": [], "readiness_probes": [], "liveness_probes": []
    }))
    .expect("a [vm.volume] declaration must be part of the service wire spec");
    let service = Service::from_submit(spec).expect("a one-replica volume service is valid");
    let id = WorkloadId::new(WORKLOAD).expect("valid workload id");
    let archived =
        WorkloadIntent::Service(service).archive_for_store().expect("rkyv archive Service");
    state
        .store
        .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
        .await
        .expect("IntentStore put Service");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("IntentStore put kind");
}

/// The handler fills `AllocStatusResponse.volume` from the persisted intent: the declared size,
/// in the `Display` form the operator wrote.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the service wire spec")]
async fn alloc_status_reports_the_declared_volume_size() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    install_volume_service(&state).await;

    let response = alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some(WORKLOAD.to_owned()) }),
    )
    .await
    .expect("alloc_status answers for an installed workload")
    .0;

    let wire = serde_json::to_value(&response).expect("the response serialises");
    assert_eq!(
        wire.pointer("/volume/size").and_then(serde_json::Value::as_str),
        Some("40GiB"),
        "the response must carry the declared volume size; got {wire}"
    );
}
