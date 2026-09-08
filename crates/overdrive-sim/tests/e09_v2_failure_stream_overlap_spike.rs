//! Diagnostic of E09-v2's overlapping healthy-stop/failure-submit schedule.
//! <!-- DES-ENFORCEMENT : exempt -->
//! No lifecycle state is seeded. Production WorkloadLifecycle, ServiceLifecycle,
//! action dispatch, ProbeRunner, and Service stream compose over existing Sim ports.
//! The selected serial per-tick order models a batch already drained by the loop;
//! this is not a broker scheduling test or native VM stop-duration measurement.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use async_trait::async_trait;
use futures::StreamExt;
use overdrive_control_plane::api::IdempotencyOutcome;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::streaming::{
    ServiceSubmitEvent, build_service_accepted, build_service_stream,
};
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
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
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
use tokio::sync::mpsc;

/// Existing driven-port composition: simulated process lifecycle plus the same
/// production probe lifecycle hooks that ExecDriver and VmDriver forward.
struct ProbedDriver {
    inner: SimDriver,
    probes: ProbeRunner,
    clock: Arc<SimClock>,
    stopping: mpsc::UnboundedSender<AllocationId>,
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
        self.inner.stop(handle).await?;
        // Existing Driver port: status becomes NotFound before the awaited
        // stop effect completes, as VmDriver::stop does. No observation write.
        // Select the native 2s shutdown-request + 10s VMM-grace partition.
        if handle.alloc.as_str().starts_with("alloc-e09-healthy-") {
            self.stopping.send(handle.alloc.clone()).unwrap();
            self.clock.sleep(Duration::from_secs(12)).await;
        }
        Ok(())
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

async fn drive(overlap: bool) {
    let seed = 257_210_u64;
    eprintln!("e09-v2-failure-stream seed={seed} overlap={overlap}");
    let (stopping, mut stopped_rx) = mpsc::unbounded_channel();
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
        clock: clock.clone(),
        stopping,
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
        id: "e09-v2-failure-stream".into(),
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

    // Nine peers remain from a ten-pair cohort when its first worker submits
    // the failure Service. Healthy allocation rows are authored by real starts.
    let mut healthy_targets = Vec::new();
    for index in 0..9 {
        let mut healthy = input.clone();
        healthy.id = format!("e09-healthy-{index}");
        healthy.startup_probes.clear();
        let (id, target, _) = submit(&state, healthy).await;
        tick(&state, &clock, "workload-lifecycle", &target, 10 + index).await;
        assert!(
            obs.alloc_status_rows()
                .await
                .unwrap()
                .iter()
                .any(|row| row.workload_id == id && row.state == AllocState::Running)
        );
        // Same existing intent port written by handlers::stop_workload; no
        // synthetic terminal rows and no direct StopAllocation injection.
        state.store.put_if_absent(IntentKey::for_workload_stop(&id).as_bytes(), b"").await.unwrap();
        healthy_targets.push(target);
    }
    let state = Arc::new(state);
    let failure = if overlap { Some(submit(&state, input.clone()).await) } else { None };
    let stream_task = if let Some((id, _, accepted)) = &failure {
        Some(start_stream(&state, id.clone(), accepted.clone()).await)
    } else {
        None
    };

    let queue_state = state.clone();
    let queue_clock = clock.clone();
    let stop_batch = tokio::spawn(async move {
        for (index, target) in healthy_targets.iter().enumerate() {
            tick(&queue_state, &queue_clock, "workload-lifecycle", target, 30 + index as u64).await;
        }
    });
    for index in 0..9 {
        let alloc = stopped_rx.recv().await.unwrap();
        // Sender and clock sleep are consecutive polls in the same task.
        // Let it park before advancing simulated time; no wall-clock sleep.
        for _ in 0..(64 + seed % 17) {
            tokio::task::yield_now().await;
        }
        clock.tick(Duration::from_secs(12));
        for _ in 0..(64 + seed % 17) {
            tokio::task::yield_now().await;
        }
        eprintln!("seed={seed}: healthy stop {} completed logical grace for {alloc}", index + 1);
    }
    stop_batch.await.unwrap();
    assert!(obs.alloc_status_rows().await.unwrap().iter().all(|row| row.state.is_terminal()));
    let (id, target, accepted) = match failure {
        Some(value) => value,
        None => submit(&state, input).await,
    };
    let stream_task = match stream_task {
        Some(task) => task,
        None => start_stream(&state, id.clone(), accepted).await,
    };
    tick(&state, &clock, "workload-lifecycle", &target, 50).await;
    // A real supervisor writes three distinct refused probe observations;
    // the real ServiceLifecycle derives the attempt count and terminal.
    for count in 51..56 {
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        clock.tick(Duration::from_millis(1000 + seed % 11));
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        tick(&state, &clock, "service-lifecycle", &target, count).await;
        if obs
            .alloc_status_rows()
            .await
            .unwrap()
            .iter()
            .any(|row| row.workload_id == id && row.state == AllocState::Failed)
        {
            break;
        }
    }
    let ended = obs
        .alloc_status_rows()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.workload_id == id)
        .unwrap();
    assert!(
        matches!(
            &ended.terminal,
            Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed { attempts: 3, .. }
            })
        ),
        "seed={seed}: production must author the actual startup failure: {ended:?}"
    );
    tick(&state, &clock, "workload-lifecycle", &target, 60).await;
    let recovered = obs
        .alloc_status_rows()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.workload_id == id)
        .unwrap();
    assert_eq!(recovered.state, AllocState::Running);
    assert_eq!(recovered.restart_count, 1);
    assert_eq!(recovered.last_terminated.as_ref().unwrap().terminal, ended.terminal);
    assert_eq!(recovered.last_terminated.as_ref().unwrap().detail, None);
    tick(&state, &clock, "service-lifecycle", &target, 61).await;
    let backends = obs.all_service_backends_rows().await.unwrap();
    assert_eq!(backends.len(), 1, "seed={seed}: the failure Service must have its backend row");
    assert_eq!(
        backends[0].backends.len(),
        1,
        "seed={seed}: recovered Running membership is present"
    );
    assert!(
        !backends[0].backends[0].healthy,
        "seed={seed}: startup failure must remain ineligible across same-ID recovery"
    );
    let terminal = stream_task.await.unwrap();
    eprintln!(
        "seed={seed}: authored={:?}; recovered_state={:?}; recovered_last_terminal={:?}; stream={terminal:?}",
        ended.terminal,
        recovered.state,
        recovered.last_terminated.as_ref().unwrap().terminal
    );
    // Stop tasks have completed; cleanly retire only our remaining supervisor.
    state.drivers.get(DriverType::Exec).unwrap().on_alloc_terminal(&recovered.alloc_id);
    assert!(
        matches!(
            terminal,
            ServiceSubmitEvent::Failed {
                reason: ServiceFailureReason::StartupProbeFailed { .. },
                ..
            }
        ),
        "seed={seed}: E09 requires StartupProbeFailed on the original unbound deploy; got {terminal:?}"
    );
}

async fn submit(
    state: &AppState,
    input: ServiceSpecInput,
) -> (overdrive_core::WorkloadId, TargetResource, ServiceSubmitEvent) {
    let svc = ServiceV2::from_submit(input).unwrap();
    let id = svc.id.clone();
    let key = IntentKey::for_workload(&id);
    let intent = WorkloadIntent::Service(svc);
    let digest = intent.spec_digest().unwrap();
    state.allocator.lock().await.allocate(*digest.as_bytes()).await.unwrap();
    let bytes = intent.archive_for_store().unwrap();
    state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
    let target = TargetResource::new(&format!("workload/{id}")).unwrap();
    (
        id,
        target,
        build_service_accepted(
            digest.to_string(),
            key.as_str().into(),
            IdempotencyOutcome::Inserted,
        ),
    )
}

async fn start_stream(
    state: &AppState,
    id: overdrive_core::WorkloadId,
    accepted: ServiceSubmitEvent,
) -> tokio::task::JoinHandle<ServiceSubmitEvent> {
    let stream = build_service_stream(state.clone(), id, accepted);
    let mut stream = Box::pin(stream);
    let accepted = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        serde_json::from_slice::<ServiceSubmitEvent>(&accepted).unwrap(),
        ServiceSubmitEvent::Accepted { .. }
    ));
    let task = tokio::spawn(async move {
        let bytes = stream.next().await.unwrap().unwrap();
        let event = serde_json::from_slice(&bytes).unwrap();
        assert!(stream.next().await.is_none(), "exactly one stream terminal");
        event
    });
    for _ in 0..128 {
        tokio::task::yield_now().await;
    }
    task
}

/// CONTRACT_SHAPE: bounded-change.
/// Universe: actual stream terminal, production-authored allocation history,
/// and backend eligibility. Nine pre-existing healthy stops must not cause
/// the original E09 unbound deploy to report only a generic Timeout.
#[tokio::test(flavor = "current_thread")]
async fn overlapping_healthy_stops_preserve_e09_startup_failure_result() {
    drive(true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Same owners, inputs and stop delays; failure submitted after peers stop.
/// The original stream reports StartupProbeFailed and remains never-Stable.
#[tokio::test(flavor = "current_thread")]
async fn completed_healthy_stops_control_reports_startup_failure() {
    drive(false).await;
}
