//! The scripted guest vsock peer of the guest-kit microVM fixture (GH #295,
//! T-1, user ruling 2026-10-18): the line protocol between a host test and
//! the in-guest agent `overdrive-guest-peer` running as PID 1, and the host
//! test's client of it.
//!
//! The agent runs **no Overdrive owner**. It boots the guest kit's kernel,
//! loads the vsock transport modules (and the guest kit's `ovd_fwd.ko` when
//! the image carries it), dials the host on the control port named by the
//! kernel parameter [`CONTROL_PORT_PARAM`], and then executes one
//! [`PeerCommand`] per line, answering one [`PeerReply`] per line. With it a
//! host test plays a guest at a leased CID through the real
//! virtio-vsock ↔ vhost boundary: it opens raw vsock connections to the
//! host's listeners from chosen source ports, with or without a first record,
//! holds and ends guest TCP sockets, drives the guest's own forwarder
//! context, and crashes or powers off the guest.
//!
//! The protocol is test support (DISTILL-owned). Every name and encoding is
//! defined once here; the agent (`bin/guest_peer.rs`) and the client import
//! it.
//!
//! # Wire
//!
//! One command per `\n`-terminated line, `VERB arg…`, single spaces; bytes
//! as lowercase hex. One reply line per command, in order, except
//! [`PeerCommand::CrashKernel`] (the guest is gone). A command run with
//! [`PeerCommand::Spawn`] replies `OK` at once; its own reply is read with
//! [`PeerCommand::Join`].

#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]
#![allow(
    clippy::too_long_first_doc_paragraph,
    reason = "protocol docs open with the full sentence they define"
)]

use std::fmt::{self, Write as _};
use std::net::{Shutdown, SocketAddrV4};
use std::str::FromStr;

/// The kernel parameter that names the host control port the agent dials
/// (`overdrive.testpeer=<port>`).
pub const CONTROL_PORT_PARAM: &str = "overdrive.testpeer=";

/// Hex-encodes `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Decodes lowercase or uppercase hex.
///
/// # Errors
///
/// The offending text when it is not an even run of hex digits.
pub fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("odd-length hex: {text:?}"));
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).map_err(|e| format!("{text:?}: {e}")))
        .collect()
}

/// A resource name: `[a-z0-9_-]{1,32}`.
#[must_use]
pub fn handle_is_valid(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 32
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// One command to the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerCommand {
    /// `VCONNECT h cid port src|any`: a vsock STREAM connection to
    /// `(cid, port)`, from local port `source_port` when `Some`.
    VsockConnect {
        /// The new resource.
        handle: String,
        /// The destination CID (2: the host).
        cid: u32,
        /// The destination port.
        port: u32,
        /// The local port to bind first.
        source_port: Option<u32>,
    },
    /// `VLISTEN h port`: a vsock listener on `(VMADDR_CID_ANY, port)`.
    VsockListen {
        /// The new resource.
        handle: String,
        /// The port.
        port: u32,
    },
    /// `VACCEPT h listener ms`: the listener's next connection.
    VsockAccept {
        /// The new resource.
        handle: String,
        /// The listener.
        listener: String,
        /// How long to wait.
        timeout_ms: u64,
    },
    /// `TLISTEN h ip:port`: a TCP listener (port 0: kernel-chosen).
    TcpListen {
        /// The new resource.
        handle: String,
        /// The address.
        addr: SocketAddrV4,
    },
    /// `TCONNECT h ip:port`: a TCP connection.
    TcpConnect {
        /// The new resource.
        handle: String,
        /// The destination.
        addr: SocketAddrV4,
    },
    /// `TACCEPT h listener ms`: the TCP listener's next connection.
    TcpAccept {
        /// The new resource.
        handle: String,
        /// The listener.
        listener: String,
        /// How long to wait.
        timeout_ms: u64,
    },
    /// `WRITE h hex`: writes the bytes whole.
    Write {
        /// The socket.
        handle: String,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// `READ h max ms`: one read of at most `max` bytes.
    Read {
        /// The socket.
        handle: String,
        /// The most bytes to return.
        max: u32,
        /// How long to wait for readiness.
        timeout_ms: u64,
    },
    /// `PUMP h tag len`: writes `len` bytes of the guest-to-peer
    /// [`crate::guest_app::StreamGen`] of `tag`.
    Pump {
        /// The socket.
        handle: String,
        /// The stream's tag.
        tag: String,
        /// How many bytes.
        len: u64,
    },
    /// `DRAIN h tag ms`: reads to the end, verifying the peer-to-guest
    /// stream of `tag`; gives up after `ms` without a byte.
    Drain {
        /// The socket.
        handle: String,
        /// The stream's tag.
        tag: String,
        /// The idle bound.
        timeout_ms: u64,
    },
    /// `SHUT h rd|wr|rdwr`.
    Shutdown {
        /// The socket.
        handle: String,
        /// Which directions.
        how: Shutdown,
    },
    /// `CLOSE h`: drops the resource.
    Close {
        /// The resource.
        handle: String,
    },
    /// `RESET h`: a TCP socket closed with `SO_LINGER{1,0}` (an `RST`); a
    /// vsock socket closed while its directions are open (a `0x3`).
    Reset {
        /// The socket.
        handle: String,
    },
    /// `FOPEN ctx`: opens a guest forwarder context
    /// (`KernelFlowForwarder::open`).
    ForwarderOpen {
        /// The new context.
        ctx: String,
    },
    /// `FATTACH ctx vsock tcp`: `attach(vsock, tcp, PairMode::Stream)`.
    ForwarderAttachStream {
        /// The context.
        ctx: String,
        /// Half A, the vsock.
        vsock: String,
        /// Half B, the TCP socket.
        tcp: String,
    },
    /// `FWATCH ctx vsock`: `watch(vsock)`.
    ForwarderWatch {
        /// The context.
        ctx: String,
        /// The vsock.
        vsock: String,
    },
    /// `FABORT ctx id`: `abort(id)`.
    ForwarderAbort {
        /// The context.
        ctx: String,
        /// The pair or watch.
        id: u64,
    },
    /// `FEND ctx ms`: the next end record within `ms`.
    ForwarderTakeEnd {
        /// The context.
        ctx: String,
        /// How long to wait.
        timeout_ms: u64,
    },
    /// `FCLOSE ctx`: drops the context (its release, EVMOD-CTL).
    ForwarderClose {
        /// The context.
        ctx: String,
    },
    /// `SPAWN task <command…>`: runs the command on its own thread.
    Spawn {
        /// The task's name.
        task: String,
        /// The command.
        command: Box<Self>,
    },
    /// `JOIN task ms`: the spawned command's reply, or `PENDING`.
    Join {
        /// The task.
        task: String,
        /// How long to wait.
        timeout_ms: u64,
    },
    /// `EXEC hex…`: runs a program to completion.
    Exec {
        /// The program and its arguments.
        argv: Vec<String>,
    },
    /// `CRASH`: crashes the guest kernel (`echo c > /proc/sysrq-trigger`):
    /// with the platform's `panic=1` the guest resets and the VMM exits
    /// (RESET-EXIT). No reply.
    CrashKernel,
    /// `POWEROFF`: replies `OK`, then powers the guest off.
    PowerOff,
}

const fn shut_token(how: Shutdown) -> &'static str {
    match how {
        Shutdown::Read => "rd",
        Shutdown::Write => "wr",
        Shutdown::Both => "rdwr",
    }
}

impl fmt::Display for PeerCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VsockConnect { handle, cid, port, source_port } => {
                let src = source_port.map_or_else(|| "any".to_owned(), |p| p.to_string());
                write!(f, "VCONNECT {handle} {cid} {port} {src}")
            }
            Self::VsockListen { handle, port } => write!(f, "VLISTEN {handle} {port}"),
            Self::VsockAccept { handle, listener, timeout_ms } => {
                write!(f, "VACCEPT {handle} {listener} {timeout_ms}")
            }
            Self::TcpListen { handle, addr } => write!(f, "TLISTEN {handle} {addr}"),
            Self::TcpConnect { handle, addr } => write!(f, "TCONNECT {handle} {addr}"),
            Self::TcpAccept { handle, listener, timeout_ms } => {
                write!(f, "TACCEPT {handle} {listener} {timeout_ms}")
            }
            Self::Write { handle, bytes } => write!(f, "WRITE {handle} {}", hex(bytes)),
            Self::Read { handle, max, timeout_ms } => write!(f, "READ {handle} {max} {timeout_ms}"),
            Self::Pump { handle, tag, len } => write!(f, "PUMP {handle} {tag} {len}"),
            Self::Drain { handle, tag, timeout_ms } => {
                write!(f, "DRAIN {handle} {tag} {timeout_ms}")
            }
            Self::Shutdown { handle, how } => write!(f, "SHUT {handle} {}", shut_token(*how)),
            Self::Close { handle } => write!(f, "CLOSE {handle}"),
            Self::Reset { handle } => write!(f, "RESET {handle}"),
            Self::ForwarderOpen { ctx } => write!(f, "FOPEN {ctx}"),
            Self::ForwarderAttachStream { ctx, vsock, tcp } => {
                write!(f, "FATTACH {ctx} {vsock} {tcp}")
            }
            Self::ForwarderWatch { ctx, vsock } => write!(f, "FWATCH {ctx} {vsock}"),
            Self::ForwarderAbort { ctx, id } => write!(f, "FABORT {ctx} {id}"),
            Self::ForwarderTakeEnd { ctx, timeout_ms } => write!(f, "FEND {ctx} {timeout_ms}"),
            Self::ForwarderClose { ctx } => write!(f, "FCLOSE {ctx}"),
            Self::Spawn { task, command } => write!(f, "SPAWN {task} {command}"),
            Self::Join { task, timeout_ms } => write!(f, "JOIN {task} {timeout_ms}"),
            Self::Exec { argv } => {
                f.write_str("EXEC")?;
                for arg in argv {
                    write!(f, " {}", hex(arg.as_bytes()))?;
                }
                Ok(())
            }
            Self::CrashKernel => f.write_str("CRASH"),
            Self::PowerOff => f.write_str("POWEROFF"),
        }
    }
}

fn field<'a>(parts: &[&'a str], at: usize, line: &str) -> Result<&'a str, String> {
    parts.get(at).copied().ok_or_else(|| format!("missing field {at}: {line:?}"))
}

fn handle_field(parts: &[&str], at: usize, line: &str) -> Result<String, String> {
    let handle = field(parts, at, line)?;
    if handle_is_valid(handle) {
        Ok(handle.to_owned())
    } else {
        Err(format!("invalid handle {handle:?}: {line:?}"))
    }
}

fn number<T: FromStr>(parts: &[&str], at: usize, line: &str) -> Result<T, String> {
    field(parts, at, line)?.parse().map_err(|_| format!("field {at} is not a number: {line:?}"))
}

impl FromStr for PeerCommand {
    type Err = String;

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per protocol verb; splitting hides the grammar"
    )]
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = line.split(' ').collect();
        let verb = field(&parts, 0, line)?;
        let exact = |n: usize| {
            if parts.len() == n {
                Ok(())
            } else {
                Err(format!("{verb} takes {} fields: {line:?}", n - 1))
            }
        };
        Ok(match verb {
            "VCONNECT" => {
                exact(5)?;
                let source_port = match field(&parts, 4, line)? {
                    "any" => None,
                    _ => Some(number(&parts, 4, line)?),
                };
                Self::VsockConnect {
                    handle: handle_field(&parts, 1, line)?,
                    cid: number(&parts, 2, line)?,
                    port: number(&parts, 3, line)?,
                    source_port,
                }
            }
            "VLISTEN" => {
                exact(3)?;
                Self::VsockListen {
                    handle: handle_field(&parts, 1, line)?,
                    port: number(&parts, 2, line)?,
                }
            }
            "VACCEPT" => {
                exact(4)?;
                Self::VsockAccept {
                    handle: handle_field(&parts, 1, line)?,
                    listener: handle_field(&parts, 2, line)?,
                    timeout_ms: number(&parts, 3, line)?,
                }
            }
            "TLISTEN" => {
                exact(3)?;
                Self::TcpListen {
                    handle: handle_field(&parts, 1, line)?,
                    addr: number(&parts, 2, line)?,
                }
            }
            "TCONNECT" => {
                exact(3)?;
                Self::TcpConnect {
                    handle: handle_field(&parts, 1, line)?,
                    addr: number(&parts, 2, line)?,
                }
            }
            "TACCEPT" => {
                exact(4)?;
                Self::TcpAccept {
                    handle: handle_field(&parts, 1, line)?,
                    listener: handle_field(&parts, 2, line)?,
                    timeout_ms: number(&parts, 3, line)?,
                }
            }
            "WRITE" => {
                exact(3)?;
                Self::Write {
                    handle: handle_field(&parts, 1, line)?,
                    bytes: unhex(field(&parts, 2, line)?)?,
                }
            }
            "READ" => {
                exact(4)?;
                Self::Read {
                    handle: handle_field(&parts, 1, line)?,
                    max: number(&parts, 2, line)?,
                    timeout_ms: number(&parts, 3, line)?,
                }
            }
            "PUMP" => {
                exact(4)?;
                Self::Pump {
                    handle: handle_field(&parts, 1, line)?,
                    tag: field(&parts, 2, line)?.to_owned(),
                    len: number(&parts, 3, line)?,
                }
            }
            "DRAIN" => {
                exact(4)?;
                Self::Drain {
                    handle: handle_field(&parts, 1, line)?,
                    tag: field(&parts, 2, line)?.to_owned(),
                    timeout_ms: number(&parts, 3, line)?,
                }
            }
            "SHUT" => {
                exact(3)?;
                let how = match field(&parts, 2, line)? {
                    "rd" => Shutdown::Read,
                    "wr" => Shutdown::Write,
                    "rdwr" => Shutdown::Both,
                    other => return Err(format!("not a shutdown direction: {other:?}")),
                };
                Self::Shutdown { handle: handle_field(&parts, 1, line)?, how }
            }
            "CLOSE" => {
                exact(2)?;
                Self::Close { handle: handle_field(&parts, 1, line)? }
            }
            "RESET" => {
                exact(2)?;
                Self::Reset { handle: handle_field(&parts, 1, line)? }
            }
            "FOPEN" => {
                exact(2)?;
                Self::ForwarderOpen { ctx: handle_field(&parts, 1, line)? }
            }
            "FATTACH" => {
                exact(4)?;
                Self::ForwarderAttachStream {
                    ctx: handle_field(&parts, 1, line)?,
                    vsock: handle_field(&parts, 2, line)?,
                    tcp: handle_field(&parts, 3, line)?,
                }
            }
            "FWATCH" => {
                exact(3)?;
                Self::ForwarderWatch {
                    ctx: handle_field(&parts, 1, line)?,
                    vsock: handle_field(&parts, 2, line)?,
                }
            }
            "FABORT" => {
                exact(3)?;
                Self::ForwarderAbort {
                    ctx: handle_field(&parts, 1, line)?,
                    id: number(&parts, 2, line)?,
                }
            }
            "FEND" => {
                exact(3)?;
                Self::ForwarderTakeEnd {
                    ctx: handle_field(&parts, 1, line)?,
                    timeout_ms: number(&parts, 2, line)?,
                }
            }
            "FCLOSE" => {
                exact(2)?;
                Self::ForwarderClose { ctx: handle_field(&parts, 1, line)? }
            }
            "SPAWN" => {
                let task = handle_field(&parts, 1, line)?;
                let rest = parts
                    .get(2..)
                    .filter(|rest| !rest.is_empty())
                    .ok_or_else(|| format!("SPAWN needs a command: {line:?}"))?;
                let command: Self = rest.join(" ").parse()?;
                if matches!(command, Self::Spawn { .. } | Self::Join { .. }) {
                    return Err(format!("SPAWN of a task command: {line:?}"));
                }
                Self::Spawn { task, command: Box::new(command) }
            }
            "JOIN" => {
                exact(3)?;
                Self::Join {
                    task: handle_field(&parts, 1, line)?,
                    timeout_ms: number(&parts, 2, line)?,
                }
            }
            "EXEC" => {
                let argv = parts[1..]
                    .iter()
                    .map(|arg| {
                        unhex(arg).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if argv.is_empty() {
                    return Err(format!("EXEC needs a program: {line:?}"));
                }
                Self::Exec { argv }
            }
            "CRASH" => {
                exact(1)?;
                Self::CrashKernel
            }
            "POWEROFF" => {
                exact(1)?;
                Self::PowerOff
            }
            other => return Err(format!("unknown verb {other:?}")),
        })
    }
}

/// The half of a forwarder pair, as the agent reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerSide {
    /// Half A (the vsock half in every pair the agent attaches).
    A,
    /// Half B.
    B,
}

/// A forwarder end record's outcome, as the agent reports it (the guest
/// side of [`overdrive_core::guest_transport::forwarder::ForwardOutcome`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerOutcome {
    /// Both directions ended cleanly.
    Completed,
    /// A watch: the peer's direction ended with a clean half-close.
    GuestHalfClose,
    /// `PeerAbort`.
    PeerAbort(PeerSide),
    /// `SocketError` with its errno.
    SocketError(PeerSide, u16),
    /// `FrameInvalid`.
    FrameInvalid(PeerSide),
    /// `Detached` (the owner's abort).
    Detached,
    /// `Unclassified` (the alarm).
    Unclassified(PeerSide),
}

impl fmt::Display for PeerOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = |s: &PeerSide| match s {
            PeerSide::A => "a",
            PeerSide::B => "b",
        };
        match self {
            Self::Completed => f.write_str("completed"),
            Self::GuestHalfClose => f.write_str("guest_half_close"),
            Self::PeerAbort(s) => write!(f, "peer_abort:{}", side(s)),
            Self::SocketError(s, errno) => write!(f, "socket_error:{}:{errno}", side(s)),
            Self::FrameInvalid(s) => write!(f, "frame_invalid:{}", side(s)),
            Self::Detached => f.write_str("detached"),
            Self::Unclassified(s) => write!(f, "unclassified:{}", side(s)),
        }
    }
}

impl FromStr for PeerOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let kind = parts.next().unwrap_or_default();
        let side = |p: Option<&str>| match p {
            Some("a") => Ok(PeerSide::A),
            Some("b") => Ok(PeerSide::B),
            other => Err(format!("not a side: {other:?} in {s:?}")),
        };
        let out = match kind {
            "completed" => Self::Completed,
            "guest_half_close" => Self::GuestHalfClose,
            "peer_abort" => Self::PeerAbort(side(parts.next())?),
            "socket_error" => {
                let side = side(parts.next())?;
                let errno = parts
                    .next()
                    .and_then(|e| e.parse().ok())
                    .ok_or_else(|| format!("socket_error needs an errno: {s:?}"))?;
                Self::SocketError(side, errno)
            }
            "frame_invalid" => Self::FrameInvalid(side(parts.next())?),
            "detached" => Self::Detached,
            "unclassified" => Self::Unclassified(side(parts.next())?),
            other => return Err(format!("not an outcome: {other:?}")),
        };
        if parts.next().is_some() {
            return Err(format!("trailing outcome fields: {s:?}"));
        }
        Ok(out)
    }
}

/// One end record of the guest's forwarder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerEnd {
    /// The pair's or watch's id in the guest context.
    pub id: u64,
    /// How it ended.
    pub outcome: PeerOutcome,
    /// Bytes half B accepted from half A.
    pub moved_ab: u64,
    /// Bytes half A accepted from half B.
    pub moved_ba: u64,
}

/// How a [`PeerCommand::Drain`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainEnd {
    /// A clean end of stream.
    Eof,
    /// `ECONNRESET`.
    Reset,
    /// Another errno.
    Error(i32),
    /// No byte within the idle bound.
    Timeout,
    /// A byte that is not the stream's, at this offset.
    Mismatch(u64),
}

impl fmt::Display for DrainEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("eof"),
            Self::Reset => f.write_str("reset"),
            Self::Error(code) => write!(f, "error:{code}"),
            Self::Timeout => f.write_str("timeout"),
            Self::Mismatch(at) => write!(f, "mismatch:{at}"),
        }
    }
}

impl FromStr for DrainEnd {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "eof" => Ok(Self::Eof),
            "reset" => Ok(Self::Reset),
            "timeout" => Ok(Self::Timeout),
            other => other.strip_prefix("error:").map_or_else(
                || {
                    other.strip_prefix("mismatch:").map_or_else(
                        || Err(format!("not a drain end: {other:?}")),
                        |at| {
                            at.parse()
                                .map(Self::Mismatch)
                                .map_err(|_| format!("bad offset {other:?}"))
                        },
                    )
                },
                |code| code.parse().map(Self::Error).map_err(|_| format!("bad errno {other:?}")),
            ),
        }
    }
}

/// One reply from the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerReply {
    /// `OK`.
    Ok,
    /// `VALUE n` (a written count, a bound port).
    Value(u64),
    /// `ACCEPTED cid port`: an accepted vsock connection's peer.
    Accepted {
        /// The peer's CID.
        cid: u32,
        /// The peer's port.
        port: u32,
    },
    /// `DATA hex` (never empty).
    Data(Vec<u8>),
    /// `EOF`.
    Eof,
    /// `TIMEOUT`.
    Timeout,
    /// `DRAINED len end`.
    Drained {
        /// Bytes verified.
        len: u64,
        /// How the read side ended.
        end: DrainEnd,
    },
    /// `FORWARD id`: an attached pair's or a watch's id.
    Forward(u64),
    /// `REFUSED text`: the forwarder refused (its error, rendered).
    Refused(String),
    /// `END id outcome ab ba`.
    End(PeerEnd),
    /// `NOEND`: no end record within the bound.
    NoEnd,
    /// `PENDING`: a spawned task has not finished.
    Pending,
    /// `EXIT code`.
    Exit(i32),
    /// `ERR errno text`.
    Err {
        /// The errno (`-1` when the failure has none).
        errno: i32,
        /// What failed.
        detail: String,
    },
}

impl fmt::Display for PeerReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => f.write_str("OK"),
            Self::Value(n) => write!(f, "VALUE {n}"),
            Self::Accepted { cid, port } => write!(f, "ACCEPTED {cid} {port}"),
            Self::Data(bytes) => write!(f, "DATA {}", hex(bytes)),
            Self::Eof => f.write_str("EOF"),
            Self::Timeout => f.write_str("TIMEOUT"),
            Self::Drained { len, end } => write!(f, "DRAINED {len} {end}"),
            Self::Forward(id) => write!(f, "FORWARD {id}"),
            Self::Refused(text) => write!(f, "REFUSED {}", text.replace('\n', " ")),
            Self::End(end) => {
                write!(f, "END {} {} {} {}", end.id, end.outcome, end.moved_ab, end.moved_ba)
            }
            Self::NoEnd => f.write_str("NOEND"),
            Self::Pending => f.write_str("PENDING"),
            Self::Exit(code) => write!(f, "EXIT {code}"),
            Self::Err { errno, detail } => write!(f, "ERR {errno} {}", detail.replace('\n', " ")),
        }
    }
}

impl FromStr for PeerReply {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, String> {
        let parts: Vec<&str> = line.split(' ').collect();
        let verb = field(&parts, 0, line)?;
        Ok(match verb {
            "OK" => Self::Ok,
            "VALUE" => Self::Value(number(&parts, 1, line)?),
            "ACCEPTED" => {
                Self::Accepted { cid: number(&parts, 1, line)?, port: number(&parts, 2, line)? }
            }
            "DATA" => Self::Data(unhex(field(&parts, 1, line)?)?),
            "EOF" => Self::Eof,
            "TIMEOUT" => Self::Timeout,
            "DRAINED" => Self::Drained {
                len: number(&parts, 1, line)?,
                end: field(&parts, 2, line)?.parse()?,
            },
            "FORWARD" => Self::Forward(number(&parts, 1, line)?),
            "REFUSED" => Self::Refused(parts[1..].join(" ")),
            "END" => Self::End(PeerEnd {
                id: number(&parts, 1, line)?,
                outcome: field(&parts, 2, line)?.parse()?,
                moved_ab: number(&parts, 3, line)?,
                moved_ba: number(&parts, 4, line)?,
            }),
            "NOEND" => Self::NoEnd,
            "PENDING" => Self::Pending,
            "EXIT" => Self::Exit(number(&parts, 1, line)?),
            "ERR" => Self::Err {
                errno: number(&parts, 1, line)?,
                detail: parts.get(2..).map(|rest| rest.join(" ")).unwrap_or_default(),
            },
            other => return Err(format!("unknown reply {other:?}")),
        })
    }
}

#[cfg(target_os = "linux")]
pub use client::{Drained, GuestPeer, GuestPeerControl, GuestPeerError, PeerRead};

#[cfg(target_os = "linux")]
mod client {
    use std::io::{self, Write};
    use std::net::{Shutdown, SocketAddrV4};
    use std::os::fd::AsFd;
    use std::time::{Duration, Instant};

    use super::{DrainEnd, PeerCommand, PeerEnd, PeerReply};
    use crate::vsock::{VMADDR_CID_ANY, VsockListener, VsockStream, wait_ready};

    /// Slack added to a command's own bound when the client waits for its
    /// reply (the agent's scheduling, the vsock round trip).
    const REPLY_SLACK: Duration = Duration::from_secs(10);

    /// A failure talking to the agent, or a reply the caller did not expect.
    #[derive(Debug, thiserror::Error)]
    pub enum GuestPeerError {
        /// The control connection failed.
        #[error("the guest peer's control connection failed: {0}")]
        Io(#[from] io::Error),
        /// The agent never connected within the bound.
        #[error("the guest peer did not connect to control port {port} within {bound:?}")]
        NeverConnected {
            /// The control port.
            port: u32,
            /// The bound.
            bound: Duration,
        },
        /// The agent closed its control connection.
        #[error("the guest peer closed its control connection during {command}")]
        Closed {
            /// The command whose reply was awaited.
            command: String,
        },
        /// No reply within the bound.
        #[error("no reply to {command} within {bound:?}")]
        NoReply {
            /// The command.
            command: String,
            /// The bound.
            bound: Duration,
        },
        /// The reply did not parse.
        #[error("malformed reply {line:?} to {command}: {detail}")]
        Malformed {
            /// The command.
            command: String,
            /// The reply line.
            line: String,
            /// Why it did not parse.
            detail: String,
        },
        /// The agent reported a failure.
        #[error("{command} failed in the guest: errno {errno}: {detail}")]
        Remote {
            /// The command.
            command: String,
            /// The errno.
            errno: i32,
            /// What failed.
            detail: String,
        },
        /// A well-formed reply of the wrong kind.
        #[error("unexpected reply {reply:?} to {command}")]
        Unexpected {
            /// The command.
            command: String,
            /// The reply.
            reply: PeerReply,
        },
    }

    /// The outcome of one guest read.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PeerRead {
        /// Bytes.
        Data(Vec<u8>),
        /// End of stream.
        Eof,
        /// Nothing within the bound.
        Timeout,
    }

    /// The outcome of a [`GuestPeer::drain`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Drained {
        /// Bytes verified.
        pub len: u64,
        /// How the read side ended.
        pub end: DrainEnd,
    }

    /// The host listener the agent dials at boot.
    #[derive(Debug)]
    pub struct GuestPeerControl {
        listener: VsockListener,
    }

    impl GuestPeerControl {
        /// Binds `(VMADDR_CID_ANY, port)`. Bind before booting the guest.
        ///
        /// # Errors
        ///
        /// The bind's error (`EADDRINUSE` when another test holds the port).
        pub fn bind(port: u32) -> io::Result<Self> {
            Ok(Self { listener: VsockListener::bind(VMADDR_CID_ANY, port)? })
        }

        /// The bound port.
        #[must_use]
        pub const fn port(&self) -> u32 {
            self.listener.port()
        }

        /// The agent's control connection, within `bound`. Connections from
        /// another CID than `cid` are closed and not counted.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::NeverConnected`] when the bound passes.
        pub fn accept_peer(&self, cid: u32, bound: Duration) -> Result<GuestPeer, GuestPeerError> {
            let deadline = Instant::now() + bound;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.listener.accept_within(left)? {
                    Some((stream, peer)) if peer.cid == cid => {
                        return Ok(GuestPeer { stream, cid, pending: Vec::new() });
                    }
                    Some(_) => {}
                    None => {
                        return Err(GuestPeerError::NeverConnected { port: self.port(), bound });
                    }
                }
            }
        }
    }

    /// The host test's end of a scripted guest.
    #[derive(Debug)]
    pub struct GuestPeer {
        stream: VsockStream,
        cid: u32,
        pending: Vec<u8>,
    }

    impl GuestPeer {
        /// The guest's CID, as the host kernel reported it at accept.
        #[must_use]
        pub const fn cid(&self) -> u32 {
            self.cid
        }

        /// Sends `command` and waits up to `bound` for its reply.
        ///
        /// # Errors
        ///
        /// A control-connection failure or a malformed reply.
        pub fn request(
            &mut self,
            command: &PeerCommand,
            bound: Duration,
        ) -> Result<PeerReply, GuestPeerError> {
            let text = command.to_string();
            (&self.stream).write_all(format!("{text}\n").as_bytes())?;
            let line = self.read_line(&text, bound)?;
            line.parse().map_err(|detail| GuestPeerError::Malformed { command: text, line, detail })
        }

        fn read_line(&mut self, command: &str, bound: Duration) -> Result<String, GuestPeerError> {
            let deadline = Instant::now() + bound;
            loop {
                if let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = self.pending.drain(..=at).collect();
                    return Ok(String::from_utf8_lossy(&line[..at]).into_owned());
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || !wait_ready(self.stream.as_fd(), libc::POLLIN, left)? {
                    return Err(GuestPeerError::NoReply { command: command.to_owned(), bound });
                }
                let mut buf = vec![0u8; 65_536];
                let n = std::io::Read::read(&mut &self.stream, &mut buf)?;
                if n == 0 {
                    return Err(GuestPeerError::Closed { command: command.to_owned() });
                }
                self.pending.extend_from_slice(&buf[..n]);
            }
        }

        fn expect(
            &mut self,
            command: &PeerCommand,
            bound: Duration,
        ) -> Result<PeerReply, GuestPeerError> {
            match self.request(command, bound + REPLY_SLACK)? {
                PeerReply::Err { errno, detail } => {
                    Err(GuestPeerError::Remote { command: command.to_string(), errno, detail })
                }
                reply => Ok(reply),
            }
        }

        fn expect_ok(&mut self, command: &PeerCommand) -> Result<(), GuestPeerError> {
            match self.expect(command, Duration::ZERO)? {
                PeerReply::Ok => Ok(()),
                reply => Err(GuestPeerError::Unexpected { command: command.to_string(), reply }),
            }
        }

        fn unexpected<T>(command: &PeerCommand, reply: PeerReply) -> Result<T, GuestPeerError> {
            Err(GuestPeerError::Unexpected { command: command.to_string(), reply })
        }

        /// A guest vsock connection to `(cid, port)`, from `source_port`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn vsock_connect(
            &mut self,
            handle: &str,
            cid: u32,
            port: u32,
            source_port: Option<u32>,
        ) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::VsockConnect {
                handle: handle.to_owned(),
                cid,
                port,
                source_port,
            })
        }

        /// A guest vsock listener on `port`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn vsock_listen(&mut self, handle: &str, port: u32) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::VsockListen { handle: handle.to_owned(), port })
        }

        /// The guest listener's next connection; its peer `(cid, port)`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn vsock_accept(
            &mut self,
            handle: &str,
            listener: &str,
            timeout: Duration,
        ) -> Result<Option<(u32, u32)>, GuestPeerError> {
            let command = PeerCommand::VsockAccept {
                handle: handle.to_owned(),
                listener: listener.to_owned(),
                timeout_ms: millis(timeout),
            };
            match self.expect(&command, timeout)? {
                PeerReply::Accepted { cid, port } => Ok(Some((cid, port))),
                PeerReply::Timeout => Ok(None),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// A guest TCP listener at `addr`; returns the bound address.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn tcp_listen(
            &mut self,
            handle: &str,
            addr: SocketAddrV4,
        ) -> Result<SocketAddrV4, GuestPeerError> {
            let command = PeerCommand::TcpListen { handle: handle.to_owned(), addr };
            match self.expect(&command, Duration::ZERO)? {
                PeerReply::Value(port) => {
                    let port = u16::try_from(port).map_err(|_| GuestPeerError::Unexpected {
                        command: command.to_string(),
                        reply: PeerReply::Value(port),
                    })?;
                    Ok(SocketAddrV4::new(*addr.ip(), port))
                }
                reply => Self::unexpected(&command, reply),
            }
        }

        /// A guest TCP connection to `addr`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn tcp_connect(
            &mut self,
            handle: &str,
            addr: SocketAddrV4,
        ) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::TcpConnect { handle: handle.to_owned(), addr })
        }

        /// The guest TCP listener's next connection; `false` within `timeout`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn tcp_accept(
            &mut self,
            handle: &str,
            listener: &str,
            timeout: Duration,
        ) -> Result<bool, GuestPeerError> {
            let command = PeerCommand::TcpAccept {
                handle: handle.to_owned(),
                listener: listener.to_owned(),
                timeout_ms: millis(timeout),
            };
            match self.expect(&command, timeout)? {
                PeerReply::Ok => Ok(true),
                PeerReply::Timeout => Ok(false),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// Writes `bytes` whole on a guest socket.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn write(&mut self, handle: &str, bytes: &[u8]) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::Write { handle: handle.to_owned(), bytes: bytes.to_vec() })
        }

        /// One guest read of at most `max` bytes.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the read's errno.
        pub fn read(
            &mut self,
            handle: &str,
            max: u32,
            timeout: Duration,
        ) -> Result<PeerRead, GuestPeerError> {
            let command =
                PeerCommand::Read { handle: handle.to_owned(), max, timeout_ms: millis(timeout) };
            match self.expect(&command, timeout)? {
                PeerReply::Data(bytes) => Ok(PeerRead::Data(bytes)),
                PeerReply::Eof => Ok(PeerRead::Eof),
                PeerReply::Timeout => Ok(PeerRead::Timeout),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// Writes `len` bytes of the guest-to-peer stream of `tag`.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the write's errno.
        pub fn pump(
            &mut self,
            handle: &str,
            tag: &str,
            len: u64,
            bound: Duration,
        ) -> Result<u64, GuestPeerError> {
            let command = PeerCommand::Pump { handle: handle.to_owned(), tag: tag.to_owned(), len };
            match self.expect(&command, bound)? {
                PeerReply::Value(n) => Ok(n),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// Reads to the end, verifying the peer-to-guest stream of `tag`.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn drain(
            &mut self,
            handle: &str,
            tag: &str,
            idle: Duration,
        ) -> Result<Drained, GuestPeerError> {
            let command = PeerCommand::Drain {
                handle: handle.to_owned(),
                tag: tag.to_owned(),
                timeout_ms: millis(idle),
            };
            // A drain may run for as long as bytes keep arriving; the client
            // bound is generous and the agent's idle bound ends it.
            match self.expect(&command, idle + Duration::from_secs(120))? {
                PeerReply::Drained { len, end } => Ok(Drained { len, end }),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// `shutdown(how)` of a guest socket.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the guest's errno.
        pub fn shutdown(&mut self, handle: &str, how: Shutdown) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::Shutdown { handle: handle.to_owned(), how })
        }

        /// Drops a guest resource.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] when the handle is unknown.
        pub fn close(&mut self, handle: &str) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::Close { handle: handle.to_owned() })
        }

        /// Ends a guest socket abortively ([`PeerCommand::Reset`]).
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] when the handle is unknown.
        pub fn reset(&mut self, handle: &str) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::Reset { handle: handle.to_owned() })
        }

        /// Opens a guest forwarder context.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] with the open's error.
        pub fn forwarder_open(&mut self, ctx: &str) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::ForwarderOpen { ctx: ctx.to_owned() })
        }

        /// Attaches the stream pair `(vsock, tcp)` in the guest context;
        /// `Ok(Err(text))` is the forwarder's refusal.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn forwarder_attach_stream(
            &mut self,
            ctx: &str,
            vsock: &str,
            tcp: &str,
        ) -> Result<Result<u64, String>, GuestPeerError> {
            let command = PeerCommand::ForwarderAttachStream {
                ctx: ctx.to_owned(),
                vsock: vsock.to_owned(),
                tcp: tcp.to_owned(),
            };
            match self.expect(&command, Duration::ZERO)? {
                PeerReply::Forward(id) => Ok(Ok(id)),
                PeerReply::Refused(text) => Ok(Err(text)),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// The guest context's next end record within `timeout`.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn forwarder_take_end(
            &mut self,
            ctx: &str,
            timeout: Duration,
        ) -> Result<Option<PeerEnd>, GuestPeerError> {
            let command =
                PeerCommand::ForwarderTakeEnd { ctx: ctx.to_owned(), timeout_ms: millis(timeout) };
            match self.expect(&command, timeout)? {
                PeerReply::End(end) => Ok(Some(end)),
                PeerReply::NoEnd => Ok(None),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// Starts `command` on its own guest thread.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn spawn(&mut self, task: &str, command: PeerCommand) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::Spawn {
                task: task.to_owned(),
                command: Box::new(command),
            })
        }

        /// A spawned task's reply within `timeout`, or `None` while it runs.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn join(
            &mut self,
            task: &str,
            timeout: Duration,
        ) -> Result<Option<PeerReply>, GuestPeerError> {
            let command = PeerCommand::Join { task: task.to_owned(), timeout_ms: millis(timeout) };
            match self.request(&command, timeout + REPLY_SLACK)? {
                PeerReply::Pending => Ok(None),
                reply => Ok(Some(reply)),
            }
        }

        /// Runs a program in the guest to completion; its exit code.
        ///
        /// # Errors
        ///
        /// [`GuestPeerError::Remote`] when it could not be started.
        pub fn exec(&mut self, argv: &[&str], bound: Duration) -> Result<i32, GuestPeerError> {
            let command =
                PeerCommand::Exec { argv: argv.iter().map(|a| (*a).to_owned()).collect() };
            match self.expect(&command, bound)? {
                PeerReply::Exit(code) => Ok(code),
                reply => Self::unexpected(&command, reply),
            }
        }

        /// Crashes the guest kernel; returns once the command is sent.
        ///
        /// # Errors
        ///
        /// The control connection's write error.
        pub fn crash_kernel(self) -> Result<(), GuestPeerError> {
            (&self.stream).write_all(format!("{}\n", PeerCommand::CrashKernel).as_bytes())?;
            Ok(())
        }

        /// Powers the guest off.
        ///
        /// # Errors
        ///
        /// A control-connection failure.
        pub fn power_off(mut self) -> Result<(), GuestPeerError> {
            self.expect_ok(&PeerCommand::PowerOff)
        }
    }

    fn millis(d: Duration) -> u64 {
        u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_command_round_trips_through_its_line() {
        let addr: SocketAddrV4 = "10.0.0.2:7000".parse().expect("addr");
        let commands = vec![
            PeerCommand::VsockConnect {
                handle: "v1".into(),
                cid: 2,
                port: 1240,
                source_port: Some(5000),
            },
            PeerCommand::VsockConnect {
                handle: "v2".into(),
                cid: 2,
                port: 1243,
                source_port: None,
            },
            PeerCommand::VsockListen { handle: "l".into(), port: 1242 },
            PeerCommand::VsockAccept { handle: "v3".into(), listener: "l".into(), timeout_ms: 5 },
            PeerCommand::TcpListen { handle: "tl".into(), addr },
            PeerCommand::TcpConnect { handle: "t".into(), addr },
            PeerCommand::TcpAccept { handle: "ts".into(), listener: "tl".into(), timeout_ms: 9 },
            PeerCommand::Write { handle: "t".into(), bytes: vec![0, 255, 16] },
            PeerCommand::Read { handle: "t".into(), max: 64, timeout_ms: 10 },
            PeerCommand::Pump { handle: "t".into(), tag: "x-1".into(), len: 99 },
            PeerCommand::Drain { handle: "t".into(), tag: "x-1".into(), timeout_ms: 3 },
            PeerCommand::Shutdown { handle: "t".into(), how: Shutdown::Write },
            PeerCommand::Close { handle: "t".into() },
            PeerCommand::Reset { handle: "t".into() },
            PeerCommand::ForwarderOpen { ctx: "f".into() },
            PeerCommand::ForwarderAttachStream {
                ctx: "f".into(),
                vsock: "v1".into(),
                tcp: "ts".into(),
            },
            PeerCommand::ForwarderWatch { ctx: "f".into(), vsock: "v1".into() },
            PeerCommand::ForwarderAbort { ctx: "f".into(), id: 7 },
            PeerCommand::ForwarderTakeEnd { ctx: "f".into(), timeout_ms: 1 },
            PeerCommand::ForwarderClose { ctx: "f".into() },
            PeerCommand::Spawn {
                task: "p".into(),
                command: Box::new(PeerCommand::Pump {
                    handle: "t".into(),
                    tag: "y".into(),
                    len: 1,
                }),
            },
            PeerCommand::Join { task: "p".into(), timeout_ms: 2 },
            PeerCommand::Exec { argv: vec!["/bin/x".into(), "a b".into()] },
            PeerCommand::CrashKernel,
            PeerCommand::PowerOff,
        ];
        for command in commands {
            let line = command.to_string();
            assert!(!line.contains('\n'));
            assert_eq!(line.parse::<PeerCommand>(), Ok(command), "{line}");
        }
    }

    #[test]
    fn every_reply_round_trips_through_its_line() {
        let replies = vec![
            PeerReply::Ok,
            PeerReply::Value(12),
            PeerReply::Accepted { cid: 16_777_216, port: 49_152 },
            PeerReply::Data(vec![1, 2, 3]),
            PeerReply::Eof,
            PeerReply::Timeout,
            PeerReply::Drained { len: 5, end: DrainEnd::Mismatch(4) },
            PeerReply::Drained { len: 5, end: DrainEnd::Error(32) },
            PeerReply::Forward(3),
            PeerReply::Refused("HalfEnded { side: A, errno: 104 }".into()),
            PeerReply::End(PeerEnd {
                id: 3,
                outcome: PeerOutcome::SocketError(PeerSide::A, 104),
                moved_ab: 10,
                moved_ba: 20,
            }),
            PeerReply::NoEnd,
            PeerReply::Pending,
            PeerReply::Exit(-9),
            PeerReply::Err { errno: 111, detail: "connect refused".into() },
        ];
        for reply in replies {
            let line = reply.to_string();
            assert_eq!(line.parse::<PeerReply>(), Ok(reply), "{line}");
        }
    }

    #[test]
    fn malformed_lines_are_refused() {
        for line in
            ["", "VCONNECT", "VCONNECT BAD 2 1 any", "WRITE t zz", "SPAWN p JOIN q 1", "NOPE"]
        {
            assert!(line.parse::<PeerCommand>().is_err(), "{line:?}");
        }
        for line in ["", "END 1 completed 1", "DRAINED 1 sideways", "WHAT"] {
            assert!(line.parse::<PeerReply>().is_err(), "{line:?}");
        }
    }
}
