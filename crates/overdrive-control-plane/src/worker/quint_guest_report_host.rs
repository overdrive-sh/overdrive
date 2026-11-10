//! Quint conformance driver — `guest_report_host` (the host reader's half of
//! the guest listener report protocol, D13 items 3 and 10; the exit report
//! join of `run_exit_watcher`, D13 to D16).
//!
//! Replays `specs/quint/service-allocation-readiness-and-ending/guest_report.qnt`
//! (referenced in place) on its recording conformance main `gr_host_conf`
//! (module `guest_report_host`). Each step's action and pick are read from the
//! single-element path `"gr_host_conf::guest_report_host::conf_step"`
//! ([`Config::nondet`]); the state projection matches the trace's qualified
//! variable names (`gr_host_conf::guest_report_host::<var>`, spike QT1).
//!
//! **The production owners under test.** The production [`VmDriver`] over
//! `SimVmm`, its beacon over a stepped `SimBeaconIo` (the guest's lines are
//! the stimulus), the production [`GuestListenerBoard`] the driver built and
//! passed to `VmDriver::new` and to `ProbeRunner::new` (one board, one
//! instance), and the production exit observer
//! (`exit_observer::spawn_with_runtime`), which takes the exit receiver itself
//! and writes the allocation's terminal row. The VM is started by the
//! production action shim (`dispatch_with_guest_network_provisioner_for_test`,
//! `TestSharedOwner` as the guest-network port, `mtls_worker()` as the mTLS
//! worker), so the shim writes the `Running` row and fires the exit-gate
//! release. Nothing is hand-installed: no row, no board entry, no lease.
//!
//! Action → production call (feature-delta § "Action to driving-port mapping
//! (host-half conformance)"):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh node; the shim starts one Service VM (`StartAllocation`), the guest's READY with the lease's launch token is delivered through `SimBeaconIo`, the shim writes `Running` |
//! | `HostRead` | `SimBeaconIo::release_read`: the real reader consumes the next scripted line and routes it |
//! | `AppendListen(p)` | `SimBeaconIo::guest_write` of `LISTEN <tcp\|udp> <port>` (the pair `p` maps to, below) |
//! | `AppendExit` | `SimBeaconIo::guest_write` of `EXIT 0` |
//! | `HostGarbage` | `SimBeaconIo::guest_write` of an unparseable line |
//! | `BeaconClose` | `SimBeaconIo::guest_close`: EOF to the real reader |
//! | `VmmExit` | `SimVmm::exit_process` (the VMM process exits) and `Driver::release_for_exit_emission`; then the handler awaits the alloc's terminal `AllocStatusRow` written by the production exit observer, polling `ObservationStore::alloc_status_row` with yields under [`SETTLE_BOUND`]. A missed bound fails the step; it never maps to `none` |
//! | `Idle` | a stutter: the reader must stay parked (no grant outstanding), so the projection compared after it is the one before it |
//!
//! Declared pairs (module constants `DECL`, `ALPHA`): `tcp80` is TCP 80,
//! `udp53` is UDP 53, `other` is TCP 8080 (declared by no Service listener).
//! The dispatched Service's `AllocationSpec.declared_listeners` is exactly the
//! `DECL` pairs, from which the board's declared filter derives.
//!
//! Projection (pinned accessors, feature-delta § "Pinned accessors"). `rset`
//! ← `GuestListenerReports::reported(alloc)` on the board this driver built,
//! the only conformance read of the board. `exitrep` ← the three-valued table
//! over `ObservationStore::alloc_status_row(alloc)`: `none` for no row or a
//! non-terminal state; `guest-reported` for `Terminated` with
//! `Stopped { by: Process }` or `Failed` with
//! `WorkloadCrashedImmediately { exit_code: Some(_) }`; `no-report` for
//! `Failed` with `WorkloadCrashedImmediately { exit_code: None }` or
//! `VmOutOfMemory`; any other terminal row fails the step. `wait`, `lines`,
//! `eof` and `gstat` are model-only: never projected. The exit event receiver
//! is never read.
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through
//! quint-connect), `cargo xtask metal run --`.

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

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use itf::Value;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Listener, WorkloadKind};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver as WorkloadDriver, DriverPayload, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::{GuestListenerReports, ReportedListeners};
use overdrive_core::transition_reason::{StoppedBy, TransitionReason};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::{BeaconIntake, GuestListenerBoard, VmDriver, VmHostLayout};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::action_shim::{ShimError, dispatch_with_guest_network_provisioner_for_test};
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::guest_network::{GuestAddressPool, SharedGuestNetworkOwner as _};
use crate::reconciler_runtime::ReconcilerRuntime;
use crate::shared_network_test_ports::{mtls_worker, shared_guest_network_owner};
use crate::vm_reclamation_boot::sim_vm_host::{GATEWAY, RecordingVmm, SimVmmHost};

/// The conformance main and the module declaring the variables.
const MAIN: &str = "gr_host_conf";
const MODULE: &str = "guest_report_host";
/// The test pool of the guest address pool (one lease is taken).
const PREFIX: &str = "100.95.0.0/29";
/// How long a step may take to settle (real milliseconds; the owners read
/// time only through `SimClock`).
const SETTLE_BOUND: Duration = Duration::from_secs(10);
/// Scheduler polls after the reader parks, so the line it consumed is routed
/// before the projection reads the board.
const ROUTE_POLLS: usize = 64;
/// The guest's beacon source port (relabelling only).
const BEACON_PEER_PORT: u32 = 50_001;

/// The spec's declared pairs, in the module's `DECL` order.
const DECL: [&str; 2] = ["tcp80", "udp53"];
/// The spec's pair alphabet: `DECL` plus one undeclared pair.
const ALPHA: [&str; 3] = ["tcp80", "udp53", "other"];

/// One spec pair as a real `(protocol, port)` and its beacon wire form
/// (`LISTEN <tcp|udp> <port>`, D13 item 2).
fn pair(name: &str) -> Option<(Proto, NonZeroU16, &'static str)> {
    let (proto, port, text) = match name {
        "tcp80" => (Proto::Tcp, 80, "LISTEN tcp 80"),
        "udp53" => (Proto::Udp, 53, "LISTEN udp 53"),
        "other" => (Proto::Tcp, 8080, "LISTEN tcp 8080"),
        _ => return None,
    };
    Some((proto, NonZeroU16::new(port).expect("a non-zero port"), text))
}

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("the step did not settle within the settle bound: {0}")]
    Unsettled(String),
    #[error("a production call failed: {0}")]
    Production(String),
    #[error("{0} has no pre-image in the model")]
    NoPreImage(String),
    #[error("ITF: {0}")]
    Itf(#[from] ItfError),
}

fn not_enabled(action: &'static str, why: impl Into<String>) -> DriverError {
    DriverError::NotEnabled { action, why: why.into() }
}

fn stage_artifacts(root: &Path) {
    std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
        .expect("write synthetic master rootfs");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
    std::fs::create_dir_all(root.join("views")).expect("the view store directory");
}

/// The started Service VM's allocation spec: the `DECL` pairs as its declared
/// listeners (the board's declared filter derives from them) over the staged
/// artifacts.
fn service_spec(root: &Path, alloc: &AllocationId) -> AllocationSpec {
    let declared_listeners: Vec<Listener> = DECL
        .iter()
        .map(|name| {
            let (protocol, port, _) = pair(name).expect("a declared pair");
            Listener { port, protocol }
        })
        .collect();
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/gr-host/alloc/gr-host-alloc")
            .expect("valid SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: root.join("vmlinuz"),
            rootfs: root.join("master.img"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: None,
        declared_listeners,
    }
}

/// One node: the production composition, one started Service VM, its beacon
/// connection open. What `init` builds; dropped (with its runtime and every
/// task on it) when the next trace's `init` replaces it.
struct World {
    rt: Runtime,
    _tmp: TempDir,
    _wiring: GuestNetworkExecWiring,
    vmm: SimVmm,
    host: Arc<SimVmmHost>,
    /// The observation port the exit observer and the shim write to; the projection reads
    /// the alloc's row through it.
    obs: Arc<dyn ObservationStore>,
    beacon: Arc<SimBeaconIo>,
    /// The ONE production board: passed to `VmDriver::new` and
    /// `ProbeRunner::new`, read through `GuestListenerReports::reported`.
    board: Arc<GuestListenerBoard>,
    vm_driver: VmDriver,
    alloc: AllocationId,
    key: BeaconConnKey,
    pid: u32,
    _exit_observer: JoinHandle<()>,
}

impl World {
    /// Runs `cond` on the node's runtime until it holds, polling every
    /// millisecond, under [`SETTLE_BOUND`].
    fn settle(
        &self,
        what: &str,
        mut cond: impl FnMut(&Self) -> bool,
    ) -> std::result::Result<(), DriverError> {
        let deadline = Instant::now() + SETTLE_BOUND;
        while !cond(self) {
            if Instant::now() > deadline {
                return Err(DriverError::Unsettled(what.to_owned()));
            }
            self.rt.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        }
        Ok(())
    }

    /// Waits until the reader is parked (no grant outstanding, no call that
    /// could take a delivery), then lets the scheduler route what it consumed.
    fn reader_quiescent(&self, what: &str) -> std::result::Result<(), DriverError> {
        self.settle(what, |world| world.beacon.parked())?;
        self.rt.block_on(async {
            for _ in 0..ROUTE_POLLS {
                tokio::task::yield_now().await;
            }
        });
        self.settle(what, |world| world.beacon.parked())
    }

    /// The alloc's row as the observation store holds it.
    fn row(
        &self,
    ) -> std::result::Result<
        Option<overdrive_core::traits::observation_store::AllocStatusRow>,
        DriverError,
    > {
        self.rt
            .block_on(self.obs.alloc_status_row(&self.alloc))
            .map_err(|error| DriverError::Production(format!("alloc_status_row: {error}")))
    }

    fn row_is_terminal(&self) -> bool {
        self.row().ok().flatten().is_some_and(|row| row.state.is_terminal())
    }
}

/// A fresh node: composition, started VM, READY delivered, `Running` written.
fn build_world() -> std::result::Result<World, DriverError> {
    let tmp = TempDir::new().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    stage_artifacts(&root);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let vmm = SimVmm::new();
    let host = SimVmmHost::new(vmm.clone(), true);
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("guest-report-host").expect("valid node ID"),
        0,
    ));
    let beacon = Arc::new(SimBeaconIo::stepped(vmm.clone()));
    let board = Arc::new(GuestListenerBoard::new());
    let alloc = AllocationId::new("gr-host-alloc").expect("valid allocation ID");

    let (state, pool, owner, wiring, vm_driver, beacon_port, exit_observer) = rt.block_on(async {
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(wiring.supervisor().open_after_boot(), "the composition's EXEC gate opens");
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
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::clone(&board) as Arc<dyn GuestListenerReports>,
        ));
        let vm_driver = VmDriver::new(
            Arc::new(RecordingVmm { inner: vmm.clone(), host: Arc::clone(&host) }),
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
            Arc::clone(&board),
        );
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(&root)
            .expect("the reconciler runtime over the node's view store");
        runtime.register(crate::noop_heartbeat()).await.expect("register a reconciler");
        let pool = Arc::new(GuestAddressPool::new(
            PREFIX.parse().expect("pool prefix"),
            GATEWAY,
            Arc::new(SimEntropy::new(0x0d13)),
        ));
        let store_path = root.join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
        // Boot step 9 on the guest-network owner port: its first `Ok`
        // `converge_shared` makes `flow_ports()` `Some`, which every lease
        // the action shim takes carries. `TestSharedOwner` provisions and
        // activates unscripted.
        let owner = shared_guest_network_owner();
        owner.converge_shared().await.expect("the test owner's converge_shared passes");
        let mtls = mtls_worker();
        mtls.start_shared_owner().await.expect("the test mTLS owner's start passes");
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mut state = AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::new(vm_driver.clone()) as Arc<dyn WorkloadDriver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            NodeId::new("local").expect("valid NodeId"),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            mtls,
            Arc::clone(&owner) as Arc<dyn crate::guest_network::SharedGuestNetworkOwner>,
            wiring.gate(),
            Arc::clone(&pool),
        );
        state.vm_host_state = Arc::clone(&host) as Arc<dyn VmHostState>;
        // The production exit observer takes the exit receiver itself and
        // writes each VMM's terminal row; the driver reads no ExitEvent.
        let exit_observer = crate::worker::exit_observer::spawn_with_runtime(
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(vm_driver.clone()) as Arc<dyn WorkloadDriver>,
            Arc::clone(&state.lifecycle_events),
            Arc::clone(&clock) as Arc<dyn Clock>,
            None,
            CancellationToken::new(),
        );
        (state, pool, owner, wiring, vm_driver, beacon_port, exit_observer)
    });

    // The production action shim starts the VM: lease, `Driver::start`, mTLS
    // intercept, then the `Running` row and the exit-gate release.
    let action = Action::StartAllocation {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("gr-host-svc").expect("valid workload ID"),
        node_id: NodeId::new("local").expect("valid NodeId"),
        spec: service_spec(&root, &alloc),
        kind: WorkloadKind::Service,
    };
    let start: JoinHandle<std::result::Result<(), ShimError>> = {
        let owner = Arc::clone(&owner);
        rt.spawn(async move {
            let now = Instant::now();
            let tick = TickContext {
                now,
                now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_200)),
                tick: 1,
                deadline: now + Duration::from_secs(30),
            };
            Box::pin(dispatch_with_guest_network_provisioner_for_test(
                vec![action],
                &state,
                &tick,
                owner.as_ref(),
            ))
            .await
        })
    };

    let mut world = World {
        rt,
        _tmp: tmp,
        _wiring: wiring,
        vmm,
        host,
        obs: obs as Arc<dyn ObservationStore>,
        beacon,
        board,
        vm_driver,
        alloc: alloc.clone(),
        key: BeaconConnKey {
            peer_cid: overdrive_core::guest_transport::GuestCid::new(
                overdrive_core::guest_transport::GUEST_CID_BASE + 2,
            )
            .expect("an in-range guest CID"),
            peer_port: VsockPort::new(BEACON_PEER_PORT),
        },
        pid: 0,
        _exit_observer: exit_observer,
    };

    // The shim leased a CID and the VMM started under it.
    world.settle("the shim leases a CID and the VMM starts", |w| {
        pool.lease_observation().leases.contains_key(&w.alloc)
            && !w.host.pids_of(&w.alloc).is_empty()
    })?;
    let (cid, launch) = pool
        .lease_observation()
        .leases
        .get(&alloc)
        .map(|view| (view.cid, view.launch))
        .ok_or_else(|| not_enabled("init", "the shim took no lease"))?;
    world.pid = world
        .host
        .pids_of(&alloc)
        .into_iter()
        .next()
        .ok_or_else(|| not_enabled("init", "no VMM started"))?;
    world.key = BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(BEACON_PEER_PORT) };

    // The guest's READY with the launch token and the beacon port it read from
    // its kernel command line (stimulus), accepted and read by the reader.
    let ready = format!("READY pid=1 port={beacon_port} launch={launch}");
    world
        .beacon
        .queue_connection(world.key, world.pid, &[ready.as_str()])
        .map_err(|error| DriverError::Production(format!("the guest's beacon: {error}")))?;
    world.beacon.release_accept();
    world.settle("the beacon connection is accepted", |w| w.beacon.release_read(w.key))?;
    world.settle("the shim's start returns", |_| start.is_finished())?;
    world
        .rt
        .block_on(start)
        .expect("the start dispatch did not panic")
        .map_err(|error| DriverError::Production(format!("StartAllocation: {error}")))?;
    let running = world.row()?.is_some_and(|row| row.state == AllocState::Running);
    if !running {
        return Err(not_enabled("init", "the shim wrote no Running row"));
    }
    world.reader_quiescent("the reader parks after READY")?;
    Ok(world)
}

struct GuestReportHostDriver {
    /// The node under test, held directly so the projection reads the production owners
    /// (the board, the observation port) through it.
    world: World,
    /// The board's reported set as of the end of the previous step (the previously
    /// projected `rset`); `Idle` compares a fresh read against it.
    last_reported: ReportedListeners,
}

impl GuestReportHostDriver {
    fn new() -> Self {
        let world = build_world().expect("the initial node builds");
        let last_reported = world.board.reported(&world.alloc);
        Self { world, last_reported }
    }

    // ----- actions ----------------------------------------------------------------

    fn init(&mut self) -> std::result::Result<(), DriverError> {
        // A fresh node per trace: the runner reuses one driver for every trace.
        self.world = build_world()?;
        self.last_reported = self.world.board.reported(&self.world.alloc);
        Ok(())
    }

    /// The real reader consumes the next scripted line and routes it.
    fn host_read(&self) -> std::result::Result<(), DriverError> {
        let world = &self.world;
        if !world.beacon.release_read(world.key) {
            return Err(not_enabled("HostRead", "the beacon connection is not open"));
        }
        world.reader_quiescent("the reader consumes the granted line")
    }

    fn guest_writes(
        &self,
        action: &'static str,
        line: &str,
    ) -> std::result::Result<(), DriverError> {
        let world = &self.world;
        if !world.beacon.guest_write(world.key, line) {
            return Err(not_enabled(action, "the beacon connection is not open"));
        }
        Ok(())
    }

    fn append_listen(&self, p: &str) -> std::result::Result<(), DriverError> {
        let (_, _, line) = pair(p).ok_or_else(|| DriverError::NoPreImage(format!("pair `{p}`")))?;
        self.guest_writes("AppendListen", line)
    }

    fn append_exit(&self) -> std::result::Result<(), DriverError> {
        self.guest_writes("AppendExit", "EXIT 0")
    }

    fn host_garbage(&self) -> std::result::Result<(), DriverError> {
        self.guest_writes("HostGarbage", "!! this is not a beacon line")
    }

    fn beacon_close(&self) -> std::result::Result<(), DriverError> {
        let world = &self.world;
        if !world.beacon.guest_close(world.key) {
            return Err(not_enabled("BeaconClose", "the beacon connection is not open"));
        }
        Ok(())
    }

    /// The VMM process exits and the exit gate is released; the production
    /// exit observer then writes the row, which the handler awaits.
    fn vmm_exit(&self) -> std::result::Result<(), DriverError> {
        let world = &self.world;
        if !world.vmm.exit_process(world.pid, Some(0), None) {
            return Err(not_enabled("VmmExit", "the VMM is gone"));
        }
        world.beacon.observe_vm_exits();
        world.host.notify();
        let handle = AllocationHandle { alloc: world.alloc.clone(), pid: Some(world.pid) };
        world.rt.block_on(world.vm_driver.release_for_exit_emission(&handle));
        world.settle("the exit observer writes the terminal row", World::row_is_terminal)
    }

    /// A stutter: the reader holds no outstanding grant, and a read-only call to
    /// `GuestListenerReports::reported(alloc)` returns the set the previous step
    /// projected (DG-M). It writes nothing and releases no read.
    fn idle(&self) -> std::result::Result<(), DriverError> {
        self.world.reader_quiescent("the reader stays parked")?;
        let now = self.world.board.reported(&self.world.alloc);
        if now != self.last_reported {
            return Err(DriverError::Production(format!(
                "the reported set changed across a stutter: {:?} before, {:?} after",
                self.last_reported, now
            )));
        }
        Ok(())
    }

    // ----- projection -------------------------------------------------------------

    fn project(&self) -> std::result::Result<HostReportState, DriverError> {
        // `rset` ← the production board's reported set (D13 item 10), the only
        // conformance read of the board.
        let reported = self.world.board.reported(&self.world.alloc);
        let mut rset = BTreeSet::new();
        for name in ALPHA {
            let (proto, port, _) = pair(name).expect("an alphabet pair");
            if reported.contains(proto, port) {
                rset.insert(name.to_owned());
            }
        }
        if reported.len() != rset.len() {
            return Err(DriverError::NoPreImage(format!(
                "a reported pair outside the model's alphabet ({} reported, {} known)",
                reported.len(),
                rset.len()
            )));
        }
        // `exitrep` ← the exit observer's row, three-valued.
        let exitrep = match self.world.row()? {
            None => "none",
            Some(row) if !row.state.is_terminal() => "none",
            Some(row) => match (row.state, row.reason) {
                (
                    AllocState::Terminated,
                    Some(TransitionReason::Stopped { by: StoppedBy::Process }),
                )
                | (
                    AllocState::Failed,
                    Some(TransitionReason::WorkloadCrashedImmediately {
                        exit_code: Some(_), ..
                    }),
                ) => "guest-reported",
                (
                    AllocState::Failed,
                    Some(
                        TransitionReason::WorkloadCrashedImmediately { exit_code: None, .. }
                        | TransitionReason::VmOutOfMemory { .. },
                    ),
                ) => "no-report",
                (state, reason) => {
                    return Err(DriverError::NoPreImage(format!(
                        "terminal row {state:?} / {reason:?}"
                    )));
                }
            },
        };
        Ok(HostReportState { rset, exitrep: exitrep.to_owned() })
    }
}

impl Driver for GuestReportHostDriver {
    type State = HostReportState;

    fn config() -> Config {
        Config { state: &[], nondet: &["gr_host_conf::guest_report_host::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        let applied: Result = (|| {
            switch!(step {
                init => self.init()?,
                HostRead => self.host_read()?,
                AppendListen(p: String) => self.append_listen(&p)?,
                AppendExit => self.append_exit()?,
                HostGarbage => self.host_garbage()?,
                BeaconClose => self.beacon_close()?,
                VmmExit => self.vmm_exit()?,
                Idle => self.idle()?,
            })
        })();
        applied?;
        self.last_reported = self.world.board.reported(&self.world.alloc);
        Ok(())
    }
}

// ----- the compared state ------------------------------------------------------

/// The compared `guest_report_host` variables: `rset` and `exitrep`. `wait`,
/// `lines`, `eof` and `gstat` are model-only and never decoded.
#[derive(Debug, PartialEq, Eq)]
struct HostReportState {
    rset: BTreeSet<String>,
    exitrep: String,
}

impl HostReportState {
    /// Decodes the compared variables by their qualified names
    /// `gr_host_conf::guest_report_host::<var>` (spike QT1).
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(value, MAIN, MODULE)?;
        Ok(Self {
            rset: quint_itf::str_set(vars.var("rset")?)?,
            exitrep: quint_itf::string(vars.var("exitrep")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for HostReportState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<GuestReportHostDriver> for HostReportState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &GuestReportHostDriver) -> Result<Self> {
        Ok(driver.project()?)
    }
}

// ----- named runs (one `#[quint_test]` per run of `gr_host_conf`) and randomized traces ---

/// Named run `host_all_declared_reported`. CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/guest_report.qnt",
    main = "gr_host_conf",
    max_samples = 1,
    test = "host_all_declared_reported"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest report host reader and board"]
fn quint_guest_report_host_all_declared_reported() -> impl Driver {
    GuestReportHostDriver::new()
}

/// Named run `host_garbage_line_is_dropped`. CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/guest_report.qnt",
    main = "gr_host_conf",
    max_samples = 1,
    test = "host_garbage_line_is_dropped"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest report host reader and board"]
fn quint_guest_report_host_garbage_line_is_dropped() -> impl Driver {
    GuestReportHostDriver::new()
}

/// Named run `host_beacon_closes_with_partial_set`. CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/guest_report.qnt",
    main = "gr_host_conf",
    max_samples = 1,
    test = "host_beacon_closes_with_partial_set"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest report host reader and board"]
fn quint_guest_report_host_beacon_closes_with_partial_set() -> impl Driver {
    GuestReportHostDriver::new()
}

/// Named run `host_listen_after_exit_is_ignored`. CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/guest_report.qnt",
    main = "gr_host_conf",
    max_samples = 1,
    test = "host_listen_after_exit_is_ignored"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest report host reader and board"]
fn quint_guest_report_host_listen_after_exit_is_ignored() -> impl Driver {
    GuestReportHostDriver::new()
}

/// Randomized traces of the recording main; the seed is printed on failure.
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/guest_report.qnt",
    main = "gr_host_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest report host reader and board"]
fn quint_guest_report_host_randomized_traces() -> impl Driver {
    GuestReportHostDriver::new()
}
