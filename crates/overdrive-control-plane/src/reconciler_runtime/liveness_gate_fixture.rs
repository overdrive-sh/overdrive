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

use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{WorkloadIntent, WorkloadKind};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::dial::GuestStreamDial;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::observation::{ProbeResultRow, ProbeRole};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
    TransitionSource,
};
use overdrive_core::vm::config::{Gid, HostArch, VmConfinement, VmmIdentity};
use overdrive_core::wall_clock::UnixInstant;
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

/// The fixture's Service with the liveness probe's `failure_threshold` set.
fn intent_with_threshold(probes: ProbeSet, threshold: u32) -> WorkloadIntent {
    use overdrive_core::aggregate::{DriverInput, ResourcesInput, Service, VmInput};
    use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
    let tcp = super::probe_count_fixture::tcp;
    let startup = match probes {
        ProbeSet::ReadinessAndLiveness => vec![],
        ProbeSet::AllTcp | ProbeSet::HttpLiveness { .. } => {
            vec![tcp(ProbeRole::Startup, super::probe_count_fixture::STARTUP_PORT)]
        }
    };
    let mut liveness = tcp(ProbeRole::Liveness, super::probe_count_fixture::LIVENESS_PORT);
    liveness.failure_threshold = Some(threshold);
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
        listeners: vec![ListenerInput {
            port: super::probe_count_fixture::LISTENER_PORT,
            protocol: "tcp".to_owned(),
        }],
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
    service_lifecycle: ReconcilerName,
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
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
            .expect("reconciler runtime");
        runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");

        let node = NodeId::new("probe-node").expect("node id");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let workload = WorkloadId::new(WORKLOAD).expect("workload id");
        let intent = intent_with_threshold(probes, liveness_threshold);
        persist_intent(&store, &workload, &intent).await;

        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let clock = Arc::new(SimClock::new());
        let dial = Arc::new(SimGuestStreamDial::new());
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
            Arc::new(overdrive_worker::GuestListenerBoard::new()),
        ));
        let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(
            Arc::clone(&clock) as Arc<dyn Clock>,
        );
        assert!(wiring.supervisor().open_after_boot());
        let vmm = overdrive_sim::SimVmm::new();
        let beacon = BeaconIntake::bind(
            Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(vmm.clone())),
            overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
        )
        .await
        .expect("bind the node beacon intake");
        let driver = Arc::new(VmDriver::new(
            Arc::new(vmm),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(overdrive_sim::SimCgroupFs::new()),
            Arc::new(overdrive_sim::SimCgroupAccounting::new()),
            Arc::clone(&runner),
            wiring.gate(),
            VmHostLayout {
                cgroup_root: PathBuf::from("/tmp/liveness-world-cgroup"),
                run_dir_root: PathBuf::from("/tmp/liveness-world-run"),
                clone_index_dir: PathBuf::from("/tmp/liveness-world-index"),
                clone_staging_dir: PathBuf::from("/tmp/liveness-world-staging"),
                arch: HostArch::X86_64,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: Vec::new() },
                    1024,
                ),
            },
            beacon,
            Arc::new(overdrive_worker::GuestListenerBoard::new()),
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
            node,
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
            service_lifecycle: ReconcilerName::new("service-lifecycle").expect("name"),
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
            self.dial.script(
                super::probe_count_fixture::ProbeWorld::target_of(role),
                o.script(role, self.probes),
            );
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

    pub(super) async fn liveness_rows(&self) -> Vec<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(&self.alloc)
            .await
            .expect("list probe rows")
            .into_iter()
            .filter(|r| r.role == ProbeRole::Liveness)
            .collect()
    }

    pub(super) fn liveness_live(&self) -> bool {
        self.runner.is_role_live(&self.alloc, ProbeRole::Liveness)
    }
}
