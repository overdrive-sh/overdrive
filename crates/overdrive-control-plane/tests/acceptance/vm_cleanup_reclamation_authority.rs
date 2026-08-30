//! Step 02-05 D12 — failed-start cleanup and VM reclamation share exactly one
//! allocation-scoped teardown authority.
//!
//! This is a production-composition test, not a driver-only or planner-only
//! substitute: a real [`VmDriver`] creates indexed filesystem residue and
//! retains its supervision claim, the production action shim persists the
//! disposition, and the periodic/boot reclamation hydration reads that same
//! row, host tree, and live driver claim.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::action_shim::{
    WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
    run_convergence_tick_with_network_provisioner_for_test,
};
use overdrive_control_plane::veth_provisioner::{VethProvisionError, VmTapPlan, WorkloadNetnsPlan};
use overdrive_control_plane::{AppState, vm_reclamation_boot};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::traits::vm_host_state::{
    VmHostObservation, VmHostState, VmHostStateProbeError,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmError, VmmProbeError,
};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_core::{SpiffeId, TransitionReason};
use overdrive_host::vm_host_state::RealVmHostState;
use overdrive_reconcilers::vm_reclamation::plan_reclamation;
use overdrive_reconcilers::{AnyReconciler, AnyState, VmReclamation, WorkloadLifecycle};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::{VmDriver, VmHostLayout};
use tempfile::TempDir;
use tokio::sync::Notify;

#[derive(Debug, Default)]
struct RecordingNetworkProvisioner;

impl WorkloadNetworkProvisioner for RecordingNetworkProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        assert!(vm_tap.is_some(), "the Job is VM-driven and must receive the VM tap plan");
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        Ok(())
    }
}

/// A deterministic substrate partition at the real rootfs cleanup boundary:
/// `Vmm::create` leaves a directory at the path that is contractually a clone
/// file, then rejects creation. `VmDriver`'s production `remove_file` therefore
/// returns `EISDIR`, retains the index and claim, and exposes a real host tree
/// to `RealVmHostState`.
struct CloneCleanupPartitionVmm {
    creates: AtomicUsize,
}

/// Deterministic execution-time interleaving seam. The real host adapter is
/// still responsible for host mutation; this wrapper pauses after the
/// reclamation executor has acquired its `VmDriver` lease and before its first
/// kill so a `StartAllocation` can contend for the same allocation.
struct KillBarrierHost {
    inner: Arc<dyn VmHostState>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

struct KillBarrierControl {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl KillBarrierHost {
    fn wrap(inner: Arc<dyn VmHostState>) -> (Self, KillBarrierControl) {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        (
            Self { inner, entered: Arc::clone(&entered), release: Arc::clone(&release) },
            KillBarrierControl { entered, release },
        )
    }
}

impl KillBarrierControl {
    async fn wait_until_kill_entered(&self) {
        self.entered.notified().await;
    }

    fn release_kill(&self) {
        self.release.notify_one();
    }
}

#[async_trait]
impl VmHostState for KillBarrierHost {
    fn kind(&self) -> &'static str {
        "test::KillBarrierHost<RealVmHostState>"
    }

    async fn probe(&self) -> Result<(), VmHostStateProbeError> {
        self.inner.probe().await
    }

    async fn observe(&self) -> std::io::Result<VmHostObservation> {
        self.inner.observe().await
    }

    async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.kill_scope(scope).await
    }

    async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
        self.inner.discard_artifacts(alloc).await
    }
}

#[async_trait]
impl Vmm for CloneCleanupPartitionVmm {
    fn kind(&self) -> &'static str {
        "clone-cleanup-partition"
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        Ok(())
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        std::fs::create_dir_all(config.rootfs.clone_dest()).map_err(|error| VmmError::Create {
            detail: format!("stage clone cleanup partition: {error}"),
        })?;
        Err(VmmError::Create { detail: "injected VMM create rejection".to_owned() })
    }

    async fn terminate(&self, _control: &VmControl, _grace: Duration) -> VmmResult<VmTermination> {
        Ok(VmTermination::Killed)
    }
}

struct Harness {
    state: AppState,
    clock: Arc<SimClock>,
    driver: Arc<VmDriver>,
    vmm: Arc<CloneCleanupPartitionVmm>,
    layout: VmHostLayout,
    alloc: AllocationId,
    workload: WorkloadId,
    spec: AllocationSpec,
    rootfs: RootfsPlan,
    target: TargetResource,
}

fn stage_artifacts(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let kernel = tmp.path().join("vmlinuz");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&kernel, header).expect("stage fixture kernel");
    let rootfs = tmp.path().join("master.img");
    std::fs::write(&rootfs, b"fixture-rootfs").expect("stage fixture rootfs");
    (kernel, rootfs)
}

fn make_driver(layout: VmHostLayout) -> (Arc<VmDriver>, Arc<CloneCleanupPartitionVmm>) {
    let vmm = Arc::new(CloneCleanupPartitionVmm { creates: AtomicUsize::new(0) });
    let driver = Arc::new(VmDriver::new(
        vmm.clone(),
        Arc::new(SimClock::new()),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        layout,
    ));
    (driver, vmm)
}

#[expect(
    clippy::too_many_lines,
    reason = "the production composition root is intentionally explicit so the test does not \
              replace a real adapter or lifecycle surface with a narrower fake"
)]
async fn build_harness(tmp: &TempDir, suffix: &str) -> Harness {
    let (kernel, master) = stage_artifacts(tmp);
    let layout = VmHostLayout {
        cgroup_root: tmp.path().join("cgroup"),
        run_dir_root: tmp.path().join("run"),
        clone_index_dir: tmp.path().join("clone-index"),
        clone_staging_dir: tmp.path().join("clone-staging"),
        arch: HostArch::X86_64,
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: Vec::new() },
            1024,
        ),
    };
    let workload =
        WorkloadId::new(&format!("cleanup-reclaim-{suffix}")).expect("valid workload id");
    let alloc = AllocationId::new(&format!("alloc-{workload}-0")).expect("valid allocation id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/{workload}/alloc/{alloc}"
        ))
        .expect("valid workload identity"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: kernel.clone(),
            rootfs: master.clone(),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    };
    let rootfs = RootfsPlan::for_alloc(
        master.clone(),
        std::fs::metadata(&master).expect("rootfs metadata").len(),
        &alloc,
        &layout.clone_staging_dir,
        &layout.clone_index_dir,
    );
    let (driver, vmm) = make_driver(layout.clone());
    let driver_dyn: Arc<dyn Driver> = driver.clone();
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let node = NodeId::new("local").expect("valid node id");
    let sim_obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs;
    let clock = Arc::new(SimClock::new());
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(&tmp.path().join("views"))
            .expect("runtime");
    runtime
        .register(AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical()))
        .await
        .expect("register workload lifecycle");
    let runtime = Arc::new(runtime);
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let mut state = AppState::new(
        Arc::clone(&store),
        store_path,
        obs,
        runtime,
        driver_dyn,
        clock_dyn,
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    state.vm_host_state = Arc::new(RealVmHostState::new(
        layout.cgroup_root.clone(),
        layout.run_dir_root.clone(),
        layout.clone_index_dir.clone(),
    ));

    let job = Job::from_submit(JobSpecInput {
        id: workload.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: kernel.display().to_string(),
            rootfs: master.display().to_string(),
        }),
    })
    .expect("valid VM job intent");
    let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive VM job intent");
    store
        .put(IntentKey::for_workload(&workload).as_bytes(), archived.as_ref())
        .await
        .expect("persist VM job intent");
    store
        .put(
            IntentKey::for_workload_kind(&workload).as_bytes(),
            &[WorkloadKind::Job.discriminator_byte()],
        )
        .await
        .expect("persist VM job kind");

    let target =
        TargetResource::new(&format!("workload/{workload}")).expect("valid workload target");
    Harness { state, clock, driver, vmm, layout, alloc, workload, spec, rootfs, target }
}

async fn run_workload_tick(h: &Harness, tick_n: u64) {
    let now = h.clock.now();
    run_convergence_tick_with_network_provisioner_for_test(
        &h.state,
        &ReconcilerName::new("workload-lifecycle").expect("valid reconciler name"),
        &h.target,
        now,
        tick_n,
        now + Duration::from_secs(60),
        &RecordingNetworkProvisioner,
    )
    .await
    .expect("production workload lifecycle tick");
}

fn lifecycle_view(h: &Harness) -> overdrive_reconcilers::WorkloadLifecycleView {
    h.state
        .runtime
        .loaded_workload_lifecycle_views_for_test(
            &ReconcilerName::new("workload-lifecycle").expect("valid reconciler name"),
        )
        .expect("registered workload lifecycle view map")
        .get(&h.target)
        .cloned()
        .unwrap_or_default()
}

async fn dispatch_action(h: &Harness, action: Action, tick_n: u64) {
    let now = Instant::now();
    dispatch_with_network_provisioner(
        vec![action],
        h.state.drivers.as_ref(),
        h.state.alloc_drivers.as_ref(),
        h.state.obs.as_ref(),
        h.state.dataplane.as_ref(),
        h.state.ca.as_ref(),
        h.state.clock.as_ref(),
        h.state.identity.as_ref(),
        h.state.lifecycle_events.as_ref(),
        &TickContext {
            now,
            now_unix: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
                1_700_000_000 + tick_n,
            )),
            tick: tick_n,
            deadline: now + Duration::from_secs(1),
        },
        &h.state.node_id,
        Arc::clone(&h.state.allocator),
        h.state.runtime.broker_mutex(),
        None,
        None,
        &h.state.net_slot_allocator,
        &RecordingNetworkProvisioner,
        h.state.vm_host_state.as_ref(),
    )
    .await
    .expect("cleanup disposition is handled by the action path");
}

fn start_action(h: &Harness) -> Action {
    Action::StartAllocation {
        alloc_id: h.alloc.clone(),
        workload_id: h.workload.clone(),
        node_id: h.state.node_id.clone(),
        spec: h.spec.clone(),
        kind: WorkloadKind::Job,
    }
}

fn pending_row(h: &Harness, detail: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: h.alloc.clone(),
        workload_id: h.workload.clone(),
        node_id: h.state.node_id.clone(),
        state: AllocState::Pending,
        updated_at: LogicalTimestamp { counter: 1, writer: h.state.node_id.clone() },
        reason: Some(TransitionReason::DriverInternalError { detail: detail.to_owned() }),
        detail: Some(detail.to_owned()),
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

async fn reclamation_states(
    h: &Harness,
) -> (overdrive_reconcilers::VmReclamationState, overdrive_reconcilers::VmReclamationState) {
    let reconciler = AnyReconciler::VmReclamation(VmReclamation::new());
    let target = TargetResource::new(&format!("node/{}", h.state.node_id)).expect("node target");
    let desired = hydrate_desired_for_test(&reconciler, &target, &h.state)
        .await
        .expect("hydrate reclamation desired");
    let actual = hydrate_actual_for_test(&reconciler, &target, &h.state)
        .await
        .expect("hydrate reclamation actual");
    let AnyState::VmReclamation(desired) = desired else {
        panic!("expected VmReclamation desired state");
    };
    let AnyState::VmReclamation(actual) = actual else {
        panic!("expected VmReclamation actual state");
    };
    (desired, actual)
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is exact; the single production-composition journey keeps allocation, cleanup, reclamation, and finalization evidence contiguous"
)]
#[tokio::test]
async fn incomplete_cleanup_is_nonterminal_and_gates_periodic_and_boot_reclamation_until_recovered()
{
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "live").await;
    run_workload_tick(&h, 1).await;

    let pending = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read cleanup row")
        .expect("incomplete cleanup has an explicit retryable row");
    assert_eq!(
        pending.state,
        AllocState::Pending,
        "incomplete cleanup must not publish terminal Failed",
    );
    let detail = pending.detail.clone().expect("cleanup detail remains durable");
    assert!(detail.contains("RootfsCloneRemove"));
    assert!(detail.contains("injected VMM create rejection"));
    assert_eq!(h.driver.live_allocations(), Some(vec![h.alloc.clone()]));
    assert!(h.rootfs.clone_dest().is_dir());
    assert!(h.rootfs.index_link().is_symlink());

    let (desired, actual) = reclamation_states(&h).await;
    assert!(!desired.allocations[&h.alloc].terminal);
    assert!(actual.host.clones.contains_key(&h.alloc));
    assert!(matches!(
        &actual.supervision,
        overdrive_reconcilers::vm_reclamation::SupervisionSet::Observed(held)
            if held.contains(&h.alloc)
    ));
    assert_eq!(
        plan_reclamation(&desired, &actual),
        Vec::new(),
        "the periodic planner cannot admit a second cleanup owner",
    );

    // A plan is only a snapshot. Exercise both real kill-capable action arms
    // directly as if a prior tick had planned them before the failed-start
    // owner appeared. Their execution-time lease must re-check the real
    // VmDriver claim and make both stale actions total no-ops.
    dispatch_action(&h, Action::DiscardStrandedArtifacts { alloc_id: h.alloc.clone() }, 2).await;
    dispatch_action(&h, Action::ReclaimAllocation { alloc_id: h.alloc.clone() }, 3).await;
    let still_pending = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read row after stale reclamation actions")
        .expect("pending row survives stale reclamation actions");
    assert_eq!(still_pending, pending);
    assert!(h.rootfs.clone_dest().is_dir());
    assert!(h.rootfs.index_link().is_symlink());
    assert_eq!(h.driver.live_allocations(), Some(vec![h.alloc.clone()]));

    vm_reclamation_boot::converge(&h.state)
        .await
        .expect("the boot drive honors the same live claim");
    assert!(h.rootfs.clone_dest().is_dir());
    assert!(h.rootfs.index_link().is_symlink());

    // The real WorkloadLifecycle/runtime composition must treat the durable
    // Pending cleanup disposition as retained work for this exact allocation.
    // It may not fall through to the scheduler and mint `...-1`.
    run_workload_tick(&h, 4).await;
    let retry_rows = h.state.obs.alloc_status_rows().await.expect("read rows after cleanup retry");
    assert_eq!(retry_rows.len(), 1, "cleanup retry must not create a replacement row");
    assert_eq!(retry_rows[0].alloc_id, h.alloc);
    assert_eq!(retry_rows[0].state, AllocState::Pending);
    assert_eq!(retry_rows[0].detail, pending.detail);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 1);
    let retry_view = lifecycle_view(&h);
    assert!(retry_view.last_failure_seen_at.contains_key(&h.alloc));
    assert!(!retry_view.restart_counts.contains_key(&h.alloc));

    // Retry is bounded by the persisted timestamp, but cleanup ownership does
    // not consume the workload crash budget. Persistent substrate failure can
    // therefore neither busy-loop nor strand N fresh allocation IDs.
    run_workload_tick(&h, 5).await;
    for tick_n in 6..=11 {
        h.clock.tick(Duration::from_secs(1));
        run_workload_tick(&h, tick_n).await;
    }
    assert_eq!(h.state.obs.alloc_status_rows().await.expect("read bounded rows").len(), 1);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 1);
    assert_eq!(h.driver.live_allocations(), Some(vec![h.alloc.clone()]));
    let retry_view = lifecycle_view(&h);
    assert!(retry_view.last_failure_seen_at.contains_key(&h.alloc));
    assert!(!retry_view.restart_counts.contains_key(&h.alloc));

    std::fs::remove_dir_all(h.rootfs.clone_dest()).expect("repair cleanup partition");
    h.clock.tick(Duration::from_secs(1));
    run_workload_tick(&h, 12).await;
    let failed = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read recovered row")
        .expect("recovered cleanup has an authoritative terminal row");
    assert_eq!(failed.state, AllocState::Failed);
    assert_eq!(failed.detail, Some(detail));
    assert_eq!(h.driver.live_allocations(), Some(Vec::new()));
    assert!(!h.rootfs.clone_dest().exists());
    assert!(!h.rootfs.index_link().exists());

    // A confirming production tick finalizes the Job failure and clears the
    // retry timestamp, so the runtime broker cannot self-reenqueue forever.
    run_workload_tick(&h, 13).await;
    assert!(
        !lifecycle_view(&h).last_failure_seen_at.contains_key(&h.alloc),
        "terminal cleanup disposition must clear lifecycle retry memory",
    );
    assert_eq!(h.state.obs.alloc_status_rows().await.expect("read terminal rows").len(), 1);

    std::fs::create_dir_all(VmRunDir::for_alloc(&h.layout.run_dir_root, &h.alloc).path())
        .expect("seed post-release stranded run directory");
    let (desired, actual) = reclamation_states(&h).await;
    assert!(desired.allocations[&h.alloc].terminal);
    assert_eq!(
        plan_reclamation(&desired, &actual),
        vec![Action::DiscardStrandedArtifacts { alloc_id: h.alloc.clone() }],
        "only the released terminal allocation authorizes artifact discard",
    );
    vm_reclamation_boot::converge(&h.state).await.expect("discard after release");
    assert!(!VmRunDir::for_alloc(&h.layout.run_dir_root, &h.alloc).path().exists());
    assert_eq!(
        h.driver.live_allocations(),
        Some(Vec::new()),
        "the executor lease is released only after authoritative cleanup completes",
    );
}

#[derive(Clone, Copy)]
enum IntentWithdrawal {
    OperatorStop,
    Delete,
}

async fn retained_cleanup_converges_after_intent_withdrawal(mode: IntentWithdrawal, suffix: &str) {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, suffix).await;
    run_workload_tick(&h, 1).await;
    let cleanup_detail = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read pending row")
        .expect("pending cleanup row")
        .detail
        .expect("durable cleanup diagnostic");

    match mode {
        IntentWithdrawal::OperatorStop => {
            h.state
                .store
                .put(IntentKey::for_workload_stop(&h.workload).as_bytes(), &[0])
                .await
                .expect("persist operator stop intent");
        }
        IntentWithdrawal::Delete => {
            h.state
                .store
                .delete(IntentKey::for_workload(&h.workload).as_bytes())
                .await
                .expect("withdraw workload intent");
        }
    }

    run_workload_tick(&h, 2).await;
    assert_eq!(h.state.obs.alloc_status_rows().await.expect("read pending rows").len(), 1);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 1);
    assert_eq!(h.driver.live_allocations(), Some(vec![h.alloc.clone()]));
    assert!(lifecycle_view(&h).last_failure_seen_at.contains_key(&h.alloc));

    run_workload_tick(&h, 3).await;
    h.clock.tick(Duration::from_secs(1));
    run_workload_tick(&h, 4).await;
    assert_eq!(h.state.obs.alloc_status_rows().await.expect("read bounded rows").len(), 1);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 1);

    std::fs::remove_dir_all(h.rootfs.clone_dest()).expect("repair cleanup partition");
    h.clock.tick(Duration::from_secs(1));
    run_workload_tick(&h, 5).await;
    let terminal = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read terminal row")
        .expect("cleanup withdrawal has an authoritative ending");
    assert_eq!(terminal.state, AllocState::Terminated);
    assert_eq!(terminal.detail, Some(cleanup_detail));
    assert_eq!(h.driver.live_allocations(), Some(Vec::new()));
    assert!(!h.rootfs.clone_dest().exists());
    assert!(!h.rootfs.index_link().exists());

    run_workload_tick(&h, 6).await;
    assert!(lifecycle_view(&h).last_failure_seen_at.is_empty());
    assert_eq!(h.state.obs.alloc_status_rows().await.expect("read final rows").len(), 1);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 1);
    let host = h.state.vm_host_state.observe().await.expect("observe converged host");
    assert!(host.clones.is_empty());
    assert!(host.run_dirs.is_empty());
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn operator_stop_retries_retained_cleanup_without_stranding_residue() {
    retained_cleanup_converges_after_intent_withdrawal(
        IntentWithdrawal::OperatorStop,
        "operator-stop",
    )
    .await;
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn intent_deletion_retries_retained_cleanup_without_stranding_residue() {
    retained_cleanup_converges_after_intent_withdrawal(IntentWithdrawal::Delete, "delete").await;
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn reclamation_lease_wins_before_kill_and_blocks_a_concurrent_start_owner() {
    let tmp = TempDir::new().expect("tempdir");
    let mut h = build_harness(&tmp, "reclaimer-first").await;
    let prior = pending_row(&h, "durable pending row with no process-local owner");
    h.state
        .obs
        .write(ObservationRow::AllocStatus(Box::new(prior.clone())))
        .await
        .expect("seed nonterminal row");
    std::fs::create_dir_all(VmRunDir::for_alloc(&h.layout.run_dir_root, &h.alloc).path())
        .expect("seed stranded run directory");

    let (barrier_host, barrier) = KillBarrierHost::wrap(Arc::clone(&h.state.vm_host_state));
    h.state.vm_host_state = Arc::new(barrier_host);

    let reclaim = dispatch_action(&h, Action::ReclaimAllocation { alloc_id: h.alloc.clone() }, 1);
    let contender = async {
        barrier.wait_until_kill_entered().await;
        assert_eq!(
            h.driver.live_allocations(),
            Some(vec![h.alloc.clone()]),
            "the kill boundary is covered by the exclusive reclamation lease",
        );

        // The real StartAllocation shim reaches the real VmDriver while that
        // lease is held. Duplicate ownership is suppressed without invoking
        // Vmm::create or changing the durable row.
        dispatch_action(&h, start_action(&h), 2).await;
        assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 0);
        assert_eq!(
            h.state.obs.alloc_status_row(&h.alloc).await.expect("read during interleaving"),
            Some(prior.clone()),
        );
        assert_eq!(h.driver.live_allocations(), Some(vec![h.alloc.clone()]));
        barrier.release_kill();
    };
    tokio::join!(reclaim, contender);

    let terminal = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read post-reclamation row")
        .expect("reclamation authors an ending");
    assert_eq!(terminal.state, AllocState::Terminated);
    assert_eq!(terminal.detail, prior.detail);
    assert_eq!(h.vmm.creates.load(Ordering::SeqCst), 0);
    assert_eq!(h.driver.live_allocations(), Some(Vec::new()));
    assert!(!VmRunDir::for_alloc(&h.layout.run_dir_root, &h.alloc).path().exists());
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn process_restart_adopts_pending_cleanup_once_and_preserves_its_error_detail() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "restart").await;
    dispatch_action(&h, start_action(&h), 1).await;
    let pending = h
        .state
        .obs
        .alloc_status_row(&h.alloc)
        .await
        .expect("read cleanup row")
        .expect("pending cleanup row");
    assert_eq!(pending.state, AllocState::Pending);
    let cleanup_detail = pending.detail.clone().expect("durable cleanup detail");

    // Make the retained target a normal file that the fresh process's real
    // reclamation adapter can remove. The original in-process owner is then
    // removed from the composition to model process death.
    std::fs::remove_dir_all(h.rootfs.clone_dest()).expect("remove injected directory partition");
    std::fs::write(h.rootfs.clone_dest(), b"orphaned-clone").expect("stage reclaimable clone");
    let (fresh_driver, _fresh_vmm) = make_driver(h.layout.clone());
    let mut registry = DriverRegistry::new();
    registry.insert(fresh_driver.clone() as Arc<dyn Driver>);
    let mut restarted = h.state.clone();
    restarted.drivers = Arc::new(registry);
    drop(h);

    vm_reclamation_boot::converge(&restarted)
        .await
        .expect("fresh boot becomes the sole cleanup owner");
    let terminal = restarted
        .obs
        .alloc_status_rows()
        .await
        .expect("read terminal rows")
        .into_iter()
        .find(|row| row.detail.as_ref() == Some(&cleanup_detail))
        .expect("the original cleanup error survives boot reclamation");
    assert_eq!(terminal.state, AllocState::Terminated);
    assert!(!terminal.detail.as_deref().unwrap().is_empty());
    let host = restarted.vm_host_state.observe().await.expect("observe post-restart host");
    assert!(host.clones.is_empty());
    assert!(host.run_dirs.is_empty());
    assert_eq!(fresh_driver.live_allocations(), Some(Vec::new()));
}
