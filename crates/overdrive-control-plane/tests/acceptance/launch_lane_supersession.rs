//! launch_lane_supersession — the convergence lane's supersession signal (ADR-0334, GH #348).
//!
//! Scenarios: the lane table's use through the real convergence tick (boundary scenarios 4 and 4a of
//! the feature delta, Gate G-3): `ReconcilerRuntime::{supersede_lane, supersede_all_lanes,
//! lane_supersession}`, the two operator handlers that raise the signal, and the action shim's
//! handling of a launch that returns `StartSuperseded`.
//!
//! Contract shape: bounded-change. Owner path: the real `stop_workload` / `restart_workload` /
//! `submit_workload` handlers -> `ReconcilerRuntime` lane table -> the real tick
//! (`run_convergence_tick_with_guest_network_provisioner_for_test`: lane open, hydration,
//! `WorkloadLifecycle::reconcile`, the real action shim) -> a `SimDriver` start the test holds with
//! `park_next_start`. Faults enter only at driven ports (the driver's start outcome, the guest-network
//! owner's teardown). The loop's shutdown arm is in `src/node_volume_boundaries.rs` (the loop is
//! crate-private).
//!
//! Oracle: the parked start's `observed_supersession()`, the tick's own result, the allocation rows,
//! the restart authority's `restart_counts`, the identity manager's held SVIDs, the guest-network
//! owner's recorded teardown calls and the driver's recorded `start` / `stop` / `release_supervision`
//! calls. Tier: in-memory acceptance (layer 2), default lane, `host_pure`.
//!
//! No test overlaps two ticks of one target (ADR-0102 admits one lane per target). The seeded schedule
//! test prints its seeds.
//!
//! Status: a test marked PENDING in its own doc is proven RED by `#[should_panic(expected = ...)]`
//! on the assertion that fails today.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    clippy::print_stderr,
    reason = "scenarios assert their own preconditions; seeds are printed for reproduction"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{RestartOutcome, StopOutcome, SubmitWorkloadRequest};
use overdrive_control_plane::guest_network::SharedGuestNetworkOwner;
use overdrive_control_plane::handlers::{restart_workload, stop_workload, submit_workload};
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{DriverInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType,
    LaunchSupersession, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::{SimDriver, StartPark};
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use overdrive_testing::placement;

placement!(host_pure);

/// The `SimClock` step before each tick: past every restart backoff window.
const PAST_BACKOFF: Duration = Duration::from_secs(3_600);
/// Real-time settle bound for an effect that must happen, in 1 ms polls.
const SETTLE_POLLS: u32 = 1_000;

// ---------------------------------------------------------------------------
// The driver double: a `SimDriver` whose start is held by `park_next_start`, wrapped so the shim's
// calls (start, stop, release_supervision) are recorded at the driven port.
// ---------------------------------------------------------------------------

struct RecordingDriver {
    inner: Arc<SimDriver>,
    starts: Mutex<Vec<AllocationId>>,
    stops: Mutex<Vec<AllocationId>>,
    supervision_releases: Mutex<Vec<AllocationId>>,
}

impl RecordingDriver {
    const fn new(inner: Arc<SimDriver>) -> Self {
        Self {
            inner,
            starts: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            supervision_releases: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(
        &self,
        spec: &AllocationSpec,
        supersession: &dyn LaunchSupersession,
    ) -> Result<AllocationHandle, DriverError> {
        self.starts.lock().push(spec.alloc.clone());
        self.inner.start(spec, supersession).await
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle).await;
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stops.lock().push(handle.alloc.clone());
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
        self.supervision_releases.lock().push(alloc.clone());
        self.inner.release_supervision(alloc);
    }
}

// ---------------------------------------------------------------------------
// The rig: the production tick over sim driven ports.
// ---------------------------------------------------------------------------

struct Rig {
    state: AppState,
    clock: Arc<SimClock>,
    sim: Arc<SimDriver>,
    driver: Arc<RecordingDriver>,
    owner: Arc<SimSharedGuestNetworkOwner>,
    tick_n: u64,
    _wiring: GuestNetworkExecWiring,
    _tmp: TempDir,
}

/// One spawned convergence tick of one workload.
struct Tick {
    handle: JoinHandle<Result<(), ConvergenceError>>,
}

impl Tick {
    fn finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Wait (real time, bounded) for the tick to end without advancing any simulated clock.
    async fn ends(&self) -> bool {
        for _ in 0..SETTLE_POLLS {
            if self.finished() {
                return true;
            }
            pump(1).await;
        }
        self.finished()
    }

    async fn join(self) -> Result<(), ConvergenceError> {
        tokio::time::timeout(Duration::from_secs(10), self.handle)
            .await
            .expect("the tick ends within the bound")
            .expect("the tick task did not panic")
    }
}

async fn pump(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn settled(cond: impl Fn() -> bool) -> bool {
    for _ in 0..SETTLE_POLLS {
        if cond() {
            return true;
        }
        pump(1).await;
    }
    cond()
}

fn workload(id: &str) -> WorkloadId {
    WorkloadId::new(id).expect("a valid workload id")
}

fn target(id: &str) -> TargetResource {
    TargetResource::new(&format!("workload/{id}")).expect("a valid target")
}

fn reconciler() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("a valid reconciler name")
}

fn service_spec(id: &str, memory_size: u64) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(memory_size),
        },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            volume: None,
            command: "/sbin/init".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

impl Rig {
    async fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let clock = Arc::new(SimClock::new());
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime
            .register(overdrive_control_plane::workload_lifecycle())
            .await
            .expect("register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let node = NodeId::new("lane-node").expect("node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let sim = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let driver = Arc::new(RecordingDriver::new(Arc::clone(&sim)));
        let owner = Arc::new(SimSharedGuestNetworkOwner::default());
        owner.converge_shared().await.expect("the shared owner converges");
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(wiring.supervisor().open_after_boot(), "the EXEC gate leaves BootClosed once");
        let worker = crate::acceptance::serve_ports::worker();
        worker.start_shared_owner().await.expect("the shared mTLS owner publishes");
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&store) as Arc<dyn IntentStore>
        );
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
            node,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            wiring.gate(),
            crate::acceptance::serve_ports::pool(),
        );
        Self { state, clock, sim, driver, owner, tick_n: 0, _wiring: wiring, _tmp: tmp }
    }

    fn runtime(&self) -> &ReconcilerRuntime {
        &self.state.runtime
    }

    /// The operator declares a Service through the real handler.
    async fn declare(&self, id: &str) {
        self.submit(&service_spec(id, 134_217_728)).await.expect("submit_workload accepts it");
    }

    async fn submit(
        &self,
        spec: &ServiceSpecInput,
    ) -> Result<(), overdrive_control_plane::error::ControlPlaneError> {
        let request = SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) };
        submit_workload(State(self.state.clone()), HeaderMap::new(), Json(request))
            .await
            .map(|_| ())
    }

    async fn stop(&self, id: &str) -> StopOutcome {
        let Json(body) = stop_workload(State(self.state.clone()), Path(id.to_owned()))
            .await
            .expect("stop_workload records the stop");
        body.outcome
    }

    async fn restart(&self, id: &str) -> RestartOutcome {
        let Json(body) = restart_workload(State(self.state.clone()), Path(id.to_owned()))
            .await
            .expect("restart_workload records the restart");
        body.outcome
    }

    /// The lane signal the runtime hands the launch of `id` (a fresh, never-raised signal when no lane
    /// is open).
    fn lane(&self, id: &str) -> Arc<dyn LaunchSupersession> {
        self.runtime().lane_supersession(&target(id))
    }

    /// One production convergence tick of workload `id`, spawned. The simulated clock moves past
    /// every backoff window first, so a tick never waits on a restart window.
    fn spawn_tick(&mut self, id: &str) -> Tick {
        self.clock.tick(PAST_BACKOFF);
        self.tick_n += 1;
        let tick_n = self.tick_n;
        let state = self.state.clone();
        let owner = Arc::clone(&self.owner);
        let clock = Arc::clone(&self.clock);
        let target = target(id);
        let handle = tokio::spawn(async move {
            let now = clock.now();
            run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &reconciler(),
                &target,
                now,
                tick_n,
                now + Duration::from_secs(1),
                &*owner,
            )
            .await
        });
        Tick { handle }
    }

    /// Hold the next launch of the workload inside `Driver::start`, spawn its tick and wait until the
    /// start is entered: the lane is open and the dispatch is held.
    async fn launch_held(&mut self, id: &str, observe_lane_signal: bool) -> (StartPark, Tick) {
        let park = self.sim.park_next_start(observe_lane_signal);
        let tick = self.spawn_tick(id);
        let tick = self.until_entered(id, &park, tick).await;
        (park, tick)
    }

    /// Follow the workload's evaluations from `tick` until one reaches the held start. A restart of a
    /// running workload may first spend an evaluation ending the predecessor; that evaluation is run
    /// to its end and the next one is the launch.
    async fn until_entered(&mut self, id: &str, park: &StartPark, mut tick: Tick) -> Tick {
        for _ in 0..4 {
            for _ in 0..SETTLE_POLLS {
                if park.has_entered() || tick.finished() {
                    break;
                }
                pump(1).await;
            }
            if park.has_entered() {
                return tick;
            }
            assert!(tick.finished(), "setup: the evaluation neither ends nor reaches the start");
            tick.join().await.expect("setup: the evaluation before the launch converges");
            tick = self.spawn_tick(id);
        }
        panic!("setup: no evaluation of {id} reached the driver's start");
    }

    /// Spawn the tick of `id` and keep it parked BEFORE hydration: the lane is open (the tick's first
    /// step) and the tick waits for the allocator lock the test holds. Returns the held guard.
    async fn lane_held_before_hydration(
        &mut self,
        id: &str,
    ) -> (
        tokio::sync::OwnedMutexGuard<
            overdrive_dataplane::allocators::PersistentServiceVipAllocator,
        >,
        Tick,
    ) {
        let guard = Arc::clone(&self.state.allocator).lock_owned().await;
        let tick = self.spawn_tick(id);
        pump(100).await;
        assert!(!tick.finished(), "setup: the tick waits for the allocator lock before hydration");
        (guard, tick)
    }

    async fn rows(&self) -> Vec<AllocStatusRow> {
        self.state.obs.alloc_status_rows().await.expect("the observation store reads its rows")
    }

    async fn rows_of(&self, id: &str) -> Vec<AllocStatusRow> {
        let id = workload(id);
        self.rows().await.into_iter().filter(|row| row.workload_id == id).collect()
    }

    fn restart_counts(&self, id: &str) -> BTreeMap<AllocationId, u32> {
        self.runtime().view_for_workload_lifecycle(&target(id)).restart_counts
    }

    fn teardown_calls(&self) -> usize {
        self.owner
            .calls()
            .into_iter()
            .filter(|op| {
                matches!(
                    op,
                    overdrive_control_plane::guest_network::GuestNetworkOperation::PairAbort
                )
            })
            .count()
    }

    fn started(&self) -> Vec<AllocationId> {
        self.driver.starts.lock().clone()
    }

    fn stopped(&self) -> Vec<AllocationId> {
        self.driver.stops.lock().clone()
    }

    fn supervision_releases(&self) -> Vec<AllocationId> {
        self.driver.supervision_releases.lock().clone()
    }

    /// Run the next evaluation of `id` to its end with an unparked driver (the launch proceeds).
    async fn next_evaluation(&mut self, id: &str) -> Result<(), ConvergenceError> {
        self.spawn_tick(id).join().await
    }
}

fn running(rows: &[AllocStatusRow]) -> Vec<&AllocStatusRow> {
    rows.iter().filter(|row| row.state == AllocState::Running).collect()
}

// ---------------------------------------------------------------------------
// A. The lane table through the real tick
// ---------------------------------------------------------------------------

/// GUARD (boundary 4a (a), edge): `supersede_lane` for a target with no open lane, and for a target
/// that never had one, does nothing: the signal of the lane the next evaluation opens is unraised.
#[tokio::test]
async fn supersede_lane_with_no_open_lane_does_nothing() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;

    rig.runtime().supersede_lane(&target("lane-x"));
    rig.runtime().supersede_lane(&target("lane-never-declared"));
    assert!(!rig.lane("lane-x").is_superseded(), "no lane is open: nothing to raise");

    let (park, tick) = rig.launch_held("lane-x", true).await;
    pump(50).await;
    assert!(
        !park.observed_supersession(),
        "a signal with no lane open must not leak into the lane the next evaluation opens"
    );
    assert!(!tick.finished(), "the launch keeps waiting: nothing superseded it");
    park.release();
    tick.join().await.expect("the launch completes");
    assert_eq!(running(&rig.rows_of("lane-x").await).len(), 1, "the launch ended Running");
}

/// Assumption A8, timing 2 (after the lane opens and before hydration; edge): the lane is
/// already open when the signal lands, and the signal is visible on it.
#[tokio::test]
async fn a_signal_after_the_lane_opens_and_before_hydration_is_visible_on_the_lane() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (guard, tick) = rig.lane_held_before_hydration("lane-x").await;
    let lane = rig.lane("lane-x");

    rig.runtime().supersede_lane(&target("lane-x"));

    assert!(
        lane.is_superseded(),
        "a signal raised after the lane opens and before hydration must be visible on that lane"
    );
    drop(guard);
    let _ = tick.join().await;
}

/// Boundary 4a (e), edge: the signal for workload X never reaches workload Y's launch.
#[tokio::test]
async fn supersede_lane_for_one_workload_never_raises_another_workloads_lane() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    rig.declare("lane-y").await;
    let (px, tx) = rig.launch_held("lane-x", true).await;
    let (py, ty) = rig.launch_held("lane-y", true).await;

    rig.runtime().supersede_lane(&target("lane-x"));

    assert!(
        settled(|| px.observed_supersession()).await,
        "the signal for workload x must reach workload x's own launch"
    );
    pump(50).await;
    assert!(
        !py.observed_supersession() && !rig.lane("lane-y").is_superseded() && !ty.finished(),
        "a signal for workload x must never raise workload y's lane"
    );
    py.release();
    ty.join().await.expect("workload y's launch completes untouched");
    tx.join().await.expect("workload x's superseded dispatch returns Ok(())");
    assert_eq!(running(&rig.rows_of("lane-y").await).len(), 1);
    assert!(rig.rows_of("lane-x").await.is_empty());
}

/// Boundary 4a (h), edge: `supersede_lane` never cancels the shutdown token: a lane opened
/// for another target after it still opens unraised.
#[tokio::test]
async fn supersede_lane_never_cancels_the_shutdown_token() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    rig.declare("lane-y").await;
    let (px, tx) = rig.launch_held("lane-x", true).await;

    rig.runtime().supersede_lane(&target("lane-x"));
    assert!(
        settled(|| px.observed_supersession()).await,
        "supersede_lane must raise the open lane of workload x before the shutdown-token check means anything"
    );
    tx.join().await.expect("workload x's superseded dispatch returns Ok(())");

    let (py, ty) = rig.launch_held("lane-y", true).await;
    pump(50).await;
    assert!(
        !py.observed_supersession() && !rig.lane("lane-y").is_superseded(),
        "a lane opened after supersede_lane for another target must open unraised: supersede_lane \
         must never cancel the shutdown token"
    );
    py.release();
    ty.join().await.expect("workload y's launch completes");
}

/// Seeded schedule (boundary 4a (h), edge): wherever `supersede_all_lanes` falls in a
/// sequence of lane opens, every launch abandons. The position comes from the printed seed.
#[tokio::test]
async fn supersede_all_lanes_at_any_point_of_a_seeded_schedule_abandons_every_launch() {
    const LANES: u64 = 3;
    for seed in 0_u64..4 {
        let position =
            seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407)
                >> 33;
        let position = usize::try_from(position % (LANES + 1)).expect("fits");
        eprintln!("supersede_all_lanes schedule: seed {seed}, called after {position} opens");

        let mut rig = Rig::new().await;
        let mut held = Vec::new();
        for n in 0..LANES {
            let id = format!("lane-{n}");
            rig.declare(&id).await;
            if usize::try_from(n).expect("fits") == position {
                rig.runtime().supersede_all_lanes();
            }
            held.push(rig.launch_held(&id, true).await);
        }
        if position == usize::try_from(LANES).expect("fits") {
            rig.runtime().supersede_all_lanes();
        }
        for (n, (park, _)) in held.iter().enumerate() {
            assert!(
                settled(|| park.observed_supersession()).await,
                "every lane must be raised wherever supersede_all_lanes falls: seed {seed}, call \
                 after {position} opens, lane {n}"
            );
        }
        for (_, tick) in held {
            tick.join().await.expect("a superseded dispatch returns Ok(())");
        }
        assert!(rig.rows().await.is_empty(), "seed {seed}: no row");
    }
}

/// Boundary 4a (the lane guard removes its entry by key): a closing lane removes only its
/// own entry and leaves no raised signal behind. A raised lane that closes leaves the target's signal
/// unraised, the next lane for the same target opens unraised, and another workload's lane stays
/// registered and raisable.
#[tokio::test]
async fn a_closing_lane_removes_only_its_own_entry_and_leaves_no_raised_signal_behind() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    rig.declare("lane-y").await;
    // x's driver ignores the signal, so the test ends x's lane by releasing its start.
    let (px, tx) = rig.launch_held("lane-x", false).await;
    let (py, ty) = rig.launch_held("lane-y", true).await;
    let during = rig.lane("lane-x");
    rig.runtime().supersede_lane(&target("lane-x"));
    assert!(
        during.is_superseded(),
        "the open lane must be raised before its entry removal can be observed"
    );

    px.release();
    tx.join().await.expect("x's launch completes and its lane closes");
    assert!(
        !rig.lane("lane-x").is_superseded(),
        "the lane guard removes its entry on return: a closed lane has no signal"
    );
    rig.runtime().supersede_lane(&target("lane-y"));
    assert!(
        settled(|| py.observed_supersession()).await,
        "closing workload x's lane must not remove workload y's entry"
    );
    ty.join().await.expect("y: a superseded dispatch returns Ok(())");

    let (guard, tick) = rig.lane_held_before_hydration("lane-x").await;
    assert!(
        !rig.lane("lane-x").is_superseded(),
        "a raised lane never leaks into the next lane for the same target"
    );
    drop(guard);
    let _ = tick.join().await;
}

// ---------------------------------------------------------------------------
// C. The handlers
// ---------------------------------------------------------------------------

/// Boundary 4a (the `KeyExists` branch; edge): a repeated stop that finds the sentinel
/// already present still raises the open lane.
#[tokio::test]
async fn a_repeated_stop_that_finds_the_sentinel_raises_the_open_lane_too() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    assert_eq!(rig.stop("lane-x").await, StopOutcome::Stopped);
    let (guard, tick) = rig.lane_held_before_hydration("lane-x").await;
    let lane = rig.lane("lane-x");
    assert!(!lane.is_superseded(), "the first stop found no open lane: nothing was raised");

    assert_eq!(
        rig.stop("lane-x").await,
        StopOutcome::AlreadyStopped,
        "the second stop is KeyExists"
    );

    assert!(
        lane.is_superseded(),
        "a repeated stop that finds the sentinel (KeyExists) must raise the open lane's signal too"
    );
    drop(guard);
    let _ = tick.join().await;
}

/// GUARD (boundary 4a (f), edge): `submit_workload` never raises the signal: not for a new workload,
/// not for a byte-identical resubmit (`Unchanged`), not for a different spec at the same id (`409`).
/// A workload's spec is immutable per id, so a submit never changes what a launch in flight was
/// started for.
#[tokio::test]
async fn submit_workload_new_unchanged_and_conflicting_never_raises_the_lane_signal() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (park, tick) = rig.launch_held("lane-x", true).await;
    let lane = rig.lane("lane-x");

    rig.declare("lane-z").await;
    rig.submit(&service_spec("lane-x", 134_217_728))
        .await
        .expect("a byte-identical resubmit is Unchanged");
    let conflict = rig.submit(&service_spec("lane-x", 268_435_456)).await;
    assert!(conflict.is_err(), "a different spec at the same id is a 409 Conflict");
    pump(50).await;

    assert!(
        !lane.is_superseded()
            && !rig.lane("lane-x").is_superseded()
            && !park.observed_supersession(),
        "submit_workload (new, Unchanged, 409) must never raise the open lane"
    );
    assert!(!tick.finished(), "the launch in flight is undisturbed");
    park.release();
    tick.join().await.expect("the launch completes");
    assert_eq!(running(&rig.rows_of("lane-x").await).len(), 1);
}

/// GUARD (boundary 4a (f), edge): evaluations submitted by anything other than an operator commit
/// (the resync pass, observation-row wakes and `EnqueueEvaluation` all submit through
/// `submit_evaluation`) never raise the open lane: they must not abandon a healthy launch.
#[tokio::test]
async fn evaluations_submitted_for_a_busy_target_never_raise_its_lane() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (park, tick) = rig.launch_held("lane-x", true).await;
    let lane = rig.lane("lane-x");

    for _ in 0..3 {
        rig.runtime().submit_evaluation(
            Evaluation { reconciler: reconciler(), target: target("lane-x") },
            rig.clock.now(),
            EvaluationEligibility::Immediate,
        );
    }
    pump(50).await;

    assert!(
        !lane.is_superseded() && !park.observed_supersession(),
        "a wake for a busy target (resync, observation row, EnqueueEvaluation) must never raise its lane"
    );
    assert!(!tick.finished(), "the launch in flight is undisturbed");
    park.release();
    tick.join().await.expect("the launch completes");
}

// ---------------------------------------------------------------------------
// D. Boundary 4 and 4a through the tick
// ---------------------------------------------------------------------------

/// Boundary 4, normal: a restart while the first launch waits ends it at once with
/// `StartSuperseded`: no allocation row, no restart-budget change, the shim released the supervision
/// claim, the guest network lease and the SVID, and the dispatch returned `Ok(())`. No simulated clock
/// advances.
#[tokio::test]
async fn a_restart_while_a_launch_waits_ends_it_at_once_and_charges_nothing() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (park, tick) = rig.launch_held("lane-x", true).await;
    let counts_before = rig.restart_counts("lane-x");
    assert_eq!(
        rig.state.identity.held_snapshot().len(),
        1,
        "setup: the held launch holds its allocation's SVID"
    );
    assert_eq!(rig.teardown_calls(), 0, "setup: nothing is torn down while the launch waits");
    let launched = rig.started();
    assert_eq!(launched.len(), 1, "setup: one start is held");

    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Restarted);

    assert!(
        tick.ends().await,
        "a restart recorded while a launch waits must end the launch at once with StartSuperseded"
    );
    tick.join().await.expect("the dispatch of a superseded launch returns Ok(())");
    assert!(park.observed_supersession(), "the driver returned StartSuperseded");
    assert!(rig.rows_of("lane-x").await.is_empty(), "a superseded launch writes no allocation row");
    assert_eq!(
        rig.restart_counts("lane-x"),
        counts_before,
        "a superseded launch changes no budget input"
    );
    assert!(
        rig.restart_counts("lane-x").values().all(|count| *count == 0),
        "nothing is charged for a superseded launch"
    );
    assert!(
        rig.state.identity.held_snapshot().is_empty(),
        "the superseded launch's SVID is released"
    );
    assert_eq!(rig.teardown_calls(), 1, "the superseded launch's guest network lease is released");
    assert_eq!(
        rig.supervision_releases(),
        launched,
        "the superseded launch's supervision claim is released"
    );
    assert_eq!(rig.sim.live_count(), 0, "the driver supervises nothing for the superseded launch");
}

/// Boundary 4, normal: a stop while a launch waits likewise ends it at once, and the
/// workload stays down: the stop's evaluation starts nothing, and only the next restart places a new
/// launch.
#[tokio::test]
async fn a_stop_while_a_launch_waits_ends_it_and_the_workload_stays_down_until_the_next_restart() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (park, tick) = rig.launch_held("lane-x", true).await;
    let counts_before = rig.restart_counts("lane-x");

    assert_eq!(rig.stop("lane-x").await, StopOutcome::Stopped);

    assert!(
        tick.ends().await,
        "a stop recorded while a launch waits must end the launch at once with StartSuperseded"
    );
    tick.join().await.expect("the dispatch of a superseded launch returns Ok(())");
    assert!(park.observed_supersession(), "the driver returned StartSuperseded");
    assert!(rig.rows_of("lane-x").await.is_empty(), "no row is written");
    assert_eq!(rig.restart_counts("lane-x"), counts_before, "no budget input changes");
    assert!(rig.state.identity.held_snapshot().is_empty(), "the SVID is released");
    assert_eq!(rig.teardown_calls(), 1, "the guest network lease is released");

    // The stop's own evaluation: the workload stays down.
    rig.next_evaluation("lane-x").await.expect("the stop's evaluation converges");
    assert_eq!(rig.started().len(), 1, "after a stop nothing starts: the workload stays down");
    assert!(rig.rows_of("lane-x").await.is_empty(), "after a stop no allocation row appears");

    // The next restart places a new launch.
    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Resumed);
    rig.next_evaluation("lane-x").await.expect("the restart's evaluation launches");
    assert_eq!(running(&rig.rows_of("lane-x").await).len(), 1, "the restart brings it up");
    assert_eq!(rig.started().len(), 2);
}

/// Boundary 4, restart arm; normal: a second restart while the replacement launch of a
/// running workload waits ends that launch: no successor row, the successor's lease, SVID and
/// supervision claim released, and the exact-old predecessor cleanup ran. Nothing is charged.
#[tokio::test]
async fn a_second_restart_while_the_replacement_launch_waits_runs_the_exact_old_cleanup() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    rig.next_evaluation("lane-x").await.expect("the first launch converges");
    let predecessor = rig.rows_of("lane-x").await;
    assert_eq!(predecessor.len(), 1, "setup: one Running predecessor");
    let predecessor = predecessor[0].alloc_id.clone();
    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Restarted);

    let (park, tick) = rig.launch_held("lane-x", true).await;
    let counts_before = rig.restart_counts("lane-x");
    assert_eq!(rig.started().len(), 2, "setup: the replacement's start is held");
    // The predecessor is already stopped once: the restart of a Running workload first ends it
    // (`StopAllocation`), then the restart arm places the replacement.
    let stops_before = rig.stopped();
    assert_eq!(stops_before, vec![predecessor.clone()], "setup: only the predecessor was stopped");
    let successor = rig.started()[1].clone();
    assert_ne!(successor, predecessor, "setup: the replacement is a new allocation");

    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Restarted);

    assert!(
        tick.ends().await,
        "a restart recorded while the replacement launch waits must end it with StartSuperseded"
    );
    tick.join().await.expect("the dispatch of a superseded replacement returns Ok(())");
    assert!(park.observed_supersession(), "the driver returned StartSuperseded");
    let rows = rig.rows_of("lane-x").await;
    assert!(
        rows.iter().all(|row| row.alloc_id != successor),
        "a superseded replacement writes no successor row"
    );
    assert_eq!(rig.restart_counts("lane-x"), counts_before, "the supersession charges nothing");
    assert!(
        !rig.state.identity.held_snapshot().contains_key(&successor),
        "the successor's SVID is released"
    );
    assert!(
        rig.supervision_releases().contains(&successor),
        "the successor's supervision claim is released"
    );
    // ADR-0106: the restart arm's exact-old cleanup runs once per dispatch whatever the successor's
    // outcome, and stopping the already-stopped predecessor is idempotent (`NotFound` is tolerated).
    // So exactly one more stop lands, on the predecessor, and the abandoned successor is never stopped.
    assert_eq!(
        rig.stopped(),
        [stops_before, vec![predecessor]].concat(),
        "the exact-old predecessor cleanup ran once, on the predecessor and nothing else"
    );
}

/// GUARD (boundary 4a (a), edge): a signal with no lane open does nothing, and the evaluation's
/// hydration sees the commit: a stop recorded before the evaluation starts nothing, and a restart
/// recorded before it launches under an unraised lane (no stale signal leaks into the lane opened
/// next).
#[tokio::test]
async fn a_signal_with_no_lane_open_does_nothing_and_hydration_sees_the_commit() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;

    assert_eq!(rig.stop("lane-x").await, StopOutcome::Stopped);
    rig.next_evaluation("lane-x").await.expect("the evaluation hydrates the stop");
    assert!(rig.started().is_empty(), "hydration saw the stop commit: nothing starts");
    assert!(rig.rows_of("lane-x").await.is_empty());

    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Resumed);
    let (park, tick) = rig.launch_held("lane-x", true).await;
    pump(50).await;
    assert!(
        !park.observed_supersession() && !rig.lane("lane-x").is_superseded(),
        "a signal raised while no lane was open must not leak into the lane the evaluation opens"
    );
    park.release();
    tick.join().await.expect("the launch completes");
    assert_eq!(running(&rig.rows_of("lane-x").await).len(), 1, "hydration saw the restart commit");
}

/// GUARD (boundary 4a (c), fault): a driver whose teardown failed while abandoning returns the
/// existing unclassified rejection (the driver's decision, not the signal's): the shim still writes a
/// `Failed` row carrying the driver's text. The shim classifies from the driver's returned error, never
/// from the lane signal.
#[tokio::test]
async fn a_teardown_failure_while_abandoning_reaches_the_shim_as_an_unclassified_rejection() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    // The driver attempted to abandon, its teardown failed, and it returned StartRejected.
    let (park, tick) = rig.launch_held("lane-x", false).await;

    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Restarted);
    park.release_rejected("teardown failed while abandoning the launch");
    assert!(tick.ends().await, "the held start ends");
    tick.join().await.expect("a recorded rejection is dispatched to a Failed row");

    let rows = rig.rows_of("lane-x").await;
    assert_eq!(rows.len(), 1, "exactly one row: the rejection");
    assert_eq!(
        rows[0].state,
        AllocState::Failed,
        "a rejection after the signal is still a Failed row"
    );
    assert_eq!(
        rows[0].reason,
        Some(TransitionReason::DriverInternalError {
            detail: "teardown failed while abandoning the launch".to_owned()
        }),
        "the rejection is the existing unclassified one, carrying the driver's text"
    );
}

/// GUARD (boundary 4a (d), edge): a driver that ignores the signal is unchanged: the launch
/// completes as an ordinary success and its row is written.
#[tokio::test]
async fn a_driver_that_ignores_the_signal_is_unchanged() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    let (park, tick) = rig.launch_held("lane-x", false).await;

    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Restarted);
    pump(50).await;
    assert!(
        !park.observed_supersession() && !tick.finished(),
        "a driver with no interruptible wait keeps waiting through the signal"
    );
    park.release();
    tick.join().await.expect("the launch completes");

    assert!(!park.observed_supersession());
    assert_eq!(
        running(&rig.rows_of("lane-x").await).len(),
        1,
        "the ordinary success row is written"
    );
}

/// PENDING (ADR-0335; boundary 4a (g), edge): a restart whose commit the evaluation already hydrated (the signal
/// lands after the lane opens and before hydration) may see its own `Start` superseded once; the next
/// evaluation relaunches, and the generation ends with one Running allocation, uncharged.
///
/// The workload is stopped when the restart lands (the operator resumes it): a Running predecessor
/// would spend the signalled evaluation on `StopAllocation`, and the `Start` would come in the next
/// evaluation under a fresh, unraised lane, so the scenario would never reach its own superseded start.
/// Pending until the generation rule lands: the compat rule counts the superseded launch's generation
/// as handled, so the next evaluation relaunches nothing.
#[tokio::test]
#[should_panic(expected = "the generation ends with exactly one Running allocation")]
async fn a_restart_the_evaluation_already_hydrated_sees_its_own_start_superseded_once() {
    let mut rig = Rig::new().await;
    rig.declare("lane-x").await;
    rig.next_evaluation("lane-x").await.expect("the first launch converges");
    assert_eq!(rig.stop("lane-x").await, StopOutcome::Stopped);
    rig.next_evaluation("lane-x").await.expect("the stop's evaluation ends the allocation");
    assert!(running(&rig.rows_of("lane-x").await).is_empty(), "setup: the workload is stopped");

    // The resumed launch's start is held; the evaluation's lane is open but it has not hydrated.
    let park = rig.sim.park_next_start(true);
    let (guard, tick) = rig.lane_held_before_hydration("lane-x").await;
    assert_eq!(rig.restart("lane-x").await, RestartOutcome::Resumed);
    drop(guard);
    assert!(
        tick.ends().await,
        "a restart whose commit the evaluation already hydrated must see its own start superseded once"
    );
    tick.join().await.expect("the dispatch returns Ok(())");
    assert!(park.observed_supersession(), "the driver returned StartSuperseded");

    // The next evaluation, from any wake, relaunches.
    rig.next_evaluation("lane-x").await.expect("the next evaluation relaunches");
    let rows = rig.rows_of("lane-x").await;
    assert_eq!(running(&rows).len(), 1, "the generation ends with exactly one Running allocation");
    assert!(
        rig.restart_counts("lane-x").values().all(|count| *count == 0),
        "the superseded launch and its relaunch charge nothing"
    );
}
