//! Bounded acceptance case: an old Exec attempt exits while its distinct
//! successor is Running and exact-old cleanup is in flight.
//! No allocation observation is authored by this fixture. Registered production
//! reconcilers, the action shim, and the exit observer author every such row.
//! The existing Driver port supplies stop latency; no production seam is added.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    reason = "repository-mandated CONTRACT_SHAPE tokens and diagnostic prose"
)]

use async_trait::async_trait;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::{
    ca::SimCa, clock::SimClock, dataplane::SimDataplane, driver::SimDriver, entropy::SimEntropy,
    observation_store::SimObservationStore,
};
use overdrive_store_local::LocalIntentStore;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

/// Existing driven-port decorator: model the old process watcher/observer
/// completing while ExecDriver::stop awaits its watcher. SimDriver emits the
/// genuine intentional ExitEvent; the production observer constructs its row.
struct ScheduledStop {
    inner: Arc<SimDriver>,
    clock: Arc<SimClock>,
    obs: Arc<SimObservationStore>,
    armed: AtomicBool,
    delay: Duration,
    running_hooks: AtomicUsize,
}

#[async_trait]
impl Driver for ScheduledStop {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.inner.inject_exit_after(&handle.alloc, self.delay, ExitKind::CleanExit);
            // Register the existing SimClock timer before advancing it.
            tokio::task::yield_now().await;
            self.inner.stop(handle).await?;
            self.clock.tick(self.delay);
            for _ in 0..128 {
                tokio::task::yield_now().await;
                if self
                    .obs
                    .alloc_status_rows()
                    .await
                    .unwrap()
                    .iter()
                    .any(|row| row.alloc_id == handle.alloc && row.state == AllocState::Terminated)
                {
                    return Ok(());
                }
            }
            panic!("schedule failed to deliver the real old-attempt exit before stop completion");
        }
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
    fn take_exit_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
    }
    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.running_hooks.fetch_add(1, Ordering::SeqCst);
        self.inner.on_alloc_running(spec);
    }
    fn on_alloc_terminal(&self, alloc: &AllocationId) {
        self.inner.on_alloc_terminal(alloc);
    }
    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

// This diagnostic keeps the production composition, seeded schedule, and
// complete state evidence in one cohesive fixture. Its stderr output is the
// reproduction record for an investigation-only seeded spike.
#[allow(
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "seeded diagnostic requires complete reproduction evidence in one fixture"
)]
async fn drive(seed: u64, contend: bool) {
    eprintln!("allocation-restart-write seed={seed} contend={contend}");
    let tmp = tempfile::TempDir::new().unwrap();
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).unwrap();
    runtime.register(workload_lifecycle()).await.unwrap();
    runtime.register(service_lifecycle()).await.unwrap();
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).unwrap());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let clock = Arc::new(SimClock::new());
    let inner = Arc::new(SimDriver::with_clock(DriverType::Exec, clock.clone()));
    let driver = Arc::new(ScheduledStop {
        inner,
        clock: clock.clone(),
        obs: obs.clone(),
        armed: AtomicBool::new(false),
        delay: Duration::from_millis(1 + seed % 7),
        running_hooks: AtomicUsize::new(0),
    });
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
        std::net::Ipv4Addr::LOCALHOST,
    );
    exit_observer::spawn(
        state.obs.clone(),
        driver.clone(),
        state.lifecycle_events.clone(),
        clock.clone(),
    );
    let svc = ServiceV2::from_submit(ServiceSpecInput {
        id: "restart-write-spike".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Http { host: None, port: 8080, path: "/ready".to_owned() },
            timeout_seconds: 1,
            interval_seconds: 1,
            max_attempts: 1,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .unwrap();
    let key = IntentKey::for_workload(&svc.id);
    let bytes = WorkloadIntent::Service(svc).archive_for_store().unwrap();
    state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
    let alloc = AllocationId::new("alloc-restart-write-spike-0").unwrap();
    let successor = AllocationId::new("alloc-restart-write-spike-1").unwrap();
    let target = TargetResource::new("workload/restart-write-spike").unwrap();
    let workload = ReconcilerName::new("workload-lifecycle").unwrap();
    let service = ReconcilerName::new("service-lifecycle").unwrap();
    let deadline = clock.now() + Duration::from_secs(30);
    run_convergence_tick(&state, &workload, &target, clock.now(), 33, deadline).await.unwrap();
    let started = obs.alloc_status_rows().await.unwrap();
    assert_eq!(started[0].state, AllocState::Running, "seed={seed}");
    clock.tick(Duration::from_secs(2));
    obs.write_probe_result(ProbeResultRow {
        alloc_id: alloc.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Fail { last_fail_reason: "HTTP 302".to_owned() },
        last_observed_at_unix_ms: u64::try_from(clock.unix_now().as_millis()).unwrap(),
        inferred: false,
    })
    .await
    .unwrap();
    run_convergence_tick(&state, &service, &target, clock.now(), 33, deadline).await.unwrap();
    let finalized = obs.alloc_status_rows().await.unwrap();
    assert_eq!(finalized[0].state, AllocState::Failed, "seed={seed}; rows={finalized:?}");
    assert!(finalized[0].terminal.is_some(), "seed={seed}");
    driver.armed.store(contend, Ordering::SeqCst);
    let result = run_convergence_tick(&state, &workload, &target, clock.now(), 33, deadline).await;
    let current = obs.alloc_status_rows().await.unwrap();
    let predecessor_occurrences = obs.alloc_lifecycle_occurrences(&alloc).await.unwrap();
    let successor_occurrences = obs.alloc_lifecycle_occurrences(&successor).await.unwrap();
    let handle = AllocationHandle { alloc: successor.clone(), pid: None };
    let driver_state = driver.status(&handle).await;
    let hooks = driver.running_hooks.load(Ordering::SeqCst);
    eprintln!(
        "seed={seed} initial={:?} finalized={:?} result={result:?} current={current:?} driver={driver_state:?} running_hooks={hooks} predecessor_occurrences={predecessor_occurrences:?} successor_occurrences={successor_occurrences:?}",
        started[0].updated_at, finalized[0].updated_at
    );
    assert!(result.is_ok(), "seed={seed}: successor and exact-old cleanup complete: {result:?}");
    assert_eq!(current.len(), 2, "seed={seed}: predecessor and successor rows stay distinct");
    let predecessor_row = current
        .iter()
        .find(|row| row.alloc_id == alloc)
        .expect("accepted predecessor remains as history");
    let successor_row = current
        .iter()
        .find(|row| row.alloc_id == successor)
        .expect("fresh successor row is accepted");
    assert_eq!(successor_row.state, AllocState::Running, "seed={seed}: successor Running commits");
    assert!(
        matches!(driver_state, Ok(AllocationState::Running)),
        "seed={seed}: distinct successor remains live; got {driver_state:?}"
    );
    assert_eq!(hooks, 2, "seed={seed}: each accepted attempt releases its Running hook once");
    assert_eq!(successor_row.restart_count, 0, "fresh successor starts per-key history at zero");
    assert!(successor_row.last_terminated.is_none());
    assert_eq!(successor_occurrences.len(), 1, "one accepted successor Running occurrence");
    assert_eq!(
        successor_occurrences.last().map(|occurrence| occurrence.to),
        Some(AllocState::Running)
    );
    if contend {
        assert_eq!(
            predecessor_row.state,
            AllocState::Terminated,
            "seed={seed}: late exit updates only the predecessor key"
        );
        assert_eq!(
            predecessor_occurrences.last().map(|occurrence| occurrence.to),
            Some(AllocState::Terminated),
            "seed={seed}: predecessor exit remains on predecessor history"
        );
    } else {
        assert_eq!(
            predecessor_row.state,
            AllocState::Failed,
            "seed={seed}: no exit leaves the predecessor terminal unchanged"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::large_futures,
    reason = "the test intentionally awaits the complete seeded production composition"
)]
#[tokio::test(flavor = "current_thread")]
async fn predecessor_exit_during_cleanup_cannot_replace_fresh_successor_publication() {
    drive(257_203, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::large_futures,
    reason = "the test intentionally awaits the complete seeded production composition"
)]
#[tokio::test(flavor = "current_thread")]
async fn fresh_successor_publication_without_predecessor_exit_control() {
    drive(257_203, false).await;
}
