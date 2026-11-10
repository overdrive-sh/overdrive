//! ADR-0276 composed shim -> hydrate -> backend row, across a restart.
//!
//! One seeded composition on the guest-network path production uses: the
//! action shim's Start writes a Running row carrying the address the guest
//! pool assigned, `ServiceLifecycle` hydrates it and publishes exactly that
//! address as the Service's backend, and after `RestartAllocation` the
//! successor's own pool-assigned address is published (never the predecessor's,
//! never the node address). No allocation observation is authored by this
//! fixture: the registered production reconcilers, the action shim and the exit
//! observer author every row; only the startup-probe observation that drives the
//! restart is written, through the observation port the probe runner uses.
//!
//! Tier 1 (seeded `overdrive-sim` composition). Contract shape: bounded-change.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::guest_network::SharedGuestNetworkOwner as _;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, IntentKey, ResourcesInput, Service, WorkloadIntent};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::id::{AllocationId, NodeId, ServiceId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::{
    ca::SimCa, clock::SimClock, dataplane::SimDataplane, driver::SimDriver, entropy::SimEntropy,
    observation_store::SimObservationStore,
};
use overdrive_store_local::LocalIntentStore;

const LISTENER_PORT: u16 = 8080;

fn running_addr(rows: &[AllocStatusRow], alloc: &AllocationId) -> Ipv4Addr {
    let row = rows.iter().find(|row| row.alloc_id == *alloc).expect("allocation row");
    assert_eq!(row.state, AllocState::Running, "{alloc} is Running; row={row:?}");
    row.workload_addr.expect("a Running row carries its workload address")
}

#[allow(clippy::too_many_lines, clippy::print_stderr)]
async fn drive(seed: u64) {
    eprintln!("service-backend-address-across-restart seed={seed}");
    let tmp = tempfile::TempDir::new().unwrap();
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).unwrap();
    runtime.register(workload_lifecycle()).await.unwrap();
    runtime.register(service_lifecycle()).await.unwrap();
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).unwrap());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
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
        driver.clone(),
        clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
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
    exit_observer::spawn(
        state.obs.clone(),
        driver.clone() as Arc<dyn Driver>,
        state.lifecycle_events.clone(),
        clock.clone(),
    );

    let svc = Service::from_submit(ServiceSpecInput {
        id: "addr-restart".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
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
    .unwrap();
    let intent = WorkloadIntent::Service(svc.clone());
    let key = IntentKey::for_workload(&svc.id);
    let bytes = intent.archive_for_store().unwrap();
    state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
    // The VIP the Service's listener is published behind, through the production allocator.
    let vip = {
        let digest = intent.spec_digest().expect("spec digest");
        let mut guard = state.allocator.lock().await;
        guard.allocate(*digest.as_bytes()).await.expect("allocate vip")
    };
    let service_id = ServiceId::derive(
        &vip,
        std::num::NonZeroU16::new(LISTENER_PORT).unwrap(),
        Proto::Tcp,
        "service-map",
    );

    let predecessor = AllocationId::new("alloc-addr-restart-0").unwrap();
    let successor = AllocationId::new("alloc-addr-restart-1").unwrap();
    let target = TargetResource::new("workload/addr-restart").unwrap();
    let workload = ReconcilerName::new("workload-lifecycle").unwrap();
    let service = ReconcilerName::new("service-lifecycle").unwrap();
    let deadline = clock.now() + Duration::from_secs(30);
    macro_rules! tick {
        ($reconciler:expr) => {
            run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                $reconciler,
                &target,
                clock.now(),
                33,
                deadline,
                guest_network.as_ref(),
            )
            .await
            .unwrap()
        };
    }
    let published = || async {
        let rows = obs.service_backends_rows(&service_id).await.unwrap();
        assert_eq!(rows.len(), 1, "seed={seed}: one backend row for the listener, got {rows:?}");
        rows[0].backends.iter().map(|backend| backend.addr).collect::<Vec<_>>()
    };

    // Start: the shim writes Running with the pool-assigned address; the Service
    // publishes exactly it.
    tick!(&workload);
    let rows = obs.alloc_status_rows().await.unwrap();
    let first_addr = running_addr(&rows, &predecessor);
    assert!(
        GUEST_NODE_PREFIX.contains(&first_addr),
        "seed={seed}: {first_addr} is a guest address"
    );
    tick!(&service);
    assert_eq!(
        published().await,
        vec![SocketAddr::new(IpAddr::V4(first_addr), LISTENER_PORT)],
        "seed={seed}: the backend is the allocation's pool-assigned address",
    );

    // The startup probe fails; the Service finalizes the attempt and the workload
    // lifecycle places a distinct successor.
    clock.tick(Duration::from_secs(2));
    let observed_at = u64::try_from(clock.unix_now().as_millis()).unwrap();
    obs.write_probe_result(ProbeResultRow {
        alloc_id: predecessor.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Fail { last_fail_reason: "HTTP 302".to_owned() },
        last_observed_at_unix_ms: observed_at,
        inferred: false,
        run_seq: ProbeRunSeq::new(u64::max(observed_at, 1)).unwrap_or(ProbeRunSeq::FIRST),
        same_status_runs: SameStatusRuns::ONE,
    })
    .await
    .unwrap();
    tick!(&service);
    tick!(&workload);

    // After the restart the successor's own address is the published backend.
    let rows = obs.alloc_status_rows().await.unwrap();
    let successor_addr = running_addr(&rows, &successor);
    assert!(
        GUEST_NODE_PREFIX.contains(&successor_addr),
        "seed={seed}: {successor_addr} is a guest address"
    );
    tick!(&service);
    assert_eq!(
        published().await,
        vec![SocketAddr::new(IpAddr::V4(successor_addr), LISTENER_PORT)],
        "seed={seed}: after the restart exactly the successor's address is published",
    );
    assert!(
        published().await.iter().all(|addr| addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)),
        "seed={seed}: the node address is never a backend",
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::large_futures,
    reason = "the test intentionally awaits the complete seeded production composition"
)]
#[tokio::test(flavor = "current_thread")]
async fn a_service_publishes_each_attempts_pool_assigned_address_across_a_restart() {
    for seed in [1_u64, 7, 295] {
        Box::pin(drive(seed)).await;
    }
}
