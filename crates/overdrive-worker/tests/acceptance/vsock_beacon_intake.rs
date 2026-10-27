//! D17 / DG-4 / QPROJ-4 / P2-S14 — the node beacon intake and READY
//! attribution by launch token (GH #295, vsock attachment replacement;
//! ATTR-TOKEN, ATTR-LIVE-CLAIM).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § *Beacon
//! intake — `overdrive-worker::VmDriver`* (D17), DG-4 (`BeaconIntake::bind`,
//! `BeaconIntakeError`, the core `BeaconIo` port) and QPROJ-4
//! (`VmDriver::beacon_observation`, `BeaconObservation`). The protocol's
//! model is `specs/quint/guest-flow-owner/owner_flows.qnt`, conformance main
//! `flows_beacon_conf` (driver:
//! `overdrive-worker/src/vm_driver/quint_flows_beacon.rs`); these bodies are
//! its example cells.
//!
//! Driving ports: `BeaconIntake::bind`, `VmDriver::new(.., beacon)`,
//! `VmDriver::start` / `stop`, `VmDriver::beacon_observation`. Driven port:
//! `BeaconIo`, supplied by `SimBeaconIo` (the QPROJ-7 arbiter of the beacon
//! accept queue and of the connections the driver closed), coupled to a
//! `SimVmm`. A stepped `SimBeaconIo` holds every accept and READY read until
//! the body releases it, so "accepted before `stop`, read after" is a real
//! interleaving, not a race.
//!
//! The attribution cells: READY of the live launch takes the CID's beacon
//! claim; a stale queued READY (P-37) is closed and does not poison the claim;
//! a connection read after `stop` takes nothing; a second connection of the
//! same CID is closed `SessionHeld`; a connection that closes with nothing
//! buffered (EOF) and one whose first line is not READY are closed without
//! effect; a connection from a CID with no live launch is closed at accept;
//! a VMM exit — a guest reset included (RESET-EXIT) — ends the launch,
//! closes its beacon session and releases its claim in one step, so the next
//! launch's READY takes the claim (D-UP-9).

#![allow(
    clippy::missing_const_for_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test fixtures assert their preconditions; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverError, DriverPayload, Resources, VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, BeaconIntakeError, BeaconObservation, VmDriver};
use tempfile::TempDir;
use tokio::task::JoinHandle;

fn sim_beacon(vmm: &SimVmm) -> Arc<SimBeaconIo> {
    Arc::new(SimBeaconIo::new(vmm.clone()))
}

fn layout(root: &Path) -> VmHostLayout {
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
    }
}

/// A `VmDriver` over sim ports whose beacon intake is bound over `io`
/// (DG-2 row 15: `VmDriver::new(.., beacon)`).
async fn driver_over(vmm: &SimVmm, io: &Arc<SimBeaconIo>, root: &Path) -> VmDriver {
    driver_over_vmm(Arc::new(vmm.clone()), io, root).await.0
}

/// As [`driver_over`], over any `Vmm` (a recording decorator of the sim),
/// returning the intake's bound beacon port too (PORTS-CONFIG: the intake
/// binds kernel-assigned and reports the port it got).
async fn driver_over_vmm(
    vmm: Arc<dyn Vmm>,
    io: &Arc<SimBeaconIo>,
    root: &Path,
) -> (VmDriver, VsockPort) {
    let beacon =
        BeaconIntake::bind(Arc::clone(io) as Arc<dyn BeaconIo>, VsockListenPort::KernelAssigned)
            .await
            .expect("the sim beacon listener binds");
    let beacon_port = beacon.bound_port();
    let clock = SimClock::new();
    let wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
    assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
    let probe_runner = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimClock::new()),
        Arc::new(SimObservationStore::single_peer(
            NodeId::new("vsock-beacon-intake").expect("valid node ID"),
            0,
        )) as Arc<dyn ObservationStore>,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    let driver = VmDriver::new(
        vmm,
        Arc::new(clock),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        probe_runner,
        wiring.gate(),
        layout(root),
        beacon,
    );
    (driver, beacon_port)
}

/// The guest-flow owner ports every fixture assignment carries (GUEST-PORTS):
/// deliberately not the 1240 / 1243 defaults, so a driver that wrote a
/// constant instead of the assignment's ports is caught.
const FLOW_STREAM: u32 = 50_240;
const FLOW_CONTROL: u32 = 50_243;

fn flow_ports() -> GuestFlowPorts {
    GuestFlowPorts::new(VsockPort::new(FLOW_STREAM), VsockPort::new(FLOW_CONTROL))
        .expect("two distinct ports above the reserved range")
}

/// Test-local `Vmm` decorator over the sim: records the kernel command line
/// of every `create` (the `VmConfig` the driver hands the VMM is the launch's
/// whole guest-visible configuration).
struct RecordsCmdline {
    inner: SimVmm,
    cmdlines: Arc<parking_lot::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl Vmm for RecordsCmdline {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        self.cmdlines.lock().push(config.cmdline.as_str().to_owned());
        self.inner.create(config, cid).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// A `VmDriver` over sim ports that starts nothing; the layout's paths are
/// never touched.
async fn fresh_driver(vmm: SimVmm) -> VmDriver {
    let io = sim_beacon(&vmm);
    driver_over(&vmm, &io, Path::new("/nonexistent/vsock-beacon-intake")).await
}

// ---------------------------------------------------------------------------
// The attribution fixture (stepped intake, one CID)
// ---------------------------------------------------------------------------

const GUEST_SOURCE_PORT: u32 = 49_152;

fn cid() -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + 2).expect("an in-range guest CID")
}

fn token(byte: u8) -> LaunchToken {
    LaunchToken::from_bytes([byte; 16]).expect("a non-zero launch token")
}

/// The kernel key of the guest's `n`-th beacon connection from `cid`.
fn conn(cid: GuestCid, n: u32) -> BeaconConnKey {
    BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(GUEST_SOURCE_PORT + n) }
}

fn ready(launch: LaunchToken) -> String {
    format!("READY pid=1 port=1234 launch={launch}")
}

/// One driver, its stepped intake, and the staged VM artifacts.
struct Node {
    _tmp: TempDir,
    root: std::path::PathBuf,
    vmm: SimVmm,
    io: Arc<SimBeaconIo>,
    driver: VmDriver,
}

impl Node {
    async fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
        let vmm = SimVmm::new();
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let driver = driver_over(&vmm, &io, &root).await;
        Self { _tmp: tmp, root, vmm, io, driver }
    }

    fn spec(&self, alloc: &str, launch: LaunchToken) -> AllocationSpec {
        AllocationSpec {
            alloc: AllocationId::new(alloc).expect("valid alloc id"),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/beacon/alloc/x")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: self.root.join("vmlinuz"),
                rootfs: self.root.join("master.img"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: Some(GuestTransportAssignment::new(
                Ipv4Addr::new(100, 95, 0, 2),
                Ipv4Addr::new(100, 95, 0, 1),
                cid(),
                launch,
                flow_ports(),
            )),
            service_ports: Vec::new(),
        }
    }

    /// Spawns `start` and returns once its VMM holds the CID's device, with
    /// the VMM's pid.
    async fn launch(
        &self,
        spec: &AllocationSpec,
    ) -> (u32, JoinHandle<Result<AllocationHandle, DriverError>>) {
        let driver = self.driver.clone();
        let owned = spec.clone();
        let start = tokio::spawn(async move { driver.start(&owned).await });
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pid) = self.vmm.vsock_device_holders().get(&cid()) {
                    break *pid;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the VMM opens its device within 10s");
        (pid, start)
    }

    /// Lets every task run until every beacon call is parked.
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

    /// Releases one accept and one READY read of `key`, then settles.
    async fn accept_and_read(&self, key: BeaconConnKey) {
        self.io.release_accept();
        self.settle().await;
        assert!(self.io.release_read(key), "the connection was accepted and is open");
        self.settle().await;
    }
}

// ---------------------------------------------------------------------------
// DG-4 — binding the intake
// ---------------------------------------------------------------------------

/// DG-4: binding the intake binds the listener through the port, once.
///
/// Live (default lane): `BeaconIntake::bind` is complete — it binds through
/// its `BeaconIo` port and maps a failure to `BeaconIntakeError::Bind` — so
/// this cell and the next defend shipped behaviour against `SimBeaconIo`.
/// The attachment-cut step (6) wires the intake into `compose_vm_driver`
/// over `VsockBeaconIo`; that wiring is the `vm_driver.beacon_bind` boot
/// refusal cell's subject, not these.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn binding_the_intake_binds_the_beacon_listener() {
    let vmm = SimVmm::new();
    let io = sim_beacon(&vmm);
    assert!(!io.is_bound(), "precondition: nothing bound");

    let Ok(intake) =
        BeaconIntake::bind(Arc::clone(&io) as Arc<dyn BeaconIo>, VsockListenPort::KernelAssigned)
            .await
    else {
        panic!("a healthy port binds");
    };
    assert!(io.is_bound(), "the intake bound the listener");
    let port = intake.bound_port().as_u32();
    assert!(
        port > 1_023 && port != u32::MAX,
        "a kernel-assigned bind reports a port above the reserved range, got {port}"
    );
    assert!(io.beacon_socket_observation().queued.is_empty(), "binding accepts nothing on its own");
}

/// DG-4: a bind failure is `BeaconIntakeError::Bind` naming the configured
/// `VsockListenPort` and the port's error; nothing is bound. Live, as above.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn a_bind_failure_is_typed_and_binds_nothing() {
    let vmm = SimVmm::new();
    let io = sim_beacon(&vmm);
    io.fail_next_bind(std::io::ErrorKind::AddrInUse);

    let configured = VsockListenPort::Fixed(BEACON_VSOCK_PORT);
    let Err(err) = BeaconIntake::bind(Arc::clone(&io) as Arc<dyn BeaconIo>, configured).await
    else {
        panic!("the scripted bind failure must surface");
    };
    let BeaconIntakeError::Bind { port, source } = &err;
    assert_eq!(*port, configured, "the error names the configured port");
    assert_eq!(source.kind(), std::io::ErrorKind::AddrInUse);
    assert!(!io.is_bound(), "nothing bound");
    assert!(
        err.to_string().starts_with(
            "binding the beacon vsock listener on port Fixed(VsockPort(1234)) failed: "
        ),
        "{err}"
    );
}

/// The typed refusal renders its pinned message (the `health.startup.refused`
/// reason `vm_driver.beacon_bind` carries it).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_bind_refusal_renders_its_pinned_message() {
    let err = BeaconIntakeError::Bind {
        port: VsockListenPort::Fixed(BEACON_VSOCK_PORT),
        source: std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use"),
    };
    assert_eq!(
        err.to_string(),
        "binding the beacon vsock listener on port Fixed(VsockPort(1234)) failed: in use"
    );
    let assigned = BeaconIntakeError::Bind {
        port: VsockListenPort::KernelAssigned,
        source: std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no free port"),
    };
    assert_eq!(
        assigned.to_string(),
        "binding the beacon vsock listener on port KernelAssigned failed: no free port"
    );
}

/// QPROJ-4: a driver that has started nothing reports three empty
/// collections, and a repeated call returns an equal value.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn a_fresh_driver_reports_an_empty_beacon_observation() {
    let driver = fresh_driver(SimVmm::new()).await;
    let first = driver.beacon_observation();
    assert_eq!(
        first,
        BeaconObservation {
            live: BTreeMap::new(),
            claims: BTreeMap::new(),
            unread: BTreeSet::new()
        }
    );
    assert_eq!(driver.beacon_observation(), first, "a repeated call changes nothing");
}

// ---------------------------------------------------------------------------
// P2-S14 — READY attribution (ATTR-TOKEN, ATTR-LIVE-CLAIM)
// ---------------------------------------------------------------------------

/// The live launch's READY takes the CID's beacon claim and completes the
/// start: the launch is live with its token, the connection holds the claim,
/// nothing is unread, and the driver closed nothing.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_ready_of_the_live_launch_takes_the_claim_and_completes_the_start() {
    let node = Node::new().await;
    let launch = token(0x5a);
    let (pid, start) = node.launch(&node.spec("beacon-live", launch)).await;
    let before = node.driver.beacon_observation();
    assert_eq!(before.live, BTreeMap::from([(cid(), launch)]), "the launch is live from start");
    assert!(before.claims.is_empty());

    node.io.queue_connection(conn(cid(), 1), pid, &[ready(launch).as_str()]).expect("queued");
    node.settle().await;
    node.io.release_accept();
    node.settle().await;
    assert_eq!(
        node.driver.beacon_observation().unread,
        BTreeSet::from([conn(cid(), 1)]),
        "accepted and held unclaimed until its first line is read"
    );
    assert!(node.io.release_read(conn(cid(), 1)));
    node.settle().await;

    let after = node.driver.beacon_observation();
    assert_eq!(after.claims, BTreeMap::from([(cid(), conn(cid(), 1))]), "READY took the claim");
    assert!(after.unread.is_empty());
    assert_eq!(after.live, before.live, "the launch stays live");
    assert!(node.io.beacon_socket_observation().closed_by_driver.is_empty(), "nothing closed");
    let handle = tokio::time::timeout(Duration::from_secs(10), start)
        .await
        .expect("the start completes")
        .expect("the start task did not panic")
        .expect("READY completes the start");
    assert_eq!(handle.pid, Some(pid));
}

/// P-37: a READY of a dead launch, still queued when the next launch of the
/// same CID is live, is closed at its read (`StaleLaunch`) and does not
/// poison the claim — the live launch's own READY then takes it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_stale_queued_ready_is_closed_and_does_not_poison_the_claim() {
    let node = Node::new().await;
    let stale = token(0x11);
    let (dead_pid, first) = node.launch(&node.spec("beacon-first", stale)).await;
    node.io.queue_connection(conn(cid(), 1), dead_pid, &[ready(stale).as_str()]).expect("queued");
    node.driver
        .stop(&AllocationHandle {
            alloc: AllocationId::new("beacon-first").expect("valid alloc id"),
            pid: Some(dead_pid),
        })
        .await
        .expect("stop the first launch");
    node.io.observe_vm_exits();
    let _ = first.await;
    assert!(
        node.io.beacon_socket_observation().queued.contains_key(&conn(cid(), 1)),
        "precondition: the dead launch's READY stays queued (P-37)"
    );

    let live = token(0x22);
    let (pid, _start) = node.launch(&node.spec("beacon-second", live)).await;
    node.accept_and_read(conn(cid(), 1)).await;
    let observation = node.driver.beacon_observation();
    assert!(observation.claims.is_empty(), "the stale READY took no claim");
    assert!(
        node.io.beacon_socket_observation().closed_by_driver.contains(&conn(cid(), 1)),
        "the stale connection was closed"
    );

    node.io.queue_connection(conn(cid(), 2), pid, &[ready(live).as_str()]).expect("queued");
    node.settle().await;
    node.accept_and_read(conn(cid(), 2)).await;
    assert_eq!(
        node.driver.beacon_observation().claims,
        BTreeMap::from([(cid(), conn(cid(), 2))]),
        "the live launch's READY is not refused SessionHeld"
    );
}

/// ATTR-LIVE-CLAIM: a connection accepted before `stop` and read after it
/// takes nothing — `stop` ended the launch's liveness — and is closed.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_ready_read_after_stop_takes_nothing() {
    let node = Node::new().await;
    let launch = token(0x33);
    let (pid, start) = node.launch(&node.spec("beacon-stopped", launch)).await;
    node.io.queue_connection(conn(cid(), 1), pid, &[ready(launch).as_str()]).expect("queued");
    node.settle().await;
    node.io.release_accept();
    node.settle().await;
    assert!(node.driver.beacon_observation().unread.contains(&conn(cid(), 1)), "accepted, unread");

    node.driver
        .stop(&AllocationHandle {
            alloc: AllocationId::new("beacon-stopped").expect("valid alloc id"),
            pid: Some(pid),
        })
        .await
        .expect("stop the launch");
    assert!(node.driver.beacon_observation().live.is_empty(), "stop ended the launch's liveness");

    assert!(node.io.release_read(conn(cid(), 1)));
    node.settle().await;
    let after = node.driver.beacon_observation();
    assert!(after.claims.is_empty(), "a read after stop takes nothing");
    assert!(after.unread.is_empty());
    assert!(node.io.beacon_socket_observation().closed_by_driver.contains(&conn(cid(), 1)));
    let outcome = tokio::time::timeout(Duration::from_secs(10), start)
        .await
        .expect("the start ends")
        .expect("the start task did not panic");
    assert!(outcome.is_err(), "a stopped launch never completes its start: {outcome:?}");
}

/// A second connection of the same CID while its claim is held is closed
/// (`SessionHeld`); the claim stays with the first.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_second_connection_of_the_same_launch_is_closed_session_held() {
    let node = Node::new().await;
    let launch = token(0x44);
    let (pid, _start) = node.launch(&node.spec("beacon-held", launch)).await;
    node.io.queue_connection(conn(cid(), 1), pid, &[ready(launch).as_str()]).expect("queued");
    node.io.queue_connection(conn(cid(), 2), pid, &[ready(launch).as_str()]).expect("queued");
    node.settle().await;
    node.accept_and_read(conn(cid(), 1)).await;
    node.accept_and_read(conn(cid(), 2)).await;

    assert_eq!(
        node.driver.beacon_observation().claims,
        BTreeMap::from([(cid(), conn(cid(), 1))]),
        "the claim stays with the first READY"
    );
    let closed = node.io.beacon_socket_observation().closed_by_driver;
    assert!(closed.contains(&conn(cid(), 2)), "the second connection is closed");
    assert!(!closed.contains(&conn(cid(), 1)), "the claimed session stays open");
}

/// A connection whose guest closes with nothing buffered fails its first
/// read at once (end of stream) and is closed without effect; the launch's
/// own READY then takes the claim.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_connection_closed_with_nothing_buffered_is_closed_without_effect() {
    let node = Node::new().await;
    let launch = token(0x55);
    let (pid, _start) = node.launch(&node.spec("beacon-eof", launch)).await;
    node.io.queue_connection(conn(cid(), 1), pid, &[]).expect("queued");
    assert!(node.io.guest_close(conn(cid(), 1)), "the guest closes its side");
    node.settle().await;
    node.accept_and_read(conn(cid(), 1)).await;

    assert!(node.driver.beacon_observation().claims.is_empty(), "end of stream takes nothing");
    assert!(node.io.beacon_socket_observation().closed_by_driver.contains(&conn(cid(), 1)));

    node.io.queue_connection(conn(cid(), 2), pid, &[ready(launch).as_str()]).expect("queued");
    node.settle().await;
    node.accept_and_read(conn(cid(), 2)).await;
    assert_eq!(node.driver.beacon_observation().claims, BTreeMap::from([(cid(), conn(cid(), 2))]));
}

/// A first line that is not READY closes the connection without effect —
/// whatever it is (another beacon message, a malformed line).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_first_line_that_is_not_ready_closes_the_connection_without_effect() {
    for (n, line) in [(1, "EXIT 0"), (2, "SHUTDOWN"), (3, "READY pid=1 port=1234"), (4, "garbage")]
    {
        let node = Node::new().await;
        let launch = token(0x66);
        let (pid, _start) = node.launch(&node.spec(&format!("beacon-nonready-{n}"), launch)).await;
        node.io.queue_connection(conn(cid(), n), pid, &[line]).expect("queued");
        node.settle().await;
        node.accept_and_read(conn(cid(), n)).await;

        let observation = node.driver.beacon_observation();
        assert!(observation.claims.is_empty(), "{line:?} takes no claim");
        assert!(observation.unread.is_empty(), "{line:?} is read and closed");
        assert!(
            node.io.beacon_socket_observation().closed_by_driver.contains(&conn(cid(), n)),
            "{line:?} closes the connection"
        );
        assert_eq!(observation.live.get(&cid()), Some(&launch), "the launch stays live");
    }
}

/// A connection from a CID with no live launch is closed at accept
/// (`NoLiveLaunch`), before any read.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_connection_from_a_cid_with_no_live_launch_is_closed_at_accept() {
    let node = Node::new().await;
    let other = GuestCid::new(GUEST_CID_BASE + 7).expect("an in-range guest CID");
    let orphan_pid = node.vmm.spawn_bare_process().pid;
    node.io
        .queue_connection(conn(other, 1), orphan_pid, &[ready(token(0x77)).as_str()])
        .expect("queued");
    node.settle().await;
    node.io.release_accept();
    node.settle().await;

    let observation = node.driver.beacon_observation();
    assert!(observation.unread.is_empty(), "never held for a read");
    assert!(observation.claims.is_empty());
    assert!(node.io.beacon_socket_observation().closed_by_driver.contains(&conn(other, 1)));
}

/// D-UP-9 (approved 2026-10-14): the VMM of a launch whose READY holds the
/// claim exits (a crash or a guest reset, RESET-EXIT) with no `stop`: the
/// driver's one per-CID step ends the launch's liveness, closes its beacon
/// session and releases the claim — the next launch's READY takes it, never
/// refused `SessionHeld`. Quint run `VmExitEndsLaunchAndReleasesBeaconClaim`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_vmm_exit_ends_the_launch_and_releases_its_claim_for_the_next_launch() {
    let node = Node::new().await;
    let first = token(0x66);
    let (dead_pid, _first_start) = node.launch(&node.spec("beacon-crashed", first)).await;
    node.io.queue_connection(conn(cid(), 1), dead_pid, &[ready(first).as_str()]).expect("queued");
    node.settle().await;
    node.accept_and_read(conn(cid(), 1)).await;
    assert_eq!(
        node.driver.beacon_observation().claims,
        BTreeMap::from([(cid(), conn(cid(), 1))]),
        "precondition: the first launch's READY holds the claim"
    );

    // The fault: the VMM exits on its own (no `driver.stop`).
    assert!(node.vmm.exit_process(dead_pid, None, Some(9)), "the live VMM exits");
    node.io.observe_vm_exits();
    node.settle().await;
    let after_exit = node.driver.beacon_observation();
    assert!(after_exit.live.is_empty(), "the exit ended the launch's liveness");
    assert!(after_exit.claims.is_empty(), "the exit released the beacon claim");
    assert!(
        node.io.beacon_socket_observation().closed_by_driver.contains(&conn(cid(), 1)),
        "the exited launch's beacon session is closed"
    );

    let next = token(0x77);
    let (pid, _start) = node.launch(&node.spec("beacon-next", next)).await;
    node.io.queue_connection(conn(cid(), 2), pid, &[ready(next).as_str()]).expect("queued");
    node.settle().await;
    node.accept_and_read(conn(cid(), 2)).await;
    let observation = node.driver.beacon_observation();
    assert_eq!(
        observation.claims,
        BTreeMap::from([(cid(), conn(cid(), 2))]),
        "the next launch's READY takes the released claim"
    );
    assert_eq!(observation.live, BTreeMap::from([(cid(), next)]));
}

// ---------------------------------------------------------------------------
// GUEST-PORTS (ADR-0217) — the driver passes the node's bound ports
// ---------------------------------------------------------------------------

/// GUEST-PORTS: the launch's kernel command line carries the one transport
/// token with `beacon=` the intake's own bound port and `stream=` /
/// `control=` the assignment's `flow_ports()`, in the pinned field order
/// after `launch=`, and no other `overdrive.transport=` token. The intake
/// binds kernel-assigned and the flow ports are not the defaults, so a
/// driver writing 1234 / 1240 / 1243 (or any constant) fails here.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_launch_token_names_the_bound_beacon_and_the_assignments_flow_ports() {
    let tmp = TempDir::new().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
        .expect("write synthetic master rootfs");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
    let vmm = SimVmm::new();
    let io = Arc::new(SimBeaconIo::new(vmm.clone()));
    let cmdlines = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let recorder = Arc::new(RecordsCmdline { inner: vmm.clone(), cmdlines: Arc::clone(&cmdlines) });
    let (driver, beacon_port) = driver_over_vmm(recorder, &io, &root).await;
    let launch = token(0x5a);
    let node = Node { _tmp: tmp, root, vmm, io, driver };
    let (_pid, _start) = node.launch(&node.spec("beacon-ports", launch)).await;

    let lines = cmdlines.lock().clone();
    assert_eq!(lines.len(), 1, "one launch, one VMM create");
    let tokens: Vec<&str> =
        lines[0].split_whitespace().filter(|p| p.starts_with("overdrive.transport=")).collect();
    let expected = format!(
        "overdrive.transport=vsock2,addr=100.95.0.2,dns=100.95.0.1,launch={launch},\
         beacon={},stream={FLOW_STREAM},control={FLOW_CONTROL}",
        beacon_port.as_u32()
    );
    assert_eq!(tokens, vec![expected.as_str()], "exactly one transport token, fields in order");
    assert_ne!(
        beacon_port, BEACON_VSOCK_PORT,
        "precondition: the intake's port is not the default"
    );
}
