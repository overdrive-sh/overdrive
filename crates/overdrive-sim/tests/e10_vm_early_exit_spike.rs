//! Bounded production-owner preservation control for GH #284. Registered
//! WorkloadLifecycle, ServiceLifecycle, and VmReclamation drive the real
//! VmDriver from an initial Running VM through startup failure and exact
//! terminal-unclaimed disposal with no intervening replacement. No observation
//! row is seeded. The Unix beacon is real, completed driver cleanup is coupled
//! to Sim host facts, and cgroup.kill is coupled to SimVmm death.
//!
//! The rejected ADR-0104 VM-only restart scenario was removed during
//! corrective re-DISTILL. Driver-neutral predecessor-to-fresh-successor
//! behavior now lives exclusively in `driver_neutral_allocation_replacement.rs`.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "bounded diagnostic retains its required Contract Shape and seed evidence"
)]

use std::io;
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
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::cgroup_fs::{CgroupFs, ProbeError};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_core::traits::vm_host_state::{
    VmHostObservation, VmHostState, VmHostStateProbeError,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
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
    view_store::SimViewStore,
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
    provisions: Mutex<Vec<(WorkloadNetnsPlan, Option<VmTapPlan>)>>,
    teardowns: Mutex<Vec<WorkloadNetnsPlan>>,
}

impl WorkloadNetworkProvisioner for Network {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        self.provisions.lock().push((workload.clone(), vm_tap.cloned()));
        Ok(())
    }
    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        let controls = self.vmm.controls.lock();
        let any_live = controls.iter().any(|control| self.vmm.inner.is_live(control.pid));
        drop(controls);
        self.live_at_teardown.lock().push(any_live);
        self.teardowns.lock().push(workload.clone());
        Ok(())
    }
}

/// Observe the existing Vmm port, without changing its lifetime behavior.
struct RecordedVmm {
    inner: SimVmm,
    controls: Mutex<Vec<VmControl>>,
    executions: Mutex<Vec<(AllocationId, CgroupPath, VmControl)>>,
    host: SimVmHostState,
    peer: Mutex<Option<BufReader<UnixStream>>>,
    cgroup_kills: Mutex<Vec<u32>>,
    host_scope_kills: Mutex<Vec<CgroupPath>>,
    artifact_discards: Mutex<Vec<AllocationId>>,
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
        self.executions.lock().push((
            config.alloc.clone(),
            config.cgroup_scope.clone(),
            process.control.clone(),
        ));
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
/// No driver event, lifecycle row, or private claim is set by this substrate
/// fixture.
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
                .executions
                .lock()
                .iter()
                .filter(|(_, scope, _)| scope.resolve(&self.root).join("cgroup.kill") == path)
                .map(|(_, _, control)| control.clone())
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
        self.inner.remove_dir(path).await?;
        // `VmDriver::stop` awaits cgroup removal and then completes the
        // remaining run-dir/rootfs cleanup before returning. Project that
        // completed production unwind onto the Sim host observation at this
        // existing driven-port boundary; the assertion is made only after the
        // whole owner tick returns.
        let cleaned = self
            .vmm
            .executions
            .lock()
            .iter()
            .filter(|(_, scope, _)| scope.resolve(&self.root) == path)
            .map(|(alloc, scope, _)| (alloc.clone(), scope.clone()))
            .collect::<Vec<_>>();
        for (alloc, scope) in cleaned {
            self.vmm.host.kill_scope(&scope).await?;
            self.vmm.host.discard_artifacts(&alloc).await?;
        }
        Ok(())
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
        self.0.host_scope_kills.lock().push(scope.clone());
        let controls: Vec<_> = self
            .0
            .executions
            .lock()
            .iter()
            .filter(|(_, created_scope, _)| created_scope == scope)
            .map(|(_, _, control)| control.clone())
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
        self.0.artifact_discards.lock().push(alloc.clone());
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

/// Connect and report READY without waiting for EXEC. A rejected Running
/// publication deliberately never releases EXEC, so this is the honest guest
/// half for that boundary; keeping the stream alive lets the production
/// failed-publication unwind send SHUTDOWN and terminate the VMM.
async fn guest_ready(path: &Path) -> BufReader<UnixStream> {
    let stream = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(stream) = UnixStream::connect(path).await {
                break stream;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("fresh replacement beacon listener becomes available");
    let mut peer = BufReader::new(stream);
    peer.get_mut().write_all(b"READY pid=1 port=8080\n").await.unwrap();
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
        executions: Mutex::new(vec![]),
        host: SimVmHostState::new(),
        peer: Mutex::new(None),
        cgroup_kills: Mutex::new(vec![]),
        host_scope_kills: Mutex::new(vec![]),
        artifact_discards: Mutex::new(vec![]),
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
    let network = Network {
        vmm: vmm.clone(),
        live_at_teardown: Mutex::new(vec![]),
        provisions: Mutex::new(vec![]),
        teardowns: Mutex::new(vec![]),
    };
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
    assert_eq!(network.provisions.lock().len(), 1);
    assert_eq!(network.teardowns.lock().as_slice(), &[network.provisions.lock()[0].0.clone()]);
    assert!(beacon_path.exists(), "seed={seed}: first attempt leaves its real beacon pathname");
    eprintln!("seed={seed}: finalized row={finalized:?}");

    if restart {
        eprintln!(
            "reproduce: cargo xtask lima run -- cargo nextest run -p overdrive-sim \
             --features integration-tests,overdrive-control-plane/integration-tests \
             --test e10_vm_early_exit_spike --run-ignored ignored-only \
             -E 'test(vm_recreation_reserves_each_execution_and_old_cleanup_cannot_cross_ids)' \
             --no-capture"
        );

        // The first fresh replacement is allowed to create a real VM but its
        // initial Running publication is rejected. The runtime has already
        // fsynced WorkloadLifecycle's next View before this dispatch and must
        // retain that ID reservation when the action returns its typed error.
        let rejected = AllocationId::new("alloc-e10-vm-early-exit-1").unwrap();
        let rejected_beacon =
            VmRunDir::for_alloc(&layout.run_dir_root, &rejected).beacon_socket(BEACON_VSOCK_PORT);
        obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
        let (rejected_result, rejected_peer) = {
            let rejected_tick = run_convergence_tick_with_network_provisioner_for_test(
                &state,
                &workload,
                &target,
                clock.now(),
                35,
                clock.now() + Duration::from_secs(30),
                &network,
            );
            tokio::pin!(rejected_tick);
            let rejected_guest = guest_ready(&rejected_beacon);
            tokio::pin!(rejected_guest);
            tokio::select! {
                peer = &mut rejected_guest => {
                    let result = rejected_tick.await;
                    (result, peer)
                }
                result = &mut rejected_tick => {
                    panic!(
                        "seed={seed}: MISSING_FUNCTIONALITY — WorkloadLifecycle completed the VM \
                         replacement under the predecessor identity before the fresh beacon \
                         {} existed; result={result:?}",
                        rejected_beacon.display(),
                    );
                }
            }
        };
        drop(rejected_peer);
        assert!(
            rejected_result.is_err(),
            "seed={seed}: the injected Running publication failure must remain typed"
        );
        assert!(
            obs.alloc_status_row(&rejected).await.unwrap().is_none(),
            "seed={seed}: rejected publication must not manufacture an allocation row"
        );
        assert!(
            !driver.live_allocations().unwrap().contains(&rejected),
            "seed={seed}: rejected start must complete its existing stop/claim unwind"
        );
        assert!(
            vmm.host.artifacts_absent(&rejected),
            "seed={seed}: the rejected start's awaited driver.stop must complete host cleanup \
             before any VmReclamation tick"
        );
        assert_eq!(network.provisions.lock().len(), 2);
        assert_eq!(
            network.teardowns.lock().as_slice(),
            &[network.provisions.lock()[0].0.clone(), network.provisions.lock()[1].0.clone(),],
            "seed={seed}: every existing C3/TAP teardown targets exactly its own provisioned plan"
        );
        let reserved = state.runtime.view_for_workload_lifecycle(&target);
        assert!(
            reserved.restart_counts.contains_key(&rejected),
            "seed={seed}: the fsynced View must retain unpublished ID {rejected}"
        );

        // Model a process boundary at the existing redb-backed runtime owner:
        // close the live handle, reopen the same directory, and re-register
        // every owner. WorkloadLifecycle's register-time bulk-load must restore
        // the unpublished reservation before the next production tick.
        let placeholder = ReconcilerRuntime::new(temp.path(), Arc::new(SimViewStore::new()))
            .expect("construct transient runtime holder while closing redb");
        let closed_runtime = std::mem::replace(&mut state.runtime, Arc::new(placeholder));
        drop(closed_runtime);
        let mut reopened =
            ReconcilerRuntime::new_with_redb_view_store_for_test(temp.path()).unwrap();
        reopened.register(workload_lifecycle()).await.unwrap();
        reopened.register(service_lifecycle()).await.unwrap();
        reopened.register(overdrive_control_plane::vm_reclamation()).await.unwrap();
        let restored = reopened.view_for_workload_lifecycle(&target);
        assert!(
            restored.restart_counts.contains_key(&rejected),
            "seed={seed}: WorkloadLifecycle register bulk-load must restore unpublished \
             reservation {rejected} after runtime restart"
        );
        assert_eq!(
            obs.alloc_status_row(&alloc).await.unwrap(),
            Some(finalized.clone()),
            "seed={seed}: a View reservation is not current; the accepted predecessor remains current"
        );
        assert!(obs.alloc_status_row(&rejected).await.unwrap().is_none());
        state.runtime = Arc::new(reopened);

        // Re-drive through the same production owner after the candidate's
        // existing backoff. The View-only reservation is not current, but it
        // consumes suffix 1; the next physical VM must therefore be suffix 2.
        clock.tick(Duration::from_secs(2));
        let replacement = AllocationId::new("alloc-e10-vm-early-exit-2").unwrap();
        let replacement_beacon = VmRunDir::for_alloc(&layout.run_dir_root, &replacement)
            .beacon_socket(BEACON_VSOCK_PORT);
        let (replacement_result, replacement_peer) = tokio::join!(
            run_convergence_tick_with_network_provisioner_for_test(
                &state,
                &workload,
                &target,
                clock.now(),
                36,
                clock.now() + Duration::from_secs(30),
                &network,
            ),
            guest(&replacement_beacon),
        );
        replacement_result.unwrap();

        let controls = vmm.controls.lock().clone();
        assert_eq!(controls.len(), 3, "seed={seed}: old, rejected, and accepted VMMs created");
        let rejected_control = &controls[1];
        let replacement_control = &controls[2];
        assert!(vmm.inner.is_live(control.pid), "seed={seed}: old cleanup is still delayed");
        assert!(
            !vmm.inner.is_live(rejected_control.pid),
            "seed={seed}: rejected execution was cleaned by its own unwind"
        );
        assert!(
            vmm.inner.is_live(replacement_control.pid),
            "seed={seed}: accepted replacement is live"
        );

        let fresh_row =
            obs.alloc_status_row(&replacement).await.unwrap().expect("fresh accepted row");
        assert_eq!(fresh_row.state, AllocState::Running);
        assert_eq!(fresh_row.restart_count, 0, "a fresh physical key starts its own count");
        assert!(fresh_row.last_terminated.is_none(), "predecessor history is not copied");
        assert_eq!(
            obs.alloc_status_row(&alloc).await.unwrap(),
            Some(finalized.clone()),
            "seed={seed}: accepting the fresh row must not overwrite its predecessor"
        );
        assert_eq!(
            obs.alloc_lifecycle_occurrences(&alloc).await.unwrap(),
            history,
            "seed={seed}: neither rejected publication nor the fresh row reauthors old history"
        );
        let network_provisions_before_reaper = network.provisions.lock().clone();
        let network_teardowns_before_reaper = network.teardowns.lock().clone();

        // Now allow the actual registered VmReclamation owner to dispose the
        // old allocation. The rejected execution was already cleaned by its
        // awaited start-unwind and has no accepted row. The replacement remains
        // supervised, so old cleanup cannot name, signal, or remove it.
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &reclamation,
            &node_target,
            clock.now(),
            37,
            clock.now() + Duration::from_secs(30),
            &network,
        )
        .await
        .unwrap();
        for _ in 0..128 {
            tokio::task::yield_now().await;
        }
        assert!(!vmm.inner.is_live(control.pid), "seed={seed}: old VMM disposed");
        assert!(vmm.host.artifacts_absent(&alloc), "seed={seed}: old artifacts absent");
        assert!(
            vmm.host.artifacts_absent(&rejected),
            "seed={seed}: rejected execution artifacts absent"
        );
        assert!(
            !vmm.host.artifacts_absent(&replacement),
            "seed={seed}: replacement artifacts survive old cleanup"
        );
        assert!(
            vmm.inner.is_live(replacement_control.pid),
            "seed={seed}: old cleanup cannot signal the replacement VMM"
        );
        assert_eq!(
            obs.alloc_status_row(&replacement).await.unwrap(),
            Some(fresh_row.clone()),
            "seed={seed}: stale old session/reaper cannot author the current row"
        );
        assert_eq!(
            *network.provisions.lock(),
            network_provisions_before_reaper,
            "seed={seed}: old VM artifact disposal does not provision or reinterpret TAP ownership"
        );
        assert_eq!(
            *network.teardowns.lock(),
            network_teardowns_before_reaper,
            "seed={seed}: old VM artifact disposal does not tear down the replacement C3/TAP plan"
        );

        let first_discards = vmm.artifact_discards.lock().clone();
        let first_scope_kills = vmm.host_scope_kills.lock().clone();
        assert!(first_discards.contains(&alloc));
        assert!(
            !first_discards.contains(&rejected),
            "seed={seed}: VmReclamation must not fabricate ownership of an already-cleaned, \
             unpublished execution"
        );
        assert!(!first_discards.contains(&replacement));

        // Exactly-once ownership: once host observations are absent, a second
        // reconciliation emits no repeated cleanup for either ended ID.
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &reclamation,
            &node_target,
            clock.now(),
            38,
            clock.now() + Duration::from_secs(30),
            &network,
        )
        .await
        .unwrap();
        assert_eq!(*vmm.artifact_discards.lock(), first_discards);
        assert_eq!(*vmm.host_scope_kills.lock(), first_scope_kills);

        // Complete the journey through the existing operator-stop owner, then
        // let reclamation observe the terminal fresh row so the Sim host
        // adapter reaches the same final complement as the real host lane.
        let stop_key = IntentKey::for_workload_stop(
            &WorkloadId::new("e10-vm-early-exit").expect("valid workload id"),
        );
        state.store.put(stop_key.as_bytes(), b"").await.unwrap();
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &workload,
            &target,
            clock.now(),
            39,
            clock.now() + Duration::from_secs(30),
            &network,
        )
        .await
        .unwrap();
        drop(replacement_peer);
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &reclamation,
            &node_target,
            clock.now(),
            40,
            clock.now() + Duration::from_secs(30),
            &network,
        )
        .await
        .unwrap();
        assert!(vmm.host.artifacts_absent(&alloc));
        assert!(vmm.host.artifacts_absent(&rejected));
        assert!(vmm.host.artifacts_absent(&replacement));
        assert!(driver.live_allocations().unwrap().is_empty());
        let provisions = network.provisions.lock().clone();
        let teardowns = network.teardowns.lock().clone();
        assert_eq!(provisions.len(), 3);
        assert_eq!(teardowns.len(), 3);
        assert!(
            provisions.iter().all(|(_, tap)| tap.is_some()),
            "seed={seed}: every VM execution used the existing C3 TAP plan"
        );
        assert!(
            provisions.iter().zip(teardowns.iter()).all(|((plan, _), teardown)| plan == teardown),
            "seed={seed}: TAP/netns ownership stays on the existing provision/teardown pair"
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
/// Universe: original VMM liveness, host artifacts, full allocation row and
/// occurrence history. Registered terminal-unclaimed disposal removes the
/// substrate while the authored ending and every occurrence remain unchanged.
#[tokio::test(flavor = "current_thread")]
async fn no_intervening_restart_reclamation_preserves_authored_ending_control() {
    drive(257_205, false).await;
}
