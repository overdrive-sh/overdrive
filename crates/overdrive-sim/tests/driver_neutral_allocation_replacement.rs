//! Seeded GH #284 corrective acceptance at the existing convergence-runtime
//! and action-shim driving ports.
//!
//! The fixtures replace only existing driven ports. Production chooses the
//! predecessor, successor, rows, ordering, and cleanup identities. No row,
//! lifecycle state, retry, action, or replacement policy is manufactured by a
//! test-only production seam.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::too_many_arguments, clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "seed diagnostics and exact Contract Shape lines are required acceptance evidence"
)]

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::action_shim::{
    AllocDriverIndex, MtlsInterceptLifecycle, ShimError, WorkloadNetworkProvisioner,
    dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_network_provisioner_for_test,
};
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
};
use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_control_plane::view_store::{
    ProbeError as ViewStoreProbeError, Result as ViewStoreResult, ViewStore,
};
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverStartClass, DriverStartFailure, DriverType, ExecPayload, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, LagAwareSubscription,
    LogicalTimestamp, NodeHealthRow, ObservationStore, ObservationStoreError, ObservationWrite,
    ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow, TransitionSource,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_sim::adapters::{
    SimMtlsInterceptLifecycle, SimMtlsInterceptLifecycleState, SimVmHostState,
};
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;

const SEED: u64 = 284_105_106;

#[derive(Clone, Debug, Eq, PartialEq)]
enum TraceEvent {
    Start(AllocationId),
    Stop(AllocationId),
    Provision(String),
    Teardown(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartBehavior {
    Success,
    IoFailure,
    Rejected,
}

struct RecordingDriver {
    driver_type: DriverType,
    behavior: StartBehavior,
    trace: Arc<Mutex<Vec<TraceEvent>>>,
    starts: Mutex<Vec<AllocationId>>,
    stops: Mutex<Vec<AllocationId>>,
    start_entered: tokio::sync::Notify,
    stop_entered: tokio::sync::Notify,
    stop_release: tokio::sync::Notify,
    block_predecessor_stop: AtomicBool,
    fail_predecessor_stop: AtomicBool,
    predecessor: AllocationId,
    view_persisted: Option<Arc<AtomicBool>>,
}

impl RecordingDriver {
    fn new(driver_type: DriverType, predecessor: AllocationId) -> Self {
        Self {
            driver_type,
            behavior: StartBehavior::Success,
            trace: Arc::new(Mutex::new(Vec::new())),
            starts: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            start_entered: tokio::sync::Notify::new(),
            stop_entered: tokio::sync::Notify::new(),
            stop_release: tokio::sync::Notify::new(),
            block_predecessor_stop: AtomicBool::new(false),
            fail_predecessor_stop: AtomicBool::new(false),
            predecessor,
            view_persisted: None,
        }
    }

    const fn with_behavior(mut self, behavior: StartBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    fn with_blocked_predecessor_stop(self) -> Self {
        self.block_predecessor_stop.store(true, Ordering::SeqCst);
        self
    }

    fn with_failed_predecessor_stop(self) -> Self {
        self.fail_predecessor_stop.store(true, Ordering::SeqCst);
        self
    }

    fn with_view_persisted(mut self, persisted: Arc<AtomicBool>) -> Self {
        self.view_persisted = Some(persisted);
        self
    }
}

#[async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        if let Some(persisted) = &self.view_persisted {
            assert!(
                persisted.load(Ordering::SeqCst),
                "successor effects must not start before the View write completes"
            );
        }
        self.starts.lock().push(spec.alloc.clone());
        self.trace.lock().push(TraceEvent::Start(spec.alloc.clone()));
        self.start_entered.notify_waiters();
        match self.behavior {
            StartBehavior::Success => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(42) })
            }
            StartBehavior::IoFailure => {
                Err(DriverError::Io(io::Error::other("injected successor start failure")))
            }
            StartBehavior::Rejected => Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Unclassified { driver: self.driver_type },
                    detail: "injected successor rejection".to_owned(),
                },
            }),
        }
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stops.lock().push(handle.alloc.clone());
        self.trace.lock().push(TraceEvent::Stop(handle.alloc.clone()));
        if handle.alloc == self.predecessor {
            self.stop_entered.notify_waiters();
            if self.block_predecessor_stop.load(Ordering::SeqCst) {
                self.stop_release.notified().await;
            }
            if self.fail_predecessor_stop.load(Ordering::SeqCst) {
                return Err(DriverError::Io(io::Error::other(
                    "injected predecessor cleanup failure",
                )));
            }
        }
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingNetwork {
    trace: Arc<Mutex<Vec<TraceEvent>>>,
}

impl WorkloadNetworkProvisioner for RecordingNetwork {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        self.trace.lock().push(TraceEvent::Provision(workload.netns.as_str().to_owned()));
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.trace.lock().push(TraceEvent::Teardown(workload.netns.as_str().to_owned()));
        Ok(())
    }
}

struct RecordingRedbViewStore {
    inner: RedbViewStore,
    persisted: Arc<AtomicBool>,
}

#[async_trait]
impl ViewStore for RecordingRedbViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> ViewStoreResult<BTreeMap<TargetResource, Vec<u8>>> {
        self.inner.bulk_load_bytes(reconciler).await
    }

    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        cbor: &[u8],
    ) -> ViewStoreResult<()> {
        self.inner.write_through_bytes(reconciler, target, cbor).await?;
        self.persisted.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn delete(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
    ) -> ViewStoreResult<()> {
        self.inner.delete(reconciler, target).await
    }

    async fn probe(&self) -> Result<(), ViewStoreProbeError> {
        self.inner.probe().await
    }
}

fn aid(raw: &str) -> AllocationId {
    AllocationId::new(raw).expect("valid allocation id")
}

fn wid(raw: &str) -> WorkloadId {
    WorkloadId::new(raw).expect("valid workload id")
}

fn nid(raw: &str) -> NodeId {
    NodeId::new(raw).expect("valid node id")
}

fn tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_284)),
        tick: 284,
        deadline: now + Duration::from_secs(2),
    }
}

fn payload(driver_type: DriverType) -> DriverPayload {
    match driver_type {
        DriverType::Exec => DriverPayload::Exec(ExecPayload {
            command: "/bin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
        }),
        DriverType::Vm => DriverPayload::Vm(VmPayload {
            command: "/sbin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
            kernel: Path::new("/srv/vm/kernel").to_path_buf(),
            rootfs: Path::new("/srv/vm/rootfs.ext4").to_path_buf(),
        }),
        other => panic!("fixture covers currently composed replacement drivers, got {other:?}"),
    }
}

fn successor_spec(
    driver_type: DriverType,
    workload: &WorkloadId,
    successor: &AllocationId,
) -> AllocationSpec {
    AllocationSpec {
        alloc: successor.clone(),
        identity: SpiffeId::for_allocation(workload, successor),
        driver: payload(driver_type),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn predecessor_row(
    predecessor: &AllocationId,
    workload: &WorkloadId,
    state: AllocState,
) -> AllocStatusRow {
    let node = nid("local");
    AllocStatusRow {
        alloc_id: predecessor.clone(),
        workload_id: workload.clone(),
        node_id: node.clone(),
        state,
        updated_at: LogicalTimestamp { counter: 7, writer: node },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(23),
            signal: None,
            stderr_tail: Some("predecessor failed".to_owned()),
        }),
        detail: Some("accepted predecessor".to_owned()),
        terminal: None,
        stderr_tail: Some("predecessor failed".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(10))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 6,
    }
}

async fn dispatch_one(
    action: Action,
    driver: Arc<RecordingDriver>,
    obs: &dyn ObservationStore,
    alloc_drivers: &AllocDriverIndex,
    slots: &NetSlotAllocator,
    network: &dyn WorkloadNetworkProvisioner,
    mtls_lifecycle: Option<&dyn MtlsInterceptLifecycle>,
) -> Result<(), ShimError> {
    let tmp = tempfile::tempdir().expect("temporary intent store");
    let intent: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let allocator = overdrive_control_plane::test_default_allocator(intent);
    let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
    let driver_dyn: Arc<dyn Driver> = driver;
    registry.insert(driver_dyn);
    let (bus, _rx) = tokio::sync::broadcast::channel(8);
    dispatch_with_network_provisioner(
        vec![action],
        &registry,
        alloc_drivers,
        obs,
        &SimDataplane::new(),
        &SimCa::new(Arc::new(SimEntropy::new(SEED))),
        &SimClock::new(),
        &IdentityMgr::new(None),
        &bus,
        &tick(),
        &nid("writer"),
        allocator,
        &Mutex::new(EvaluationBroker::new()),
        None,
        mtls_lifecycle,
        slots,
        network,
        &SimVmHostState::new(),
    )
    .await
}

async fn seeded_predecessor(
    driver_type: DriverType,
) -> (Arc<SimObservationStore>, AllocationId, AllocationId, WorkloadId) {
    let stem = driver_type.as_str();
    let workload = wid(&format!("{stem}-ordering"));
    let predecessor = aid(&format!("alloc-{stem}-ordering-0"));
    let successor = aid(&format!("alloc-{stem}-ordering-1"));
    let obs = Arc::new(SimObservationStore::single_peer(nid("local"), SEED));
    obs.write_alloc_lifecycle(
        predecessor_row(&predecessor, &workload, AllocState::Failed),
        TransitionSource::Reconciler,
    )
    .await
    .expect("seed accepted predecessor");
    (obs, predecessor, successor, workload)
}

/// S-284-SIM-01 — the successor starts and publishes before a blocked
/// predecessor cleanup for both Exec and VM; the action remains in flight only
/// for the one post-successor exact-old cleanup attempt.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending corrective re-DELIVER roadmap: successor-first exact-old cleanup ordering"]
async fn successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver() {
    eprintln!("seed={SEED}: successor-first driver-neutral replacement");
    for driver_type in [DriverType::Exec, DriverType::Vm] {
        let (obs, predecessor, successor, workload) = seeded_predecessor(driver_type).await;
        let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
        let driver = Arc::new(
            RecordingDriver::new(driver_type, predecessor.clone()).with_blocked_predecessor_stop(),
        );
        let slots = NetSlotAllocator::new();
        slots
            .adopt(predecessor.clone(), NetSlot::new(7).expect("valid old slot"))
            .expect("old slot is owned");
        let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };
        let mtls = SimMtlsInterceptLifecycle::new();
        mtls.start_alloc(&successor_spec(driver_type, &workload, &predecessor))
            .await
            .expect("predecessor mTLS lifecycle is live");
        let alloc_drivers = AllocDriverIndex::default();
        alloc_drivers.lock().insert(predecessor.clone(), driver_type);
        let action = Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: successor_spec(driver_type, &workload, &successor),
            kind: WorkloadKind::Service,
        };

        let start_entered = driver.start_entered.notified();
        let stop_entered = driver.stop_entered.notified();
        tokio::pin!(start_entered);
        tokio::pin!(stop_entered);
        let dispatch = dispatch_one(
            action,
            Arc::clone(&driver),
            obs.as_ref(),
            &alloc_drivers,
            &slots,
            &network,
            Some(&mtls),
        );
        tokio::pin!(dispatch);

        tokio::select! {
            () = &mut start_entered => {}
            () = &mut stop_entered => panic!(
                "MISSING_CORRECTED_BEHAVIOR seed={SEED}: predecessor cleanup began before \
                 successor start for {driver_type}"
            ),
            result = &mut dispatch => panic!(
                "replacement returned before either owned effect was observed: {result:?}"
            ),
        }
        tokio::select! {
            () = &mut stop_entered => {}
            result = &mut dispatch => panic!(
                "replacement returned without the required post-successor old cleanup: {result:?}"
            ),
        }

        let fresh = obs
            .alloc_status_row(&successor)
            .await
            .unwrap()
            .expect("successor Running row is accepted before old cleanup completes");
        assert_eq!(fresh.state, AllocState::Running);
        assert_eq!(fresh.restart_count, 0);
        assert!(fresh.last_terminated.is_none());
        assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before.clone()));
        assert_eq!(alloc_drivers.lock().get(&successor), Some(&driver_type));
        assert_eq!(alloc_drivers.lock().get(&predecessor), Some(&driver_type));
        let mtls_while_old_cleanup_blocked = mtls.snapshot();
        assert_eq!(
            mtls_while_old_cleanup_blocked.allocations.get(&successor),
            Some(&SimMtlsInterceptLifecycleState::Live),
            "successor mTLS is live before old cleanup begins"
        );
        assert_eq!(
            mtls_while_old_cleanup_blocked.allocations.get(&predecessor),
            Some(&SimMtlsInterceptLifecycleState::Live),
            "blocked old cleanup still owns only predecessor mTLS"
        );

        driver.stop_release.notify_one();
        dispatch.await.expect("successful successor and old cleanup");
        assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));
        assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&successor));
        assert_eq!(driver.stops.lock().as_slice(), std::slice::from_ref(&predecessor));
        assert!(!alloc_drivers.lock().contains_key(&predecessor));
        assert_eq!(alloc_drivers.lock().get(&successor), Some(&driver_type));
        let mtls_after = mtls.snapshot();
        assert!(!mtls_after.allocations.contains_key(&predecessor));
        assert_eq!(
            mtls_after.allocations.get(&successor),
            Some(&SimMtlsInterceptLifecycleState::Live)
        );
        assert!(!slots.snapshot().contains_key(&predecessor));
        if driver_type == DriverType::Vm {
            assert!(slots.snapshot().contains_key(&successor));
        }

        let trace = driver.trace.lock().clone();
        let start = trace
            .iter()
            .position(|event| event == &TraceEvent::Start(successor.clone()))
            .expect("successor start event");
        let stop = trace
            .iter()
            .position(|event| event == &TraceEvent::Stop(predecessor.clone()))
            .expect("predecessor stop event");
        assert!(start < stop, "successor start precedes predecessor stop: {trace:?}");
    }
}

/// S-284-SIM-02 — the four successor/cleanup outcome partitions keep the
/// successor error primary when both fail and attempt exact-old cleanup once.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending corrective re-DELIVER roadmap: successor and cleanup failure precedence"]
async fn successor_and_cleanup_outcomes_follow_the_ratified_precedence_table() {
    eprintln!("seed={SEED}: successor/cleanup precedence table");
    for (start_behavior, cleanup_fails) in [
        (StartBehavior::Success, false),
        (StartBehavior::Success, true),
        (StartBehavior::IoFailure, false),
        (StartBehavior::IoFailure, true),
    ] {
        let (obs, predecessor, successor, workload) = seeded_predecessor(DriverType::Exec).await;
        let mut driver = RecordingDriver::new(DriverType::Exec, predecessor.clone())
            .with_behavior(start_behavior);
        if cleanup_fails {
            driver = driver.with_failed_predecessor_stop();
        }
        let driver = Arc::new(driver);
        let slots = NetSlotAllocator::new();
        let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };
        let index = AllocDriverIndex::default();
        index.lock().insert(predecessor.clone(), DriverType::Exec);

        let result = dispatch_one(
            Action::RestartAllocation {
                alloc_id: predecessor.clone(),
                spec: successor_spec(DriverType::Exec, &workload, &successor),
                kind: WorkloadKind::Service,
            },
            Arc::clone(&driver),
            obs.as_ref(),
            &index,
            &slots,
            &network,
            None,
        )
        .await;

        match (start_behavior, cleanup_fails, result) {
            (StartBehavior::Success, false, Ok(())) => {}
            (StartBehavior::Success, true, Err(ShimError::Driver(DriverError::Io(error)))) => {
                assert!(error.to_string().contains("predecessor cleanup"));
            }
            (StartBehavior::IoFailure, _, Err(ShimError::Driver(DriverError::Io(error)))) => {
                assert!(
                    error.to_string().contains("successor start"),
                    "successor failure remains primary when both fail: {error}"
                );
            }
            partition => panic!("MISSING_CORRECTED_BEHAVIOR: wrong precedence {partition:?}"),
        }
        assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&successor));
        assert_eq!(driver.stops.lock().as_slice(), std::slice::from_ref(&predecessor));
        let trace = driver.trace.lock().clone();
        assert_eq!(
            trace.iter().filter(|event| event == &&TraceEvent::Stop(predecessor.clone())).count(),
            1,
            "one predecessor cleanup attempt per action"
        );
        let start = trace
            .iter()
            .position(|event| event == &TraceEvent::Start(successor.clone()))
            .expect("successor attempt");
        let stop = trace
            .iter()
            .position(|event| event == &TraceEvent::Stop(predecessor.clone()))
            .expect("old cleanup attempt");
        assert!(start < stop, "successor outcome precedes cleanup: {trace:?}");
    }
}

/// S-284-SIM-03 — an accepted StartRejected successor is published at its
/// fresh key with zero/None per-key history; the predecessor remains immutable
/// and can no longer be numeric-current.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending corrective re-DELIVER roadmap: fresh-key rejected successor publication"]
async fn accepted_failed_successor_publishes_at_fresh_key_with_zero_history() {
    let (obs, predecessor, successor, workload) = seeded_predecessor(DriverType::Exec).await;
    let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
    let driver = Arc::new(
        RecordingDriver::new(DriverType::Exec, predecessor.clone())
            .with_behavior(StartBehavior::Rejected),
    );
    let index = AllocDriverIndex::default();
    index.lock().insert(predecessor.clone(), DriverType::Exec);

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: successor_spec(DriverType::Exec, &workload, &successor),
            kind: WorkloadKind::Service,
        },
        Arc::clone(&driver),
        obs.as_ref(),
        &index,
        &NetSlotAllocator::new(),
        &RecordingNetwork::default(),
        None,
    )
    .await
    .expect("StartRejected is represented as a Failed successor row");

    let failed = obs.alloc_status_row(&successor).await.unwrap().expect("fresh Failed row");
    assert_eq!(failed.state, AllocState::Failed);
    assert_eq!(failed.restart_count, 0);
    assert!(failed.last_terminated.is_none());
    assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));
    assert_eq!(driver.starts.lock().as_slice(), &[successor]);
    assert_eq!(driver.stops.lock().as_slice(), &[predecessor]);
}

struct RejectFreshPublication {
    inner: Arc<SimObservationStore>,
    successor: AllocationId,
    running_attempts: AtomicUsize,
}

#[async_trait]
impl ObservationStore for RejectFreshPublication {
    async fn write(&self, row: ObservationWrite) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        if current.alloc_id == self.successor && current.state == AllocState::Running {
            self.running_attempts.fetch_add(1, Ordering::SeqCst);
            return Ok(None);
        }
        self.inner.write_alloc_lifecycle(current, source).await
    }

    async fn alloc_lifecycle_occurrences(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.alloc_lifecycle_occurrences(alloc_id).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        self.inner.subscribe_all_events().await
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_rows().await
    }

    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_row(alloc_id).await
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        self.inner.node_health_rows().await
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
        self.inner.issued_certificate_rows().await
    }

    async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }

    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        self.inner.service_hydration_results_rows(service_id).await
    }

    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
        self.inner.reconcile_conflict_rows(service_id).await
    }

    async fn write_probe_result(&self, row: ProbeResultRow) -> Result<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }

    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }

    async fn workflow_terminal_rows(
        &self,
    ) -> Result<Vec<(CorrelationKey, WorkflowStatus)>, ObservationStoreError> {
        self.inner.workflow_terminal_rows().await
    }

    async fn workflow_signal(
        &self,
        key: &SignalKey,
    ) -> Result<Option<SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }
}

/// S-284-SIM-04 — two rejected fresh-key Running publications fully unwind
/// the successor, publish no row, preserve predecessor history, and do not
/// invent a second allocation proposal inside the shim.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending corrective re-DELIVER roadmap: rejected fresh-key publication unwind"]
async fn rejected_successor_publication_fully_unwinds_without_immediate_second_proposal() {
    let (inner, predecessor, successor, workload) = seeded_predecessor(DriverType::Exec).await;
    let before = inner.alloc_status_row(&predecessor).await.unwrap().unwrap();
    let obs = RejectFreshPublication {
        inner,
        successor: successor.clone(),
        running_attempts: AtomicUsize::new(0),
    };
    let driver = Arc::new(RecordingDriver::new(DriverType::Exec, predecessor.clone()));
    let index = AllocDriverIndex::default();
    index.lock().insert(predecessor.clone(), DriverType::Exec);

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: successor_spec(DriverType::Exec, &workload, &successor),
            kind: WorkloadKind::Service,
        },
        Arc::clone(&driver),
        &obs,
        &index,
        &NetSlotAllocator::new(),
        &RecordingNetwork::default(),
        None,
    )
    .await
    .expect("a twice-rejected Running proposal unwinds and returns without another action");

    assert_eq!(obs.running_attempts.load(Ordering::SeqCst), 2);
    assert!(obs.alloc_status_row(&successor).await.unwrap().is_none());
    assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));
    assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&successor));
    assert_eq!(
        driver.stops.lock().as_slice(),
        &[successor.clone(), predecessor.clone()],
        "successor unwind and later predecessor cleanup remain exact-ID complements"
    );
    assert!(!index.lock().contains_key(&successor));
}

fn intent_for(driver_type: DriverType, workload: &str) -> WorkloadIntent {
    let driver = match driver_type {
        DriverType::Exec => DriverInput::Exec(ExecInput {
            command: "/bin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
        }),
        DriverType::Vm => DriverInput::Vm(VmInput {
            command: "/sbin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
            kernel: "/srv/vm/kernel".to_owned(),
            rootfs: "/srv/vm/rootfs.ext4".to_owned(),
        }),
        other => panic!("fixture covers Exec and VM, got {other:?}"),
    };
    WorkloadIntent::Job(
        Job::from_submit(JobSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver,
        })
        .expect("valid workload intent"),
    )
}

/// S-284-SIM-05 — the redb-backed runtime durably reserves before the first
/// successor effect. A successor-owned launch error leaves no row; after
/// runtime reopen, both Exec and VM skip the consumed ID and start higher.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending corrective re-DELIVER roadmap: durable reservation before effects and reopen"]
async fn durable_reservation_precedes_effect_and_reopen_skips_unrowed_identity_for_every_driver() {
    eprintln!("seed={SEED}: durable reservation and reopen");
    for driver_type in [DriverType::Exec, DriverType::Vm] {
        let tmp = tempfile::tempdir().expect("temporary data directory");
        let persisted = Arc::new(AtomicBool::new(false));
        let store_path = tmp.path().join("intent.redb");
        let intent_store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent"));
        let workload = format!("{}-reservation", driver_type.as_str());
        let first = aid(&format!("alloc-{workload}-0"));
        let second = aid(&format!("alloc-{workload}-1"));
        let driver = Arc::new(
            RecordingDriver::new(driver_type, first.clone())
                .with_behavior(StartBehavior::IoFailure)
                .with_view_persisted(Arc::clone(&persisted)),
        );
        let recording_store = RecordingRedbViewStore {
            inner: RedbViewStore::open(tmp.path()).expect("open redb ViewStore"),
            persisted: Arc::clone(&persisted),
        };
        let mut runtime =
            ReconcilerRuntime::new(tmp.path(), Arc::new(recording_store)).expect("runtime");
        runtime.register(workload_lifecycle()).await.expect("register WorkloadLifecycle");
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&intent_store) as Arc<dyn IntentStore>
        );
        let mut state = AppState::new(
            Arc::clone(&intent_store),
            store_path,
            Arc::new(SimObservationStore::single_peer(nid("local"), SEED)),
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::new(SimClock::new()),
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(SEED)))),
            Arc::new(IdentityMgr::new(None)),
            nid("local"),
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            std::net::Ipv4Addr::LOCALHOST,
        );
        let intent = intent_for(driver_type, &workload);
        let key = IntentKey::for_workload(&wid(&workload));
        let bytes = intent.archive_for_store().expect("archive intent");
        state.store.put(key.as_bytes(), bytes.as_ref()).await.expect("persist intent");
        state
            .store
            .put(
                IntentKey::for_workload_kind(&wid(&workload)).as_bytes(),
                &[WorkloadKind::Job.discriminator_byte()],
            )
            .await
            .expect("persist workload kind");
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        let name = ReconcilerName::new("workload-lifecycle").expect("reconciler name");
        let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };

        let first_result = run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &name,
            &target,
            Instant::now(),
            1,
            Instant::now() + Duration::from_secs(2),
            &network,
        )
        .await;
        assert!(first_result.is_err(), "injected successor start error remains typed");
        assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&first));
        assert!(
            state.runtime.view_for_workload_lifecycle(&target).restart_counts.contains_key(&first),
            "the returned View consumed the first identity before driver.start"
        );
        assert!(state.obs.alloc_status_row(&first).await.unwrap().is_none());

        let placeholder =
            ReconcilerRuntime::new(&tmp.path().join("placeholder"), Arc::new(SimViewStore::new()))
                .expect("placeholder runtime");
        let closed = std::mem::replace(&mut state.runtime, Arc::new(placeholder));
        drop(closed);
        let mut reopened =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("reopen");
        reopened.register(workload_lifecycle()).await.expect("bulk-load WorkloadLifecycle");
        assert!(
            reopened.view_for_workload_lifecycle(&target).restart_counts.contains_key(&first),
            "redb bulk-load restores the unrowed consumed identity"
        );
        state.runtime = Arc::new(reopened);

        // Replace only the driven driver behavior; the same type and trace
        // remain. AppState exposes the existing registry, so install a second
        // test driver of the same kind for the re-driven evaluation.
        let succeeding = Arc::new(
            RecordingDriver::new(driver_type, first.clone())
                .with_view_persisted(Arc::clone(&persisted)),
        );
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(Arc::clone(&succeeding) as Arc<dyn Driver>);
        state.drivers = Arc::new(registry);

        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &name,
            &target,
            Instant::now(),
            2,
            Instant::now() + Duration::from_secs(2),
            &RecordingNetwork { trace: Arc::clone(&succeeding.trace) },
        )
        .await
        .expect("reopened runtime re-drives above the consumed gap");
        assert_eq!(succeeding.starts.lock().as_slice(), std::slice::from_ref(&second));
        assert!(state.obs.alloc_status_row(&first).await.unwrap().is_none());
        assert_eq!(
            state.obs.alloc_status_row(&second).await.unwrap().map(|row| row.state),
            Some(AllocState::Running)
        );
    }
}
