//! Bounded DESIGN witness: terminal VM cleanup hands off from ending authorship
//! to registered VmReclamation. Allocation rows are production-authored.
//! The real VmDriver is composed over existing Sim Vmm/cgroup/probe ports;
//! a protocol peer supplies READY and remains connected after receiving EXEC.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "bounded diagnostic retains its required Contract Shape and seed evidence"
)]

use std::path::Path;
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
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationHandle, Driver, DriverError};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::traits::vm_host_state::{
    VmHostObservation, VmHostState, VmHostStateProbeError,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
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

async fn with_clock<F: std::future::Future>(clock: &SimClock, seed: u64, future: F) -> F::Output {
    tokio::pin!(future);
    tokio::select! {
        output = &mut future => output,
        () = async {
            for _ in 0..4096 {
                for _ in 0..16 { tokio::task::yield_now().await; }
                clock.tick(Duration::from_millis(10 + seed % 7));
            }
        } => panic!("seed={seed}: bounded fixture schedule exhausted"),
    }
}

async fn drive(seed: u64, finalize: bool) {
    eprintln!("vm-finalize-ownership seed={seed} finalize={finalize}");
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
    });
    let probes = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        clock.clone(),
        obs.clone(),
    ));
    let driver = Arc::new(VmDriver::new(
        vmm.clone(),
        clock.clone(),
        Arc::new(SimCgroupFs::new()),
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
        id: "vm-finalize-ownership".to_owned(),
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
    let alloc = AllocationId::new("alloc-vm-finalize-ownership-0").unwrap();
    let target = TargetResource::new("workload/vm-finalize-ownership").unwrap();
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
    let handle = AllocationHandle { alloc: alloc.clone(), pid: Some(control.pid) };
    if finalize {
        clock.tick(Duration::from_secs(2));
        obs.write_probe_result(ProbeResultRow {
            alloc_id: alloc.clone(),
            probe_idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            status: ProbeStatus::Fail { last_fail_reason: "HTTP 302".to_owned() },
            last_observed_at_unix_ms: u64::try_from(clock.unix_now().as_millis()).unwrap(),
            inferred: false,
        })
        .await
        .unwrap();
        with_clock(
            &clock,
            seed,
            run_convergence_tick_with_network_provisioner_for_test(
                &state,
                &service,
                &target,
                clock.now(),
                34,
                deadline,
                &network,
            ),
        )
        .await
        .unwrap();
    }
    let owned_before_stop = driver.live_allocations().unwrap().contains(&alloc);
    let live_before_stop = vmm.inner.is_live(control.pid);
    let later_stop = with_clock(&clock, seed, driver.stop(&handle)).await;
    let live_after_stop = vmm.inner.is_live(control.pid);
    let row = obs.alloc_status_rows().await.unwrap().remove(0);
    eprintln!(
        "seed={seed} finalize={finalize} row={row:?} owned_before_stop={owned_before_stop} live_before_stop={live_before_stop} subsequent_stop={later_stop:?} live_after_stop={live_after_stop} network_teardowns={}",
        network.live_at_teardown.lock().len()
    );
    if finalize {
        assert_eq!(row.state, AllocState::Failed, "seed={seed}: production startup terminal");
        assert!(row.terminal.is_some(), "seed={seed}: terminal authored by ServiceLifecycle");
        let before = obs.alloc_lifecycle_occurrences(&alloc).await.unwrap();
        clock.tick(Duration::from_secs(30));
        run_convergence_tick_with_network_provisioner_for_test(
            &state,
            &reclamation,
            &node_target,
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
            "seed={seed} after_reclamation_live={} host_artifacts_absent={} row_unchanged={} occurrences_unchanged={}",
            vmm.inner.is_live(control.pid),
            vmm.host.artifacts_absent(&alloc),
            after == row,
            occurrences == before
        );
        assert!(
            !vmm.inner.is_live(control.pid),
            "seed={seed}: existing reclamation terminates the VMM"
        );
        assert!(vmm.host.artifacts_absent(&alloc) && !vmm.host.has_scope(&alloc));
        assert_eq!(after, row, "seed={seed}: disposal preserves authored startup terminal");
        assert_eq!(
            occurrences, before,
            "seed={seed}: disposal and late watcher author no new ending"
        );
    } else {
        assert!(
            owned_before_stop && live_before_stop,
            "seed={seed}: orderly-stop control has its owner"
        );
        assert!(later_stop.is_ok(), "seed={seed}: orderly stop succeeds");
        assert!(!live_after_stop, "seed={seed}: orderly stop terminates VMM");
    }
    // Fixture disposal after recording/asserting the production result.
    vmm.terminate(&control, Duration::ZERO).await.unwrap();
    drop(peer);
    assert!(later_stop.is_ok() || matches!(later_stop, Err(DriverError::NotFound { .. })));
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn vm_finalize_failed_reclamation_preserves_terminal_seeded_convergence() {
    drive(257_204, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn vm_orderly_stop_retains_owner_until_termination_control() {
    drive(257_204, false).await;
}
