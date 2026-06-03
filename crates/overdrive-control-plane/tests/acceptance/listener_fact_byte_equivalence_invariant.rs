//! Acceptance test — edge-half of invariant B (ADR-0062 § Decision (2);
//! feature-delta sub-decision 2, the "writer-bumped invalidation"
//! discipline).
//!
//! Invariant B (whole): the in-memory `ListenerFactStore` is, at every
//! observable point, byte-equal to what `rebuild_from_intent` would
//! reproject from the committed intent SSOT joined with the allocator
//! memo. The STORE-LEVEL half (a fresh store fed the same `upsert`
//! sequence equals one rebuilt over the same intent) is pinned by the
//! proptest in `listener_facts.rs` (step 01-01).
//!
//! THIS test pins the HANDLER-EDGE half: after a sequence of real
//! Service / Job submissions driven through the public `submit_workload`
//! HTTP handler (the single atomic intent-change edge, step 01-03), the
//! edge-maintained `state.listener_facts` equals a fresh
//! `rebuild_from_intent` over the same committed intent set + the same
//! allocator. The handler-edge upsert (step 01-03) and the boot rebuild
//! (step 01-02) agree.
//!
//! Port-to-port per `.claude/rules/testing.md` § "Port-to-port at all
//! levels": the test enters through the `submit_workload` driving port —
//! it does NOT call `ListenerFactStore::upsert` directly. It would not
//! flip RED→GREEN if the handler short-circuited around the edge upsert,
//! because the rebuild reprojects from the intent the handler committed
//! while the edge-maintained store would stay empty — the two would
//! diverge and `assert_eq!` would fail.

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

use overdrive_control_plane::AppState;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::handlers::submit_workload;
use overdrive_control_plane::listener_facts::ListenerFactStore;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;

use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures (mirror service_vip_submit_acceptance.rs)
// ---------------------------------------------------------------------------

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> =
        Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
            VipRange::default(),
            Arc::clone(&store) as Arc<dyn IntentStore>,
        )));
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        NodeId::new("writer-1").expect("NodeId"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn service_spec(id: &str, listeners: Vec<(u16, &str)>) -> SubmitSpecInput {
    SubmitSpecInput::Service(ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        listeners: listeners
            .into_iter()
            .map(|(port, protocol)| ListenerInput { port, protocol: protocol.to_owned() })
            .collect(),
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
}

fn job_spec(id: &str) -> SubmitSpecInput {
    SubmitSpecInput::Job(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 67_108_864 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/run".to_string(), args: vec![] }),
    })
}

async fn submit(state: AppState, spec: SubmitSpecInput) {
    let _response: Response =
        submit_workload(State(state), HeaderMap::new(), Json(SubmitWorkloadRequest { spec }))
            .await
            .expect("submit must succeed");
}

// ---------------------------------------------------------------------------
// Edge-half of invariant B — the handler edge agrees with the boot rebuild.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edge_maintained_store_equals_rebuild_from_committed_intent() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // A representative mixed submission sequence through the driving
    // port: multi-listener Service, single-listener Service, a Job (no
    // VIP — contributes no facts), and a mixed-protocol Service.
    submit(state.clone(), service_spec("web", vec![(80, "tcp"), (443, "tcp"), (53, "udp")])).await;
    submit(state.clone(), service_spec("api", vec![(9000, "tcp")])).await;
    submit(state.clone(), job_spec("batch")).await;
    submit(state.clone(), service_spec("dns", vec![(5300, "udp"), (5301, "tcp")])).await;

    // Snapshot the edge-maintained store (clone out from under the guard;
    // drop the guard before the rebuild's own allocator `.await`).
    let edge_store: ListenerFactStore = {
        let guard = state.listener_facts.lock().await;
        guard.clone()
    };

    // Reproject from the SAME committed intent SSOT + the SAME allocator.
    let rebuilt = ListenerFactStore::rebuild_from_intent(
        &state.store,
        &state.intent_redb_path,
        &state.allocator,
    )
    .await
    .expect("rebuild_from_intent");

    assert_eq!(
        edge_store, rebuilt,
        "edge-maintained ListenerFactStore must equal a fresh rebuild over the committed intent set",
    );
}
