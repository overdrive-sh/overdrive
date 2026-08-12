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
//! host  -> guest  "EXEC <json-argv>"             exactly once, after READY, before exec
//! guest -> host   "EXIT <i32>"                   exactly once, after waitpid
//! host  -> guest  "SHUTDOWN"                     at most once (§D4)
//!                 EOF                             terminates the session
//! ```
//!
//! `EXEC` (ADR-0082 §D7 amendment 2026-08-12, GH #42) carries the
//! operator's command to the guest — the kernel cmdline never does. Its
//! payload is a JSON-encoded `argv: Vec<String>` (`argv[0]` the program),
//! chosen over space-tokenizing because the operator's arguments may
//! themselves contain spaces or embedded newlines; JSON string escaping
//! keeps the encoded line single-line by construction, preserving the
//! module's one-message-per-line framing.
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
    /// The first token was not `READY`, `EXIT`, `SHUTDOWN`, or an `EXEC`
    /// carrying a payload (a bare `EXEC` with no trailing-space payload
    /// falls through to this generic bucket too, since it never reaches
    /// the dedicated `EXEC ` prefix match).
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
    /// `EXEC`'s JSON-decoded argv was the empty array — there is no
    /// `argv[0]` to exec, so this is invalid by construction rather than
    /// a valid-but-degenerate command.
    #[error("EXEC argv must not be empty in line {raw:?}")]
    EmptyArgv {
        /// The full line that failed to parse.
        raw: String,
    },
    /// `EXEC`'s payload was not valid JSON, or not a JSON array of
    /// strings. Carries the JSON error's `Display` text as `detail`
    /// rather than embedding `serde_json::Error` itself via `#[source]`
    /// — `serde_json::Error` is neither `Clone` nor `PartialEq`, and
    /// embedding it would break this enum's derives.
    #[error("EXEC payload is not a valid JSON string array ({detail}) in line {raw:?}")]
    MalformedArgv {
        /// The full line that failed to parse.
        raw: String,
        /// The underlying JSON-parse failure's `Display` text.
        detail: String,
    },
}

/// One message on the beacon wire. Both directions share this one type
/// (§D7's "no second parser") — [`Ready`](Self::Ready) and
/// [`Exit`](Self::Exit) travel guest → host, [`Exec`](Self::Exec) and
/// [`Shutdown`](Self::Shutdown) travel host → guest; nothing on the wire
/// itself distinguishes direction beyond which endpoint reads or writes
/// it.
///
/// Not `Copy` — [`Exec`](Self::Exec)'s `Vec<String>` cannot be. Call
/// sites that relied on implicit copies move or `.clone()` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Sent at most once, host → guest, immediately after `READY` is
    /// accepted and before the guest execs anything (ADR-0082 §D7
    /// amendment 2026-08-12, GH #42). Carries the operator's command as
    /// a JSON-encoded argv on the wire (`EXEC <json-argv>`); `argv[0]`
    /// is the program, `argv` the full vector. This is how
    /// `overdrive-init` learns what to exec — the kernel cmdline never
    /// carries it (`KernelCmdline` stays platform-only, ADR-0082 §D2).
    Exec {
        /// The operator's command and its arguments. `argv[0]` is the
        /// program to exec.
        argv: Vec<String>,
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
            Self::Exec { argv } => {
                // `Vec<String>` serialisation is total; the `fmt::Error`
                // arm is the unreachable sentinel — never `.expect()`
                // (`core` library crate, no panics on the happy path).
                write!(f, "EXEC {}", serde_json::to_string(argv).map_err(|_| fmt::Error)?)
            }
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

        // `EXEC`'s payload is a single JSON array that may itself
        // contain spaces — decode the raw remainder as one JSON blob
        // rather than participating in the generic space-tokenizer
        // below (ADR-0082 §D7 amendment: "does NOT space-tokenize the
        // payload", unlike READY/EXIT/SHUTDOWN).
        if let Some(json) = line.strip_prefix("EXEC ") {
            return parse_exec(line, json);
        }

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

/// Parses `EXEC`'s payload — the raw JSON-array remainder after the
/// `"EXEC "` prefix, never space-tokenized (ADR-0082 §D7 amendment). An
/// empty argv is rejected structurally: there is no `argv[0]` to exec.
fn parse_exec(raw: &str, json: &str) -> Result<BeaconMessage, BeaconParseError> {
    let argv: Vec<String> = serde_json::from_str(json).map_err(|source| {
        BeaconParseError::MalformedArgv { raw: raw.to_string(), detail: source.to_string() }
    })?;
    if argv.is_empty() {
        return Err(BeaconParseError::EmptyArgv { raw: raw.to_string() });
    }
    Ok(BeaconMessage::Exec { argv })
}
