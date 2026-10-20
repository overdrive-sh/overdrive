//! Guest transport wire contracts — flow request, control open, control
//! message, datagram frame, the node pair capacity and the deadlines (D5,
//! D18, D18a, ATTR-TOKEN, INB-1, R2-END, EVMOD-DGRAM, HOST-NEUTRAL).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Wire contracts — flow request, control message, datagram frame"
//! and DG-8. One SSOT consumed by the host owner, `overdrive-init` and the
//! forwarder module's UAPI mirror (`overdrive-fwd`); the host side is
//! guest-neutral (ADR-0207). `decode` accepts only canonical encodings, so
//! round-trip equality holds over every valid value and every accepted byte
//! string. The forwarder's end record is not here: it is pinned with the
//! forwarder (EVMOD-REC, [`super::forwarder`]) and never leaves the machine.
//!
//! RED scaffold (DISTILL, Mandate 7): every function and method body panics
//! until DELIVER step 2 ("core types and wire codec") lands it. Constants
//! carry the values the design pins.
//!
//! Items marked **E-list** are the redirect-era surface EVMOD-ADOPT and
//! EVMOD-DGRAM remove (feature-delta § *Bridge-era code for DELIVER to
//! delete*, list E): request kind `3` (`DatagramSeqpacket`), the flow event
//! record (`FlowEvent`, `FlowCell`, `CellEpoch`, `FlowEventDirection`,
//! `FlowEventKind`, `FLOW_EVENT_LEN`), `WireField::Event*` and
//! `FlowWireError::UnknownEventKind`. They are not part of the pinned
//! contract, no test defends them, and DELIVER deletes them together with
//! their remaining users (the sockmap forwarder of `overdrive-dataplane`
//! and the redirect-era owner seams).

// SCAFFOLD: true — netns-density-295 DISTILL (vsock DELIVER step 2).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lint self-trips when DELIVER lands the bodies"
)]

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::LaunchToken;
use crate::net::ListenPort;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// Flow id
// ---------------------------------------------------------------------------

/// Identifies one flow within one VM's control session. The opener allocates
/// it. Top bit = origin: 0 guest-opened, 1 host-opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId(u32);

/// Which side opened a flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowOrigin {
    /// `overdrive-init` opened it.
    Guest,
    /// The host owner opened it.
    Host,
}

impl FlowId {
    /// `seq` must be < 2^31.
    pub fn new(_origin: FlowOrigin, _seq: u32) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }

    /// The flow's origin (the top bit).
    pub const fn origin(self) -> FlowOrigin {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// The raw 32-bit id, origin bit included.
    pub const fn get(self) -> u32 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// ---------------------------------------------------------------------------
// Flow request (32 bytes)
// ---------------------------------------------------------------------------

/// Length of an encoded [`FlowRequest`].
pub const FLOW_REQUEST_LEN: usize = 32;

/// The four flow kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind {
    /// Kind 1: guest → host, STREAM 1240.
    TcpConnect,
    /// Kind 2: guest → host, STREAM 1240 (the association's one connection,
    /// EVMOD-DGRAM).
    DatagramStream,
    /// **E-list.** The former kind 3 (SEQPACKET 1241). Kind `3` is not
    /// defined (`UnknownKind`).
    DatagramSeqpacket,
    /// Kind 4: host → guest, STREAM 1242.
    TcpAccept,
}

impl FlowKind {
    /// The kind's canonical label.
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// The 32-byte request an opener writes first on its flow vsock connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRequest {
    /// Kind 1.
    TcpConnect {
        /// Guest-origin flow id.
        flow: FlowId,
        /// Original destination.
        destination: SocketAddrV4,
        /// The opening launch's token.
        launch: LaunchToken,
    },
    /// Kind 2.
    DatagramStream {
        /// Guest-origin flow id.
        flow: FlowId,
        /// Original destination.
        destination: SocketAddrV4,
        /// The opening launch's token.
        launch: LaunchToken,
    },
    /// **E-list.** The former kind 3; not decoded (`UnknownKind`).
    DatagramSeqpacket {
        /// Guest-origin flow id.
        flow: FlowId,
        /// Original destination.
        destination: SocketAddrV4,
        /// The opening launch's token.
        launch: LaunchToken,
    },
    /// Kind 4 (INB-1): opened only by `GuestStreamDial`.
    TcpAccept {
        /// Host-origin flow id.
        flow: FlowId,
        /// The address the guest binds its connection to the application to
        /// (port 0 on the wire); the guest gateway for every request
        /// (ADR-0174).
        peer: Ipv4Addr,
        /// The declared guest listen port.
        listen_port: ListenPort,
        /// The launch the host addressed the flow to.
        launch: LaunchToken,
    },
}

impl FlowRequest {
    /// The request's kind.
    pub fn kind(&self) -> FlowKind {
        panic!("{SCAFFOLD}")
    }

    /// The request's flow id.
    pub fn flow(&self) -> FlowId {
        panic!("{SCAFFOLD}")
    }

    /// The request's launch token.
    pub fn launch(&self) -> LaunchToken {
        panic!("{SCAFFOLD}")
    }

    /// The canonical 32-byte encoding.
    pub fn encode(&self) -> [u8; FLOW_REQUEST_LEN] {
        panic!("{SCAFFOLD}")
    }

    /// Decodes a canonical encoding.
    pub fn decode(_bytes: &[u8; FLOW_REQUEST_LEN]) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// Control session open (24 bytes)
// ---------------------------------------------------------------------------

/// Length of an encoded [`ControlOpen`].
pub const CONTROL_OPEN_LEN: usize = 24;

/// Written once by `overdrive-init`, first on the control session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlOpen {
    /// The guest's own launch token.
    pub launch: LaunchToken,
}

impl ControlOpen {
    /// The canonical 24-byte encoding.
    pub fn encode(&self) -> [u8; CONTROL_OPEN_LEN] {
        panic!("{SCAFFOLD}")
    }

    /// Decodes a canonical encoding.
    pub fn decode(_bytes: &[u8; CONTROL_OPEN_LEN]) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// Control message (8 bytes)
// ---------------------------------------------------------------------------

/// Length of an encoded [`ControlMessage`].
pub const CONTROL_MESSAGE_LEN: usize = 8;

/// A Linux errno value as a kernel reported it; never 0. Both the host and
/// the guest run Linux 7.0, so one numbering serves both.
///
/// `FromStr` / `Display` are decimal; serde matches them. `FromStr::Err` is
/// [`FlowWireError`]: `NotDecimal { field: ControlErrno }` for non-decimal
/// text, `InvalidField { field: ControlErrno }` for 0 (D-UP-13).
///
/// RED scaffold: `new`, `get`, `Display`, `FromStr` and serde panic until
/// DELIVER step 2; the named constants are plain data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KernelErrno(NonZeroU16);

const fn errno_const(raw: u16) -> KernelErrno {
    match NonZeroU16::new(raw) {
        Some(value) => KernelErrno(value),
        None => panic!("a named errno constant is non-zero"),
    }
}

impl KernelErrno {
    /// `ENODEV` (19): no vhost-vsock device holds the CID.
    pub const ENODEV: Self = errno_const(19);
    /// `ECONNRESET` (104): the device-release sweep reset the connection.
    pub const ECONNRESET: Self = errno_const(104);
    /// `ETIMEDOUT` (110): the guest's inbound connect timed out.
    pub const ETIMEDOUT: Self = errno_const(110);
    /// `ECONNREFUSED` (111): nothing listens on the guest port.
    pub const ECONNREFUSED: Self = errno_const(111);

    /// `0` → `InvalidField { field: ControlErrno }`.
    pub fn new(_raw: u16) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }

    /// The errno value.
    pub const fn get(self) -> u16 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for KernelErrno {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; see the type's text-form rule.
impl std::str::FromStr for KernelErrno {
    type Err = FlowWireError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

impl Serialize for KernelErrno {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

impl<'de> Deserialize<'de> for KernelErrno {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a flow was refused (codes 1..=12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRefusal {
    /// Code 1: CID unknown, Provisioned or Retiring.
    NotActive,
    /// Code 2: destination excluded by static policy.
    Policy,
    /// Code 3: leg-F or the DNS responder unavailable, or the allocation not
    /// intercept-live.
    IntakeUnavailable,
    /// Code 4: pair table, per-allocation quota or guest parking cells
    /// exhausted.
    Capacity,
    /// Code 5: the request failed to decode.
    Malformed,
    /// Code 6: the request version is not supported.
    Unsupported,
    /// Code 7: node forwarding quiesced.
    Quiesced,
    /// Code 8: `MtlsResolve` returned `MeshUnreachable`.
    MeshUnreachable,
    /// Code 9: `MtlsResolve` returned an error.
    ResolveFailed,
    /// Code 10: a non-mesh connect failed.
    DestinationUnreachable,
    /// Code 11 (INB-1): the guest's connect to the application for a
    /// `TcpAccept` failed with `errno` (`ECONNREFUSED` when nothing listens).
    ApplicationRefused {
        /// The guest kernel's errno; carried in bytes 2..4 of the message.
        errno: KernelErrno,
    },
    /// Code 12 (D26): the destination is host-internal.
    HostInternal,
}

impl FlowRefusal {
    /// The refusal code on the wire.
    pub const fn as_u8(self) -> u8 {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// The refusal's canonical label.
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// An 8-byte message on the per-VM control session. Pairing replies and
/// aborts only: a flow's half-close never touches the session (R2-END).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMessage {
    /// Type 1: the acceptor paired `flow`.
    Paired {
        /// The flow.
        flow: FlowId,
    },
    /// Type 2: the acceptor refused `flow`.
    Refused {
        /// The flow.
        flow: FlowId,
        /// Why.
        refusal: FlowRefusal,
    },
    /// Type 3: either side aborts `flow`.
    Abort {
        /// The flow.
        flow: FlowId,
    },
    /// Type 4 (INB-1), guest → host, `TcpAccept` only: the guest's connect to
    /// the application did not complete within
    /// [`GUEST_INBOUND_CONNECT_TIMEOUT`].
    TimedOut {
        /// The flow.
        flow: FlowId,
        /// The guest kernel's errno (`ETIMEDOUT`).
        errno: KernelErrno,
    },
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "the `&self` / `&[u8; 8]` signatures are pinned by the design, uniform with the 32- and 24-byte codecs"
)]
impl ControlMessage {
    /// The canonical 8-byte encoding.
    pub fn encode(&self) -> [u8; CONTROL_MESSAGE_LEN] {
        panic!("{SCAFFOLD}")
    }

    /// Decodes a canonical encoding.
    pub fn decode(_bytes: &[u8; CONTROL_MESSAGE_LEN]) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }
}

/// The wire field an `InvalidField` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireField {
    /// The flow id's sequence (`>= 2^31`).
    FlowSeq,
    /// The request's port.
    Port,
    /// The request's listen port.
    ListenPort,
    /// The control message's detail (refusal code) byte.
    ControlDetail,
    /// The control message's errno field.
    ControlErrno,
    /// The control message's flow field.
    ControlFlow,
    /// A launch token field (all zero).
    LaunchToken,
    /// A reserved byte of the control open.
    ControlOpenReserved,
    /// **E-list.** The removed flow event record's cell.
    EventCell,
    /// **E-list.** The removed flow event record's direction byte.
    EventDirection,
    /// **E-list.** The removed flow event record's reserved bytes.
    EventReserved,
}

/// Why bytes are not a canonical encoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FlowWireError {
    /// Unrecognised magic.
    #[error("flow request magic {observed:02x?} is not recognised")]
    BadMagic {
        /// The bytes read.
        observed: [u8; 2],
    },
    /// Unsupported version.
    #[error("flow request version {version} is not supported")]
    UnsupportedVersion {
        /// The version read.
        version: u8,
    },
    /// Undefined request kind.
    #[error("flow request kind {kind} is not defined")]
    UnknownKind {
        /// The kind byte read.
        kind: u8,
    },
    /// The flow id's origin does not match the kind.
    #[error("flow id origin does not match kind {kind:?}")]
    OriginMismatch {
        /// The decoded kind.
        kind: FlowKind,
    },
    /// A field carries an invalid value.
    #[error("field {field:?} has an invalid value")]
    InvalidField {
        /// The field.
        field: WireField,
    },
    /// Undefined control message type.
    #[error("control message type {ty} is not defined")]
    UnknownControlType {
        /// The type byte read.
        ty: u8,
    },
    /// Undefined refusal code.
    #[error("refusal code {code} is not defined")]
    UnknownRefusal {
        /// The code read.
        code: u8,
    },
    /// **E-list.** The removed flow event record's kind.
    #[error("flow event kind {kind} is not defined")]
    UnknownEventKind {
        /// The kind byte read.
        kind: u8,
    },
    /// `FromStr` of a wire newtype: the text is not a decimal integer of the
    /// newtype's width (D-UP-13).
    #[error("field {field:?} is not a decimal integer")]
    NotDecimal {
        /// The field the newtype carries.
        field: WireField,
    },
}

// ---------------------------------------------------------------------------
// Datagram frame constants (every datagram of an association, both
// directions: magic, BE logical payload length, payload)
// ---------------------------------------------------------------------------

/// Frame magic.
pub const DATAGRAM_FRAME_MAGIC: [u8; 4] = *b"OVD1";
/// Frame header length (magic + BE `u32` length).
pub const DATAGRAM_FRAME_HEADER_LEN: usize = 8;
/// Largest logical payload of one guest datagram.
pub const MAX_GUEST_DATAGRAM_PAYLOAD: u32 = 59_000;
/// The largest frame: `MAX_GUEST_DATAGRAM_PAYLOAD + DATAGRAM_FRAME_HEADER_LEN`.
pub const MAX_GUEST_DATAGRAM_FRAME: u32 = 59_008;

#[allow(
    clippy::cast_possible_truncation,
    reason = "DATAGRAM_FRAME_HEADER_LEN is 8; the compile-time check runs on the constant"
)]
const _: () = assert!(
    MAX_GUEST_DATAGRAM_FRAME == MAX_GUEST_DATAGRAM_PAYLOAD + DATAGRAM_FRAME_HEADER_LEN as u32,
    "the largest frame is the largest payload plus the header"
);

/// One framed datagram (§ Wire contracts, Datagram frame; DG-8). The Rust
/// codec of the frame the forwarder writes and reads on an association's
/// vsock STREAM, both directions (EVMOD-DGRAM, ADR-0201); the forwarder
/// probe's `DatagramRoundTrip` stage and the module's Tier-3 suite use it as
/// their oracle. An empty datagram is the frame whose declared length is 0;
/// a payload that itself parses as a frame is ordinary payload, stripped of
/// exactly one outer frame (no escape rule). The golden empty and maximum
/// frames, and the frame constants, are asserted equal across this codec and
/// the module's UAPI header (R5-51).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatagramFrame<'a> {
    payload: &'a [u8],
}

impl<'a> DatagramFrame<'a> {
    /// `Err(Oversize)` when `payload.len() > MAX_GUEST_DATAGRAM_PAYLOAD`.
    pub fn new(_payload: &'a [u8]) -> Result<Self, DatagramFrameError> {
        panic!("{SCAFFOLD}")
    }

    /// The logical payload.
    pub fn payload(&self) -> &'a [u8] {
        panic!("{SCAFFOLD}")
    }

    /// Header + payload, pre-sized (`DATAGRAM_FRAME_HEADER_LEN + payload.len()`).
    pub fn encode(&self) -> Vec<u8> {
        panic!("{SCAFFOLD}")
    }

    /// Accepts exactly one canonical frame: magic, length ≤ max, and
    /// `bytes.len() == DATAGRAM_FRAME_HEADER_LEN + length`.
    pub fn decode(_bytes: &'a [u8]) -> Result<Self, DatagramFrameError> {
        panic!("{SCAFFOLD}")
    }
}

/// Why bytes are not one canonical datagram frame, or a payload cannot be
/// framed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DatagramFrameError {
    /// Fewer bytes than the frame header.
    #[error("datagram frame is {len} bytes, shorter than its header")]
    Truncated {
        /// The number of bytes offered.
        len: usize,
    },
    /// The magic is not `OVD1`.
    #[error("datagram frame magic {observed:02x?} is not OVD1")]
    BadMagic {
        /// The four bytes read.
        observed: [u8; 4],
    },
    /// The payload (offered or declared) exceeds `MAX_GUEST_DATAGRAM_PAYLOAD`.
    #[error("datagram frame length {length} exceeds {max}")]
    Oversize {
        /// The offered or declared length.
        length: u64,
        /// `MAX_GUEST_DATAGRAM_PAYLOAD`.
        max: u32,
    },
    /// The declared length disagrees with the bytes carried after the header.
    #[error("datagram frame declares {declared} payload bytes but carries {actual}")]
    LengthMismatch {
        /// The header's declared length.
        declared: u32,
        /// The payload bytes actually carried.
        actual: usize,
    },
}

// ---------------------------------------------------------------------------
// Node pair capacity (D3, ADR-0208)
// ---------------------------------------------------------------------------

/// The node-wide bound on concurrent flows (the host's pair capacity).
pub const GUEST_FLOW_MAX_PAIRS: u32 = 65_536;

// ---------------------------------------------------------------------------
// E-list: the flow event record of the removed sockmap forwarder (R2-END,
// R2-DRAIN, A3, UDP-EMPTY). Replaced by the forwarder's end record
// (EVMOD-REC). Not built by step 2; DELIVER deletes it with its users.
// ---------------------------------------------------------------------------

/// Length of an encoded [`FlowEvent`].
pub const FLOW_EVENT_LEN: usize = 24;

/// A flow's slot in a side's drain cell array; cells `2 × slot` (dir 0) and
/// `2 × slot + 1` (dir 1). Below the side's slot count.
///
/// RED scaffold: `new`, `get` and `Display` panic until DELIVER step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowCell(u32);

impl FlowCell {
    /// Rejects `raw` at or above the side's slot bound with
    /// `InvalidField { field: EventCell }`.
    pub fn new(_raw: u32) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }

    /// The slot index.
    pub const fn get(self) -> u32 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for FlowCell {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Incremented each time a slot's cells are initialised for a new flow.
///
/// RED scaffold: `new`, `get` and `Display` panic until DELIVER step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellEpoch(u32);

impl CellEpoch {
    /// Every `u32` is an epoch.
    pub fn new(_raw: u32) -> Self {
        panic!("{SCAFFOLD}")
    }

    /// The epoch value.
    pub const fn get(self) -> u32 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for CellEpoch {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// The direction byte of a [`FlowEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEventDirection {
    /// `0`: into the flow vsock (host: `FlowDirection::TowardGuest`).
    IntoVsock,
    /// `1`: out of the flow vsock (host: `FromGuest`).
    OutOfVsock,
}

/// The kind byte of a [`FlowEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEventKind {
    /// `1`: the peer ended its direction cleanly (vsock SHUTDOWN 0x2).
    EndHalfClose,
    /// `2`: a RST, or a SHUTDOWN 0x3 while a direction was still open.
    EndAbort,
    /// `3`: a drain cell's `sent` met its goal.
    DrainComplete,
    /// `4`: a redirect to the cell's route target was refused for the first
    /// time.
    RedirectRefused,
    /// `5` (host only, UDP-EMPTY): a SEQPACKET record of an association
    /// declared length 0.
    EmptyDatagram,
}

impl FlowEventKind {
    /// The kind's canonical label.
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// One record of a side's `FLOW_EVENT_RING` (native byte order: the record
/// never leaves the machine that wrote it). A record is acted on only when
/// its `cell` names a live flow and its `epoch` equals that flow's epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowEvent {
    /// The flow's slot.
    pub cell: FlowCell,
    /// The cell's epoch when the record was written.
    pub epoch: CellEpoch,
    /// The direction.
    pub dir: FlowEventDirection,
    /// What happened.
    pub kind: FlowEventKind,
    /// Non-zero only for `DrainComplete`: the cell's `sent` that met its goal.
    pub sent: u64,
}

impl FlowEvent {
    /// The 24-byte record.
    pub fn encode(&self) -> [u8; FLOW_EVENT_LEN] {
        panic!("{SCAFFOLD}")
    }

    /// Decodes a record: an unknown kind is `UnknownEventKind`; a direction
    /// other than 0 or 1 is `InvalidField { field: EventDirection }`; a
    /// non-zero reserved byte, or a non-zero `sent` on a kind other than
    /// `DrainComplete`, is `InvalidField { field: EventReserved }`.
    pub fn decode(_bytes: &[u8; FLOW_EVENT_LEN]) -> Result<Self, FlowWireError> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// Deadlines (one SSOT)
// ---------------------------------------------------------------------------

/// Acceptor reading a request; host matching datagram kinds 2 and 3.
pub const FLOW_REQUEST_DEADLINE: Duration = Duration::from_secs(2);
/// Guest opener of a datagram association waiting for `Paired` / `Refused`.
pub const FLOW_PAIRING_DEADLINE: Duration = Duration::from_secs(5);
/// INB-1: the guest's bound on its connect to the application for a
/// `TcpAccept`; on expiry it replies `TimedOut`.
pub const GUEST_INBOUND_CONNECT_TIMEOUT: Duration = Duration::from_millis(1_500);
/// INB-1: the host dial's wait for the pairing reply (`Paired`, `Refused` or
/// `TimedOut`) of a `TcpAccept`; on expiry the owner closes the host vsock,
/// sends `Abort { flow }` while the control session is live, and returns
/// `GuestDialError::PairingDeadline`; a late reply is discarded (U-5).
pub const GUEST_DIAL_PAIRING_DEADLINE: Duration = Duration::from_secs(3);

const _: () = assert!(
    GUEST_DIAL_PAIRING_DEADLINE.as_nanos() > GUEST_INBOUND_CONNECT_TIMEOUT.as_nanos(),
    "a guest that answers within its own bound is always heard"
);

/// Guest re-opening a lost control session.
pub const CONTROL_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);
