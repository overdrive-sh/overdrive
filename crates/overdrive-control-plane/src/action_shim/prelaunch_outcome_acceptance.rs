//! Launch outcomes that precede the driver: every one writes a row, or is a
//! named no-row outcome (ADR-0337, ADR-0338, ADR-0334 item 5; feature delta
//! § "Launch outcomes: every one writes a row or is a named no-row outcome").
//!
//! # Scenario family
//!
//! The action shim's two launch arms (`StartAllocation`, `RestartAllocation`)
//! fail before the driver runs in several ways. A failure that writes no row is
//! invisible to the operator and, because the reconciler re-issues an id on
//! every tick, is never charged. This file pins, per path, which outcome each
//! arm has:
//!
//! * superseded launch (N-SUP): no row, lease, SVID and supervision claim
//!   released, `launch.superseded` event, `Ok(())`; in the restart arm the
//!   exact-old predecessor cleanup still runs;
//! * `ensure_intercept_identity` failure (S5, R7): a `Failed` row
//!   `MtlsInterceptInstallFailed { stage: "identity_issue", detail }`;
//! * empty driver registry in a restart (R3): a `Failed` successor row;
//! * admission refusal (N-ADM, S2, R4): no row, no charge, the wait predicate
//!   true afterwards, the reconciler paced by the pool's occupancy;
//! * unchanged rows (S3, S6) and the read-side failures (S0, R0).
//! * early returns that stay as they are: a finalised Job row (S1), a stale
//!   restart predecessor (R2), a restart's guest-network failure (R6), a
//!   restart's duplicate owner (R9), activation withheld at the EXEC gate's
//!   fail-stop (S10, R14), and a launch that already won its boot race before
//!   the stop's signal.
//!
//! # Contract shape and owner path
//!
//! Every test drives the shim through its pinned driving port
//! (`dispatch_with_guest_network_provisioner_for_test`) or through the real
//! `WorkloadLifecycle` tick (`run_convergence_tick_with_guest_network_
//! provisioner_for_test`), over the production `GuestAddressPool`, the
//! production `IdentityMgr`, the production `MtlsInterceptWorker` on sim ports,
//! and `SimObservationStore`. Faults enter only at driven ports: a `Ca` double
//! whose `issue_svid` fails, a `Driver` double whose `start` returns
//! `StartSuperseded`, `SimDriver` scripts, the test owner's provision and
//! teardown refusals, an observation-store wrapper whose point read fails, and
//! the same wrapper's point-read hook, which takes the pool's last slot between
//! the reconciler's hydration and the shim's admission (another target's
//! concurrent admission, the lost race ADR-0338 names).
//!
//! The `Failed` predecessor rows of the restart-arm refusal test and the
//! teardown scenarios are written with the same shape the exit observer writes
//! for a crash (the precedent of `admission_refusal_acceptance`); the rest of
//! the rows are authored by production.
//!
//! # Oracle
//!
//! The dispatch result; the allocation rows and lifecycle occurrences; the
//! lifecycle bus; the owner's journal; the pool's `snapshot` and `observe`;
//! `IdentityMgr::held_snapshot`; the driver doubles' own records; the pinned
//! tracing events; and the reconciler's `WorkloadLifecycleView`.
//!
//! # Tier and placement
//!
//! Tier 1, in-process over `Sim*` and test doubles, default lane for the
//! integration feature set. `host_pure`: no node-global kernel state.
//!
//! # Pending versus guard
//!
//! A test of behaviour DELIVER has not built is pending: it keeps its full body
//! and is marked `#[should_panic(expected = ...)]` with the message of the
//! assertion in its own body that fails today. A test that passes today is a
//! guard and says so.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "acceptance bodies fail at once on a broken precondition, naming it"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::issue_failure::SvidIssueFailure;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_network::{
    GuestNetworkExecSupervisor, GuestNetworkExecWiring, MAX_GUEST_NETWORK_ATTACHMENTS,
    SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_core::guest_transport::{GuestCid, GuestTransportAssignment};
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, SpiffeId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, PublicCertifiedKeyId, PublicCertifiedKeyStatusRowV1,
};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverRegistry, DriverStartClass, DriverStartFailure, DriverType, LaunchSupersession,
    Resources, VmPayload, VmStartFailure,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, LagAwareSubscription,
    LogicalTimestamp, NodeHealthRow, ObservationStore, ObservationStoreError, ObservationWrite,
    ServiceBackendRow, TransitionSource, WorkloadSvidIssueStatusRowV1,
};
use overdrive_core::traits::vm_host_state::VmHostState as _;
use overdrive_core::traits::vmm::{VMM_REAP_BOUND, VmTermination, Vmm};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::vm::config::{
    Gid, HostArch, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfig, VmConfinement,
    VmRunDir, VmmIdentity,
};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_sim::SimVmm;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
use overdrive_store_local::LocalIntentStore;
use overdrive_testing::placement;
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use tempfile::TempDir;
use tokio::sync::broadcast::{Receiver, error::TryRecvError};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::{LifecycleEvent, ShimError, dispatch_with_guest_network_provisioner_for_test};
use crate::guest_network::{GuestAddressPool, GuestNetworkError, SharedGuestNetworkOwner};
use crate::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use crate::shared_network_test_ports::{TestOwnerCall, TestSharedOwner};

placement!(host_pure);

/// The cause text the failing CA double carries; the row's `detail` must carry it.
const CA_CAUSE: &str = "prelaunch-ca-refused-to-sign";
/// The text the failing observation lifecycle write carries.
const WRITE_CAUSE: &str = "prelaunch-lifecycle-write-refused";
/// The text the failing observation point read carries.
const READ_CAUSE: &str = "prelaunch-point-read-refused";

// ---------------------------------------------------------------------------
// Driven-port doubles.
// ---------------------------------------------------------------------------

/// `Ca` double: delegates to `SimCa`; `issue_svid` fails while armed.
struct FailingCa {
    inner: SimCa,
    armed: AtomicBool,
    attempts: AtomicUsize,
}

impl FailingCa {
    fn new() -> Self {
        Self {
            inner: SimCa::new(Arc::new(SimEntropy::new(328))),
            armed: AtomicBool::new(false),
            attempts: AtomicUsize::new(0),
        }
    }

    fn arm(&self, armed: bool) {
        self.armed.store(armed, Ordering::SeqCst);
    }
}

impl Ca for FailingCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.inner.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.inner.issue_intermediate(node)
    }

    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.armed.load(Ordering::SeqCst) {
            return Err(CaError::SigningFailed { reason: CA_CAUSE.to_owned() });
        }
        self.inner.issue_svid(req)
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.inner.trust_bundle()
    }
}

/// What the store and the spec held when a launch reached the driver.
#[derive(Clone)]
struct SeenAtStart {
    row: Option<AllocStatusRow>,
    address: Option<Ipv4Addr>,
}

/// The typed outcome the scripted `start` returns.
#[derive(Clone, Copy)]
enum ScriptedStart {
    /// `StartSuperseded`.
    Superseded,
    /// `StartRejected` carrying `AllocationAlreadyOwned` for the started allocation.
    DuplicateOwner,
    /// A non-start `DriverError` (`GuestTransportMissing`, the composition error
    /// `VmDriver::start` can return besides `StartRejected`).
    OtherError,
}

/// `Driver` double for a scripted launch outcome. By default `start` returns
/// `StartSuperseded` (the typed outcome a driver returns after it observed the
/// lane signal; a test cannot raise the lane signal through the runtime today
/// because the lane table is DELIVER's), and every effect the shim asks of a
/// driver is recorded.
struct ScriptedStartDriver {
    outcome: ScriptedStart,
    /// The observation store the shim writes to, attached by the test after
    /// the rig is built; `start` reads the allocation's row from it, which is
    /// what the store holds at the instant the launch reaches the driver.
    obs: Mutex<Option<Arc<dyn ObservationStore>>>,
    /// What each `start` saw: the allocation's row in the store and the
    /// guest address of the transport the shim injected into the spec.
    seen_at_start: Mutex<Vec<SeenAtStart>>,
    starts: Mutex<Vec<AllocationId>>,
    stops: Mutex<Vec<AllocationId>>,
    supervision_releases: Mutex<Vec<AllocationId>>,
}

impl ScriptedStartDriver {
    fn new() -> Self {
        Self::scripted(ScriptedStart::Superseded)
    }

    /// Attach the observation store `start` reads the allocation's row from.
    fn attach_obs(&self, obs: Arc<dyn ObservationStore>) {
        *self.obs.lock() = Some(obs);
    }

    fn duplicate_owner() -> Self {
        Self::scripted(ScriptedStart::DuplicateOwner)
    }

    fn other_error() -> Self {
        Self::scripted(ScriptedStart::OtherError)
    }

    fn scripted(outcome: ScriptedStart) -> Self {
        Self {
            outcome,
            obs: Mutex::new(None),
            seen_at_start: Mutex::new(Vec::new()),
            starts: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            supervision_releases: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Driver for ScriptedStartDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(
        &self,
        spec: &AllocationSpec,
        _supersession: &dyn LaunchSupersession,
    ) -> Result<AllocationHandle, DriverError> {
        self.starts.lock().push(spec.alloc.clone());
        let obs = self.obs.lock().clone();
        let row = match obs {
            Some(obs) => obs.alloc_status_row(&spec.alloc).await.expect("row readable at start"),
            None => None,
        };
        self.seen_at_start.lock().push(SeenAtStart {
            row,
            address: spec.transport.as_ref().map(GuestTransportAssignment::workload_addr),
        });
        match self.outcome {
            ScriptedStart::Superseded => Err(DriverError::StartSuperseded),
            ScriptedStart::OtherError => {
                Err(DriverError::GuestTransportMissing { alloc: spec.alloc.clone() })
            }
            ScriptedStart::DuplicateOwner => Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned {
                        alloc: spec.alloc.clone(),
                    }),
                    detail: "allocation already has an active VM start or supervisor".to_owned(),
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

    fn release_supervision(&self, alloc: &AllocationId) {
        self.supervision_releases.lock().push(alloc.clone());
    }
}

/// Delegating `ObservationStore` over `SimObservationStore` with two faults,
/// both at the driven port: the allocation point read can fail, and a one-shot
/// hook runs on the next point read (the shim's own `find_prior_alloc_row`, the
/// first thing a launch arm does after the reconciler's hydration and View
/// write). The hook is how a test models another target's admission taking the
/// pool's last slot between hydration and admission.
struct FaultObs {
    inner: Arc<SimObservationStore>,
    fail_point_reads: AtomicBool,
    /// The next lifecycle write of a row in this state fails once, as an
    /// unreachable observation store.
    fail_next_write_of: Mutex<Option<AllocState>>,
    /// Every lifecycle row written through this store, in order. A launch's
    /// `Pending` row is overwritten by its ending row, so this is how a test
    /// reads the address the `Pending` row carried.
    written: Mutex<Vec<AllocStatusRow>>,
    point_read_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl FaultObs {
    fn over(inner: Arc<SimObservationStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            fail_point_reads: AtomicBool::new(false),
            fail_next_write_of: Mutex::new(None),
            written: Mutex::new(Vec::new()),
            point_read_hook: Mutex::new(None),
        })
    }
}

#[async_trait]
impl ObservationStore for FaultObs {
    async fn public_certified_key_status_row(
        &self,
        id: &PublicCertifiedKeyId,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, ObservationStoreError> {
        self.inner.public_certified_key_status_row(id).await
    }

    async fn gateway_application_status_row(
        &self,
        node_id: &NodeId,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, ObservationStoreError> {
        self.inner.gateway_application_status_row(node_id).await
    }

    async fn record_workload_svid_issue_failure(
        &self,
        alloc_id: &AllocationId,
        node_id: &NodeId,
        failure: SvidIssueFailure,
    ) -> Result<(), ObservationStoreError> {
        self.inner.record_workload_svid_issue_failure(alloc_id, node_id, failure).await
    }

    async fn workload_svid_issue_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<WorkloadSvidIssueStatusRowV1>, ObservationStoreError> {
        self.inner.workload_svid_issue_status_row(alloc_id).await
    }

    async fn write(&self, row: ObservationWrite) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        let armed = {
            let mut guard = self.fail_next_write_of.lock();
            if *guard == Some(current.state) { guard.take() } else { None }
        };
        if armed.is_some() {
            return Err(ObservationStoreError::Unreachable { peer: WRITE_CAUSE.to_owned() });
        }
        self.written.lock().push(current.clone());
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
        if self.fail_point_reads.load(Ordering::SeqCst) {
            return Err(ObservationStoreError::Unreachable { peer: READ_CAUSE.to_owned() });
        }
        let hook = self.point_read_hook.lock().take();
        if let Some(hook) = hook {
            hook();
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

// ---------------------------------------------------------------------------
// Pinned tracing events.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RenderedFields(String);

impl Visit for RenderedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

struct CapturedEvent {
    name: &'static str,
    fields: String,
}

struct EventCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = RenderedFields::default();
        event.record(&mut fields);
        self.events.lock().push(CapturedEvent { name: event.metadata().name(), fields: fields.0 });
    }
}

// ---------------------------------------------------------------------------
// The fixture.
// ---------------------------------------------------------------------------

const WORKLOAD: &str = "prelaunch-service";
const NODE: &str = "prelaunch-node";

fn node_id() -> NodeId {
    NodeId::new(NODE).expect("node id")
}

fn workload_id() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("workload id")
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("allocation id")
}

fn reconciler_name() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("reconciler name")
}

fn target() -> TargetResource {
    TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target")
}

fn spec(name: &str) -> AllocationSpec {
    let alloc_id = alloc(name);
    AllocationSpec {
        identity: SpiffeId::for_workload(&workload_id()),
        alloc: alloc_id,
        driver: DriverPayload::Vm(VmPayload {
            volume: None,
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/prelaunch/kernel"),
            rootfs: PathBuf::from("/prelaunch/rootfs.ext4"),
        }),
        resources: Resources {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(64 * 1024 * 1024),
        },
        probe_descriptors: Vec::new(),
        transport: None,
        declared_listeners: vec![overdrive_core::aggregate::Listener {
            protocol: overdrive_core::dataplane::Proto::Tcp,
            port: NonZeroU16::new(8080).expect("non-zero listener port"),
        }],
    }
}

fn start_action(name: &str, kind: WorkloadKind) -> Action {
    Action::StartAllocation {
        alloc_id: alloc(name),
        workload_id: workload_id(),
        node_id: node_id(),
        spec: spec(name),
        kind,
    }
}

fn restart_action(predecessor: &str, successor: &str) -> Action {
    Action::RestartAllocation {
        alloc_id: alloc(predecessor),
        spec: spec(successor),
        kind: WorkloadKind::Service,
    }
}

fn tick_context() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 1,
        deadline: now + Duration::from_secs(1),
    }
}

struct Rig {
    state: crate::AppState,
    owner: Arc<TestSharedOwner>,
    pool: Arc<GuestAddressPool>,
    ca: Arc<FailingCa>,
    obs: Arc<FaultObs>,
    clock: Arc<SimClock>,
    exec: Arc<GuestNetworkExecSupervisor>,
    base: Instant,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    _events_guard: tracing::subscriber::DefaultGuard,
    _tmp: TempDir,
}

impl Rig {
    async fn build(driver: Arc<dyn Driver>) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let clock = Arc::new(SimClock::new());
        let owner = Arc::new(TestSharedOwner::new());
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        let pool = crate::shared_network_test_ports::guest_pool();
        let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
        let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
            identity,
            MtlsLimits::default(),
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
        ));
        let resolve: Arc<dyn MtlsResolve> =
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
        let worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimMtlsIntercept::new()) as Arc<dyn MtlsIntercept>,
        ));
        worker.start_shared_owner().await.expect("sim shared mTLS owner starts");
        owner.converge_shared().await.expect("the fixture owner converges before AppState");
        let exec = wiring.supervisor();
        assert!(exec.open_after_boot(), "the fixture EXEC gate opens once");

        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
            .expect("reconciler runtime");
        runtime.register(crate::noop_heartbeat()).await.expect("register heartbeat");
        runtime.register(crate::workload_lifecycle()).await.expect("register workload-lifecycle");
        runtime.register(crate::vm_reclamation()).await.expect("register vm-reclamation");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let obs = FaultObs::over(Arc::new(SimObservationStore::single_peer(node_id(), 0)));
        let ca = Arc::new(FailingCa::new());
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let state = crate::AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            driver,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::clone(&ca) as Arc<dyn Ca>,
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node_id(),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            wiring.gate(),
            Arc::clone(&pool),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(EventCapture { events: Arc::clone(&events) }),
        );
        Self {
            state,
            owner,
            pool,
            ca,
            obs,
            clock,
            exec,
            base: Instant::now(),
            events,
            _events_guard: events_guard,
            _tmp: tmp,
        }
    }

    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            &self.state,
            &tick_context(),
            self.owner.as_ref(),
        ))
        .await
    }

    /// One real `WorkloadLifecycle` tick over the production runtime.
    async fn tick(&self, n: u64) -> Result<(), ConvergenceError> {
        run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &reconciler_name(),
            &target(),
            self.base + Duration::from_millis(n * 100),
            n,
            self.base + Duration::from_secs(120),
            self.owner.as_ref(),
        )
        .await
    }

    /// Advance the injected wall clock past the restart backoff window.
    fn elapse_backoff(&self) {
        self.clock.tick(Duration::from_secs(5));
    }

    async fn row(&self, name: &str) -> Option<AllocStatusRow> {
        self.state.obs.alloc_status_row(&alloc(name)).await.expect("row readable")
    }

    async fn rows(&self) -> Vec<AllocStatusRow> {
        self.state.obs.alloc_status_rows().await.expect("rows readable")
    }

    /// The address the allocation's `Pending` row carried (ADR-0353): the
    /// lease's address, read from the rows written through the store because
    /// the ending row overwrites the `Pending` row. Panics when no `Pending`
    /// row with an address was written.
    fn pending_address(&self, name: &str) -> Ipv4Addr {
        let wanted = alloc(name);
        self.obs
            .written
            .lock()
            .iter()
            .find(|row| row.alloc_id == wanted && row.state == AllocState::Pending)
            .and_then(|row| row.workload_addr)
            .unwrap_or_else(|| panic!("precondition: {name} wrote a Pending row with an address"))
    }

    fn has_lease(&self, name: &str) -> bool {
        self.pool.snapshot().contains_key(&alloc(name))
    }

    fn held_svid(&self, name: &str) -> bool {
        self.state.identity.held_snapshot().contains_key(&alloc(name))
    }

    fn teardowns_of(&self, name: &str) -> usize {
        let wanted = alloc(name);
        self.owner
            .journal()
            .iter()
            .filter(|entry| {
                matches!(&entry.call, TestOwnerCall::Teardown { alloc, .. } if *alloc == wanted)
            })
            .count()
    }

    fn view(&self) -> overdrive_reconcilers::WorkloadLifecycleView {
        self.state.runtime.view_for_workload_lifecycle(&target())
    }

    fn event_names(&self) -> Vec<&'static str> {
        self.events.lock().iter().map(|event| event.name).collect()
    }

    /// Take `count` slots of the pool through its own admission.
    fn fill_pool(&self, prefix: &str, count: u32) {
        for index in 0..count {
            self.pool
                .assign(
                    alloc(&format!("{prefix}-{index:05}")),
                    &[],
                    crate::shared_network_test_ports::test_owner_flow_ports(),
                )
                .expect("below the cap every assignment succeeds");
        }
    }

    /// The Failed row a crash of `name` leaves, in the exact shape the exit
    /// observer writes, and the lease the allocation held while it ran.
    #[cfg(test)]
    async fn seed_crashed_predecessor(&self, name: &str, with_lease: bool) {
        let row = AllocStatusRow {
            alloc_id: alloc(name),
            workload_id: workload_id(),
            node_id: node_id(),
            state: AllocState::Failed,
            updated_at: LogicalTimestamp { counter: 0, writer: node_id() },
            reason: Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(17),
                signal: None,
                stderr_tail: None,
            }),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        };
        self.state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("seed the crashed predecessor row");
        if with_lease {
            self.pool
                .assign(alloc(name), &[], crate::shared_network_test_ports::test_owner_flow_ports())
                .expect("the predecessor's lease is admitted");
        }
    }

    /// A row of `name` in the given shape, written as the observation store
    /// would hold it after an earlier dispatch.
    #[cfg(test)]
    async fn seed_row(
        &self,
        name: &str,
        state: AllocState,
        kind: WorkloadKind,
        terminal: Option<TerminalCondition>,
    ) {
        let row = AllocStatusRow {
            alloc_id: alloc(name),
            workload_id: workload_id(),
            node_id: node_id(),
            state,
            updated_at: LogicalTimestamp { counter: 0, writer: node_id() },
            reason: None,
            detail: None,
            terminal,
            stderr_tail: None,
            kind,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        };
        self.state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("seed the row");
    }

    /// The `Pending` row of a launch whose owner already holds a lease: the
    /// state an owner claim has in production (the lease is assigned, its
    /// address is on the row). Returns the row as written.
    #[cfg(test)]
    async fn seed_pending_owner(&self, name: &str) -> AllocStatusRow {
        let plan = self
            .pool
            .assign(alloc(name), &[], crate::shared_network_test_ports::test_owner_flow_ports())
            .expect("the owner's lease is admitted");
        let row = AllocStatusRow {
            alloc_id: alloc(name),
            workload_id: workload_id(),
            node_id: node_id(),
            state: AllocState::Pending,
            updated_at: LogicalTimestamp { counter: 1, writer: node_id() },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: Vec::new(),
            started_at: None,
            workload_addr: Some(plan.assignment().workload_addr()),
            last_terminated: None,
            restart_count: 0,
        };
        self.state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("seed the owner's Pending row");
        self.row(name).await.expect("the owner's Pending row is stored")
    }

    async fn put_service(&self) {
        let service = Service::from_submit(ServiceSpecInput {
            id: WORKLOAD.to_owned(),
            replicas: 1,
            resources: ResourcesInput {
                cpu: CpuQuantity::from_millis(100),
                memory: ByteSize::new(128 * 1024 * 1024),
            },
            driver: DriverInput::Vm(VmInput {
                volume: None,
                command: "/bin/serve".to_owned(),
                args: Vec::new(),
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .expect("valid service spec");
        let id = service.id.clone();
        let archived =
            WorkloadIntent::Service(service).archive_for_store().expect("archive the service");
        self.state
            .store
            .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
            .await
            .expect("put the service intent");
        self.state
            .store
            .put(
                IntentKey::for_workload_kind(&id).as_bytes(),
                &[WorkloadKind::Service.discriminator_byte()],
            )
            .await
            .expect("put the workload kind");
    }
}

/// Unwrap the identity-issue row's reason, naming the shape when it differs.
fn identity_issue_detail(row: &AllocStatusRow) -> String {
    match &row.reason {
        Some(TransitionReason::MtlsInterceptInstallFailed { stage, detail })
            if stage == "identity_issue" =>
        {
            detail.clone()
        }
        other => panic!("expected MtlsInterceptInstallFailed identity_issue, got {other:?}"),
    }
}

fn drain_lifecycle(lifecycle: &mut Receiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut published = Vec::new();
    loop {
        match lifecycle.try_recv() {
            Ok(event) => published.push(event),
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            Err(TryRecvError::Lagged(missed)) => {
                panic!("the lifecycle bus lagged by {missed}: its events cannot be checked")
            }
        }
    }
    published
}

fn is_admission_refusal(result: &Result<(), ConvergenceError>) -> bool {
    matches!(
        result,
        Err(ConvergenceError::Shim(ShimError::GuestNetwork(
            GuestNetworkError::AdmissionCapReached { .. }
        )))
    )
}

// ---------------------------------------------------------------------------
// N-SUP: a superseded launch.
// ---------------------------------------------------------------------------

/// The reason text an abandoned launch's row must present (ADR-0355). The
/// `LaunchSuperseded` variant does not exist yet, so the tests read the row's
/// reason through `TransitionReason::human_readable`, whose wording the design
/// pins.
fn reason_text(row: &AllocStatusRow) -> String {
    row.reason.as_ref().map_or_else(|| "<no reason>".to_owned(), TransitionReason::human_readable)
}

/// Assert `row` is an abandoned-launch row of `address` (ADR-0355 section 1):
/// `Terminated`, `terminal: None`, never started, the Pending row's address and
/// count facts carried, and the wording of `wanted` (`superseded` or `lost`).
fn assert_abandoned_launch_row(row: &AllocStatusRow, address: Option<Ipv4Addr>, wanted: &str) {
    assert_eq!(
        row.state,
        AllocState::Terminated,
        "an abandoned launch ends its Pending row as Terminated, never Failed; got {:?} ({})",
        row.state,
        reason_text(row)
    );
    assert!(
        reason_text(row).contains(&format!("launch abandoned: {wanted}")),
        "the abandoned row names why: expected `launch abandoned: {wanted}`, got `{}`",
        reason_text(row)
    );
    assert_eq!(row.terminal, None, "the workload's intent still stands: no terminal claim");
    assert_eq!(row.started_at, None, "the launch never ran");
    assert!(address.is_some(), "precondition: the launch had a guest address");
    assert_eq!(row.workload_addr, address, "the abandoned row carries the lease's address");
    assert_eq!(row.restart_count, 0, "the Pending row's restart count is carried, not advanced");
    assert_eq!(row.last_terminated, None, "no crash is recorded for an abandoned launch");
}

/// N-SUP, W1, ADR-0355.
///
/// CONTRACT_SHAPE: bounded-change. A `StartAllocation` whose launch returns
/// `StartSuperseded` ends its `Pending` row with one `Terminated` row
/// `LaunchSuperseded` (terminal `None`, the lease's address carried), releases
/// the lease, the SVID and the supervision claim, emits `launch.superseded`,
/// and returns `Ok(())`. Universe: the allocation's rows, lifecycle events, the
/// lease, the held SVID, the driver's supervision releases.
#[tokio::test]
async fn start_arm_superseded_launch_ends_its_row_as_abandoned_and_releases_lease_svid_and_claim() {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(
        result.is_ok(),
        "a superseded launch is a clean outcome: the start dispatch returns Ok(()), got {result:?}"
    );
    assert_eq!(driver.starts.lock().as_slice(), [alloc(id)], "the launch reached the driver once");
    let address = driver.seen_at_start.lock().first().and_then(|seen| seen.address);
    let row =
        rig.row(id).await.expect("a superseded launch ends its Pending row with an abandoned row");
    assert_abandoned_launch_row(&row, address, "superseded");
    assert!(!rig.has_lease(id), "the launch's guest-network lease is released");
    assert_eq!(rig.teardowns_of(id), 1, "the launch's network is torn down once");
    assert!(!rig.held_svid(id), "the SVID the shim minted before the launch is dropped");
    assert_eq!(
        driver.supervision_releases.lock().as_slice(),
        [alloc(id)],
        "the launch's supervision claim is released"
    );
    assert!(rig.event_names().contains(&"launch.superseded"), "one launch.superseded event");
    assert!(
        rig.events
            .lock()
            .iter()
            .filter(|event| event.name == "launch.superseded")
            .all(|event| event.fields.contains(id) && event.fields.contains(WORKLOAD)),
        "the event names the allocation and the workload target"
    );
    assert!(driver.stops.lock().is_empty(), "nothing started, so nothing is stopped");
}

/// N-SUP, W2, ADR-0355.
///
/// CONTRACT_SHAPE: bounded-change. A `RestartAllocation` whose successor launch
/// returns `StartSuperseded` ends the successor's `Pending` row as abandoned
/// (`LaunchSuperseded`, address carried), releases the successor's lease, SVID
/// and supervision claim, leaves the predecessor's row unchanged, and still
/// runs the exact-old predecessor cleanup once.
#[tokio::test]
async fn restart_arm_superseded_successor_ends_its_row_as_abandoned_and_runs_exact_old_predecessor_cleanup()
 {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    let predecessor_row_before = rig.row(predecessor).await;

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(
        result.is_ok(),
        "a superseded launch is a clean outcome: the restart dispatch returns Ok(()), got {result:?}"
    );
    let address = driver.seen_at_start.lock().first().and_then(|seen| seen.address);
    let row = rig
        .row(successor)
        .await
        .expect("a superseded successor ends its Pending row with an abandoned row");
    assert_abandoned_launch_row(&row, address, "superseded");
    assert_eq!(
        rig.row(predecessor).await,
        predecessor_row_before,
        "the predecessor row is unchanged"
    );
    assert!(!rig.has_lease(successor), "the successor's lease is released");
    assert_eq!(rig.teardowns_of(successor), 1, "the successor's network is torn down once");
    assert!(!rig.held_svid(successor), "the successor's SVID is dropped");
    assert_eq!(
        driver.supervision_releases.lock().as_slice(),
        [alloc(successor)],
        "the successor's supervision claim is released"
    );
    assert_eq!(
        driver.stops.lock().as_slice(),
        [alloc(predecessor)],
        "the exact-old predecessor cleanup stops the predecessor once"
    );
    assert_eq!(rig.teardowns_of(predecessor), 1, "the predecessor's network is torn down once");
    assert!(!rig.has_lease(predecessor), "the predecessor's lease is released");
    assert!(rig.event_names().contains(&"launch.superseded"), "one launch.superseded event");
}

/// N-SUP, W1, ADR-0355 section 4. A failing lease teardown does not
/// suppress the abandoned row: the row is written, then the teardown error is
/// returned.
#[tokio::test]
async fn start_arm_teardown_failure_does_not_suppress_the_abandoned_row() {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    rig.owner.script_teardown_failure(true);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    let address = driver.seen_at_start.lock().first().and_then(|seen| seen.address);
    let row =
        rig.row(id).await.expect("the abandoned row is written even when the lease teardown fails");
    assert_abandoned_launch_row(&row, address, "superseded");
    assert!(
        matches!(result, Err(ShimError::GuestNetwork(_))),
        "the teardown failure is returned after the row: {result:?}"
    );
    assert_eq!(rig.teardowns_of(id), 1, "the teardown was attempted once");
}

/// N-SUP, N-REC, ADR-0355 section 4. A failing write of the
/// abandoned row is the typed observation error; the `Pending` row stays for
/// the next boot to close.
#[tokio::test]
async fn start_arm_failing_abandoned_row_write_is_returned_as_the_typed_observation_error() {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    *rig.obs.fail_next_write_of.lock() = Some(AllocState::Terminated);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(
        matches!(&result, Err(ShimError::Observation(ObservationStoreError::Unreachable { peer })) if peer == WRITE_CAUSE),
        "a failing abandoned-row write is returned as the typed observation error: {result:?}"
    );
    assert_eq!(
        rig.row(id).await.map(|row| row.state),
        Some(AllocState::Pending),
        "the Pending row stays until the next boot closes it"
    );
}

/// RB-8 W3, ADR-0355 section 4. A non-start `DriverError` from `driver.start`
/// ends the launch's `Pending` row as a `Failed` row `DriverInternalError`
/// (charged like any start failure), carrying the address and releasing the
/// lease; the launch is never left as a `Pending` row.
#[tokio::test]
async fn start_arm_a_non_start_driver_error_ends_in_a_failed_driver_internal_error_row() {
    let driver = Arc::new(ScriptedStartDriver::other_error());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    let row = rig
        .row(id)
        .await
        .unwrap_or_else(|| panic!("a non-start driver error ends the launch in a Failed DriverInternalError row, got no row (dispatch: {result:?})"));
    assert_eq!(row.state, AllocState::Failed, "ended as Failed, never left Pending: {row:?}");
    assert!(
        matches!(row.reason, Some(TransitionReason::DriverInternalError { .. })),
        "the reason is DriverInternalError: {:?}",
        row.reason
    );
    assert!(row.workload_addr.is_some(), "the Pending row's address is carried");
    assert!(!rig.has_lease(id), "the lease is released");
}

// ---------------------------------------------------------------------------
// N-2: the guest address is on record before the VMM can exist.
// ---------------------------------------------------------------------------

/// RB-6, ADR-0353. When the launch reaches the driver, so before
/// `Vmm::create` is reachable, the store holds the allocation's `Pending` row
/// and its `workload_addr` is the address of the lease the pool handed out
/// (the transport the shim injected into the spec).
///
/// CONTRACT_SHAPE: bounded-change. Universe: the allocation's row at the
/// instant of `Driver::start`.
#[tokio::test]
async fn start_arm_records_the_pending_row_with_the_leases_address_before_the_driver_starts() {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    let id = "alloc-prelaunch-service-0";

    rig.dispatch(start_action(id, WorkloadKind::Service)).await.expect("dispatch");

    let seen = driver.seen_at_start.lock().first().cloned().expect("the launch reached the driver");
    let address = seen.address.expect("the shim injected the lease's transport into the spec");
    let row =
        seen.row.as_ref().expect("the store holds the launch's Pending row when the driver starts");
    assert_eq!(row.state, AllocState::Pending, "the record is the existing Pending state");
    assert_eq!(row.workload_addr, Some(address), "the row carries the lease's address");
    assert_eq!((row.started_at, row.reason.is_none(), row.terminal.clone()), (None, true, None));
}

/// RB-6, ADR-0353. The restart arm records the successor's `Pending`
/// row with the successor lease's address before the successor reaches the
/// driver.
#[tokio::test]
async fn restart_arm_records_the_successors_pending_row_with_the_leases_address_before_the_driver_starts()
 {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    rig.seed_crashed_predecessor("alloc-prelaunch-service-0", true).await;
    let successor = "alloc-prelaunch-service-1";

    rig.dispatch(restart_action("alloc-prelaunch-service-0", successor)).await.expect("dispatch");

    let seen =
        driver.seen_at_start.lock().first().cloned().expect("the successor reached the driver");
    let address = seen.address.expect("the shim injected the lease's transport into the spec");
    let row = seen
        .row
        .as_ref()
        .expect("the store holds the successor's Pending row when the driver starts");
    assert_eq!(row.alloc_id, alloc(successor));
    assert_eq!(row.state, AllocState::Pending, "the record is the existing Pending state");
    assert_eq!(row.workload_addr, Some(address), "the row carries the lease's address");
}

/// RB-6, N-REC, ADR-0353. When the `Pending` write fails no VMM can
/// exist: the driver is never reached, the lease is retired and torn down, and
/// the typed error is returned.
#[tokio::test]
async fn start_arm_failing_pending_write_never_reaches_the_driver_and_releases_the_lease() {
    let driver = Arc::new(ScriptedStartDriver::new());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    driver.attach_obs(Arc::clone(&rig.obs) as Arc<dyn ObservationStore>);
    *rig.obs.fail_next_write_of.lock() = Some(AllocState::Pending);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(
        driver.starts.lock().is_empty(),
        "the driver is never reached when the Pending write fails"
    );
    assert!(
        matches!(&result, Err(ShimError::Observation(ObservationStoreError::Unreachable { peer })) if peer == WRITE_CAUSE),
        "the typed error is returned: {result:?}"
    );
    assert!(!rig.has_lease(id), "the lease is released");
    assert_eq!(rig.teardowns_of(id), 1, "the lease's network is torn down once");
}

/// RB-6, ADR-0353 terminal-row carry. The `Failed` row of a start the
/// driver rejects carries the `Pending` row's address.
#[tokio::test]
async fn start_arm_failed_start_row_carries_the_pending_rows_address() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    driver.script_next_start_rejection(Some("scripted rejection"));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(result.is_ok(), "a rejected start is recorded as a row: {result:?}");
    let row = rig.row(id).await.expect("the rejected start has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        row.workload_addr.is_some(),
        "the Failed row of a rejected start carries the Pending row's address"
    );
}

// ---------------------------------------------------------------------------
// RB-2: a guest-address lease is released only once the scope is absent.
// ---------------------------------------------------------------------------

/// Start `id` over a `SimDriver` and a `SimVmHostState`, leaving a `Running`
/// allocation with its lease. Returns the rig and the host double.
async fn running_vm_over_a_host(id: &str) -> (Rig, SimVmHostState) {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let mut rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let host = SimVmHostState::new();
    rig.state.vm_host_state = Arc::new(host.clone());
    rig.dispatch(start_action(id, WorkloadKind::Service)).await.expect("the start dispatches");
    assert_eq!(
        rig.row(id).await.map(|row| row.state),
        Some(AllocState::Running),
        "precondition: the allocation is Running"
    );
    assert!(rig.has_lease(id), "precondition: the running allocation holds a lease");
    (rig, host)
}

/// RB-2, ADR-0156 amended. A stop whose VM still has its cgroup scope
/// (the VMM was killed and is not yet reaped) completes (the row is
/// `Terminated`) but keeps the guest-address lease: the lease is released only
/// once the scope is observed absent, because the scope's presence is the proof
/// that the VMM still holds its vsock device and so its CID. After the scope is
/// gone, the network-reclaim action that a terminal row with a lease already
/// triggers releases the lease, through the one release site.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the allocation's row, the pool's
/// lease and occupancy, the host's scopes.
#[tokio::test]
async fn a_stop_keeps_the_lease_held_while_the_vms_scope_is_present_and_a_reclaim_releases_it_after()
 {
    let id = "alloc-prelaunch-service-0";
    let (rig, host) = running_vm_over_a_host(id).await;
    host.set_scope(alloc(id), BTreeSet::from([4242]));

    rig.dispatch(Action::StopAllocation { alloc_id: alloc(id), terminal: None })
        .await
        .expect("the stop completes");

    assert_eq!(
        rig.row(id).await.map(|row| row.state),
        Some(AllocState::Terminated),
        "the stop completes while the scope is present"
    );
    let held = rig.pool.observe(&[alloc(id)]);
    assert_eq!(
        held.leases.get(&alloc(id)),
        Some(&overdrive_core::traits::guest_attachment_view::GuestAttachmentLease::Retiring),
        "the lease is held while the VM's scope is present (Retiring, still counted)"
    );
    assert_eq!(held.occupancy.held, 1, "the held lease counts toward the attachment cap");

    // The VMM is reaped and the reclamation pass removes the scope.
    host.kill_scope(&overdrive_core::cgroup::CgroupPath::for_alloc(&alloc(id)))
        .await
        .expect("the scope is removed");
    rig.dispatch(Action::ReclaimAllocationNetwork { alloc_id: alloc(id) })
        .await
        .expect("the reclaim attempt dispatches");
    assert!(!rig.has_lease(id), "the reclaim attempt releases the lease once the scope is absent");
}

/// RB-6, ADR-0353 terminal-row carry. The stop's terminal row carries
/// the running row's `workload_addr`: boot reserves a stuck VM's offset from the
/// allocation's last row, and a stopped-but-unreaped VMM is exactly that VM.
#[tokio::test]
async fn a_stops_terminal_row_carries_the_prior_rows_workload_addr() {
    let id = "alloc-prelaunch-service-0";
    let (rig, _host) = running_vm_over_a_host(id).await;
    let running = rig.row(id).await.expect("the running row");
    assert!(running.workload_addr.is_some(), "precondition: the Running row carries the address");

    rig.dispatch(Action::StopAllocation { alloc_id: alloc(id), terminal: None })
        .await
        .expect("the stop completes");

    let stopped = rig.row(id).await.expect("the terminal row");
    assert_eq!(stopped.state, AllocState::Terminated);
    assert_eq!(
        stopped.workload_addr, running.workload_addr,
        "the stop's terminal row carries the prior row's workload_addr"
    );
}

/// GUARD (RB-6, ADR-0353 "a stale duplicate dispatch"). Dispatching a start
/// again for an allocation whose row is already `Running` never writes a
/// `Pending` row over it: the existing non-terminal row is the record and
/// carries the same address.
#[tokio::test]
async fn guard_a_duplicate_start_dispatch_never_regresses_a_running_row_to_pending() {
    let id = "alloc-prelaunch-service-0";
    let (rig, _host) = running_vm_over_a_host(id).await;
    let running = rig.row(id).await.expect("the running row");

    let second = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    let after = rig.row(id).await.expect("the row survives the duplicate dispatch");
    assert_eq!(
        after.state,
        AllocState::Running,
        "a duplicate start never writes Pending over a Running row (second dispatch: {second:?})"
    );
    assert_eq!(after.workload_addr, running.workload_addr, "the record keeps its address");
}

/// GUARD (RB-6, ADR-0353 restart arm, `dominating(.., None)`). A restart
/// dispatched twice for the same successor never regresses the successor's
/// `Running` row to `Pending`. ADR-0353 states the guard for the start arm
/// only; this pins it for the restart arm, whose successor write has no prior
/// row to dominate.
#[tokio::test]
async fn guard_a_duplicate_restart_dispatch_never_regresses_the_successors_running_row() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    rig.dispatch(restart_action(predecessor, successor)).await.expect("the restart dispatches");
    assert_eq!(
        rig.row(successor).await.map(|row| row.state),
        Some(AllocState::Running),
        "precondition: the successor is Running"
    );

    let second = rig.dispatch(restart_action(predecessor, successor)).await;

    assert_eq!(
        rig.row(successor).await.map(|row| row.state),
        Some(AllocState::Running),
        "a duplicate restart never writes Pending over the successor's Running row (second dispatch: {second:?})"
    );
}

/// GUARD (RB-2). With no scope left, a stop releases the lease at once: the
/// scope gate costs nothing for a reaped VMM.
#[tokio::test]
async fn guard_a_stop_releases_the_lease_at_once_when_no_scope_remains() {
    let id = "alloc-prelaunch-service-0";
    let (rig, _host) = running_vm_over_a_host(id).await;

    rig.dispatch(Action::StopAllocation { alloc_id: alloc(id), terminal: None })
        .await
        .expect("the stop completes");

    assert_eq!(rig.row(id).await.map(|row| row.state), Some(AllocState::Terminated));
    assert!(!rig.has_lease(id), "no scope, so the lease is released at the stop");
}

// ---------------------------------------------------------------------------
// RB-2 over an unreapable VMM: the lease outlives the stop until the scope is
// removed (feature delta "The guest-address lease outlives an unreaped VMM").
// ---------------------------------------------------------------------------

/// How many allocations the next-fit walk below assigns and releases: more than
/// the pool's usable offsets (the `/16` guest prefix minus the network, the
/// gateway and the broadcast address), so the cursor passes the held offset.
const WRAPPING_WALK: u32 = 65_600;

/// A VMM the stop killed and the host cannot reap, with the scope it still
/// occupies. The reap is `SimVmm::exit_process`; the scope's hold reads the
/// same fact, so one call ends both.
struct UnreapedVm {
    vmm: SimVmm,
    pid: u32,
    cid: GuestCid,
    root: TempDir,
    _process: overdrive_core::traits::vmm::VmProcess,
}

/// A launch config whose rootfs master is a real file under `root`.
fn vm_config(root: &std::path::Path, name: &str) -> VmConfig {
    let alloc = alloc(name);
    let master = root.join(format!("{alloc}-master.img"));
    std::fs::write(&master, [0_u8; 512]).expect("write the rootfs master");
    let kernel = KernelImage::validate(root.join("vmlinux"), HostArch::X86_64, b"\x7fELF")
        .expect("an ELF header validates");
    VmConfig {
        volume: None,
        alloc: alloc.clone(),
        kernel,
        rootfs: RootfsPlan::for_alloc(master, 512, &alloc, root, &root.join("clone-index")),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(128 * 1024 * 1024),
        vcpus: std::num::NonZeroU8::new(1).expect("nonzero"),
        run_dir: VmRunDir::for_alloc(root, &alloc),
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

/// Launch a VMM on the CID of `id`'s lease, kill it with a SIGKILL it never
/// reaps (`SimVmm::inject_unreapable_process`, then `terminate`: the outcome is
/// `KilledNotReaped` after the reap bound on the VMM's injected clock), and
/// leave its scope in the host, held until the VMM is reaped.
async fn kill_the_vmm_without_a_reap(rig: &Rig, host: &SimVmHostState, id: &str) -> UnreapedVm {
    let cid = rig
        .pool
        .snapshot()
        .get(&alloc(id))
        .expect("the running allocation's lease")
        .assignment()
        .cid();
    let root = TempDir::new().expect("tempdir");
    let vmm_clock = Arc::new(SimClock::new());
    let vmm = SimVmm::new(Arc::clone(&vmm_clock) as Arc<dyn Clock>);
    vmm.inject_unreapable_process();
    let process = vmm.create(&vm_config(root.path(), id), cid).await.expect("the VMM launches");
    let pid = process.control.pid;
    let control = process.control.clone();
    let killing = tokio::spawn({
        let vmm = vmm.clone();
        async move { vmm.terminate(&control, Duration::ZERO).await }
    });
    for _ in 0..10_000 {
        if killing.is_finished() {
            break;
        }
        vmm_clock.tick(VMM_REAP_BOUND);
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        matches!(killing.await.expect("the kill task ends"), Ok(VmTermination::KilledNotReaped)),
        "precondition: the VMM is killed and cannot be reaped"
    );
    assert!(vmm.is_unreaped(pid), "precondition: the killed VMM is unreaped");
    assert_eq!(
        vmm.vsock_device_holders(),
        BTreeMap::from([(cid, pid)]),
        "precondition: the unreaped VMM still holds its CID's device"
    );
    host.set_scope(alloc(id), BTreeSet::from([pid]));
    host.set_run_dir(alloc(id));
    host.hold_scope_while(&alloc(id), {
        let vmm = vmm.clone();
        move || vmm.is_unreaped(pid)
    });
    UnreapedVm { vmm, pid, cid, root, _process: process }
}

impl Rig {
    /// One real `VmReclamation` tick over the production runtime, on the
    /// node's own target.
    async fn reclamation_pass(&self, n: u64) -> Result<(), ConvergenceError> {
        run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &ReconcilerName::new("vm-reclamation").expect("reconciler name"),
            &TargetResource::new(&format!("node/{NODE}")).expect("target"),
            self.base + Duration::from_millis(n * 100),
            n,
            self.base + Duration::from_secs(120),
            self.owner.as_ref(),
        )
        .await
    }

    /// Advance the injected wall clock past any reclaim backoff.
    fn elapse_reclaim_backoff(&self) {
        self.clock.tick(Duration::from_secs(3_600));
    }
}

fn operator_stop(id: &str) -> Action {
    Action::StopAllocation {
        alloc_id: alloc(id),
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
    }
}

/// RB-2 Tier 1 (a), ADR-0156 amended. A stop whose VMM was killed and
/// not reaped completes (`Terminated`) and leaves the allocation's lease
/// Retiring: it counts toward the cap, and an assign that wraps to its offset
/// takes another, so no launch is handed the CID the unreaped VMM still holds.
///
/// Owner path: the production stop arm over `SimDriver`, the production pool,
/// `SimVmm` (an unreapable process) and `SimVmHostState` (its held scope).
/// Fault: the unreaped VMM. Oracle: the row, the pool's `observe`, the CIDs the
/// pool hands out across one full next-fit walk, `SimVmm::vsock_device_holders`.
#[tokio::test]
async fn a_stop_of_an_unreapable_vm_keeps_its_lease_counted_and_no_wrapping_assign_takes_its_cid() {
    let id = "alloc-prelaunch-service-0";
    let (rig, host) = running_vm_over_a_host(id).await;
    let vm = kill_the_vmm_without_a_reap(&rig, &host, id).await;

    rig.dispatch(operator_stop(id)).await.expect("the stop completes");

    assert_eq!(
        rig.row(id).await.map(|row| row.state),
        Some(AllocState::Terminated),
        "the stop completes while the VMM is unreaped"
    );
    let held = rig.pool.observe(&[alloc(id)]);
    assert_eq!(
        held.leases.get(&alloc(id)),
        Some(&overdrive_core::traits::guest_attachment_view::GuestAttachmentLease::Retiring),
        "the unreaped VM's lease stays held Retiring while its scope is present"
    );
    assert_eq!(
        (held.occupancy.held, held.occupancy.retiring),
        (1, 1),
        "the retiring lease counts toward the attachment cap"
    );
    for step in 0..WRAPPING_WALK {
        let walker = alloc(&format!("alloc-wrapping-walk-{step}"));
        let plan = rig
            .pool
            .assign(walker.clone(), &[], crate::shared_network_test_ports::test_owner_flow_ports())
            .expect("below the cap every assignment succeeds");
        assert_ne!(
            plan.assignment().cid(),
            vm.cid,
            "an assign that wraps to the held offset takes another (step {step})"
        );
        rig.pool.release(&walker);
    }
    assert_eq!(
        vm.vmm.vsock_device_holders(),
        BTreeMap::from([(vm.cid, vm.pid)]),
        "the unreaped VMM still holds its CID's device after the walk"
    );
}

/// RB-2 Tier 1 (b), ADR-0156 amended. After the scripted reap
/// (`SimVmm::exit_process`), a `VmReclamation` pass removes the scope and a
/// network reclaim attempt releases the lease; a launch on the freed CID is not
/// refused by the device, so no launch clashed.
///
/// Owner path: the production stop arm, `WorkloadLifecycle`'s reclaim tick and
/// `VmReclamation`'s pass over `SimVmm` and `SimVmHostState`. Oracle: the pool,
/// the host's scopes, the device holders and the fresh VMM's liveness.
#[tokio::test]
async fn after_the_reap_a_reclamation_pass_and_a_reclaim_attempt_release_the_lease_and_no_launch_clashes()
 {
    let id = "alloc-prelaunch-service-0";
    let (rig, host) = running_vm_over_a_host(id).await;
    let vm = kill_the_vmm_without_a_reap(&rig, &host, id).await;
    rig.dispatch(operator_stop(id)).await.expect("the stop completes");
    assert_eq!(rig.row(id).await.map(|row| row.state), Some(AllocState::Terminated));
    assert!(rig.has_lease(id), "the lease is held until the VMM is reaped");

    assert!(vm.vmm.exit_process(vm.pid, None, Some(9)), "the scripted reap reaches the VMM");

    assert!(vm.vmm.vsock_device_holders().is_empty(), "the reap closed the VMM's device");
    assert!(host.has_scope(&alloc(id)), "the reap alone removes no scope");
    assert!(rig.has_lease(id), "the reap alone releases no lease");
    rig.reclamation_pass(1).await.expect("the reclamation pass removes the empty scope");
    assert!(!host.has_scope(&alloc(id)), "the pass removed the scope once the VMM was reaped");
    rig.elapse_reclaim_backoff();
    rig.tick(2).await.expect("the reclaim attempt releases the lease");
    assert!(!rig.has_lease(id), "the reclaim attempt after the scope's removal releases the lease");
    assert_eq!(rig.pool.observe(&[]).occupancy.held, 0, "no lease counts toward the cap");

    let fresh = vm
        .vmm
        .create(&vm_config(vm.root.path(), "alloc-prelaunch-service-9"), vm.cid)
        .await
        .expect("a launch on the freed CID spawns");
    assert!(vm.vmm.is_live(fresh.control.pid), "the launch on the freed CID did not clash");
    assert_eq!(
        vm.vmm.vsock_device_holders(),
        BTreeMap::from([(vm.cid, fresh.control.pid)]),
        "the fresh VMM holds the freed CID's device"
    );
}

/// RB-2 Tier 1 (c), ADR-0156 amended. When the scope observation at
/// the stop fails, the stop returns the typed `ShimError::Reclamation` before
/// any effect: no teardown, no terminal row (the `Running` row stands), and the
/// replay of the same stop succeeds.
///
/// Fault: `SimVmHostState::script_observe_failure`. Oracle: the dispatch
/// result, the row, the owner's journal, the lease, then the replay's row and
/// lease.
#[tokio::test]
async fn a_failing_scope_observation_at_the_stop_gives_a_typed_error_with_no_teardown_and_no_row_and_the_replay_succeeds()
 {
    let id = "alloc-prelaunch-service-0";
    let (rig, host) = running_vm_over_a_host(id).await;
    let running = rig.row(id).await.expect("the running row");
    host.script_observe_failure(std::io::ErrorKind::PermissionDenied);

    let result = rig.dispatch(operator_stop(id)).await;

    assert!(
        matches!(
            &result,
            Err(ShimError::Reclamation(super::reclamation::ReclamationError::Host(error)))
                if error.kind() == std::io::ErrorKind::PermissionDenied
        ),
        "a failing scope observation at the stop is the typed reclamation error: {result:?}"
    );
    assert_eq!(rig.teardowns_of(id), 0, "nothing is torn down before the observation");
    assert_eq!(
        rig.row(id).await,
        Some(running),
        "no terminal row is written: the Running row stands"
    );
    assert!(rig.has_lease(id), "the lease is untouched");

    let replay = rig.dispatch(operator_stop(id)).await;

    assert!(replay.is_ok(), "the replay of the same stop succeeds: {replay:?}");
    assert_eq!(rig.row(id).await.map(|row| row.state), Some(AllocState::Terminated));
    assert!(!rig.has_lease(id), "the replay finds no scope and releases the lease");
    assert_eq!(rig.teardowns_of(id), 1, "the replay tears the network down once");
}

/// RB-2 Tier 1 (d), ADR-0156 amended. While the scope is held, every
/// reclaim attempt `WorkloadLifecycle` makes leaves the lease held: the scope's
/// presence, not the number of attempts, decides. The lease is released by the
/// first attempt after the reap and the pass's removal of the scope.
///
/// Oracle: the pool's lease state and occupancy and the host's scope after each
/// attempt; the view's attempt count proves the attempts were made.
#[tokio::test]
async fn the_lease_stays_held_across_several_reclaim_attempts_while_the_scope_is_held() {
    let id = "alloc-prelaunch-service-0";
    let (rig, host) = running_vm_over_a_host(id).await;
    let vm = kill_the_vmm_without_a_reap(&rig, &host, id).await;
    rig.dispatch(operator_stop(id)).await.expect("the stop completes");

    for attempt in 1..=3_u64 {
        rig.elapse_reclaim_backoff();
        rig.tick(attempt).await.expect("a reclaim attempt over a held scope completes");
        let held = rig.pool.observe(&[alloc(id)]);
        assert_eq!(
            held.leases.get(&alloc(id)),
            Some(&overdrive_core::traits::guest_attachment_view::GuestAttachmentLease::Retiring),
            "the lease stays held while the scope is held, attempt {attempt}"
        );
        assert_eq!(held.occupancy.held, 1, "the held lease still counts, attempt {attempt}");
        assert!(host.has_scope(&alloc(id)), "the held scope is not removed, attempt {attempt}");
    }
    assert_eq!(
        rig.view().reclaim_attempts.get(&alloc(id)).copied(),
        Some(3),
        "three reclaim attempts were made"
    );

    assert!(vm.vmm.exit_process(vm.pid, None, Some(9)), "the scripted reap reaches the VMM");
    rig.reclamation_pass(10).await.expect("the pass removes the scope after the reap");
    assert!(!host.has_scope(&alloc(id)), "the scope is removed after the reap");
    rig.elapse_reclaim_backoff();
    rig.tick(4).await.expect("the attempt after the removal completes");
    assert!(!rig.has_lease(id), "the lease is released after the scope's removal");
}

// ---------------------------------------------------------------------------
// S5 and R7: identity issuance fails before the driver runs.
// ---------------------------------------------------------------------------

/// PENDING (S5). A Service's `StartAllocation` whose SVID issuance fails
/// writes a `Failed` row `MtlsInterceptInstallFailed { stage: "identity_issue" }`
/// whose `detail` carries the issuance cause, releases the guest lease,
/// emits the lifecycle event and returns `Ok(())`. Today it returns the
/// error and writes no row.
#[tokio::test]
#[should_panic(
    expected = "an identity issuance failure is recorded as a row: the start dispatch returns Ok(()), got Err("
)]
async fn start_arm_identity_issue_failure_writes_a_failed_row_naming_the_cause() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.ca.arm(true);
    let mut lifecycle = rig.state.lifecycle_events.subscribe();
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(
        result.is_ok(),
        "an identity issuance failure is recorded as a row: the start dispatch returns Ok(()), got {result:?}"
    );
    let row = rig.row(id).await.expect("the failed launch has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        identity_issue_detail(&row).contains(CA_CAUSE),
        "the row's detail carries the issuance cause: {:?}",
        row.reason
    );
    assert_eq!(row.terminal, None, "the reconciler owns the terminal claim");
    assert_eq!(row.started_at, None);
    assert_eq!(
        row.workload_addr,
        Some(rig.pending_address(id)),
        "the Failed row carries the Pending row's address (the lease's)"
    );
    assert_eq!(row.kind, WorkloadKind::Service);
    assert!(!rig.has_lease(id), "the guest lease is released");
    assert!(driver.started_specs().is_empty(), "the driver never ran");
    assert!(!rig.held_svid(id), "no SVID is held");
    let published = drain_lifecycle(&mut lifecycle);
    assert!(
        published.iter().any(|event| event.alloc_id == alloc(id)
            && matches!(&event.reason, TransitionReason::MtlsInterceptInstallFailed { .. })),
        "the lifecycle event for the failed row is emitted: {published:?}"
    );
}

/// PENDING (R7). A restart whose successor SVID issuance fails writes the
/// same `Failed` successor row, unwinds the successor, and runs the exact-old
/// predecessor cleanup once.
#[tokio::test]
#[should_panic(
    expected = "an identity issuance failure is recorded as a row: the restart dispatch returns Ok(()), got Err("
)]
async fn restart_arm_identity_issue_failure_writes_a_failed_successor_row_then_cleans_the_predecessor()
 {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    rig.ca.arm(true);
    let mut lifecycle = rig.state.lifecycle_events.subscribe();

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(
        result.is_ok(),
        "an identity issuance failure is recorded as a row: the restart dispatch returns Ok(()), got {result:?}"
    );
    let row = rig.row(successor).await.expect("the failed successor has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(identity_issue_detail(&row).contains(CA_CAUSE), "detail carries the cause");
    assert_eq!(row.terminal, None);
    assert_eq!(
        row.workload_addr,
        Some(rig.pending_address(successor)),
        "the Failed successor row carries the successor's Pending row address"
    );
    assert!(!rig.has_lease(successor), "the successor's lease is released");
    assert!(driver.started_specs().is_empty(), "the driver never ran");
    assert_eq!(rig.teardowns_of(predecessor), 1, "the predecessor's one teardown ran");
    assert!(!rig.has_lease(predecessor), "the predecessor's lease is released");
    assert_eq!(
        driver.stop_calls(),
        [alloc(predecessor)],
        "the exact-old predecessor stop ran once"
    );
    assert!(
        drain_lifecycle(&mut lifecycle).iter().any(|event| event.alloc_id == alloc(successor)),
        "the lifecycle event for the failed successor is emitted"
    );
}

/// PENDING (S5-J). A Job's launch whose identity issuance fails writes the
/// same row, with the Job kind.
#[tokio::test]
#[should_panic(
    expected = "an identity issuance failure is recorded as a row: the Job start dispatch returns Ok(()), got Err("
)]
async fn a_job_identity_issue_failure_writes_the_same_failed_row() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.ca.arm(true);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Job)).await;

    assert!(
        result.is_ok(),
        "an identity issuance failure is recorded as a row: the Job start dispatch returns Ok(()), got {result:?}"
    );
    let row = rig.row(id).await.expect("the failed Job launch has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert_eq!(row.kind, WorkloadKind::Job);
    assert!(identity_issue_detail(&row).contains(CA_CAUSE), "detail carries the cause");
    assert_eq!(
        row.terminal, None,
        "a Job's pre-driver row carries no terminal; the next tick finalises it"
    );
    assert_eq!(
        row.workload_addr,
        Some(rig.pending_address(id)),
        "the Job's Failed row carries the Pending row's address"
    );
}

/// PENDING (S5, teardown fault). A guest-network teardown failure does not
/// suppress the row, and is returned after it.
#[tokio::test]
#[should_panic(expected = "expected MtlsInterceptInstallFailed identity_issue, got None")]
async fn start_arm_teardown_failure_does_not_suppress_the_identity_issue_row() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.ca.arm(true);
    rig.owner.script_teardown_failure(true);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    let row = rig
        .row(id)
        .await
        .expect("the identity_issue row is written even when the lease teardown fails");
    assert!(identity_issue_detail(&row).contains(CA_CAUSE), "detail carries the cause");
    assert_eq!(
        row.workload_addr,
        Some(rig.pending_address(id)),
        "the row carries the Pending row's address even when the teardown fails"
    );
    assert!(
        matches!(result, Err(ShimError::GuestNetwork(_))),
        "the teardown failure is returned after the row: {result:?}"
    );
    assert_eq!(rig.teardowns_of(id), 1, "the teardown was attempted once");
}

// ---------------------------------------------------------------------------
// R3: an empty driver registry in a restart.
// ---------------------------------------------------------------------------

/// PENDING (R3). A node that booted without the VM driver while a VM
/// workload's intent persists: the restart's successor is a `Failed` row
/// `DriverInternalError`, not `HandleMissing` with no row.
#[tokio::test]
#[should_panic(
    expected = "R3: an empty driver registry in a restart writes a Failed successor row, not HandleMissing: Err(HandleMissing"
)]
async fn restart_with_no_driver_composed_writes_a_failed_successor_row_not_handle_missing() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let mut rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.state.drivers = Arc::new(DriverRegistry::new());
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, false).await;

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(
        !matches!(result, Err(ShimError::HandleMissing { .. })),
        "R3: an empty driver registry in a restart writes a Failed successor row, not HandleMissing: {result:?}"
    );
    let row = rig.row(successor).await.expect("the failed successor has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        matches!(&row.reason, Some(TransitionReason::DriverInternalError { detail })
            if detail.contains("no vm driver composed")),
        "the row names the missing driver: {:?}",
        row.reason
    );
    assert_eq!(row.terminal, None);
    assert_eq!(
        row.workload_addr,
        Some(rig.pending_address(successor)),
        "the Failed successor row carries the Pending row's address"
    );
    assert!(!rig.has_lease(successor), "the successor's lease is released");
}

// ---------------------------------------------------------------------------
// Rows that stay as they are.
// ---------------------------------------------------------------------------

/// GUARD (S3, passes today). A non-identity guest-network error writes the
/// unchanged `Failed` row `WorkloadNetnsProvisionFailed { stage: "guest_network" }`,
/// releases the lease and returns `Ok(())`.
#[tokio::test]
async fn start_arm_guest_network_provision_failure_row_is_unchanged() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.owner.script_provision_failure(true);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(result.is_ok(), "the provision failure is recorded as a row: {result:?}");
    let row = rig.row(id).await.expect("the failed launch has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        matches!(&row.reason, Some(TransitionReason::WorkloadNetnsProvisionFailed { stage, .. })
            if stage == "guest_network"),
        "the row keeps its reason: {:?}",
        row.reason
    );
    assert_eq!(row.terminal, None);
    assert!(!rig.has_lease(id), "the lease is released");
    assert!(driver.started_specs().is_empty(), "the driver never ran");
    assert_eq!(
        rig.ca.attempts.load(Ordering::SeqCst),
        0,
        "no SVID is issued for a refused network"
    );
}

/// GUARD (S6, passes today). No driver of the kind composed: a `Failed` row
/// `DriverInternalError`, lease released, `Ok(())`.
#[tokio::test]
async fn start_arm_with_no_driver_composed_row_is_unchanged() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let mut rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.state.drivers = Arc::new(DriverRegistry::new());
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(result.is_ok(), "the missing driver is recorded as a row: {result:?}");
    let row = rig.row(id).await.expect("the failed launch has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        matches!(&row.reason, Some(TransitionReason::DriverInternalError { detail })
            if detail.contains("no vm driver composed")),
        "the row names the missing driver: {:?}",
        row.reason
    );
    assert_eq!(row.terminal, None);
    assert!(!rig.has_lease(id), "the lease is released");
}

// ---------------------------------------------------------------------------
// S0 and R0: the prior-row read fails.
// ---------------------------------------------------------------------------

/// GUARD (S0, passes today). A failing prior-row read: the dispatch returns the
/// observation error with no effect taken and no row.
#[tokio::test]
async fn start_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let journal_before = rig.owner.journal().len();
    rig.obs.fail_point_reads.store(true, Ordering::SeqCst);
    let id = "alloc-prelaunch-service-0";

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(
        matches!(&result, Err(ShimError::Observation(ObservationStoreError::Unreachable { peer })) if peer == READ_CAUSE),
        "the dispatch returns the observation error: {result:?}"
    );
    assert!(rig.obs.inner.alloc_status_rows().await.expect("rows").is_empty(), "no row");
    assert_eq!(rig.owner.journal().len(), journal_before, "no network effect");
    assert!(!rig.has_lease(id), "no lease");
    assert!(driver.started_specs().is_empty(), "no driver start");
    assert_eq!(rig.ca.attempts.load(Ordering::SeqCst), 0, "no SVID issued");
}

/// GUARD (R0, passes today). The same for the restart arm: the predecessor row
/// cannot be read, so no effect is taken, including the predecessor cleanup.
#[tokio::test]
async fn restart_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    let journal_before = rig.owner.journal().len();
    let rows_before = rig.obs.inner.alloc_status_rows().await.expect("rows");
    rig.obs.fail_point_reads.store(true, Ordering::SeqCst);

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(
        matches!(&result, Err(ShimError::Observation(ObservationStoreError::Unreachable { peer })) if peer == READ_CAUSE),
        "the dispatch returns the observation error: {result:?}"
    );
    assert_eq!(rig.obs.inner.alloc_status_rows().await.expect("rows"), rows_before, "no row");
    assert_eq!(rig.owner.journal().len(), journal_before, "no network effect");
    assert!(rig.has_lease(predecessor), "the predecessor's lease is untouched");
    assert!(driver.started_specs().is_empty(), "no driver start");
    assert!(driver.stop_calls().is_empty(), "no predecessor cleanup ran");
    assert_eq!(rig.ca.attempts.load(Ordering::SeqCst), 0, "no SVID issued");
}

// ---------------------------------------------------------------------------
// N-ADM: an admission refusal writes no row and charges nothing.
// ---------------------------------------------------------------------------

/// GUARD (N-ADM, start arm; passes today). The reconciler emits a placement
/// below the cap; another target takes the last slot before the shim's
/// admission; the refusal writes no row and leaves the lease untaken; the next
/// hydration reads the cap and emits nothing, however many ticks follow.
///
/// Seed: no randomness; the pool's fill order is by index.
#[tokio::test]
async fn start_arm_admission_refusal_writes_no_row_charges_nothing_and_the_next_tick_emits_nothing()
{
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    rig.fill_pool("held", MAX_GUEST_NETWORK_ATTACHMENTS - 1);
    let pool = Arc::clone(&rig.pool);
    *rig.obs.point_read_hook.lock() = Some(Box::new(move || {
        pool.assign(
            alloc("raced-last-slot"),
            &[],
            crate::shared_network_test_ports::test_owner_flow_ports(),
        )
        .expect("another target admits into the last slot");
    }));

    let first = rig.tick(1).await;

    assert!(is_admission_refusal(&first), "the placement is refused at the cap: {first:?}");
    assert!(rig.rows().await.is_empty(), "a refusal writes no allocation row");
    assert!(driver.started_specs().is_empty(), "no launch started");
    assert!(!rig.has_lease("alloc-prelaunch-service-0"), "the refused launch holds no lease");
    let view = rig.view();
    assert!(
        view.restart_counts.values().all(|count| *count == 0),
        "a refusal charges no restart attempt: {:?}",
        view.restart_counts
    );
    let issued_after_refusal = view.issued_generation.len();
    for tick in 2..8 {
        rig.elapse_backoff();
        let result = rig.tick(tick).await;
        assert!(result.is_ok(), "an evaluation at the cap emits nothing and succeeds: {result:?}");
    }
    assert!(driver.started_specs().is_empty(), "no launch however many ticks run");
    assert!(rig.rows().await.is_empty(), "still no row");
    assert_eq!(
        rig.view().issued_generation.len(),
        issued_after_refusal,
        "no id is issued while the pool stays at the cap"
    );
}

/// GUARD (N-ADM, restart arm; passes today). A restart whose successor is
/// refused by a lost race still cleans its predecessor up: the predecessor's
/// lease is released, the refusal writes no successor row, charges nothing
/// beyond the predecessor's `+ 1`, and the next hydration reads one slot fewer
/// and emits again, assigning the same count (one extra id per lost race).
#[tokio::test]
async fn restart_arm_admission_refusal_releases_the_predecessor_and_the_next_tick_emits_again() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    let predecessor = "alloc-prelaunch-service-0";
    rig.fill_pool("held", MAX_GUEST_NETWORK_ATTACHMENTS - 2);
    rig.seed_crashed_predecessor(predecessor, true).await;
    let pool = Arc::clone(&rig.pool);
    *rig.obs.point_read_hook.lock() = Some(Box::new(move || {
        pool.assign(
            alloc("raced-last-slot"),
            &[],
            crate::shared_network_test_ports::test_owner_flow_ports(),
        )
        .expect("another target admits into the last slot");
    }));

    let first = rig.tick(1).await;

    assert!(is_admission_refusal(&first), "the successor is refused at the cap: {first:?}");
    assert!(rig.row("alloc-prelaunch-service-1").await.is_none(), "no successor row");
    assert_eq!(rig.rows().await.len(), 1, "only the predecessor's row exists");
    assert!(!rig.has_lease(predecessor), "the predecessor's lease is released by its cleanup");
    assert_eq!(rig.teardowns_of(predecessor), 1, "the predecessor's one teardown ran");
    let view = rig.view();
    assert_eq!(
        view.restart_counts.get(&alloc("alloc-prelaunch-service-1")),
        Some(&1),
        "the reservation carries the predecessor's count plus one"
    );

    let second = rig.tick(2).await;

    assert!(second.is_ok(), "the next evaluation reads one slot fewer and launches: {second:?}");
    let started: Vec<_> = driver.started_specs().iter().map(|s| s.alloc.clone()).collect();
    assert_eq!(
        started,
        [alloc("alloc-prelaunch-service-2")],
        "the reconciler emitted again after the release: one extra id per lost race"
    );
    assert_eq!(
        rig.view().restart_counts.get(&alloc("alloc-prelaunch-service-2")),
        Some(&1),
        "the relaunch is charged the same count, not twice"
    );
}

/// PENDING (item 6, start arm). After the start-arm refusal the wait predicate
/// over the resulting rows, the View and the pool is true (no row, pool at the
/// cap). The predicate is a scaffold today.
#[tokio::test]
#[should_panic(
    expected = "the wait predicate must be true after an admission refusal: it panicked: RED scaffold: start_blocked_only_by_capacity is not implemented"
)]
async fn after_a_start_arm_admission_refusal_the_wait_predicate_is_true() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    rig.fill_pool("held", MAX_GUEST_NETWORK_ATTACHMENTS - 1);
    let pool = Arc::clone(&rig.pool);
    *rig.obs.point_read_hook.lock() = Some(Box::new(move || {
        pool.assign(
            alloc("raced-last-slot"),
            &[],
            crate::shared_network_test_ports::test_owner_flow_ports(),
        )
        .expect("another target admits into the last slot");
    }));
    let first = rig.tick(1).await;
    assert!(is_admission_refusal(&first), "precondition: the placement is refused: {first:?}");

    assert_wait_predicate_true(&rig, &[alloc("alloc-prelaunch-service-0")]).await;
}

/// PENDING (item 6, restart arm). After a restart-arm refusal whose predecessor
/// held no lease (the pool stays at the cap, another target's admission), the
/// wait predicate is true. The predecessor row is production-authored: a
/// first launch the driver rejected.
#[tokio::test]
#[should_panic(
    expected = "the wait predicate must be true after an admission refusal: it panicked: RED scaffold: start_blocked_only_by_capacity is not implemented"
)]
async fn after_a_restart_arm_admission_refusal_the_wait_predicate_is_true() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    rig.fill_pool("held", MAX_GUEST_NETWORK_ATTACHMENTS - 1);
    driver.script_next_start_rejection(Some("prelaunch first launch rejected"));
    let first = rig.tick(1).await;
    assert!(
        first.is_ok(),
        "precondition: the rejected first launch is recorded as a row: {first:?}"
    );
    assert_eq!(rig.rows().await.len(), 1, "precondition: a Failed predecessor row exists");
    let pool = Arc::clone(&rig.pool);
    *rig.obs.point_read_hook.lock() = Some(Box::new(move || {
        pool.assign(
            alloc("raced-last-slot"),
            &[],
            crate::shared_network_test_ports::test_owner_flow_ports(),
        )
        .expect("another target admits into the last slot");
    }));
    rig.elapse_backoff();
    let second = rig.tick(2).await;
    assert!(is_admission_refusal(&second), "precondition: the restart is refused: {second:?}");
    assert_eq!(rig.rows().await.len(), 1, "the refusal wrote no successor row");

    assert_wait_predicate_true(
        &rig,
        &[alloc("alloc-prelaunch-service-0"), alloc("alloc-prelaunch-service-1")],
    )
    .await;
}

/// Evaluate `start_blocked_only_by_capacity` over the resulting rows, the View
/// and the pool, and fail with the pinned message when it is false or not yet
/// implemented.
async fn assert_wait_predicate_true(rig: &Rig, allocs: &[AllocationId]) {
    let rows = rig.rows().await;
    let row_refs: Vec<&AllocStatusRow> = rows.iter().collect();
    let view = rig.view();
    let observation = rig.pool.observe(allocs);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        overdrive_reconcilers::start_blocked_only_by_capacity(
            WorkloadKind::Service,
            true,
            &row_refs,
            &view,
            0,
            &observation,
        )
    }));
    match outcome {
        Ok(true) => {}
        Ok(false) => panic!(
            "the wait predicate must be true after an admission refusal: it returned false over rows {rows:?}"
        ),
        Err(payload) => {
            let reason = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a non-string panic".to_owned());
            panic!(
                "the wait predicate must be true after an admission refusal: it panicked: {reason}"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// The charge: a following tick, the real WorkloadLifecycle, the injected clock.
// ---------------------------------------------------------------------------

/// A terminal-condition probe used by the charge tests below.
fn backoff_exhausted(row: &AllocStatusRow) -> bool {
    matches!(row.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
}

/// PENDING (3a iii, S5 then R7 chain). A Service whose SVID issuance persistently
/// fails: the first launch is a `Failed` row (count 0); each following tick,
/// with the backoff window elapsed, emits one `RestartAllocation` whose id's
/// `restart_counts` entry is the failed row's plus one, until the ceiling, after
/// which the next tick finalises `BackoffExhausted`. The launches are the
/// ceiling plus the first, never one per tick.
#[tokio::test]
#[should_panic(
    expected = "the first launch's identity failure is recorded as a row so the tick succeeds: Err("
)]
async fn a_persistent_identity_failure_is_charged_until_the_restart_ceiling_then_exhausted() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    rig.ca.arm(true);

    let first = rig.tick(1).await;

    assert!(
        first.is_ok(),
        "the first launch's identity failure is recorded as a row so the tick succeeds: {first:?}"
    );
    let ceiling = overdrive_reconciler_ceiling();
    for step in 1..=ceiling {
        rig.elapse_backoff();
        let result = rig.tick(u64::from(step) + 1).await;
        assert!(result.is_ok(), "restart {step} is recorded as a row too: {result:?}");
        let id = alloc(&format!("alloc-prelaunch-service-{step}"));
        let row = rig.state.obs.alloc_status_row(&id).await.expect("row").expect("a failed row");
        assert_eq!(row.state, AllocState::Failed);
        assert!(identity_issue_detail(&row).contains(CA_CAUSE));
        assert_eq!(
            rig.view().restart_counts.get(&id),
            Some(&step),
            "the id's count is the failed predecessor's plus one"
        );
    }
    rig.elapse_backoff();
    let finalise = rig.tick(u64::from(ceiling) + 2).await;
    assert!(finalise.is_ok(), "the ceiling tick finalises: {finalise:?}");
    let last =
        rig.row(&format!("alloc-prelaunch-service-{ceiling}")).await.expect("the last failed row");
    assert!(backoff_exhausted(&last), "the row carries BackoffExhausted: {:?}", last.terminal);
    let rows_at_exhaustion = rig.rows().await.len();
    for tick in 0..4 {
        rig.elapse_backoff();
        let result = rig.tick(u64::from(ceiling) + 3 + tick).await;
        assert!(result.is_ok(), "an exhausted workload is idle: {result:?}");
    }
    assert_eq!(rig.rows().await.len(), rows_at_exhaustion, "no launch after exhaustion");
    assert!(driver.started_specs().is_empty(), "the driver never ran");
}

/// The reconciler's restart ceiling, read from the production constant.
fn overdrive_reconciler_ceiling() -> u32 {
    overdrive_reconcilers::RESTART_BACKOFF_CEILING
}

impl Rig {
    async fn put_job(&self) {
        let job =
            overdrive_core::aggregate::Job::from_submit(overdrive_core::aggregate::JobSpecInput {
                id: WORKLOAD.to_owned(),
                replicas: 1,
                resources: ResourcesInput {
                    cpu: CpuQuantity::from_millis(100),
                    memory: ByteSize::new(128 * 1024 * 1024),
                },
                driver: DriverInput::Vm(VmInput {
                    volume: None,
                    command: "/bin/true".to_owned(),
                    args: Vec::new(),
                    kernel: "/kernel".to_owned(),
                    rootfs: "/rootfs".to_owned(),
                }),
            })
            .expect("valid job spec");
        let id = job.id.clone();
        let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive the job");
        self.state
            .store
            .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
            .await
            .expect("put the job intent");
    }
}

/// PENDING (3a iii, J-1). A Job whose SVID issuance fails writes one `Failed`
/// row; the next tick finalises it `Failed { exit_code: Some(0) }` (the
/// misleading code is today's behaviour for any Job whose start is rejected,
/// pinned here by ruling J-1); and no further `StartAllocation` is emitted
/// however many ticks run, however far the clock advances.
#[tokio::test]
#[should_panic(
    expected = "the Job's identity failure is recorded as a row so the first tick succeeds: Err("
)]
async fn a_job_identity_failure_is_final_and_never_relaunched() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_job().await;
    rig.ca.arm(true);

    let first = rig.tick(1).await;

    assert!(
        first.is_ok(),
        "the Job's identity failure is recorded as a row so the first tick succeeds: {first:?}"
    );
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1, "one Failed row: {rows:?}");
    assert_eq!(rows[0].kind, WorkloadKind::Job);
    assert!(identity_issue_detail(&rows[0]).contains(CA_CAUSE));
    let issuances_after_first = rig.ca.attempts.load(Ordering::SeqCst);
    rig.elapse_backoff();
    let second = rig.tick(2).await;
    assert!(second.is_ok(), "the finalising tick succeeds: {second:?}");
    let finalised = rig.rows().await;
    assert_eq!(finalised.len(), 1, "finalising adds no allocation");
    assert_eq!(
        finalised[0].terminal,
        Some(TerminalCondition::Failed { exit_code: Some(0) }),
        "the next tick finalises the Job's failed row"
    );
    for tick in 3..12 {
        rig.elapse_backoff();
        let result = rig.tick(tick).await;
        assert!(result.is_ok(), "an idle finished Job: {result:?}");
    }
    assert_eq!(rig.rows().await.len(), 1, "no further StartAllocation, so no further row");
    assert_eq!(
        rig.ca.attempts.load(Ordering::SeqCst),
        issuances_after_first,
        "no further launch reached identity issuance"
    );
    assert!(driver.started_specs().is_empty(), "the driver never ran");
}

// ---------------------------------------------------------------------------
// Named no-row and early-return outcomes that stay as they are: S1, R2, R6,
// R9, S10 and R14 (ADR-0337 launch-outcome table). S7, S9 and R11 are covered
// by `tests/acceptance/lifecycle_broadcast.rs`,
// `tests/acceptance/action_shim_running_write_failure_stops_alloc.rs` and
// `tests/integration/mtls_install_fail_closed.rs`.
// ---------------------------------------------------------------------------

/// GUARD (S1, N-IDEM; passes today). A late `StartAllocation` for an id whose
/// current row is a finalised Job row (a terminal claim on a Job is immutable)
/// is an exact no-op: `Ok(())`, no new row, the prior row untouched, no lease,
/// no SVID, no driver start.
#[tokio::test]
async fn start_arm_for_a_finalised_job_row_is_a_no_op() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let id = "alloc-prelaunch-service-0";
    rig.seed_row(
        id,
        AllocState::Failed,
        WorkloadKind::Job,
        Some(TerminalCondition::Failed { exit_code: Some(0) }),
    )
    .await;
    let rows_before = rig.rows().await;
    let journal_before = rig.owner.journal().len();
    let mut lifecycle = rig.state.lifecycle_events.subscribe();

    let result = rig.dispatch(start_action(id, WorkloadKind::Job)).await;

    assert!(result.is_ok(), "a start for a finalised Job row is a no-op: {result:?}");
    assert_eq!(rig.rows().await, rows_before, "the finalised row is untouched and no row is added");
    assert_eq!(rig.owner.journal().len(), journal_before, "no network effect");
    assert!(!rig.has_lease(id), "no lease is taken");
    assert!(driver.started_specs().is_empty(), "no driver start");
    assert_eq!(rig.ca.attempts.load(Ordering::SeqCst), 0, "no SVID issued");
    assert!(drain_lifecycle(&mut lifecycle).is_empty(), "no lifecycle event is published");
}

/// Dispatch a restart whose predecessor row is already seeded and assert the
/// fence: `Ok(())`, no successor row, no effect, no predecessor cleanup.
async fn assert_restart_is_fenced(rig: &Rig, driver: &SimDriver, predecessor: &str) {
    let successor = "alloc-prelaunch-service-1";
    let rows_before = rig.rows().await;
    let journal_before = rig.owner.journal().len();

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(result.is_ok(), "a fenced restart is a no-op: {result:?}");
    assert!(rig.row(successor).await.is_none(), "no successor row");
    assert_eq!(rig.rows().await, rows_before, "the predecessor's row is untouched");
    assert_eq!(rig.owner.journal().len(), journal_before, "no network effect");
    assert!(!rig.has_lease(successor), "the successor takes no lease");
    assert!(driver.started_specs().is_empty(), "no driver start");
    assert!(driver.stop_calls().is_empty(), "no predecessor cleanup ran");
    assert_eq!(rig.ca.attempts.load(Ordering::SeqCst), 0, "no SVID issued");
}

/// GUARD (R2, N-STALE; passes today). A restart whose predecessor row is a
/// finalised Job row is fenced before any effect.
#[tokio::test]
async fn restart_arm_with_a_finalised_job_predecessor_is_fenced() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    rig.seed_row(
        predecessor,
        AllocState::Failed,
        WorkloadKind::Job,
        Some(TerminalCondition::Failed { exit_code: Some(0) }),
    )
    .await;

    assert_restart_is_fenced(&rig, &driver, predecessor).await;
}

/// GUARD (R2, N-STALE; passes today). A restart whose predecessor row is no
/// longer `Failed` or `Terminated` (the row changed between hydration and
/// dispatch) is fenced before any effect.
#[tokio::test]
async fn restart_arm_with_a_running_predecessor_is_fenced() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    rig.seed_row(predecessor, AllocState::Running, WorkloadKind::Service, None).await;

    assert_restart_is_fenced(&rig, &driver, predecessor).await;
}

/// GUARD (R6; passes today). A non-admission guest-network error in a restart
/// writes the `Failed` successor row `WorkloadNetnsProvisionFailed
/// { stage: "guest_network" }`, releases the successor's lease, still runs the
/// predecessor's cleanup once, and returns `Ok(())`.
#[tokio::test]
async fn restart_arm_guest_network_provision_failure_writes_a_failed_successor_row() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    rig.owner.script_provision_failure(true);

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(result.is_ok(), "the provision failure is recorded as a row: {result:?}");
    let row = rig.row(successor).await.expect("the failed successor has a row");
    assert_eq!(row.state, AllocState::Failed);
    assert!(
        matches!(&row.reason, Some(TransitionReason::WorkloadNetnsProvisionFailed { stage, .. })
            if stage == "guest_network"),
        "the row keeps its reason: {:?}",
        row.reason
    );
    assert_eq!(row.terminal, None);
    assert!(!rig.has_lease(successor), "the successor's lease is released");
    assert!(driver.started_specs().is_empty(), "the driver never ran");
    assert_eq!(
        rig.ca.attempts.load(Ordering::SeqCst),
        0,
        "no SVID is issued for a refused network"
    );
    assert_eq!(driver.stop_calls(), [alloc(predecessor)], "the predecessor cleanup ran once");
    assert!(!rig.has_lease(predecessor), "the predecessor's lease is released");
}

/// GUARD (R9, N-IDEM; passes today). A restart whose successor start reports a
/// duplicate VM owner is an idempotent stale dispatch. The owner is a launch of
/// the same restart that already has its `Pending` row and its lease (an owner
/// claim with no row is not a state `serve` and `deploy` produce), so the
/// duplicate dispatch writes no row of its own: it returns `Ok(())`, leaves the
/// owner's row unchanged, keeps the owner's lease, and the exact-old
/// predecessor cleanup still runs once. The start arm's duplicate owner (S7) is
/// covered by `lifecycle_broadcast`.
///
/// Owner path: the production restart arm over `ScriptedStartDriver`, the
/// production pool and `TestSharedOwner`. Oracle: the successor's row and lease
/// before and after, the driver's records, the owner's journal.
#[tokio::test]
async fn restart_arm_duplicate_vm_owner_writes_no_row_of_its_own_keeps_the_owners_row_and_lease_and_still_cleans_the_predecessor()
 {
    let driver = Arc::new(ScriptedStartDriver::duplicate_owner());
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    let owner_row = rig.seed_pending_owner(successor).await;
    let mut lifecycle = rig.state.lifecycle_events.subscribe();

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(result.is_ok(), "a duplicate owner is an idempotent no-op: {result:?}");
    assert_eq!(
        driver.starts.lock().as_slice(),
        [alloc(successor)],
        "the launch reached the driver"
    );
    assert_eq!(
        rig.row(successor).await,
        Some(owner_row),
        "a duplicate owner writes no row of its own: the owner's row is unchanged"
    );
    assert!(
        rig.has_lease(successor),
        "a duplicate owner keeps the owner's lease: the dispatch never releases it"
    );
    assert_eq!(rig.teardowns_of(successor), 0, "the owner's network is not torn down");
    assert!(
        drain_lifecycle(&mut lifecycle).iter().all(|event| event.alloc_id != alloc(successor)),
        "no lifecycle event names the successor"
    );
    assert_eq!(
        driver.stops.lock().as_slice(),
        [alloc(predecessor)],
        "the exact-old predecessor cleanup stops the predecessor once"
    );
    assert_eq!(rig.teardowns_of(predecessor), 1, "the predecessor's network is torn down once");
    assert!(!rig.has_lease(predecessor), "the predecessor's lease is released");
}

/// GUARD (S10, N-FAILSTOP; passes today). Guest-network activation withheld at
/// the EXEC gate's fail-stop: the start arm returns `Ok(())` after the Running
/// row is written, names the withheld activation, and takes no teardown (the
/// next boot's reclamation owns the rest).
#[tokio::test]
async fn start_arm_activation_withheld_at_fail_stop_leaves_the_running_row_and_returns_ok() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let id = "alloc-prelaunch-service-0";
    assert!(
        rig.exec.fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded).is_some(),
        "precondition: the EXEC gate enters fail-stop once"
    );

    let result = rig.dispatch(start_action(id, WorkloadKind::Service)).await;

    assert!(result.is_ok(), "a withheld activation returns Ok(()): {result:?}");
    let row = rig.row(id).await.expect("the Running row exists");
    assert_eq!(row.state, AllocState::Running);
    assert!(
        rig.event_names().contains(&"guest_network.activation_withheld"),
        "the withheld activation is named: {:?}",
        rig.event_names()
    );
    assert_eq!(rig.teardowns_of(id), 0, "no teardown while the fail-stop ends serve");
    assert!(rig.has_lease(id), "the lease is left for the next boot's reclamation");
}

/// GUARD (R14, N-FAILSTOP; passes today). The same in the restart arm: the
/// successor's Running row exists and `Ok(())` is returned, and the arm skips
/// `finish_restart`, so the predecessor's cleanup does not run.
#[tokio::test]
async fn restart_arm_activation_withheld_at_fail_stop_skips_the_predecessor_cleanup() {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    let predecessor = "alloc-prelaunch-service-0";
    let successor = "alloc-prelaunch-service-1";
    rig.seed_crashed_predecessor(predecessor, true).await;
    assert!(
        rig.exec.fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded).is_some(),
        "precondition: the EXEC gate enters fail-stop once"
    );

    let result = rig.dispatch(restart_action(predecessor, successor)).await;

    assert!(result.is_ok(), "a withheld activation returns Ok(()): {result:?}");
    let row = rig.row(successor).await.expect("the successor's Running row exists");
    assert_eq!(row.state, AllocState::Running);
    assert!(
        rig.event_names().contains(&"guest_network.activation_withheld"),
        "the withheld activation is named: {:?}",
        rig.event_names()
    );
    assert!(driver.stop_calls().is_empty(), "finish_restart is skipped: no predecessor stop");
    assert_eq!(rig.teardowns_of(predecessor), 0, "the predecessor's network is not torn down");
    assert!(rig.has_lease(predecessor), "the predecessor's lease is left for reclamation");
}

// ---------------------------------------------------------------------------
// The boot race already won, then stopped.
// ---------------------------------------------------------------------------

/// Real-time poll bound of the boot-race test below: the lane, the launch and
/// the gate settle on the tokio runtime, not on the injected clock.
const RACE_POLL: Duration = Duration::from_millis(10);
const RACE_POLLS: u32 = 400;

/// GUARD (feature delta § Superseding a launch, late success; passes today and
/// keeps passing once the lane table lands). A launch that already won its
/// guest-boot race (`start` returned `Ok`, the Running row is written) and
/// whose evaluation is still open (its activation waits on the recovering EXEC
/// gate) is an ordinary success when the stop's supersession signal arrives
/// late: the allocation row stays `Running`, no `StartSuperseded` outcome, no
/// charge, no `launch.superseded` event; the stop then proceeds as the ordinary
/// stop of a running allocation, releasing the lease.
///
/// The signal is raised through `ReconcilerRuntime::supersede_lane` with the
/// lane open (the tick has not returned); the assertions hold whether or not
/// the signal reaches the launch.
#[tokio::test]
async fn a_launch_that_won_its_boot_race_before_the_stop_signal_is_stopped_as_an_ordinary_running_allocation()
 {
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let rig = Rig::build(Arc::clone(&driver) as Arc<dyn Driver>).await;
    rig.put_service().await;
    let id = "alloc-prelaunch-service-0";
    let park = driver.park_next_start(true);
    assert!(
        rig.exec.begin_recovery(SharedGuestNetworkComponent::LegF),
        "precondition: the EXEC gate recovers, so the launch's activation waits after its Running row"
    );

    let (tick_result, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(rig.tick(1), async {
            park.entered().await;
            park.release();
            let mut running = false;
            for _ in 0..RACE_POLLS {
                if rig.row(id).await.is_some_and(|row| row.state == AllocState::Running) {
                    running = true;
                    break;
                }
                tokio::time::sleep(RACE_POLL).await;
            }
            assert!(
                running,
                "precondition: the launch won its boot race and wrote its Running row while the evaluation is open"
            );
            rig.state.runtime.supersede_lane(&target());
            tokio::time::sleep(RACE_POLL).await;
            assert!(
                rig.exec.complete_attempt(BTreeSet::new()),
                "the recovering gate reopens so the launch's activation completes"
            );
        })
    })
    .await
    .expect("the launch, the late signal and the reopened gate settle within the bound");

    assert!(
        tick_result.is_ok(),
        "a launch that won its race is an ordinary success: {tick_result:?}"
    );
    assert!(
        !park.observed_supersession(),
        "a start that already returned never observes the late signal"
    );
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1, "one row, the launch's own: {rows:?}");
    assert_eq!(rows[0].alloc_id, alloc(id));
    assert_eq!(rows[0].state, AllocState::Running, "the allocation stays Running");
    assert!(
        !rig.event_names().contains(&"launch.superseded"),
        "no superseded outcome for a launch that won: {:?}",
        rig.event_names()
    );
    assert!(
        rig.view().restart_counts.values().all(|count| *count == 0),
        "no charge: {:?}",
        rig.view().restart_counts
    );
    assert!(rig.has_lease(id), "the running allocation holds its lease");
    assert_eq!(driver.started_specs().len(), 1, "one launch");
    assert!(driver.stop_calls().is_empty(), "nothing stopped yet");

    let stopped = rig
        .dispatch(Action::StopAllocation {
            alloc_id: alloc(id),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        })
        .await;

    assert!(stopped.is_ok(), "the stop of a running allocation succeeds: {stopped:?}");
    let row = rig.row(id).await.expect("the stopped row");
    assert_eq!(row.state, AllocState::Terminated, "an ordinary stop of a running allocation");
    assert_eq!(row.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::Operator }));
    assert_eq!(driver.stop_calls(), [alloc(id)], "the driver stops the running allocation once");
    assert!(!rig.has_lease(id), "no stuck lease after the stop");
    assert_eq!(rig.rows().await.len(), 1, "the stop adds no row");
}
