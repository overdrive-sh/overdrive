//! Quint conformance driver — `restart_generation`, conformance mains `rg_conf` (the eight
//! service runs) and `rg_conf_job` (the two finished-Job runs); feature delta § "Observable
//! projections the conformance driver reads" (the restart-generation table) and § "Formal protocol
//! model".
//!
//! Replays `specs/quint/restart-generation/restart_generation.qnt` (referenced in place) against
//! the production composition of the restart authority: the operator handlers (`submit_workload`,
//! `stop_workload`, `restart_workload`), the `WorkloadLifecycle` reconciler driven through the
//! runtime's convergence tick, the real action shim (admission, identity issuance, driver start),
//! the production `GuestAddressPool`, `IdentityMgr` and exit observer, over `SimDriver`,
//! `SimObservationStore` and the shared guest-network owner double. The driver writes no row,
//! intent, View entry or sim state the production path writes itself.
//!
//! # The held launch
//!
//! The model's launch is one held dispatch: `WlStart(a)` issues allocation `a` and a later action
//! resolves it **by the environment flags at that instant, in the shim's order: admission, then
//! identity issuance, then the driver**. Holding the dispatch at the driver would already have
//! passed admission and identity at `WlStart`, so a `CapFill` or `IdFaultOn` between the two
//! could never produce `LaunchRefused` or `LaunchPreFail`. The test-owned seam is therefore the
//! observation store's point read `alloc_status_row`: it is the first thing either launch arm does
//! (`find_prior_alloc_row`), after the reconciler's hydration (which reads `alloc_status_rows`,
//! never the point read) and the View write that records the issued id, and before admission.
//! `WlStart` arms a one-shot hold on that read (`HoldObs`, a delegating `ObservationStore`
//! double: a driven port), spawns the convergence tick and waits for the dispatch to be parked
//! there. A resolution action arms the sim for its outcome, releases the hold and lets the real
//! dispatch run to its end. Only `LaunchLost` also parks `SimDriver::start`
//! (`park_next_start(true)`), because the lane signal is observed by the driver's wait.
//!
//! | Action | Owner call or environment stimulus |
//! |---|---|
//! | `Init` | a fresh node and a `serve`-shaped composition over it |
//! | `Declare` | `submit_workload` (a Service, or a Job on `rg_conf_job`) |
//! | `OpStop` | `stop_workload` |
//! | `OpResume` | `restart_workload` (`Resumed`) |
//! | `WorkloadRestart(a)` | `restart_workload` (`Restarted`), then one convergence tick when `a` is `Running` (the tick emits the stop) |
//! | `WlStart(a)` | one convergence tick, parked before the action shim's admission |
//! | `LaunchRunning(a)` / `LaunchPreFail(a)` / `LaunchRefused(a)` | release the held dispatch; the sim CA flag and the sim guest pool are as `IdFault*` / `Cap*` left them |
//! | `LaunchStartFail(a)` | arm the next `Driver::start` to fail with a typed start failure, release the held dispatch |
//! | `LaunchLost(a)` | park `SimDriver::start` observing the lane signal, release the held dispatch, `ReconcilerRuntime::supersede_lane` |
//! | `StopAlloc(a)` | the convergence tick that emits `StopAllocation` and its dispatch |
//! | `VmmCrash(a)` | `SimDriver::inject_exit_after`; the production exit observer writes the `Failed` row |
//! | `JobExit(a)` | `SimDriver::inject_exit_after` (a clean exit); the exit observer writes the row |
//! | `CpCrash` | the `serve` runtime is dropped (every task dies with it, the held tick included), the next composition reopens the stores and runs boot reclamation |
//! | `NodeReboot` | every `Running` allocation's process exits, then as `CpCrash` |
//! | `IdFaultOn` / `IdFaultOff` | the sim CA's `issue_svid` fails / succeeds |
//! | `CapFill` / `CapDrain` | fill the sim guest pool to its cap through its own admission / release the fillers |
//!
//! After an action that ends an allocation (`VmmCrash`, `JobExit`, the launch resolutions) the
//! driver runs one further convergence tick **only when the owner's own decision
//! (`ReconcilerRuntime::restart_status_for_alloc`) says the ended row will not be restarted**:
//! that evaluation is the one that finalises `BackoffExhausted` or a Job's terminal claim, which
//! the model's `exhausted` and `jobTerm` read. When a restart would follow, no tick runs, so the
//! next `WlStart` stays the placement.
//!
//! # Projection (feature delta § Observable projections)
//!
//! | Model variable | Production accessor |
//! |---|---|
//! | `dg` | `WorkloadLifecycle::hydrate_desired(..).generation` |
//! | `stopped` | `WorkloadLifecycle::hydrate_desired(..).desired_to_stop` |
//! | `minted[a]`, `ig[a]` | `ReconcilerRuntime::issued_generation_for_alloc` |
//! | `cnt[a]` | `ReconcilerRuntime::attempt_index_for_alloc - 1` |
//! | `basisA` (the count basis, RB-13) | the newest row that is not `is_launch_abandoned`, derived from `alloc_status_rows` by the projection |
//! | `hasRow(a)`, the row's state | `ObservationStore::alloc_status_rows` |
//! | `exhausted` | `alloc_status_rows`: the current row carries `BackoffExhausted` |
//! | `why == W_PREFAIL` | a `Failed` row with an identity-issue or driver-internal reason |
//! | `why == W_REFUSED` | no row, not live in the driver, the dispatch's own `Err(AdmissionCapReached)` |
//! | `as == ABANDONED` (`why == W_LOST`) | a `Terminated` row with `terminal == None` and reason `LaunchSuperseded` or `LaunchLostWithControlPlane` (ADR-0355), not live in the driver, the held tick over (completed `Ok`, or dropped by a rebuild). The row is written by the shim and read through `alloc_status_rows()`; a launch that ended with no such row fails the projection |
//! | `as == PENDING` | issued, the `Pending` row of the allocation (ADR-0353), the held tick future has not completed. The shim writes the row after admission, so the held launch (parked before admission) has none yet: the projection awaits a hold after that write |
//! | `jobTerm(a)` | a `kind == Job` row with a `terminal` |
//!
//! `faults`, `declared`, `idFault`, `capFull`, `conf_step`, `og` and `gl` are model-only.
//!
//! **Status: pending.** The driver is compiled. A run whose trace reaches behaviour DELIVER has
//! not built carries `#[should_panic(expected = ...)]` naming its own divergence; a run that
//! passes against production today is a GUARD and carries none. See each test's doc. Every
//! `rg_conf` / `rg_conf_job` run reaches a `PENDING` allocation, which the hold parks before the shim's
//! admission, while the `Pending` row (ADR-0353) is written after it, so every pin carries
//! `#[should_panic(expected = "awaits owner API")]` until a hold after that write exists. The derived
//! `charged` and `exhausted` read the count basis (RB-13), and an abandoned row projects as state 5.
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through quint-connect).

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    clippy::too_many_arguments,
    reason = "a conformance driver asserts its own preconditions; CONTRACT_SHAPE lines are machine-read"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::significant_drop_tightening,
    clippy::type_complexity,
    reason = "test support and conformance drivers: handler signatures follow the action map"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State as AxumState};
use axum::http::HeaderMap;
use itf::Value;
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput, WorkloadKind};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::guest_network::{GuestNetworkExecWiring, MAX_GUEST_NETWORK_ATTACHMENTS};
use overdrive_core::id::{AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, PublicCertifiedKeyId, PublicCertifiedKeyStatusRowV1,
};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
    DriverStartFailure, DriverType, ExitKind, LaunchSupersession, Resources, VmStartFailure,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, LagAwareSubscription, NodeHealthRow,
    ObservationStore, ObservationStoreError, ObservationWrite, ServiceBackendRow, TransitionSource,
    WorkloadSvidIssueStatusRowV1,
};
use overdrive_core::transition_reason::{TerminalCondition, TransitionReason, is_launch_abandoned};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_reconcilers::AnyState;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::{SimDriver, StartPark};
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_testing::placement;
use parking_lot::Mutex;
use quint_connect::{Config, Driver as QuintDriver, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::AppState;
use crate::action_shim::ShimError;
use crate::api::{RestartOutcome, SubmitWorkloadRequest};
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::guest_network::{GuestAddressPool, GuestNetworkError, SharedGuestNetworkOwner as _};
use crate::handlers::{restart_workload, stop_workload, submit_workload};
use crate::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, hydrate_desired_for_test,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use crate::shared_network_test_ports::{
    TestSharedOwner, guest_pool, mtls_worker, shared_guest_network_owner, test_owner_flow_ports,
};
use crate::vm_reclamation_boot::{ConvergeError, converge};

placement!(host_pure);

/// The main whose workload is a Job, and the main of every service run; both declare their
/// variables in the module `restart_generation`.
const MAIN_SERVICE: &str = "rg_conf";
const MAIN_JOB: &str = "rg_conf_job";
const MODULE: &str = "restart_generation";
/// The instance's allocation ids (`NA`) and its failure limit (`LIMIT`).
const NA: i64 = 4;
const LIMIT: i64 = 3;
/// The one workload of the model.
const WORKLOAD: &str = "rg-w";
const NODE: &str = "rg-node";
/// The `SimClock` step before each tick: past every restart backoff window.
const PAST_BACKOFF: Duration = Duration::from_secs(3_600);
/// How long a step may take to settle (real time; owners read `SimClock`).
const SETTLE_BOUND: Duration = Duration::from_secs(10);

/// Why a driver step cannot proceed. Every message names the step, so a pending run's
/// `should_panic` pins the step it diverges at.
#[derive(Debug, thiserror::Error)]
enum RgError {
    #[error("restart-generation divergence at step {step} {action}: {why}")]
    Step { step: u64, action: &'static str, why: String },
    #[error("restart-generation divergence at step {step} projection: {why}")]
    Projection { step: u64, why: String },
    /// The driver has no seam for what the model now says: the step names it.
    #[error(
        "restart-generation step {step} awaits owner API that does not exist yet (ADR-0353, ADR-0355): {awaits}"
    )]
    OwnerApiPending { step: u64, awaits: &'static str },
    #[error("ITF: {0}")]
    Itf(#[from] ItfError),
}

/// The step-less failure of a handler body; `RgCore::at` adds the step and the action.
#[derive(Debug)]
struct Why(String);

impl<T: std::fmt::Display> From<T> for Why {
    fn from(value: T) -> Self {
        Self(value.to_string())
    }
}

fn why(text: impl Into<String>) -> Why {
    Why(text.into())
}

fn target() -> TargetResource {
    TargetResource::new(&format!("workload/{WORKLOAD}")).expect("a valid target")
}

fn reconciler_name() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("a valid reconciler name")
}

fn node_id() -> NodeId {
    NodeId::new(NODE).expect("a valid node id")
}

/// The production allocation id of the `n`th attempt (`mint_alloc_id`'s grammar).
fn candidate(n: i64) -> AllocationId {
    AllocationId::new(&format!("alloc-{WORKLOAD}-{n}")).expect("a valid allocation id")
}

// ----- driven-port doubles ---------------------------------------------------------

/// A hold on the next point read: `entered` rises when a read parks, `released` lets it go.
struct Hold {
    entered: watch::Sender<bool>,
    released: watch::Sender<bool>,
}

impl Hold {
    fn has_entered(&self) -> bool {
        *self.entered.borrow()
    }

    fn release(&self) {
        self.released.send_replace(true);
    }
}

/// Delegating `ObservationStore` over `SimObservationStore` with one scripted fault at the driven
/// port: a one-shot hold on the next point read `alloc_status_row`, the first thing a launch arm
/// of the action shim does (before admission).
struct HoldObs {
    inner: Arc<SimObservationStore>,
    hold: Mutex<Option<Arc<Hold>>>,
}

impl HoldObs {
    fn over(inner: Arc<SimObservationStore>) -> Arc<Self> {
        Arc::new(Self { inner, hold: Mutex::new(None) })
    }

    fn arm(&self) -> Arc<Hold> {
        let (entered, _) = watch::channel(false);
        let (released, _) = watch::channel(false);
        let hold = Arc::new(Hold { entered, released });
        *self.hold.lock() = Some(Arc::clone(&hold));
        hold
    }
}

#[async_trait]
impl ObservationStore for HoldObs {
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
        failure: overdrive_core::ca::issue_failure::SvidIssueFailure,
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
        let hold = self.hold.lock().take();
        if let Some(hold) = hold {
            hold.entered.send_replace(true);
            let mut released = hold.released.subscribe();
            if released.wait_for(|released| *released).await.is_err() {
                // the hold was dropped without a release: the read proceeds
            }
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

/// `Ca` double over `SimCa`: `issue_svid` fails while armed (the model's `idFault`). A driven-port
/// double: `SimCa` has no failure scripting.
struct ToggleCa {
    inner: SimCa,
    fail: AtomicBool,
}

impl Ca for ToggleCa {
    fn root(&self) -> std::result::Result<RootCaHandle, CaError> {
        self.inner.root()
    }

    fn issue_intermediate(
        &self,
        node: &NodeId,
    ) -> std::result::Result<IntermediateHandle, CaError> {
        self.inner.issue_intermediate(node)
    }

    fn issue_svid(&self, req: &SvidRequest) -> std::result::Result<SvidMaterial, CaError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(CaError::SigningFailed { reason: "rg-identity-fault".to_owned() });
        }
        self.inner.issue_svid(req)
    }

    fn trust_bundle(&self) -> std::result::Result<TrustBundle, CaError> {
        self.inner.trust_bundle()
    }
}

/// `Driver` double over `SimDriver`: the next `start` fails with a typed start failure while
/// armed (the model's `LaunchStartFail`: a start failure that is not the driver-internal
/// fallback, so its row is distinguishable from a pre-driver failure's).
struct OutcomeDriver {
    inner: Arc<SimDriver>,
    fail_next_start: AtomicBool,
}

#[async_trait]
impl Driver for OutcomeDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(
        &self,
        spec: &AllocationSpec,
        supersession: &dyn LaunchSupersession,
    ) -> std::result::Result<AllocationHandle, DriverError> {
        if self.fail_next_start.swap(false, Ordering::SeqCst) {
            return Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::KernelNotFound {
                        path: "/rg/kernel".to_owned(),
                    }),
                    detail: "rg-typed-start-failure".to_owned(),
                },
            });
        }
        self.inner.start(spec, supersession).await
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle).await;
    }

    async fn stop(&self, handle: &AllocationHandle) -> std::result::Result<(), DriverError> {
        self.inner.stop(handle).await
    }

    async fn status(
        &self,
        handle: &AllocationHandle,
    ) -> std::result::Result<AllocationState, DriverError> {
        self.inner.status(handle).await
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> std::result::Result<(), DriverError> {
        self.inner.resize(handle, resources).await
    }

    fn take_exit_receiver(
        &self,
    ) -> Option<tokio::sync::mpsc::Receiver<overdrive_core::traits::driver::ExitEvent>> {
        self.inner.take_exit_receiver()
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
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

// ----- the node and its serve process -----------------------------------------------

/// What outlives a `serve` process: the node's stores, clock and the environment flags.
struct Node {
    tmp: TempDir,
    clock: Arc<SimClock>,
    obs: Arc<HoldObs>,
    ca: Arc<ToggleCa>,
}

impl Node {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        Self {
            tmp,
            clock: Arc::new(SimClock::new()),
            obs: HoldObs::over(Arc::new(SimObservationStore::single_peer(node_id(), 0))),
            ca: Arc::new(ToggleCa {
                inner: SimCa::new(Arc::new(SimEntropy::new(0x2d1))),
                fail: AtomicBool::new(false),
            }),
        }
    }
}

/// One convergence tick held (or run) on a `serve` process's runtime.
type Tick = JoinHandle<std::result::Result<(), ConvergenceError>>;

/// How a launch's dispatch ended without leaving a row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TickEnd {
    /// The dispatch returned `Ok(())`.
    Done,
    /// The dispatch returned the admission refusal.
    Refused,
    /// The dispatch returned another error.
    Failed(String),
    /// The held tick was dropped with its `serve` process.
    Lost,
}

/// One `serve` process: its runtime (its tasks die with it) and composition.
struct Serve {
    rt: Runtime,
    state: AppState,
    owner: Arc<TestSharedOwner>,
    pool: Arc<GuestAddressPool>,
    sim: Arc<SimDriver>,
    driver: Arc<OutcomeDriver>,
    /// The held launch of each model allocation whose dispatch has not been reaped.
    ticks: BTreeMap<i64, (Tick, Arc<Hold>)>,
    tick_n: u64,
    _wiring: GuestNetworkExecWiring,
    _exit_observer: JoinHandle<()>,
}

fn boot_serve(node: &Node) -> Serve {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let root = node.tmp.path().to_path_buf();
    let clock = Arc::clone(&node.clock);
    let (state, owner, pool, sim, driver, wiring) = rt.block_on(async {
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        let owner = shared_guest_network_owner();
        owner.converge_shared().await.expect("the test owner's converge_shared passes");
        assert!(wiring.supervisor().open_after_boot(), "the composition's EXEC gate opens");
        let worker = mtls_worker();
        worker.start_shared_owner().await.expect("the test mTLS owner's start_shared_owner passes");
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(&root)
            .expect("the reconciler runtime over the node's view store");
        runtime.register(crate::workload_lifecycle()).await.expect("register workload-lifecycle");
        let store_path = root.join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("the intent store opens"));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let sim =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock) as Arc<dyn Clock>));
        let driver = Arc::new(OutcomeDriver {
            inner: Arc::clone(&sim),
            fail_next_start: AtomicBool::new(false),
        });
        let pool = guest_pool();
        let state = AppState::new(
            store,
            store_path,
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::clone(&node.ca) as Arc<dyn Ca>,
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node_id(),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn crate::guest_network::SharedGuestNetworkOwner>,
            wiring.gate(),
            Arc::clone(&pool),
        );
        (state, owner, pool, sim, driver, wiring)
    });
    // The production exit observer writes each process exit's terminal row.
    let exit_observer = {
        let _entered = rt.enter();
        crate::worker::exit_observer::spawn(
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&state.lifecycle_events),
            Arc::clone(&clock) as Arc<dyn Clock>,
        )
    };
    Serve {
        rt,
        state,
        owner,
        pool,
        sim,
        driver,
        ticks: BTreeMap::new(),
        tick_n: 0,
        _wiring: wiring,
        _exit_observer: exit_observer,
    }
}

// ----- the driver --------------------------------------------------------------------

/// The driver's body, shared by the service-run and the Job-run drivers.
struct RgCore {
    /// Whether the workload the model declares is a Job (`rg_conf_job`).
    job: bool,
    node: Option<Node>,
    serve: Option<Serve>,
    /// The step counter, for the divergence messages.
    step_no: u64,
    /// Model allocation id -> production allocation (relabelling, fixed when first seen).
    alloc_of: BTreeMap<i64, AllocationId>,
    /// How each reaped launch's dispatch ended.
    ends: BTreeMap<i64, TickEnd>,
    /// The sim pool's arming (never projected): filled to its cap, and by which fillers.
    cap_filled: bool,
    fillers: Vec<AllocationId>,
}

impl RgCore {
    fn new(job: bool) -> Self {
        Self {
            job,
            node: None,
            serve: None,
            step_no: 0,
            alloc_of: BTreeMap::new(),
            ends: BTreeMap::new(),
            cap_filled: false,
            fillers: Vec::new(),
        }
    }

    fn at<T>(
        &mut self,
        action: &'static str,
        body: impl FnOnce(&mut Self) -> std::result::Result<T, Why>,
    ) -> std::result::Result<T, RgError> {
        self.step_no += 1;
        let step = self.step_no;
        body(self).map_err(|Why(why)| RgError::Step { step, action, why })
    }

    fn serve(&self) -> std::result::Result<&Serve, Why> {
        self.serve.as_ref().ok_or_else(|| why("no serve process: `Init` has not run"))
    }

    fn serve_mut(&mut self) -> std::result::Result<&mut Serve, Why> {
        self.serve.as_mut().ok_or_else(|| why("no serve process: `Init` has not run"))
    }

    fn node(&self) -> std::result::Result<&Node, Why> {
        self.node.as_ref().ok_or_else(|| why("no node: `Init` has not run"))
    }

    fn pump(&self) -> std::result::Result<(), Why> {
        self.serve()?.rt.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        Ok(())
    }

    /// Polls `cond` on the serve process's runtime until it holds.
    fn settle(
        &self,
        what: &str,
        mut cond: impl FnMut(&Self) -> bool,
    ) -> std::result::Result<(), Why> {
        let deadline = std::time::Instant::now() + SETTLE_BOUND;
        while !cond(self) {
            if std::time::Instant::now() > deadline {
                return Err(why(format!("did not settle within the bound: {what}")));
            }
            self.pump()?;
        }
        Ok(())
    }

    fn rows(&self) -> std::result::Result<Vec<AllocStatusRow>, Why> {
        let serve = self.serve()?;
        serve
            .rt
            .block_on(self.node()?.obs.alloc_status_rows())
            .map_err(|error| why(format!("alloc_status_rows: {error}")))
    }

    fn row_of(&self, a: i64) -> std::result::Result<Option<AllocStatusRow>, Why> {
        let Some(alloc) = self.alloc_of.get(&a) else { return Ok(None) };
        Ok(self.rows()?.into_iter().find(|row| &row.alloc_id == alloc))
    }

    fn alloc(&self, a: i64) -> std::result::Result<AllocationId, Why> {
        self.alloc_of
            .get(&a)
            .cloned()
            .ok_or_else(|| why(format!("allocation {a} was never issued")))
    }

    // ----- ticks ------------------------------------------------------------------

    fn spawn_tick(&mut self) -> std::result::Result<Tick, Why> {
        let clock = Arc::clone(&self.node()?.clock);
        clock.tick(PAST_BACKOFF);
        let serve = self.serve_mut()?;
        serve.tick_n += 1;
        let tick_n = serve.tick_n;
        let state = serve.state.clone();
        let owner = Arc::clone(&serve.owner);
        Ok(serve.rt.spawn(async move {
            let now = clock.now();
            Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &reconciler_name(),
                &target(),
                now,
                tick_n,
                now + Duration::from_secs(1),
                owner.as_ref(),
            ))
            .await
        }))
    }

    /// One convergence tick run to its end (the dispatch included): its `Err` is a divergence.
    fn run_tick(&mut self, what: &str) -> std::result::Result<(), Why> {
        let tick = self.spawn_tick()?;
        self.settle(&format!("the {what} tick ends"), |_| tick.is_finished())?;
        self.serve()?
            .rt
            .block_on(tick)
            .map_err(|error| why(format!("the {what} tick panicked: {error}")))?
            .map_err(|error| why(format!("the {what} tick returned `{error}`")))
    }

    /// The reconciler's own decision about the workload's current row: when the ended current
    /// row has no terminal claim yet and the reconciler will not restart it, one evaluation
    /// finalises it (`BackoffExhausted`, a Job's `Completed`/`Failed`), which the model's
    /// `exhausted` and `jobTerm` read. When a restart would follow, no tick runs: the next
    /// `WlStart` is that placement.
    fn finalize_observed(&mut self) -> std::result::Result<(), Why> {
        let rows = self.rows()?;
        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        let Some(current) = overdrive_reconcilers::current_alloc(&refs) else { return Ok(()) };
        let ended = current.state == AllocState::Failed
            || (current.kind == WorkloadKind::Job && current.state == AllocState::Terminated);
        if !ended || current.terminal.is_some() {
            return Ok(());
        }
        let current = current.clone();
        let (kind, run_desired, generation) = {
            let (job, stopped, generation, kind) = self.desired()?;
            (kind, job && !stopped, generation)
        };
        let serve = self.serve()?;
        let (_, will_restart) = serve.state.runtime.restart_status_for_alloc(
            &target(),
            kind,
            run_desired,
            &current,
            generation,
        );
        if will_restart {
            return Ok(());
        }
        self.run_tick("finalising")
    }

    /// `(a job is declared, desired_to_stop, generation, kind)` as `hydrate_desired` reads them.
    fn desired(&self) -> std::result::Result<(bool, bool, u64, WorkloadKind), Why> {
        let serve = self.serve()?;
        let reconciler = serve
            .state
            .runtime
            .get(&reconciler_name())
            .ok_or_else(|| why("workload-lifecycle is not registered"))?;
        let desired = serve
            .rt
            .block_on(hydrate_desired_for_test(reconciler, &target(), &serve.state))
            .map_err(|error| why(format!("hydrate_desired: {error}")))?;
        match desired {
            AnyState::WorkloadLifecycle(state) => Ok((
                state.job.is_some(),
                state.desired_to_stop,
                state.generation,
                state.workload_kind,
            )),
            other => Err(why(format!("hydrate_desired returned {other:?}"))),
        }
    }

    // ----- actions ------------------------------------------------------------------

    fn init(&mut self) -> std::result::Result<(), Why> {
        self.serve = None;
        self.node = None;
        self.alloc_of.clear();
        self.ends.clear();
        self.cap_filled = false;
        self.fillers.clear();
        let node = Node::new();
        self.boot(node)
    }

    /// A new `serve` process over `node`, boot reclamation run.
    fn boot(&mut self, node: Node) -> std::result::Result<(), Why> {
        let serve = boot_serve(&node);
        self.node = Some(node);
        self.serve = Some(serve);
        if self.cap_filled {
            self.fill_pool()?;
        }
        let serve = self.serve()?;
        serve
            .rt
            .block_on(converge(&serve.state))
            .map_err(|error: ConvergeError| why(format!("boot reclamation: {error}")))
    }

    fn declare(&mut self) -> std::result::Result<(), Why> {
        let resources = ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(134_217_728),
        };
        let driver = DriverInput::Vm(VmInput {
            volume: None,
            command: "/sbin/init".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        });
        let spec = if self.job {
            SubmitSpecInput::Job(JobSpecInput {
                id: WORKLOAD.to_owned(),
                replicas: 1,
                resources,
                driver,
            })
        } else {
            SubmitSpecInput::Service(ServiceSpecInput {
                id: WORKLOAD.to_owned(),
                replicas: 1,
                resources,
                driver,
                listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
                startup_probes: vec![],
                readiness_probes: vec![],
                liveness_probes: vec![],
            })
        };
        let serve = self.serve()?;
        serve
            .rt
            .block_on(submit_workload(
                AxumState(serve.state.clone()),
                HeaderMap::new(),
                Json(SubmitWorkloadRequest { spec }),
            ))
            .map_err(|error| why(format!("submit_workload: {error}")))?;
        Ok(())
    }

    fn op_stop(&mut self) -> std::result::Result<(), Why> {
        let serve = self.serve()?;
        let _stopped = serve
            .rt
            .block_on(stop_workload(AxumState(serve.state.clone()), Path(WORKLOAD.to_owned())))
            .map_err(|error| why(format!("stop_workload: {error}")))?;
        Ok(())
    }

    fn restart(&self, expected: RestartOutcome) -> std::result::Result<(), Why> {
        let serve = self.serve()?;
        let response = serve
            .rt
            .block_on(restart_workload(AxumState(serve.state.clone()), Path(WORKLOAD.to_owned())))
            .map_err(|error| why(format!("restart_workload: {error}")))?;
        if response.0.outcome == expected {
            Ok(())
        } else {
            Err(why(format!(
                "restart_workload answered {:?}, not {expected:?}",
                response.0.outcome
            )))
        }
    }

    fn workload_restart(&mut self, a: i64) -> std::result::Result<(), Why> {
        self.restart(RestartOutcome::Restarted)?;
        // A running allocation ends: the production reconciler emits its stop on the next tick.
        if self.row_of(a)?.is_some_and(|row| row.state == AllocState::Running) {
            self.run_tick("restart-stop")?;
        }
        self.job_relaunch_check()
    }

    /// A finished Job issues no id, so after a restart or resume of a Job one further unheld
    /// convergence tick runs: an erroneous relaunch would issue an id and write a row there,
    /// which the Job projection then refuses (`RgState::project`). A no-op for a service run.
    fn job_relaunch_check(&mut self) -> std::result::Result<(), Why> {
        if self.job {
            self.run_tick("post-restart relaunch check")?;
        }
        Ok(())
    }

    /// The model allocation `a` of the launch just held: the one id the View issued that no model
    /// id names yet.
    fn label_new_allocation(&mut self, a: i64) -> std::result::Result<(), Why> {
        let serve = self.serve()?;
        let fresh: Vec<AllocationId> = (0..=NA + 2)
            .map(candidate)
            .filter(|alloc| {
                serve.state.runtime.issued_generation_for_alloc(&target(), alloc).is_some()
                    && !self.alloc_of.values().any(|known| known == alloc)
            })
            .collect();
        match fresh.as_slice() {
            [one] => {
                self.alloc_of.insert(a, one.clone());
                Ok(())
            }
            [] => {
                Err(why(format!("the evaluation issued no allocation id for model allocation {a}")))
            }
            many => Err(why(format!(
                "the evaluation issued {} allocation ids for model allocation {a}: {many:?}",
                many.len()
            ))),
        }
    }

    fn wl_start(&mut self, a: i64) -> std::result::Result<(), Why> {
        let hold = self.node()?.obs.arm();
        let tick = self.spawn_tick()?;
        self.settle("the evaluation parks before admission or ends", |_| {
            hold.has_entered() || tick.is_finished()
        })?;
        if !hold.has_entered() {
            let result = self
                .serve()?
                .rt
                .block_on(tick)
                .map_err(|error| why(format!("the tick panicked: {error}")))?;
            return Err(why(format!(
                "the evaluation reached no launch dispatch for allocation {a} (tick result: {result:?})"
            )));
        }
        self.label_new_allocation(a)?;
        self.ends.remove(&a);
        self.serve_mut()?.ticks.insert(a, (tick, hold));
        Ok(())
    }

    /// Reaps the finished held tick of `a` and records how its dispatch ended.
    fn reap(&mut self, a: i64) -> std::result::Result<(), Why> {
        let serve = self.serve_mut()?;
        let Some((tick, _)) = serve.ticks.remove(&a) else {
            return Err(why(format!("allocation {a} has no held dispatch")));
        };
        let result = serve
            .rt
            .block_on(tick)
            .map_err(|error| why(format!("the held tick panicked: {error}")))?;
        let end = match result {
            Ok(()) => TickEnd::Done,
            Err(ConvergenceError::Shim(ShimError::GuestNetwork(
                GuestNetworkError::AdmissionCapReached { .. },
            ))) => TickEnd::Refused,
            Err(error) => TickEnd::Failed(error.to_string()),
        };
        self.ends.insert(a, end);
        Ok(())
    }

    /// Releases the held dispatch of `a` and waits for its dispatch to end.
    fn resolve(&mut self, a: i64) -> std::result::Result<(), Why> {
        let hold = self
            .serve()?
            .ticks
            .get(&a)
            .map(|(_, hold)| Arc::clone(hold))
            .ok_or_else(|| why(format!("allocation {a} has no held dispatch")))?;
        hold.release();
        self.settle("the released dispatch ends", |core| {
            core.serve
                .as_ref()
                .and_then(|serve| serve.ticks.get(&a))
                .is_none_or(|(tick, _)| tick.is_finished())
        })?;
        self.reap(a)?;
        self.finalize_observed()
    }

    fn launch_start_fail(&mut self, a: i64) -> std::result::Result<(), Why> {
        self.serve()?.driver.fail_next_start.store(true, Ordering::SeqCst);
        self.resolve(a)
    }

    fn launch_lost(&mut self, a: i64) -> std::result::Result<(), Why> {
        let park: StartPark = self.serve()?.sim.park_next_start(true);
        let hold = self
            .serve()?
            .ticks
            .get(&a)
            .map(|(_, hold)| Arc::clone(hold))
            .ok_or_else(|| why(format!("allocation {a} has no held dispatch")))?;
        hold.release();
        self.settle("the held dispatch reaches the driver's start", |_| park.has_entered())?;
        self.serve()?.state.runtime.supersede_lane(&target());
        self.settle("supersede_lane raises the held launch's lane signal", |_| {
            park.observed_supersession()
        })
        .map_err(|_| {
            why(format!(
                "supersede_lane must raise the signal of the lane the held launch of allocation {a} is dispatching under"
            ))
        })?;
        self.settle("the superseded dispatch ends", |core| {
            core.serve
                .as_ref()
                .and_then(|serve| serve.ticks.get(&a))
                .is_none_or(|(tick, _)| tick.is_finished())
        })?;
        self.reap(a)?;
        self.finalize_observed()
    }

    fn stop_alloc(&mut self, _a: i64) -> std::result::Result<(), Why> {
        self.run_tick("stop")?;
        self.finalize_observed()
    }

    /// The process of `a` exits: the production exit observer writes its terminal row.
    fn exit_process(&mut self, a: i64, kind: ExitKind) -> std::result::Result<(), Why> {
        let alloc = self.alloc(a)?;
        {
            // `inject_exit_after` spawns its emit task: it needs the serve runtime's context.
            let serve = self.serve()?;
            let _entered = serve.rt.enter();
            serve.sim.inject_exit_after(&alloc, Duration::ZERO, kind);
        }
        self.settle("the exit observer writes the terminal row", |core| {
            core.row_of(a).ok().flatten().is_some_and(|row| row.state != AllocState::Running)
        })?;
        self.finalize_observed()
    }

    fn vmm_crash(&mut self, a: i64) -> std::result::Result<(), Why> {
        self.exit_process(a, ExitKind::Crashed { exit_code: Some(1), signal: None })
    }

    fn job_exit(&mut self, a: i64) -> std::result::Result<(), Why> {
        self.exit_process(a, ExitKind::CleanExit)
    }

    /// The `serve` process dies: its runtime (every task of the process, the held ticks
    /// included) is dropped, and the next composition reopens the node's stores.
    fn cp_crash(&mut self) -> std::result::Result<(), Why> {
        if let Some(serve) = self.serve.take() {
            for a in serve.ticks.keys() {
                self.ends.insert(*a, TickEnd::Lost);
            }
            drop(serve);
        }
        let node = self.node.take().ok_or_else(|| why("no node: `Init` has not run"))?;
        self.boot(node)
    }

    fn node_reboot(&mut self) -> std::result::Result<(), Why> {
        let running: Vec<i64> = self
            .alloc_of
            .keys()
            .copied()
            .filter(|a| {
                self.row_of(*a).ok().flatten().is_some_and(|row| row.state == AllocState::Running)
            })
            .collect();
        for a in running {
            self.vmm_crash(a)?;
        }
        self.cp_crash()
    }

    fn id_fault(&self, on: bool) -> std::result::Result<(), Why> {
        self.node()?.ca.fail.store(on, Ordering::SeqCst);
        Ok(())
    }

    /// Takes every free slot of the sim pool through its own admission.
    fn fill_pool(&mut self) -> std::result::Result<(), Why> {
        let pool = Arc::clone(&self.serve()?.pool);
        let mut fillers = Vec::new();
        while !pool.observe(&[]).occupancy.at_cap() {
            let filler = AllocationId::new(&format!("cap-{:05}", fillers.len()))
                .map_err(|error| why(format!("filler id: {error}")))?;
            pool.assign(filler.clone(), &[], test_owner_flow_ports()).map_err(|error| {
                why(format!("the pool refused a filler below its cap: {error}"))
            })?;
            fillers.push(filler);
            if fillers.len() > MAX_GUEST_NETWORK_ATTACHMENTS as usize {
                return Err(why("the pool never reached its cap"));
            }
        }
        self.fillers = fillers;
        self.cap_filled = true;
        Ok(())
    }

    fn drain_pool(&mut self) -> std::result::Result<(), Why> {
        let pool = Arc::clone(&self.serve()?.pool);
        for filler in std::mem::take(&mut self.fillers) {
            pool.release(&filler);
        }
        self.cap_filled = false;
        Ok(())
    }
}

/// The driver of the eight service runs (`rg_conf`).
struct RgDriver {
    core: RgCore,
}

/// The driver of the two finished-Job runs (`rg_conf_job`).
struct RgJobDriver {
    core: RgCore,
}

macro_rules! rg_driver {
    ($driver:ident, $main:expr, $job:expr) => {
        impl $driver {
            fn new() -> Self {
                Self { core: RgCore::new($job) }
            }
        }

        impl QuintDriver for $driver {
            type State = RgState;

            fn config() -> Config {
                Config { state: &[], nondet: &[concat!($main, "::restart_generation::conf_step")] }
            }

            fn step(&mut self, step: &Step) -> quint_connect::Result {
                let core = &mut self.core;
                switch!(step {
                    Init => core.at("Init", |c| c.init())?,
                    Declare => core.at("Declare", |c| c.declare())?,
                    OpStop => core.at("OpStop", |c| c.op_stop())?,
                    OpResume => core.at("OpResume", |c| {
                        c.restart(RestartOutcome::Resumed)?;
                        c.job_relaunch_check()
                    })?,
                    WorkloadRestart(a: i64) => core.at("WorkloadRestart", |c| c.workload_restart(a))?,
                    WlStart(a: i64) => core.at("WlStart", |c| c.wl_start(a))?,
                    StopAlloc(a: i64) => core.at("StopAlloc", |c| c.stop_alloc(a))?,
                    LaunchRunning(a: i64) => core.at("LaunchRunning", |c| c.resolve(a))?,
                    LaunchPreFail(a: i64) => core.at("LaunchPreFail", |c| c.resolve(a))?,
                    LaunchRefused(a: i64) => core.at("LaunchRefused", |c| c.resolve(a))?,
                    LaunchStartFail(a: i64) => core.at("LaunchStartFail", |c| c.launch_start_fail(a))?,
                    LaunchLost(a: i64) => core.at("LaunchLost", |c| c.launch_lost(a))?,
                    VmmCrash(a: i64) => core.at("VmmCrash", |c| c.vmm_crash(a))?,
                    JobExit(a: i64) => core.at("JobExit", |c| c.job_exit(a))?,
                    CpCrash => core.at("CpCrash", |c| c.cp_crash())?,
                    NodeReboot => core.at("NodeReboot", |c| c.node_reboot())?,
                    IdFaultOn => core.at("IdFaultOn", |c| c.id_fault(true))?,
                    IdFaultOff => core.at("IdFaultOff", |c| c.id_fault(false))?,
                    CapFill => core.at("CapFill", |c| c.fill_pool())?,
                    CapDrain => core.at("CapDrain", |c| c.drain_pool())?,
                })
            }
        }

        impl State<$driver> for RgState {
            fn from_spec(value: Value) -> quint_connect::Result<Self> {
                Ok(RgState::from_value(&value)?)
            }

            fn from_driver(driver: &$driver) -> quint_connect::Result<Self> {
                Ok(RgState::project(&driver.core)?)
            }
        }
    };
}

rg_driver!(RgDriver, "rg_conf", false);
rg_driver!(RgJobDriver, "rg_conf_job", true);

// ----- the compared state ------------------------------------------------------------

/// Why an allocation ended, reduced to the classes the model separates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhyClass {
    /// Any other end, or none.
    Other,
    /// A pre-driver failure: a charged `Failed` row (`W_PREFAIL`).
    PreFail,
    /// An admission refusal: no row, no charge (`W_REFUSED`).
    Refused,
    /// A launch that ended before running without a failure: an abandoned row (`W_LOST`, ADR-0355).
    Lost,
}

/// The compared facts of one model allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AllocFacts {
    /// 0 unminted, 1 pending, 2 running, 3 failed, 4 terminated, 5 abandoned (ADR-0355).
    state: i64,
    why: WhyClass,
    /// The generation the allocation was issued for.
    ig: i64,
    /// The restart count the allocation was issued with.
    cnt: i64,
    /// `jobTerm`: a Job row with a terminal claim.
    job_term: bool,
}

/// The compared `rg_conf` / `rg_conf_job` variables.
#[derive(Debug, PartialEq, Eq)]
struct RgState {
    dg: i64,
    stopped: bool,
    allocs: BTreeMap<i64, AllocFacts>,
    /// The spec's derived `exhausted(s)`, the production current row's `BackoffExhausted`.
    exhausted: bool,
}

impl RgState {
    /// The spec side: the qualified variable `<main>::restart_generation::st`.
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let (vars, job) = match Qualified::of(state, MAIN_JOB, MODULE)?.var("st") {
            Ok(_) => (Qualified::of(state, MAIN_JOB, MODULE)?, true),
            Err(_) => (Qualified::of(state, MAIN_SERVICE, MODULE)?, false),
        };
        let st = vars.var("st")?;
        let as_ = quint_itf::int_map(quint_itf::field(st, "as")?, quint_itf::int)?;
        let why_ = quint_itf::int_map(quint_itf::field(st, "why")?, quint_itf::int)?;
        let ig = quint_itf::int_map(quint_itf::field(st, "ig")?, quint_itf::int)?;
        let cnt = quint_itf::int_map(quint_itf::field(st, "cnt")?, quint_itf::int)?;
        // The spec's derived defs, over its own state: `hasRow`, `curA`, `charged`, `exhausted`.
        // A PENDING allocation has its `Pending` row (ADR-0353), an abandoned launch (state 5) its terminal row
        // (ADR-0355); only a refused launch (`W_REFUSED`) has none.
        let has_row = |a: i64| matches!(as_.get(&a), Some(1..=5)) && why_.get(&a) != Some(&10);
        // RB-13 (ADR-0355): the count basis is the newest row that ran, `last_ran_alloc`: the newest row that is not abandoned.
        let basis_a = (1..=NA).filter(|a| has_row(*a) && as_.get(a) != Some(&5)).max().unwrap_or(0);
        let charged = if basis_a == 0 {
            0
        } else {
            cnt.get(&basis_a).copied().unwrap_or(0) + i64::from(as_.get(&basis_a) == Some(&3))
        };
        let non_terminal = as_.values().any(|state| *state == 1 || *state == 2);
        let mut allocs = BTreeMap::new();
        for a in 1..=NA {
            let state = as_.get(&a).copied().unwrap_or(0);
            allocs.insert(
                a,
                AllocFacts {
                    state,
                    why: match why_.get(&a) {
                        Some(9) => WhyClass::PreFail,
                        Some(10) => WhyClass::Refused,
                        Some(7) => WhyClass::Lost,
                        _ => WhyClass::Other,
                    },
                    ig: ig.get(&a).copied().unwrap_or(0),
                    cnt: cnt.get(&a).copied().unwrap_or(0),
                    job_term: job && has_row(a) && (state == 3 || state == 4),
                },
            );
        }
        Ok(Self {
            dg: quint_itf::int(quint_itf::field(st, "dg")?)?,
            stopped: quint_itf::boolean(quint_itf::field(st, "stopped")?)?,
            allocs,
            exhausted: charged >= LIMIT && !non_terminal,
        })
    }

    /// The production side, through the owner's pinned accessors only.
    fn project(core: &RgCore) -> std::result::Result<Self, RgError> {
        let step = core.step_no;
        let fail = |text: String| RgError::Projection { step, why: text };
        let serve = core.serve.as_ref().ok_or_else(|| fail("no serve process".into()))?;
        let rows = core.rows().map_err(|Why(text)| fail(text))?;
        if core.job {
            // A finished Job issues no id and writes no unlabelled row: an id in the View's issued
            // set, or an observed row, that no model allocation names is a divergence, not a skip.
            for n in 0..=NA + 2 {
                let issued = candidate(n);
                if serve.state.runtime.issued_generation_for_alloc(&target(), &issued).is_some()
                    && !core.alloc_of.values().any(|known| known == &issued)
                {
                    return Err(fail(format!(
                        "the Job issued allocation id {issued}, which no model allocation names"
                    )));
                }
            }
            if let Some(row) =
                rows.iter().find(|row| !core.alloc_of.values().any(|known| known == &row.alloc_id))
            {
                return Err(fail(format!(
                    "the Job has an observed row for {}, which no model allocation names",
                    row.alloc_id
                )));
            }
        }
        let (_, stopped, dg, _) = core.desired().map_err(|Why(text)| fail(text))?;
        let live = serve.driver.live_allocations().unwrap_or_default();
        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        let current = overdrive_reconcilers::current_alloc(&refs);
        let mut allocs = BTreeMap::new();
        for a in 1..=NA {
            let Some(alloc) = core.alloc_of.get(&a) else {
                allocs.insert(
                    a,
                    AllocFacts { state: 0, why: WhyClass::Other, ig: 0, cnt: 0, job_term: false },
                );
                continue;
            };
            let ig = serve.state.runtime.issued_generation_for_alloc(&target(), alloc).ok_or_else(
                || fail(format!("allocation {a} ({alloc}) has no issued generation")),
            )?;
            let cnt = serve.state.runtime.attempt_index_for_alloc(&target(), alloc);
            let row = rows.iter().find(|row| &row.alloc_id == alloc);
            let (state, why) = if let Some(row) = row {
                let state = match row.state {
                    AllocState::Running => 2,
                    AllocState::Failed => 3,
                    // An abandoned launch is a `Terminated` row with `terminal == None` and reason
                    // `LaunchSuperseded` or `LaunchLostWithControlPlane` (ADR-0355).
                    AllocState::Terminated if is_launch_abandoned(row) => 5,
                    AllocState::Terminated => 4,
                    AllocState::Pending => 1,
                    other => {
                        return Err(fail(format!(
                            "allocation {a} has a {other:?} row the model has no state for"
                        )));
                    }
                };
                let prefail = row.state == AllocState::Failed
                    && match &row.reason {
                        Some(TransitionReason::MtlsInterceptInstallFailed { stage, .. }) => {
                            stage == "identity_issue"
                        }
                        Some(TransitionReason::DriverInternalError { .. }) => true,
                        _ => false,
                    };
                let why = if prefail {
                    WhyClass::PreFail
                } else if state == 5 {
                    WhyClass::Lost
                } else {
                    WhyClass::Other
                };
                (state, why)
            } else {
                let held = serve.ticks.get(&a).is_some_and(|(tick, _)| !tick.is_finished());
                if held {
                    // The hold parks the launch at the point read of its prior row, before the shim's
                    // admission; the `Pending` row (ADR-0353) is written after admission, once the pool
                    // has handed the lease out. No test-owned seam holds the launch after that write.
                    return Err(RgError::OwnerApiPending {
                        step,
                        awaits: "a hold of the launch after its `Pending` row is written: the hold parks before admission, which precedes the row (ADR-0353)",
                    });
                }
                if live.contains(alloc) {
                    return Err(fail(format!(
                        "allocation {a} has no row but is live in the driver"
                    )));
                }
                match core.ends.get(&a) {
                    Some(TickEnd::Refused) => (3, WhyClass::Refused),
                    Some(TickEnd::Done | TickEnd::Lost) => {
                        return Err(fail(format!(
                            "the launch of allocation {a} ended before running and left no abandoned row (ADR-0355)"
                        )));
                    }
                    Some(TickEnd::Failed(error)) => {
                        return Err(fail(format!(
                            "allocation {a}: the dispatch ended with `{error}` and wrote no row"
                        )));
                    }
                    None => {
                        return Err(fail(format!(
                            "allocation {a} has no row and no recorded dispatch end"
                        )));
                    }
                }
            };
            allocs.insert(
                a,
                AllocFacts {
                    state,
                    why,
                    ig: i64::try_from(ig).map_err(|_| fail("generation out of range".into()))?,
                    cnt: i64::from(cnt) - 1,
                    job_term: row
                        .is_some_and(|row| row.kind == WorkloadKind::Job && row.terminal.is_some()),
                },
            );
        }
        Ok(Self {
            dg: i64::try_from(dg).map_err(|_| fail("generation out of range".into()))?,
            stopped,
            allocs,
            exhausted: current.is_some_and(|row| {
                matches!(row.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
            }),
        })
    }
}

impl<'de> Deserialize<'de> for RgState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

// ----- named runs (critical scenarios, one per `rg_conf` / `rg_conf_job` run) ----------

/// Named run `control_plane_crash_during_restart_launch_relaunches_once`.
/// PENDING (ADR-0353, ADR-0355: no `Pending` row, no abandoned row; awaits owner API). Also SD-6, ADR-0335: the restart generation is stamped when emitted, so after the control plane lost the replacement launch (step 7) the next evaluation (step 8) places nothing; it waits for "a generation is handled when its successor is observed".
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "control_plane_crash_during_restart_launch_relaunches_once",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_control_plane_crash_during_restart_launch_relaunches_once() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `second_restart_loses_launch_then_one_placement`.
/// PENDING (ADR-0353, ADR-0355: no `Pending` row, no abandoned row; awaits owner API). Also ADR-0334: `ReconcilerRuntime::supersede_lane` is a scaffold; it waits for the convergence lane table (step 10). SD-6 (the steps after the loss) is unobserved until it lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "second_restart_loses_launch_then_one_placement",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_second_restart_loses_launch_then_one_placement() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `resume_while_running_stops_and_replaces`.
/// PENDING (ADR-0353: the `Pending` row of a launch in flight is not written yet, step 3 awaits owner API); a GUARD once it is (passes against production before ADR-0353: a restart with no lost launch); keeps passing once SD-6 lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "resume_while_running_stops_and_replaces",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_resume_while_running_stops_and_replaces() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `restart_of_exhausted_workload_relaunches_uncharged`.
/// PENDING (ADR-0353: no `Pending` row; awaits owner API). Also ADR-0337: a pre-driver failure (identity issuance) returns `Err` and writes no row (step 5); it waits for the charged `Failed` row. Later steps (SD-6 replacement of an exhausted row) are unobserved until it lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "restart_of_exhausted_workload_relaunches_uncharged",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_restart_of_exhausted_workload_relaunches_uncharged() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `generation_replacement_forgives_predecessor_failure`.
/// PENDING (ADR-0337): as above (step 5). Later steps (the carried count of a generation replacement) are unobserved until it lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "generation_replacement_forgives_predecessor_failure",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_generation_replacement_forgives_predecessor_failure() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `persistent_prelaunch_failure_ends_in_backoff_exhausted`.
/// PENDING (ADR-0337): as above (step 5); `BackoffExhausted` after the pre-driver failures is unobserved until it lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "persistent_prelaunch_failure_ends_in_backoff_exhausted",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_persistent_prelaunch_failure_ends_in_backoff_exhausted() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `transient_prelaunch_failure_recovers`.
/// PENDING (ADR-0337): as above (step 5).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "transient_prelaunch_failure_recovers",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_transient_prelaunch_failure_recovers() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `admission_refusal_leaves_no_row_and_waits_for_capacity`.
/// PENDING (ADR-0353: the `Pending` row of a launch in flight is not written yet, step 3 awaits owner API); a GUARD once it is (passes against production before ADR-0353: the refusal already writes no row and charges nothing, ADR-0338); keeps passing once the occupancy gate lands.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf",
    test = "admission_refusal_leaves_no_row_and_waits_for_capacity",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_admission_refusal_leaves_no_row_and_waits_for_capacity() -> impl QuintDriver {
    RgDriver::new()
}

/// Named run `restart_of_completed_job_does_nothing_and_issues_no_id`.
/// PENDING (ADR-0353: the `Pending` row of a launch in flight is not written yet, step 3 awaits owner API); a GUARD once it is (passes against production before ADR-0353: a finished Job's row is final and a restart issues no id, ADR-0335 item 3).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf_job",
    test = "restart_of_completed_job_does_nothing_and_issues_no_id",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_restart_of_completed_job_does_nothing_and_issues_no_id() -> impl QuintDriver {
    RgJobDriver::new()
}

/// Named run `resume_of_stopped_job_does_nothing_and_issues_no_id`.
/// PENDING (ADR-0353: no `Pending` row; awaits owner API). Also: today production gives a resumed operator-stopped Job a new allocation id (the check that the
/// Job issues no id the model names was vacuous before); DELIVER step 9 (generation helper) ends it.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/restart-generation/restart_generation.qnt",
    main = "rg_conf_job",
    test = "resume_of_stopped_job_does_nothing_and_issues_no_id",
    max_samples = 1
)]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id() -> impl QuintDriver {
    RgJobDriver::new()
}

/// PENDING (ADR-0353, ADR-0355): every trace that places an allocation reaches the `Pending` row no owner writes yet, so
/// the first divergence a seed hits is the owner-API pending error (`awaits owner API`), at a projection.
/// Randomized `rg_conf` traces (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/restart-generation/restart_generation.qnt", main = "rg_conf")]
#[should_panic(expected = "awaits owner API")]
fn quint_rg_randomized() -> impl QuintDriver {
    RgDriver::new()
}
