//! The production composition the probe-run-counting evidence drives
//! (netns-density-295 PROBE-RUN-COUNT / ADR-0191, PROBE-NOSAMPLE /
//! ADR-0187, PROBE-NOSAMPLE-EVENT / ADR-0192): R5-PROBE-COUNT
//! (`probe_run_count.rs`) and the `probe-count` conformance driver
//! (`quint_probe_count.rs`).
//!
//! Composition (feature-delta § *Formal protocol model* → "Probe-run
//! counting", conformance placement):
//!
//! - the production [`ReconcilerRuntime`] with `service-lifecycle`
//!   registered (plus `workload-lifecycle` where a scenario needs the
//!   restart / GC authority), inside the [`AppState`] that
//!   [`run_convergence_tick`] takes — hydrate → reconcile → view
//!   write-through → action dispatch, awaited;
//! - the production [`ProbeRunner`] with the allocation started
//!   ([`ProbeRunner::start_alloc`]), whose probe tasks write the probe rows;
//! - Sim adapters at the driven ports only: [`SimObservationStore`],
//!   [`SimClock`], [`SimGuestStreamDial`] (the runner's required
//!   `guest_dial`) and [`SimDriver`] (the `Driver` port the action shim
//!   stops the allocation through). The runner's `TcpProber` /
//!   `HttpProber` are Sim adapters too, but a VM probe's target —
//!   `workload_addr`, inside the guest prefix — is dialled, so no call
//!   reaches them.
//!
//! Preconditions are set up, never consequences: the Service intent (with
//! its probe declarations) is persisted, its VIP allocated through the
//! production allocator, and the allocation's `Running` row written as the
//! owning node writes it on start. Every probe row, verdict, view entry and
//! terminal row the scenarios observe is authored by production.

#![allow(
    clippy::needless_pass_by_ref_mut,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    dead_code,
    reason = "a test composition asserts its own preconditions; protocol names are prose; the two consumers (R5-PROBE-COUNT and the conformance driver) each use a subset of the composition"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
};
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, SpiffeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
    TransitionSource,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_core::vm::config::VsockPort;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::workload_lifecycle::project_probe_descriptors;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::{SimDialScript, SimGuestStreamDial};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use tempfile::TempDir;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{ConvergenceError, ReconcilerRuntime, run_convergence_tick};
use crate::AppState;
use crate::action_shim::ShimError;

/// The probed Service.
pub(super) const WORKLOAD: &str = "probed";
/// The Service's one listener.
pub(super) const LISTENER_PORT: u16 = 8080;
/// The allocation's workload address (`workload_addr`): the VM's address
/// inside the guest prefix, the target every VM probe dials.
pub(super) const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);
/// The node's guest gateway.
const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// Each role probes its own port so a scenario can script the roles apart.
pub(super) const STARTUP_PORT: u16 = 8081;
pub(super) const READINESS_PORT: u16 = 8082;
pub(super) const LIVENESS_PORT: u16 = 8083;
/// Every probe's period.
pub(super) const PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// The liveness failure threshold (`LT` in the spec).
pub(super) const LIVENESS_FAILURE_THRESHOLD: u32 = 3;
/// The readiness success threshold (`RT` in the spec).
pub(super) const READINESS_SUCCESS_THRESHOLD: u32 = 2;

/// Yield budget that lets every spawned probe task reach its next park.
const SETTLE_YIELDS: usize = 64;

/// Which probes the Service declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeSet {
    /// A TCP readiness and a TCP liveness probe, no startup probe (the
    /// `[[health_check.startup]] = []` opt-out: Stable at the first look).
    /// The conformance driver's composition.
    ReadinessAndLiveness,
    /// A TCP startup, a TCP readiness and a TCP liveness probe.
    AllTcp,
    /// A TCP startup and readiness probe and an HTTP liveness probe on
    /// `path`.
    HttpLiveness { path: &'static str },
}

/// What one probe run's dial does for a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RunOutcome {
    /// The dial succeeds (TCP: healthy; HTTP: the guest answers `200`).
    Pass,
    /// The dial is refused by the guest application (`Refused`,
    /// `ECONNREFUSED`): a failed probe.
    Fail,
    /// The HTTP request runs and the guest answers `503`: a failed probe.
    HttpServerError,
    /// The dial is refused for node flow capacity: no sample.
    NoSampleCapacity,
    /// The dial is refused because node forwarding is quiesced: no sample.
    NoSampleQuiesced,
    /// The dial succeeds but the row's observation write fails.
    WriteFail,
}

impl RunOutcome {
    fn script(&self, role: ProbeRole, probes: ProbeSet) -> SimDialScript {
        let http = matches!(probes, ProbeSet::HttpLiveness { .. }) && role == ProbeRole::Liveness;
        match self {
            Self::Pass | Self::WriteFail if http => SimDialScript::Accept {
                reply: b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n".to_vec(),
            },
            Self::Pass | Self::WriteFail => SimDialScript::accept(),
            Self::HttpServerError => SimDialScript::Accept {
                reply: b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n".to_vec(),
            },
            Self::Fail => {
                SimDialScript::Refuse(GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED })
            }
            Self::NoSampleCapacity => SimDialScript::Refuse(GuestDialError::Capacity),
            Self::NoSampleQuiesced => SimDialScript::Refuse(GuestDialError::Quiesced),
        }
    }
}

/// The composed production owners and the driven-port sims.
pub(super) struct ProbeWorld {
    pub(super) state: AppState,
    pub(super) obs: Arc<SimObservationStore>,
    pub(super) clock: Arc<SimClock>,
    pub(super) dial: Arc<SimGuestStreamDial>,
    pub(super) driver: Arc<SimDriver>,
    pub(super) runner: ProbeRunner,
    pub(super) probes: ProbeSet,
    pub(super) workload: WorkloadId,
    pub(super) alloc: AllocationId,
    pub(super) target: TargetResource,
    pub(super) service_id: Option<ServiceId>,
    service_lifecycle: ReconcilerName,
    workload_lifecycle: ReconcilerName,
    tick_n: u64,
    row_counter: u64,
    _tmp: TempDir,
}

fn tcp(role: ProbeRole, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: (role == ProbeRole::Liveness).then_some(LIVENESS_FAILURE_THRESHOLD),
        success_threshold: (role == ProbeRole::Readiness).then_some(READINESS_SUCCESS_THRESHOLD),
        inferred: false,
    }
}

fn service_intent(probes: ProbeSet) -> WorkloadIntent {
    let startup = match probes {
        ProbeSet::ReadinessAndLiveness => vec![],
        ProbeSet::AllTcp | ProbeSet::HttpLiveness { .. } => {
            vec![tcp(ProbeRole::Startup, STARTUP_PORT)]
        }
    };
    let liveness = match probes {
        ProbeSet::HttpLiveness { path } => ProbeDescriptor {
            mechanic: ProbeMechanic::Http {
                path: path.to_owned(),
                port: LIVENESS_PORT,
                host: None,
            },
            ..tcp(ProbeRole::Liveness, LIVENESS_PORT)
        },
        ProbeSet::ReadinessAndLiveness | ProbeSet::AllTcp => {
            tcp(ProbeRole::Liveness, LIVENESS_PORT)
        }
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
        readiness_probes: vec![tcp(ProbeRole::Readiness, READINESS_PORT)],
        liveness_probes: vec![liveness],
    })
    .expect("valid probed service spec");
    WorkloadIntent::Service(svc)
}

/// The allocation spec the VM driver hands the probe runner on Running:
/// the intent's probe declarations and the allocation's guest transport.
pub(super) fn vm_spec(
    workload: &WorkloadId,
    alloc: &AllocationId,
    intent: &WorkloadIntent,
) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(workload, alloc),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/serve".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: project_probe_descriptors(intent),
        transport: Some(GuestTransportAssignment::new(
            WORKLOAD_ADDR,
            GATEWAY,
            GuestCid::new(GUEST_CID_BASE).expect("first leased CID"),
            LaunchToken::from_bytes([7; 16]).expect("non-zero launch token"),
            // GUEST-PORTS: the owner's bound flow ports the assignment
            // carries; the probe runner dials through the scripted
            // `SimGuestStreamDial`, which reads none of them.
            GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
                .expect("two distinct ports above the reserved range"),
        )),
        service_ports: vec![NonZeroU16::new(LISTENER_PORT).expect("non-zero listener port")],
    }
}

/// Let every spawned probe task run to its next park.
pub(super) async fn settle() {
    for _ in 0..SETTLE_YIELDS {
        tokio::task::yield_now().await;
    }
}

impl ProbeWorld {
    /// Compose the world: persist the intent, allocate its VIP, register the
    /// allocation Running, start its probe tasks and, for the opt-out probe
    /// set, take the first look (which announces Stable).
    pub(super) async fn compose(probes: ProbeSet) -> Self {
        Self::compose_with(probes, false).await
    }

    /// As [`Self::compose`], with `workload-lifecycle` also registered (the
    /// single restart and GC authority, ADR-0087).
    pub(super) async fn compose_with_workload_lifecycle(probes: ProbeSet) -> Self {
        Self::compose_with(probes, true).await
    }

    async fn compose_with(probes: ProbeSet, with_workload_lifecycle: bool) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
            .expect("reconciler runtime");
        runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");
        if with_workload_lifecycle {
            runtime
                .register(crate::workload_lifecycle())
                .await
                .expect("register workload-lifecycle");
        }

        let node = NodeId::new("probe-node").expect("node id");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let workload = WorkloadId::new(WORKLOAD).expect("workload id");
        let intent = service_intent(probes);
        persist_intent(&store, &workload, &intent).await;

        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let dial = Arc::new(SimGuestStreamDial::new());
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let state = AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node.clone(),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            crate::shared_network_test_ports::mtls_worker(),
            crate::shared_network_test_ports::shared_guest_network_owner(),
            crate::shared_network_test_ports::guest_network_exec_gate(),
            crate::shared_network_test_ports::guest_pool(),
        );

        // The VIP the readiness projection publishes the backend under
        // (production allocator path, as the submit handler allocates it).
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

        let runner = ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&dial) as Arc<dyn overdrive_core::guest_transport::dial::GuestStreamDial>,
        );

        let alloc = AllocationId::new(&format!("{WORKLOAD}-0")).expect("alloc id");
        let mut world = Self {
            state,
            obs,
            clock,
            dial,
            driver,
            runner,
            probes,
            workload,
            alloc,
            target: TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target"),
            service_id: Some(service_id),
            service_lifecycle: ReconcilerName::new("service-lifecycle").expect("name"),
            workload_lifecycle: ReconcilerName::new("workload-lifecycle").expect("name"),
            tick_n: 0,
            row_counter: 0,
            _tmp: tmp,
        };
        let alloc = world.alloc.clone();
        world.register_running(&alloc).await;
        let spec = vm_spec(&world.workload, &alloc, &intent);
        let _token = world.runner.start_alloc(&spec);
        settle().await;
        if probes == ProbeSet::ReadinessAndLiveness {
            // The startup opt-out announces Stable at the first look; take
            // it now so every later look evaluates liveness (a Stable tick
            // skips liveness for that allocation).
            world.look(true).await.expect("first look announces Stable");
        }
        world
    }

    /// Write `alloc`'s `Running` row as the owning node writes it on start
    /// (precondition: the allocation exists and runs). `#[cfg(test)]` marks
    /// the fixture literal as a scenario precondition for the dst-lint
    /// crash-observability and logical-timestamp clauses (the module is
    /// already test-only through its `mod` declaration).
    #[cfg(test)]
    pub(super) async fn register_running(&mut self, alloc: &AllocationId) {
        self.row_counter += 1;
        let row = AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: self.workload.clone(),
            node_id: NodeId::new("probe-node").expect("node id"),
            state: AllocState::Running,
            updated_at: LogicalTimestamp {
                counter: self.row_counter,
                writer: NodeId::new("probe-node").expect("node id"),
            },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: vec![],
            started_at: Some(UnixInstant::from_unix_duration(self.clock.unix_now())),
            workload_addr: Some(WORKLOAD_ADDR),
            last_terminated: None,
            restart_count: 0,
        };
        self.state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write Running row");
    }

    /// The dial target of `role`'s probe.
    pub(super) const fn target_of(role: ProbeRole) -> SocketAddrV4 {
        let port = match role {
            ProbeRole::Startup => STARTUP_PORT,
            ProbeRole::Readiness => READINESS_PORT,
            ProbeRole::Liveness => LIVENESS_PORT,
        };
        SocketAddrV4::new(WORKLOAD_ADDR, port)
    }

    /// The roles this world declares.
    pub(super) fn roles(&self) -> Vec<ProbeRole> {
        match self.probes {
            ProbeSet::ReadinessAndLiveness => vec![ProbeRole::Readiness, ProbeRole::Liveness],
            ProbeSet::AllTcp | ProbeSet::HttpLiveness { .. } => {
                vec![ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness]
            }
        }
    }

    /// One probe period: every declared role's next dial does `outcome`
    /// (`per_role` overrides it for the named roles), then the probe tasks
    /// run once.
    pub(super) async fn run_with(
        &mut self,
        outcome: &RunOutcome,
        per_role: &[(ProbeRole, RunOutcome)],
    ) {
        let mut write_failures = 0;
        for role in self.roles() {
            let role_outcome =
                per_role.iter().find(|(r, _)| *r == role).map_or(outcome, |(_, o)| o);
            self.dial.script(Self::target_of(role), role_outcome.script(role, self.probes));
            if *role_outcome == RunOutcome::WriteFail {
                write_failures += 1;
            }
        }
        for _ in 0..write_failures {
            self.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
                "injected probe-row write failure",
            )));
        }
        self.clock.tick(PROBE_INTERVAL);
        settle().await;
    }

    /// One probe period with the same outcome for every declared role.
    pub(super) async fn run(&mut self, outcome: RunOutcome) {
        self.run_with(&outcome, &[]).await;
    }

    /// A look: one `service-lifecycle` convergence tick (hydrate →
    /// reconcile → view write-through → action dispatch, awaited). Before
    /// it, the `Driver` port is scripted so a liveness stop the look
    /// dispatches succeeds (`stop_ok`) or fails with an error other than
    /// `NotFound` (the allocation stays Running). A failed stop surfaces as
    /// the tick's `ShimError::Driver`; every other tick error is returned.
    pub(super) async fn look(&mut self, stop_ok: bool) -> Result<(), ConvergenceError> {
        self.driver.script_stop_failure((!stop_ok).then_some("scripted Driver::stop failure"));
        let name = self.service_lifecycle.clone();
        let result = self.tick(&name).await;
        self.driver.script_stop_failure(None);
        match result {
            Err(ConvergenceError::Shim(ShimError::Driver(_))) if !stop_ok => Ok(()),
            other => other,
        }
    }

    /// One `workload-lifecycle` convergence tick (requires
    /// [`Self::compose_with_workload_lifecycle`]).
    pub(super) async fn workload_tick(&mut self) -> Result<(), ConvergenceError> {
        let name = self.workload_lifecycle.clone();
        self.tick(&name).await
    }

    async fn tick(&mut self, name: &ReconcilerName) -> Result<(), ConvergenceError> {
        self.tick_n += 1;
        let now: Instant = self.clock.now();
        let result = run_convergence_tick(
            &self.state,
            name,
            &self.target,
            now,
            self.tick_n,
            now + Duration::from_secs(1),
        )
        .await;
        settle().await;
        result
    }

    /// The stored probe row of `alloc`'s `role` probe 0.
    pub(super) async fn row_of(
        &self,
        alloc: &AllocationId,
        role: ProbeRole,
    ) -> Option<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(alloc)
            .await
            .expect("list probe rows")
            .into_iter()
            .find(|row| row.role == role && row.probe_idx == ProbeIdx::new(0))
    }

    /// The stored probe row of this world's allocation's `role` probe 0.
    pub(super) async fn row(&self, role: ProbeRole) -> Option<ProbeResultRow> {
        self.row_of(&self.alloc, role).await
    }

    /// `ServiceLifecycleView::liveness_last_acted_run` of `alloc`'s liveness
    /// probe 0, through the runtime's pinned view accessor.
    pub(super) fn acted_run_of(&self, alloc: &AllocationId) -> Option<ProbeRunSeq> {
        self.state
            .runtime
            .loaded_service_lifecycle_views_for_test(&self.service_lifecycle)
            .and_then(|views| views.get(&self.target).cloned())
            .and_then(|view| {
                view.liveness_last_acted_run.get(&(alloc.clone(), ProbeIdx::new(0))).copied()
            })
    }

    /// As [`Self::acted_run_of`] for this world's allocation.
    pub(super) fn acted_run(&self) -> Option<ProbeRunSeq> {
        self.acted_run_of(&self.alloc)
    }

    /// The allocation's current row, if any.
    pub(super) async fn alloc_row(&self, alloc: &AllocationId) -> Option<AllocStatusRow> {
        self.obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .find(|row| row.alloc_id == *alloc)
    }

    /// The allocation still runs: its row carries no liveness terminal.
    pub(super) async fn running(&self) -> bool {
        self.alloc_row(&self.alloc).await.is_some_and(|row| {
            !matches!(
                row.terminal,
                Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe })
            )
        })
    }

    /// The `Driver::stop` calls the action shim made for `alloc`.
    pub(super) fn stop_calls_for(&self, alloc: &AllocationId) -> usize {
        self.driver.stop_calls().iter().filter(|a| *a == alloc).count()
    }

    /// Whether the readiness projection publishes this allocation as a
    /// healthy backend (`None`: no backend row lists it).
    pub(super) async fn backend_healthy(&self) -> Option<bool> {
        let service_id = self.service_id?;
        let spiffe = SpiffeId::for_allocation(&self.workload, &self.alloc);
        self.obs
            .service_backends_rows(&service_id)
            .await
            .expect("backend rows")
            .into_iter()
            .flat_map(|row| row.backends)
            .find(|backend| backend.alloc == spiffe)
            .map(|backend| backend.healthy)
    }

    /// The probe dials made with `DialPurpose::Probe` to `role`'s target.
    pub(super) fn probe_dials_of(&self, role: ProbeRole) -> usize {
        self.dial
            .dials_of(Self::target_of(role))
            .iter()
            .filter(|d| d.purpose == DialPurpose::Probe)
            .count()
    }

    /// Delete the workload's intent (the operator removes the workload).
    pub(super) async fn delete_intent(&self) {
        self.state
            .store
            .delete(IntentKey::for_workload(&self.workload).as_bytes())
            .await
            .expect("delete intent");
    }

    /// Persist the workload's intent again (the operator resubmits it).
    pub(super) async fn resubmit_intent(&self) {
        persist_intent(&self.state.store, &self.workload, &service_intent(self.probes)).await;
    }

    /// The Service intent this world declares.
    pub(super) fn intent(&self) -> WorkloadIntent {
        service_intent(self.probes)
    }
}

async fn persist_intent(store: &LocalIntentStore, workload: &WorkloadId, intent: &WorkloadIntent) {
    let archived = intent.archive_for_store().expect("rkyv archive");
    store
        .put(IntentKey::for_workload(workload).as_bytes(), archived.as_ref())
        .await
        .expect("put intent");
    store
        .put(
            IntentKey::for_workload_kind(workload).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("put kind");
}

// ---------------------------------------------------------------------
// `tracing` event capture — the `probe.not_sampled` oracle. The probe
// tasks run on the test's current-thread runtime, so a thread-default
// subscriber sees their events.
// ---------------------------------------------------------------------

/// One captured event: its name and its fields' debug renderings.
#[derive(Debug, Clone)]
pub(super) struct CapturedEvent {
    pub(super) name: String,
    pub(super) fields: BTreeMap<String, String>,
}

/// An in-memory `tracing` layer recording every event.
#[derive(Clone, Default)]
pub(super) struct EventCapture(Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

impl EventCapture {
    /// Install the capture as the thread-default subscriber.
    pub(super) fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }

    /// The captured `probe.not_sampled` events, in order.
    pub(super) fn not_sampled(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .expect("capture lock")
            .iter()
            .filter(|e| e.name == "probe.not_sampled")
            .cloned()
            .collect()
    }

    /// Forget every captured event.
    pub(super) fn clear(&self) {
        self.0.lock().expect("capture lock").clear();
    }
}

struct FieldRecorder<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldRecorder<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldRecorder(&mut fields));
        self.0
            .lock()
            .expect("capture lock")
            .push(CapturedEvent { name: event.metadata().name().to_owned(), fields });
    }
}
