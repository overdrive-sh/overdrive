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

use overdrive_core::guest_transport::{GUEST_CID_BASE, GUEST_NODE_GATEWAY};
use overdrive_core::vm::beacon::BeaconMessage;

use crate::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use crate::guest_kit::{
    GuestBoot, GuestFile, GuestImageSpec, GuestInit, GuestKit, GuestKitError,
    LOOPBACK_MODULE_GUEST_PATH,
};
use crate::vm_fixture;
use crate::vsock::{VMADDR_CID_ANY, VMADDR_PORT_ANY, VsockListener, VsockStream};

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
    let mut vm = kit.boot(&GuestBoot {
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
    let mut vm = kit.boot(&GuestBoot {
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
        if vm.try_exit()?.is_some() {
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
