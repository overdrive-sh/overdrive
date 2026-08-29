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
//! 3. Require and parse the platform `overdrive.net=` token.
//! 4. Verify the NIC is down, suppress IPv6 and ARP notification with
//!    write/readback checks, then apply address, netmask, link state,
//!    default route, and resolver configuration.
//! 5. Send `READY` exactly once. Any earlier error returns through PID 1's
//!    emergency power-off path without publishing `READY` or guest `EXIT`.
//! 6. Block for exactly one host -> guest message and require it to be
//!    `EXEC { argv }` — the operator's command arrives here, over the
//!    beacon, never on the kernel cmdline (`KernelCmdline` stays
//!    platform-only, ADR-0082 §D2).
//! 7. Exec `argv[0]` with `argv`, forwarding stdio untouched.
//! 8. Send `EXIT <status>` with the command's real exit status once it
//!    resolves — never validating the operator's own I/O.
//! 9. Read at most one line for a `SHUTDOWN` request (or `EOF`), then
//!    power off (`reboot(RB_POWER_OFF)`).
//!
//! # Scope note (step 01-03)
//!
//! This step lands the crate, its beacon-speaking logic, AND consumption
//! of the pinned `EXEC` channel. The real end-to-end boot (a real
//! kernel, a real Cloud Hypervisor vsock device, a real operator command
//! sourced from a real deploy spec) is exercised at step 01-08 under
//! Tier-3. Two things this file deliberately does not attempt, both out
//! of this step's design surface (ADR-0082 §D7 amendment's ownership
//! table):
//!
//! - **Concurrent `SHUTDOWN`-during-execution.** ADR-0082 §D4's
//!   `VmDriver::stop` can write `SHUTDOWN` while the operator's command
//!   is still running. Racing that write against the child's `wait()`
//!   is Slice 03's concern (US-VM-3/4/7,
//!   `stop-restart-and-vmm-death`) with its own Tier-3 scenarios
//!   (S-VM-45..47). This step reads for `SHUTDOWN` only after the
//!   operator's command has already finished, which satisfies the wire
//!   contract's "at most one `SHUTDOWN`" without inventing the
//!   concurrent race Slice 03 is scoped to test.
//! - **Who writes `EXEC`, and where the command ultimately comes from.**
//!   This file only *consumes* `EXEC` ([`recv_exec`]) — it neither
//!   writes it nor sources the operator's command. `VmDriver` **writes**
//!   `EXEC` on the just-accepted beacon session, gating the host-side
//!   `Running` continuation, at step **01-07** (owns `VmDriver` and the
//!   `LiveVm` session). The operator `command`/`args` **source**
//!   (`AllocationSpec.command`/`args` threaded through `DriverInput::Vm`,
//!   driven by a real `[vm]+[job]` deploy) lands at step **01-08** (owns
//!   spec-parse dispatch, the composition root, and the S-VM-01 walking
//!   skeleton). Both have a named landing step in the ADR's ownership
//!   table — neither is an unowned deferral.

// A minimal PID 1 has no `tracing` sink, no log aggregation, and no
// operator shell — `/dev/console` (fd 0/1/2, held before devtmpfs is up
// per ADR-0082 §D7) IS the diagnostic channel. `eprintln!` on the
// emergency path is the intended mechanism, not a stopgap. Matches the
// project's own "allowed in CLI/binary-boundary crates via crate-level
// override" pattern (`.claude/rules/development.md` § Cargo.toml
// conventions on `expect_used`).
#![allow(clippy::print_stderr)]
#![deny(unsafe_code)]
// Guest network configuration uses the kernel's legacy ifreq/route ioctl ABI;
// nix generates the request wrappers, and each required unsafe call is kept in
// the smallest possible function with a local SAFETY justification.

use std::ffi::CString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nix::errno::Errno;
use nix::kmod::{ModuleInitFlags, finit_module};
use nix::libc;
use nix::mount::{MsFlags, mount};
use nix::net::if_::if_nameindex;
use nix::sys::reboot::{self, RebootMode};
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, VsockAddr};
use overdrive_core::vm::beacon::{self, BeaconMessage};

nix::ioctl_write_ptr_bad!(
    #[allow(unsafe_code, reason = "nix-generated wrapper for the required Linux SIOCSIFADDR ABI")]
    set_interface_address,
    libc::SIOCSIFADDR,
    libc::ifreq
);
nix::ioctl_write_ptr_bad!(
    #[allow(
        unsafe_code,
        reason = "nix-generated wrapper for the required Linux SIOCSIFNETMASK ABI"
    )]
    set_interface_netmask,
    libc::SIOCSIFNETMASK,
    libc::ifreq
);
nix::ioctl_read_bad!(
    #[allow(unsafe_code, reason = "nix-generated wrapper for the required Linux SIOCGIFFLAGS ABI")]
    get_interface_flags,
    libc::SIOCGIFFLAGS,
    libc::ifreq
);
nix::ioctl_write_ptr_bad!(
    #[allow(unsafe_code, reason = "nix-generated wrapper for the required Linux SIOCSIFFLAGS ABI")]
    set_interface_flags,
    libc::SIOCSIFFLAGS,
    libc::ifreq
);
nix::ioctl_write_ptr_bad!(
    #[allow(unsafe_code, reason = "nix-generated wrapper for the required Linux SIOCADDRT ABI")]
    add_ipv4_route,
    libc::SIOCADDRT,
    libc::rtentry
);

fn main() {
    if let Err(err) = run() {
        eprintln!("overdrive-init: fatal: {err}");
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
        |conn| send(conn, &BeaconMessage::Ready { pid, port: beacon::BEACON_VSOCK_PORT }),
        recv_exec,
        exec_operator_command,
        |conn, status| send(conn, &BeaconMessage::Exit { status }),
        |conn| read_shutdown_or_eof(conn).map(|_| ()),
        || {
            reboot::reboot(RebootMode::RB_POWER_OFF).map_err(InitError::Reboot)?;
            Ok(())
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_guest_lifecycle<
    Conn,
    Root,
    Modules,
    Connect,
    Network,
    Ready,
    ReceiveExec,
    Execute,
    SendExit,
    Shutdown,
    PowerOff,
>(
    root: Root,
    modules: Modules,
    connect: Connect,
    network: Network,
    ready: Ready,
    receive_exec: ReceiveExec,
    execute: Execute,
    send_exit: SendExit,
    shutdown: Shutdown,
    power_off: PowerOff,
) -> Result<(), InitError>
where
    Root: FnOnce() -> Result<(), InitError>,
    Modules: FnOnce() -> Result<(), InitError>,
    Connect: FnOnce() -> Result<Conn, InitError>,
    Network: FnOnce() -> Result<(), InitError>,
    Ready: FnOnce(&mut Conn) -> Result<(), InitError>,
    ReceiveExec: FnOnce(&Conn) -> Result<Vec<String>, InitError>,
    Execute: FnOnce(&[String]) -> Result<i32, InitError>,
    SendExit: FnOnce(&mut Conn, i32) -> Result<(), InitError>,
    Shutdown: FnOnce(&Conn) -> Result<(), InitError>,
    PowerOff: FnOnce() -> Result<(), InitError>,
{
    let mut conn = complete_pre_ready_init(root, modules, connect, network, ready)?;
    let argv = receive_exec(&conn)?;
    let status = execute(&argv)?;
    send_exit(&mut conn, status)?;
    shutdown(&conn)?;
    power_off()
}

fn complete_pre_ready_init<Conn, Root, Modules, Connect, Network, Ready>(
    root: Root,
    modules: Modules,
    connect: Connect,
    network: Network,
    ready: Ready,
) -> Result<Conn, InitError>
where
    Root: FnOnce() -> Result<(), InitError>,
    Modules: FnOnce() -> Result<(), InitError>,
    Connect: FnOnce() -> Result<Conn, InitError>,
    Network: FnOnce() -> Result<(), InitError>,
    Ready: FnOnce(&mut Conn) -> Result<(), InitError>,
{
    root()?;
    modules()?;
    let mut conn = connect()?;
    network()?;
    ready(&mut conn)?;
    Ok(conn)
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
    /// The platform-owned `overdrive.net=` kernel token was malformed.
    #[error("invalid guest network kernel parameter: {detail}")]
    GuestNetworkConfig { detail: String },
    /// Reading a file needed to discover or configure the guest network failed.
    #[error("guest network {operation} failed: {source}")]
    GuestNetworkIo {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    /// A Linux networking ioctl failed.
    #[error("guest network syscall {operation} failed: {source}")]
    GuestNetworkSyscall {
        operation: &'static str,
        #[source]
        source: Errno,
    },
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
    GuestNetworkConfig,
    GuestNetworkIo,
    GuestNetworkSyscall,
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
        InitError::GuestNetworkConfig { .. } => Some(PreReadyErrorClass::GuestNetworkConfig),
        InitError::GuestNetworkIo { .. } => Some(PreReadyErrorClass::GuestNetworkIo),
        InitError::GuestNetworkSyscall { .. } => Some(PreReadyErrorClass::GuestNetworkSyscall),
        InitError::NoExecReceived
        | InitError::UnexpectedBeaconMessage(_)
        | InitError::BeaconParse(_)
        | InitError::Spawn { .. }
        | InitError::Reboot(_) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuestNetworkConfig {
    addr: Ipv4Addr,
    prefix_len: u8,
    gateway: Ipv4Addr,
    dns: Ipv4Addr,
}

fn parse_guest_network_cmdline(cmdline: &str) -> Result<Option<GuestNetworkConfig>, InitError> {
    const PREFIX: &str = "overdrive.net=";

    let mut values = cmdline.split_whitespace().filter_map(|token| token.strip_prefix(PREFIX));
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(guest_network_config_error("more than one overdrive.net token was present"));
    }

    let mut fields = value.split(',');
    let address_with_prefix = fields
        .next()
        .ok_or_else(|| guest_network_config_error("guest address and prefix are missing"))?;
    let gateway = fields
        .next()
        .and_then(|field| field.strip_prefix("gw="))
        .ok_or_else(|| guest_network_config_error("gw field is missing or out of order"))?;
    let dns = fields
        .next()
        .and_then(|field| field.strip_prefix("dns="))
        .ok_or_else(|| guest_network_config_error("dns field is missing or out of order"))?;
    if fields.next().is_some() {
        return Err(guest_network_config_error("unexpected extra guest network field"));
    }

    let (addr, prefix_len) = address_with_prefix
        .rsplit_once('/')
        .ok_or_else(|| guest_network_config_error("guest address has no prefix length"))?;
    let addr = parse_ipv4(addr, "guest address")?;
    let prefix_len = prefix_len
        .parse::<u8>()
        .map_err(|_| guest_network_config_error("prefix length is not an integer"))?;
    if prefix_len > 32 {
        return Err(guest_network_config_error("prefix length exceeds 32"));
    }

    Ok(Some(GuestNetworkConfig {
        addr,
        prefix_len,
        gateway: parse_ipv4(gateway, "gateway")?,
        dns: parse_ipv4(dns, "DNS responder")?,
    }))
}

#[cfg(test)]
fn required_guest_network_config(cmdline: &str) -> Result<GuestNetworkConfig, InitError> {
    parse_guest_network_cmdline(cmdline)?.ok_or_else(|| {
        guest_network_config_error("required platform overdrive.net token was missing")
    })
}

fn guest_network_config_error(detail: impl Into<String>) -> InitError {
    InitError::GuestNetworkConfig { detail: detail.into() }
}

fn parse_ipv4(raw: &str, field: &'static str) -> Result<Ipv4Addr, InitError> {
    raw.parse().map_err(|_| guest_network_config_error(format!("{field} is not valid IPv4")))
}

fn configure_guest_network() -> Result<(), InitError> {
    configure_guest_network_with(|| fs::read("/proc/cmdline"), &mut LinuxGuestNetworkOps)
}

fn configure_guest_network_with<Read, Ops>(
    read_cmdline: Read,
    ops: &mut Ops,
) -> Result<(), InitError>
where
    Read: FnOnce() -> std::io::Result<Vec<u8>>,
    Ops: GuestNetworkOps,
{
    let cmdline = read_cmdline()
        .map_err(|source| InitError::GuestNetworkIo { operation: "read /proc/cmdline", source })?;
    let cmdline = std::str::from_utf8(&cmdline).map_err(|source| InitError::GuestNetworkIo {
        operation: "decode /proc/cmdline as UTF-8",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })?;
    let Some(config) = parse_guest_network_cmdline(cmdline)? else {
        // Legacy/non-mesh VMs have no allocated guest wire. The platform
        // token itself is the assignment marker; when it is absent there is
        // deliberately no NIC mutation to perform before READY.
        return Ok(());
    };
    apply_guest_network_with(ops, config)
}

fn mount_procfs_at(target: &Path) -> Result<(), InitError> {
    let flags = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    match mount(Some("proc"), target, Some("proc"), flags, None::<&str>) {
        Ok(()) | Err(Errno::EBUSY) => Ok(()),
        Err(source) => Err(InitError::ProcMount { target: target.to_path_buf(), source }),
    }
}

trait GuestNetworkOps {
    type IoctlSocket;

    fn interface(&mut self) -> Result<String, InitError>;
    fn require_down(&mut self, interface: &str) -> Result<(), InitError>;
    fn write_ipv6_disabled(&mut self, interface: &str) -> Result<(), InitError>;
    fn read_ipv6_disabled(&mut self, interface: &str) -> Result<bool, InitError>;
    fn write_arp_notify_disabled(&mut self, interface: &str) -> Result<(), InitError>;
    fn read_arp_notify_disabled(&mut self, interface: &str) -> Result<bool, InitError>;
    fn open_ioctl_socket(&mut self) -> Result<Self::IoctlSocket, InitError>;
    fn set_address(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        address: Ipv4Addr,
    ) -> Result<(), InitError>;
    fn set_netmask(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        prefix_len: u8,
    ) -> Result<(), InitError>;
    fn bring_up(&mut self, socket: &Self::IoctlSocket, interface: &str) -> Result<(), InitError>;
    fn add_route(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        gateway: Ipv4Addr,
    ) -> Result<(), InitError>;
    fn write_resolver(&mut self, dns: Ipv4Addr) -> Result<(), InitError>;
}

fn apply_guest_network_with<Ops: GuestNetworkOps>(
    ops: &mut Ops,
    config: GuestNetworkConfig,
) -> Result<(), InitError> {
    let interface = ops.interface()?;
    ops.require_down(&interface)?;
    ops.write_ipv6_disabled(&interface)?;
    if !ops.read_ipv6_disabled(&interface)? {
        return Err(guest_network_config_error("guest IPv6 suppression readback was not 1"));
    }
    ops.write_arp_notify_disabled(&interface)?;
    if !ops.read_arp_notify_disabled(&interface)? {
        return Err(guest_network_config_error(
            "guest ARP notification suppression readback was not 0",
        ));
    }
    let socket = ops.open_ioctl_socket()?;
    ops.set_address(&socket, &interface, config.addr)?;
    ops.set_netmask(&socket, &interface, config.prefix_len)?;
    ops.bring_up(&socket, &interface)?;
    ops.add_route(&socket, &interface, config.gateway)?;
    ops.write_resolver(config.dns)
}

struct LinuxGuestNetworkOps;

impl LinuxGuestNetworkOps {
    fn sysctl_path(interface: &str, leaf: &str) -> PathBuf {
        Path::new("/proc/sys/net/ipv6/conf").join(interface).join(leaf)
    }

    fn sysctl_write(path: &Path, value: &str, operation: &'static str) -> Result<(), InitError> {
        fs::write(path, value).map_err(|source| InitError::GuestNetworkIo { operation, source })
    }

    fn sysctl_equals(path: &Path, value: &str, operation: &'static str) -> Result<bool, InitError> {
        fs::read_to_string(path)
            .map(|actual| actual.trim() == value)
            .map_err(|source| InitError::GuestNetworkIo { operation, source })
    }
}

impl GuestNetworkOps for LinuxGuestNetworkOps {
    type IoctlSocket = std::os::fd::OwnedFd;

    fn interface(&mut self) -> Result<String, InitError> {
        single_non_loopback_interface()
    }

    fn require_down(&mut self, interface: &str) -> Result<(), InitError> {
        let socket =
            socket::socket(AddressFamily::Inet, SockType::Datagram, SockFlag::empty(), None)
                .map_err(|source| InitError::GuestNetworkSyscall {
                    operation: "create guest admission ioctl socket",
                    source,
                })?;
        let flags = read_interface_flags(socket.as_raw_fd(), interface)?;
        require_interface_down(flags)
    }

    fn write_ipv6_disabled(&mut self, interface: &str) -> Result<(), InitError> {
        Self::sysctl_write(
            &Self::sysctl_path(interface, "disable_ipv6"),
            "1",
            "write guest IPv6 suppression",
        )
    }

    fn read_ipv6_disabled(&mut self, interface: &str) -> Result<bool, InitError> {
        Self::sysctl_equals(
            &Self::sysctl_path(interface, "disable_ipv6"),
            "1",
            "read back guest IPv6 suppression",
        )
    }

    fn write_arp_notify_disabled(&mut self, interface: &str) -> Result<(), InitError> {
        let path = Path::new("/proc/sys/net/ipv4/conf").join(interface).join("arp_notify");
        Self::sysctl_write(&path, "0", "write guest ARP notification suppression")
    }

    fn read_arp_notify_disabled(&mut self, interface: &str) -> Result<bool, InitError> {
        let path = Path::new("/proc/sys/net/ipv4/conf").join(interface).join("arp_notify");
        Self::sysctl_equals(&path, "0", "read back guest ARP notification suppression")
    }

    fn open_ioctl_socket(&mut self) -> Result<Self::IoctlSocket, InitError> {
        socket::socket(AddressFamily::Inet, SockType::Datagram, SockFlag::empty(), None).map_err(
            |source| InitError::GuestNetworkSyscall {
                operation: "create IPv4 ioctl socket",
                source,
            },
        )
    }

    #[allow(
        unsafe_code,
        reason = "the synchronous address ioctl reads one initialized ifreq during the call"
    )]
    fn set_address(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        address: Ipv4Addr,
    ) -> Result<(), InitError> {
        let request = ifreq_with_address(interface, address)?;
        // SAFETY: the initialized request and owned socket outlive this synchronous ioctl.
        unsafe { set_interface_address(socket.as_raw_fd(), &raw const request) }.map_err(
            |source| InitError::GuestNetworkSyscall { operation: "set guest IPv4 address", source },
        )?;
        Ok(())
    }

    #[allow(
        unsafe_code,
        reason = "the synchronous netmask ioctl reads one initialized ifreq during the call"
    )]
    fn set_netmask(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        prefix_len: u8,
    ) -> Result<(), InitError> {
        let request = ifreq_with_address(interface, prefix_netmask(prefix_len))?;
        // SAFETY: the initialized request and owned socket outlive this synchronous ioctl.
        unsafe { set_interface_netmask(socket.as_raw_fd(), &raw const request) }.map_err(
            |source| InitError::GuestNetworkSyscall { operation: "set guest IPv4 netmask", source },
        )?;
        Ok(())
    }

    fn bring_up(&mut self, socket: &Self::IoctlSocket, interface: &str) -> Result<(), InitError> {
        bring_interface_up(socket.as_raw_fd(), interface)
    }

    fn add_route(
        &mut self,
        socket: &Self::IoctlSocket,
        interface: &str,
        gateway: Ipv4Addr,
    ) -> Result<(), InitError> {
        add_default_route(socket.as_raw_fd(), interface, gateway)
    }

    fn write_resolver(&mut self, dns: Ipv4Addr) -> Result<(), InitError> {
        fs::write("/etc/resolv.conf", format!("nameserver {dns}\n")).map_err(|source| {
            InitError::GuestNetworkIo { operation: "write /etc/resolv.conf", source }
        })
    }
}

fn single_non_loopback_interface() -> Result<String, InitError> {
    let interfaces = if_nameindex().map_err(|source| InitError::GuestNetworkSyscall {
        operation: "enumerate guest network interfaces",
        source,
    })?;
    for interface in &interfaces {
        let name = interface
            .name()
            .to_str()
            .map_err(|_| guest_network_config_error("network interface name is not valid UTF-8"))?;
        if name != "lo" {
            return Ok(name.to_owned());
        }
    }
    Err(guest_network_config_error("no non-loopback network interface was present"))
}

fn interface_name(name: &str) -> Result<[libc::c_char; libc::IFNAMSIZ], InitError> {
    if name.is_empty() || name.len() >= libc::IFNAMSIZ {
        return Err(guest_network_config_error(format!(
            "network interface name {name:?} does not fit IFNAMSIZ"
        )));
    }
    let mut bytes = [0; libc::IFNAMSIZ];
    for (destination, source) in bytes.iter_mut().zip(name.as_bytes()) {
        *destination = *source as libc::c_char;
    }
    Ok(bytes)
}

fn ifreq_with_address(name: &str, address: Ipv4Addr) -> Result<libc::ifreq, InitError> {
    Ok(libc::ifreq {
        ifr_name: interface_name(name)?,
        ifr_ifru: libc::__c_anonymous_ifr_ifru { ifru_addr: sockaddr_ipv4(address) },
    })
}

fn ifreq_with_flags(name: &str, flags: libc::c_short) -> Result<libc::ifreq, InitError> {
    Ok(libc::ifreq {
        ifr_name: interface_name(name)?,
        ifr_ifru: libc::__c_anonymous_ifr_ifru { ifru_flags: flags },
    })
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Linux AF_INET is the ABI constant 2 and always fits sa_family_t"
)]
fn sockaddr_ipv4(address: Ipv4Addr) -> libc::sockaddr {
    let mut data = [0; 14];
    for (destination, octet) in data[2..6].iter_mut().zip(address.octets()) {
        *destination = octet as libc::c_char;
    }
    libc::sockaddr { sa_family: libc::AF_INET as libc::sa_family_t, sa_data: data }
}

fn prefix_netmask(prefix_len: u8) -> Ipv4Addr {
    let mask = if prefix_len == 0 { 0 } else { u32::MAX << (32 - prefix_len) };
    Ipv4Addr::from(mask)
}

fn require_interface_down(flags: libc::c_short) -> Result<(), InitError> {
    let up_flag = libc::c_short::try_from(libc::IFF_UP)
        .map_err(|_| guest_network_config_error("Linux IFF_UP does not fit ifreq flags"))?;
    if flags & up_flag != 0 {
        return Err(guest_network_config_error(
            "guest interface was already up before platform configuration",
        ));
    }
    Ok(())
}

#[allow(
    unsafe_code,
    reason = "the flags ioctl and union read are isolated here and locally justified"
)]
fn read_interface_flags(fd: libc::c_int, interface: &str) -> Result<libc::c_short, InitError> {
    let mut request = ifreq_with_flags(interface, 0)?;
    // SAFETY: `request` is initialized with the correct Linux `ifreq` layout,
    // is uniquely borrowed for the call, and the kernel only mutates it before
    // returning synchronously.
    unsafe { get_interface_flags(fd, &raw mut request) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "read guest interface flags", source }
    })?;
    // SAFETY: SIOCGIFFLAGS initialized the flags member of the union above.
    Ok(unsafe { request.ifr_ifru.ifru_flags })
}

#[allow(
    unsafe_code,
    reason = "the flags ioctl pair and union read are isolated here and locally justified"
)]
fn bring_interface_up(fd: libc::c_int, interface: &str) -> Result<(), InitError> {
    let current = read_interface_flags(fd, interface)?;
    let up_flag = libc::c_short::try_from(libc::IFF_UP)
        .map_err(|_| guest_network_config_error("Linux IFF_UP does not fit ifreq flags"))?;
    let flags = current | up_flag;
    let request = ifreq_with_flags(interface, flags)?;
    // SAFETY: `request` and `fd` satisfy the same synchronous ioctl lifetime
    // contract as the preceding SIOCGIFFLAGS call.
    unsafe { set_interface_flags(fd, &raw const request) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "bring guest interface up", source }
    })?;
    Ok(())
}

#[allow(
    unsafe_code,
    reason = "the synchronous route ioctl uses an initialized rtentry and live device C string"
)]
fn add_default_route(fd: libc::c_int, interface: &str, gateway: Ipv4Addr) -> Result<(), InitError> {
    let device = CString::new(interface)
        .map_err(|_| guest_network_config_error("network interface name contains NUL"))?;
    let route = libc::rtentry {
        rt_pad1: 0,
        rt_dst: sockaddr_ipv4(Ipv4Addr::UNSPECIFIED),
        rt_gateway: sockaddr_ipv4(gateway),
        rt_genmask: sockaddr_ipv4(Ipv4Addr::UNSPECIFIED),
        rt_flags: (libc::RTF_UP | libc::RTF_GATEWAY) as libc::c_ushort,
        rt_pad2: 0,
        rt_pad3: 0,
        rt_tos: 0,
        rt_class: 0,
        rt_pad4: [0; 3],
        rt_metric: 0,
        rt_dev: device.as_ptr().cast_mut(),
        rt_mtu: 0,
        rt_window: 0,
        rt_irtt: 0,
    };
    // SAFETY: `route` and its `device` C string both outlive this synchronous
    // ioctl; all pointer-free sockaddr fields are fully initialized.
    unsafe { add_ipv4_route(fd, &raw const route) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "install guest default route", source }
    })?;
    Ok(())
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
    conn.write_all(line.as_bytes()).map_err(InitError::Io)
}

/// Reads at most one line off the beacon connection, or `Ok(None)` on
/// `EOF`. Shared preamble for [`recv_exec`] (which requires a line) and
/// [`read_shutdown_or_eof`] (which tolerates its absence) — both clone a
/// second handle over the SAME fd (`dup`; `try_clone` only needs
/// `&self`, so `conn` need not be `&mut` even though the caller also
/// uses it, via a separate `&mut` borrow, to write `READY`/`EXIT`) and
/// block for exactly one `\n`-terminated line.
fn read_one_line(conn: &File) -> Result<Option<String>, InitError> {
    let read_handle = conn.try_clone().map_err(InitError::Io)?;
    let mut reader = BufReader::new(read_handle);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).map_err(InitError::Io)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
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
        BeaconMessage::Exec { argv } => Ok(argv),
        other => Err(InitError::UnexpectedBeaconMessage(other)),
    }
}

/// Reads at most one line off the beacon connection and, if present,
/// parses it as a [`BeaconMessage`]. Tolerates `EOF` (`Ok(None)`) and a
/// malformed or unexpected line (folded into `Ok(None)` too, but not
/// silently — the raw line and the parse failure are logged to
/// `/dev/console` first) — either way the guest's next and only
/// remaining action is to power off (ADR-0082 §D4).
fn read_shutdown_or_eof(conn: &File) -> Result<Option<BeaconMessage>, InitError> {
    let Some(line) = read_one_line(conn)? else {
        return Ok(None);
    };
    match line.parse() {
        Ok(msg) => Ok(Some(msg)),
        Err(err) => {
            eprintln!("overdrive-init: unexpected post-EXIT host line {line:?}: {err}");
            Ok(None)
        }
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
fn exec_operator_command(argv: &[String]) -> Result<i32, InitError> {
    // `argv` is never empty here: the only producer of this value is
    // `recv_exec`, which only returns `Ok` after `BeaconMessage::from_str`
    // has already rejected an empty argv as `BeaconParseError::EmptyArgv`
    // (`.claude/rules/development.md` § "Logically unreachable None/Err").
    let command = argv.first().unwrap_or_else(|| {
        unreachable!(
            "recv_exec only returns argv already validated non-empty by BeaconMessage::from_str"
        )
    });
    let status = Command::new(command)
        .args(&argv[1..])
        .status()
        .map_err(|source| InitError::Spawn { command: command.clone(), source })?;
    Ok(exit_status_to_wire(status))
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

    use proptest::prelude::*;

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
        Shutdown,
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
            |_| {
                trace.borrow_mut().push(PreReadyStage::Ready);
                ready()
            },
            |_| {
                trace.borrow_mut().push(PreReadyStage::Exec);
                Ok(vec!["/bin/true".to_owned()])
            },
            |_| {
                trace.borrow_mut().push(PreReadyStage::Operator);
                Ok(0)
            },
            |_, _| {
                trace.borrow_mut().push(PreReadyStage::Exit);
                Ok(())
            },
            |_| {
                trace.borrow_mut().push(PreReadyStage::Shutdown);
                Ok(())
            },
            || {
                trace.borrow_mut().push(PreReadyStage::PowerOff);
                Ok(())
            },
        );
        (result, trace.into_inner())
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
            (guest_network_config_error("token"), PreReadyErrorClass::GuestNetworkConfig),
            (
                InitError::GuestNetworkIo { operation: "read", source: io() },
                PreReadyErrorClass::GuestNetworkIo,
            ),
            (
                InitError::GuestNetworkSyscall { operation: "ioctl", source: Errno::EPERM },
                PreReadyErrorClass::GuestNetworkSyscall,
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(pre_ready_error_class(&error), Some(expected));
        }
        assert_eq!(pre_ready_error_class(&InitError::NoExecReceived), None);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn guest_network_parser_maps_the_mesh_token_and_preserves_non_mesh_cmdlines() {
        let parsed = parse_guest_network_cmdline(
            "console=ttyS0 panic=1 overdrive.net=100.96.0.166/30,gw=100.96.0.165,dns=100.96.0.165 root=/dev/vda",
        )
        .unwrap();

        assert_eq!(
            parsed,
            Some(GuestNetworkConfig {
                addr: "100.96.0.166".parse().unwrap(),
                prefix_len: 30,
                gateway: "100.96.0.165".parse().unwrap(),
                dns: "100.96.0.165".parse().unwrap(),
            }),
        );
        assert_eq!(
            prefix_netmask(parsed.expect("network token parsed").prefix_len),
            Ipv4Addr::new(255, 255, 255, 252),
            "the guest applies the prefix as the matching IPv4 netmask",
        );
        assert_eq!(
            parse_guest_network_cmdline("console=ttyS0 panic=1 root=/dev/vda rw").unwrap(),
            None,
            "a non-mesh cmdline must retain the absence of guest network configuration",
        );
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn guest_network_parser_rejects_partial_or_duplicate_platform_tokens() {
        assert!(
            parse_guest_network_cmdline("overdrive.net=100.96.0.166/30,gw=100.96.0.165").is_err(),
            "a token without DNS must fail closed",
        );
        assert!(
            parse_guest_network_cmdline(
                "overdrive.net=100.96.0.166/30,gw=100.96.0.165,dns=100.96.0.165 overdrive.net=100.96.0.170/30,gw=100.96.0.169,dns=100.96.0.169",
            )
            .is_err(),
            "two platform network tokens are ambiguous and must fail closed",
        );
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn malformed_guest_addressing_tokens_are_rejected_by_the_q3_grammar() {
        for malformed in [
            "overdrive.net=not-an-ip/30,gw=100.96.0.165,dns=100.96.0.165",
            "overdrive.net=100.96.0.166/nope,gw=100.96.0.165,dns=100.96.0.165",
            "overdrive.net=100.96.0.166/33,gw=100.96.0.165,dns=100.96.0.165",
            "overdrive.net=100.96.0.166/30,dns=100.96.0.165,gw=100.96.0.165",
            "overdrive.net=100.96.0.166/30,gw=100.96.0.165,dns=100.96.0.165,extra=1",
        ] {
            assert!(
                parse_guest_network_cmdline(malformed).is_err(),
                "malformed Q3 token must fail closed: {malformed}"
            );
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum NetworkStage {
        Enumerate,
        NicDown,
        Ipv6Write,
        Ipv6Read,
        ArpWrite,
        ArpRead,
        IoctlSocket,
        Address,
        Netmask,
        Link,
        Route,
        Resolver,
    }

    impl NetworkStage {
        const ORDERED: [Self; 12] = [
            Self::Enumerate,
            Self::NicDown,
            Self::Ipv6Write,
            Self::Ipv6Read,
            Self::ArpWrite,
            Self::ArpRead,
            Self::IoctlSocket,
            Self::Address,
            Self::Netmask,
            Self::Link,
            Self::Route,
            Self::Resolver,
        ];
    }

    struct FakeNetworkOps {
        fail: Option<NetworkStage>,
        false_readback: Option<NetworkStage>,
        visited: Vec<NetworkStage>,
    }

    impl FakeNetworkOps {
        fn failing(stage: NetworkStage) -> Self {
            Self { fail: Some(stage), false_readback: None, visited: Vec::new() }
        }

        fn hit(&mut self, stage: NetworkStage) -> Result<(), InitError> {
            self.visited.push(stage);
            if self.fail == Some(stage) {
                return Err(match stage {
                    NetworkStage::Enumerate
                    | NetworkStage::IoctlSocket
                    | NetworkStage::Address
                    | NetworkStage::Netmask
                    | NetworkStage::Link
                    | NetworkStage::Route => InitError::GuestNetworkSyscall {
                        operation: "injected guest network syscall",
                        source: Errno::EPERM,
                    },
                    NetworkStage::Ipv6Write
                    | NetworkStage::Ipv6Read
                    | NetworkStage::ArpWrite
                    | NetworkStage::ArpRead
                    | NetworkStage::Resolver => InitError::GuestNetworkIo {
                        operation: "injected guest network I/O",
                        source: std::io::Error::other("injected"),
                    },
                    NetworkStage::NicDown => {
                        guest_network_config_error("guest interface was already up")
                    }
                });
            }
            Ok(())
        }
    }

    impl GuestNetworkOps for FakeNetworkOps {
        type IoctlSocket = ();

        fn interface(&mut self) -> Result<String, InitError> {
            self.hit(NetworkStage::Enumerate)?;
            Ok("eth0".to_owned())
        }
        fn require_down(&mut self, _: &str) -> Result<(), InitError> {
            self.hit(NetworkStage::NicDown)
        }
        fn write_ipv6_disabled(&mut self, _: &str) -> Result<(), InitError> {
            self.hit(NetworkStage::Ipv6Write)
        }
        fn read_ipv6_disabled(&mut self, _: &str) -> Result<bool, InitError> {
            self.hit(NetworkStage::Ipv6Read)?;
            Ok(self.false_readback != Some(NetworkStage::Ipv6Read))
        }
        fn write_arp_notify_disabled(&mut self, _: &str) -> Result<(), InitError> {
            self.hit(NetworkStage::ArpWrite)
        }
        fn read_arp_notify_disabled(&mut self, _: &str) -> Result<bool, InitError> {
            self.hit(NetworkStage::ArpRead)?;
            Ok(self.false_readback != Some(NetworkStage::ArpRead))
        }
        fn open_ioctl_socket(&mut self) -> Result<Self::IoctlSocket, InitError> {
            self.hit(NetworkStage::IoctlSocket)
        }
        fn set_address(&mut self, _: &(), _: &str, _: Ipv4Addr) -> Result<(), InitError> {
            self.hit(NetworkStage::Address)
        }
        fn set_netmask(&mut self, _: &(), _: &str, _: u8) -> Result<(), InitError> {
            self.hit(NetworkStage::Netmask)
        }
        fn bring_up(&mut self, _: &(), _: &str) -> Result<(), InitError> {
            self.hit(NetworkStage::Link)
        }
        fn add_route(&mut self, _: &(), _: &str, _: Ipv4Addr) -> Result<(), InitError> {
            self.hit(NetworkStage::Route)
        }
        fn write_resolver(&mut self, _: Ipv4Addr) -> Result<(), InitError> {
            self.hit(NetworkStage::Resolver)
        }
    }

    fn config() -> GuestNetworkConfig {
        GuestNetworkConfig {
            addr: Ipv4Addr::new(100, 96, 0, 166),
            prefix_len: 30,
            gateway: Ipv4Addr::new(100, 96, 0, 165),
            dns: Ipv4Addr::new(100, 96, 0, 165),
        }
    }

    /// CONTRACT_SHAPE: bounded-change (cmdline read failure suppresses all network stages).
    #[test]
    fn cmdline_read_failure_is_pre_ready_and_closed() {
        let mut ops = FakeNetworkOps { fail: None, false_readback: None, visited: Vec::new() };
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                configure_guest_network_with(
                    || Err(std::io::Error::other("injected cmdline read failure")),
                    &mut ops,
                )
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
        assert!(ops.visited.is_empty());
    }

    fn assert_stage_failure_suppresses_later(stage: NetworkStage) {
        let mut ops = FakeNetworkOps::failing(stage);
        let (result, trace) = lifecycle_trace(
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || apply_guest_network_with(&mut ops, config()),
            || Ok(()),
        );
        let error = result.expect_err("injected stage fails");
        match stage {
            NetworkStage::Enumerate
            | NetworkStage::IoctlSocket
            | NetworkStage::Address
            | NetworkStage::Netmask
            | NetworkStage::Link
            | NetworkStage::Route => {
                assert!(matches!(error, InitError::GuestNetworkSyscall { .. }));
            }
            NetworkStage::Ipv6Write
            | NetworkStage::Ipv6Read
            | NetworkStage::ArpWrite
            | NetworkStage::ArpRead
            | NetworkStage::Resolver => {
                assert!(matches!(error, InitError::GuestNetworkIo { .. }));
            }
            NetworkStage::NicDown => {
                assert!(matches!(error, InitError::GuestNetworkConfig { .. }));
            }
        }
        let failed_at = NetworkStage::ORDERED
            .iter()
            .position(|candidate| *candidate == stage)
            .expect("stage belongs to the complete apply order");
        assert_eq!(ops.visited, NetworkStage::ORDERED[..=failed_at]);
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

    /// CONTRACT_SHAPE: bounded-change (enumeration failure suppresses all network mutation).
    #[test]
    fn interface_enumeration_failure_is_pre_ready_and_closed() {
        assert_stage_failure_suppresses_later(NetworkStage::Enumerate);
    }

    /// CONTRACT_SHAPE: bounded-change (IPv6 write failure suppresses later network mutation).
    #[test]
    fn ipv6_disable_write_failure_suppresses_later_network_setup() {
        assert_stage_failure_suppresses_later(NetworkStage::Ipv6Write);
    }

    /// CONTRACT_SHAPE: bounded-change (ARP write failure suppresses later network mutation).
    #[test]
    fn arp_notify_write_failure_suppresses_later_network_setup() {
        assert_stage_failure_suppresses_later(NetworkStage::ArpWrite);
    }

    /// CONTRACT_SHAPE: bounded-change (socket failure suppresses every static apply operation).
    #[test]
    fn ioctl_socket_failure_suppresses_all_static_apply() {
        assert_stage_failure_suppresses_later(NetworkStage::IoctlSocket);
    }

    /// CONTRACT_SHAPE: bounded-change (address failure suppresses netmask, link, route, resolver).
    #[test]
    fn address_failure_suppresses_netmask_link_route_and_resolver() {
        assert_stage_failure_suppresses_later(NetworkStage::Address);
    }

    /// CONTRACT_SHAPE: bounded-change (netmask failure suppresses link, route, resolver).
    #[test]
    fn netmask_failure_suppresses_link_route_and_resolver() {
        assert_stage_failure_suppresses_later(NetworkStage::Netmask);
    }

    /// CONTRACT_SHAPE: bounded-change (link failure suppresses route and resolver).
    #[test]
    fn link_failure_suppresses_route_and_resolver() {
        assert_stage_failure_suppresses_later(NetworkStage::Link);
    }

    /// CONTRACT_SHAPE: bounded-change (route failure suppresses resolver).
    #[test]
    fn route_failure_suppresses_resolver() {
        assert_stage_failure_suppresses_later(NetworkStage::Route);
    }

    /// CONTRACT_SHAPE: bounded-change (resolver failure suppresses READY and EXEC).
    #[test]
    fn resolver_write_failure_suppresses_ready_and_exec() {
        assert_stage_failure_suppresses_later(NetworkStage::Resolver);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn assigned_guest_network_rejects_missing_platform_token() {
        proptest!(|(noise in prop::collection::vec(any::<u8>(), 0..128))| {
            let cmdline = format!("console={noise:?}");
            let rejected = matches!(
                required_guest_network_config(&cmdline),
                Err(InitError::GuestNetworkConfig { .. })
            );
            prop_assert!(rejected);
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn guest_network_token_rejects_malformed_address() {
        proptest!(|(noise in prop::collection::vec(any::<u8>(), 0..64))| {
            let token = format!(
                "overdrive.net=invalid-{noise:?}/30,gw=1.1.1.1,dns=1.1.1.1"
            );
            prop_assert!(parse_guest_network_cmdline(&token).is_err());
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn guest_network_token_rejects_invalid_prefix() {
        proptest!(|(offset in any::<u16>())| {
            let prefix = 33_u32 + u32::from(offset);
            let token = format!(
                "overdrive.net=1.1.1.2/{prefix},gw=1.1.1.1,dns=1.1.1.1"
            );
            prop_assert!(parse_guest_network_cmdline(&token).is_err());
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn guest_network_token_rejects_malformed_gateway() {
        proptest!(|(noise in prop::collection::vec(any::<u8>(), 0..64))| {
            let token = format!(
                "overdrive.net=1.1.1.2/30,gw=invalid-{noise:?},dns=1.1.1.1"
            );
            prop_assert!(parse_guest_network_cmdline(&token).is_err());
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn guest_network_token_rejects_malformed_dns() {
        proptest!(|(noise in prop::collection::vec(any::<u8>(), 0..64))| {
            let token = format!(
                "overdrive.net=1.1.1.2/30,gw=1.1.1.1,dns=invalid-{noise:?}"
            );
            prop_assert!(parse_guest_network_cmdline(&token).is_err());
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn network_admission_rejects_an_interface_that_is_already_up() {
        proptest!(|(flags in any::<i16>())| {
            let up = libc::c_short::try_from(libc::IFF_UP).unwrap();
            prop_assert_eq!(require_interface_down(flags).is_err(), flags & up != 0);
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn ipv6_readback_must_confirm_disabled() {
        proptest!(|(confirmed in any::<bool>())| {
            let mut ops = FakeNetworkOps {
                fail: None,
                false_readback: (!confirmed).then_some(NetworkStage::Ipv6Read),
                visited: Vec::new(),
            };
            prop_assert_eq!(apply_guest_network_with(&mut ops, config()).is_ok(), confirmed);
            if !confirmed {
                prop_assert_eq!(ops.visited.last(), Some(&NetworkStage::Ipv6Read));
            }
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn arp_notify_readback_must_confirm_zero() {
        proptest!(|(confirmed in any::<bool>())| {
            let mut ops = FakeNetworkOps {
                fail: None,
                false_readback: (!confirmed).then_some(NetworkStage::ArpRead),
                visited: Vec::new(),
            };
            prop_assert_eq!(apply_guest_network_with(&mut ops, config()).is_ok(), confirmed);
            if !confirmed {
                prop_assert_eq!(ops.visited.last(), Some(&NetworkStage::ArpRead));
            }
        });
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn ready_requires_every_guest_init_stage() {
        use std::cell::RefCell;
        let lifecycle = RefCell::new(Vec::new());
        let mut ops = FakeNetworkOps { fail: None, false_readback: None, visited: Vec::new() };
        complete_guest_lifecycle(
            || {
                lifecycle.borrow_mut().push(PreReadyStage::Root);
                Ok(())
            },
            || {
                lifecycle.borrow_mut().push(PreReadyStage::Modules);
                Ok(())
            },
            || {
                lifecycle.borrow_mut().push(PreReadyStage::Connect);
                Ok(())
            },
            || {
                lifecycle.borrow_mut().push(PreReadyStage::Network);
                apply_guest_network_with(&mut ops, config())
            },
            |_| {
                lifecycle.borrow_mut().push(PreReadyStage::Ready);
                Ok(())
            },
            |_| {
                lifecycle.borrow_mut().push(PreReadyStage::Exec);
                Ok(vec!["/bin/true".to_owned()])
            },
            |_| {
                lifecycle.borrow_mut().push(PreReadyStage::Operator);
                Ok(0)
            },
            |_, _| {
                lifecycle.borrow_mut().push(PreReadyStage::Exit);
                Ok(())
            },
            |_| {
                lifecycle.borrow_mut().push(PreReadyStage::Shutdown);
                Ok(())
            },
            || {
                lifecycle.borrow_mut().push(PreReadyStage::PowerOff);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            lifecycle.into_inner(),
            [
                PreReadyStage::Root,
                PreReadyStage::Modules,
                PreReadyStage::Connect,
                PreReadyStage::Network,
                PreReadyStage::Ready,
                PreReadyStage::Exec,
                PreReadyStage::Operator,
                PreReadyStage::Exit,
                PreReadyStage::Shutdown,
                PreReadyStage::PowerOff,
            ]
        );
        assert_eq!(
            ops.visited,
            [
                NetworkStage::Enumerate,
                NetworkStage::NicDown,
                NetworkStage::Ipv6Write,
                NetworkStage::Ipv6Read,
                NetworkStage::ArpWrite,
                NetworkStage::ArpRead,
                NetworkStage::IoctlSocket,
                NetworkStage::Address,
                NetworkStage::Netmask,
                NetworkStage::Link,
                NetworkStage::Route,
                NetworkStage::Resolver,
            ]
        );
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn arbitrary_malformed_cmdline_bytes_cannot_produce_a_network_plan(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
            let mut ops = FakeNetworkOps { fail: None, false_readback: None, visited: Vec::new() };
            let result = configure_guest_network_with(|| Ok(bytes.clone()), &mut ops);
            if std::str::from_utf8(&bytes).is_err() {
                let rejected_as_decode = matches!(
                    result,
                    Err(InitError::GuestNetworkIo {
                        operation: "decode /proc/cmdline as UTF-8",
                        ..
                    })
                );
                prop_assert!(rejected_as_decode);
                prop_assert!(ops.visited.is_empty());
            }
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn guest_network_field_boundaries_accept_only_the_pinned_grammar(
            addr in any::<[u8; 4]>(), gateway in any::<[u8; 4]>(), dns in any::<[u8; 4]>(), prefix in 0_u8..=40,
        ) {
            let token = format!(
                "overdrive.net={}/{}{},gw={},dns={}",
                Ipv4Addr::from(addr),
                prefix,
                "",
                Ipv4Addr::from(gateway),
                Ipv4Addr::from(dns),
            );
            let parsed = parse_guest_network_cmdline(&token);
            prop_assert_eq!(parsed.is_ok(), prefix <= 32);
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn nic_admission_is_exactly_the_iff_up_bit(flags in any::<i16>()) {
            let up = libc::c_short::try_from(libc::IFF_UP).unwrap();
            prop_assert_eq!(require_interface_down(flags).is_err(), flags & up != 0);
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn suppression_readbacks_gate_every_static_mutation(ipv6_disabled in any::<bool>(), arp_quiet in any::<bool>()) {
            let false_readback = if !ipv6_disabled {
                Some(NetworkStage::Ipv6Read)
            } else if !arp_quiet {
                Some(NetworkStage::ArpRead)
            } else {
                None
            };
            let mut ops = FakeNetworkOps { fail: None, false_readback, visited: Vec::new() };
            let result = apply_guest_network_with(&mut ops, config());
            prop_assert_eq!(result.is_ok(), ipv6_disabled && arp_quiet);
            if !ipv6_disabled {
                prop_assert_eq!(ops.visited.last(), Some(&NetworkStage::Ipv6Read));
            } else if !arp_quiet {
                prop_assert_eq!(ops.visited.last(), Some(&NetworkStage::ArpRead));
            }
        }
    }
}
