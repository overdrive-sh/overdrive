//! Quint conformance driver — `cid_lease` (CID-NOCLAIM, RESET-EXIT,
//! NO-SURVIVE, RESTART-SSOT; QPROJ-1, QPROJ-2, QPROJ-2W, QPROJ-7; T-5 ruling
//! as revised by DR2-CLOSE, DR5-CLOSE and DR6-CLOSE).
//!
//! Replays `specs/quint/guest-flow-owner/cid_lease.qnt` (referenced in place)
//! on its recording conformance main `lease_conf_ok` (the five usable offsets
//! of the `/29` test pool, the production restart ceiling), for the one run
//! pending GH #325 on `lease_conf_325`, and for item 7's pool-exhaustion run
//! on the three-offset recording main `lease_conf_three` (D-18; the model's
//! three offsets are the last three of the `/29`: two background Service
//! leases hold usable offsets 0 and 1, checked Admitted there at every
//! compared step, model offset `m` ↦ usable offset `m + 2`, `lease` / `dev`
//! compared on usable offsets 2..=4 only, a cursor at 0, 1 or past 4 ↦
//! model 0; [`Instance`]; the feature-delta `cid_lease` map, "Projecting
//! `lease_conf_three` onto the five-offset pool"). Each step's action and picks are
//! read from the single-element path `"<main>::cid_lease::conf_step"`
//! ([`Config::nondet`]); the state projection matches the trace's qualified
//! variable names (`<main>::cid_lease::<var>`, spike QT1).
//!
//! **The production owners under test.** Placement is owned by the
//! `WorkloadLifecycle` reconciler (QPROJ-2W), so the composition registers it
//! in a `ReconcilerRuntime` and drives it only through the runtime's
//! convergence tick (`run_convergence_tick_with_guest_network_provisioner_for_test`:
//! hydrate, `reconcile`, view write-through, `action_shim::dispatch`) and the
//! real operator handlers (`submit_workload`, `stop_workload`,
//! `restart_workload`). Every lease is taken and returned by the action shim
//! dispatching that reconciler's actions over the production
//! `GuestAddressPool` (QPROJ-2); the driver never calls `pool.assign`,
//! `retire` or `release`, and never writes the `IntentStore`. The VM driver is
//! the production `VmDriver` over `SimVmm` (the VMM processes, their exits and
//! guest resets), its beacon over `SimBeaconIo` (the guest's READY is the
//! driver's stimulus), and the production exit observer writes each terminal
//! row. Boot reclamation is the production `vm_reclamation_boot::converge`.
//! Only the accepted driven port of the tick is substituted: a
//! `GuestNetworkProvisioner` whose `provision` and `teardown` the driver holds
//! and returns at `Launch` and `Release` — the step boundaries the model has
//! inside one dispatch, so the dispatch's own order is what is compared.
//! Neither `spawn_convergence_loop` nor `spawn_interest_router` runs: every
//! tick is a driver step.
//!
//! **What survives a `serve` crash.** One node: one `SimVmm` (its processes
//! survive as orphans), one simulated host, one `SimClock`, one
//! `SimObservationStore` (the node's observation store), the
//! `LocalIntentStore` file and the reconciler runtime's view store directory.
//! A `serve` process is its own tokio runtime: a crash drops the runtime, so
//! every task of the process — held dispatches, the exit observer, a pending
//! boot reclamation — dies with it, and its `AppState`, pool and runtime are
//! dropped, releasing every store handle. Every later composition — the next
//! process, and the S-4 read while none is up — reopens the durable files
//! from disk through the composition root's `StoreAdapters::open` over the
//! node's `data_dir` (the intent store, the view store, then its bulk load at
//! registration; D-21), never a retained handle of an earlier process, so
//! what the next boot reads is what the files hold.
//!
//! Action → production call or environment fault (feature-delta § "Per-module
//! projection and action map", `cid_lease`):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh node and `serve` process; boot reclamation over nothing; `submit_workload` of each model workload (a VM Service with one TCP listener) |
//! | `Assign(a)` | one convergence tick of `a`'s workload (the `SimClock` advanced past every backoff): the dispatch takes the lease and is held at the substituted `provision` |
//! | `AssignRefused(a)` | one convergence tick of `a`'s workload with no free offset: `pool.assign` refuses `PoolExhausted` and the dispatch retires the crashed allocation's lease, held at its `teardown`; `a` has no placement lease after the step (C). Unreachable on the five-offset instances (two workloads hold at most four of five offsets); reached on `lease_conf_three` |
//! | `Launch(a)` | the held `provision` returns; the dispatch starts the VMM (`VmDriver::start`, the VMM opens its CID's device), the guest's READY with the lease's launch token is delivered through `SimBeaconIo`, and the dispatch writes the `Running` row; for a restart it then retires the predecessor's lease and is held at its `teardown`. A VMM that finds its device held (the hazard-only launch clash) is an error |
//! | `Stop(a)` | `stop_workload`, then one tick: the dispatch retires the lease and is held at `teardown` |
//! | `VmmCrash(a)` | the VMM exits by itself (`SimVmm::exit_process`); the exit observer writes the terminal row; no tick |
//! | `GuestReset(a)` | `SimVmm::guest_reset` (the fork ends the VMM, RESET-EXIT); then as `VmmCrash(a)` |
//! | `Finalize(a)` | one tick past the ceiling: `FinalizeFailed`; with a lease the dispatch retires it and is held at `teardown`, without one it writes the terminal |
//! | `Release(a)` | the held `teardown` of the placement lease, else of the predecessor lease, returns; the dispatch releases the lease; a workload then stopped gets one more tick, which must take no lease |
//! | `Redeploy(a)` | `restart_workload`; its outcome is checked against the pre-state (`done` → `Resumed`, `failed` → `Restarted`) |
//! | `Crash` | the `serve` process dies (its runtime and composition dropped); every running VMM survives as an orphan holding its device |
//! | `Restart` | a new `serve` process over the same node: `WorkloadLifecycle` registered again (its views bulk-loaded), the VM driver and exit observer started; its boot reclamation (`converge`) starts and parks on its first held kill |
//! | `Reclaim` | D-UP-19: the driver grants each kill `converge` requests until it returns; then one tick of every stopped workload, each of which must take no lease |
//! | `OrphanExit(o)` | the orphan holding offset `o`'s CID exits by itself |
//! | `AssignPick(a)`, `AssignTake(a)` | hazard-only (`assignSplit`); an error |
//!
//! Projection (QPROJ-1 sources). `lease`, `cursor`, `al(a).off`, `.pred`,
//! `.pst` and the lease part of `al(a).st` ← O
//! `GuestAddressPool::lease_observation()` while a `serve` process is up.
//! While none is (from a `Crash` to the next boot's first step), no pool
//! exists (NO-SURVIVE): `lease` and `cursor` are **not compared**
//! ([`WhileUp::NoPool`]; projecting "all free" and 0 would feed driver-chosen
//! literals into `from_driver`, testing.md checklist item 5, and the model's
//! own `Crash` empties them; the next boot's first step compares them
//! against the new pool), and `al` / `att` are the placement owner's
//! hydration over the node's durable stores (S-4), read through a
//! composition built for the read over the reopened files and dropped
//! before the next `Restart`. `dev` and the
//! `running` part of `al(a).st` ← S `SimVmm::vsock_device_holders` (QPROJ-7
//! arbiter, Tier-3 pair V-30 Q2a / Q2c / Q2d and V-RECLAIM); `leased` ⇔ the
//! placement lease is Admitted, no VMM holds its CID's device and its
//! allocation has no row (the owner-read form, architect 2026-10-19: the
//! held provisioner is the step's pacing device only and is never read);
//! `wait` / `done` / `ended` / `failed` without a placement lease, `retry`,
//! `ex` ← O QPROJ-2W (`hydrate_desired`, `hydrate_actual`, `current_alloc`,
//! `restart_status_for_alloc`); `att` ← O `attempt_index_for_alloc − 1`. `up`
//! and `reclaimed` are stimulus (the driver's own `Crash`, `Restart` and
//! `Reclaim` set them; comparing them would compare the driver with itself)
//! and their effects are projected through `lease`, `cursor`, `al`, `att` and
//! `dev`. `chg`, `bootLeft` and `g` are not compared (model-only, stimulus,
//! ghost). The placement lease of `a` is the lease its latest `Assign` step
//! added (C); every other lease of an allocation of `a`'s workload is its
//! predecessor lease. Model offset `o` is the `o`-th of the instance's model
//! offsets, its CID `GUEST_CID_BASE + 2 + background + o`. On
//! `lease_conf_three` every compared step first checks that each background
//! lease is still Admitted on its usable offset (0, 1) and fails the step
//! otherwise; the background leases and devices are never projected. A workload's
//! allocation ids and VMM pids are a relabelling the driver fixes when it
//! first sees them, never a source of state.
//!
//! **Runs not pinned (model-only by design).** `CrashIsNotEnabledInThe325Window`
//! (holds on `lease_conf_ok`, violated on `lease_conf_325`) and its positive
//! twin `CrashIsEnabledInThe325WindowWithoutTheGuard` (holds on
//! `lease_conf_325`, violated on `lease_conf_ok`) check the model's own
//! #325-window guard — a `Crash` not enabled while a `Redeploy`'s placement
//! is leased and not launched — which exists only to keep randomized
//! `lease_conf_ok` traces off the GH #325 defect. No production owner
//! refuses a `serve` crash, so there is nothing to drive (the spec README's
//! `cid_lease` run table and the feature-delta `cid_lease` map list both as
//! "model only, not a `#[quint_test]`"). The behaviour in that window is the
//! pinned `RedeployLeasedAtServeCrashIsPlacedAgain` on `lease_conf_325`
//! (`pending #325`).
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through
//! quint-connect), `cargo xtask lima run --`.

#![allow(
    clippy::or_fun_call,
    clippy::single_match_else,
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    clippy::type_complexity,
    reason = "a conformance driver asserts its own preconditions; CONTRACT_SHAPE lines are machine-read"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::significant_drop_tightening,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State as AxumState};
use axum::http::HeaderMap;
use itf::Value;
use overdrive_core::aggregate::{DriverInput, ResourcesInput, VmInput, WorkloadKind};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid, VsockListenPort};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::GuestAttachmentLease;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_reconcilers::{AnyState, WorkloadLifecycleState, current_alloc};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};
use parking_lot::Mutex;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::quint_itf::{self, ItfError, Qualified};
use super::{
    ForwardingActivation, GuestAddressPool, GuestLeaseView, GuestNetworkPlan,
    GuestNetworkProvisioner, Result as GuestNetworkResult, SharedGuestNetworkOwner as _,
};
use crate::AppState;
use crate::api::{RestartOutcome, SubmitWorkloadRequest};
use crate::handlers::{restart_workload, stop_workload, submit_workload};
use crate::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
    run_convergence_tick_with_guest_network_provisioner_for_test,
};
use crate::vm_reclamation_boot::sim_vm_host::{GATEWAY, RecordingVmm, SimVmmHost};
use crate::vm_reclamation_boot::{ConvergeError, converge};

/// The conformance main (D-UP-1), the main of the run pending GH #325
/// (item 9) and the three-offset recording main of item 7's pool-exhaustion
/// run; the module declaring the variables.
const MAIN: &str = "lease_conf_ok";
const MAIN_325: &str = "lease_conf_325";
const MAIN_THREE: &str = "lease_conf_three";
const MODULE: &str = "cid_lease";

/// Every instance's pool: the `/29` test pool, five usable offsets (2..=6).
const PREFIX: &str = "100.95.0.0/29";
/// The first usable prefix offset (0 is the network, 1 the gateway).
const FIRST_USABLE: u32 = 2;
/// The model's workloads.
const ALLOCS: [i64; 2] = [1, 2];
/// The background workloads of a three-offset instance (stimulus, outside
/// the model): each holds one offset for the whole trace.
const BACKGROUND: [i64; 2] = [101, 102];

/// One conformance instance: its recording main, the model's `N` and how
/// many of the `/29`'s usable offsets background workloads hold first.
///
/// No CIDR prefix leaves exactly three usable offsets (a `/29` leaves five,
/// a `/30` one), so the three-offset instance runs on the production `/29`
/// pool with two background workloads placed and launched at `init` through
/// the same production paths as the model's (`submit_workload`, a tick, the
/// held `provision` returned, the guest's READY). Next-fit places them on
/// the first two usable offsets, so the model's offsets `0..3` are the last
/// three (prefix offsets 4..=6) and the pool's cursor stands at the model's
/// offset 0. A background workload's lease and device are never projected,
/// but every compared step checks both background leases are still Admitted
/// on usable offsets 0 and 1 and fails otherwise. The model's `Crash`,
/// `Restart`, `Stop` and `Redeploy` are not on this instance's named run
/// (each would release a background lease and break the offset bijection),
/// and the instance carries no `#[quint_run]`: it keeps `lease_ok`'s
/// `CEIL` = 2 while production's ceiling is 5, which agree only for the
/// named run's `att` < 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Instance {
    offsets: i64,
    background: u32,
}

/// `lease_conf_ok` and `lease_conf_325`: the five usable offsets.
const FIVE: Instance = Instance { offsets: 5, background: 0 };
/// `lease_conf_three`: the model's three offsets over the `/29`.
const THREE: Instance = Instance { offsets: 3, background: 2 };
/// How long a step may take to settle (real milliseconds; the owners read
/// time only through `SimClock`).
const SETTLE_BOUND: Duration = Duration::from_secs(10);
/// The `SimClock` step before each tick: past every restart backoff, so the
/// reconciler's backoff never defers a placement the model takes in that
/// step (feature-delta `cid_lease`, `Assign`).
const PAST_BACKOFF: Duration = Duration::from_secs(3_600);
/// The workloads' declared listener port (outside the reserved guest range).
const LISTENER_PORT: u16 = 8_080;
/// The guest's beacon source ports (relabelling only).
const BEACON_PORT_BASE: u32 = 50_000;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("hazard-only action `{0}` is not mapped (it is never reached by the design)")]
    HazardOnly(&'static str),
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

/// The workload of model allocation `a`.
fn workload(a: i64) -> String {
    format!("lease-w{a}")
}

/// The reconciler's target of model allocation `a`'s workload.
fn target(a: i64) -> TargetResource {
    TargetResource::new(&format!("workload/{}", workload(a))).expect("a valid target")
}

fn reconciler_name() -> ReconcilerName {
    ReconcilerName::new("workload-lifecycle").expect("a valid reconciler name")
}

impl Instance {
    /// The first prefix offset of the model's offsets.
    const fn first_model(self) -> u32 {
        FIRST_USABLE + self.background
    }

    /// The model offset of `cid`, when it is one of the model's offsets.
    fn offset_of(self, cid: GuestCid) -> Option<i64> {
        let prefix_offset = cid.get().checked_sub(GUEST_CID_BASE)?;
        let o = i64::from(prefix_offset.checked_sub(self.first_model())?);
        (o < self.offsets).then_some(o)
    }

    /// The CID background workload `index` (0 or 1) holds: usable offset
    /// `index`.
    fn background_cid(self, index: u32) -> GuestCid {
        debug_assert!(index < self.background, "a background index of this instance");
        GuestCid::new(GUEST_CID_BASE + FIRST_USABLE + index).expect("an in-range guest CID")
    }

    /// `cid` is a background workload's offset (never projected).
    fn is_background(self, cid: GuestCid) -> bool {
        cid.get()
            .checked_sub(GUEST_CID_BASE)
            .is_some_and(|o| (FIRST_USABLE..self.first_model()).contains(&o))
    }

    /// The CID of model offset `o`.
    fn cid_of(self, o: i64) -> GuestCid {
        GuestCid::new(
            GUEST_CID_BASE + self.first_model() + u32::try_from(o).expect("non-negative offset"),
        )
        .expect("an in-range guest CID")
    }

    /// The model's cursor for the pool's `next_fit`: the model offset of the
    /// first model offset at or after it, 0 when none is (next-fit wraps).
    fn cursor_of(self, next_fit: GuestCid) -> i64 {
        let at = next_fit.get().saturating_sub(GUEST_CID_BASE);
        match at.checked_sub(self.first_model()) {
            None => 0,
            Some(o) => {
                let o = i64::from(o);
                if o < self.offsets { o } else { 0 }
            }
        }
    }
}

// ----- the substituted driven port: the held provisioner --------------------------

/// The two step boundaries the model has inside one dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Provision,
    Teardown,
}

#[derive(Debug, Default)]
struct HoldState {
    /// Calls waiting to return, in arrival order.
    waiting: Vec<(Held, AllocationId)>,
    /// Returns the driver granted and the call has not taken.
    granted: Vec<(Held, AllocationId)>,
}

/// The tick's accepted guest-network driven port, held at `provision` and
/// `teardown` until the driver returns them. It records nothing else and
/// authors no lease, row or decision; `activate` returns `Activated` (no
/// guest owner runs in this composition).
#[derive(Debug, Default)]
struct HeldProvisioner {
    state: Mutex<HoldState>,
    wake: Notify,
}

impl HeldProvisioner {
    async fn hold(&self, kind: Held, alloc: &AllocationId) {
        self.state.lock().waiting.push((kind, alloc.clone()));
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.state.lock();
                if let Some(at) = state.granted.iter().position(|(k, a)| *k == kind && a == alloc) {
                    state.granted.remove(at);
                    if let Some(at) =
                        state.waiting.iter().position(|(k, a)| *k == kind && a == alloc)
                    {
                        state.waiting.remove(at);
                    }
                    return;
                }
            }
            notified.await;
        }
    }

    /// The calls waiting to return.
    fn waiting(&self) -> Vec<(Held, AllocationId)> {
        self.state.lock().waiting.clone()
    }

    fn is_held(&self, kind: Held, alloc: &AllocationId) -> bool {
        self.state.lock().waiting.iter().any(|(k, a)| *k == kind && a == alloc)
    }

    /// Returns the waiting call; `false` when none waits.
    fn release(&self, kind: Held, alloc: &AllocationId) -> bool {
        let released = {
            let mut state = self.state.lock();
            let waiting = state.waiting.iter().any(|(k, a)| *k == kind && a == alloc);
            if waiting {
                state.granted.push((kind, alloc.clone()));
            }
            waiting
        };
        self.wake.notify_waiters();
        released
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for HeldProvisioner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.hold(Held::Provision, plan.alloc()).await;
        Ok(())
    }

    async fn activate(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<ForwardingActivation> {
        Ok(ForwardingActivation::Activated)
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.hold(Held::Teardown, plan.alloc()).await;
        Ok(())
    }
}

// ----- the node and its serve processes ------------------------------------------

/// What outlives a `serve` process: the machine and its persistent state.
struct Node {
    tmp: TempDir,
    vmm: SimVmm,
    host: Arc<SimVmmHost>,
    clock: Arc<SimClock>,
    obs: Arc<SimObservationStore>,
}

impl Node {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        stage_artifacts(tmp.path());
        let vmm = SimVmm::new();
        Self {
            host: SimVmmHost::new(vmm.clone(), true),
            vmm,
            clock: Arc::new(SimClock::new()),
            obs: Arc::new(SimObservationStore::single_peer(
                NodeId::new("cid-lease").expect("valid node ID"),
                0,
            )),
            tmp,
        }
    }

    fn root(&self) -> &FsPath {
        self.tmp.path()
    }
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
/// and the dispatches it has in flight.
struct Serve {
    rt: Runtime,
    state: AppState,
    provisioner: Arc<HeldProvisioner>,
    beacon: Arc<SimBeaconIo>,
    /// The port this process's beacon intake bound (`BeaconIntake::bound_port`,
    /// PORTS-CONFIG); the guest's READY names it (GUEST-PORTS). Stimulus.
    beacon_port: VsockPort,
    /// The boot reclamation of this process (C): pending, or its result.
    reclaim: Option<JoinHandle<std::result::Result<(), ConvergeError>>>,
    reclaimed: bool,
    /// The tick of each model workload whose dispatch has not returned.
    ticks: BTreeMap<i64, JoinHandle<std::result::Result<(), ConvergenceError>>>,
    tick_n: u64,
}

/// The production composition over `node`'s persistent state, nothing
/// spawned: no exit observer, no boot reclamation, no tick.
struct Composed {
    rt: Runtime,
    state: AppState,
    pool: Arc<GuestAddressPool>,
    reconcilers: Arc<ReconcilerRuntime>,
    vm_driver: VmDriver,
    beacon: Arc<SimBeaconIo>,
    beacon_port: VsockPort,
}

/// The production composition of a `serve` process over `node`: the node's
/// intent store, observation store and view store, a fresh pool and VM
/// driver. Constructs only; spawns nothing.
fn compose(node: &Node, boot: u32) -> Composed {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let root = node.root().to_path_buf();
    let beacon = Arc::new(SimBeaconIo::stepped(node.vmm.clone()));
    let clock = Arc::clone(&node.clock);
    let (state, pool, reconcilers, vm_driver, beacon_port) = rt.block_on(async {
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(wiring.supervisor().open_after_boot(), "the composition's EXEC gate opens");
        // PORTS-CONFIG: a kernel-assigned beacon port (the sim chooses one
        // above 1023); nothing here reads a port number.
        let intake = BeaconIntake::bind(
            Arc::clone(&beacon) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds");
        let beacon_port = intake.bound_port();
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&node.obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
        ));
        let vm_driver = VmDriver::new(
            Arc::new(RecordingVmm { inner: node.vmm.clone(), host: Arc::clone(&node.host) }),
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
        );
        // D-21: every composition reopens the node's stores from disk
        // through the composition root's open (`StoreAdapters::open` over
        // the node's `data_dir`); no handle of an earlier process is reused.
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
        // Boot step 9 on the guest-network owner port: its first `Ok`
        // `converge_shared` makes `flow_ports()` `Some` (PORTS-CONFIG), the
        // ports every lease the action shim takes carries. The owner is the
        // test-local `TestSharedOwner`; placement, not forwarding, is the
        // subject here.
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
        (state, pool, reconcilers, vm_driver, beacon_port)
    });
    Composed { rt, state, pool, reconcilers, vm_driver, beacon, beacon_port }
}

/// A new `serve` process over `node`: the production composition with the
/// node's persistent state; its exit observer and boot reclamation started.
fn boot_serve(node: &Node, boot: u32) -> (Serve, Arc<GuestAddressPool>, Arc<ReconcilerRuntime>) {
    let Composed { rt, state, pool, reconcilers, vm_driver, beacon, beacon_port } =
        compose(node, boot);
    let clock = Arc::clone(&node.clock);
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
        provisioner: Arc::new(HeldProvisioner::default()),
        beacon,
        beacon_port,
        reclaim: Some(reclaim),
        reclaimed: false,
        ticks: BTreeMap::new(),
        tick_n: 0,
    };
    (serve, pool, reconcilers)
}

/// The reconciler's reading of one workload (QPROJ-2W).
struct Reading {
    desired: WorkloadLifecycleState,
    actual: WorkloadLifecycleState,
}

impl Reading {
    /// `run` = `job.is_some() ∧ ¬desired_to_stop`.
    fn run(&self) -> bool {
        self.desired.job.is_some() && !self.desired.desired_to_stop
    }

    /// The current allocation by the reconciler's own rule.
    fn current(&self) -> Option<&AllocStatusRow> {
        let rows: Vec<&AllocStatusRow> = self.actual.allocations.values().collect();
        current_alloc(&rows)
    }

    fn row(&self, alloc: &AllocationId) -> Option<&AllocStatusRow> {
        self.actual.allocations.get(alloc)
    }
}

fn ended_crash_class(row: &AllocStatusRow) -> bool {
    matches!(row.state, AllocState::Failed | AllocState::Terminated)
}

struct CidLeaseDriver {
    /// The instance the driver replays.
    inst: Instance,
    /// The production pool of the current `serve` process (QPROJ-2). After a
    /// crash it is the dead process's pool, never read.
    pool: Arc<GuestAddressPool>,
    /// The production reconciler runtime of the current `serve` process
    /// (QPROJ-2W). After a crash it is a placeholder over its own view store,
    /// so the node's view store is free for the next process; never read.
    reconcilers: Arc<ReconcilerRuntime>,
    node: Node,
    serve: Option<Serve>,
    boots: u32,
    crashes: u32,
    /// Placement lease of each model workload: the allocation its latest
    /// `Assign` step leased (C); cleared by a crash.
    placement: BTreeMap<i64, AllocationId>,
    /// Allocation ↦ model workload (relabelling, fixed when first seen).
    alloc_of: BTreeMap<AllocationId, i64>,
    next_beacon_port: u32,
}

/// What a projection reads the placement owner through: the live `serve`
/// process's composition, or, while no `serve` process is up, a composition
/// over the node's durable stores built for the read and dropped after it
/// (round-9 S-4: `al` / `att` through the durable stores' hydration).
struct Reader<'a> {
    rt: &'a Runtime,
    state: &'a AppState,
    reconcilers: &'a ReconcilerRuntime,
}

/// The pool's leases as one `lease_observation()` found them (empty while
/// no `serve` process is up: no pool exists, NO-SURVIVE).
type Leases = BTreeMap<AllocationId, GuestLeaseView>;

impl CidLeaseDriver {
    fn new() -> Self {
        Self::over(FIVE)
    }

    fn over(inst: Instance) -> Self {
        let node = Node::new();
        let (serve, pool, reconcilers) = boot_serve(&node, 1);
        Self {
            inst,
            pool,
            reconcilers,
            node,
            serve: Some(serve),
            boots: 1,
            crashes: 0,
            placement: BTreeMap::new(),
            alloc_of: BTreeMap::new(),
            next_beacon_port: BEACON_PORT_BASE,
        }
    }

    fn serve(&self, action: &'static str) -> std::result::Result<&Serve, DriverError> {
        self.serve.as_ref().ok_or_else(|| not_enabled(action, "serve is down"))
    }

    fn serve_mut(&mut self, action: &'static str) -> std::result::Result<&mut Serve, DriverError> {
        self.serve.as_mut().ok_or_else(|| not_enabled(action, "serve is down"))
    }

    /// Runs `cond` on the serve process's runtime until it holds, polling
    /// every millisecond.
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
            match &self.serve {
                Some(serve) => {
                    serve.rt.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
                }
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        Ok(())
    }

    /// The live `serve` process's composition as a [`Reader`].
    fn up_reader(&self) -> std::result::Result<Reader<'_>, DriverError> {
        let serve = self.serve("project")?;
        Ok(Reader { rt: &serve.rt, state: &serve.state, reconcilers: &self.reconcilers })
    }

    /// QPROJ-2W: the reconciler's own hydration of `a`'s workload, through
    /// the live `serve` process.
    fn reading(&self, a: i64) -> std::result::Result<Reading, DriverError> {
        Self::reading_in(&self.up_reader()?, a)
    }

    /// QPROJ-2W: the reconciler's own hydration of `a`'s workload.
    fn reading_in(reader: &Reader<'_>, a: i64) -> std::result::Result<Reading, DriverError> {
        let name = reconciler_name();
        let reconciler = reader.reconcilers.get(&name).ok_or_else(|| {
            DriverError::Production("workload-lifecycle is not registered".into())
        })?;
        let target = target(a);
        let (desired, actual) = reader.rt.block_on(async {
            let desired = hydrate_desired_for_test(reconciler, &target, reader.state).await;
            let actual = hydrate_actual_for_test(reconciler, &target, reader.state).await;
            (desired, actual)
        });
        let unpack = |state| match state {
            Ok(AnyState::WorkloadLifecycle(state)) => Ok(state),
            Ok(_) => Err(DriverError::Production("not the workload-lifecycle state".into())),
            Err(error) => Err(DriverError::Production(format!("hydration failed: {error}"))),
        };
        Ok(Reading { desired: unpack(desired)?, actual: unpack(actual)? })
    }

    /// The reconciler's decision for the failed allocation `row`
    /// (`restart_status_for_alloc(..).1`, RESTART-SSOT).
    fn decision(reader: &Reader<'_>, a: i64, reading: &Reading, row: &AllocStatusRow) -> bool {
        reader
            .reconcilers
            .restart_status_for_alloc(
                &target(a),
                WorkloadKind::Service,
                reading.run(),
                row,
                reading.desired.generation,
            )
            .1
    }

    /// The pool's leases (QPROJ-2) while a `serve` process is up; none
    /// while it is down.
    fn leases(&self) -> Leases {
        if self.serve.is_some() { self.pool.lease_observation().leases } else { Leases::new() }
    }

    /// The leases of model workload `a`'s allocations.
    fn leases_of(
        &self,
        leases: &Leases,
        a: i64,
    ) -> Vec<(AllocationId, GuestCid, GuestAttachmentLease)> {
        leases
            .iter()
            .filter(|(alloc, _)| self.alloc_of.get(*alloc) == Some(&a))
            .map(|(alloc, view)| (alloc.clone(), view.cid, view.state))
            .collect()
    }

    /// `a`'s placement lease while the pool holds it.
    fn placement_lease_in(
        &self,
        leases: &Leases,
        a: i64,
    ) -> Option<(AllocationId, GuestCid, GuestAttachmentLease)> {
        let alloc = self.placement.get(&a)?;
        self.leases_of(leases, a).into_iter().find(|(lease, _, _)| lease == alloc)
    }

    /// `a`'s placement lease in the live pool.
    fn placement_lease(&self, a: i64) -> Option<(AllocationId, GuestCid, GuestAttachmentLease)> {
        self.placement_lease_in(&self.leases(), a)
    }

    /// `a`'s predecessor lease: any other lease of its workload.
    fn predecessor_lease_in(
        &self,
        leases: &Leases,
        a: i64,
    ) -> Option<(AllocationId, GuestCid, GuestAttachmentLease)> {
        let placement = self.placement.get(&a);
        self.leases_of(leases, a).into_iter().find(|(lease, _, _)| Some(lease) != placement)
    }

    /// The VMM of `alloc`'s launch holding `cid`'s device (S arbiter).
    fn launch_vmm(&self, alloc: &AllocationId, cid: GuestCid) -> Option<u32> {
        // conformance-lint: sim-arbiter-ok QPROJ-7 the simulated host's VM scopes (the VMM processes an allocation's launch created; the VmHostState port's fact), Tier-3 pair V-30 Q2c / Q2d and V-RECLAIM
        let pids = self.node.host.pids_of(alloc);
        // conformance-lint: sim-arbiter-ok QPROJ-7 SimVmm vsock_device_holders (the kernel's vhost-vsock holder per CID), Tier-3 pair V-30 Q2a / Q2c / Q2d
        self.node.vmm.vsock_device_holders().get(&cid).copied().filter(|pid| pids.contains(pid))
    }

    // ----- ticks and boot reclamation --------------------------------------------

    /// One production convergence tick of `a`'s workload, spawned on the
    /// serve process's runtime (its dispatch may be held at the provisioner).
    fn spawn_tick(&mut self, action: &'static str, a: i64) -> std::result::Result<(), DriverError> {
        let clock = Arc::clone(&self.node.clock);
        let serve = self.serve_mut(action)?;
        if serve.ticks.get(&a).is_some_and(|tick| !tick.is_finished()) {
            return Err(not_enabled(action, format!("a dispatch of {a}'s workload is held")));
        }
        serve.tick_n += 1;
        let tick_n = serve.tick_n;
        let state = serve.state.clone();
        let provisioner = Arc::clone(&serve.provisioner);
        let tick = serve.rt.spawn(async move {
            let now = clock.now();
            run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &reconciler_name(),
                &target(a),
                now,
                tick_n,
                now + Duration::from_secs(1),
                provisioner.as_ref(),
            )
            .await
        });
        serve.ticks.insert(a, tick);
        Ok(())
    }

    fn tick_finished(&self, a: i64) -> bool {
        self.serve
            .as_ref()
            .and_then(|serve| serve.ticks.get(&a))
            .is_none_or(JoinHandle::is_finished)
    }

    /// Takes the finished tick of `a` and checks it returned `Ok`.
    fn reap_tick(&mut self, action: &'static str, a: i64) -> std::result::Result<(), DriverError> {
        let serve = self.serve_mut(action)?;
        if let Some(tick) = serve.ticks.remove(&a) {
            if !tick.is_finished() {
                serve.ticks.insert(a, tick);
                return Ok(());
            }
            let outcome = serve.rt.block_on(tick).expect("the tick did not panic");
            outcome
                .map_err(|error| DriverError::Production(format!("the tick failed: {error}")))?;
        }
        Ok(())
    }

    /// Takes the finished restart tick whose successor was refused by the
    /// full CID pool. The action shim preserves that typed refusal as the
    /// tick's error after it completes the predecessor cleanup; the model's
    /// `AssignRefused` action includes both effects in that order.
    fn reap_pool_refused_tick(
        &mut self,
        action: &'static str,
        a: i64,
    ) -> std::result::Result<(), DriverError> {
        let serve = self.serve_mut(action)?;
        let Some(tick) = serve.ticks.remove(&a) else {
            return Err(not_enabled(action, format!("{a} has no refused restart tick")));
        };
        if !tick.is_finished() {
            serve.ticks.insert(a, tick);
            return Ok(());
        }
        let outcome = serve.rt.block_on(tick).expect("the tick did not panic");
        match outcome {
            Err(ConvergenceError::Shim(crate::action_shim::ShimError::GuestNetwork(
                super::GuestNetworkError::PoolExhausted { .. },
            ))) => Ok(()),
            Ok(()) => Err(not_enabled(action, "the pool-refused restart returned Ok")),
            Err(error) => Err(DriverError::Production(format!("the tick failed: {error}"))),
        }
    }

    /// `a`'s dispatch is held at one of the provisioner's boundaries.
    fn held_for(&self, a: i64) -> Option<(Held, AllocationId)> {
        let serve = self.serve.as_ref()?;
        serve
            .provisioner
            .waiting()
            .into_iter()
            .find(|(_, alloc)| self.alloc_of.get(alloc) == Some(&a))
    }

    /// One tick of every stopped workload, each of which must take no lease
    /// (`StoppedWorkloadNotPlaced`, checked against the reconciler).
    fn tick_stopped_takes_no_lease(
        &mut self,
        action: &'static str,
    ) -> std::result::Result<(), DriverError> {
        for a in ALLOCS {
            if self.reading(a)?.desired.desired_to_stop && self.placement_lease(a).is_none() {
                self.tick_takes_no_lease(action, a)?;
            }
        }
        Ok(())
    }

    fn tick_takes_no_lease(
        &mut self,
        action: &'static str,
        a: i64,
    ) -> std::result::Result<(), DriverError> {
        let before = self.serve(action)?.provisioner.waiting();
        self.spawn_tick(action, a)?;
        self.settle("the stopped workload's tick returns or places", |driver| {
            driver.tick_finished(a)
                || driver.serve.as_ref().is_some_and(|serve| serve.provisioner.waiting() != before)
        })?;
        if self.serve(action)?.provisioner.waiting() != before {
            return Err(not_enabled(action, format!("the tick placed stopped workload {a}")));
        }
        self.reap_tick(action, a)
    }

    /// Settles boot reclamation: until it returned, or it waits on a held
    /// kill of a live orphan.
    fn settle_reclaim(&mut self) -> std::result::Result<(), DriverError> {
        self.settle("boot reclamation returns or parks on a held kill", |driver| {
            let Some(serve) = driver.serve.as_ref() else { return true };
            let parked =
                driver.node.host.kills_requested().iter().any(|pid| driver.node.vmm.is_live(*pid));
            serve.reclaim.as_ref().is_none_or(JoinHandle::is_finished) || parked
        })?;
        if let Some(serve) = self.serve.as_mut()
            && serve.reclaim.as_ref().is_some_and(JoinHandle::is_finished)
        {
            let handle = serve.reclaim.take().expect("checked above");
            let outcome = serve.rt.block_on(handle).expect("converge did not panic");
            serve.reclaimed = outcome.is_ok();
        }
        Ok(())
    }

    // ----- actions --------------------------------------------------------------

    fn init(&mut self) -> std::result::Result<(), DriverError> {
        // A fresh node and `serve` process per trace (the runner reuses one
        // driver for every trace).
        *self = Self::over(self.inst);
        self.settle_reclaim()?;
        let background = &BACKGROUND[..usize::try_from(self.inst.background).unwrap_or(0)];
        for a in ALLOCS.iter().chain(background).copied() {
            let request = SubmitWorkloadRequest {
                spec: SubmitSpecInput::Service(ServiceSpecInput {
                    id: workload(a),
                    replicas: 1,
                    resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
                    driver: DriverInput::Vm(VmInput {
                        command: "/sbin/init".to_owned(),
                        args: Vec::new(),
                        kernel: self.node.root().join("vmlinuz").display().to_string(),
                        rootfs: self.node.root().join("master.img").display().to_string(),
                    }),
                    listeners: vec![ListenerInput {
                        port: LISTENER_PORT,
                        protocol: "tcp".to_owned(),
                    }],
                    startup_probes: Vec::new(),
                    readiness_probes: Vec::new(),
                    liveness_probes: Vec::new(),
                }),
            };
            let serve = self.serve("init")?;
            serve
                .rt
                .block_on(submit_workload(
                    AxumState(serve.state.clone()),
                    HeaderMap::new(),
                    Json(request),
                ))
                .map_err(|error| DriverError::Production(format!("submit_workload: {error}")))?;
        }
        // The background workloads take the first usable offsets and run
        // for the whole trace, through the same production paths as the
        // model's (stimulus; never projected).
        for b in background.iter().copied() {
            self.assign(b)?;
            self.launch(b)?;
        }
        // Next-fit from a fresh pool places them on usable offsets 0 and 1;
        // the relabelling of the model's offsets rests on it.
        self.check_background("init")?;
        Ok(())
    }

    /// On a three-offset instance: each background workload's lease is
    /// Admitted on its usable offset (0, 1) in the live pool (O QPROJ-2).
    /// A step that released or moved one breaks the offset relabelling, so
    /// it fails the step. A no-op on the five-offset instances.
    fn check_background(&self, action: &'static str) -> std::result::Result<(), DriverError> {
        let leases = self.leases();
        for (index, b) in BACKGROUND
            .iter()
            .copied()
            .take(usize::try_from(self.inst.background).unwrap_or(0))
            .enumerate()
        {
            let index = u32::try_from(index).expect("two background workloads");
            let expected = self.inst.background_cid(index);
            let held = self.placement.get(&b).and_then(|alloc| leases.get(alloc));
            match held {
                Some(view)
                    if view.cid == expected && view.state == GuestAttachmentLease::Admitted => {}
                other => {
                    return Err(not_enabled(
                        action,
                        format!(
                            "background workload {b} must hold an Admitted lease on CID {} \
                             (usable offset {index}); the pool shows {other:?}",
                            expected.get()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// `AssignRefused(a)`: the same tick as `Assign`, from `ended` with
    /// `retry` and no free offset. `pool.assign` refuses `PoolExhausted`
    /// (an admission refusal, `action_shim/mod.rs:540-548`), so the dispatch
    /// places nothing and runs the predecessor cleanup at once
    /// (`finish_restart`): it retires the crashed allocation's lease and is
    /// held at that lease's `teardown`. The step leased nothing, so `a` has
    /// no placement lease after it (C); the retired lease is its predecessor.
    fn assign_refused(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let crashed = self
            .placement
            .get(&a)
            .cloned()
            .ok_or_else(|| not_enabled("AssignRefused", format!("{a} has no lease")))?;
        let before: BTreeSet<AllocationId> = self.alloc_of.keys().cloned().collect();
        self.node.clock.tick(PAST_BACKOFF);
        self.spawn_tick("AssignRefused", a)?;
        let leased_new = |driver: &Self| {
            driver.serve.as_ref().is_some_and(|serve| {
                serve
                    .provisioner
                    .waiting()
                    .iter()
                    .any(|(kind, alloc)| *kind == Held::Provision && !before.contains(alloc))
            })
        };
        self.settle(
            "the refused restart holds at the predecessor's teardown, or returns",
            |driver| driver.tick_finished(a) || driver.held_for(a).is_some() || leased_new(driver),
        )?;
        if leased_new(self) {
            return Err(not_enabled(
                "AssignRefused",
                "the tick leased an offset: the pool had one free",
            ));
        }
        match self.held_for(a) {
            Some((Held::Teardown, alloc)) if alloc == crashed => {
                self.placement.remove(&a);
                Ok(())
            }
            _ => {
                self.reap_tick("AssignRefused", a)?;
                Err(not_enabled(
                    "AssignRefused",
                    format!("the refused restart of {a} retired no lease of {crashed}"),
                ))
            }
        }
    }

    fn assign(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let before: BTreeSet<AllocationId> = self.alloc_of.keys().cloned().collect();
        self.node.clock.tick(PAST_BACKOFF);
        self.spawn_tick("Assign", a)?;
        self.settle("the tick leases and holds at provision, or returns", |driver| {
            driver.tick_finished(a)
                || driver.serve.as_ref().is_some_and(|serve| {
                    serve
                        .provisioner
                        .waiting()
                        .iter()
                        .any(|(kind, alloc)| *kind == Held::Provision && !before.contains(alloc))
                })
        })?;
        let held = self
            .serve("Assign")?
            .provisioner
            .waiting()
            .into_iter()
            .find(|(kind, alloc)| *kind == Held::Provision && !before.contains(alloc));
        let Some((_, alloc)) = held else {
            self.reap_tick("Assign", a)?;
            return Err(not_enabled(
                "Assign",
                format!("the tick of {a}'s workload placed nothing"),
            ));
        };
        self.alloc_of.insert(alloc.clone(), a);
        self.placement.insert(a, alloc);
        Ok(())
    }

    fn launch(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let alloc = self
            .placement
            .get(&a)
            .cloned()
            .ok_or_else(|| not_enabled("Launch", format!("{a} has no placement lease")))?;
        let (cid, launch) = self
            .pool
            .lease_observation()
            .leases
            .get(&alloc)
            .map(|view| (view.cid, view.launch))
            .ok_or_else(|| not_enabled("Launch", format!("{alloc} holds no lease")))?;
        if !self.serve("Launch")?.provisioner.release(Held::Provision, &alloc) {
            return Err(not_enabled("Launch", format!("{alloc}'s dispatch is not held")));
        }
        self.settle("the VMM opens its device, or the dispatch ends", |driver| {
            driver.launch_vmm(&alloc, cid).is_some()
                || driver.tick_finished(a)
                || driver.held_for(a).is_some_and(|(kind, _)| kind == Held::Teardown)
        })?;
        let Some(pid) = self.launch_vmm(&alloc, cid) else {
            return Err(DriverError::HazardOnly("Launch with its CID's device held (a clash)"));
        };
        // The guest's READY, with the launch token and the beacon port it
        // read from its kernel command line (stimulus; GUEST-PORTS: `port=`
        // is the port this process's intake bound).
        self.next_beacon_port += 1;
        let key = BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(self.next_beacon_port) };
        let ready =
            format!("READY pid=1 port={} launch={launch}", self.serve("Launch")?.beacon_port);
        {
            let beacon = Arc::clone(&self.serve("Launch")?.beacon);
            beacon
                .queue_connection(key, pid, &[ready.as_str()])
                .map_err(|error| DriverError::Production(format!("the guest's beacon: {error}")))?;
            beacon.release_accept();
        }
        self.settle("the beacon connection is accepted", |driver| {
            driver.serve.as_ref().is_some_and(|serve| serve.beacon.release_read(key))
        })?;
        self.settle("the dispatch returns, or holds at a teardown", |driver| {
            driver.tick_finished(a)
                || driver.held_for(a).is_some_and(|(kind, _)| kind == Held::Teardown)
        })?;
        if self.held_for(a).is_none() {
            self.reap_tick("Launch", a)?;
        }
        Ok(())
    }

    fn stop(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let serve = self.serve("Stop")?;
        let _stopped = serve
            .rt
            .block_on(stop_workload(AxumState(serve.state.clone()), Path(workload(a))))
            .map_err(|error| DriverError::Production(format!("stop_workload: {error}")))?;
        self.node.clock.tick(PAST_BACKOFF);
        self.spawn_tick("Stop", a)?;
        self.settle("the stop's dispatch holds at teardown", |driver| {
            driver.tick_finished(a)
                || driver.held_for(a).is_some_and(|(kind, _)| kind == Held::Teardown)
        })?;
        if self.held_for(a).is_none() {
            self.reap_tick("Stop", a)?;
            return Err(not_enabled("Stop", format!("the stop of {a} retired no lease")));
        }
        Ok(())
    }

    /// The VMM of `a`'s placement lease ended: the exit observer writes its
    /// terminal row. No tick runs (DR5-CLOSE, item 7.1).
    fn vmm_ended(
        &mut self,
        action: &'static str,
        a: i64,
        end: impl FnOnce(&SimVmm, u32) -> bool,
    ) -> std::result::Result<(), DriverError> {
        let (alloc, cid, _) = self
            .placement_lease(a)
            .ok_or_else(|| not_enabled(action, format!("{a} has no lease")))?;
        let pid = self
            .launch_vmm(&alloc, cid)
            .ok_or_else(|| not_enabled(action, format!("{a} has no running VMM")))?;
        if !end(&self.node.vmm, pid) {
            return Err(not_enabled(action, "the VMM is gone"));
        }
        self.serve(action)?.beacon.observe_vm_exits();
        self.node.host.notify();
        self.settle("the exit observer writes the terminal row", |driver| {
            driver
                .reading(a)
                .ok()
                .is_some_and(|reading| reading.row(&alloc).is_some_and(ended_crash_class))
        })
    }

    fn vmm_crash(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        self.vmm_ended("VmmCrash", a, |vmm, pid| vmm.exit_process(pid, Some(1), None))
    }

    fn guest_reset(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        self.vmm_ended("GuestReset", a, SimVmm::guest_reset)
    }

    fn finalize(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let leased = self.placement_lease(a).is_some();
        self.node.clock.tick(PAST_BACKOFF);
        self.spawn_tick("Finalize", a)?;
        self.settle("FinalizeFailed's dispatch holds at teardown, or returns", |driver| {
            driver.tick_finished(a)
                || driver.held_for(a).is_some_and(|(kind, _)| kind == Held::Teardown)
        })?;
        if leased && self.held_for(a).is_none() {
            self.reap_tick("Finalize", a)?;
            return Err(not_enabled("Finalize", format!("the finalize of {a} retired no lease")));
        }
        if self.held_for(a).is_none() {
            self.reap_tick("Finalize", a)?;
        }
        Ok(())
    }

    fn release(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let placement = self.placement.get(&a).cloned();
        let pool_refused_restart = placement.is_none();
        let provisioner = Arc::clone(&self.serve("Release")?.provisioner);
        let released = match &placement {
            Some(alloc) if provisioner.is_held(Held::Teardown, alloc) => {
                provisioner.release(Held::Teardown, alloc);
                self.placement.remove(&a);
                alloc.clone()
            }
            _ => {
                let (_, alloc) = self
                    .held_for(a)
                    .filter(|(kind, _)| *kind == Held::Teardown)
                    .ok_or_else(|| not_enabled("Release", format!("no teardown of {a} is held")))?;
                provisioner.release(Held::Teardown, &alloc);
                alloc
            }
        };
        self.settle("the dispatch releases the lease and returns, or holds again", |driver| {
            driver.tick_finished(a)
                || driver.held_for(a).is_some_and(|(_, alloc)| alloc != released)
        })?;
        if self.held_for(a).is_none() {
            if pool_refused_restart {
                self.reap_pool_refused_tick("Release", a)?;
            } else {
                self.reap_tick("Release", a)?;
            }
            let reading = self.reading(a)?;
            if reading.desired.desired_to_stop && self.placement_lease(a).is_none() {
                self.tick_takes_no_lease("Release", a)?;
            }
        }
        Ok(())
    }

    fn redeploy(&mut self, a: i64) -> std::result::Result<(), DriverError> {
        let pre = self.al_view(a)?.st;
        let serve = self.serve("Redeploy")?;
        let response = serve
            .rt
            .block_on(restart_workload(AxumState(serve.state.clone()), Path(workload(a))))
            .map_err(|error| DriverError::Production(format!("restart_workload: {error}")))?;
        let expected = match pre.as_str() {
            "done" => RestartOutcome::Resumed,
            "failed" => RestartOutcome::Restarted,
            other => return Err(not_enabled("Redeploy", format!("{a} is {other}"))),
        };
        if response.0.outcome != expected {
            return Err(DriverError::Production(format!(
                "restart_workload answered {:?} from {pre}, not {expected:?}",
                response.0.outcome
            )));
        }
        Ok(())
    }

    /// `serve` crashes: its runtime (every task of the process) and its
    /// composition die; the VMMs do not.
    fn crash(&mut self) {
        if let Some(serve) = self.serve.take() {
            drop(serve);
        }
        // The node's view store must be free for the next process.
        self.crashes += 1;
        let placeholder = self.node.root().join(format!("down-{}", self.crashes));
        std::fs::create_dir_all(&placeholder).expect("a placeholder view store directory");
        self.reconcilers = Arc::new(
            ReconcilerRuntime::new_with_redb_view_store_for_test(&placeholder)
                .expect("a placeholder reconciler runtime"),
        );
        self.node.host.clear_requests();
        self.placement.clear();
    }

    fn restart(&mut self) -> std::result::Result<(), DriverError> {
        if self.serve.is_some() {
            return Err(not_enabled("Restart", "serve is up"));
        }
        self.boots += 1;
        let (serve, pool, reconcilers) = boot_serve(&self.node, self.boots);
        self.serve = Some(serve);
        self.pool = pool;
        self.reconcilers = reconcilers;
        self.settle_reclaim()
    }

    /// The orphan VMM holding offset `o`'s CID.
    fn orphan_at(&self, action: &'static str, o: i64) -> std::result::Result<u32, DriverError> {
        let cid = self.inst.cid_of(o);
        self.node
            .vmm
            .vsock_device_holders()
            .get(&cid)
            .copied()
            .ok_or_else(|| not_enabled(action, format!("no VMM holds offset {o}'s device")))
    }

    /// D-UP-19: the one boot reclamation (the `converge` this process
    /// started) kills every orphan before any `assign`; then every stopped
    /// workload's tick takes no lease.
    fn reclaim(&mut self) -> std::result::Result<(), DriverError> {
        if self.serve("Reclaim")?.reclaim.is_none() {
            return Err(not_enabled("Reclaim", "this process's boot reclamation already returned"));
        }
        loop {
            self.settle_reclaim()?;
            if self.serve("Reclaim")?.reclaim.is_none() {
                break;
            }
            let pending: Vec<u32> = self
                .node
                .host
                .kills_requested()
                .iter()
                .copied()
                .filter(|pid| self.node.vmm.is_live(*pid))
                .collect();
            if pending.is_empty() {
                return Err(DriverError::Unsettled(
                    "boot reclamation neither returned nor requested a kill".to_owned(),
                ));
            }
            for pid in pending {
                self.node.host.grant_kill(pid);
                self.settle("the orphan is killed", |driver| !driver.node.vmm.is_live(pid))?;
            }
        }
        if !self.serve("Reclaim")?.reclaimed {
            return Err(not_enabled("Reclaim", "boot reclamation returned an error"));
        }
        self.tick_stopped_takes_no_lease("Reclaim")
    }

    fn orphan_exit(&mut self, o: i64) -> std::result::Result<(), DriverError> {
        let pid = self.orphan_at("OrphanExit", o)?;
        self.node.vmm.exit_process(pid, Some(0), None);
        self.node.host.notify();
        self.settle_reclaim()
    }

    // ----- the projection ------------------------------------------------------

    /// `al(a)` while `serve` is up (a handler's pre-state: stimulus).
    fn al_view(&self, a: i64) -> std::result::Result<AlView, DriverError> {
        self.al_view_in(&self.up_reader()?, &self.leases(), a)
    }

    /// `al(a)` read through `reader` over the pool's `leases`.
    fn al_view_in(
        &self,
        reader: &Reader<'_>,
        leases: &Leases,
        a: i64,
    ) -> std::result::Result<AlView, DriverError> {
        let reading = Self::reading_in(reader, a)?;
        let placement = self.placement_lease_in(leases, a);
        let (pred, pst) = match self.predecessor_lease_in(leases, a) {
            None => (-1, "none"),
            Some((_, cid, state)) => (
                self.inst
                    .offset_of(cid)
                    .ok_or_else(|| DriverError::NoPreImage(format!("CID {}", cid.get())))?,
                match state {
                    GuestAttachmentLease::Admitted => "held",
                    GuestAttachmentLease::Retiring => "retiring",
                },
            ),
        };
        let mut retry = false;
        let mut ex = false;
        let (st, off) = match &placement {
            Some((alloc, cid, state)) => {
                let off = self
                    .inst
                    .offset_of(*cid)
                    .ok_or_else(|| DriverError::NoPreImage(format!("CID {}", cid.get())))?;
                let row = reading.row(alloc);
                // conformance-lint: sim-arbiter-ok QPROJ-7 SimVmm vsock_device_holders (no VMM holds the lease's CID device), Tier-3 pair V-30 Q2a / Q2c / Q2d
                let unheld = !self.node.vmm.vsock_device_holders().contains_key(cid);
                let st = match state {
                    GuestAttachmentLease::Retiring => "retiring",
                    GuestAttachmentLease::Admitted if self.launch_vmm(alloc, *cid).is_some() => {
                        "running"
                    }
                    // The owner-read form (architect 2026-10-19): Admitted, no
                    // VMM holds the CID's device and the allocation has no
                    // row — exactly the dispatch held before `start`.
                    GuestAttachmentLease::Admitted if unheld && row.is_none() => "leased",
                    GuestAttachmentLease::Admitted
                        if unheld && row.is_some_and(ended_crash_class) =>
                    {
                        "ended"
                    }
                    GuestAttachmentLease::Admitted => {
                        return Err(DriverError::NoPreImage(format!(
                            "{alloc}'s Admitted lease is neither leased, running nor ended"
                        )));
                    }
                };
                if matches!(st, "ended" | "retiring")
                    && let Some(row) = row
                {
                    let dec = Self::decision(reader, a, &reading, row);
                    retry = dec;
                    ex = reading.run() && !dec;
                }
                (st, off)
            }
            None => {
                let cur = reading.current();
                let dec = cur.is_some_and(|row| Self::decision(reader, a, &reading, row));
                let st = if reading.desired.desired_to_stop {
                    "done"
                } else if reading.run()
                    && cur.is_some_and(|row| {
                        matches!(row.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
                    })
                    && !dec
                {
                    "failed"
                } else if reading.run()
                    && cur.is_some_and(|row| ended_crash_class(row) && row.terminal.is_none())
                    && !dec
                {
                    "ended"
                } else {
                    "wait"
                };
                if st == "ended" {
                    retry = dec;
                    ex = reading.run() && !dec;
                }
                (st, -1)
            }
        };
        Ok(AlView { st: st.to_owned(), off, retry, ex, pred, pst: pst.to_owned() })
    }

    /// `att(a)`: `attempt_index_for_alloc(target, x) − 1` (RESTART-SSOT).
    fn att_in(
        &self,
        reader: &Reader<'_>,
        leases: &Leases,
        a: i64,
    ) -> std::result::Result<i64, DriverError> {
        let reading = Self::reading_in(reader, a)?;
        let x = self
            .placement_lease_in(leases, a)
            .map(|(alloc, _, _)| alloc)
            .or_else(|| reading.current().map(|row| row.alloc_id.clone()));
        Ok(x.map_or(0, |alloc| {
            i64::from(reader.reconcilers.attempt_index_for_alloc(&target(a), &alloc)) - 1
        }))
    }

    /// `al` and `att` of every model workload, through `reader`.
    fn workloads_in(
        &self,
        reader: &Reader<'_>,
        leases: &Leases,
    ) -> std::result::Result<(BTreeMap<i64, AlView>, BTreeMap<i64, i64>), DriverError> {
        let mut al = BTreeMap::new();
        let mut att = BTreeMap::new();
        for a in ALLOCS {
            al.insert(a, self.al_view_in(reader, leases, a)?);
            att.insert(a, self.att_in(reader, leases, a)?);
        }
        Ok((al, att))
    }

    fn project(&self) -> Result<CidLeaseState> {
        let offsets = self.inst.offsets;
        if self.serve.is_some() {
            self.check_background("project")?;
        }
        // conformance-lint: sim-arbiter-ok QPROJ-7 SimVmm vsock_device_holders (the kernel's vhost-vsock holder per CID), Tier-3 pair V-30 Q2a/Q2c/Q2d (overdrive-host vsock_vmm_device) and V-RECLAIM
        let holders = self.node.vmm.vsock_device_holders();
        let up = self.serve.is_some();
        let leases = self.leases();
        let mut dev = (0..offsets).map(|o| (o, 0)).collect::<BTreeMap<_, _>>();
        for (cid, pid) in &holders {
            if self.inst.is_background(*cid) {
                continue;
            }
            let o = self
                .inst
                .offset_of(*cid)
                .ok_or_else(|| DriverError::NoPreImage(format!("held CID {}", cid.get())))?;
            let holder = match leases.iter().find(|(_, view)| view.cid == *cid) {
                // conformance-lint: sim-arbiter-ok QPROJ-7 the simulated host's VM scopes (which launch a holder VMM belongs to), Tier-3 pair V-30 Q2c / Q2d and V-RECLAIM
                Some((alloc, _)) if self.node.host.pids_of(alloc).contains(pid) => *self
                    .alloc_of
                    .get(alloc)
                    .ok_or_else(|| DriverError::NoPreImage(format!("allocation {alloc}")))?,
                Some((alloc, _)) => {
                    return Err(DriverError::NoPreImage(format!(
                        "VMM {pid} on {alloc}'s CID is not its launch"
                    ))
                    .into());
                }
                None => -2,
            };
            dev.insert(o, holder);
        }
        let (lease, cursor, al, att) = if up {
            let mut lease = (0..offsets).map(|o| (o, 0)).collect::<BTreeMap<_, _>>();
            // O QPROJ-2: the live pool, read in one snapshot.
            let observation = self.pool.lease_observation();
            for (alloc, view) in &observation.leases {
                if self.inst.is_background(view.cid) {
                    continue;
                }
                let o = self
                    .inst
                    .offset_of(view.cid)
                    .ok_or_else(|| DriverError::NoPreImage(format!("CID {}", view.cid.get())))?;
                let a = *self
                    .alloc_of
                    .get(alloc)
                    .ok_or_else(|| DriverError::NoPreImage(format!("allocation {alloc}")))?;
                lease.insert(o, a);
            }
            let cursor = self.inst.cursor_of(observation.next_fit);
            let (al, att) = self.workloads_in(&self.up_reader()?, &observation.leases)?;
            (WhileUp::Read(lease), WhileUp::Read(cursor), al, att)
        } else {
            // No `serve` process is up: no pool exists (NO-SURVIVE), so
            // `lease` and `cursor` are not compared (S-4: no literal is fed
            // into the projection); `al` / `att` come from the placement
            // owner's hydration over the node's durable stores, read
            // through a composition over the reopened files, built for the
            // read and dropped before the next `Restart` opens them.
            let reader = compose(&self.node, 0);
            let (al, att) = self.workloads_in(
                &Reader { rt: &reader.rt, state: &reader.state, reconcilers: &reader.reconcilers },
                &Leases::new(),
            )?;
            drop(reader);
            (WhileUp::NoPool, WhileUp::NoPool, al, att)
        };
        Ok(CidLeaseState { dev, lease, cursor, al, att })
    }
}

impl CidLeaseDriver {
    /// One step of the trace, on any instance (the action map is shared).
    fn step_on(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init()?,
            Assign(a: i64) => self.assign(a)?,
            AssignRefused(a: i64) => self.assign_refused(a)?,
            Launch(a: i64) => self.launch(a)?,
            Stop(a: i64) => self.stop(a)?,
            VmmCrash(a: i64) => self.vmm_crash(a)?,
            GuestReset(a: i64) => self.guest_reset(a)?,
            Finalize(a: i64) => self.finalize(a)?,
            Release(a: i64) => self.release(a)?,
            Redeploy(a: i64) => self.redeploy(a)?,
            Crash => self.crash(),
            Restart => self.restart()?,
            Reclaim => self.reclaim()?,
            OrphanExit(o: i64) => self.orphan_exit(o)?,
            AssignPick => return Err(DriverError::HazardOnly("AssignPick").into()),
            AssignTake => return Err(DriverError::HazardOnly("AssignTake").into()),
        })
    }
}

impl Driver for CidLeaseDriver {
    type State = CidLeaseState;

    fn config() -> Config {
        Config { state: &[], nondet: &["lease_conf_ok::cid_lease::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.step_on(step)
    }
}

/// The same driver on the main of the run pending GH #325 (item 9).
struct CidLease325Driver {
    inner: CidLeaseDriver,
}

impl Driver for CidLease325Driver {
    type State = CidLeaseState325;

    fn config() -> Config {
        Config { state: &[], nondet: &["lease_conf_325::cid_lease::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.inner.step_on(step)
    }
}

/// The same driver on the three-offset recording main `lease_conf_three`
/// (item 7's pool exhaustion; [`THREE`]).
struct CidLeaseThreeDriver {
    inner: CidLeaseDriver,
}

impl Driver for CidLeaseThreeDriver {
    type State = CidLeaseStateThree;

    fn config() -> Config {
        Config { state: &[], nondet: &["lease_conf_three::cid_lease::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.inner.step_on(step)
    }
}

// ----- the compared state ------------------------------------------------------

/// One workload's compared fields (`chg` is model-only).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlView {
    st: String,
    off: i64,
    retry: bool,
    ex: bool,
    pred: i64,
    pst: String,
}

/// A variable the live pool owns (`lease`, `cursor`): read from it while a
/// `serve` process is up; not compared while none is (S-4).
///
/// The spec side always decodes [`WhileUp::Read`]; the driver side is
/// [`WhileUp::NoPool`] from a `Crash` to the next boot's first step, and
/// `NoPool` compares equal to any value — the comparison is skipped, not
/// satisfied by a driver-chosen value (testing.md checklist item 5). The
/// next boot's first projected step compares the new pool against the
/// model again.
#[derive(Debug, Clone)]
enum WhileUp<T> {
    /// The live pool's value (O QPROJ-2), or the spec's.
    Read(T),
    /// No `serve` process is up: no pool exists to read (NO-SURVIVE).
    NoPool,
}

impl<T: PartialEq> PartialEq for WhileUp<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Read(a), Self::Read(b)) => a == b,
            (Self::NoPool, _) | (_, Self::NoPool) => true,
        }
    }
}

/// The compared `cid_lease` variables. `up` and `reclaimed` are stimulus
/// (the driver's own `Crash` / `Restart` / `Reclaim` set them; architect
/// 2026-10-19, checklist item 5), `bootLeft` stimulus, `g` ghost. `lease`
/// and `cursor` are not compared while no `serve` process is up
/// ([`WhileUp`], S-4).
#[derive(Debug, PartialEq)]
struct CidLeaseState {
    dev: BTreeMap<i64, i64>,
    lease: WhileUp<BTreeMap<i64, i64>>,
    cursor: WhileUp<i64>,
    al: BTreeMap<i64, AlView>,
    att: BTreeMap<i64, i64>,
}

impl CidLeaseState {
    /// Decodes the compared variables by their qualified names
    /// `<main>::cid_lease::<var>` (D-UP-1, spike QT1); a state of any other
    /// main fails with the missing qualified variable.
    fn from_value(value: &Value, main: &str) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(value, main, MODULE)?;
        Ok(Self {
            dev: quint_itf::int_map(vars.var("dev")?, quint_itf::int)?,
            lease: WhileUp::Read(quint_itf::int_map(vars.var("lease")?, quint_itf::int)?),
            cursor: WhileUp::Read(quint_itf::int(vars.var("cursor")?)?),
            al: quint_itf::int_map(vars.var("al")?, |entry| {
                Ok(AlView {
                    st: quint_itf::string(quint_itf::field(entry, "st")?)?,
                    off: quint_itf::int(quint_itf::field(entry, "off")?)?,
                    retry: quint_itf::boolean(quint_itf::field(entry, "retry")?)?,
                    ex: quint_itf::boolean(quint_itf::field(entry, "ex")?)?,
                    pred: quint_itf::int(quint_itf::field(entry, "pred")?)?,
                    pst: quint_itf::string(quint_itf::field(entry, "pst")?)?,
                })
            })?,
            att: quint_itf::int_map(vars.var("att")?, quint_itf::int)?,
        })
    }
}

impl<'de> Deserialize<'de> for CidLeaseState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value, MAIN).map_err(serde::de::Error::custom)
    }
}

impl State<CidLeaseDriver> for CidLeaseState {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value, MAIN)?)
    }

    fn from_driver(driver: &CidLeaseDriver) -> Result<Self> {
        driver.project()
    }
}

/// [`CidLeaseState`] decoded from `lease_conf_325`.
#[derive(Debug, PartialEq)]
struct CidLeaseState325(CidLeaseState);

impl<'de> Deserialize<'de> for CidLeaseState325 {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        CidLeaseState::from_value(&value, MAIN_325).map(Self).map_err(serde::de::Error::custom)
    }
}

impl State<CidLease325Driver> for CidLeaseState325 {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self(CidLeaseState::from_value(&value, MAIN_325)?))
    }

    fn from_driver(driver: &CidLease325Driver) -> Result<Self> {
        driver.inner.project().map(Self)
    }
}

/// [`CidLeaseState`] decoded from `lease_conf_three`.
#[derive(Debug, PartialEq)]
struct CidLeaseStateThree(CidLeaseState);

impl<'de> Deserialize<'de> for CidLeaseStateThree {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        CidLeaseState::from_value(&value, MAIN_THREE).map(Self).map_err(serde::de::Error::custom)
    }
}

impl State<CidLeaseThreeDriver> for CidLeaseStateThree {
    /// The raw ITF state, decoded by the shared ITF helpers (QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self(CidLeaseState::from_value(&value, MAIN_THREE)?))
    }

    fn from_driver(driver: &CidLeaseThreeDriver) -> Result<Self> {
        driver.inner.project().map(Self)
    }
}

// ----- named runs (critical scenarios, § "Named runs pinned by each driver") -----

/// Named run `AssignThenVmmOpensItsOwnDevice` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "AssignThenVmmOpensItsOwnDevice"
)]
fn quint_cid_lease_assign_then_vmm_opens_its_own_device() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `NextFitContinuesFromTheCursor` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "NextFitContinuesFromTheCursor"
)]
fn quint_cid_lease_next_fit_continues_from_the_cursor() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ReleaseWhileTheVmmRunsIsRefused` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ReleaseWhileTheVmmRunsIsRefused"
)]
fn quint_cid_lease_release_while_the_vmm_runs_is_refused() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `StopExitsTheVmmThenReleaseIsLast` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "StopExitsTheVmmThenReleaseIsLast"
)]
fn quint_cid_lease_stop_exits_the_vmm_then_release_is_last() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `GuestResetEndsTheVmmAndPlacementRetries` (`lease_conf_ok`):
/// the successor is leased while the predecessor's lease is held, and the
/// predecessor is released after the successor's launch.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "GuestResetEndsTheVmmAndPlacementRetries"
)]
fn quint_cid_lease_guest_reset_ends_the_vmm_and_placement_retries() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `VmmCrashFreesItsCidAndPlacementRetries` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "VmmCrashFreesItsCidAndPlacementRetries"
)]
fn quint_cid_lease_vmm_crash_frees_its_cid_and_placement_retries() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `StopOfAnEndedWorkloadReclaimsItsLease` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "StopOfAnEndedWorkloadReclaimsItsLease"
)]
fn quint_cid_lease_stop_of_an_ended_workload_reclaims_its_lease() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `StoppedWorkloadStaysStoppedAcrossServeCrash` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "StoppedWorkloadStaysStoppedAcrossServeCrash"
)]
fn quint_cid_lease_stopped_workload_stays_stopped_across_serve_crash() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `StoppedWorkloadIsPlacedAgainOnlyAfterRedeploy` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "StoppedWorkloadIsPlacedAgainOnlyAfterRedeploy"
)]
fn quint_cid_lease_stopped_workload_is_placed_again_only_after_redeploy() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `FifthCrashAtCeilingBoundaryIsPlacedAgain` (`lease_conf_ok`;
/// RESTART-SSOT, GH #324).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "FifthCrashAtCeilingBoundaryIsPlacedAgain"
)]
fn quint_cid_lease_fifth_crash_at_ceiling_boundary_is_placed_again() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `FailureAtCeilingExhaustsTheBudget` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "FailureAtCeilingExhaustsTheBudget"
)]
fn quint_cid_lease_failure_at_ceiling_exhausts_the_budget() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `RedeployPlacesAnExhaustedWorkloadAgainWithItsCount` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "RedeployPlacesAnExhaustedWorkloadAgainWithItsCount"
)]
fn quint_cid_lease_redeploy_places_an_exhausted_workload_again_with_its_count() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ServeCrashPlacesAWorkloadAtTheCeilingAgain` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ServeCrashPlacesAWorkloadAtTheCeilingAgain"
)]
fn quint_cid_lease_serve_crash_places_a_workload_at_the_ceiling_again() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ReclaimedWorkloadPastTheCeilingIsPlacedAgain` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ReclaimedWorkloadPastTheCeilingIsPlacedAgain"
)]
fn quint_cid_lease_reclaimed_workload_past_the_ceiling_is_placed_again() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ServeCrashDuringFinalizeFinalizesAfterRestart` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ServeCrashDuringFinalizeFinalizesAfterRestart"
)]
fn quint_cid_lease_serve_crash_during_finalize_finalizes_after_restart() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ServeCrashBeforeLaunchPlacesTheSuccessorAgainWithItsCount` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ServeCrashBeforeLaunchPlacesTheSuccessorAgainWithItsCount"
)]
fn quint_cid_lease_serve_crash_before_launch_places_the_successor_again_with_its_count()
-> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ServeCrashOrphanIsReclaimedBeforeAnyAssign` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ServeCrashOrphanIsReclaimedBeforeAnyAssign"
)]
fn quint_cid_lease_serve_crash_orphan_is_reclaimed_before_any_assign() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ReclamationReleasesTheOrphanDeviceThenAssignProceeds` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ReclamationReleasesTheOrphanDeviceThenAssignProceeds"
)]
fn quint_cid_lease_reclamation_releases_the_orphan_device_then_assign_proceeds() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ReclamationKillsEveryOrphanBeforeAssign` (`lease_conf_ok`;
/// D-UP-19: one `vm_reclamation_boot::converge` call kills both orphans).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ReclamationKillsEveryOrphanBeforeAssign"
)]
fn quint_cid_lease_reclamation_kills_every_orphan_before_assign() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `OrphanExitsByItselfBeforeReclamation` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "OrphanExitsByItselfBeforeReclamation"
)]
fn quint_cid_lease_orphan_exits_by_itself_before_reclamation() -> impl Driver {
    CidLeaseDriver::new()
}
/// Named run `ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim` (`lease_conf_ok`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_ok",
    max_samples = 1,
    test = "ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim"
)]
fn quint_cid_lease_serve_crash_with_unlaunched_lease_leaves_nothing_to_reclaim() -> impl Driver {
    CidLeaseDriver::new()
}

/// Named run `RedeployLeasedAtServeCrashIsPlacedAgain` (`lease_conf_325`):
/// a generation replacement leased at a `serve` crash is placed again — the
/// behaviour GH #325 is to deliver; production loses it today (item 9).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_325",
    max_samples = 1,
    test = "RedeployLeasedAtServeCrashIsPlacedAgain"
)]
#[ignore = "pending #325"]
fn quint_cid_lease_redeploy_leased_at_serve_crash_is_placed_again() -> impl Driver {
    CidLease325Driver { inner: CidLeaseDriver::new() }
}

/// Named run `PoolExhaustedRestartCleansThePredecessorThenPlaces`
/// (`lease_conf_three`; item 7.4): a restart that finds no free offset is
/// refused by the pool (`PoolExhausted`, the production admission refusal),
/// so its dispatch cleans the predecessor at once; once that lease is
/// released the workload is placed with its charged count. The composed
/// path is the action shim's `RestartAllocation` dispatch (`finish_restart`
/// after the refusal) driven by `WorkloadLifecycle`'s tick over the
/// production pool, three model offsets on the `/29` ([`THREE`]).
///
/// The recording main `lease_conf_three` (D-18; `lease_ok`'s three offsets
/// with `conf_step` recorded, `cid_lease.qnt`, checked in the current
/// evidence record) is projected onto the `/29` by the background-lease
/// relabelling of [`Instance`]. Its `CEIL` is the model-checking 2, not
/// production's 5; the run's last step places workload 2 with `att = 1`, and
/// `restart_decision(att, ..) = att < CEIL` agrees for `att` ∈ {0, 1} under
/// both, so the ceiling does not enter the comparison; the main carries no
/// `#[quint_run]`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_conf_three",
    max_samples = 1,
    test = "PoolExhaustedRestartCleansThePredecessorThenPlaces"
)]
fn quint_cid_lease_pool_exhausted_restart_cleans_the_predecessor_then_places() -> impl Driver {
    CidLeaseThreeDriver { inner: CidLeaseDriver::over(THREE) }
}

// ----- randomized traces -----------------------------------------------------------

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt", main = "lease_conf_ok")]
fn quint_cid_lease_lease_conf_ok() -> impl Driver {
    CidLeaseDriver::new()
}
