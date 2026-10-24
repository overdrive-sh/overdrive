//! Guest flow owner — the guest half of the vsock attachment replacement
//! (GH #295): the control session, the flow table and the UDP slot pool of
//! `overdrive-init`, over the guest-owner seam [`GuestOwnerIo`] and the guest
//! forwarder context (`FlowForwarder`, EVMOD-PORT).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § DG-5 ("Guest flow owner", module `guest_flow`; DG-5-A, DG-5-D, DG-5-E,
//! DG-5-F, DG-5-Q), § *Guest adaptation contract* (guest intake model,
//! TcpAccept guest side, flow end, QPROJ-5 `guest_observation`), § *[REF] vsock
//! data path* ("Owners' use of the forwarder", "Owner response to an ended half
//! at attach", EVMOD-PAIR, EVMOD-REC, EVMOD-DGRAM), § *Per-kind total orders*
//! and § *Control session*. Every item below is the one those sections pin.
//!
//! The production owner ([`GuestFlowOwner`]) runs over the guest-owner port
//! ([`GuestOwnerIo`]); [`KernelGuestOwnerIo`] supplies the guest-kernel
//! adapter. The source-local [`sim`] fixture exercises the same owner over
//! `overdrive-sim` sockets and its `SimFlowForwarder` (decision B-1).

#![cfg_attr(not(test), expect(dead_code, reason = "some crate-private observations are test-only"))]
#![cfg_attr(test, allow(dead_code, reason = "test scenarios use only relevant owner projections"))]
#![allow(
    clippy::redundant_pub_crate,
    reason = "DG-5 pins `pub(crate)` visibility for these binary-crate items"
)]
#![allow(
    clippy::doc_markdown,
    reason = "protocol leg names (V_g, V_h, I, C, S) and kernel names are prose, not code"
)]

#[cfg(all(test, feature = "integration-tests"))]
mod quint_fwd_pair;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_itf;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_owner_flows;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_udp_slots;
// In-guest real-kernel cells (D-UP-18): the native metal microVM lane only.
#[cfg(all(test, feature = "integration-tests", feature = "kvm-tests"))]
mod real_kernel;
#[cfg(test)]
pub(crate) mod sim;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::SocketAddrV4;
use std::num::NonZeroU64;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use overdrive_core::guest_transport::forwarder::{
    AbortResult, ForwardEnd, ForwardId, ForwarderError, ForwarderInventory, PairState,
};
use overdrive_core::guest_transport::wire::{FLOW_REQUEST_LEN, FlowId};

use crate::transport::GuestTransportToken;

// ---------------------------------------------------------------------------
// Newtypes
// ---------------------------------------------------------------------------

/// Kernel socket cookie of a guest socket; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SocketCookie(NonZeroU64);

impl SocketCookie {
    /// Validates a raw cookie (`0` is never a kernel cookie).
    pub(crate) fn new(raw: u64) -> Result<Self, SocketCookieError> {
        NonZeroU64::new(raw).map(Self).ok_or(SocketCookieError::Zero)
    }

    /// The raw cookie.
    pub(crate) const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Decimal form.
impl fmt::Display for SocketCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decimal form; validated by [`SocketCookie::new`].
impl FromStr for SocketCookie {
    type Err = SocketCookieError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(SocketCookieError::Unparseable);
        }
        let raw = s.parse::<u64>().map_err(|_| SocketCookieError::Unparseable)?;
        Self::new(raw)
    }
}

/// Why a value is not a [`SocketCookie`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SocketCookieError {
    /// `0` is never a kernel socket cookie.
    #[error("socket cookie is zero")]
    Zero,
    /// Not a decimal integer.
    #[error("socket cookie is not a decimal integer")]
    Unparseable,
}

// ---------------------------------------------------------------------------
// Guest socket handles
// ---------------------------------------------------------------------------

/// Correlates an event with a handle the owner holds; carries no capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SocketId(u64);

/// The seam instance a handle was issued by, and the socket it names.
#[derive(Debug, PartialEq, Eq)]
struct Handle {
    seam: SeamInstance,
    id: SocketId,
}

/// Identity of one seam instance: a handle of another instance is unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeamInstance(u64);

impl SeamInstance {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// vsock to the host on a [`HostChannel`]: the control session, a TcpConnect
/// V_g, a datagram V_g.
#[must_use]
#[derive(Debug)]
pub(crate) struct HostConn(Handle);
/// V_g accepted on `GUEST_INBOUND_PORT` (a dial).
#[must_use]
#[derive(Debug)]
pub(crate) struct InboundConn(Handle);
/// I: the TCP intake's child for an application's captured `connect()`,
/// unattached, observed only for its error.
#[must_use]
#[derive(Debug)]
pub(crate) struct CapturedConn(Handle);
/// C: connecting toward the application (TcpAccept).
#[must_use]
#[derive(Debug)]
pub(crate) struct AppConn(Handle);
/// A half attached to the forwarder: linger, abort, close only.
#[must_use]
#[derive(Debug)]
pub(crate) struct GuestPairSocket(Handle);

/// A pair the guest forwarder accepted: its id and the two attached halves.
#[must_use]
#[derive(Debug)]
pub(crate) struct GuestPair {
    /// The forwarder's id of the pair.
    pub(crate) id: ForwardId,
    /// V_g.
    pub(crate) vsock: GuestPairSocket,
    /// I (TcpConnect) or C (TcpAccept).
    pub(crate) outer: GuestPairSocket,
}

/// An attach the forwarder refused, with both halves handed back unattached.
#[must_use]
#[derive(Debug)]
pub(crate) struct GuestAttachRefused {
    /// The forwarder's refusal (`HalfEnded` is the end of the flow,
    /// EVMOD-ATTACH-END).
    pub(crate) error: ForwarderError,
    /// The vsock half, unattached.
    pub(crate) vsock: GuestOwnedSocket,
    /// The outer half, unattached.
    pub(crate) outer: GuestOwnedSocket,
}

/// A datagram attach the forwarder refused while the slot socket remains
/// resident and only the consumed V_g is returned.
#[must_use]
#[derive(Debug)]
pub(crate) struct GuestSlotAttachRefused {
    /// The forwarder's refusal, with its existing semantic variant and payload.
    pub(crate) error: ForwarderError,
    /// The consumed flow vsock, unattached.
    pub(crate) vsock: HostConn,
}

/// Any guest socket handle, for `abort` / `close`.
#[must_use]
#[derive(Debug)]
pub(crate) enum GuestOwnedSocket {
    /// A host channel connection.
    Host(HostConn),
    /// An accepted V_g.
    Inbound(InboundConn),
    /// A captured intake child.
    Captured(CapturedConn),
    /// An application connection.
    App(AppConn),
    /// An attached half.
    Pair(GuestPairSocket),
}

macro_rules! handle_impls {
    ($($ty:ident => $variant:ident),* $(,)?) => {
        $(
            impl $ty {
                /// The socket this handle names.
                pub(crate) const fn id(&self) -> SocketId {
                    self.0.id
                }
            }

            impl From<$ty> for GuestOwnedSocket {
                fn from(handle: $ty) -> Self {
                    Self::$variant(handle)
                }
            }
        )*
    };
}

handle_impls!(
    HostConn => Host,
    InboundConn => Inbound,
    CapturedConn => Captured,
    AppConn => App,
    GuestPairSocket => Pair,
);

impl GuestOwnedSocket {
    /// The socket this handle names.
    pub(crate) const fn id(&self) -> SocketId {
        match self {
            Self::Host(h) => h.id(),
            Self::Inbound(h) => h.id(),
            Self::Captured(h) => h.id(),
            Self::App(h) => h.id(),
            Self::Pair(h) => h.id(),
        }
    }
}

// ---------------------------------------------------------------------------
// Event and value vocabulary
// ---------------------------------------------------------------------------

/// Terminal readiness of an unattached socket (never `POLLIN` / `POLLOUT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestHangup {
    /// `POLLRDHUP`: the peer ended its writes.
    ReadHangup,
    /// `POLLHUP`.
    Hangup,
    /// `POLLERR`, with the socket error.
    Error(std::io::ErrorKind),
}

/// The host vsock channels the guest opens (CID 2); both `SOCK_STREAM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HostChannel {
    /// The boot token's `stream=` port (GUEST-PORTS, ADR-0217; default 1240):
    /// TcpConnect and datagram flows.
    FlowStream,
    /// The boot token's `control=` port (default 1243).
    Control,
}

/// The only reads guest userspace makes ("No payload in guest userspace").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestRecord {
    /// `CONTROL_MESSAGE_LEN` (8).
    ControlMessage,
    /// `FLOW_REQUEST_LEN` (32), on V_g.
    FlowRequest,
}

/// The guest's listeners whose failure the owner cannot repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestListener {
    /// `GUEST_INBOUND_PORT`.
    Inbound,
    /// `GUEST_TCP_INTAKE_PORT`.
    TcpIntake,
}

/// Every input event of the guest owner. Each is delivered exactly once.
#[derive(Debug)]
pub(crate) enum GuestIoEvent {
    /// A connect from `start_connect_host` / `start_connect_app` completed.
    Connected {
        /// The connecting socket.
        socket: SocketId,
        /// The connect's outcome.
        result: Result<(), std::io::ErrorKind>,
    },
    /// A `start_read_*` completed: exactly the record's bytes, `Ok(empty)` at
    /// EOF before any byte, `Err(UnexpectedEof)` at EOF mid-record.
    Read {
        /// The read socket.
        socket: SocketId,
        /// The record, EOF, or the read error.
        result: Result<Vec<u8>, std::io::ErrorKind>,
    },
    /// V_g accepted on `GUEST_INBOUND_PORT`.
    InboundAccepted {
        /// The accepted V_g.
        conn: InboundConn,
    },
    /// The TCP intake accepted an application's captured `connect()`. The
    /// child I is not attached; the application's early bytes wait in its
    /// receive queue (EVMOD-PAIR). The owner resets it at once when it
    /// already holds `GUEST_TCP_PENDING_MAX` pending flows.
    OutboundCaptured {
        /// I.
        child: CapturedConn,
        /// The application's original destination.
        original_destination: SocketAddrV4,
        /// The application socket's cookie.
        cookie: SocketCookie,
    },
    /// Terminal readiness on an unattached socket: the control session's
    /// `HostConn`, a `CapturedConn` before `Paired`, a flow V_g before its
    /// attach (an `InboundConn` or a flow `HostConn`, DG-5-E) are watched
    /// from issue (level-triggered: reported once per socket per kind until
    /// the socket ends). An attached socket is not watched: its end is its
    /// pair's `ForwardEnd`.
    Hangup {
        /// The socket.
        socket: SocketId,
        /// The readiness kind.
        hangup: GuestHangup,
    },
    /// A slot socket became readable (no read; the first datagram of an
    /// association, or of the next one after an end, U-2).
    SlotReady {
        /// The slot socket's bound address.
        slot: SocketAddrV4,
    },
    /// A `sock_release` report (ring buffer).
    SocketReleased {
        /// The released socket.
        cookie: SocketCookie,
    },
    /// An end record of the guest forwarder context (`ForwardEnd`,
    /// EVMOD-REC).
    ForwardEnded {
        /// The record.
        end: ForwardEnd,
    },
    /// A listener's accept failed with an error other than a per-connection
    /// one (a connection aborted before it was accepted, and `EMFILE` /
    /// `ENFILE`, which the seam retries).
    ListenerFailed {
        /// The listener.
        listener: GuestListener,
        /// The accept error.
        kind: std::io::ErrorKind,
    },
}

// ---------------------------------------------------------------------------
// The guest-owner seam
// ---------------------------------------------------------------------------

/// The guest owner's effect seam (DG-5). Synchronous: `overdrive-init` has no
/// async runtime (std + nix).
///
/// Contract (every method):
///
/// - **Preconditions are types.** Handles are move-only and exist from the
///   call or event that issued them to the call that consumed them; the
///   per-kind orders of § *Per-kind total orders* are the only orders the
///   types admit (TcpConnect: captured → connect host → `write_request` →
///   `Paired` → `attach_outbound`; TcpAccept: inbound → read →
///   `inbound_ended` → `start_connect_app` → `Connected` → `attach_inbound`
///   → `Paired`; datagram: slot ready → connect host → `write_slot_request`
///   → `Paired` → `attach_slot` → end record or abort → `end_association` →
///   `release_slot`). An operation on a `HostConn` / `AppConn` before its
///   `Connected` event fails with the kernel's `NotConnected`, which the owner
///   never issues. A handle from another seam instance is unreachable
///   (`unreachable!`).
/// - **Attaches are all-or-nothing.** Ok: the guest forwarder holds both
///   halves. Err: it holds neither, and the halves come back in
///   [`GuestAttachRefused`] for the owner to close by "Owner response to an
///   ended half at attach".
/// - **Writes** (`write_host`, `write_request`, `write_slot_request`) write
///   the whole record or fail; a failed write may have sent a prefix the peer
///   reads as a truncated record, so the owner ends that connection (or, on
///   the session, takes the session-loss path).
/// - **Events.** Each [`GuestIoEvent`] is delivered once; `Connected` /
///   `Read` name a handle the owner still holds or one it has since consumed
///   (the owner discards the latter). A `Hangup` is level-triggered and
///   reported once per socket per kind. A `ForwardEnded` names an id of the
///   guest's one context; each id's record is delivered once (EVMOD-REC).
/// - **An accepted, unattached V_g is observed (DG-5-E).** A `Hangup` on a
///   flow V_g before its attach ends the flow in the dispatch that receives
///   it: an error or reset aborts it and sends `Abort`; `ReadHangup` is the
///   host's close of V_h, so the flow ends with no message.
/// - **No payload.** Only `HostConn` (session and request records) and
///   `InboundConn` (the request) are ever read, and only whole fixed records.
pub(crate) trait GuestOwnerIo: Send {
    /// Blocks until the next event (`Some`) or until `deadline` passes on the
    /// owner's clock (`None`; never `None` before it, never with an event
    /// pending). `deadline = None`: waits for an event. Err: the readiness
    /// wait itself failed (fatal to the owner).
    fn next_event(&mut self, deadline: Option<Instant>) -> std::io::Result<Option<GuestIoEvent>>;

    // --- host channels and records ---
    /// Non-blocking vsock connect to the host (CID 2) on `channel`, with the
    /// channel's socket type; completion is a `Connected` event.
    fn start_connect_host(&mut self, channel: HostChannel) -> std::io::Result<HostConn>;
    /// Starts reading exactly one `record` on a connected `HostConn` (the
    /// control session: `ControlMessage`); completion is a `Read` event.
    fn start_read_host(&mut self, conn: &HostConn, record: GuestRecord) -> std::io::Result<()>;
    /// Starts reading the 32-byte `TcpAccept` request on V_g.
    fn start_read_inbound(&mut self, conn: &InboundConn) -> std::io::Result<()>;
    /// Writes one whole `ControlOpen` or control message on the session
    /// (`Paired`, `Refused { errno }`, `TimedOut { errno }`, `Abort`).
    fn write_host(&mut self, conn: &HostConn, bytes: &[u8]) -> std::io::Result<()>;
    /// TCPACCEPT-END: after the request read, `true` iff V_g already reports
    /// `POLLRDHUP` or EOF (the host closed V_h). Non-blocking; reads no byte.
    fn inbound_ended(&mut self, conn: &InboundConn) -> std::io::Result<bool>;

    // --- TcpConnect (guest opens; § Per-kind total orders) ---
    /// Writes the 32-byte flow request on a connected flow V_g, once.
    fn write_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()>;
    /// On `Paired`: `FlowForwarder::attach(I, V_g, Stream)` (EVMOD-PAIR).
    /// Consumes both; `Err` hands both back unattached.
    fn attach_outbound(
        &mut self,
        child: CapturedConn,
        vsock: HostConn,
    ) -> Result<GuestPair, GuestAttachRefused>;

    // --- TcpAccept (a dial; the host opens) ---
    /// Creates C with `SO_LINGER{1,0}`, `IP_TRANSPARENT` bound to `client`
    /// (the request's peer: the guest gateway with port 0, so the kernel
    /// picks the port, ADR-0174) and the platform `SO_MARK`; starts the
    /// non-blocking connect to `destination` (`workload_addr:listen_port`).
    /// Completion is a `Connected` event; the owner bounds it by
    /// `GUEST_INBOUND_CONNECT_TIMEOUT`.
    fn start_connect_app(
        &mut self,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> std::io::Result<AppConn>;
    /// After C connected and before `Paired` is written:
    /// `FlowForwarder::attach(V_g, C, Stream)`. Consumes both; `Err` hands
    /// both back unattached.
    fn attach_inbound(
        &mut self,
        vsock: InboundConn,
        app: AppConn,
    ) -> Result<GuestPair, GuestAttachRefused>;

    // --- the guest forwarder context (`FlowForwarder`, EVMOD-PORT) ---
    /// `FlowForwarder::abort(id)` (an `Abort { flow }` for a paired flow,
    /// session loss, a slot release).
    fn abort_forward(&mut self, id: ForwardId) -> Result<AbortResult, ForwarderError>;
    /// `FlowForwarder::pair_state(id)`; read-only.
    fn pair_state(&mut self, id: ForwardId) -> Result<Option<PairState>, ForwarderError>;
    /// `FlowForwarder::inventory()`; read-only.
    fn inventory(&mut self) -> Result<ForwarderInventory, ForwarderError>;

    // --- ends of sockets ---
    /// Clears `SO_LINGER` on a completed TCP half before its close.
    fn clear_linger(&mut self, socket: &GuestPairSocket) -> std::io::Result<()>;
    /// Close with `SO_LINGER{1,0}`. Consumes the handle.
    fn abort(&mut self, socket: GuestOwnedSocket);
    /// Close. Consumes the handle.
    fn close(&mut self, socket: GuestOwnedSocket);

    // --- datagram associations and slots ---
    /// The kernel's (cookie, destination) owner of `slot`.
    fn slot_owner(
        &mut self,
        slot: SocketAddrV4,
    ) -> std::io::Result<Option<(SocketCookie, SocketAddrV4)>>;
    /// Step 2: writes the `DatagramStream` request on the association's V_g,
    /// once.
    fn write_slot_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()>;
    /// Step 5, on `Paired`: `FlowForwarder::attach(S, V_g, Datagram { udp: A
    /// })`. Consumes V_g; every `Err` hands it back unattached in
    /// [`GuestSlotAttachRefused`] (S stays resident in its slot).
    fn attach_slot(
        &mut self,
        slot: SocketAddrV4,
        vsock: HostConn,
    ) -> Result<ForwardId, GuestSlotAttachRefused>;
    /// The end-of-association order after the pair's abort or end record (or
    /// V_g's close before `Paired`): discards S's receive queue without
    /// reading payload, then disconnects S (EVMOD-DGRAM, P-72). Never fails in
    /// a way the owner can act on: an error is counted and the slot is still
    /// returned.
    fn end_association(&mut self, slot: SocketAddrV4);
    /// Returns a slot whose association ended to the pool.
    fn release_slot(&mut self, slot: SocketAddrV4);
    /// Exact per-slot counters in both directions (idle release): the pair's
    /// `moved_ab` / `moved_ba` (`pair_state`) while attached.
    fn slot_counters(&mut self, slot: SocketAddrV4) -> std::io::Result<(u64, u64)>;

    // --- audits ---
    /// The subset of `cookies` that still name a live socket (release audit).
    fn live_cookies(
        &mut self,
        cookies: &BTreeSet<SocketCookie>,
    ) -> std::io::Result<BTreeSet<SocketCookie>>;
}

// ---------------------------------------------------------------------------
// The owner's time port
// ---------------------------------------------------------------------------

/// The owner's crate-private, synchronous time port (the core `Clock` port is
/// async and the guest has no runtime).
pub(crate) trait GuestClock: Send + Sync {
    /// Monotonic now. Never goes backwards.
    fn now(&self) -> Instant;
}

/// Production [`GuestClock`]: `Instant::now()`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MonotonicGuestClock;

impl GuestClock for MonotonicGuestClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

// ---------------------------------------------------------------------------
// The owner
// ---------------------------------------------------------------------------

mod owner;
pub(crate) use owner::GuestFlowOwner;

/// The only failures the owner cannot recover from without a restart.
/// `overdrive-init` writes `overdrive-init: guest flow owner failed:
/// <Display>` to the console and takes the existing guest-exit path; the
/// host sees the VMM exit.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GuestOwnerFatal {
    /// `next_event`'s readiness wait itself failed.
    #[error("waiting for guest transport events failed: {source}")]
    EventWait {
        /// The wait's error.
        #[source]
        source: std::io::Error,
    },
    /// A listener bound once at setup failed (`ListenerFailed`).
    #[error("guest listener {listener:?} failed: {kind:?}")]
    ListenerFailed {
        /// The listener.
        listener: GuestListener,
        /// The accept error.
        kind: std::io::ErrorKind,
    },
    /// The guest forwarder context failed (`ForwarderError::Io` or
    /// `EndDecode`).
    #[error("the guest forwarder failed: {source}")]
    Forwarder {
        /// The forwarder's error.
        #[source]
        source: ForwarderError,
    },
}

// ---------------------------------------------------------------------------
// Production seam
// ---------------------------------------------------------------------------

mod kernel_io;
pub(crate) use kernel_io::KernelGuestOwnerIo;

// ---------------------------------------------------------------------------
// QPROJ-5 observation
// ---------------------------------------------------------------------------

/// The guest flow owner's observable state (QPROJ-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSideObservation {
    /// The guest owner holds a control-session connection it has not
    /// observed lost (D-UP-8): true from the dispatch that starts its connect
    /// until the dispatch that observes the loss (DG-5-F). The owner holds at
    /// most one such connection.
    pub session: bool,
    /// Host-opened (TcpAccept) flows the guest holds, by host-origin flow id.
    pub accepted: BTreeMap<FlowId, GuestAcceptPhase>,
    /// Paired TCP flows the guest holds (either opener), by flow id, each with
    /// its attached pair; per-direction state through
    /// `FlowForwarder::pair_state` (EVMOD-PORT). A flow leaves the map in the
    /// dispatch that reads its pair's end record.
    pub flows: BTreeMap<FlowId, ForwardId>,
    /// TcpConnect flows the guest opened that are not yet Paired; their
    /// intake children are unattached and observed only for an error.
    pub awaiting_paired: BTreeSet<FlowId>,
    /// Every UDP slot, by the slot socket's bound address
    /// (`workload_addr:GUEST_UDP_SLOT_BASE + n`).
    pub slots: BTreeMap<SocketAddrV4, SlotObservation>,
    /// Cumulative attaches the forwarder refused `HalfEnded`, each ended by
    /// the guest owner as a flow abort (EVMOD-ATTACH-END; QPROJ-8). Never
    /// decremented.
    pub forwarder_attach_ended: u64,
    /// Cumulative attaches the forwarder accepted (`Ok`), raised in the
    /// dispatch that made the call (QPROJ-8, `attRes`). Never decremented.
    pub forwarder_attached: u64,
    /// The last end record the guest owner consumed in a dispatch (its
    /// pair's `take_end`, EVMOD-REC): depth 1, replaced at each consumption,
    /// `None` until the first (D-11).
    pub last_consumed_end: Option<ForwardEnd>,
    /// Cumulative end records the guest owner consumed. Never decremented.
    pub consumed_ends: u64,
}

/// Phase of a host-opened (TcpAccept) flow the guest holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAcceptPhase {
    /// V_g accepted, request read and token matched; V_g unattached and
    /// observed only for its end; the connect to the application is issued or
    /// pending.
    Accepted,
    /// (V_g, C) attached and `Paired` sent in one dispatch (EVMOD-PAIR).
    Paired {
        /// The attached pair.
        pair: ForwardId,
    },
}

/// One UDP slot (QPROJ-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotObservation {
    /// The (application socket, original destination) owning the slot.
    pub owner: Option<(SocketCookie, SocketAddrV4)>,
    /// The slot's current association, if any.
    pub association: Option<FlowId>,
    /// The association's datagram pair (S, V_g) is attached (EVMOD-DGRAM).
    pub attached: Option<ForwardId>,
}
