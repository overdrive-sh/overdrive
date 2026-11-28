//! restart_generation_is_placed_exactly_once_across_supersession_and_crash — the seeded
//! restart-generation invariant of the node-local persistent volume feature (GH #348, ADR-0335,
//! ADR-0337, ADR-0338; assumption A10, RA2, RA10, RA11).
//!
//! # Placement
//!
//! This binary lives in `overdrive-control-plane`, not `overdrive-sim`: the invariant is driven
//! through the real `stop_workload` / `restart_workload` / `submit_workload` handlers, whose
//! signatures are axum extractors, and `overdrive-sim` has no axum edge. Seeds are drawn from
//! `SimEntropy` (a `StdRng::seed_from_u64` behind the `Entropy` port), because this crate has no
//! `rand` edge.
//!
//! # Scenario family
//!
//! A restart generation is handled once the allocation issued for it has a row. A launch that is
//! abandoned (superseded by an operator commit or by shutdown, or lost with the control plane)
//! leaves the generation pending, so the next evaluation places one fresh allocation for it,
//! charging nothing. A launch that fails before the driver (the identity issuer) writes a charged
//! `Failed` row; a Service retries it to `BackoffExhausted`, a Job's is final (J-1). A restart never
//! gives a Job whose current allocation carries a terminal claim a new allocation (H1).
//!
//! * Matrix: origin {restart of a `Running` workload, resume of an operator-stopped workload} x
//!   interruption {`stop_workload`, a second `restart_workload`, `supersede_all_lanes`
//!   (shutdown), a control-plane crash during the launch}; then the next boot.
//! * Fault dimension, injected at the CA port: pre-launch failure {none, persistent, transient}.
//! * Workload-kind dimension: Service and Job (the restart of a running Job (H1) and the J-1
//!   pre-driver case).
//!
//! # Contract shape and owner path
//!
//! Bounded-change. The owner path is production end to end: the real handlers commit the operator
//! intent (the stop sentinel, the generation bump), the real convergence tick
//! (`run_convergence_tick_with_guest_network_provisioner_for_test`) runs the runtime's lane table,
//! `WorkloadLifecycle` hydration and `reconcile`, persists the View (`SimViewStore`, the driven
//! port the runtime's `RedbViewStore` implements) and dispatches through the real action shim,
//! over the production `GuestAddressPool`, `IdentityMgr` and `MtlsInterceptWorker` on sim ports.
//! Faults enter only at driven ports: the `Ca` port (a wrapper over `SimCa` whose `issue_svid`
//! fails), `SimDriver` (`park_next_start(true)` holds a launch and observes the lane signal), the
//! interruptions `ReconcilerRuntime::supersede_all_lanes`, and a control-plane crash (the tick
//! future is dropped mid-dispatch, the runtime is rebuilt over the persisted View, the intent
//! store is reopened from its file and `vm_reclamation_boot::converge` runs, as `run_server`
//! does before the loop). No row, View entry or terminal state is seeded.
//!
//! # Oracle
//!
//! The allocation rows; `ReconcilerRuntime::issued_generation_for_alloc` and
//! `attempt_index_for_alloc`'s source (`restart_counts`, read through
//! `view_for_workload_lifecycle`); the generation at `workloads/<id>/generation`; every
//! `Driver::start` call the shim made (outcome, in-flight concurrency); the CA's `issue_svid`
//! attempts. Assertion (b)'s "launches" are the driver's `start` calls; for a launch that fails
//! before the driver they are the CA attempts.
//!
//! # Seeds
//!
//! Every schedule derives from a printed seed (`SimEntropy::new(seed)`): the clock step between
//! evaluations (held, 100 ms, or past every backoff window), the evaluations before and after the
//! interruption, whether the next boot sits before the relaunch, and the number of transient
//! failures. The same seed gives the same trace (`the_same_seed_gives_the_same_trace`). A failing
//! assertion carries the seed, the cell and the schedule.
//!
//! # Tier
//!
//! Tier 1, in-process over `Sim*` ports, `host_pure`. It opens a real redb in a `TempDir`, so it
//! lives under the crate's `integration-tests` entrypoint (`tests/integration.rs`) and not in the
//! default lane. Every test is a few seconds of in-process evaluation; none is expected to come
//! near the 60 s slow-test budget (the longest are the six-seed matrix cells, which run 6 worlds
//! each).
//!
//! # Pending versus guard
//!
//! A test of behaviour DELIVER has not built is PENDING: it keeps its full body and is
//! `#[should_panic(expected = ...)]` with the message of the assertion in its own body that fails
//! today. A test that passes today is a GUARD and says so. Where the property fails only after
//! several steps, the first failing assertion is the one named.
//!
//! The tests marked "count basis" share one cause (ADR-0355 section 2, RB-13). A launch that ended
//! before running (abandoned by shutdown, or lost with the control plane) leaves a `Pending` row
//! that ends `LaunchSuperseded` or `LaunchLostWithControlPlane`, and the relaunch reads its count
//! from the newest row that ran, not from that row. The charge assertion ("must charge nothing" or
//! "must re-assign the count") is what they guard.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    clippy::print_stderr,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::format_push_string,
    clippy::single_match_else,
    clippy::collapsible_if,
    clippy::uninlined_format_args,
    reason = "scenarios assert their own preconditions; seeds and schedules are printed for reproduction"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::guest_network::{GuestAddressPool, SharedGuestNetworkOwner};
use overdrive_control_plane::handlers::{restart_workload, stop_workload, submit_workload};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::view_store::ViewStore;
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, vm_reclamation_boot, workload_lifecycle};
use overdrive_core::aggregate::IntentKey;
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitKind,
    LaunchSupersession, Resources,
};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::{TerminalCondition, TransitionReason};
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::{SimDriver, StartPark};
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use overdrive_store_local::LocalIntentStore;
use overdrive_testing::placement;
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;
use tokio::task::JoinHandle;

placement!(host_pure);

/// The seeds every test runs; consecutive so the residues the schedule keys on (clock step, boot
/// position) all occur.
const SEEDS: [u64; 6] = [348_001, 348_002, 348_003, 348_004, 348_005, 348_006];
/// The `SimClock` step that passes every restart backoff window.
const PAST_BACKOFF: Duration = Duration::from_secs(3_600);
/// Bound, in executor yields, on every wait for an effect that must happen. The runtime is
/// single-threaded and the ports are sim, so a wait is a count of yields, not a duration.
const SETTLE_YIELD_CAP: u32 = 20_000;
/// Consecutive yields over which the observable state must not change for the world to be settled.
const QUIESCENT_YIELDS: u32 = 16;
const NODE: &str = "local";
const WORKLOAD: &str = "rg-w";
const CA_CAUSE: &str = "identity issuer unavailable (seeded fault)";
const LANE_SIGNAL_MESSAGE: &str = "the interruption must raise the supersession signal of the held launch's lane, ending it with StartSuperseded (ADR-0334)";

// ---------------------------------------------------------------------------
// Seeded schedule
// ---------------------------------------------------------------------------

/// What a seed decides. Everything is derived from the seed so a failure reproduces from it.
#[derive(Clone, Copy, Debug)]
struct Schedule {
    /// The clock step between evaluations when no fault forces a backoff window to pass.
    advance: Duration,
    idle_before: usize,
    idle_after: usize,
    /// Whether the next boot sits between a non-crash interruption and the relaunch.
    boot_between: bool,
    /// How many launches a transient fault fails.
    transient_failures: u32,
    /// Evaluations a Job origin runs after its restart.
    job_settle_ticks: usize,
}

fn schedule(seed: u64) -> Schedule {
    let entropy = SimEntropy::new(seed);
    let below = |n: u64| entropy.u64() % n;
    Schedule {
        advance: [Duration::ZERO, Duration::from_millis(100), PAST_BACKOFF][(seed % 3) as usize],
        idle_before: below(3) as usize,
        idle_after: 1 + below(3) as usize,
        boot_between: seed % 2 == 1,
        transient_failures: 1 + below(3) as u32,
        job_settle_ticks: 6 + below(5) as usize,
    }
}

// ---------------------------------------------------------------------------
// Driven-port doubles
// ---------------------------------------------------------------------------

/// What a launch ended with, as the driver saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchOutcome {
    InFlight,
    Started,
    Superseded,
    Rejected,
    Other,
    /// The dispatch future was dropped mid-launch: the control plane died.
    Dropped,
}

impl LaunchOutcome {
    const fn abandoned(self) -> bool {
        matches!(self, Self::Superseded | Self::Dropped)
    }
}

#[derive(Clone, Debug)]
struct StartRecord {
    alloc: AllocationId,
    outcome: LaunchOutcome,
}

/// Every `Driver::start` call across every boot of the node, and the in-flight concurrency.
#[derive(Default)]
struct DriverLog {
    records: Mutex<Vec<StartRecord>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl DriverLog {
    fn records(&self) -> Vec<StartRecord> {
        self.records.lock().clone()
    }
}

/// Removes the in-flight mark and records `Dropped` when the dispatch future is dropped mid-launch.
struct StartGuard {
    log: Arc<DriverLog>,
    index: usize,
    done: bool,
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        if !self.done {
            self.log.records.lock()[self.index].outcome = LaunchOutcome::Dropped;
            self.log.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// A `SimDriver` whose `start` calls are recorded at the driven port.
struct TraceDriver {
    inner: Arc<SimDriver>,
    log: Arc<DriverLog>,
}

#[async_trait]
impl Driver for TraceDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(
        &self,
        spec: &AllocationSpec,
        supersession: &dyn LaunchSupersession,
    ) -> Result<AllocationHandle, DriverError> {
        let index = {
            let mut records = self.log.records.lock();
            records
                .push(StartRecord { alloc: spec.alloc.clone(), outcome: LaunchOutcome::InFlight });
            records.len() - 1
        };
        let now_in_flight = self.log.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.log.max_in_flight.fetch_max(now_in_flight, Ordering::SeqCst);
        let mut guard = StartGuard { log: Arc::clone(&self.log), index, done: false };
        let result = self.inner.start(spec, supersession).await;
        guard.done = true;
        self.log.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.log.records.lock()[index].outcome = match &result {
            Ok(_) => LaunchOutcome::Started,
            Err(DriverError::StartSuperseded) => LaunchOutcome::Superseded,
            Err(DriverError::StartRejected { .. }) => LaunchOutcome::Rejected,
            Err(_) => LaunchOutcome::Other,
        };
        result
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
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

/// How the CA's `issue_svid` behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaMode {
    Healthy,
    /// Every issuance fails.
    Persistent,
    /// The next `n` issuances fail, then the CA recovers.
    Failing(u32),
}

/// `Ca` double: delegates to `SimCa`; `issue_svid` follows its [`CaMode`] and counts attempts.
struct FaultyCa {
    inner: SimCa,
    mode: Mutex<CaMode>,
    attempts: AtomicUsize,
}

impl FaultyCa {
    fn new() -> Self {
        Self {
            inner: SimCa::new(Arc::new(SimEntropy::new(348))),
            mode: Mutex::new(CaMode::Healthy),
            attempts: AtomicUsize::new(0),
        }
    }

    fn set(&self, mode: CaMode) {
        *self.mode.lock() = mode;
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl Ca for FaultyCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.inner.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.inner.issue_intermediate(node)
    }

    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        {
            let mut mode = self.mode.lock();
            match *mode {
                CaMode::Healthy => {}
                CaMode::Persistent => {
                    return Err(CaError::SigningFailed { reason: CA_CAUSE.to_owned() });
                }
                CaMode::Failing(remaining) => {
                    *mode = if remaining <= 1 {
                        CaMode::Healthy
                    } else {
                        CaMode::Failing(remaining - 1)
                    };
                    return Err(CaError::SigningFailed { reason: CA_CAUSE.to_owned() });
                }
            }
        }
        self.inner.issue_svid(req)
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.inner.trust_bundle()
    }
}

fn mtls_worker() -> Arc<MtlsInterceptWorker> {
    let enforcement: Arc<dyn overdrive_core::traits::mtls_enforcement::MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(
            Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
            overdrive_core::traits::mtls_enforcement::MtlsLimits::default(),
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
        ));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(SimMtlsResolve::new(
            BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    let intercept: Arc<dyn MtlsIntercept> = Arc::new(SimMtlsIntercept::new());
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

fn guest_pool() -> Arc<GuestAddressPool> {
    Arc::new(GuestAddressPool::new(
        "100.95.0.0/16".parse().expect("static guest prefix"),
        Ipv4Addr::new(100, 95, 0, 1),
        Arc::new(SimEntropy::new(0x295)),
    ))
}

// ---------------------------------------------------------------------------
// The world: one node over stores that outlive its boots
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Service,
    Job,
}

/// One boot of the node: the production composition over the sim ports.
struct Node {
    state: AppState,
    sim: Arc<SimDriver>,
    owner: Arc<SimSharedGuestNetworkOwner>,
    observer: JoinHandle<()>,
    _wiring: GuestNetworkExecWiring,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.observer.abort();
    }
}

/// One spawned convergence tick of the workload.
struct Tick {
    handle: JoinHandle<Result<(), ConvergenceError>>,
}

impl Tick {
    async fn join(self) -> Result<(), ConvergenceError> {
        tokio::time::timeout(Duration::from_secs(10), self.handle)
            .await
            .expect("the tick ends within the bound")
            .expect("the tick task did not panic")
    }

    /// The control plane dies mid-dispatch: the tick future is dropped where it stands.
    async fn abort(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

/// The observable state a re-evaluation must leave untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
struct IssueState {
    restart_counts: BTreeMap<AllocationId, u32>,
    issued_generation: BTreeMap<AllocationId, u64>,
    driver_starts: usize,
    ca_attempts: usize,
    rows: usize,
}

struct World {
    seed: u64,
    sched: Schedule,
    label: String,
    tmp: TempDir,
    clock: Arc<SimClock>,
    obs: Arc<SimObservationStore>,
    views: Arc<SimViewStore>,
    ca: Arc<FaultyCa>,
    log: Arc<DriverLog>,
    lines: Vec<String>,
    tick_n: u64,
    /// Restart commits this world made (cross-checked against the intent store).
    commits: u64,
    node: Option<Node>,
}

fn workload_id() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
}

fn target() -> TargetResource {
    TargetResource::new(&format!("workload/{WORKLOAD}")).expect("a valid target")
}

fn reconciler() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("a valid reconciler name")
}

fn attempt_suffix(id: &AllocationId) -> u32 {
    id.as_str().rsplit_once('-').and_then(|(_, n)| n.parse().ok()).unwrap_or(0)
}

fn is_identity_issue(row: &AllocStatusRow) -> bool {
    matches!(
        &row.reason,
        Some(TransitionReason::MtlsInterceptInstallFailed { stage, .. }) if stage == "identity_issue"
    )
}

impl World {
    async fn new(seed: u64, label: &str) -> Self {
        eprintln!("restart-generation invariant: {label} seed={seed}");
        let node_id = NodeId::new(NODE).expect("node id");
        let mut world = Self {
            seed,
            sched: schedule(seed),
            label: label.to_owned(),
            tmp: TempDir::new().expect("tempdir"),
            clock: Arc::new(SimClock::new()),
            obs: Arc::new(SimObservationStore::single_peer(node_id, seed)),
            views: Arc::new(SimViewStore::new()),
            ca: Arc::new(FaultyCa::new()),
            log: Arc::new(DriverLog::default()),
            lines: Vec::new(),
            tick_n: 0,
            commits: 0,
            node: None,
        };
        world.node = Some(world.boot_node().await);
        world.line(format!("boot 1; schedule={:?}", world.sched));
        world
    }

    async fn boot_node(&self) -> Node {
        let store_path = self.tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("the intent store opens"));
        let mut runtime =
            ReconcilerRuntime::new(self.tmp.path(), Arc::clone(&self.views) as Arc<dyn ViewStore>)
                .expect("reconciler runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        let sim = Arc::new(SimDriver::with_clock(DriverType::Vm, self.clock.clone()));
        let driver = Arc::new(TraceDriver { inner: Arc::clone(&sim), log: Arc::clone(&self.log) });
        let owner = Arc::new(SimSharedGuestNetworkOwner::default());
        owner.converge_shared().await.expect("the shared guest-network owner converges");
        let wiring = GuestNetworkExecWiring::new(self.clock.clone() as Arc<dyn Clock>);
        assert!(wiring.supervisor().open_after_boot(), "the EXEC gate leaves BootClosed once");
        let worker = mtls_worker();
        worker.start_shared_owner().await.expect("the shared mTLS owner publishes");
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&store) as Arc<dyn IntentStore>
        );
        let state = AppState::new(
            store,
            store_path,
            Arc::clone(&self.obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            self.clock.clone() as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::clone(&self.ca) as Arc<dyn Ca>,
            Arc::new(IdentityMgr::new(None)),
            NodeId::new(NODE).expect("node id"),
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            wiring.gate(),
            guest_pool(),
        );
        let observer = exit_observer::spawn(
            state.obs.clone(),
            driver as Arc<dyn Driver>,
            state.lifecycle_events.clone(),
            self.clock.clone() as Arc<dyn Clock>,
        );
        Node { state, sim, owner, observer, _wiring: wiring }
    }

    /// The next boot: the old process is gone (its runtime, driver, pool and tasks), the intent
    /// store is reopened from its file, the runtime is rebuilt over the persisted View, and boot
    /// reclamation runs before any evaluation, as `run_server` does.
    async fn reboot(&mut self) {
        drop(self.node.take());
        let node = self.boot_node().await;
        let reclaimed = vm_reclamation_boot::converge(&node.state).await;
        self.node = Some(node);
        let n = self.log.records().len();
        self.line(format!(
            "reboot; boot reclamation={:?}; driver starts so far={n}",
            reclaimed.is_ok()
        ));
        if let Err(error) = reclaimed {
            self.fail(&format!("setup: boot reclamation must succeed on the next boot: {error:?}"))
                .await;
        }
    }

    fn node(&self) -> &Node {
        self.node.as_ref().expect("the node is booted")
    }

    fn state(&self) -> &AppState {
        &self.node().state
    }

    fn sim(&self) -> &Arc<SimDriver> {
        &self.node().sim
    }

    fn runtime(&self) -> &ReconcilerRuntime {
        &self.state().runtime
    }

    fn line(&mut self, line: String) {
        self.lines.push(line);
    }

    // -- operator inputs: the real handlers ---------------------------------

    fn spec_json() -> serde_json::Value {
        json!({
            "id": WORKLOAD,
            "replicas": 1,
            "resources": {"cpu": "0", "memory": "1MiB"},
            "vm": {"command": "/sbin/init", "args": [], "kernel": "/kernel", "rootfs": "/rootfs"},
        })
    }

    async fn deploy(&mut self, kind: Kind) {
        let spec = match kind {
            Kind::Service => {
                let mut spec = Self::spec_json();
                spec["listeners"] = json!([{"port": 8080, "protocol": "tcp"}]);
                spec["startup_probes"] = json!([]);
                spec["readiness_probes"] = json!([]);
                spec["liveness_probes"] = json!([]);
                SubmitSpecInput::Service(
                    serde_json::from_value(spec).expect("a VM service wire spec"),
                )
            }
            Kind::Job => SubmitSpecInput::Job(
                serde_json::from_value(Self::spec_json()).expect("a VM job wire spec"),
            ),
        };
        let response = submit_workload(
            State(self.state().clone()),
            HeaderMap::new(),
            Json(SubmitWorkloadRequest { spec }),
        )
        .await
        .expect("the handler admits a valid VM workload");
        assert!(response.status().is_success(), "the deploy is accepted: {}", response.status());
        self.line(format!("deploy {kind:?}"));
    }

    async fn stop(&mut self) {
        let _response = stop_workload(State(self.state().clone()), Path(WORKLOAD.to_owned()))
            .await
            .expect("stop_workload records the stop");
        self.line("operator: stop_workload".to_owned());
    }

    async fn restart(&mut self) {
        let _response = restart_workload(State(self.state().clone()), Path(WORKLOAD.to_owned()))
            .await
            .expect("restart_workload records the restart");
        self.commits += 1;
        let committed = self.commits;
        self.line(format!("operator: restart_workload (commit {committed})"));
    }

    /// The generation committed at `workloads/<id>/generation` (absent reads 0).
    async fn committed_generation(&self) -> u64 {
        let key = IntentKey::for_workload_generation(&workload_id());
        match self.state().store.get(key.as_bytes()).await.expect("the intent store reads") {
            None => 0,
            Some(bytes) => u64::from_be_bytes(
                bytes.as_ref().try_into().expect("the generation is eight big-endian bytes"),
            ),
        }
    }

    // -- ticks ---------------------------------------------------------------

    fn spawn_tick(&mut self, advance: Duration) -> Tick {
        self.clock.tick(advance);
        self.tick_n += 1;
        let tick_n = self.tick_n;
        let state = self.state().clone();
        let owner = Arc::clone(&self.node().owner);
        let clock = Arc::clone(&self.clock);
        let target = target();
        let handle = tokio::spawn(async move {
            let now = clock.now();
            Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &reconciler(),
                &target,
                now,
                tick_n,
                now + Duration::from_secs(60),
                &*owner,
            ))
            .await
        });
        Tick { handle }
    }

    /// One production evaluation, run to its end.
    async fn eval(&mut self, advance: Duration) {
        let tick = self.spawn_tick(advance);
        let result = tick.join().await;
        let n = self.tick_n;
        self.line(format!(
            "evaluation {n}: {}",
            result.map_or_else(|e| format!("{e:?}"), |()| "ok".to_owned())
        ));
        self.settle().await;
    }

    /// Yield the executor until the observable state (the allocation rows, the driver starts, the
    /// CA attempts) has not changed for [`QUIESCENT_YIELDS`] consecutive yields. Bounded by
    /// [`SETTLE_YIELD_CAP`]; no wall-clock is read.
    async fn settle(&self) {
        let mut last = self.settle_fingerprint().await;
        let mut stable = 0;
        for _ in 0..SETTLE_YIELD_CAP {
            tokio::task::yield_now().await;
            let now = self.settle_fingerprint().await;
            if now == last {
                stable += 1;
                if stable >= QUIESCENT_YIELDS {
                    return;
                }
            } else {
                stable = 0;
                last = now;
            }
        }
        panic!(
            "the world did not quiesce within {SETTLE_YIELD_CAP} yields (seed {}, test {}): the observable state kept changing",
            self.seed, self.label
        );
    }

    async fn settle_fingerprint(&self) -> String {
        format!(
            "{:?}|{}|{}",
            self.rows()
                .await
                .iter()
                .map(|r| format!("{}:{:?}", r.alloc_id, r.state))
                .collect::<Vec<_>>(),
            self.log.records().len(),
            self.ca.attempts()
        )
    }

    /// As [`Self::settle`], and the clock moves one step so a scheduled driver exit is emitted and
    /// the exit observer writes its row.
    async fn settle_exit(&self) {
        self.settle().await;
        self.clock.tick(Duration::from_millis(100));
        self.settle().await;
    }

    async fn idle(&mut self, evaluations: usize, advance: Duration) {
        for _ in 0..evaluations {
            self.eval(advance).await;
        }
    }

    /// Evaluate until the workload has exactly one `Running` row, at most `max` times.
    async fn converge_until_running(&mut self, advance: Duration, max: usize) -> bool {
        for _ in 0..max {
            self.eval(advance).await;
            if self.running().await.len() == 1 {
                return true;
            }
        }
        false
    }

    /// Spawn evaluations until one reaches the parked driver `start`; evaluations that end first
    /// (the stop of a running predecessor, a launch that fails before the driver) are run to
    /// their end. `None` when no evaluation reaches the start.
    async fn until_entered(&mut self, park: &StartPark, advance: Duration) -> Option<Tick> {
        for _ in 0..10 {
            let mut tick = self.spawn_tick(advance);
            let outcome = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::select! {
                    biased;
                    () = park.entered() => None,
                    res = &mut tick.handle => Some(res),
                }
            })
            .await
            .expect("an evaluation reaches the held start or ends within the bound");
            match outcome {
                None => {
                    let n = self.tick_n;
                    self.line(format!("evaluation {n}: reached the held driver start"));
                    return Some(tick);
                }
                Some(res) => {
                    let n = self.tick_n;
                    self.line(format!(
                        "evaluation {n}: ended before the held start: {:?}",
                        res.expect("the tick task did not panic").map_err(|e| format!("{e:?}"))
                    ));
                    self.settle().await;
                }
            }
        }
        None
    }

    /// Yield (bounded by [`SETTLE_YIELD_CAP`], no wall-clock) until the parked start returns
    /// `StartSuperseded`; `false` when it never does.
    async fn wait_superseded(&self, park: &StartPark) -> bool {
        for _ in 0..SETTLE_YIELD_CAP {
            if park.observed_supersession() {
                return true;
            }
            tokio::task::yield_now().await;
        }
        park.observed_supersession()
    }

    // -- observation -----------------------------------------------------------

    async fn rows(&self) -> Vec<AllocStatusRow> {
        let id = workload_id();
        let mut rows: Vec<AllocStatusRow> = self
            .obs
            .alloc_status_rows()
            .await
            .expect("the observation store reads its rows")
            .into_iter()
            .filter(|row| row.workload_id == id)
            .collect();
        rows.sort_by_key(|row| attempt_suffix(&row.alloc_id));
        rows
    }

    async fn running(&self) -> Vec<AllocStatusRow> {
        self.rows().await.into_iter().filter(|row| row.state == AllocState::Running).collect()
    }

    async fn failed(&self) -> Vec<AllocStatusRow> {
        self.rows().await.into_iter().filter(|row| row.state == AllocState::Failed).collect()
    }

    /// The current allocation: the row with the highest attempt suffix.
    async fn current(&self) -> Option<AllocStatusRow> {
        self.rows().await.into_iter().last()
    }

    fn restart_counts(&self) -> BTreeMap<AllocationId, u32> {
        self.runtime().view_for_workload_lifecycle(&target()).restart_counts
    }

    fn issued_generations(&self) -> BTreeMap<AllocationId, u64> {
        self.runtime().view_for_workload_lifecycle(&target()).issued_generation
    }

    async fn issue_state(&self) -> IssueState {
        IssueState {
            restart_counts: self.restart_counts(),
            issued_generation: self.issued_generations(),
            driver_starts: self.log.records().len(),
            ca_attempts: self.ca.attempts(),
            rows: self.rows().await.len(),
        }
    }

    async fn snapshot(&self) -> String {
        let mut out = String::new();
        for row in self.rows().await {
            out.push_str(&format!(
                "    row {} {:?} terminal={:?} reason={:?}\n",
                row.alloc_id, row.state, row.terminal, row.reason
            ));
        }
        out.push_str(&format!("    restart_counts={:?}\n", self.restart_counts()));
        out.push_str(&format!("    issued_generation={:?}\n", self.issued_generations()));
        out.push_str(&format!("    driver starts={:?}\n", self.log.records()));
        out.push_str(&format!(
            "    max in flight={} ca attempts={} commits={}\n",
            self.log.max_in_flight.load(Ordering::SeqCst),
            self.ca.attempts(),
            self.commits
        ));
        out
    }

    /// The whole trace of the run: the schedule log and the final state. Deterministic per seed.
    async fn trace(&self) -> String {
        format!("{}\n{}", self.lines.join("\n"), self.snapshot().await)
    }

    async fn fail(&self, message: &str) -> ! {
        panic!(
            "{message}\n  seed={} test={}\n  schedule:\n    {}\n  state:\n{}",
            self.seed,
            self.label,
            self.lines.join("\n    "),
            self.snapshot().await
        );
    }
}

// ---------------------------------------------------------------------------
// The matrix
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// `workload restart` of a `Running` workload.
    RunningRestart,
    /// `workload restart` of an operator-stopped workload (the resume).
    StoppedResume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interruption {
    /// `stop_workload` while the replacement launch is held.
    Stop,
    /// A second `restart_workload` while the replacement launch is held.
    SecondRestart,
    /// `supersede_all_lanes` (node shutdown) while the replacement launch is held.
    Shutdown,
    /// The control plane dies while the replacement launch is held.
    Crash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pre {
    /// The CA is healthy.
    None,
    /// The CA fails the first launches after the commit, then recovers.
    Transient,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    origin: Origin,
    interruption: Interruption,
    pre: Pre,
}

/// What a driven cell observed. The drive asserts nothing about the property; [`check_cell`] does.
struct Observed {
    /// Whether an evaluation reached the held driver start.
    launch_reached_driver: bool,
    /// For a supersession interruption: whether the held start returned `StartSuperseded`.
    signal: Option<bool>,
    /// For a stop: whether the workload stayed down (no Running row, no launch) until the restart.
    down_after_stop: Option<bool>,
    running_after: bool,
    /// Whether two further evaluations with the successor's row present changed nothing.
    reevaluation_clean: bool,
    committed: u64,
}

async fn drive_cell(seed: u64, cell: Cell, label: &str) -> (World, Observed) {
    let mut w = World::new(seed, label).await;
    w.line(format!("cell={cell:?}"));
    let s = w.sched;
    let mut obs = Observed {
        launch_reached_driver: false,
        signal: None,
        down_after_stop: None,
        running_after: false,
        reevaluation_clean: false,
        committed: 0,
    };

    // Origin: a Running workload (and, for a resume, an operator-stopped one).
    w.deploy(Kind::Service).await;
    assert!(
        w.converge_until_running(PAST_BACKOFF, 6).await,
        "setup: the first placement reaches Running (seed {seed}, cell {cell:?})"
    );
    w.idle(s.idle_before, s.advance).await;
    if cell.origin == Origin::StoppedResume {
        w.stop().await;
        w.idle(2, s.advance).await;
        assert!(
            w.running().await.is_empty()
                && w.current().await.is_some_and(|row| row.state == AllocState::Terminated),
            "setup: the operator stop ends the allocation (seed {seed}, cell {cell:?})"
        );
    }

    // The commit, and the launch it causes held at the driver.
    if cell.pre == Pre::Transient {
        w.ca.set(CaMode::Failing(s.transient_failures));
        w.line(format!("fault: the next {} identity issuances fail", s.transient_failures));
    }
    w.restart().await;
    let advance = if cell.pre == Pre::Transient { PAST_BACKOFF } else { s.advance };
    let park = w.sim().park_next_start(true);
    let Some(held) = w.until_entered(&park, advance).await else {
        return (w, obs);
    };
    obs.launch_reached_driver = true;
    let relaunch_floor = w.log.records().len();

    // The interruption.
    match cell.interruption {
        Interruption::Stop => w.stop().await,
        Interruption::SecondRestart => w.restart().await,
        Interruption::Shutdown => {
            w.runtime().supersede_all_lanes();
            w.line("shutdown: supersede_all_lanes".to_owned());
        }
        Interruption::Crash => {}
    }
    if cell.interruption == Interruption::Crash {
        held.abort().await;
        w.line("crash: the dispatch future is dropped mid-launch".to_owned());
    } else {
        let signalled = w.wait_superseded(&park).await;
        obs.signal = Some(signalled);
        if !signalled {
            // No signal reached the held start, so it never ends. Leave it parked.
            held.abort().await;
            return (w, obs);
        }
        let result = held.join().await;
        w.line(format!("held evaluation ended: {:?}", result.map_err(|e| format!("{e:?}"))));
    }

    // The next boot: always after a shutdown or a crash, and by the seed after a stop or a
    // second restart.
    let reboot =
        matches!(cell.interruption, Interruption::Shutdown | Interruption::Crash) || s.boot_between;
    if reboot {
        w.reboot().await;
    }

    // Convergence after the interruption.
    match cell.interruption {
        Interruption::Stop => {
            w.idle(s.idle_after, advance).await;
            let starts_since = w.log.records().len() - relaunch_floor;
            obs.down_after_stop = Some(w.running().await.is_empty() && starts_since == 0);
            w.restart().await;
            obs.running_after = w.converge_until_running(advance, 12).await;
        }
        _ => {
            w.idle(s.idle_after.saturating_sub(1), advance).await;
            obs.running_after = w.converge_until_running(advance, 12).await;
        }
    }
    obs.committed = w.committed_generation().await;

    // A second evaluation with the successor's row present issues nothing.
    let before = w.issue_state().await;
    w.idle(2, PAST_BACKOFF).await;
    obs.reevaluation_clean = w.issue_state().await == before;
    (w, obs)
}

/// The properties, in the order they are asserted: the interruption takes effect; (a) the
/// workload is up for the committed generation (or, after a stop, stays down); (b) one launch in
/// flight and exactly one relaunch per abandoned launch; (c) nothing charged and no `Failed` row
/// for an abandoned launch; (d) a second evaluation issues nothing.
async fn check_cell(w: &World, obs: &Observed, cell: Cell) {
    if !obs.launch_reached_driver {
        if cell.pre == Pre::Transient {
            w.fail(
                "a transient identity_issue failure must write a charged Failed row and be relaunched until a launch reaches the driver (ADR-0337)",
            )
            .await;
        }
        w.fail("setup: an evaluation must reach the held driver start").await;
    }
    if cell.interruption != Interruption::Crash {
        if obs.signal != Some(true) {
            w.fail(LANE_SIGNAL_MESSAGE).await;
        }
    }
    let committed = w.committed_generation().await;
    if committed != w.commits {
        w.fail(&format!(
            "setup: the committed generation {committed} must equal the restart commits {}",
            w.commits
        ))
        .await;
    }

    // (a)
    if cell.interruption == Interruption::Stop && obs.down_after_stop != Some(true) {
        w.fail("after a stop the workload must stay down until the next restart").await;
    }
    if !obs.running_after {
        w.fail(&format!(
            "the abandoned generation must be relaunched: after {:?} the workload has no Running allocation for the committed generation {committed}",
            cell.interruption
        ))
        .await;
    }
    let running = w.running().await;
    let running = running.first().expect("one Running row was observed");
    let stamped = w.runtime().issued_generation_for_alloc(&target(), &running.alloc_id);
    if stamped != Some(committed) {
        w.fail(&format!(
            "the Running allocation {} must have been issued for the committed generation {committed}, issued_generation reads {stamped:?}",
            running.alloc_id
        ))
        .await;
    }

    check_launches(w, committed).await;
    check_charges(w, cell).await;

    // (d)
    if !obs.reevaluation_clean {
        w.fail("a second evaluation with the successor's row present must issue nothing").await;
    }
}

/// (b): at most one launch in flight; per generation at most one launch completes; each abandoned
/// launch of the committed generation is followed by exactly one relaunch; a generation overridden
/// by a later commit is not relaunched.
async fn check_launches(w: &World, committed: u64) {
    let max = w.log.max_in_flight.load(Ordering::SeqCst);
    if max > 1 {
        w.fail(&format!("at most one replacement launch may be in flight, saw {max}")).await;
    }
    let mut per_generation: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
    for record in w.log.records() {
        let Some(generation) = w.runtime().issued_generation_for_alloc(&target(), &record.alloc)
        else {
            w.fail(&format!("launch {} carries no issued_generation entry", record.alloc)).await;
        };
        let entry = per_generation.entry(generation).or_default();
        entry.0 += 1;
        if record.outcome.abandoned() {
            entry.1 += 1;
        }
    }
    for (generation, (launches, abandoned)) in per_generation {
        let completed = launches - abandoned;
        if completed > 1 {
            w.fail(&format!(
                "generation {generation} must be placed exactly once: {launches} launches, {abandoned} abandoned"
            ))
            .await;
        }
        if generation == committed && completed != 1 {
            w.fail(&format!(
                "each abandoned launch of generation {generation} must be followed by exactly one relaunch: {launches} launches, {abandoned} abandoned"
            ))
            .await;
        }
        if generation < committed && abandoned > 0 && completed != 0 {
            w.fail(&format!(
                "generation {generation} was overridden by a later commit and must not be relaunched: {launches} launches, {abandoned} abandoned"
            ))
            .await;
        }
    }
}

/// (c): an abandoned launch leaves no `Failed` row and changes no count; a relaunch re-assigns the
/// count the abandoned launch had.
async fn check_charges(w: &World, cell: Cell) {
    let failed = w.failed().await;
    let counts = w.restart_counts();
    if cell.pre == Pre::None {
        if !failed.is_empty() {
            w.fail("an abandoned replacement launch must write no Failed row").await;
        }
        if let Some((alloc, count)) = counts.iter().find(|(_, count)| **count != 0) {
            w.fail(&format!("an abandoned replacement launch must charge nothing: restart_counts[{alloc}] = {count}")).await;
        }
    } else {
        let expected = w.sched.transient_failures as usize;
        if failed.len() != expected || !failed.iter().all(is_identity_issue) {
            w.fail(&format!(
                "the {expected} transient failures must present as Failed rows MtlsInterceptInstallFailed {{ stage: \"identity_issue\" }}, and nothing else may be Failed"
            ))
            .await;
        }
    }
    let records = w.log.records();
    for (index, record) in records.iter().enumerate() {
        if !record.outcome.abandoned() {
            continue;
        }
        let generation = w.runtime().issued_generation_for_alloc(&target(), &record.alloc);
        let relaunch = records[index + 1..].iter().find(|later| {
            w.runtime().issued_generation_for_alloc(&target(), &later.alloc) == generation
        });
        if let Some(relaunch) = relaunch
            && counts.get(&relaunch.alloc) != counts.get(&record.alloc)
        {
            w.fail(&format!(
                "a relaunch must re-assign the count of the abandoned launch: {} has {:?}, {} has {:?}",
                record.alloc,
                counts.get(&record.alloc),
                relaunch.alloc,
                counts.get(&relaunch.alloc)
            ))
            .await;
        }
    }
}

async fn run_cell(origin: Origin, interruption: Interruption, pre: Pre, label: &str) {
    for seed in SEEDS {
        let cell = Cell { origin, interruption, pre };
        let (world, observed) = drive_cell(seed, cell, label).await;
        check_cell(&world, &observed, cell).await;
    }
}

// ---------------------------------------------------------------------------
// Matrix tests, a Service with a healthy CA
// ---------------------------------------------------------------------------

/// ADR-0334: a restart of a `Running` workload whose replacement launch is
/// held is interrupted by `stop_workload`. The signal ends the launch, the workload stays down
/// until the next restart, and that restart places once for the new generation.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn restart_of_a_running_workload_interrupted_by_stop_places_exactly_once() {
    run_cell(Origin::RunningRestart, Interruption::Stop, Pre::None, "running-restart/stop").await;
}

/// ADR-0334: a restart of a `Running` workload whose replacement launch is
/// held is overridden by a second `restart_workload`: the first launch is abandoned, the new
/// generation is placed once, the first generation is not relaunched.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn restart_of_a_running_workload_overridden_by_a_second_restart_places_exactly_once() {
    run_cell(
        Origin::RunningRestart,
        Interruption::SecondRestart,
        Pre::None,
        "running-restart/second-restart",
    )
    .await;
}

/// GUARD (count basis, ADR-0355 section 2): a restart of a `Running` workload whose replacement launch is
/// held is abandoned by node shutdown (`supersede_all_lanes`); the next boot relaunches the same
/// generation once, uncharged. The launch ends its `Pending` row `LaunchSuperseded` (ADR-0355 W2).
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn restart_of_a_running_workload_abandoned_by_shutdown_is_relaunched_on_the_next_boot() {
    run_cell(Origin::RunningRestart, Interruption::Shutdown, Pre::None, "running-restart/shutdown")
        .await;
}

/// GUARD (count basis, ADR-0355 section 2): a restart of a `Running` workload whose replacement launch is
/// held when the control plane dies. The View was persisted before the dispatch and records the
/// issued id; the launch left its `Pending` row, which boot ends `LaunchLostWithControlPlane`
/// (ADR-0355 W4) and the next evaluation relaunches.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn restart_of_a_running_workload_lost_with_the_control_plane_is_relaunched_on_the_next_boot()
{
    run_cell(Origin::RunningRestart, Interruption::Crash, Pre::None, "running-restart/crash").await;
}

/// ADR-0334: the resume of an operator-stopped workload whose launch is
/// held is interrupted by `stop_workload`; the workload stays down until the next restart.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn resume_of_a_stopped_workload_interrupted_by_stop_places_exactly_once() {
    run_cell(Origin::StoppedResume, Interruption::Stop, Pre::None, "stopped-resume/stop").await;
}

/// ADR-0334: the resume of an operator-stopped workload whose launch is held
/// is overridden by a second `restart_workload`.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn resume_of_a_stopped_workload_overridden_by_a_second_restart_places_exactly_once() {
    run_cell(
        Origin::StoppedResume,
        Interruption::SecondRestart,
        Pre::None,
        "stopped-resume/second-restart",
    )
    .await;
}

/// GUARD (count basis, ADR-0355 section 2): the resume of an operator-stopped workload whose launch is held
/// is abandoned by node shutdown; the next boot relaunches it. The launch ends `LaunchSuperseded`
/// (ADR-0355 W1).
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn resume_of_a_stopped_workload_abandoned_by_shutdown_is_relaunched_on_the_next_boot() {
    run_cell(Origin::StoppedResume, Interruption::Shutdown, Pre::None, "stopped-resume/shutdown")
        .await;
}

/// GUARD (count basis, ADR-0355 section 2): the resume of an operator-stopped workload whose launch is lost
/// with the control plane. Boot ends the `Pending` row `LaunchLostWithControlPlane` (ADR-0355 W4)
/// and the workload is relaunched.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn resume_of_a_stopped_workload_lost_with_the_control_plane_is_relaunched_on_the_next_boot() {
    run_cell(Origin::StoppedResume, Interruption::Crash, Pre::None, "stopped-resume/crash").await;
}

// ---------------------------------------------------------------------------
// The matrix with a transient identity fault ahead of the interrupted launch
// ---------------------------------------------------------------------------

/// PENDING (ADR-0337): after a restart of a `Running` workload, or the resume of an
/// operator-stopped one, the identity issuer fails the first launches and recovers. Each failure
/// is a charged `Failed` row, the launch that reaches the driver is a crash-restart successor, and
/// it is interrupted by each of the four interruptions; its relaunch re-assigns the same count.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "transient failures must present as Failed rows MtlsInterceptInstallFailed"
)]
async fn a_transient_identity_fault_then_an_interrupted_launch_places_once() {
    for (origin, label) in [
        (Origin::RunningRestart, "running-restart/transient"),
        (Origin::StoppedResume, "stopped-resume/transient"),
    ] {
        for interruption in [
            Interruption::Crash,
            Interruption::Shutdown,
            Interruption::Stop,
            Interruption::SecondRestart,
        ] {
            run_cell(origin, interruption, Pre::Transient, label).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Pre-launch failure at the CA port: persistent and transient, no interruption
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /// A first placement (`deploy`): the start arm.
    FirstPlacement,
    /// A restart of a `Running` workload: the restart arm.
    RunningRestart,
    /// The resume of an operator-stopped workload: the restart arm.
    StoppedResume,
}

struct PreRun {
    world: World,
    baseline: IssueState,
    committed: u64,
}

/// Drive a Service to `start`, arm the CA, apply the operator commit (none for a first placement)
/// and run evaluations: with the clock moving past every backoff window (`moving`) until the
/// outcome settles, or with the clock held for twelve evaluations.
async fn drive_pre_launch_failure(
    seed: u64,
    start: Start,
    mode: CaMode,
    moving: bool,
    label: &str,
) -> PreRun {
    let mut w = World::new(seed, label).await;
    let advance = if moving { PAST_BACKOFF } else { Duration::ZERO };
    w.deploy(Kind::Service).await;
    if start != Start::FirstPlacement {
        assert!(
            w.converge_until_running(PAST_BACKOFF, 6).await,
            "setup: the first placement reaches Running (seed {seed}, origin {start:?})"
        );
        if start == Start::StoppedResume {
            w.stop().await;
            w.idle(2, PAST_BACKOFF).await;
            assert!(
                w.running().await.is_empty(),
                "setup: the operator stop ends the allocation (seed {seed}, origin {start:?})"
            );
        }
    }
    let baseline = w.issue_state().await;
    w.ca.set(mode);
    w.line(format!("fault: CA {mode:?}"));
    if start != Start::FirstPlacement {
        w.restart().await;
    }
    for _ in 0..if moving { 40 } else { 12 } {
        w.eval(advance).await;
        let settled = match mode {
            CaMode::Persistent => {
                moving
                    && w.current().await.is_some_and(|row| {
                        matches!(row.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
                    })
            }
            _ => w.running().await.len() == 1,
        };
        if settled {
            break;
        }
    }
    let committed = w.committed_generation().await;
    PreRun { world: w, baseline, committed }
}

async fn check_persistent(run: &PreRun, held_clock: bool) {
    let w = &run.world;
    let failed = w.failed().await;
    if failed.is_empty() || !failed.iter().all(is_identity_issue) {
        w.fail(
            "a persistent identity_issue failure must write a charged Failed row naming MtlsInterceptInstallFailed { stage: \"identity_issue\" } (ADR-0337)",
        )
        .await;
    }
    let launches = w.ca.attempts() - run.baseline.ca_attempts;
    let counts_gained = w.restart_counts().len() - run.baseline.restart_counts.len();
    let issued_gained = w.issued_generations().len() - run.baseline.issued_generation.len();
    if held_clock {
        // A generation replacement inherits its predecessor's `last_failure_seen_at`
        // (`workload_lifecycle.rs` ~1311-1333), so its first crash restart is immediate and the
        // later ones are gated: with the clock held the launches are the replacement plus at most
        // one ungated crash restart, and nothing after that within the window.
        if launches > 2 {
            w.fail(&format!(
                "with the clock held within one backoff window a persistent failure may launch the replacement and at most one ungated crash restart, saw {launches}"
            ))
            .await;
        }
    } else {
        let current = w.current().await.expect("a current allocation");
        if !matches!(current.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
            || current.state != AllocState::Failed
        {
            w.fail("a persistent identity_issue failure must end the Service Failed with terminal BackoffExhausted").await;
        }
        let bound = RESTART_BACKOFF_CEILING as usize + 1;
        if launches > bound {
            w.fail(&format!(
                "the launches after one operator commit must be at most the restart ceiling plus the one uncharged replacement ({bound}), saw {launches}"
            ))
            .await;
        }
    }
    if counts_gained > launches || issued_gained > launches {
        w.fail(&format!(
            "issued_generation and restart_counts must gain no more entries than there were launches: launches={launches} restart_counts+{counts_gained} issued_generation+{issued_gained}"
        ))
        .await;
    }
}

async fn check_transient(run: &PreRun) {
    let w = &run.world;
    let failures = w.sched.transient_failures as usize;
    let failed = w.failed().await;
    if failed.len() != failures || !failed.iter().all(is_identity_issue) {
        w.fail(&format!(
            "the {failures} transient failures must present as Failed rows MtlsInterceptInstallFailed {{ stage: \"identity_issue\" }}"
        ))
        .await;
    }
    let running = w.running().await;
    let [running] = running.as_slice() else {
        w.fail("a transient identity_issue failure must recover: the Service reaches Running")
            .await;
    };
    let stamped = w.runtime().issued_generation_for_alloc(&target(), &running.alloc_id);
    if stamped != Some(run.committed) {
        w.fail(&format!(
            "the recovered allocation must be issued for the committed generation {}, issued_generation reads {stamped:?}",
            run.committed
        ))
        .await;
    }
    let launches = w.ca.attempts() - run.baseline.ca_attempts;
    let counts_gained = w.restart_counts().len() - run.baseline.restart_counts.len();
    let issued_gained = w.issued_generations().len() - run.baseline.issued_generation.len();
    if launches != failures + 1 || counts_gained > launches || issued_gained > launches {
        w.fail(&format!(
            "issued_generation and restart_counts must gain no more entries than there were launches ({} failed + 1 recovered): launches={launches} restart_counts+{counts_gained} issued_generation+{issued_gained}",
            failures
        ))
        .await;
    }
}

/// PENDING (ADR-0337): after a restart of a `Running` workload, the resume of an operator-stopped
/// one, or a first placement (`deploy`, the start arm), a persistent identity failure writes
/// charged `Failed` rows and the Service ends `BackoffExhausted` within the restart ceiling plus
/// the one uncharged generation replacement (the first launch, for a first placement). The clock
/// moves past every backoff window and the schedule does not enter this path, so one seed serves
/// every origin.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "a persistent identity_issue failure must write a charged Failed row naming MtlsInterceptInstallFailed"
)]
async fn a_persistent_identity_failure_ends_backoff_exhausted() {
    for (start, label) in [
        (Start::RunningRestart, "persistent/restart"),
        (Start::StoppedResume, "persistent/resume"),
        (Start::FirstPlacement, "persistent/first-placement"),
    ] {
        let run = drive_pre_launch_failure(SEEDS[0], start, CaMode::Persistent, true, label).await;
        check_persistent(&run, false).await;
    }
}

/// PENDING (ADR-0337): with `SimClock` held within one backoff window a persistent failure does
/// not launch on every evaluation: the failed launch writes its row, and the backoff gate bounds
/// the rest. The schedule does not enter this path, so one seed serves every origin.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "a persistent identity_issue failure must write a charged Failed row naming MtlsInterceptInstallFailed"
)]
async fn a_persistent_identity_failure_launches_a_bounded_number_of_times_within_one_backoff_window()
 {
    for start in [Start::RunningRestart, Start::StoppedResume, Start::FirstPlacement] {
        let run = drive_pre_launch_failure(
            SEEDS[0],
            start,
            CaMode::Persistent,
            false,
            "persistent/held-clock",
        )
        .await;
        check_persistent(&run, true).await;
        // After the ungated restart every further launch is gated: one launch per backoff window.
        let mut w = run.world;
        for window in 0..3 {
            let before = w.ca.attempts();
            w.eval(PAST_BACKOFF).await;
            for _ in 0..4 {
                w.eval(Duration::ZERO).await;
            }
            let launched = w.ca.attempts() - before;
            if launched > 1 {
                w.fail(&format!(
                    "after the ungated crash restart at most one launch per backoff window, saw {launched} in window {window} (origin {start:?})"
                ))
                .await;
            }
        }
    }
}

/// PENDING (ADR-0337): after a restart of a `Running` workload, the resume of an operator-stopped
/// one, or a first placement, a transient identity failure recovers to `Running`; the failed
/// launches present as `Failed` rows with the pinned reason and `issued_generation` and
/// `restart_counts` gain no more entries than there were launches. The seed picks how many
/// launches fail, so every seed runs for every origin.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "transient failures must present as Failed rows MtlsInterceptInstallFailed"
)]
async fn a_transient_identity_failure_recovers_to_running() {
    for (start, label) in [
        (Start::RunningRestart, "transient/restart"),
        (Start::StoppedResume, "transient/resume"),
        (Start::FirstPlacement, "transient/first-placement"),
    ] {
        for seed in SEEDS {
            let run = drive_pre_launch_failure_transient(seed, start, label).await;
            check_transient(&run).await;
        }
    }
}

async fn drive_pre_launch_failure_transient(seed: u64, start: Start, label: &str) -> PreRun {
    let failures = schedule(seed).transient_failures;
    drive_pre_launch_failure(seed, start, CaMode::Failing(failures), true, label).await
}

// ---------------------------------------------------------------------------
// Workload kind: a Job
// ---------------------------------------------------------------------------

/// Run a Job to `Running` (CA healthy).
async fn running_job(seed: u64, label: &str) -> World {
    let mut w = World::new(seed, label).await;
    w.deploy(Kind::Job).await;
    assert!(
        w.converge_until_running(PAST_BACKOFF, 6).await,
        "setup: the Job's first placement reaches Running (seed {seed})"
    );
    w
}

/// PENDING (ADR-0335 item 3, H1): a restart of a running Job ends it and issues nothing after.
/// Today the stop of the running instance leaves the generation pending and the reconciler issues
/// an id for it that the shim fences.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_restart_of_a_running_job_ends_it_and_issues_nothing_after() {
    for seed in SEEDS {
        let mut w = running_job(seed, "job/restart-of-running").await;
        let before = w.issue_state().await;
        w.restart().await;
        w.idle(1, PAST_BACKOFF).await;
        assert!(
            w.running().await.is_empty(),
            "setup: the restart ends the running Job instance (seed {seed})"
        );
        w.idle(w.sched.job_settle_ticks, PAST_BACKOFF).await;
        if w.sched.boot_between {
            w.reboot().await;
            w.idle(2, PAST_BACKOFF).await;
        }
        let mut after = w.issue_state().await;
        after.rows = before.rows;
        if after != before {
            w.fail("a restart of a running Job must end it and issue nothing after: no id, no issued_generation or restart_counts entry, no launch (H1)")
                .await;
        }
        if !w.running().await.is_empty() {
            w.fail("a restart of a running Job must leave it down").await;
        }
    }
}

/// The Job pre-driver case (J-1, RA11): a Job whose identity issuance fails writes one `Failed`
/// row; the next tick finalises it `Failed { exit_code: Some(0) }`; no further `StartAllocation`
/// is emitted however many ticks run, and the next boot changes nothing.
async fn job_identity_failure(seed: u64, label: &str) {
    let mut w = World::new(seed, label).await;
    w.ca.set(CaMode::Persistent);
    w.deploy(Kind::Job).await;
    w.eval(PAST_BACKOFF).await;
    let rows = w.rows().await;
    let [row] = rows.as_slice() else {
        w.fail("a Job whose SVID issuance fails must write exactly one Failed row (J-1)").await;
    };
    if row.state != AllocState::Failed || !is_identity_issue(row) {
        w.fail("a Job whose SVID issuance fails must write one Failed row MtlsInterceptInstallFailed { stage: \"identity_issue\" } (J-1)").await;
    }
    w.eval(PAST_BACKOFF).await;
    let finalised = w.current().await.and_then(|row| row.terminal);
    if finalised != Some(TerminalCondition::Failed { exit_code: Some(0) }) {
        w.fail(&format!("the next tick must finalise the Job Failed {{ exit_code: Some(0) }}, terminal reads {finalised:?}")).await;
    }
    let settled = w.issue_state().await;
    for round in 0..w.sched.job_settle_ticks {
        w.eval(PAST_BACKOFF).await;
        if round == 3 && w.sched.boot_between {
            w.reboot().await;
        }
    }
    let after = w.issue_state().await;
    if after != settled || w.ca.attempts() != 1 || !w.running().await.is_empty() {
        w.fail(&format!(
            "a Job's pre-driver failure is final: no further StartAllocation however many ticks run (ca attempts {})",
            w.ca.attempts()
        ))
        .await;
    }
}

/// PENDING (ADR-0337, J-1): persistent identity failure of a Job.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "a Job whose SVID issuance fails must write one Failed row MtlsInterceptInstallFailed"
)]
async fn a_job_whose_identity_issuance_persistently_fails_is_finalised_failed_and_never_relaunched()
{
    for seed in SEEDS {
        job_identity_failure(seed, "job/persistent-identity-failure").await;
    }
}

// ---------------------------------------------------------------------------
// A crash-restart successor abandoned by the interruption
// ---------------------------------------------------------------------------

/// A Service crashes; the crash-restart successor's launch is held and abandoned by `interruption`.
/// The relaunch is for the same crash: it re-assigns the same count, not one more.
async fn crash_restart_successor_abandoned(seed: u64, interruption: Interruption, label: &str) {
    let mut w = World::new(seed, label).await;
    w.deploy(Kind::Service).await;
    assert!(w.converge_until_running(PAST_BACKOFF, 6).await, "setup: Running (seed {seed})");
    let crashed = w.running().await.remove(0).alloc_id;
    let park = w.sim().park_next_start(true);
    w.sim().inject_exit_after(
        &crashed,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );
    w.settle_exit().await;
    let Some(held) = w.until_entered(&park, PAST_BACKOFF).await else {
        panic!("setup: the crash restart's launch reaches the driver (seed {seed})");
    };
    match interruption {
        Interruption::Shutdown => {
            w.runtime().supersede_all_lanes();
            if !w.wait_superseded(&park).await {
                held.abort().await;
                w.fail(LANE_SIGNAL_MESSAGE).await;
            }
            let _ = held.join().await;
        }
        Interruption::Crash => held.abort().await,
        other => panic!("not part of this scenario: {other:?}"),
    }
    w.reboot().await;
    if !w.converge_until_running(PAST_BACKOFF, 12).await {
        w.fail("the abandoned crash restart must be relaunched until the workload is Running")
            .await;
    }
    let counts = w.restart_counts();
    let abandoned = AllocationId::new(&format!("alloc-{WORKLOAD}-1")).expect("alloc id");
    let relaunched = w.running().await.remove(0).alloc_id;
    if counts.get(&abandoned) != Some(&1) || counts.get(&relaunched) != Some(&1) {
        w.fail(&format!(
            "a crash-restart relaunch must re-assign the count of the abandoned launch (1), not charge twice: abandoned {abandoned}={:?} relaunched {relaunched}={:?}",
            counts.get(&abandoned),
            counts.get(&relaunched)
        ))
        .await;
    }
    let failed = w.failed().await;
    if failed.len() != 1 || failed[0].alloc_id.as_str() != format!("alloc-{WORKLOAD}-0") {
        w.fail(
            "only the crashed allocation may be Failed: the abandoned launch writes no Failed row",
        )
        .await;
    }
}

/// GUARD (count basis, RB-13), charge (c): a crash-restart successor lost with the control plane
/// is relaunched with the same count (1, not 2).
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_crash_restart_successor_lost_with_the_control_plane_is_relaunched_with_the_same_count() {
    for seed in SEEDS {
        crash_restart_successor_abandoned(seed, Interruption::Crash, "crash-restart/crash").await;
    }
}

/// GUARD (count basis, RB-13): the same for a launch abandoned by node shutdown
/// (`LaunchSuperseded`).
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_crash_restart_successor_abandoned_by_shutdown_is_relaunched_with_the_same_count() {
    for seed in SEEDS {
        crash_restart_successor_abandoned(seed, Interruption::Shutdown, "crash-restart/shutdown")
            .await;
    }
}

// ---------------------------------------------------------------------------
// A Job whose first launch is abandoned (ADR-0355: a launch that never ran is never a natural exit)
// ---------------------------------------------------------------------------

/// A Job's first launch is held at the driver and abandoned by `interruption`; the next boot
/// relaunches it. The Job is not finalised (`is_natural_exit` excludes an abandoned row), the
/// launch that never ran charges nothing, and exactly one launch completes.
async fn job_first_launch_abandoned(seed: u64, interruption: Interruption, label: &str) {
    let mut w = World::new(seed, label).await;
    w.deploy(Kind::Job).await;
    let park = w.sim().park_next_start(true);
    let Some(held) = w.until_entered(&park, PAST_BACKOFF).await else {
        w.fail("setup: the Job's first launch must reach the held driver start").await;
    };
    match interruption {
        Interruption::Shutdown => {
            w.runtime().supersede_all_lanes();
            w.line("shutdown: supersede_all_lanes".to_owned());
            if !w.wait_superseded(&park).await {
                held.abort().await;
                w.fail(LANE_SIGNAL_MESSAGE).await;
            }
            let _ = held.join().await;
        }
        Interruption::Crash => {
            held.abort().await;
            w.line("crash: the dispatch future is dropped mid-launch".to_owned());
        }
        other => panic!("not part of this scenario: {other:?}"),
    }
    w.reboot().await;
    if !w.converge_until_running(PAST_BACKOFF, 12).await {
        w.fail("the abandoned Job launch must be relaunched until the Job is Running").await;
    }
    if let Some(row) = w.rows().await.into_iter().find(|row| row.terminal.is_some()) {
        w.fail(&format!(
            "a Job whose launch never ran must not be finalised: {} carries terminal {:?}",
            row.alloc_id, row.terminal
        ))
        .await;
    }
    if !w.failed().await.is_empty() {
        w.fail("an abandoned Job launch must write no Failed row").await;
    }
    if let Some((alloc, count)) = w.restart_counts().iter().find(|(_, count)| **count != 0) {
        w.fail(&format!(
            "a launch abandoned before anything ran must charge nothing: restart_counts[{alloc}] = {count}"
        ))
        .await;
    }
    let completed = w.log.records().iter().filter(|record| !record.outcome.abandoned()).count();
    if completed != 1 {
        w.fail(&format!(
            "the abandoned Job launch must be followed by exactly one relaunch that completes, saw {completed}"
        ))
        .await;
    }
    let before = w.issue_state().await;
    w.idle(2, PAST_BACKOFF).await;
    if w.issue_state().await != before {
        w.fail("a second evaluation with the relaunched Job's row present must issue nothing")
            .await;
    }
}

/// GUARD (count basis, RB-13), S-INV-01, Job dimension: a Job whose first launch is lost with the
/// control plane is relaunched once on the next boot and is not finalised, because
/// `is_natural_exit` excludes the abandoned row.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_job_whose_first_launch_is_lost_with_the_control_plane_is_relaunched_once_and_not_finalised()
 {
    for seed in SEEDS {
        job_first_launch_abandoned(seed, Interruption::Crash, "job/first-launch-crash").await;
    }
}

/// GUARD (count basis, RB-13), S-INV-01, Job dimension: a Job whose first launch is abandoned by
/// node shutdown is relaunched once on the next boot and is not finalised.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_job_whose_first_launch_is_abandoned_by_shutdown_is_relaunched_once_and_not_finalised() {
    for seed in SEEDS {
        job_first_launch_abandoned(seed, Interruption::Shutdown, "job/first-launch-shutdown").await;
    }
}

// ---------------------------------------------------------------------------
// GUARDs that hold today
// ---------------------------------------------------------------------------

/// GUARD (d, a): a restart of a `Running` workload with no interruption places one successor for
/// the committed generation, and a second evaluation with its row present issues nothing.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_restart_with_no_interruption_places_once_and_a_second_evaluation_issues_nothing() {
    for seed in SEEDS {
        let mut w = World::new(seed, "baseline/restart").await;
        w.deploy(Kind::Service).await;
        assert!(w.converge_until_running(PAST_BACKOFF, 6).await, "setup: Running (seed {seed})");
        w.restart().await;
        assert!(
            w.converge_until_running(w.sched.advance, 8).await,
            "setup: the restarted workload is Running again (seed {seed})"
        );
        let running = w.running().await.remove(0);
        let committed = w.committed_generation().await;
        assert_eq!(
            w.runtime().issued_generation_for_alloc(&target(), &running.alloc_id),
            Some(committed),
            "the successor was issued for the committed generation (seed {seed})"
        );
        let before = w.issue_state().await;
        w.idle(3, PAST_BACKOFF).await;
        assert_eq!(
            w.issue_state().await,
            before,
            "a second evaluation with the row present must issue nothing (seed {seed})"
        );
        check_launches(&w, committed).await;
    }
}

/// GUARD (a): an operator stop keeps the workload down until the next restart. Across evaluations
/// and the next boot there is no launch; the next restart places once for the new generation.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn an_operator_stop_keeps_the_workload_down_until_the_next_restart() {
    for seed in SEEDS {
        let mut w = World::new(seed, "baseline/stop").await;
        w.deploy(Kind::Service).await;
        assert!(w.converge_until_running(PAST_BACKOFF, 6).await, "setup: Running (seed {seed})");
        w.stop().await;
        w.idle(3, PAST_BACKOFF).await;
        let starts = w.log.records().len();
        assert!(w.running().await.is_empty(), "the stop ends the allocation (seed {seed})");
        if w.sched.boot_between {
            w.reboot().await;
        }
        w.idle(w.sched.idle_after + 2, PAST_BACKOFF).await;
        assert!(
            w.running().await.is_empty() && w.log.records().len() == starts,
            "after a stop the workload stays down until the next restart (seed {seed})"
        );
        w.restart().await;
        assert!(
            w.converge_until_running(PAST_BACKOFF, 8).await,
            "the restart resumes it (seed {seed})"
        );
        let running = w.running().await.remove(0);
        assert_eq!(
            w.runtime().issued_generation_for_alloc(&target(), &running.alloc_id),
            Some(w.committed_generation().await),
            "the resumed allocation was issued for the committed generation (seed {seed})"
        );
    }
}

/// GUARD, reproducibility: the same seed gives the same trace. A control-plane-crash cell (no lane
/// table needed) runs twice per seed; the schedule log, the rows, the View and every driver start
/// are identical.
/// Contract shape: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn the_same_seed_gives_the_same_trace() {
    for seed in SEEDS {
        let cell = Cell {
            origin: Origin::RunningRestart,
            interruption: Interruption::Crash,
            pre: Pre::None,
        };
        let (first, _) = drive_cell(seed, cell, "determinism/first").await;
        let (second, _) = drive_cell(seed, cell, "determinism/second").await;
        let (a, b) = (first.trace().await, second.trace().await);
        assert_eq!(a, b, "the same seed {seed} must give the same trace");
        let other = drive_cell(seed + 1, cell, "determinism/other").await.0.trace().await;
        assert_ne!(a, other, "a different seed gives a different schedule (seed {seed})");
    }
}
