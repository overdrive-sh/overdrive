//! Simulation seam for the guest flow owner (DG-5, QPROJ-7): [`SimGuestOwnerIo`]
//! and [`SimGuestClock`]. `#[cfg(test)]` only; `overdrive-init` takes no
//! `overdrive-sim` dev-dependency.
//!
//! A fixture, not a scaffold: fully functional, and it enforces the seam
//! contract the real adapter has (`GuestOwnerIo` rustdoc):
//!
//! - a handle of another seam instance is `unreachable!`;
//! - an operation on a `HostConn` / `AppConn` before its `Connected` event was
//!   delivered fails `NotConnected`;
//! - consuming calls are all-or-nothing (an `Err` aborts every consumed socket);
//! - writes are whole records (a control-session write is a 24-byte
//!   `ControlOpen` or an 8-byte control message, a request write is 32 bytes);
//! - every event is delivered once; a `Hangup` is level-triggered and reported
//!   once per socket per kind while the socket is watched (the session
//!   `HostConn`, a `CapturedConn`, every `GuestPairSocket`), and never after the
//!   socket ends.
//!
//! **Time and blocking.** The owner and the sim read the same
//! [`SimGuestClock`]; only the driver advances it. `next_event` returns the
//! next *released* event; `Ok(None)` at once when the deadline it is given is
//! at or before the clock's now; a call with no event released and no
//! deadline passed is a driver defect (DG-5): it panics rather than block a
//! single-threaded driver, with the typed payload [`SimWouldBlock`] (raised by
//! `std::panic::panic_any`, distinguishable from every other panic). It never
//! answers that case with an `Err`: an `Err` from `next_event` is the owner's
//! fatal wait failure (`GuestOwnerFatal::EventWait`), which only
//! `fail_event_wait` scripts. `test_support::settle` catches exactly that
//! payload as the owner's blocking wait (the quiescence point).
//!
//! **Held until released.** Every effect the owner starts (a host or
//! application connect, a read) and every guest-kernel fact (a V_g in the
//! accept queue, a readiness event, a `sock_release` report) is held until the
//! driver releases it through the stepping API below. A driver injection that
//! *is* the release (the host writes a control message and it is read, the
//! host ends a vsock, the application half-closes) makes the event deliverable
//! at once.
//!
//! **Read surface.** [`SimGuestOwnerIo::guest_socket_observation`] is the pinned
//! QPROJ-7 arbiter surface (Tier-3 pairings R5-31, R5-28, R5-14, R5-27, R5-10,
//! R5-45). Every other read below (`effects`, `slot_state`, `pending_connects`,
//! …) is test support for acceptance oracles and must not feed a conformance
//! `State` projection.
//!
//! The sim decodes the records the owner writes with the core wire codec
//! (`ControlMessage::decode`, `FlowRequest::decode`) and encodes the host's
//! records with it; those paths panic until DELIVER vsock new-step 2 lands the
//! codec.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::significant_drop_tightening,
    clippy::struct_excessive_bools,
    clippy::option_option,
    clippy::option_if_let_else,
    clippy::collapsible_if,
    clippy::map_unwrap_or,
    clippy::manual_let_else,
    clippy::single_match_else,
    reason = "test fixture: a seam-contract violation is a driver or owner defect and panics"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, FLOW_REQUEST_LEN, FlowId, FlowRequest,
};
use overdrive_core::guest_transport::{GUEST_UDP_SLOT_BASE, GUEST_UDP_SLOTS};
use overdrive_core::net::ListenPort;

use super::{
    AppConn, CapturedConn, CellGeneration, GuestClock, GuestFlowDirection, GuestHangup,
    GuestIoEvent, GuestListener, GuestOwnedSocket, GuestOwnerIo, GuestPair, GuestPairKey,
    GuestPairSocket, GuestRecord, Handle, HostChannel, HostConn, InboundConn, InstalledConn,
    SeamInstance, SlotEnd, SlotLegs, SocketCookie, SocketId,
};

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// The owner's clock under test: advanced only by the driver. `SimGuestOwnerIo`
/// reads the same instance.
#[derive(Debug)]
pub(crate) struct SimGuestClock {
    now: Mutex<Instant>,
}

impl SimGuestClock {
    /// A clock starting at the current host instant (any fixed origin works:
    /// only differences are observed).
    pub(crate) fn new() -> Self {
        Self { now: Mutex::new(Instant::now()) }
    }

    /// Moves the clock forward by `by`.
    pub(crate) fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }
}

impl GuestClock for SimGuestClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// ---------------------------------------------------------------------------
// Test-support vocabulary
// ---------------------------------------------------------------------------

/// The first terminal event a far end observes (no production type is pinned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FarEndEvent {
    /// A FIN: the far end reads end of stream.
    Eof,
    /// A reset (linger close, abort).
    Reset,
}

/// The QPROJ-7 read surface of the guest-owner seam sim (field names and
/// meanings are the contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimGuestSocketObservation {
    /// V_g in the guest accept queue.
    pub(crate) queued_inbound: BTreeSet<FlowId>,
    /// V_g torn down there (K-A4).
    pub(crate) reset_in_queue: BTreeSet<FlowId>,
    /// Control messages the guest wrote and the host has not consumed.
    pub(crate) outbox: Vec<ControlMessage>,
    /// The control session has ended (and no new one has connected since).
    pub(crate) session_ended: bool,
    /// Undispatched readiness events: slot, registration generation.
    pub(crate) readiness: BTreeSet<(SocketAddrV4, CellGeneration)>,
    /// Frames parked in each slot's framing cell, tagged by origin.
    pub(crate) parked: BTreeMap<SocketAddrV4, BTreeSet<(SocketCookie, SocketAddrV4)>>,
    /// Queued `sock_release` reports.
    pub(crate) release_reports: BTreeSet<SocketCookie>,
    /// Datagram legs the guest closed and the host has not observed.
    pub(crate) legs_closed_unobserved: BTreeSet<FlowId>,
    /// Ends of the guest's flow vsocks the host has not observed.
    pub(crate) vsock_ends_in_flight: BTreeSet<FlowId>,
    /// The guest's flow vsocks that have ended (shutdown, abort, close).
    pub(crate) vsock_ended: BTreeSet<FlowId>,
    /// Each guest far end's (application leg's) first terminal event.
    pub(crate) far_end: BTreeMap<FlowId, FarEndEvent>,
}

/// The panic payload of a `next_event` call the sim cannot answer (DG-5:
/// nothing released and no deadline passed, so a single-threaded driver would
/// block). Raised with `std::panic::panic_any`; test support for
/// `test_support::settle`, which stops at it and re-raises every other panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimWouldBlock {
    /// The deadline the owner passed to the blocking wait (its earliest armed
    /// deadline at that point; `None`: it waited for an event only).
    pub(crate) deadline: Option<Instant>,
}

/// What one `next_event` call returned (test support for `settle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimWaitOutcome {
    /// It returned a released event.
    Event,
    /// It returned `None`: its deadline had passed.
    DeadlinePassed,
    /// The scripted wait failure (`fail_event_wait`).
    Failed,
    /// Nothing released and no deadline passed: the sim panicked with
    /// [`SimWouldBlock`] (the owner reached a blocking wait).
    WouldBlock,
}

/// One `next_event` call of the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimWait {
    /// The deadline the owner passed (its earliest armed deadline).
    pub(crate) deadline: Option<Instant>,
    /// What the call returned.
    pub(crate) outcome: SimWaitOutcome,
}

/// A seam operation that can be scripted to fail once with an `io::ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SimGuestOp {
    /// `start_connect_host` (synchronous failure).
    StartConnectHost,
    /// `start_read_host`.
    StartReadHost,
    /// `start_read_inbound`.
    StartReadInbound,
    /// `write_host`.
    WriteHost,
    /// `inbound_ended`.
    InboundEnded,
    /// `rearm_captured`.
    RearmCaptured,
    /// `install_outbound`.
    InstallOutbound,
    /// `write_request`.
    WriteRequest,
    /// `route_outbound`.
    RouteOutbound,
    /// `install_inbound`.
    InstallInbound,
    /// `start_connect_app` (synchronous failure).
    StartConnectApp,
    /// `route_inbound`.
    RouteInbound,
    /// `drained`.
    Drained,
    /// `remove_pair`.
    RemovePair,
    /// `shutdown_write`.
    ShutdownWrite,
    /// `clear_linger`.
    ClearLinger,
    /// `slot_owner`.
    SlotOwner,
    /// `install_slot_legs`.
    InstallSlotLegs,
    /// `write_slot_requests`.
    WriteSlotRequests,
    /// `route_slot`.
    RouteSlot,
    /// `retry_quarantined`.
    RetryQuarantined,
    /// `slot_counters`.
    SlotCounters,
    /// `live_cookies`.
    LiveCookies,
    /// `listener_map`.
    ListenerMap,
}

/// What a pending connect targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimConnectTarget {
    /// A host channel (CID 2).
    Host(HostChannel),
    /// The guest application: C bound to `client`, connecting to `destination`.
    App {
        /// The transparent client address C is bound to.
        client: SocketAddrV4,
        /// `workload_addr:listen_port`.
        destination: SocketAddrV4,
        /// The flow of the installed V_g C is armed toward.
        flow: Option<FlowId>,
    },
}

/// How the host ends the control session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimSessionEnd {
    /// The host closed it (EOF).
    Eof,
    /// It was reset (host owner died, connection reset).
    Reset,
}

/// Which application connection an application-side stimulus targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimAppTarget {
    /// The captured connection of the application socket with this cookie.
    Captured(SocketCookie),
    /// The outer (application) leg of this flow.
    Flow(FlowId),
}

/// What the application does to its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimAppEnd {
    /// Writes then `shutdown(SHUT_WR)`: the owner's leg reads EOF (`POLLRDHUP`).
    HalfClose,
    /// Errors / resets: the owner's leg reports `POLLERR` (`ECONNRESET`).
    Reset,
}

/// Test-support snapshot of one slot in the sim's guest kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimSlotState {
    /// The kernel slot map's owner.
    pub(crate) owner: Option<(SocketCookie, SocketAddrV4)>,
    /// Free: claimable by a new (socket, destination).
    pub(crate) in_pool: bool,
    /// Quarantined by a cell-creation failure.
    pub(crate) quarantined: bool,
    /// The cells' current generation.
    pub(crate) generation: CellGeneration,
    /// Frames parked in the framing cell (count, not deduplicated).
    pub(crate) parked_frames: usize,
    /// The framing cell is routed to Q_g.
    pub(crate) routed: bool,
    /// The association's flow, once its requests were written.
    pub(crate) flow: Option<FlowId>,
    /// T_g / Q_g are installed.
    pub(crate) legs_installed: bool,
    /// Frames forwarded to the host over the slot's lifetime.
    pub(crate) forwarded_frames: u64,
}

/// Every seam call the owner made, in order (test-support oracle for ordering
/// obligations; not a projection source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SimGuestEffect {
    /// `start_connect_host`.
    ConnectHost { socket: SocketId, channel: HostChannel },
    /// `start_read_host`.
    ReadHost { socket: SocketId, record: GuestRecord },
    /// `start_read_inbound`.
    ReadInbound { socket: SocketId },
    /// `write_host` of a `ControlOpen`.
    WriteControlOpen { socket: SocketId },
    /// `write_host` of a control message.
    WriteControl { socket: SocketId, message: ControlMessage },
    /// `inbound_ended`.
    InboundEnded { socket: SocketId, ended: bool },
    /// `rearm_captured`.
    RearmCaptured { socket: SocketId },
    /// `install_outbound`.
    InstallOutbound { vsock: SocketId, child: SocketId },
    /// `write_request`.
    WriteRequest { socket: SocketId, request: FlowRequest },
    /// `route_outbound`.
    RouteOutbound { pair: GuestPairKey, flow: Option<FlowId> },
    /// `install_inbound`.
    InstallInbound { socket: SocketId },
    /// `start_connect_app`.
    StartConnectApp { socket: SocketId, client: SocketAddrV4, destination: SocketAddrV4 },
    /// `route_inbound`.
    RouteInbound { pair: GuestPairKey, flow: Option<FlowId> },
    /// `drained`.
    Drained { pair: GuestPairKey, direction: GuestFlowDirection },
    /// `remove_pair`.
    RemovePair { pair: GuestPairKey },
    /// `shutdown_write`.
    ShutdownWrite { socket: SocketId, flow: Option<FlowId>, vsock: bool },
    /// `clear_linger`.
    ClearLinger { socket: SocketId },
    /// `abort`.
    Abort { socket: SocketId, flow: Option<FlowId> },
    /// `close`.
    Close { socket: SocketId, flow: Option<FlowId> },
    /// `slot_owner`.
    SlotOwner { slot: SocketAddrV4 },
    /// `install_slot_legs`.
    InstallSlotLegs { slot: SocketAddrV4 },
    /// `write_slot_requests`.
    WriteSlotRequests { slot: SocketAddrV4, flow: FlowId },
    /// `route_slot`.
    RouteSlot { slot: SocketAddrV4 },
    /// `end_association`.
    EndAssociation { slot: SocketAddrV4, end: SlotEnd },
    /// `release_slot`.
    ReleaseSlot { slot: SocketAddrV4 },
    /// `retry_quarantined`.
    RetryQuarantined { slot: SocketAddrV4 },
    /// `slot_counters`.
    SlotCounters { slot: SocketAddrV4 },
    /// `live_cookies`.
    LiveCookies,
    /// `listener_map`.
    ListenerMap,
}

/// A test-support cookie constructor (the production `SocketCookie::new` is a
/// DELIVER step).
pub(crate) fn sim_cookie(raw: u64) -> SocketCookie {
    SocketCookie(NonZeroU64::new(raw).expect("sim cookies are non-zero"))
}

/// A test-support generation constructor.
pub(crate) const fn sim_generation(raw: u64) -> CellGeneration {
    CellGeneration(raw)
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Host,
    Inbound,
    Captured,
    Installed,
    App,
    /// T_g of a slot's legs (the `SlotLegs` handle names it).
    LegStream,
    /// Q_g of a slot's legs (no handle of its own).
    LegSeqpacket,
    PairVsock,
    PairOuter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connect {
    None,
    InFlight,
    /// The completion is deliverable, not yet delivered.
    Released,
    Delivered(Result<(), ErrorKind>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HangupKey {
    ReadHangup,
    Hangup,
    Error,
}

const fn hangup_key(h: GuestHangup) -> HangupKey {
    match h {
        GuestHangup::ReadHangup => HangupKey::ReadHangup,
        GuestHangup::Hangup => HangupKey::Hangup,
        GuestHangup::Error(_) => HangupKey::Error,
    }
}

#[derive(Debug)]
struct Sock {
    stage: Stage,
    channel: Option<HostChannel>,
    connect: Connect,
    app_target: Option<(SocketAddrV4, SocketAddrV4)>,
    read_pending: Option<GuestRecord>,
    flow: Option<FlowId>,
    cookie: Option<SocketCookie>,
    /// The socket is app-facing (I or C): carries `SO_LINGER{1,0}` from creation.
    outer: bool,
    linger: bool,
    /// The host ended this vsock (V_h closed, host owner died).
    peer_ended: bool,
    /// Terminal conditions present on the socket (level).
    conditions: BTreeMap<HangupKey, GuestHangup>,
    reported: BTreeSet<HangupKey>,
    /// For a V_g installed toward I: the captured child.
    routes_to: Option<SocketId>,
    slot: Option<SocketAddrV4>,
    partner: Option<SocketId>,
    pair: Option<GuestPairKey>,
    closed: bool,
}

impl Sock {
    fn new(stage: Stage) -> Self {
        Self {
            stage,
            channel: None,
            connect: Connect::None,
            app_target: None,
            read_pending: None,
            flow: None,
            cookie: None,
            outer: false,
            linger: false,
            peer_ended: false,
            conditions: BTreeMap::new(),
            reported: BTreeSet::new(),
            routes_to: None,
            slot: None,
            partner: None,
            pair: None,
            closed: false,
        }
    }

    const fn connected(&self) -> bool {
        matches!(self.connect, Connect::Delivered(Ok(())))
    }

    const fn watched(&self) -> bool {
        matches!(self.stage, Stage::Captured | Stage::PairVsock | Stage::PairOuter)
    }

    /// A guest flow vsock (V_g in any stage), not the session and not a leg.
    fn is_flow_vsock(&self) -> bool {
        match self.stage {
            Stage::Host => self.channel == Some(HostChannel::FlowStream) && self.slot.is_none(),
            Stage::Inbound | Stage::Installed | Stage::PairVsock => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct InboundRec {
    request: FlowRequest,
    socket: Option<SocketId>,
}

#[derive(Debug)]
struct SlotRec {
    owner: Option<(SocketCookie, SocketAddrV4)>,
    quarantined: bool,
    generation: CellGeneration,
    parked: BTreeSet<(SocketCookie, SocketAddrV4)>,
    parked_frames: usize,
    routed: bool,
    flow: Option<FlowId>,
    legs: Option<(SocketId, SocketId)>,
    to_host: u64,
    to_guest: u64,
    forwarded: u64,
    fail_next_cell_creation: bool,
}

#[derive(Debug)]
struct PairRec {
    vsock: SocketId,
    outer: SocketId,
}

#[derive(Debug)]
struct SimState {
    seam: SeamInstance,
    next_socket: u64,
    next_pair: u64,
    sockets: BTreeMap<SocketId, Sock>,
    deliverable: VecDeque<GuestIoEvent>,
    waits: Vec<SimWait>,
    effects: Vec<SimGuestEffect>,
    failures: BTreeMap<SimGuestOp, VecDeque<ErrorKind>>,
    event_wait_failure: Option<ErrorKind>,
    drained_default: bool,
    drained: BTreeMap<(FlowId, GuestFlowDirection), bool>,
    session: Option<SocketId>,
    session_ended: bool,
    /// The host has ended the session, and the guest has not observed it yet
    /// (`host_end_session_deferred`): its buffered messages stay readable,
    /// the guest's writes are lost, and the end is released by
    /// `release_session_end`.
    host_closed: Option<SimSessionEnd>,
    host_messages: VecDeque<ControlMessage>,
    control_opens: Vec<[u8; CONTROL_OPEN_LEN]>,
    outbox: Vec<ControlMessage>,
    inbound: BTreeMap<FlowId, InboundRec>,
    reset_in_queue: BTreeSet<FlowId>,
    pairs: BTreeMap<GuestPairKey, PairRec>,
    far_end: BTreeMap<FlowId, FarEndEvent>,
    vsock_ends_in_flight: BTreeSet<FlowId>,
    vsock_ended: BTreeSet<FlowId>,
    legs_closed_unobserved: BTreeSet<FlowId>,
    slots: BTreeMap<SocketAddrV4, SlotRec>,
    readiness: BTreeSet<(SocketAddrV4, CellGeneration)>,
    release_reports: BTreeSet<SocketCookie>,
    live_cookies: BTreeSet<SocketCookie>,
    listener_map: BTreeSet<ListenPort>,
    exhausted_sends: u64,
}

fn lock(state: &Mutex<SimState>) -> MutexGuard<'_, SimState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SimState {
    fn handle(&mut self) -> Handle {
        let id = SocketId(self.next_socket);
        self.next_socket += 1;
        Handle { seam: self.seam, id }
    }

    fn check(&self, handle: &Handle) -> SocketId {
        if handle.seam != self.seam {
            unreachable!("guest-owner seam: a handle of another seam instance was passed");
        }
        match self.sockets.get(&handle.id) {
            Some(sock) if !sock.closed => handle.id,
            _ => unreachable!("guest-owner seam: the handle names no open socket of this seam"),
        }
    }

    fn sock(&mut self, id: SocketId) -> &mut Sock {
        self.sockets.get_mut(&id).unwrap_or_else(|| unreachable!("socket {id:?} is not in the sim"))
    }

    fn scripted(&mut self, op: SimGuestOp) -> io::Result<()> {
        match self.failures.get_mut(&op).and_then(VecDeque::pop_front) {
            Some(kind) => Err(io::Error::from(kind)),
            None => Ok(()),
        }
    }

    fn effect(&mut self, effect: SimGuestEffect) {
        self.effects.push(effect);
    }

    /// Adds a terminal condition to a socket; reported now if the socket is
    /// watched and this kind was not reported yet, else when it becomes watched.
    fn condition(&mut self, id: SocketId, hangup: GuestHangup) {
        let sock = self.sock(id);
        if sock.closed {
            return;
        }
        let key = hangup_key(hangup);
        sock.conditions.entry(key).or_insert(hangup);
        self.report_conditions(id);
    }

    fn report_conditions(&mut self, id: SocketId) {
        let sock = self.sock(id);
        if sock.closed || !sock.watched() {
            return;
        }
        let fresh: Vec<GuestHangup> = sock
            .conditions
            .iter()
            .filter(|(key, _)| !sock.reported.contains(key))
            .map(|(_, h)| *h)
            .collect();
        for hangup in fresh {
            self.sock(id).reported.insert(hangup_key(hangup));
            self.deliverable.push_back(GuestIoEvent::Hangup { socket: id, hangup });
        }
    }

    /// The guest ended one of its sockets (`abort` / `close`).
    fn end_socket(&mut self, id: SocketId, reset: bool) {
        let session = self.session;
        let sock = self.sock(id);
        if sock.closed {
            return;
        }
        sock.closed = true;
        sock.read_pending = None;
        let flow = sock.flow;
        let outer = sock.outer;
        let linger = sock.linger;
        let flow_vsock = sock.is_flow_vsock();
        let stage = sock.stage;
        let partner = sock.partner;
        // A socket that ends is no longer watched: drop its undelivered hangups.
        self.deliverable
            .retain(|e| !matches!(e, GuestIoEvent::Hangup { socket, .. } if *socket == id));
        if session == Some(id) {
            self.session = None;
            self.session_ended = true;
            self.host_messages.clear();
        }
        if let Some(flow) = flow {
            if outer {
                let event = if reset || linger { FarEndEvent::Reset } else { FarEndEvent::Eof };
                self.far_end.entry(flow).or_insert(event);
            }
            if flow_vsock && self.vsock_ended.insert(flow) {
                self.vsock_ends_in_flight.insert(flow);
            }
            if matches!(stage, Stage::LegStream | Stage::LegSeqpacket) {
                self.legs_closed_unobserved.insert(flow);
            }
        }
        if matches!(stage, Stage::LegStream) {
            if let Some(q) = partner {
                self.end_socket(q, reset);
            }
            if let Some(slot) = self.sockets.get(&id).and_then(|s| s.slot) {
                if let Some(rec) = self.slots.get_mut(&slot) {
                    rec.legs = None;
                    rec.routed = false;
                }
            }
        }
    }

    fn consume_abort(&mut self, ids: &[SocketId]) {
        for id in ids {
            self.end_socket(*id, true);
        }
    }

    fn require_connected(&self, id: SocketId) -> io::Result<()> {
        if self.sockets.get(&id).is_some_and(Sock::connected) {
            Ok(())
        } else {
            Err(io::Error::from(ErrorKind::NotConnected))
        }
    }

    fn new_pair(&mut self, vsock: SocketId, outer: SocketId) -> (GuestPairKey, Option<FlowId>) {
        let key = GuestPairKey(self.next_pair);
        self.next_pair += 1;
        let flow = self.sock(vsock).flow;
        for (id, stage) in [(vsock, Stage::PairVsock), (outer, Stage::PairOuter)] {
            let sock = self.sock(id);
            sock.stage = stage;
            sock.pair = Some(key);
            if sock.flow.is_none() {
                sock.flow = flow;
            }
        }
        self.pairs.insert(key, PairRec { vsock, outer });
        self.report_conditions(vsock);
        self.report_conditions(outer);
        (key, flow)
    }

    fn slot_mut(&mut self, slot: SocketAddrV4) -> &mut SlotRec {
        self.slots.get_mut(&slot).unwrap_or_else(|| unreachable!("{slot} is not a guest UDP slot"))
    }

    fn free_slot(&self) -> Option<SocketAddrV4> {
        self.slots.iter().find(|(_, r)| r.owner.is_none() && !r.quarantined).map(|(s, _)| *s)
    }

    fn park(&mut self, slot: SocketAddrV4, tag: (SocketCookie, SocketAddrV4)) {
        let rec = self.slot_mut(slot);
        if rec.routed {
            rec.forwarded += 1;
            rec.to_host += 1;
        } else {
            rec.parked.insert(tag);
            rec.parked_frames += 1;
            let generation = rec.generation;
            self.readiness.insert((slot, generation));
        }
    }

    fn flow_socket(&self, flow: FlowId, pred: impl Fn(&Sock) -> bool) -> Option<SocketId> {
        self.sockets
            .iter()
            .find(|(_, s)| !s.closed && s.flow == Some(flow) && pred(s))
            .map(|(id, _)| *id)
    }
}

// ---------------------------------------------------------------------------
// The seam sim
// ---------------------------------------------------------------------------

/// The guest-owner seam under test: a cloneable handle over shared state. The
/// owner owns its `Box`; the driver keeps a clone to inject, release and read.
#[derive(Clone)]
pub(crate) struct SimGuestOwnerIo {
    state: Arc<Mutex<SimState>>,
    clock: Arc<SimGuestClock>,
}

impl SimGuestOwnerIo {
    /// A guest with `GUEST_UDP_SLOTS` slots bound on
    /// `workload_addr:GUEST_UDP_SLOT_BASE + n` (all free, generation 0), an
    /// empty listener map, and no session. Reads `clock` — the instance the
    /// owner reads.
    pub(crate) fn new(workload_addr: Ipv4Addr, clock: Arc<SimGuestClock>) -> Self {
        let slots = (0..GUEST_UDP_SLOTS)
            .map(|n| {
                let slot = SocketAddrV4::new(workload_addr, GUEST_UDP_SLOT_BASE + n);
                let rec = SlotRec {
                    owner: None,
                    quarantined: false,
                    generation: CellGeneration(0),
                    parked: BTreeSet::new(),
                    parked_frames: 0,
                    routed: false,
                    flow: None,
                    legs: None,
                    to_host: 0,
                    to_guest: 0,
                    forwarded: 0,
                    fail_next_cell_creation: false,
                };
                (slot, rec)
            })
            .collect();
        let state = SimState {
            seam: SeamInstance::next(),
            next_socket: 1,
            next_pair: 1,
            sockets: BTreeMap::new(),
            deliverable: VecDeque::new(),
            waits: Vec::new(),
            effects: Vec::new(),
            failures: BTreeMap::new(),
            event_wait_failure: None,
            drained_default: true,
            drained: BTreeMap::new(),
            session: None,
            session_ended: false,
            host_closed: None,
            host_messages: VecDeque::new(),
            control_opens: Vec::new(),
            outbox: Vec::new(),
            inbound: BTreeMap::new(),
            reset_in_queue: BTreeSet::new(),
            pairs: BTreeMap::new(),
            far_end: BTreeMap::new(),
            vsock_ends_in_flight: BTreeSet::new(),
            vsock_ended: BTreeSet::new(),
            legs_closed_unobserved: BTreeSet::new(),
            slots,
            readiness: BTreeSet::new(),
            release_reports: BTreeSet::new(),
            live_cookies: BTreeSet::new(),
            listener_map: BTreeSet::new(),
            exhausted_sends: 0,
        };
        Self { state: Arc::new(Mutex::new(state)), clock }
    }

    fn st(&self) -> MutexGuard<'_, SimState> {
        lock(&self.state)
    }

    /// The clock this seam reads.
    pub(crate) fn clock(&self) -> Arc<SimGuestClock> {
        Arc::clone(&self.clock)
    }

    // ----- QPROJ-7 read surface -------------------------------------------

    /// QPROJ-7 arbiter read surface (see the module docs for its pairings).
    pub(crate) fn guest_socket_observation(&self) -> SimGuestSocketObservation {
        let st = self.st();
        SimGuestSocketObservation {
            queued_inbound: st
                .inbound
                .iter()
                .filter(|(_, rec)| rec.socket.is_none())
                .map(|(flow, _)| *flow)
                .collect(),
            reset_in_queue: st.reset_in_queue.clone(),
            outbox: st.outbox.clone(),
            session_ended: st.session_ended,
            readiness: st.readiness.clone(),
            parked: st
                .slots
                .iter()
                .filter(|(_, r)| !r.parked.is_empty())
                .map(|(s, r)| (*s, r.parked.clone()))
                .collect(),
            release_reports: st.release_reports.clone(),
            legs_closed_unobserved: st.legs_closed_unobserved.clone(),
            vsock_ends_in_flight: st.vsock_ends_in_flight.clone(),
            vsock_ended: st.vsock_ended.clone(),
            far_end: st.far_end.clone(),
        }
    }

    // ----- waits and delivery -----------------------------------------------

    /// Events released and not yet taken by `next_event`.
    pub(crate) fn deliverable(&self) -> usize {
        self.st().deliverable.len()
    }

    /// The deadline of the owner's most recent `next_event` call (outer `None`:
    /// no call yet).
    pub(crate) fn last_wait(&self) -> Option<Option<Instant>> {
        self.st().waits.last().map(|w| w.deadline)
    }

    /// The owner's most recent `next_event` call: its deadline and what it
    /// returned (`None`: no call yet).
    pub(crate) fn last_wait_record(&self) -> Option<SimWait> {
        self.st().waits.last().copied()
    }

    /// The owner's earliest armed deadline as its most recent wait passed it,
    /// while that deadline is still armed: the wait blocked
    /// ([`SimWouldBlock`] — after `test_support::settle` this is the owner's
    /// current earliest deadline, including one armed while handling its last
    /// input) or returned an event. A wait that returned at its deadline
    /// consumed it (`None` until the owner waits again). Test support for a
    /// driver's time actions; never a projection source.
    pub(crate) fn armed_deadline(&self) -> Option<Instant> {
        match self.st().waits.last() {
            Some(SimWait {
                deadline,
                outcome: SimWaitOutcome::Event | SimWaitOutcome::WouldBlock,
            }) => *deadline,
            _ => None,
        }
    }

    /// Every seam call the owner made, in order.
    pub(crate) fn effects(&self) -> Vec<SimGuestEffect> {
        self.st().effects.clone()
    }

    /// Clears the effect log (to scope an ordering oracle to one phase).
    pub(crate) fn clear_effects(&self) {
        self.st().effects.clear();
    }

    // ----- scripting -----------------------------------------------------------

    /// The next call of `op` fails with `kind` (queued per op).
    pub(crate) fn fail_next(&self, op: SimGuestOp, kind: ErrorKind) {
        self.st().failures.entry(op).or_default().push_back(kind);
    }

    /// The next `next_event` call fails: the readiness wait itself failed.
    pub(crate) fn fail_event_wait(&self, kind: ErrorKind) {
        self.st().event_wait_failure = Some(kind);
    }

    /// Creating a new cell for `slot` fails at its next `end_association`.
    pub(crate) fn fail_cell_creation(&self, slot: SocketAddrV4) {
        self.st().slot_mut(slot).fail_next_cell_creation = true;
    }

    /// What `drained` answers when no per-flow answer is set (default `true`).
    pub(crate) fn set_drained_default(&self, drained: bool) {
        self.st().drained_default = drained;
    }

    /// What `drained` answers for `flow`'s `direction`.
    pub(crate) fn set_drained(&self, flow: FlowId, direction: GuestFlowDirection, drained: bool) {
        self.st().drained.insert((flow, direction), drained);
    }

    // ----- connects the owner started -------------------------------------------

    /// Connects in flight, in issue order.
    pub(crate) fn pending_connects(&self) -> Vec<(SocketId, SimConnectTarget)> {
        let st = self.st();
        st.sockets
            .iter()
            .filter(|(_, s)| !s.closed && s.connect == Connect::InFlight)
            .map(|(id, s)| {
                let target = match (s.channel, s.app_target) {
                    (Some(channel), _) => SimConnectTarget::Host(channel),
                    (None, Some((client, destination))) => {
                        SimConnectTarget::App { client, destination, flow: s.flow }
                    }
                    (None, None) => unreachable!("a connecting socket has a target"),
                };
                (*id, target)
            })
            .collect()
    }

    /// Releases the completion of `socket`'s connect.
    pub(crate) fn complete_connect(&self, socket: SocketId, result: Result<(), ErrorKind>) {
        let mut st = self.st();
        let sock = st.sock(socket);
        assert!(
            !sock.closed && sock.connect == Connect::InFlight,
            "driver defect: {socket:?} has no connect in flight"
        );
        sock.connect = Connect::Released;
        st.deliverable.push_back(GuestIoEvent::Connected { socket, result });
    }

    /// Releases the oldest in-flight host connect on `channel`.
    pub(crate) fn complete_host_connect(
        &self,
        channel: HostChannel,
        result: Result<(), ErrorKind>,
    ) -> SocketId {
        let socket = self
            .pending_connects()
            .into_iter()
            .find(|(_, t)| *t == SimConnectTarget::Host(channel))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("driver defect: no {channel:?} connect in flight"));
        self.complete_connect(socket, result);
        socket
    }

    /// Releases the in-flight application connect armed toward `flow`'s V_g.
    pub(crate) fn complete_app_connect(
        &self,
        flow: FlowId,
        result: Result<(), ErrorKind>,
    ) -> SocketId {
        let socket = self
            .pending_connects()
            .into_iter()
            .find(|(_, t)| matches!(t, SimConnectTarget::App { flow: Some(f), .. } if *f == flow))
            .map(|(id, _)| id)
            .unwrap_or_else(|| {
                panic!("driver defect: no application connect in flight for {flow:?}")
            });
        self.complete_connect(socket, result);
        socket
    }

    // ----- control session (host side) ------------------------------------------

    /// The live session socket (`Connected` delivered, not ended).
    pub(crate) fn session_socket(&self) -> Option<SocketId> {
        self.st().session
    }

    /// Every `ControlOpen` the guest wrote, raw, in order.
    pub(crate) fn control_opens(&self) -> Vec<[u8; CONTROL_OPEN_LEN]> {
        self.st().control_opens.clone()
    }

    /// The host writes `message` on the live session (buffered until read).
    pub(crate) fn host_send_control(&self, message: ControlMessage) {
        let mut st = self.st();
        assert!(st.session.is_some(), "driver defect: no live session to write on");
        st.host_messages.push_back(message);
    }

    /// Releases the session's pending read with the next buffered host message.
    pub(crate) fn release_control_read(&self) {
        let mut st = self.st();
        let session = st.session.expect("driver defect: no live session");
        assert!(
            st.sock(session).read_pending == Some(GuestRecord::ControlMessage),
            "driver defect: the owner has no control-message read pending on the session"
        );
        let message =
            st.host_messages.pop_front().expect("driver defect: no host message buffered");
        st.sock(session).read_pending = None;
        st.deliverable.push_back(GuestIoEvent::Read {
            socket: session,
            result: Ok(message.encode().to_vec()),
        });
    }

    /// `host_send_control` then `release_control_read`.
    pub(crate) fn host_control(&self, message: ControlMessage) {
        self.host_send_control(message);
        self.release_control_read();
    }

    /// The host consumes every message the guest wrote and returns them.
    pub(crate) fn take_outbox(&self) -> Vec<ControlMessage> {
        std::mem::take(&mut self.st().outbox)
    }

    /// The host consumes the first written message equal to `message`;
    /// `false` when the outbox holds none.
    pub(crate) fn host_take_message(&self, message: ControlMessage) -> bool {
        let mut st = self.st();
        match st.outbox.iter().position(|m| *m == message) {
            Some(i) => {
                st.outbox.remove(i);
                true
            }
            None => false,
        }
    }

    /// The guest owner process dies (the driver dropped the owner): the kernel
    /// closes every socket it held — linger resets each application leg, every
    /// flow vsock and leg ends, the session ends.
    pub(crate) fn guest_owner_dies(&self) {
        let mut st = self.st();
        let open: Vec<SocketId> =
            st.sockets.iter().filter(|(_, s)| !s.closed).map(|(id, _)| *id).collect();
        for id in open {
            st.end_socket(id, true);
        }
        st.session = None;
        st.session_ended = true;
        st.host_closed = None;
    }

    /// The host ends the control session: a pending read completes at EOF (or
    /// `ECONNRESET`); with no read pending, the session socket reports
    /// `POLLHUP` (or `POLLERR`).
    pub(crate) fn end_session(&self, how: SimSessionEnd) {
        let mut st = self.st();
        let Some(session) = st.session.take() else { return };
        st.session_ended = true;
        st.host_closed = None;
        st.host_messages.clear();
        let pending = st.sock(session).read_pending.take();
        if pending.is_some() {
            let result = match how {
                SimSessionEnd::Eof => Ok(Vec::new()),
                SimSessionEnd::Reset => Err(ErrorKind::ConnectionReset),
            };
            st.deliverable.push_back(GuestIoEvent::Read { socket: session, result });
        } else {
            let hangup = match how {
                SimSessionEnd::Eof => GuestHangup::Hangup,
                SimSessionEnd::Reset => GuestHangup::Error(ErrorKind::ConnectionReset),
            };
            st.sock(session).conditions.insert(hangup_key(hangup), hangup);
            st.deliverable.push_back(GuestIoEvent::Hangup { socket: session, hangup });
            st.sock(session).reported.insert(hangup_key(hangup));
        }
    }

    /// The host owner dies (`serve` killed): every guest flow vsock ends with no
    /// announcement, then the session is reset (P-42 order).
    pub(crate) fn host_owner_dies(&self) {
        let flows: Vec<FlowId> = {
            let st = self.st();
            let queued = st.inbound.iter().filter(|(_, r)| r.socket.is_none()).map(|(f, _)| *f);
            let open = st
                .sockets
                .values()
                .filter(|s| !s.closed && s.is_flow_vsock())
                .filter_map(|s| s.flow);
            queued.chain(open).collect()
        };
        for flow in flows {
            self.host_vsock_end(flow);
        }
        self.end_session(SimSessionEnd::Reset);
    }

    /// The host ends the control session, and the guest has not observed it
    /// yet (K-B4: an owner's in-flight messages are delivered before the
    /// survivor observes the loss). `session_ended` holds from now on; the
    /// host's buffered messages stay readable (`release_control_read`); the
    /// guest's later writes are lost; nothing is delivered until
    /// `release_session_end`.
    pub(crate) fn host_end_session_deferred(&self, how: SimSessionEnd) {
        let mut st = self.st();
        assert!(st.session.is_some(), "driver defect: no live session to end");
        st.session_ended = true;
        st.host_closed = Some(how);
    }

    /// The host has ended the session and the guest has not observed it.
    pub(crate) fn session_end_pending(&self) -> bool {
        self.st().host_closed.is_some()
    }

    /// Releases a deferred end (`host_end_session_deferred`) to the guest,
    /// as `end_session` with the deferred kind.
    pub(crate) fn release_session_end(&self) {
        let how = self.st().host_closed.take().expect("driver defect: no deferred session end");
        self.end_session(how);
    }

    /// The host consumes the oldest message the guest wrote (FIFO).
    pub(crate) fn host_take_next(&self) -> Option<ControlMessage> {
        let mut st = self.st();
        if st.outbox.is_empty() { None } else { Some(st.outbox.remove(0)) }
    }

    /// The host consumes the oldest message the guest wrote that matches
    /// `pred`.
    pub(crate) fn host_take_first(
        &self,
        pred: impl Fn(&ControlMessage) -> bool,
    ) -> Option<ControlMessage> {
        let mut st = self.st();
        let i = st.outbox.iter().position(pred)?;
        Some(st.outbox.remove(i))
    }

    /// A scripted error on `flow`'s guest vsock (`POLLERR` with `kind`).
    pub(crate) fn flow_vsock_error(&self, flow: FlowId, kind: ErrorKind) {
        let mut st = self.st();
        let id = st
            .flow_socket(flow, Sock::is_flow_vsock)
            .expect("driver defect: no open vsock for that flow");
        st.condition(id, GuestHangup::Error(kind));
    }

    // ----- host-opened flows (TcpAccept) -------------------------------------------

    /// The host connects V_h to the guest (CID, 1242) and writes `request`:
    /// V_g waits in the guest accept queue with the request buffered.
    pub(crate) fn host_open_accept(&self, request: FlowRequest) {
        let FlowRequest::TcpAccept { flow, .. } = request else {
            panic!("driver defect: the host opens only TcpAccept flows on 1242");
        };
        self.st().inbound.insert(flow, InboundRec { request, socket: None });
    }

    /// Releases the guest's accept of `flow`'s V_g (`InboundAccepted`).
    pub(crate) fn release_inbound_accept(&self, flow: FlowId) {
        let mut st = self.st();
        let handle = st.handle();
        let id = handle.id;
        let rec = st.inbound.get_mut(&flow).expect("driver defect: no V_g queued for that flow");
        assert!(rec.socket.is_none(), "driver defect: V_g already accepted");
        rec.socket = Some(id);
        let mut sock = Sock::new(Stage::Inbound);
        sock.flow = Some(flow);
        st.sockets.insert(id, sock);
        st.deliverable.push_back(GuestIoEvent::InboundAccepted { conn: InboundConn(handle) });
    }

    /// Releases the pending read of `flow`'s request on V_g.
    pub(crate) fn release_inbound_request(&self, flow: FlowId) {
        let mut st = self.st();
        let rec = st.inbound.get(&flow).expect("driver defect: no V_g for that flow");
        let id = rec.socket.expect("driver defect: V_g not accepted yet");
        let bytes = rec.request.encode().to_vec();
        let sock = st.sock(id);
        assert!(
            sock.read_pending == Some(GuestRecord::FlowRequest),
            "driver defect: the owner has no request read pending on V_g"
        );
        sock.read_pending = None;
        st.deliverable.push_back(GuestIoEvent::Read { socket: id, result: Ok(bytes) });
    }

    /// The host ends `flow`'s vsock (closes V_h, aborts, or dies): a V_g still in
    /// the accept queue is torn down there (K-A4); an accepted V_g reports the
    /// end through `inbound_ended`; a routed V_g reports `POLLRDHUP`.
    pub(crate) fn host_vsock_end(&self, flow: FlowId) {
        let mut st = self.st();
        if st.inbound.get(&flow).is_some_and(|r| r.socket.is_none()) {
            st.inbound.remove(&flow);
            st.reset_in_queue.insert(flow);
            return;
        }
        let Some(id) = st.flow_socket(flow, Sock::is_flow_vsock) else { return };
        st.sock(id).peer_ended = true;
        st.condition(id, GuestHangup::ReadHangup);
    }

    /// The host observes the guest's end of `flow`'s vsock.
    pub(crate) fn host_observe_vsock_end(&self, flow: FlowId) {
        self.st().vsock_ends_in_flight.remove(&flow);
    }

    /// The host observes the end of `flow`'s datagram legs.
    pub(crate) fn host_observe_legs_end(&self, flow: FlowId) {
        self.st().legs_closed_unobserved.remove(&flow);
    }

    // ----- application side -------------------------------------------------------

    /// The application's `connect()` is captured: the intake child I is
    /// accepted (`OutboundCaptured`). Marks `cookie` live.
    pub(crate) fn app_connect(
        &self,
        cookie: SocketCookie,
        original_destination: SocketAddrV4,
        parked: bool,
    ) -> SocketId {
        let mut st = self.st();
        let handle = st.handle();
        let id = handle.id;
        let mut sock = Sock::new(Stage::Captured);
        sock.cookie = Some(cookie);
        sock.outer = true;
        sock.linger = true;
        st.sockets.insert(id, sock);
        st.live_cookies.insert(cookie);
        st.deliverable.push_back(GuestIoEvent::OutboundCaptured {
            child: CapturedConn(handle),
            original_destination,
            cookie,
            parked,
        });
        id
    }

    /// The application half-closes or resets its connection.
    pub(crate) fn app_end(&self, target: SimAppTarget, how: SimAppEnd) {
        let mut st = self.st();
        let id = match target {
            SimAppTarget::Captured(cookie) => st
                .sockets
                .iter()
                .find(|(_, s)| !s.closed && s.outer && s.cookie == Some(cookie))
                .map(|(id, _)| *id),
            SimAppTarget::Flow(flow) => st.flow_socket(flow, |s| s.outer),
        }
        .unwrap_or_else(|| panic!("driver defect: no open application leg for {target:?}"));
        let hangup = match how {
            SimAppEnd::HalfClose => GuestHangup::ReadHangup,
            SimAppEnd::Reset => GuestHangup::Error(ErrorKind::ConnectionReset),
        };
        st.condition(id, hangup);
    }

    /// The application sends a datagram to `destination` from socket `cookie`
    /// (the guest capture program): the slot the pair owns, or the lowest free
    /// slot; `Err(PermissionDenied)` when none is free (D25 exhaustion).
    pub(crate) fn app_send(
        &self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
    ) -> Result<SocketAddrV4, ErrorKind> {
        let mut st = self.st();
        st.live_cookies.insert(cookie);
        let tag = (cookie, destination);
        let owned =
            st.slots.iter().find(|(_, r)| r.owner == Some(tag) && !r.quarantined).map(|(s, _)| *s);
        let slot = match owned.or_else(|| st.free_slot()) {
            Some(slot) => slot,
            None => {
                st.exhausted_sends += 1;
                return Err(ErrorKind::PermissionDenied);
            }
        };
        st.slot_mut(slot).owner = Some(tag);
        st.park(slot, tag);
        Ok(slot)
    }

    /// As `app_send`, claiming `slot` (which must be free) for a new pair — the
    /// driver's choice of the model's nondeterministic slot.
    pub(crate) fn app_send_on(
        &self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
        slot: SocketAddrV4,
    ) {
        let mut st = self.st();
        st.live_cookies.insert(cookie);
        let tag = (cookie, destination);
        let rec = st.slot_mut(slot);
        assert!(
            rec.owner == Some(tag) || (rec.owner.is_none() && !rec.quarantined),
            "driver defect: {slot} is owned by another (socket, destination)"
        );
        rec.owner = Some(tag);
        st.park(slot, tag);
    }

    /// Sends that found no free slot (`guest_udp_slot_exhausted`).
    pub(crate) fn exhausted_sends(&self) -> u64 {
        self.st().exhausted_sends
    }

    /// The host forwards a datagram to the guest on `slot`'s association.
    pub(crate) fn host_datagram(&self, slot: SocketAddrV4) {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        assert!(rec.routed, "driver defect: {slot} has no routed association");
        rec.to_guest += 1;
    }

    /// Releases the readiness event `(slot, generation)` (`SlotReady`).
    pub(crate) fn release_slot_ready(&self, slot: SocketAddrV4, generation: CellGeneration) {
        let mut st = self.st();
        assert!(st.readiness.remove(&(slot, generation)), "driver defect: no such readiness event");
        st.deliverable.push_back(GuestIoEvent::SlotReady { slot, generation });
    }

    /// The application socket `cookie` is released; `report` = the
    /// `sock_release` report was queued (else it was lost, K-B2).
    pub(crate) fn app_close_socket(&self, cookie: SocketCookie, report: bool) {
        let mut st = self.st();
        st.live_cookies.remove(&cookie);
        if report {
            st.release_reports.insert(cookie);
        }
    }

    /// Releases the queued `sock_release` report of `cookie` (`SocketReleased`).
    pub(crate) fn release_socket_report(&self, cookie: SocketCookie) {
        let mut st = self.st();
        assert!(
            st.release_reports.remove(&cookie),
            "driver defect: no report queued for that cookie"
        );
        st.deliverable.push_back(GuestIoEvent::SocketReleased { cookie });
    }

    /// Test-support snapshot of `slot` in the sim's guest kernel.
    pub(crate) fn slot_state(&self, slot: SocketAddrV4) -> SimSlotState {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        SimSlotState {
            owner: rec.owner,
            in_pool: rec.owner.is_none() && !rec.quarantined,
            quarantined: rec.quarantined,
            generation: rec.generation,
            parked_frames: rec.parked_frames,
            routed: rec.routed,
            flow: rec.flow,
            legs_installed: rec.legs.is_some(),
            forwarded_frames: rec.forwarded,
        }
    }

    // ----- listeners -----------------------------------------------------------------

    /// Replaces the kernel listener map.
    pub(crate) fn set_listener_map(&self, ports: BTreeSet<ListenPort>) {
        self.st().listener_map = ports;
    }

    /// The listener map changed: a `ListenerMapWake` record.
    pub(crate) fn listener_map_wake(&self) {
        self.st().deliverable.push_back(GuestIoEvent::ListenerMapWake);
    }

    /// A setup listener's accept failed (`ListenerFailed`).
    pub(crate) fn listener_failed(&self, listener: GuestListener, kind: ErrorKind) {
        self.st().deliverable.push_back(GuestIoEvent::ListenerFailed { listener, kind });
    }
}

impl GuestOwnerIo for SimGuestOwnerIo {
    fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<Option<GuestIoEvent>> {
        let now = self.clock.now();
        let mut st = self.st();
        if let Some(kind) = st.event_wait_failure.take() {
            st.waits.push(SimWait { deadline, outcome: SimWaitOutcome::Failed });
            return Err(io::Error::from(kind));
        }
        if let Some(event) = st.deliverable.pop_front() {
            st.waits.push(SimWait { deadline, outcome: SimWaitOutcome::Event });
            if let GuestIoEvent::Connected { socket, result } = &event {
                let (socket, result) = (*socket, *result);
                if let Some(sock) = st.sockets.get_mut(&socket) {
                    sock.connect = Connect::Delivered(result);
                    let session = result.is_ok()
                        && !sock.closed
                        && sock.channel == Some(HostChannel::Control);
                    if session {
                        st.session = Some(socket);
                        st.session_ended = false;
                    }
                }
            }
            return Ok(Some(event));
        }
        match deadline {
            Some(at) if now >= at => {
                st.waits.push(SimWait { deadline, outcome: SimWaitOutcome::DeadlinePassed });
                Ok(None)
            }
            _ => {
                st.waits.push(SimWait { deadline, outcome: SimWaitOutcome::WouldBlock });
                drop(st);
                // DG-5: a driver defect panics rather than block; never an
                // `Err` (that is the owner's fatal `EventWait`).
                std::panic::panic_any(SimWouldBlock { deadline })
            }
        }
    }

    fn start_connect_host(&mut self, channel: HostChannel) -> io::Result<HostConn> {
        let mut st = self.st();
        st.scripted(SimGuestOp::StartConnectHost)?;
        let handle = st.handle();
        let mut sock = Sock::new(Stage::Host);
        sock.channel = Some(channel);
        sock.connect = Connect::InFlight;
        st.sockets.insert(handle.id, sock);
        st.effect(SimGuestEffect::ConnectHost { socket: handle.id, channel });
        Ok(HostConn(handle))
    }

    fn start_read_host(&mut self, conn: &HostConn, record: GuestRecord) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.require_connected(id)?;
        st.scripted(SimGuestOp::StartReadHost)?;
        st.effect(SimGuestEffect::ReadHost { socket: id, record });
        if st.sock(id).channel == Some(HostChannel::Control) {
            assert_eq!(
                record,
                GuestRecord::ControlMessage,
                "seam contract: the session reads control messages"
            );
        }
        st.sock(id).read_pending = Some(record);
        Ok(())
    }

    fn start_read_inbound(&mut self, conn: &InboundConn) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::StartReadInbound)?;
        st.effect(SimGuestEffect::ReadInbound { socket: id });
        st.sock(id).read_pending = Some(GuestRecord::FlowRequest);
        Ok(())
    }

    fn write_host(&mut self, conn: &HostConn, bytes: &[u8]) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.require_connected(id)?;
        if st.session != Some(id) {
            return Err(io::Error::from(ErrorKind::BrokenPipe));
        }
        st.scripted(SimGuestOp::WriteHost)?;
        match bytes.len() {
            CONTROL_OPEN_LEN => {
                let mut open = [0u8; CONTROL_OPEN_LEN];
                open.copy_from_slice(bytes);
                st.control_opens.push(open);
                st.effect(SimGuestEffect::WriteControlOpen { socket: id });
            }
            CONTROL_MESSAGE_LEN => {
                let mut raw = [0u8; CONTROL_MESSAGE_LEN];
                raw.copy_from_slice(bytes);
                let message = ControlMessage::decode(&raw).unwrap_or_else(|e| {
                    panic!("owner defect: malformed control message written: {e}")
                });
                // A message written after the host ended the session (not yet
                // observed by the guest) never reaches the host.
                if st.host_closed.is_none() {
                    st.outbox.push(message);
                }
                st.effect(SimGuestEffect::WriteControl { socket: id, message });
            }
            n => panic!(
                "seam contract: a session write is one whole 24- or 8-byte record, not {n} bytes"
            ),
        }
        Ok(())
    }

    fn inbound_ended(&mut self, conn: &InboundConn) -> io::Result<bool> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::InboundEnded)?;
        let ended = st.sock(id).peer_ended;
        st.effect(SimGuestEffect::InboundEnded { socket: id, ended });
        Ok(ended)
    }

    fn rearm_captured(&mut self, child: &CapturedConn) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&child.0);
        st.scripted(SimGuestOp::RearmCaptured)?;
        st.effect(SimGuestEffect::RearmCaptured { socket: id });
        Ok(())
    }

    fn install_outbound(
        &mut self,
        vsock: HostConn,
        child: &CapturedConn,
    ) -> io::Result<InstalledConn> {
        let mut st = self.st();
        let v = st.check(&vsock.0);
        let c = st.check(&child.0);
        if let Err(e) =
            st.require_connected(v).and_then(|()| st.scripted(SimGuestOp::InstallOutbound))
        {
            st.consume_abort(&[v]);
            return Err(e);
        }
        let sock = st.sock(v);
        sock.stage = Stage::Installed;
        sock.routes_to = Some(c);
        st.effect(SimGuestEffect::InstallOutbound { vsock: v, child: c });
        Ok(InstalledConn(vsock.0))
    }

    fn write_request(
        &mut self,
        vsock: &InstalledConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&vsock.0);
        st.scripted(SimGuestOp::WriteRequest)?;
        let decoded = FlowRequest::decode(request)
            .unwrap_or_else(|e| panic!("owner defect: malformed flow request: {e}"));
        let FlowRequest::TcpConnect { flow, .. } = decoded else {
            panic!("seam contract: write_request carries a TcpConnect request, got {decoded:?}");
        };
        let routes_to = st.sock(id).routes_to;
        st.sock(id).flow = Some(flow);
        if let Some(child) = routes_to {
            st.sock(child).flow = Some(flow);
        }
        st.effect(SimGuestEffect::WriteRequest { socket: id, request: decoded });
        Ok(())
    }

    fn route_outbound(
        &mut self,
        vsock: InstalledConn,
        child: CapturedConn,
    ) -> io::Result<GuestPair> {
        let mut st = self.st();
        let v = st.check(&vsock.0);
        let c = st.check(&child.0);
        if let Err(e) = st.scripted(SimGuestOp::RouteOutbound) {
            st.consume_abort(&[v, c]);
            return Err(e);
        }
        let (key, flow) = st.new_pair(v, c);
        st.effect(SimGuestEffect::RouteOutbound { pair: key, flow });
        Ok(GuestPair { key, vsock: GuestPairSocket(vsock.0), outer: GuestPairSocket(child.0) })
    }

    fn install_inbound(&mut self, vsock: InboundConn) -> io::Result<InstalledConn> {
        let mut st = self.st();
        let id = st.check(&vsock.0);
        if let Err(e) = st.scripted(SimGuestOp::InstallInbound) {
            st.consume_abort(&[id]);
            return Err(e);
        }
        st.sock(id).stage = Stage::Installed;
        st.effect(SimGuestEffect::InstallInbound { socket: id });
        Ok(InstalledConn(vsock.0))
    }

    fn start_connect_app(
        &mut self,
        vsock: &InstalledConn,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> io::Result<AppConn> {
        let mut st = self.st();
        let v = st.check(&vsock.0);
        st.scripted(SimGuestOp::StartConnectApp)?;
        let flow = st.sock(v).flow;
        let handle = st.handle();
        let mut sock = Sock::new(Stage::App);
        sock.connect = Connect::InFlight;
        sock.app_target = Some((client, destination));
        sock.flow = flow;
        sock.outer = true;
        sock.linger = true;
        st.sockets.insert(handle.id, sock);
        st.effect(SimGuestEffect::StartConnectApp { socket: handle.id, client, destination });
        Ok(AppConn(handle))
    }

    fn route_inbound(&mut self, vsock: InstalledConn, app: AppConn) -> io::Result<GuestPair> {
        let mut st = self.st();
        let v = st.check(&vsock.0);
        let a = st.check(&app.0);
        if let Err(e) = st.require_connected(a).and_then(|()| st.scripted(SimGuestOp::RouteInbound))
        {
            st.consume_abort(&[v, a]);
            return Err(e);
        }
        let (key, flow) = st.new_pair(v, a);
        st.effect(SimGuestEffect::RouteInbound { pair: key, flow });
        Ok(GuestPair { key, vsock: GuestPairSocket(vsock.0), outer: GuestPairSocket(app.0) })
    }

    fn drained(&mut self, pair: GuestPairKey, direction: GuestFlowDirection) -> io::Result<bool> {
        let mut st = self.st();
        st.scripted(SimGuestOp::Drained)?;
        st.effect(SimGuestEffect::Drained { pair, direction });
        let flow = st.pairs.get(&pair).and_then(|p| st.sockets.get(&p.vsock)).and_then(|s| s.flow);
        let answer = flow
            .and_then(|f| st.drained.get(&(f, direction)).copied())
            .unwrap_or(st.drained_default);
        Ok(answer)
    }

    fn remove_pair(&mut self, pair: GuestPairKey) -> io::Result<()> {
        let mut st = self.st();
        st.scripted(SimGuestOp::RemovePair)?;
        st.pairs.remove(&pair);
        st.effect(SimGuestEffect::RemovePair { pair });
        Ok(())
    }

    fn shutdown_write(&mut self, socket: &GuestPairSocket) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&socket.0);
        st.scripted(SimGuestOp::ShutdownWrite)?;
        let (flow, stage) = {
            let s = st.sock(id);
            (s.flow, s.stage)
        };
        let vsock = stage == Stage::PairVsock;
        if let Some(flow) = flow {
            if vsock {
                if st.vsock_ended.insert(flow) {
                    st.vsock_ends_in_flight.insert(flow);
                }
            } else {
                st.far_end.entry(flow).or_insert(FarEndEvent::Eof);
            }
        }
        st.effect(SimGuestEffect::ShutdownWrite { socket: id, flow, vsock });
        Ok(())
    }

    fn clear_linger(&mut self, socket: &GuestPairSocket) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&socket.0);
        st.scripted(SimGuestOp::ClearLinger)?;
        st.sock(id).linger = false;
        st.effect(SimGuestEffect::ClearLinger { socket: id });
        Ok(())
    }

    fn abort(&mut self, socket: GuestOwnedSocket) {
        let mut st = self.st();
        let id = st.check(owned_handle(&socket));
        let flow = st.sock(id).flow;
        st.end_socket(id, true);
        st.effect(SimGuestEffect::Abort { socket: id, flow });
    }

    fn close(&mut self, socket: GuestOwnedSocket) {
        let mut st = self.st();
        let id = st.check(owned_handle(&socket));
        let flow = st.sock(id).flow;
        st.end_socket(id, false);
        st.effect(SimGuestEffect::Close { socket: id, flow });
    }

    fn slot_owner(
        &mut self,
        slot: SocketAddrV4,
    ) -> io::Result<Option<(SocketCookie, SocketAddrV4)>> {
        let mut st = self.st();
        st.scripted(SimGuestOp::SlotOwner)?;
        st.effect(SimGuestEffect::SlotOwner { slot });
        Ok(st.slot_mut(slot).owner)
    }

    fn install_slot_legs(
        &mut self,
        slot: SocketAddrV4,
        stream: HostConn,
        seqpacket: HostConn,
    ) -> io::Result<SlotLegs> {
        let mut st = self.st();
        let t = st.check(&stream.0);
        let q = st.check(&seqpacket.0);
        assert_eq!(
            st.sock(t).channel,
            Some(HostChannel::FlowStream),
            "seam contract: T_g is a 1240 STREAM"
        );
        assert_eq!(
            st.sock(q).channel,
            Some(HostChannel::FlowSeqpacket),
            "seam contract: Q_g is a 1241 SEQPACKET"
        );
        let result = st
            .require_connected(t)
            .and_then(|()| st.require_connected(q))
            .and_then(|()| st.scripted(SimGuestOp::InstallSlotLegs));
        if let Err(e) = result {
            st.consume_abort(&[t, q]);
            return Err(e);
        }
        for (id, stage, partner) in [(t, Stage::LegStream, Some(q)), (q, Stage::LegSeqpacket, None)]
        {
            let sock = st.sock(id);
            sock.stage = stage;
            sock.slot = Some(slot);
            sock.partner = partner;
        }
        let rec = st.slot_mut(slot);
        assert!(rec.legs.is_none(), "owner defect: {slot} already has legs installed");
        rec.legs = Some((t, q));
        st.effect(SimGuestEffect::InstallSlotLegs { slot });
        Ok(SlotLegs(stream.0))
    }

    fn write_slot_requests(
        &mut self,
        legs: &SlotLegs,
        stream: &[u8; FLOW_REQUEST_LEN],
        seqpacket: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let mut st = self.st();
        let t = st.check(&legs.0);
        st.scripted(SimGuestOp::WriteSlotRequests)?;
        let s = FlowRequest::decode(stream)
            .unwrap_or_else(|e| panic!("owner defect: malformed request: {e}"));
        let p = FlowRequest::decode(seqpacket)
            .unwrap_or_else(|e| panic!("owner defect: malformed request: {e}"));
        let (
            FlowRequest::DatagramStream { flow, .. },
            FlowRequest::DatagramSeqpacket { flow: flow_q, .. },
        ) = (s, p)
        else {
            panic!(
                "seam contract: DatagramStream on T_g and DatagramSeqpacket on Q_g, got {s:?} / {p:?}"
            );
        };
        assert_eq!(flow, flow_q, "owner defect: the two legs name different flows");
        let slot = st.sock(t).slot.expect("legs belong to a slot");
        let q = st.sock(t).partner;
        st.sock(t).flow = Some(flow);
        if let Some(q) = q {
            st.sock(q).flow = Some(flow);
        }
        st.slot_mut(slot).flow = Some(flow);
        st.effect(SimGuestEffect::WriteSlotRequests { slot, flow });
        Ok(())
    }

    fn route_slot(&mut self, legs: &SlotLegs) -> io::Result<()> {
        let mut st = self.st();
        let t = st.check(&legs.0);
        st.scripted(SimGuestOp::RouteSlot)?;
        let slot = st.sock(t).slot.expect("legs belong to a slot");
        let rec = st.slot_mut(slot);
        rec.routed = true;
        let frames = u64::try_from(rec.parked_frames).unwrap_or(u64::MAX);
        rec.forwarded += frames;
        rec.to_host += frames;
        rec.parked.clear();
        rec.parked_frames = 0;
        st.effect(SimGuestEffect::RouteSlot { slot });
        Ok(())
    }

    fn end_association(&mut self, slot: SocketAddrV4, legs: Option<SlotLegs>) -> SlotEnd {
        let mut st = self.st();
        if let Some(legs) = &legs {
            let t = st.check(&legs.0);
            assert_eq!(
                st.slot_mut(slot).legs.map(|(t, _)| t),
                Some(t),
                "owner defect: legs of another slot"
            );
            // steps 1-2: cut the reassembly output, close T_g and Q_g
            st.end_socket(t, false);
        }
        let rec = st.slot_mut(slot);
        // step 3: the old cells close first — every parked frame is discarded
        rec.parked.clear();
        rec.parked_frames = 0;
        rec.routed = false;
        rec.flow = None;
        rec.legs = None;
        let end = if std::mem::take(&mut rec.fail_next_cell_creation) {
            rec.quarantined = true;
            rec.owner = None;
            SlotEnd::Quarantined
        } else {
            rec.generation = CellGeneration(rec.generation.0 + 1);
            SlotEnd::Recreated(rec.generation)
        };
        // steps 4-5 (disconnect the slot socket, remove the unframe tuple) leave
        // no state the sim models
        st.effect(SimGuestEffect::EndAssociation { slot, end });
        end
    }

    fn release_slot(&mut self, slot: SocketAddrV4) {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        assert!(
            !rec.quarantined && rec.legs.is_none() && rec.flow.is_none() && !rec.routed,
            "seam contract: release_slot on {slot} whose association has not ended (Recreated)"
        );
        rec.owner = None;
        st.effect(SimGuestEffect::ReleaseSlot { slot });
    }

    fn retry_quarantined(&mut self, slot: SocketAddrV4) -> io::Result<CellGeneration> {
        let mut st = self.st();
        st.effect(SimGuestEffect::RetryQuarantined { slot });
        assert!(
            st.slot_mut(slot).quarantined,
            "seam contract: retry_quarantined on {slot}, which is not quarantined"
        );
        st.scripted(SimGuestOp::RetryQuarantined)?;
        let rec = st.slot_mut(slot);
        rec.quarantined = false;
        rec.generation = CellGeneration(rec.generation.0 + 1);
        Ok(rec.generation)
    }

    fn slot_counters(&mut self, slot: SocketAddrV4) -> io::Result<(u64, u64)> {
        let mut st = self.st();
        st.scripted(SimGuestOp::SlotCounters)?;
        st.effect(SimGuestEffect::SlotCounters { slot });
        let rec = st.slot_mut(slot);
        Ok((rec.to_host, rec.to_guest))
    }

    fn live_cookies(
        &mut self,
        cookies: &BTreeSet<SocketCookie>,
    ) -> io::Result<BTreeSet<SocketCookie>> {
        let mut st = self.st();
        st.scripted(SimGuestOp::LiveCookies)?;
        st.effect(SimGuestEffect::LiveCookies);
        Ok(cookies.intersection(&st.live_cookies).copied().collect())
    }

    fn listener_map(&mut self) -> io::Result<BTreeSet<ListenPort>> {
        let mut st = self.st();
        st.scripted(SimGuestOp::ListenerMap)?;
        st.effect(SimGuestEffect::ListenerMap);
        Ok(st.listener_map.clone())
    }
}

const fn owned_handle(socket: &GuestOwnedSocket) -> &Handle {
    match socket {
        GuestOwnedSocket::Host(h) => &h.0,
        GuestOwnedSocket::Inbound(h) => &h.0,
        GuestOwnedSocket::Captured(h) => &h.0,
        GuestOwnedSocket::Installed(h) => &h.0,
        GuestOwnedSocket::App(h) => &h.0,
        GuestOwnedSocket::Legs(h) => &h.0,
        GuestOwnedSocket::Pair(h) => &h.0,
    }
}

// ---------------------------------------------------------------------------
// The sim's own invariants (default lane; no core codec, no owner)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod sim_tests {
    use super::*;

    const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);

    fn sim() -> (SimGuestOwnerIo, Arc<SimGuestClock>) {
        let clock = Arc::new(SimGuestClock::new());
        (SimGuestOwnerIo::new(WL, Arc::clone(&clock)), clock)
    }

    fn slot(n: u16) -> SocketAddrV4 {
        SocketAddrV4::new(WL, GUEST_UDP_SLOT_BASE + n)
    }

    fn dest(n: u8) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, n), 53)
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn next_event_returns_none_only_once_the_deadline_has_passed() {
        let (mut io, clock) = sim();
        let deadline = clock.now() + Duration::from_millis(250);
        clock.advance(Duration::from_millis(250));
        assert!(io.next_event(Some(deadline)).expect("wait ok").is_none());
        assert_eq!(io.last_wait(), Some(Some(deadline)));
    }

    /// The payload of a `next_event` call that panicked with `SimWouldBlock`.
    fn blocked_wait(io: &mut SimGuestOwnerIo, deadline: Option<Instant>) -> SimWouldBlock {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = io.next_event(deadline);
        }))
        .expect_err("a single-threaded driver would block: the sim panics");
        *payload.downcast::<SimWouldBlock>().expect("the payload is the typed SimWouldBlock")
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn next_event_with_nothing_released_and_no_deadline_passed_panics_sim_would_block() {
        let (mut io, clock) = sim();
        let deadline = clock.now() + Duration::from_secs(1);
        let before = io.guest_socket_observation();
        assert_eq!(
            blocked_wait(&mut io, Some(deadline)),
            SimWouldBlock { deadline: Some(deadline) }
        );
        assert_eq!(
            io.last_wait_record(),
            Some(SimWait { deadline: Some(deadline), outcome: SimWaitOutcome::WouldBlock })
        );
        assert_eq!(io.guest_socket_observation(), before, "the blocked wait changes nothing else");
        clock.advance(Duration::from_secs(1));
        assert!(
            io.next_event(Some(deadline)).expect("a passed deadline is no wait failure").is_none(),
            "the seam is usable after the panic; the passed deadline returns None"
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn next_event_waiting_forever_with_nothing_released_panics_sim_would_block() {
        let (mut io, _clock) = sim();
        assert_eq!(blocked_wait(&mut io, None), SimWouldBlock { deadline: None });
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn armed_deadline_is_the_deadline_of_the_blocking_or_event_wait_until_consumed() {
        let (mut io, clock) = sim();
        assert_eq!(io.armed_deadline(), None, "no wait yet");
        io.listener_map_wake();
        let deadline = clock.now() + Duration::from_secs(1);
        assert!(matches!(io.next_event(Some(deadline)), Ok(Some(GuestIoEvent::ListenerMapWake))));
        assert_eq!(io.armed_deadline(), Some(deadline), "an event wait leaves it armed");
        let later = deadline + Duration::from_secs(1);
        blocked_wait(&mut io, Some(later));
        assert_eq!(
            io.armed_deadline(),
            Some(later),
            "the blocking wait names the current deadline"
        );
        clock.advance(Duration::from_secs(2));
        assert!(io.next_event(Some(later)).expect("wait ok").is_none());
        assert_eq!(io.armed_deadline(), None, "a wait that returned at its deadline consumed it");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_deadline_at_now_returns_none_without_blocking() {
        let (mut io, clock) = sim();
        let now = clock.now();
        assert!(io.next_event(Some(now)).expect("no wait failure").is_none());
        assert_eq!(
            io.last_wait_record(),
            Some(SimWait { deadline: Some(now), outcome: SimWaitOutcome::DeadlinePassed })
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn an_event_pending_is_returned_even_past_the_deadline_and_only_once() {
        let (mut io, clock) = sim();
        io.listener_map_wake();
        let past = clock.now();
        clock.advance(Duration::from_secs(1));
        assert!(matches!(io.next_event(Some(past)), Ok(Some(GuestIoEvent::ListenerMapWake))));
        assert!(io.next_event(Some(past)).expect("wait ok").is_none(), "delivered once");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_scripted_wait_failure_is_returned_once() {
        let (mut io, clock) = sim();
        io.fail_event_wait(ErrorKind::Interrupted);
        assert_eq!(io.next_event(None).expect_err("scripted").kind(), ErrorKind::Interrupted);
        io.listener_map_wake();
        let _ = clock;
        assert!(matches!(io.next_event(None), Ok(Some(GuestIoEvent::ListenerMapWake))));
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn host_connect_completes_only_when_released_and_session_ops_need_connected() {
        let (mut io, _clock) = sim();
        let conn = io.start_connect_host(HostChannel::Control).expect("connect starts");
        assert_eq!(
            io.pending_connects(),
            vec![(conn.id(), SimConnectTarget::Host(HostChannel::Control))]
        );
        assert_eq!(
            io.write_host(&conn, &[0u8; CONTROL_OPEN_LEN]).expect_err("before Connected").kind(),
            ErrorKind::NotConnected
        );
        assert_eq!(
            io.start_read_host(&conn, GuestRecord::ControlMessage)
                .expect_err("before Connected")
                .kind(),
            ErrorKind::NotConnected
        );
        let released = io.complete_host_connect(HostChannel::Control, Ok(()));
        assert_eq!(released, conn.id());
        // Released but not yet delivered: still NotConnected.
        assert_eq!(
            io.write_host(&conn, &[0u8; CONTROL_OPEN_LEN]).expect_err("not delivered").kind(),
            ErrorKind::NotConnected
        );
        match io.next_event(None) {
            Ok(Some(GuestIoEvent::Connected { socket, result: Ok(()) })) => {
                assert_eq!(socket, conn.id());
            }
            other => panic!("expected Connected, got {other:?}"),
        }
        assert_eq!(io.session_socket(), Some(conn.id()));
        io.write_host(&conn, &[7u8; CONTROL_OPEN_LEN]).expect("whole ControlOpen");
        assert_eq!(io.control_opens(), vec![[7u8; CONTROL_OPEN_LEN]]);
        io.start_read_host(&conn, GuestRecord::ControlMessage).expect("read starts");
        assert!(io.pending_connects().is_empty());
        assert!(!io.guest_socket_observation().session_ended);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_refused_control_connect_opens_no_session() {
        let (mut io, _clock) = sim();
        let conn = io.start_connect_host(HostChannel::Control).expect("connect starts");
        io.complete_host_connect(HostChannel::Control, Err(ErrorKind::ConnectionRefused));
        assert!(matches!(
            io.next_event(None),
            Ok(Some(GuestIoEvent::Connected { result: Err(ErrorKind::ConnectionRefused), .. }))
        ));
        assert_eq!(io.session_socket(), None);
        assert_eq!(
            io.write_host(&conn, &[0u8; 8]).expect_err("no session").kind(),
            ErrorKind::NotConnected
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn session_end_completes_a_pending_read_at_eof_and_ends_the_session() {
        let (mut io, _clock) = sim();
        let conn = io.start_connect_host(HostChannel::Control).expect("connect");
        io.complete_host_connect(HostChannel::Control, Ok(()));
        let _ = io.next_event(None);
        io.start_read_host(&conn, GuestRecord::ControlMessage).expect("read");
        io.end_session(SimSessionEnd::Eof);
        match io.next_event(None) {
            Ok(Some(GuestIoEvent::Read { socket, result: Ok(bytes) })) => {
                assert_eq!(socket, conn.id());
                assert!(bytes.is_empty(), "EOF before any byte");
            }
            other => panic!("expected Read EOF, got {other:?}"),
        }
        assert!(io.guest_socket_observation().session_ended);
        assert_eq!(
            io.write_host(&conn, &[0u8; 8]).expect_err("ended").kind(),
            ErrorKind::BrokenPipe
        );
        io.close(conn.into());
        assert_eq!(io.session_socket(), None);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn session_end_with_no_read_pending_reports_one_hangup() {
        let (mut io, _clock) = sim();
        let conn = io.start_connect_host(HostChannel::Control).expect("connect");
        io.complete_host_connect(HostChannel::Control, Ok(()));
        let _ = io.next_event(None);
        io.end_session(SimSessionEnd::Reset);
        match io.next_event(None) {
            Ok(Some(GuestIoEvent::Hangup {
                socket,
                hangup: GuestHangup::Error(ErrorKind::ConnectionReset),
            })) => {
                assert_eq!(socket, conn.id());
            }
            other => panic!("expected Hangup, got {other:?}"),
        }
        assert_eq!(io.deliverable(), 0);
        io.abort(conn.into());
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_hangup_is_reported_once_per_socket_per_kind_and_dropped_when_the_socket_ends() {
        let (mut io, _clock) = sim();
        let cookie = sim_cookie(9);
        io.app_connect(cookie, dest(1), true);
        let child = match io.next_event(None) {
            Ok(Some(GuestIoEvent::OutboundCaptured { child, cookie: c, parked: true, .. })) => {
                assert_eq!(c, cookie);
                child
            }
            other => panic!("expected OutboundCaptured, got {other:?}"),
        };
        io.app_end(SimAppTarget::Captured(cookie), SimAppEnd::HalfClose);
        io.app_end(SimAppTarget::Captured(cookie), SimAppEnd::HalfClose);
        assert_eq!(io.deliverable(), 1, "level-triggered: one ReadHangup");
        io.app_end(SimAppTarget::Captured(cookie), SimAppEnd::Reset);
        assert_eq!(io.deliverable(), 2, "a different kind is reported once more");
        io.abort(child.into());
        assert_eq!(io.deliverable(), 0, "an ended socket reports nothing");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[should_panic(expected = "another seam instance")]
    fn a_handle_of_another_seam_instance_is_unreachable() {
        let (mut a, _ca) = sim();
        let (mut b, _cb) = sim();
        let conn = a.start_connect_host(HostChannel::Control).expect("connect");
        let _ = b.start_read_host(&conn, GuestRecord::ControlMessage);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn consuming_install_of_unconnected_legs_fails_and_aborts_both() {
        let (mut io, _clock) = sim();
        let t = io.start_connect_host(HostChannel::FlowStream).expect("T_g");
        let q = io.start_connect_host(HostChannel::FlowSeqpacket).expect("Q_g");
        let err = io.install_slot_legs(slot(0), t, q).expect_err("not connected");
        assert_eq!(err.kind(), ErrorKind::NotConnected);
        assert!(io.pending_connects().is_empty(), "both consumed sockets aborted");
        assert!(io.effects().iter().all(|e| !matches!(e, SimGuestEffect::InstallSlotLegs { .. })));
        assert!(!io.slot_state(slot(0)).legs_installed);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn the_kernel_claims_one_slot_per_socket_destination_lowest_first() {
        let (io, _clock) = sim();
        let k = sim_cookie(42);
        assert_eq!(io.app_send(k, dest(1)), Ok(slot(0)));
        assert_eq!(io.app_send(k, dest(2)), Ok(slot(1)));
        assert_eq!(io.app_send(k, dest(1)), Ok(slot(0)), "an owned destination reuses its slot");
        let obs = io.guest_socket_observation();
        assert_eq!(
            obs.readiness,
            BTreeSet::from([(slot(0), sim_generation(0)), (slot(1), sim_generation(0))])
        );
        assert_eq!(obs.parked.get(&slot(0)), Some(&BTreeSet::from([(k, dest(1))])));
        assert_eq!(io.slot_state(slot(0)).parked_frames, 2);
        assert!(!io.slot_state(slot(0)).in_pool);
        assert!(io.slot_state(slot(2)).in_pool);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn the_kernel_refuses_a_new_destination_when_every_slot_is_owned() {
        let (io, _clock) = sim();
        let k = sim_cookie(1);
        for n in 0..GUEST_UDP_SLOTS {
            let [hi, lo] = n.to_be_bytes();
            let d = SocketAddrV4::new(Ipv4Addr::new(10, 1, hi, lo), 9);
            assert_eq!(io.app_send(k, d), Ok(slot(n)));
        }
        assert_eq!(io.app_send(k, dest(200)), Err(ErrorKind::PermissionDenied));
        assert_eq!(io.exhausted_sends(), 1);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_is_released_by_slot_and_generation() {
        let (mut io, _clock) = sim();
        let k = sim_cookie(3);
        let s = io.app_send(k, dest(1)).expect("claims");
        io.release_slot_ready(s, sim_generation(0));
        match io.next_event(None) {
            Ok(Some(GuestIoEvent::SlotReady { slot: got, generation })) => {
                assert_eq!((got, generation), (s, sim_generation(0)));
            }
            other => panic!("expected SlotReady, got {other:?}"),
        }
        assert!(io.guest_socket_observation().readiness.is_empty());
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn ending_an_association_recreates_cells_and_keeps_the_slot_owned() {
        let (mut io, _clock) = sim();
        let k = sim_cookie(5);
        let s = io.app_send(k, dest(1)).expect("claims");
        io.app_send(k, dest(1)).expect("parks again");
        let end = io.end_association(s, None);
        assert_eq!(end, SlotEnd::Recreated(sim_generation(1)));
        let state = io.slot_state(s);
        assert_eq!(state.parked_frames, 0, "parked frames discarded");
        assert_eq!(state.owner, Some((k, dest(1))), "U-2: the slot stays owned");
        let obs = io.guest_socket_observation();
        assert!(obs.parked.is_empty());
        assert!(
            obs.readiness.contains(&(s, sim_generation(0))),
            "the old-generation event stays queued (stale)"
        );
        io.release_slot(s);
        assert!(io.slot_state(s).in_pool);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn a_cell_creation_failure_quarantines_until_the_retry_succeeds() {
        let (mut io, _clock) = sim();
        let k = sim_cookie(6);
        let s = io.app_send(k, dest(1)).expect("claims");
        io.fail_cell_creation(s);
        assert_eq!(io.end_association(s, None), SlotEnd::Quarantined);
        let state = io.slot_state(s);
        assert!(state.quarantined && !state.in_pool && state.owner.is_none());
        assert_eq!(io.app_send(k, dest(1)), Ok(slot(1)), "the next send takes a free slot");
        io.fail_next(SimGuestOp::RetryQuarantined, ErrorKind::OutOfMemory);
        assert_eq!(io.retry_quarantined(s).expect_err("scripted").kind(), ErrorKind::OutOfMemory);
        assert!(io.slot_state(s).quarantined);
        assert_eq!(io.retry_quarantined(s).expect("retry"), sim_generation(1));
        assert!(io.slot_state(s).in_pool);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[should_panic(expected = "has not ended")]
    fn releasing_a_slot_with_installed_legs_violates_the_seam_contract() {
        let (mut io, _clock) = sim();
        let s = io.app_send(sim_cookie(7), dest(1)).expect("claims");
        let t = io.start_connect_host(HostChannel::FlowStream).expect("T_g");
        let q = io.start_connect_host(HostChannel::FlowSeqpacket).expect("Q_g");
        io.complete_connect(t.id(), Ok(()));
        io.complete_connect(q.id(), Ok(()));
        let _ = io.next_event(None);
        let _ = io.next_event(None);
        let _legs = io.install_slot_legs(s, t, q).expect("installs");
        io.release_slot(s);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn release_reports_are_held_until_released_and_a_lost_report_is_not_queued() {
        let (mut io, _clock) = sim();
        let (a, b) = (sim_cookie(11), sim_cookie(12));
        io.app_send(a, dest(1)).expect("claims");
        io.app_send(b, dest(1)).expect("claims");
        io.app_close_socket(a, true);
        io.app_close_socket(b, false);
        assert_eq!(io.guest_socket_observation().release_reports, BTreeSet::from([a]));
        assert_eq!(io.deliverable(), 0, "held until released");
        io.release_socket_report(a);
        assert!(
            matches!(io.next_event(None), Ok(Some(GuestIoEvent::SocketReleased { cookie })) if cookie == a)
        );
        let live = io.live_cookies(&BTreeSet::from([a, b])).expect("audit read");
        assert!(live.is_empty(), "both sockets are gone");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn guest_owner_death_resets_application_legs_and_ends_the_session() {
        let (mut io, _clock) = sim();
        let conn = io.start_connect_host(HostChannel::Control).expect("connect");
        io.complete_host_connect(HostChannel::Control, Ok(()));
        let _ = io.next_event(None);
        let cookie = sim_cookie(4);
        io.app_connect(cookie, dest(1), true);
        let _child = io.next_event(None);
        io.guest_owner_dies();
        assert!(io.guest_socket_observation().session_ended);
        assert_eq!(io.session_socket(), None);
        assert!(io.pending_connects().is_empty());
        drop(conn);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn clones_share_one_state() {
        let (io, _clock) = sim();
        let driver = io.clone();
        driver.listener_map_wake();
        assert_eq!(io.deliverable(), 1);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn scripted_failures_apply_to_the_next_call_of_that_op_only() {
        let (mut io, _clock) = sim();
        io.fail_next(SimGuestOp::StartConnectHost, ErrorKind::ConnectionRefused);
        assert_eq!(
            io.start_connect_host(HostChannel::Control).expect_err("scripted").kind(),
            ErrorKind::ConnectionRefused
        );
        assert!(io.pending_connects().is_empty());
        let _conn = io.start_connect_host(HostChannel::Control).expect("next call succeeds");
        assert_eq!(io.pending_connects().len(), 1);
    }
}
