//! The flow forwarder's core vocabulary and driven port (EVMOD-PORT, GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] vsock data path — the in-kernel event-driven forwarder (EVMOD)"
//! → "[REF] Driven port — `FlowForwarder` (EVMOD-PORT)". Pure data and the
//! port trait; no I/O. One SSOT for the host owner
//! (`overdrive-control-plane::guest_network`), `overdrive-init`, the kernel
//! adapter (`overdrive-fwd::KernelFlowForwarder`) and the sim
//! (`overdrive-sim::SimFlowForwarder`). The end record these types decode
//! from (EVMOD-REC) never leaves the machine; nothing here is a wire or
//! persisted format other than the decimal text form of the two newtypes.
//!
//! RED scaffold (DISTILL, Mandate 7): the validating constructors, the
//! accessors, the text and serde forms of [`ForwardId`] and
//! [`ForwarderGeneration`], and the `as_str` labels panic until DELIVER
//! step 2 ("core types and wire codec") lands them. Plain data — the enums,
//! the structs, [`ForwarderGeneration::FIRST`] and the error `Display` texts
//! — is real. No surface the design does not pin is added here.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock DELIVER step 2).
#![allow(
    clippy::unused_self,
    clippy::trivially_copy_pass_by_ref,
    reason = "RED scaffold bodies panic before reading their inputs; `AbortCause::as_str(&self)` is the pinned signature"
)]

use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::wire::KernelErrno;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// ForwardId
// ---------------------------------------------------------------------------

/// A forwarded pair or a watched vsock, named by the forwarder context that
/// created it. Never reused within one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForwardId(NonZeroU64);

impl ForwardId {
    /// `0` → [`ForwardIdError::Zero`].
    pub fn new(_raw: u64) -> Result<Self, ForwardIdError> {
        panic!("{SCAFFOLD}")
    }

    /// The raw id.
    pub const fn get(self) -> u64 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for ForwardId {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; `0` is `Zero`, any other non-decimal text is `NotDecimal`.
impl FromStr for ForwardId {
    type Err = ForwardIdError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

/// Matches `Display`.
impl Serialize for ForwardId {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Matches `FromStr`.
impl<'de> Deserialize<'de> for ForwardId {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a value is not a [`ForwardId`] or a [`ForwarderGeneration`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForwardIdError {
    /// The value 0.
    #[error("forward id is zero")]
    Zero,
    /// Text that is not a decimal `u64`.
    #[error("forward id is not a decimal integer")]
    NotDecimal,
}

// ---------------------------------------------------------------------------
// ForwarderGeneration
// ---------------------------------------------------------------------------

/// The generation of an owner's forwarder context: 1 for the first context
/// an owner opens, + 1 for each later open in the same process (only the
/// host's `Forwarder` recovery opens a later one). A [`ForwardId`] names a
/// pair only together with the generation it was issued in. Observation
/// only (QPROJ-8); no kernel ABI carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForwarderGeneration(NonZeroU64);

impl ForwarderGeneration {
    /// The first context's generation, 1.
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    /// `0` → [`ForwardIdError::Zero`].
    pub fn new(_raw: u64) -> Result<Self, ForwardIdError> {
        panic!("{SCAFFOLD}")
    }

    /// The raw generation.
    pub const fn get(self) -> u64 {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// The next generation; `None` only at `u64::MAX`.
    pub const fn next(self) -> Option<Self> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for ForwarderGeneration {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; `0` is `Zero`, any other non-decimal text is `NotDecimal`.
impl FromStr for ForwarderGeneration {
    type Err = ForwardIdError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

/// Matches `Display`.
impl Serialize for ForwarderGeneration {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Matches `FromStr`.
impl<'de> Deserialize<'de> for ForwarderGeneration {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// Pairs, outcomes and state
// ---------------------------------------------------------------------------

/// The half of a pair, in attach order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairSide {
    /// The first half named at attach.
    A,
    /// The second half named at attach.
    B,
}

impl PairSide {
    /// The side's canonical label.
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// What the two halves of a pair are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairMode {
    /// Both halves stream sockets (one vsock STREAM, one TCP).
    Stream,
    /// One vsock STREAM half carrying ADR-0149 frames and one connected UDP
    /// half, named by `udp` (ADR-0201).
    Datagram {
        /// The UDP half.
        udp: PairSide,
    },
}

/// Why a pair or a watch aborted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortCause {
    /// The vsock half's peer closed or reset with a direction open, or
    /// stopped receiving while this side still sent (EVMOD-END).
    PeerAbort {
        /// The vsock half.
        side: PairSide,
    },
    /// A `recvmsg` / `sendmsg` of that half failed (a TCP reset, `EPIPE`, the
    /// device release's `ECONNRESET`, a UDP `ECONNREFUSED` or `EPERM`).
    SocketError {
        /// The failing half.
        side: PairSide,
        /// Its errno.
        errno: KernelErrno,
    },
    /// Datagram mode: a frame with a bad magic or an oversized length, or a
    /// frame left incomplete when the vsock peer ended its direction with a
    /// clean-class end (`0x2`; an abort-class end is `PeerAbort`).
    FrameInvalid {
        /// The vsock half.
        side: PairSide,
    },
    /// The owner's `abort` or `abort_all`.
    Detached,
    /// An end-of-stream on that vsock half whose class was never recorded:
    /// a broken invariant (record cause 5), counted `unclassified_eof`.
    Unclassified {
        /// The vsock half.
        side: PairSide,
    },
}

impl AbortCause {
    /// The cause's canonical label (the cause class; never its side or
    /// errno).
    pub const fn as_str(&self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// How a pair or a watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardOutcome {
    /// Pair: both directions ended cleanly.
    Completed,
    /// Watch: the guest's direction ended with a clean half-close.
    GuestHalfClose,
    /// Pair or watch.
    Aborted {
        /// Why.
        cause: AbortCause,
    },
}

/// One id's end record, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardEnd {
    /// The pair or watch.
    pub id: ForwardId,
    /// How it ended.
    pub outcome: ForwardOutcome,
    /// Bytes accepted by half B from half A (payload bytes in datagram
    /// mode; zero for a watch).
    pub moved_ab: u64,
    /// Bytes accepted by half A from half B.
    pub moved_ba: u64,
}

/// How a direction's source ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEnd {
    /// A clean end (a TCP FIN, a vsock `0x2`).
    Clean,
    /// An abort-class end.
    Abort,
}

/// One direction of a live pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectionState {
    /// The source's end, once it has ended.
    pub source_end: Option<SourceEnd>,
    /// The forwarder has shut the target's send direction.
    pub target_shut: bool,
    /// Bytes the target accepted.
    pub moved: u64,
}

/// Read-only state of a live pair (the conformance pairing's accessor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairState {
    /// Half A to half B.
    pub a_to_b: DirectionState,
    /// Half B to half A.
    pub b_to_a: DirectionState,
}

/// One context's inventory and cumulative counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ForwarderInventory {
    /// Pairs attached and not yet ended.
    pub live_pairs: u32,
    /// Watches installed and not yet ended.
    pub live_watches: u32,
    /// End records queued and not yet read.
    pub queued_ends: u32,
    /// Cumulative pairs attached.
    pub attached: u64,
    /// Cumulative pairs completed.
    pub completed: u64,
    /// Cumulative pairs and watches aborted.
    pub aborted: u64,
    /// Cumulative ends with no recorded class (an alarm).
    pub unclassified_eof: u64,
    /// Cumulative datagrams above `MAX_GUEST_DATAGRAM_PAYLOAD` consumed at
    /// the UDP source.
    pub dgram_oversize: u64,
}

/// The result of an `abort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortResult {
    /// The id was live; its `Detached` record is queued.
    Aborted,
    /// The id's record was already queued or read, or the id is unknown to
    /// this context.
    AlreadyEnded,
}

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// The forwarder of one side, through one context. `S` is the adapter's
/// socket handle: `dyn std::os::fd::AsFd` for the kernel adapter, the sim's
/// socket type for the sim. Every method is synchronous and non-blocking.
///
/// # Contract
///
/// - **Preconditions.** `attach`: exactly one half is a vsock STREAM and, in
///   [`PairMode::Stream`], the other is a TCP socket, in
///   [`PairMode::Datagram`] the named half a UDP socket; each half in a state
///   the control interface's accepted-state table admits (a half whose peer
///   already ended its direction is admitted); neither half carries a ULP;
///   neither is attached or watched; the caller has not shut either half's
///   send direction. `watch`: one vsock STREAM in an admitted state. Every
///   violation is a typed error and changes nothing. A half that has ended
///   returns [`ForwarderError::HalfEnded`], which the caller handles as the
///   end of the flow.
/// - **Postconditions.** `attach` returns a live id; the forwarder holds a
///   reference to both halves and has queued one run per direction. `abort`
///   returns [`AbortResult::Aborted`] and the id's record
///   ([`AbortCause::Detached`]) is queued before it returns, or
///   [`AbortResult::AlreadyEnded`] when the id's record was already queued or
///   read. `abort_all` aborts every live id of the context. `take_end`
///   returns each id's record once.
/// - **Edge cases.** An id of another context, or unknown, is
///   `AlreadyEnded` for `abort` and `None` for `pair_state`. A pair whose
///   both sources have already ended cleanly when attached completes in its
///   first run; one with a vsock half whose peer sent `0x3` or an `RST`
///   while bytes were still queued aborts in its first run; one with a half
///   reset before attach, or a vsock half in `CLOSING`, is refused
///   `HalfEnded`. A watch on a vsock whose guest direction already ended
///   queues its record at once. `abort` of a watch queues its record,
///   unhooks it and drops the forwarder's reference, and changes nothing on
///   the socket (WATCH-ABORT); only the context's release shuts a watched
///   vsock. In datagram mode a vsock half whose `buffer_size` is below
///   `MAX_GUEST_DATAGRAM_FRAME` is `BufferTooSmall`. An
///   [`AbortCause::Unclassified`] outcome is a broken invariant: the owner
///   handles it as an abort and reports it as an alarm.
/// - **Observable invariants.** At most one record per id; no record after
///   the context's release; `live_pairs + live_watches` equals the ids
///   attached or watched minus the records queued or read; the forwarder
///   never shuts a target before its source's clean end and never forwards a
///   byte past the first abort.
pub trait FlowForwarder<S: ?Sized>: Send + Sync {
    /// Earned Trust. Kernel adapter: the probe stages.
    fn probe(&self) -> Result<(), ForwarderError>;
    /// Attaches the pair `(a, b)` in `mode`.
    fn attach(&self, a: &S, b: &S, mode: PairMode) -> Result<ForwardId, ForwarderError>;
    /// Watches one vsock STREAM.
    fn watch(&self, vsock: &S) -> Result<ForwardId, ForwarderError>;
    /// Aborts one id of this context.
    fn abort(&self, id: ForwardId) -> Result<AbortResult, ForwarderError>;
    /// Aborts every live id of this context; returns how many.
    fn abort_all(&self) -> Result<u32, ForwarderError>;
    /// The oldest queued end record, `None` when none is queued.
    fn take_end(&self) -> Result<Option<ForwardEnd>, ForwarderError>;
    /// The state of a live pair of this context; `None` otherwise.
    fn pair_state(&self, id: ForwardId) -> Result<Option<PairState>, ForwarderError>;
    /// This context's inventory and counters.
    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError>;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The control-interface operation an `Io` error names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwarderOp {
    /// Opening the device.
    Open,
    /// `OVD_FWD_VERSION`.
    Version,
    /// `OVD_FWD_ATTACH`.
    Attach,
    /// `OVD_FWD_WATCH`.
    Watch,
    /// `OVD_FWD_ABORT`.
    Abort,
    /// `OVD_FWD_ABORT_ALL`.
    AbortAll,
    /// `read()` of end records.
    Read,
    /// `OVD_FWD_INVENTORY`.
    Inventory,
    /// `OVD_FWD_PAIR_STATE`.
    PairState,
}

/// The stages of the forwarder probe (Earned Trust).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwarderProbeStage {
    /// The device opens.
    DeviceOpen,
    /// ABI, permanence and `FD_CLOEXEC`.
    Version,
    /// Bytes both ways, byte-exact.
    StreamRoundTrip,
    /// A tail after `SHUT_WR`, then end-of-stream, then `Completed`.
    HalfClose,
    /// A vsock peer closed with a direction open is `PeerAbort`.
    PeerAbortClassified,
    /// A target that does not read stops the sender.
    Backpressure,
    /// 0, 1 and 59,000-byte datagrams and an empty one, in order.
    DatagramRoundTrip,
    /// `0x2` → `GuestHalfClose`, `0x3` with a direction open → `Aborted`.
    WatchClassified,
    /// A second context closed with a live pair resets its TCP far end.
    ContextRelease,
    /// Inventory zero.
    Cleanup,
}

/// What a probe stage observed instead of its expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwarderProbeObservation {
    /// The bytes received differ from the bytes sent.
    BytesDiffer,
    /// Nothing was received.
    NothingReceived,
    /// The tail after a half-close was truncated.
    TailTruncated,
    /// The end record's outcome was not the expected one.
    WrongOutcome,
    /// The sender was not blocked by a target that did not read.
    SenderNotBlocked,
    /// A datagram was split or merged.
    DatagramSplit,
    /// The far end was not reset by the context's release.
    FarEndNotReset,
    /// Pairs remained after the probe.
    ResidueRemains {
        /// How many.
        pairs: u32,
    },
    /// The context descriptor lacks `FD_CLOEXEC` (EV-CTL pin 4.8).
    ContextInheritable,
}

/// The end-record field an `EndDecode` error names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardRecordField {
    /// `id` (zero).
    Id,
    /// `kind` (undefined).
    Kind,
    /// `cause` (undefined, or non-zero on a kind without one).
    Cause,
    /// `side` (neither 0 nor 1, or non-zero where unused).
    Side,
    /// `errno` (zero for cause 2, or non-zero elsewhere).
    Errno,
    /// A reserved byte.
    Reserved,
}

/// Every failure of the forwarder port.
#[derive(Debug, thiserror::Error)]
pub enum ForwarderError {
    /// The device cannot be opened.
    #[error("the flow forwarder device cannot be opened")]
    DeviceAbsent {
        /// The open's error.
        #[source]
        source: std::io::Error,
    },
    /// The module's ABI is not the one this build expects.
    #[error("forwarder ABI {found} is not the expected {expected}")]
    AbiMismatch {
        /// This build's ABI.
        expected: u32,
        /// The module's ABI.
        found: u32,
    },
    /// A half of an unsupported kind for the mode, or not exactly one vsock
    /// STREAM half.
    #[error("socket {side:?} is not a supported kind for this pair")]
    UnsupportedSocket {
        /// The offending half.
        side: PairSide,
    },
    /// A TCP half with a ULP.
    #[error("socket {side:?} carries an upper-layer protocol")]
    UpperLayerProtocol {
        /// The offending half.
        side: PairSide,
    },
    /// A half that never connected, listens, or whose own send this side
    /// shut (an owner defect).
    #[error("socket {side:?} is not connected")]
    NotConnected {
        /// The offending half.
        side: PairSide,
    },
    /// EVMOD-ATTACH-END: the half was connected and has ended (a pending
    /// reset error, a vsock `CLOSING` or the device release's sweep);
    /// `errno` is its cause (`end_errno`). Not an owner defect: the owner
    /// aborts the flow.
    #[error("socket {side:?} has already ended: errno {errno}")]
    HalfEnded {
        /// The ended half.
        side: PairSide,
        /// Its cause.
        errno: KernelErrno,
    },
    /// The half is already attached or watched.
    #[error("socket {side:?} is already attached or watched")]
    AlreadyHooked {
        /// The offending half.
        side: PairSide,
    },
    /// Datagram mode: the vsock half's buffer is below the largest frame.
    #[error("vsock socket {side:?} has a buffer smaller than the largest datagram frame")]
    BufferTooSmall {
        /// The vsock half.
        side: PairSide,
    },
    /// A control operation failed.
    #[error("forwarder operation {op:?} failed")]
    Io {
        /// The operation.
        op: ForwarderOp,
        /// Its error.
        #[source]
        source: std::io::Error,
    },
    /// An end record failed to decode.
    #[error("a forwarder end record failed to decode")]
    EndDecode {
        /// The offending field.
        field: ForwardRecordField,
    },
    /// A probe stage failed with an error.
    #[error("forwarder probe stage {stage:?} failed")]
    Probe {
        /// The stage.
        stage: ForwarderProbeStage,
        /// The stage's error.
        #[source]
        source: Box<Self>,
    },
    /// A probe stage observed something other than its expectation.
    #[error("forwarder probe stage {stage:?} observed {observed:?}")]
    ProbeMismatch {
        /// The stage.
        stage: ForwarderProbeStage,
        /// What it observed.
        observed: ForwarderProbeObservation,
    },
}
