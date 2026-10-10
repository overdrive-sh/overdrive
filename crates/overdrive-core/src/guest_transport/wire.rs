//! Guest transport wire contracts — flow request, control open, control
//! message, datagram frame constants and deadlines (D5, D18, D18a, D23,
//! ATTR-TOKEN, HALFCLOSE-SIGNAL).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Wire contracts — flow request, control message, datagram frame
//! (D5, D18, D23)". One SSOT consumed by the host owner, the host forwarder,
//! `overdrive-init` and the guest programs. `decode` accepts only canonical
//! encodings, so round-trip equality holds over every valid value and every
//! accepted byte string.
//!
//! RED scaffold (DISTILL, Mandate 7): every function and method body panics
//! until DELIVER vsock new-step 2 ("core types and wire codec") lands it.
//! Constants carry the values the design pins.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 2).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lint self-trips when DELIVER lands the bodies"
)]

use std::net::SocketAddrV4;
use std::time::Duration;

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
    /// Kind 2: guest → host, STREAM 1240.
    DatagramStream,
    /// Kind 3: guest → host, SEQPACKET 1241.
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
    /// Kind 3.
    DatagramSeqpacket {
        /// Guest-origin flow id.
        flow: FlowId,
        /// Original destination.
        destination: SocketAddrV4,
        /// The opening launch's token.
        launch: LaunchToken,
    },
    /// Kind 4.
    TcpAccept {
        /// Host-origin flow id.
        flow: FlowId,
        /// The intake connection's kernel-reported peer.
        client: SocketAddrV4,
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

/// Why a flow was refused (codes 1..=12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRefusal {
    /// Code 1.
    NotActive,
    /// Code 2.
    Policy,
    /// Code 3.
    IntakeUnavailable,
    /// Code 4.
    Capacity,
    /// Code 5.
    Malformed,
    /// Code 6.
    Unsupported,
    /// Code 7.
    Quiesced,
    /// Code 8.
    MeshUnreachable,
    /// Code 9.
    ResolveFailed,
    /// Code 10.
    DestinationUnreachable,
    /// Code 11.
    ApplicationNotListening,
    /// Code 12.
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

/// An 8-byte message on the per-VM control session.
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
    /// Type 4: guest → host listen state of one port.
    ListenState {
        /// The guest listen port.
        port: ListenPort,
        /// Whether it listens.
        listening: bool,
    },
    /// Type 5 (HALFCLOSE-SIGNAL): the sender's direction of `flow` is drained
    /// and half-closed; it shuts its flow vsock for writing only after
    /// `HalfCloseAck`.
    HalfClose {
        /// The flow.
        flow: FlowId,
    },
    /// Type 6 (HALFCLOSE-SIGNAL): the receiver holds the peer's `HalfClose`.
    HalfCloseAck {
        /// The flow.
        flow: FlowId,
    },
}

#[expect(
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
    /// The control message's detail byte.
    ControlDetail,
    /// The control message's port field.
    ControlPort,
    /// The control message's flow field.
    ControlFlow,
    /// A launch token field (all zero).
    LaunchToken,
    /// A reserved byte of the control open.
    ControlOpenReserved,
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
/// One framed datagram (§ Wire contracts, Datagram frame; DG-8). The Rust
/// codec of the frame: the forwarder probe (`EmptyDatagram`, `MaxDatagram`,
/// `EscapedFrame`) and the escape rule's test vectors use it, and the golden
/// empty and maximum frames are asserted equal across this codec, the host
/// programs and the guest programs (P3-S1).
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

/// `max_entries` of the forwarder's fragment-tracking LRU map
/// (FRAG-MAP-LIFECYCLE, ADR-0165).
pub const GUEST_UNFRAME_FRAGMENTS_MAX: u32 = 4_096;

// ---------------------------------------------------------------------------
// Deadlines (one SSOT)
// ---------------------------------------------------------------------------

/// Acceptor reading a request; host matching datagram kinds 2 and 3.
pub const FLOW_REQUEST_DEADLINE: Duration = Duration::from_secs(2);
/// Opener waiting for `Paired` / `Refused`: datagram associations and
/// `TcpAccept`.
pub const FLOW_PAIRING_DEADLINE: Duration = Duration::from_secs(5);
/// K2 drain before half-close; expiry aborts.
pub const FLOW_DRAIN_DEADLINE: Duration = Duration::from_secs(10);
/// Guest re-opening a lost control session.
pub const CONTROL_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);
