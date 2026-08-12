//! `overdrive-init` — the in-guest PID 1 agent (ADR-0082 §D7, §D4, GH #42).
//!
//! Five duties, matching ADR-0082 §D4's guest half of the beacon
//! lifecycle:
//!
//! 1. Dial the host over the beacon vsock connection
//!    ([`beacon::VMADDR_CID_HOST`] / [`beacon::BEACON_VSOCK_PORT`]).
//! 2. Send `READY` — exactly once, before exec'ing anything.
//! 3. Exec the operator's command, forwarding stdio untouched.
//! 4. Send `EXIT <status>` with the command's real exit status once it
//!    resolves — never validating the operator's own I/O.
//! 5. Read at most one line for a `SHUTDOWN` request (or `EOF`), then
//!    power off (`reboot(RB_POWER_OFF)`).
//!
//! # Scope note (step 01-03)
//!
//! This step lands the crate and its beacon-speaking logic; the real
//! end-to-end boot (a real kernel, a real Cloud Hypervisor vsock device,
//! a real operator command) is exercised at step 01-08 under Tier-3. Two
//! things this file deliberately does not attempt, both out of this
//! step's design surface:
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
//! - **How the guest learns the operator's `command`/`args`.** No ADR
//!   pins this channel yet — `KernelCmdline` explicitly excludes them
//!   (ADR-0082 §D2, gap 4: "`[vm]` carries kernel/rootfs/command/args,
//!   never a cmdline"), and `feature-delta.md:213-214` states only that
//!   "the guest agent execs them inside the guest" without naming how
//!   they arrive. Host-side wiring (`VmDriver`, step 01-07) — which any
//!   real channel needs on the other end — is explicitly out of this
//!   step's boundary. [`operator_command`] reads this process's own
//!   `argv` as the minimal, protocol-non-inventing placeholder: no new
//!   `BeaconMessage` variant, no new virtio device, no new rootfs
//!   convention. Whatever real channel a later step supplies, only that
//!   one function needs to change.

// A minimal PID 1 has no `tracing` sink, no log aggregation, and no
// operator shell — `/dev/console` (fd 0/1/2, held before devtmpfs is up
// per ADR-0082 §D7) IS the diagnostic channel. `eprintln!` on the
// emergency path is the intended mechanism, not a stopgap. Matches the
// project's own "allowed in CLI/binary-boundary crates via crate-level
// override" pattern (`.claude/rules/development.md` § Cargo.toml
// conventions on `expect_used`).
#![allow(clippy::print_stderr)]
// See `main.rs`'s module doc — every syscall this binary needs
// (`AF_VSOCK` socket/connect, `reboot(2)`) has a safe nix wrapper, and
// `std::process::Command` (not raw `fork`/`exec`) spawns the operator's
// command. Zero `unsafe` is required anywhere in this crate.
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use nix::sys::reboot::{self, RebootMode};
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, VsockAddr};
use overdrive_core::vm::beacon::{self, BeaconMessage};

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
    let mut conn = connect_beacon()?;

    send(&mut conn, BeaconMessage::Ready { pid, port: beacon::BEACON_VSOCK_PORT })?;

    let status = exec_operator_command()?;
    send(&mut conn, BeaconMessage::Exit { status })?;

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
    /// Creating the `AF_VSOCK` beacon socket failed.
    #[error("could not create the beacon vsock socket: {0}")]
    Socket(#[source] nix::errno::Errno),
    /// Connecting the beacon socket to the host failed.
    #[error("could not connect the beacon vsock socket to the host: {0}")]
    Connect(#[source] nix::errno::Errno),
    /// A read or write on the beacon connection failed.
    #[error("beacon connection I/O failed: {0}")]
    Io(#[source] std::io::Error),
    /// Spawning the operator's command failed.
    #[error("could not spawn the operator's command: {0}")]
    Spawn(#[source] std::io::Error),
    /// `reboot(RB_POWER_OFF)` failed.
    #[error("reboot(RB_POWER_OFF) failed: {0}")]
    Reboot(#[source] nix::errno::Errno),
}

/// Opens the guest -> host beacon connection: `AF_VSOCK`, dialing
/// [`beacon::VMADDR_CID_HOST`] on [`beacon::BEACON_VSOCK_PORT`]
/// (ADR-0082 §§D2.2/D7).
fn connect_beacon() -> Result<File, InitError> {
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
fn send(conn: &mut File, msg: BeaconMessage) -> Result<(), InitError> {
    let line = format!("{msg}\n");
    conn.write_all(line.as_bytes()).map_err(InitError::Io)
}

/// Reads at most one line off the beacon connection and, if present,
/// parses it as a [`BeaconMessage`]. Tolerates `EOF` (`Ok(None)`) and a
/// malformed or unexpected line (folded into `Ok(None)` too) — either
/// way the guest's next and only remaining action is to power off
/// (ADR-0082 §D4).
fn read_shutdown_or_eof(conn: &File) -> Result<Option<BeaconMessage>, InitError> {
    // A second handle over the SAME fd (`dup`) — `try_clone` only needs
    // `&self`, so `conn` need not be `&mut` even though the caller also
    // uses it (via a separate `&mut` borrow) to write `READY`/`EXIT`.
    let read_handle = conn.try_clone().map_err(InitError::Io)?;
    let mut reader = BufReader::new(read_handle);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).map_err(InitError::Io)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(line.parse().ok())
}

/// Execs the operator's command, forwarding stdio untouched (the
/// default `std::process::Command` behaviour — the guest kernel opens
/// `/dev/console` as fd 0/1/2 for init before devtmpfs is up, ADR-0082
/// §D7, so inheriting IS forwarding), and returns the guest's own
/// encoding of the real exit status. Never inspects or validates the
/// operator's own stdout/stderr/exit code beyond encoding it onto the
/// wire.
fn exec_operator_command() -> Result<i32, InitError> {
    let (command, args) = operator_command();
    let status = Command::new(command).args(args).status().map_err(InitError::Spawn)?;
    Ok(exit_status_to_wire(status))
}

/// Where `overdrive-init` learns the operator's `command`/`args` — see
/// this module's doc "Scope note" for why this reads `argv` rather than
/// a real per-launch channel. `argv[0]` is this process's own
/// invocation path (skipped); `argv[1]` is the command, `argv[2..]` its
/// arguments.
fn operator_command() -> (String, Vec<String>) {
    let mut argv = std::env::args().skip(1);
    let command = argv.next().unwrap_or_default();
    (command, argv.collect())
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
