//! GH #160 — `service_backends` `ObservationStore` table wires through
//! to `hydrate_desired` for the `ServiceMapHydrator` reconciler.
//!
//! Tier 1 DST: exercises the full production `hydrate_desired` path
//! through `SimObservationStore` — writes a `ServiceBackendRow`, then
//! invokes `hydrate_desired_for_test` and verifies that the returned
//! `ServiceMapHydratorState.desired` is correctly populated with the
//! right `ServiceVip`, backends, and fingerprint.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV1, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{NodeId, ServiceId, ServiceVip};
use overdrive_core::reconcilers::{AnyReconciler, AnyState, ServiceMapHydrator, TargetResource};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn service_id(n: u64) -> ServiceId {
    ServiceId::new(n).expect("valid ServiceId")
}

fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
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
        Arc::new(SimDataplane::new()),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn sample_backends() -> Vec<Backend> {
    vec![
        Backend {
            alloc: SpiffeId::from_str("spiffe://overdrive.local/job/payments/alloc/a1")
                .expect("spiffe"),
            addr: SocketAddr::from_str("10.0.0.1:8080").expect("addr"),
            weight: 100,
            healthy: true,
        },
        Backend {
            alloc: SpiffeId::from_str("spiffe://overdrive.local/job/payments/alloc/a2")
                .expect("spiffe"),
            addr: SocketAddr::from_str("10.0.0.2:8080").expect("addr"),
            weight: 100,
            healthy: true,
        },
    ]
}

fn hydrator_reconciler() -> AnyReconciler {
    AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        std::net::Ipv4Addr::UNSPECIFIED,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Write a `ServiceBackendRow` into the obs store, then verify
/// `hydrate_desired` projects it into the correct
/// `ServiceMapHydratorState.desired` with the right VIP, backends,
/// and fingerprint.
/// Persist a single-listener Service intent (`udp` on 5353) and allocate
/// its VIP via the production allocator path, so the hydrator can source
/// the listener-bearing protocol fact from the intent (ADR-0060 C3).
/// Returns the allocator-issued VIP + the listener port.
async fn persist_service_and_allocate_vip(
    state: &AppState,
    listener_port: u16,
    protocol: &str,
) -> (ServiceVip, u16) {
    let svc = ServiceV1::from_submit(ServiceSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: listener_port, protocol: protocol.to_string() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("valid service spec");
    let intent = WorkloadIntent::Service(svc.clone());
    let key = IntentKey::for_workload(&svc.id);
    let archived = intent.archive_for_store().expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
    let kind_key = IntentKey::for_workload_kind(&svc.id);
    state
        .store
        .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
        .await
        .expect("put kind");

    let digest = intent.spec_digest().expect("spec_digest");
    let bytes: [u8; 32] = *digest.as_bytes();
    let mut guard = state.allocator.lock().await;
    let vip = guard.allocate(bytes).await.expect("allocate vip");
    drop(guard);

    // The hydrator now sources the listener-bearing `(port, protocol)`
    // fact from the in-memory `ListenerFactStore` (step 01-04 read-path
    // switch), not from a per-tick intent-store scan. Populate the keyed
    // store for the service's listeners — mirroring the submit-edge
    // `upsert` the handler performs in production.
    let svc_listeners = svc.listeners.clone();
    {
        let mut facts = state.listener_facts.lock().await;
        facts.upsert(svc.id.clone(), &vip, &svc_listeners);
    }
    (vip, listener_port)
}

#[tokio::test]
async fn hydrate_desired_projects_service_backends_row() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
    let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

    // Persist the Service intent (udp:5353) + allocate its VIP — the
    // listener-bearing protocol fact source per ADR-0060 C3.
    let (vip, port) = persist_service_and_allocate_vip(&state, 5353, "udp").await;
    let vip_addr = vip.try_as_ipv4().expect("allocator issues IPv4");
    let sid = ServiceId::derive(&vip, std::num::NonZeroU16::new(port).expect("nz"), "service-map");
    let backends = sample_backends();

    // Write the service_backends row keyed by the derived ServiceId,
    // carrying the allocator-issued VIP (the row carries NO protocol).
    let row = ServiceBackendRow {
        service_id: sid,
        vip: vip_addr,
        backends: backends.clone(),
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    };
    obs.write(ObservationRow::ServiceBackend(row)).await.expect("write service_backends");

    // Exercise hydrate_desired through the test-only public wrapper.
    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let hydrated = overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
        &hydrator_reconciler(),
        &target,
        &state,
    )
    .await
    .expect("hydrate_desired must succeed");

    let smh_state = match hydrated {
        AnyState::ServiceMapHydrator(s) => s,
        other => panic!("expected ServiceMapHydrator state, got {other:?}"),
    };

    assert_eq!(smh_state.desired.len(), 1, "exactly one service in desired");
    let desired = smh_state.desired.get(&sid).expect("service_id present in desired");

    assert_eq!(desired.vip, vip, "VIP must match the allocator-issued VIP");
    assert_eq!(desired.backends, backends, "backends must match");

    let expected_fp = overdrive_core::dataplane::fingerprint::fingerprint(&vip, &backends);
    assert_eq!(desired.fingerprint, expected_fp, "fingerprint must match canonical computation");

    // (port, proto) sourced from the listener-bearing intent fact (C3),
    // NOT defaulted to Tcp — the listener declared udp:5353.
    assert_eq!(
        desired.port,
        std::num::NonZeroU16::new(port).expect("non-zero"),
        "port must be sourced from the listener fact"
    );
    assert_eq!(
        desired.proto,
        overdrive_core::dataplane::backend_key::Proto::Udp,
        "proto must be sourced from the udp listener fact, never defaulted to Tcp"
    );

    assert!(smh_state.actual.is_empty(), "actual must be empty from hydrate_desired");
}

/// When no `ServiceBackendRow` exists for a service, `hydrate_desired`
/// returns an empty desired map.
#[tokio::test]
async fn hydrate_desired_returns_empty_when_no_service_backends() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));

    let sid = service_id(99);
    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

    let hydrated = overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
        &hydrator_reconciler(),
        &target,
        &state,
    )
    .await
    .expect("hydrate_desired must succeed");

    let smh_state = match hydrated {
        AnyState::ServiceMapHydrator(s) => s,
        other => panic!("expected ServiceMapHydrator state, got {other:?}"),
    };

    assert!(smh_state.desired.is_empty(), "desired must be empty when no rows exist");
}

/// LWW semantics: when two `ServiceBackendRow`s are written for the
/// same service, the one with the dominating timestamp wins.
#[tokio::test]
async fn service_backends_lww_newer_wins() {
    let obs = SimObservationStore::single_peer(node_id("local"), 42);
    let sid = service_id(7);

    let older_backends = vec![Backend {
        alloc: SpiffeId::from_str("spiffe://overdrive.local/job/old/alloc/a1").expect("spiffe"),
        addr: SocketAddr::from_str("10.0.0.1:8080").expect("addr"),
        weight: 50,
        healthy: true,
    }];
    let newer_backends = vec![Backend {
        alloc: SpiffeId::from_str("spiffe://overdrive.local/job/new/alloc/a1").expect("spiffe"),
        addr: SocketAddr::from_str("10.0.0.2:9090").expect("addr"),
        weight: 200,
        healthy: false,
    }];

    let older = ServiceBackendRow {
        service_id: sid,
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: older_backends,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    };
    let newer = ServiceBackendRow {
        service_id: sid,
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: newer_backends.clone(),
        updated_at: LogicalTimestamp { counter: 5, writer: node_id("writer-1") },
    };

    // Write newer first, then older — older must be rejected.
    obs.write(ObservationRow::ServiceBackend(newer.clone())).await.expect("write newer");
    obs.write(ObservationRow::ServiceBackend(older)).await.expect("write older");

    let rows = obs.service_backends_rows(&sid).await.expect("read");
    assert_eq!(rows.len(), 1, "exactly one row per service_id");
    assert_eq!(rows[0].backends, newer_backends, "newer row must win LWW");
}
