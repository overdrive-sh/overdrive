//! `overdrive-init` — the in-guest PID 1 agent (ADR-0082 §D7, §D4, GH #42).
//!
//! Six duties, matching ADR-0082 §D4's guest half of the beacon
//! lifecycle plus the §D7 amendment (2026-08-12) that pins the
//! operator-command channel:
//!
//! 1. Dial the host over the beacon vsock connection
//!    ([`beacon::VMADDR_CID_HOST`] / [`beacon::BEACON_VSOCK_PORT`]).
//! 2. Send `READY` — exactly once, before exec'ing anything.
//! 3. Block for exactly one host -> guest message and require it to be
//!    `EXEC { argv }` — the operator's command arrives here, over the
//!    beacon, never on the kernel cmdline (`KernelCmdline` stays
//!    platform-only, ADR-0082 §D2).
//! 4. Exec `argv[0]` with `argv`, forwarding stdio untouched.
//! 5. Send `EXIT <status>` with the command's real exit status once it
//!    resolves — never validating the operator's own I/O.
//! 6. Read at most one line for a `SHUTDOWN` request (or `EOF`), then
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

    send(&mut conn, &BeaconMessage::Ready { pid, port: beacon::BEACON_VSOCK_PORT })?;

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
    /// Creating the `AF_VSOCK` beacon socket failed.
    #[error("could not create the beacon vsock socket: {0}")]
    Socket(#[source] nix::errno::Errno),
    /// Connecting the beacon socket to the host failed.
    #[error("could not connect the beacon vsock socket to the host: {0}")]
    Connect(#[source] nix::errno::Errno),
    /// A read or write on the beacon connection failed.
    #[error("beacon connection I/O failed: {0}")]
    Io(#[source] std::io::Error),
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
        unreachable!("recv_exec only returns argv already validated non-empty by BeaconMessage::from_str")
    });
    let status = Command::new(command).args(&argv[1..]).status().map_err(|source| {
        InitError::Spawn { command: command.clone(), source }
    })?;
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
