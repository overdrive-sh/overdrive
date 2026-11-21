//! workload-spiffe-identity D1 (ADR-0344), validation item V-3 — the backend a
//! Service publishes for a Running allocation presents the identity of its
//! WORKLOAD.
//!
//! One seeded composition on the production path: the action shim's Start writes
//! the Running row (address from the guest pool), `ServiceLifecycle` hydrates it
//! and publishes the Service's backend row. The observable is the identity the
//! published backend carries (the `Backend` field is named once, in
//! [`published_identity`]). No allocation observation is authored by this fixture.
//!
//! The same derivation builds the identity the allocation itself presents
//! (`allocation_spec_for_live_intent`, S-WSI-06 and the hydrator/dataplane tests in
//! `overdrive-sim`); this test pins the publishing site.
//!
//! RED today: the published identity is the per-allocation
//! `.../workload/<w>/alloc/<a>`.
//!
//! Tier 1 (seeded `overdrive-sim` composition). CONTRACT_SHAPE: bounded-change.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::guest_network::SharedGuestNetworkOwner as _;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, IntentKey, ResourcesInput, Service, WorkloadIntent};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::{
    ca::SimCa, clock::SimClock, dataplane::SimDataplane, driver::SimDriver, entropy::SimEntropy,
    observation_store::SimObservationStore,
};
use overdrive_store_local::LocalIntentStore;

use overdrive_testing::placement;

placement!(host_pure);

const LISTENER_PORT: u16 = 8080;
const SEED: u64 = 295;

/// The identity a published backend presents. The single place the `Backend`
/// field holding the identity is named.
const fn published_identity(backend: &Backend) -> &SpiffeId {
    &backend.workload_identity
}

/// A Running allocation of workload `payments` is published behind
/// `spiffe://overdrive.local/workload/payments`, the wire shape pinned by the
/// `SpiffeId::for_workload` constructor tests (spelled out here so this RED is the
/// per-allocation identity, not the constructor scaffold).
#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "a Running allocation is published behind its workload's identity")]
async fn a_running_allocation_is_published_behind_its_workloads_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
    runtime.register(service_lifecycle()).await.expect("register service-lifecycle");
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("store"));
    let node = NodeId::new("local").expect("node id");
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), SEED));
    let clock = Arc::new(SimClock::new());
    let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
    let mtls_worker = crate::acceptance::serve_ports::worker();
    mtls_worker.start_shared_owner().await.expect("the shared intercept owner starts");
    let guest_network = crate::acceptance::serve_ports::owner();
    guest_network.converge_shared().await.expect("the fixture owner reports its bound ports");
    let exec_wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let allocator =
        overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs.clone(),
        Arc::new(runtime),
        driver,
        clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(SEED)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        Ipv4Addr::LOCALHOST,
        mtls_worker,
        Arc::clone(&guest_network)
            as Arc<dyn overdrive_control_plane::guest_network::SharedGuestNetworkOwner>,
        exec_wiring.gate(),
        crate::acceptance::serve_ports::pool(),
    );
    assert!(
        exec_wiring.supervisor().open_after_boot(),
        "the EXEC gate opens only after the fixture owner has converged"
    );

    let service = Service::from_submit(ServiceSpecInput {
        id: "payments".to_owned(),
        replicas: 1,
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(64 * 1024 * 1024),
        },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            volume: None,
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: LISTENER_PORT, protocol: "tcp".to_owned() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Http {
                host: None,
                port: LISTENER_PORT,
                path: "/ready".to_owned(),
            },
            timeout_seconds: 1,
            interval_seconds: 1,
            max_attempts: 1,
            failure_threshold: None,
            success_threshold: None,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("a valid service");
    let intent = WorkloadIntent::Service(service.clone());
    state
        .store
        .put(
            IntentKey::for_workload(&service.id).as_bytes(),
            intent.archive_for_store().expect("archive").as_ref(),
        )
        .await
        .expect("put the intent");
    let vip = {
        let digest = intent.spec_digest().expect("spec digest");
        let mut guard = state.allocator.lock().await;
        guard.allocate(*digest.as_bytes()).await.expect("allocate vip")
    };
    let service_id = ServiceId::derive(
        &vip,
        std::num::NonZeroU16::new(LISTENER_PORT).expect("non-zero"),
        Proto::Tcp,
        "service-map",
    );

    let target = TargetResource::new("workload/payments").expect("target");
    let deadline = clock.now() + Duration::from_secs(30);
    for reconciler in ["workload-lifecycle", "service-lifecycle"] {
        run_convergence_tick_with_guest_network_provisioner_for_test(
            &state,
            &ReconcilerName::new(reconciler).expect("reconciler name"),
            &target,
            clock.now(),
            33,
            deadline,
            guest_network.as_ref(),
        )
        .await
        .expect("convergence tick");
    }

    let rows = obs.service_backends_rows(&service_id).await.expect("service_backends rows");
    assert_eq!(rows.len(), 1, "one backend row for the listener, got {rows:?}");
    let [backend] = rows[0].backends.as_slice() else {
        panic!("one Running allocation publishes one backend, got {:?}", rows[0].backends);
    };
    assert_eq!(
        published_identity(backend).as_str(),
        "spiffe://overdrive.local/workload/payments",
        "a Running allocation is published behind its workload's identity"
    );
}
