//! Quint conformance driver: `hook_ownership` (D1, D3; D2 and D4 by runs; ADR-0255,
//! ADR-0256, ADR-0266). Scenario S-9 of `feature-delta.md`.
//!
//! Replays `specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt`
//! (referenced in place) on its recording main `hooks_conf`: every named run of that
//! instance through `#[quint_test]`, and randomized traces through `#[quint_run]`. Each
//! step's action and picks are read from `hooks_conf::hook_ownership::conf_step`.
//!
//! **Placement.** The protocol's owners are this crate's exit observer
//! (`exit_observer::spawn_with_runtime`, D1) and its action shim (`action_shim::dispatch*`,
//! D3, D4), so the driver lives beside the observer, in the owning crate's `src/`
//! (testing.md "Placement and lane"), and reaches the observer's real constructor and the
//! shim's real dispatch entry. Its sibling is `guest_network::quint_cid_lease`.
//!
//! **The production owners under test.** The exit observer runs for real, consuming
//! `ExitEvent`s from the receiver of the driver it was spawned for. The shim dispatches
//! `StartAllocation`, `RestartAllocation`, `StopAllocation` and `FinalizeFailed`. The probe
//! supervisors are a real `ProbeRunner` over `SimClock`, `SimTcpProber` and
//! `SimObservationStore`, reached through a `Driver` whose probe hooks forward to it as
//! `VmDriver`'s do. Process start, stop and the Running-confirmed gate are the `SimDriver`'s.
//! That `Driver` ([`PacedDriver`]) is the only double: it records hook order, it is the
//! watcher (it knows a guest died and queues an `ExitEvent` only after the gate), and it
//! parks a shim dispatch at the step boundaries the model has inside one dispatch (see
//! "Pacing"). It authors no row, no decision and no supervisor.
//!
//! **Pacing.** The model splits one shim dispatch into separate steps. Each dispatch runs
//! on its own OS thread with its own current-thread runtime, and parks (a condition
//! variable, thread-local script) at: the end of `Driver::start` (`Start`), before
//! `release_for_exit_emission` (`Register` is what the dispatch did up to there; `Release`
//! is the call), the end of `Driver::stop` (`StopEffects`) and before `on_alloc_terminal`
//! (`StopRow` runs the dispatch up to there; `StopHook` is the call). The exit observer and
//! the probe tasks run on the driver's own current-thread runtime, which only runs while a
//! step awaits it. `Emit` holds the watcher's event in test-local pending state;
//! `Consume(a, wrote)` forwards only that event to the real observer receiver. This
//! preserves one-observer-event-per-Consume pacing when several ids are queued.
//!
//! Action -> production call or environment fault:
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh composition; the production observer spawned on the observer runtime |
//! | `Start(1)` | shim `dispatch(StartAllocation)`, parked after `Driver::start` |
//! | `Start(2)` | shim `dispatch(RestartAllocation { alloc_id: 1, spec: 2 })` when id 1's row is a crash `Failed` row with no terminal; `dispatch(StartAllocation)` after a stop or give-up row (the restart arm does nothing for those); parked after `Driver::start` |
//! | `Register(a)` | the parked dispatch continues through the `Running` row write and `on_alloc_running`; it must have called `on_alloc_running` by the time it parks before the gate release (the pre-D3 order reaches the gate first and fails the step) |
//! | `Release(a)` | the parked dispatch performs `release_for_exit_emission` and completes |
//! | `Die(a)` | the guest dies: the watcher double records it (environment fault, no production call) |
//! | `Emit(a)` | the watcher queues its `ExitEvent` in test-local pending state (enabled only after the gate release) |
//! | `Consume(a, wrote)` | the pending event is sent to the production observer receiver; the runtime runs through this event's release call; `wrote = false` first scripts a non-retryable write failure on `SimObservationStore` |
//! | `StopEffects(a)` | shim `dispatch(StopAllocation)`, parked after `Driver::stop` |
//! | `StopRow(a, wrote)` | the dispatch continues to the row write (failing it when `wrote = false`: the dispatch ends with its error) and parks before `on_alloc_terminal`; with no dispatch parked it is the retry, a fresh `StopAllocation` |
//! | `StopHook(a)` | the parked dispatch calls `on_alloc_terminal` and completes (when `FinalizeFailed` already ended the id, the retry stop has no hook to call: that earlier hook is the call) |
//! | `Finalize(a)` | shim `dispatch(FinalizeFailed)` to completion |
//! | `Idle` | nothing |
//!
//! Projection (feature-delta "Pinned conformance projection"). `al(a).sup` <-
//! `ProbeRunner::is_role_live(a, role)` for the three roles, cross-checked against
//! `ProbeRunner::active_alloc_count()`; `al(a).row` <- `ObservationStore::alloc_status_row(a)`
//! is a terminal (`Failed` or `Terminated`) row. `started`, `regd`, `rel`, `dead`, `ev`,
//! `vmstop`, `srw`, `stopped`, `fin`, `late` and `conf_step` are model-only ghosts and are not
//! compared; the gate and the queued event are observed only through the recorded call order
//! (S-4, `tests/acceptance/terminal_hook_ownership.rs`).
//!
//! Lane: `integration-tests` (quint-connect spawns the `quint` CLI), `cargo xtask metal run --`.

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::significant_drop_tightening,
    reason = "a conformance driver asserts its own preconditions; handler signatures follow the action map"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use itf::Value;
use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver as DriverPort, DriverError,
    DriverPayload, DriverRegistry, DriverType, ExitEvent, ExitKind, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use parking_lot::{Condvar, Mutex};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::exit_observer;
use crate::AppState;
use crate::action_shim::dispatch_with_guest_network_provisioner_for_test;
use crate::guest_network::SharedGuestNetworkOwner;
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::identity_mgr::IdentityMgr;
use crate::shared_network_test_ports::TestSharedOwner;
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
};

const MAIN: &str = "hooks_conf";
const MODULE: &str = "hook_ownership";
/// How long a step may take to settle (real time; the owners read time only through `SimClock`).
const SETTLE_BOUND: Duration = Duration::from_secs(10);

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError2 {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("the step did not settle within the settle bound: {0}")]
    Unsettled(String),
    #[error("a production call failed: {0}")]
    Production(String),
    #[error("ITF: {0}")]
    Itf(#[from] ItfError),
}

fn not_enabled(action: &'static str, why: impl Into<String>) -> DriverError2 {
    DriverError2::NotEnabled { action, why: why.into() }
}

fn alloc_of(a: i64) -> AllocationId {
    AllocationId::new(&format!("alloc-hooks-{a}")).expect("valid alloc id")
}

// ----- pacing ---------------------------------------------------------------------------

/// A step boundary inside one shim dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    /// The end of `Driver::start`.
    StartDone,
    /// Before `release_for_exit_emission` does its work.
    ReleaseCall,
    /// The end of `Driver::stop`.
    StopDone,
    /// Before `on_alloc_terminal` does its work (from the shim thread only).
    HookTerminal,
}

#[derive(Debug, Default)]
struct PauseState {
    parked: Option<Point>,
    grant: Option<Point>,
    /// The driver dropped the dispatch: every later park returns at once.
    abandoned: bool,
}

/// The pause script of one dispatch thread.
struct Script {
    points: Vec<Point>,
    state: Mutex<PauseState>,
    cv: Condvar,
}

thread_local! {
    /// Set only on a dispatch thread; the observer and the driver thread never park.
    static SCRIPT: RefCell<Option<Arc<Script>>> = const { RefCell::new(None) };
}

fn pause(point: Point) {
    let Some(script) = SCRIPT.with(|s| s.borrow().clone()) else { return };
    if !script.points.contains(&point) {
        return;
    }
    let mut state = script.state.lock();
    state.parked = Some(point);
    script.cv.notify_all();
    while state.grant != Some(point) && !state.abandoned {
        script.cv.wait(&mut state);
    }
    state.grant = None;
    state.parked = None;
    script.cv.notify_all();
}

/// One dispatch in flight on its own thread.
struct Dispatch {
    script: Arc<Script>,
    thread: Option<JoinHandle<std::result::Result<(), String>>>,
}

impl Drop for Dispatch {
    fn drop(&mut self) {
        self.script.state.lock().abandoned = true;
        self.script.cv.notify_all();
    }
}

impl Dispatch {
    fn parked(&self) -> Option<Point> {
        self.script.state.lock().parked
    }

    fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn grant(&self, point: Point) {
        let mut state = self.script.state.lock();
        state.grant = Some(point);
        // The park the grant releases is over. Clearing it here, under the same
        // lock, means a later `wait_parked_or_done` can only be satisfied by the
        // dispatch's NEXT park (or its end) — never by the stale arrival the
        // dispatch thread has not yet woken to clear itself.
        state.parked = None;
        self.script.cv.notify_all();
    }

    /// Polls until the dispatch is parked at `point` or finished.
    fn wait_parked_or_done(&self, what: &str) -> std::result::Result<(), DriverError2> {
        let deadline = Instant::now() + SETTLE_BOUND;
        while self.parked().is_none() && !self.finished() {
            if Instant::now() > deadline {
                return Err(DriverError2::Unsettled(what.to_owned()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn join(&mut self) -> std::result::Result<std::result::Result<(), String>, DriverError2> {
        let deadline = Instant::now() + SETTLE_BOUND;
        while !self.finished() {
            if Instant::now() > deadline {
                return Err(DriverError2::Unsettled("the dispatch did not complete".to_owned()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let thread = self.thread.take().expect("joined once");
        thread.join().map_err(|_| DriverError2::Production("the dispatch thread panicked".into()))
    }
}

// ----- the recorded, paced driver ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Running(AllocationId),
    Release(AllocationId),
    Terminal(AllocationId),
    ReleaseSupervision(AllocationId),
}

/// A VM-typed `Driver` over a `SimDriver` whose probe hooks forward to a real `ProbeRunner`.
struct PacedDriver {
    sim: Arc<SimDriver>,
    runner: Arc<ProbeRunner>,
    /// The observer runtime's handle: probe tasks spawn there, not on a dispatch thread.
    observer_rt: tokio::runtime::Handle,
    exit_tx: mpsc::Sender<ExitEvent>,
    exit_rx: Mutex<Option<mpsc::Receiver<ExitEvent>>>,
    calls: Mutex<Vec<Call>>,
}

impl PacedDriver {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }

    fn has(&self, call: &Call) -> bool {
        self.calls.lock().contains(call)
    }

    fn call_count(&self, call: &Call) -> usize {
        self.calls.lock().iter().filter(|recorded| *recorded == call).count()
    }
}

#[async_trait]
impl DriverPort for PacedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(
        &self,
        spec: &AllocationSpec,
    ) -> std::result::Result<AllocationHandle, DriverError> {
        let handle = self.sim.start(spec).await;
        pause(Point::StartDone);
        handle
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        pause(Point::ReleaseCall);
        self.calls.lock().push(Call::Release(handle.alloc.clone()));
        self.sim.release_for_exit_emission(handle).await;
    }

    async fn stop(&self, handle: &AllocationHandle) -> std::result::Result<(), DriverError> {
        let outcome = self.sim.stop(handle).await;
        pause(Point::StopDone);
        outcome
    }

    async fn status(
        &self,
        handle: &AllocationHandle,
    ) -> std::result::Result<AllocationState, DriverError> {
        self.sim.status(handle).await
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> std::result::Result<(), DriverError> {
        self.sim.resize(handle, resources).await
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.calls.lock().push(Call::Running(spec.alloc.clone()));
        let _entered = self.observer_rt.enter();
        let _token = self.runner.start_alloc(spec);
    }

    fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        pause(Point::HookTerminal);
        self.calls.lock().push(Call::Terminal(alloc_id.clone()));
        self.runner.stop_alloc(alloc_id);
    }

    fn on_alloc_stable(&self, alloc_id: &AllocationId) {
        self.runner.stop_role(alloc_id, ProbeRole::Startup);
    }

    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        self.sim.live_allocations()
    }

    fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
        self.sim.try_begin_reclamation(alloc)
    }

    fn release_supervision(&self, alloc: &AllocationId) {
        self.calls.lock().push(Call::ReleaseSupervision(alloc.clone()));
        self.sim.release_supervision(alloc);
    }
}

fn tcp_descriptor(role: ProbeRole, idx: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: if role == ProbeRole::Liveness { Some(3) } else { None },
        success_threshold: if role == ProbeRole::Readiness { Some(1) } else { None },
        inferred: false,
    }
}

fn spec_of(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::from_str("spiffe://overdrive.local/workload/hooks/alloc/0")
            .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: vec![
            tcp_descriptor(ProbeRole::Startup, 0),
            tcp_descriptor(ProbeRole::Readiness, 0),
            tcp_descriptor(ProbeRole::Liveness, 0),
        ],
        transport: None,
        declared_listeners: Vec::new(),
    }
}

/// What the dispatch threads share with the driver thread.
struct Composition {
    /// The observation port the owners write to; the projection reads rows through it
    /// (`ObservationStore::alloc_status_row`).
    obs: Arc<dyn ObservationStore>,
    /// The same store's fault-injection handle (environment fault: a failing row write). Never
    /// read by the projection.
    write_faults: Arc<SimObservationStore>,
    runner: Arc<ProbeRunner>,
    driver: Arc<PacedDriver>,
    /// The production dispatch composition's state (drivers, lifecycle bus, allocator, guest
    /// pool, EXEC gate, shared guest-network owner) the VM seam dispatches over.
    state: Arc<AppState>,
    /// The one shared guest-network owner (converged before `state` copied its flow ports).
    owner: Arc<TestSharedOwner>,
    _supervisor: Arc<GuestNetworkExecSupervisor>,
    _gate: Arc<GuestNetworkExecGate>,
    ticks: Mutex<u64>,
}

impl Composition {
    async fn dispatch(&self, action: Action) -> std::result::Result<(), String> {
        let tick_no = {
            let mut ticks = self.ticks.lock();
            *ticks += 1;
            *ticks
        };
        let now = Instant::now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000 + tick_no)),
            tick: tick_no,
            deadline: now + Duration::from_secs(1),
        };
        dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            self.state.as_ref(),
            &tick,
            self.owner.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())
    }
}

// ----- the driver -----------------------------------------------------------------------

struct HookOwnershipDriver {
    /// Runs the production exit observer and the probe tasks; only while a step awaits it.
    rt: Runtime,
    comp: Arc<Composition>,
    _observer: tokio::task::JoinHandle<()>,
    _tmp: TempDir,
    /// The dispatch each model id has parked or in flight (`Start`/`Register`/`Release`,
    /// `StopEffects`/`StopRow`/`StopHook`).
    start: BTreeMap<i64, Dispatch>,
    stop: BTreeMap<i64, Dispatch>,
    /// Watcher events emitted by the model but not yet forwarded to the observer.
    pending_exits: BTreeMap<i64, ExitEvent>,
    /// Allocations whose guest the watcher knows died (`Die`).
    dead: Vec<i64>,
    /// Stops that completed without parking because an earlier hook had already ended the id.
    hook_already_fired: Vec<i64>,
}

impl HookOwnershipDriver {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let rt = Builder::new_current_thread().enable_all().build().expect("observer runtime");
        let obs =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let clock = Arc::new(SimClock::new());
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::new(overdrive_worker::GuestListenerBoard::new()),
        ));
        let (exit_tx, exit_rx) = mpsc::channel(16);
        let driver = Arc::new(PacedDriver {
            sim: Arc::new(SimDriver::with_clock(
                DriverType::Vm,
                Arc::clone(&clock) as Arc<dyn Clock>,
            )),
            runner: Arc::clone(&runner),
            observer_rt: rt.handle().clone(),
            exit_tx,
            exit_rx: Mutex::new(Some(exit_rx)),
            calls: Mutex::new(Vec::new()),
        });
        let drivers = {
            let mut registry = DriverRegistry::new();
            registry.insert(Arc::clone(&driver) as Arc<dyn DriverPort>);
            Arc::new(registry)
        };
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent"));
        let (events, _rx) = broadcast::channel(64);
        let owner = Arc::new(TestSharedOwner::new());
        let (supervisor, gate, state) = rt.block_on(async {
            let mut runtime =
                crate::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
                    tmp.path(),
                )
                .expect("runtime");
            runtime.register(crate::noop_heartbeat()).await.expect("register heartbeat");
            // Converged BEFORE `AppState::new`, which copies the owner's bound flow ports.
            owner.converge_shared().await.expect("the fixture owner converges");
            let worker = crate::shared_network_test_ports::mtls_worker();
            worker.start_shared_owner().await.expect("the shared mTLS owner starts over the sim");
            let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
            let gate = wiring.gate();
            let supervisor = wiring.supervisor();
            assert!(supervisor.open_after_boot(), "the EXEC gate opens after boot");
            let allocator =
                crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
            let mut state = AppState::new(
                store,
                store_path,
                Arc::clone(&obs) as Arc<dyn ObservationStore>,
                Arc::new(runtime),
                Arc::clone(&driver) as Arc<dyn DriverPort>,
                Arc::clone(&clock) as Arc<dyn Clock>,
                Arc::new(SimDataplane::new()),
                Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
                Arc::new(IdentityMgr::new(None)),
                NodeId::new("node-001").expect("valid node id"),
                allocator,
                crate::test_empty_listener_facts(),
                std::net::Ipv4Addr::LOCALHOST,
                worker,
                Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
                Arc::clone(&gate),
                crate::shared_network_test_ports::guest_pool(),
            );
            state.drivers = drivers;
            state.lifecycle_events = Arc::new(events);
            (supervisor, gate, Arc::new(state))
        });
        let comp = Arc::new(Composition {
            obs: Arc::clone(&obs) as Arc<dyn ObservationStore>,
            write_faults: Arc::clone(&obs),
            runner,
            driver: Arc::clone(&driver),
            state,
            owner,
            _supervisor: supervisor,
            _gate: gate,
            ticks: Mutex::new(0),
        });
        // The production exit observer, for the driver it is spawned for.
        let observer = {
            let _entered = rt.enter();
            exit_observer::spawn_with_runtime(
                Arc::clone(&obs) as Arc<dyn ObservationStore>,
                Arc::clone(&driver) as Arc<dyn DriverPort>,
                Arc::clone(&comp.state.lifecycle_events),
                Arc::clone(&clock) as Arc<dyn Clock>,
                None,
                CancellationToken::new(),
            )
        };
        Self {
            rt,
            comp,
            _observer: observer,
            _tmp: tmp,
            start: BTreeMap::new(),
            stop: BTreeMap::new(),
            pending_exits: BTreeMap::new(),
            dead: Vec::new(),
            hook_already_fired: Vec::new(),
        }
    }

    fn spawn(&self, action: Action, points: Vec<Point>) -> Dispatch {
        let script = Arc::new(Script {
            points,
            state: Mutex::new(PauseState::default()),
            cv: Condvar::new(),
        });
        let comp = Arc::clone(&self.comp);
        let thread_script = Arc::clone(&script);
        let thread = std::thread::spawn(move || {
            SCRIPT.with(|s| *s.borrow_mut() = Some(thread_script));
            let rt = Builder::new_current_thread().enable_all().build().expect("dispatch runtime");
            rt.block_on(comp.dispatch(action))
        });
        Dispatch { script, thread: Some(thread) }
    }

    /// Runs the observer runtime until `cond` holds.
    fn settle(
        &self,
        what: &str,
        mut cond: impl FnMut(&Self) -> bool,
    ) -> std::result::Result<(), DriverError2> {
        let deadline = Instant::now() + SETTLE_BOUND;
        while !cond(self) {
            if Instant::now() > deadline {
                return Err(DriverError2::Unsettled(what.to_owned()));
            }
            self.rt.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        }
        Ok(())
    }

    fn row_is_terminal(&self, a: i64) -> bool {
        let id = alloc_of(a);
        let row = self.rt.block_on(self.comp.obs.alloc_status_row(&id)).expect("read alloc row");
        row.is_some_and(|r| matches!(r.state, AllocState::Failed | AllocState::Terminated))
    }

    // ---- actions ----

    /// Every trace starts from a fresh composition: the previous one's observer, supervisors and
    /// rows are dropped with it.
    fn init(&mut self) {
        *self = Self::new();
    }

    fn start(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        let id = alloc_of(a);
        // A fresh id after a crash-failed attempt is the shim's restart; after a stop or a give-up
        // terminal it is a fresh start (the shim's restart arm does nothing for those rows).
        let predecessor_crashed = a == 2
            && self
                .rt
                .block_on(self.comp.obs.alloc_status_row(&alloc_of(1)))
                .expect("read alloc row")
                .is_some_and(|r| r.state == AllocState::Failed && r.terminal.is_none());
        let action = if a == 1 || !predecessor_crashed {
            Action::StartAllocation {
                alloc_id: id.clone(),
                workload_id: WorkloadId::new("hooks").expect("workload id"),
                node_id: NodeId::new("node-001").expect("valid node id"),
                spec: spec_of(&id),
                kind: WorkloadKind::Service,
            }
        } else {
            Action::RestartAllocation {
                alloc_id: alloc_of(1),
                spec: spec_of(&id),
                kind: WorkloadKind::Service,
            }
        };
        let dispatch = self.spawn(action, vec![Point::StartDone, Point::ReleaseCall]);
        dispatch.wait_parked_or_done("the dispatch reached the end of Driver::start")?;
        if dispatch.parked() != Some(Point::StartDone) {
            let mut dispatch = dispatch;
            let outcome = dispatch.join()?;
            return Err(not_enabled(
                "Start",
                format!("the dispatch ended before Driver::start returned: {outcome:?}"),
            ));
        }
        self.start.insert(a, dispatch);
        Ok(())
    }

    fn register(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        let dispatch =
            self.start.get(&a).ok_or_else(|| not_enabled("Register", "no start parked"))?;
        dispatch.grant(Point::StartDone);
        dispatch.wait_parked_or_done("the dispatch reached the gate release")?;
        if dispatch.finished() {
            let mut dispatch = self.start.remove(&a).expect("present");
            let outcome = dispatch.join()?;
            return Err(DriverError2::Production(format!(
                "the start dispatch ended before the gate release: {outcome:?}"
            )));
        }
        if !self.comp.driver.has(&Call::Running(alloc_of(a))) {
            if dispatch.parked() != Some(Point::ReleaseCall) {
                return Err(DriverError2::Unsettled(
                    "the dispatch parked at an unexpected point".to_owned(),
                ));
            }
            return Err(DriverError2::Unsettled(format!(
                "the dispatch reached release_for_exit_emission before on_alloc_running for \
                 {} (the pre-D3 order); calls: {:?}",
                alloc_of(a),
                self.comp.driver.calls(),
            )));
        }
        Ok(())
    }

    fn release(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        let mut dispatch =
            self.start.remove(&a).ok_or_else(|| not_enabled("Release", "no start parked"))?;
        if dispatch.parked() != Some(Point::ReleaseCall) {
            return Err(not_enabled(
                "Release",
                "the dispatch is not parked before the gate release",
            ));
        }
        dispatch.grant(Point::ReleaseCall);
        dispatch
            .join()?
            .map_err(|e| DriverError2::Production(format!("the start dispatch failed: {e}")))
    }

    fn die(&mut self, a: i64) {
        self.dead.push(a);
    }

    fn emit(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        let id = alloc_of(a);
        if !self.dead.contains(&a) || !self.comp.driver.has(&Call::Release(id.clone())) {
            return Err(not_enabled("Emit", "the guest is alive or the gate is not released"));
        }
        if self.pending_exits.contains_key(&a) {
            return Err(not_enabled("Emit", "the watcher already queued this ExitEvent"));
        }
        self.pending_exits.insert(
            a,
            ExitEvent {
                alloc: id,
                kind: ExitKind::Crashed { exit_code: Some(1), signal: None },
                intentional_stop: false,
                stderr_tail: None,
                oom: None,
            },
        );
        Ok(())
    }

    fn consume(&mut self, a: i64, wrote: bool) -> std::result::Result<(), DriverError2> {
        let id = alloc_of(a);
        let event = self
            .pending_exits
            .remove(&a)
            .ok_or_else(|| not_enabled("Consume", "the watcher has no queued ExitEvent"))?;
        // A shim dispatch may already have released this id's supervision
        // claim. Consume waits for the observer's *new* release, not mere
        // membership in the recording driver's call history.
        let release = Call::ReleaseSupervision(id);
        let releases_before = self.comp.driver.call_count(&release);
        // A write failure is scripted only where the observer will attempt a write; against an id
        // that already has a terminal row its write is the same-attempt no-write (`NoWrite`), the
        // other input the model's `wrote = false` stands for, and a queued failure would leak into
        // the next dispatch's write.
        if !wrote && !self.row_is_terminal(a) {
            self.comp.write_faults.inject_write_failure(ObservationStoreError::Io(
                io::Error::from(io::ErrorKind::PermissionDenied),
            ));
        }
        self.comp
            .driver
            .exit_tx
            .try_send(event)
            .map_err(|error| DriverError2::Production(error.to_string()))?;
        // The observer's last step for one event is `release_supervision`, after every outcome.
        self.settle("the observer handled the ExitEvent", |s| {
            s.comp.driver.call_count(&release) > releases_before
        })
    }

    fn stop_effects(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        let id = alloc_of(a);
        let dispatch = self.spawn(
            Action::StopAllocation {
                alloc_id: id,
                terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
            },
            vec![Point::StopDone, Point::HookTerminal],
        );
        dispatch.wait_parked_or_done("the dispatch reached the end of Driver::stop")?;
        if dispatch.parked() != Some(Point::StopDone) {
            return Err(not_enabled(
                "StopEffects",
                "the dispatch ended before Driver::stop returned",
            ));
        }
        self.stop.insert(a, dispatch);
        Ok(())
    }

    fn stop_row(&mut self, a: i64, wrote: bool) -> std::result::Result<(), DriverError2> {
        let row_is_terminated = self
            .rt
            .block_on(self.comp.obs.alloc_status_row(&alloc_of(a)))
            .expect("read alloc row")
            .is_some_and(|r| r.state == AllocState::Terminated);
        // Scripted only where the stop arm will attempt the write (a `Terminated` row with the
        // same terminal ends the arm without one).
        if !wrote && !row_is_terminated {
            self.comp.write_faults.inject_write_failure(ObservationStoreError::Io(
                io::Error::from(io::ErrorKind::PermissionDenied),
            ));
        }
        if let Some(dispatch) = self.stop.get(&a) {
            dispatch.grant(Point::StopDone);
        } else {
            // The retry: a fresh StopAllocation (the guest is already stopped; not-found is tolerated).
            let dispatch = self.spawn(
                Action::StopAllocation {
                    alloc_id: alloc_of(a),
                    terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
                },
                vec![Point::HookTerminal],
            );
            self.stop.insert(a, dispatch);
        }
        let dispatch = self.stop.get(&a).expect("inserted above");
        dispatch.wait_parked_or_done("the dispatch ran to the row write")?;
        if dispatch.finished() {
            // Either the row write failed: the dispatch ended with its error, without the hook
            // (B2). Or the id's end was already claimed by `FinalizeFailed`, which removed the
            // shim's driver index entry, so the stop arm has no hook to call and the dispatch
            // completes without parking; that hook then already fired.
            let mut dispatch = self.stop.remove(&a).expect("present");
            let outcome = dispatch.join()?;
            if wrote {
                outcome
                    .map_err(|e| DriverError2::Production(format!("the stop retry failed: {e}")))?;
                if !self.comp.driver.has(&Call::Terminal(alloc_of(a))) {
                    return Err(DriverError2::Unsettled(
                        "the stop dispatch landed its row without ever calling on_alloc_terminal"
                            .to_owned(),
                    ));
                }
                self.hook_already_fired.push(a);
            }
        }
        Ok(())
    }

    fn stop_hook(&mut self, a: i64) -> std::result::Result<(), DriverError2> {
        if let Some(at) = self.hook_already_fired.iter().position(|x| *x == a) {
            self.hook_already_fired.remove(at);
            return Ok(());
        }
        let mut dispatch =
            self.stop.remove(&a).ok_or_else(|| not_enabled("StopHook", "no stop parked"))?;
        if dispatch.parked() != Some(Point::HookTerminal) {
            return Err(not_enabled("StopHook", "the stop dispatch is not parked before the hook"));
        }
        dispatch.grant(Point::HookTerminal);
        dispatch
            .join()?
            .map_err(|e| DriverError2::Production(format!("the stop dispatch failed: {e}")))
    }

    fn finalize(&self, a: i64) -> std::result::Result<(), DriverError2> {
        let mut dispatch = self.spawn(
            Action::FinalizeFailed {
                alloc_id: alloc_of(a),
                terminal: Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
            },
            Vec::new(),
        );
        dispatch
            .join()?
            .map_err(|e| DriverError2::Production(format!("the finalize dispatch failed: {e}")))
    }

    /// Pins the model id `a` to the projection.
    fn project(&self) -> std::result::Result<HookState, DriverError2> {
        let mut al = BTreeMap::new();
        let mut supervised = 0_usize;
        for a in [1_i64, 2] {
            let id = alloc_of(a);
            let sup = [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness]
                .into_iter()
                .any(|role| self.comp.runner.is_role_live(&id, role));
            if sup {
                supervised += 1;
            }
            al.insert(a, AllocView { sup, row: self.row_is_terminal(a) });
        }
        let counted = self.comp.runner.active_alloc_count();
        if counted != supervised {
            return Err(DriverError2::Production(format!(
                "active_alloc_count() is {counted} but {supervised} allocations have a live role"
            )));
        }
        Ok(HookState { al })
    }

    fn step_on(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            Start(a: i64) => self.start(a)?,
            Register(a: i64) => self.register(a)?,
            Release(a: i64) => self.release(a)?,
            Die(a: i64) => self.die(a),
            Emit(a: i64) => self.emit(a)?,
            Consume(a: i64, wrote: bool) => self.consume(a, wrote)?,
            StopEffects(a: i64) => self.stop_effects(a)?,
            StopRow(a: i64, wrote: bool) => self.stop_row(a, wrote)?,
            StopHook(a: i64) => self.stop_hook(a)?,
            Finalize(a: i64) => self.finalize(a)?,
            Idle => {},
        })
    }
}

impl Driver for HookOwnershipDriver {
    type State = HookState;

    fn config() -> Config {
        Config { state: &[], nondet: &["hooks_conf::hook_ownership::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.step_on(step)
    }
}

// ----- the compared state ---------------------------------------------------------------

/// The compared fields of one model allocation id (`started`, `regd`, `rel`, `dead`, `ev`, `vmstop`,
/// `srw`, `stopped`, `fin` and `late` are model-only ghosts).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AllocView {
    /// A probe supervisor is present.
    sup: bool,
    /// A terminal row exists.
    row: bool,
}

#[derive(Debug, PartialEq)]
struct HookState {
    al: BTreeMap<i64, AllocView>,
}

impl HookState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(value, MAIN, MODULE)?;
        Ok(Self {
            al: quint_itf::int_map(vars.var("al")?, |entry| {
                Ok(AllocView {
                    sup: quint_itf::boolean(quint_itf::field(entry, "sup")?)?,
                    row: quint_itf::boolean(quint_itf::field(entry, "row")?)?,
                })
            })?,
        })
    }
}

impl<'de> Deserialize<'de> for HookState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<HookOwnershipDriver> for HookState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &HookOwnershipDriver) -> Result<Self> {
        Ok(driver.project()?)
    }
}

// ----- named runs (one `#[quint_test]` per run of `hooks_conf`) and randomized traces ---------

/// Named run `crash_without_restart` (#334). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "crash_without_restart"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_crash_without_restart() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Named run `crash_then_fresh_id_restart` (#318). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "crash_then_fresh_id_restart"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_crash_then_fresh_id_restart() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Named run `write_exhaustion_still_stops_probes` (D1 outcome independence). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "write_exhaustion_still_stops_probes"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_write_exhaustion_still_stops_probes() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Named run `operator_or_liveness_stop_emits_no_exit_event` (D4, B2). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "operator_or_liveness_stop_emits_no_exit_event"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_operator_or_liveness_stop_emits_no_exit_event() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Named run `finalize_on_live_vm_stops_probes` (D4). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "finalize_on_live_vm_stops_probes"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_finalize_on_live_vm_stops_probes() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Named run `double_fire_is_idempotent` (D2). CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf",
    max_samples = 1,
    test = "double_fire_is_idempotent"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_double_fire_is_idempotent() -> impl Driver {
    HookOwnershipDriver::new()
}

/// Randomized traces of the recording main; the seed is printed on failure.
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/hook_ownership.qnt",
    main = "hooks_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_hook_ownership_randomized_traces() -> impl Driver {
    HookOwnershipDriver::new()
}
