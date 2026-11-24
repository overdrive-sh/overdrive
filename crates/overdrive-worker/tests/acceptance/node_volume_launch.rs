//! Node-local persistent volume (GH #348, ADR-0317, ADR-0318, ADR-0311): the VM
//! launch path's two gates, `VolumeEnsure` (G-1) and `VolumeClaim` (G-2), and the
//! three seeded invariants of the feature delta's § Required boundary scenarios.
//!
//! Driving port: `VmDriver::start` (through the `Driver` trait) over `SimVmm` and a
//! stepped `SimBeaconIo`, composed exactly as the node composes it: the pool arrives
//! through `SimVolumePoolProbe::probe` and `VmDriver::new(.., volumes, std::sync::Arc::new(overdrive_sim::SimVmHostState::new()))`. Faults enter
//! only at driven ports: `SimVolumeBacking` (no space, command failure, a failing `claim_state`,
//! a stalled command, a claim held until it drains, a node reboot) and `SimClock`.
//! Every observation is the start's result or the recording `Vmm` double's view of the
//! `VmConfig` it was asked to create; no row, claim or volume is seeded.
//!
//! The reason an allocation row carries, and the budget charge, are the one total
//! conversion `TransitionReason::from(&DriverStartFailure)` (covered in
//! `overdrive-core`) plus the existing shim write for every start failure.
//!
//! Pending markers: each pending test names the assertion it exists to make. The driver does
//! not yet read `VmPayload.volume`, and `SimVolumeBacking` is a scaffold that creates nothing
//! (`ensure` fails typed, `claim_state` reports `NotFound`, faults are ignored), so a launch that
//! declares a volume still boots a VMM with no second disk and the test fails on its own oracle.
//! Sizes come from `testing::volume`, never the grammar, so no test here depends on a parser that
//! is not written. The three seeded invariants of the feature delta (`single_writer`,
//! `data_survives_restart`, `no_drain_failure_charges_budget`) are DST catalogue invariants in
//! `overdrive-sim` (`crate::invariants::volume_launch`), not tests in this file.
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
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver as _, DriverError, DriverPayload, DriverStartClass,
    Resources, VmPayload, VmStartFailure, VolumePayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmError, VmmProbeError,
};
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking, VolumePoolProbe};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VOLUME_DISK_SERIAL, VmConfig, VmConfinement, VmmIdentity,
    VolumeAttachment, VsockPort,
};
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeSize, VolumeUnavailableCause};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{
    SimCgroupAccounting, SimCgroupFs, SimVmm, SimVolumeBacking, SimVolumePoolProbe,
};
use overdrive_testing::placement;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, GuestListenerBoard, VmDriver};
use parking_lot::Mutex;
use tempfile::TempDir;

placement!(host_pure);

const GUEST_SOURCE_PORT: u32 = 49_152;
/// The launch's claim bound and `ensure` deadline (ADR-0317, ADR-0318), in seconds.
const BOUND_SECS: u64 = 30;
const WORKLOAD: &str = "dev-box";

fn gib(count: u64) -> VolumeSize {
    volume_size_gib(count)
}

fn workload() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
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
    clock: SimClock,
    pool: Option<SimVolumeBacking>,
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
        let clock = SimClock::new();
        let driver =
            Self::build_driver(&root, &recorder, &io, &clock, pool.as_ref(), &on_node).await;
        Self { _tmp: tmp, root, vmm, recorder, io, clock, pool, driver, next: 0 }
    }

    async fn build_driver(
        root: &std::path::Path,
        recorder: &Arc<RecordingVmm>,
        io: &Arc<SimBeaconIo>,
        clock: &SimClock,
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
            Arc::new(SimCgroupFs::new()),
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

    /// A control-plane restart: the driver is rebuilt over the same node (the same VMM
    /// processes, the same pool) and probes the pool again; every in-flight launch is gone.
    async fn control_plane_restart(&mut self, on_node: &PoolOnNode) {
        self.driver = Self::build_driver(
            &self.root,
            &self.recorder,
            &self.io,
            &self.clock,
            self.pool.as_ref(),
            on_node,
        )
        .await;
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

    /// Spawns `VmDriver::start` of the next allocation, declaring a volume of `volume_gib` when given.
    fn begin(&mut self, volume_gib: Option<u64>) -> Launch {
        let index = self.next;
        self.next += 1;
        let token = LaunchToken::from_bytes([0x5a + index as u8; 16]).expect("a non-zero token");
        let spec = self.spec(index, token, volume_gib.map(gib));
        let driver = self.driver.clone();
        let start = tokio::spawn(async move {
            driver
                .start(&spec, &overdrive_sim::adapters::SimLaunchSupersession::new())
                .await
                .map(|_| ())
        });
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

    /// Waits until the launch's VMM opened its device, then has that VMM end by itself with
    /// `exit_code` before the guest reports READY: the VMM's own ending wins the boot race.
    async fn vmm_exits_during_start(&self, launch: &Launch, exit_code: i32) {
        let cid = Self::cid(launch.index);
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pid) = self.vmm.vsock_device_holders().get(&cid) {
                    break *pid;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the launch reached Vmm::create");
        assert!(self.vmm.exit_process(pid, Some(exit_code), None), "the VMM was live");
        self.settle().await;
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

    /// Each launch's volume disk as `(device, guest serial)`; the serial is read as text, so the
    /// observation does not depend on the serial grammar.
    fn attached(&self) -> Vec<Option<(PathBuf, String)>> {
        self.attachments()
            .into_iter()
            .map(|attachment| attachment.map(|a| (a.device, VOLUME_DISK_SERIAL.to_owned())))
            .collect()
    }

    /// Lets simulated time pass, a second at a time up to `bound_secs`, until the launch ends,
    /// and returns its result. A launch still running at the bound is the failure of the test
    /// that expected it to end.
    async fn end_of(&self, launch: Launch, bound_secs: u64, what: &str) -> Result<(), DriverError> {
        for _ in 0..=bound_secs {
            if launch.start.is_finished() {
                return launch.result().await;
            }
            self.pass(1).await;
        }
        // A loaded host may not have scheduled the launch's last wake-up yet: real time, not
        // simulated, bounds the grace.
        for _ in 0..1_000 {
            if launch.start.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            launch.start.is_finished(),
            "{what}: the launch was still running after {bound_secs} simulated seconds"
        );
        launch.result().await
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

fn volume_failure(what: &str, result: Result<(), DriverError>) -> VolumeUnavailableCause {
    match result {
        Err(DriverError::StartRejected { failure }) => match failure.class {
            DriverStartClass::Vm(VmStartFailure::VolumeUnavailable { cause }) => cause,
            other => panic!("{what} must fail VolumeUnavailable, got {other:?}"),
        },
        other => panic!("{what} must fail VolumeUnavailable, got {other:?}"),
    }
}

fn pool(capacity_gib: u64) -> SimVolumeBacking {
    SimVolumeBacking::with_capacity(gib(capacity_gib))
}

// ----- Gate available (boundary 1) --------------------------------------------------------

/// S-VOL-40: `ensure` succeeds, the claim is free: the VMM is created once, with the
/// volume as its second disk, whose device is the one `ensure` returned and whose serial
/// is the fixed `ovd-volume`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_declared_volume_is_ensured_and_attached_as_the_second_disk() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    node.complete_ready(&launch, Duration::from_secs(10)).await;
    launch.result().await.expect("a launch with a free claim must start");
    assert_eq!(sim.volumes_created(), 1, "a declared volume must be created once by the launch");
    let device = sim.ensure(&workload(), gib(40)).await.expect("ensure is idempotent");
    assert_eq!(
        node.attached(),
        vec![Some((device, VOLUME_DISK_SERIAL.to_owned()))],
        "the VMM must attach the ensured device as its volume disk"
    );
}

/// Boundary 6: a workload with no `[vm.volume]` makes no volume call and attaches no
/// second disk, on a node with no pool composed. A GUARD: it holds before the launch
/// path learns about volumes and must keep holding.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_workload_without_a_volume_launches_unchanged_on_a_node_with_no_pool() {
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let launch = node.begin(None);
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the VMM was created");
    launch.result().await.expect("a volumeless launch succeeds");
    assert_eq!(node.attachments(), vec![None], "no second disk");
}

/// Boundary 6: on a node whose pool is composed, a workload with no volume still never
/// touches it: nothing is created. A GUARD: it holds before the launch path and the sim
/// exist (nothing creates a volume), and it must keep holding once both do.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_composed_pool_is_untouched_by_a_workload_without_a_volume() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(None);
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the VMM was created");
    launch.result().await.expect("a volumeless launch succeeds");
    assert_eq!(node.attachments(), vec![None]);
    assert_eq!(sim.volumes_created(), 0, "no volume is created for a volumeless workload");
}

// ----- Gate unavailable (boundary 2): each cause is typed, charges the budget, attaches nothing ---

/// S-VOL-41: the driver holds no pool (`volumes: None`, the one route to `PoolAbsent`) (the boot probe found no labelled
/// partition) and the workload declares a volume.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_a_node_with_no_pool_fails_a_declared_volume_pool_absent() {
    let mut node = Node::new(Some(pool(1024)), PoolOnNode::NotComposed).await;
    let launch = node.begin(Some(40));
    let result =
        node.end_of(launch, BOUND_SECS + 5, "a declared volume on a node with no pool").await;
    let cause = volume_failure("a declared volume on a node with no pool", result);
    assert_eq!(cause, VolumeUnavailableCause::PoolAbsent);
    assert!(node.attachments().is_empty(), "no VMM is created");
}

/// S-VOL-41: not enough free extents: typed, naming the declared size and the free space,
/// nothing created, no VMM.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_no_free_extent_fails_insufficient_space_naming_both_figures() {
    let sim = pool(10);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    let what = "a declared volume that does not fit";
    let cause = volume_failure(what, node.end_of(launch, BOUND_SECS + 5, what).await);
    assert_eq!(
        cause,
        VolumeUnavailableCause::InsufficientSpace {
            needed: gib(40),
            free: ByteCount::new(10 * 1024 * 1024 * 1024),
        }
    );
    assert_eq!(sim.volumes_created(), 0, "nothing is created");
    assert!(node.attachments().is_empty());
}

/// S-VOL-41: an `lvm` command fails: typed as a command failure naming the operation
/// that was in progress; no VMM.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_a_failing_lvm_command_fails_naming_the_operation() {
    let sim = pool(1024);
    sim.fail_next_command();
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    let what = "a declared volume whose lvm command fails";
    let cause = volume_failure(what, node.end_of(launch, BOUND_SECS + 5, what).await);
    assert!(
        matches!(cause, VolumeUnavailableCause::PoolCommandFailed { .. }),
        "a failing lvm command must be a command failure naming the operation, got {cause:?}"
    );
    assert!(node.attachments().is_empty());
}

/// S-SUP-02 / ADR-0317: the driver holds no `ensure` timer. The adapter owns the 30 s deadline, so
/// a launch parked inside `ensure` stays there however much simulated time passes: 60 s later it
/// has not finished, no row-shaped outcome exists, and `ensure` is still parked; once `ensure`
/// returns, the launch proceeds (claim poll, then attach). The deadline `Err` itself reaches the
/// driver as any other `PoolCommand` failure and is covered by the failing-command test above and,
/// for the real timer, by the Tier-3 `sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_driver_holds_no_ensure_timer_a_parked_ensure_outlives_the_deadline() {
    let sim = pool(1024);
    sim.pause_ensure_after_create(&workload());
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    for _ in 0..1_000 {
        if sim.ensure_parked(&workload()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        sim.ensure_parked(&workload()),
        "the launch must park inside ensure before simulated time passes"
    );
    node.pass(2 * BOUND_SECS).await;
    assert!(
        !launch.start.is_finished(),
        "the driver has no ensure timer: the launch is still waiting on ensure after {} simulated seconds",
        2 * BOUND_SECS
    );
    assert!(sim.ensure_parked(&workload()), "ensure is still parked");
    assert_eq!(sim.claim_state_calls(), 0, "no claim poll while ensure has not returned");
    assert!(node.attachments().is_empty(), "no VMM while ensure has not returned");
    sim.resume_ensure(&workload());
    node.complete_ready(&launch, Duration::from_secs(10)).await.expect("the launch attaches");
    launch.result().await.expect("the launch proceeds once ensure returns");
}

/// S-VOL-41b (F-3): any `Err` from `claim_state` ends the launch at once as
/// `PoolCommandFailed { operation: Claim }`: not polled again until the 30 s bound, not a `Held`,
/// no VMM. Only `Ok(Held)` is polled. The volume `ensure` just created stays (nothing is deleted).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_a_claim_state_error_ends_the_launch_at_once_as_a_claim_command_failure() {
    let sim = pool(1024);
    sim.fail_next_claim_state();
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    // Five simulated seconds, far inside the 30 s claim bound: a launch that polled on is still running.
    for _ in 0..5 {
        if launch.start.is_finished() {
            break;
        }
        node.pass(1).await;
    }
    for _ in 0..200 {
        if launch.start.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        launch.start.is_finished(),
        "a launch whose claim_state fails must end at once, not poll on towards the claim bound"
    );
    let what = "a declared volume whose claim_state fails";
    let cause = volume_failure(what, launch.result().await);
    assert_eq!(
        cause,
        VolumeUnavailableCause::PoolCommandFailed { operation: VolumeOperation::Claim },
        "a claim_state error must end the launch as a Claim command failure"
    );
    assert!(node.attachments().is_empty(), "no VMM is created");
    assert_eq!(sim.volumes_created(), 1, "the ensured volume is left in place");
}

/// S-VOL-42: the claim stays held past the 30 s bound: the launch fails `Held` (a unit cause:
/// it carries no wait duration), attaches nothing, and the volume keeps its holder.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_a_claim_held_past_the_bound_fails_held_and_attaches_nothing() {
    let sim = pool(1024);
    sim.ensure(&workload(), gib(40)).await.expect("the volume whose claim is held must exist");
    sim.hold_claim(&workload());
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    let what = "a declared volume whose claim stays held";
    match volume_failure(what, node.end_of(launch, BOUND_SECS + 5, what).await) {
        VolumeUnavailableCause::Held => {}
        other => panic!("a claim held past the bound must fail Held, got {other:?}"),
    }
    assert!(node.attachments().is_empty(), "no VMM was created");
    assert_eq!(sim.claim_state(&workload()).await.unwrap(), ClaimState::Held);
}

// ----- Boundary scenarios 3 to 5 ----------------------------------------------------------

/// Boundary 3 (unrelated state): once a launch has attached its volume, nothing that
/// happens to the pool reaches the running allocation: stop proceeds and the volume
/// stays, still claimed until its holder drains.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn pool_faults_after_the_launch_do_not_reach_the_running_allocation_or_its_stop() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    let pid = node.complete_ready(&launch, Duration::from_secs(10)).await.expect("created");
    launch.result().await.expect("a launch with a free claim must start");
    assert_eq!(
        sim.volumes_created(),
        1,
        "the launch must have created the volume the pool faults then leave in place"
    );
    sim.exhaust_space();
    sim.fail_next_command();
    assert!(node.vmm.is_live(pid), "the running VMM is unaffected by the pool");
    // Stop through the Driver port, as the action shim reaches it: the handle names the allocation.
    let handle = AllocationHandle {
        alloc: AllocationId::new("vol-alloc-0").expect("valid alloc id"),
        pid: Some(pid),
    };
    let driver = node.driver.clone();
    let stop = tokio::spawn(async move { driver.stop(&handle).await });
    for _ in 0..30 {
        if stop.is_finished() {
            break;
        }
        node.pass(1).await;
    }
    assert!(
        stop.is_finished(),
        "stop proceeds despite the pool faults: it never waits on the pool"
    );
    stop.await
        .expect("the stop task did not panic")
        .expect("Driver::stop reaches the VMM, not the pool");
    assert!(!node.vmm.is_live(pid), "stop ended the VMM");
    assert_eq!(sim.volumes_created(), 1, "the volume stays after the stop");
}

/// Boundary 4 / ADR-0317: a successor whose predecessor's VMM died but whose claim is still
/// held (in-flight I/O draining) waits for the CLAIM, not for the process: nothing fails and
/// nothing is created while it waits, and it attaches the moment the claim drains.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_replacement_waits_for_the_claim_not_for_the_process_exit() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let first = node.begin(Some(40));
    let pid = node.complete_ready(&first, Duration::from_secs(10)).await.expect("created");
    first.result().await.expect("a launch with a free claim must start");
    assert!(node.vmm.exit_process(pid, Some(9), None), "the VMM is killed");
    let second = node.begin(Some(40));
    node.pass(10).await;
    // The successor has had the chance to act before the claim is asserted to hold it back.
    node.settle().await;
    assert_eq!(
        node.attachments().len(),
        1,
        "a replacement must not create a second VMM while the claim is held"
    );
    assert_eq!(node.recorder.refused_while_held.load(Ordering::SeqCst), 0);
    sim.drain(&workload());
    node.pass(2).await;
    node.complete_ready(&second, Duration::from_secs(10)).await.expect("the successor attaches");
    second.result().await.expect("the successor launches after the drain");
    assert_eq!(sim.volumes_created(), 1, "the same volume");
}

/// Boundary 5: after a node reboot the volume is present but inactive; the next launch's
/// `ensure` activates the SAME volume and attaches it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn after_a_node_reboot_the_next_launch_reactivates_the_same_volume() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let first = node.begin(Some(40));
    let pid = node.complete_ready(&first, Duration::from_secs(10)).await.expect("created");
    first.result().await.expect("a launch with a free claim must start");
    let device =
        node.attachments()[0].clone().expect("the first launch must attach a volume disk").device;
    assert!(node.vmm.exit_process(pid, Some(9), None));
    sim.node_reboot();
    assert!(sim.claim_state(&workload()).await.is_err(), "inactive: no device node");
    let second = node.begin(Some(40));
    node.complete_ready(&second, Duration::from_secs(10)).await.expect("the successor attaches");
    second.result().await.expect("the successor launches");
    assert_eq!(sim.volumes_created(), 1, "reactivation creates nothing");
    assert_eq!(
        node.attachments()[1].clone().expect("the successor must attach a volume disk").device,
        device
    );
}

/// Boundary 5: the control plane crashes after `ensure` created the volume and before the
/// VMM spawned; the next launch's `ensure` finds it and proceeds: one volume, never two.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_control_plane_restart_between_create_and_attach_reuses_the_volume() {
    let sim = pool(1024);
    sim.pause_ensure_after_create(&workload());
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let lost = node.begin(Some(40));
    for _ in 0..1_000 {
        if sim.ensure_parked(&workload()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        sim.ensure_parked(&workload()),
        "the launch must park after the create effect of its ensure"
    );
    lost.start.abort();
    node.control_plane_restart(&PoolOnNode::Composed).await;
    sim.resume_ensure(&workload());
    let next = node.begin(Some(40));
    node.complete_ready(&next, Duration::from_secs(10)).await.expect("the successor attaches");
    next.result().await.expect("the successor launches");
    assert_eq!(sim.volumes_created(), 1, "never a second volume");
}

// ----- The VMM's own busy-disk exit (ADR-0319) ---------------------------------------------

/// ADR-0319: the VMM refuses the volume disk's exclusive open (exit 111) while the claim
/// poll saw it free. With a volume attached that is a held volume, the same cause as a claim
/// held past the bound, and the launch attaches nothing further.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn sad_a_vmm_that_exits_busy_at_start_with_a_volume_fails_held() {
    let sim = pool(1024);
    let mut node = Node::new(Some(sim.clone()), PoolOnNode::Composed).await;
    let launch = node.begin(Some(40));
    node.vmm_exits_during_start(&launch, 111).await;
    let what = "a volume launch whose VMM exits busy";
    let cause = volume_failure(what, node.end_of(launch, BOUND_SECS + 5, what).await);
    assert_eq!(cause, VolumeUnavailableCause::Held);
    assert_eq!(node.attachments().len(), 1, "one VMM was created and none replaced it");
}

/// The same exit status without a volume is not a held volume: 111 means a busy disk only when
/// a volume disk was attached, so the launch takes the generic unreported-exit failure.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_vmm_exit_111_without_a_volume_is_the_generic_unreported_exit() {
    let mut node = Node::new(None, PoolOnNode::NotComposed).await;
    let launch = node.begin(None);
    node.vmm_exits_during_start(&launch, 111).await;
    let result =
        node.end_of(launch, BOUND_SECS + 5, "a volumeless launch whose VMM exits 111").await;
    let Err(DriverError::StartRejected { failure }) = result else {
        panic!("a VMM exit during start must reject the start, got {result:?}");
    };
    assert_eq!(
        failure.class,
        DriverStartClass::Vm(VmStartFailure::GuestExitUnreported {
            vmm_exit_code: Some(111),
            vmm_signal: None,
        }),
    );
}
