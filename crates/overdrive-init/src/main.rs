//! `overdrive-init` — the in-guest PID 1 agent (ADR-0082 §D7, §D4, GH #42).
//!
//! Eight duties, matching ADR-0082 §D4's guest half of the beacon
//! lifecycle plus the §D7 amendment (2026-08-12) that pins the
//! operator-command channel, plus the §D4 amendment (2026-08-14,
//! DISTILL DWD-21) that pins the guest vsock transport's load path:
//!
//! 0. Load the guest vsock transport modules ([`load_vsock_modules`])
//!    — a no-op on a vsock=y appliance kernel (ADR-0068 §4), required
//!    on the stock `CONFIG_VSOCKETS=m` kernel the Tier-3 fixture
//!    stages.
//! 1. Dial the host over the beacon vsock connection
//!    ([`beacon::VMADDR_CID_HOST`] / [`beacon::BEACON_VSOCK_PORT`]),
//!    retrying with a bounded backoff — the virtio-vsock PCI probe
//!    completes asynchronously after step 0's module load.
//! 2. Send `READY` — exactly once, before exec'ing anything.
//! 3. If the platform supplied `overdrive.net=`, apply the guest address,
//!    netmask, link state, default route, and resolver configuration. A
//!    failure reports non-zero `EXIT` and stops before operator exec.
//! 4. Block for exactly one host -> guest message and require it to be
//!    `EXEC { argv }` — the operator's command arrives here, over the
//!    beacon, never on the kernel cmdline (`KernelCmdline` stays
//!    platform-only, ADR-0082 §D2).
//! 5. Exec `argv[0]` with `argv`, forwarding stdio untouched.
//! 6. Send `EXIT <status>` with the command's real exit status once it
//!    resolves — never validating the operator's own I/O.
//! 7. Read at most one line for a `SHUTDOWN` request (or `EOF`), then
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
use std::process::Command;
use std::time::Duration;

use nix::errno::Errno;
use nix::kmod::{ModuleInitFlags, finit_module};
use nix::libc;
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

const GUEST_NETWORK_FAILURE_STATUS: i32 = 78;

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
    load_vsock_modules()?;
    let mut conn = connect_beacon()?;

    send(&mut conn, &BeaconMessage::Ready { pid, port: beacon::BEACON_VSOCK_PORT })?;

    if let Err(err) = configure_guest_network() {
        eprintln!("overdrive-init: guest network setup failed before EXEC: {err}");
        send(&mut conn, &BeaconMessage::Exit { status: GUEST_NETWORK_FAILURE_STATUS })?;
        return Err(err);
    }

    let argv = recv_exec(&conn)?;
    let status = exec_operator_command(&argv)?;
    send(&mut conn, &BeaconMessage::Exit { status })?;

    // At most one SHUTDOWN, or EOF — either way there is nothing left
    // to do but power off (ADR-0082 §D4). The parsed value is
    // deliberately unused: this call exists so the socket's "at most
    // one SHUTDOWN, or EOF" contract is genuinely read through the
    // shared Published Language parser, not assumed away.
    let _ = read_shutdown_or_eof(&conn)?;

    reboot::reboot(RebootMode::RB_POWER_OFF).map_err(InitError::Reboot)?;
    Ok(())
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

fn guest_network_config_error(detail: impl Into<String>) -> InitError {
    InitError::GuestNetworkConfig { detail: detail.into() }
}

fn parse_ipv4(raw: &str, field: &'static str) -> Result<Ipv4Addr, InitError> {
    raw.parse().map_err(|_| guest_network_config_error(format!("{field} is not valid IPv4")))
}

fn configure_guest_network() -> Result<(), InitError> {
    let cmdline = fs::read_to_string("/proc/cmdline")
        .map_err(|source| InitError::GuestNetworkIo { operation: "read /proc/cmdline", source })?;
    let Some(config) = parse_guest_network_cmdline(&cmdline)? else {
        return Ok(());
    };
    apply_guest_network(config)
}

#[allow(
    unsafe_code,
    reason = "two synchronous address/netmask ioctls use initialized ifreq values with bounded lifetimes"
)]
fn apply_guest_network(config: GuestNetworkConfig) -> Result<(), InitError> {
    let interface = single_non_loopback_interface()?;
    let ioctl_socket =
        socket::socket(AddressFamily::Inet, SockType::Datagram, SockFlag::empty(), None).map_err(
            |source| InitError::GuestNetworkSyscall {
                operation: "create IPv4 ioctl socket",
                source,
            },
        )?;
    let fd = ioctl_socket.as_raw_fd();

    let address_request = ifreq_with_address(&interface, config.addr)?;
    // SAFETY: `address_request` is a fully initialized Linux `ifreq`, its
    // pointer remains valid for the synchronous ioctl, and `fd` owns an
    // AF_INET datagram socket for the duration of the call.
    unsafe { set_interface_address(fd, &raw const address_request) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "set guest IPv4 address", source }
    })?;

    let netmask_request = ifreq_with_address(&interface, prefix_netmask(config.prefix_len))?;
    // SAFETY: same initialized-ifreq/socket lifetime argument as the address
    // ioctl above; SIOCSIFNETMASK reads but does not retain this pointer.
    unsafe { set_interface_netmask(fd, &raw const netmask_request) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "set guest IPv4 netmask", source }
    })?;

    bring_interface_up(fd, &interface)?;
    add_default_route(fd, &interface, config.gateway)?;

    fs::write("/etc/resolv.conf", format!("nameserver {}\n", config.dns)).map_err(|source| {
        InitError::GuestNetworkIo { operation: "write /etc/resolv.conf", source }
    })?;
    Ok(())
}

fn single_non_loopback_interface() -> Result<String, InitError> {
    let entries = fs::read_dir("/sys/class/net")
        .map_err(|source| InitError::GuestNetworkIo { operation: "read /sys/class/net", source })?;
    let mut interfaces = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| InitError::GuestNetworkIo {
            operation: "enumerate /sys/class/net",
            source,
        })?;
        let name = entry.file_name().into_string().map_err(|name| {
            guest_network_config_error(format!(
                "network interface name is not UTF-8: {}",
                name.to_string_lossy()
            ))
        })?;
        if name != "lo" {
            interfaces.push(name);
        }
    }
    interfaces.sort_unstable();
    match interfaces.as_slice() {
        [interface] => Ok(interface.clone()),
        [] => Err(guest_network_config_error("no non-loopback network interface was present")),
        _ => Err(guest_network_config_error(format!(
            "more than one non-loopback network interface was present: {}",
            interfaces.join(",")
        ))),
    }
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

#[allow(
    unsafe_code,
    reason = "the flags ioctl pair and union read are isolated here and locally justified"
)]
fn bring_interface_up(fd: libc::c_int, interface: &str) -> Result<(), InitError> {
    let mut request = ifreq_with_flags(interface, 0)?;
    // SAFETY: `request` is initialized with the correct Linux `ifreq` layout,
    // is uniquely borrowed for the call, and the kernel only mutates it before
    // returning synchronously.
    unsafe { get_interface_flags(fd, &raw mut request) }.map_err(|source| {
        InitError::GuestNetworkSyscall { operation: "read guest interface flags", source }
    })?;
    // SAFETY: SIOCGIFFLAGS initialized the flags member of the union above.
    let up_flag = libc::c_short::try_from(libc::IFF_UP)
        .map_err(|_| guest_network_config_error("Linux IFF_UP does not fit ifreq flags"))?;
    let flags = unsafe { request.ifr_ifru.ifru_flags } | up_flag;
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
    for filename in beacon::GUEST_VSOCK_MODULE_FILES {
        let path = format!("{}/{filename}", beacon::GUEST_VSOCK_MODULE_DIR);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(InitError::ModuleOpen { path, source }),
        };
        match finit_module(&file, c"", ModuleInitFlags::empty()) {
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
    let mut last_err = Errno::UnknownErrno;
    for _attempt in 0..VSOCK_CONNECT_MAX_ATTEMPTS {
        match connect_beacon_once() {
            Ok(conn) => return Ok(conn),
            Err(InitError::Socket(errno) | InitError::Connect(errno)) => {
                last_err = errno;
                std::thread::sleep(VSOCK_CONNECT_RETRY_DELAY);
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
    let sock = socket::socket(AddressFamily::Vsock, SockType::Stream, SockFlag::empty(), None)
        .map_err(InitError::Socket)?;
    let addr = VsockAddr::new(beacon::VMADDR_CID_HOST, beacon::BEACON_VSOCK_PORT.as_u32());
    socket::connect(sock.as_raw_fd(), &addr).map_err(InitError::Connect)?;
    // `OwnedFd -> File` is a safe conversion (io-safety: the `OwnedFd`
    // already guarantees exclusive ownership of a valid fd). Wrapping
    // the raw vsock fd in `File` gives ordinary `Read`/`Write` access —
    // the fd's socket family is irrelevant to the `read`/`write`
    // syscalls `File` issues.
    Ok(File::from(sock))
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
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn guest_network_parser_reads_the_platforms_single_space_free_token() {
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
    fn guest_network_parser_leaves_non_mesh_cmdlines_unchanged() {
        assert_eq!(
            parse_guest_network_cmdline("console=ttyS0 panic=1 root=/dev/vda rw").unwrap(),
            None,
        );
    }
}
