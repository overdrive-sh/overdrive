//! Recovery R2/R6 composition: a real VM failed-start cleanup failure is
//! durably closed as Failed before ordinary VM Artifact Disposal reclaims the
//! surviving resource-specific residue.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::action_shim::{
    WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::{
    NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
};
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocLifecyclePredecessor, AllocState, ObservationStore, TransitionSource,
};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmError, VmmProbeError,
};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmmIdentity,
};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_host::vm_host_state::RealVmHostState;
use overdrive_reconcilers::vm_reclamation::{
    SupervisionSet, VmAllocFacts, VmReclamationState, plan_reclamation,
};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::{VmDriver, VmHostLayout};
use tempfile::TempDir;

#[derive(Default)]
struct RecordingNetworkProvisioner {
    provisions: AtomicUsize,
    teardowns: AtomicUsize,
}

impl WorkloadNetworkProvisioner for RecordingNetworkProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        assert!(vm_tap.is_some(), "the VM allocation receives its tap plan");
        self.provisions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.teardowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Leaves an allocation-indexed directory where the rootfs clone must be a
/// file, then rejects creation. The real `VmDriver` attempts every cleanup
/// stage; clone removal fails with `EISDIR`, so the ordered unclassified
/// composite and durable index remain available to Artifact Disposal.
struct CloneRemovalPartitionVmm {
    creates: AtomicUsize,
    terminates: AtomicUsize,
}

#[async_trait]
impl Vmm for CloneRemovalPartitionVmm {
    fn kind(&self) -> &'static str {
        "clone-removal-partition"
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        Ok(())
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        std::fs::create_dir_all(config.rootfs.clone_dest()).map_err(|source| VmmError::Create {
            detail: format!("stage clone-removal partition: {source}"),
        })?;
        Err(VmmError::Create { detail: "injected VMM create rejection".to_owned() })
    }

    async fn terminate(&self, _control: &VmControl, _grace: Duration) -> VmmResult<VmTermination> {
        self.terminates.fetch_add(1, Ordering::SeqCst);
        Ok(VmTermination::Killed)
    }
}

fn stage_artifacts(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let kernel = tmp.path().join("vmlinuz");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&kernel, header).expect("stage kernel");
    let rootfs = tmp.path().join("master.img");
    std::fs::write(&rootfs, b"fixture rootfs").expect("stage rootfs");
    (kernel, rootfs)
}

fn spec(
    alloc: &AllocationId,
    workload: &WorkloadId,
    kernel: PathBuf,
    rootfs: PathBuf,
) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/{workload}/alloc/{alloc}"
        ))
        .expect("valid identity"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel,
            rootfs,
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
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    action: Action,
    drivers: &DriverRegistry,
    alloc_drivers: &overdrive_control_plane::action_shim::AllocDriverIndex,
    obs: &dyn ObservationStore,
    store: Arc<dyn IntentStore>,
    clock: &dyn Clock,
    node: &NodeId,
    net_slots: &NetSlotAllocator,
    network: &RecordingNetworkProvisioner,
    host: &dyn VmHostState,
    tick_number: u64,
) {
    let (events, _receiver) = tokio::sync::broadcast::channel(16);
    let now = Instant::now();
    dispatch_with_network_provisioner(
        vec![action],
        drivers,
        alloc_drivers,
        obs,
        &SimDataplane::new(),
        &SimCa::new(Arc::new(SimEntropy::new(0))),
        clock,
        &IdentityMgr::new(None),
        &events,
        &TickContext {
            now,
            now_unix: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
                1_700_000_000 + tick_number,
            )),
            tick: tick_number,
            deadline: now + Duration::from_secs(2),
        },
        node,
        Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
            VipRange::default(),
            store,
        ))),
        &parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
        None,
        None,
        net_slots,
        network,
        host,
    )
    .await
    .expect("the production action boundary handles the action");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "one production-composition journey retains the Failed write, reclamation plan, stale-action replay, and sibling complement"
)]
#[tokio::test]
async fn failed_vm_cleanup_hands_only_stranded_vm_artifacts_to_disposal() {
    let tmp = TempDir::new().expect("tempdir");
    let (kernel, master) = stage_artifacts(&tmp);
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
    let alloc = AllocationId::new("alloc-failed-cleanup-disposal").expect("valid alloc");
    let workload = WorkloadId::new("failed-cleanup-disposal").expect("valid workload");
    let sibling = AllocationId::new("alloc-failed-cleanup-sibling").expect("valid sibling");
    let node = NodeId::new("local").expect("valid node");
    let rootfs = RootfsPlan::for_alloc(
        master.clone(),
        std::fs::metadata(&master).expect("rootfs metadata").len(),
        &alloc,
        &layout.clone_staging_dir,
        &layout.clone_index_dir,
    );
    let sibling_rootfs = RootfsPlan::for_alloc(
        master.clone(),
        std::fs::metadata(&master).expect("rootfs metadata").len(),
        &sibling,
        &layout.clone_staging_dir,
        &layout.clone_index_dir,
    );

    // Pre-existing sibling resources span all three VM-exclusive host
    // surfaces and must be byte-for-byte unchanged by target disposal.
    let sibling_scope = CgroupPath::for_alloc(&sibling).resolve(&layout.cgroup_root);
    std::fs::create_dir_all(&sibling_scope).expect("stage sibling scope");
    std::fs::write(sibling_scope.join("cgroup.procs"), b"4242\n")
        .expect("stage sibling process fact");
    let sibling_run = layout.run_dir_root.join(sibling.as_str());
    std::fs::create_dir_all(&sibling_run).expect("stage sibling run dir");
    std::fs::write(sibling_run.join("marker"), b"sibling-run").expect("stage sibling run marker");
    std::fs::create_dir_all(sibling_rootfs.clone_dest().parent().expect("clone parent"))
        .expect("stage clone parent");
    std::fs::write(sibling_rootfs.clone_dest(), b"sibling-clone").expect("stage sibling clone");
    std::fs::create_dir_all(sibling_rootfs.index_link().parent().expect("index parent"))
        .expect("stage index parent");
    std::os::unix::fs::symlink(sibling_rootfs.clone_dest(), sibling_rootfs.index_link())
        .expect("stage sibling clone index");

    let vmm = Arc::new(CloneRemovalPartitionVmm {
        creates: AtomicUsize::new(0),
        terminates: AtomicUsize::new(0),
    });
    let driver = Arc::new(VmDriver::new(
        Arc::clone(&vmm) as Arc<dyn Vmm>,
        Arc::new(SimClock::new()),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        layout.clone(),
    ));
    let mut drivers = DriverRegistry::new();
    drivers.insert(Arc::clone(&driver) as Arc<dyn Driver>);
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let clock = SimClock::new();
    let net_slots = NetSlotAllocator::new();
    let network = RecordingNetworkProvisioner::default();
    let host = RealVmHostState::new(
        layout.cgroup_root.clone(),
        layout.run_dir_root.clone(),
        layout.clone_index_dir.clone(),
    );

    dispatch(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec: spec(&alloc, &workload, kernel, master),
            kind: WorkloadKind::Job,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &clock,
        &node,
        &net_slots,
        &network,
        &host,
        1,
    )
    .await;

    let failed =
        obs.alloc_status_row(&alloc).await.expect("read current").expect("Failed is durable");
    assert_eq!(failed.state, AllocState::Failed);
    assert!(failed.started_at.is_none(), "a rejected create never reaches EXEC/Running");
    let detail = failed.detail.as_deref().expect("ordered composite is retained");
    assert!(detail.contains("primary rejection:"));
    assert!(detail.contains("injected VMM create rejection"));
    assert!(detail.contains("rootfs clone remove:"));
    let occurrences = obs.alloc_lifecycle_occurrences(&alloc).await.expect("read occurrences");
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].from, AllocLifecyclePredecessor::Absent);
    assert_eq!(occurrences[0].to, AllocState::Failed);
    assert_eq!(occurrences[0].source, TransitionSource::Driver(DriverType::Vm));
    assert_eq!(driver.live_allocations(), Some(Vec::new()), "claim releases after the write");
    assert_eq!(vmm.creates.load(Ordering::SeqCst), 1);
    assert_eq!(vmm.terminates.load(Ordering::SeqCst), 0, "no VMM control/EXEC existed");
    assert!(!net_slots.snapshot().contains_key(&alloc));
    assert_eq!(network.provisions.load(Ordering::SeqCst), 1);
    assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
    assert!(rootfs.clone_dest().is_dir(), "the failed cleanup leaves indexed residue");
    assert!(rootfs.index_link().is_symlink());

    // The injected EISDIR cause is transient. Once the target has the lawful
    // clone-file shape again, ordinary Artifact Disposal owns its recovery;
    // no Pending token or second cleanup protocol participates.
    std::fs::remove_dir(rootfs.clone_dest()).expect("clear transient clone shape");
    std::fs::write(rootfs.clone_dest(), b"recoverable target residue")
        .expect("restore lawful clone-file shape");

    let mut desired = VmReclamationState::default();
    desired.allocations.insert(
        alloc.clone(),
        VmAllocFacts { workload_id: workload.clone(), terminal: failed.state.is_terminal() },
    );
    desired
        .allocations
        .insert(sibling.clone(), VmAllocFacts { workload_id: workload, terminal: false });
    let actual = VmReclamationState {
        allocations: BTreeMap::default(),
        host: host.observe().await.expect("observe host residue"),
        supervision: SupervisionSet::Observed(BTreeSet::from([sibling.clone()])),
    };
    let actions = plan_reclamation(&desired, &actual);
    assert_eq!(actions, vec![Action::DiscardStrandedArtifacts { alloc_id: alloc.clone() }]);

    let stale_disposal = actions[0].clone();
    dispatch(
        stale_disposal.clone(),
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &clock,
        &node,
        &net_slots,
        &network,
        &host,
        2,
    )
    .await;
    assert!(!rootfs.clone_dest().exists());
    assert!(!rootfs.index_link().exists());
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("read Failed after disposal"),
        Some(failed.clone()),
        "Artifact Disposal authors no second ending",
    );
    assert_eq!(
        obs.alloc_lifecycle_occurrences(&alloc).await.expect("read occurrences after disposal"),
        occurrences,
    );

    // A stale replay is idempotent: there is still exactly one Failed
    // occurrence, no target residue reappears, and every sibling byte remains.
    dispatch(
        stale_disposal,
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        store,
        &clock,
        &node,
        &net_slots,
        &network,
        &host,
        3,
    )
    .await;
    assert!(!rootfs.clone_dest().exists());
    assert!(!rootfs.index_link().exists());
    assert_eq!(obs.alloc_lifecycle_occurrences(&alloc).await.unwrap(), occurrences);
    assert_eq!(std::fs::read(sibling_scope.join("cgroup.procs")).unwrap(), b"4242\n");
    assert_eq!(std::fs::read(sibling_run.join("marker")).unwrap(), b"sibling-run");
    assert_eq!(std::fs::read(sibling_rootfs.clone_dest()).unwrap(), b"sibling-clone");
    assert_eq!(
        std::fs::read_link(sibling_rootfs.index_link()).unwrap(),
        sibling_rootfs.clone_dest(),
    );
    assert_eq!(network.provisions.load(Ordering::SeqCst), 1);
    assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
}
