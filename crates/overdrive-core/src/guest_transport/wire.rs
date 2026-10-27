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
use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::LaunchToken;
use crate::net::ListenPort;

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
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public constructor"
    )]
    pub fn new(origin: FlowOrigin, seq: u32) -> Result<Self, FlowWireError> {
        if seq >= (1 << 31) {
            return Err(FlowWireError::InvalidField { field: WireField::FlowSeq });
        }
        let origin_bit = match origin {
            FlowOrigin::Guest => 0,
            FlowOrigin::Host => 1 << 31,
        };
        Ok(Self(origin_bit | seq))
    }

    /// The flow's origin (the top bit).
    pub const fn origin(self) -> FlowOrigin {
        if self.0 & (1 << 31) == 0 { FlowOrigin::Guest } else { FlowOrigin::Host }
    }

    /// The raw 32-bit id, origin bit included.
    pub const fn get(self) -> u32 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Flow request (32 bytes)
// ---------------------------------------------------------------------------

/// Length of an encoded [`FlowRequest`].
pub const FLOW_REQUEST_LEN: usize = 32;

/// The three flow kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind {
    /// Kind 1: guest → host, STREAM 1240.
    TcpConnect,
    /// Kind 2: guest → host, STREAM 1240 (the association's one connection,
    /// EVMOD-DGRAM).
    DatagramStream,
    /// Kind 4: host → guest, STREAM 1242.
    TcpAccept,
}

impl FlowKind {
    /// The kind's canonical label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TcpConnect => "tcp_connect",
            Self::DatagramStream => "datagram_stream",
            Self::TcpAccept => "tcp_accept",
        }
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
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public accessor"
    )]
    pub fn kind(&self) -> FlowKind {
        match self {
            Self::TcpConnect { .. } => FlowKind::TcpConnect,
            Self::DatagramStream { .. } => FlowKind::DatagramStream,
            Self::TcpAccept { .. } => FlowKind::TcpAccept,
        }
    }

    /// The request's flow id.
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public accessor"
    )]
    pub fn flow(&self) -> FlowId {
        match self {
            Self::TcpConnect { flow, .. }
            | Self::DatagramStream { flow, .. }
            | Self::TcpAccept { flow, .. } => *flow,
        }
    }

    /// The request's launch token.
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public accessor"
    )]
    pub fn launch(&self) -> LaunchToken {
        match self {
            Self::TcpConnect { launch, .. }
            | Self::DatagramStream { launch, .. }
            | Self::TcpAccept { launch, .. } => *launch,
        }
    }

    /// The canonical 32-byte encoding.
    pub fn encode(&self) -> [u8; FLOW_REQUEST_LEN] {
        let mut bytes = [0; FLOW_REQUEST_LEN];
        bytes[0..2].copy_from_slice(b"OF");
        bytes[2] = 2;
        let (kind, flow, address, port, listen_port, launch) = match self {
            Self::TcpConnect { flow, destination, launch } => {
                (1, *flow, destination.ip().octets(), destination.port(), 0, *launch)
            }
            Self::DatagramStream { flow, destination, launch } => {
                (2, *flow, destination.ip().octets(), destination.port(), 0, *launch)
            }
            Self::TcpAccept { flow, peer, listen_port, launch } => {
                (4, *flow, peer.octets(), 0, listen_port.get(), *launch)
            }
        };
        bytes[3] = kind;
        bytes[4..8].copy_from_slice(&flow.get().to_be_bytes());
        bytes[8..12].copy_from_slice(&address);
        bytes[12..14].copy_from_slice(&port.to_be_bytes());
        bytes[14..16].copy_from_slice(&listen_port.to_be_bytes());
        bytes[16..32].copy_from_slice(launch.as_bytes());
        bytes
    }

    /// Decodes a canonical encoding.
    pub fn decode(bytes: &[u8; FLOW_REQUEST_LEN]) -> Result<Self, FlowWireError> {
        let observed = [bytes[0], bytes[1]];
        if observed != *b"OF" {
            return Err(FlowWireError::BadMagic { observed });
        }
        if bytes[2] != 2 {
            return Err(FlowWireError::UnsupportedVersion { version: bytes[2] });
        }
        let kind = match bytes[3] {
            1 => FlowKind::TcpConnect,
            2 => FlowKind::DatagramStream,
            4 => FlowKind::TcpAccept,
            kind => return Err(FlowWireError::UnknownKind { kind }),
        };
        let flow = FlowId(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
        let expected_origin = match kind {
            FlowKind::TcpConnect | FlowKind::DatagramStream => FlowOrigin::Guest,
            FlowKind::TcpAccept => FlowOrigin::Host,
        };
        if flow.origin() != expected_origin {
            return Err(FlowWireError::OriginMismatch { kind });
        }
        let address = Ipv4Addr::from([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let port = u16::from_be_bytes([bytes[12], bytes[13]]);
        let listen_port_raw = u16::from_be_bytes([bytes[14], bytes[15]]);
        let token_bytes: [u8; 16] = bytes[16..32]
            .try_into()
            .map_err(|_| FlowWireError::InvalidField { field: WireField::LaunchToken })?;
        let launch = LaunchToken::from_bytes(token_bytes)
            .map_err(|_| FlowWireError::InvalidField { field: WireField::LaunchToken })?;

        match kind {
            FlowKind::TcpConnect | FlowKind::DatagramStream => {
                if port == 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::Port });
                }
                if listen_port_raw != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ListenPort });
                }
                let destination = SocketAddrV4::new(address, port);
                if kind == FlowKind::TcpConnect {
                    Ok(Self::TcpConnect { flow, destination, launch })
                } else {
                    Ok(Self::DatagramStream { flow, destination, launch })
                }
            }
            FlowKind::TcpAccept => {
                if port != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::Port });
                }
                let listen_port = ListenPort::new(listen_port_raw)
                    .map_err(|_| FlowWireError::InvalidField { field: WireField::ListenPort })?;
                Ok(Self::TcpAccept { flow, peer: address, listen_port, launch })
            }
        }
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
        let mut bytes = [0; CONTROL_OPEN_LEN];
        bytes[0..2].copy_from_slice(b"OC");
        bytes[2] = 1;
        bytes[8..24].copy_from_slice(self.launch.as_bytes());
        bytes
    }

    /// Decodes a canonical encoding.
    pub fn decode(bytes: &[u8; CONTROL_OPEN_LEN]) -> Result<Self, FlowWireError> {
        let observed = [bytes[0], bytes[1]];
        if observed != *b"OC" {
            return Err(FlowWireError::BadMagic { observed });
        }
        if bytes[2] != 1 {
            return Err(FlowWireError::UnsupportedVersion { version: bytes[2] });
        }
        if bytes[3..8].iter().any(|byte| *byte != 0) {
            return Err(FlowWireError::InvalidField { field: WireField::ControlOpenReserved });
        }
        let token_bytes: [u8; 16] = bytes[8..24]
            .try_into()
            .map_err(|_| FlowWireError::InvalidField { field: WireField::LaunchToken })?;
        let launch = LaunchToken::from_bytes(token_bytes)
            .map_err(|_| FlowWireError::InvalidField { field: WireField::LaunchToken })?;
        Ok(Self { launch })
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
    pub fn new(raw: u16) -> Result<Self, FlowWireError> {
        NonZeroU16::new(raw)
            .map(Self)
            .ok_or(FlowWireError::InvalidField { field: WireField::ControlErrno })
    }

    /// The errno value.
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

/// Decimal form.
impl fmt::Display for KernelErrno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decimal form; see the type's text-form rule.
impl std::str::FromStr for KernelErrno {
    type Err = FlowWireError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !is_decimal(s) {
            return Err(FlowWireError::NotDecimal { field: WireField::ControlErrno });
        }
        let raw = s
            .parse::<u16>()
            .map_err(|_| FlowWireError::NotDecimal { field: WireField::ControlErrno })?;
        Self::new(raw)
    }
}

impl Serialize for KernelErrno {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for KernelErrno {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
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
    /// Code 4: pair table or per-allocation quota exhausted.
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
        match self {
            Self::NotActive => 1,
            Self::Policy => 2,
            Self::IntakeUnavailable => 3,
            Self::Capacity => 4,
            Self::Malformed => 5,
            Self::Unsupported => 6,
            Self::Quiesced => 7,
            Self::MeshUnreachable => 8,
            Self::ResolveFailed => 9,
            Self::DestinationUnreachable => 10,
            Self::ApplicationRefused { .. } => 11,
            Self::HostInternal => 12,
        }
    }

    /// The refusal's canonical label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotActive => "not_active",
            Self::Policy => "policy",
            Self::IntakeUnavailable => "intake_unavailable",
            Self::Capacity => "capacity",
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported",
            Self::Quiesced => "quiesced",
            Self::MeshUnreachable => "mesh_unreachable",
            Self::ResolveFailed => "resolve_failed",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::ApplicationRefused { .. } => "application_refused",
            Self::HostInternal => "host_internal",
        }
    }
}

/// An 8-byte message on the per-VM control session. Pairing replies,
/// timeouts and aborts use this channel; a flow's half-close never touches
/// the session (R2-END).
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
        let (kind, detail, errno, flow) = match self {
            Self::Paired { flow } => (1, 0, 0, *flow),
            Self::Refused { flow, refusal } => {
                let errno = match refusal {
                    FlowRefusal::ApplicationRefused { errno } => errno.get(),
                    _ => 0,
                };
                (2, refusal.as_u8(), errno, *flow)
            }
            Self::Abort { flow } => (3, 0, 0, *flow),
            Self::TimedOut { flow, errno } => (4, 0, errno.get(), *flow),
        };
        let mut bytes = [0; CONTROL_MESSAGE_LEN];
        bytes[0] = kind;
        bytes[1] = detail;
        bytes[2..4].copy_from_slice(&errno.to_be_bytes());
        bytes[4..8].copy_from_slice(&flow.get().to_be_bytes());
        bytes
    }

    /// Decodes a canonical encoding.
    pub fn decode(bytes: &[u8; CONTROL_MESSAGE_LEN]) -> Result<Self, FlowWireError> {
        let kind = bytes[0];
        let detail = bytes[1];
        let errno_raw = u16::from_be_bytes([bytes[2], bytes[3]]);
        let flow = FlowId(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
        match kind {
            1 => {
                if detail != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlDetail });
                }
                if errno_raw != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlErrno });
                }
                Ok(Self::Paired { flow })
            }
            2 => {
                let refusal = match detail {
                    1 => FlowRefusal::NotActive,
                    2 => FlowRefusal::Policy,
                    3 => FlowRefusal::IntakeUnavailable,
                    4 => FlowRefusal::Capacity,
                    5 => FlowRefusal::Malformed,
                    6 => FlowRefusal::Unsupported,
                    7 => FlowRefusal::Quiesced,
                    8 => FlowRefusal::MeshUnreachable,
                    9 => FlowRefusal::ResolveFailed,
                    10 => FlowRefusal::DestinationUnreachable,
                    11 => FlowRefusal::ApplicationRefused { errno: KernelErrno::new(errno_raw)? },
                    12 => FlowRefusal::HostInternal,
                    code => return Err(FlowWireError::UnknownRefusal { code }),
                };
                if !matches!(refusal, FlowRefusal::ApplicationRefused { .. }) && errno_raw != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlErrno });
                }
                Ok(Self::Refused { flow, refusal })
            }
            3 => {
                if detail != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlDetail });
                }
                if errno_raw != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlErrno });
                }
                Ok(Self::Abort { flow })
            }
            4 => {
                if detail != 0 {
                    return Err(FlowWireError::InvalidField { field: WireField::ControlDetail });
                }
                let errno = KernelErrno::new(errno_raw)?;
                Ok(Self::TimedOut { flow, errno })
            }
            ty => Err(FlowWireError::UnknownControlType { ty }),
        }
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
    pub fn new(payload: &'a [u8]) -> Result<Self, DatagramFrameError> {
        if payload.len() > MAX_GUEST_DATAGRAM_PAYLOAD as usize {
            return Err(DatagramFrameError::Oversize {
                length: u64::try_from(payload.len()).unwrap_or(u64::MAX),
                max: MAX_GUEST_DATAGRAM_PAYLOAD,
            });
        }
        Ok(Self { payload })
    }

    /// The logical payload.
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public accessor"
    )]
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// Header + payload, pre-sized (`DATAGRAM_FRAME_HEADER_LEN + payload.len()`).
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(DATAGRAM_FRAME_HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&DATAGRAM_FRAME_MAGIC);
        let length = u32::try_from(self.payload.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(self.payload);
        bytes
    }

    /// Accepts exactly one canonical frame: magic, length ≤ max, and
    /// `bytes.len() == DATAGRAM_FRAME_HEADER_LEN + length`.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, DatagramFrameError> {
        if bytes.len() < DATAGRAM_FRAME_HEADER_LEN {
            return Err(DatagramFrameError::Truncated { len: bytes.len() });
        }
        let observed: [u8; 4] = bytes
            .get(..4)
            .and_then(|slice| slice.try_into().ok())
            .ok_or(DatagramFrameError::Truncated { len: bytes.len() })?;
        if observed != DATAGRAM_FRAME_MAGIC {
            return Err(DatagramFrameError::BadMagic { observed });
        }
        let length_bytes: [u8; 4] = bytes
            .get(4..8)
            .and_then(|slice| slice.try_into().ok())
            .ok_or(DatagramFrameError::Truncated { len: bytes.len() })?;
        let declared = u32::from_be_bytes(length_bytes);
        if declared > MAX_GUEST_DATAGRAM_PAYLOAD {
            return Err(DatagramFrameError::Oversize {
                length: u64::from(declared),
                max: MAX_GUEST_DATAGRAM_PAYLOAD,
            });
        }
        let payload = bytes
            .get(DATAGRAM_FRAME_HEADER_LEN..)
            .ok_or(DatagramFrameError::Truncated { len: bytes.len() })?;
        if payload.len() != declared as usize {
            return Err(DatagramFrameError::LengthMismatch { declared, actual: payload.len() });
        }
        Ok(Self { payload })
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

fn is_decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Deadlines (one SSOT)
// ---------------------------------------------------------------------------

/// Acceptor reading a flow request.
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
