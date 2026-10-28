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

use super::{
    AllocDriverIndex, MtlsInterceptLifecycle, ShimError, WorkloadNetworkProvisioner,
    dispatch_with_network_provisioner_and_guest,
};
use crate::guest_network::{GuestAddressPool, SharedGuestNetworkOwner};
use crate::identity_mgr::IdentityMgr;
use crate::shared_network_test_ports::TestSharedOwner;
use crate::veth_provisioner::{NetSlotAllocator, VethProvisionError, WorkloadNetnsPlan};
use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverType, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::mtls_enforcement::{EnforcedConnectionId, MtlsEnforcementError};
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, LagAwareSubscription,
    LogicalTimestamp, NodeHealthRow, ObservationStore, ObservationStoreError, ObservationWrite,
    ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow, TransitionSource,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_sim::adapters::SimVmHostState;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_worker::{
    HandleTeardownFailure, MtlsInterceptInstallError, MtlsInterceptStopError,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartBehavior {
    Success,
    IoFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanupFailure {
    None,
    Driver,
    Mtls,
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

/// Records every workload-netns provision and teardown. A VM allocation has
/// no workload netns and never calls the provisioner (D-UP-15), so every
/// recorded event for these VM-only cases is a defect the assertions name.
#[derive(Default)]
struct RecordingNetwork {
    trace: Arc<Mutex<Vec<TraceEvent>>>,
}

/// D-UP-15: no VM allocation provisions or tears down a workload netns.
fn assert_no_workload_netns(trace: &[TraceEvent]) {
    assert!(
        !trace
            .iter()
            .any(|event| matches!(event, TraceEvent::Provision(_) | TraceEvent::Teardown(_))),
        "a VM allocation provisions and tears down no workload network namespace (D-UP-15): \
         {trace:?}"
    );
}

struct RecordingMtlsLifecycle {
    inner: Mutex<RecordingMtlsState>,
    trace: Arc<Mutex<Vec<TraceEvent>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordingMtlsStateKind {
    Live,
    TeardownPending,
}

#[derive(Default)]
struct RecordingMtlsState {
    allocations: BTreeMap<AllocationId, RecordingMtlsStateKind>,
    stop_faults: BTreeMap<AllocationId, VecDeque<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordingMtlsSnapshot {
    allocations: BTreeMap<AllocationId, RecordingMtlsStateKind>,
}

impl RecordingMtlsLifecycle {
    fn new(trace: Arc<Mutex<Vec<TraceEvent>>>) -> Self {
        Self { inner: Mutex::new(RecordingMtlsState::default()), trace }
    }

    fn snapshot(&self) -> RecordingMtlsSnapshot {
        RecordingMtlsSnapshot { allocations: self.inner.lock().allocations.clone() }
    }

    fn inject_stop_failure_once(&self, alloc_id: AllocationId, detail: impl Into<String>) {
        self.inner.lock().stop_faults.entry(alloc_id).or_default().push_back(detail.into());
    }

    fn scripted_stop_error(alloc_id: &AllocationId, detail: String) -> MtlsInterceptStopError {
        let connection = EnforcedConnectionId::new(alloc_id.clone(), 0);
        let source = Arc::new(MtlsEnforcementError::TeardownFailed {
            id: connection.clone(),
            source: std::io::Error::other(detail),
        });
        MtlsInterceptStopError::HandleTeardown {
            alloc_id: alloc_id.clone(),
            failures: vec![HandleTeardownFailure { connection, source }],
        }
    }
}

#[async_trait]
impl MtlsInterceptLifecycle for RecordingMtlsLifecycle {
    async fn start_alloc(&self, spec: &AllocationSpec) -> Result<(), MtlsInterceptInstallError> {
        self.trace.lock().push(TraceEvent::MtlsStart(spec.alloc.clone()));
        self.inner.lock().allocations.insert(spec.alloc.clone(), RecordingMtlsStateKind::Live);
        Ok(())
    }

    async fn stop_alloc(&self, alloc_id: &AllocationId) -> Result<(), MtlsInterceptStopError> {
        self.trace.lock().push(TraceEvent::MtlsStop(alloc_id.clone()));
        let mut state = self.inner.lock();
        let Some(_) = state.allocations.get(alloc_id) else {
            return Ok(());
        };
        state.allocations.insert(alloc_id.clone(), RecordingMtlsStateKind::TeardownPending);
        if let Some(detail) = state.stop_faults.get_mut(alloc_id).and_then(VecDeque::pop_front) {
            return Err(Self::scripted_stop_error(alloc_id, detail));
        }
        state.allocations.remove(alloc_id);
        drop(state);
        Ok(())
    }
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
    let allocator = crate::test_default_allocator(intent);
    let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
    let driver_dyn: Arc<dyn Driver> = driver;
    registry.insert(driver_dyn);
    let (bus, _rx) = tokio::sync::broadcast::channel(8);

    let guest_network = TestSharedOwner::new();
    guest_network
        .probe_startup()
        .await
        .expect("the fixture guest-network owner passes its startup probe");
    guest_network
        .converge_shared()
        .await
        .expect("the fixture guest-network owner reports bound flow ports before EXEC");
    let flow_ports = guest_network
        .flow_ports()
        .expect("the converged fixture owner reports both bound flow ports");
    let guest_pool = GuestAddressPool::new(
        GUEST_NODE_PREFIX,
        GUEST_NODE_GATEWAY,
        Arc::new(SimEntropy::new(SEED)),
    );
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    assert!(
        wiring.supervisor().open_after_boot(),
        "the EXEC gate opens only after the exact fixture owner converges"
    );
    let gate = wiring.gate();

    dispatch_with_network_provisioner_and_guest(
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
        Some((&guest_network, &guest_pool, Some(flow_ports))),
        Some(gate.as_ref()),
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
        let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };
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
            &mtls,
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
            Some(&RecordingMtlsStateKind::Live),
            "successor mTLS is live before old cleanup begins"
        );
        assert_eq!(
            mtls_while_old_cleanup_blocked.allocations.get(&predecessor),
            Some(&RecordingMtlsStateKind::Live),
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
        assert_eq!(mtls_after.allocations.get(&successor), Some(&RecordingMtlsStateKind::Live));
        let trace = driver.trace.lock().clone();
        assert_no_workload_netns(&trace);
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

/// S-284-SIM-02 — every driver/mTLS exact-old cleanup failure stage is
/// exercised after both successful and failed successor outcomes for the VM
/// (a VM has no workload netns to fail tearing down, D-UP-15). Each stage short-circuits later old-cleanup stages, preserves an
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
            for cleanup_failure in
                [CleanupFailure::None, CleanupFailure::Driver, CleanupFailure::Mtls]
            {
                let (obs, predecessor, successor, workload) = seeded_predecessor(driver_type).await;
                let before = obs.alloc_status_row(&predecessor).await.unwrap().unwrap();
                let mut configured_driver = RecordingDriver::new(driver_type, predecessor.clone())
                    .with_behavior(start_behavior);
                if cleanup_failure == CleanupFailure::Driver {
                    configured_driver = configured_driver.with_failed_predecessor_stop();
                }
                let driver = Arc::new(configured_driver);
                let slots = NetSlotAllocator::new();
                let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };
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
                    &mtls,
                )
                .await;

                match (start_behavior, cleanup_failure, result) {
                    (StartBehavior::Success, CleanupFailure::None, Ok(()))
                    | (StartBehavior::Success, CleanupFailure::Mtls, Err(ShimError::MtlsStop(_))) =>
                        {}
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
                assert_no_workload_netns(&trace);
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
                    assert!(
                        start < successor_mtls_unwind && successor_mtls_unwind < stop,
                        "failed successor launch unwind must complete before predecessor driver cleanup: {trace:?}"
                    );
                } else {
                    let successor_mtls_start = trace
                        .iter()
                        .position(|event| event == &TraceEvent::MtlsStart(successor.clone()))
                        .expect("accepted successor mTLS start");
                    assert!(
                        start < successor_mtls_start && successor_mtls_start < stop,
                        "accepted successor completes before predecessor driver cleanup: {trace:?}"
                    );
                }

                let lifecycle = mtls.snapshot();
                match cleanup_failure {
                    CleanupFailure::None => {
                        assert!(!lifecycle.allocations.contains_key(&predecessor));
                        assert!(!index.lock().contains_key(&predecessor));
                    }
                    CleanupFailure::Driver => {
                        assert_eq!(
                            lifecycle.allocations.get(&predecessor),
                            Some(&RecordingMtlsStateKind::Live)
                        );
                        assert_eq!(index.lock().get(&predecessor), Some(&driver_type));
                    }
                    CleanupFailure::Mtls => {
                        assert_eq!(
                            lifecycle.allocations.get(&predecessor),
                            Some(&RecordingMtlsStateKind::TeardownPending)
                        );
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
                        Some(&RecordingMtlsStateKind::Live)
                    );
                } else {
                    assert!(obs.alloc_status_row(&successor).await.unwrap().is_none());
                    assert!(!index.lock().contains_key(&successor));
                    assert!(!lifecycle.allocations.contains_key(&successor));
                }
                assert_eq!(obs.alloc_status_row(&predecessor).await.unwrap(), Some(before));

                if start_behavior == StartBehavior::IoFailure
                    && cleanup_failure != CleanupFailure::None
                {
                    let cleanup_detail = match cleanup_failure {
                        CleanupFailure::Driver => "injected predecessor cleanup failure",
                        CleanupFailure::Mtls => "injected predecessor mTLS cleanup failure",
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
/// the successor driver/mTLS ownership (a VM owns no workload netns, D-UP-15)
/// before predecessor driver
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
    let network = RecordingNetwork { trace: Arc::clone(&driver.trace) };
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
        &mtls,
    )
    .await
    .expect("a rejected Running proposal unwinds without a second proposal or action");

    let trace = driver.trace.lock().clone();
    assert_no_workload_netns(&trace);
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
    let predecessor_driver_cleanup = trace
        .iter()
        .position(|event| event == &TraceEvent::Stop(predecessor.clone()))
        .expect("one exact predecessor driver cleanup");
    let predecessor_mtls_cleanup = trace
        .iter()
        .position(|event| event == &TraceEvent::MtlsStop(predecessor.clone()))
        .expect("one exact predecessor mTLS cleanup");
    assert!(
        successor_start < successor_driver_unwind
            && successor_driver_unwind < successor_mtls_unwind
            && successor_mtls_unwind < predecessor_driver_cleanup
            && predecessor_driver_cleanup < predecessor_mtls_cleanup,
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
    assert!(!index.lock().contains_key(&successor));
    assert!(!index.lock().contains_key(&predecessor));
}
