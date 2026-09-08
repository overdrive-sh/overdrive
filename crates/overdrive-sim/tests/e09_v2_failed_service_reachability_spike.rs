//! Diagnostic only: backend convergence after a production-owner same-ID restart.
//! The native VM capture establishes the VM restart path. This smaller seeded
//! composition exercises its driver-independent backend consequence through
//! registered reconcilers, action dispatch, and the production ProbeRunner.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use async_trait::async_trait;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_sim::adapters::{
    ca::SimCa,
    clock::SimClock,
    dataplane::SimDataplane,
    driver::SimDriver,
    entropy::SimEntropy,
    observation_store::SimObservationStore,
    probers::{SimExecProber, SimHttpProber, SimTcpProber},
};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use std::{net::Ipv4Addr, sync::Arc, time::Duration};

/// Existing driven-port composition: simulated process lifecycle plus the same
/// production probe lifecycle hooks that ExecDriver and VmDriver forward.
struct ProbedDriver {
    inner: SimDriver,
    probes: ProbeRunner,
}

#[async_trait]
impl Driver for ProbedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await
    }
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.inner.status(handle).await
    }
    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        self.inner.resize(handle, resources).await
    }
    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle).await;
    }
    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.probes.start_alloc(spec);
    }
    fn on_alloc_terminal(&self, alloc: &AllocationId) {
        self.probes.stop_alloc(alloc);
    }
    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

async fn tick(
    state: &AppState,
    clock: &SimClock,
    owner: &str,
    target: &TargetResource,
    count: u64,
) {
    run_convergence_tick(
        state,
        &ReconcilerName::new(owner).unwrap(),
        target,
        clock.now(),
        count,
        clock.now() + Duration::from_secs(30),
    )
    .await
    .unwrap();
}

/// CONTRACT_SHAPE: bounded-change.
/// Universe: stored Service backend eligibility and the actual persisted
/// ServiceLifecycle terminal decision for the same allocation identity. No
/// observation row, terminal set, or fingerprint is injected by this test.
#[tokio::test(flavor = "current_thread")]
async fn terminal_startup_veto_converges_after_same_id_restart() {
    let seed = 257_209_u64;
    eprintln!("e09-v2-backend-veto seed={seed}");
    let tmp = tempfile::tempdir().unwrap();
    let node = NodeId::new("local").unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let tcp = Arc::new(SimTcpProber::new());
    for _ in 0..100 {
        tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "connection refused".into() });
    }
    let driver = Arc::new(ProbedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        probes: ProbeRunner::new(
            tcp,
            Arc::new(SimHttpProber::new()),
            Arc::new(SimExecProber::new()),
            clock.clone(),
            obs.clone(),
        ),
    });
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).unwrap();
    runtime.register(workload_lifecycle()).await.unwrap();
    runtime.register(service_lifecycle()).await.unwrap();
    runtime.register(overdrive_control_plane::vm_reclamation()).await.unwrap();
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).unwrap());
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
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        Ipv4Addr::LOCALHOST,
    );
    let input = ServiceSpecInput {
        id: "e09-v2-veto".into(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".into(),
            args: vec!["3600".into()],
        }),
        listeners: vec![ListenerInput { port: 18081, protocol: "tcp".into() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".into(), port: 18999 },
            timeout_seconds: 1,
            interval_seconds: 1,
            max_attempts: 3,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    };
    // Validated input is submitted through the existing intent/allocator ports,
    // as in the adjacent owner-path diagnostic. Every allocation/probe/backend
    // observation below is authored by production, never seeded by the test.
    let svc = ServiceV2::from_submit(input).unwrap();
    let key = IntentKey::for_workload(&svc.id);
    let intent = WorkloadIntent::Service(svc);
    state.allocator.lock().await.allocate(*intent.spec_digest().unwrap().as_bytes()).await.unwrap();
    let bytes = intent.archive_for_store().unwrap();
    state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
    let target = TargetResource::new("workload/e09-v2-veto").unwrap();
    tick(&state, &clock, "workload-lifecycle", &target, 10).await;
    tick(&state, &clock, "service-lifecycle", &target, 11).await;
    assert_eq!(obs.alloc_status_rows().await.unwrap()[0].state, AllocState::Running);
    assert!(
        obs.all_service_backends_rows().await.unwrap()[0].backends[0].healthy,
        "seed={seed}: no-readiness preterminal control is eligible"
    );
    for count in 12..16 {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        clock.tick(Duration::from_millis(1000 + seed % 11));
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        tick(&state, &clock, "service-lifecycle", &target, count).await;
        // Inspect the deciding tick before the sole publisher
        // projects the now-Failed membership as empty on its next tick.
        if obs.alloc_status_rows().await.unwrap()[0].state == AllocState::Failed {
            break;
        }
    }
    let ended = obs.alloc_status_rows().await.unwrap().remove(0);
    eprintln!("seed={seed}: terminal={ended:?}");
    assert_eq!(ended.state, AllocState::Failed);
    assert!(
        !obs.all_service_backends_rows().await.unwrap()[0].backends[0].healthy,
        "seed={seed}: deciding startup tick withdrew eligibility"
    );
    // Accepted Failed observation wakes membership convergence before the
    // workload owner's next same-ID restart. This is not a forced task abort.
    tick(&state, &clock, "service-lifecycle", &target, 16).await;
    assert!(obs.all_service_backends_rows().await.unwrap()[0].backends.is_empty());
    tick(&state, &clock, "workload-lifecycle", &target, 17).await;
    let current = obs.alloc_status_rows().await.unwrap().remove(0);
    assert_eq!(current.state, AllocState::Running);
    assert_eq!(current.alloc_id, ended.alloc_id);
    tick(&state, &clock, "service-lifecycle", &target, 18).await;
    for count in 19..25 {
        tick(&state, &clock, "service-lifecycle", &target, count).await;
    }
    let view = state.runtime.loaded_service_lifecycle_views_for_test(
        &ReconcilerName::new("service-lifecycle").unwrap(),
    );
    eprintln!("seed={seed}: runtime view={view:?}");
    assert!(
        view.as_ref().unwrap().get(&target).unwrap().terminal_announced.contains(&ended.alloc_id),
        "seed={seed}: same allocation identity still carries the production owner's terminal veto"
    );
    let rows = obs.all_service_backends_rows().await.unwrap();
    eprintln!("seed={seed}: converged backend rows={rows:?}");
    assert!(
        !rows[0].backends[0].healthy,
        "seed={seed}: stored backend regained eligibility despite unchanged ServiceLifecycle terminal veto"
    );
}
