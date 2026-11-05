//! The production composition the liveness-gate evidence drives (D9, B1,
//! G-6): the S-32 and S-43 acceptance tests and the `liveness_gate`
//! conformance driver.
//!
//! The production [`ReconcilerRuntime`] with `service-lifecycle` registered,
//! the production action shim (`run_convergence_tick`), and a production
//! [`VmDriver`] holding the real [`ProbeRunner`] as the node's `Driver`, so the
//! shim's `Stable` and terminal hooks reach the runner exactly as in `serve`.
//! Sim adapters sit at the driven ports only: `SimObservationStore`,
//! `SimClock`, `SimGuestStreamDial`, `SimVmm`. Preconditions are set up (the
//! Service intent, its VIP, the allocation's `Running` row and the driver's
//! `on_alloc_running` call, which the shim makes when it starts an
//! allocation); every probe row, `Stable` row and stop is authored by
//! production.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    dead_code,
    reason = "a test composition asserts its own preconditions; protocol names are prose; two consumers use subsets"
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::dial::GuestStreamDial;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
    TransitionSource,
};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::beacon::BeaconConnKey;
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::SimVmm;
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};
use tempfile::TempDir;

use super::probe_count_fixture::{
    PROBE_INTERVAL, ProbeSet, RunOutcome, WORKLOAD, WORKLOAD_ADDR, persist_intent, settle, vm_spec,
};
use super::{ConvergenceError, ReconcilerRuntime, run_convergence_tick};
use crate::AppState;
use crate::action_shim::build_alloc_status_row;

/// The liveness failure threshold of the shared fixture's Service.
const LIVENESS_FAILURE_THRESHOLD: u32 = super::probe_count_fixture::LIVENESS_FAILURE_THRESHOLD;

/// The fixture's one Service builder: the caller sets the startup descriptors, the
/// declared listeners and the liveness probe's `failure_threshold`; the readiness
/// probe and the rest are shared.
fn service_intent(
    startup: Vec<ProbeDescriptor>,
    listeners: Vec<ListenerInput>,
    threshold: u32,
    kernel: &str,
    rootfs: &str,
) -> WorkloadIntent {
    let tcp = super::probe_count_fixture::tcp;
    let mut liveness = tcp(ProbeRole::Liveness, super::probe_count_fixture::LIVENESS_PORT);
    liveness.failure_threshold = Some(threshold);
    let svc = Service::from_submit(ServiceSpecInput {
        id: WORKLOAD.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: kernel.to_owned(),
            rootfs: rootfs.to_owned(),
        }),
        listeners,
        startup_probes: startup,
        readiness_probes: vec![tcp(
            ProbeRole::Readiness,
            super::probe_count_fixture::READINESS_PORT,
        )],
        liveness_probes: vec![liveness],
    })
    .expect("valid probed service spec");
    WorkloadIntent::Service(svc)
}

fn tcp_listener(port: u16) -> ListenerInput {
    ListenerInput { port, protocol: "tcp".to_owned() }
}

/// The second port the guest-listener Service declares beside the fixture's
/// `LISTENER_PORT` (8080): the Service declares two TCP listeners.
pub(super) const SECOND_LISTENER_PORT: u16 = 9090;

/// The Service of a probed world: the startup probe the `ProbeSet` declares, one listener.
fn probed_intent(probes: ProbeSet, threshold: u32, kernel: &str, rootfs: &str) -> WorkloadIntent {
    let startup = match probes {
        ProbeSet::ReadinessAndLiveness => vec![],
        ProbeSet::AllTcp | ProbeSet::HttpLiveness { .. } => {
            vec![super::probe_count_fixture::tcp(
                ProbeRole::Startup,
                super::probe_count_fixture::STARTUP_PORT,
            )]
        }
    };
    let listeners = vec![tcp_listener(super::probe_count_fixture::LISTENER_PORT)];
    service_intent(startup, listeners, threshold, kernel, rootfs)
}

/// A Service with no explicit startup probe, as D15's default rule leaves it:
/// the sole startup descriptor is the platform's guest-listener gate
/// (interval 2 s, 30 attempts: the 60 s startup deadline), over two declared
/// TCP listeners.
fn guest_listener_intent(kernel: &str, rootfs: &str) -> WorkloadIntent {
    let gate = ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::GuestListeners,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: true,
    };
    let listeners = vec![
        tcp_listener(super::probe_count_fixture::LISTENER_PORT),
        tcp_listener(SECOND_LISTENER_PORT),
    ];
    service_intent(vec![gate], listeners, LIVENESS_FAILURE_THRESHOLD, kernel, rootfs)
}

/// What a world composes: the probed Service (started at once, or unstarted), the
/// restartable one beside `workload-lifecycle`, or the restartable Service whose
/// startup gate is the guest-listener gate.
#[derive(Debug, Clone, Copy)]
enum WorldShape {
    Probed(ProbeSet),
    Restartable(ProbeSet),
    RestartableGuestListeners,
}

impl WorldShape {
    /// The probe set the dial helpers read (unused by the guest-listener world).
    const fn probes(self) -> ProbeSet {
        match self {
            Self::Probed(p) | Self::Restartable(p) => p,
            Self::RestartableGuestListeners => ProbeSet::ReadinessAndLiveness,
        }
    }

    const fn restartable(self) -> bool {
        !matches!(self, Self::Probed(_))
    }
}

/// The composed production owners and the driven-port sims.
pub(super) struct LivenessWorld {
    pub(super) state: AppState,
    pub(super) obs: Arc<SimObservationStore>,
    pub(super) clock: Arc<SimClock>,
    pub(super) dial: Arc<SimGuestStreamDial>,
    pub(super) runner: Arc<ProbeRunner>,
    pub(super) driver: Arc<VmDriver>,
    pub(super) probes: ProbeSet,
    pub(super) workload: WorkloadId,
    pub(super) alloc: AllocationId,
    pub(super) target: TargetResource,
    pub(super) intent: WorkloadIntent,
    pub(super) service_id: ServiceId,
    /// The simulated machine the real `VmDriver` boots VMMs on.
    pub(super) vmm: SimVmm,
    /// The node beacon the guests' READY lines arrive on.
    pub(super) beacon: Arc<SimBeaconIo>,
    /// The workload address of the current allocation (the probes' dial target).
    pub(super) addr: Ipv4Addr,
    service_lifecycle: ReconcilerName,
    workload_lifecycle: ReconcilerName,
    beacon_port: VsockPort,
    next_beacon_port: u32,
    /// The beacon connection of the most recently placed instance's guest.
    beacon_key: Option<BeaconConnKey>,
    /// Every allocation the node has placed, in placement order.
    placed: Vec<AllocationId>,
    _exit_observer: Option<tokio::task::JoinHandle<()>>,
    tick_n: u64,
    row_counter: u64,
    tmp: TempDir,
}

impl LivenessWorld {
    /// Compose the world: persist the intent, allocate its VIP, write the
    /// allocation's `Running` row and make the driver's `on_alloc_running`
    /// call (supervisor registered, liveness retained). No look is taken.
    pub(super) async fn compose(probes: ProbeSet) -> Self {
        let mut world = Self::compose_unstarted(probes, LIVENESS_FAILURE_THRESHOLD).await;
        world.start_allocation().await;
        world
    }

    /// The world before the allocation runs: the intent, VIP and owners are
    /// composed; no row exists and no supervisor is registered. `liveness_threshold`
    /// is the liveness probe's `failure_threshold`.
    pub(super) async fn compose_unstarted(probes: ProbeSet, liveness_threshold: u32) -> Self {
        Self::build(WorldShape::Probed(probes), liveness_threshold).await
    }

    /// The node's production composition with `workload-lifecycle` registered
    /// beside `service-lifecycle` (the single restart authority, ADR-0087) and
    /// the production exit observer running. Nothing is started: the
    /// allocation is placed, and later restarted, by
    /// [`Self::place_instance`], which runs the real `VmDriver::start` over
    /// `SimVmm` and a `SimBeaconIo` (the `quint_cid_lease` composition).
    pub(super) async fn compose_restartable(probes: ProbeSet) -> Self {
        Self::build(WorldShape::Restartable(probes), LIVENESS_FAILURE_THRESHOLD).await
    }

    /// [`Self::compose_restartable`] for a Service with no explicit startup
    /// probe: its startup gate is the platform's guest-listener gate over two
    /// declared TCP listeners (8080 and [`SECOND_LISTENER_PORT`]). The
    /// `ProbeSet` fields the dial helpers read are unused by this world.
    pub(super) async fn compose_restartable_guest_listeners() -> Self {
        Self::build(WorldShape::RestartableGuestListeners, LIVENESS_FAILURE_THRESHOLD).await
    }

    async fn build(shape: WorldShape, liveness_threshold: u32) -> Self {
        let (probes, restartable) = (shape.probes(), shape.restartable());
        let tmp = TempDir::new().expect("tempdir");
        let kernel = tmp.path().join("vmlinuz");
        let rootfs = tmp.path().join("master.img");
        // The staged boot artifacts the real `VmDriver::start` reads (as
        // `quint_cid_lease` stages them).
        tokio::fs::write(&rootfs, b"deterministic-fixture-rootfs-bytes")
            .await
            .expect("stage rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        tokio::fs::write(&kernel, &header).await.expect("stage the synthetic kernel image");

        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
            .expect("reconciler runtime");
        runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");
        if restartable {
            runtime
                .register(crate::workload_lifecycle())
                .await
                .expect("register workload-lifecycle");
        }

        let node = NodeId::new(if restartable { "local" } else { "probe-node" }).expect("node id");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let workload = WorkloadId::new(WORKLOAD).expect("workload id");
        let (kernel_path, rootfs_path) =
            (kernel.display().to_string(), rootfs.display().to_string());
        let intent = match shape {
            WorldShape::RestartableGuestListeners => {
                guest_listener_intent(&kernel_path, &rootfs_path)
            }
            WorldShape::Probed(_) | WorldShape::Restartable(_) => {
                probed_intent(probes, liveness_threshold, &kernel_path, &rootfs_path)
            }
        };
        persist_intent(&store, &workload, &intent).await;

        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let clock = Arc::new(SimClock::new());
        let dial = Arc::new(SimGuestStreamDial::new());
        // One board for the node (DG-6): the driver's beacon reader writes it
        // and the probe runner reads it.
        let board = Arc::new(overdrive_worker::GuestListenerBoard::new());
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
            Arc::clone(&board) as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(
            Arc::clone(&clock) as Arc<dyn Clock>
        );
        assert!(wiring.supervisor().open_after_boot());
        let vmm = SimVmm::new();
        let beacon = Arc::new(SimBeaconIo::new(vmm.clone()));
        let beacon_intake = BeaconIntake::bind(
            Arc::clone(&beacon) as Arc<dyn overdrive_core::vm::beacon::BeaconIo>,
            overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
        )
        .await
        .expect("bind the node beacon intake");
        let beacon_port = beacon_intake.bound_port();
        let driver = Arc::new(VmDriver::new(
            Arc::new(vmm.clone()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(overdrive_sim::SimCgroupFs::new()),
            Arc::new(overdrive_sim::SimCgroupAccounting::new()),
            Arc::clone(&runner),
            wiring.gate(),
            VmHostLayout {
                cgroup_root: tmp.path().join("cgroup"),
                run_dir_root: tmp.path().join("run"),
                clone_index_dir: tmp.path().join("clone-index"),
                clone_staging_dir: tmp.path().join("clone-staging"),
                arch: HostArch::X86_64,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: Vec::new() },
                    1024,
                ),
            },
            beacon_intake,
            Arc::clone(&board),
        ));

        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mtls_worker = crate::shared_network_test_ports::mtls_worker();
        mtls_worker.start_shared_owner().await.expect("shared mTLS owner starts over sim adapters");
        let guest_owner = crate::shared_network_test_ports::shared_guest_network_owner();
        crate::guest_network::SharedGuestNetworkOwner::converge_shared(guest_owner.as_ref())
            .await
            .expect("shared owner reports its bound flow ports");
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
            mtls_worker,
            guest_owner,
            wiring.gate(),
            crate::shared_network_test_ports::guest_pool(),
        );
        let service_id = {
            let digest = intent.spec_digest().expect("spec digest");
            let mut guard = state.allocator.lock().await;
            let vip: ServiceVip = guard.allocate(*digest.as_bytes()).await.expect("allocate vip");
            drop(guard);
            ServiceId::derive(
                &vip,
                NonZeroU16::new(super::probe_count_fixture::LISTENER_PORT).expect("port"),
                Proto::Tcp,
                "service-map",
            )
        };

        let exit_observer = restartable.then(|| {
            crate::worker::exit_observer::spawn(
                Arc::clone(&obs) as Arc<dyn ObservationStore>,
                Arc::clone(&driver) as Arc<dyn Driver>,
                Arc::clone(&state.lifecycle_events),
                Arc::clone(&clock) as Arc<dyn Clock>,
            )
        });
        let alloc = AllocationId::new(&format!("{WORKLOAD}-0")).expect("alloc id");
        Self {
            state,
            obs,
            clock,
            dial,
            runner,
            driver,
            probes,
            workload,
            alloc,
            target: TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target"),
            intent,
            service_id,
            vmm,
            beacon,
            addr: WORKLOAD_ADDR,
            service_lifecycle: ReconcilerName::new("service-lifecycle").expect("name"),
            workload_lifecycle: ReconcilerName::new("workload-lifecycle").expect("name"),
            beacon_port,
            next_beacon_port: 50_000,
            beacon_key: None,
            placed: Vec::new(),
            _exit_observer: exit_observer,
            tick_n: 0,
            row_counter: 0,
            tmp,
        }
    }

    /// The node starts the allocation: its `Running` row, the driver's
    /// `on_alloc_running` call (the shim makes it on start) and the
    /// alloc-to-driver index entry.
    pub(super) async fn start_allocation(&mut self) {
        let alloc = self.alloc.clone();
        self.register_running(&alloc).await;
        let spec = vm_spec(&self.workload, &alloc, &self.intent);
        self.driver.on_alloc_running(&spec);
        self.state.alloc_drivers.lock().insert(alloc, DriverType::Vm);
        settle().await;
    }

    /// The guest crashes: the row a crashed allocation carries (`Failed`,
    /// no terminal claim), as the exit observer writes it.
    pub(super) async fn crash(&self) {
        let mut row = self.alloc_row().await.expect("a crash needs a row");
        row.state = AllocState::Failed;
        row.terminal = None;
        row.updated_at =
            LogicalTimestamp::dominating(0, row.updated_at.writer.clone(), Some(&row.updated_at));
        self.obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write the crashed row");
    }

    /// A look with the node's driver registry emptied: the shim's row write
    /// happens but fires no driver hook (a scenario that applies the hook as
    /// its own step).
    pub(super) async fn look_without_hooks(&mut self) -> Result<(), ConvergenceError> {
        let saved = std::mem::replace(
            &mut self.state.drivers,
            Arc::new(overdrive_core::traits::driver::DriverRegistry::new()),
        );
        let result = self.look().await;
        self.state.drivers = saved;
        result
    }

    async fn register_running(&mut self, alloc: &AllocationId) {
        self.row_counter += 1;
        let writer = NodeId::new("probe-node").expect("node id");
        let row = build_alloc_status_row(
            alloc.clone(),
            self.workload.clone(),
            writer.clone(),
            AllocState::Running,
            LogicalTimestamp::dominating(self.row_counter - 1, writer, None),
            None,
            None,
            None,
            None,
            WorkloadKind::Service,
            Some(UnixInstant::from_unix_duration(self.clock.unix_now())),
            Some(WORKLOAD_ADDR),
            None,
        );
        self.obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write Running row");
    }

    /// One probe period: every declared role's next dial does `outcome`
    /// (`per_role` overrides), then the probe tasks run once.
    pub(super) async fn period(&self, outcome: &RunOutcome, per_role: &[(ProbeRole, RunOutcome)]) {
        for role in [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness] {
            let o = per_role.iter().find(|(r, _)| *r == role).map_or(outcome, |(_, o)| o);
            let port = match role {
                ProbeRole::Startup => super::probe_count_fixture::STARTUP_PORT,
                ProbeRole::Readiness => super::probe_count_fixture::READINESS_PORT,
                ProbeRole::Liveness => super::probe_count_fixture::LIVENESS_PORT,
            };
            self.dial.script(SocketAddrV4::new(self.addr, port), o.script(role, self.probes));
        }
        self.clock.tick(PROBE_INTERVAL);
        settle().await;
    }

    /// One `service-lifecycle` convergence tick (hydrate, reconcile, view
    /// write-through, action dispatch, awaited).
    pub(super) async fn look(&mut self) -> Result<(), ConvergenceError> {
        self.tick_n += 1;
        let now: Instant = self.clock.now();
        let result = run_convergence_tick(
            &self.state,
            &self.service_lifecycle,
            &self.target,
            now,
            self.tick_n,
            now + Duration::from_secs(1),
        )
        .await;
        settle().await;
        result
    }

    /// The `SimClock` step before each restart-authority tick: past every
    /// restart backoff, so the backoff never defers a placement the scenario
    /// takes in that step (as `quint_cid_lease` steps it).
    const PAST_BACKOFF: Duration = Duration::from_secs(3_600);

    /// One `workload-lifecycle` convergence tick (the single restart
    /// authority), after the clock has passed the restart backoff. The
    /// dispatch is awaited; a start waits for the guest's READY, so a tick
    /// that places an allocation is driven by [`Self::place_instance`].
    pub(super) async fn restart_authority_look(&mut self) -> Result<(), ConvergenceError> {
        self.clock.tick(Self::PAST_BACKOFF);
        self.tick_n += 1;
        let now: Instant = self.clock.now();
        let result = run_convergence_tick(
            &self.state,
            &self.workload_lifecycle,
            &self.target,
            now,
            self.tick_n,
            now + Duration::from_secs(1),
        )
        .await;
        settle().await;
        result
    }

    /// The restart authority places the workload's next allocation: a
    /// `workload-lifecycle` tick whose dispatch leases a guest address and
    /// CID and runs the production `VmDriver::start` over `SimVmm`; the
    /// guest's READY (the launch token the lease carries, on the beacon the
    /// intake bound) is delivered as stimulus once the VMM holds its CID's
    /// device. Returns the new allocation, which becomes the world's current
    /// one; the driver's `on_alloc_running` was called by the shim.
    pub(super) async fn place_instance(&mut self) -> AllocationId {
        const SETTLE_BOUND: Duration = Duration::from_secs(10);
        self.clock.tick(Self::PAST_BACKOFF);
        self.tick_n += 1;
        let tick_n = self.tick_n;
        let state = self.state.clone();
        let name = self.workload_lifecycle.clone();
        let target = self.target.clone();
        let now: Instant = self.clock.now();
        let tick = tokio::spawn(async move {
            run_convergence_tick(&state, &name, &target, now, tick_n, now + Duration::from_secs(1))
                .await
        });

        let deadline = std::time::Instant::now() + SETTLE_BOUND;
        let (alloc, cid, launch, pid) = loop {
            let leases = self.state.guest_pool.lease_observation().leases;
            let fresh = leases.iter().find(|(alloc, _)| !self.placed.contains(alloc));
            if let Some((alloc, lease)) = fresh
                && let Some(pid) = self.vmm.vsock_device_holders().get(&lease.cid).copied()
            {
                break (alloc.clone(), lease.cid, lease.launch, pid);
            }
            assert!(
                !tick.is_finished(),
                "the restart authority's tick ended without placing an allocation: {:?}",
                tick.await
            );
            assert!(std::time::Instant::now() < deadline, "no VMM opened a fresh lease's device");
            tokio::time::sleep(Duration::from_millis(1)).await;
        };

        self.next_beacon_port += 1;
        let key = BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(self.next_beacon_port) };
        let ready = format!("READY pid=1 port={} launch={launch}", self.beacon_port);
        self.beacon.queue_connection(key, pid, &[ready.as_str()]).expect("the guest's beacon");
        self.beacon_key = Some(key);
        tokio::time::timeout(SETTLE_BOUND, tick)
            .await
            .expect("the placement tick returns once the guest is ready")
            .expect("the placement tick does not panic")
            .expect("the placement tick succeeds");
        settle().await;

        self.placed.push(alloc.clone());
        self.alloc = alloc.clone();
        self.addr = self
            .alloc_row()
            .await
            .and_then(|row| row.workload_addr)
            .expect("the placed allocation's row carries its workload address");
        alloc
    }

    /// The current instance's guest writes one line on its beacon connection
    /// (a `LISTEN` report). A report the guest never writes is the omitted
    /// call: the host sees nothing. `false` when the connection is gone.
    pub(super) fn guest_writes(&self, line: &str) -> bool {
        let key = self.beacon_key.expect("an instance is placed before its guest writes");
        self.beacon.guest_write(key, line)
    }

    /// Every allocation the node has placed, oldest first.
    pub(super) fn placed(&self) -> &[AllocationId] {
        &self.placed
    }

    /// Script the next observation write to fail once.
    pub(super) fn fail_next_write(&self) {
        self.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected observation write failure",
        )));
    }

    /// A control-plane restart: the runtime is dropped and a new one loads the
    /// persisted views from the same files.
    pub(super) async fn restart_control_plane(&mut self) {
        let parked = TempDir::new().expect("tempdir");
        let placeholder = Arc::new(
            ReconcilerRuntime::new_with_redb_view_store_for_test(parked.path())
                .expect("placeholder runtime"),
        );
        drop(std::mem::replace(&mut self.state.runtime, placeholder));
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(self.tmp.path())
            .expect("reloaded runtime");
        runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");
        self.state.runtime = Arc::new(runtime);
    }

    pub(super) async fn alloc_row(&self) -> Option<AllocStatusRow> {
        self.obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .find(|row| row.alloc_id == self.alloc)
    }

    /// The stored probe rows of the current allocation for `role`.
    pub(super) async fn rows_for(&self, role: ProbeRole) -> Vec<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(&self.alloc)
            .await
            .expect("list probe rows")
            .into_iter()
            .filter(|r| r.role == role)
            .collect()
    }

    pub(super) fn liveness_live(&self) -> bool {
        self.runner.is_role_live(&self.alloc, ProbeRole::Liveness)
    }
}

/// Whether a row's terminal is the committed `Stable`.
pub(super) fn is_stable(terminal: Option<&TerminalCondition>) -> bool {
    matches!(terminal, Some(TerminalCondition::Stable { .. }))
}

/// The budget is spent: the restart authority gives the Service up. Takes the give-up
/// tick, hands the row's terminal to `expect` (the cause-specific oracle), takes a later
/// tick and asserts the give-up stays final (`is_cause`), that no allocation was placed
/// after the `total` already placed, and that nothing is supervised.
pub(super) async fn run_to_give_up(
    w: &mut LivenessWorld,
    tag: &str,
    total: u64,
    expect: impl Fn(Option<&TerminalCondition>),
    is_cause: impl Fn(Option<&TerminalCondition>) -> bool,
) {
    w.restart_authority_look().await.expect("the give-up tick");
    let row = w.alloc_row().await.expect("row");
    expect(row.terminal.as_ref());
    w.restart_authority_look().await.expect("a later tick");
    let row = w.alloc_row().await.expect("row");
    assert!(
        is_cause(row.terminal.as_ref()),
        "{tag}: the give-up stays final, got {:?}",
        row.terminal
    );
    assert_eq!(w.placed().len() as u64, total, "{tag}: no allocation after the give-up");
    assert_eq!(w.runner.active_alloc_count(), 0, "{tag}: nothing is supervised after the give-up");
}
