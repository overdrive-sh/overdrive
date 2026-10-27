//! `overdrive-init` — the in-guest PID 1 agent (ADR-0082 §D7, §D4, GH #42).
//!
//! Nine duties, matching ADR-0082 §D4's guest half of the beacon
//! lifecycle plus the §D7 amendment (2026-08-12) that pins the
//! operator-command channel, plus the §D4 amendment (2026-08-14,
//! DISTILL DWD-21) that pins the guest vsock transport's load path:
//!
//! 0. Create the minimal root and mount procfs.
//! 1. Load the guest vsock transport modules ([`load_vsock_modules`])
//!    — a no-op on a vsock=y appliance kernel (ADR-0068 §4), required
//!    on the stock `CONFIG_VSOCKETS=m` kernel the Tier-3 fixture
//!    stages.
//! 2. Dial the host over the beacon vsock connection
//!    ([`beacon::VMADDR_CID_HOST`] / [`beacon::BEACON_VSOCK_PORT`]),
//!    retrying with a bounded backoff — the virtio-vsock PCI probe
//!    completes asynchronously after step 0's module load.
//! 3. Require and parse the platform `overdrive.transport=` token
//!    ([`transport::parse_transport_token`], DG-5).
//! 4. Configure the guest transport ([`transport::configure_guest_transport`],
//!    G-V7), then write the resolver configuration.
//! 5. Construct the guest flow owner over the setup's sockets and programs
//!    and start its `run` on its own thread (DG-5; READY is not gated on the
//!    control session, G-V3).
//! 6. Send `READY` exactly once, carrying the token's launch token. Any earlier error returns through PID 1's
//!    emergency power-off path without publishing `READY` or guest `EXIT`.
//! 6. Block for exactly one host -> guest message and require it to be
//!    `EXEC { argv }` — the operator's command arrives here, over the
//!    beacon, never on the kernel cmdline (`KernelCmdline` stays
//!    platform-only, ADR-0082 §D2).
//! 7. Exec `argv[0]` with `argv`, forwarding stdio untouched, while the
//!    PID-1 owner supervises its child-led process group.
//! 8. Send `EXIT <status>` with the command's real exit status once it
//!    resolves — never validating the operator's own I/O.
//! 9. Power off (`reboot(RB_POWER_OFF)`) immediately after the group is
//!    reaped and the direct child's status has been reported.
//!
//! # Ownership note
//!
//! The same PID-1 supervisor owns the running command and its control stream.
//! It polls for `SHUTDOWN` while reaping the direct child and adopted
//! descendants, sends one group termination sequence, and preserves the
//! direct child's status. There is no post-`EXIT` control read or second
//! supervisor.
//!
//! The real end-to-end boot (a real kernel, a real Cloud Hypervisor vsock
//! device, and a real operator command sourced from a deploy spec) is exercised
//! by the qualified Tier-3 VM tests. The host `VmDriver` owns the accepted
//! session's `EXEC` release and the guest's `READY`/`EXEC` ordering; this file
//! consumes that one `EXEC` frame and owns all post-acceptance process-group
//! supervision.
// A minimal PID 1 has no `tracing` sink, no log aggregation, and no
// operator shell — `/dev/console` (fd 0/1/2, held before devtmpfs is up
// per ADR-0082 §D7) IS the diagnostic channel. `eprintln!` on the
// emergency path is the intended mechanism, not a stopgap. Matches the
// project's own "allowed in CLI/binary-boundary crates via crate-level
// override" pattern (`.claude/rules/development.md` § Cargo.toml
// conventions on `expect_used`).
#![allow(clippy::print_stderr)]
#![deny(unsafe_code)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::kmod::{ModuleInitFlags, finit_module};
use nix::mount::{MsFlags, mount};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::reboot::{self, RebootMode};
use nix::sys::signal::{Signal, killpg};
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, VsockAddr};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use overdrive_core::vm::beacon::{self, BeaconMessage};

// netns-density-295 (vsock attachment replacement, DG-5): the guest transport
// setup (called from the pre-READY path, DG-2 row 16) and the guest flow
// owner.
mod guest_flow;
mod transport;

use std::sync::Arc;

use guest_flow::{
    GuestClock, GuestFlowOwner, GuestOwnerFatal, GuestOwnerIo, KernelGuestOwnerIo,
    MonotonicGuestClock,
};
use transport::{
    GuestTransportSetupError, GuestTransportToken, KernelGuestTransportOps,
    configure_guest_transport, parse_transport_token,
};

fn main() {
    if let Err(err) = run() {
        let mut stderr = std::io::stderr().lock();
        // DG-5: a guest transport setup failure is exactly one console line
        // naming the setup error, then the existing pre-READY exit path.
        let _ = writeln!(stderr, "{}", init_failure_console_line(&err));
        let _ = stderr.flush();
        // Best-effort emergency power-off — a guest that hangs forever
        // on agent failure is worse than one that fails loudly and
        // still exits the VM. `reboot` only returns on failure itself.
        let _ = reboot::reboot(RebootMode::RB_POWER_OFF);
        std::process::exit(1);
    }
}

/// The agent's whole lifecycle. Returns only on failure — the happy
/// path ends by powering off the guest, which (on success) never
/// returns control to this function at all.
fn run() -> Result<(), InitError> {
    let pid = std::process::id();
    complete_guest_lifecycle(
        || bootstrap_guest_root_at(Path::new("/"), mount_procfs_at),
        load_vsock_modules,
        connect_beacon,
        configure_guest_network,
        |(token, ops): (GuestTransportToken, KernelGuestTransportOps)| {
            let owner_io = KernelGuestOwnerIo::new(ops).map_err(InitError::GuestTransportSetup)?;
            let _owner_thread = start_guest_flow_owner(
                token,
                Box::new(owner_io),
                Arc::new(MonotonicGuestClock),
                report_guest_owner_fatal,
            )?;
            Ok(token)
        },
        |conn, token: GuestTransportToken| send(conn, &ready_message(pid, &token)),
        recv_exec,
        exec_operator_command,
        |conn, status| send(conn, &BeaconMessage::Exit { status }),
        || {
            reboot::reboot(RebootMode::RB_POWER_OFF).map_err(InitError::Reboot)?;
            Ok(())
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_guest_lifecycle<
    Conn,
    Setup,
    Token,
    Root,
    Modules,
    Connect,
    Network,
    StartOwner,
    Ready,
    ReceiveExec,
    Execute,
    SendExit,
    PowerOff,
>(
    root: Root,
    modules: Modules,
    connect: Connect,
    network: Network,
    start_owner: StartOwner,
    ready: Ready,
    receive_exec: ReceiveExec,
    execute: Execute,
    send_exit: SendExit,
    power_off: PowerOff,
) -> Result<(), InitError>
where
    Root: FnOnce() -> Result<(), InitError>,
    Modules: FnOnce() -> Result<(), InitError>,
    Connect: FnOnce() -> Result<Conn, InitError>,
    Network: FnOnce() -> Result<Setup, InitError>,
    StartOwner: FnOnce(Setup) -> Result<Token, InitError>,
    Ready: FnOnce(&mut Conn, Token) -> Result<(), InitError>,
    ReceiveExec: FnOnce(&Conn) -> Result<Vec<String>, InitError>,
    Execute: FnOnce(&mut Conn, &[String]) -> Result<i32, InitError>,
    SendExit: FnOnce(&mut Conn, i32) -> Result<(), InitError>,
    PowerOff: FnOnce() -> Result<(), InitError>,
{
    let mut conn = complete_pre_ready_init(root, modules, connect, network, start_owner, ready)?;
    let argv = receive_exec(&conn)?;
    let status = execute(&mut conn, &argv)?;
    send_exit(&mut conn, status)?;
    power_off()
}

/// The pre-READY path: root, modules, beacon dial, guest network (the
/// transport token is parsed and the transport configured there, DG-5), the
/// guest flow owner started on its own thread over the setup's results
/// (DG-5), then READY carrying the token the network step produced.
fn complete_pre_ready_init<Conn, Setup, Token, Root, Modules, Connect, Network, StartOwner, Ready>(
    root: Root,
    modules: Modules,
    connect: Connect,
    network: Network,
    start_owner: StartOwner,
    ready: Ready,
) -> Result<Conn, InitError>
where
    Root: FnOnce() -> Result<(), InitError>,
    Modules: FnOnce() -> Result<(), InitError>,
    Connect: FnOnce() -> Result<Conn, InitError>,
    Network: FnOnce() -> Result<Setup, InitError>,
    StartOwner: FnOnce(Setup) -> Result<Token, InitError>,
    Ready: FnOnce(&mut Conn, Token) -> Result<(), InitError>,
{
    root()?;
    modules()?;
    let mut conn = connect()?;
    let setup = network()?;
    let token = start_owner(setup)?;
    ready(&mut conn, token)?;
    Ok(conn)
}

/// DG-5: the guest flow owner is constructed after `configure_guest_transport`
/// and its `run` starts on its own thread, before READY (READY is not gated on
/// the control session, G-V3). `run` returns only its `GuestOwnerFatal`, which
/// goes to `on_fatal`.
fn start_guest_flow_owner<OnFatal>(
    token: GuestTransportToken,
    io: Box<dyn GuestOwnerIo>,
    clock: Arc<dyn GuestClock>,
    on_fatal: OnFatal,
) -> Result<std::thread::JoinHandle<()>, InitError>
where
    OnFatal: FnOnce(GuestOwnerFatal) + Send + 'static,
{
    std::thread::Builder::new()
        .name("guest-flow-owner".to_owned())
        .spawn(move || on_fatal(GuestFlowOwner::new(token, io, clock).run()))
        .map_err(|source| InitError::GuestNetworkIo {
            operation: "spawn the guest flow owner thread",
            source,
        })
}

/// DG-5: a guest flow owner failure is one console line, then the existing
/// guest-exit path (the host sees the VMM exit).
#[allow(clippy::needless_pass_by_value, reason = "the owner thread hands its fatal over by value")]
fn report_guest_owner_fatal(fatal: GuestOwnerFatal) {
    eprintln!("{}", guest_owner_fatal_console_line(&fatal));
    let _ = reboot::reboot(RebootMode::RB_POWER_OFF);
}

/// The one console line of a guest flow owner failure (DG-5).
fn guest_owner_fatal_console_line(fatal: &GuestOwnerFatal) -> String {
    format!("overdrive-init: guest flow owner failed: {fatal}")
}

/// The one console line `main` writes for a failed lifecycle: a guest
/// transport setup failure names the setup error (DG-5); every other failure
/// keeps the existing fatal line.
fn init_failure_console_line(err: &InitError) -> String {
    match err {
        InitError::GuestTransportSetup(source) => {
            format!("overdrive-init: guest transport setup failed: {source}")
        }
        other => format!("overdrive-init: fatal: {other}"),
    }
}

/// READY with the launch token of the guest's `overdrive.transport=` token
/// (ATTR-TOKEN, DG-7).
const fn ready_message(pid: u32, token: &GuestTransportToken) -> BeaconMessage {
    BeaconMessage::Ready { pid, port: token.ports.beacon, launch: token.launch }
}

fn ensure_guest_directories_at(root: &Path) -> Result<(), InitError> {
    for relative in ["proc", "etc"] {
        let path = root.join(relative);
        fs::create_dir_all(&path).map_err(|source| InitError::GuestDirectory { path, source })?;
    }
    Ok(())
}

fn bootstrap_guest_root_at<MountProc>(root: &Path, mount_procfs: MountProc) -> Result<(), InitError>
where
    MountProc: FnOnce(&Path) -> Result<(), InitError>,
{
    ensure_guest_directories_at(root)?;
    mount_procfs(&root.join("proc"))
}

/// `overdrive-init`'s typed failure surface. Distinct failure modes get
/// distinct variants (`.claude/rules/development.md` § "Errors") so
/// `main`'s emergency power-off path can log the real cause on
/// `/dev/console` rather than a flattened string.
#[derive(Debug, thiserror::Error)]
enum InitError {
    /// A staged vsock module file exists but could not be opened for a
    /// reason other than absence (permission, I/O, ...). Absence itself
    /// is NOT an error — see [`load_vsock_modules`] — this variant is
    /// every other read failure (ADR-0082 §D4 amendment 2026-08-14).
    #[error("could not open staged vsock module {path}: {source}")]
    ModuleOpen {
        /// The in-guest module path that could not be opened.
        path: String,
        /// The underlying open failure.
        #[source]
        source: std::io::Error,
    },
    /// `finit_module(2)` itself failed for a reason other than "already
    /// loaded" (`EEXIST`, tolerated as success — either a prior module
    /// in this boot already loaded it, or the appliance kernel carries
    /// it built in).
    #[error("finit_module({path}) failed: {source}")]
    ModuleLoad {
        /// The in-guest module path whose load failed.
        path: String,
        /// The underlying `finit_module(2)` failure.
        #[source]
        source: Errno,
    },
    /// Creating the `AF_VSOCK` beacon socket failed.
    #[error("could not create the beacon vsock socket: {0}")]
    Socket(#[source] Errno),
    /// Connecting the beacon socket to the host failed after
    /// [`VSOCK_CONNECT_MAX_ATTEMPTS`] retries — the virtio-vsock PCI
    /// probe completes asynchronously after [`load_vsock_modules`]
    /// returns, so a single attempt would race it (ADR-0082 §D4
    /// amendment 2026-08-14). Carries the LAST attempt's failure.
    #[error("could not connect the beacon vsock socket to the host after repeated retries: {0}")]
    Connect(#[source] Errno),
    /// A read or write on the beacon connection failed.
    #[error("beacon connection I/O failed: {0}")]
    Io(#[source] std::io::Error),
    /// Creating one of PID 1's required minimal-root directories failed.
    #[error("could not create required guest directory {path}: {source}")]
    GuestDirectory {
        /// The absolute directory path that could not be created.
        path: PathBuf,
        /// The underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// Mounting procfs for PID 1's kernel-cmdline input failed.
    #[error("could not mount procfs at {target}: {source}")]
    ProcMount {
        /// The procfs mountpoint.
        target: PathBuf,
        /// The underlying `mount(2)` failure.
        #[source]
        source: Errno,
    },
    /// Reading the kernel command line or writing the resolver
    /// configuration failed.
    #[error("guest network {operation} failed: {source}")]
    GuestNetworkIo {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    /// The `overdrive.transport=` token is absent or malformed, or a guest
    /// transport setup step failed (G-V7, DG-5).
    #[error("guest transport setup failed: {0}")]
    GuestTransportSetup(#[source] GuestTransportSetupError),
    /// The beacon connection closed (EOF) before the host sent `EXEC` —
    /// distinct from a parse failure: nothing arrived on the connection
    /// at all, so the guest has no command to run.
    #[error("beacon connection closed before the host sent EXEC")]
    NoExecReceived,
    /// The host's first message parsed as a valid `BeaconMessage`, but
    /// was not `EXEC` (ADR-0082 §D7 amendment: `EXEC` must be the first
    /// host -> guest message, before the guest execs anything).
    #[error("expected EXEC as the host's first message, received {0:?} instead")]
    UnexpectedBeaconMessage(BeaconMessage),
    /// The host's first message failed to parse as a beacon message at
    /// all (malformed line, empty argv, non-JSON payload, ...).
    #[error("could not parse the host's first message: {0}")]
    BeaconParse(#[from] beacon::BeaconParseError),
    /// Spawning the operator's command failed. Names the attempted
    /// command (`argv[0]`) so the guest's `/dev/console` diagnostic
    /// never reduces to a bare `No such file or directory` with nothing
    /// to attribute it to.
    #[error("could not spawn the operator's command {command:?}: {source}")]
    Spawn {
        /// The command that failed to spawn (`argv[0]`).
        command: String,
        /// The underlying spawn failure.
        #[source]
        source: std::io::Error,
    },
    /// A wait/reap syscall failed after EXEC was accepted.
    #[error("waitpid failed: {0}")]
    Wait(#[source] Errno),
    /// A process-group probe or signal failed after EXEC was accepted.
    #[error("process group {operation} failed for pgid {pgid}: {source}")]
    ProcessGroup {
        operation: &'static str,
        pgid: i32,
        #[source]
        source: Errno,
    },
    /// `reboot(RB_POWER_OFF)` failed.
    #[error("reboot(RB_POWER_OFF) failed: {0}")]
    Reboot(#[source] Errno),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreReadyErrorClass {
    ModuleOpen,
    ModuleLoad,
    Socket,
    Connect,
    BeaconIo,
    GuestDirectory,
    ProcMount,
    GuestNetworkIo,
    GuestTransportSetup,
}

#[cfg(test)]
const fn pre_ready_error_class(error: &InitError) -> Option<PreReadyErrorClass> {
    match error {
        InitError::ModuleOpen { .. } => Some(PreReadyErrorClass::ModuleOpen),
        InitError::ModuleLoad { .. } => Some(PreReadyErrorClass::ModuleLoad),
        InitError::Socket(_) => Some(PreReadyErrorClass::Socket),
        InitError::Connect(_) => Some(PreReadyErrorClass::Connect),
        InitError::Io(_) => Some(PreReadyErrorClass::BeaconIo),
        InitError::GuestDirectory { .. } => Some(PreReadyErrorClass::GuestDirectory),
        InitError::ProcMount { .. } => Some(PreReadyErrorClass::ProcMount),
        InitError::GuestNetworkIo { .. } => Some(PreReadyErrorClass::GuestNetworkIo),
        InitError::GuestTransportSetup(_) => Some(PreReadyErrorClass::GuestTransportSetup),
        InitError::NoExecReceived
        | InitError::UnexpectedBeaconMessage(_)
        | InitError::BeaconParse(_)
        | InitError::Spawn { .. }
        | InitError::Wait(_)
        | InitError::ProcessGroup { .. }
        | InitError::Reboot(_) => None,
    }
}

/// The guest network step of the pre-READY path (DG-2 row 16, DG-5): read
/// `/proc/cmdline`, parse the one `overdrive.transport=` token, configure the
/// guest transport, then write the resolver configuration (`nameserver
/// <gateway>`). Returns the parsed token, whose launch token READY carries,
/// and the setup's sockets and programs, which the guest flow owner takes.
fn configure_guest_network() -> Result<(GuestTransportToken, KernelGuestTransportOps), InitError> {
    configure_guest_network_with(
        || fs::read("/proc/cmdline"),
        |token| {
            let mut ops = KernelGuestTransportOps::new();
            configure_guest_transport(token, &mut ops)?;
            Ok(ops)
        },
        write_resolver,
    )
}

fn configure_guest_network_with<Read, Setup, Ops, Resolver>(
    read_cmdline: Read,
    setup: Setup,
    write_resolver: Resolver,
) -> Result<(GuestTransportToken, Ops), InitError>
where
    Read: FnOnce() -> std::io::Result<Vec<u8>>,
    Setup: FnOnce(&GuestTransportToken) -> Result<Ops, GuestTransportSetupError>,
    Resolver: FnOnce(Ipv4Addr) -> Result<(), InitError>,
{
    let cmdline = read_cmdline()
        .map_err(|source| InitError::GuestNetworkIo { operation: "read /proc/cmdline", source })?;
    let cmdline = std::str::from_utf8(&cmdline).map_err(|source| InitError::GuestNetworkIo {
        operation: "decode /proc/cmdline as UTF-8",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })?;
    let token = parse_transport_token(cmdline).map_err(InitError::GuestTransportSetup)?;
    let ops = setup(&token).map_err(InitError::GuestTransportSetup)?;
    write_resolver(token.gateway)?;
    Ok((token, ops))
}

fn write_resolver(dns: Ipv4Addr) -> Result<(), InitError> {
    fs::write("/etc/resolv.conf", format!("nameserver {dns}\n"))
        .map_err(|source| InitError::GuestNetworkIo { operation: "write /etc/resolv.conf", source })
}

fn mount_procfs_at(target: &Path) -> Result<(), InitError> {
    let flags = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    match mount(Some("proc"), target, Some("proc"), flags, None::<&str>) {
        Ok(()) | Err(Errno::EBUSY) => Ok(()),
        Err(source) => Err(InitError::ProcMount { target: target.to_path_buf(), source }),
    }
}

/// The in-guest module directory + ordered filename list — see
/// [`beacon::GUEST_VSOCK_MODULE_DIR`] / [`beacon::GUEST_VSOCK_MODULE_FILES`]'s
/// own docs for the shared-const contract with the staging fixture.
/// Loads the guest vsock transport before [`connect_beacon`] dials
/// `AF_VSOCK` (ADR-0082 §D4 amendment 2026-08-14, DISTILL DWD-21) —
/// matches the spike's proven-12/12 mechanism
/// (`spike-scratch/increment-a/probe/src/bin/guest_init.rs`).
///
/// Two tolerated outcomes, both success, so this ONE binary is correct
/// on both the stock `CONFIG_VSOCKETS=m` test kernel and a vsock=y
/// appliance kernel (ADR-0068 §4) with no `#[cfg(test)]` branch and no
/// test-only parameter (kernel-config-variance resilience, the
/// documented `[D2]` fallback — never "production shaped by
/// simulation", `.claude/rules/development.md`):
///
/// - **The staged file is absent** (`io::ErrorKind::NotFound`) — a
///   vsock=y image stages none of these, so there is nothing to load;
///   skip to the next filename.
/// - **`finit_module` returns `EEXIST`** — the module is already
///   loaded (a dependency loaded transitively, or built into the
///   kernel); already-satisfied, not a failure.
///
/// A genuine failure on either the open or the load surfaces as a
/// typed [`InitError`] — logged to `/dev/console` by `main`'s existing
/// emergency path, never swallowed.
fn load_vsock_modules() -> Result<(), InitError> {
    load_vsock_modules_with(
        |path| match File::open(path) {
            Ok(file) => Ok(Some(file)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(source),
        },
        |file| finit_module(file, c"", ModuleInitFlags::empty()),
    )
}

fn load_vsock_modules_with<Handle, Open, Load>(
    mut open: Open,
    mut load: Load,
) -> Result<(), InitError>
where
    Open: FnMut(&str) -> std::io::Result<Option<Handle>>,
    Load: FnMut(&Handle) -> Result<(), Errno>,
{
    for filename in beacon::GUEST_VSOCK_MODULE_FILES {
        let path = format!("{}/{filename}", beacon::GUEST_VSOCK_MODULE_DIR);
        let file = match open(&path) {
            Ok(Some(file)) => file,
            Ok(None) => continue,
            Err(source) => return Err(InitError::ModuleOpen { path, source }),
        };
        match load(&file) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(source) => return Err(InitError::ModuleLoad { path, source }),
        }
    }
    Ok(())
}

/// Bounded retry budget for [`connect_beacon`] — matches the spike's
/// proven-12/12 shape (`probe/src/bin/guest_init.rs`): the
/// virtio-vsock PCI probe completes ASYNCHRONOUSLY after
/// [`load_vsock_modules`] returns, so a single connect attempt races
/// it. Harmless on a vsock=y kernel (the first attempt succeeds);
/// required on the stock `CONFIG_VSOCKETS=m` test kernel.
const VSOCK_CONNECT_MAX_ATTEMPTS: u32 = 25;

/// Delay between [`connect_beacon`] retry attempts — matches the
/// spike's measured `sleep_ms(100)`.
const VSOCK_CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Opens the guest -> host beacon connection: `AF_VSOCK`, dialing
/// [`beacon::VMADDR_CID_HOST`] on [`beacon::BEACON_VSOCK_PORT`]
/// (ADR-0082 §§D2.2/D7), retrying up to [`VSOCK_CONNECT_MAX_ATTEMPTS`]
/// times with a [`VSOCK_CONNECT_RETRY_DELAY`] pause between attempts
/// (ADR-0082 §D4 amendment 2026-08-14) — see [`connect_beacon_once`]
/// for the single-attempt body this loop retries.
fn connect_beacon() -> Result<File, InitError> {
    connect_beacon_with(connect_beacon_once, || std::thread::sleep(VSOCK_CONNECT_RETRY_DELAY))
}

fn connect_beacon_with<Conn, Attempt, Pause>(
    mut attempt: Attempt,
    mut pause: Pause,
) -> Result<Conn, InitError>
where
    Attempt: FnMut() -> Result<Conn, InitError>,
    Pause: FnMut(),
{
    let mut last_err = Errno::UnknownErrno;
    for _attempt in 0..VSOCK_CONNECT_MAX_ATTEMPTS {
        match attempt() {
            Ok(conn) => return Ok(conn),
            Err(InitError::Socket(errno) | InitError::Connect(errno)) => {
                last_err = errno;
                pause();
            }
            // `connect_beacon_once` only ever returns `Socket` or
            // `Connect` — every other `InitError` variant is
            // unreachable from this call site.
            Err(other) => return Err(other),
        }
    }
    Err(InitError::Connect(last_err))
}

/// One `AF_VSOCK` socket-then-connect attempt — the body
/// [`connect_beacon`]'s retry loop repeats. Returns only
/// [`InitError::Socket`] or [`InitError::Connect`] on failure.
fn connect_beacon_once() -> Result<File, InitError> {
    let sock = connect_beacon_once_with(
        || socket::socket(AddressFamily::Vsock, SockType::Stream, SockFlag::empty(), None),
        |sock| {
            let addr = VsockAddr::new(beacon::VMADDR_CID_HOST, beacon::BEACON_VSOCK_PORT.as_u32());
            socket::connect(sock.as_raw_fd(), &addr)
        },
    )?;
    // `OwnedFd -> File` is a safe conversion (io-safety: the `OwnedFd`
    // already guarantees exclusive ownership of a valid fd). Wrapping
    // the raw vsock fd in `File` gives ordinary `Read`/`Write` access —
    // the fd's socket family is irrelevant to the `read`/`write`
    // syscalls `File` issues.
    Ok(File::from(sock))
}

fn connect_beacon_once_with<Socket, Create, Connect>(
    create: Create,
    connect: Connect,
) -> Result<Socket, InitError>
where
    Create: FnOnce() -> Result<Socket, Errno>,
    Connect: FnOnce(&Socket) -> Result<(), Errno>,
{
    let socket = create().map_err(InitError::Socket)?;
    connect(&socket).map_err(InitError::Connect)?;
    Ok(socket)
}

/// Writes one beacon message as a single wire line (`Display` + `\n` —
/// the Published Language's own [`BeaconMessage`] never embeds the
/// terminator; the transport owns line framing, per `vm::beacon`'s
/// module doc).
fn send(conn: &mut File, msg: &BeaconMessage) -> Result<(), InitError> {
    let line = format!("{msg}\n");
    conn.write_all(line.as_bytes()).map_err(InitError::Io)?;
    match msg {
        BeaconMessage::Ready { .. } => eprintln!("overdrive-init: beacon boundary tx READY"),
        BeaconMessage::Exit { .. } => eprintln!("overdrive-init: beacon boundary tx EXIT"),
        _ => {}
    }
    Ok(())
}

/// Reads exactly one line from the shared beacon stream without buffering
/// past its newline. This is intentionally byte-oriented: a buffered reader
/// would consume a coalesced SHUTDOWN/EXEC frame and lose it at the handoff
/// from `recv_exec` to the sole execution owner.
fn read_one_line(conn: &File) -> Result<Option<String>, InitError> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match (&*conn).read(&mut byte).map_err(InitError::Io)? {
            0 if bytes.is_empty() => return Ok(None),
            0 => return Err(unexpected_eof()),
            1 => {
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
            _ => unreachable!("one-byte read cannot return more than one byte"),
        }
    }
}

fn unexpected_eof() -> InitError {
    InitError::Io(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "beacon connection closed during a frame",
    ))
}

/// Blocks for exactly one host -> guest message and requires it to be
/// `EXEC` (ADR-0082 §D7 amendment, "Candidate A": `READY` is sent before
/// this call; the host writes `EXEC` on the just-accepted session
/// before the guest execs anything). Distinguishes "the host never sent
/// anything" ([`InitError::NoExecReceived`]) from "the host sent
/// something, but it didn't parse" ([`InitError::BeaconParse`]) from
/// "it parsed, but wasn't EXEC" ([`InitError::UnexpectedBeaconMessage`])
/// so a downstream spawn failure is never misdiagnosed as a missing
/// command (`.claude/rules/development.md` § "Errors" — distinct
/// failure modes get distinct variants).
fn recv_exec(conn: &File) -> Result<Vec<String>, InitError> {
    let line = read_one_line(conn)?.ok_or(InitError::NoExecReceived)?;
    match line.parse::<BeaconMessage>()? {
        BeaconMessage::Exec { argv } => {
            eprintln!("overdrive-init: beacon boundary rx EXEC");
            Ok(argv)
        }
        other => Err(InitError::UnexpectedBeaconMessage(other)),
    }
}

/// Execs the operator's command (`argv`, received via `EXEC` —
/// [`recv_exec`]), forwarding stdio untouched (the default
/// `std::process::Command` behaviour — the guest kernel opens
/// `/dev/console` as fd 0/1/2 for init before devtmpfs is up, ADR-0082
/// §D7, so inheriting IS forwarding), and returns the guest's own
/// encoding of the real exit status. Never inspects or validates the
/// operator's own stdout/stderr/exit code beyond encoding it onto the
/// wire.
fn exec_operator_command(conn: &mut File, argv: &[String]) -> Result<i32, InitError> {
    // `argv` is never empty here: the only producer of this value is
    // `recv_exec`, which only returns `Ok` after `BeaconMessage::from_str`
    // has already rejected an empty argv as `BeaconParseError::EmptyArgv`
    // (`.claude/rules/development.md` § "Logically unreachable None/Err").
    let command = argv.first().unwrap_or_else(|| {
        unreachable!(
            "recv_exec only returns argv already validated non-empty by BeaconMessage::from_str"
        )
    });
    let mut child_command = Command::new(command);
    child_command.args(&argv[1..]).process_group(0);
    let mut child = child_command
        .spawn()
        .map_err(|source| InitError::Spawn { command: command.clone(), source })?;
    let child_pid =
        Pid::from_raw(i32::try_from(child.id()).map_err(|_| InitError::Wait(Errno::EOVERFLOW))?);
    supervise_command(conn, &mut child, child_pid)
}

const GUEST_STOP_GRACE: Duration = Duration::from_secs(5);
const GUEST_SUPERVISION_POLL: Duration = Duration::from_millis(10);

fn supervise_command(
    conn: &mut File,
    _child: &mut std::process::Child,
    child_pid: Pid,
) -> Result<i32, InitError> {
    let supervisor_started = Instant::now();
    let mut direct_status = None;
    let mut control_error = None;
    let mut control_frame = Vec::new();
    let mut control_closed = false;
    let mut shutdown_requested = false;
    let mut shutdown_deadline = None;
    let mut sigkill_sent = false;

    std::thread::sleep(GUEST_SUPERVISION_POLL);

    loop {
        reap_children(child_pid, &mut direct_status)?;

        let group_live = process_group_live(child_pid)?;
        if direct_status.is_some() && !group_live {
            break;
        }

        if let Some(deadline) = shutdown_deadline
            && !sigkill_sent
            && Instant::now() >= deadline
        {
            signal_group(child_pid, Signal::SIGKILL, "SIGKILL")?;
            sigkill_sent = true;
        }

        if direct_status.is_some() && group_live && shutdown_deadline.is_none() {
            begin_group_termination(child_pid, &mut shutdown_deadline, &mut control_error);
        }

        if control_error.is_none() && !control_closed {
            let ready = poll_control(conn)?;
            if ready {
                let mut byte = [0_u8; 1];
                match conn.read(&mut byte).map_err(InitError::Io)? {
                    0 => {
                        control_closed = true;
                        // The production writer closes its host->guest half
                        // after a successful SHUTDOWN submission. That EOF
                        // is not a second stream fault; the bounded group
                        // teardown already owns completion.
                        if !shutdown_requested {
                            control_error = Some(unexpected_eof());
                        }
                        begin_group_termination(
                            child_pid,
                            &mut shutdown_deadline,
                            &mut control_error,
                        );
                    }
                    1 => {
                        control_frame.push(byte[0]);
                        if byte[0] == b'\n' {
                            let line = String::from_utf8_lossy(&control_frame).into_owned();
                            control_frame.clear();
                            handle_control_line(
                                &line,
                                child_pid,
                                supervisor_started,
                                &mut shutdown_requested,
                                &mut shutdown_deadline,
                                &mut control_error,
                            );
                        }
                    }
                    _ => unreachable!("one-byte read cannot return more than one byte"),
                }
            }
        } else {
            // `poll_control` normally supplies the supervisor's bounded wait.
            // Once EOF or a typed control error disables that poll, keep the
            // same interval between nonblocking waitpid/killpg probes so the
            // remaining guest grace does not spin a full vCPU.
            std::thread::sleep(GUEST_SUPERVISION_POLL);
        }
    }

    guest_diagnostic("group-complete", supervisor_started);
    // Both the successful EXIT path and main's existing fatal path request
    // poweroff immediately after this supervisor returns. Record the same
    // boundary here so the diagnostic remains available on either outcome.
    guest_diagnostic("poweroff-requested", supervisor_started);
    let status = direct_status.map_or_else(
        || unreachable!("group completion requires direct status"),
        exit_status_to_wire,
    );
    if let Some(error) = control_error {
        return Err(error);
    }
    Ok(status)
}

fn handle_control_line(
    line: &str,
    child_pid: Pid,
    supervisor_started: Instant,
    shutdown_requested: &mut bool,
    shutdown_deadline: &mut Option<Instant>,
    control_error: &mut Option<InitError>,
) {
    match line.parse::<BeaconMessage>() {
        Ok(BeaconMessage::Shutdown) => {
            *shutdown_requested = true;
            if shutdown_deadline.is_none() {
                guest_diagnostic("shutdown-received", supervisor_started);
            }
        }
        Ok(message) => {
            *control_error = Some(InitError::UnexpectedBeaconMessage(message));
        }
        Err(error) => {
            *control_error = Some(InitError::BeaconParse(error));
        }
    }
    begin_group_termination(child_pid, shutdown_deadline, control_error);
}

fn guest_diagnostic(stage: &str, supervisor_started: Instant) {
    eprintln!("overdrive-init: {stage} elapsed_ms={}", supervisor_started.elapsed().as_millis());
}

fn poll_control(conn: &File) -> Result<bool, InitError> {
    let timeout = PollTimeout::try_from(GUEST_SUPERVISION_POLL).unwrap_or(PollTimeout::MAX);
    loop {
        let mut descriptors = [PollFd::new(
            conn.as_fd(),
            PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR,
        )];
        match poll(&mut descriptors, timeout) {
            Ok(ready) => {
                return Ok(
                    ready != 0 && descriptors[0].revents().is_some_and(|flags| !flags.is_empty())
                );
            }
            Err(Errno::EINTR) => {}
            Err(source) => return Err(InitError::Wait(source)),
        }
    }
}

fn reap_children(
    child_pid: Pid,
    direct_status: &mut Option<std::process::ExitStatus>,
) -> Result<(), InitError> {
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => return Ok(()),
            Ok(WaitStatus::Exited(pid, code)) if pid == child_pid => {
                *direct_status = Some(std::process::ExitStatus::from_raw(code << 8));
            }
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == child_pid => {
                *direct_status = Some(std::process::ExitStatus::from_raw(signal as i32));
            }
            Ok(
                WaitStatus::Exited(_, _)
                | WaitStatus::Signaled(_, _, _)
                | WaitStatus::Stopped(_, _)
                | WaitStatus::Continued(_),
            ) => {}
            #[cfg(target_os = "linux")]
            Ok(WaitStatus::PtraceEvent(_, _, _) | WaitStatus::PtraceSyscall(_)) => {}
            Err(Errno::EINTR) => {}
            Err(Errno::ECHILD) if direct_status.is_some() => return Ok(()),
            Err(source) => return Err(InitError::Wait(source)),
        }
    }
}

fn process_group_live(child_pid: Pid) -> Result<bool, InitError> {
    match killpg(child_pid, None) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(Errno::EINTR) => process_group_live(child_pid),
        Err(source) => Err(InitError::ProcessGroup {
            operation: "probe process group",
            pgid: child_pid.as_raw(),
            source,
        }),
    }
}

fn signal_group(child_pid: Pid, signal: Signal, operation: &'static str) -> Result<(), InitError> {
    loop {
        match killpg(child_pid, signal) {
            Ok(()) | Err(Errno::ESRCH) => return Ok(()),
            Err(Errno::EINTR) => {}
            Err(source) => {
                return Err(InitError::ProcessGroup {
                    operation,
                    pgid: child_pid.as_raw(),
                    source,
                });
            }
        }
    }
}

fn begin_group_termination(
    child_pid: Pid,
    shutdown_deadline: &mut Option<Instant>,
    control_error: &mut Option<InitError>,
) {
    if shutdown_deadline.is_some() {
        return;
    }
    *shutdown_deadline = Some(Instant::now() + GUEST_STOP_GRACE);
    if let Err(error) = signal_group(child_pid, Signal::SIGTERM, "SIGTERM")
        && control_error.is_none()
    {
        *control_error = Some(error);
    }
}

/// Encodes a real `std::process::ExitStatus` onto the beacon's signed
/// decimal integer field. Normal exit -> the real exit code
/// (`WEXITSTATUS`). Signal termination -> `128 + signal`, the common
/// shell convention — a guest-local encoding choice; the wire format
/// only pins "a signed decimal integer" (ADR-0082 §D7); `overdrive-init`
/// owns how it is computed.
fn exit_status_to_wire(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    if let Some(signal) = status.signal() {
        return 128 + signal;
    }
    // Unreachable on Unix in practice — a process always either exits
    // or is signalled — but the API surface allows it, so name the
    // fallback rather than panic.
    -1
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        clippy::ignored_unit_patterns,
        clippy::unwrap_used
    )]

    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct ScratchRoot(std::path::PathBuf);

    impl ScratchRoot {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("overdrive-init-{label}-{}-{sequence}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// CONTRACT_SHAPE: bounded-change (empty minimal root gains only required bootstrap directories).
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn minimal_guest_root_bootstrap_creates_proc_and_etc_preconditions() {
        let root = ScratchRoot::new("minimal-root");
        let mounted_procfs = Cell::new(false);

        bootstrap_guest_root_at(root.path(), |target| {
            assert_eq!(target, root.path().join("proc"));
            mounted_procfs.set(true);
            Ok(())
        })
        .unwrap();

        let entries = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            entries,
            std::collections::BTreeSet::from(["etc".to_owned(), "proc".to_owned()]),
            "bootstrap may add exactly the declared proc and etc directories",
        );
        assert!(root.path().join("proc").is_dir(), "proc must be a directory");
        assert!(root.path().join("etc").is_dir(), "etc must be a directory");
        assert!(mounted_procfs.get(), "PID 1 must mount procfs after creating its mountpoint");
    }

    /// CONTRACT_SHAPE: bounded-change (directory failure prevents proc mount and all later init).
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn minimal_root_directory_failure_is_typed_and_suppresses_all_later_init() {
        let root = ScratchRoot::new("not-a-directory");
        let blocker = root.path().join("proc");
        fs::write(&blocker, b"blocks create_dir_all").unwrap();
        let mounted = Cell::new(false);

        let (result, trace) = lifecycle_trace(
            || {
                bootstrap_guest_root_at(root.path(), |_| {
                    mounted.set(true);
                    Ok(())
                })
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
        );

        assert!(matches!(result, Err(InitError::GuestDirectory { .. })));
        assert_eq!(trace, [PreReadyStage::Root]);
        assert!(!mounted.get(), "proc mount must be suppressed after root creation failure");
    }

    /// CONTRACT_SHAPE: bounded-change (proc mount failure remains typed and pre-READY).
    #[test]
    fn minimal_root_proc_mount_failure_is_typed_and_suppresses_all_later_init() {
        let root = ScratchRoot::new("proc-mount-failure");
        let (result, trace) = lifecycle_trace(
            || {
                bootstrap_guest_root_at(root.path(), |target| {
                    Err(InitError::ProcMount { target: target.to_path_buf(), source: Errno::EPERM })
                })
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::ProcMount { .. })));
        assert_eq!(trace, [PreReadyStage::Root]);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PreReadyStage {
        Root,
        Modules,
        Connect,
        Network,
        Ready,
        Exec,
        Operator,
        Exit,
        PowerOff,
    }

    fn lifecycle_trace<Root, Modules, Connect, Network, Ready>(
        root: Root,
        modules: Modules,
        connect: Connect,
        network: Network,
        ready: Ready,
    ) -> (Result<(), InitError>, Vec<PreReadyStage>)
    where
        Root: FnOnce() -> Result<(), InitError>,
        Modules: FnOnce() -> Result<(), InitError>,
        Connect: FnOnce() -> Result<(), InitError>,
        Network: FnOnce() -> Result<(), InitError>,
        Ready: FnOnce() -> Result<(), InitError>,
    {
        use std::cell::RefCell;
        let trace = RefCell::new(Vec::new());
        let result = complete_guest_lifecycle(
            || {
                trace.borrow_mut().push(PreReadyStage::Root);
                root()
            },
            || {
                trace.borrow_mut().push(PreReadyStage::Modules);
                modules()
            },
            || {
                trace.borrow_mut().push(PreReadyStage::Connect);
                connect()?;
                Ok(())
            },
            || {
                trace.borrow_mut().push(PreReadyStage::Network);
                network()
            },
            Ok,
            |_, ()| {
                trace.borrow_mut().push(PreReadyStage::Ready);
                ready()
            },
            |_| {
                trace.borrow_mut().push(PreReadyStage::Exec);
                Ok(vec!["/bin/true".to_owned()])
            },
            |_, _| {
                trace.borrow_mut().push(PreReadyStage::Operator);
                Ok(0)
            },
            |_, _| {
                trace.borrow_mut().push(PreReadyStage::Exit);
                Ok(())
            },
            || {
                trace.borrow_mut().push(PreReadyStage::PowerOff);
                Ok(())
            },
        );
        (result, trace.into_inner())
    }

    /// CONTRACT_SHAPE: pure-function.
    /// S-VLL-09: once EXEC completes, preserve READY/EXEC/EXIT ordering and
    /// request poweroff immediately; no second control read is a prerequisite.
    #[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
    #[test]
    fn completed_command_powers_off_without_waiting_for_shutdown() {
        let (result, trace) =
            lifecycle_trace(|| Ok(()), || Ok(()), || Ok(()), || Ok(()), || Ok(()));
        assert!(result.is_ok(), "the existing complete lifecycle must succeed");
        assert_eq!(
            trace,
            [
                PreReadyStage::Root,
                PreReadyStage::Modules,
                PreReadyStage::Connect,
                PreReadyStage::Network,
                PreReadyStage::Ready,
                PreReadyStage::Exec,
                PreReadyStage::Operator,
                PreReadyStage::Exit,
                PreReadyStage::PowerOff
            ],
            "S-VLL-09: completed child must not wait for post-EXIT SHUTDOWN",
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    /// S-VLL-10b: the production File-based lifecycle boundary preserves the
    /// exact typed pre-EXEC control-stream error and starts no command.
    #[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
    #[test]
    fn pre_exec_control_stream_faults_preserve_the_exact_typed_error_at_file_boundary() {
        use std::cell::RefCell;

        fn drive(frame: &[u8], label: &str) -> (InitError, Vec<PreReadyStage>) {
            let root = ScratchRoot::new(label);
            let control_stream = root.path().join("control-stream");
            fs::write(&control_stream, frame).unwrap();

            let trace = RefCell::new(Vec::new());
            let result = complete_guest_lifecycle(
                || {
                    trace.borrow_mut().push(PreReadyStage::Root);
                    Ok(())
                },
                || {
                    trace.borrow_mut().push(PreReadyStage::Modules);
                    Ok(())
                },
                || {
                    trace.borrow_mut().push(PreReadyStage::Connect);
                    File::open(&control_stream).map_err(InitError::Io)
                },
                || {
                    trace.borrow_mut().push(PreReadyStage::Network);
                    Ok(())
                },
                Ok,
                |_, ()| {
                    trace.borrow_mut().push(PreReadyStage::Ready);
                    Ok(())
                },
                |conn| {
                    trace.borrow_mut().push(PreReadyStage::Exec);
                    recv_exec(conn)
                },
                |_, _| {
                    trace.borrow_mut().push(PreReadyStage::Operator);
                    Ok(0)
                },
                |_, _| {
                    trace.borrow_mut().push(PreReadyStage::Exit);
                    Ok(())
                },
                || {
                    trace.borrow_mut().push(PreReadyStage::PowerOff);
                    Ok(())
                },
            );

            (
                result.expect_err("the injected pre-EXEC stream fault must survive"),
                trace.into_inner(),
            )
        }

        let expected_trace = [
            PreReadyStage::Root,
            PreReadyStage::Modules,
            PreReadyStage::Connect,
            PreReadyStage::Network,
            PreReadyStage::Ready,
            PreReadyStage::Exec,
        ];

        let (eof, trace) = drive(b"", "pre-exec-eof");
        assert!(matches!(eof, InitError::NoExecReceived));
        assert_eq!(trace, expected_trace);

        let (malformed, trace) = drive(b"EXEC not-json\n", "pre-exec-malformed");
        assert!(matches!(
            malformed,
            InitError::BeaconParse(beacon::BeaconParseError::MalformedArgv { ref raw, .. })
                if raw == "EXEC not-json"
        ));
        assert_eq!(trace, expected_trace);

        let (unexpected, trace) = drive(b"SHUTDOWN\n", "pre-exec-unexpected");
        assert!(matches!(unexpected, InitError::UnexpectedBeaconMessage(BeaconMessage::Shutdown)));
        assert_eq!(trace, expected_trace);
    }

    /// CONTRACT_SHAPE: bounded-change.
    /// S-VLL-10b: after EXEC starts a real direct child, EOF, malformed
    /// control and duplicate EXEC retain three distinct typed causes at the
    /// approved private File/process boundary. Native S10b separately proves
    /// group teardown, reaping, poweroff and the empty EXIT complement.
    #[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
    #[cfg(feature = "integration-tests")]
    #[test]
    fn post_exec_control_stream_faults_preserve_exact_typed_errors_at_file_process_boundary() {
        fn drive(frame: Option<&[u8]>, label: &str) -> InitError {
            use std::net::Shutdown;
            use std::os::fd::OwnedFd;
            use std::os::unix::net::UnixStream;

            let (reader, mut writer) = UnixStream::pair().expect("create private control stream");
            if let Some(frame) = frame {
                writer.write_all(frame).expect("write post-EXEC control frame");
            } else {
                writer.shutdown(Shutdown::Write).expect("close post-EXEC control stream");
            }
            let reader: OwnedFd = reader.into();
            let mut conn = File::from(reader);
            let command = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "trap 'exit 41' TERM; while :; do /bin/sleep 1; done".to_owned(),
            ];
            let result = exec_operator_command(&mut conn, &command);
            drop(writer);
            result.expect_err(label)
        }

        let eof = drive(None, "EOF after EXEC must remain typed");
        assert!(
            matches!(eof, InitError::Io(ref source)
                if source.kind() == std::io::ErrorKind::UnexpectedEof),
            "EOF after EXEC must retain InitError::Io(UnexpectedEof), got {eof:?}"
        );

        let malformed =
            drive(Some(b"not-a-beacon-frame\n"), "malformed control after EXEC must remain typed");
        assert!(
            matches!(
                malformed,
                InitError::BeaconParse(beacon::BeaconParseError::UnknownKind {
                    ref kind,
                    ref raw,
                }) if kind == "not-a-beacon-frame" && raw == "not-a-beacon-frame"
            ),
            "malformed control after EXEC must retain InitError::BeaconParse, got {malformed:?}"
        );

        let duplicate = drive(
            Some(b"EXEC [\"/bin/false\"]\n"),
            "duplicate EXEC after execution starts must remain typed",
        );
        assert!(
            matches!(
                duplicate,
                InitError::UnexpectedBeaconMessage(BeaconMessage::Exec { ref argv })
                    if argv == &["/bin/false".to_owned()]
            ),
            "duplicate EXEC must retain InitError::UnexpectedBeaconMessage(Exec), got {duplicate:?}"
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    /// Once the accepted SHUTDOWN frame is followed by control-stream EOF,
    /// supervision must remain parked between nonblocking process-group probes
    /// while a TERM-ignoring command consumes the bounded guest grace.
    #[allow(
        unsafe_code,
        reason = "Linux getrusage(RUSAGE_THREAD) is the observable CPU-consumption boundary"
    )]
    #[cfg(all(feature = "integration-tests", target_os = "linux"))]
    #[test]
    fn shutdown_eof_parks_supervision_during_the_term_ignoring_grace() {
        use std::mem::MaybeUninit;
        use std::net::Shutdown;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        use nix::libc;

        fn thread_cpu_time() -> Duration {
            fn timeval_duration(value: libc::timeval) -> Duration {
                Duration::from_secs(u64::try_from(value.tv_sec).expect("nonnegative CPU seconds"))
                    + Duration::from_micros(
                        u64::try_from(value.tv_usec).expect("nonnegative CPU microseconds"),
                    )
            }

            let mut usage = MaybeUninit::<libc::rusage>::uninit();
            // SAFETY: `usage` points to writable storage for one `rusage` value;
            // a zero return initializes it completely for the current thread.
            let result = unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) };
            assert_eq!(result, 0, "getrusage(RUSAGE_THREAD) failed");
            // SAFETY: the successful syscall above initialized `usage`.
            let usage = unsafe { usage.assume_init() };
            timeval_duration(usage.ru_utime) + timeval_duration(usage.ru_stime)
        }

        let root = ScratchRoot::new("shutdown-eof-supervision-cpu");
        let child_ready = root.path().join("child-ready");
        let (reader, mut writer) = UnixStream::pair().expect("create private control stream");
        let reader: OwnedFd = reader.into();
        let child_ready_for_command = child_ready.clone();
        let supervisor = std::thread::spawn(move || {
            let mut conn = File::from(reader);
            let command = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "trap '' TERM; : > \"$1\"; while :; do /bin/sleep 1; done".to_owned(),
                "shutdown-eof-child".to_owned(),
                child_ready_for_command.display().to_string(),
            ];
            let cpu_before = thread_cpu_time();
            let wall_before = Instant::now();
            let result = exec_operator_command(&mut conn, &command);
            (result, thread_cpu_time().saturating_sub(cpu_before), wall_before.elapsed())
        });

        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while !child_ready.exists() {
            assert!(Instant::now() < ready_deadline, "TERM-ignoring child did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        writer.write_all(b"SHUTDOWN\n").expect("write accepted SHUTDOWN frame");
        writer.shutdown(Shutdown::Write).expect("close host write half after SHUTDOWN");

        let (result, supervisor_cpu, elapsed) = supervisor.join().expect("join guest supervisor");
        assert_eq!(result.expect("accepted SHUTDOWN remains successful"), 137);
        assert!(elapsed >= GUEST_STOP_GRACE, "TERM-ignoring command skipped the guest grace");
        assert!(
            supervisor_cpu < Duration::from_secs(1),
            "control EOF must park supervision during the five-second grace; supervisor consumed {supervisor_cpu:?} CPU"
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    /// S-VLL-10a: the real direct-child wait result uses the retained
    /// exit-status mapping as its oracle, including signal death as EXIT 137.
    #[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
    #[cfg(feature = "integration-tests")]
    #[test]
    fn direct_child_signal_wait_result_matches_the_retained_exit_status_mapping() {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let (reader, _writer) = UnixStream::pair().expect("create held-open control stream");
        let reader: OwnedFd = reader.into();
        let mut conn = File::from(reader);
        let command = vec!["/bin/sh".to_owned(), "-c".to_owned(), "kill -KILL $$".to_owned()];
        let expected = exit_status_to_wire(std::process::ExitStatus::from_raw(nix::libc::SIGKILL));

        let observed = exec_operator_command(&mut conn, &command)
            .expect("a signal-terminated direct child is a successful supervision result");

        assert_eq!(expected, 137, "the retained signal mapping must encode SIGKILL as 137");
        assert_eq!(
            observed, expected,
            "the production wait-result path must use the retained exit-status mapping"
        );
    }

    /// CONTRACT_SHAPE: bounded-change (READY write failure prevents operator EXEC).
    #[test]
    fn ready_send_failure_is_pre_ready_and_suppresses_exec() {
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Err(InitError::Io(std::io::Error::other("injected READY write failure"))),
        );
        assert!(matches!(result, Err(InitError::Io(_))));
        assert_eq!(
            trace,
            [
                PreReadyStage::Root,
                PreReadyStage::Modules,
                PreReadyStage::Connect,
                PreReadyStage::Network,
                PreReadyStage::Ready,
            ],
            "EXEC, operator execution, EXIT, SHUTDOWN, and poweroff must all remain absent",
        );
    }

    /// CONTRACT_SHAPE: bounded-change (open failure is typed and suppresses module load).
    #[test]
    fn module_open_failure_is_pre_ready_and_closed() {
        let loaded = Cell::new(false);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || {
                load_vsock_modules_with::<(), _, _>(
                    |_| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected")),
                    |_| {
                        loaded.set(true);
                        Ok(())
                    },
                )
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::ModuleOpen { .. })));
        assert_eq!(trace, [PreReadyStage::Root, PreReadyStage::Modules]);
        assert!(!loaded.get());
    }

    /// CONTRACT_SHAPE: bounded-change (load failure is typed and suppresses later modules).
    #[test]
    fn module_load_failure_is_pre_ready_and_closed() {
        let loads = Cell::new(0_u32);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || {
                load_vsock_modules_with(
                    |_| Ok(Some(())),
                    |_| {
                        loads.set(loads.get() + 1);
                        Err(Errno::ENOEXEC)
                    },
                )
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::ModuleLoad { .. })));
        assert_eq!(trace, [PreReadyStage::Root, PreReadyStage::Modules]);
        assert_eq!(loads.get(), 1);
    }

    /// CONTRACT_SHAPE: bounded-change (socket failure is typed and suppresses connect).
    #[test]
    fn beacon_socket_failure_is_pre_ready_and_closed() {
        let connected = Cell::new(false);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || {
                connect_beacon_once_with::<(), _, _>(
                    || Err(Errno::EAFNOSUPPORT),
                    |_| {
                        connected.set(true);
                        Ok(())
                    },
                )
            },
            || Ok(()),
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::Socket(Errno::EAFNOSUPPORT))));
        assert_eq!(trace, [PreReadyStage::Root, PreReadyStage::Modules, PreReadyStage::Connect]);
        assert!(!connected.get());
    }

    /// CONTRACT_SHAPE: bounded-change (connect failure is typed and remains pre-READY).
    #[test]
    fn beacon_connect_failure_is_pre_ready_and_closed() {
        let attempts = Cell::new(0_u32);
        let pauses = Cell::new(0_u32);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || {
                connect_beacon_with(
                    || {
                        attempts.set(attempts.get() + 1);
                        Err::<(), _>(InitError::Connect(Errno::ECONNREFUSED))
                    },
                    || pauses.set(pauses.get() + 1),
                )
            },
            || Ok(()),
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::Connect(Errno::ECONNREFUSED))));
        assert_eq!(attempts.get(), VSOCK_CONNECT_MAX_ATTEMPTS);
        assert_eq!(pauses.get(), VSOCK_CONNECT_MAX_ATTEMPTS);
        assert_eq!(trace, [PreReadyStage::Root, PreReadyStage::Modules, PreReadyStage::Connect]);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_sanctioned_pre_ready_failure_maps_to_the_closed_init_error_set() {
        let io = || std::io::Error::other("injected");
        let errors = [
            (
                InitError::ModuleOpen { path: "/module".to_owned(), source: io() },
                PreReadyErrorClass::ModuleOpen,
            ),
            (
                InitError::ModuleLoad { path: "/module".to_owned(), source: Errno::EPERM },
                PreReadyErrorClass::ModuleLoad,
            ),
            (InitError::Socket(Errno::EPERM), PreReadyErrorClass::Socket),
            (InitError::Connect(Errno::ECONNREFUSED), PreReadyErrorClass::Connect),
            (InitError::Io(io()), PreReadyErrorClass::BeaconIo),
            (
                InitError::GuestDirectory { path: PathBuf::from("/proc"), source: io() },
                PreReadyErrorClass::GuestDirectory,
            ),
            (
                InitError::ProcMount { target: PathBuf::from("/proc"), source: Errno::EPERM },
                PreReadyErrorClass::ProcMount,
            ),
            (
                InitError::GuestNetworkIo { operation: "read", source: io() },
                PreReadyErrorClass::GuestNetworkIo,
            ),
            (
                InitError::GuestTransportSetup(GuestTransportSetupError::TokenMissing),
                PreReadyErrorClass::GuestTransportSetup,
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(pre_ready_error_class(&error), Some(expected));
        }
        assert_eq!(pre_ready_error_class(&InitError::NoExecReceived), None);
    }

    /// The token a well-formed `overdrive.transport=` command line yields.
    fn transport_token() -> GuestTransportToken {
        GuestTransportToken {
            workload_addr: Ipv4Addr::new(100, 95, 0, 166),
            gateway: Ipv4Addr::new(100, 95, 0, 1),
            launch: overdrive_core::guest_transport::LaunchToken::from_bytes([0x5a; 16]).unwrap(),
            // GUEST-PORTS (ADR-0217): the host listener ports the VM driver
            // appends; not the 1234 / 1240 / 1243 defaults.
            ports: crate::transport::GuestHostPorts {
                beacon: overdrive_core::vm::config::VsockPort::new(50_234),
                stream: overdrive_core::vm::config::VsockPort::new(50_240),
                control: overdrive_core::vm::config::VsockPort::new(50_243),
            },
        }
    }

    /// The `overdrive.transport=` token of [`transport_token`].
    fn transport_cmdline() -> String {
        format!(
            "console=ttyS0 overdrive.transport=vsock2,addr=100.95.0.166,dns=100.95.0.1,launch={},beacon=50234,stream=50240,control=50243",
            "5a".repeat(16)
        )
    }

    /// CONTRACT_SHAPE: bounded-change (cmdline read failure suppresses every
    /// transport setup step).
    #[test]
    fn cmdline_read_failure_is_pre_ready_and_closed() {
        let setup_ran = Cell::new(false);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                configure_guest_network_with(
                    || Err(std::io::Error::other("injected cmdline read failure")),
                    |_| {
                        setup_ran.set(true);
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .map(|_| ())
            },
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::GuestNetworkIo { .. })));
        assert_eq!(
            trace,
            [
                PreReadyStage::Root,
                PreReadyStage::Modules,
                PreReadyStage::Connect,
                PreReadyStage::Network,
            ]
        );
        assert!(!setup_ran.get(), "no transport setup step may run");
    }

    /// CONTRACT_SHAPE: bounded-change (the resolver write keeps its place
    /// after the transport setup, and its failure suppresses READY).
    #[test]
    fn resolver_write_failure_suppresses_ready_and_exec() {
        let configured = Cell::new(false);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                configure_guest_network_with(
                    || Ok(transport_cmdline().into_bytes()),
                    |token| {
                        assert_eq!(*token, transport_token());
                        configured.set(true);
                        Ok(())
                    },
                    |dns| {
                        assert_eq!(dns, transport_token().gateway, "the resolver is the gateway");
                        Err(InitError::GuestNetworkIo {
                            operation: "injected resolver write",
                            source: std::io::Error::other("injected"),
                        })
                    },
                )
                .map(|_| ())
            },
            || Ok(()),
        );
        assert!(matches!(result, Err(InitError::GuestNetworkIo { .. })));
        assert!(configured.get(), "the transport is configured before the resolver write");
        assert_eq!(
            trace,
            [
                PreReadyStage::Root,
                PreReadyStage::Modules,
                PreReadyStage::Connect,
                PreReadyStage::Network,
            ],
            "READY, EXEC, operator execution, EXIT, SHUTDOWN, and poweroff must all remain absent",
        );
    }

    // -----------------------------------------------------------------------
    // P6-S11 wiring cells (DG-5): the guest transport setup's console line,
    // the guest flow owner's thread before READY, READY's launch token.
    // -----------------------------------------------------------------------

    /// DG-5: on any guest transport setup failure `overdrive-init` writes
    /// exactly one console line `overdrive-init: guest transport setup failed:
    /// <Display>` and takes the pre-READY exit path: neither the guest flow
    /// owner nor READY starts. Every other failure keeps the existing line.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_setup_failure_is_one_console_line_and_starts_neither_owner_nor_ready() {
        let io = || std::io::Error::other("boom");
        let setup_errors: Vec<(GuestTransportSetupError, &str)> = vec![
            (
                GuestTransportSetupError::TokenMissing,
                "kernel command line has no overdrive.transport= token",
            ),
            (
                GuestTransportSetupError::TokenMalformed {
                    field: transport::TransportTokenField::Launch,
                },
                "overdrive.transport= field launch is malformed",
            ),
            (
                GuestTransportSetupError::DummyDevice { source: io() },
                "creating the transport dummy device failed: boom",
            ),
            (
                GuestTransportSetupError::InboundListenerBind { source: io() },
                "binding the inbound vsock listener failed: boom",
            ),
        ];
        for (error, display) in setup_errors {
            let err = InitError::GuestTransportSetup(error);
            let line = init_failure_console_line(&err);
            assert_eq!(line, format!("overdrive-init: guest transport setup failed: {display}"));
            assert!(!line.contains('\n'), "exactly one line");
        }
        assert_eq!(
            init_failure_console_line(&InitError::NoExecReceived),
            "overdrive-init: fatal: beacon connection closed before the host sent EXEC"
        );

        let owner_started = Cell::new(false);
        let ready_sent = Cell::new(false);
        let result = complete_pre_ready_init(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || -> Result<(), InitError> {
                Err(InitError::GuestTransportSetup(GuestTransportSetupError::TokenMissing))
            },
            |()| {
                owner_started.set(true);
                Ok(())
            },
            |_, ()| {
                ready_sent.set(true);
                Ok(())
            },
        );
        assert!(matches!(
            result,
            Err(InitError::GuestTransportSetup(GuestTransportSetupError::TokenMissing))
        ));
        assert!(!owner_started.get(), "no guest flow owner after a setup failure");
        assert!(!ready_sent.get(), "no READY after a setup failure");
    }

    /// DG-5: a guest flow owner failure is one console line naming it.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_guest_owner_failure_is_one_console_line() {
        let fatal = GuestOwnerFatal::EventWait { source: std::io::Error::other("boom") };
        assert_eq!(
            guest_owner_fatal_console_line(&fatal),
            "overdrive-init: guest flow owner failed: waiting for guest transport events failed: boom"
        );
    }

    /// DG-5 / G-V3: the pre-READY path constructs the guest flow owner after
    /// the transport setup and starts its `run` on its own thread before READY
    /// — READY is not gated on the control session: at READY the owner has
    /// already made its first seam call and no control session exists. The
    /// owner's only exit, a `GuestOwnerFatal`, reaches the fatal handler.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn the_guest_flow_owner_runs_on_its_own_thread_before_ready() {
        use std::sync::Mutex;
        use std::sync::mpsc;
        use std::time::Duration;

        use guest_flow::sim::{SimGuestClock, SimGuestOwnerIo};

        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(transport_token().workload_addr, Arc::clone(&clock));
        // the owner's readiness wait fails: its `run` returns the fatal
        io.fail_event_wait(std::io::ErrorKind::Interrupted);
        let (tx, rx) = mpsc::channel::<String>();
        let fatal_seen = Arc::new(Mutex::new(None::<String>));
        let at_ready = Cell::new((false, true));
        let result = complete_pre_ready_init(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(transport_token()),
            |token: GuestTransportToken| {
                let owner_io: Box<dyn GuestOwnerIo> = Box::new(io.clone());
                let owner_clock: Arc<dyn GuestClock> = clock.clone();
                let seen = Arc::clone(&fatal_seen);
                let _thread = start_guest_flow_owner(token, owner_io, owner_clock, move |fatal| {
                    *seen.lock().unwrap() = Some(fatal.to_string());
                    tx.send(fatal.to_string()).unwrap();
                })?;
                Ok(token)
            },
            |_, token: GuestTransportToken| {
                // the owner thread is running: it has called the seam (its
                // first dispatch) or already ended on the scripted wait failure
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let started = || !io.effects().is_empty() || fatal_seen.lock().unwrap().is_some();
                while !started() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(token, transport_token());
                at_ready.set((started(), io.session_socket().is_none()));
                Ok(())
            },
        );
        assert!(result.is_ok(), "{result:?}");
        let (started, no_session) = at_ready.get();
        assert!(started, "the guest flow owner runs on its own thread before READY");
        assert!(no_session, "READY is not gated on the control session");
        let fatal = rx.recv_timeout(Duration::from_secs(10)).expect("the owner's run returned");
        assert_eq!(fatal, "waiting for guest transport events failed: operation interrupted");
    }

    /// DG-7 / ATTR-TOKEN: READY carries the launch token of the guest's one
    /// `overdrive.transport=` token, read from the kernel command line, after
    /// the transport setup and the resolver write.
    /// READY carries the transport token's launch token and its beacon port
    /// (GUEST-PORTS).
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn ready_carries_the_launch_token_of_the_transport_token() {
        let ready_line = std::cell::RefCell::new(None::<String>);
        let result = complete_pre_ready_init(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                configure_guest_network_with(
                    || Ok(transport_cmdline().into_bytes()),
                    |_| Ok(()),
                    |_| Ok(()),
                )
            },
            |(token, ()): (GuestTransportToken, ())| Ok(token),
            |_, token: GuestTransportToken| {
                *ready_line.borrow_mut() = Some(ready_message(1, &token).to_string());
                Ok(())
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            ready_line.into_inner(),
            // GUEST-PORTS (ADR-0217): READY's `port=` is the token's
            // `beacon=` (50234 in the fixture), never the 1234 default.
            Some(format!("READY pid=1 port=50234 launch={}", "5a".repeat(16)))
        );
    }
}
