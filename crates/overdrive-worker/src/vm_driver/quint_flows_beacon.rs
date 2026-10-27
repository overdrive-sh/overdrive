//! Quint conformance driver — `owner_flows`, conformance main
//! `flows_beacon_conf` (D17, ATTR-TOKEN, ATTR-LIVE-CLAIM, D-UP-9; QPROJ-1,
//! QPROJ-4, QPROJ-7; D-UP-1, D-UP-20, PD-6).
//!
//! Replays `specs/quint/guest-flow-owner/owner_flows.qnt`, main
//! `flows_beacon_conf` (configuration `beacon`, recording `conf_step`),
//! against the production owner of the beacon protocol: a `VmDriver` whose
//! node beacon intake is `BeaconIntake::bind` over a stepped `SimBeaconIo`
//! coupled to a `SimVmm`. The spec is referenced in place; the driver
//! re-implements no rule of the protocol.
//!
//! Step decoding (D-UP-1, spike QT1): `quint test` emits no `mbt::*`
//! variables, so the driver reads the action and its picks from the
//! write-only `conf_step` record, `Config::nondet =
//! &["flows_beacon_conf::owner_flows::conf_step"]` — one element, the
//! qualified ITF name. The state projection matches qualified names too
//! (`flows_beacon_conf::owner_flows::<var>`, [`itf_vars`]).
//!
//! Action → production call or environment stimulus (feature-delta
//! § *Conformance observation surface and action map*, `owner_flows`
//! `flows_beacon_conf`; D-UP-20: only these actions are enabled):
//!
//! | Action | Driver |
//! |---|---|
//! | `InitWith(o)` | a fresh node: one `SimVmm`, a stepped `SimBeaconIo`, and a `VmDriver` over them, its intake bound on a kernel-assigned port (PORTS-CONFIG; `o` selects a guest-side option this instance does not model) |
//! | `Provision` | stimulus: a new assignment of the CID — a `GuestTransportAssignment` whose token is `LaunchToken::generate(&SimEntropy)` |
//! | `VmStart` | `VmDriver::start` of a spec whose transport carries the assignment's token (`SimVmm::create` `Ok`) |
//! | `VmStop` | the VMM process exits (`SimVmm::exit_process`); the driver observes the exit and, in one per-CID step, ends the launch's liveness, closes its beacon sessions and releases its beacon claim (D-UP-9). K-A2′ on the beacon seam: the accepted, unread connections of that VM fail their read |
//! | `DriverStop` | `VmDriver::stop` |
//! | `GuestOpenBeacon(b)` | the seam queues a connection on the intake's bound beacon port with READY (the launch's token) buffered |
//! | `HostAcceptBeacon(b)` / `HostReadReady(b)` | the seam releases the accept / the READY read |
//!
//! Every other action of `owner_flows` (activation, teardown, release,
//! quiescence, the owner's session, flow and dial actions) is never enabled
//! in this configuration; `switch!` fails loudly on one.
//!
//! Projection (QPROJ-4, QPROJ-7): `drvLive` ← O
//! `VmDriver::beacon_observation().live`; `bclaim` ← O `.claims`;
//! `bcn(b).st`: `pending` ⇔ S `SimBeaconIo` `queued` (QPROJ-7 arbiter, marked
//! below; Tier-3 pair V-22 P-37 beacon part), `reading` ⇔ O `.unread`,
//! `live` ⇔ O `.claims`, `closed` ⇔ S `SimBeaconIo` `closed_by_driver`.
//! `vmGen`, `vmUp`, `alloc`, `gen`, `bcn(b).gen` are stimulus; `readyCross`,
//! `bHeldByOther` ghost; every flow, session and dial variable is constant
//! in this configuration and not compared; `conf_step` is model-only.
//! Connection `b` is the guest's beacon connection from source port
//! `49_152 + b` of the one CID the instance models (`GUEST_CID_BASE + 2`).
//!
//! Every critical named run of the `flows_beacon_conf` table (feature-delta
//! § *Formal protocol model*) is pinned with `#[quint_test(max_samples = 1)]`
//! (a named run is deterministic: one sample replays it); `#[quint_run]`
//! replays randomized traces (seed printed on failure). All are pending the
//! beacon relocation, attachment-cut step 6 (`BeaconIntake::bind` and
//! `VmDriver::beacon_observation` are RED scaffolds until then).
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through
//! quint-connect), `cargo xtask lima run --`.

#![allow(
    clippy::needless_pass_by_ref_mut,
    clippy::or_fun_call,
    clippy::expect_used,
    clippy::doc_markdown,
    reason = "a conformance driver asserts its own preconditions; CONTRACT_SHAPE lines are machine-read"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use itf::Value;
use overdrive_core::SpiffeId;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver as _, DriverError as StartError, DriverPayload,
    Resources, VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use super::{VmDriver, VmHostLayout};
use crate::beacon_intake::BeaconIntake;
use crate::probe_runner::ProbeRunner;

/// The conformance main this driver replays (D-UP-1, PD-6).
const MAIN: &str = "flows_beacon_conf";
/// The module that declares the compared variables.
const MODULE: &str = "owner_flows";
/// The guest-side source port of beacon connection `b` is `BASE_PORT + b`.
const BASE_PORT: u32 = 49_152;
/// The modelled beacon connections.
const BCONNS: [i64; 2] = [1, 2];
/// How long a step may take to settle (real milliseconds; the driver reads
/// time only through `SimClock`, which no step advances).
const SETTLE_BOUND: Duration = Duration::from_secs(10);

/// ITF decoding for the worker's conformance driver (test support): a trace
/// state is a record of variables named `<main>::<module>::<var>` (spike QT1,
/// caveat 1), Quint integers arrive as plain numbers or `#bigint`.
mod itf_vars {
    use itf::Value;

    /// Why a trace value does not have the shape the driver compares.
    #[derive(Debug, thiserror::Error)]
    pub(super) enum ItfError {
        #[error("the trace has no `{0}` variable")]
        MissingVar(String),
        #[error("the trace's record has no `{0}` field")]
        MissingField(&'static str),
        #[error("expected {expected} in the trace, found {found:?}")]
        Shape { expected: &'static str, found: Value },
    }

    fn shape(expected: &'static str, found: &Value) -> ItfError {
        ItfError::Shape { expected, found: found.clone() }
    }

    /// The variable `<main>::<module>::<name>` of trace state `state` —
    /// matched by its full qualified name, never by suffix.
    pub(super) fn var<'a>(
        state: &'a Value,
        main: &str,
        module: &str,
        name: &str,
    ) -> Result<&'a Value, ItfError> {
        let Value::Record(record) = state else {
            return Err(shape("a record of variables", state));
        };
        let qualified = format!("{main}::{module}::{name}");
        record.get(&qualified).ok_or(ItfError::MissingVar(qualified))
    }

    /// Field `name` of a Quint record.
    pub(super) fn field<'a>(value: &'a Value, name: &'static str) -> Result<&'a Value, ItfError> {
        let Value::Record(record) = value else { return Err(shape("a record", value)) };
        record.get(name).ok_or(ItfError::MissingField(name))
    }

    /// A Quint `int`.
    pub(super) fn int(value: &Value) -> Result<i64, ItfError> {
        match value {
            Value::Number(n) => Ok(*n),
            Value::BigInt(big) => {
                i64::try_from(big.get().clone()).map_err(|_| shape("a 64-bit int", value))
            }
            other => Err(shape("an int", other)),
        }
    }

    /// A Quint `bool`.
    pub(super) fn boolean(value: &Value) -> Result<bool, ItfError> {
        match value {
            Value::Bool(b) => Ok(*b),
            other => Err(shape("a bool", other)),
        }
    }

    /// A Quint `str`.
    pub(super) fn string(value: &Value) -> Result<String, ItfError> {
        match value {
            Value::String(s) => Ok(s.clone()),
            other => Err(shape("a string", other)),
        }
    }

    /// The entries of a Quint map.
    pub(super) fn map_entries(value: &Value) -> Result<Vec<(&Value, &Value)>, ItfError> {
        match value {
            Value::Map(map) => Ok(map.iter().collect()),
            other => Err(shape("a map", other)),
        }
    }
}

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("the step did not settle within the settle bound: {0}")]
    Unsettled(String),
    #[error("{0} has no pre-image in the model (the bijection is fixed by the driver)")]
    NoPreImage(String),
    #[error(transparent)]
    Itf(#[from] itf_vars::ItfError),
}

/// One assignment of the CID (stimulus): its allocation and launch token.
struct LeaseGen {
    alloc: AllocationId,
    launch: LaunchToken,
}

/// The current launch: its VMM and its pending or returned `start`.
struct Launch {
    alloc: AllocationId,
    /// The launch token its READY carries (`vmGen`'s).
    token: LaunchToken,
    pid: Option<u32>,
    start: JoinHandle<std::result::Result<AllocationHandle, StartError>>,
}

struct FlowsBeaconDriver {
    runtime: Runtime,
    tmp: TempDir,
    vmm: SimVmm,
    io: Arc<SimBeaconIo>,
    /// The production owner of the beacon protocol.
    driver: VmDriver,
    /// The port the driver's intake bound (`BeaconIntake::bound_port`,
    /// PORTS-CONFIG): the guest's READY names it (`port=`, GUEST-PORTS) —
    /// stimulus, never projected.
    beacon_port: VsockPort,
    entropy: SimEntropy,
    /// Assignments by generation (stimulus).
    leases: BTreeMap<u32, LeaseGen>,
    generation: u32,
    launch: Option<Launch>,
    /// The VMM pid each connection's guest dialed from (stimulus).
    conn_vm: BTreeMap<i64, u32>,
}

/// The CID every assignment of this node uses: the first usable offset of
/// the test prefix. A relabelling constant, computed where it is needed so
/// the driver's construction calls no step-2 newtype (the driver reaches
/// trace replay and goes RED on the production path).
fn node_cid() -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + 2).expect("an in-range guest CID")
}

/// The guest-flow owner's bound ports an assignment carries (GUEST-PORTS):
/// part of the `Provision` stimulus; no guest-flow owner runs in this
/// configuration, so fixture values in the valid range.
fn assignment_flow_ports() -> GuestFlowPorts {
    GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
        .expect("two distinct ports above the reserved range")
}

fn key(cid: GuestCid, b: i64) -> BeaconConnKey {
    let b = u32::try_from(b).expect("connection ids are positive");
    BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(BASE_PORT + b) }
}

fn stage_artifacts(root: &std::path::Path) {
    std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
        .expect("write synthetic master rootfs");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
}

/// A `VmDriver` over `vmm` whose intake is bound over `io`, and the port the
/// intake bound.
async fn vm_driver(
    root: &std::path::Path,
    vmm: &SimVmm,
    io: &Arc<SimBeaconIo>,
) -> (VmDriver, VsockPort) {
    let clock = SimClock::new();
    let wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
    assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
    // PORTS-CONFIG: kernel-assigned (the sim chooses a port above 1023); the
    // guest's beacon connections are keyed by its CID and source port only.
    let intake =
        BeaconIntake::bind(Arc::clone(io) as Arc<dyn BeaconIo>, VsockListenPort::KernelAssigned)
            .await
            .expect("the sim beacon listener binds");
    let beacon_port = intake.bound_port();
    let driver = VmDriver::new(
        Arc::new(vmm.clone()),
        Arc::new(clock.clone()),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(clock),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("flows-beacon").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
        )),
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
    (driver, beacon_port)
}

impl FlowsBeaconDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let tmp = TempDir::new().expect("tmpdir");
        stage_artifacts(tmp.path());
        let vmm = SimVmm::new();
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let (driver, beacon_port) = runtime.block_on(vm_driver(tmp.path(), &vmm, &io));
        Self {
            runtime,
            tmp,
            vmm,
            io,
            driver,
            beacon_port,
            entropy: SimEntropy::new(0x295),
            leases: BTreeMap::new(),
            generation: 0,
            launch: None,
            conn_vm: BTreeMap::new(),
        }
    }

    /// Drives the runtime until every beacon call is parked and the driver's
    /// beacon observation is stable.
    fn settle(&self, what: &str) -> std::result::Result<(), DriverError> {
        let deadline = std::time::Instant::now() + SETTLE_BOUND;
        let mut stable = 0;
        let mut last = None;
        while stable < 3 {
            if std::time::Instant::now() > deadline {
                return Err(DriverError::Unsettled(what.to_owned()));
            }
            self.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
            let now = (self.io.parked(), self.driver.beacon_observation());
            stable = if now.0 && last.as_ref() == Some(&now.1) { stable + 1 } else { 0 };
            last = Some(now.1);
        }
        Ok(())
    }

    fn vm_pid(&self, action: &'static str) -> std::result::Result<u32, DriverError> {
        self.launch
            .as_ref()
            .and_then(|launch| launch.pid)
            .ok_or(DriverError::NotEnabled { action, why: "no VM was started".to_owned() })
    }

    // ----- actions --------------------------------------------------------------

    /// `InitWith(o)`: a fresh node; `o` selects a guest-side option this
    /// configuration does not model.
    fn init_with(&mut self, _option: &str) -> std::result::Result<(), DriverError> {
        if let Some(launch) = self.launch.take() {
            launch.start.abort();
        }
        *self = Self::new();
        self.settle("a fresh intake parks on accept")
    }

    /// `Provision` (stimulus): a new assignment, and its launch token, for
    /// the next `VmStart`.
    fn provision(&mut self) -> std::result::Result<(), DriverError> {
        self.generation += 1;
        let alloc = AllocationId::new(&format!("beacon-gen-{}", self.generation))
            .expect("valid allocation id");
        let launch = LaunchToken::generate(&self.entropy);
        self.leases.insert(self.generation, LeaseGen { alloc, launch });
        self.settle("the assignment changes no beacon state")
    }

    fn vm_start(&mut self) -> std::result::Result<(), DriverError> {
        let lease = self.leases.get(&self.generation).ok_or(DriverError::NotEnabled {
            action: "VmStart",
            why: "no assignment is provisioned".to_owned(),
        })?;
        let spec = AllocationSpec {
            alloc: lease.alloc.clone(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/flows-beacon/alloc/x")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: self.tmp.path().join("vmlinuz"),
                rootfs: self.tmp.path().join("master.img"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: Some(GuestTransportAssignment::new(
                Ipv4Addr::new(100, 95, 0, 2),
                Ipv4Addr::new(100, 95, 0, 1),
                node_cid(),
                lease.launch,
                assignment_flow_ports(),
            )),
            service_ports: Vec::new(),
        };
        let alloc = lease.alloc.clone();
        let token = lease.launch;
        let driver = self.driver.clone();
        let start = self.runtime.spawn(async move { driver.start(&spec).await });
        self.launch = Some(Launch { alloc, token, pid: None, start });
        let deadline = std::time::Instant::now() + SETTLE_BOUND;
        while !self.vmm.vsock_device_holders().contains_key(&node_cid()) {
            if std::time::Instant::now() > deadline
                || self.launch.as_ref().is_some_and(|launch| launch.start.is_finished())
            {
                return Err(DriverError::Unsettled("the VMM opens its device".to_owned()));
            }
            self.runtime.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        }
        let pid = self.vmm.vsock_device_holders().get(&node_cid()).copied();
        if let Some(launch) = self.launch.as_mut() {
            launch.pid = pid;
        }
        self.settle("the launch is live")
    }

    /// `VmStop`: the VMM exits. The production driver observes the exit
    /// through its exit watcher and takes the D-UP-9 step; the seam's K-A2′
    /// fact (an accepted, unread connection of the dead VM fails its next
    /// read) is delivered by releasing that read.
    fn vm_stop(&mut self) -> std::result::Result<(), DriverError> {
        let pid = self.vm_pid("VmStop")?;
        // `false` only when `VmDriver::stop` already terminated this VMM (the
        // model's `DriverStop` then `VmStop` order): the exit has already
        // happened and there is nothing left to exit.
        let exited_now = self.vmm.exit_process(pid, Some(0), None);
        tracing::debug!(pid, exited_now, "flows_beacon_conf: VmStop");
        self.io.observe_vm_exits();
        let unread = self.driver.beacon_observation().unread;
        for (b, vm) in &self.conn_vm {
            let conn = key(node_cid(), *b);
            if *vm == pid && unread.contains(&conn) && !self.io.release_read(conn) {
                return Err(DriverError::NotEnabled {
                    action: "VmStop",
                    why: format!("unread connection {b} has no read to fail"),
                });
            }
        }
        self.settle("the VM's exit is observed and its beacon state released")
    }

    fn driver_stop(&mut self) -> std::result::Result<(), DriverError> {
        let launch = self
            .launch
            .as_ref()
            .ok_or(DriverError::NotEnabled { action: "DriverStop", why: "no launch".to_owned() })?;
        let handle = AllocationHandle { alloc: launch.alloc.clone(), pid: launch.pid };
        let driver = self.driver.clone();
        self.runtime.block_on(driver.stop(&handle)).map_err(|error| DriverError::NotEnabled {
            action: "DriverStop",
            why: format!("VmDriver::stop refused: {error}"),
        })?;
        self.io.observe_vm_exits();
        self.settle("the launch's beacon session is closed")
    }

    fn guest_open_beacon(&mut self, b: i64) -> std::result::Result<(), DriverError> {
        let pid = self.vm_pid("GuestOpenBeacon")?;
        let launch =
            self.launch.as_ref().map(|launch| launch.token).ok_or(DriverError::NotEnabled {
                action: "GuestOpenBeacon",
                why: "no VM was started".to_owned(),
            })?;
        // GUEST-PORTS: READY's `port=` is the beacon port the guest's
        // command line named, the one the intake bound.
        let ready = format!("READY pid=1 port={} launch={launch}", self.beacon_port);
        self.io.queue_connection(key(node_cid(), b), pid, &[ready.as_str()]).map_err(|error| {
            DriverError::NotEnabled { action: "GuestOpenBeacon", why: error.to_string() }
        })?;
        self.conn_vm.insert(b, pid);
        self.settle("the queued connection waits for accept")
    }

    fn host_accept_beacon(&mut self, _b: i64) -> std::result::Result<(), DriverError> {
        self.io.release_accept();
        self.settle("the accept is delivered")
    }

    fn host_read_ready(&mut self, b: i64) -> std::result::Result<(), DriverError> {
        if !self.io.release_read(key(node_cid(), b)) {
            return Err(DriverError::NotEnabled {
                action: "HostReadReady",
                why: format!("connection {b} is not accepted and open"),
            });
        }
        self.settle("the READY read is delivered")
    }
}

impl Driver for FlowsBeaconDriver {
    type State = FlowsBeaconState;

    fn config() -> Config {
        Config { state: &[], nondet: &["flows_beacon_conf::owner_flows::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            InitWith(o: String) => self.init_with(&o)?,
            Provision => self.provision()?,
            VmStart => self.vm_start()?,
            VmStop => self.vm_stop()?,
            DriverStop => self.driver_stop()?,
            GuestOpenBeacon(b: i64) => self.guest_open_beacon(b)?,
            HostAcceptBeacon(b: i64) => self.host_accept_beacon(b)?,
            HostReadReady(b: i64) => self.host_read_ready(b)?,
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// The compared `flows_beacon_conf` variables.
#[derive(Debug, PartialEq, Eq)]
struct FlowsBeaconState {
    drv_live: bool,
    bclaim: i64,
    bcn: BTreeMap<i64, String>,
}

impl FlowsBeaconState {
    /// The spec side: the qualified variables `flows_beacon_conf::owner_flows::*`.
    fn from_value(state: &Value) -> std::result::Result<Self, itf_vars::ItfError> {
        let var = |name: &str| itf_vars::var(state, MAIN, MODULE, name);
        let bcn = itf_vars::map_entries(var("bcn")?)?
            .into_iter()
            .map(|(b, conn)| {
                Ok((itf_vars::int(b)?, itf_vars::string(itf_vars::field(conn, "st")?)?))
            })
            .collect::<std::result::Result<_, itf_vars::ItfError>>()?;
        Ok(Self {
            drv_live: itf_vars::boolean(var("drvLive")?)?,
            bclaim: itf_vars::int(var("bclaim")?)?,
            bcn,
        })
    }
}

impl<'de> Deserialize<'de> for FlowsBeaconState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<FlowsBeaconDriver> for FlowsBeaconState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &FlowsBeaconDriver) -> Result<Self> {
        // The pinned owner accessor (QPROJ-4).
        let observation = driver.driver.beacon_observation();
        // conformance-lint: sim-arbiter-ok QPROJ-7 SimBeaconIo beacon accept queue (queued) and closed_by_driver, Tier-3 pair V-22 (P-37 beacon part)
        let sockets = driver.io.beacon_socket_observation();
        let index_of = |conn: &BeaconConnKey| -> std::result::Result<i64, DriverError> {
            BCONNS
                .into_iter()
                .find(|b| key(node_cid(), *b) == *conn)
                .ok_or_else(|| DriverError::NoPreImage(format!("beacon connection {conn:?}")))
        };
        let bclaim = observation.claims.get(&node_cid()).map(index_of).transpose()?.unwrap_or(0);
        let mut bcn = BTreeMap::new();
        for b in BCONNS {
            let conn = key(node_cid(), b);
            let st = if sockets.queued.contains_key(&conn) {
                "pending"
            } else if observation.unread.contains(&conn) {
                "reading"
            } else if observation.claims.values().any(|held| *held == conn) {
                "live"
            } else if sockets.closed_by_driver.contains(&conn) {
                "closed"
            } else {
                "none"
            };
            bcn.insert(b, st.to_owned());
        }
        Ok(Self { drv_live: observation.live.contains_key(&node_cid()), bclaim, bcn })
    }
}

// ----- named runs (critical scenarios) -------------------------------------------

/// Named run `BeaconReadyReleasedThenDriverStopReleasesClaim`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "BeaconReadyReleasedThenDriverStopReleasesClaim",
    max_samples = 1
)]
fn quint_flows_beacon_ready_released_then_driver_stop_releases_claim() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `BeaconAfterDriverStopIsRefusedAtAccept` (NoLiveLaunch).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "BeaconAfterDriverStopIsRefusedAtAccept",
    max_samples = 1
)]
fn quint_flows_beacon_after_driver_stop_is_refused_at_accept() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `DriverStopBetweenAcceptAndReadyClosesWithoutClaim`
/// (ATTR-LIVE-CLAIM).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "DriverStopBetweenAcceptAndReadyClosesWithoutClaim",
    max_samples = 1
)]
fn quint_flows_beacon_driver_stop_between_accept_and_ready_closes_without_claim() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `StaleLaunchBeaconIsClosedAtReadyAndDoesNotPoisonClaim`
/// (K-A2′, ATTR-TOKEN).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "StaleLaunchBeaconIsClosedAtReadyAndDoesNotPoisonClaim",
    max_samples = 1
)]
fn quint_flows_beacon_stale_launch_is_closed_at_ready_and_does_not_poison_claim() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `SecondBeaconConnectionOfSameLaunchIsHeld` (SessionHeld).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "SecondBeaconConnectionOfSameLaunchIsHeld",
    max_samples = 1
)]
fn quint_flows_beacon_second_connection_of_same_launch_is_held() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `VmExitEndsLaunchAndReleasesBeaconClaim` (D-UP-9: the VMM exit
/// ends the launch and releases the claim in one step; the next launch's
/// READY takes it).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "VmExitEndsLaunchAndReleasesBeaconClaim",
    max_samples = 1
)]
fn quint_flows_beacon_vm_exit_ends_launch_and_releases_claim() -> impl Driver {
    FlowsBeaconDriver::new()
}

/// Named run `VmDeathClosesUnreadBeaconConnection` (K-A2′).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf",
    test = "VmDeathClosesUnreadBeaconConnection",
    max_samples = 1
)]
fn quint_flows_beacon_vm_death_closes_unread_connection() -> impl Driver {
    FlowsBeaconDriver::new()
}

// ----- randomized traces -----------------------------------------------------------

/// Randomized `flows_beacon_conf` traces (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_beacon_conf"
)]
fn quint_flows_beacon_randomized() -> impl Driver {
    FlowsBeaconDriver::new()
}
