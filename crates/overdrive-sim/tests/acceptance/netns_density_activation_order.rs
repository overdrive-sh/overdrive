//! S-ND295-53 — activation waits out a shared guest-network recovery and never
//! turns it into a failure (D-295-R5, FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the action-shim order through no Failed row for an observed recovery)).
//!
//! # Contract under test
//!
//! The action shim raises an allocation's TAP only after the protection-live
//! event, and only through the EXEC gate used purely as a wait:
//!
//! - an activation that meets a Recovering gate waits, then runs exactly once
//!   after the gate reopens;
//! - an activation that meets a latched quiescence raises nothing, drops its
//!   claim, waits again, and runs after the reopen;
//! - at FailStop neither activation nor the command is released, no row is
//!   written, and `guest_network.activation_withheld { alloc, reason:
//!   "fail_stop" }` is emitted;
//! - none of these writes a Failed row or advances the restart budget.
//!
//! # Production owner path
//!
//! `dispatch_with_guest_network_provisioner_for_test` drives the real action
//! shim with a `StartAllocation`. The EXEC gate reaches the shim through
//! `AppState` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim, and the `AppState` constructors)); this file's seam fixture keeps the paired
//! `GuestNetworkExecSupervisor`, which only the test moves (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (gate state outside `run_server*`)). No
//! supervisor runs in this lane, so the test is the only caller of
//! `quiesce_managed_taps` and `restore_quiesced_taps`.
//!
//! # Oracle
//!
//! The activation count comes from `SimSharedGuestNetworkOwner::calls()`: the
//! sim records `TapSetUp` for a raised activation and for every restore call,
//! and nothing for a latched activation (FD § "Public deterministic shared-owner simulation API" (the pending `activate` and `restore_quiesced_taps` recording rules)). The test brackets
//! each of its own quiesce and restore calls with `calls().len()`; every
//! `TapSetUp` outside those brackets is an activation. EXEC release and the
//! command hook are observed at the driver port by a recording decorator over
//! `SimDriver`; the withheld event is captured by a test-local tracing
//! `Layer`; the restart budget is read through
//! `ReconcilerRuntime::view_for_workload_lifecycle`.
//!
//! # Faults and schedule
//!
//! Gate transitions come only from the supervisor capability; the latch comes
//! only from the sim owner's `quiesce_managed_taps`. The latched case parks
//! one `activate` call at the driven port (the owner decorator controls
//! ordering only; it authors no lease, row, or decision). Seeds choose the
//! component, the number of parked polls between transitions, whether a
//! quiescence is taken, and how many failed recovery attempts precede the
//! outcome.
//!
//! Reproduce with `OVERDRIVE_ND295_ACTIVATION_SEEDS=<seed>[,<seed>…] cargo
//! xtask lima run -- cargo nextest run -p overdrive-sim --test acceptance
//! --run-ignored ignored-only --no-capture -E 'test(/netns_density_activation_order/)'`.

#![allow(
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    reason = "seeded acceptance harness: every verdict prints its seed; fixture preconditions fail loudly"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::ops::Range;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::Poll;
use std::time::Duration;

use overdrive_control_plane::action_shim::{
    ShimError, dispatch_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::guest_network::{
    GuestAddressPool, GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner,
    Result as GuestNetworkResult, SharedGuestNetworkAudit, SharedGuestNetworkAuditError,
    SharedGuestNetworkOwner, TapActivation, TapQuiescence,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
    SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverType, ExitEvent, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_core::traits::mtls_enforcement::MtlsLimits;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{WorkloadLifecycleView, backoff_for_attempt};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_sim::adapters::{
    SimIdentityRead, SimMtlsEnforcement, SimMtlsIntercept, SimMtlsResolve,
};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

const SEEDS_ENV: &str = "OVERDRIVE_ND295_ACTIVATION_SEEDS";
const DEFAULT_SEEDS: [u64; 4] =
    [0x0295_0053_0000_0001, 0x0295_0053_0000_0002, 0x0295_0053_0000_0003, 0x0295_0053_0000_0004];
/// The Phase-1 single-node baseline id (`workload_lifecycle::baseline_nodes_phase1`).
const NODE: &str = "local";
/// Today's action-pool constants (`guest_network::action_pool`): node guest
/// prefix, bridge, and the gateway that is also the guest DNS address.
const GUEST_PREFIX: &str = "100.95.0.0/16";
const GUEST_BRIDGE: &str = "ovd-gbr0";
const GUEST_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// Bounded poll budget for a dispatch that must complete; each poll yields to
/// the runtime and advances the sim clock by [`COMPLETION_TICK`].
const COMPLETION_POLL_BUDGET: usize = 256;
const COMPLETION_TICK: Duration = Duration::from_millis(5);
/// Bounded poll budget for a dispatch to reach the parked `activate`.
const PARK_POLL_BUDGET: usize = 64;
/// Bounded poll budget for the parked dispatch's Running write to land.
const ROW_POLL_BUDGET: usize = 64;
/// Real time granted between polls that wait for progress, so work the
/// dispatch hands to another thread can finish.
const REAL_TIME_SLICE: Duration = Duration::from_millis(1);
const ACTIVATION_WITHHELD: &str = "guest_network.activation_withheld";
/// Components a supervisor may report as first failing while recovering.
const RECOVERY_COMPONENTS: [SharedGuestNetworkComponent; 5] = [
    SharedGuestNetworkComponent::Bridge,
    SharedGuestNetworkComponent::TcxLink,
    SharedGuestNetworkComponent::EndpointMap,
    SharedGuestNetworkComponent::BridgeGuard,
    SharedGuestNetworkComponent::IpSets,
];

fn seeds() -> Vec<u64> {
    std::env::var(SEEDS_ENV).ok().map_or_else(
        || DEFAULT_SEEDS.to_vec(),
        |raw| {
            raw.split(',')
                .map(|seed| {
                    seed.trim().parse().unwrap_or_else(|_| {
                        panic!("{SEEDS_ENV} must be a comma-separated list of u64 seeds: {raw}")
                    })
                })
                .collect()
        },
    )
}

// ---------------------------------------------------------------------------
// Ordered observation journal (driver port + tracing events + test marks).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Entry {
    /// A structured tracing event, fields rendered as strings.
    Event { name: String, fields: BTreeMap<String, String> },
    /// `Driver::release_for_exit_emission` — the deferred EXEC release.
    ExecRelease(AllocationId),
    /// `Driver::on_alloc_running` — the command hook after release.
    CommandRunning(AllocationId),
    /// The test moved the gate to Open with `complete_attempt(None)`.
    Reopened,
}

#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<Entry>>>);

impl Journal {
    fn push(&self, entry: Entry) {
        self.0.lock().push(entry);
    }

    fn entries(&self) -> Vec<Entry> {
        self.0.lock().clone()
    }

    fn position(&self, pred: impl Fn(&Entry) -> bool) -> Option<usize> {
        self.0.lock().iter().position(pred)
    }
}

struct FieldText<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldText<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

/// Test-local `Layer` recording every `guest_network.*` event into the
/// journal. Thread-local capture suffices: the runtime is current-thread and
/// the test polls the dispatch future itself.
struct EventCapture(Journal);

impl<S> Layer<S> for EventCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldText(&mut fields));
        let name = event.metadata().name();
        let named = if name.starts_with("guest_network.") {
            Some(name.to_owned())
        } else {
            fields
                .get("message")
                .map(|message| message.trim_matches('"').to_owned())
                .filter(|message| message.starts_with("guest_network."))
        };
        if let Some(name) = named {
            self.0.push(Entry::Event { name, fields });
        }
    }
}

fn field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields.get(key).map(|value| value.trim_matches('"').to_owned())
}

// ---------------------------------------------------------------------------
// Driven-port decorators (ordering and observation only).
// ---------------------------------------------------------------------------

struct ActivationPark {
    entered: Arc<AtomicBool>,
    release: oneshot::Receiver<()>,
}

/// Handle the test keeps for one parked `activate` call.
struct ParkHandle {
    entered: Arc<AtomicBool>,
    release: oneshot::Sender<()>,
}

impl ParkHandle {
    fn entered(&self) -> bool {
        self.entered.load(Ordering::SeqCst)
    }

    fn release(self) {
        // The parked call proceeds whether it receives the value or observes
        // the dropped sender; either is a release.
        self.release.send(()).unwrap_or(());
    }
}

/// The one owner instance: `SimSharedGuestNetworkOwner` behind a decorator
/// that can park the next `activate` call before the sim observes its latch.
/// It implements both driven ports by delegation and authors nothing.
struct ActivationOwner {
    inner: SimSharedGuestNetworkOwner,
    park_next_activate: Mutex<Option<ActivationPark>>,
}

impl ActivationOwner {
    fn new() -> Self {
        Self { inner: SimSharedGuestNetworkOwner::default(), park_next_activate: Mutex::new(None) }
    }

    fn calls(&self) -> Vec<GuestNetworkOperation> {
        self.inner.calls()
    }

    fn arm_activation_park(&self) -> ParkHandle {
        let entered = Arc::new(AtomicBool::new(false));
        let (release_tx, release) = oneshot::channel();
        *self.park_next_activate.lock() =
            Some(ActivationPark { entered: Arc::clone(&entered), release });
        ParkHandle { entered, release: release_tx }
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for ActivationOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.inner.provision(plan).await
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<TapActivation> {
        let park = self.park_next_activate.lock().take();
        if let Some(park) = park {
            park.entered.store(true, Ordering::SeqCst);
            park.release.await.unwrap_or(());
        }
        self.inner.activate(plan).await
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.inner.teardown(plan).await
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for ActivationOwner {
    async fn probe_startup(&self) -> GuestNetworkResult<()> {
        self.inner.probe_startup().await
    }

    async fn sweep_stale(&self) -> GuestNetworkResult<()> {
        self.inner.sweep_stale().await
    }

    async fn converge_shared(&self) -> GuestNetworkResult<()> {
        self.inner.converge_shared().await
    }

    async fn audit_shared(&self) -> Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        self.inner.audit_shared().await
    }

    async fn quiesce_managed_taps(&self) -> GuestNetworkResult<TapQuiescence> {
        self.inner.quiesce_managed_taps().await
    }

    async fn restore_quiesced_taps(&self) -> GuestNetworkResult<()> {
        self.inner.restore_quiesced_taps().await
    }
}

/// `SimDriver` behind a decorator recording the EXEC release and the command
/// hook at the driver port. Every other call delegates unchanged.
struct RecordingDriver {
    inner: SimDriver,
    journal: Journal,
}

#[async_trait::async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.journal.push(Entry::ExecRelease(handle.alloc.clone()));
        self.inner.release_for_exit_emission(handle).await;
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

    fn take_exit_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.journal.push(Entry::CommandRunning(spec.alloc.clone()));
        self.inner.on_alloc_running(spec);
    }

    fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        self.inner.on_alloc_terminal(alloc_id);
    }

    fn on_alloc_stable(&self, alloc_id: &AllocationId) {
        self.inner.on_alloc_stable(alloc_id);
    }

    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        self.inner.live_allocations()
    }

    fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
        self.inner.try_begin_reclamation(alloc)
    }

    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

// ---------------------------------------------------------------------------
// Seam fixture (TS § Seam fixture).
// ---------------------------------------------------------------------------

/// One polled step of the parked dispatch future.
#[allow(clippy::large_enum_variant, reason = "a transient verdict of one poll, never stored")]
enum Step {
    Pending,
    Done(Result<(), ShimError>),
    Panicked(String),
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Poll the dispatch once, converting a panic of the system under test into a
/// verdict, then yield so spawned tasks run.
async fn step<F>(dispatch: &mut Pin<Box<F>>) -> Step
where
    F: Future<Output = Result<(), ShimError>>,
{
    let polled = std::future::poll_fn(|cx| {
        Poll::Ready(std::panic::catch_unwind(AssertUnwindSafe(|| dispatch.as_mut().poll(cx))))
    })
    .await;
    tokio::task::yield_now().await;
    match polled {
        Ok(Poll::Ready(outcome)) => Step::Done(outcome),
        Ok(Poll::Pending) => Step::Pending,
        Err(payload) => Step::Panicked(panic_text(payload.as_ref())),
    }
}

/// Every `TapSetUp` outside the test's own quiesce/restore brackets is an
/// activation (TS S-ND295-53 oracle).
fn activations(calls: &[GuestNetworkOperation], brackets: &[Range<usize>]) -> Vec<usize> {
    calls
        .iter()
        .enumerate()
        .filter(|(index, operation)| {
            **operation == GuestNetworkOperation::TapSetUp
                && !brackets.iter().any(|bracket| bracket.contains(index))
        })
        .map(|(index, _)| index)
        .collect()
}

struct Fixture {
    _tmp: TempDir,
    seed: u64,
    state: AppState,
    clock: Arc<SimClock>,
    driver: Arc<RecordingDriver>,
    owner: Arc<ActivationOwner>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    gate: Arc<GuestNetworkExecGate>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    pool: Arc<GuestAddressPool>,
    #[allow(dead_code, reason = "passed to AppState by DELIVER 05-01's pinned constructor")]
    worker: Arc<MtlsInterceptWorker>,
    journal: Journal,
    reconciler: ReconcilerName,
    tick: AtomicU64,
    brackets: Mutex<Vec<Range<usize>>>,
}

impl Fixture {
    async fn compose(seed: u64) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime = ReconcilerRuntime::new(tmp.path(), Arc::new(SimViewStore::new()))
            .expect("reconciler runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let node_id = NodeId::new(NODE).expect("node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id.clone(), seed));
        let clock = Arc::new(SimClock::new());
        let journal = Journal::default();
        let driver = Arc::new(RecordingDriver {
            inner: SimDriver::with_clock(DriverType::Vm, clock.clone()),
            journal: journal.clone(),
        });

        // The one owner instance: the seams' `provisioner` and AppState's owner.
        let owner = Arc::new(ActivationOwner::new());
        // One EXEC wiring over the fixture clock; the gate starts BootClosed
        // and only the kept supervisor moves it.
        let wiring = GuestNetworkExecWiring::new(clock.clone() as Arc<dyn Clock>);
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        // One pool from the doc-hidden constructor with today's constants.
        let pool = Arc::new(GuestAddressPool::new(
            GUEST_PREFIX.parse().expect("static guest prefix"),
            GUEST_BRIDGE.to_owned(),
            GUEST_GATEWAY,
            GUEST_GATEWAY,
        ));
        // One worker over the sim mTLS ports, its shared owner started.
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
            )),
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
            clock.clone() as Arc<dyn Clock>,
            Arc::new(SimMtlsIntercept::new()),
        ));
        worker.start_shared_owner().await.expect("worker shared owner starts over sim ports");

        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
        // DELIVER 05-01 changes this one call: the pinned `AppState::new`
        // appends `worker`, `owner`, `gate`, and `pool` as required parameters
        // (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim, and the `AppState` constructors)). Until then the fixture keeps them for its bodies.
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver.clone() as Arc<dyn Driver>,
            clock.clone() as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
        );
        Self {
            _tmp: tmp,
            seed,
            state,
            clock,
            driver,
            owner,
            supervisor,
            gate,
            pool,
            worker,
            journal,
            reconciler: ReconcilerName::new("workload-lifecycle").expect("reconciler name"),
            tick: AtomicU64::new(0),
            brackets: Mutex::new(Vec::new()),
        }
    }

    fn capture(&self) -> DefaultGuard {
        tracing::subscriber::set_default(
            Registry::default().with(EventCapture(self.journal.clone())),
        )
    }

    fn harness_failure(&self, what: &str) -> ! {
        panic!(
            "seed={}: harness precondition failed (not a contract verdict): {what}\n\
             reproduce: {SEEDS_ENV}={} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
             --test acceptance --run-ignored ignored-only --no-capture \
             -E 'test(/netns_density_activation_order/)'",
            self.seed, self.seed
        );
    }

    fn expect_transition(&self, moved: bool, transition: &str) {
        if !moved {
            self.harness_failure(&format!("supervisor transition {transition} was refused"));
        }
    }

    /// Store and allocator effects of `handlers::submit_workload` for a
    /// Service intent (the intent the restart-budget probe hydrates).
    async fn operator_deploys_service(&self, workload: &str) {
        let service = Service::from_submit(ServiceSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: 256 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/nd295/kernel".to_owned(),
                rootfs: "/nd295/rootfs.ext4".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
        .expect("valid service spec");
        let intent = WorkloadIntent::Service(service.clone());
        let archived = intent.archive_for_store().expect("archive workload intent");
        let digest = intent.spec_digest().expect("spec digest");
        if let Ok(name) =
            MeshServiceName::new(&format!("{}.{}", service.id.as_str(), MeshServiceName::SUFFIX))
        {
            self.state.frontend_addr_allocator.assign(&name).expect("frontend address");
        }
        let vip = {
            let mut guard = self.state.allocator.lock().await;
            guard.allocate(*digest.as_bytes()).await.expect("service vip")
        };
        let key = IntentKey::for_workload(&service.id);
        let outcome = self
            .state
            .store
            .put_if_absent(key.as_bytes(), archived.as_ref())
            .await
            .expect("intent write");
        if !matches!(outcome, PutOutcome::Inserted) {
            self.harness_failure(&format!("service {workload} was not fresh"));
        }
        let kind_key = IntentKey::for_workload_kind(&service.id);
        self.state
            .store
            .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
            .await
            .expect("kind write");
        self.state.listener_facts.lock().await.upsert(service.id.clone(), &vip, &service.listeners);
    }

    fn start_action(workload: &str, alloc: &AllocationId) -> Action {
        let spec = AllocationSpec {
            alloc: alloc.clone(),
            identity: SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/{workload}/alloc/{alloc}"
            ))
            .expect("SPIFFE ID"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: PathBuf::from("/nd295/kernel"),
                rootfs: PathBuf::from("/nd295/rootfs.ext4"),
            }),
            resources: Resources { cpu_milli: 0, memory_bytes: 256 * 1024 },
            probe_descriptors: Vec::new(),
            network: None,
            service_ports: vec![NonZeroU16::new(8080).expect("non-zero listener port")],
        };
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: WorkloadId::new(workload).expect("workload id"),
            node_id: NodeId::new(NODE).expect("node id"),
            spec,
            kind: WorkloadKind::Service,
        }
    }

    fn tick_context(&self) -> TickContext {
        let now = self.clock.now();
        TickContext {
            now,
            now_unix: UnixInstant::from_clock(self.clock.as_ref()),
            tick: self.tick.fetch_add(1, Ordering::SeqCst),
            deadline: now + Duration::from_secs(1),
        }
    }

    /// The production action owner with only the guest-network driven port
    /// substituted by the one owner instance.
    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        let tick = self.tick_context();
        dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            &self.state,
            &tick,
            self.owner.as_ref(),
        )
        .await
    }

    /// One production `workload-lifecycle` evaluation for `workload`.
    async fn converge(&self, workload: &str) -> Result<(), String> {
        let n = self.tick.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now();
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &self.reconciler,
            &target,
            now,
            n,
            now + Duration::from_secs(1),
            self.owner.as_ref(),
        ))
        .await
        .map_err(|error| format!("{error:?}"))
    }

    fn view(&self, workload: &str) -> WorkloadLifecycleView {
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        self.state.runtime.view_for_workload_lifecycle(&target)
    }

    async fn row(&self, alloc: &AllocationId) -> Option<AllocStatusRow> {
        self.state.obs.alloc_status_row(alloc).await.expect("alloc status row read")
    }

    /// The test's own quiescence call, bracketed in the sim owner's log.
    async fn quiesce(&self) -> GuestNetworkResult<TapQuiescence> {
        let start = self.owner.calls().len();
        let outcome = self.owner.quiesce_managed_taps().await;
        self.brackets.lock().push(start..self.owner.calls().len());
        outcome
    }

    /// The test's own restore call, bracketed in the sim owner's log.
    async fn restore(&self) -> GuestNetworkResult<()> {
        let start = self.owner.calls().len();
        let outcome = self.owner.restore_quiesced_taps().await;
        self.brackets.lock().push(start..self.owner.calls().len());
        outcome
    }

    fn activations(&self) -> Vec<usize> {
        activations(&self.owner.calls(), &self.brackets.lock())
    }

    /// Poll the parked dispatch `polls` times while the gate must not let the
    /// activation through; any activation, completion, or panic is a verdict.
    async fn wait_parked<F>(&self, dispatch: &mut Pin<Box<F>>, polls: usize, while_: &str)
    where
        F: Future<Output = Result<(), ShimError>>,
    {
        for poll in 0..polls {
            let outcome = step(dispatch).await;
            let activations = self.activations();
            assert!(
                activations.is_empty(),
                "seed={}: activation ran while {while_} (poll {poll}): TapSetUp at owner call \
                 indices {activations:?} outside the test's quiesce/restore brackets {:?}; calls \
                 {:?}. The activation must wait on the EXEC gate until it reopens (FD § \"[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24\" (the action-shim order and the EXEC-gate wait))",
                self.seed,
                self.brackets.lock(),
                self.owner.calls(),
            );
            match outcome {
                Step::Pending => {}
                Step::Done(result) => panic!(
                    "seed={}: the dispatch completed while {while_} (poll {poll}) with \
                     {result:?}; it must wait on the EXEC gate (FD § \"[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24\" (the action-shim order and the EXEC-gate wait))",
                    self.seed
                ),
                Step::Panicked(message) => panic!(
                    "seed={}: the dispatch panicked while {while_} (poll {poll}): {message}",
                    self.seed
                ),
            }
        }
    }

    /// Poll the dispatch until it completes, within a bounded budget.
    async fn run_to_completion<F>(
        &self,
        dispatch: &mut Pin<Box<F>>,
        after: &str,
    ) -> Result<(), ShimError>
    where
        F: Future<Output = Result<(), ShimError>>,
    {
        for poll in 0..COMPLETION_POLL_BUDGET {
            match step(dispatch).await {
                Step::Pending => {
                    self.clock.tick(COMPLETION_TICK);
                    tokio::time::sleep(REAL_TIME_SLICE).await;
                }
                Step::Done(result) => return result,
                Step::Panicked(message) => panic!(
                    "seed={}: the dispatch panicked {after} (poll {poll}): {message}",
                    self.seed
                ),
            }
        }
        panic!(
            "seed={}: the dispatch did not complete within {COMPLETION_POLL_BUDGET} polls {after}; \
             owner calls {:?}; journal {:?}",
            self.seed,
            self.owner.calls(),
            self.journal.entries(),
        );
    }

    fn exec_releases(&self, alloc: &AllocationId) -> Vec<usize> {
        self.journal
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, Entry::ExecRelease(released) if released == alloc))
            .map(|(index, _)| index)
            .collect()
    }

    fn command_hooks(&self, alloc: &AllocationId) -> usize {
        self.journal
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::CommandRunning(running) if running == alloc))
            .count()
    }

    fn withheld_events(&self) -> Vec<BTreeMap<String, String>> {
        self.journal
            .entries()
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Event { name, fields } if name == ACTIVATION_WITHHELD => Some(fields),
                _ => None,
            })
            .collect()
    }

    /// The single activation happened after the reopen, the EXEC release and
    /// the command followed it once, and the row is Running.
    async fn assert_single_activation_after_reopen(&self, alloc: &AllocationId, reopen_at: usize) {
        let activations = self.activations();
        assert!(
            activations.len() == 1 && activations[0] >= reopen_at,
            "seed={}: expected exactly one activation (TapSetUp outside the test's brackets), \
             after the reopen at owner call index {reopen_at}; got {activations:?}; calls {:?}; \
             brackets {:?}",
            self.seed,
            self.owner.calls(),
            self.brackets.lock(),
        );
        let reopened =
            self.journal.position(|entry| matches!(entry, Entry::Reopened)).unwrap_or(usize::MAX);
        let releases = self.exec_releases(alloc);
        assert!(
            releases.len() == 1 && releases[0] > reopened,
            "seed={}: expected exactly one EXEC release for {alloc}, after the reopen (journal \
             index {reopened}); got journal indices {releases:?}; journal {:?}",
            self.seed,
            self.journal.entries(),
        );
        assert_eq!(
            self.command_hooks(alloc),
            1,
            "seed={}: the command hook runs once for {alloc} after its activation",
            self.seed
        );
        let row = self.row(alloc).await;
        assert_eq!(
            row.as_ref().map(|row| row.state),
            Some(AllocState::Running),
            "seed={}: an observed recovery never writes a Failed row for {alloc}: {row:?}",
            self.seed
        );
    }

    /// Two production evaluations, one backoff window apart, leave the
    /// restart budget where it started and dispatch no restart.
    async fn assert_restart_budget_unchanged(
        &self,
        workload: &str,
        alloc: &AllocationId,
        before: &WorkloadLifecycleView,
    ) {
        for evaluation in 0..2 {
            if let Err(error) = self.converge(workload).await {
                panic!(
                    "seed={}: restart-budget evaluation {evaluation} of {workload} failed: {error}",
                    self.seed
                );
            }
            self.clock.tick(backoff_for_attempt(0) * 2);
        }
        let after = self.view(workload);
        assert_eq!(
            after.restart_counts, before.restart_counts,
            "seed={}: an observed recovery must not advance the restart budget of {workload} \
             (WorkloadLifecycleView.restart_counts)",
            self.seed
        );
        assert_eq!(
            after.last_failure_seen_at, before.last_failure_seen_at,
            "seed={}: an observed recovery must not record a workload failure for {workload}",
            self.seed
        );
        let prefix = format!("alloc-{workload}-");
        let started: Vec<AllocationId> = self
            .driver
            .inner
            .started_specs()
            .into_iter()
            .map(|spec| spec.alloc)
            .filter(|started| started.as_str().starts_with(&prefix))
            .collect();
        assert_eq!(
            started,
            vec![alloc.clone()],
            "seed={}: no restart successor is started for {workload}",
            self.seed
        );
    }
}

fn workload_for(prefix: &str, seed: u64) -> String {
    format!("{prefix}-{seed:016x}")
}

fn first_alloc(workload: &str) -> AllocationId {
    AllocationId::new(&format!("alloc-{workload}-0")).expect("allocation id")
}

// ---------------------------------------------------------------------------
// Bodies.
// ---------------------------------------------------------------------------

async fn activation_during_recovery(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    let workload = workload_for("a53", seed);
    fixture.operator_deploys_service(&workload).await;
    let alloc = first_alloc(&workload);
    let before = fixture.view(&workload);

    let component = RECOVERY_COMPONENTS[rng.gen_range(0..RECOVERY_COMPONENTS.len())];
    let quiesce = rng.gen_bool(0.5);
    let failed_attempts = rng.gen_range(0..=2_usize);
    eprintln!(
        "seed={seed} S-ND295-53 recovery: component={component:?} quiesce={quiesce} \
         failed_attempts={failed_attempts}"
    );

    fixture.expect_transition(fixture.supervisor.open_after_boot(), "open_after_boot");
    fixture.expect_transition(fixture.supervisor.begin_recovery(component), "begin_recovery");
    let mut dispatch = Box::pin(fixture.dispatch(Fixture::start_action(&workload, &alloc)));
    fixture.wait_parked(&mut dispatch, rng.gen_range(1..=4), "the gate is Recovering").await;
    if quiesce {
        let quiescence = fixture.quiesce().await;
        assert!(
            quiescence.as_ref().is_ok_and(|result| result.unconfirmed.is_empty()),
            "seed={seed}: the scripted full quiescence succeeds: {quiescence:?}"
        );
        fixture
            .wait_parked(&mut dispatch, rng.gen_range(1..=3), "the gate is Recovering and latched")
            .await;
    }
    for _ in 0..failed_attempts {
        fixture.expect_transition(
            fixture.supervisor.complete_attempt(Some(component)),
            "complete_attempt(Some)",
        );
        fixture
            .wait_parked(
                &mut dispatch,
                rng.gen_range(1..=3),
                "a recovery attempt left the gate Recovering",
            )
            .await;
    }
    if quiesce {
        let restored = fixture.restore().await;
        assert!(restored.is_ok(), "seed={seed}: the scripted restore succeeds: {restored:?}");
        fixture
            .wait_parked(
                &mut dispatch,
                rng.gen_range(1..=3),
                "the latch is clear but the gate is Recovering",
            )
            .await;
    }

    let reopen_at = fixture.owner.calls().len();
    fixture.expect_transition(fixture.supervisor.complete_attempt(None), "complete_attempt(None)");
    fixture.journal.push(Entry::Reopened);
    let outcome = fixture.run_to_completion(&mut dispatch, "after the gate reopened").await;
    assert!(outcome.is_ok(), "seed={seed}: the start completes after the reopen: {outcome:?}");

    fixture.assert_single_activation_after_reopen(&alloc, reopen_at).await;
    fixture.assert_restart_budget_unchanged(&workload, &alloc, &before).await;
    eprintln!("seed={seed} S-ND295-53 recovery: GREEN");
}

async fn latched_activation_retries(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    let workload = workload_for("b53", seed);
    fixture.operator_deploys_service(&workload).await;
    let alloc = first_alloc(&workload);
    let before = fixture.view(&workload);

    let component = RECOVERY_COMPONENTS[rng.gen_range(0..RECOVERY_COMPONENTS.len())];
    let failed_attempts = rng.gen_range(0..=2_usize);
    eprintln!(
        "seed={seed} S-ND295-53 latched: component={component:?} failed_attempts={failed_attempts}"
    );

    // The gate is Open, so the activation claims it and reaches the owner;
    // the owner parks that call before the sim reads its latch.
    fixture.expect_transition(fixture.supervisor.open_after_boot(), "open_after_boot");
    let park = fixture.owner.arm_activation_park();
    let mut dispatch = Box::pin(fixture.dispatch(Fixture::start_action(&workload, &alloc)));
    let mut parked = false;
    for poll in 0..PARK_POLL_BUDGET {
        match step(&mut dispatch).await {
            Step::Pending if park.entered() => {
                parked = true;
                break;
            }
            Step::Pending => tokio::time::sleep(REAL_TIME_SLICE).await,
            Step::Done(result) => fixture.harness_failure(&format!(
                "the dispatch completed before reaching activate (poll {poll}): {result:?}"
            )),
            Step::Panicked(message) => fixture.harness_failure(&format!(
                "the dispatch panicked before reaching activate (poll {poll}): {message}"
            )),
        }
    }
    if !parked {
        fixture.harness_failure("the dispatch never reached the parked activate call");
    }

    // Recovery begins and the latch is set while that activation is in flight.
    fixture.expect_transition(fixture.supervisor.begin_recovery(component), "begin_recovery");
    let quiescence = fixture.quiesce().await;
    assert!(
        quiescence.as_ref().is_ok_and(|result| result.unconfirmed.is_empty()),
        "seed={seed}: the scripted full quiescence succeeds: {quiescence:?}"
    );
    park.release();
    fixture
        .wait_parked(
            &mut dispatch,
            rng.gen_range(1..=4),
            "the activation met a latched quiescence and the gate is Recovering",
        )
        .await;
    for _ in 0..failed_attempts {
        fixture.expect_transition(
            fixture.supervisor.complete_attempt(Some(component)),
            "complete_attempt(Some)",
        );
        fixture
            .wait_parked(
                &mut dispatch,
                rng.gen_range(1..=3),
                "a recovery attempt left the gate Recovering",
            )
            .await;
    }
    let restored = fixture.restore().await;
    assert!(restored.is_ok(), "seed={seed}: the scripted restore succeeds: {restored:?}");
    fixture
        .wait_parked(
            &mut dispatch,
            rng.gen_range(1..=3),
            "the latch is clear but the gate is Recovering",
        )
        .await;

    let reopen_at = fixture.owner.calls().len();
    fixture.expect_transition(fixture.supervisor.complete_attempt(None), "complete_attempt(None)");
    fixture.journal.push(Entry::Reopened);
    let outcome = fixture.run_to_completion(&mut dispatch, "after the gate reopened").await;
    assert!(outcome.is_ok(), "seed={seed}: the start completes after the reopen: {outcome:?}");

    fixture.assert_single_activation_after_reopen(&alloc, reopen_at).await;
    fixture.assert_restart_budget_unchanged(&workload, &alloc, &before).await;
    eprintln!("seed={seed} S-ND295-53 latched: GREEN");
}

async fn fail_stop_withholds(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let fixture = Fixture::compose(seed).await;
    let _capture = fixture.capture();
    let workload = workload_for("c53", seed);
    fixture.operator_deploys_service(&workload).await;
    let alloc = first_alloc(&workload);
    let before = fixture.view(&workload);

    let component = RECOVERY_COMPONENTS[rng.gen_range(0..RECOVERY_COMPONENTS.len())];
    let quiesce = rng.gen_bool(0.5);
    let failed_restore = quiesce && rng.gen_bool(0.5);
    let failed_attempts = rng.gen_range(0..=2_usize);
    eprintln!(
        "seed={seed} S-ND295-53 fail-stop: component={component:?} quiesce={quiesce} \
         failed_restore={failed_restore} failed_attempts={failed_attempts}"
    );

    fixture.expect_transition(fixture.supervisor.open_after_boot(), "open_after_boot");
    fixture.expect_transition(fixture.supervisor.begin_recovery(component), "begin_recovery");
    let mut dispatch = Box::pin(fixture.dispatch(Fixture::start_action(&workload, &alloc)));
    fixture.wait_parked(&mut dispatch, rng.gen_range(1..=4), "the gate is Recovering").await;
    // The durable Running write precedes the activation wait (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the action-shim order));
    // keep the dispatch parked until that write is visible.
    let mut parked_row = fixture.row(&alloc).await;
    for _ in 0..ROW_POLL_BUDGET {
        if parked_row.as_ref().is_some_and(|row| row.state == AllocState::Running) {
            break;
        }
        fixture.wait_parked(&mut dispatch, 1, "the gate is Recovering").await;
        tokio::time::sleep(REAL_TIME_SLICE).await;
        parked_row = fixture.row(&alloc).await;
    }
    assert_eq!(
        parked_row.as_ref().map(|row| row.state),
        Some(AllocState::Running),
        "seed={seed}: the Running row precedes the activation wait: {parked_row:?}"
    );
    if quiesce {
        let quiescence = fixture.quiesce().await;
        assert!(
            quiescence.as_ref().is_ok_and(|result| result.unconfirmed.is_empty()),
            "seed={seed}: the scripted full quiescence succeeds: {quiescence:?}"
        );
        fixture
            .wait_parked(&mut dispatch, rng.gen_range(1..=3), "the gate is Recovering and latched")
            .await;
    }
    for _ in 0..failed_attempts {
        fixture.expect_transition(
            fixture.supervisor.complete_attempt(Some(component)),
            "complete_attempt(Some)",
        );
        fixture
            .wait_parked(
                &mut dispatch,
                rng.gen_range(1..=3),
                "a recovery attempt left the gate Recovering",
            )
            .await;
    }
    if failed_restore {
        fixture.owner.inner.script_restore_failure(true);
        let restored = fixture.restore().await;
        assert!(restored.is_err(), "seed={seed}: the scripted restore failure: {restored:?}");
        fixture
            .wait_parked(&mut dispatch, rng.gen_range(1..=3), "a failed restore kept the latch")
            .await;
    }

    let receipt =
        fixture.supervisor.fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
    if receipt.is_none() {
        fixture.harness_failure("fail_stop returned no receipt");
    }
    let outcome = fixture.run_to_completion(&mut dispatch, "after fail-stop").await;
    assert!(
        outcome.is_ok(),
        "seed={seed}: at fail-stop the start arm withholds and returns Ok: {outcome:?}"
    );

    let activations = fixture.activations();
    assert!(
        activations.is_empty(),
        "seed={seed}: no activation at fail-stop; TapSetUp outside the brackets at {activations:?}; \
         calls {:?}",
        fixture.owner.calls()
    );
    let releases = fixture.exec_releases(&alloc);
    assert!(
        releases.is_empty() && fixture.command_hooks(&alloc) == 0,
        "seed={seed}: no EXEC release and no command at fail-stop; journal {:?}",
        fixture.journal.entries()
    );
    let withheld = fixture.withheld_events();
    assert!(
        withheld.len() == 1
            && field(&withheld[0], "alloc").is_some_and(|value| value.contains(alloc.as_str()))
            && field(&withheld[0], "reason").as_deref() == Some("fail_stop"),
        "seed={seed}: exactly one {ACTIVATION_WITHHELD} {{ alloc: {alloc}, reason: \"fail_stop\" }}; \
         captured {withheld:?}"
    );
    let row = fixture.row(&alloc).await;
    assert_eq!(
        row, parked_row,
        "seed={seed}: fail-stop writes no row for {alloc} after the Running write"
    );
    fixture.assert_restart_budget_unchanged(&workload, &alloc, &before).await;
    eprintln!("seed={seed} S-ND295-53 fail-stop: GREEN");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-53 — Activation waits out a recovery and never turns it into a failure.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 06-04 (S-ND295-53)"]
async fn activation_during_recovery_runs_once_after_reopen_without_a_failed_row() {
    for seed in seeds() {
        eprintln!(
            "seed={seed} body=activation_during_recovery_runs_once_after_reopen_without_a_failed_row"
        );
        activation_during_recovery(seed).await;
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-53 — Activation waits out a recovery and never turns it into a failure.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 06-04 (S-ND295-53)"]
async fn a_latched_activation_retries_after_reopen() {
    for seed in seeds() {
        eprintln!("seed={seed} body=a_latched_activation_retries_after_reopen");
        latched_activation_retries(seed).await;
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-53 — Activation waits out a recovery and never turns it into a failure.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 06-04 (S-ND295-53)"]
async fn fail_stop_withholds_activation_and_the_command() {
    for seed in seeds() {
        eprintln!("seed={seed} body=fail_stop_withholds_activation_and_the_command");
        fail_stop_withholds(seed).await;
    }
}
