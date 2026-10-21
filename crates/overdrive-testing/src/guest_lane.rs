//! The in-guest test lane of the guest-kit fixture (GH #295, T-2, approved
//! with T-1 on 2026-10-18).
//!
//! Some cells observe a configured guest from the application's position:
//! `overdrive-init`'s `transport::real_kernel` and `guest_flow::real_kernel`
//! (D-UP-18). They run *inside* a microVM whose PID 1 is the production
//! `overdrive-init` — the agent that configures the guest transport on the
//! real guest kernel before READY — as that guest's operator command.
//!
//! [`run_in_guest_cell`] does exactly what the host side of a launch does
//! for such a guest, and no more: it plays the node beacon (accepts the
//! guest's beacon connection, reads READY, answers `EXEC` with the cell's
//! argv, reads `EXIT`) and holds the guest owner's control session open
//! (accepted and drained; nothing is answered on it). It runs no host flow
//! owner: the cells are about the guest, and the guest owner's host peer is
//! not their subject.
//!
//! **Two lanes with the scripted peer as PID 1.** Some in-guest cells must be
//! the guest's only holder of a resource the production `overdrive-init`
//! would hold:
//! - [`run_in_guest_owner_cell`] (D-22, R5-28 / R5-31's Tier-3 half): the
//!   cell constructs the production `GuestFlowOwner` over
//!   `KernelGuestOwnerIo` and is the guest's only owner; the lane is its
//!   scripted host peer ([`OwnerLaneHost`]) over the real vhost-vsock and
//!   talks to the cell on a [`LaneScript`] line channel.
//! - [`run_scripted_in_guest_cell`] (U-EV-UNLOAD): nothing in the guest holds
//!   `ovd_fwd`, so the cell can unload it, end the boot, or need a module the
//!   kit does not load.
//!
//! **Ports (GUEST-PORTS, ADR-0217; PORTS-CONFIG, ADR-0218).** The lane binds
//! its beacon, flow-stream and control listeners on kernel-assigned ports
//! ([`HostListeners::bind`]) and writes the three bound ports on the guest's
//! kernel command line as `beacon=`, `stream=`, `control=`, exactly as the VM
//! driver does; no lane binds a fixed host port, so lanes of different
//! workspaces never collide on the node-global vsock port space.
//!
//! The cell binary is the package's libtest binary built with
//! `integration-tests,kvm-tests` for the host target by the lane's setup
//! step (`cargo xtask guest-kit-prepare`, which stages it at
//! `<target>/guest-kit/in-guest/<package>--<bin>`; this module builds
//! nothing and refuses `LaneNotPrepared` when it is absent), staged with its ELF
//! interpreter and shared libraries; the guest program
//! `overdrive-guest-peer run-with-modules` loads the vsock loopback module
//! (some cells open connections to the guest's own listeners over
//! `VMADDR_CID_LOCAL`) and execs it with `--exact <cell>`. The guest console
//! carries libtest's output; [`CellRun::passed`] is true only for
//! `test result: ok. 1 passed`, so an in-guest cell that is still ignored
//! (`0 passed; … 1 ignored`) is not a pass.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "kernel and file names in prose")]

use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use std::str::FromStr as _;

use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, FLOW_REQUEST_LEN, FlowId, FlowOrigin,
    FlowRequest,
};
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_INBOUND_PORT, GUEST_NODE_GATEWAY, LaunchToken,
};
use overdrive_core::net::ListenPort;
use overdrive_core::vm::beacon::BeaconMessage;

use crate::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use crate::guest_kit::{
    GuestBoot, GuestFile, GuestImageSpec, GuestInit, GuestKit, GuestKitError,
    LOOPBACK_MODULE_GUEST_PATH, ScriptedBootExtra,
};
use crate::guest_peer::{PeerCommand, PeerReply};
use crate::vm_fixture;
use crate::vsock::{VMADDR_CID_ANY, VMADDR_CID_HOST, VMADDR_PORT_ANY, VsockListener, VsockStream};

/// The three node-global host listeners a production guest connects to,
/// bound on kernel-assigned ports (`VMADDR_PORT_ANY`).
pub struct HostListeners {
    /// The beacon listener (READY, EXEC, EXIT).
    pub beacon: VsockListener,
    /// The guest-flow stream listener. The lane serves no flow; the listener
    /// is held so the port the token names is a real, held port.
    pub stream: VsockListener,
    /// The guest owner's control listener.
    pub control: VsockListener,
}

impl HostListeners {
    /// Binds all three on kernel-assigned ports.
    ///
    /// # Errors
    ///
    /// [`GuestKitError::ControlBind`] naming `VMADDR_PORT_ANY` and the
    /// kernel's error (`EADDRNOTAVAIL` when the auto-bind found no free port).
    pub fn bind() -> Result<Self, GuestKitError> {
        let any = || {
            VsockListener::bind(VMADDR_CID_ANY, VMADDR_PORT_ANY)
                .map_err(|source| GuestKitError::ControlBind { port: VMADDR_PORT_ANY, source })
        };
        Ok(Self { beacon: any()?, stream: any()?, control: any()? })
    }

    /// The `overdrive.transport=` token for a guest at `cid` with the lane's
    /// launch token: the pinned seven fields, the three ports the bound
    /// listeners report.
    #[must_use]
    pub fn transport_token(&self, cid: u32) -> String {
        format!(
            "overdrive.transport=vsock2,addr={},dns={GUEST_NODE_GATEWAY},launch={LANE_LAUNCH_TOKEN},beacon={},stream={},control={}",
            lane_workload_addr(cid),
            self.beacon.port(),
            self.stream.port(),
            self.control.port()
        )
    }
}

/// Where the cell binary lives in the guest.
pub const IN_GUEST_TESTS_PATH: &str = "/opt/overdrive-testing/in-guest-tests";

/// How long the guest has to boot and send READY.
pub const READY_BOUND: Duration = Duration::from_secs(60);
/// How long one cell may run.
pub const CELL_BOUND: Duration = Duration::from_secs(300);
/// The launch token every lane guest carries (32 lowercase hex, non-zero;
/// the lane runs no owner that checks it).
pub const LANE_LAUNCH_TOKEN: &str = "6c616e652d6c61756e63682d746f6b65";

/// A package's libtest binary, built for the in-guest lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InGuestTests {
    /// The executable.
    pub binary: PathBuf,
}

impl InGuestTests {
    /// The libtest binary of `package`'s `bin` target that the lane setup
    /// step (`cargo xtask guest-kit-prepare`) staged at
    /// `<target>/guest-kit/in-guest/<package>--<bin>` — checked, not built.
    ///
    /// # Errors
    ///
    /// [`GuestKitError::LaneNotPrepared`] when the binary is not there.
    pub fn prepared(package: &str, bin: &str) -> Result<Self, GuestKitError> {
        let root = vm_fixture::workspace_root();
        let binary = vm_fixture::cargo_target_dir(&root)
            .join("guest-kit")
            .join("in-guest")
            .join(format!("{package}--{bin}"));
        let binary = crate::guest_kit::prepared(
            binary,
            &format!("the in-guest test binary of {package} bin {bin}"),
        )?;
        Ok(Self { binary })
    }
}

/// One cell's run in a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRun {
    /// The cell.
    pub cell: String,
    /// The operator command's exit status as the guest's `EXIT` reported it.
    pub exit_status: i32,
    /// The guest console (libtest's output among the kernel's).
    pub console: String,
}

impl CellRun {
    /// The cell ran and passed: exit 0 and libtest's `1 passed` summary.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.exit_status == 0 && self.console.contains("test result: ok. 1 passed")
    }
}

/// Accepts and holds connections on one host listener, draining what they
/// send, until dropped.
struct Holder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Holder {
    fn start(listener: VsockListener, cid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::Builder::new()
            .name("guest-lane-control".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || {
                    let mut held: Vec<VsockStream> = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !stop.load(Ordering::SeqCst) {
                        match listener.accept_within(Duration::from_millis(50)) {
                            Ok(Some((stream, peer))) if peer.cid == cid => held.push(stream),
                            Ok(_) => {}
                            Err(_) => return,
                        }
                        held.retain(|stream| match stream.read_within(&mut buf, Duration::ZERO) {
                            Ok(Some(0)) | Err(_) => false,
                            Ok(_) => true,
                        });
                    }
                }
            })
            .unwrap_or_else(|e| panic!("spawn the control holder: {e}"));
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_line(stream: &VsockStream, bound: Duration) -> io::Result<Option<String>> {
    let deadline = Instant::now() + bound;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.read_within(&mut byte, left)? {
            None => return Ok(None),
            Some(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Some(_) if byte[0] == b'\n' => {
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            Some(_) => line.push(byte[0]),
        }
    }
}

fn lane_error(detail: String) -> GuestKitError {
    GuestKitError::Staging { detail, source: io::Error::other("in-guest lane") }
}

/// The workload address a lane guest at `cid` is given: inside the guest
/// prefix, never the gateway.
#[must_use]
pub fn lane_workload_addr(cid: u32) -> Ipv4Addr {
    let offset = cid.wrapping_sub(GUEST_CID_BASE);
    let [_, _, hi, lo] = offset.to_be_bytes();
    let gateway = GUEST_NODE_GATEWAY.octets();
    Ipv4Addr::new(gateway[0], gateway[1], hi.max(1), lo.max(2))
}

/// Runs the in-guest `cell` of `tests` in a fresh guest at `cid`, with
/// `work` for the image and the run files.
///
/// # Errors
///
/// A [`GuestKitError`] for every host-side failure (the image, the launch,
/// the beacon exchange); a failing cell is a [`CellRun`] that did not pass.
pub fn run_in_guest_cell(
    kit: &GuestKit,
    tests: &InGuestTests,
    cell: &str,
    cid: u32,
    work: &Path,
) -> Result<CellRun, GuestKitError> {
    let name = cell.replace("::", "-");
    let image = kit.image(
        work,
        &name,
        &GuestImageSpec {
            init: GuestInit::OverdriveInit,
            forwarder_module: true,
            loopback_module: true,
            extra: vec![GuestFile {
                host: tests.binary.clone(),
                guest: PathBuf::from(IN_GUEST_TESTS_PATH),
                with_shared_libraries: true,
            }],
        },
    )?;
    let listeners = HostListeners::bind()?;
    let token = listeners.transport_token(cid);
    let HostListeners { beacon, stream: _stream, control } = listeners;
    let _holder = Holder::start(control, cid);
    let run_dir = work.join(format!("{name}-run"));
    let vm = kit.boot(&GuestBoot {
        image: &image,
        cid,
        cmdline_extra: vec![token],
        memory_mib: 512,
        vcpus: 2,
        run_dir: &run_dir,
    })?;
    let deadline = Instant::now() + READY_BOUND;
    let conn = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match beacon.accept_within(left) {
            Ok(Some((stream, peer))) if peer.cid == cid => break stream,
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(lane_error(format!(
                    "{cell}: the guest sent no beacon within {READY_BOUND:?}; console:\n{}",
                    vm.console_tail(8192)
                )));
            }
            Err(e) => return Err(lane_error(format!("{cell}: beacon accept: {e}"))),
        }
    };
    let ready = read_line(&conn, READY_BOUND)
        .map_err(|e| lane_error(format!("{cell}: reading READY: {e}")))?
        .ok_or_else(|| {
            lane_error(format!("{cell}: no READY; console:\n{}", vm.console_tail(8192)))
        })?;
    match ready.parse::<BeaconMessage>() {
        Ok(BeaconMessage::Ready { .. }) => {}
        other => {
            return Err(lane_error(format!("{cell}: expected READY, got {ready:?} ({other:?})")));
        }
    }
    let argv: Vec<String> = [
        GUEST_PEER_GUEST_PATH,
        subcommand::RUN_WITH_MODULES,
        LOOPBACK_MODULE_GUEST_PATH,
        "--",
        IN_GUEST_TESTS_PATH,
        "--exact",
        cell,
        "--test-threads=1",
        "--nocapture",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let exec = BeaconMessage::Exec { argv };
    (&conn)
        .write_all(format!("{exec}\n").as_bytes())
        .map_err(|e| lane_error(format!("{cell}: writing EXEC: {e}")))?;
    let exit = read_line(&conn, CELL_BOUND)
        .map_err(|e| {
            lane_error(format!("{cell}: reading EXIT: {e}; console:\n{}", vm.console_tail(16_384)))
        })?
        .ok_or_else(|| {
            lane_error(format!(
                "{cell}: no EXIT within {CELL_BOUND:?}; console:\n{}",
                vm.console_tail(16_384)
            ))
        })?;
    let exit_status = match exit.parse::<BeaconMessage>() {
        Ok(BeaconMessage::Exit { status }) => status,
        other => {
            return Err(lane_error(format!("{cell}: expected EXIT, got {exit:?} ({other:?})")));
        }
    };
    // The guest powers off once EXIT is sent; the console is complete when
    // the VMM has exited.
    let _ = vm.wait_exit(Duration::from_secs(30))?;
    Ok(CellRun { cell: cell.to_owned(), exit_status, console: vm.console() })
}

/// How a production guest's boot ended, as the lane's beacon saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootOutcome {
    /// The guest sent READY.
    Ready {
        /// The READY line.
        line: String,
    },
    /// The guest sent no READY and its VMM exited (a pre-READY refusal: the
    /// guest powers off).
    ExitedBeforeReady {
        /// The guest console.
        console: String,
    },
}

/// Boots a production guest from `spec` (its PID 1 must be
/// `GuestInit::OverdriveInit`) at `cid` and waits for READY or for the VMM
/// to exit, within [`READY_BOUND`]. The guest is ended before return.
///
/// # Errors
///
/// A host-side [`GuestKitError`]; a guest that neither sends READY nor exits
/// within the bound is an error naming its console.
pub fn boot_to_ready(
    kit: &GuestKit,
    spec: &GuestImageSpec,
    cid: u32,
    work: &Path,
) -> Result<BootOutcome, GuestKitError> {
    let image = kit.image(work, "boot-to-ready", spec)?;
    let listeners = HostListeners::bind()?;
    let token = listeners.transport_token(cid);
    let HostListeners { beacon, stream: _stream, control } = listeners;
    let _holder = Holder::start(control, cid);
    let run_dir = work.join("boot-to-ready-run");
    let vm = kit.boot(&GuestBoot {
        image: &image,
        cid,
        cmdline_extra: vec![token],
        memory_mib: 256,
        vcpus: 1,
        run_dir: &run_dir,
    })?;
    let deadline = Instant::now() + READY_BOUND;
    let mut conn: Option<VsockStream> = None;
    loop {
        if conn.is_none()
            && let Ok(Some((stream, peer))) = beacon.accept_within(Duration::from_millis(100))
            && peer.cid == cid
        {
            conn = Some(stream);
        }
        if let Some(stream) = &conn {
            match read_line(stream, Duration::from_millis(100)) {
                Ok(Some(line))
                    if line
                        .parse::<BeaconMessage>()
                        .is_ok_and(|m| matches!(m, BeaconMessage::Ready { .. })) =>
                {
                    return Ok(BootOutcome::Ready { line });
                }
                Ok(Some(line)) => {
                    return Err(lane_error(format!("the guest sent {line:?} instead of READY")));
                }
                Ok(None) | Err(_) => {}
            }
        }
        if vm.try_exit().is_some() {
            return Ok(BootOutcome::ExitedBeforeReady { console: vm.console() });
        }
        if Instant::now() >= deadline {
            return Err(lane_error(format!(
                "the guest neither sent READY nor exited within {READY_BOUND:?}; console:\n{}",
                vm.console_tail(8192)
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// The scripted lanes: a cell inside a guest whose PID 1 is the scripted peer
// ---------------------------------------------------------------------------

/// The in-guest command line that runs `cell` of `tests` after loading the
/// vsock loopback module and `modules` (guest paths).
fn cell_argv(cell: &str, modules: &[&Path]) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        GUEST_PEER_GUEST_PATH.to_owned(),
        subcommand::RUN_WITH_MODULES.to_owned(),
        LOOPBACK_MODULE_GUEST_PATH.to_owned(),
    ];
    argv.extend(modules.iter().map(|m| m.display().to_string()));
    argv.extend(
        ["--", IN_GUEST_TESTS_PATH, "--exact", cell, "--test-threads=1", "--nocapture"]
            .iter()
            .map(|s| (*s).to_owned()),
    );
    argv
}

/// Runs the in-guest `cell` of `tests` in a fresh guest at `cid` whose PID 1
/// is the scripted peer (no Overdrive owner, no beacon).
///
/// The kit's
/// `ovd_fwd.ko` is loaded by the peer at boot and nothing holds it, so a cell
/// may unload it (V-EV-UNLOAD, U-EV-UNLOAD). `modules` are further module
/// files the image carries and the launcher loads before the cell (e.g. the
/// ULP control's `tls`, [`crate::guest_kit::host_module_file`]). The guest is
/// powered off after the cell.
///
/// # Errors
///
/// A host-side [`GuestKitError`]; a failing cell is a [`CellRun`] that did
/// not pass.
pub fn run_scripted_in_guest_cell(
    kit: &GuestKit,
    tests: &InGuestTests,
    cell: &str,
    cid: u32,
    work: &Path,
    modules: &[GuestFile],
    memory_mib: u32,
) -> Result<CellRun, GuestKitError> {
    let name = cell.replace("::", "-");
    let mut extra = vec![GuestFile {
        host: tests.binary.clone(),
        guest: PathBuf::from(IN_GUEST_TESTS_PATH),
        with_shared_libraries: true,
    }];
    extra.extend(modules.iter().cloned());
    let image = kit.image(
        work,
        &name,
        &GuestImageSpec {
            init: GuestInit::ScriptedPeer,
            forwarder_module: true,
            loopback_module: true,
            extra,
        },
    )?;
    let mut guest = kit.boot_scripted_with(
        &image,
        cid,
        &work.join(format!("{name}-run")),
        &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(memory_mib), vcpus: Some(2) },
    )?;
    let module_paths: Vec<&Path> = modules.iter().map(|m| m.guest.as_path()).collect();
    let argv = cell_argv(cell, &module_paths);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let exit_status = guest.peer().exec(&argv, CELL_BOUND)?;
    let console = guest.vm.console();
    let _ = guest.take_peer().power_off();
    let _ = guest.vm.wait_exit(Duration::from_secs(30))?;
    Ok(CellRun { cell: cell.to_owned(), exit_status, console })
}

/// The kernel parameter naming the host port of the owner lane's script
/// channel (`overdrive.lanescript=<port>`).
pub const LANE_SCRIPT_PARAM: &str = "overdrive.lanescript=";

/// What the owner lane's two ends agree on (D-22): the ports and the script
/// lines. Test support only.
pub mod owner_lane {
    /// The in-guest application's TCP port (`workload_addr:APP_PORT`): the
    /// `listen_port` of every `TcpAccept` the lane opens.
    pub const APP_PORT: u16 = 8081;
    /// The guest port of the cell's raw vsock listener (the V-24 witness).
    pub const RAW_PORT: u32 = 40_001;
    /// Host-origin flow sequence numbers the cells use.
    pub const ABORTED_FLOW: u32 = 1;
    /// The positive control's flow: served end to end on the same lane.
    pub const CONTROL_FLOW: u32 = 2;
    /// The burst's first flow sequence number.
    pub const BURST_FIRST: u32 = 100;
    /// The burst's flow count (R5-31: a 1,000-flow burst, P-38).
    pub const BURST: u32 = 1_000;

    /// host → cell: the host accepted the owner's control session.
    pub const SESSION_ACCEPTED: &str = "SESSION-ACCEPTED";
    /// cell → host: the cell stopped dispatching its owner, so a `V_h` the
    /// host opens now stays queued at the guest until the cell resumes.
    pub const PAUSED: &str = "PAUSED";
    /// host → cell: the host sent `Abort` for [`ABORTED_FLOW`], a flow the
    /// guest does not hold (nothing of it was opened yet).
    pub const ABORT_SENT: &str = "ABORT-SENT";
    /// cell → host (raw witness): the cell's raw listener is bound.
    pub const LISTENING: &str = "LISTENING";
    /// host → cell: `V_h` opened, the request written, `V_h` closed; the
    /// guest has not accepted `V_g` (it is still queued).
    pub const QUEUED: &str = "QUEUED";
    /// host → cell: `V_h` opened and the request written; `V_h` stays open.
    pub const REQUESTED: &str = "REQUESTED";
    /// cell → host: the guest owner holds the flow `Accepted` (request read,
    /// connect to the application issued) and is stopped there.
    pub const ACCEPTED: &str = "ACCEPTED";
    /// cell → host: the guest owner holds the flow `Paired`.
    pub const PAIRED: &str = "PAIRED";
    /// host → cell: the host closed `V_h` (it wrote nothing after the
    /// request).
    pub const CLOSED: &str = "CLOSED";
    /// host → cell: the flow was queued and closed, then the host closed the
    /// control session.
    pub const QUEUED_THEN_SESSION_CLOSED: &str = "QUEUED-THEN-SESSION-CLOSED";
    /// host → cell (raw witness): connected and the request written, closed.
    pub const RAW_QUEUED: &str = "RAW-QUEUED";
    /// host → cell (raw witness): connected, nothing written yet.
    pub const RAW_CONNECTED: &str = "RAW-CONNECTED";
    /// cell → host (raw witness): the cell accepted the connection.
    pub const RAW_ACCEPTED: &str = "RAW-ACCEPTED";
    /// cell → host: every burst flow is `Paired`.
    pub const ALL_PAIRED: &str = "ALL-PAIRED";
    /// host → cell: every burst flow's `V_h` is closed.
    pub const CLOSED_ALL: &str = "CLOSED-ALL";
    /// cell → host: the cell's checks are done.
    pub const DONE: &str = "DONE";
}

/// A line channel between a host test and its in-guest cell: one
/// `\n`-terminated ASCII line per step, in both directions. Test support of
/// the owner lane (D-22); it carries no protocol of the product.
#[derive(Debug)]
pub struct LaneScript {
    stream: VsockStream,
    pending: Vec<u8>,
}

impl LaneScript {
    /// The cell's end, inside a guest: connects to the host (CID 2) on the
    /// port the guest's [`LANE_SCRIPT_PARAM`] names.
    ///
    /// # Panics
    ///
    /// Outside an owner-lane guest (no parameter) or when the host does not
    /// accept within [`READY_BOUND`] — the cell fails closed.
    #[must_use]
    pub fn connect_from_guest() -> Self {
        let cmdline = std::fs::read_to_string("/proc/cmdline")
            .unwrap_or_else(|e| panic!("precondition: /proc/cmdline is readable: {e}"));
        let port = cmdline
            .split_whitespace()
            .find_map(|w| w.strip_prefix(LANE_SCRIPT_PARAM))
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or_else(|| {
                panic!(
                    "not running in an owner-lane guest: no {LANE_SCRIPT_PARAM}<port> on the \
                     kernel command line (this cell runs only inside the in-guest owner lane)"
                )
            });
        let stream = VsockStream::connect_retrying(VMADDR_CID_HOST, port, READY_BOUND)
            .unwrap_or_else(|e| panic!("the lane's script channel connects to port {port}: {e}"));
        Self { stream, pending: Vec::new() }
    }

    /// Wraps an accepted connection (the host's end).
    #[must_use]
    pub const fn from_stream(stream: VsockStream) -> Self {
        Self { stream, pending: Vec::new() }
    }

    /// Sends one line.
    ///
    /// # Errors
    ///
    /// The write's error.
    pub fn say(&self, line: &str) -> io::Result<()> {
        (&self.stream).write_all(format!("{line}\n").as_bytes())
    }

    /// The next line within `bound`, `None` when none arrived.
    ///
    /// # Errors
    ///
    /// A read error; `UnexpectedEof` when the other end closed.
    pub fn hear_within(&mut self, bound: Duration) -> io::Result<Option<String>> {
        let deadline = Instant::now() + bound;
        loop {
            if let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=at).collect();
                return Ok(Some(String::from_utf8_lossy(&line[..at]).into_owned()));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let mut buf = [0u8; 256];
            match self.stream.read_within(&mut buf, left)? {
                None => return Ok(None),
                Some(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Some(n) => self.pending.extend_from_slice(&buf[..n]),
            }
            if Instant::now() >= deadline && !self.pending.contains(&b'\n') {
                return Ok(None);
            }
        }
    }

    /// The next line within `bound`; a timeout is an error naming `what`.
    ///
    /// # Errors
    ///
    /// A read error, the other end's close, or no line within `bound`.
    pub fn hear(&mut self, what: &str, bound: Duration) -> io::Result<String> {
        self.hear_within(bound)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("no {what} line on the lane's script channel within {bound:?}"),
            )
        })
    }
}

/// The host side of the owner lane (D-22): the scripted host peer of a guest
/// whose only flow owner is the in-guest cell's production `GuestFlowOwner`.
///
/// It holds the guest owner's control listener, dials the guest's inbound
/// port as the host's `GuestStreamDial` does on the wire, and talks to the
/// cell over the [`LaneScript`]. It never runs a host owner: it writes
/// exactly the wire behaviour the host half's seeded-sim rig asserts.
#[derive(Debug)]
pub struct OwnerLaneHost {
    cid: u32,
    launch: LaunchToken,
    control: VsockListener,
    /// The script channel to the cell.
    pub script: LaneScript,
}

impl OwnerLaneHost {
    /// The guest's CID.
    #[must_use]
    pub const fn cid(&self) -> u32 {
        self.cid
    }

    /// The launch token the guest's transport token carries.
    #[must_use]
    pub const fn launch(&self) -> LaunchToken {
        self.launch
    }

    /// Accepts the guest owner's next control connection within `bound` and
    /// reads its `ControlOpen` (`CONTROL_OPEN_LEN` bytes); the session's host
    /// end.
    ///
    /// # Errors
    ///
    /// No connection within `bound`, or a short `ControlOpen`.
    pub fn accept_session(&self, bound: Duration) -> io::Result<VsockStream> {
        let deadline = Instant::now() + bound;
        let session = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.control.accept_within(left)? {
                Some((stream, peer)) if peer.cid == self.cid => break stream,
                Some(_) => {}
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("the guest owner opened no control session within {bound:?}"),
                    ));
                }
            }
        };
        let mut open = [0u8; CONTROL_OPEN_LEN];
        read_exact_within(&session, &mut open, bound)?;
        Ok(session)
    }

    /// Opens `V_h` to the guest's inbound port (`GUEST_INBOUND_PORT`) and
    /// writes the host-origin `TcpAccept` request `seq` for `listen_port`,
    /// carrying the guest's launch token and the gateway as peer (ADR-0174),
    /// and nothing else. The caller decides when `V_h` closes.
    ///
    /// # Errors
    ///
    /// The connect's or the write's error.
    pub fn open_tcp_accept(&self, seq: u32, listen_port: u16) -> io::Result<VsockStream> {
        self.open_tcp_accept_on(GUEST_INBOUND_PORT.as_u32(), seq, listen_port)
    }

    /// [`Self::open_tcp_accept`] to the guest's `port` (the V-24 witness's
    /// raw listener).
    ///
    /// # Errors
    ///
    /// The connect's or the write's error.
    pub fn open_tcp_accept_on(
        &self,
        port: u32,
        seq: u32,
        listen_port: u16,
    ) -> io::Result<VsockStream> {
        let request = self.tcp_accept_request(seq, listen_port)?;
        let v_h = VsockStream::connect(self.cid, port, None)?;
        (&v_h).write_all(&request)?;
        Ok(v_h)
    }

    /// The encoded host-origin `TcpAccept` request `seq` for `listen_port`,
    /// with the guest's launch token and the gateway as peer (ADR-0174).
    ///
    /// # Errors
    ///
    /// A `seq` or `listen_port` outside its newtype's range.
    pub fn tcp_accept_request(
        &self,
        seq: u32,
        listen_port: u16,
    ) -> io::Result<[u8; FLOW_REQUEST_LEN]> {
        let flow = FlowId::new(FlowOrigin::Host, seq)
            .map_err(|e| io::Error::other(format!("flow id {seq}: {e:?}")))?;
        let listen_port = ListenPort::from_str(&listen_port.to_string())
            .map_err(|e| io::Error::other(format!("listen port {listen_port}: {e:?}")))?;
        Ok(FlowRequest::TcpAccept {
            flow,
            peer: GUEST_NODE_GATEWAY,
            listen_port,
            launch: self.launch,
        }
        .encode())
    }

    /// A plain vsock connection to the guest's `port` (the V-24 witness's
    /// raw listener).
    ///
    /// # Errors
    ///
    /// The connect's error.
    pub fn connect_raw(&self, port: u32) -> io::Result<VsockStream> {
        VsockStream::connect(self.cid, port, None)
    }
}

/// Writes one control message on a session's host end.
///
/// # Errors
///
/// The write's error.
pub fn send_control(session: &VsockStream, message: &ControlMessage) -> io::Result<()> {
    let mut w = session;
    w.write_all(&message.encode())
}

/// Every control message the guest writes on `session` until `quiet`
/// passes with no byte (or the session ends). A partial record at the end is
/// an error.
///
/// # Errors
///
/// A read error, a partial record, or an undecodable one.
pub fn control_messages_within(
    session: &VsockStream,
    quiet: Duration,
) -> io::Result<Vec<ControlMessage>> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match session.read_within(&mut buf, quiet) {
            Ok(None | Some(0)) => break,
            Ok(Some(n)) => bytes.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => break,
            Err(e) => return Err(e),
        }
    }
    if bytes.len() % CONTROL_MESSAGE_LEN != 0 {
        return Err(io::Error::other(format!(
            "a partial control record: {} bytes on the session",
            bytes.len()
        )));
    }
    bytes
        .chunks_exact(CONTROL_MESSAGE_LEN)
        .map(|chunk| {
            let record: [u8; CONTROL_MESSAGE_LEN] = chunk
                .try_into()
                .unwrap_or_else(|_| unreachable!("chunks_exact yields whole records"));
            ControlMessage::decode(&record)
                .map_err(|e| io::Error::other(format!("an undecodable control record: {e:?}")))
        })
        .collect()
}

fn read_exact_within(stream: &VsockStream, buf: &mut [u8], bound: Duration) -> io::Result<()> {
    let deadline = Instant::now() + bound;
    let mut got = 0;
    while got < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.read_within(&mut buf[got..], left)? {
            None => return Err(io::Error::from(io::ErrorKind::TimedOut)),
            Some(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Some(n) => got += n,
        }
    }
    Ok(())
}

/// Runs the in-guest owner `cell` of `tests` in a fresh guest at `cid`
/// (D-22).
///
/// PID 1 is the scripted peer, so the cell's production
/// `GuestFlowOwner` is the guest's **only** owner (no `overdrive-init` owner
/// contends for the inbound port, the UDP slots, the capture programs or
/// the control port). The guest's command line carries the transport token
/// of [`HostListeners::transport_token`] — the cell configures the guest
/// transport with the production setup and owns its control session against
/// this lane's control listener — and the [`LANE_SCRIPT_PARAM`] of the
/// script channel. `host` plays the scripted host peer while the cell runs;
/// the cell's run and `host`'s result are returned.
///
/// # Errors
///
/// A host-side [`GuestKitError`]; a failing cell is a [`CellRun`] that did
/// not pass.
pub fn run_in_guest_owner_cell<R>(
    kit: &GuestKit,
    tests: &InGuestTests,
    cell: &str,
    cid: u32,
    work: &Path,
    memory_mib: u32,
    host: impl FnOnce(&mut OwnerLaneHost) -> R,
) -> Result<(CellRun, R), GuestKitError> {
    let name = cell.replace("::", "-");
    let image = kit.image(
        work,
        &name,
        &GuestImageSpec {
            init: GuestInit::ScriptedPeer,
            forwarder_module: true,
            loopback_module: true,
            extra: vec![GuestFile {
                host: tests.binary.clone(),
                guest: PathBuf::from(IN_GUEST_TESTS_PATH),
                with_shared_libraries: true,
            }],
        },
    )?;
    let listeners = HostListeners::bind()?;
    let token = listeners.transport_token(cid);
    let HostListeners { beacon: _beacon, stream: _stream, control } = listeners;
    let script_listener = VsockListener::bind(VMADDR_CID_ANY, VMADDR_PORT_ANY)
        .map_err(|source| GuestKitError::ControlBind { port: VMADDR_PORT_ANY, source })?;
    let mut guest = kit.boot_scripted_with(
        &image,
        cid,
        &work.join(format!("{name}-run")),
        &ScriptedBootExtra {
            cmdline: vec![token, format!("{LANE_SCRIPT_PARAM}{}", script_listener.port())],
            memory_mib: Some(memory_mib),
            vcpus: Some(2),
        },
    )?;
    let argv = cell_argv(cell, &[]);
    guest.peer().spawn("cell", PeerCommand::Exec { argv })?;
    let deadline = Instant::now() + READY_BOUND;
    let script = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match script_listener.accept_within(left) {
            Ok(Some((stream, peer))) if peer.cid == cid => break LaneScript::from_stream(stream),
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(lane_error(format!(
                    "{cell}: the cell opened no script channel within {READY_BOUND:?}; console:\n{}",
                    guest.vm.console_tail(16_384)
                )));
            }
            Err(e) => return Err(lane_error(format!("{cell}: script accept: {e}"))),
        }
    };
    let launch = LaunchToken::from_str(LANE_LAUNCH_TOKEN)
        .map_err(|e| lane_error(format!("the lane's launch token parses: {e:?}")))?;
    let mut lane = OwnerLaneHost { cid, launch, control, script };
    let result = host(&mut lane);
    let exit_status = match guest.peer().join("cell", CELL_BOUND)? {
        Some(PeerReply::Exit(code)) => code,
        Some(other) => {
            return Err(lane_error(format!("{cell}: the cell's task replied {other:?}")));
        }
        None => {
            return Err(lane_error(format!(
                "{cell}: the cell did not finish within {CELL_BOUND:?}; console:\n{}",
                guest.vm.console_tail(16_384)
            )));
        }
    };
    let console = guest.vm.console();
    let _ = guest.take_peer().power_off();
    let _ = guest.vm.wait_exit(Duration::from_secs(30))?;
    Ok((CellRun { cell: cell.to_owned(), exit_status, console }, result))
}
