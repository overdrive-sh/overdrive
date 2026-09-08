//! Bounded diagnostic of the native E10 sequence; no observation rows are seeded.
//! Registered WorkloadLifecycle, ServiceLifecycle and VmReclamation drive the
//! real VmDriver. The Unix beacon is real. The fixture couples cgroup.kill to
//! SimVmm death and peer closure, matching independent native kernel evidence.
//! Invariant: an instance whose ending has been authored cannot author another
//! natural crash by claiming a same-ID start with no new instance (brief 105a.3).
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "bounded diagnostic retains its required Contract Shape and seed evidence"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_control_plane::action_shim::WorkloadNetworkProvisioner;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_network_provisioner_for_test,
};
use overdrive_control_plane::veth_provisioner::{VethProvisionError, VmTapPlan, WorkloadNetnsPlan};
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV2, VmInput, WorkloadIntent,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::cgroup_fs::{CgroupFs, ProbeError};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_core::traits::vm_host_state::{
    VmHostObservation, VmHostState, VmHostStateProbeError,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::{
    ServiceFailureReason, TerminalCondition, TransitionReason,
};
use overdrive_core::vm::beacon::BEACON_VSOCK_PORT;
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::{
    ca::SimCa,
    clock::SimClock,
    dataplane::SimDataplane,
    entropy::SimEntropy,
    observation_store::SimObservationStore,
    probers::{SimExecProber, SimHttpProber, SimTcpProber},
    vm_host_state::SimVmHostState,
};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::VmDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct Network {
    vmm: Arc<RecordedVmm>,
    live_at_teardown: Mutex<Vec<bool>>,
}

impl WorkloadNetworkProvisioner for Network {
    fn provision(
        &self,
        _: &WorkloadNetnsPlan,
        _: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }
    fn teardown(&self, _: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        let controls = self.vmm.controls.lock();
        let any_live = controls.iter().any(|control| self.vmm.inner.is_live(control.pid));
        drop(controls);
        self.live_at_teardown.lock().push(any_live);
        Ok(())
    }
}

/// Observe the existing Vmm port, without changing its lifetime behavior.
struct RecordedVmm {
    inner: SimVmm,
    controls: Mutex<Vec<VmControl>>,
    scopes: Mutex<Vec<(CgroupPath, VmControl)>>,
    host: SimVmHostState,
    peer: Mutex<Option<BufReader<UnixStream>>>,
    cgroup_kills: Mutex<Vec<u32>>,
}

#[async_trait]
impl Vmm for RecordedVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }
    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let process = self.inner.create(config).await?;
        self.controls.lock().push(process.control.clone());
        self.scopes.lock().push((config.cgroup_scope.clone(), process.control.clone()));
        self.host.set_scope(config.alloc.clone(), std::iter::once(process.control.pid).collect());
        self.host.set_run_dir(config.alloc.clone());
        self.host.set_clone(config.alloc.clone(), config.rootfs.clone_dest().to_path_buf());
        Ok(process)
    }
    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// Existing driven-port composition: a host cgroup kill terminates the SimVmm
/// processes created in that scope. The stock Sim adapters are independent;
/// this fixture links their existing methods at the shared substrate boundary.
struct CoupledHost(Arc<RecordedVmm>);

/// Couple successful cgroup.kill writes to the processes created in that scope.
/// Peer closure models destruction of VMM-owned vsock connections. No driver
/// event, lifecycle row, or private claim is set by this substrate fixture.
struct CoupledCgroupFs {
    inner: SimCgroupFs,
    vmm: Arc<RecordedVmm>,
    root: PathBuf,
    seed: u64,
}

#[async_trait]
impl CgroupFs for CoupledCgroupFs {
    async fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        self.inner.create_dir(path).await
    }
    async fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.write(path, bytes).await?;
        if path.file_name().is_some_and(|name| name == "cgroup.kill") && bytes == b"1\n" {
            let controls: Vec<_> = self
                .vmm
                .scopes
                .lock()
                .iter()
                .filter(|(scope, _)| scope.resolve(&self.root).join("cgroup.kill") == path)
                .map(|(_, control)| control.clone())
                .collect();
            for control in controls {
                if self.vmm.inner.is_live(control.pid) {
                    self.vmm.cgroup_kills.lock().push(control.pid);
                    self.vmm
                        .inner
                        .terminate(&control, Duration::ZERO)
                        .await
                        .map_err(std::io::Error::other)?;
                }
            }
            self.vmm.peer.lock().take();
            // RealCgroupFs awaits tokio::fs::write: syscall effects can occur
            // before the awaiting caller resumes. Select that legal schedule
            // at this driven port, without aborting or overlapping any owner.
            // Native F proves the host effects, not these private task polls.
            for _ in 0..(32 + self.seed % 17) {
                tokio::task::yield_now().await;
            }
        }
        Ok(())
    }
    async fn remove_dir(&self, path: &Path) -> std::io::Result<()> {
        self.inner.remove_dir(path).await
    }
    async fn probe(&self) -> Result<(), ProbeError> {
        self.inner.probe().await
    }
    fn kind(&self) -> &'static str {
        "e10-coupled-cgroup-fs"
    }
}

#[async_trait]
impl VmHostState for CoupledHost {
    fn kind(&self) -> &'static str {
        "fixture-coupled-host"
    }
    async fn probe(&self) -> Result<(), VmHostStateProbeError> {
        self.0.host.probe().await
    }
    async fn observe(&self) -> std::io::Result<VmHostObservation> {
        self.0.host.observe().await
    }
    async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
        let controls: Vec<_> = self
            .0
            .scopes
            .lock()
            .iter()
            .filter(|(created_scope, _)| created_scope == scope)
            .map(|(_, control)| control.clone())
            .collect();
        for control in controls {
            self.0
                .inner
                .terminate(&control, Duration::ZERO)
                .await
                .map_err(std::io::Error::other)?;
        }
        self.0.host.kill_scope(scope).await
    }
    async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
        self.0.host.discard_artifacts(alloc).await
    }
}

async fn guest(path: &Path) -> BufReader<UnixStream> {
    let stream = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(stream) = UnixStream::connect(path).await {
                break stream;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("fixture beacon listener becomes available");
    let mut peer = BufReader::new(stream);
    peer.get_mut().write_all(b"READY pid=1 port=8080\n").await.unwrap();
    let mut exec = String::new();
    peer.read_line(&mut exec).await.unwrap();
    assert!(exec.starts_with("EXEC "), "production driver sends EXEC: {exec:?}");
    // A long-lived guest workload need not voluntarily terminate on host
    // write-half closure. Keep its peer endpoint alive; never inject VMM death.
    peer
}

async fn drive(seed: u64, restart: bool) {
    eprintln!("e10-vm-early-exit seed={seed} restart={restart}");
    let temp = tempfile::tempdir().unwrap();
    let kernel = temp.path().join("kernel");
    let rootfs = temp.path().join("rootfs");
    let mut header = vec![0; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&kernel, header).unwrap();
    std::fs::write(&rootfs, b"sim-rootfs").unwrap();
    let layout = VmHostLayout {
        cgroup_root: temp.path().join("cgroup"),
        run_dir_root: temp.path().join("run"),
        clone_index_dir: temp.path().join("index"),
        clone_staging_dir: temp.path().join("clones"),
        arch: HostArch::X86_64,
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
    };
    let node = NodeId::new("local").unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let vmm = Arc::new(RecordedVmm {
        inner: SimVmm::new(),
        controls: Mutex::new(vec![]),
        scopes: Mutex::new(vec![]),
        host: SimVmHostState::new(),
        peer: Mutex::new(None),
        cgroup_kills: Mutex::new(vec![]),
    });
    let http = Arc::new(SimHttpProber::new());
    http.enqueue_outcome(ProbeOutcome::Fail { reason: "HTTP 302".to_owned() });
    let probes = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        http,
        Arc::new(SimExecProber::new()),
        clock.clone(),
        obs.clone(),
    ));
    let driver = Arc::new(VmDriver::new(
        vmm.clone(),
        clock.clone(),
        Arc::new(CoupledCgroupFs {
            inner: SimCgroupFs::new(),
            vmm: vmm.clone(),
            root: layout.cgroup_root.clone(),
            seed,
        }),
        Arc::new(SimCgroupAccounting::new()),
        probes,
        layout.clone(),
    ));
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(temp.path()).unwrap();
    runtime.register(workload_lifecycle()).await.unwrap();
    runtime.register(service_lifecycle()).await.unwrap();
    runtime.register(overdrive_control_plane::vm_reclamation()).await.unwrap();
    let store = Arc::new(LocalIntentStore::open(temp.path().join("intent.redb")).unwrap());
    let allocator =
        overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
    let mut state = AppState::new(
        store,
        temp.path().join("intent.redb"),
        obs.clone(),
        Arc::new(runtime),
        driver.clone(),
        clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    state.vm_host_state = Arc::new(CoupledHost(vmm.clone()));
    overdrive_control_plane::worker::exit_observer::spawn(
        obs.clone(),
        driver.clone(),
        state.lifecycle_events.clone(),
        clock.clone(),
    );
    let svc = ServiceV2::from_submit(ServiceSpecInput {
        id: "e10-vm-early-exit".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/long-lived-http-server".to_owned(),
            args: vec![],
            kernel: kernel.display().to_string(),
            rootfs: rootfs.display().to_string(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Http { host: None, port: 8080, path: "/ready".to_owned() },
            timeout_seconds: 1,
            interval_seconds: 1,
            max_attempts: 1,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .unwrap();
    let key = IntentKey::for_workload(&svc.id);
    let bytes = WorkloadIntent::Service(svc).archive_for_store().unwrap();
    state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
    let alloc = AllocationId::new("alloc-e10-vm-early-exit-0").unwrap();
    let target = TargetResource::new("workload/e10-vm-early-exit").unwrap();
    let workload = ReconcilerName::new("workload-lifecycle").unwrap();
    let service = ReconcilerName::new("service-lifecycle").unwrap();
    let network = Network { vmm: vmm.clone(), live_at_teardown: Mutex::new(vec![]) };
    let deadline = clock.now() + Duration::from_secs(30);
    let beacon_path =
        VmRunDir::for_alloc(&layout.run_dir_root, &alloc).beacon_socket(BEACON_VSOCK_PORT);
    let (start, peer) = tokio::join!(
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &workload,
            &target,
            clock.now(),
            33,
            deadline,
            &network,
        ),
        guest(&beacon_path),
    );
    start.unwrap();
    *vmm.peer.lock() = Some(peer);
    let control = vmm.controls.lock()[0].clone();
    assert!(vmm.inner.is_live(control.pid), "seed={seed}: live VMM control");
    assert_eq!(obs.alloc_status_rows().await.unwrap()[0].state, AllocState::Running);
    assert!(driver.live_allocations().unwrap().contains(&alloc));
    let reclamation = ReconcilerName::new("vm-reclamation").unwrap();
    let node_target = TargetResource::new("node/local").unwrap();
    run_convergence_tick_with_network_provisioner_for_test(
        &state,
        &reclamation,
        &node_target,
        clock.now(),
        33,
        deadline,
        &network,
    )
    .await
    .unwrap();
    assert!(
        vmm.inner.is_live(control.pid),
        "seed={seed}: reclamation respects existing authorship claim"
    );
    // Let the production supervisor park before advancing its injected clock.
    // Fixed, seed-selected logical progress does not depend on filesystem speed.
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    clock.tick(Duration::from_millis(2_000 + seed % 17));
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let probe_rows = obs.list_probe_results_for_alloc(&alloc).await.unwrap();
    assert!(
        probe_rows.iter().any(|row| matches!(
            &row.status,
            ProbeStatus::Fail { last_fail_reason } if last_fail_reason == "HTTP 302"
        )),
        "seed={seed}: production ProbeRunner must publish the simulated HTTP failure"
    );
    run_convergence_tick_with_network_provisioner_for_test(
        &state,
        &service,
        &target,
        clock.now(),
        34,
        deadline,
        &network,
    )
    .await
    .unwrap();
    let finalized = obs.alloc_status_rows().await.unwrap().remove(0);
    let history = obs.alloc_lifecycle_occurrences(&alloc).await.unwrap();
    assert_eq!(finalized.state, AllocState::Failed, "seed={seed}: production startup ending");
    assert!(
        matches!(
            &finalized.terminal,
            Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed { last_fail, attempts: 1, .. }
            }) if last_fail == "HTTP 302"
        ),
        "seed={seed}: ServiceLifecycle authors the startup-probe failure"
    );
    assert!(!driver.live_allocations().unwrap().contains(&alloc));
    assert!(vmm.inner.is_live(control.pid));
    assert_eq!(
        *network.live_at_teardown.lock(),
        vec![true],
        "seed={seed}: startup network teardown did not terminate the original VMM"
    );
    assert!(beacon_path.exists(), "seed={seed}: first attempt leaves its real beacon pathname");
    eprintln!("seed={seed}: finalized row={finalized:?}");

    if restart {
        // Standing Service intent and the registered owner's own restart policy.
        // Never dispatch a made-up action or edit private view/backoff state.
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &workload,
            &target,
            clock.now(),
            35,
            deadline,
            &network,
        )
        .await
        .unwrap();
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        let after = obs.alloc_status_rows().await.unwrap().remove(0);
        let occurrences = obs.alloc_lifecycle_occurrences(&alloc).await.unwrap();
        eprintln!(
            "seed={seed}: pre-sweep creates={} killed_pids={:?} live={} after={after:?} occurrences={occurrences:#?}",
            vmm.controls.lock().len(),
            vmm.cgroup_kills.lock(),
            vmm.inner.is_live(control.pid)
        );
        assert!(clock.now() < deadline, "seed={seed}: restart precedes first normal 30s sweep");
        assert_eq!(vmm.controls.lock().len(), 1, "seed={seed}: replacement never creates a VMM");
        assert_eq!(
            *vmm.cgroup_kills.lock(),
            vec![control.pid],
            "seed={seed}: failed-start cleanup kills the initial VMM"
        );
        assert!(!vmm.inner.is_live(control.pid));
        // A failed replacement may add a start-rejection ending. It cannot
        // turn the previously finalized first instance into a NATURAL crash.
        // This checks provenance, not Terminated or any particular remedy.
        let fabricated_crash = occurrences.iter().any(|occurrence| {
            !history.contains(occurrence)
                && matches!(
                    occurrence.reason,
                    Some(TransitionReason::WorkloadCrashedImmediately { .. })
                )
        });
        assert!(
            !fabricated_crash,
            "seed={seed}: already-ended VM authored a second natural crash while a replacement start had produced no new VM"
        );
    } else {
        clock.tick(Duration::from_secs(30));
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &reclamation,
            &node_target,
            clock.now(),
            35,
            clock.now() + Duration::from_secs(30),
            &network,
        )
        .await
        .unwrap();
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        assert!(!vmm.inner.is_live(control.pid));
        assert!(vmm.host.artifacts_absent(&alloc));
        assert_eq!(
            obs.alloc_status_rows().await.unwrap()[0],
            finalized,
            "seed={seed}: registered disposal preserves terminal ending"
        );
        assert_eq!(
            obs.alloc_lifecycle_occurrences(&alloc).await.unwrap(),
            history,
            "seed={seed}: no restart, no second ending from the old watcher"
        );
        eprintln!(
            "seed={seed}: registered reclamation control passed; terminal and occurrences unchanged"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Universe: original VMM liveness, Vmm::create results, cgroup-killed PIDs,
/// and allocation lifecycle occurrences. Startup failure releases the claim;
/// replacement rejection may add its own failure, never another natural ending
/// for the already-finalized instance (brief §105a.3).
#[tokio::test(flavor = "current_thread")]
async fn same_id_restart_cannot_reauthor_finalized_vm_as_natural_crash() {
    drive(257_205, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Universe: original VMM liveness, host artifacts, full allocation row and
/// occurrence history. Registered terminal-unclaimed disposal removes the
/// substrate while the authored ending and every occurrence remain unchanged.
#[tokio::test(flavor = "current_thread")]
async fn no_intervening_restart_reclamation_preserves_authored_ending_control() {
    drive(257_205, false).await;
}
