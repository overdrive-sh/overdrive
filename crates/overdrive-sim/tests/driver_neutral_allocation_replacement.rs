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

use std::collections::{BTreeMap, VecDeque};
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
    NetSlotAllocator, VethProvisionError, WorkloadNetnsPlan,
};
use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_control_plane::view_store::{
    ProbeError as ViewStoreProbeError, Result as ViewStoreResult, ViewStore,
};
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverStartClass, DriverStartFailure, DriverType, Resources, VmPayload,
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
use overdrive_sim::adapters::{SimMtlsInterceptLifecycle, SimVmHostState};
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

fn service_intent(workload: &str) -> WorkloadIntent {
    let driver = DriverInput::Vm(VmInput {
        command: "/sbin/workload".to_owned(),
        args: vec!["--serve".to_owned()],
        kernel: "/srv/vm/kernel".to_owned(),
        rootfs: "/srv/vm/rootfs.ext4".to_owned(),
    });
    WorkloadIntent::Service(
        Service::from_submit(ServiceSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver,
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .expect("valid Service intent"),
    )
}

/// S-284-SIM-05 — the full production owner path for the composed VM driver:
/// registered WorkloadLifecycle authors the initial StartRejected predecessor,
/// the runtime fsyncs a fresh successor reservation before dispatch, rejected
/// successor publication fully unwinds, and runtime reopen re-drives above the
/// consumed ID. No row or replacement Action is seeded by the test.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn production_owner_replacement_survives_rejected_publication_and_reopen_for_vm() {
    eprintln!(
        "seed={SEED}; reproduce: cargo xtask lima run -- cargo nextest run -p overdrive-sim \
         --features integration-tests,overdrive-control-plane/integration-tests \
         --test driver_neutral_allocation_replacement --run-ignored ignored-only \
         -E 'test(production_owner_replacement_survives_rejected_publication_and_reopen_for_vm)' \
         --no-capture"
    );

    Box::pin(production_owner_replacement_case(DriverType::Vm)).await;
}

async fn production_owner_replacement_case(driver_type: DriverType) {
    let temp = tempfile::tempdir().expect("temporary owner-path data directory");
    let workload = format!("{}-owner-path", driver_type.as_str());
    let predecessor = aid(&format!("alloc-{workload}-0"));
    let rejected = aid(&format!("alloc-{workload}-1"));
    let accepted = aid(&format!("alloc-{workload}-2"));
    let persisted = Arc::new(AtomicBool::new(false));
    let driver = Arc::new(OwnerPathDriver::new(driver_type, Arc::clone(&persisted)));
    let inner = Arc::new(SimObservationStore::single_peer(nid("local"), SEED));
    let observations = Arc::new(RejectFreshPublication {
        inner: Arc::clone(&inner),
        successor: rejected.clone(),
        running_attempts: AtomicUsize::new(0),
    });
    let intent_path = temp.path().join("intent.redb");
    let intent = Arc::new(LocalIntentStore::open(&intent_path).expect("open intent store"));
    let recording_view = RecordingRedbViewStore {
        inner: RedbViewStore::open(temp.path()).expect("open durable ViewStore"),
        persisted: Arc::clone(&persisted),
    };
    let mut runtime =
        ReconcilerRuntime::new(temp.path(), Arc::new(recording_view)).expect("runtime");
    runtime.register(workload_lifecycle()).await.expect("register WorkloadLifecycle");
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&intent) as Arc<dyn IntentStore>
    );
    let clock = Arc::new(SimClock::new());
    let mut state = AppState::new(
        Arc::clone(&intent),
        intent_path,
        Arc::clone(&observations) as Arc<dyn ObservationStore>,
        Arc::new(runtime),
        Arc::clone(&driver) as Arc<dyn Driver>,
        Arc::clone(&clock) as Arc<dyn overdrive_core::traits::clock::Clock>,
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(SEED)))),
        Arc::new(IdentityMgr::new(None)),
        nid("local"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        Arc::new(overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::new(
            Arc::new(overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement::new(
                Arc::new(overdrive_sim::adapters::SimIdentityRead::new(
                    std::collections::BTreeMap::new(),
                    None,
                )),
                overdrive_core::traits::mtls_enforcement::MtlsLimits::default(),
                Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            )),
            Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
                std::collections::BTreeMap::new(),
                overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
            )),
            Arc::new(overdrive_sim::adapters::clock::SimClock::new()),
            Arc::new(overdrive_sim::adapters::SimMtlsIntercept::new()),
        )),
        Arc::new(overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner::default()),
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
            overdrive_sim::adapters::clock::SimClock::new(),
        ))
        .gate(),
        Arc::new(overdrive_control_plane::guest_network::GuestAddressPool::new(
            "100.95.0.0/16".parse().expect("static guest prefix"),
            std::net::Ipv4Addr::new(100, 95, 0, 1),
            Arc::new(overdrive_sim::adapters::entropy::SimEntropy::new(0x295)),
        )),
    );
    let desired = service_intent(&workload);
    let workload_id = wid(&workload);
    state
        .store
        .put(
            IntentKey::for_workload(&workload_id).as_bytes(),
            desired.archive_for_store().expect("archive Service intent").as_ref(),
        )
        .await
        .expect("persist Service intent");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("persist Service kind");
    let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
    let owner = ReconcilerName::new("workload-lifecycle").expect("owner name");
    let network = RecordingNetwork::default();

    persisted.store(false, Ordering::SeqCst);
    run_convergence_tick_with_network_provisioner_for_test(
        &state,
        &owner,
        &target,
        clock.now(),
        1,
        clock.now() + Duration::from_secs(2),
        &network,
    )
    .await
    .expect("production owner publishes the initial rejected predecessor");
    let predecessor_row = inner
        .alloc_status_row(&predecessor)
        .await
        .expect("read predecessor")
        .expect("initial StartRejected becomes an accepted Failed predecessor");
    assert_eq!(predecessor_row.state, AllocState::Failed);
    let predecessor_history =
        inner.alloc_lifecycle_occurrences(&predecessor).await.expect("read predecessor history");

    persisted.store(false, Ordering::SeqCst);
    clock.tick(Duration::from_secs(2));
    run_convergence_tick_with_network_provisioner_for_test(
        &state,
        &owner,
        &target,
        clock.now(),
        2,
        clock.now() + Duration::from_secs(2),
        &network,
    )
    .await
    .expect("rejected successor publication fully unwinds without another proposal");

    let placeholder =
        ReconcilerRuntime::new(&temp.path().join("placeholder"), Arc::new(SimViewStore::new()))
            .expect("placeholder runtime while closing redb");
    let closed = std::mem::replace(&mut state.runtime, Arc::new(placeholder));
    drop(closed);
    let reopened_view = RecordingRedbViewStore {
        inner: RedbViewStore::open(temp.path()).expect("reopen durable ViewStore"),
        persisted: Arc::clone(&persisted),
    };
    let mut reopened =
        ReconcilerRuntime::new(temp.path(), Arc::new(reopened_view)).expect("reopen runtime");
    reopened.register(workload_lifecycle()).await.expect("bulk-load WorkloadLifecycle");
    let restored = reopened.view_for_workload_lifecycle(&target);
    let restored_predecessor = restored.restart_counts.contains_key(&predecessor);
    let restored_rejected = restored.restart_counts.contains_key(&rejected);
    state.runtime = Arc::new(reopened);

    persisted.store(false, Ordering::SeqCst);
    clock.tick(Duration::from_secs(2));
    run_convergence_tick_with_network_provisioner_for_test(
        &state,
        &owner,
        &target,
        clock.now(),
        3,
        clock.now() + Duration::from_secs(2),
        &network,
    )
    .await
    .expect("reopened owner re-drives above the consumed rejected successor");

    assert_eq!(
        driver.starts.lock().as_slice(),
        &[predecessor.clone(), rejected.clone(), accepted.clone()],
        "MISSING_CORRECTED_BEHAVIOR: production owners must create three distinct physical attempts"
    );
    assert_eq!(driver.persistence_at_start.lock().as_slice(), &[true, true, true]);
    assert!(restored_predecessor, "reopen restores the initial issued-ID reservation");
    assert!(restored_rejected, "reopen restores the rejected successor reservation");
    assert_eq!(observations.running_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(inner.alloc_status_row(&predecessor).await.unwrap(), Some(predecessor_row));
    assert_eq!(inner.alloc_lifecycle_occurrences(&predecessor).await.unwrap(), predecessor_history);
    assert!(inner.alloc_status_row(&rejected).await.unwrap().is_none());
    let accepted_row =
        inner.alloc_status_row(&accepted).await.unwrap().expect("higher successor Running row");
    assert_eq!(accepted_row.state, AllocState::Running);
    assert_eq!(accepted_row.restart_count, 0);
    assert!(accepted_row.last_terminated.is_none());
    let final_view = state.runtime.view_for_workload_lifecycle(&target);
    assert!(final_view.restart_counts.contains_key(&predecessor));
    assert!(final_view.restart_counts.contains_key(&rejected));
    assert!(final_view.restart_counts.contains_key(&accepted));
    assert_eq!(
        driver.stops.lock().as_slice(),
        &[rejected, predecessor.clone(), predecessor],
        "successor unwind and both post-successor predecessor cleanups use exact IDs"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartBehavior {
    Success,
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
        }
    }

    const fn with_behavior(mut self, behavior: StartBehavior) -> Self {
        self.behavior = behavior;
        self
    }
}

#[async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.starts.lock().push(spec.alloc.clone());
        self.trace.lock().push(TraceEvent::Start(spec.alloc.clone()));
        // Retain a permit when the observer has not been polled yet. The
        // replacement-order assertion intentionally races this notification
        // against the dispatch future; `notify_waiters` can lose that event
        // before registration and turn a valid run into a scheduler-dependent
        // predecessor-cleanup failure.
        self.start_entered.notify_one();
        match self.behavior {
            StartBehavior::Success => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(42) })
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
            // As above, preserve the event until the assertion's waiter is
            // registered so the ordering check remains deterministic under
            // full-workspace load.
            self.stop_entered.notify_one();
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

struct OwnerPathDriver {
    driver_type: DriverType,
    behaviors: Mutex<VecDeque<StartBehavior>>,
    starts: Mutex<Vec<AllocationId>>,
    stops: Mutex<Vec<AllocationId>>,
    view_persisted: Arc<AtomicBool>,
    persistence_at_start: Mutex<Vec<bool>>,
}

impl OwnerPathDriver {
    fn new(driver_type: DriverType, view_persisted: Arc<AtomicBool>) -> Self {
        Self {
            driver_type,
            behaviors: Mutex::new(VecDeque::from([
                StartBehavior::Rejected,
                StartBehavior::Success,
                StartBehavior::Success,
            ])),
            starts: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            view_persisted,
            persistence_at_start: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Driver for OwnerPathDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.starts.lock().push(spec.alloc.clone());
        self.persistence_at_start.lock().push(self.view_persisted.load(Ordering::SeqCst));
        let behavior = self.behaviors.lock().pop_front();
        match behavior.expect("three owner-path starts are bounded") {
            StartBehavior::Success => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(84) })
            }
            StartBehavior::Rejected => Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Unclassified { driver: self.driver_type },
                    detail: "seeded initial predecessor rejection".to_owned(),
                },
            }),
        }
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stops.lock().push(handle.alloc.clone());
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

/// Records every workload-netns provision and teardown. A VM allocation has
/// no workload netns and never calls the provisioner (D-UP-15), so every
/// recorded event for these VM-only cases is a defect the assertions name.
#[derive(Default)]
struct RecordingNetwork {
    trace: Arc<Mutex<Vec<TraceEvent>>>,
}

impl WorkloadNetworkProvisioner for RecordingNetwork {
    fn provision(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
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

fn payload(_driver_type: DriverType) -> DriverPayload {
    DriverPayload::Vm(VmPayload {
        command: "/sbin/workload".to_owned(),
        args: vec!["--serve".to_owned()],
        kernel: Path::new("/srv/vm/kernel").to_path_buf(),
        rootfs: Path::new("/srv/vm/rootfs.ext4").to_path_buf(),
    })
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
        transport: None,
        service_ports: Vec::new(),
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
    mtls_lifecycle: &dyn MtlsInterceptLifecycle,
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

/// S-284-SIM-03 — an accepted StartRejected successor is published at its
/// fresh key with zero/None per-key history; the predecessor remains immutable
/// and can no longer be numeric-current.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn accepted_failed_successor_publishes_at_fresh_key_with_zero_history() {
    let (obs, predecessor, successor, workload) = seeded_predecessor(DriverType::Vm).await;
    let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
    let driver = Arc::new(
        RecordingDriver::new(DriverType::Vm, predecessor.clone())
            .with_behavior(StartBehavior::Rejected),
    );
    let index = AllocDriverIndex::default();
    index.lock().insert(predecessor.clone(), DriverType::Vm);
    let mtls = SimMtlsInterceptLifecycle::new();

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: successor_spec(DriverType::Vm, &workload, &successor),
            kind: WorkloadKind::Service,
        },
        Arc::clone(&driver),
        obs.as_ref(),
        &index,
        &NetSlotAllocator::new(),
        &RecordingNetwork::default(),
        &mtls,
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
