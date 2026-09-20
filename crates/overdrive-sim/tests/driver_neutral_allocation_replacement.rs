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
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_workload_netns_plan, responder_addr_for_slot,
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
use overdrive_sim::adapters::{
    SimMtlsInterceptLifecycle, SimMtlsInterceptLifecycleState, SimVmHostState,
};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptInstallError, MtlsInterceptStopError};
use parking_lot::Mutex;
use tracing::{Event, Subscriber, subscriber::set_default};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

const SEED: u64 = 284_105_106;

#[derive(Clone, Debug, Eq, PartialEq)]
enum TraceEvent {
    Start(AllocationId),
    Stop(AllocationId),
    MtlsStart(AllocationId),
    MtlsStop(AllocationId),
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
    IoFailure,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanupFailure {
    None,
    Driver,
    Mtls,
    Network,
}

#[derive(Clone, Default)]
struct CapturedEvents {
    events: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn snapshot(&self) -> Vec<String> {
        self.events.lock().clone()
    }
}

struct EventVisitor<'a>(&'a mut String);

impl tracing::field::Visit for EventVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut rendered = event.metadata().name().to_owned();
        event.record(&mut EventVisitor(&mut rendered));
        self.events.lock().push(rendered);
    }
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

    fn with_blocked_predecessor_stop(self) -> Self {
        self.block_predecessor_stop.store(true, Ordering::SeqCst);
        self
    }

    fn with_failed_predecessor_stop(self) -> Self {
        self.fail_predecessor_stop.store(true, Ordering::SeqCst);
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
            StartBehavior::IoFailure => unreachable!("owner-path sequence has no I/O failure"),
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

#[derive(Default)]
struct RecordingNetwork {
    trace: Arc<Mutex<Vec<TraceEvent>>>,
    fail_teardown: Mutex<Option<String>>,
}

struct RecordingMtlsLifecycle {
    inner: SimMtlsInterceptLifecycle,
    trace: Arc<Mutex<Vec<TraceEvent>>>,
}

impl RecordingMtlsLifecycle {
    fn new(trace: Arc<Mutex<Vec<TraceEvent>>>) -> Self {
        Self { inner: SimMtlsInterceptLifecycle::new(), trace }
    }

    fn snapshot(&self) -> overdrive_sim::adapters::SimMtlsInterceptLifecycleSnapshot {
        self.inner.snapshot()
    }

    fn inject_stop_failure_once(&self, alloc_id: AllocationId, detail: impl Into<String>) {
        self.inner.inject_stop_failure_once(alloc_id, detail);
    }
}

#[async_trait]
impl MtlsInterceptLifecycle for RecordingMtlsLifecycle {
    async fn start_alloc(&self, spec: &AllocationSpec) -> Result<(), MtlsInterceptInstallError> {
        self.trace.lock().push(TraceEvent::MtlsStart(spec.alloc.clone()));
        self.inner.start_alloc(spec).await
    }

    async fn stop_alloc(&self, alloc_id: &AllocationId) -> Result<(), MtlsInterceptStopError> {
        self.trace.lock().push(TraceEvent::MtlsStop(alloc_id.clone()));
        self.inner.stop_alloc(alloc_id).await
    }
}

impl RecordingNetwork {
    const fn with_failed_teardown(trace: Arc<Mutex<Vec<TraceEvent>>>, netns: String) -> Self {
        Self { trace, fail_teardown: Mutex::new(Some(netns)) }
    }
}

impl WorkloadNetworkProvisioner for RecordingNetwork {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        _vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError> {
        self.trace.lock().push(TraceEvent::Provision(workload.netns.as_str().to_owned()));
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.trace.lock().push(TraceEvent::Teardown(workload.netns.as_str().to_owned()));
        if self.fail_teardown.lock().as_deref() == Some(workload.netns.as_str()) {
            return Err(VethProvisionError::SysctlSetFailed {
                key: "net.ipv4.ip_forward".to_owned(),
                value: "1".to_owned(),
                path: format!("/sim/{}", workload.netns.as_str()),
                source: io::Error::other("injected predecessor network cleanup failure"),
            });
        }
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
        network: None,
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
/// predecessor cleanup for the VM; the action remains in flight only
/// for the one post-successor exact-old cleanup attempt.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver() {
    eprintln!("seed={SEED}: successor-first driver-neutral replacement");
    for driver_type in [DriverType::Vm] {
        let (obs, predecessor, successor, workload) = seeded_predecessor(driver_type).await;
        let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
        let driver = Arc::new(
            RecordingDriver::new(driver_type, predecessor.clone()).with_blocked_predecessor_stop(),
        );
        let slots = NetSlotAllocator::new();
        slots
            .adopt(predecessor.clone(), NetSlot::new(7).expect("valid old slot"))
            .expect("old slot is owned");
        let network = RecordingNetwork { trace: Arc::clone(&driver.trace), ..Default::default() };
        let mtls = RecordingMtlsLifecycle::new(Arc::clone(&driver.trace));
        mtls.start_alloc(&successor_spec(driver_type, &workload, &predecessor))
            .await
            .expect("predecessor mTLS lifecycle is live");
        driver.trace.lock().clear();
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

/// S-284-SIM-02 — every driver/mTLS/network exact-old cleanup failure stage is
/// exercised after both successful and failed successor outcomes for Exec and
/// VM. Each stage short-circuits later old-cleanup stages, preserves an
/// accepted successor, and reports a both-fail cleanup error only as secondary
/// structured tracing.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn successor_and_cleanup_outcomes_follow_the_ratified_precedence_table() {
    eprintln!("seed={SEED}: successor/cleanup precedence table");
    let captured = CapturedEvents::default();
    let _capture = set_default(Registry::default().with(captured.clone()));

    for driver_type in [DriverType::Vm] {
        for start_behavior in [StartBehavior::IoFailure, StartBehavior::Success] {
            for cleanup_failure in [
                CleanupFailure::None,
                CleanupFailure::Driver,
                CleanupFailure::Mtls,
                CleanupFailure::Network,
            ] {
                let (obs, predecessor, successor, workload) = seeded_predecessor(driver_type).await;
                let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
                let mut configured_driver = RecordingDriver::new(driver_type, predecessor.clone())
                    .with_behavior(start_behavior);
                if cleanup_failure == CleanupFailure::Driver {
                    configured_driver = configured_driver.with_failed_predecessor_stop();
                }
                let driver = Arc::new(configured_driver);
                let slots = NetSlotAllocator::new();
                let old_slot = NetSlot::new(7).expect("valid predecessor slot");
                slots
                    .adopt(predecessor.clone(), old_slot)
                    .expect("predecessor owns its exact slot");
                let old_plan =
                    derive_workload_netns_plan(old_slot, responder_addr_for_slot(old_slot));
                let successor_slot = NetSlot::new(0).expect("valid successor slot");
                let successor_plan = derive_workload_netns_plan(
                    successor_slot,
                    responder_addr_for_slot(successor_slot),
                );
                let network = if cleanup_failure == CleanupFailure::Network {
                    RecordingNetwork::with_failed_teardown(
                        Arc::clone(&driver.trace),
                        old_plan.netns.as_str().to_owned(),
                    )
                } else {
                    RecordingNetwork { trace: Arc::clone(&driver.trace), ..Default::default() }
                };
                let mtls = RecordingMtlsLifecycle::new(Arc::clone(&driver.trace));
                mtls.start_alloc(&successor_spec(driver_type, &workload, &predecessor))
                    .await
                    .expect("predecessor mTLS is live");
                driver.trace.lock().clear();
                if cleanup_failure == CleanupFailure::Mtls {
                    mtls.inject_stop_failure_once(
                        predecessor.clone(),
                        "injected predecessor mTLS cleanup failure",
                    );
                }
                let index = AllocDriverIndex::default();
                index.lock().insert(predecessor.clone(), driver_type);
                let event_start = captured.snapshot().len();

                let result = dispatch_one(
                    Action::RestartAllocation {
                        alloc_id: predecessor.clone(),
                        spec: successor_spec(driver_type, &workload, &successor),
                        kind: WorkloadKind::Service,
                    },
                    Arc::clone(&driver),
                    obs.as_ref(),
                    &index,
                    &slots,
                    &network,
                    Some(&mtls),
                )
                .await;

                match (start_behavior, cleanup_failure, result) {
                    (StartBehavior::Success, CleanupFailure::None, Ok(()))
                    | (StartBehavior::Success, CleanupFailure::Mtls, Err(ShimError::MtlsStop(_)))
                    | (
                        StartBehavior::Success,
                        CleanupFailure::Network,
                        Err(ShimError::WorkloadNetnsProvision(_)),
                    ) => {}
                    (
                        StartBehavior::Success,
                        CleanupFailure::Driver,
                        Err(ShimError::Driver(DriverError::Io(error))),
                    ) => assert!(error.to_string().contains("predecessor cleanup")),
                    (
                        StartBehavior::IoFailure,
                        _,
                        Err(ShimError::Driver(DriverError::Io(error))),
                    ) => assert!(
                        error.to_string().contains("successor start"),
                        "successor failure remains primary when both fail: {error}"
                    ),
                    partition => {
                        panic!("MISSING_CORRECTED_BEHAVIOR: wrong precedence {partition:?}")
                    }
                }

                let trace = driver.trace.lock().clone();
                assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&successor));
                assert_eq!(driver.stops.lock().as_slice(), std::slice::from_ref(&predecessor));
                let start = trace
                    .iter()
                    .position(|event| event == &TraceEvent::Start(successor.clone()))
                    .expect("successor attempt");
                let stop = trace
                    .iter()
                    .position(|event| event == &TraceEvent::Stop(predecessor.clone()))
                    .expect("one exact predecessor driver cleanup attempt");
                let successor_network_provision = trace
                    .iter()
                    .position(|event| {
                        event == &TraceEvent::Provision(successor_plan.netns.as_str().to_owned())
                    })
                    .expect("successor structural network provision");
                if start_behavior == StartBehavior::IoFailure {
                    assert!(
                        !trace.contains(&TraceEvent::Stop(successor.clone())),
                        "a failed launch yields no successor handle for driver stop: {trace:?}"
                    );
                    assert!(
                        !trace.contains(&TraceEvent::MtlsStart(successor.clone())),
                        "mTLS never becomes live for a successor whose driver launch failed"
                    );
                    let successor_mtls_unwind = trace
                        .iter()
                        .position(|event| event == &TraceEvent::MtlsStop(successor.clone()))
                        .expect("failed launch attempts exact successor mTLS unwind");
                    let successor_network_unwind = trace
                        .iter()
                        .position(|event| {
                            event == &TraceEvent::Teardown(successor_plan.netns.as_str().to_owned())
                        })
                        .expect("failed launch completes exact successor network unwind");
                    assert!(
                        successor_network_provision < start
                            && start < successor_mtls_unwind
                            && successor_mtls_unwind < successor_network_unwind
                            && successor_network_unwind < stop,
                        "failed successor launch unwind must complete before predecessor driver cleanup: {trace:?}"
                    );
                } else {
                    let successor_mtls_start = trace
                        .iter()
                        .position(|event| event == &TraceEvent::MtlsStart(successor.clone()))
                        .expect("accepted successor mTLS start");
                    assert!(
                        successor_network_provision < start
                            && start < successor_mtls_start
                            && successor_mtls_start < stop,
                        "accepted successor completes before predecessor driver cleanup: {trace:?}"
                    );
                }

                let old_teardown_attempted = trace.iter().any(|event| {
                    event == &TraceEvent::Teardown(old_plan.netns.as_str().to_owned())
                });
                let old_teardown_count = trace
                    .iter()
                    .filter(|event| {
                        *event == &TraceEvent::Teardown(old_plan.netns.as_str().to_owned())
                    })
                    .count();
                let lifecycle = mtls.snapshot();
                match cleanup_failure {
                    CleanupFailure::None => {
                        assert!(!lifecycle.allocations.contains_key(&predecessor));
                        assert!(old_teardown_attempted);
                        assert_eq!(old_teardown_count, 1);
                        assert!(!slots.snapshot().contains_key(&predecessor));
                        assert!(!index.lock().contains_key(&predecessor));
                    }
                    CleanupFailure::Driver => {
                        assert_eq!(
                            lifecycle.allocations.get(&predecessor),
                            Some(&SimMtlsInterceptLifecycleState::Live)
                        );
                        assert!(!old_teardown_attempted);
                        assert_eq!(old_teardown_count, 0);
                        assert!(slots.snapshot().contains_key(&predecessor));
                        assert_eq!(index.lock().get(&predecessor), Some(&driver_type));
                    }
                    CleanupFailure::Mtls => {
                        assert_eq!(
                            lifecycle.allocations.get(&predecessor),
                            Some(&SimMtlsInterceptLifecycleState::TeardownPending)
                        );
                        assert!(!old_teardown_attempted);
                        assert_eq!(old_teardown_count, 0);
                        assert!(slots.snapshot().contains_key(&predecessor));
                        assert_eq!(index.lock().get(&predecessor), Some(&driver_type));
                    }
                    CleanupFailure::Network => {
                        assert!(!lifecycle.allocations.contains_key(&predecessor));
                        assert!(old_teardown_attempted);
                        assert_eq!(old_teardown_count, 1);
                        assert!(slots.snapshot().contains_key(&predecessor));
                        assert_eq!(index.lock().get(&predecessor), Some(&driver_type));
                    }
                }

                if start_behavior == StartBehavior::Success {
                    let accepted = obs
                        .alloc_status_row(&successor)
                        .await
                        .unwrap()
                        .expect("accepted successor survives old cleanup outcome");
                    assert_eq!(accepted.state, AllocState::Running);
                    assert_eq!(accepted.restart_count, 0);
                    assert!(accepted.last_terminated.is_none());
                    assert_eq!(index.lock().get(&successor), Some(&driver_type));
                    assert_eq!(
                        lifecycle.allocations.get(&successor),
                        Some(&SimMtlsInterceptLifecycleState::Live)
                    );
                } else {
                    assert!(obs.alloc_status_row(&successor).await.unwrap().is_none());
                    assert!(!index.lock().contains_key(&successor));
                    assert!(!lifecycle.allocations.contains_key(&successor));
                    assert!(!slots.snapshot().contains_key(&successor));
                }
                assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));

                if start_behavior == StartBehavior::IoFailure
                    && cleanup_failure != CleanupFailure::None
                {
                    let cleanup_detail = match cleanup_failure {
                        CleanupFailure::Driver => "injected predecessor cleanup failure",
                        CleanupFailure::Mtls => "injected predecessor mTLS cleanup failure",
                        CleanupFailure::Network => "injected predecessor network cleanup failure",
                        CleanupFailure::None => unreachable!(),
                    };
                    assert!(
                        captured.snapshot()[event_start..].iter().any(|event| {
                            event.contains("injected successor start failure")
                                && event.contains(cleanup_detail)
                        }),
                        "both-fail partition reports cleanup only through secondary structured tracing"
                    );
                }
            }
        }
    }
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

/// S-284-SIM-04 — one rejected fresh-key Running publication fully unwinds
/// the successor driver/mTLS/network ownership before predecessor driver
/// cleanup begins, publish no row, preserve predecessor history, and do not
/// invent a second allocation proposal inside the shim.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn rejected_successor_publication_fully_unwinds_without_immediate_second_proposal() {
    let (inner, predecessor, successor, workload) = seeded_predecessor(DriverType::Vm).await;
    let before = inner.alloc_status_row(&predecessor).await.unwrap().unwrap();
    let obs = RejectFreshPublication {
        inner,
        successor: successor.clone(),
        running_attempts: AtomicUsize::new(0),
    };
    let driver = Arc::new(RecordingDriver::new(DriverType::Vm, predecessor.clone()));
    let index = AllocDriverIndex::default();
    index.lock().insert(predecessor.clone(), DriverType::Vm);
    let slots = NetSlotAllocator::new();
    let predecessor_slot = NetSlot::new(7).expect("valid predecessor slot");
    slots.adopt(predecessor.clone(), predecessor_slot).expect("predecessor slot ownership");
    let successor_slot = NetSlot::new(0).expect("valid successor slot");
    let successor_plan =
        derive_workload_netns_plan(successor_slot, responder_addr_for_slot(successor_slot));
    let predecessor_plan =
        derive_workload_netns_plan(predecessor_slot, responder_addr_for_slot(predecessor_slot));
    let network = RecordingNetwork { trace: Arc::clone(&driver.trace), ..Default::default() };
    let mtls = RecordingMtlsLifecycle::new(Arc::clone(&driver.trace));
    mtls.start_alloc(&successor_spec(DriverType::Vm, &workload, &predecessor))
        .await
        .expect("predecessor mTLS is live");
    driver.trace.lock().clear();

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: predecessor.clone(),
            spec: successor_spec(DriverType::Vm, &workload, &successor),
            kind: WorkloadKind::Service,
        },
        Arc::clone(&driver),
        &obs,
        &index,
        &slots,
        &network,
        Some(&mtls),
    )
    .await
    .expect("a rejected Running proposal unwinds without a second proposal or action");

    let trace = driver.trace.lock().clone();
    let successor_network_provision = trace
        .iter()
        .position(|event| event == &TraceEvent::Provision(successor_plan.netns.as_str().to_owned()))
        .expect("successor owns its structural network before launch");
    let successor_start = trace
        .iter()
        .position(|event| event == &TraceEvent::Start(successor.clone()))
        .expect("one exact successor driver start");
    let successor_driver_unwind = trace
        .iter()
        .position(|event| event == &TraceEvent::Stop(successor.clone()))
        .expect("rejected publication stops exact successor driver ownership");
    let successor_mtls_unwind = trace
        .iter()
        .position(|event| event == &TraceEvent::MtlsStop(successor.clone()))
        .expect("rejected publication stops exact successor mTLS ownership");
    let successor_network_unwind = trace
        .iter()
        .position(|event| event == &TraceEvent::Teardown(successor_plan.netns.as_str().to_owned()))
        .expect("rejected publication tears down exact successor network ownership");
    let predecessor_driver_cleanup = trace
        .iter()
        .position(|event| event == &TraceEvent::Stop(predecessor.clone()))
        .expect("one exact predecessor driver cleanup");
    let predecessor_mtls_cleanup = trace
        .iter()
        .position(|event| event == &TraceEvent::MtlsStop(predecessor.clone()))
        .expect("one exact predecessor mTLS cleanup");
    let predecessor_network_cleanup = trace
        .iter()
        .position(|event| {
            event == &TraceEvent::Teardown(predecessor_plan.netns.as_str().to_owned())
        })
        .expect("one exact predecessor network cleanup");
    assert!(
        successor_network_provision < successor_start
            && successor_start < successor_driver_unwind
            && successor_driver_unwind < successor_mtls_unwind
            && successor_mtls_unwind < successor_network_unwind
            && successor_network_unwind < predecessor_driver_cleanup
            && predecessor_driver_cleanup < predecessor_mtls_cleanup
            && predecessor_mtls_cleanup < predecessor_network_cleanup,
        "rejected-publication successor unwind must finish before exact predecessor cleanup: {trace:?}"
    );
    assert!(
        !trace.contains(&TraceEvent::MtlsStart(successor.clone())),
        "rejected Running publication cannot make successor mTLS live"
    );
    assert_eq!(obs.running_attempts.load(Ordering::SeqCst), 1);
    assert!(obs.alloc_status_row(&successor).await.unwrap().is_none());
    assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));
    assert_eq!(driver.starts.lock().as_slice(), std::slice::from_ref(&successor));
    assert_eq!(
        driver.stops.lock().as_slice(),
        &[successor.clone(), predecessor.clone()],
        "successor unwind and later predecessor cleanup remain exact-ID complements"
    );
    let lifecycle = mtls.snapshot();
    assert!(!lifecycle.allocations.contains_key(&successor));
    assert!(!lifecycle.allocations.contains_key(&predecessor));
    assert!(!slots.snapshot().contains_key(&successor));
    assert!(!slots.snapshot().contains_key(&predecessor));
    assert!(!index.lock().contains_key(&successor));
    assert!(!index.lock().contains_key(&predecessor));
}
