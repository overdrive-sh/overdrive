//! The host↔guest vsock beacon Published Language (ADR-0082 §D7, GH #42).
//!
//! A pure module depended on by both `overdrive-init` (the guest-side
//! PID-1 agent) and the host-side beacon session (`VmDriver`, landing
//! step 01-07) so the wire format lives in exactly one place and the two
//! sides cannot drift (Hera's DD-6). Line-oriented ASCII; every [`BeaconMessage`]
//! renders as ONE line via [`Display`](fmt::Display) and parses back via
//! [`FromStr`] — the caller owns line framing (a trailing `\n` on write;
//! `BufRead::read_line` or equivalent on read), matching the "two
//! distinct reads for READY and EXIT" observation (`spike/findings.md`,
//! P2: `separate_reads=2`) — the wire is never parsed out of one blob.
//!
//! ```text
//! guest -> host   "READY pid=<u32> port=<u32>"   exactly once, before exec
//! guest -> host   "EXIT <i32>"                   exactly once, after waitpid
//! host  -> guest  "SHUTDOWN"                     at most once (§D4)
//!                 EOF                             terminates the session
//! ```
//!
//! The `READY` key-value payload (`pid=`, `port=`) matches the ONLY
//! concrete instantiation of ADR-0082 §D7's generic `"READY
//! <k=v>...\n"` notation anywhere in this feature's design record: the
//! spike's measured capture (`spike/findings.md:230`,
//! `"READY pid=1 port=1234\n"`), which §D7 itself cites as supporting
//! evidence (P2). The two field names and their order are pinned from
//! that evidence, not invented.

use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use crate::vm::config::VsockPort;

/// The fixed vsock port every VM's beacon connection dials — the value
/// [`crate::vm::config::VmRunDir::beacon_socket`] is called with. A
/// platform-wide policy constant, not a per-launch derivation: mirrors
/// `VM_BOOT_DEADLINE`'s policy-constant shape (ADR-0082 §D3) — there is
/// no per-workload input to persist. Matches the spike's measured probe
/// port (`spike/findings.md`, P2). Defined here — not in
/// `overdrive-worker`, which is not on the guest's dependency path — so
/// both `overdrive-init` and the host-side driver reference the same
/// literal value with no risk of drift.
pub const BEACON_VSOCK_PORT: VsockPort = VsockPort::new(1234);

/// The vsock CID that always addresses "the host" from a guest's
/// perspective. ABI-stable per `linux/vm_sockets.h`'s
/// `VMADDR_CID_HOST` — a kernel ABI constant, not an Overdrive-specific
/// choice.
pub const VMADDR_CID_HOST: u32 = 2;

/// A beacon line that failed to parse. Carries the raw line and the
/// specific field that rejected it so a caller can log the exact wire
/// content that misbehaved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BeaconParseError {
    /// The line had no content at all.
    #[error("empty beacon line")]
    Empty,
    /// The first token was not `READY`, `EXIT`, or `SHUTDOWN`.
    #[error("unrecognised beacon message kind {kind:?} in line {raw:?}")]
    UnknownKind {
        /// The unrecognised first token.
        kind: String,
        /// The full line that failed to parse.
        raw: String,
    },
    /// The message kind's field count did not match its wire shape.
    #[error("{kind} message expects {expected} field(s), found {actual} in line {raw:?}")]
    FieldCount {
        /// The message kind (`"READY"` / `"EXIT"` / `"SHUTDOWN"`).
        kind: &'static str,
        /// The number of fields this kind's wire shape requires.
        expected: usize,
        /// The number of fields actually present.
        actual: usize,
        /// The full line that failed to parse.
        raw: String,
    },
    /// A `key=value` field was missing its `key=` prefix.
    #[error("{kind} field {field} must be formatted as {field}=<value>, found {found:?} in line {raw:?}")]
    MissingKey {
        /// The message kind (`"READY"` / `"EXIT"` / `"SHUTDOWN"`).
        kind: &'static str,
        /// The field name the missing prefix was expected to carry.
        field: &'static str,
        /// The token actually found in that position.
        found: String,
        /// The full line that failed to parse.
        raw: String,
    },
    /// A field expected to hold an integer did not parse as one.
    #[error("{kind} field {field} is not a valid integer: {source} in line {raw:?}")]
    InvalidInt {
        /// The message kind (`"READY"` / `"EXIT"` / `"SHUTDOWN"`).
        kind: &'static str,
        /// The field name whose value failed to parse.
        field: &'static str,
        /// The full line that failed to parse.
        raw: String,
        /// The underlying integer-parse failure.
        #[source]
        source: ParseIntError,
    },
}

/// One message on the beacon wire. Both directions share this one type
/// (§D7's "no second parser") — [`Ready`](Self::Ready) and
/// [`Exit`](Self::Exit) travel guest → host, [`Shutdown`](Self::Shutdown)
/// travels host → guest; nothing on the wire itself distinguishes
/// direction beyond which endpoint reads or writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeaconMessage {
    /// Sent exactly once, guest → host, before the guest execs the
    /// operator's command. `pid` is the guest agent's own process id
    /// (`1`, since the agent IS PID 1); `port` echoes the vsock port the
    /// guest dialed to reach this connection.
    Ready {
        /// The guest agent's own process id.
        pid: u32,
        /// The vsock port the guest dialed.
        port: VsockPort,
    },
    /// Sent exactly once, guest → host, after the guest's `waitpid` on
    /// the operator's command resolves. `status` carries the guest's own
    /// encoding of the real exit outcome (an ordinary exit code, or a
    /// signal-termination convention) — the wire only pins "a signed
    /// decimal integer"; `overdrive-init` owns how it is computed.
    Exit {
        /// The guest's encoded real exit status.
        status: i32,
    },
    /// Sent at most once, host → guest, requesting a graceful shutdown
    /// (ADR-0082 §D4).
    Shutdown,
}

impl fmt::Display for BeaconMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready { pid, port } => write!(f, "READY pid={pid} port={port}"),
            Self::Exit { status } => write!(f, "EXIT {status}"),
            Self::Shutdown => f.write_str("SHUTDOWN"),
        }
    }
}

impl FromStr for BeaconMessage {
    type Err = BeaconParseError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        // Tolerate a trailing line terminator a caller read via
        // `read_until(b'\n', ..)` rather than `BufRead::lines()` (which
        // already strips it). `Display` never emits one, so this branch
        // never fires on a round-tripped message.
        let line = line.trim_end_matches(['\n', '\r']);
        let mut tokens = line.split(' ').filter(|token| !token.is_empty());
        let kind = tokens.next().ok_or(BeaconParseError::Empty)?;
        let rest: Vec<&str> = tokens.collect();

        match kind {
            "READY" => parse_ready(line, &rest),
            "EXIT" => parse_exit(line, &rest),
            "SHUTDOWN" => parse_shutdown(line, &rest),
            other => {
                Err(BeaconParseError::UnknownKind { kind: other.to_string(), raw: line.to_string() })
            }
        }
    }
}

/// Checks `rest`'s length against `kind`'s wire shape, returning a
/// [`BeaconParseError::FieldCount`] on mismatch. Factored out of
/// `parse_ready` / `parse_exit` / `parse_shutdown`, which otherwise
/// repeated this construction identically save for `kind` and
/// `expected`.
fn check_field_count(
    kind: &'static str,
    expected: usize,
    rest: &[&str],
    raw: &str,
) -> Result<(), BeaconParseError> {
    if rest.len() == expected {
        return Ok(());
    }
    Err(BeaconParseError::FieldCount { kind, expected, actual: rest.len(), raw: raw.to_string() })
}

/// Strips a `field=` prefix from `token`, returning the value substring.
fn strip_key<'a>(
    kind: &'static str,
    field: &'static str,
    token: &'a str,
    raw: &str,
) -> Result<&'a str, BeaconParseError> {
    token.strip_prefix(field).and_then(|rest| rest.strip_prefix('=')).ok_or_else(|| {
        BeaconParseError::MissingKey { kind, field, found: token.to_string(), raw: raw.to_string() }
    })
}

/// Parses a `key=<u32>` field into its integer value.
fn parse_u32_field(
    kind: &'static str,
    field: &'static str,
    token: &str,
    raw: &str,
) -> Result<u32, BeaconParseError> {
    let value = strip_key(kind, field, token, raw)?;
    value
        .parse::<u32>()
        .map_err(|source| BeaconParseError::InvalidInt { kind, field, raw: raw.to_string(), source })
}

fn parse_ready(raw: &str, rest: &[&str]) -> Result<BeaconMessage, BeaconParseError> {
    const KIND: &str = "READY";
    check_field_count(KIND, 2, rest, raw)?;
    let pid = parse_u32_field(KIND, "pid", rest[0], raw)?;
    let port = parse_u32_field(KIND, "port", rest[1], raw)?;
    Ok(BeaconMessage::Ready { pid, port: VsockPort::new(port) })
}

fn parse_exit(raw: &str, rest: &[&str]) -> Result<BeaconMessage, BeaconParseError> {
    const KIND: &str = "EXIT";
    check_field_count(KIND, 1, rest, raw)?;
    let status = rest[0].parse::<i32>().map_err(|source| BeaconParseError::InvalidInt {
        kind: KIND,
        field: "status",
        raw: raw.to_string(),
        source,
    })?;
    Ok(BeaconMessage::Exit { status })
}

fn parse_shutdown(raw: &str, rest: &[&str]) -> Result<BeaconMessage, BeaconParseError> {
    const KIND: &str = "SHUTDOWN";
    check_field_count(KIND, 0, rest, raw)?;
    Ok(BeaconMessage::Shutdown)
}
