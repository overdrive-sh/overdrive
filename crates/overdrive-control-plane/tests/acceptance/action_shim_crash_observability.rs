//! T-C + T-G + T-H (ADR-0078 § D6) — the action shim writes the
//! crash-observability facts, at the real call sites, against a real
//! observation store.
//!
//! WHY-NEW-FILE: crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs
//!   CLOSEST-EXISTING: crates/overdrive-control-plane/tests/acceptance/finalize_failed_forward_carries_workload_addr.rs
//!   EXTENSION-COST: that file's module doc, filename and mutation
//!     rationale all scope it to ONE branch — the `workload_addr`
//!     forward-carry in the `FinalizeFailed` arm — and it drives only
//!     `Action::FinalizeFailed`. Three of the four scenarios here drive
//!     `Action::RestartAllocation` against a terminal prior, which that
//!     file's whole `finalize_and_read_successor` harness (an
//!     `InertDriver` whose `start` deliberately errors) structurally
//!     cannot express.
//!   PARALLEL-RATIONALE: different action under test
//!     (`RestartAllocation`, which requires a driver that actually
//!     STARTS), a different driven-port shape (a driver double whose
//!     accept/reject behaviour is the variable under test), and a
//!     different assertion surface (`last_terminated` / `restart_count`
//!     rather than `workload_addr`).
//!
//! # PORT-TO-PORT litmus
//!
//! Drives the production driving port `action_shim::dispatch` and
//! asserts at the driven-port boundary (the `AllocStatusRow` written to
//! the `SimObservationStore`) — never on internal state. This is the
//! falsifiable form of the § D2 per-site disposition table: a site that
//! passes the wrong `prior`, or a builder that stops computing the facts
//! from `(prior, state)`, turns these RED.
//!
//! Default lane: sim adapters for every port, `mtls_worker: None`, no
//! root, no netns. The Tier-3 sibling that drives the REAL exit observer
//! through two crash cycles is
//! `tests/integration/workload_lifecycle/crash_observability_two_cycles.rs`
//! (T-F).

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
#[cfg(feature = "integration-tests")]
use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(feature = "integration-tests")]
use overdrive_control_plane::action_shim::MtlsInterceptLifecycle;
use overdrive_control_plane::action_shim::{
    ShimError, WorkloadNetworkProvisioner, dispatch, dispatch_with_network_provisioner,
};
#[cfg(feature = "integration-tests")]
use overdrive_control_plane::veth_provisioner::NetSlot;
use overdrive_control_plane::veth_provisioner::{
    NET_SLOT_MAX, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, SpiffeId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
    DriverStartFailure, DriverType, Resources, VmPayload, VmStartFailure,
};
use overdrive_core::traits::intent_store::IntentStore;
#[cfg(feature = "integration-tests")]
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, PumpLiveness,
};
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocLifecyclePredecessor, AllocState, AllocStatusRow,
    LagAwareSubscription, LogicalTimestamp, NodeHealthRow, ObservationStore, ObservationStoreError,
    ObservationWrite, ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow,
    TransitionSource,
};
use overdrive_core::transition_reason::{TerminalCondition, TransitionReason};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
#[cfg(feature = "integration-tests")]
use overdrive_worker::mtls_intercept::{InterceptError, Result as InterceptResult};
#[cfg(feature = "integration-tests")]
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
#[cfg(feature = "integration-tests")]
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use tempfile::TempDir;
use tokio::sync::{Semaphore, oneshot};

/// Driver double whose `start` outcome is the variable under test:
/// `Accept` models a driver that brings the workload back up (T-C),
/// `Reject` models the `StartRejected` shape (T-G).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StartOutcome {
    Accept,
    Reject,
}

struct ScriptedDriver {
    outcome: StartOutcome,
    driver_type: DriverType,
    terminal_calls: Arc<AtomicUsize>,
    start_calls: Arc<AtomicUsize>,
    stop_calls: Arc<AtomicUsize>,
    stop_failures_remaining: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Driver for ScriptedDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.start_calls.fetch_add(1, Ordering::SeqCst);
        match self.outcome {
            StartOutcome::Accept => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(4242) })
            }
            StartOutcome::Reject => Err(DriverError::StartRejected {
                failure: overdrive_core::traits::driver::DriverStartFailure {
                    class: overdrive_core::traits::driver::DriverStartClass::Unclassified {
                        driver: self.driver_type,
                    },
                    detail: "scripted rejection: no capacity".to_owned(),
                },
            }),
        }
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        if self
            .stop_failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
            .is_ok()
        {
            return Err(DriverError::Io(std::io::Error::other("injected driver stop I/O")));
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

    fn on_alloc_terminal(&self, _alloc_id: &AllocationId) {
        self.terminal_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn release_supervision(&self, _alloc_id: &AllocationId) {
        self.terminal_calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CountingNetworkProvisioner {
    attempts: AtomicUsize,
    teardowns: AtomicUsize,
    failures_remaining: AtomicUsize,
}

impl CountingNetworkProvisioner {
    const fn fail_first_teardown() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            teardowns: AtomicUsize::new(0),
            failures_remaining: AtomicUsize::new(1),
        }
    }

    const fn succeed() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            teardowns: AtomicUsize::new(0),
            failures_remaining: AtomicUsize::new(0),
        }
    }
}

impl WorkloadNetworkProvisioner for CountingNetworkProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
            .is_ok()
        {
            return Err(VethProvisionError::SysctlSetFailed {
                key: "net.ipv4.ip_forward".to_owned(),
                value: "1".to_owned(),
                path: "/injected/finalization/teardown".to_owned(),
                source: std::io::Error::other("injected first teardown failure"),
            });
        }
        self.teardowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A C3 driven-port double that fails only the post-assignment provision
/// operation and records the externally observable cleanup order.
struct ProvisionFailureNetwork {
    trace: Arc<parking_lot::Mutex<Vec<&'static str>>>,
    teardown_fails: bool,
}

impl ProvisionFailureNetwork {
    fn error(operation: &str) -> VethProvisionError {
        VethProvisionError::SysctlSetFailed {
            key: "net.ipv4.ip_forward".to_owned(),
            value: "1".to_owned(),
            path: format!("/injected/post-assignment/{operation}"),
            source: std::io::Error::other(format!("injected {operation} failure")),
        }
    }
}

impl WorkloadNetworkProvisioner for ProvisionFailureNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        self.trace.lock().push("provision");
        Err(Self::error("provision"))
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.trace.lock().push("teardown");
        if self.teardown_fails { Err(Self::error("teardown")) } else { Ok(()) }
    }
}

#[derive(Clone, Copy)]
enum ProvisionFailureArm {
    Start,
    Restart,
}

struct ProvisionFailureResult {
    result: Result<(), ShimError>,
    row: Option<AllocStatusRow>,
    slot_held: bool,
    slot_released_at_failed_write: bool,
    driver_starts: usize,
    trace: Vec<&'static str>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the accepted C3 composition needs all real action-shim ports to observe one post-assignment failure"
)]
async fn drive_post_assignment_provision_failure(
    arm: ProvisionFailureArm,
    teardown_fails: bool,
    failed_write: bool,
    exhaust_slots: bool,
) -> ProvisionFailureResult {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let inner =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    if matches!(arm, ProvisionFailureArm::Restart) {
        let mut prior = seeded_failed_row(0, 0, None);
        prior.state = AllocState::Running;
        prior.reason = Some(TransitionReason::Started);
        prior.terminal = None;
        inner
            .write_alloc_lifecycle(prior, TransitionSource::Reconciler)
            .await
            .expect("seed prior Running row");
    }

    let net_slots = Arc::new(NetSlotAllocator::new());
    let trace = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let slot_released_at_failed_write = Arc::new(AtomicBool::new(false));
    let (pending, _entered) = PendingTerminalObservationStore::new(
        Arc::clone(&inner),
        alloc_id(),
        if failed_write { TerminalWriteOutcome::Failed } else { TerminalWriteOutcome::Accepted },
    );
    let observed_slots = Arc::clone(&net_slots);
    let observed_release = Arc::clone(&slot_released_at_failed_write);
    let observed_alloc = alloc_id();
    let pending = pending.with_write_trace(Arc::clone(&trace)).with_write_hook(move || {
        observed_release
            .store(!observed_slots.snapshot().contains_key(&observed_alloc), Ordering::SeqCst);
    });
    // The decorator still records the write at the actual driven-port
    // boundary, but this table does not need to suspend dispatch there.
    pending.resolve();

    let driver_starts = Arc::new(AtomicUsize::new(0));
    let driver: Arc<dyn Driver> = Arc::new(ScriptedDriver {
        outcome: StartOutcome::Accept,
        driver_type: DriverType::Vm,
        terminal_calls: Arc::new(AtomicUsize::new(0)),
        start_calls: Arc::clone(&driver_starts),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    });
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    if exhaust_slots {
        for slot in 0..=NET_SLOT_MAX {
            let holder = format!("post-assignment-slot-holder-{slot}");
            net_slots
                .assign(AllocationId::new(&holder).expect("valid held allocation id"))
                .expect("each held allocation takes one distinct slot");
        }
    }

    let action = match arm {
        ProvisionFailureArm::Start => Action::StartAllocation {
            alloc_id: alloc_id(),
            workload_id: workload_id(),
            node_id: node_id(),
            spec: vm_spec(),
            kind: WorkloadKind::Service,
        },
        ProvisionFailureArm::Restart => Action::RestartAllocation {
            alloc_id: alloc_id(),
            spec: vm_spec(),
            kind: WorkloadKind::Service,
        },
    };
    let network = ProvisionFailureNetwork { trace: Arc::clone(&trace), teardown_fails };
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 1,
        deadline: now + Duration::from_secs(2),
    };

    let result = dispatch_with_network_provisioner(
        vec![action],
        &drivers,
        &alloc_drivers,
        &pending,
        &overdrive_sim::adapters::dataplane::SimDataplane::new(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &NodeId::new("writer-1").expect("writer node"),
        allocator,
        &parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
        None,
        None,
        net_slots.as_ref(),
        &network,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await;

    ProvisionFailureResult {
        result,
        row: inner.alloc_status_row(&alloc_id()).await.expect("read allocation row"),
        slot_held: net_slots.snapshot().contains_key(&alloc_id()),
        slot_released_at_failed_write: slot_released_at_failed_write.load(Ordering::SeqCst),
        driver_starts: driver_starts.load(Ordering::SeqCst),
        trace: trace.lock().clone(),
    }
}

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-crashobs-0").expect("valid alloc id")
}

fn workload_id() -> WorkloadId {
    WorkloadId::new("crashobs").expect("valid workload id")
}

fn node_id() -> NodeId {
    NodeId::new("node-001").expect("valid node id")
}

fn spec() -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/crashobs/alloc/0")
            .expect("valid spiffe id"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            },
        ),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
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

fn vm_spec() -> AllocationSpec {
    AllocationSpec {
        driver: overdrive_core::traits::driver::DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/nonexistent/kernel"),
            rootfs: PathBuf::from("/nonexistent/rootfs"),
        }),
        ..spec()
    }
}

struct RejectedStartResult {
    dispatches: Vec<Result<(), ShimError>>,
    row: AllocStatusRow,
    occurrence: AllocLifecycleOccurrenceRow,
    slot_held: bool,
    teardown_attempts: usize,
    teardown_completions: usize,
    terminal_calls: usize,
}

#[derive(Clone, Copy)]
enum RejectedStartArm {
    Start,
    Restart,
}

#[allow(
    clippy::too_many_lines,
    reason = "the shared fresh/restart composition returns one complete observation, occurrence, release, and network-cleanup result"
)]
async fn drive_rejected_start(
    arm: RejectedStartArm,
    network: &CountingNetworkProvisioner,
    dispatch_count: usize,
) -> RejectedStartResult {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    if matches!(arm, RejectedStartArm::Restart) {
        obs.write_alloc_lifecycle(
            AllocStatusRow {
                alloc_id: alloc_id(),
                workload_id: workload_id(),
                node_id: node_id(),
                state: AllocState::Running,
                updated_at: LogicalTimestamp { counter: 0, writer: node_id() },
                reason: Some(TransitionReason::Started),
                detail: None,
                terminal: None,
                stderr_tail: None,
                kind: WorkloadKind::Service,
                listeners: Vec::new(),
                started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(
                    1_700_000_000,
                ))),
                workload_addr: None,
                last_terminated: None,
                restart_count: 0,
            },
            TransitionSource::Reconciler,
        )
        .await
        .expect("seed prior Running row");
    }

    let terminal_calls = Arc::new(AtomicUsize::new(0));
    let driver: Arc<dyn Driver> = Arc::new(ScriptedDriver {
        outcome: StartOutcome::Reject,
        driver_type: DriverType::Vm,
        terminal_calls: Arc::clone(&terminal_calls),
        start_calls: Arc::new(AtomicUsize::new(0)),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    });
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slots = NetSlotAllocator::new();
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(2),
    };
    let action = match arm {
        RejectedStartArm::Start => Action::StartAllocation {
            alloc_id: alloc_id(),
            workload_id: workload_id(),
            node_id: node_id(),
            spec: vm_spec(),
            kind: WorkloadKind::Service,
        },
        RejectedStartArm::Restart => Action::RestartAllocation {
            alloc_id: alloc_id(),
            spec: vm_spec(),
            kind: WorkloadKind::Service,
        },
    };
    let mut dispatches = Vec::with_capacity(dispatch_count);
    for _ in 0..dispatch_count {
        dispatches.push(
            dispatch_with_network_provisioner(
                vec![action.clone()],
                &drivers,
                &alloc_drivers,
                obs.as_ref(),
                &overdrive_sim::adapters::dataplane::SimDataplane::new(),
                &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                    overdrive_sim::adapters::entropy::SimEntropy::new(0),
                )),
                &overdrive_sim::adapters::clock::SimClock::new(),
                &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
                &lifecycle_tx,
                &tick,
                &NodeId::new("writer-1").expect("writer node"),
                Arc::clone(&allocator),
                &parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
                None,
                None,
                &net_slots,
                network,
                &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
            )
            .await,
        );
    }
    let row = obs
        .alloc_status_row(&alloc_id())
        .await
        .expect("read allocation row")
        .expect("a rejected start persists Failed");
    let occurrence = obs
        .alloc_lifecycle_occurrences(&alloc_id())
        .await
        .expect("read lifecycle occurrences")
        .into_iter()
        .last()
        .expect("the Failed transition has an occurrence");

    RejectedStartResult {
        dispatches,
        row,
        occurrence,
        slot_held: net_slots.snapshot().contains_key(&alloc_id()),
        teardown_attempts: network.attempts.load(Ordering::SeqCst),
        teardown_completions: network.teardowns.load(Ordering::SeqCst),
        terminal_calls: terminal_calls.load(Ordering::SeqCst),
    }
}

/// CONTRACT_SHAPE: bounded-change (typed rejection persists Failed before release).
#[tokio::test]
async fn driver_rejected_start_persists_failed_occurrence_before_release() {
    let network = CountingNetworkProvisioner::succeed();
    let mut outcome = drive_rejected_start(RejectedStartArm::Start, &network, 1).await;

    outcome
        .dispatches
        .pop()
        .expect("one dispatch")
        .expect("successful structural teardown closes the rejected start");
    assert_eq!(outcome.row.state, AllocState::Failed);
    assert_eq!(outcome.occurrence.alloc_id, outcome.row.alloc_id);
    assert_eq!(outcome.occurrence.to, outcome.row.state);
    assert_eq!(outcome.occurrence.from, AllocLifecyclePredecessor::Absent);
    assert_eq!(outcome.occurrence.source, TransitionSource::Driver(DriverType::Vm));
    assert_eq!(outcome.terminal_calls, 1, "supervision releases only after Failed is durable");
    assert!(!outcome.slot_held);
    assert_eq!(outcome.teardown_attempts, 1);
    assert_eq!(outcome.teardown_completions, 1);
}

/// CONTRACT_SHAPE: bounded-change (cleanup failure cannot erase typed rejection evidence).
#[tokio::test]
async fn driver_rejected_restart_persists_failed_occurrence_before_release() {
    let network = CountingNetworkProvisioner::succeed();
    let mut outcome = drive_rejected_start(RejectedStartArm::Restart, &network, 1).await;

    outcome
        .dispatches
        .pop()
        .expect("one dispatch")
        .expect("successful structural teardown closes the rejected restart");
    assert_eq!(outcome.row.state, AllocState::Failed);
    assert_eq!(outcome.occurrence.alloc_id, outcome.row.alloc_id);
    assert_eq!(outcome.occurrence.to, outcome.row.state);
    assert!(matches!(
        outcome.occurrence.from,
        AllocLifecyclePredecessor::State(AllocState::Running)
    ));
    assert_eq!(outcome.terminal_calls, 1, "the rejected claim releases after Failed is durable");
    assert!(!outcome.slot_held);
    assert_eq!(outcome.teardown_attempts, 1);
    assert_eq!(outcome.teardown_completions, 1);
}

/// CONTRACT_SHAPE: bounded-change (cleanup replay cannot be suppressed by duplicate ownership).
#[tokio::test]
async fn rejected_start_teardown_failure_replays_after_failed_closure() {
    let network = CountingNetworkProvisioner::fail_first_teardown();
    let outcome = drive_rejected_start(RejectedStartArm::Start, &network, 2).await;

    assert!(
        outcome.dispatches[0].is_err(),
        "the injected structural teardown failure remains visible after Failed is durable"
    );
    assert!(
        outcome.dispatches[1].is_ok(),
        "replay retries and completes structural teardown instead of being suppressed"
    );
    assert_eq!(outcome.row.state, AllocState::Failed);
    assert_eq!(outcome.occurrence.alloc_id, outcome.row.alloc_id);
    assert_eq!(outcome.occurrence.to, outcome.row.state);
    assert!(matches!(
        outcome.occurrence.from,
        AllocLifecyclePredecessor::State(AllocState::Failed)
    ));
    assert_eq!(
        outcome.terminal_calls, 2,
        "each rejected start releases after the durable boundary"
    );
    assert!(!outcome.slot_held, "replay releases the retained structural-network slot");
    assert_eq!(outcome.teardown_attempts, 2);
    assert_eq!(outcome.teardown_completions, 1);
}

/// Seed a terminal `Failed` row at `counter` carrying the crash facts a
/// SIGKILL produces, plus whatever crash history the scenario needs the
/// successor write to forward or replace.
fn seeded_failed_row(
    counter: u64,
    restart_count: u32,
    last_terminated: Option<overdrive_core::traits::observation_store::LastTerminated>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc_id(),
        workload_id: workload_id(),
        node_id: node_id(),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter, writer: node_id() },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: Some("Segmentation fault".to_owned()),
        }),
        detail: Some("killed by SIGKILL".to_owned()),
        terminal: None,
        stderr_tail: Some("Segmentation fault".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated,
        restart_count,
    }
}

/// Dispatch `action` against a store seeded with `seed`, and return the
/// LWW-winner row afterwards.
async fn dispatch_against_seed(seed: AllocStatusRow, action: Action) -> AllocStatusRow {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    obs.write_alloc_lifecycle(
        seed,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior row");

    let start_outcome = if matches!(action, Action::RestartAllocation { .. }) {
        StartOutcome::Accept
    } else {
        StartOutcome::Reject
    };
    dispatch_with_driver(obs.as_ref(), store, action, start_outcome).await;

    obs.alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a successor row must exist after dispatch")
}

async fn dispatch_with_driver(
    obs: &dyn ObservationStore,
    store: Arc<dyn IntentStore>,
    action: Action,
    outcome: StartOutcome,
) {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let driver: Arc<dyn Driver> = Arc::new(ScriptedDriver {
        outcome,
        driver_type: DriverType::Exec,
        terminal_calls: Arc::new(AtomicUsize::new(0)),
        start_calls: Arc::new(AtomicUsize::new(0)),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    });
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    dispatch(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
        None,
        None,
        &net_slot_allocator,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed");
}

/// Dispatch a `RestartAllocation` against `seed`, with the driver's
/// `start` outcome as the variable.
async fn restart_against(seed: AllocStatusRow, outcome: StartOutcome) -> AllocStatusRow {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    obs.write_alloc_lifecycle(
        seed,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior row");

    dispatch_with_driver(
        obs.as_ref(),
        store,
        Action::RestartAllocation {
            alloc_id: alloc_id(),
            spec: spec(),
            kind: WorkloadKind::Service,
            // `None` is the crash-loop restart pathway — the restart cause
            // is implicit in the prior alloc's terminal, which is exactly
            // the shape under test.
        },
        outcome,
    )
    .await;

    obs.alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a successor row must exist after dispatch")
}

#[derive(Debug, Clone, Copy)]
enum TerminalWriteOutcome {
    Accepted,
    Failed,
    RejectedByConcurrentExit,
    RejectedTwiceByConcurrentExit,
    ExactRequestedTerminal,
    PointReadFailed,
}

/// Observation-store boundary double that parks exactly the terminal compound
/// write while delegating every other operation to the real simulation
/// adapter. This exposes the interval in which VM supervision must remain the
/// exclusive reclamation/same-id ownership fence.
struct PendingTerminalObservationStore {
    inner: Arc<SimObservationStore>,
    target: AllocationId,
    outcome: TerminalWriteOutcome,
    write_trace: Option<Arc<parking_lot::Mutex<Vec<&'static str>>>>,
    write_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    entered: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    resume: Semaphore,
    terminal_proposals: AtomicUsize,
    point_reads: AtomicUsize,
}

impl PendingTerminalObservationStore {
    fn new(
        inner: Arc<SimObservationStore>,
        target: AllocationId,
        outcome: TerminalWriteOutcome,
    ) -> (Self, oneshot::Receiver<()>) {
        let (entered, entered_rx) = oneshot::channel();
        (
            Self {
                inner,
                target,
                outcome,
                write_trace: None,
                write_hook: None,
                entered: parking_lot::Mutex::new(Some(entered)),
                resume: Semaphore::new(0),
                terminal_proposals: AtomicUsize::new(0),
                point_reads: AtomicUsize::new(0),
            },
            entered_rx,
        )
    }

    fn resolve(&self) {
        self.resume.add_permits(1);
    }

    fn with_write_trace(mut self, trace: Arc<parking_lot::Mutex<Vec<&'static str>>>) -> Self {
        self.write_trace = Some(trace);
        self
    }

    fn with_write_hook(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.write_hook = Some(Arc::new(hook));
        self
    }

    fn terminal_proposal_count(&self) -> usize {
        self.terminal_proposals.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ObservationStore for PendingTerminalObservationStore {
    async fn write(&self, row: ObservationWrite) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        if current.alloc_id == self.target {
            if let Some(hook) = &self.write_hook {
                hook();
            }
            if let Some(trace) = &self.write_trace {
                trace.lock().push("failed-write");
            }
            let proposal = self.terminal_proposals.fetch_add(1, Ordering::SeqCst);
            let entered = { self.entered.lock().take() };
            if let Some(entered) = entered {
                let _ = entered.send(());
                self.resume
                    .acquire()
                    .await
                    .expect("terminal-write test owns the semaphore")
                    .forget();
                if matches!(self.outcome, TerminalWriteOutcome::Failed) {
                    return Err(ObservationStoreError::Io(std::io::Error::from(
                        std::io::ErrorKind::PermissionDenied,
                    )));
                }
            }
            let rejection_limit = match self.outcome {
                TerminalWriteOutcome::RejectedByConcurrentExit => 1,
                TerminalWriteOutcome::RejectedTwiceByConcurrentExit => 2,
                TerminalWriteOutcome::Accepted
                | TerminalWriteOutcome::Failed
                | TerminalWriteOutcome::ExactRequestedTerminal
                | TerminalWriteOutcome::PointReadFailed => 0,
            };
            if matches!(self.outcome, TerminalWriteOutcome::RejectedTwiceByConcurrentExit)
                && proposal >= rejection_limit
            {
                panic!("a third terminal proposal reached the observation boundary");
            }
            if proposal < rejection_limit {
                let mut exit_observation = current.clone();
                exit_observation.reason = Some(TransitionReason::Stopped {
                    by: overdrive_core::transition_reason::StoppedBy::Operator,
                });
                exit_observation.terminal = None;
                self.inner
                    .write_alloc_lifecycle(
                        exit_observation,
                        TransitionSource::Driver(DriverType::Exec),
                    )
                    .await?
                    .expect("the concurrent exit observation wins the stale timestamp");
            }
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
        let read = self.point_reads.fetch_add(1, Ordering::SeqCst);
        if matches!(self.outcome, TerminalWriteOutcome::PointReadFailed) && read == 1 {
            return Err(ObservationStoreError::Io(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )));
        }
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

#[derive(Clone)]
struct TerminalFenceDriver {
    held: Arc<parking_lot::Mutex<BTreeSet<AllocationId>>>,
    terminal_calls: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
    stop_entered: Arc<parking_lot::Mutex<Option<oneshot::Sender<()>>>>,
    stop_resume: Arc<Semaphore>,
}

impl TerminalFenceDriver {
    fn holding(alloc: &AllocationId) -> Self {
        Self {
            held: Arc::new(parking_lot::Mutex::new(BTreeSet::from([alloc.clone()]))),
            terminal_calls: Arc::new(AtomicUsize::new(0)),
            releases: Arc::new(AtomicUsize::new(0)),
            stop_entered: Arc::new(parking_lot::Mutex::new(None)),
            stop_resume: Arc::new(Semaphore::new(0)),
        }
    }

    fn holding_with_blocked_stop(alloc: &AllocationId) -> (Self, oneshot::Receiver<()>) {
        let driver = Self::holding(alloc);
        let (entered, entered_rx) = oneshot::channel();
        *driver.stop_entered.lock() = Some(entered);
        (driver, entered_rx)
    }

    fn resolve_stop(&self) {
        self.stop_resume.add_permits(1);
    }

    fn try_insert_claim(&self, alloc: &AllocationId) -> bool {
        let mut held = self.held.lock();
        if held.contains(alloc) {
            false
        } else {
            held.insert(alloc.clone());
            true
        }
    }
}

#[async_trait::async_trait]
impl Driver for TerminalFenceDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        if self.try_insert_claim(&spec.alloc) {
            Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
        } else {
            Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned {
                        alloc: spec.alloc.clone(),
                    }),
                    detail: "allocation already has a terminal-write owner".to_owned(),
                },
            })
        }
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        let entered = { self.stop_entered.lock().take() };
        if let Some(entered) = entered {
            let _ = entered.send(());
            self.stop_resume.acquire().await.expect("stop-race test owns the semaphore").forget();
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

    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        Some(self.held.lock().iter().cloned().collect())
    }

    fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
        self.try_insert_claim(alloc)
    }

    fn on_alloc_terminal(&self, _alloc_id: &AllocationId) {
        self.terminal_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn release_supervision(&self, alloc_id: &AllocationId) {
        self.held.lock().remove(alloc_id);
        self.releases.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum TerminalActionArm {
    FinalizeFailed,
    StopAllocation,
}

impl TerminalActionArm {
    const fn action(self, alloc_id: AllocationId) -> Action {
        match self {
            Self::FinalizeFailed => {
                Action::FinalizeFailed { alloc_id, terminal: Some(self.terminal()) }
            }
            Self::StopAllocation => {
                Action::StopAllocation { alloc_id, terminal: Some(self.terminal()) }
            }
        }
    }

    const fn terminal(self) -> TerminalCondition {
        match self {
            Self::FinalizeFailed => TerminalCondition::BackoffExhausted { attempts: 3 },
            Self::StopAllocation => TerminalCondition::Stopped {
                by: overdrive_core::transition_reason::StoppedBy::Operator,
            },
        }
    }

    const fn terminal_state(self) -> AllocState {
        match self {
            Self::FinalizeFailed => AllocState::Failed,
            Self::StopAllocation => AllocState::Terminated,
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the production dispatch must remain live while the test inspects the pending compound-write ownership fence"
)]
async fn assert_terminal_write_partition(arm: TerminalActionArm, outcome: TerminalWriteOutcome) {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let alloc = alloc_id();
    let mut prior = seeded_failed_row(7, 0, None);
    if matches!(arm, TerminalActionArm::StopAllocation) {
        prior.kind = WorkloadKind::Job;
    }
    prior.state = AllocState::Running;
    prior.reason = Some(TransitionReason::Started);
    prior.terminal = None;
    if matches!(outcome, TerminalWriteOutcome::ExactRequestedTerminal) {
        prior.state = arm.terminal_state();
        prior.terminal = Some(arm.terminal());
    }

    let inner =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    inner
        .write_alloc_lifecycle(prior.clone(), TransitionSource::Reconciler)
        .await
        .expect("seed prior Running row");
    let (pending_store, entered) =
        PendingTerminalObservationStore::new(Arc::clone(&inner), alloc.clone(), outcome);
    let pending_store = Arc::new(pending_store);

    let driver = Arc::new(TerminalFenceDriver::holding(&alloc));
    let driver_port: Arc<dyn Driver> = driver.clone();
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver_port);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    alloc_drivers.lock().insert(alloc.clone(), DriverType::Vm);

    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = overdrive_sim::adapters::clock::SimClock::new();
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("writer node");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let net_slots = NetSlotAllocator::new();
    let host_state = overdrive_sim::adapters::vm_host_state::SimVmHostState::new();
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: if matches!(
            outcome,
            TerminalWriteOutcome::RejectedByConcurrentExit
                | TerminalWriteOutcome::RejectedTwiceByConcurrentExit
        ) {
            6
        } else {
            8
        },
        deadline: now + Duration::from_secs(2),
    };

    let dispatch = dispatch(
        vec![arm.action(alloc.clone())],
        &drivers,
        &alloc_drivers,
        pending_store.as_ref(),
        &dataplane,
        &ca,
        &clock,
        &identity,
        &lifecycle_tx,
        &tick,
        &writer_node,
        allocator,
        &broker,
        None,
        None,
        &net_slots,
        &host_state,
    );
    tokio::pin!(dispatch);

    let bypasses_terminal_write = matches!(
        outcome,
        TerminalWriteOutcome::ExactRequestedTerminal | TerminalWriteOutcome::PointReadFailed
    );
    if !bypasses_terminal_write {
        tokio::select! {
            entered = entered => entered.expect("terminal write reached the pending partition"),
            completed = &mut dispatch => panic!("terminal dispatch completed before its compound write resolved: {completed:?}"),
        }

        assert_eq!(driver.terminal_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            driver.live_allocations(),
            Some(vec![alloc.clone()]),
            "VM reclamation must still observe the allocation as supervised while the terminal write is pending",
        );
        assert_eq!(
            driver.releases.load(Ordering::SeqCst),
            0,
            "release_supervision must not run before the terminal compound write resolves",
        );
        assert!(
            !driver.try_begin_reclamation(&alloc),
            "the real reclamation-lease primitive cannot acquire the VM supervision slot while the terminal write is pending",
        );
        assert!(
            matches!(
                driver.start(&vm_spec()).await,
                Err(DriverError::StartRejected {
                    failure: DriverStartFailure {
                        class: DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned { .. }),
                        ..
                    }
                })
            ),
            "a same-id start cannot acquire the VM supervision slot while the terminal write is pending",
        );

        pending_store.resolve();
    }
    let result = dispatch.await;
    match outcome {
        TerminalWriteOutcome::Accepted
        | TerminalWriteOutcome::RejectedByConcurrentExit
        | TerminalWriteOutcome::RejectedTwiceByConcurrentExit
        | TerminalWriteOutcome::ExactRequestedTerminal => {
            result.expect("accepted terminal write completes");
        }
        TerminalWriteOutcome::Failed | TerminalWriteOutcome::PointReadFailed => assert!(
            matches!(result, Err(ShimError::Observation(_))),
            "a failed terminal write returns its store error after abandonment: {result:?}",
        ),
    }
    assert_eq!(
        driver.releases.load(Ordering::SeqCst),
        1,
        "both accepted and failed terminal writes release supervision exactly once after resolution",
    );
    match outcome {
        TerminalWriteOutcome::Accepted
        | TerminalWriteOutcome::RejectedByConcurrentExit
        | TerminalWriteOutcome::RejectedTwiceByConcurrentExit
        | TerminalWriteOutcome::ExactRequestedTerminal => assert!(
            driver.try_begin_reclamation(&alloc),
            "the reclamation lease becomes available only after the terminal write resolves",
        ),
        TerminalWriteOutcome::Failed | TerminalWriteOutcome::PointReadFailed => assert!(
            driver.start(&vm_spec()).await.is_ok(),
            "a same-id start becomes possible only after failed-write abandonment resolves",
        ),
    }

    let expected_proposals = match outcome {
        TerminalWriteOutcome::Accepted | TerminalWriteOutcome::Failed => 1,
        TerminalWriteOutcome::RejectedByConcurrentExit
        | TerminalWriteOutcome::RejectedTwiceByConcurrentExit => 2,
        TerminalWriteOutcome::ExactRequestedTerminal | TerminalWriteOutcome::PointReadFailed => 0,
    };
    assert_eq!(
        pending_store.terminal_proposal_count(),
        expected_proposals,
        "the terminal proposal count stays within the bounded Stop contract for {outcome:?}",
    );

    let current = inner
        .alloc_status_row(&alloc)
        .await
        .expect("read current row")
        .expect("seed remains present");
    let occurrences =
        inner.alloc_lifecycle_occurrences(&alloc).await.expect("read lifecycle occurrences");
    match outcome {
        TerminalWriteOutcome::Accepted => {
            assert_eq!(current.state, arm.terminal_state());
            assert_eq!(occurrences.len(), 2, "current and occurrence commit together");
        }
        TerminalWriteOutcome::RejectedByConcurrentExit => {
            assert_eq!(current.state, arm.terminal_state());
            assert_eq!(
                current.terminal,
                Some(TerminalCondition::Stopped {
                    by: overdrive_core::transition_reason::StoppedBy::Operator,
                }),
                "the terminal action must rebase after the equal-timestamp exit observation wins",
            );
            assert_eq!(
                occurrences.len(),
                3,
                "Running, concurrent exit, and rebased terminal commit in order",
            );
        }
        TerminalWriteOutcome::RejectedTwiceByConcurrentExit => {
            assert_eq!(current.state, AllocState::Terminated);
            assert_eq!(
                current.terminal, None,
                "the exit observer's second accepted row remains authoritative",
            );
            assert_eq!(
                occurrences.len(),
                3,
                "only Running and the two competing exit observations are durable",
            );
        }
        TerminalWriteOutcome::ExactRequestedTerminal => {
            assert_eq!(current, prior, "the exact requested terminal is not re-authored");
            assert_eq!(occurrences.len(), 1, "the exact terminal appends no occurrence");
        }
        TerminalWriteOutcome::Failed | TerminalWriteOutcome::PointReadFailed => {
            assert_eq!(current, prior, "failed compound write mutates no current row");
            assert_eq!(occurrences.len(), 1, "failed compound write appends no occurrence");
        }
    }

    let expected_events = match outcome {
        TerminalWriteOutcome::Accepted | TerminalWriteOutcome::RejectedByConcurrentExit => 1,
        TerminalWriteOutcome::RejectedTwiceByConcurrentExit
        | TerminalWriteOutcome::ExactRequestedTerminal
        | TerminalWriteOutcome::Failed
        | TerminalWriteOutcome::PointReadFailed => 0,
    };
    let mut events = Vec::new();
    while let Ok(event) = lifecycle_rx.try_recv() {
        events.push(event);
    }
    assert_eq!(
        events.len(),
        expected_events,
        "only an accepted Stop terminal proposal broadcasts an occurrence",
    );

    let route = alloc_drivers.lock().get(&alloc).copied();
    match outcome {
        TerminalWriteOutcome::Accepted
        | TerminalWriteOutcome::RejectedByConcurrentExit
        | TerminalWriteOutcome::RejectedTwiceByConcurrentExit
        | TerminalWriteOutcome::ExactRequestedTerminal => {
            assert!(route.is_none(), "completed Stop removes its process-local driver route");
        }
        TerminalWriteOutcome::Failed | TerminalWriteOutcome::PointReadFailed => {
            assert_eq!(route, Some(DriverType::Vm), "store failures preserve the driver route");
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn finalize_failed_holds_supervision_through_terminal_write_resolution() {
    for outcome in [TerminalWriteOutcome::Accepted, TerminalWriteOutcome::Failed] {
        assert_terminal_write_partition(TerminalActionArm::FinalizeFailed, outcome).await;
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn stop_allocation_holds_supervision_through_terminal_write_resolution() {
    for outcome in [TerminalWriteOutcome::Accepted, TerminalWriteOutcome::Failed] {
        assert_terminal_write_partition(TerminalActionArm::StopAllocation, outcome).await;
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn stop_allocation_retries_after_equal_timestamp_exit_wins() {
    assert_terminal_write_partition(
        TerminalActionArm::StopAllocation,
        TerminalWriteOutcome::RejectedByConcurrentExit,
    )
    .await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the production stop dispatch must remain live while the test deterministically authors the exit-observer winner"
)]
async fn stop_allocation_rebases_terminal_write_on_exit_observer_winner() {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let alloc = alloc_id();
    let mut running = seeded_failed_row(7, 0, None);
    running.kind = WorkloadKind::Job;
    running.state = AllocState::Running;
    running.reason = Some(TransitionReason::Started);
    running.terminal = None;

    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    obs.write_alloc_lifecycle(running.clone(), TransitionSource::Reconciler)
        .await
        .expect("seed prior Running row");

    let (driver, stop_entered) = TerminalFenceDriver::holding_with_blocked_stop(&alloc);
    let driver = Arc::new(driver);
    let driver_port: Arc<dyn Driver> = driver.clone();
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver_port);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    alloc_drivers.lock().insert(alloc.clone(), DriverType::Vm);

    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = overdrive_sim::adapters::clock::SimClock::new();
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("writer node");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let net_slots = NetSlotAllocator::new();
    let host_state = overdrive_sim::adapters::vm_host_state::SimVmHostState::new();
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 8,
        deadline: now + Duration::from_secs(2),
    };
    let terminal = Some(TerminalCondition::Stopped {
        by: overdrive_core::transition_reason::StoppedBy::Operator,
    });

    let dispatch = dispatch(
        vec![Action::StopAllocation { alloc_id: alloc.clone(), terminal: terminal.clone() }],
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        &dataplane,
        &ca,
        &clock,
        &identity,
        &lifecycle_tx,
        &tick,
        &writer_node,
        allocator,
        &broker,
        None,
        None,
        &net_slots,
        &host_state,
    );
    tokio::pin!(dispatch);

    tokio::select! {
        entered = stop_entered => entered.expect("operator stop reached the deterministic exit race"),
        completed = &mut dispatch => panic!("operator stop completed before the exit observer partition: {completed:?}"),
    }

    let mut observed_exit = running.clone();
    observed_exit.state = AllocState::Terminated;
    observed_exit.updated_at =
        LogicalTimestamp::dominating(9, running.node_id.clone(), Some(&running.updated_at));
    observed_exit.reason = Some(TransitionReason::Stopped {
        by: overdrive_core::transition_reason::StoppedBy::Operator,
    });
    observed_exit.terminal = None;
    obs.write_alloc_lifecycle(observed_exit, TransitionSource::Driver(DriverType::Vm))
        .await
        .expect("exit observer wins while Driver::stop is awaiting quiescence")
        .expect("exit observation dominates Running");

    driver.resolve_stop();
    dispatch.await.expect("operator stop must author its terminal after cleanup");

    let current = obs
        .alloc_status_row(&alloc)
        .await
        .expect("read current allocation row")
        .expect("terminal allocation row exists");
    assert_eq!(current.state, AllocState::Terminated);
    assert_eq!(
        current.terminal, terminal,
        "the operator terminal must dominate the intentional-stop observation authored during cleanup",
    );
    assert_eq!(current.updated_at.counter, 11);

    let occurrences =
        obs.alloc_lifecycle_occurrences(&alloc).await.expect("read lifecycle occurrences");
    assert_eq!(
        occurrences.len(),
        3,
        "Running, intentional-stop, and operator-terminal each occur exactly once",
    );
    let operator_terminal = occurrences.last().expect("operator terminal occurrence");
    assert_eq!(operator_terminal.from, AllocLifecyclePredecessor::State(AllocState::Terminated),);
    assert_eq!(operator_terminal.to, AllocState::Terminated);
    assert_eq!(operator_terminal.terminal, current.terminal);
    assert_eq!(operator_terminal.source, TransitionSource::Reconciler);
    assert_eq!(driver.terminal_calls.load(Ordering::SeqCst), 1);
    assert_eq!(driver.releases.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// T-C — the shim writes the facts on a successful restart
// ---------------------------------------------------------------------------

/// T-C: seed a `Failed` row at counter `K` carrying
/// `WorkloadCrashedImmediately`; dispatch `Action::RestartAllocation`;
/// the stored row is `Running`, `restart_count == 1`, and
/// `last_terminated` snapshots the seeded row.
///
/// This is § D2 site 5 — the crash-observability site — exercised
/// end-to-end through the real dispatcher.
#[tokio::test]
async fn restart_allocation_snapshots_the_crash_and_counts_the_restart() {
    let seed = seeded_failed_row(7, 0, None);
    let row = restart_against(seed.clone(), StartOutcome::Accept).await;

    assert_eq!(row.state, AllocState::Running, "a successful restart lands Running");
    assert_eq!(row.restart_count, 1, "the observed restart is counted");

    let lt = row.last_terminated.as_ref().expect("the recovered row must carry last_terminated");
    assert_eq!(lt.state, AllocState::Failed, "the snapshot describes the terminal it superseded");
    assert_eq!(lt.reason, seed.reason, "the typed cause-class rides the snapshot verbatim");
    assert_eq!(lt.detail, seed.detail, "the verbatim driver text rides the snapshot");
    assert_eq!(lt.stderr_tail, seed.stderr_tail, "the workload's dying words ride the snapshot");
    assert_eq!(lt.started_at, seed.started_at, "the dead generation's start wall-clock rides it");
    assert_eq!(
        lt.terminated_at, seed.updated_at,
        "the snapshot identifies exactly WHICH durable observation it summarises",
    );

    assert!(
        row.updated_at.dominates(&lt.terminated_at),
        "the recovered row must strictly dominate the terminal it snapshots",
    );
}

// ---------------------------------------------------------------------------
// T-G — a driver-REJECTED restart forwards; it neither counts nor overwrites
// ---------------------------------------------------------------------------

/// T-G: seed a `Failed` row already carrying crash history; dispatch
/// `RestartAllocation` against a driver returning `StartRejected`. The
/// successor row stays `Failed`, `restart_count` is UNCHANGED, and
/// `last_terminated` is the FORWARDED prior value — not a snapshot of
/// the rejected row.
///
/// Pins § D1's `terminal → terminal` edge case at a real call site.
/// Nothing restarted, so nothing is counted; the prior crash's facts are
/// lost to the accepted depth-1 limit, and the *attempt* is counted
/// separately by the reconciler's own budget.
#[tokio::test]
async fn driver_rejected_restart_forwards_and_does_not_count() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Terminated,
        reason: Some(TransitionReason::Stopped {
            by: overdrive_core::transition_reason::StoppedBy::Process,
        }),
        detail: Some("an earlier, unrelated terminal".to_owned()),
        terminal: Some(TerminalCondition::Completed { exit_code: 0 }),
        stderr_tail: None,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_600_000_000))),
        terminated_at: LogicalTimestamp { counter: 3, writer: node_id() },
    };
    let seed = seeded_failed_row(7, 2, Some(earlier.clone()));

    let row = restart_against(seed, StartOutcome::Reject).await;

    assert_eq!(row.state, AllocState::Failed, "a rejected restart lands Failed");
    assert_eq!(row.restart_count, 2, "nothing restarted — the count is UNCHANGED");
    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "the prior's snapshot is FORWARDED, not overwritten with the rejected row",
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// T-H — `FinalizeFailed` does not self-duplicate
// ---------------------------------------------------------------------------

/// T-H: seed a `Failed` row carrying a `reason`; dispatch
/// `FinalizeFailed { terminal: Some(BackoffExhausted { .. }) }`. The
/// written row's `last_terminated` is the FORWARDED prior value (`None`
/// on a first failure), NOT a snapshot of the row's own `reason` /
/// `detail` / `stderr_tail`.
///
/// This is the falsifiable form of § D2 site 3. `FinalizeFailed` is not a
/// *new* terminal — it is the same terminal restamped with a terminal
/// claim — so snapshotting there would put five facts on one row twice.
#[tokio::test]
async fn finalize_failed_does_not_snapshot_its_own_row() {
    let seed = seeded_failed_row(7, 0, None);

    let row = dispatch_against_seed(
        seed.clone(),
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 3 }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Failed, "a genuine terminal claim lands Failed");
    assert_eq!(
        row.last_terminated, None,
        "FinalizeFailed FORWARDS the prior's (absent) snapshot — it must NOT self-describe, \
         which would put reason/detail/stderr_tail/started_at/terminal on the row twice",
    );
    assert_eq!(row.restart_count, 0, "restamping a terminal is not a restart");
    // The row's OWN fields still carry the crash facts — that is where a
    // terminal row's facts live (§ D1's "last_terminated never describes
    // the row that carries it").
    assert_eq!(row.reason, seed.reason, "the row's own reason is forward-carried as before");
    assert_eq!(row.stderr_tail, seed.stderr_tail, "and so is its own stderr_tail");
}

/// The companion to T-H: a `FinalizeFailed` against a prior that ALREADY
/// carries a snapshot forwards that snapshot verbatim rather than
/// dropping it. Kills a mutant that replaces the forward-carry with a
/// literal `None` — which `finalize_failed_does_not_snapshot_its_own_row`
/// alone cannot catch, because there the expected value IS `None`.
#[tokio::test]
async fn finalize_failed_forwards_an_existing_snapshot() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Failed,
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(1),
            signal: None,
            stderr_tail: None,
        }),
        detail: Some("the crash this alloc previously survived".to_owned()),
        terminal: None,
        stderr_tail: None,
        started_at: None,
        terminated_at: LogicalTimestamp { counter: 4, writer: node_id() },
    };
    let seed = seeded_failed_row(7, 5, Some(earlier.clone()));

    let row = dispatch_against_seed(
        seed,
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
        },
    )
    .await;

    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "the terminal row must keep describing the crash the alloc previously survived",
    );
    assert_eq!(row.restart_count, 5, "and the monotone counter rides through the terminal");
}

/// CONTRACT_SHAPE: bounded-change (one pre-READY terminal write, no restart side effect).
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn unreported_pre_ready_vmm_exit_finalizes_once_without_restart_or_view_change() {
    let mut seed = seeded_failed_row(7, 4, None);
    seed.kind = WorkloadKind::Job;
    seed.reason =
        Some(TransitionReason::VmGuestExitUnreported { vmm_exit_code: None, vmm_signal: Some(9) });

    let before = seed.clone();
    let row = dispatch_against_seed(
        seed,
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::Failed { exit_code: None }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Failed);
    assert_eq!(row.terminal, Some(TerminalCondition::Failed { exit_code: None }));
    assert_eq!(row.restart_count, 4, "finalization must not count a restart");
    assert!(matches!(
        row.reason,
        Some(TransitionReason::VmGuestExitUnreported { vmm_exit_code: None, vmm_signal: Some(9) })
    ));
    assert_eq!(row.alloc_id, before.alloc_id);
    assert_eq!(row.workload_id, before.workload_id);
    assert_eq!(row.node_id, before.node_id);
    assert_eq!(row.detail, before.detail);
    assert_eq!(row.stderr_tail, before.stderr_tail);
    assert_eq!(row.kind, before.kind);
    assert_eq!(row.listeners, before.listeners);
    assert_eq!(row.started_at, before.started_at);
    assert_eq!(row.workload_addr, before.workload_addr);
    assert_eq!(row.last_terminated, before.last_terminated);
    assert_eq!(row.updated_at.counter, before.updated_at.counter + 1);
    assert_eq!(row.updated_at.writer, before.updated_at.writer);
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn same_job_finalization_is_terminal_and_count_preserving() {
    let mut seed = seeded_failed_row(11, 4, None);
    seed.kind = WorkloadKind::Job;
    seed.reason = Some(TransitionReason::WorkloadCrashedImmediately {
        exit_code: Some(78),
        signal: None,
        stderr_tail: None,
    });
    let terminal = Some(TerminalCondition::Failed { exit_code: Some(78) });
    let action = Action::FinalizeFailed { alloc_id: alloc_id(), terminal: terminal.clone() };

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    obs.write_alloc_lifecycle(seed.clone(), TransitionSource::Reconciler)
        .await
        .expect("seed the pre-final row");

    let terminal_calls = Arc::new(AtomicUsize::new(0));
    let driver: Arc<dyn Driver> = Arc::new(ScriptedDriver {
        outcome: StartOutcome::Accept,
        driver_type: DriverType::Exec,
        terminal_calls: Arc::clone(&terminal_calls),
        start_calls: Arc::new(AtomicUsize::new(0)),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    });
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
    let net_slots = NetSlotAllocator::new();
    net_slots.assign(alloc_id()).expect("pre-final allocation owns one network slot");
    let network = CountingNetworkProvisioner::succeed();
    let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    for _ in 0..2 {
        dispatch_with_network_provisioner(
            vec![action.clone()],
            &drivers,
            &alloc_drivers,
            obs.as_ref(),
            &overdrive_sim::adapters::dataplane::SimDataplane::new(),
            &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(0),
            )),
            &overdrive_sim::adapters::clock::SimClock::new(),
            &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
            &lifecycle_tx,
            &tick,
            &NodeId::new("writer-1").expect("writer node"),
            Arc::clone(&allocator),
            &parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
            None,
            None,
            &net_slots,
            &network,
            &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
        )
        .await
        .expect("first finalization commits and replay is an exact no-op");
    }

    let row = obs
        .alloc_status_row(&alloc_id())
        .await
        .expect("read final row")
        .expect("final row remains present");
    assert_eq!(row.state, AllocState::Failed);
    assert_eq!(row.terminal, terminal);
    assert_eq!(row.restart_count, seed.restart_count, "finalization never counts a restart");
    assert_eq!(row.updated_at.counter, seed.updated_at.counter + 1);
    assert_eq!(network.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 2, "terminal hook and release run once");
    assert!(!net_slots.snapshot().contains_key(&alloc_id()));
    let occurrences = obs
        .alloc_lifecycle_occurrences(&alloc_id())
        .await
        .expect("read compound lifecycle occurrences");
    assert_eq!(occurrences.len(), 2, "seed plus exactly one final occurrence");
    let event = lifecycle_rx.try_recv().expect("first finalization broadcasts once");
    assert_eq!(event.alloc_id, alloc_id());
    assert!(matches!(
        lifecycle_rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// § D2 site 6 — StopAllocation forwards
// ---------------------------------------------------------------------------

/// § D2 site 6: `Running → Terminated` is a non-terminal prior, so
/// `StopAllocation` forwards both fields. A Terminated row carrying a
/// prior generation's `last_terminated` keeps that history visible on the
/// durable surface an operator polls after the stop.
#[tokio::test]
async fn stop_allocation_forwards_the_crash_history() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Failed,
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        started_at: None,
        terminated_at: LogicalTimestamp { counter: 2, writer: node_id() },
    };
    let mut seed = seeded_failed_row(7, 1, Some(earlier.clone()));
    seed.state = AllocState::Running;

    let row = dispatch_against_seed(
        seed,
        Action::StopAllocation {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::Stopped {
                by: overdrive_core::transition_reason::StoppedBy::Operator,
            }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Terminated, "the stop lands Terminated");
    assert_eq!(row.restart_count, 1, "the monotone counter survives an operator stop");
    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "a stopped alloc still shows the crash it previously survived",
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// § D2 — the structured `alloc.restart.observed` event
// ---------------------------------------------------------------------------
//
// ADR-0078 § D2 mandates a structured event at the single increment site so a
// crash-and-recover is *alertable*, not merely pollable. Its emission is gated
// on `facts.restart_count > superseded.restart_count` inside
// `build_alloc_status_row` — a comparison with NO effect on the written row, so
// nothing else in the suite can falsify it. Without these two tests the gate's
// `>` mutants (`==`, `<`, `>=`) all survive and the "alertable" claim is
// folklore.
//
// Capture mechanism: thread-local `tracing::subscriber::set_default`, the same
// harness `sim_observation_lww_reject_logging.rs` uses. Thread-local is
// sufficient because `build_alloc_status_row` emits synchronously on the
// caller's thread inside `dispatch`, and every test here is
// `flavor = "current_thread"`.

use std::sync::Mutex;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// Records every event the subscriber sees as `"<name> | f=v …"`.
#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn restart_observed(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("captured events mutex")
            .iter()
            .filter(|e| e.contains("alloc.restart.observed"))
            .cloned()
            .collect()
    }
}

struct V<'a>(&'a mut String);

impl tracing::field::Visit for V<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut buf = String::new();
        buf.push_str(event.metadata().name());
        event.record(&mut V(&mut buf));
        self.inner.lock().expect("captured events mutex").push(buf);
    }
}

/// The event FIRES on the write that observes a restart landing, and carries
/// the alloc, the workload, the new count, and the state it recovered from.
#[tokio::test(flavor = "current_thread")]
async fn restart_landing_emits_the_structured_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let seed = seeded_failed_row(7, 4, None);
    let row = restart_against(seed, StartOutcome::Accept).await;
    assert_eq!(row.restart_count, 5, "precondition: the restart was counted");

    let events = captured.restart_observed();
    assert_eq!(
        events.len(),
        1,
        "exactly one alloc.restart.observed event per observed restart; got {events:?}",
    );
    let event = &events[0];
    assert!(event.contains("restart_count=5"), "the event carries the NEW count; got {event:?}");
    assert!(event.contains("alloc-crashobs-0"), "and the alloc; got {event:?}");
    assert!(event.contains("crashobs"), "and the workload; got {event:?}");
    assert!(
        event.contains("prior_state=failed"),
        "and the terminal state it recovered from; got {event:?}",
    );
}

/// The event does NOT fire when no restart landed — a driver-rejected restart
/// against the same terminal prior. An alert that fires on a non-event is
/// noise, not signal.
///
/// Together with the test above this pins the `>` gate exactly: `>=` and `==`
/// both fire here (the counter is unchanged), and `<` fails to fire above.
#[tokio::test(flavor = "current_thread")]
async fn a_rejected_restart_emits_no_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let seed = seeded_failed_row(7, 4, None);
    let row = restart_against(seed, StartOutcome::Reject).await;
    assert_eq!(row.restart_count, 4, "precondition: nothing restarted, nothing counted");

    assert!(
        captured.restart_observed().is_empty(),
        "no restart landed — the event must stay silent; got {:?}",
        captured.restart_observed(),
    );
}

/// A forward-carry write (a `StopAllocation` against a `Running` prior that
/// already carries a non-zero count) emits nothing either. Kills the `>=`
/// mutant on a path where the counter is carried rather than incremented.
#[tokio::test(flavor = "current_thread")]
async fn a_forward_carry_write_emits_no_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let mut seed = seeded_failed_row(7, 4, None);
    seed.state = AllocState::Running;
    let row = dispatch_against_seed(
        seed,
        Action::StopAllocation { alloc_id: alloc_id(), terminal: None },
    )
    .await;
    assert_eq!(row.restart_count, 4, "precondition: the count was forwarded, not incremented");

    assert!(
        captured.restart_observed().is_empty(),
        "a forward-carry is not a restart — the event must stay silent; got {:?}",
        captured.restart_observed(),
    );
}

// ---------------------------------------------------------------------------
// BTR-1..3 — bounded lifecycle/network correction executable evidence.
//
// BTR-1/BTR-2 retain focused edge/error tables complementary to their
// registered seeded invariants. BTR-3's cross-port state machine moves to the
// socket-free Tier-1 lifecycle invariant; the integration fixture below keeps
// only real worker/listener/guard evidence.
// ---------------------------------------------------------------------------

/// S-GTI-BTR-01 / `@contract-shape:bounded-change` `@in-memory` `@error` —
/// after cleanup, a `StopAllocation` whose compound terminal proposal loses
/// LWW twice completes successfully after exactly two fresh-read proposals.
///
/// The activated test uses a test-owned [`ObservationStore`] decorator that
/// delegates reads to [`SimObservationStore`], returns `Ok(None)` for exactly
/// the first two terminal proposals, and rejects any third proposal at the
/// port boundary. It drives the real [`dispatch`] entry point and asserts the
/// competing current row remains authoritative, supervision is released once,
/// the [`overdrive_control_plane::action_shim::AllocDriverIndex`] route is
/// absent, and the lifecycle bus receives no fabricated occurrence. The same
/// table covers first-proposal acceptance, one rejection then acceptance, an
/// exact requested terminal found by a fresh read, and existing typed
/// read/write errors. No cancellation or replay partition belongs here.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn stop_allocation_second_lww_rejection_completes_without_event() {
    for outcome in [
        TerminalWriteOutcome::RejectedTwiceByConcurrentExit,
        TerminalWriteOutcome::Accepted,
        TerminalWriteOutcome::RejectedByConcurrentExit,
        TerminalWriteOutcome::ExactRequestedTerminal,
        TerminalWriteOutcome::Failed,
        TerminalWriteOutcome::PointReadFailed,
    ] {
        assert_terminal_write_partition(TerminalActionArm::StopAllocation, outcome).await;
    }
}

/// S-GTI-BTR-02 / `@contract-shape:bounded-change` `@in-memory` `@error` —
/// every provision error after successful slot assignment runs the existing
/// allocation-keyed structural teardown before the Failed disposition.
///
/// The activated table drives both `StartAllocation` and
/// `RestartAllocation` through [`dispatch_with_network_provisioner`]. A
/// test-owned [`WorkloadNetworkProvisioner`] records `provision -> teardown`
/// and fails provisioning after the production allocator has assigned a slot.
/// Successful teardown must release that slot; teardown failure must retain
/// it. In both partitions the durable row keeps the original
/// `WorkloadNetnsProvisionFailed` cause. A store-write failure keeps its
/// existing precedence over the captured teardown error. Slot exhaustion is
/// the pre-assignment complement and must make no teardown call.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn post_assignment_provision_failure_tears_down_before_slot_release() {
    for arm in [ProvisionFailureArm::Start, ProvisionFailureArm::Restart] {
        let outcome = drive_post_assignment_provision_failure(arm, false, false, false).await;

        outcome.result.expect("successful teardown and Failed write resolve the action");
        assert_eq!(
            outcome.trace,
            ["provision", "teardown", "failed-write"],
            "the Failed disposition commits only after the assigned slot's teardown"
        );
        assert!(!outcome.slot_held, "successful teardown releases the assigned slot");
        assert!(
            outcome.slot_released_at_failed_write,
            "the slot is absent at the Failed disposition boundary"
        );
        assert_eq!(outcome.driver_starts, 0, "the provision seam precedes driver start");
        let row = outcome.row.expect("the original provision cause is durably recorded");
        assert_eq!(row.state, AllocState::Failed);
        assert!(matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "netns_provision"
        ));
    }

    for arm in [ProvisionFailureArm::Start, ProvisionFailureArm::Restart] {
        let outcome = drive_post_assignment_provision_failure(arm, true, false, false).await;

        assert!(matches!(outcome.result, Err(ShimError::WorkloadNetnsProvision(_))));
        assert_eq!(outcome.trace, ["provision", "teardown", "failed-write"]);
        assert!(outcome.slot_held, "a failed teardown retains the assigned slot for recovery");
        assert!(
            !outcome.slot_released_at_failed_write,
            "a failed teardown remains visibly held at the Failed disposition boundary"
        );
        let row = outcome.row.expect("the original provision cause is still durable");
        assert!(matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "netns_provision"
        ));
    }

    let outcome =
        drive_post_assignment_provision_failure(ProvisionFailureArm::Start, true, true, false)
            .await;
    assert!(matches!(outcome.result, Err(ShimError::Observation(_))), "the store error wins");
    assert_eq!(outcome.trace, ["provision", "teardown", "failed-write"]);
    assert!(
        outcome.slot_held,
        "the failed teardown still retains its slot when recording also fails"
    );
    assert!(
        !outcome.slot_released_at_failed_write,
        "the failed teardown remains visibly held at the rejected write boundary"
    );
    assert!(outcome.row.is_none(), "the rejected Failed write leaves no fresh row");

    let outcome =
        drive_post_assignment_provision_failure(ProvisionFailureArm::Start, false, false, true)
            .await;
    outcome.result.expect("pre-assignment exhaustion still writes the existing Failed disposition");
    assert_eq!(
        outcome.trace,
        ["failed-write"],
        "slot exhaustion never calls provision or teardown"
    );
    assert!(!outcome.slot_held, "the target allocation never acquired a slot");
    assert!(
        outcome.slot_released_at_failed_write,
        "the pre-assignment complement is absent at its Failed disposition boundary"
    );
    let row = outcome.row.expect("slot exhaustion is durably classified");
    assert!(matches!(
        row.reason,
        Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
            if stage == "net_slot_assign"
    ));
}

#[cfg(feature = "integration-tests")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplacementPartition {
    MtlsError,
    Success,
}

#[cfg(feature = "integration-tests")]
struct TraceGuard(Arc<parking_lot::Mutex<Vec<&'static str>>>);

#[cfg(feature = "integration-tests")]
impl Drop for TraceGuard {
    fn drop(&mut self) {
        self.0.lock().push("mtls-rule-drop");
    }
}

#[cfg(feature = "integration-tests")]
impl InterceptGuard for TraceGuard {}

#[cfg(feature = "integration-tests")]
struct RecordingIntercept {
    listeners: parking_lot::Mutex<Vec<SocketAddrV4>>,
    trace: Arc<parking_lot::Mutex<Vec<&'static str>>>,
}

#[cfg(feature = "integration-tests")]
impl RecordingIntercept {
    fn inbound_addr(&self) -> SocketAddrV4 {
        self.listeners.lock()[1]
    }
}

#[cfg(feature = "integration-tests")]
impl MtlsIntercept for RecordingIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<std::net::TcpListener> {
        let listener = std::net::TcpListener::bind(addr)
            .map_err(|source| InterceptError::TransparentListener { addr, source })?;
        let local_addr = listener
            .local_addr()
            .map_err(|source| InterceptError::TransparentListener { addr, source })?;
        let std::net::SocketAddr::V4(local_addr) = local_addr else {
            unreachable!("the IPv4 bind request must return an IPv4 listener address")
        };
        self.listeners.lock().push(local_addr);
        Ok(listener)
    }

    fn install_outbound(
        &self,
        _host_veth: &str,
        _agent_leg_f_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        Ok(Box::new(TraceGuard(Arc::clone(&self.trace))))
    }

    fn install_inbound(
        &self,
        _virt: SocketAddrV4,
        _agent_leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        Ok(Box::new(TraceGuard(Arc::clone(&self.trace))))
    }
}

#[cfg(feature = "integration-tests")]
struct GatedReplacementEnforcement {
    trace: Arc<parking_lot::Mutex<Vec<&'static str>>>,
    enforced: tokio::sync::Notify,
    stop_entered: tokio::sync::Notify,
    stop_release: tokio::sync::Notify,
    block_first_stop: AtomicBool,
    fail_first_stop: AtomicBool,
}

#[cfg(feature = "integration-tests")]
#[async_trait::async_trait]
impl MtlsEnforcement for GatedReplacementEnforcement {
    async fn probe(&self) -> Result<(), MtlsEnforcementError> {
        Ok(())
    }

    async fn enforce(
        &self,
        connection: InterceptedConnection,
    ) -> Result<EnforcedConnection, MtlsEnforcementError> {
        drop(connection.leg);
        self.enforced.notify_one();
        Ok(EnforcedConnection::new(EnforcedConnectionId::new(connection.alloc, 0)))
    }

    fn liveness(&self, _handle: &EnforcedConnection) -> PumpLiveness {
        PumpLiveness::Running
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<(), MtlsEnforcementError> {
        self.stop_entered.notify_one();
        if self.block_first_stop.swap(false, Ordering::SeqCst) {
            self.stop_release.notified().await;
        }
        self.trace.lock().push("mtls-stop-complete");
        if self.fail_first_stop.swap(false, Ordering::SeqCst) {
            return Err(MtlsEnforcementError::TeardownFailed {
                id: handle.id().clone(),
                source: std::io::Error::other("injected prior mTLS teardown failure"),
            });
        }
        Ok(())
    }
}

#[cfg(feature = "integration-tests")]
struct ReplacementDriver {
    driver_type: DriverType,
}

#[cfg(feature = "integration-tests")]
#[async_trait::async_trait]
impl Driver for ReplacementDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(42) })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
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

#[cfg(feature = "integration-tests")]
struct ReplacementNetwork;

#[cfg(feature = "integration-tests")]
impl WorkloadNetworkProvisioner for ReplacementNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        Ok(())
    }
}

#[cfg(feature = "integration-tests")]
struct ReplacementOutcome {
    result: Result<(), ShimError>,
    trace: Vec<&'static str>,
}

#[cfg(feature = "integration-tests")]
#[allow(
    clippy::too_many_lines,
    reason = "the real worker/listener fixture needs the complete action-shim composition"
)]
async fn drive_same_id_replacement(partition: ReplacementPartition) -> ReplacementOutcome {
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let alloc = alloc_id();
    let mut prior = seeded_failed_row(0, 0, None);
    prior.state = AllocState::Running;
    prior.reason = Some(TransitionReason::Started);
    prior.terminal = None;
    obs.write_alloc_lifecycle(prior, TransitionSource::Reconciler)
        .await
        .expect("seed running prior");

    let trace = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let enforcement = Arc::new(GatedReplacementEnforcement {
        trace: Arc::clone(&trace),
        enforced: tokio::sync::Notify::new(),
        stop_entered: tokio::sync::Notify::new(),
        stop_release: tokio::sync::Notify::new(),
        block_first_stop: AtomicBool::new(true),
        fail_first_stop: AtomicBool::new(partition == ReplacementPartition::MtlsError),
    });
    let intercept = Arc::new(RecordingIntercept {
        listeners: parking_lot::Mutex::new(Vec::new()),
        trace: Arc::clone(&trace),
    });
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    let worker = Arc::new(MtlsInterceptWorker::new(
        Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
        resolve,
        Arc::new(overdrive_sim::adapters::clock::SimClock::new()),
        Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
    ));
    let mut prior_spec = spec();
    prior_spec.host_veth = Some("ovd-hv-prior".to_owned());
    worker.start_alloc(&prior_spec).await.expect("prior interception installs");
    let _prior_connection =
        std::net::TcpStream::connect(intercept.inbound_addr()).expect("connect prior inbound leg");
    tokio::time::timeout(Duration::from_secs(2), enforcement.enforced.notified())
        .await
        .expect("prior mTLS connection reaches the existing enforcement port");

    let net_slots = NetSlotAllocator::new();
    let prior_slot = NetSlot::new(7).expect("valid non-minimal prior slot");
    net_slots
        .adopt(alloc.clone(), prior_slot)
        .expect("prior allocation owns a non-minimal structural slot");
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        for driver_type in [DriverType::Exec, DriverType::Vm] {
            registry.insert(Arc::new(ReplacementDriver { driver_type }));
        }
        registry
    };
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let network = ReplacementNetwork;
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let clock = overdrive_sim::adapters::clock::SimClock::new();
    let writer_node = NodeId::new("writer-1").expect("writer node");
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let host = overdrive_sim::adapters::vm_host_state::SimVmHostState::new();
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 1,
        deadline: now + Duration::from_secs(2),
    };
    let mtls_lifecycle = (&worker) as &dyn MtlsInterceptLifecycle;
    let result = dispatch_with_network_provisioner(
        vec![Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: spec(),
            kind: WorkloadKind::Service,
        }],
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        &dataplane,
        &ca,
        &clock,
        &identity,
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
            VipRange::default(),
            store,
        ))),
        &broker,
        None,
        Some(mtls_lifecycle),
        &net_slots,
        &network,
        &host,
    );
    tokio::pin!(result);

    let release_prior_mtls_stop = async {
        tokio::time::timeout(Duration::from_secs(2), enforcement.stop_entered.notified())
            .await
            .expect("same-id replacement reaches the prior mTLS stop");
        assert!(
            std::net::TcpStream::connect(intercept.inbound_addr()).is_err(),
            "the real prior listener must be closed before connection teardown completes"
        );
        enforcement.stop_release.notify_one();
    };
    tokio::pin!(release_prior_mtls_stop);
    let result = tokio::select! {
        result = &mut result => panic!("same-id replacement completed before prior mTLS teardown was released: {result:?}"),
        () = &mut release_prior_mtls_stop => result.await,
    };
    ReplacementOutcome { result, trace: trace.lock().clone() }
}

#[cfg(feature = "integration-tests")]
fn assert_real_worker_stop_trace(trace: &[&'static str]) {
    assert_eq!(
        trace,
        ["mtls-rule-drop", "mtls-stop-complete"],
        "the worker must drop its real-listener task guard before the awaited enforcement teardown returns"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-GTI-BTR-03 integration-lane complement. This test owns only the facts a
/// socket-free lifecycle Sim cannot observe: a real loopback listener closes,
/// and the worker-held intercept guard drops before the awaited stop returns.
/// Cross-port driver/network/identity ordering, retry convergence, and slot
/// ownership belong exclusively to the seeded Tier-1 lifecycle invariant.
#[cfg(feature = "integration-tests")]
#[tokio::test]
async fn same_id_restart_real_worker_closes_prior_listener_and_drops_guard_before_stop_completion()
{
    let mtls_error = drive_same_id_replacement(ReplacementPartition::MtlsError).await;
    assert!(matches!(mtls_error.result, Err(ShimError::MtlsStop(_))));
    assert_real_worker_stop_trace(&mtls_error.trace);

    let success = drive_same_id_replacement(ReplacementPartition::Success).await;
    success.result.expect("successful real-worker teardown permits replacement");
    assert_real_worker_stop_trace(&success.trace);
}
