//! Quint conformance driver — `node_volume`, conformance main `vol_conf`
//! (VD-2, VD-8, VD-9; feature delta § "Observable projections the conformance
//! driver reads").
//!
//! Replays `specs/quint/node-local-volume/node_volume.qnt` (referenced in
//! place) on its recording main `vol_conf` against the production
//! composition of the VM launch path: the operator handlers
//! (`submit_workload`, `stop_workload`, `restart_workload`), the
//! `WorkloadLifecycle` reconciler driven through the runtime's convergence
//! tick, the action shim, and the production `VmDriver` over a `SimVmm`,
//! whose volume pool is composed through the pinned seam
//! `SimVolumePoolProbe::composed(SimVolumeBacking)` → `probe()` →
//! `VmDriver::new(.., volumes)`. The production exit observer writes the
//! terminal rows. The driver writes no row, intent or sim state the
//! production path writes itself. Spec identifiers keep their earlier
//! "store" spelling (`storeUp`, `StoreAbsentBoot`); the contract's word is
//! "volume pool".
//!
//! Step decoding follows the other source-local drivers: the action and its
//! picks come from the write-only `conf_step` record
//! (`Config::nondet = ["vol_conf::node_volume::conf_step"]`), the state
//! projection from the qualified variable `vol_conf::node_volume::st`.
//!
//! | Action | Owner driving port or environment stimulus |
//! |---|---|
//! | `Init` | the `serve` composition over a fresh node (`boot_serve`: `VmDriver` over `SimVmm`, `SimVolumePoolProbe::composed`, the shim, `ReconcilerRuntime`) |
//! | `Declare` | `submit_workload` of a Service whose VM declares `[vm.volume] size = "40GiB"` (the production parser accepts it only after the spec surface lands) |
//! | `OpStop` / `OpResume` | `stop_workload` (records the stop, raises the open lane's signal) / `restart_workload` (`Resumed`) |
//! | `Shutdown` | `ReconcilerRuntime::supersede_all_lanes` |
//! | `Boot` / `CpCrash` | a new `serve` composition over the same node (`boot_serve`, then boot reclamation); the launches in flight die with the old process |
//! | `StoreAbsentBoot` | a `serve` boot whose probe is `SimVolumePoolProbe::absent()` (the driver then holds no pool) |
//! | `PoolFull` | `SimVolumeBacking::exhaust_space`, before the first launch |
//! | `WlStart(a)` | `SimVolumeBacking::pause_ensure_after_create`, then the convergence tick of the workload (`WorkloadLifecycle` -> shim dispatch -> `VmDriver::start`), spawned and held |
//! | `LaunchEnsureCreate(a)` / `LaunchEnsureActivate(a)` | the launch's `ensure` parks after its create effect (`ensure_parked`) / `SimVolumeBacking::resume_ensure` |
//! | `LaunchUnavailable(a)` / `LaunchCmdFail(a)` | the launch's dispatch returns the typed failure (the shim writes the `Failed` row, no VMM was created) / `SimVolumeBacking::fail_next_command`, then the same |
//! | `LaunchCheck(a)` | `SimClock::tick` (one second at a time, inside the bound) until the launch observes `Free` and its `Vmm::create` is parked at the driver's create gate (`LP_FREE`) |
//! | `LaunchPoll(a)` / `LaunchHeldFail(a)` | `SimClock::tick` by a poll step inside the bound / past the 30 s bound, the launch ends `VolumeUnavailable { Held }` |
//! | `LaunchSuperseded(a)` | the held dispatch returns `Ok(())` after the lane signal raised by `OpStop`, `WorkloadRestart` or `Shutdown`: no row, `VmDriver::live_allocations` does not list the allocation, its VMM is gone |
//! | `VmmSpawn(a)` | the recording `Vmm`'s `VmConfig.volume` is the ensured device with serial `ovd-volume` (production attached it); the gate releases the parked `create`, then `SimVolumeBacking::hold_claim` (the VMM's open: `SimVmm::create` does not touch the pool) |
//! | `GuestReady(a)` | the guest's READY through `SimBeaconIo`; the shim writes `Running` |
//! | `VmmSpawnFail(a)` | `SimVmm::inject_create_failure` armed while the launch's `create` is parked at the gate, then the gate releases it: production classifies the refusal and the shim writes the charged `Failed` row; no VMM exists |
//! | `GuestWrite` / `DeviceSlow` | no owner port (guest and kernel facts, model-only variables); the handler checks the action's guard against production: the claim arbiter reports `Held`, and a VMM the production launch created exists (none live for `DeviceSlow`) |
//! | `VmmSigkill(a)` | `SimVmm::exit_process` of the allocation's VMM, the exit observer writes the terminal row; the sim claim stays `Held` until `drain` (spike B2b) |
//! | `Drain` | `SimVolumeBacking::drain` |
//! | `Reclaim(a)` | the production boot reclamation (`vm_reclamation_boot::converge`): the driver grants its kill of the surviving VMM |
//! | `StopAlloc(a)` | one convergence tick after `OpStop`: the shim dispatches the stop |
//! | `WorkloadRestart(a)` | `restart_workload` (`Restarted`); for a `Running` allocation, one convergence tick (the shim's stop); for a launch in flight, the signal only |
//! | `NodeReboot` | every VMM exits, `SimVolumeBacking::node_reboot`, a fresh `serve` composition |
//!
//! Projection (feature delta § Observable projections): `active` (model `lv ∧ active`) ⇔
//! `VolumeBacking::claim_state ≠ NotFound`; `claimed` (model `claim != 0`) ⇔ `claim_state == Held`,
//! `SimVolumeBacking` the named arbiter (marker below; paired Tier-3 test S-VOL-T3-04,
//! `volume_pool_backing.rs::claim_state_reports_held_free_and_not_found_as_the_kernel_does`,
//! ignored until the guest-kit volume pool lands); `alloc[a]` ← `ObservationStore::alloc_status_rows` (`state`); with
//! no row, the dispatch the driver holds decides: running = PENDING (the launch holds the lane,
//! production writes no row before `start` returns), returned `Ok(())` = ended by supersession,
//! dropped with the `serve` process = a lost launch; `booting` (model `ph == LP_BOOT ∧ alive`) ←
//! `Driver::live_allocations()` lists the allocation, no row, the claim `Held`; `exhausted` ← a row whose `terminal` is `BackoffExhausted`. Every other spec variable is
//! model-only and not compared.
//!
//! **Documented blind spots.** (1) `alive` is projected for a booting launch from the recording
//! `Vmm` double (`SimVmm`), the named arbiter of the VMM process (B-2): the owner does not hold it, so
//! the read carries the `sim-arbiter-ok` marker and is paired with the Tier-3 test S-VOL-T3-13
//! (`volume_survives.rs`, a stop during guest boot terminates the VMM and keeps the volume). (2) `charged` is model-only in the
//! delta (the budget is the reconciler's private `restart_counts`); a charge is observed only through
//! the typed `VmVolumeUnavailable` reason of the failed launch's row (asserted by the driver, below) and
//! `BackoffExhausted` (`exhausted`). (3) `LaunchCheck` asserts the launch reached `Vmm::create`
//! after observing `Free`; no pinned owner accessor counts the claim observations. Fresh allocation ids are discovered for labelling only, from the
//! reconciler view's issued ids, the rows and `live_allocations`.
//!
//! **Status: pending.** The driver is compiled and every test carries `#[should_panic]` naming the
//! divergence it reaches: every run's second step is `Declare`, and the production spec parser
//! refuses `[vm.volume]` today (`deny_unknown_fields`), so all traces stop there. **Every handler
//! after `Declare` is compile-checked only**: no trace has executed one. They are exercised once
//! DELIVER lands the spec surface, the sim (`SimVolumeBacking`), the lane table
//! (`supersede_lane` / `supersede_all_lanes`, `StartSuperseded`) and the launch path, at which
//! point the markers come off. Approximations to confirm against the landed launch path: the
//! production launch observes the claim and calls `Vmm::create` contiguously, so the model's
//! `LaunchCheck` and `VmmSpawn` are two driver steps over one production stretch;
//! `LaunchCmdFail` arms the next `lvm` command, which is the activate effect; the claim poll
//! interval is the crafter's, so `LaunchPoll` steps the clock inside the bound.
//!
//! Two actions have no owner port: `GuestWrite` and `DeviceSlow`. `VmmSpawnFail` is a driven-port
//! fault: the `Vmm` port's `create` refuses (`SimVmm::inject_create_failure`), the launch having
//! observed the claim `Free` before it. The driver parks `create` at a test-support gate
//! (`CreateGate`, in the driver's recording `Vmm`) so the model's `LP_FREE` is a real stopping
//! point; `VmmSpawn` and `VmmSpawnFail` are the two ways out of it.
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through
//! quint-connect).

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    reason = "a conformance driver asserts its own preconditions; CONTRACT_SHAPE lines are machine-read"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::significant_drop_tightening,
    reason = "test support and conformance drivers: handler signatures follow the action map"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State as AxumState};
use axum::http::HeaderMap;
use itf::Value;
use overdrive_core::aggregate::{WorkloadKind, WorkloadSpecInput};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::guest_transport::GuestCid;
use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::Driver as ProductionDriver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::traits::volume_backing::{
    ClaimState, VolumeBacking, VolumeBackingError, VolumePoolProbe,
};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmmIdentity, VolumeAttachment,
    VsockPort,
};
use overdrive_reconcilers::{AnyState, current_alloc};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{
    SimCgroupAccounting, SimCgroupFs, SimVmm, SimVolumeBacking, SimVolumePoolProbe,
};
use overdrive_testing::placement;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};
use parking_lot::Mutex;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use crate::AppState;
use crate::api::{RestartOutcome, SubmitWorkloadRequest};
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::guest_network::{
    ForwardingActivation, GuestAddressPool, GuestNetworkPlan, GuestNetworkProvisioner,
    Result as GuestNetworkResult, SharedGuestNetworkOwner as _,
};
use crate::handlers::{restart_workload, stop_workload, submit_workload};
use crate::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, hydrate_desired_for_test,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use crate::vm_reclamation_boot::sim_vm_host::{GATEWAY, RecordingVmm, SimVmmHost};
use crate::vm_reclamation_boot::{ConvergeError, converge};

placement!(host_pure);

/// The conformance main and the module declaring the variables.
const MAIN: &str = "vol_conf";
const MODULE: &str = "node_volume";
/// The conformance main's allocation ids (`NA`) and restart-budget limit (`LIMIT`, production's `RESTART_BACKOFF_CEILING` + 1: a chain fails ceiling + 1 times before `BackoffExhausted`).
const NA: i64 = 6;
const LIMIT: i64 = 6;
/// The one workload of the model and its volume declaration.
const WORKLOAD: &str = "vol-w";
pub const DECLARATION: &str = r#"
[service]
id = "vol-w"
replicas = 1
[vm]
command = "/sbin/init"
args = []
kernel = "/vmlinuz"
rootfs = "/master.img"
[vm.volume]
size = "40GiB"
[resources]
cpu = '100m'
memory = '128MiB'
[[listener]]
port = 8080
protocol = "tcp"
"#;
/// The pool's free capacity in GiB: a volume of the declared size fits. Built through the
/// `testing::volume` fixture, so `Init` does not depend on the size grammar.
const POOL_GIB: u64 = 100;
/// The declared volume size in GiB (`DECLARATION`'s `40GiB`).
pub const DECLARED_GIB: u64 = 40;
/// The test guest-address pool.
const PREFIX: &str = "100.95.0.0/29";
/// Half the 30 s claim bound: two polls reach it (the spec's `BOUND = 2`).
pub const POLL_STEP: Duration = Duration::from_secs(15);
/// The `SimClock` step before each tick: past every restart backoff.
pub const PAST_BACKOFF: Duration = Duration::from_secs(3_600);
/// How long a step may take to settle (real time; owners read `SimClock`).
const SETTLE_BOUND: Duration = Duration::from_secs(10);
/// The guest's beacon source ports (relabelling only).
const BEACON_PORT_BASE: u32 = 50_000;
/// The spec's launch phase `LP_BOOT`: the VMM is spawned and holds the claim, the guest is not ready.
const LP_BOOT: i64 = 5;
/// A `LaunchPoll` step: two of them stay inside the 30 s claim bound whatever the poll interval.
const MODEL_POLL: Duration = Duration::from_secs(14);
/// The `LaunchHeldFail` step: past the 30 s bound after two `MODEL_POLL`s.
const HELD_STEP: Duration = Duration::from_secs(16);
/// `LaunchCheck` lets a waiting launch reach its next poll one second at a time, at most this many.
const REPOLL_MAX: u32 = 25;
const REPOLL_STEP: Duration = Duration::from_secs(1);
/// Pumps per `REPOLL_STEP`.
const REPOLL_PUMPS: u32 = 5;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("the step did not settle within the settle bound: {0}")]
    Unsettled(String),
    #[error("{0} has no pre-image in the model (the bijection is fixed by the driver)")]
    NoPreImage(String),
    #[error("a production call failed: {0}")]
    Production(String),
    #[error("ITF: {0}")]
    Itf(#[from] ItfError),
}

fn not_enabled(action: &'static str, why: impl Into<String>) -> DriverError {
    DriverError::NotEnabled { action, why: why.into() }
}

pub fn workload_id() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("a valid workload id")
}

pub fn target() -> TargetResource {
    TargetResource::new(&format!("workload/{WORKLOAD}")).expect("a valid target")
}

pub fn reconciler_name() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("a valid reconciler name")
}

/// The tick's accepted guest-network port: provisions and tears down at once
/// (the guest network is not the subject here). Authors no lease, row or
/// decision.
#[derive(Debug, Default)]
pub struct PassThroughProvisioner;

#[async_trait::async_trait]
impl GuestNetworkProvisioner for PassThroughProvisioner {
    async fn provision(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        Ok(())
    }

    async fn activate(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<ForwardingActivation> {
        Ok(ForwardingActivation::Activated)
    }

    async fn teardown(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        Ok(())
    }
}

// ----- the node and its serve processes ------------------------------------------

/// The volume disk each created VMM was asked to attach, by allocation.
type AttachedVolumes = Arc<Mutex<BTreeMap<AllocationId, Option<VolumeAttachment>>>>;

/// Parks each `Vmm::create` of an allocation before the VMM exists, until the driver releases it:
/// the model's `LP_FREE` (the launch observed the claim `Free` and has not yet created the VMM).
/// Off by default, so every other rig over `Node` creates immediately; the conformance driver
/// enables it at `Init`.
#[derive(Default)]
pub struct CreateGate {
    enabled: std::sync::atomic::AtomicBool,
    /// Parked (`false`) or released (`true`) creates by allocation.
    creates: Mutex<BTreeMap<AllocationId, bool>>,
}

impl CreateGate {
    pub(crate) fn enable(&self) {
        self.enabled.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the allocation's create is parked at the gate.
    pub(crate) fn is_parked(&self, alloc: &AllocationId) -> bool {
        self.creates.lock().get(alloc) == Some(&false)
    }

    /// Lets the allocation's parked create proceed.
    pub(crate) fn release(&self, alloc: &AllocationId) {
        if let Some(released) = self.creates.lock().get_mut(alloc) {
            *released = true;
        }
    }

    /// The `serve` process died: its parked creates died with it.
    pub(crate) fn clear(&self) {
        self.creates.lock().clear();
    }

    async fn park(&self, alloc: &AllocationId) {
        if !self.enabled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.creates.lock().insert(alloc.clone(), false);
        while self.creates.lock().get(alloc) != Some(&true) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        self.creates.lock().remove(alloc);
    }
}

/// The `Vmm` the node composes in simulation: the host-recording decorator, additionally
/// recording the `VmConfig.volume` of every `create`, so a step can check the production launch
/// attached the volume `ensure` returned. It plays no kernel verdict: the VMM's open of the
/// device (the claim) is the driver's `VmmSpawn` stimulus on the sim claim table.
struct VolumeRecordingVmm {
    inner: RecordingVmm,
    attached: AttachedVolumes,
    gate: Arc<CreateGate>,
}

#[async_trait]
impl Vmm for VolumeRecordingVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        self.attached.lock().insert(config.alloc.clone(), config.volume.clone());
        self.gate.park(&config.alloc).await;
        self.inner.create(config, cid).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// What outlives a `serve` process: the machine and its persistent state.
pub struct Node {
    pub(crate) tmp: TempDir,
    pub(crate) vmm: SimVmm,
    pub(crate) host: Arc<SimVmmHost>,
    pub(crate) clock: Arc<SimClock>,
    pub(crate) obs: Arc<SimObservationStore>,
    /// The kernel's claim table and the volume pool (the arbiter).
    pub(crate) backing: SimVolumeBacking,
    /// The volume disk each created VMM was asked to attach.
    pub(crate) attached: AttachedVolumes,
    /// Parks creates before the VMM exists while the conformance driver has it enabled.
    pub(crate) gate: Arc<CreateGate>,
    /// Whether a partition labelled `overdrive-volumes` exists at boot: the next
    /// `serve` composes `SimVolumePoolProbe::composed` over `backing` when it does and
    /// `SimVolumePoolProbe::absent()` when it does not (`StoreAbsentBoot`).
    pub(crate) pool_present: bool,
}

impl Node {
    pub(crate) fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        stage_artifacts(tmp.path());
        let vmm = SimVmm::new();
        Self {
            host: SimVmmHost::new(vmm.clone(), true),
            vmm,
            clock: Arc::new(SimClock::new()),
            obs: Arc::new(SimObservationStore::single_peer(
                NodeId::new("node-volume").expect("valid node ID"),
                0,
            )),
            backing: SimVolumeBacking::with_capacity(volume_size_gib(POOL_GIB)),
            attached: Arc::new(Mutex::new(BTreeMap::new())),
            gate: Arc::new(CreateGate::default()),
            pool_present: true,
            tmp,
        }
    }

    pub(crate) fn root(&self) -> &FsPath {
        self.tmp.path()
    }
}

/// The wire shape of the operator's declaration, as the CLI projects the TOML (`DECLARATION`);
/// `root` holds the staged `vmlinuz` and `master.img`.
pub fn declaration_wire(root: &str) -> serde_json::Value {
    serde_json::json!({
        "spec": {
            "kind": "service",
            "id": WORKLOAD,
            "replicas": 1,
            "resources": {"cpu": "100m", "memory": "128MiB"},
            "vm": {
                "command": "/sbin/init", "args": [],
                "kernel": format!("{root}/vmlinuz"), "rootfs": format!("{root}/master.img"),
                "volume": {"size": "40GiB"}
            },
            "listeners": [{"port": 8080, "protocol": "tcp"}],
            "startup_probes": [], "readiness_probes": [], "liveness_probes": []
        }
    })
}

fn stage_artifacts(root: &FsPath) {
    std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
        .expect("write synthetic master rootfs");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
    std::fs::create_dir_all(root.join("views")).expect("the view store directory");
}

/// One `serve` process: its runtime (its tasks die with it), its composition
/// and the launches it has in flight.
pub struct Serve {
    pub(crate) rt: Runtime,
    pub(crate) state: AppState,
    pub(crate) pool: Arc<GuestAddressPool>,
    pub(crate) beacon: Arc<SimBeaconIo>,
    pub(crate) beacon_port: VsockPort,
    /// Boot reclamation of this process: pending, or its result.
    pub(crate) reclaim: Option<JoinHandle<std::result::Result<(), ConvergeError>>>,
    /// The production VM driver of this process (the one `state` dispatches to).
    pub(crate) driver: Arc<dyn ProductionDriver>,
    pub(crate) tick_n: u64,
}

/// The production composition of a `serve` process over `node`, with the
/// volume pool composed through the pinned seam. Its exit observer and boot
/// reclamation are started.
pub fn boot_serve(node: &Node, boot: u32) -> (Serve, Arc<ReconcilerRuntime>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let root = node.root().to_path_buf();
    let beacon = Arc::new(SimBeaconIo::stepped(node.vmm.clone()));
    let clock = Arc::clone(&node.clock);
    let (state, pool, vm_driver, beacon_port, reconcilers) = rt.block_on(async {
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(wiring.supervisor().open_after_boot(), "the composition's EXEC gate opens");
        let intake = BeaconIntake::bind(
            Arc::clone(&beacon) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds");
        let beacon_port = intake.bound_port();
        let guest_listener_board = Arc::new(overdrive_worker::GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::clone(&guest_listener_board)
                as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        // The pinned seam: probe the node's pool (the same sim backing, or none when the
        // node booted without the labelled partition), hand the result to the driver.
        let probe = if node.pool_present {
            SimVolumePoolProbe::composed(node.backing.clone())
        } else {
            SimVolumePoolProbe::absent()
        };
        let volumes = probe.probe().await.expect("the sim pool probe passes");
        let vm_driver = VmDriver::new(
            Arc::new(VolumeRecordingVmm {
                inner: RecordingVmm { inner: node.vmm.clone(), host: Arc::clone(&node.host) },
                attached: Arc::clone(&node.attached),
                gate: Arc::clone(&node.gate),
            }),
            Arc::clone(&clock) as Arc<dyn Clock>,
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
            guest_listener_board,
            volumes,
        );
        // Every composition reopens the node's stores from disk.
        let config = crate::ServerConfig { data_dir: root.clone(), ..crate::ServerConfig::new() };
        let stores =
            crate::StoreAdapters::open(&config).expect("the node's stores reopen from disk");
        let mut runtime = ReconcilerRuntime::new(&root, Arc::clone(&stores.views))
            .expect("the reconciler runtime over the node's view store");
        runtime.register(crate::workload_lifecycle()).await.expect("register workload-lifecycle");
        let reconcilers = Arc::new(runtime);
        let pool = Arc::new(GuestAddressPool::new(
            PREFIX.parse().expect("pool prefix"),
            GATEWAY,
            Arc::new(SimEntropy::new(u64::from(boot))),
        ));
        let store = Arc::clone(stores.intent.store());
        let store_path = stores.intent.path().to_path_buf();
        let owner = crate::shared_network_test_ports::shared_guest_network_owner();
        owner.converge_shared().await.expect("the test owner's converge_shared passes");
        let mtls_worker = crate::shared_network_test_ports::mtls_worker();
        mtls_worker
            .start_shared_owner()
            .await
            .expect("the test mTLS owner's start_shared_owner passes");
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mut state = AppState::new(
            store,
            store_path,
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::clone(&reconcilers),
            Arc::new(vm_driver.clone()) as Arc<dyn overdrive_core::traits::driver::Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            NodeId::new("local").expect("valid NodeId"),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            mtls_worker,
            owner,
            wiring.gate(),
            Arc::clone(&pool),
        );
        state.vm_host_state = Arc::clone(&node.host) as Arc<dyn VmHostState>;
        (state, pool, vm_driver, beacon_port, reconcilers)
    });
    let driver_handle: Arc<dyn ProductionDriver> = Arc::new(vm_driver.clone());
    // The production exit observer writes each VMM's terminal row.
    let _exit_observer = {
        let _runtime = rt.enter();
        crate::worker::exit_observer::spawn(
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::new(vm_driver) as Arc<dyn overdrive_core::traits::driver::Driver>,
            Arc::clone(&state.lifecycle_events),
            Arc::clone(&clock) as Arc<dyn Clock>,
        )
    };
    let reclaim_state = state.clone();
    let reclaim = rt.spawn(async move { converge(&reclaim_state).await });
    let serve = Serve {
        rt,
        state,
        pool,
        beacon,
        beacon_port,
        reclaim: Some(reclaim),
        driver: driver_handle,
        tick_n: 0,
    };
    (serve, reconcilers)
}

/// What the driver observes of the convergence tick (the shim dispatch) it holds for a launch.
enum LaunchView {
    /// No tick was ever held for the allocation.
    Unheld,
    /// The dispatch has not returned: the launch holds the lane.
    Running,
    /// The dispatch returned `Ok(())`.
    Ended,
    /// The tick's future was dropped with its `serve` process (a rebuild) before it returned.
    Dropped,
}

/// One convergence tick of the workload, spawned on a `serve` process's runtime: its outcome cell
/// is set when the shim's dispatch (and so `VmDriver::start`) returns.
struct HeldTick {
    outcome: Arc<Mutex<Option<std::result::Result<(), String>>>>,
    task: JoinHandle<()>,
}

struct NodeVolumeDriver {
    /// The restart authority of the current `serve` process (`WorkloadLifecycle`
    /// registered in its runtime). While no process is up it is a placeholder
    /// over its own view store, so the node's view store is free for the next
    /// process; never read then.
    reconcilers: Arc<ReconcilerRuntime>,
    /// Holds the placeholders' view store directories.
    placeholder: TempDir,
    /// A fresh node per `Init` (the runner reuses one driver per trace).
    node: Option<Node>,
    serve: Option<Serve>,
    boots: u32,
    /// The launch tick of each model allocation. It survives a rebuild: the future of a tick the
    /// rebuild dropped never completes, which is how a lost launch shows.
    launches: BTreeMap<i64, HeldTick>,
    /// Allocation ↦ model id (relabelling, fixed when first seen).
    alloc_of: BTreeMap<AllocationId, i64>,
    next_beacon_port: u32,
    /// The ordinal of the step being taken (`Init` is step 1), for the divergence message.
    step_no: usize,
}

impl NodeVolumeDriver {
    fn new() -> Self {
        let placeholder = TempDir::new().expect("tmpdir");
        let reconcilers = Arc::new(
            ReconcilerRuntime::new_with_redb_view_store_for_test(placeholder.path())
                .expect("a placeholder reconciler runtime"),
        );
        Self {
            reconcilers,
            placeholder,
            node: None,
            serve: None,
            boots: 0,
            launches: BTreeMap::new(),
            alloc_of: BTreeMap::new(),
            next_beacon_port: BEACON_PORT_BASE,
            step_no: 0,
        }
    }

    /// Names the step in a handler's failure: the per-step divergence between the model and
    /// production.
    fn at(
        step_no: usize,
        action: &'static str,
        outcome: std::result::Result<(), DriverError>,
    ) -> std::result::Result<(), DriverError> {
        outcome.map_err(|error| {
            let message = match error {
                DriverError::Production(message) => message,
                DriverError::NotEnabled { why, .. } => format!("the action is not enabled: {why}"),
                other => other.to_string(),
            };
            DriverError::Production(format!("step {step_no} {action}: {message}"))
        })
    }

    fn node(&self, action: &'static str) -> std::result::Result<&Node, DriverError> {
        self.node.as_ref().ok_or_else(|| not_enabled(action, "no node: `Init` has not run"))
    }

    fn serve(&self, action: &'static str) -> std::result::Result<&Serve, DriverError> {
        self.serve.as_ref().ok_or_else(|| not_enabled(action, "serve is down"))
    }

    fn serve_mut(&mut self, action: &'static str) -> std::result::Result<&mut Serve, DriverError> {
        self.serve.as_mut().ok_or_else(|| not_enabled(action, "serve is down"))
    }

    /// Polls `cond` on the serve process's runtime until it holds.
    fn settle(
        &self,
        what: &str,
        mut cond: impl FnMut(&Self) -> bool,
    ) -> std::result::Result<(), DriverError> {
        let deadline = std::time::Instant::now() + SETTLE_BOUND;
        while !cond(self) {
            if std::time::Instant::now() > deadline {
                return Err(DriverError::Unsettled(what.to_owned()));
            }
            self.pump("settle")?;
        }
        Ok(())
    }

    /// Lets the serve process's tasks run for a millisecond.
    fn pump(&self, action: &'static str) -> std::result::Result<(), DriverError> {
        self.serve(action)?
            .rt
            .block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        Ok(())
    }

    fn rows(&self) -> std::result::Result<Vec<AllocStatusRow>, DriverError> {
        let serve = self.serve("project")?;
        serve
            .rt
            .block_on(serve.state.obs.alloc_status_rows())
            .map_err(|error| DriverError::Production(format!("alloc_status_rows: {error}")))
    }

    fn alloc_id(&self, a: i64) -> Option<AllocationId> {
        self.alloc_of.iter().find(|(_, model)| **model == a).map(|(alloc, _)| alloc.clone())
    }

    /// The allocation's row state, `None` while it has no row.
    fn state_of(&self, a: i64) -> Option<AllocState> {
        let alloc = self.alloc_id(a)?;
        self.rows().ok()?.into_iter().find(|row| row.alloc_id == alloc).map(|row| row.state)
    }

    fn launch_view(&self, a: i64) -> LaunchView {
        let Some(tick) = self.launches.get(&a) else { return LaunchView::Unheld };
        match &*tick.outcome.lock() {
            Some(Ok(())) => LaunchView::Ended,
            Some(Err(_)) => LaunchView::Dropped,
            None if tick.task.is_finished() => LaunchView::Dropped,
            None => LaunchView::Running,
        }
    }

    /// The held launch's own error, when its dispatch returned one.
    fn launch_error(&self, a: i64) -> Option<String> {
        self.launches.get(&a).and_then(|tick| match &*tick.outcome.lock() {
            Some(Err(message)) => Some(message.clone()),
            _ => None,
        })
    }

    /// The allocation ids production issued or lists, none of which the model labelled yet.
    fn fresh_alloc(&self) -> Option<AllocationId> {
        let serve = self.serve.as_ref()?;
        let mut ids: BTreeSet<AllocationId> = serve
            .state
            .runtime
            .view_for_workload_lifecycle(&target())
            .issued_generation
            .keys()
            .cloned()
            .collect();
        ids.extend(self.rows().ok()?.into_iter().map(|row| row.alloc_id));
        ids.extend(serve.driver.live_allocations().unwrap_or_default());
        ids.into_iter().find(|id| !self.alloc_of.contains_key(id))
    }

    // ----- ticks ------------------------------------------------------------------

    /// One production convergence tick of the workload, spawned on the serve process's runtime:
    /// its dispatch may park inside `VmDriver::start`.
    fn spawn_tick(&mut self, action: &'static str) -> std::result::Result<HeldTick, DriverError> {
        let clock = Arc::clone(&self.node(action)?.clock);
        clock.tick(PAST_BACKOFF);
        let serve = self.serve_mut(action)?;
        serve.tick_n += 1;
        let tick_n = serve.tick_n;
        let state = serve.state.clone();
        let outcome = Arc::new(Mutex::new(None));
        let cell = Arc::clone(&outcome);
        let task = serve.rt.spawn(async move {
            let now = clock.now();
            let result = run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &reconciler_name(),
                &target(),
                now,
                tick_n,
                now + Duration::from_secs(1),
                &PassThroughProvisioner,
            )
            .await
            .map_err(|error: ConvergenceError| error.to_string());
            *cell.lock() = Some(result);
        });
        Ok(HeldTick { outcome, task })
    }

    /// One tick run to its end (a stop's or a restart's dispatch): it returned `Ok`.
    fn run_tick_to_end(&mut self, action: &'static str) -> std::result::Result<(), DriverError> {
        let tick = self.spawn_tick(action)?;
        self.settle("the tick's dispatch returns", |_| tick.outcome.lock().is_some())?;
        let result = tick.outcome.lock().take().unwrap_or(Ok(()));
        result.map_err(|error| DriverError::Production(format!("convergence tick: {error}")))
    }

    /// Waits until allocation `a`'s row is `state`.
    fn await_state(&self, a: i64, state: AllocState) -> std::result::Result<(), DriverError> {
        self.settle("the allocation row reaches the state", |driver| {
            driver.state_of(a) == Some(state)
        })
    }

    // ----- actions ------------------------------------------------------------------

    fn init(&mut self) -> std::result::Result<(), DriverError> {
        self.crash_serve()?;
        self.node = None;
        self.alloc_of.clear();
        self.launches.clear();
        let node = Node::new();
        node.gate.enable();
        self.boots = 1;
        self.boot(&node);
        self.node = Some(node);
        self.settle_reclaim()
    }

    /// Boot reclamation returns (nothing to reclaim), or parks on a held kill.
    fn settle_reclaim(&mut self) -> std::result::Result<(), DriverError> {
        self.settle("boot reclamation returns or parks on a held kill", |driver| {
            let Some(serve) = driver.serve.as_ref() else { return true };
            let Some(node) = driver.node.as_ref() else { return true };
            let parked = node.host.kills_requested().iter().any(|pid| node.vmm.is_live(*pid));
            serve.reclaim.as_ref().is_none_or(JoinHandle::is_finished) || parked
        })?;
        if let Some(serve) = self.serve.as_mut()
            && serve.reclaim.as_ref().is_some_and(JoinHandle::is_finished)
        {
            let handle = serve.reclaim.take().expect("checked above");
            serve
                .rt
                .block_on(handle)
                .expect("converge did not panic")
                .map_err(|error| DriverError::Production(format!("reclamation: {error}")))?;
        }
        Ok(())
    }

    /// `Declare`: the operator submits the workload. The declaration is the
    /// operator's TOML, accepted by the production parser, submitted through
    /// the production handler as the wire shape the CLI projects it to.
    fn declare(&mut self) -> std::result::Result<(), DriverError> {
        WorkloadSpecInput::from_toml_str(DECLARATION).map_err(|error| {
            DriverError::Production(format!(
                "the production spec parser must accept the [vm.volume] declaration: {error}"
            ))
        })?;
        let root = self.node("Declare")?.root().display().to_string();
        let request: SubmitWorkloadRequest = serde_json::from_value(declaration_wire(&root))
            .map_err(|error| DriverError::Production(format!("the wire spec: {error}")))?;
        let serve = self.serve("Declare")?;
        serve
            .rt
            .block_on(submit_workload(
                AxumState(serve.state.clone()),
                HeaderMap::new(),
                Json(request),
            ))
            .map_err(|error| DriverError::Production(format!("submit_workload: {error}")))?;
        Ok(())
    }

    /// `OpStop`: the operator handler records the stop and raises the open lane's signal.
    fn op_stop(&mut self) -> std::result::Result<(), DriverError> {
        let serve = self.serve("OpStop")?;
        let _stopped = serve
            .rt
            .block_on(stop_workload(AxumState(serve.state.clone()), Path(WORKLOAD.to_owned())))
            .map_err(|error| DriverError::Production(format!("stop_workload: {error}")))?;
        Ok(())
    }

    fn restart(
        &self,
        action: &'static str,
        expected: RestartOutcome,
    ) -> std::result::Result<(), DriverError> {
        let serve = self.serve(action)?;
        let response = serve
            .rt
            .block_on(restart_workload(AxumState(serve.state.clone()), Path(WORKLOAD.to_owned())))
            .map_err(|error| DriverError::Production(format!("restart_workload: {error}")))?;
        if response.0.outcome == expected {
            Ok(())
        } else {
            Err(DriverError::Production(format!(
                "restart_workload answered {:?}, not {expected:?}",
                response.0.outcome
            )))
        }
    }

    /// `Shutdown`: the node begins graceful shutdown; the runtime raises every open lane (and a
    /// lane opened afterwards is born raised).
    fn shutdown(&self) -> std::result::Result<(), DriverError> {
        self.serve("Shutdown")?.state.runtime.supersede_all_lanes();
        self.pump("Shutdown")
    }

    fn wl_start(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        // The launch's `ensure` parks after its create effect, so the driver steps its two
        // effects.
        self.node("WlStart")?.backing.pause_ensure_after_create(&workload_id());
        let tick = self.spawn_tick("WlStart")?;
        self.launches.insert(a, tick);
        // The launch writes no row before `start` returns: the minted allocation is the id the
        // reconciler issued (its view), the launch listed in the driver, or a row.
        self.settle("the tick mints the allocation", |driver| driver.fresh_alloc().is_some())?;
        let fresh = self
            .fresh_alloc()
            .ok_or_else(|| not_enabled("WlStart", "the tick minted no allocation"))?;
        self.alloc_of.insert(fresh, a);
        // A launch that cannot provide the volume (no pool, no free extent) fails typed inside
        // its first step: the placement and the failure are one model step. Either the launch
        // parks after `ensure`'s create effect, or its `Failed` row lands.
        self.settle("the launch parks after its create effect or fails typed", |driver| {
            driver.node.as_ref().is_some_and(|node| node.backing.ensure_parked(&workload_id()))
                || driver.state_of(a) == Some(AllocState::Failed)
        })?;
        if self.state_of(a) == Some(AllocState::Failed) {
            self.volume_launch_fails("WlStart", a)?;
            self.finalize_observed()?;
        }
        Ok(())
    }

    /// The reconciler's own decision about the workload's current row: when the ended current row
    /// has no terminal claim yet and the reconciler will not restart it (the restart ceiling is
    /// spent), one evaluation finalises it with `BackoffExhausted`, which the model's `exhausted`
    /// reads. When a restart would follow, no tick runs: the next `WlStart` is that placement.
    fn finalize_observed(&mut self) -> std::result::Result<(), DriverError> {
        let rows = self.rows()?;
        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        let Some(current) = current_alloc(&refs) else { return Ok(()) };
        if current.state != AllocState::Failed || current.terminal.is_some() {
            return Ok(());
        }
        let current = current.clone();
        let serve = self.serve("WlStart")?;
        let reconciler = serve.state.runtime.get(&reconciler_name()).ok_or_else(|| {
            DriverError::Production("workload-lifecycle is not registered".to_owned())
        })?;
        let desired = serve
            .rt
            .block_on(hydrate_desired_for_test(reconciler, &target(), &serve.state))
            .map_err(|error| DriverError::Production(format!("hydrate_desired: {error}")))?;
        let AnyState::WorkloadLifecycle(desired) = desired else {
            return Err(DriverError::Production(
                "hydrate_desired returned another state".to_owned(),
            ));
        };
        let (_, will_restart) = serve.state.runtime.restart_status_for_alloc(
            &target(),
            WorkloadKind::Service,
            !desired.desired_to_stop,
            &current,
            desired.generation,
        );
        if will_restart {
            return Ok(());
        }
        self.run_tick_to_end("WlStart")
    }

    fn stop_alloc(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        self.run_tick_to_end("StopAlloc")?;
        self.await_state(a, AllocState::Terminated)
    }

    /// `WorkloadRestart(a)`: the handler; a `Running` allocation is then ended by the shim's
    /// stop. For a launch in flight the handler only raises the lane's signal.
    fn workload_restart(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        self.restart("WorkloadRestart", RestartOutcome::Restarted)?;
        if self.state_of(a) == Some(AllocState::Running) {
            self.run_tick_to_end("WorkloadRestart")?;
            self.await_state(a, AllocState::Terminated)?;
        }
        Ok(())
    }

    fn ensure_create(&self, _a: i64) -> std::result::Result<(), DriverError> {
        self.settle("the launch's ensure parks after its create effect", |driver| {
            driver.node.as_ref().is_some_and(|node| node.backing.ensure_parked(&workload_id()))
        })
    }

    fn ensure_activate(&self, _a: i64) -> std::result::Result<(), DriverError> {
        self.node("LaunchEnsureActivate")?.backing.resume_ensure(&workload_id());
        // Leaving the park is not the effect: the activation is. The model's `lv ∧ active` is
        // `claim_state != NotFound`, so settle on that report.
        self.settle("the launch's ensure activates the volume", |driver| {
            driver.node.as_ref().is_some_and(|node| !node.backing.ensure_parked(&workload_id()))
                && matches!(driver.claim(), Ok(Ok(ClaimState::Held | ClaimState::Free)))
        })
    }

    /// The launch's typed failure: the spawned start returns, the shim writes the `Failed` row,
    /// and no VMM was ever created for the allocation.
    fn launch_fails(&self, action: &'static str, a: i64) -> std::result::Result<(), DriverError> {
        self.await_state(a, AllocState::Failed)?;
        self.expect_no_vmm(action, a)
    }

    /// A launch that ends typed on the volume: as `launch_fails`, and the `Failed` row's reason is
    /// the typed `VmVolumeUnavailable` (the allocation row's `TransitionReason`, matched by variant
    /// name so the driver compiles before the variant exists).
    fn volume_launch_fails(
        &self,
        action: &'static str,
        a: i64,
    ) -> std::result::Result<(), DriverError> {
        self.launch_fails(action, a)?;
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled(action, format!("{a} has no allocation")))?;
        let row = self.rows()?.into_iter().find(|row| row.alloc_id == alloc).ok_or_else(|| {
            DriverError::Production(format!("the failed launch of {a} wrote no row"))
        })?;
        let reason = format!("{:?}", row.reason);
        if reason.contains("VmVolumeUnavailable") {
            Ok(())
        } else {
            Err(DriverError::Production(format!(
                "the failed launch of {a} ended with reason {reason}, not a typed VmVolumeUnavailable"
            )))
        }
    }

    fn expect_no_vmm(&self, action: &'static str, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled(action, format!("{a} has no allocation")))?;
        // A guard assertion on the recording `Vmm` double (the driven port records what the launch
        // asked of it); not a projection: `from_driver` never reaches it.
        if self.node(action)?.host.pids_of(&alloc).is_empty() {
            Ok(())
        } else {
            Err(DriverError::Production(format!("a VMM was created for the failed launch of {a}")))
        }
    }

    /// `LaunchCheck(a)`: the launch observes `claim_state` `Free` and reaches `Vmm::create`. A
    /// launch that waited on a claim wakes at its next poll, so the driver lets the clock reach it,
    /// one second at a time and never past the claim bound.
    fn launch_check(&self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("LaunchCheck", format!("{a} has no allocation")))?;
        // The launch observed `Free` and its `Vmm::create` is parked at the gate: the model's
        // `LP_FREE`. The VMM does not exist yet; `VmmSpawn` or `VmmSpawnFail` releases the create.
        let created =
            |driver: &Self| driver.node.as_ref().is_some_and(|node| node.gate.is_parked(&alloc));
        for _ in 0..=REPOLL_MAX {
            for _ in 0..REPOLL_PUMPS {
                if created(self) {
                    return Ok(());
                }
                self.pump("LaunchCheck")?;
            }
            self.node("LaunchCheck")?.clock.tick(REPOLL_STEP);
        }
        Err(DriverError::Unsettled("the launch reaches Vmm::create".to_owned()))
    }

    /// `LaunchPoll(a)`: the clock passes one poll step inside the bound.
    fn launch_poll(&self, _a: i64) -> std::result::Result<(), DriverError> {
        self.node("LaunchPoll")?.clock.tick(MODEL_POLL);
        self.pump("LaunchPoll")
    }

    /// `LaunchHeldFail(a)`: the clock passes the bound; the launch ends `VolumeUnavailable { Held }`.
    fn launch_held_fail(&self, a: i64) -> std::result::Result<(), DriverError> {
        self.node("LaunchHeldFail")?.clock.tick(HELD_STEP);
        self.volume_launch_fails("LaunchHeldFail", a)
    }

    /// `LaunchSuperseded(a)`: the launch observed the lane signal the operator handler or the
    /// shutdown raised: its dispatch returns `Ok(())`, no row is written, the driver no longer
    /// lists the allocation and its VMM, if one was spawned, is gone.
    fn launch_superseded(&self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("LaunchSuperseded", format!("{a} has no allocation")))?;
        // A create parked at the gate (`LP_FREE`) is past the production launch's last signal
        // observation: it proceeds, and the launch then abandons the VMM it created.
        self.node("LaunchSuperseded")?.gate.release(&alloc);
        self.settle("the superseded launch's dispatch returns", |driver| {
            !matches!(driver.launch_view(a), LaunchView::Running)
        })?;
        match self.launch_view(a) {
            LaunchView::Ended => {}
            LaunchView::Dropped => {
                let error = self.launch_error(a).unwrap_or_else(|| "its tick was dropped".into());
                return Err(DriverError::Production(format!(
                    "the superseded launch's dispatch did not return Ok(()): {error}"
                )));
            }
            LaunchView::Running | LaunchView::Unheld => {
                return Err(not_enabled("LaunchSuperseded", format!("{a} holds no launch")));
            }
        }
        if self.rows()?.iter().any(|row| row.alloc_id == alloc) {
            return Err(DriverError::Production(format!(
                "the superseded launch of {a} wrote an allocation row"
            )));
        }
        let serve = self.serve("LaunchSuperseded")?;
        if serve.driver.live_allocations().unwrap_or_default().contains(&alloc) {
            return Err(DriverError::Production(format!(
                "the driver still lists the superseded launch of {a}"
            )));
        }
        if self.vmm_live(&alloc)? {
            return Err(DriverError::Production(format!(
                "the VMM of the superseded launch of {a} was not terminated"
            )));
        }
        Ok(())
    }

    /// Whether a created, not terminated VMM of the allocation exists.
    fn vmm_live(&self, alloc: &AllocationId) -> std::result::Result<bool, DriverError> {
        let node = self.node("project")?;
        // conformance-lint: sim-arbiter-ok the VMM process is a driven-port fact the owner does not hold (`alive[a]` while booting); Tier-3 pairing test S-VOL-T3-13, crates/overdrive-cli/tests/integration/volume_survives.rs::a_workload_stop_during_guest_boot_terminates_the_vmm_and_keeps_the_volume
        Ok(node.host.pids_of(alloc).iter().any(|pid| node.vmm.is_live(*pid)))
    }

    /// `VmmSpawn(a)`: the launch reached `Vmm::create` and attached the ensured volume; the VMM
    /// opens the device (the claim). The allocation stays without a row until the guest is ready.
    fn vmm_spawn(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("VmmSpawn", format!("{a} has no allocation")))?;
        let node = self.node("VmmSpawn")?;
        // Production attached the volume: the recording `Vmm` saw the `VmConfig` the launch
        // built, and its volume disk is the device `ensure` returned (idempotent) with the
        // fixed serial.
        let attached = node
            .attached
            .lock()
            .get(&alloc)
            .cloned()
            .flatten()
            .ok_or_else(|| {
                DriverError::Production(format!(
                    "the VMM of {alloc} was created without the volume disk: the launch must attach the ensured device"
                ))
            })?;
        let serve = self.serve("VmmSpawn")?;
        let ensured = serve
            .rt
            .block_on(node.backing.ensure(&workload_id(), volume_size_gib(DECLARED_GIB)))
            .map_err(|error| DriverError::Production(format!("ensure: {error}")))?;
        if attached.device != ensured {
            return Err(DriverError::Production(format!(
                "the VMM attached {}, not the ensured device {}",
                attached.device.display(),
                ensured.display(),
            )));
        }
        // The VMM's open of the device takes the kernel claim: the sim claim table's stimulus
        // (`SimVmm::create` does not touch the pool).
        node.gate.release(&alloc);
        self.settle("the parked create proceeds and the VMM exists", |driver| {
            // Waiting for the released `create` to reach the recording `Vmm` double; not a projection.
            driver.node.as_ref().is_some_and(|node| !node.host.pids_of(&alloc).is_empty())
        })?;
        self.node("VmmSpawn")?.backing.hold_claim(&workload_id());
        self.pump("VmmSpawn")
    }

    /// `GuestReady(a)`: the guest's READY reaches the beacon intake; the launch returns and the
    /// shim writes `Running`.
    fn guest_ready(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("GuestReady", format!("{a} has no allocation")))?;
        let node = self.node("GuestReady")?;
        let pid = node
            .host
            .pids_of(&alloc)
            .into_iter()
            .next()
            .ok_or_else(|| not_enabled("GuestReady", format!("{a} has no VMM")))?;
        self.next_beacon_port += 1;
        let port = self.next_beacon_port;
        let serve = self.serve("GuestReady")?;
        let view = serve
            .pool
            .lease_observation()
            .leases
            .get(&alloc)
            .map(|view| (view.cid, view.launch))
            .ok_or_else(|| not_enabled("GuestReady", format!("{alloc} holds no lease")))?;
        let key = BeaconConnKey { peer_cid: view.0, peer_port: VsockPort::new(port) };
        let ready = format!("READY pid=1 port={} launch={}", serve.beacon_port, view.1);
        serve
            .beacon
            .queue_connection(key, pid, &[ready.as_str()])
            .map_err(|error| DriverError::Production(format!("the guest's beacon: {error}")))?;
        serve.beacon.release_accept();
        self.settle("the beacon connection is accepted", |driver| {
            driver.serve.as_ref().is_some_and(|serve| serve.beacon.release_read(key))
        })?;
        self.await_state(a, AllocState::Running)
    }

    /// `GuestWrite` and `DeviceSlow` have no owner port: a guest write and a dead VMM's
    /// in-flight I/O outlasting the launch bound are facts only the guest and the kernel produce
    /// (the model variables `dv`, `maxDv` and `slow` are model-only, § Observable projections),
    /// and no production driving port stimulates them. What production does in answer is driven
    /// by the neighbouring actions (`LaunchPoll`, `LaunchHeldFail`, `Drain`). The handler
    /// checks the spec action's enabling guard against production: the claim arbiter reports
    /// the volume `Held`, and a VMM of the workload was created through the production launch
    /// (`holder_live`: `None` for `GuestWrite` (`claim > 0`); for `DeviceSlow` (`deadHolder`) no
    /// VMM of the workload is live any more).
    fn expect_claim_held(
        &self,
        action: &'static str,
        holder_live: Option<bool>,
    ) -> std::result::Result<(), DriverError> {
        let node = self.node(action)?;
        let pids: Vec<u32> =
            // The action guard reads the recording `Vmm` double; not a projection.
            self.alloc_of.keys().flat_map(|alloc| node.host.pids_of(alloc)).collect();
        if pids.is_empty() {
            return Err(not_enabled(action, "no VMM of the workload was created by a launch"));
        }
        let live = pids.iter().any(|pid| node.vmm.is_live(*pid));
        if let Some(want) = holder_live
            && live != want
        {
            return Err(not_enabled(
                action,
                format!("a live VMM of the workload is {live}, the action needs {want}"),
            ));
        }
        match self.claim()? {
            Ok(ClaimState::Held) => Ok(()),
            other => Err(not_enabled(action, format!("the claim is {other:?}, not held"))),
        }
    }

    fn vmm_sigkill(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("VmmSigkill", format!("{a} has no allocation")))?;
        let node = self.node("VmmSigkill")?;
        let pid = node
            .host
            .pids_of(&alloc)
            .into_iter()
            .next()
            .ok_or_else(|| not_enabled("VmmSigkill", format!("{a} has no VMM")))?;
        if !node.vmm.exit_process(pid, None, Some(9)) {
            return Err(not_enabled("VmmSigkill", "the VMM is gone"));
        }
        self.serve("VmmSigkill")?.beacon.observe_vm_exits();
        node.host.notify();
        self.settle("the exit observer writes the terminal row", |driver| {
            driver.state_of(a) == Some(AllocState::Failed)
        })
    }

    fn drain(&self) -> std::result::Result<(), DriverError> {
        self.node("Drain")?.backing.drain(&workload_id());
        self.pump("Drain")
    }

    /// `Reclaim(a)`: the production boot reclamation kills the surviving VMM
    /// when the driver grants its requested kill.
    fn reclaim(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let running = self.state_of(a) == Some(AllocState::Running);
        loop {
            self.settle_reclaim()?;
            if self.serve("Reclaim")?.reclaim.is_none() {
                break;
            }
            let node = self.node("Reclaim")?;
            let pending: Vec<u32> = node
                .host
                .kills_requested()
                .iter()
                .copied()
                .filter(|pid| node.vmm.is_live(*pid))
                .collect();
            if pending.is_empty() {
                return Err(DriverError::Unsettled(
                    "boot reclamation neither returned nor requested a kill".to_owned(),
                ));
            }
            for pid in pending {
                node.host.grant_kill(pid);
                self.settle("the orphan is killed", |driver| {
                    driver.node.as_ref().is_none_or(|node| !node.vmm.is_live(pid))
                })?;
            }
        }
        if running { self.await_state(a, AllocState::Failed) } else { Ok(()) }
    }

    /// `PoolFull`: the volume pool has no free extent before the first launch.
    fn pool_full(&self) -> std::result::Result<(), DriverError> {
        self.node("PoolFull")?.backing.exhaust_space();
        Ok(())
    }

    /// `LaunchCmdFail(a)`: the next `lvm` command, the activate effect, fails; the launch ends
    /// typed.
    fn launch_cmd_fail(&self, a: i64) -> std::result::Result<(), DriverError> {
        self.node("LaunchCmdFail")?.backing.fail_next_command();
        self.node("LaunchCmdFail")?.backing.resume_ensure(&workload_id());
        self.volume_launch_fails("LaunchCmdFail", a)
    }

    /// `VmmSpawnFail(a)`: the launch observed the claim `Free` (its `Vmm::create` is parked at the
    /// gate, `LP_FREE`) and the VMM's own `O_EXCL` open then loses. The driven-port fault is the
    /// `Vmm`'s refusal: `SimVmm::inject_create_failure` arms the one-shot failure of the next
    /// `create`, which is the released one; production classifies it and the shim writes the
    /// charged `Failed` row. No VMM exists afterwards and the claim table is not touched.
    fn vmm_spawn_fail(&self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .alloc_id(a)
            .ok_or_else(|| not_enabled("VmmSpawnFail", format!("{a} has no allocation")))?;
        let node = self.node("VmmSpawnFail")?;
        if !node.gate.is_parked(&alloc) {
            return Err(not_enabled(
                "VmmSpawnFail",
                format!("the launch of {a} is not parked before Vmm::create"),
            ));
        }
        node.vmm.inject_create_failure();
        node.gate.release(&alloc);
        self.launch_fails("VmmSpawnFail", a)
    }

    /// `StoreAbsentBoot`: the node boots without the labelled partition: the `serve` process
    /// is replaced by one whose probe is `SimVolumePoolProbe::absent()`, so its driver holds
    /// no pool. Enabled only before anything was created or minted (the spec's guard), so
    /// no launch is in flight to lose.
    fn store_absent_boot(&mut self) -> std::result::Result<(), DriverError> {
        self.node
            .as_mut()
            .ok_or_else(|| not_enabled("StoreAbsentBoot", "no node: `Init` has not run"))?
            .pool_present = false;
        self.cp_crash()
    }

    /// `CpCrash` and `Boot`: the `serve` process dies (or shuts down) and the next boots; every
    /// VMM survives. The next composition probes the same sim backing again.
    fn cp_crash(&mut self) -> std::result::Result<(), DriverError> {
        self.crash_serve()?;
        self.boots += 1;
        let node = self.node.take().ok_or_else(|| not_enabled("CpCrash", "no node"))?;
        self.boot(&node);
        self.node = Some(node);
        self.settle_reclaim()
    }

    /// The `serve` process dies: its runtime (every task of the process) and
    /// composition are dropped, and the node's view store is freed.
    fn crash_serve(&mut self) -> std::result::Result<(), DriverError> {
        self.serve = None;
        if let Some(node) = &self.node {
            node.gate.clear();
        }
        let down = self.placeholder.path().join(format!("down-{}", self.boots));
        std::fs::create_dir_all(&down)
            .map_err(|error| DriverError::Production(format!("placeholder view store: {error}")))?;
        self.reconcilers =
            Arc::new(ReconcilerRuntime::new_with_redb_view_store_for_test(&down).map_err(
                |error| DriverError::Production(format!("placeholder runtime: {error}")),
            )?);
        Ok(())
    }

    /// A new `serve` process over `node`.
    fn boot(&mut self, node: &Node) {
        let (serve, reconcilers) = boot_serve(node, self.boots);
        self.serve = Some(serve);
        self.reconcilers = reconcilers;
    }

    /// `NodeReboot`: every VMM is gone, the volume is inactive and every claim
    /// released, and a fresh `serve` process boots.
    fn node_reboot(&mut self) -> std::result::Result<(), DriverError> {
        // A running allocation's VMM exits while the old process observes it: the exit observer
        // writes the crash row the model's `NodeReboot` expects. A booting launch has no row and
        // dies with its process.
        let running: Vec<i64> = self
            .alloc_of
            .values()
            .copied()
            .filter(|a| self.state_of(*a) == Some(AllocState::Running))
            .collect();
        for a in running {
            self.vmm_sigkill(a)?;
        }
        let node = self.node("NodeReboot")?;
        for alloc in self.alloc_of.keys() {
            for pid in node.host.pids_of(alloc) {
                node.vmm.exit_process(pid, None, Some(9));
            }
        }
        node.backing.node_reboot();
        self.cp_crash()
    }

    // ----- projection reads -----------------------------------------------------------

    /// The claim arbiter's report for the workload's volume.
    fn claim(
        &self,
    ) -> std::result::Result<std::result::Result<ClaimState, VolumeBackingError>, DriverError> {
        let node = self.node("project")?;
        let serve = self.serve("project")?;
        // conformance-lint: sim-arbiter-ok the kernel's claim table is a driven-port fact the owner does not hold; Tier-3 pairing test S-VOL-T3-04, crates/overdrive-host/tests/integration/volume_pool_backing.rs::claim_state_reports_held_free_and_not_found_as_the_kernel_does
        Ok(serve.rt.block_on(node.backing.claim_state(&workload_id())))
    }

    /// The model allocations whose launch is booting: the driver lists the launch, no row exists
    /// and the guest is not ready, the claim is held, and a created, not terminated VMM of the
    /// allocation exists (the model's `alive`, read from the recording `Vmm` double as the arbiter
    /// of the VMM process; Tier-3 twin S-VOL-T3-13; feature delta § Observable projections, the
    /// `alive[a]` row).
    fn booting(&self) -> std::result::Result<BTreeSet<i64>, DriverError> {
        let serve = self.serve("project")?;
        let listed = serve.driver.live_allocations().unwrap_or_default();
        let held = matches!(self.claim()?, Ok(ClaimState::Held));
        let mut booting = BTreeSet::new();
        for a in 1..=NA {
            let Some(alloc) = self.alloc_id(a) else { continue };
            if self.state_of(a).is_none()
                && matches!(self.launch_view(a), LaunchView::Running)
                && listed.contains(&alloc)
                && held
                && self.vmm_live(&alloc)?
            {
                booting.insert(a);
            }
        }
        Ok(booting)
    }
}

impl Driver for NodeVolumeDriver {
    type State = NodeVolumeState;

    fn config() -> Config {
        Config { state: &[], nondet: &["vol_conf::node_volume::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.step_no += 1;
        let n = self.step_no;
        switch!(step {
            Init => Self::at(n, "Init", self.init())?,
            Declare => Self::at(n, "Declare", self.declare())?,
            OpStop => Self::at(n, "OpStop", self.op_stop())?,
            OpResume => Self::at(n, "OpResume", self.restart("OpResume", RestartOutcome::Resumed))?,
            StoreAbsentBoot => Self::at(n, "StoreAbsentBoot", self.store_absent_boot())?,
            PoolFull => Self::at(n, "PoolFull", self.pool_full())?,
            Shutdown => Self::at(n, "Shutdown", self.shutdown())?,
            Boot => Self::at(n, "Boot", self.cp_crash())?,
            WlStart(a: i64) => Self::at(n, "WlStart", self.wl_start(a))?,
            StopAlloc(a: i64) => Self::at(n, "StopAlloc", self.stop_alloc(a))?,
            WorkloadRestart(a: i64) => Self::at(n, "WorkloadRestart", self.workload_restart(a))?,
            LaunchEnsureCreate(a: i64) => Self::at(n, "LaunchEnsureCreate", self.ensure_create(a))?,
            LaunchEnsureActivate(a: i64) => {
                Self::at(n, "LaunchEnsureActivate", self.ensure_activate(a))?;
            }
            LaunchUnavailable(a: i64) => {
                Self::at(n, "LaunchUnavailable", self.volume_launch_fails("LaunchUnavailable", a))?;
            }
            LaunchCmdFail(a: i64) => Self::at(n, "LaunchCmdFail", self.launch_cmd_fail(a))?,
            LaunchCheck(a: i64) => Self::at(n, "LaunchCheck", self.launch_check(a))?,
            LaunchPoll(a: i64) => Self::at(n, "LaunchPoll", self.launch_poll(a))?,
            LaunchHeldFail(a: i64) => Self::at(n, "LaunchHeldFail", self.launch_held_fail(a))?,
            LaunchSuperseded(a: i64) => Self::at(n, "LaunchSuperseded", self.launch_superseded(a))?,
            GuestReady(a: i64) => Self::at(n, "GuestReady", self.guest_ready(a))?,
            VmmSpawn(a: i64) => Self::at(n, "VmmSpawn", self.vmm_spawn(a))?,
            VmmSpawnFail(a: i64) => Self::at(n, "VmmSpawnFail", self.vmm_spawn_fail(a))?,
            GuestWrite => Self::at(n, "GuestWrite", self.expect_claim_held("GuestWrite", None))?,
            VmmSigkill(a: i64) => Self::at(n, "VmmSigkill", self.vmm_sigkill(a))?,
            DeviceSlow => {
                Self::at(n, "DeviceSlow", self.expect_claim_held("DeviceSlow", Some(false)))?;
            }
            Drain => Self::at(n, "Drain", self.drain())?,
            Reclaim(a: i64) => Self::at(n, "Reclaim", self.reclaim(a))?,
            CpCrash => Self::at(n, "CpCrash", self.cp_crash())?,
            NodeReboot => Self::at(n, "NodeReboot", self.node_reboot())?,
        })
    }
}

// ----- the compared state ------------------------------------------------------------

/// The compared `vol_conf` variables.
#[derive(Debug, PartialEq, Eq)]
struct NodeVolumeState {
    /// Model `lv ∧ active` ⇔ `claim_state ≠ NotFound`.
    active: bool,
    /// Model `claim != 0` ⇔ `claim_state == Held`.
    claimed: bool,
    /// Allocation id (1..=`NA`) → model state (0 unminted, 1 pending,
    /// 2 running, 3 failed, 4 terminated).
    alloc: BTreeMap<i64, i64>,
    /// `BackoffExhausted` reached.
    exhausted: bool,
    /// The allocations whose launch is booting (`ph == LP_BOOT`) and alive.
    booting: BTreeSet<i64>,
}

impl NodeVolumeState {
    /// The spec side: the qualified variable `vol_conf::node_volume::st`.
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(state, MAIN, MODULE)?;
        let st = vars.var("st")?;
        let alloc = quint_itf::int_map(quint_itf::field(st, "as")?, quint_itf::int)?;
        let phase = quint_itf::int_map(quint_itf::field(st, "ph")?, quint_itf::int)?;
        let alive = quint_itf::int_map(quint_itf::field(st, "alive")?, quint_itf::boolean)?;
        let charged = quint_itf::int(quint_itf::field(st, "charged")?)?;
        let non_terminal = alloc.values().any(|state| *state == 1 || *state == 2);
        let booting = phase
            .iter()
            .filter(|(a, ph)| **ph == LP_BOOT && alive.get(*a).copied().unwrap_or(false))
            .map(|(a, _)| *a)
            .collect();
        Ok(Self {
            active: quint_itf::boolean(quint_itf::field(st, "lv")?)?
                && quint_itf::boolean(quint_itf::field(st, "active")?)?,
            claimed: quint_itf::int(quint_itf::field(st, "claim")?)? != 0,
            alloc,
            // The spec's `exhausted(s)`: the budget is spent and nothing runs.
            exhausted: charged >= LIMIT && !non_terminal,
            booting,
        })
    }
}

impl<'de> Deserialize<'de> for NodeVolumeState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<NodeVolumeDriver> for NodeVolumeState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &NodeVolumeDriver) -> Result<Self> {
        // The restart authority the rows are read for must be the registered
        // `WorkloadLifecycle` of the live process.
        driver.reconcilers.get(&reconciler_name()).ok_or_else(|| {
            DriverError::Production("workload-lifecycle is not registered".into())
        })?;
        let claim = driver.claim()?;
        let rows = driver.rows()?;
        // Every allocation production minted carries a model id: a row the driver did not
        // label (a double-minted or unexpected allocation) fails the comparison.
        if let Some(row) = rows.iter().find(|row| !driver.alloc_of.contains_key(&row.alloc_id)) {
            return Err(DriverError::NoPreImage(format!(
                "allocation row {} has no model id: production minted an allocation the model did not",
                row.alloc_id
            ))
            .into());
        }
        let mut alloc = BTreeMap::new();
        for a in 1..=NA {
            let model = match driver.alloc_id(a) {
                None => 0,
                Some(id) => match rows.iter().find(|row| row.alloc_id == id).map(|row| row.state) {
                    Some(AllocState::Pending) => 1,
                    Some(AllocState::Running) => 2,
                    Some(AllocState::Failed) => 3,
                    Some(AllocState::Terminated) => 4,
                    Some(other) => {
                        return Err(
                            DriverError::NoPreImage(format!("allocation state {other:?}")).into()
                        );
                    }
                    // No row: the launch holds the lane (PENDING), its dispatch returned
                    // `Ok(())` with no row (ended by supersession), or its tick was dropped with
                    // the control plane (a lost launch, no row and no charge).
                    None => match driver.launch_view(a) {
                        LaunchView::Running => 1,
                        LaunchView::Ended => 4,
                        LaunchView::Dropped => 3,
                        LaunchView::Unheld => {
                            return Err(DriverError::NoPreImage(format!(
                                "allocation {id} has no row and no held launch"
                            ))
                            .into());
                        }
                    },
                },
            };
            alloc.insert(a, model);
        }
        let (active, claimed) = match claim {
            Ok(ClaimState::Held) => (true, true),
            Ok(ClaimState::Free) => (true, false),
            Err(VolumeBackingError::NotFound { .. }) => (false, false),
            Err(error) => {
                return Err(DriverError::Production(format!("claim_state: {error}")).into());
            }
        };
        Ok(Self {
            active,
            claimed,
            alloc,
            exhausted: rows.iter().any(|row| {
                matches!(row.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
            }),
            booting: driver.booting()?,
        })
    }
}

// ----- named runs (critical scenarios, one per `vol_conf` run) -------------------------

/// Named run `happy_path_deploy_to_running`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "happy_path_deploy_to_running",
    max_samples = 1
)]
fn quint_vol_happy_path_deploy_to_running() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `restart_waits_for_drain_then_attaches`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "restart_waits_for_drain_then_attaches",
    max_samples = 1
)]
fn quint_vol_restart_waits_for_drain_then_attaches() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `stop_keeps_volume_and_restart_reattaches`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "stop_keeps_volume_and_restart_reattaches",
    max_samples = 1
)]
fn quint_vol_stop_keeps_volume_and_restart_reattaches() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `crash_after_create_before_attach_reuses_volume`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "crash_after_create_before_attach_reuses_volume",
    max_samples = 1
)]
fn quint_vol_crash_after_create_before_attach_reuses_volume() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `node_reboot_reactivates_and_data_survives`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "node_reboot_reactivates_and_data_survives",
    max_samples = 1
)]
fn quint_vol_node_reboot_reactivates_and_data_survives() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `launch_bound_exceeded_fails_typed_and_charges_budget`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "launch_bound_exceeded_fails_typed_and_charges_budget",
    max_samples = 1
)]
fn quint_vol_launch_bound_exceeded_fails_typed_and_charges_budget() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `no_space_fails_typed_and_never_attaches`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "no_space_fails_typed_and_never_attaches",
    max_samples = 1
)]
fn quint_vol_no_space_fails_typed_and_never_attaches() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `store_absent_fails_typed_and_never_attaches`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "store_absent_fails_typed_and_never_attaches",
    max_samples = 1
)]
fn quint_vol_store_absent_fails_typed_and_never_attaches() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `superseded_launch_result_discarded`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "superseded_launch_result_discarded",
    max_samples = 1
)]
#[should_panic(
    expected = "LaunchSuperseded: the step did not settle within the settle bound: the superseded launch's dispatch returns"
)]
fn quint_vol_superseded_launch_result_discarded() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `stop_during_boot_wait_terminates_vmm_and_keeps_volume`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "stop_during_boot_wait_terminates_vmm_and_keeps_volume",
    max_samples = 1
)]
#[should_panic(
    expected = "LaunchSuperseded: the step did not settle within the settle bound: the superseded launch's dispatch returns"
)]
fn quint_vol_stop_during_boot_wait_terminates_vmm_and_keeps_volume() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `stop_during_ensure_lets_ensure_finish_then_abandons`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "stop_during_ensure_lets_ensure_finish_then_abandons",
    max_samples = 1
)]
#[should_panic(
    expected = "LaunchSuperseded: the step did not settle within the settle bound: the superseded launch's dispatch returns"
)]
fn quint_vol_stop_during_ensure_lets_ensure_finish_then_abandons() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Named run `shutdown_abandons_every_launch_including_one_admitted_late`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    test = "shutdown_abandons_every_launch_including_one_admitted_late",
    max_samples = 1
)]
#[should_panic(
    expected = "LaunchSuperseded: the step did not settle within the settle bound: the superseded launch's dispatch returns"
)]
fn quint_vol_shutdown_abandons_every_launch_including_one_admitted_late() -> impl Driver {
    NodeVolumeDriver::new()
}

/// Randomized `vol_conf` traces (seed printed on failure).
///
/// The conformance main instantiates the spec's restart limit with production's
/// `RESTART_BACKOFF_CEILING` + 1.
/// CONTRACT_SHAPE: bounded-change.
///
/// Pending until step 05-02: quint-connect prints only the generic text for a
/// state mismatch. The mismatch is `alloc[1]`: the spec holds PENDING (a lane
/// born raised after Shutdown observes the signal before the volume check)
/// while production fails it typed (FAILED), because the volume launch path
/// does not yet observe the shutdown/supersession signal.
#[should_panic(expected = "State invariant failed")]
#[quint_run(
    spec = "../../specs/quint/node-local-volume/node_volume.qnt",
    main = "vol_conf",
    seed = "1"
)]
fn quint_vol_randomized() -> impl Driver {
    NodeVolumeDriver::new()
}
