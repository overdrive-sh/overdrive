//! netns-density-295 correctness-recovery proof §3.2 — node-wide guest-network
//! attachment admission (seeded Tier-1 simulation).
//!
//! # Contract under test (the CORRECT contract, never today's behaviour)
//!
//! ADR-0121 / feature-delta C-295-F (F1) and Gate G-295-0: a node admits at
//! most **16,384 active allocations**, each holding one guest-network
//! attachment (address lease, TAP, FDB entry, TCX link, endpoint-map entry,
//! guard member — ADR-0117 T1). A node at 16,384 returns the existing
//! `NoCapacity` placement outcome and never calls the guest-address pool.
//! The recovery plan (correctness gap 1) states the defect: the cap is
//! evaluated over workload-local rows, not over the node-wide admitted and
//! in-flight population.
//!
//! Attachment classes, all observed at the guest-network driven port and the
//! observation store (never at private state):
//!
//! | Class     | Lease held | Allocation row                         | Counts as admitted |
//! |-----------|------------|----------------------------------------|--------------------|
//! | in-flight | yes        | none / `Pending` / `Draining` / `Suspended` | yes           |
//! | Running   | yes        | `Running`                              | yes                |
//! | retiring  | yes        | terminal (`Failed` / `Terminated`)     | NOT decided — see below |
//! | released  | no         | any                                    | no                 |
//!
//! ADR-0121 explicitly reserves `/16` pool headroom "for predecessor/successor
//! overlap and cleanup residue". Whether a retiring attachment counts against
//! the 16,384 cap is therefore NOT pinned by the accepted design. This proof
//! does not invent that answer: it records the retiring case as an
//! observation and asserts only the invariant that holds under either
//! reading — the admitted population (in-flight + Running) never exceeds
//! 16,384.
//!
//! # Invariants asserted
//!
//! | Id    | Kind     | Statement |
//! |-------|----------|-----------|
//! | NA-1  | safety   | A new workload placed while 16,384 distinct workloads are Running acquires no attachment. |
//! | NA-2  | safety   | A new workload placed while 16,383 are Running and one admitted workload is in flight (lease held, no `Running` row yet) acquires no attachment. |
//! | NA-4a | safety   | Replacing a crashed Service allocation never makes the admitted population exceed 16,384 at any lease acquisition. |
//! | NA-4a-L | liveness | Once capacity exists, the crashed Service is replaced and its old attachment released (a guard against NA-4a passing vacuously). |
//! | NA-4b | safety   | Resuming an operator-stopped Service never makes the admitted population exceed 16,384 at any lease acquisition. |
//! | NA-4b-L | liveness | Once capacity exists, the stopped Service resumes (a guard against NA-4b passing vacuously). |
//! | NA-5  | liveness | With 16,383 attachments live after a release, a new workload's placement acquires an attachment (both the in-flight and the plain-placement variants). |
//! | NA-G  | safety   | The peak admitted population at any lease acquisition over the whole run is at most 16,384. |
//!
//! Observations (not verdicts): OBS-RETIRING (placement at 16,383 Running + 1
//! retiring) and OBS-OVERLAP (peak lease population including retiring and
//! replacement overlap, compared with the ADR-0121 headroom).
//!
//! # Production owner path (causes are driven, consequences are observed)
//!
//! - **Operator input**: the Job and Service intents, stop sentinel, and
//!   restart generation reach the real `LocalIntentStore` with exactly the
//!   store effects of `handlers::submit_workload` (`put_if_absent` of
//!   `workloads/<id>`, then `put` of `workloads/<id>/kind`; for a Service also
//!   the frontend-address assignment, VIP allocation and listener-fact
//!   upsert),
//!   `handlers::stop_workload` (`put_if_absent` of `workloads/<id>/stop`) and
//!   `handlers::restart_workload` (`txn[IncrementU64 generation, Delete
//!   stop]`). The handlers take axum extractors and `overdrive-sim` has no
//!   axum edge, so their store effects are reproduced here, not the HTTP
//!   framing. The handlers author no admission decision.
//! - **Every admission decision** runs through
//!   `run_convergence_tick_with_guest_network_provisioner_for_test`, the
//!   production evaluation with only the guest-network driven port
//!   substituted. The registered `WorkloadLifecycle` hydrates `desired` and
//!   `actual`. The pure `reconcile` calls `overdrive_core::scheduler::schedule`
//!   (or emits `RestartAllocation`). The runtime `ViewStore` persists the view,
//!   and the production action shim dispatches. The shim takes the lease from
//!   the process-global guest-address pool and then calls the port.
//! - **Driven ports substituted**: `SimDriver`, `SimObservationStore`,
//!   `SimViewStore`, `SimClock`, `SimCa`, `SimDataplane`, and
//!   `SimSharedGuestNetworkOwner` behind [`LeaseLedger`]. The ledger observes
//!   lease acquisition (the shim assigns the pool lease and immediately calls
//!   `provision`) and lease release (the shim calls `release` synchronously
//!   after a successful `teardown`). On request it parks exactly one
//!   `provision` call. That produces the in-flight interleaving that
//!   `spawn_convergence_loop` reaches by running up to
//!   `CONVERGENCE_MAX_IN_FLIGHT = 8` evaluations on distinct targets at once.
//!   The decorator controls ordering only. It never authors a row, a lease, or
//!   a decision.
//! - **Crash stimulus**: `SimDriver::inject_exit_after(.., Crashed)`, consumed
//!   by the production `worker::exit_observer`, which authors the `Failed`
//!   row. No row is seeded. The crash victim is a Service because a crashed
//!   Job is a natural exit: `WorkloadLifecycle` finalizes it and releases its
//!   attachment, but never replaces it.
//!
//! # Outcome oracle
//!
//! At every lease acquisition after the fill, the ledger reads the
//! observation store and counts the admitted population exactly (live leases
//! whose row is absent or non-terminal). Each placement report also carries
//! the port-exposed diagnostics that locate the defect: node-wide `Running`
//! rows versus the workload-local `Running` rows that `hydrate_actual` hands
//! to the scheduler.
//!
//! # Evidence tier, bounds, and reproduction
//!
//! Tier 1: in-process, current-thread runtime, explicit interleaving, seeded
//! victims, fill order and block order. The run is bounded by the fill
//! (16,384 evaluations), four blocks, and fixed settle budgets. It never
//! touches kernel state. The guest-address pool is process-global, so this
//! binary holds exactly one test. Reproduce with
//! `OVERDRIVE_ND295_ADMISSION_SEED=<seed> cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --no-capture -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'`.

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::doc_markdown,
    reason = "seeded proof harness: fixture preconditions fail loudly, the verdict table is the evidence"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use overdrive_control_plane::guest_network::{
    GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, Service, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{DriverType, ExitKind};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome, TxnOp};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::view_store::SimViewStore;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use tokio::sync::oneshot;

/// The accepted T1 node-wide admission cap (ADR-0117 / ADR-0121; the
/// production constant is the private `MAX_GUEST_NETWORK_ATTACHMENTS` in
/// `overdrive-core::scheduler`).
const CAP: usize = 16_384;
/// ADR-0121 pool headroom: the `/16` action pool minus network, broadcast
/// and gateway (`guest_network::action_pool`).
const POOL_CAPACITY: usize = 65_533;
/// The Phase-1 single-node baseline id the scheduler places onto
/// (`workload_lifecycle::baseline_nodes_phase1`).
const NODE: &str = "local";
/// Per-workload memory demand; see `SimNode::operator_deploys`.
const DEMAND_MEMORY_BYTES: u64 = 256 * 1024;
const SEED_ENV: &str = "OVERDRIVE_ND295_ADMISSION_SEED";
const DEFAULT_SEED: u64 = 0x0295_0032_A0D1_0001;
/// Bounded convergence retries for release/replacement settling.
const SETTLE_TICKS: usize = 6;
/// Bounded clock nudges while waiting for the exit observer.
const CRASH_POLL_BUDGET: usize = 2_000;

// ---------------------------------------------------------------------------
// Port observation: the guest-network driven port, decorated.
// ---------------------------------------------------------------------------

struct Park {
    entered: oneshot::Sender<AllocationId>,
    release: oneshot::Receiver<()>,
}

#[derive(Default)]
struct LedgerState {
    live: BTreeSet<AllocationId>,
    lease_peak: usize,
    admitted_peak: usize,
    admitted_peak_at: Option<AllocationId>,
    window_admitted_peak: Option<usize>,
    window_acquired: Vec<AllocationId>,
}

/// Records lease acquisition and release at the guest-network driven port.
///
/// The action shim assigns the pool lease (`assign_action_plan`) and
/// immediately calls `GuestNetworkProvisioner::provision`. It calls
/// `release_action_plan` synchronously after a successful `teardown`, and it
/// calls `teardown` only while a plan is held. So `live` is exactly the
/// node's held lease set.
struct LeaseLedger {
    inner: SimSharedGuestNetworkOwner,
    obs: Arc<dyn ObservationStore>,
    measure_admitted: AtomicBool,
    state: Mutex<LedgerState>,
    park_next: Mutex<Option<Park>>,
}

impl LeaseLedger {
    fn new(obs: Arc<dyn ObservationStore>) -> Self {
        Self {
            inner: SimSharedGuestNetworkOwner::default(),
            obs,
            measure_admitted: AtomicBool::new(false),
            state: Mutex::new(LedgerState::default()),
            park_next: Mutex::new(None),
        }
    }

    fn live(&self) -> BTreeSet<AllocationId> {
        self.state.lock().live.clone()
    }

    fn live_count(&self) -> usize {
        self.state.lock().live.len()
    }

    fn holds_for(&self, workload: &str) -> bool {
        self.state.lock().live.iter().any(|alloc| workload_of(alloc) == Some(workload))
    }

    /// After the fill: count the admitted population exactly at every lease
    /// acquisition. The fill itself is bounded by construction (the harness
    /// checks one new lease per filler and no retiring or in-flight lease).
    fn start_measuring(&self) {
        let mut state = self.state.lock();
        state.admitted_peak = state.live.len();
        state.admitted_peak_at = None;
        drop(state);
        self.measure_admitted.store(true, Ordering::SeqCst);
    }

    fn begin_window(&self) {
        let mut state = self.state.lock();
        state.window_admitted_peak = None;
        state.window_acquired.clear();
    }

    fn window(&self) -> (Option<usize>, Vec<AllocationId>) {
        let state = self.state.lock();
        (state.window_admitted_peak, state.window_acquired.clone())
    }

    fn peaks(&self) -> (usize, Option<AllocationId>, usize) {
        let state = self.state.lock();
        (state.admitted_peak, state.admitted_peak_at.clone(), state.lease_peak)
    }

    /// Park the next `provision` call after its lease was taken. Returns the
    /// signal that it parked and the handle that releases it.
    fn arm_park(&self) -> (oneshot::Receiver<AllocationId>, oneshot::Sender<()>) {
        let (entered, entered_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        *self.park_next.lock() = Some(Park { entered, release });
        (entered_rx, release_tx)
    }
}

/// Admitted = live lease whose row is absent or non-terminal.
fn admitted_among(live: &BTreeSet<AllocationId>, rows: &[AllocStatusRow]) -> usize {
    let terminal: BTreeSet<&AllocationId> =
        rows.iter().filter(|row| row.state.is_terminal()).map(|row| &row.alloc_id).collect();
    live.iter().filter(|alloc| !terminal.contains(alloc)).count()
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for LeaseLedger {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        let live_now = {
            let mut state = self.state.lock();
            state.live.insert(plan.alloc().clone());
            state.lease_peak = state.lease_peak.max(state.live.len());
            state.window_acquired.push(plan.alloc().clone());
            self.measure_admitted.load(Ordering::SeqCst).then(|| state.live.clone())
        };
        if let Some(live_now) = live_now {
            let rows = self.obs.alloc_status_rows().await.expect("observation read in ledger");
            let admitted = admitted_among(&live_now, &rows);
            let mut state = self.state.lock();
            if admitted > state.admitted_peak {
                state.admitted_peak = admitted;
                state.admitted_peak_at = Some(plan.alloc().clone());
            }
            state.window_admitted_peak =
                Some(state.window_admitted_peak.map_or(admitted, |peak| peak.max(admitted)));
        }
        let park = self.park_next.lock().take();
        if let Some(park) = park {
            park.entered.send(plan.alloc().clone()).expect("harness is awaiting the park signal");
            park.release.await.expect("harness releases the parked provision");
        }
        self.inner.provision(plan).await
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.inner.activate(plan).await
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        let outcome = self.inner.teardown(plan).await;
        if outcome.is_ok() {
            self.state.lock().live.remove(plan.alloc());
        }
        outcome
    }
}

/// `mint_alloc_id` grammar: `alloc-<workload>-<attempt>`.
fn workload_of(alloc: &AllocationId) -> Option<&str> {
    alloc.as_str().strip_prefix("alloc-")?.rsplit_once('-').map(|(workload, _)| workload)
}

// ---------------------------------------------------------------------------
// Node composition (production AppState + registered WorkloadLifecycle).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Census {
    running: usize,
    in_flight: usize,
    retiring: usize,
}

impl Census {
    const fn admitted(self) -> usize {
        self.running + self.in_flight
    }

    const fn live(self) -> usize {
        self.running + self.in_flight + self.retiring
    }
}

#[derive(Debug, Clone)]
struct Decision {
    before: Census,
    node_running_rows: usize,
    scheduler_input_rows: usize,
    admitted: bool,
    acquired: Vec<AllocationId>,
    admitted_at_acquisition: Option<usize>,
    dispatch: Result<(), String>,
}

impl Decision {
    fn evidence(&self) -> String {
        format!(
            "before: live={} (Running {}, in-flight {}, retiring {}; admitted {}); node-wide \
             Running rows={}; workload-local Running rows handed to the scheduler={}; acquired \
             attachment={} {:?}; admitted population at that acquisition={:?}; dispatch={:?}",
            self.before.live(),
            self.before.running,
            self.before.in_flight,
            self.before.retiring,
            self.before.admitted(),
            self.node_running_rows,
            self.scheduler_input_rows,
            self.admitted,
            self.acquired,
            self.admitted_at_acquisition,
            self.dispatch,
        )
    }
}

struct SimNode {
    _tmp: TempDir,
    state: AppState,
    driver: Arc<SimDriver>,
    clock: Arc<SimClock>,
    ledger: Arc<LeaseLedger>,
    reconciler: ReconcilerName,
    tick: AtomicU64,
    seed: u64,
    trace: Mutex<Vec<String>>,
}

impl SimNode {
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
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
        let state = AppState::new(
            store,
            store_path,
            obs.clone(),
            Arc::new(runtime),
            driver.clone(),
            clock.clone(),
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            std::net::Ipv4Addr::LOCALHOST,
        );
        // The production exit observer authors a crashed allocation's
        // `Failed` row from the driver's exit event (production composes it
        // in `run_server_with_obs_and_driver`).
        exit_observer::spawn(
            state.obs.clone(),
            driver.clone(),
            state.lifecycle_events.clone(),
            clock.clone(),
        );
        Self {
            _tmp: tmp,
            state,
            driver,
            clock,
            ledger: Arc::new(LeaseLedger::new(obs)),
            reconciler: ReconcilerName::new("workload-lifecycle").expect("reconciler name"),
            tick: AtomicU64::new(0),
            seed,
            trace: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, line: String) {
        self.trace.lock().push(line);
    }

    fn harness_failure(&self, what: &str) -> ! {
        let trace = self.trace.lock().join("\n  ");
        panic!(
            "seed={seed}: harness precondition failed (not a contract verdict): {what}\n\
             trace:\n  {trace}\n\
             reproduce: {SEED_ENV}={seed} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
             --features integration-tests --no-capture \
             -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'",
            seed = self.seed,
        );
    }

    fn workload_id(workload: &str) -> WorkloadId {
        WorkloadId::new(workload).expect("valid workload id")
    }

    /// Store effects of `handlers::submit_workload` for a fresh Job intent.
    ///
    /// Demand is 0 mCPU and 256 KiB so the existing CPU/memory check can
    /// never bind before the attachment cap, even once the scheduler sees the
    /// node-wide population. The baseline node has 4,000 mCPU and 8 GiB, and
    /// (CAP + 2) × 256 KiB < 8 GiB. The existing pure cap test isolates the
    /// boundary the same way. With a larger demand, a correct node-wide
    /// implementation would refuse the fill on CPU/memory long before
    /// 16,384, and this oracle would stop being usable after the fix.
    async fn operator_deploys(&self, workload: &str) {
        let job = Job::from_submit(JobSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: DEMAND_MEMORY_BYTES },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/nd295/kernel".to_owned(),
                rootfs: "/nd295/rootfs.ext4".to_owned(),
            }),
        })
        .expect("valid job spec");
        let archived =
            WorkloadIntent::Job(job.clone()).archive_for_store().expect("archive workload intent");
        let key = IntentKey::for_workload(&job.id);
        let outcome = self
            .state
            .store
            .put_if_absent(key.as_bytes(), archived.as_ref())
            .await
            .expect("intent write");
        if !matches!(outcome, PutOutcome::Inserted) {
            self.harness_failure(&format!("workload {workload} was not fresh"));
        }
        let kind_key = IntentKey::for_workload_kind(&job.id);
        self.state
            .store
            .put(kind_key.as_bytes(), &[WorkloadKind::Job.discriminator_byte()])
            .await
            .expect("kind write");
    }

    /// Store and allocator effects of `handlers::submit_workload` for a fresh
    /// Service intent: frontend address, VIP, intent, kind, listener facts.
    /// A Service is used where the proof needs a crashed allocation to be
    /// REPLACED. A crashed Job is a natural exit, which `WorkloadLifecycle`
    /// finalizes (`is_natural_exit`) instead of restarting.
    async fn operator_deploys_service(&self, workload: &str) {
        let service = Service::from_submit(ServiceSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 0, memory_bytes: DEMAND_MEMORY_BYTES },
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
        if let Ok(name) = MeshServiceName::new(&format!(
            "{}.{}",
            service.id.as_str(),
            MeshServiceName::SUFFIX
        )) {
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
        let mut facts = self.state.listener_facts.lock().await;
        facts.upsert(service.id.clone(), &vip, &service.listeners);
        drop(facts);
        self.note(format!("operator deploys service {workload}"));
    }

    /// Store effect of `handlers::stop_workload`.
    async fn operator_stops(&self, workload: &str) {
        let stop_key = IntentKey::for_workload_stop(&Self::workload_id(workload));
        self.state.store.put_if_absent(stop_key.as_bytes(), b"").await.expect("stop write");
        self.note(format!("operator stops {workload}"));
    }

    /// Store effect of `handlers::restart_workload` (a resume when stopped).
    async fn operator_restarts(&self, workload: &str) {
        let id = Self::workload_id(workload);
        let gen_key = IntentKey::for_workload_generation(&id);
        let stop_key = IntentKey::for_workload_stop(&id);
        self.state
            .store
            .txn(vec![
                TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key.as_bytes()) },
                TxnOp::Delete { key: Bytes::copy_from_slice(stop_key.as_bytes()) },
            ])
            .await
            .expect("restart txn");
        self.note(format!("operator restarts {workload}"));
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
            self.ledger.as_ref(),
        ))
        .await
        .map_err(|error| format!("{error:?}"))
    }

    async fn rows(&self) -> BTreeMap<AllocationId, AllocStatusRow> {
        self.state
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc status rows")
            .into_iter()
            .map(|row| (row.alloc_id.clone(), row))
            .collect()
    }

    async fn census(&self) -> (Census, usize, BTreeMap<AllocationId, AllocStatusRow>) {
        let rows = self.rows().await;
        let mut census = Census::default();
        for alloc in self.ledger.live() {
            match rows.get(&alloc).map(|row| row.state) {
                Some(AllocState::Running) => census.running += 1,
                Some(state) if state.is_terminal() => census.retiring += 1,
                _ => census.in_flight += 1,
            }
        }
        let node_running_rows = rows.values().filter(|row| row.state == AllocState::Running).count();
        (census, node_running_rows, rows)
    }

    async fn expect_census(&self, expected: Census, when: &str) {
        let (census, _, _) = self.census().await;
        if census != expected {
            self.harness_failure(&format!("{when}: census {census:?} != expected {expected:?}"));
        }
        self.note(format!("{when}: census {census:?}"));
    }

    /// One placement decision for a freshly deployed workload.
    async fn probe_placement(&self, workload: &str) -> Decision {
        let (before, node_running_rows, rows) = self.census().await;
        let scheduler_input_rows = rows
            .values()
            .filter(|row| row.workload_id.as_str() == workload && row.state == AllocState::Running)
            .count();
        self.ledger.begin_window();
        let dispatch = self.converge(workload).await;
        let (admitted_at_acquisition, acquired) = self.ledger.window();
        let decision = Decision {
            before,
            node_running_rows,
            scheduler_input_rows,
            admitted: self.ledger.holds_for(workload),
            acquired,
            admitted_at_acquisition,
            dispatch,
        };
        self.note(format!("placement {workload}: {}", decision.evidence()));
        decision
    }

    /// Converge a stopped workload until its lease is released.
    async fn settle_released(&self, workload: &str) {
        for _ in 0..SETTLE_TICKS {
            if !self.ledger.holds_for(workload) {
                return;
            }
            if let Err(error) = self.converge(workload).await {
                self.note(format!("settle {workload}: {error}"));
            }
        }
        if self.ledger.holds_for(workload) {
            self.harness_failure(&format!("{workload} still holds its lease after stop"));
        }
    }

    /// Crash the workload's Running allocation through the driver exit port.
    async fn crash(&self, workload: &str) -> AllocationId {
        let rows = self.rows().await;
        let Some(alloc) = rows
            .values()
            .find(|row| row.workload_id.as_str() == workload && row.state == AllocState::Running)
            .map(|row| row.alloc_id.clone())
        else {
            self.harness_failure(&format!("{workload} has no Running allocation to crash"));
        };
        self.driver.inject_exit_after(
            &alloc,
            Duration::ZERO,
            ExitKind::Crashed { exit_code: None, signal: Some(9) },
        );
        for _ in 0..CRASH_POLL_BUDGET {
            tokio::task::yield_now().await;
            self.clock.tick(Duration::from_millis(1));
            tokio::time::sleep(Duration::from_millis(1)).await;
            let row = self.state.obs.alloc_status_row(&alloc).await.expect("row read");
            if row.is_some_and(|row| row.state == AllocState::Failed) {
                self.note(format!("crashed {workload} ({alloc}) -> Failed row by exit observer"));
                return alloc;
            }
        }
        self.harness_failure(&format!("exit observer never published Failed for {alloc}"));
    }
}

// ---------------------------------------------------------------------------
// Verdicts.
// ---------------------------------------------------------------------------

struct Verdict {
    id: &'static str,
    green: bool,
    evidence: String,
}

struct Report {
    verdicts: Vec<Verdict>,
    observations: Vec<(&'static str, String)>,
}

impl Report {
    fn verdict(&mut self, node: &SimNode, id: &'static str, green: bool, evidence: String) {
        let label = if green { "GREEN" } else { "RED" };
        node.note(format!("verdict [{label}] {id}: {evidence}"));
        eprintln!("seed={} [{label}] {id}: {evidence}", node.seed);
        self.verdicts.push(Verdict { id, green, evidence });
    }

    fn observe(&mut self, node: &SimNode, id: &'static str, evidence: String) {
        node.note(format!("observation {id}: {evidence}"));
        eprintln!("seed={} [OBSERVATION] {id}: {evidence}", node.seed);
        self.observations.push((id, evidence));
    }
}

#[derive(Debug, Clone, Copy)]
enum Block {
    Placement,
    InFlight,
    Retiring,
    Resume,
}

/// CONTRACT_SHAPE: bounded-change.
///
/// Every admission path on one node keeps the node-wide admitted population
/// (in-flight + Running) at or below the T1 cap, and releasing an attachment
/// frees an admission.
#[tokio::test(flavor = "current_thread")]
async fn node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads() {
    let seed = std::env::var(SEED_ENV)
        .ok()
        .map_or(DEFAULT_SEED, |raw| raw.parse().expect("seed env var must be a u64"));
    eprintln!("seed={seed} invariant=node_wide_attachment_admission cap={CAP}");
    let mut rng = StdRng::seed_from_u64(seed);
    let node = SimNode::compose(seed).await;

    // --- Fill: 16,384 distinct workloads, each admitted by the production
    //     reconciler evaluation, each reaching Running with one lease. ---
    let mut fillers: Vec<String> = (0..CAP).map(|index| format!("f{index:05}")).collect();
    fillers.shuffle(&mut rng);
    let fill_started = Instant::now();
    for (index, workload) in fillers.iter().enumerate() {
        node.operator_deploys(workload).await;
        if let Err(error) = node.converge(workload).await {
            node.harness_failure(&format!("filler {workload} evaluation failed: {error}"));
        }
        if node.ledger.live_count() != index + 1 {
            node.harness_failure(&format!(
                "filler {workload} did not acquire exactly one attachment (live={})",
                node.ledger.live_count()
            ));
        }
        if (index + 1) % 4_096 == 0 {
            eprintln!("seed={seed} fill progress {}/{CAP} after {:?}", index + 1, fill_started.elapsed());
        }
    }
    let baseline = Census { running: CAP, in_flight: 0, retiring: 0 };
    node.expect_census(baseline, "after fill").await;
    node.ledger.start_measuring();
    eprintln!("seed={seed} fill: {CAP} distinct workloads Running in {:?}", fill_started.elapsed());

    // Seeded victims (distinct fillers) and block order.
    let mut victim_indices = BTreeSet::new();
    while victim_indices.len() < 3 {
        victim_indices.insert(rng.gen_range(0..CAP));
    }
    let mut victims: Vec<String> =
        victim_indices.into_iter().map(|index| fillers[index].clone()).collect();
    victims.shuffle(&mut rng);
    let (released_victim, displaced_victim, resumed_victim) =
        (victims[0].clone(), victims[1].clone(), victims[2].clone());
    let mut blocks = [Block::Placement, Block::InFlight, Block::Retiring, Block::Resume];
    blocks.shuffle(&mut rng);
    let plan = format!(
        "victims release={released_victim} displace-for-service={displaced_victim} \
         resume={resumed_victim}; block order {blocks:?}"
    );
    node.note(plan.clone());
    eprintln!("seed={seed} {plan}");

    let mut report = Report { verdicts: Vec::new(), observations: Vec::new() };

    for block in blocks {
        node.expect_census(baseline, &format!("before block {block:?}")).await;
        match block {
            Block::Placement => {
                node.operator_deploys("na1-probe").await;
                let decision = node.probe_placement("na1-probe").await;
                report.verdict(&node, "NA-1", !decision.admitted, decision.evidence());
                node.operator_stops("na1-probe").await;
                node.settle_released("na1-probe").await;
            }
            Block::InFlight => {
                node.operator_stops(&released_victim).await;
                node.settle_released(&released_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, retiring: 0 },
                    "after releasing one attachment",
                )
                .await;
                node.operator_deploys("na2-held").await;
                node.operator_deploys("na2-probe").await;

                // One evaluation parks after its lease was taken and before
                // its Running row exists (the in-flight window of one of the
                // eight concurrent production evaluations).
                let (entered, release) = node.ledger.arm_park();
                let held = node.converge("na2-held");
                let mut held = std::pin::pin!(held);
                let parked_alloc = tokio::select! {
                    biased;
                    outcome = &mut held => {
                        node.harness_failure(&format!(
                            "na2-held evaluation finished without reaching provision: {outcome:?}"
                        ));
                    }
                    parked = entered => parked.expect("park signal"),
                };
                node.note(format!("na2-held parked in flight at provision ({parked_alloc})"));
                let held_admitted = node.ledger.holds_for("na2-held");
                report.verdict(
                    &node,
                    "NA-5 (release, then in-flight admission)",
                    held_admitted,
                    format!(
                        "after {released_victim} released its attachment (live={}), a new \
                         workload's placement acquired an attachment={held_admitted} \
                         ({parked_alloc})",
                        CAP - 1
                    ),
                );
                let decision = node.probe_placement("na2-probe").await;
                report.verdict(&node, "NA-2", !decision.admitted, decision.evidence());
                release.send(()).expect("parked provision is still waiting");
                if let Err(error) = held.await {
                    node.harness_failure(&format!("na2-held completion failed: {error}"));
                }
                node.operator_stops("na2-probe").await;
                node.settle_released("na2-probe").await;
            }
            Block::Retiring => {
                // Make room for one restartable Service, then crash it.
                node.operator_stops(&displaced_victim).await;
                node.settle_released(&displaced_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, retiring: 0 },
                    "after releasing one attachment for the Service",
                )
                .await;
                node.operator_deploys_service("na3-svc").await;
                let placed = node.probe_placement("na3-svc").await;
                report.verdict(
                    &node,
                    "NA-5 (release, then Service placement)",
                    placed.admitted,
                    placed.evidence(),
                );
                if !placed.admitted {
                    node.harness_failure("the Service crash victim was never placed");
                }
                let crashed_alloc = node.crash("na3-svc").await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, retiring: 1 },
                    "after crash (lease still held)",
                )
                .await;
                node.operator_deploys("na3-probe").await;
                let decision = node.probe_placement("na3-probe").await;
                report.observe(
                    &node,
                    "OBS-RETIRING",
                    format!(
                        "placement with one retiring attachment ({crashed_alloc}): {}. ADR-0121 \
                         does not pin whether a retiring attachment counts against the cap; \
                         either answer is consistent with NA-4a below",
                        decision.evidence()
                    ),
                );

                // Replace the crashed Service while the placement above (if
                // it was admitted) still holds its attachment.
                node.ledger.begin_window();
                let replacement = node.converge("na3-svc").await;
                let (admitted_at_acquisition, acquired) = node.ledger.window();
                let (after, _, _) = node.census().await;
                report.verdict(
                    &node,
                    "NA-4a",
                    admitted_at_acquisition.is_none_or(|admitted| admitted <= CAP),
                    format!(
                        "replacement evaluation for crashed Service na3-svc ({crashed_alloc}): \
                         acquired {acquired:?}; admitted population at that acquisition=\
                         {admitted_at_acquisition:?}; after: {after:?} (admitted {}); \
                         dispatch={replacement:?}",
                        after.admitted()
                    ),
                );

                // Free the probe's attachment (if it held one), then require
                // the crashed Service to be replaced. That liveness guard
                // stops NA-4a from passing vacuously when no replacement is
                // ever attempted.
                if node.ledger.holds_for("na3-probe") {
                    node.operator_stops("na3-probe").await;
                    node.settle_released("na3-probe").await;
                }
                for _ in 0..SETTLE_TICKS {
                    let (census, _, _) = node.census().await;
                    if census == baseline {
                        break;
                    }
                    if let Err(error) = node.converge("na3-svc").await {
                        node.note(format!("replacement settle: {error}"));
                    }
                }
                let (settled, _, rows) = node.census().await;
                let running_successor: Vec<&AllocationId> = rows
                    .values()
                    .filter(|row| {
                        row.workload_id.as_str() == "na3-svc"
                            && row.state == AllocState::Running
                            && row.alloc_id != crashed_alloc
                    })
                    .map(|row| &row.alloc_id)
                    .collect();
                report.verdict(
                    &node,
                    "NA-4a-L (liveness: crashed Service is replaced once capacity exists)",
                    !running_successor.is_empty() && !node.ledger.live().contains(&crashed_alloc),
                    format!(
                        "after the probe's attachment was released: Running successor(s) \
                         {running_successor:?}; crashed {crashed_alloc} still holds an \
                         attachment={}; census {settled:?}",
                        node.ledger.live().contains(&crashed_alloc)
                    ),
                );
            }
            Block::Resume => {
                // A restartable Service takes the freed attachment and is
                // then operator-stopped. A Job refills the node, and the
                // operator resumes the Service at the cap. (A stopped Job
                // is not a usable resume victim: the action shim drops a
                // RestartAllocation whose Job predecessor carries a terminal
                // claim, see `allocation_attempt_transition`.)
                node.operator_stops(&resumed_victim).await;
                node.settle_released(&resumed_victim).await;
                node.expect_census(
                    Census { running: CAP - 1, in_flight: 0, retiring: 0 },
                    "after operator stop released one attachment",
                )
                .await;
                node.operator_deploys_service("na4-svc").await;
                let service_placed = node.probe_placement("na4-svc").await;
                if !service_placed.admitted {
                    node.harness_failure("the Service resume victim was never placed");
                }
                node.operator_stops("na4-svc").await;
                node.settle_released("na4-svc").await;
                node.operator_deploys("na4-filler").await;
                let refill = node.probe_placement("na4-filler").await;
                report.verdict(
                    &node,
                    "NA-5 (release, then placement)",
                    refill.admitted,
                    format!("after na4-svc released its attachment: {}", refill.evidence()),
                );
                if !refill.admitted {
                    node.harness_failure("the refill below the cap was never placed");
                }

                node.operator_restarts("na4-svc").await;
                node.ledger.begin_window();
                let resume = node.converge("na4-svc").await;
                let (admitted_at_acquisition, acquired) = node.ledger.window();
                let (after, _, _) = node.census().await;
                report.verdict(
                    &node,
                    "NA-4b",
                    admitted_at_acquisition.is_none_or(|admitted| admitted <= CAP),
                    format!(
                        "operator resume of Service na4-svc with {CAP} Running: acquired \
                         {acquired:?}; admitted population at that acquisition=\
                         {admitted_at_acquisition:?}; after: {after:?} (admitted {}); \
                         dispatch={resume:?}",
                        after.admitted()
                    ),
                );

                // Free the refill's attachment, then require the resume to
                // complete. That liveness guard stops NA-4b from passing
                // vacuously.
                node.operator_stops("na4-filler").await;
                node.settle_released("na4-filler").await;
                for _ in 0..SETTLE_TICKS {
                    if node.ledger.holds_for("na4-svc") {
                        break;
                    }
                    if let Err(error) = node.converge("na4-svc").await {
                        node.note(format!("resume settle: {error}"));
                    }
                }
                let (settled, _, rows) = node.census().await;
                let resumed: Vec<&AllocationId> = rows
                    .values()
                    .filter(|row| {
                        row.workload_id.as_str() == "na4-svc" && row.state == AllocState::Running
                    })
                    .map(|row| &row.alloc_id)
                    .collect();
                report.verdict(
                    &node,
                    "NA-4b-L (liveness: stopped Service resumes once capacity exists)",
                    !resumed.is_empty(),
                    format!(
                        "after the refill's attachment was released: Running na4-svc \
                         allocation(s) {resumed:?}; census {settled:?}"
                    ),
                );
                if resumed.is_empty() {
                    // Keep the block's end-state precondition: put the node
                    // back at 16,384 with a fresh placement.
                    node.operator_deploys("na4-refill").await;
                    if let Err(error) = node.converge("na4-refill").await {
                        node.note(format!("resume fallback refill: {error}"));
                    }
                }
            }
        }
        node.expect_census(baseline, &format!("after block {block:?}")).await;
    }

    let (admitted_peak, admitted_peak_at, lease_peak) = node.ledger.peaks();
    report.verdict(
        &node,
        "NA-G",
        admitted_peak <= CAP,
        format!(
            "peak admitted population at any lease acquisition over the run={admitted_peak} \
             (reached at {admitted_peak_at:?}); the fill is bounded to {CAP} by construction"
        ),
    );
    report.observe(
        &node,
        "OBS-OVERLAP",
        format!(
            "peak lease population including retiring and replacement overlap={lease_peak}; \
             ADR-0121 pool headroom={POOL_CAPACITY}; the accepted design records no bound on \
             overlap beyond the pool"
        ),
    );

    let table: String = report
        .verdicts
        .iter()
        .map(|entry| {
            format!(
                "  [{}] {}: {}\n",
                if entry.green { "GREEN" } else { "RED" },
                entry.id,
                entry.evidence
            )
        })
        .chain(report.observations.iter().map(|(id, evidence)| format!("  [OBS] {id}: {evidence}\n")))
        .collect();
    eprintln!("seed={seed} node-wide admission verdicts:\n{table}");
    let red: Vec<&str> =
        report.verdicts.iter().filter(|entry| !entry.green).map(|entry| entry.id).collect();
    assert!(
        red.is_empty(),
        "seed={seed}: node-wide attachment admission contract violated: {red:?}\n\
         verdicts:\n{table}\
         reproduce: {SEED_ENV}={seed} cargo xtask lima run -- cargo nextest run -p overdrive-sim \
         --features integration-tests --no-capture \
         -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'"
    );
}
