//! Guest flow owner — the guest half of the vsock attachment replacement
//! (GH #295): the control session, the flow table and the UDP slot pool of
//! `overdrive-init`, over the guest-owner seam [`GuestOwnerIo`].
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § DG-5 ("Guest flow owner", module `guest_flow`), § *Per-kind total orders*
//! (TcpConnect, datagram association, UDP slot release, end-of-association
//! order, TcpAccept), § *Control session* (D18a), § *Flow end*
//! (HALFCLOSE-SIGNAL) and QPROJ-5 (the read-only `guest_observation`). Every
//! item below is the one those sections pin.
//!
//! RED scaffold (DISTILL, Mandate 7): the owner (`GuestFlowOwner`), the newtype
//! constructors and codecs, and the production seam `KernelGuestOwnerIo` panic
//! until DELIVER vsock new-step 5 lands them. Plain data (the event, handle and
//! label vocabulary, the observation types, `GuestOwnerFatal` with its pinned
//! `Display`, the production clock) is written for real. The simulation seam
//! [`sim`] (`#[cfg(test)]`) is a fully functional fixture.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 5).
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "RED scaffold — DELIVER vsock new-step 5 wires it into main")
)]
#![cfg_attr(
    test,
    allow(dead_code, reason = "RED scaffold — DELIVER vsock new-step 5 wires it into main")
)]
#![allow(
    clippy::redundant_pub_crate,
    reason = "DG-5 pins `pub(crate)` visibility for these binary-crate items"
)]
#![allow(
    clippy::doc_markdown,
    reason = "protocol leg names (V_g, T_g, Q_g, …) and kernel names are prose, not code"
)]

#[cfg(all(test, feature = "integration-tests"))]
mod quint_flows_half;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_itf;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_owner_flows;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_udp_slots;
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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use overdrive_core::guest_transport::wire::{FLOW_REQUEST_LEN, FlowId};
use overdrive_core::net::ListenPort;

use crate::transport::{GuestTransportToken, KernelGuestTransportOps};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// Newtypes (QPROJ-5 types, DG-5)
// ---------------------------------------------------------------------------

/// Kernel socket cookie of a guest socket; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SocketCookie(NonZeroU64);

impl SocketCookie {
    /// Validates a raw cookie: 0 is `SocketCookieError::Zero`.
    pub(crate) fn new(_raw: u64) -> Result<Self, SocketCookieError> {
        panic!("{SCAFFOLD}")
    }

    /// The raw cookie.
    pub(crate) fn get(self) -> u64 {
        let _ = self;
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form.
impl fmt::Display for SocketCookie {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; validated by [`SocketCookie::new`].
impl FromStr for SocketCookie {
    type Err = SocketCookieError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a value is not a [`SocketCookie`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SocketCookieError {
    /// The kernel never issues cookie 0.
    #[error("socket cookie is zero")]
    Zero,
    /// Not a decimal integer.
    #[error("socket cookie is not a decimal integer")]
    Unparseable,
}

/// Readiness-registration generation of one UDP slot's cells (CELL-RECREATE).
/// 0 at boot; +1 at each recreate; `u64`, never wraps in practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CellGeneration(u64);

impl CellGeneration {
    /// Validates a raw generation.
    pub(crate) fn new(_raw: u64) -> Result<Self, CellGenerationError> {
        panic!("{SCAFFOLD}")
    }

    /// The raw generation.
    pub(crate) fn get(self) -> u64 {
        let _ = self;
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form.
impl fmt::Display for CellGeneration {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; validated by [`CellGeneration::new`].
impl FromStr for CellGeneration {
    type Err = CellGenerationError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a value is not a [`CellGeneration`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CellGenerationError {
    /// Not a decimal integer.
    #[error("cell generation is not a decimal integer")]
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
/// V_g, T_g, Q_g.
#[must_use]
#[derive(Debug)]
pub(crate) struct HostConn(Handle);
/// V_g accepted on `GUEST_INBOUND_PORT`.
#[must_use]
#[derive(Debug)]
pub(crate) struct InboundConn(Handle);
/// I: the TCP intake's child for an application's captured `connect()`.
#[must_use]
#[derive(Debug)]
pub(crate) struct CapturedConn(Handle);
/// A flow V_g installed (TcpConnect: route V_g → I; TcpAccept: no route yet).
#[must_use]
#[derive(Debug)]
pub(crate) struct InstalledConn(Handle);
/// C: armed toward an installed V_g, connecting.
#[must_use]
#[derive(Debug)]
pub(crate) struct AppConn(Handle);
/// T_g and Q_g of one slot's association, installed.
#[must_use]
#[derive(Debug)]
pub(crate) struct SlotLegs(Handle);
/// Routed: terminal readiness, shutdown, linger, abort, close only.
#[must_use]
#[derive(Debug)]
pub(crate) struct GuestPairSocket(Handle);

/// Names one routed TCP pair for `drained` / `remove_pair`; no capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GuestPairKey(u64);

/// A routed guest TCP pair: the flow vsock and the outer (application) leg.
#[must_use]
pub(crate) struct GuestPair {
    /// Names the pair.
    pub(crate) key: GuestPairKey,
    /// V_g.
    pub(crate) vsock: GuestPairSocket,
    /// I (TcpConnect) or C (TcpAccept).
    pub(crate) outer: GuestPairSocket,
}

/// Any guest socket handle, for `abort` / `close`.
#[must_use]
pub(crate) enum GuestOwnedSocket {
    /// A host channel connection.
    Host(HostConn),
    /// An accepted V_g.
    Inbound(InboundConn),
    /// A captured intake child.
    Captured(CapturedConn),
    /// An installed V_g.
    Installed(InstalledConn),
    /// An application connection.
    App(AppConn),
    /// A slot's legs.
    Legs(SlotLegs),
    /// A routed pair socket.
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
    InstalledConn => Installed,
    AppConn => App,
    SlotLegs => Legs,
    GuestPairSocket => Pair,
);

impl GuestOwnedSocket {
    /// The socket this handle names.
    pub(crate) const fn id(&self) -> SocketId {
        match self {
            Self::Host(h) => h.id(),
            Self::Inbound(h) => h.id(),
            Self::Captured(h) => h.id(),
            Self::Installed(h) => h.id(),
            Self::App(h) => h.id(),
            Self::Legs(h) => h.id(),
            Self::Pair(h) => h.id(),
        }
    }
}

// ---------------------------------------------------------------------------
// Event vocabulary
// ---------------------------------------------------------------------------

/// Terminal readiness (never `POLLIN` / `POLLOUT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestHangup {
    /// `POLLRDHUP`: the peer ended its writes.
    ReadHangup,
    /// `POLLHUP`.
    Hangup,
    /// `POLLERR`, with the socket error.
    Error(std::io::ErrorKind),
}

/// Outcome of the end-of-association order for one slot (CELL-RECREATE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotEnd {
    /// Both cells recreated; the slot's new generation.
    Recreated(CellGeneration),
    /// Creating a new cell failed: the slot is quarantined (no association,
    /// detached from its owner, not in the pool; steps 4–5 ran).
    Quarantined,
}

/// The host vsock channels the guest opens (CID 2), with their socket types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HostChannel {
    /// `GUEST_FLOW_STREAM_PORT` (1240), `SOCK_STREAM`.
    FlowStream,
    /// `GUEST_FLOW_DATAGRAM_PORT` (1241), `SOCK_SEQPACKET`.
    FlowSeqpacket,
    /// `GUEST_FLOW_CONTROL_PORT` (1243), `SOCK_STREAM`.
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

/// Direction of a guest TCP pair for `drained`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GuestFlowDirection {
    /// Toward the outer (application) leg.
    TowardOuter,
    /// Toward the host (the flow vsock).
    TowardHost,
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
    /// The TCP intake accepted an application's captured `connect()`; the
    /// kernel already installed it with route I → a parking cell (or, with no
    /// free cell, did not: `parked = false`, and the owner resets it).
    OutboundCaptured {
        /// I.
        child: CapturedConn,
        /// The application's original destination.
        original_destination: SocketAddrV4,
        /// The application socket's cookie.
        cookie: SocketCookie,
        /// I is installed toward a parking cell.
        parked: bool,
    },
    /// Terminal readiness on a watched socket: the control session's
    /// `HostConn`, a `CapturedConn` before `Paired`, and every
    /// `GuestPairSocket` are watched from issue (level-triggered: reported
    /// once per socket per kind until the socket ends).
    Hangup {
        /// The socket.
        socket: SocketId,
        /// The readiness kind.
        hangup: GuestHangup,
    },
    /// A slot's framing cell became readable, as registered at `generation`.
    SlotReady {
        /// The slot socket's bound address.
        slot: SocketAddrV4,
        /// The registration's cell generation.
        generation: CellGeneration,
    },
    /// A `sock_release` report (ring buffer).
    SocketReleased {
        /// The released socket.
        cookie: SocketCookie,
    },
    /// The listener map changed (`fexit` wake record).
    ListenerMapWake,
    /// A listener's accept failed with an error other than a per-connection
    /// one (`ECONNABORTED`, and `EMFILE` / `ENFILE`, which the seam retries).
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

/// Synchronous: `overdrive-init` has no async runtime (std + nix).
///
/// Contract (every method):
///
/// - **Preconditions are types.** Handles are move-only and exist from the
///   call or event that issued them to the call that consumed them; the
///   per-kind orders of § *Per-kind total orders* are the only orders the
///   types admit (TcpConnect: captured → rearm → connect host →
///   `install_outbound` → `write_request` → `route_outbound`; TcpAccept:
///   inbound → read → `inbound_ended` → `install_inbound` → `start_connect_app`
///   → `route_inbound`; datagram: two host connects → `install_slot_legs` →
///   `write_slot_requests` → `route_slot` → `end_association`). An operation
///   on a `HostConn` / `AppConn` before its `Connected` event fails with the
///   kernel's `NotConnected`, which the owner never issues. A handle from
///   another seam instance is unreachable (`unreachable!`).
/// - **Consuming calls are all-or-nothing.** A call that consumes handles and
///   returns `Err` has aborted every consumed socket and removed every entry
///   it wrote.
/// - **Writes** (`write_host`, `write_request`, `write_slot_requests`) write the
///   whole record or fail; a failed write may have sent a prefix the peer
///   reads as a truncated record — which every peer treats as a read failure
///   of that connection — so the owner ends that connection (or, on the
///   session, takes the session-loss path).
/// - **Events.** Each `GuestIoEvent` is delivered once; `Connected` / `Read`
///   name a handle the owner still holds or one it has since consumed (the
///   owner discards the latter). A `Hangup` is level-triggered and reported
///   once per socket per kind.
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
    /// Writes one whole `ControlOpen` or control message on the session.
    fn write_host(&mut self, conn: &HostConn, bytes: &[u8]) -> std::io::Result<()>;
    /// TCPACCEPT-END: after the request read, `true` iff V_g already reports
    /// `POLLRDHUP` or EOF (the host closed V_h). Non-blocking; reads no byte.
    fn inbound_ended(&mut self, conn: &InboundConn) -> std::io::Result<bool>;

    // --- TcpConnect (guest opens; § Per-kind total orders steps 2, 7) ---
    /// Re-arms I (`SO_RCVLOWAT`) after its `OutboundCaptured` event.
    fn rearm_captured(&mut self, child: &CapturedConn) -> std::io::Result<()>;
    /// Installs the connected V_g with route V_g → I.
    fn install_outbound(
        &mut self,
        vsock: HostConn,
        child: &CapturedConn,
    ) -> std::io::Result<InstalledConn>;
    /// Writes the 32-byte flow request on an installed V_g, once.
    fn write_request(
        &mut self,
        vsock: &InstalledConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()>;
    /// On `Paired`: routes I's parking cell → V_g and re-arms the cell.
    fn route_outbound(
        &mut self,
        vsock: InstalledConn,
        child: CapturedConn,
    ) -> std::io::Result<GuestPair>;

    // --- TcpAccept (host opens; steps 3–5) ---
    /// Installs V_g after a matching request and `inbound_ended == false`
    /// (no route yet).
    fn install_inbound(&mut self, vsock: InboundConn) -> std::io::Result<InstalledConn>;
    /// Creates C with `SO_LINGER{1,0}`, `IP_TRANSPARENT` bound to `client`
    /// and the platform `SO_MARK`; arms C with route C → V_g; starts the
    /// non-blocking connect to `destination` (`workload_addr:listen_port`).
    /// Completion is a `Connected` event; refused → `ApplicationNotListening`.
    fn start_connect_app(
        &mut self,
        vsock: &InstalledConn,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> std::io::Result<AppConn>;
    /// Routes V_g → C (C installed at establishment by guest `sock_ops`).
    fn route_inbound(&mut self, vsock: InstalledConn, app: AppConn) -> std::io::Result<GuestPair>;

    // --- pairs ---
    /// Whether the pair's `direction` has drained.
    fn drained(
        &mut self,
        pair: GuestPairKey,
        direction: GuestFlowDirection,
    ) -> std::io::Result<bool>;
    /// Convergent: an absent pair counts as removed.
    fn remove_pair(&mut self, pair: GuestPairKey) -> std::io::Result<()>;
    /// `shutdown(SHUT_WR)` on a pair socket.
    fn shutdown_write(&mut self, socket: &GuestPairSocket) -> std::io::Result<()>;
    /// Clears `SO_LINGER{1,0}` before a clean close.
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
    /// Step 2: installs connected T_g (route T_g → reassembly cell → U) and Q_g.
    fn install_slot_legs(
        &mut self,
        slot: SocketAddrV4,
        stream: HostConn,
        seqpacket: HostConn,
    ) -> std::io::Result<SlotLegs>;
    /// Step 2: writes `DatagramStream` on T_g and `DatagramSeqpacket` on Q_g, once.
    fn write_slot_requests(
        &mut self,
        legs: &SlotLegs,
        stream: &[u8; FLOW_REQUEST_LEN],
        seqpacket: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()>;
    /// Step 5, on `Paired`: routes the framing cell → Q_g; re-arms.
    fn route_slot(&mut self, legs: &SlotLegs) -> std::io::Result<()>;
    /// The whole end-of-association order (CELL-RECREATE) as one call,
    /// consuming the legs if any. Never fails: a cell-creation failure is
    /// `SlotEnd::Quarantined`, and the other steps still run.
    fn end_association(&mut self, slot: SocketAddrV4, legs: Option<SlotLegs>) -> SlotEnd;
    /// Returns a slot whose association ended (`Recreated`) to the pool.
    fn release_slot(&mut self, slot: SocketAddrV4);
    /// Audit: retries a quarantined slot's cell creation; `Ok` returns it to
    /// the pool.
    fn retry_quarantined(&mut self, slot: SocketAddrV4) -> std::io::Result<CellGeneration>;
    /// Exact per-slot counters in both directions (idle release).
    fn slot_counters(&mut self, slot: SocketAddrV4) -> std::io::Result<(u64, u64)>;

    // --- audits ---
    /// The subset of `cookies` that still name a live socket (release audit).
    fn live_cookies(
        &mut self,
        cookies: &BTreeSet<SocketCookie>,
    ) -> std::io::Result<BTreeSet<SocketCookie>>;
    /// The listener map: application ports listening on `0.0.0.0` / `workload_addr`.
    fn listener_map(&mut self) -> std::io::Result<BTreeSet<ListenPort>>;
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

/// The guest flow owner: control session, flow table, UDP slot pool.
pub(crate) struct GuestFlowOwner {
    _scaffold: (),
}

impl GuestFlowOwner {
    /// Takes the setup's results through `io`; opens nothing yet.
    pub(crate) fn new(
        _token: GuestTransportToken,
        _io: Box<dyn GuestOwnerIo>,
        _clock: Arc<dyn GuestClock>,
    ) -> Self {
        panic!("{SCAFFOLD}")
    }

    /// Driving port: handles exactly one input — the next event, or the
    /// earliest due deadline (request, pairing, drain, reconnect backoff,
    /// slot idle, audit tick) computed from `clock.now()` — then returns.
    /// The first dispatch with no session starts the control-session
    /// connect. Per-flow failures end that flow and return `Ok`. Blocking.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "RED scaffold body panics before mutating; DG-5 pins `&mut self`"
    )]
    pub(crate) fn dispatch_next(&mut self) -> Result<(), GuestOwnerFatal> {
        let _ = self;
        panic!("{SCAFFOLD}")
    }

    /// `dispatch_next` until it returns `Err`; runs on its own thread.
    pub(crate) fn run(self) -> GuestOwnerFatal {
        let _ = self;
        panic!("{SCAFFOLD}")
    }

    /// QPROJ-5: an owned copy of the owner's session flag, flow tables and
    /// slot table.
    ///
    /// - **Preconditions:** none. Infallible; callable on a constructed owner
    ///   at any time, before and after the control session.
    /// - **Postconditions:** the call accepts, routes, releases and closes
    ///   nothing and changes no state; no lock is held on return.
    /// - **Consistency point:** between two owner steps (one dispatch).
    /// - **Edge cases:** before the session opens, `session = false` and every
    ///   table is empty except `slots`, which lists every slot with
    ///   `owner = None`. A slot with an `owner` and no `association` has not
    ///   started pairing; `routed` implies `association` is `Some`. After the
    ///   session ends, `session = false` and the tables hold only what the
    ///   owner has not yet ended.
    /// - **Observable invariants:** a `FlowId` in `awaiting_paired` is in
    ///   neither `accepted` nor `flows`; a repeated call with no intervening
    ///   owner step returns an equal value.
    pub(crate) fn guest_observation(&self) -> GuestSideObservation {
        let _ = self;
        panic!("{SCAFFOLD}")
    }
}

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
}

// ---------------------------------------------------------------------------
// Production seam
// ---------------------------------------------------------------------------

/// Production [`GuestOwnerIo`] over the guest kernel, built from the sockets
/// and programs `configure_guest_transport` set up.
pub(crate) struct KernelGuestOwnerIo {
    _scaffold: (),
}

impl KernelGuestOwnerIo {
    /// Takes ownership of the setup's sockets and programs.
    pub(crate) fn new(_ops: KernelGuestTransportOps) -> Self {
        panic!("{SCAFFOLD}")
    }
}

impl GuestOwnerIo for KernelGuestOwnerIo {
    fn next_event(&mut self, _deadline: Option<Instant>) -> std::io::Result<Option<GuestIoEvent>> {
        panic!("{SCAFFOLD}")
    }
    fn start_connect_host(&mut self, _channel: HostChannel) -> std::io::Result<HostConn> {
        panic!("{SCAFFOLD}")
    }
    fn start_read_host(&mut self, _conn: &HostConn, _record: GuestRecord) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn start_read_inbound(&mut self, _conn: &InboundConn) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn write_host(&mut self, _conn: &HostConn, _bytes: &[u8]) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn inbound_ended(&mut self, _conn: &InboundConn) -> std::io::Result<bool> {
        panic!("{SCAFFOLD}")
    }
    fn rearm_captured(&mut self, _child: &CapturedConn) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn install_outbound(
        &mut self,
        _vsock: HostConn,
        _child: &CapturedConn,
    ) -> std::io::Result<InstalledConn> {
        panic!("{SCAFFOLD}")
    }
    fn write_request(
        &mut self,
        _vsock: &InstalledConn,
        _request: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn route_outbound(
        &mut self,
        _vsock: InstalledConn,
        _child: CapturedConn,
    ) -> std::io::Result<GuestPair> {
        panic!("{SCAFFOLD}")
    }
    fn install_inbound(&mut self, _vsock: InboundConn) -> std::io::Result<InstalledConn> {
        panic!("{SCAFFOLD}")
    }
    fn start_connect_app(
        &mut self,
        _vsock: &InstalledConn,
        _client: SocketAddrV4,
        _destination: SocketAddrV4,
    ) -> std::io::Result<AppConn> {
        panic!("{SCAFFOLD}")
    }
    fn route_inbound(
        &mut self,
        _vsock: InstalledConn,
        _app: AppConn,
    ) -> std::io::Result<GuestPair> {
        panic!("{SCAFFOLD}")
    }
    fn drained(
        &mut self,
        _pair: GuestPairKey,
        _direction: GuestFlowDirection,
    ) -> std::io::Result<bool> {
        panic!("{SCAFFOLD}")
    }
    fn remove_pair(&mut self, _pair: GuestPairKey) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn shutdown_write(&mut self, _socket: &GuestPairSocket) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn clear_linger(&mut self, _socket: &GuestPairSocket) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn abort(&mut self, _socket: GuestOwnedSocket) {
        panic!("{SCAFFOLD}")
    }
    fn close(&mut self, _socket: GuestOwnedSocket) {
        panic!("{SCAFFOLD}")
    }
    fn slot_owner(
        &mut self,
        _slot: SocketAddrV4,
    ) -> std::io::Result<Option<(SocketCookie, SocketAddrV4)>> {
        panic!("{SCAFFOLD}")
    }
    fn install_slot_legs(
        &mut self,
        _slot: SocketAddrV4,
        _stream: HostConn,
        _seqpacket: HostConn,
    ) -> std::io::Result<SlotLegs> {
        panic!("{SCAFFOLD}")
    }
    fn write_slot_requests(
        &mut self,
        _legs: &SlotLegs,
        _stream: &[u8; FLOW_REQUEST_LEN],
        _seqpacket: &[u8; FLOW_REQUEST_LEN],
    ) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn route_slot(&mut self, _legs: &SlotLegs) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
    fn end_association(&mut self, _slot: SocketAddrV4, _legs: Option<SlotLegs>) -> SlotEnd {
        panic!("{SCAFFOLD}")
    }
    fn release_slot(&mut self, _slot: SocketAddrV4) {
        panic!("{SCAFFOLD}")
    }
    fn retry_quarantined(&mut self, _slot: SocketAddrV4) -> std::io::Result<CellGeneration> {
        panic!("{SCAFFOLD}")
    }
    fn slot_counters(&mut self, _slot: SocketAddrV4) -> std::io::Result<(u64, u64)> {
        panic!("{SCAFFOLD}")
    }
    fn live_cookies(
        &mut self,
        _cookies: &BTreeSet<SocketCookie>,
    ) -> std::io::Result<BTreeSet<SocketCookie>> {
        panic!("{SCAFFOLD}")
    }
    fn listener_map(&mut self) -> std::io::Result<BTreeSet<ListenPort>> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// QPROJ-5 observation
// ---------------------------------------------------------------------------

/// The guest flow owner's observable state (QPROJ-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSideObservation {
    /// The guest's control session is live.
    pub session: bool,
    /// TcpAccept flows the guest holds, by host-origin flow id.
    pub accepted: BTreeMap<FlowId, GuestAcceptPhase>,
    /// Paired TCP flows the guest holds (either opener), by flow id.
    pub flows: BTreeMap<FlowId, FlowEndObservation>,
    /// TcpConnect flows the guest opened that are not yet Paired: the
    /// application leg's EOF recorded before `Paired`.
    pub awaiting_paired: BTreeMap<FlowId, bool /* outer_eof */>,
    /// Every UDP slot, by the slot socket's bound address
    /// (`workload_addr:GUEST_UDP_SLOT_BASE + n`).
    pub slots: BTreeMap<SocketAddrV4, SlotObservation>,
}

/// Phase of a TcpAccept flow the guest holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAcceptPhase {
    /// V_g accepted and installed; C not yet routed.
    Accepted,
    /// V_g routed to C; `Paired` sent.
    Routed,
}

/// One UDP slot (QPROJ-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotObservation {
    /// The (application socket, original destination) owning the slot.
    pub owner: Option<(SocketCookie, SocketAddrV4)>,
    /// The slot's current association, if any.
    pub association: Option<FlowId>,
    /// The framing cell is routed to the association's Q_g.
    pub routed: bool,
    /// The slot's current cell generation (CELL-RECREATE).
    pub generation: CellGeneration,
}

/// HALFCLOSE-SIGNAL state of one direction pair of a paired TCP flow, at this
/// owner (mirrors QPROJ-3's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowEndObservation {
    /// The outer leg read EOF.
    pub outer_eof: bool,
    /// This side's announcement of its direction.
    pub announce: HalfCloseAnnounce,
    /// This side acknowledged the peer's `HalfClose`.
    pub peer_announced: bool,
    /// This side shut its outer leg down for writing.
    pub outer_shut: bool,
}

/// This side's announcement of its direction (mirrors QPROJ-3's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfCloseAnnounce {
    /// Not announced.
    None,
    /// `HalfClose` sent, `HalfCloseAck` not yet received (HalfClosing).
    Pending,
    /// Acknowledged; the flow vsock is shut down for writing.
    Done,
}
