//! Shared composition for the restart-authority composed scenarios
//! (feature `service-allocation-readiness-and-ending`, decisions D5 to D8 and
//! D12): the production [`ReconcilerRuntime`] with `service-lifecycle` and
//! `workload-lifecycle`, the production action shim (through the convergence
//! tick), the production exit observer, the production [`ProbeRunner`] and the
//! production Service stream, with Sim adapters at the driven ports only
//! (`SimObservationStore`, `SimClock`, `SimGuestStreamDial`, `SimDriver`).
//!
//! The node's `Driver` is the shared [`HookedDriver`] (a `SimDriver` with the real
//! `ProbeRunner` behind the supervision hooks, recording call order, owning its exit
//! channel so a scenario can deliver OOM facts through the production exit observer).
//!
//! Preconditions are set up (the Service intent and its VIP). Every allocation
//! row, probe row, verdict, view entry and terminal row the scenarios observe
//! is authored by production.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "a test composition asserts its own preconditions; each scenario file uses a subset"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_network_provisioner_for_test,
};
use overdrive_control_plane::streaming::{ServiceSubmitEvent, build_service_accepted};
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::dial::GuestDialError;
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
};
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, SpiffeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType, ExitEvent, ExitKind, OomFacts};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocStatusRow, ObservationStore, ObservationStoreError,
};
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_stream_dial::{SimDialScript, SimGuestStreamDial};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use parking_lot::Mutex;
use tempfile::TempDir;

pub const WORKLOAD: &str = "restarting";
pub const LISTENER_PORT: u16 = 8080;
pub const STARTUP_PORT: u16 = 8081;
pub const READINESS_PORT: u16 = 8082;
pub const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);
const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// Every probe's period.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// The startup probe's `max_attempts`; with the one-second interval the
/// startup deadline is two seconds.
pub const STARTUP_MAX_ATTEMPTS: u32 = 2;
/// The Service stream's progress deadline the scenarios pass (the 90 s floor:
/// above `STARTUP_MAX_ATTEMPTS` x `PROBE_INTERVAL` + the 30 s margin).
pub const STREAM_DEADLINE: Duration = Duration::from_secs(90);

/// The seed of every Sim adapter in the world (each is deterministic: the
/// observation store takes it explicitly, the clock and the dial are scripted).
pub const SEED: u64 = 0;

/// What the Service declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One TCP startup probe, no readiness or liveness probe.
    StartupProbe,
    /// One TCP startup probe and one TCP readiness probe (success threshold 1).
    StartupAndReadiness,
    /// One TCP startup probe at the platform default window (30 attempts, two
    /// seconds apart: a 60 s startup deadline).
    DefaultStartupWindow,
    /// The explicit opt-out: `[[health_check.startup]] = []`.
    OptOut,
}

pub fn tcp(role: ProbeRole, port: u16) -> ProbeDescriptor {
    tcp_with(role, port, STARTUP_MAX_ATTEMPTS, 1)
}

pub fn tcp_with(role: ProbeRole, port: u16, max_attempts: u32, interval: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port },
        timeout_seconds: 1,
        interval_seconds: interval,
        max_attempts,
        failure_threshold: (role == ProbeRole::Liveness).then_some(1),
        success_threshold: (role == ProbeRole::Readiness).then_some(1),
        inferred: false,
    }
}

pub fn intent_of(shape: Shape) -> WorkloadIntent {
    let startup = match shape {
        Shape::StartupProbe | Shape::StartupAndReadiness => {
            vec![tcp(ProbeRole::Startup, STARTUP_PORT)]
        }
        Shape::DefaultStartupWindow => vec![tcp_with(ProbeRole::Startup, STARTUP_PORT, 30, 2)],
        Shape::OptOut => vec![],
    };
    let readiness = match shape {
        Shape::StartupAndReadiness => vec![tcp(ProbeRole::Readiness, READINESS_PORT)],
        _ => vec![],
    };
    let svc = Service::from_submit(ServiceSpecInput {
        id: WORKLOAD.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: LISTENER_PORT, protocol: "tcp".to_owned() }],
        startup_probes: startup,
        readiness_probes: readiness,
        liveness_probes: vec![],
    })
    .expect("valid service spec");
    WorkloadIntent::Service(svc)
}

pub use crate::acceptance::hooked_driver::Call as DriverCall;
use crate::acceptance::hooked_driver::{HookedDriver, NoopNetworkProvisioner};

fn transport() -> GuestTransportAssignment {
    GuestTransportAssignment::new(
        WORKLOAD_ADDR,
        GATEWAY,
        GuestCid::new(GUEST_CID_BASE).expect("first leased CID"),
        LaunchToken::from_bytes([7; 16]).expect("non-zero launch token"),
        GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
            .expect("two distinct ports above the reserved range"),
    )
}

/// What one startup probe period's dial does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dial {
    /// The guest binds the listener: the probe passes.
    Binds,
    /// The guest is alive and refuses the connection: the probe fails.
    Refuses,
}

/// The composed world.
pub struct RestartWorld {
    pub state: AppState,
    pub obs: Arc<SimObservationStore>,
    pub clock: Arc<SimClock>,
    pub dial: Arc<SimGuestStreamDial>,
    pub sim: Arc<SimDriver>,
    pub driver: Arc<HookedDriver>,
    pub runner: Arc<ProbeRunner>,
    pub workload: WorkloadId,
    pub target: TargetResource,
    pub service_id: ServiceId,
    service_lifecycle: ReconcilerName,
    workload_lifecycle: ReconcilerName,
    tick_n: u64,
    _observer: tokio::task::JoinHandle<()>,
    tmp: TempDir,
}

impl RestartWorld {
    /// Compose the world: the Service intent persisted, its VIP allocated, the
    /// exit observer running over the node's driver. No allocation exists yet.
    pub async fn compose(shape: Shape) -> Self {
        eprintln!("restart-authority world: seed={SEED} shape={shape:?} (SimClock, SimObservationStore, scripted SimGuestStreamDial)");
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        runtime.register(service_lifecycle()).await.expect("register service-lifecycle");

        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let workload = WorkloadId::new(WORKLOAD).expect("workload id");
        let intent = intent_of(shape);
        let archived = intent.archive_for_store().expect("rkyv archive");
        store
            .put(IntentKey::for_workload(&workload).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
        store
            .put(
                IntentKey::for_workload_kind(&workload).as_bytes(),
                &[WorkloadKind::Service.discriminator_byte()],
            )
            .await
            .expect("put kind");

        let node = NodeId::new("local").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, SEED));
        let clock = Arc::new(SimClock::new());
        let dial = Arc::new(SimGuestStreamDial::new());
        let sim = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&dial) as Arc<dyn overdrive_core::guest_transport::dial::GuestStreamDial>,
            Arc::new(overdrive_worker::GuestListenerBoard::new()),
        ));
        let driver = Arc::new(
            HookedDriver::new(Arc::clone(&sim), Arc::clone(&runner), Arc::clone(&clock))
                .with_default_transport(transport()),
        );
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&store) as Arc<dyn IntentStore>,
        );
        let state = AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(SEED),
            ))),
            Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
            NodeId::new("writer-1").expect("node id"),
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            crate::acceptance::serve_ports::worker(),
            crate::acceptance::serve_ports::owner(),
            crate::acceptance::serve_ports::exec_gate(),
            crate::acceptance::serve_ports::pool(),
        );

        let service_id = {
            let digest = intent.spec_digest().expect("spec digest");
            let mut guard = state.allocator.lock().await;
            let vip: ServiceVip = guard.allocate(*digest.as_bytes()).await.expect("allocate vip");
            drop(guard);
            ServiceId::derive(
                &vip,
                NonZeroU16::new(LISTENER_PORT).expect("non-zero port"),
                Proto::Tcp,
                "service-map",
            )
        };

        let observer = overdrive_control_plane::worker::exit_observer::spawn_with_runtime(
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&driver) as Arc<dyn Driver>,
            state.lifecycle_events.clone(),
            Arc::clone(&clock) as Arc<dyn Clock>,
            None,
            tokio_util::sync::CancellationToken::new(),
        );

        Self {
            state,
            obs,
            clock,
            dial,
            sim,
            driver,
            runner,
            target: TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target"),
            workload,
            service_id,
            service_lifecycle: ReconcilerName::new("service-lifecycle").expect("name"),
            workload_lifecycle: ReconcilerName::new("workload-lifecycle").expect("name"),
            tick_n: 0,
            _observer: observer,
            tmp,
        }
    }

    async fn tick(&mut self, name: &ReconcilerName) -> Result<(), String> {
        self.tick_n += 1;
        let now: Instant = self.clock.now();
        let result = run_convergence_tick_with_network_provisioner_for_test(
            &self.state,
            name,
            &self.target,
            now,
            self.tick_n,
            now + Duration::from_secs(1),
            &NoopNetworkProvisioner,
        )
        .await
        .map_err(|e| e.to_string());
        settle().await;
        result
    }

    /// One `workload-lifecycle` convergence tick (hydrate, reconcile, view
    /// write-through, action dispatch, awaited).
    pub async fn workload_tick(&mut self) -> Result<(), String> {
        let name = self.workload_lifecycle.clone();
        self.tick(&name).await
    }

    /// One `service-lifecycle` convergence tick.
    pub async fn service_tick(&mut self) -> Result<(), String> {
        let name = self.service_lifecycle.clone();
        self.tick(&name).await
    }

    /// Advance the clock by `by` (the restart backoff, a probe interval).
    pub async fn advance(&self, by: Duration) {
        self.clock.tick(by);
        settle().await;
    }

    /// One startup-probe period: the dial of the startup target does `dial`,
    /// then the probe tasks run once.
    pub async fn startup_period(&self, dial: Dial) {
        self.startup_period_for(dial, PROBE_INTERVAL).await;
    }

    /// As [`Self::startup_period`] for a probe whose interval is `every`.
    pub async fn startup_period_for(&self, dial: Dial, every: Duration) {
        let target = SocketAddrV4::new(WORKLOAD_ADDR, STARTUP_PORT);
        let script = match dial {
            Dial::Binds => SimDialScript::accept(),
            Dial::Refuses => SimDialScript::Refuse(GuestDialError::Refused {
                errno: KernelErrno::ECONNREFUSED,
            }),
        };
        self.dial.script(target, script);
        self.advance(every).await;
    }

    /// One readiness-probe period passing.
    pub async fn readiness_passes(&self) {
        self.dial.script(SocketAddrV4::new(WORKLOAD_ADDR, READINESS_PORT), SimDialScript::accept());
        self.advance(PROBE_INTERVAL).await;
    }

    /// The guest of `alloc` exits on its own with `exit_code` (and `oom`
    /// facts), delivered through the production exit observer.
    pub async fn guest_exits(
        &self,
        alloc: &AllocationId,
        exit_code: Option<i32>,
        oom: Option<OomFacts>,
    ) {
        let event = ExitEvent {
            alloc: alloc.clone(),
            kind: ExitKind::Crashed { exit_code, signal: exit_code.is_none().then_some(9) },
            intentional_stop: false,
            stderr_tail: Some("guest stderr tail".to_owned()),
            oom,
        };
        self.driver.deliver_exit(event).await;
        settle().await;
    }

    /// Script every later `Driver::stop` to fail (`Some`) or to behave
    /// normally (`None`).
    pub fn script_stop_failure(&self, reason: Option<&str>) {
        self.sim.script_stop_failure(reason);
    }

    /// Queue one observation write failure.
    pub fn fail_next_observation_write(&self) {
        self.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected observation write failure",
        )));
    }

    /// Every allocation row of the workload, by allocation id.
    pub async fn rows(&self) -> BTreeMap<AllocationId, AllocStatusRow> {
        self.obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .filter(|row| row.workload_id == self.workload)
            .map(|row| (row.alloc_id.clone(), row))
            .collect()
    }

    /// The allocation ids in placement order (the shim mints them in order).
    pub async fn attempts(&self) -> Vec<AllocationId> {
        self.rows().await.into_keys().collect()
    }

    /// The most recently placed allocation (the highest-ordered id).
    pub async fn current(&self) -> AllocationId {
        self.attempts().await.pop().expect("an allocation exists")
    }

    pub async fn row(&self, alloc: &AllocationId) -> Option<AllocStatusRow> {
        self.rows().await.remove(alloc)
    }

    /// The stored startup probe row of `alloc`.
    pub async fn startup_row(&self, alloc: &AllocationId) -> Option<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(alloc)
            .await
            .expect("list probe rows")
            .into_iter()
            .find(|row| row.role == ProbeRole::Startup && row.probe_idx == ProbeIdx::new(0))
    }

    /// Whether the readiness projection publishes `alloc` as a backend
    /// (`None`: no backend row lists it).
    pub async fn backend_healthy(&self, alloc: &AllocationId) -> Option<bool> {
        let spiffe = SpiffeId::for_allocation(&self.workload, alloc);
        self.obs
            .service_backends_rows(&self.service_id)
            .await
            .expect("backend rows")
            .into_iter()
            .flat_map(|row| row.backends)
            .find(|backend| backend.alloc == spiffe)
            .map(|backend| backend.healthy)
    }

    /// What the mesh resolver's `BackendIndex` answers for `alloc`'s backend
    /// address, after applying every written `ServiceBackendRow`: `Some(true)`
    /// when it resolves to the backend (`Mesh`), `Some(false)` for the
    /// fail-closed `MeshUnreachable`, `None` when no row lists the allocation.
    pub async fn resolver_routes_to(&self, alloc: &AllocationId) -> Option<bool> {
        use overdrive_control_plane::mtls_resolve_adapter::BackendIndex;
        use overdrive_core::traits::mtls_resolve::MtlsResolution;
        let rows = self.obs.service_backends_rows(&self.service_id).await.expect("backend rows");
        let mut index = BackendIndex::default();
        for row in &rows {
            index.apply_row(row.service_id, &row.backends);
        }
        let spiffe = SpiffeId::for_allocation(&self.workload, alloc);
        let backend = rows.iter().flat_map(|row| row.backends.iter()).find(|b| b.alloc == spiffe)?;
        let std::net::SocketAddr::V4(addr) = backend.addr else { return None };
        Some(matches!(index.classify(addr, Proto::Tcp), MtlsResolution::Mesh(_)))
    }

    /// Open the production Service stream and collect its lines (the stream is
    /// subscribed when this returns).
    pub async fn open_stream(&self) -> StreamCollector {
        let accepted = build_service_accepted(
            "digest".to_owned(),
            format!("workloads/{WORKLOAD}"),
            overdrive_control_plane::api::IdempotencyOutcome::Inserted,
        );
        let stream = overdrive_control_plane::streaming::build_service_stream(
            self.state.clone(),
            self.workload.clone(),
            accepted,
            STREAM_DEADLINE,
        );
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let task = tokio::spawn(async move {
            let mut stream = Box::pin(stream);
            while let Some(item) = stream.next().await {
                let bytes = item.expect("stream line");
                let event: ServiceSubmitEvent =
                    serde_json::from_slice(&bytes).expect("a service submit event line");
                sink.lock().push(event);
            }
        });
        settle().await;
        StreamCollector { lines, _task: task }
    }

    /// Place and start the first allocation through `workload-lifecycle`.
    pub async fn place_first_attempt(&mut self) -> AllocationId {
        self.workload_tick().await.expect("workload-lifecycle places the first attempt");
        self.current().await
    }

}

/// The lines a Service stream has yielded so far.
pub struct StreamCollector {
    lines: Arc<Mutex<Vec<ServiceSubmitEvent>>>,
    _task: tokio::task::JoinHandle<()>,
}

impl StreamCollector {
    pub fn lines(&self) -> Vec<ServiceSubmitEvent> {
        self.lines.lock().clone()
    }

    /// The first terminal line (everything after `Accepted`).
    pub fn first_terminal(&self) -> Option<ServiceSubmitEvent> {
        self.lines().into_iter().nth(1)
    }

    /// The stream has closed (a terminal line was yielded).
    pub fn closed(&self) -> bool {
        self.lines.lock().len() > 1
    }
}

/// Let every spawned task (probe tasks, the observer, the stream) run to its
/// next park.
pub async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// A snapshot of every port-exposed name the restart-authority scenarios
/// observe: allocation rows (state, terminal, reason, startup row number),
/// the supervised-allocation count and the driver call counts.
pub type Observed = BTreeMap<String, String>;

impl RestartWorld {
    pub async fn observe(&self) -> Observed {
        let mut out = Observed::new();
        for (alloc, row) in self.rows().await {
            out.insert(format!("alloc/{alloc}/state"), format!("{:?}", row.state));
            out.insert(format!("alloc/{alloc}/terminal"), format!("{:?}", row.terminal));
            out.insert(format!("alloc/{alloc}/reason"), format!("{:?}", row.reason));
            let run = self.startup_row(&alloc).await.map(|r| r.run_seq.get());
            out.insert(format!("alloc/{alloc}/startup_run_seq"), format!("{run:?}"));
        }
        out.insert("supervised_allocations".to_owned(), self.runner.active_alloc_count().to_string());
        let calls = self.driver.calls();
        out.insert(
            "driver/stop_calls".to_owned(),
            calls.iter().filter(|c| matches!(c, DriverCall::Stop(_))).count().to_string(),
        );
        out.insert(
            "driver/terminal_hook_calls".to_owned(),
            calls.iter().filter(|c| matches!(c, DriverCall::Terminal(_))).count().to_string(),
        );
        out
    }

    /// A control-plane restart: the runtime is dropped and a new one loads the
    /// persisted views from the same files.
    pub async fn restart_control_plane(&mut self) {
        let parked = TempDir::new().expect("tempdir");
        let placeholder = Arc::new(
            ReconcilerRuntime::new_with_redb_view_store_for_test(parked.path())
                .expect("placeholder runtime"),
        );
        drop(std::mem::replace(&mut self.state.runtime, placeholder));
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(self.tmp.path())
            .expect("reloaded runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        runtime.register(service_lifecycle()).await.expect("register service-lifecycle");
        self.state.runtime = Arc::new(runtime);
    }

    /// The guest of `alloc` exits while the next `Driver::stop` is dispatched.
    pub fn crash_while_next_stop_is_dispatched(&self, alloc: &AllocationId, exit_code: i32) {
        self.driver.exit_during_next_stop(ExitEvent {
            alloc: alloc.clone(),
            kind: ExitKind::Crashed { exit_code: Some(exit_code), signal: None },
            intentional_stop: false,
            stderr_tail: Some("guest stderr tail".to_owned()),
            oom: None,
        });
    }

    /// The next attempt: wait out the restart backoff, then one
    /// `workload-lifecycle` tick places the successor.
    pub async fn restart_successor(&mut self) -> AllocationId {
        self.advance(Duration::from_secs(2)).await;
        self.workload_tick().await.expect("workload-lifecycle places the successor");
        self.current().await
    }

    /// One never-ready attempt: the guest stays alive and refuses until the
    /// startup deadline, then `service-lifecycle` looks (and, by D7, stops it).
    pub async fn never_ready_until_the_look(&mut self, periods: u32) -> Result<(), String> {
        for _ in 0..periods {
            self.startup_period(Dial::Refuses).await;
        }
        self.service_tick().await
    }
}

/// The universe-bound state delta (Mandate 8) for a step that changes nothing
/// observable: every key of the universe (every observed name) is unchanged.
pub fn assert_universe_unchanged(before: &Observed, after: &Observed) {
    for key in before.keys().chain(after.keys()) {
        assert_eq!(
            before.get(key),
            after.get(key),
            "{key} is in the universe and not named by the step: it must be unchanged"
        );
    }
}
