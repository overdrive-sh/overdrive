//! Node-local persistent volume (GH #348, ADR-0348): `VmDriver::stop` and the
//! start-failure cleanup over a VMM process that is killed but never reaped.
//!
//! Contract: `Vmm::terminate` waits `VMM_REAP_BOUND` for the reap after SIGKILL and reports
//! `VmTermination::KilledNotReaped`; the driver treats that as a completed stop, cleans up what it
//! can, and leaves the scope to reclamation. The stuck process keeps the volume claim and the
//! vsock device (the guest address) until it is reaped.
//!
//! Owner path: the production `VmDriver` (through the `Driver` trait), composed as the node
//! composes it over `SimVmm`, `SimBeaconIo` (through the beacon harness), `SimClock`,
//! `SimCgroupFs`, `SimVmHostState` and `SimVolumeBacking`. Fault stimulus: the pinned
//! `SimVmm::inject_unreapable_process` and its reap, `SimVmm::exit_process`. Oracles: the stop
//! and start results, the driver's own `status` / `live_allocations`, the exit-event channel, and
//! the driven ports' state (`SimVmm::vsock_device_holders` / `is_unreaped`, the volume claim, the
//! run and clone directories, the `SimVmHostState` scope).
//!
//! `kill_scope` calls are observed through the scripted-failure queue: a `ResourceBusy` queued on
//! `SimVmHostState` is consumed by the first `kill_scope` call, so a direct probe call afterwards
//! returns `Ok` exactly when the driver called `kill_scope` before it.
//!
//! The reap bound lives in the `Vmm` adapter (and its sim), so the driver already satisfies the
//! stop clauses here: they are guards. The scope-removal clauses (ADR-0351) are in
//! `vm_driver_scope_single_remover.rs`.
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

use std::collections::BTreeSet;
use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_core::guest_transport::GuestCid;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver as _, DriverError, DriverPayload, DriverStartClass,
    ExitEvent, Resources, VmPayload, VmStartFailure, VolumePayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vm_host_state::VmHostState as _;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VMM_REAP_BOUND, VmControl, VmProcess, VmTermination, Vmm, VmmError,
    VmmProbeError,
};
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking as _, VolumePoolProbe};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
    VolumeAttachment,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{
    SimCgroupAccounting, SimCgroupFs, SimVmHostState, SimVmm, SimVolumeBacking, SimVolumePoolProbe,
};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{GuestListenerBoard, VmDriver};
use parking_lot::Mutex;
use tempfile::TempDir;
use tokio::sync::mpsc;

use super::support::{BeaconHarness, GuestSession};

const WORKLOAD: &str = "stuck-box";

pub(super) fn workload() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
}

/// The `Vmm` a node composes in simulation: `SimVmm` underneath, recording the volume each
/// `create` was asked to attach and playing the kernel's verdict on the claim (Cloud
/// Hypervisor's `O_EXCL` open: refused while the claim is held). The claim a VMM takes lasts
/// while its process is live or killed-but-unreaped, as a real process's open file does.
pub(super) struct VolumeVmm {
    inner: SimVmm,
    pool: SimVolumeBacking,
    creates: Mutex<Vec<Option<VolumeAttachment>>>,
    refused_while_held: AtomicUsize,
}

#[async_trait]
impl Vmm for VolumeVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        self.creates.lock().push(config.volume.clone());
        if config.volume.is_some() {
            match self.pool.claim_state(&workload()).await {
                Ok(ClaimState::Free) => {}
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
        let process = self.inner.create(config, cid).await?;
        if config.volume.is_some() {
            let sim = self.inner.clone();
            let pid = process.control.pid;
            self.pool.hold_claim_while_live(&workload(), move || {
                sim.is_live(pid) || sim.is_unreaped(pid)
            });
        }
        Ok(process)
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// One node: a production `VmDriver` over the recording `Vmm`, the beacon harness, one clock
/// shared by the driver and the `SimVmm` (the reap bound runs on it), the host-state port and
/// the volume pool.
pub(super) struct Node {
    pub(super) tmp: TempDir,
    pub(super) clock: SimClock,
    pub(super) sim: SimVmm,
    pub(super) host: SimVmHostState,
    pub(super) pool: SimVolumeBacking,
    pub(super) cgroup_fs: SimCgroupFs,
    pub(super) beacon: BeaconHarness,
    pub(super) recorder: Arc<VolumeVmm>,
    pub(super) driver: VmDriver,
    pub(super) exit_rx: mpsc::Receiver<ExitEvent>,
    pub(super) cgroup_root: PathBuf,
    pub(super) run_dir_root: PathBuf,
    pub(super) clone_staging: PathBuf,
    pub(super) clone_index: PathBuf,
}

impl Node {
    pub(super) async fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        std::fs::write(tmp.path().join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(tmp.path().join("vmlinuz"), &header)
            .expect("stage the synthetic kernel image");

        let clock = SimClock::new();
        let sim = SimVmm::new(Arc::new(clock.clone()));
        let pool = SimVolumeBacking::with_capacity(volume_size_gib(1024));
        let recorder = Arc::new(VolumeVmm {
            inner: sim.clone(),
            pool: pool.clone(),
            creates: Mutex::new(Vec::new()),
            refused_while_held: AtomicUsize::new(0),
        });
        let beacon = BeaconHarness::new(&sim);
        let host = SimVmHostState::new();
        let cgroup_fs = SimCgroupFs::new();
        let wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
        assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
        let board = Arc::new(GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(SimClock::new()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("vm-stop-unreaped").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            Arc::clone(&board) as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        let probe: Arc<dyn VolumePoolProbe> = Arc::new(SimVolumePoolProbe::composed(pool.clone()));
        let volumes = probe.probe().await.expect("the sim probe passes");
        let cgroup_root = tmp.path().join("cgroup");
        let run_dir_root = tmp.path().join("run");
        let clone_staging = tmp.path().join("clone-staging");
        let clone_index = tmp.path().join("clone-index");
        let layout = VmHostLayout {
            cgroup_root: cgroup_root.clone(),
            run_dir_root: run_dir_root.clone(),
            clone_index_dir: clone_index.clone(),
            clone_staging_dir: clone_staging.clone(),
            arch: HostArch::X86_64,
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                1024,
            ),
        };
        let driver = VmDriver::new(
            beacon.vmm(Arc::clone(&recorder) as Arc<dyn Vmm>),
            Arc::new(clock.clone()),
            Arc::new(cgroup_fs.clone()),
            Arc::new(SimCgroupAccounting::new()),
            probe_runner,
            wiring.gate(),
            layout,
            beacon.intake().await,
            board,
            volumes,
            Arc::new(host.clone()),
        );
        let exit_rx = driver.take_exit_receiver().expect("the exit receiver is taken once");
        Self {
            tmp,
            clock,
            sim,
            host,
            pool,
            cgroup_fs,
            beacon,
            recorder,
            driver,
            exit_rx,
            cgroup_root,
            run_dir_root,
            clone_staging,
            clone_index,
        }
    }

    /// A VM allocation spec on guest CID `GUEST_CID_BASE + cid_offset`; it declares the node's
    /// volume when `with_volume`.
    pub(super) fn spec(&self, name: &str, cid_offset: u32, with_volume: bool) -> AllocationSpec {
        AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: AllocationId::new(name).expect("valid alloc id"),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/stuck-box")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                volume: with_volume
                    .then(|| VolumePayload { workload: workload(), size: volume_size_gib(40) }),
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: self.tmp.path().join("vmlinuz"),
                rootfs: self.tmp.path().join("master.img"),
            }),
            resources: Resources {
                cpu: CpuQuantity::from_millis(100),
                memory: ByteSize::new(128 * 1024 * 1024),
            },
            probe_descriptors: Vec::new(),
            transport: Some(super::support::transport(
                Ipv4Addr::new(100, 96, 0, 6 + cid_offset as u8),
                Ipv4Addr::new(100, 96, 0, 5),
                cid_offset,
            )),
        }
    }

    pub(super) fn cid(cid_offset: u32) -> GuestCid {
        super::support::leased_cid(cid_offset)
    }

    /// Lets spawned tasks reach their next wait: yields, then a sliver of real time so blocking
    /// pool work (the run-directory removal) completes.
    pub(super) async fn settle(&self) {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    /// Advances simulated time by `by` in 100 ms steps, letting the driver react after each.
    pub(super) async fn advance(&self, by: Duration) {
        let step = Duration::from_millis(100);
        let mut left = by;
        while !left.is_zero() {
            let now = left.min(step);
            self.clock.tick(now);
            self.settle().await;
            left -= now;
        }
    }

    /// Starts `spec`; its guest reports READY once the VMM exists.
    pub(super) async fn start_running(
        &self,
        spec: &AllocationSpec,
    ) -> (AllocationHandle, GuestSession) {
        let driver = self.driver.clone();
        let spec_owned = spec.clone();
        let start_task = tokio::spawn(async move {
            driver.start(&spec_owned, &overdrive_sim::adapters::SimLaunchSupersession::new()).await
        });
        let guest = self.beacon.dial_ready(spec).await;
        let handle = start_task
            .await
            .expect("start task did not panic")
            .expect("start resolves Ok once the beacon accepts");
        (handle, guest)
    }

    /// Spawns `Driver::stop` of `handle` and lets it reach its first wait.
    pub(super) async fn begin_stop(
        &self,
        handle: &AllocationHandle,
    ) -> tokio::task::JoinHandle<Result<(), DriverError>> {
        let driver = self.driver.clone();
        let handle = handle.clone();
        let stop = tokio::spawn(async move { driver.stop(&handle).await });
        self.settle().await;
        stop
    }

    /// The scope the driver made for `alloc`, seeded into the host-state port with `pid` as its
    /// member: the sim of the kernel directory a real launch creates.
    pub(super) fn seed_scope(&self, alloc: &AllocationId, pid: u32) -> CgroupPath {
        self.host.set_scope(alloc.clone(), BTreeSet::from([pid]));
        CgroupPath::for_alloc(alloc)
    }

    pub(super) fn run_dir_of(&self, alloc: &AllocationId) -> VmRunDir {
        VmRunDir::for_alloc(&self.run_dir_root, alloc)
    }

    pub(super) fn rootfs_plan_of(&self, alloc: &AllocationId) -> RootfsPlan {
        let master = self.tmp.path().join("master.img");
        let bytes = std::fs::metadata(&master).expect("stat the master rootfs").len();
        RootfsPlan::for_alloc(master, bytes, alloc, &self.clone_staging, &self.clone_index)
    }

    /// The volume attachments the VMM was asked to create, in order.
    pub(super) fn attachments(&self) -> Vec<Option<VolumeAttachment>> {
        self.recorder.creates.lock().clone()
    }

    /// Whether a `kill_scope` call reached the host-state port since a `ResourceBusy` was queued
    /// on it by [`Self::queue_busy_probe`]: the queued failure is consumed by the first call, so a
    /// direct probe call sees `Ok` exactly when it was consumed.
    pub(super) async fn kill_scope_was_called(&self, scope: &CgroupPath) -> bool {
        self.host.kill_scope(scope).await.is_ok()
    }

    /// Queues a `ResourceBusy` failure for the next `kill_scope` call.
    pub(super) fn queue_busy_probe(&self) {
        self.host.script_kill_scope_failure(io::ErrorKind::ResourceBusy);
    }
}

/// Awaits a spawned task for at most ten real seconds.
pub(super) async fn join<T>(task: tokio::task::JoinHandle<T>, what: &str) -> T {
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap_or_else(|_| panic!("{what}: the task did not end"))
        .expect("the task did not panic")
}

// ----- ADR-0348 items 1 and 2: the bounded wait, and a completed stop ---------------------

/// Items 1 and 2. A stop of a VMM that is killed but not reaped returns `Ok` once the reap bound
/// has passed, not before, and the shim-visible outcome is a completed stop (`status` reports the
/// allocation gone).
/// A GUARD: the bound lives in the adapter, and the driver already treats the outcome as a
/// completed stop.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_stop_of_an_unreapable_vmm_completes_after_the_reap_bound() {
    let node = Node::new().await;
    node.sim.inject_unreapable_process();
    let spec = node.spec("alloc-unreaped-bound", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");

    let stop = node.begin_stop(&handle).await;
    node.advance(
        VMM_REAP_BOUND.checked_sub(Duration::from_millis(100)).expect("the bound exceeds 100 ms"),
    )
    .await;
    assert!(
        !stop.is_finished(),
        "the stop must wait for the reap until the bound has passed (still waiting at the bound \
         less 100 ms)"
    );
    assert!(node.sim.is_unreaped(pid), "the killed VMM is not reaped");

    node.advance(Duration::from_millis(100)).await;
    let result = join(stop, "the stop at the reap bound").await;
    assert!(result.is_ok(), "a stop over a killed-not-reaped VMM completes Ok: {result:?}");
    assert!(node.sim.is_unreaped(pid), "the stop completing does not reap the process");
    assert!(
        matches!(node.driver.status(&handle).await, Err(DriverError::NotFound { .. })),
        "after the stop the allocation reports gone, as for any stop"
    );
}

// ----- ADR-0348 item 3: the claim and the guest address stay held until the reap ------------

/// Item 3 (S-RB-10 family). After a stop completes over an unreapable VMM the volume claim and
/// the VMM's vsock device (its CID) are still held; the scripted reap releases both.
/// A GUARD: the sim couples both to the unreaped state.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_stop_keeps_the_claim_and_the_guest_address_held_until_the_reap() {
    let node = Node::new().await;
    node.sim.inject_unreapable_process();
    let spec = node.spec("alloc-unreaped-held", 2, true);
    let (handle, _guest) = node.start_running(&spec).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");
    let cid = Node::cid(2);

    let stop = node.begin_stop(&handle).await;
    node.advance(VMM_REAP_BOUND).await;
    join(stop, "the stop at the reap bound").await.expect("the stop completes");

    assert_eq!(
        node.pool.claim_state(&workload()).await.expect("claim state"),
        ClaimState::Held,
        "the stuck process keeps the volume claim after the stop returned"
    );
    assert_eq!(
        node.sim.vsock_device_holders().get(&cid),
        Some(&pid),
        "the stuck process keeps its vsock device (the guest address) after the stop returned"
    );

    assert!(node.sim.exit_process(pid, None, Some(9)), "the scripted reap reaches the process");
    assert!(!node.sim.is_unreaped(pid), "the reap clears the unreaped state");
    assert_eq!(
        node.pool.claim_state(&workload()).await.expect("claim state"),
        ClaimState::Free,
        "the reap releases the volume claim"
    );
    assert!(node.sim.vsock_device_holders().is_empty(), "the reap releases the vsock device");
}

// ----- ADR-0348 item 4: the late reap is silent -----------------------------------------------

/// Item 4. The late reap of a stopped allocation emits no `ExitEvent` and changes nothing the
/// driver reports: its `status` and `live_allocations` are the same before and after.
/// A GUARD: the exit watcher finds the supervision entry already ended by the stop.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_the_late_reap_emits_no_exit_event_and_changes_no_reported_state() {
    let mut node = Node::new().await;
    node.sim.inject_unreapable_process();
    let spec = node.spec("alloc-unreaped-late", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");
    let stop = node.begin_stop(&handle).await;
    node.advance(VMM_REAP_BOUND).await;
    join(stop, "the stop at the reap bound").await.expect("the stop completes");
    assert!(
        matches!(node.exit_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "precondition: the stop itself emitted no exit event"
    );
    let live_before = node.driver.live_allocations();
    let status_before = node.driver.status(&handle).await.map_err(|e| e.to_string());

    assert!(node.sim.exit_process(pid, None, Some(9)), "the scripted reap reaches the process");
    node.advance(Duration::from_secs(1)).await;

    assert!(
        matches!(node.exit_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "the late reap of a stopped allocation must emit no ExitEvent"
    );
    assert_eq!(
        node.driver.live_allocations(),
        live_before,
        "the late reap must not change the allocations the driver reports as held"
    );
    assert_eq!(
        node.driver.status(&handle).await.map_err(|e| e.to_string()),
        status_before,
        "the late reap must not change the allocation's reported state"
    );
}

// ----- ADR-0348 item 5: a replacement waits on the claim ---------------------------------------

/// Item 5 (ADR-0317). A replacement launch of the same volume workload waits on the claim the
/// stuck process still holds: it is not charged (the launch is still waiting) before its claim
/// bound, creates no VMM while the claim is held, and attaches the volume after the reap.
/// A GUARD: the claim wait already exists and the sim couples the claim to the reap.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_replacement_waits_on_the_claim_and_attaches_after_the_reap() {
    let node = Node::new().await;
    node.sim.inject_unreapable_process();
    let first = node.spec("alloc-unreaped-first", 2, true);
    let (handle, _guest) = node.start_running(&first).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");
    let stop = node.begin_stop(&handle).await;
    node.advance(VMM_REAP_BOUND).await;
    join(stop, "the stop at the reap bound").await.expect("the stop completes");
    assert_eq!(node.attachments().len(), 1, "precondition: one VMM was created so far");

    let second = node.spec("alloc-unreaped-second", 3, true);
    let driver = node.driver.clone();
    let second_for_task = second.clone();
    let replacement = tokio::spawn(async move {
        driver.start(&second_for_task, &overdrive_sim::adapters::SimLaunchSupersession::new()).await
    });
    node.settle().await;

    node.advance(Duration::from_secs(29)).await;
    assert!(
        !replacement.is_finished(),
        "the replacement must still be waiting on the claim before its bound: it is not charged"
    );
    assert_eq!(
        node.attachments().len(),
        1,
        "no replacement VMM is created while the stuck process holds the claim"
    );

    assert!(node.sim.exit_process(pid, None, Some(9)), "the scripted reap reaches the process");
    node.advance(Duration::from_millis(500)).await;
    let _guest = node.beacon.dial_ready(&second).await;
    let handle = join(replacement, "the replacement start")
        .await
        .expect("the replacement attaches the volume and starts after the reap");
    assert!(handle.pid.is_some(), "the replacement runs");
    let attachments = node.attachments();
    assert_eq!(attachments.len(), 2, "exactly one replacement VMM was created");
    assert!(
        attachments[1].is_some(),
        "the replacement attaches the volume as its second disk after the reap"
    );
}

// ----- ADR-0348 item 6: no kill_scope after KilledNotReaped -------------------------------------

/// Item 6, stop. After `KilledNotReaped` the stop does not call `VmHostState::kill_scope` (the
/// scope is known to be populated), while the run directory and the clone are removed. The scope
/// stays for reclamation.
/// PENDING: RED until the driver omits the scope kill after `KilledNotReaped` (ADR-0348 item 6). Since
/// ADR-0351 the driver calls `kill_scope` on every cleanup, so this guard no longer holds today. The
/// call count is read from the scripted-failure queue (see the module doc).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_stop_makes_no_scope_kill_after_a_vmm_that_is_not_reaped() {
    let node = Node::new().await;
    node.sim.inject_unreapable_process();
    let spec = node.spec("alloc-unreaped-stop-scope", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");
    let scope = node.seed_scope(&spec.alloc, pid);
    node.queue_busy_probe();

    let stop = node.begin_stop(&handle).await;
    node.advance(VMM_REAP_BOUND).await;
    join(stop, "the stop at the reap bound").await.expect("the stop completes");

    assert!(
        !node.run_dir_of(&spec.alloc).path().exists(),
        "the run directory is removed even though the VMM is not reaped"
    );
    assert!(
        !node.rootfs_plan_of(&spec.alloc).clone_dest().exists(),
        "the rootfs clone is removed even though the VMM is not reaped"
    );
    assert!(
        !node.kill_scope_was_called(&scope).await,
        "no VmHostState::kill_scope call may follow KilledNotReaped in a stop"
    );
    assert!(node.host.has_scope(&spec.alloc), "the populated scope is left to reclamation");
}

/// Item 6, start-failure cleanup. A launch whose guest never beacons ends at the boot deadline;
/// its cleanup terminates the VMM, which is not reaped. No `kill_scope` call follows, the run
/// directory and the clone are removed, and the reported failure stays the boot deadline.
/// PENDING: RED until the driver omits the scope kill after `KilledNotReaped` (ADR-0348 item 6). Since
/// ADR-0351 the driver calls `kill_scope` on every cleanup, so this guard no longer holds today. The
/// call count is read from the scripted-failure queue (see the module doc).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_start_failure_cleanup_makes_no_scope_kill_after_a_vmm_that_is_not_reaped() {
    let node = Node::new().await;
    node.sim.inject_unreapable_process();
    let spec = node.spec("alloc-unreaped-start-fail", 2, false);
    let driver = node.driver.clone();
    let spec_owned = spec.clone();
    let start = tokio::spawn(async move {
        driver.start(&spec_owned, &overdrive_sim::adapters::SimLaunchSupersession::new()).await
    });
    let pid = node.beacon.next_created_pid().await;
    let scope = node.seed_scope(&spec.alloc, pid);
    node.queue_busy_probe();

    for _ in 0..120 {
        if start.is_finished() {
            break;
        }
        node.advance(Duration::from_secs(1)).await;
    }
    let error = join(start, "the start past its boot deadline")
        .await
        .expect_err("a guest that never beacons fails the start");
    assert!(
        matches!(
            &error,
            DriverError::StartRejected { failure }
                if matches!(
                    failure.class,
                    DriverStartClass::Vm(VmStartFailure::BootDeadlineExceeded { .. })
                )
        ),
        "the reported failure stays the boot deadline: {error:?}"
    );

    assert!(node.sim.is_unreaped(pid), "precondition: the cleanup's kill left the VMM unreaped");
    assert!(
        !node.run_dir_of(&spec.alloc).path().exists(),
        "the run directory is removed by the start-failure cleanup"
    );
    assert!(
        !node.rootfs_plan_of(&spec.alloc).clone_dest().exists(),
        "the rootfs clone is removed by the start-failure cleanup"
    );
    assert!(
        !node.kill_scope_was_called(&scope).await,
        "no VmHostState::kill_scope call may follow KilledNotReaped in a start-failure cleanup"
    );
    assert!(node.host.has_scope(&spec.alloc), "the populated scope is left to reclamation");
}

// ----- The reapable guard ---------------------------------------------------------------------

/// A VMM that is reaped normally never sees the new outcome: its stop completes within the guest
/// shutdown deadline, well before the reap bound, the process is gone, the claim and the guest
/// address are released at once and the stop emits no exit event. Unchanged by the feature.
/// A GUARD.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_stop_of_a_reapable_vmm_is_unchanged() {
    let mut node = Node::new().await;
    let spec = node.spec("alloc-reapable", 2, true);
    let (handle, _guest) = node.start_running(&spec).await;
    let pid = handle.pid.expect("the handle carries the VMM pid");

    let stop = node.begin_stop(&handle).await;
    node.advance(Duration::from_secs(2)).await;
    assert!(
        stop.is_finished(),
        "a reapable VMM's stop completes by the shutdown deadline, not after the reap bound"
    );
    join(stop, "the stop of a reapable VMM").await.expect("the stop completes");

    assert!(!node.sim.is_unreaped(pid), "a reapable VMM is never killed-not-reaped");
    assert!(!node.sim.is_live(pid), "the process is gone");
    assert!(node.sim.vsock_device_holders().is_empty(), "its vsock device is released at once");
    assert_eq!(
        node.pool.claim_state(&workload()).await.expect("claim state"),
        ClaimState::Free,
        "its volume claim is released at once"
    );
    assert!(
        matches!(node.exit_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "a stop emits no exit event"
    );
}
