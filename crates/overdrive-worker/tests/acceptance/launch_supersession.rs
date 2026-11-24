//! Node-local persistent volume (GH #348, ADR-0334): a launch is superseded.
//!
//! Scenario: a `workload stop`, a `workload restart` or node shutdown raises the convergence
//! lane's supersession signal while `VmDriver::start` is waiting. The launch abandons its wait
//! at the observation points of the feature delta's § Superseding a launch (SD-4), tears down
//! exactly what the start-failure path tears down, and returns `DriverError::StartSuperseded`.
//!
//! Contract shape: `Driver::start(&spec, &dyn LaunchSupersession)`, driven through the pinned
//! driving port over `VmDriver` composed as the node composes it (`SimVmm`, `SimBeaconIo`,
//! `SimClock`, `SimCgroupFs`, `SimVolumeBacking` through the pool probe). The signal is a
//! `SimLaunchSupersession` the test raises. Faults enter only at driven ports: `SimVolumeBacking`
//! (a parked or stalled `ensure`, a held claim, a parked `claim_state`), `SimBeaconIo` (a failed
//! accept, through a decorator at the `BeaconIo` port), `SimCgroupFs` (a failing scope removal),
//! `SimClock`.
//!
//! Oracle: the start's result, and the driven ports' own state after it: the VMM double's live
//! processes and `create` calls, the cgroup double's entries, the run, clone and index
//! directories, the driver's `beacon_observation`, and the volume double's call counts and
//! volumes. The zero-grace terminate of the VMM is not observable at the `SimVmm` port (it
//! ignores the grace); the oracle for it is that no VMM is left alive.
//!
//! Pending markers: a test of behaviour DELIVER has not built keeps its full body and is marked
//! `#[should_panic(expected = ..)]` with the assertion of this body that fails today. Today the
//! driver ignores the signal (`_supersession`) and does not read the volume. A test that holds
//! today and must keep holding says GUARD. Tests that need a volume also need the volume launch
//! path and a working `SimVolumeBacking` (a scaffold today); they fail on the first assertion
//! that names the missing launch behaviour.
//!
//! Evidence tier: in-memory acceptance (layer 2).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "test fixtures assert their preconditions"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver as _, DriverError, DriverPayload, Resources, VmPayload, VolumePayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmError, VmmProbeError,
};
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking, VolumePoolProbe};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VOLUME_DISK_SERIAL, VmConfig, VmConfinement, VmmIdentity,
    VolumeAttachment, VsockPort,
};
use overdrive_core::volume::VolumeSize;
use overdrive_sim::adapters::SimLaunchSupersession;
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{
    SimCgroupAccounting, SimCgroupFs, SimOp, SimVmm, SimVolumeBacking, SimVolumePoolProbe,
};
use overdrive_testing::placement;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, GuestListenerBoard, VmDriver};
use parking_lot::Mutex;
use tempfile::TempDir;

placement!(host_pure);

const GUEST_SOURCE_PORT: u32 = 49_152;
/// The launch's claim bound and boot deadline (ADR-0317), in seconds.
const BOUND_SECS: u64 = 30;
const WORKLOAD: &str = "dev-box";

fn gib(count: u64) -> VolumeSize {
    volume_size_gib(count)
}

fn workload() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
}

fn pool(capacity_gib: u64) -> SimVolumeBacking {
    SimVolumeBacking::with_capacity(gib(capacity_gib))
}

/// A `BeaconIo` decorator over the simulated beacon: `fail_accept` makes the pending and every
/// later `accept` fail (the listener dying under the driver). A driven-port fault; every other
/// call is the simulation's own.
struct FaultableBeaconIo {
    inner: Arc<SimBeaconIo>,
    failed: tokio::sync::watch::Sender<bool>,
}

impl FaultableBeaconIo {
    fn fail_accept(&self) {
        self.failed.send_replace(true);
    }
}

#[async_trait]
impl BeaconIo for FaultableBeaconIo {
    async fn bind(&self, port: VsockListenPort) -> Result<VsockPort, std::io::Error> {
        self.inner.bind(port).await
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), std::io::Error> {
        let mut failed = self.failed.subscribe();
        if *failed.borrow() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "listener failed",
            ));
        }
        tokio::select! {
            biased;
            _ = failed.wait_for(|failed| *failed) => {
                Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "listener failed"))
            }
            accepted = self.inner.accept() => accepted,
        }
    }

    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, std::io::Error> {
        self.inner.read_line(socket).await
    }

    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), std::io::Error> {
        self.inner.write_line(socket, line).await
    }

    async fn close(&self, socket: BeaconSocket) {
        self.inner.close(socket).await;
    }
}

/// The `Vmm` the node composes in simulation: `SimVmm` underneath, recording the volume
/// each `create` was asked to attach, and playing the kernel's verdict on the claim
/// (Cloud Hypervisor's `O_EXCL` open: refused while the claim is held, taken otherwise).
struct RecordingVmm {
    inner: SimVmm,
    pool: Option<SimVolumeBacking>,
    creates: Mutex<Vec<Option<VolumeAttachment>>>,
    refused_while_held: AtomicUsize,
}

#[async_trait]
impl Vmm for RecordingVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        self.creates.lock().push(config.volume.clone());
        if let (Some(_), Some(pool)) = (&config.volume, &self.pool) {
            match pool.claim_state(&workload()).await {
                Ok(ClaimState::Free) => pool.hold_claim(&workload()),
                Ok(ClaimState::Held) => {
                    self.refused_while_held.fetch_add(1, Ordering::SeqCst);
                    return Err(VmmError::Create {
                        detail: "the volume's exclusive claim is held (EBUSY)".to_owned(),
                    });
                }
                Err(error) => {
                    return Err(VmmError::Create { detail: format!("no device node: {error}") });
                }
            }
        }
        self.inner.create(config, cid).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// One node: a `VmDriver` over the recording `Vmm`, a stepped beacon and the volume pool.
struct Node {
    _tmp: TempDir,
    root: PathBuf,
    vmm: SimVmm,
    recorder: Arc<RecordingVmm>,
    io: Arc<SimBeaconIo>,
    gate: Arc<FaultableBeaconIo>,
    clock: SimClock,
    cgroup_fs: SimCgroupFs,
    driver: VmDriver,
    next: u32,
}

/// What the node's boot probe hands the driver.
enum PoolOnNode {
    /// No labelled partition: the probe yields `None` and the driver holds no pool.
    NotComposed,
    /// The probe yields a pool over this simulated backing.
    Composed,
}

impl Node {
    async fn new(pool: Option<SimVolumeBacking>, on_node: PoolOnNode) -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
        let vmm = SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
        let recorder = Arc::new(RecordingVmm {
            inner: vmm.clone(),
            pool: pool.clone(),
            creates: Mutex::new(Vec::new()),
            refused_while_held: AtomicUsize::new(0),
        });
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let gate = Arc::new(FaultableBeaconIo {
            inner: Arc::clone(&io),
            failed: tokio::sync::watch::channel(false).0,
        });
        let clock = SimClock::new();
        let cgroup_fs = SimCgroupFs::new();
        let driver = Self::build_driver(
            &root,
            &recorder,
            &gate,
            &clock,
            &cgroup_fs,
            pool.as_ref(),
            &on_node,
        )
        .await;
        Self { _tmp: tmp, root, vmm, recorder, io, gate, clock, cgroup_fs, driver, next: 0 }
    }

    async fn build_driver(
        root: &std::path::Path,
        recorder: &Arc<RecordingVmm>,
        io: &Arc<FaultableBeaconIo>,
        clock: &SimClock,
        cgroup_fs: &SimCgroupFs,
        pool: Option<&SimVolumeBacking>,
        on_node: &PoolOnNode,
    ) -> VmDriver {
        let probe: Arc<dyn VolumePoolProbe> = match (on_node, pool) {
            (PoolOnNode::Composed, Some(pool)) => {
                Arc::new(SimVolumePoolProbe::composed(pool.clone()))
            }
            _ => Arc::new(SimVolumePoolProbe::absent()),
        };
        let volumes = probe.probe().await.expect("the sim probe passes");
        let intake = BeaconIntake::bind(
            Arc::clone(io) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds");
        let wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
        assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
        let board = Arc::new(GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(SimClock::new()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("node-volume-launch").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            Arc::clone(&board) as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        VmDriver::new(
            Arc::clone(recorder) as Arc<dyn Vmm>,
            Arc::new(clock.clone()),
            Arc::new(cgroup_fs.clone()),
            Arc::new(SimCgroupAccounting::new()),
            probe_runner,
            wiring.gate(),
            VmHostLayout {
                cgroup_root: root.join("cgroup"),
                run_dir_root: root.join("run"),
                clone_index_dir: root.join("clone-index"),
                clone_staging_dir: root.join("clone-staging"),
                arch: HostArch::X86_64,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                    1024,
                ),
            },
            intake,
            board,
            volumes,
            std::sync::Arc::new(overdrive_sim::SimVmHostState::new()),
        )
    }

    fn cid(index: u32) -> GuestCid {
        GuestCid::new(GUEST_CID_BASE + 2 + index).expect("an in-range guest CID")
    }

    fn spec(&self, index: u32, token: LaunchToken, volume: Option<VolumeSize>) -> AllocationSpec {
        AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: AllocationId::new(&format!("vol-alloc-{index}")).expect("valid alloc id"),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/dev-box")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                volume: volume.map(|size| VolumePayload { workload: workload(), size }),
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: self.root.join("vmlinuz"),
                rootfs: self.root.join("master.img"),
            }),
            resources: Resources {
                cpu: CpuQuantity::from_millis(100),
                memory: ByteSize::new(128 * 1024 * 1024),
            },
            probe_descriptors: Vec::new(),
            transport: Some(GuestTransportAssignment::new(
                Ipv4Addr::new(100, 95, 0, 2 + index as u8),
                Ipv4Addr::new(100, 95, 0, 1),
                Self::cid(index),
                token,
                GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
                    .expect("two distinct ports above the reserved range"),
            )),
        }
    }

    /// Spawns `VmDriver::start` of the next allocation under `signal`, declaring a volume of
    /// `volume_gib` when given.
    fn begin(&mut self, volume_gib: Option<u64>, signal: &Arc<SimLaunchSupersession>) -> Launch {
        let index = self.next;
        self.next += 1;
        let token = LaunchToken::from_bytes([0x5a + index as u8; 16]).expect("a non-zero token");
        let spec = self.spec(index, token, volume_gib.map(gib));
        let driver = self.driver.clone();
        let signal = Arc::clone(signal);
        let start = tokio::spawn(async move { driver.start(&spec, &*signal).await.map(|_| ()) });
        Launch { index, token, start }
    }

    async fn settle(&self) {
        for _ in 0..1_000 {
            tokio::time::sleep(Duration::from_millis(1)).await;
            if self.io.parked() {
                break;
            }
        }
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    /// Waits until the launch's VMM opened its device (the launch reached `Vmm::create`),
    /// then plays the guest's READY so the start completes. `None` if no VMM appears
    /// (the launch ended first, or is still waiting).
    async fn complete_ready(&self, launch: &Launch, within: Duration) -> Option<u32> {
        let cid = Self::cid(launch.index);
        let pid = tokio::time::timeout(within, async {
            loop {
                if let Some(pid) = self.vmm.vsock_device_holders().get(&cid) {
                    break *pid;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .ok()?;
        let key = BeaconConnKey {
            peer_cid: cid,
            peer_port: VsockPort::new(GUEST_SOURCE_PORT + launch.index),
        };
        let ready = format!("READY pid=1 port=1234 launch={}", launch.token);
        self.io.queue_connection(key, pid, &[ready.as_str()]).expect("queued");
        self.settle().await;
        self.io.release_accept();
        self.settle().await;
        assert!(self.io.release_read(key), "READY is readable");
        self.settle().await;
        Some(pid)
    }

    /// Advances simulated time by `secs` in one-second steps, letting the launch react: each
    /// step yields to the runtime and gives the launch a moment of real time to reach its next
    /// wait, so an assertion about what a launch has NOT done is made after it had the chance.
    async fn pass(&self, secs: u64) {
        for _ in 0..secs {
            self.clock.tick(Duration::from_secs(1));
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// The volume attachments the VMM was asked to create, in order.
    fn attachments(&self) -> Vec<Option<VolumeAttachment>> {
        self.recorder.creates.lock().clone()
    }

    /// Waits (real time) until the launch's VMM exists: the launch is then in its boot wait.
    async fn wait_vmm(&self, index: u32) -> u32 {
        let cid = Self::cid(index);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = self.vmm.vsock_device_holders().get(&cid) {
                    break *pid;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the launch created its VMM and is in its boot wait")
    }

    /// Waits (real time, no simulated time) until `condition` holds; `false` when it never did.
    async fn eventually(&self, condition: impl Fn() -> bool) -> bool {
        for _ in 0..1_000 {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        condition()
    }

    /// Waits for the launch to end after `sim_secs` simulated seconds, ticked one at a time
    /// (0 ticks none: the launch must end on the signal alone, with the clock untouched), then a
    /// real-time grace. The launch still running is the failure of the test that expected it
    /// to have been abandoned; `what` names the launch in the message.
    async fn end_of_abandoned(
        &self,
        launch: Launch,
        sim_secs: u64,
        what: &str,
    ) -> Result<(), DriverError> {
        for _ in 0..sim_secs {
            if launch.start.is_finished() {
                break;
            }
            self.pass(1).await;
        }
        for _ in 0..400 {
            if launch.start.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            launch.start.is_finished(),
            "{what} must be abandoned on the supersession signal, not run on to its own bound"
        );
        launch.result().await
    }

    /// As `end_of_abandoned`, and the result must be the typed `StartSuperseded`.
    async fn expect_superseded(&self, launch: Launch, sim_secs: u64, what: &str) {
        let result = self.end_of_abandoned(launch, sim_secs, what).await;
        assert!(
            matches!(result, Err(DriverError::StartSuperseded)),
            "{what} must end StartSuperseded, got {result:?}"
        );
    }

    /// Nothing of launch `index` is left on the host or in a driven port: no VMM alive, no run
    /// directory, rootfs clone or index link, no cgroup scope, no beacon record.
    fn assert_nothing_left(&self, index: u32, what: &str) {
        let alloc = format!("vol-alloc-{index}");
        assert!(self.vmm.live_pids().is_empty(), "{what} must leave no VMM alive");
        let names_alloc = |dir: &str| -> bool {
            std::fs::read_dir(self.root.join(dir)).is_ok_and(|entries| {
                entries.flatten().any(|e| e.file_name().to_string_lossy().contains(&alloc))
            })
        };
        assert!(!names_alloc("run"), "{what} must remove the run directory");
        assert!(!names_alloc("clone-staging"), "{what} must remove the rootfs clone");
        assert!(!names_alloc("clone-index"), "{what} must remove the clone index link");
        assert!(
            !self.cgroup_fs.snapshot().keys().any(|path| path.to_string_lossy().contains(&alloc)),
            "{what} must remove the cgroup scope"
        );
        assert!(
            self.driver.beacon_observation().live.is_empty(),
            "{what} must clear the beacon record"
        );
    }

    /// The launch created no host artifact at all: no `Vmm::create` was ever asked for.
    fn assert_nothing_created(&self, index: u32, what: &str) {
        assert!(self.attachments().is_empty(), "{what} must not create a VMM");
        self.assert_nothing_left(index, what);
    }

    /// The scope path the cgroup double keys launch `index`'s removal on.
    fn scope_path(&self, index: u32) -> PathBuf {
        let alloc = AllocationId::new(&format!("vol-alloc-{index}")).expect("valid alloc id");
        CgroupPath::for_alloc(&alloc).resolve(&self.root.join("cgroup"))
    }
}

/// One spawned `VmDriver::start`.
struct Launch {
    index: u32,
    token: LaunchToken,
    start: tokio::task::JoinHandle<Result<(), DriverError>>,
}

impl Launch {
    async fn result(self) -> Result<(), DriverError> {
        tokio::time::timeout(Duration::from_secs(20), self.start)
            .await
            .expect("the start ended")
            .expect("the start task did not panic")
    }
}

fn unraised() -> Arc<SimLaunchSupersession> {
    Arc::new(SimLaunchSupersession::new())
}

// ----- Points 5 and 6: the guest-boot waits (every VM launch, volume or not) --------------

/// Point 5: the signal is raised while the launch waits for the guest's READY. The launch
/// abandons the wait at once (simulated time is never advanced), tears down through the
/// start-failure path (VMM terminated, clone and link, scope, run directory, beacon record) and
/// returns `StartSuperseded`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_during_the_guest_boot_wait_abandons_the_wait_and_tears_the_vmm_down() {
    let what = "a launch superseded during its guest-boot wait";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    assert_eq!(node.attachments().len(), 1, "the VMM was created before the signal");
    signal.raise();
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_left(0, what);
}

/// Point 5: the signal and the boot deadline arrive at the same instant (the signal is raised
/// and the clock is ticked to the deadline in one step). Superseded wins over the typed boot
/// failure.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_signal_wins_over_the_boot_deadline_at_the_same_instant() {
    let what = "a launch superseded at its boot deadline";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    signal.raise();
    node.clock.tick(Duration::from_secs(BOUND_SECS));
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_left(0, what);
}

/// Point 5, GUARD (holds today and must keep holding): a launch whose boot race already selected
/// `accept_ready` before the signal is an ordinary `Ok`; a signal raised after is ignored, the
/// VMM stays alive and its beacon record stays.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_launch_that_won_its_boot_race_before_the_signal_is_an_ordinary_success() {
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    let pid = node.complete_ready(&launch, Duration::from_secs(10)).await.expect("VMM created");
    launch.result().await.expect("a launch that received READY starts");
    signal.raise();
    node.settle().await;
    assert!(node.vmm.is_live(pid), "a started VMM is not torn down by a late signal");
    assert_eq!(
        node.driver.beacon_observation().live.len(),
        1,
        "a started launch keeps its beacon record"
    );
}

/// Point 6: the guest-boot race ends on its beacon-error arm (the listener failed under the
/// driver), the launch waits the second boot wait for the VMM's own end, and the signal is
/// raised there. The launch abandons that wait at once and tears down.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_in_the_second_boot_wait_abandons_it_and_tears_down() {
    let what = "a launch superseded in its second boot wait";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    node.gate.fail_accept();
    node.settle().await;
    assert!(!launch.start.is_finished(), "the beacon-error arm waits for the VMM's own end");
    signal.raise();
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_left(0, what);
}

/// Point 6: the signal and the second boot deadline arrive at the same instant. Superseded wins
/// over the unclassified beacon-accept rejection.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_signal_wins_over_the_second_boot_deadline_at_the_same_instant() {
    let what = "a launch superseded at its second boot deadline";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    node.gate.fail_accept();
    node.settle().await;
    signal.raise();
    node.clock.tick(Duration::from_secs(BOUND_SECS));
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_left(0, what);
}

/// A teardown that fails while abandoning is not hidden: the scope removal fails, so `start`
/// returns the unclassified `StartRejected` carrying the failure, never `StartSuperseded`. The
/// VMM is still terminated.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_failing_teardown_is_reported_as_a_start_rejection_never_as_superseded() {
    let what = "a superseded launch whose teardown fails";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    node.cgroup_fs.inject_error(
        SimOp::RemoveDir,
        node.scope_path(0),
        std::io::ErrorKind::PermissionDenied,
    );
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    signal.raise();
    let result = node.end_of_abandoned(launch, 0, what).await;
    match result {
        Err(DriverError::StartRejected { failure }) => assert!(
            failure.detail.to_lowercase().contains("permission denied"),
            "the rejection must carry the teardown failure, got {:?}",
            failure.detail
        ),
        other => panic!("{what} must end StartRejected carrying the failure, got {other:?}"),
    }
    assert!(node.vmm.live_pids().is_empty(), "the VMM is terminated even when the scope is not");
}

/// Point 1 for a launch with no volume: the signal is raised before `start` is called (a lane
/// born raised). The launch returns `StartSuperseded` having created nothing.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_raised_before_start_returns_superseded_having_created_nothing() {
    let what = "a launch whose signal was raised before start";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let launch = node.begin(None, &Arc::new(SimLaunchSupersession::raised()));
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_created(0, what);
}

/// GUARD (holds today): a launch whose signal is never raised is exactly the existing launch,
/// with no volume: one `create`, no second disk, `Ok`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_launch_whose_signal_is_never_raised_is_the_existing_launch() {
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let signal = unraised();
    let launch = node.begin(None, &signal);
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the VMM was created");
    launch.result().await.expect("an unsuperseded launch succeeds");
    assert!(!overdrive_core::traits::LaunchSupersession::is_superseded(&*signal), "never raised");
    assert_eq!(node.attachments(), vec![None], "one create, no second disk");
}

/// GUARD (holds today): a signal raised for another launch's instance, one this launch was never
/// given, does not affect it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_signal_raised_for_another_launch_does_not_affect_this_one() {
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let other = SimLaunchSupersession::raised();
    let launch = node.begin(None, &unraised());
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the VMM was created");
    launch.result().await.expect("a launch whose own signal is unraised succeeds");
    node.settle().await;
    assert_eq!(node.vmm.live_pids().len(), 1, "the VMM stays alive");
    assert!(overdrive_core::traits::LaunchSupersession::is_superseded(&other));
}

/// Two launches on one driver, each with its own signal: raising launch A's abandons A only.
/// B keeps waiting and completes `Ok` when its READY arrives; B's VMM and beacon record stay.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_abandons_only_its_own_launch() {
    let what = "the launch whose own signal was raised";
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let (signal_a, signal_b) = (unraised(), unraised());
    let a = node.begin(None, &signal_a);
    let b = node.begin(None, &signal_b);
    node.wait_vmm(0).await;
    node.wait_vmm(1).await;
    node.settle().await;
    signal_a.raise();
    node.expect_superseded(a, 0, what).await;
    assert_eq!(node.vmm.live_pids().len(), 1, "only launch A's VMM is torn down");
    node.complete_ready(&b, Duration::from_secs(10)).await.expect("B's VMM is still there");
    b.result().await.expect("the other launch is unaffected and starts");
    assert_eq!(node.driver.beacon_observation().live.len(), 1, "B's beacon record stays");
}

// ----- Points 1 to 4 and 9: a volume launch -----------------------------------------------

/// Point 1 for a volume launch: the signal is raised before `start`. The launch returns
/// `StartSuperseded`; `ensure` is never called, the claim is never polled, nothing is created.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_volume_launch_superseded_before_ensure_creates_nothing_and_never_calls_the_pool() {
    let what = "a volume launch whose signal was raised before start";
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40), &Arc::new(SimLaunchSupersession::raised()));
    node.expect_superseded(launch, 0, what).await;
    assert_eq!(sim.ensure_calls(), 0, "{what} must never call ensure");
    assert_eq!(sim.claim_state_calls(), 0, "{what} must never poll the claim");
    assert_eq!(sim.volumes_created(), 0, "{what} must create no volume");
    node.assert_nothing_created(0, what);
}

/// Point 2: the signal is raised while `ensure` is in flight, and `ensure` then ends either `Ok`
/// or with a pool command failure (`fail_next_command`; the real 30 s deadline is the same
/// `PoolCommand` `Err` at the driver, and is covered by the Tier-3 sad test
/// `sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure`). `ensure` is never
/// interrupted (the launch waits for it), then the launch abandons: `StartSuperseded` wins over
/// either outcome, no row, no `VolumeUnavailable`, nothing charged, the claim is never polled, no
/// VMM is created. On `Ok` the ensured volume stays.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_during_ensure_waits_for_ensure_then_abandons_whatever_ensure_returns() {
    for ensure_fails in [false, true] {
        let what = if ensure_fails {
            "a launch superseded while ensure was in flight and then failed"
        } else {
            "a launch superseded while ensure was in flight and then succeeded"
        };
        let sim = pool(1024);
        sim.pause_ensure_after_create(&workload());
        if ensure_fails {
            sim.fail_next_command();
        }
        let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
        let signal = unraised();
        let launch = node.begin(Some(40), &signal);
        assert!(
            node.eventually(|| sim.ensure_parked(&workload())).await,
            "the launch must be inside ensure before the signal is raised"
        );
        signal.raise();
        node.settle().await;
        assert!(!launch.start.is_finished(), "ensure is never interrupted by the signal");
        sim.resume_ensure(&workload());
        node.expect_superseded(launch, 0, what).await;
        if !ensure_fails {
            assert_eq!(sim.volumes_created(), 1, "the ensured volume stays");
        }
        assert_eq!(sim.claim_state_calls(), 0, "no claim poll after the signal");
        node.assert_nothing_created(0, what);
    }
}

/// A volume launch's setup used by the claim-wait tests: the launch's first claim poll is held
/// in flight, and the claim is taken once `ensure` has activated the volume (only an active
/// volume has a device node a VMM can hold). The poll then reads `Held` and the launch waits.
async fn begin_held_claim_wait(
    node: &mut Node,
    sim: &SimVolumeBacking,
    signal: &Arc<SimLaunchSupersession>,
) -> Launch {
    sim.pause_ensure_after_create(&workload());
    sim.park_next_claim_state();
    let launch = node.begin(Some(40), signal);
    assert!(
        node.eventually(|| sim.ensure_parked(&workload())).await,
        "the launch must be inside ensure before its volume is activated"
    );
    sim.resume_ensure(&workload());
    assert!(
        node.eventually(|| sim.claim_state_parked()).await,
        "the launch must be inside its first claim poll once ensure has activated the volume"
    );
    sim.hold_claim(&workload());
    sim.release_claim_state();
    assert!(
        node.eventually(|| sim.claim_state_calls() >= 1).await,
        "the launch must poll the claim after ensure"
    );
    launch
}

/// Point 3: the signal is raised while the claim is `Held` and the driver sleeps on the poll
/// interval. The launch returns at once: the clock is not advanced to the interval's end, and
/// `claim_state` is not called again.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_during_the_claim_poll_sleep_returns_at_once_without_polling_again() {
    let what = "a launch superseded while it sleeps between claim polls";
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = begin_held_claim_wait(&mut node, &sim, &signal).await;
    node.pass(3).await;
    assert!(!launch.start.is_finished(), "a Held claim is waited for");
    let polls = sim.claim_state_calls();
    signal.raise();
    node.expect_superseded(launch, 0, what).await;
    node.settle().await;
    assert_eq!(sim.claim_state_calls(), polls, "{what} must not poll the claim again");
    assert_eq!(sim.volumes_created(), 1, "the ensured volume stays");
    node.assert_nothing_created(0, what);
}

/// Point 3: the claim is still `Held` when the 30 s bound is reached and the signal arrives in
/// the same step. Superseded wins over `VolumeUnavailable { Held }`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_signal_wins_over_a_claim_held_at_the_bound() {
    let what = "a launch superseded at its claim bound";
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = begin_held_claim_wait(&mut node, &sim, &signal).await;
    node.pass(BOUND_SECS - 1).await;
    assert!(!launch.start.is_finished(), "a Held claim is waited for until the bound");
    signal.raise();
    node.clock.tick(Duration::from_secs(1));
    node.expect_superseded(launch, 0, what).await;
    node.assert_nothing_created(0, what);
}

/// Point 4: the signal is raised while a `claim_state` call is in flight and that call returns
/// `Free`. The launch observes the signal before the first host artifact: nothing is created.
/// The in-flight call is not interrupted.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_signal_during_the_last_claim_poll_creates_nothing_after_the_claim_wait() {
    let what = "a launch superseded as its claim wait ends";
    let sim = pool(1024);
    sim.park_next_claim_state();
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = node.begin(Some(40), &signal);
    assert!(
        node.eventually(|| sim.claim_state_parked()).await,
        "the launch must be inside claim_state before the signal is raised"
    );
    signal.raise();
    node.settle().await;
    assert!(!launch.start.is_finished(), "an in-flight claim_state is never interrupted");
    sim.release_claim_state();
    node.expect_superseded(launch, 0, what).await;
    assert_eq!(sim.volumes_created(), 1, "the ensured volume stays");
    node.assert_nothing_created(0, what);
}

/// Point 5 for a volume launch: the signal is raised in the guest-boot wait of a launch that
/// attached the volume. The VMM is torn down (none left alive) and the volume is kept.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_volume_launch_superseded_in_its_boot_wait_tears_the_vmm_down_and_keeps_the_volume() {
    let what = "a volume launch superseded during its guest-boot wait";
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = node.begin(Some(40), &signal);
    node.wait_vmm(0).await;
    node.settle().await;
    signal.raise();
    node.expect_superseded(launch, 0, what).await;
    assert_eq!(sim.volumes_created(), 1, "{what} must keep the ensured volume");
    node.assert_nothing_left(0, what);
}

/// Point 5 for a volume launch: the VMM exits busy (111) and the signal is raised in the same
/// instant, neither seen by the launch before it is next polled. The stop wins over the busy-disk
/// exit: the launch ends `StartSuperseded`, never `VolumeUnavailable { Held }` or an unreported
/// exit; the VMM is torn down and the volume is kept.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_stop_beats_a_busy_disk_exit_at_the_same_instant_in_the_boot_wait() {
    let what = "a volume launch whose VMM exits busy as the signal is raised";
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = node.begin(Some(40), &signal);
    let pid = node.wait_vmm(0).await;
    node.settle().await;
    // Both events land with no await between them: the single-threaded runtime cannot poll the
    // launch task until `expect_superseded` yields, so the boot race sees both ready at once.
    assert!(node.vmm.exit_process(pid, Some(111), None), "the VMM was live");
    signal.raise();
    node.expect_superseded(launch, 0, what).await;
    assert_eq!(sim.volumes_created(), 1, "{what} must keep the ensured volume");
    node.assert_nothing_left(0, what);
}

/// A volume launch whose signal is never raised is the volume launch of the launch path: `ensure`
/// once, the claim polled, one VMM with the volume as its second disk.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_volume_launch_whose_signal_is_never_raised_is_the_ordinary_volume_launch() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let signal = unraised();
    let launch = node.begin(Some(40), &signal);
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the VMM was created");
    launch.result().await.expect("an unsuperseded volume launch succeeds");
    assert_eq!(sim.ensure_calls(), 1, "an unsuperseded volume launch must ensure the volume once");
    assert!(sim.claim_state_calls() >= 1, "the claim is polled");
    let attachments = node.attachments();
    assert_eq!(attachments.len(), 1, "one VMM is created");
    let attachment = attachments[0]
        .as_ref()
        .expect("the VMM's VmConfig.volume is Some: the volume is its second disk");
    // `ensure` is idempotent: this reads the device the launch's own call returned.
    let device = sim.ensure(&workload(), gib(40)).await.expect("ensure is idempotent");
    assert_eq!(
        attachment.device, device,
        "the VMM attaches the ensured device ({VOLUME_DISK_SERIAL}) as its second disk"
    );
    assert!(
        attachment.device.to_string_lossy().starts_with("/dev/mapper/overdrive-ovd--"),
        "the attached device is the device-mapper node of the volume: {:?}",
        attachment.device
    );
}
