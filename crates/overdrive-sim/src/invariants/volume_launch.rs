//! Seeded node-local persistent volume launch invariants (GH #348, ADR-0317, ADR-0318).
//!
//! Three invariants of the feature delta's § Required boundary scenarios, each driven over the
//! production launch path and a seed-chosen schedule:
//!
//! * `VolumeSingleWriter` (`single_writer`): no launch reaches the VMM while the volume's claim is
//!   held, and at most one VMM is alive, whatever the schedule of launches, kills, delayed claim
//!   releases, reboots, control-plane restarts and faults.
//! * `VolumeDataSurvivesRestart` (`data_survives_restart`): one volume is ever created for the
//!   workload and every launch attaches that one volume's device, across kills, reboots, a crash
//!   between the create and the attach, and control-plane restarts.
//! * `VolumeNoDrainFailureChargesBudget` (`no_drain_failure_charges_budget`, with the delivered
//!   `assert_eventually!`): a successor that finds the claim still held waits for it, never fails
//!   and never creates a second VMM; once the claim drains, it eventually attaches.
//!
//! The composition is the production one at the driver boundary: `VmDriver::start` over `SimVmm`, a
//! stepped `SimBeaconIo`, `SimClock`, and the pool reaching the driver through
//! `SimVolumePoolProbe::probe` and `VmDriver::new(.., volumes)`. Faults enter only at driven ports:
//! `SimVolumeBacking` (a claim held until a drain, a command failure, a crash between the create
//! and the activation, a node reboot) and `SimClock`. The only observations are each start's result
//! and the recording `Vmm` double's view of the `VmConfig` it was asked to create; no row, claim or
//! volume is seeded. The restart budget is the reconciler's: a start that returns an error is what
//! the action shim charges, so "no failure" is observed as the successor's start not returning one.
//!
//! The schedule derives from the seed alone (`StdRng`), so the seed names the run; a failure prints
//! the seed and the schedule. NOT in `Invariant::ALL` (pending, until the volume launch path and
//! `SimVolumeBacking` land): reachable through `Harness::new().only(..).run(seed)`.

#![allow(
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::cast_possible_truncation,
    clippy::expect_used,
    clippy::similar_names,
    clippy::too_long_first_doc_paragraph,
    reason = "long owner-driving evaluators; doc comments name pinned identifiers"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::BTreeSet;
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
    AllocationSpec, Driver as _, DriverError, DriverPayload, Resources, VmPayload, VolumePayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmError, VmmProbeError,
};
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking, VolumePoolProbe};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmmIdentity, VolumeAttachment,
    VsockPort,
};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, GuestListenerBoard, VmDriver};
use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use tokio::sync::Mutex as LaneMutex;

use crate::adapters::beacon_io::SimBeaconIo;
use crate::adapters::clock::SimClock;
use crate::adapters::guest_stream_dial::SimGuestStreamDial;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::probers::{SimHttpProber, SimTcpProber};
use crate::adapters::vmm::SimVmm;
use crate::adapters::volume_backing::{SimVolumeBacking, SimVolumePoolProbe};
use crate::adapters::{SimCgroupAccounting, SimCgroupFs};
use crate::harness::{InvariantResult, InvariantStatus};

const HOST: &str = "host-0";
const WORKLOAD: &str = "dev-box";
/// The claim bound and `ensure` deadline of the launch path (ADR-0317, ADR-0318), in seconds.
const BOUND_SECS: u64 = 30;
const GUEST_SOURCE_PORT: u32 = 49_152;
/// Consecutive settle steps without an observable change that count as quiescent. The rig runs
/// on its own current-thread runtime with one blocking thread (see [`isolated`]), so simulated time
/// moves only through `SimClock::tick` and a seed fixes the whole trajectory. A launch's chain of
/// file operations advances by at least one operation per step and shows nothing observable until
/// it reaches the VMM, so the quiet run must outlast the longest chain.
const QUIET_STEPS: usize = 64;
/// The most settle steps before the rig gives up: a node that never goes quiet is a rig defect,
/// reported as a panic naming the bound, not a silent return.
const MAX_STEPS: usize = 4096;
const SINGLE_WRITER: &str = "volume-single-writer";
const DATA_SURVIVES: &str = "volume-data-survives-restart";
const NO_DRAIN_FAILURE: &str = "volume-no-drain-failure-charges-budget";

fn workload() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
}

/// The `Vmm` the node composes in simulation: `SimVmm` underneath, recording the volume each
/// `create` was asked to attach, and playing the kernel's verdict on the claim (Cloud Hypervisor's
/// `O_EXCL` open: refused while the claim is held, taken otherwise).
struct RecordingVmm {
    inner: SimVmm,
    pool: SimVolumeBacking,
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
        if config.volume.is_some() {
            match self.pool.claim_state(&workload()).await {
                Ok(ClaimState::Free) => self.pool.hold_claim(&workload()),
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

/// One spawned `VmDriver::start`. It holds its workload's launch lane for the whole start, so the
/// workload's launches run one at a time, as one convergence lane runs them in production
/// (ADR-0102): a launch queued behind another starts only when that one has returned or was ended.
struct Launch {
    index: u32,
    token: LaunchToken,
    start: tokio::task::JoinHandle<Result<(), DriverError>>,
}

/// One node: a `VmDriver` over the recording `Vmm`, a stepped beacon and the volume pool.
struct Rig {
    _tmp: TempDir,
    root: PathBuf,
    vmm: SimVmm,
    recorder: Arc<RecordingVmm>,
    io: Arc<SimBeaconIo>,
    clock: SimClock,
    pool: SimVolumeBacking,
    driver: VmDriver,
    next: u32,
    /// The workload's launch lane: one `start` at a time, in arrival order.
    lane: Arc<LaneMutex<()>>,
    /// Launches the control plane has begun and not ended; a control-plane restart ends them all.
    pending: Vec<Launch>,
    /// Launches whose guest READY has been played.
    readied: BTreeSet<u32>,
}

impl Rig {
    async fn new() -> Result<Self, String> {
        let tmp = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .map_err(|e| format!("stage rootfs: {e}"))?;
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).map_err(|e| format!("stage kernel: {e}"))?;
        let vmm = SimVmm::new();
        let pool = SimVolumeBacking::with_capacity(volume_size_gib(1024));
        let recorder = Arc::new(RecordingVmm {
            inner: vmm.clone(),
            pool: pool.clone(),
            creates: Mutex::new(Vec::new()),
            refused_while_held: AtomicUsize::new(0),
        });
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let clock = SimClock::new();
        let driver = Self::build_driver(&root, &recorder, &io, &clock, &pool).await?;
        Ok(Self {
            _tmp: tmp,
            root,
            vmm,
            recorder,
            io,
            clock,
            pool,
            driver,
            next: 0,
            lane: Arc::new(LaneMutex::new(())),
            pending: Vec::new(),
            readied: BTreeSet::new(),
        })
    }

    async fn build_driver(
        root: &std::path::Path,
        recorder: &Arc<RecordingVmm>,
        io: &Arc<SimBeaconIo>,
        clock: &SimClock,
        pool: &SimVolumeBacking,
    ) -> Result<VmDriver, String> {
        // The pinned seam: the pool reaches the driver as a probe result.
        let volumes = SimVolumePoolProbe::composed(pool.clone())
            .probe()
            .await
            .map_err(|e| format!("the sim pool probe composes: {e}"))?;
        let intake = BeaconIntake::bind(
            Arc::clone(io) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .map_err(|e| format!("the sim beacon listener binds: {e}"))?;
        let wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
        if !wiring.supervisor().open_after_boot() {
            return Err("the EXEC gate did not open".to_owned());
        }
        let board = Arc::new(GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(SimClock::new()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("node-volume-launch").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::clone(&board) as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        Ok(VmDriver::new(
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
        ))
    }

    /// A control-plane restart: every in-flight launch is gone (the process that ran them is), and
    /// the driver is rebuilt over the same node (the same VMM processes, the same pool) and probes
    /// the pool again.
    async fn control_plane_restart(&mut self) -> Result<(), String> {
        for launch in self.pending.drain(..) {
            launch.start.abort();
        }
        self.lane = Arc::new(LaneMutex::new(()));
        // Let the aborted tasks drop before the successor driver exists.
        self.settle().await;
        self.driver =
            Self::build_driver(&self.root, &self.recorder, &self.io, &self.clock, &self.pool)
                .await?;
        Ok(())
    }

    fn cid(index: u32) -> GuestCid {
        GuestCid::new(GUEST_CID_BASE + 2 + index).expect("an in-range guest CID")
    }

    fn spec(&self, index: u32, token: LaunchToken) -> AllocationSpec {
        AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: AllocationId::new(&format!("vol-alloc-{index}")).expect("valid alloc id"),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/dev-box")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                volume: Some(VolumePayload { workload: workload(), size: volume_size_gib(40) }),
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
                Ipv4Addr::new(100, 95, 0, 2 + (index % 200) as u8),
                Ipv4Addr::new(100, 95, 0, 1),
                Self::cid(index),
                token,
                GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
                    .expect("two distinct ports above the reserved range"),
            )),
        }
    }

    /// Spawns `VmDriver::start` of the next allocation, declaring the workload's volume. The task
    /// takes the workload's launch lane first, so it starts only once every earlier launch of the
    /// workload has returned or been ended.
    fn begin(&mut self) -> Launch {
        let index = self.next;
        self.next += 1;
        let token = LaunchToken::from_bytes([0x5a + index as u8; 16]).expect("a non-zero token");
        let spec = self.spec(index, token);
        let driver = self.driver.clone();
        let lane = Arc::clone(&self.lane);
        let start = tokio::spawn(async move {
            let _lane = lane.lock_owned().await;
            driver.start(&spec, &crate::adapters::SimLaunchSupersession::new()).await.map(|_| ())
        });
        Launch { index, token, start }
    }

    /// Begins a launch and keeps it among the control plane's in-flight launches.
    fn begin_tracked(&mut self) {
        let launch = self.begin();
        self.pending.push(launch);
    }

    /// What the rig can observe of the node: a change in it means the launches are still moving.
    fn fingerprint(&self) -> Vec<usize> {
        let finished = self.pending.iter().filter(|launch| launch.start.is_finished()).count();
        vec![
            self.recorder.creates.lock().len(),
            self.recorder.refused_while_held.load(Ordering::SeqCst),
            self.vmm.live_pids().len(),
            self.vmm.vsock_device_holders().len(),
            usize::from(self.io.parked()),
            self.pool.volumes_created(),
            usize::from(self.pool.ensure_parked(&workload())),
            self.pending.len(),
            finished,
            self.readied.len(),
        ]
    }

    /// Runs the launches to quiescence without real time. A step yields so every runnable task
    /// reaches its next wait, then drains the rig runtime's single blocking thread (the launch's
    /// `tokio::fs` calls run there, in order): the drain returns only after every earlier blocking
    /// job completed, so a step ends in a state the schedule alone determines. The node is quiet
    /// when the observable state held still for `QUIET_STEPS` consecutive steps; no wall-clock
    /// time is read.
    ///
    /// # Panics
    ///
    /// When the observable state is still changing after `MAX_STEPS` steps.
    async fn settle(&self) {
        let mut last = self.fingerprint();
        let mut quiet = 0;
        for _ in 0..MAX_STEPS {
            tokio::task::yield_now().await;
            drop(tokio::task::spawn_blocking(|| ()).await);
            let now = self.fingerprint();
            if now == last {
                quiet += 1;
                if quiet >= QUIET_STEPS {
                    return;
                }
            } else {
                quiet = 0;
                last = now;
            }
        }
        panic!("the volume launch rig did not go quiet within {MAX_STEPS} settle steps: {last:?}");
    }

    /// The VMM process holding `cid`'s vsock device, once the launch reached `Vmm::create`.
    fn holder(&self, index: u32) -> Option<u32> {
        self.vmm.vsock_device_holders().get(&Self::cid(index)).copied()
    }

    /// Plays the guest's READY for a launch whose VMM exists, so its start completes.
    async fn play_ready(&self, index: u32, token: LaunchToken, pid: u32) -> Option<u32> {
        let cid = Self::cid(index);
        let key =
            BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(GUEST_SOURCE_PORT + index) };
        let ready = format!("READY pid=1 port=1234 launch={token}");
        self.io.queue_connection(key, pid, &[ready.as_str()]).ok()?;
        self.settle().await;
        self.io.release_accept();
        self.settle().await;
        if !self.io.release_read(key) {
            return None;
        }
        self.settle().await;
        Some(pid)
    }

    /// Settles, then plays READY if the launch's VMM opened its device. `None` if no VMM appears.
    async fn complete_ready(&mut self, launch: &Launch) -> Option<u32> {
        self.settle().await;
        let pid = self.holder(launch.index)?;
        self.readied.insert(launch.index);
        self.play_ready(launch.index, launch.token, pid).await
    }

    /// Plays READY for every in-flight launch whose VMM has appeared since: a launch that waited
    /// behind its lane or the claim reaches its VMM later, and the guest answers it then.
    async fn ready_attached_launches(&mut self) {
        let attached: Vec<(u32, LaunchToken, u32)> = self
            .pending
            .iter()
            .filter(|launch| !self.readied.contains(&launch.index))
            .filter_map(|launch| Some((launch.index, launch.token, self.holder(launch.index)?)))
            .collect();
        for (index, token, pid) in attached {
            self.readied.insert(index);
            self.play_ready(index, token, pid).await;
        }
    }

    /// Advances simulated time by `secs` in one-second steps, letting the launches run to their
    /// next wait after each.
    async fn pass(&self, secs: u64) {
        for _ in 0..secs {
            self.clock.tick(Duration::from_secs(1));
            self.settle().await;
        }
    }

    fn attachments(&self) -> Vec<Option<VolumeAttachment>> {
        self.recorder.creates.lock().clone()
    }

    fn live(&self) -> usize {
        self.vmm.live_pids().len()
    }
}

/// One step of a seeded schedule.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// Launch the next allocation and complete its READY if it can attach.
    Launch,
    /// Kill the live VMM; its claim lingers until a drain.
    Kill,
    /// The dead holder's in-flight I/O drains (a delayed claim release).
    Drain,
    /// Simulated seconds pass.
    Pass(u8),
    /// The node reboots: every VMM is gone, the volume inactive.
    Reboot,
    /// The control plane restarts.
    Restart,
    /// The control plane crashes after the create effect of an `ensure` and before the attach.
    CrashMidEnsure,
    /// The next `lvm` command fails, then a launch runs into it.
    CommandFailure,
}

fn schedule(seed: u64, salt: u64) -> Vec<Step> {
    let mut rng = StdRng::seed_from_u64(seed ^ salt);
    let len = rng.gen_range(4..=12);
    (0..len)
        .map(|_| match rng.gen_range(0..16_u8) {
            0..=3 => Step::Launch,
            4..=5 => Step::Kill,
            6..=7 => Step::Drain,
            8..=10 => Step::Pass(rng.gen_range(1..=40)),
            11 => Step::Reboot,
            12 => Step::Restart,
            13 | 14 => Step::CrashMidEnsure,
            _ => Step::CommandFailure,
        })
        .collect()
}

/// What a schedule left behind, for the oracles.
struct Observed {
    attachments: Vec<Option<VolumeAttachment>>,
    volumes_created: usize,
    refused_while_held: usize,
    max_alive: usize,
}

async fn run_schedule(steps: &[Step]) -> Result<Observed, String> {
    let mut rig = Rig::new().await?;
    let mut max_alive = 0;
    for step in steps {
        match *step {
            Step::Launch => {
                rig.begin_tracked();
                rig.settle().await;
            }
            Step::Kill => {
                if let Some(pid) = rig.vmm.live_pids().first().copied() {
                    rig.vmm.exit_process(pid, Some(9), None);
                }
            }
            Step::Drain => rig.pool.drain(&workload()),
            Step::Pass(secs) => rig.pass(u64::from(secs)).await,
            Step::Reboot => {
                for pid in rig.vmm.live_pids() {
                    rig.vmm.exit_process(pid, Some(9), None);
                }
                rig.pool.node_reboot();
                // A reboot takes the control plane with it.
                rig.control_plane_restart().await?;
            }
            Step::Restart => rig.control_plane_restart().await?,
            Step::CrashMidEnsure => {
                rig.pool.pause_ensure_after_create(&workload());
                rig.begin_tracked();
                rig.settle().await;
                // The restart ends the parked launch together with every other in-flight one.
                rig.control_plane_restart().await?;
                rig.pool.resume_ensure(&workload());
            }
            Step::CommandFailure => {
                rig.pool.fail_next_command();
                rig.begin_tracked();
                rig.settle().await;
            }
        }
        rig.settle().await;
        rig.ready_attached_launches().await;
        max_alive = max_alive.max(rig.live());
    }
    for launch in rig.pending.drain(..) {
        launch.start.abort();
    }
    Ok(Observed {
        attachments: rig.attachments(),
        volumes_created: rig.pool.volumes_created(),
        refused_while_held: rig.recorder.refused_while_held.load(Ordering::SeqCst),
        max_alive,
    })
}

/// Runs `make()`'s future to completion on a runtime of its own, in its own thread: a current-thread
/// runtime with ONE blocking thread. The launch path's `tokio::fs` calls then complete in order and
/// [`Rig::settle`] can drain them exactly, so the outcome is a function of the seed and not of how
/// quickly a host thread ran. The harness's runtime only awaits the result.
async fn isolated<T, F, Fut>(make: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T>,
{
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("volume-launch-rig".to_owned())
        .spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(1)
                .enable_all()
                .build()
                .map(|runtime| runtime.block_on(make()))
                .map_err(|error| format!("the rig runtime builds: {error}"));
            drop(send.send(outcome));
        })
        .map_err(|error| format!("the rig thread spawns: {error}"))?;
    receive.await.map_err(|_| "the rig thread ended without a result".to_owned())?
}

fn result(name: &str, outcome: Result<(), String>) -> InvariantResult {
    match outcome {
        Ok(()) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Pass,
            tick: 3,
            host: HOST.to_owned(),
            cause: None,
        },
        Err(cause) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Fail,
            tick: 3,
            host: HOST.to_owned(),
            cause: Some(cause),
        },
    }
}

/// `single_writer`: a launch only creates its VMM once the claim was free, and at most one VMM is
/// alive, for any seeded schedule.
pub async fn evaluate_single_writer(seed: u64) -> InvariantResult {
    let steps = schedule(seed, 0x0001);
    let outcome = async {
        let rig_steps = steps.clone();
        let seen = isolated(move || async move { run_schedule(&rig_steps).await }).await??;
        let fail = |what: &str| Err(format!("seed {seed}: {what}; schedule {steps:?}"));
        if seen.attachments.iter().any(Option::is_none) {
            return fail("a launch that declares a volume must hold it through its VMM");
        }
        if seen.max_alive > 1 {
            return fail("two writers were alive at once");
        }
        if seen.refused_while_held > 0 {
            return fail("a launch reached the VMM while the claim was held");
        }
        Ok(())
    }
    .await;
    result(SINGLE_WRITER, outcome)
}

/// `data_survives_restart`: one volume is ever created and every launch attaches its one device,
/// for any seeded schedule.
pub async fn evaluate_data_survives_restart(seed: u64) -> InvariantResult {
    let steps = schedule(seed, 0x0002);
    let outcome = async {
        let rig_steps = steps.clone();
        let seen = isolated(move || async move { run_schedule(&rig_steps).await }).await??;
        let fail = |what: &str| Err(format!("seed {seed}: {what}; schedule {steps:?}"));
        let devices: BTreeSet<_> =
            seen.attachments.iter().flatten().map(|a| a.device.clone()).collect();
        if seen.attachments.iter().any(Option::is_none) {
            return fail("every launch must attach the volume's one device");
        }
        if seen.volumes_created > 1 {
            return fail("a second volume was created for the workload");
        }
        if !seen.attachments.is_empty() && seen.volumes_created == 0 {
            return fail("a launch attached a volume nobody created");
        }
        if devices.len() > 1 {
            return fail("launches attached different devices");
        }
        Ok(())
    }
    .await;
    result(DATA_SURVIVES, outcome)
}

/// `no_drain_failure_charges_budget` and the `assert_eventually!` a replacement attaches: for a
/// drain delay chosen by the seed inside the 30 s bound, the successor of a killed VMM neither
/// fails nor creates a second VMM while the claim is held, and eventually attaches once it drains.
pub async fn evaluate_no_drain_failure_charges_budget(seed: u64) -> InvariantResult {
    let delay = StdRng::seed_from_u64(seed ^ 0x0003).gen_range(0..BOUND_SECS);
    let outcome = isolated(move || async move {
        let fail = |what: &str| Err(format!("seed {seed}: {what}; drain delay {delay} s"));
        let mut rig = Rig::new().await?;
        let first = rig.begin();
        let Some(pid) = rig.complete_ready(&first).await else {
            return fail("the first launch must reach its VMM");
        };
        if first.start.is_finished() && !matches!(first.start.await, Ok(Ok(()))) {
            return fail("the first launch must start");
        }
        if rig.attachments().first().is_none_or(Option::is_none) {
            return fail("the first launch must attach the volume");
        }
        rig.vmm.exit_process(pid, Some(9), None);
        let second = rig.begin();
        rig.pass(delay).await;
        // The successor has had the chance to act before it is judged to have waited.
        rig.settle().await;
        if second.start.is_finished() {
            return fail(
                "the successor ended before the claim drained: a drain failure is charged",
            );
        }
        if rig.attachments().len() != 1 {
            return fail("the replacement must wait for the claim, not create a second VMM");
        }
        rig.pool.drain(&workload());
        rig.pass(2).await;
        if rig.complete_ready(&second).await.is_none() {
            return fail("the replacement must eventually attach once the claim drains");
        }
        if !second.start.is_finished() {
            return fail("the replacement's start must complete once its guest is ready");
        }
        match second.start.await {
            Ok(Ok(())) => Ok(()),
            _ => fail("no failure may be charged inside the bound"),
        }
    })
    .await
    .and_then(|inner| inner);
    result(NO_DRAIN_FAILURE, outcome)
}
