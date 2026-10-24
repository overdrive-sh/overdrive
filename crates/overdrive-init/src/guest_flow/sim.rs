//! Simulation seam for the guest flow owner (DG-5, QPROJ-7): [`SimGuestOwnerIo`]
//! and [`SimGuestClock`]. `#[cfg(test)]` only.
//!
//! A test fixture, not a scaffold: fully functional, and it enforces the seam
//! contract the real adapter has (`GuestOwnerIo` rustdoc):
//!
//! - a handle of another seam instance is `unreachable!`;
//! - an operation on a `HostConn` / `AppConn` before its `Connected` event was
//!   delivered fails `NotConnected`;
//! - attaches are all-or-nothing: the guest forwarder holds both halves, or it
//!   refused and the halves come back unattached;
//! - writes are whole records (a control-session write is a 24-byte
//!   `ControlOpen` or an 8-byte control message, a request write is 32 bytes);
//! - every event is delivered once; a `Hangup` is level-triggered and reported
//!   once per socket per kind while the socket is unattached and watched (the
//!   session `HostConn`, a flow `HostConn` or an `InboundConn` before its
//!   attach, a `CapturedConn` before `Paired`), and never after the socket
//!   ends or is attached.
//!
//! **The guest's kernel (decision B-1).** Every guest socket a forwarder pair
//! can hold is an `overdrive-sim` [`SimSocket`] of its kind (vsock STREAM, TCP,
//! UDP), and the guest's one forwarder context is an `overdrive-sim`
//! [`SimFlowForwarder`] opened on its own [`SimForwarderDevice`] — the named
//! arbiter of the guest forwarder's kernel facts (QPROJ-8: per-direction ends
//! and target shutdowns, the class of each vsock transition, the queued end
//! records, the live pairs, the context table). The seam's `attach_*`,
//! `abort_forward`, `pair_state` and `inventory` call that context; its end
//! records reach the owner as `ForwardEnded` events once the driver releases
//! them ([`SimGuestOwnerIo::release_forward_end`], `take_end` on the context's
//! readiness). The far sides of a flow's halves (the guest application behind
//! I / C / S, the host behind V_g) are scripted on the halves' [`SimSocket`]s
//! ([`SimGuestOwnerIo::flow_socket`], [`SimGuestOwnerIo::slot_socket`]); the
//! forwarder's runs are stepped on the context
//! ([`SimGuestOwnerIo::run_forwarder`]). Facts of unattached sockets (accept
//! queues, readiness, hangups, connects, reads, the slot pool's readiness and
//! release reports) are this seam's own: the QPROJ-7 arbiter surface
//! [`SimGuestSocketObservation`].
//!
//! **Time and quiescence (DG-5-Q).** The owner and the sim read the same
//! [`SimGuestClock`]; only the driver advances it. `next_event` returns the
//! next *released* event, or `Ok(None)` when the deadline it is given is at or
//! before the clock's now. A call with nothing released and no deadline passed
//! is a driver defect: a correct driver dispatches only while
//! [`SimGuestOwnerIo::released_undelivered`] or the owner's `next_deadline()`
//! is due, so the sim treats that call as `unreachable!`, never as a signal.
//! It never answers with an `Err` the production seam would not: an `Err` from
//! `next_event` is the owner's fatal wait failure
//! (`GuestOwnerFatal::EventWait`), which only `fail_event_wait` scripts.
//!
//! **Held until released.** Every effect the owner starts (a host or
//! application connect, a read) and every guest-kernel fact (a V_g in the
//! accept queue, a slot readiness event, a `sock_release` report, a forwarder
//! end record) is held until the driver releases it through the stepping API
//! below (D-UP-19: the driver chooses the accept order).
//!
//! **Read surface.** [`SimGuestOwnerIo::guest_socket_observation`] is the
//! pinned QPROJ-7 arbiter surface (Tier-3 pairings: V-24, R5-14, V-EV-DGRAM,
//! V-31 item 2c, R5-10); the forwarder's facts are read through
//! [`SimGuestOwnerIo::forwarder_pair_state`],
//! [`SimGuestOwnerIo::forwarder_inventory`] and
//! [`SimGuestOwnerIo::forwarder_contexts`] (QPROJ-8, paired with V-EV-END,
//! V-EV-CTL, V-EV-PAIR, V-EV-DGRAM and the module's Tier-3 suite). Every other
//! read below (`effects`, `outbox`, `slot_state`, `pending_connects`, …) is
//! test support for acceptance oracles and stimulus bookkeeping; it never
//! feeds a conformance `State` projection.
//!
//! **Open seam gap (DISTILL blocker, routed to DESIGN).** DG-5 pins
//! `attach_slot(..) -> Result<ForwardId, GuestAttachRefused>`, but
//! `GuestAttachRefused` names an `outer` half and a slot socket has no handle
//! type: a refused slot attach cannot be represented. The sim panics with that
//! message if the forwarder refuses a slot attach.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::significant_drop_tightening,
    clippy::struct_excessive_bools,
    clippy::option_if_let_else,
    clippy::too_many_lines,
    reason = "test fixture: a seam-contract violation is a driver or owner defect and panics"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwarderError, ForwarderInventory,
    PairMode, PairSide, PairState,
};
use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, FLOW_REQUEST_LEN, FlowId, FlowRequest,
};
use overdrive_core::guest_transport::{GUEST_UDP_SLOT_BASE, GUEST_UDP_SLOTS};
use overdrive_sim::adapters::{
    SimContextState, SimFarEnd, SimFlowForwarder, SimForwarderDevice, SimLocalRead, SimShutdown,
    SimSocket, SimSocketKind, SimSocketSetup,
};

use super::{
    AppConn, CapturedConn, GuestAttachRefused, GuestClock, GuestHangup, GuestIoEvent,
    GuestListener, GuestOwnedSocket, GuestOwnerIo, GuestPair, GuestPairSocket, GuestRecord,
    GuestSlotAttachRefused, Handle, HostChannel, HostConn, InboundConn, SeamInstance, SocketCookie,
    SocketId,
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
    /// A clock at an arbitrary fixed origin.
    pub(crate) fn new() -> Self {
        Self { now: Mutex::new(Instant::now()) }
    }

    /// Advances the clock by `by`.
    pub(crate) fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }

    /// Advances the clock to exactly `at`; never backwards.
    pub(crate) fn advance_to(&self, at: Instant) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(at >= *now, "driver defect: the guest clock never goes backwards");
        *now = at;
    }
}

impl GuestClock for SimGuestClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// ---------------------------------------------------------------------------
// The pinned QPROJ-7 read surface and test-support vocabulary
// ---------------------------------------------------------------------------

/// The first terminal event a far end (the guest application) saw on a flow's
/// outer leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FarEndEvent {
    /// A clean end of stream.
    Eof,
    /// A reset.
    Reset,
}

impl From<SimFarEnd> for FarEndEvent {
    fn from(end: SimFarEnd) -> Self {
        match end {
            SimFarEnd::Eof => Self::Eof,
            SimFarEnd::Reset => Self::Reset,
        }
    }
}

/// QPROJ-7: the guest socket facts the owner does not hold (field names and
/// meanings are the contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimGuestSocketObservation {
    /// V_g in the guest accept queue.
    pub(crate) queued_inbound: BTreeSet<FlowId>,
    /// V_g torn down there (K-A4).
    pub(crate) reset_in_queue: BTreeSet<FlowId>,
    /// Undispatched readiness events, by slot.
    pub(crate) readiness: BTreeSet<SocketAddrV4>,
    /// Datagrams queued in each slot socket, by origin.
    pub(crate) parked: BTreeMap<SocketAddrV4, BTreeSet<(SocketCookie, SocketAddrV4)>>,
    /// Queued `sock_release` reports.
    pub(crate) release_reports: BTreeSet<SocketCookie>,
    /// Each guest far end's first terminal event.
    pub(crate) far_end: BTreeMap<FlowId, FarEndEvent>,
}

/// A seam operation whose next call [`SimGuestOwnerIo::fail_next`] scripts to
/// fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SimGuestOp {
    StartConnectHost,
    StartReadHost,
    StartReadInbound,
    WriteHost,
    InboundEnded,
    WriteRequest,
    StartConnectApp,
    ClearLinger,
    SlotOwner,
    WriteSlotRequest,
    SlotCounters,
    LiveCookies,
}

/// What an in-flight connect targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimConnectTarget {
    /// A host channel.
    Host(HostChannel),
    /// C toward the application: bound to `client`, to `destination`, for the
    /// dial `flow` whose request the owner had just checked.
    App {
        /// The bind address.
        client: SocketAddrV4,
        /// `workload_addr:listen_port`.
        destination: SocketAddrV4,
        /// The dial.
        flow: Option<FlowId>,
    },
}

/// How the host ends the control session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimSessionEnd {
    /// EOF / `POLLRDHUP` (the only shape K-A5 admits for a host close).
    Eof,
    /// An I/O error on the session read (the guest's own read fails; K-A5 never
    /// produces it from a host close, but a read can fail).
    Reset,
}

/// A half of a flow's guest pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimHalf {
    /// V_g.
    Vsock,
    /// I (TcpConnect) or C (TcpAccept).
    Outer,
}

/// Test-support view of one UDP slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimSlotState {
    /// The slot's (application socket, destination) owner.
    pub(crate) owner: Option<(SocketCookie, SocketAddrV4)>,
    /// The association the owner wrote a request for, if any.
    pub(crate) flow: Option<FlowId>,
    /// The association's datagram pair.
    pub(crate) attached: Option<ForwardId>,
    /// The slot socket is connected to its application socket's address.
    pub(crate) connected: bool,
    /// Datagrams discarded unread by the owner's end-of-association order.
    pub(crate) discarded: usize,
    /// End-of-association orders the owner ran on this slot.
    pub(crate) ended: usize,
}

/// One seam call the owner made (test support: ordering oracles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SimGuestEffect {
    ConnectHost { socket: SocketId, channel: HostChannel },
    ReadHost { socket: SocketId, record: GuestRecord },
    ReadInbound { socket: SocketId, flow: FlowId },
    WriteControlOpen { socket: SocketId },
    WriteControl { socket: SocketId, message: ControlMessage },
    InboundEnded { socket: SocketId, ended: bool },
    WriteRequest { socket: SocketId, request: FlowRequest },
    AttachOutbound { flow: Option<FlowId>, accepted: Option<ForwardId> },
    StartConnectApp { socket: SocketId, client: SocketAddrV4, destination: SocketAddrV4 },
    AttachInbound { flow: Option<FlowId>, accepted: Option<ForwardId> },
    AbortForward { id: ForwardId },
    PairState { id: ForwardId },
    Inventory,
    ClearLinger { socket: SocketId },
    Abort { socket: SocketId, flow: Option<FlowId> },
    Close { socket: SocketId, flow: Option<FlowId> },
    SlotOwner { slot: SocketAddrV4 },
    WriteSlotRequest { socket: SocketId, request: FlowRequest },
    AttachSlot { slot: SocketAddrV4, accepted: Option<ForwardId> },
    EndAssociation { slot: SocketAddrV4 },
    ReleaseSlot { slot: SocketAddrV4 },
    SlotCounters { slot: SocketAddrV4 },
    LiveCookies,
}

/// A test-support cookie constructor (the owner's `SocketCookie::new` is a
/// DELIVER scaffold; the sim names cookies directly).
pub(crate) fn sim_cookie(raw: u64) -> SocketCookie {
    SocketCookie(NonZeroU64::new(raw).expect("sim cookies are non-zero"))
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Host,
    Inbound,
    Captured,
    App,
    Pair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connect {
    None,
    InFlight,
    /// The completion is released, not yet delivered.
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
    /// The kernel socket (a C has none until its connect completed `Ok`).
    kernel: Option<SimSocket>,
    channel: Option<HostChannel>,
    connect: Connect,
    app_target: Option<(SocketAddrV4, SocketAddrV4)>,
    read_pending: Option<GuestRecord>,
    flow: Option<FlowId>,
    /// An outer leg (I or C): carries `SO_LINGER{1,0}` from creation.
    outer: bool,
    linger: bool,
    /// The host closed this vsock (V_h closed) while unattached.
    peer_ended: bool,
    /// Terminal conditions present on the socket (level).
    conditions: BTreeMap<HangupKey, GuestHangup>,
    reported: BTreeSet<HangupKey>,
    attached: Option<ForwardId>,
    slot: Option<SocketAddrV4>,
    /// A guest-opened flow V_g's intake child I (linked at the connect the
    /// owner issues in the dispatch of the child's `OutboundCaptured`).
    partner: Option<SocketId>,
    closed: bool,
}

impl Sock {
    fn new(stage: Stage, kernel: Option<SimSocket>) -> Self {
        Self {
            stage,
            kernel,
            channel: None,
            connect: Connect::None,
            app_target: None,
            read_pending: None,
            flow: None,
            outer: false,
            linger: false,
            peer_ended: false,
            conditions: BTreeMap::new(),
            reported: BTreeSet::new(),
            attached: None,
            slot: None,
            partner: None,
            closed: false,
        }
    }

    const fn connected(&self) -> bool {
        matches!(self.connect, Connect::Delivered(Ok(())))
    }

    /// Unattached and of a watched kind (DG-5-E).
    const fn watched(&self) -> bool {
        matches!(self.stage, Stage::Host | Stage::Inbound | Stage::Captured)
    }

    fn kernel(&self) -> &SimSocket {
        self.kernel.as_ref().unwrap_or_else(|| unreachable!("the socket has no kernel socket yet"))
    }
}

#[derive(Debug)]
struct InboundRec {
    /// The request the host wrote, `None` when it wrote no byte.
    request: Option<FlowRequest>,
    socket: Option<SocketId>,
    /// The host closed V_h (after writing the request).
    host_closed: bool,
}

#[derive(Debug)]
struct SlotRec {
    /// The slot socket S of the current association (renewed by each end of
    /// association: the disconnect, then the next association's connect).
    kernel: SimSocket,
    owner: Option<(SocketCookie, SocketAddrV4)>,
    flow: Option<FlowId>,
    vsock: Option<SocketId>,
    /// Most recent association V_g, retained for QPROJ-8 after the
    /// association leaves the live slot record (D-5).
    last_vsock: Option<SocketId>,
    attached: Option<ForwardId>,
    /// The origin of each datagram the application queued in S, oldest first
    /// (the kernel queue length is S's own `queued()`).
    origins: VecDeque<(SocketCookie, SocketAddrV4)>,
    discarded: usize,
    ended: usize,
    connected: bool,
}

/// One pair the forwarder accepted.
#[derive(Debug, Clone, Copy)]
struct PairRec {
    flow: Option<FlowId>,
    vsock: Option<SocketId>,
    outer: Option<SocketId>,
    slot: Option<SocketAddrV4>,
    /// The half (`A` or `B`) V_g is in the pair.
    vsock_side: PairSide,
}

#[derive(Debug)]
struct SimState {
    seam: SeamInstance,
    workload_addr: Ipv4Addr,
    next_socket: u64,
    sockets: BTreeMap<SocketId, Sock>,
    deliverable: VecDeque<GuestIoEvent>,
    effects: Vec<SimGuestEffect>,
    failures: BTreeMap<SimGuestOp, VecDeque<ErrorKind>>,
    event_wait_failure: Option<ErrorKind>,
    session: Option<SocketId>,
    session_ended: bool,
    /// The host has ended the session and the guest has not observed it yet.
    host_closed: Option<SimSessionEnd>,
    host_messages: VecDeque<ControlMessage>,
    control_opens: Vec<[u8; CONTROL_OPEN_LEN]>,
    outbox: Vec<ControlMessage>,
    inbound: BTreeMap<FlowId, InboundRec>,
    reset_in_queue: BTreeSet<FlowId>,
    /// The dial whose request the owner checked last (`inbound_ended` false):
    /// the next `start_connect_app` is its C (the per-kind order).
    last_checked_dial: Option<FlowId>,
    /// The intake child / slot of the event dispatched last: the owner's
    /// `start_connect_host(FlowStream)` in that dispatch opens its V_g (the
    /// per-kind orders).
    last_capture: Option<SocketId>,
    last_slot: Option<SocketAddrV4>,
    pairs: BTreeMap<ForwardId, PairRec>,
    /// Far-end events the seam recorded at the guest's own closes (the first
    /// one per flow); the kernel's `peer_end` of attached halves is merged in
    /// at observation.
    far_end: BTreeMap<FlowId, FarEndEvent>,
    vsock_ended: BTreeSet<FlowId>,
    slots: BTreeMap<SocketAddrV4, SlotRec>,
    readiness: BTreeSet<SocketAddrV4>,
    release_reports: BTreeSet<SocketCookie>,
    live_cookies: BTreeSet<SocketCookie>,
    exhausted_sends: u64,
    requests: Vec<FlowRequest>,
    /// End records taken from the context and not yet delivered.
    taken_ends: VecDeque<ForwardEnd>,
}

fn lock<T>(state: &Mutex<T>) -> MutexGuard<'_, T> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SimState {
    fn handle(&mut self) -> Handle {
        let id = SocketId(self.next_socket);
        self.next_socket += 1;
        Handle { seam: self.seam, id }
    }

    const fn same(&self, id: SocketId) -> Handle {
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

    fn require_connected(&self, id: SocketId) -> io::Result<()> {
        if self.sockets.get(&id).is_some_and(Sock::connected) {
            Ok(())
        } else {
            Err(io::Error::from(ErrorKind::NotConnected))
        }
    }

    /// Adds a terminal condition; reported now if the socket is watched and
    /// this kind was not reported yet.
    fn condition(&mut self, id: SocketId, hangup: GuestHangup) {
        let sock = self.sock(id);
        if sock.closed {
            return;
        }
        sock.conditions.entry(hangup_key(hangup)).or_insert(hangup);
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

    /// A socket stops being watched: its undelivered hangups are dropped.
    fn unwatch(&mut self, id: SocketId) {
        self.deliverable
            .retain(|e| !matches!(e, GuestIoEvent::Hangup { socket, .. } if *socket == id));
    }

    /// The far end of `flow`'s outer leg saw `event` (first one wins).
    fn far(&mut self, flow: Option<FlowId>, event: FarEndEvent) {
        if let Some(flow) = flow {
            self.far_end.entry(flow).or_insert(event);
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
        let (flow, outer, linger, stage, channel, kernel) =
            (sock.flow, sock.outer, sock.linger, sock.stage, sock.channel, sock.kernel.clone());
        if let Some(kernel) = &kernel {
            kernel.close_from_owner(reset || linger);
        }
        // The SimSocket is the QPROJ-8 arbiter; its first far-end event wins.
        let kernel_end = if outer { kernel.as_ref().and_then(SimSocket::peer_end) } else { None };
        self.unwatch(id);
        if session == Some(id) {
            self.session = None;
            self.session_ended = true;
            self.host_messages.clear();
        }
        if outer {
            let event = match kernel_end {
                Some(end) => end.into(),
                None if reset || linger => FarEndEvent::Reset,
                None => FarEndEvent::Eof,
            };
            self.far(flow, event);
        }
        let flow_vsock = matches!(stage, Stage::Inbound)
            || (matches!(stage, Stage::Host | Stage::Pair)
                && channel == Some(HostChannel::FlowStream))
            || matches!(stage, Stage::Pair) && !outer;
        if flow_vsock && let Some(flow) = flow {
            self.vsock_ended.insert(flow);
        }
    }

    fn slot_mut(&mut self, slot: SocketAddrV4) -> &mut SlotRec {
        self.slots.get_mut(&slot).unwrap_or_else(|| unreachable!("{slot} is not a guest UDP slot"))
    }

    fn free_slot(&self) -> Option<SocketAddrV4> {
        self.slots.iter().find(|(_, r)| r.owner.is_none()).map(|(s, _)| *s)
    }

    /// The (open) socket of `flow` matching `pred`.
    fn flow_socket(&self, flow: FlowId, pred: impl Fn(&Sock) -> bool) -> Option<SocketId> {
        self.sockets
            .iter()
            .filter(|(_, s)| !s.closed && s.flow == Some(flow) && pred(s))
            .map(|(id, _)| *id)
            .next_back()
    }

    fn pair_of(&self, flow: FlowId) -> Option<ForwardId> {
        self.pairs.iter().filter(|(_, p)| p.flow == Some(flow)).map(|(id, _)| *id).next_back()
    }
}

// ---------------------------------------------------------------------------
// The guest's forwarder context
// ---------------------------------------------------------------------------

/// The guest's one forwarder context, until the guest owner's death releases
/// it (the device's context table keeps the record).
#[derive(Debug)]
struct GuestContext {
    device: SimForwarderDevice,
    ctx: Mutex<Option<SimFlowForwarder>>,
}

impl GuestContext {
    fn open() -> Self {
        let device = SimForwarderDevice::new();
        let ctx = device.open();
        Self { device, ctx: Mutex::new(Some(ctx)) }
    }

    fn with<R>(&self, f: impl FnOnce(&SimFlowForwarder) -> R) -> R {
        let ctx = lock(&self.ctx);
        f(ctx.as_ref().expect("driver defect: the guest forwarder context was released"))
    }

    fn release(&self) {
        let ctx = lock(&self.ctx).take();
        if let Some(ctx) = ctx {
            ctx.release();
        }
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
    fwd: Arc<GuestContext>,
}

impl SimGuestOwnerIo {
    /// A guest with `GUEST_UDP_SLOTS` slot sockets bound on
    /// `workload_addr:GUEST_UDP_SLOT_BASE + n` (all free), one open forwarder
    /// context (the setup's `open_forwarder`), and no session. Reads `clock` —
    /// the instance the owner reads.
    pub(crate) fn new(workload_addr: Ipv4Addr, clock: Arc<SimGuestClock>) -> Self {
        let slots = (0..GUEST_UDP_SLOTS)
            .map(|n| {
                let slot = SocketAddrV4::new(workload_addr, GUEST_UDP_SLOT_BASE + n);
                let rec = SlotRec {
                    kernel: SimSocket::udp(),
                    owner: None,
                    flow: None,
                    vsock: None,
                    last_vsock: None,
                    attached: None,
                    origins: VecDeque::new(),
                    discarded: 0,
                    ended: 0,
                    connected: false,
                };
                (slot, rec)
            })
            .collect();
        let state = SimState {
            seam: SeamInstance::next(),
            workload_addr,
            next_socket: 1,
            sockets: BTreeMap::new(),
            deliverable: VecDeque::new(),
            effects: Vec::new(),
            failures: BTreeMap::new(),
            event_wait_failure: None,
            session: None,
            session_ended: false,
            host_closed: None,
            host_messages: VecDeque::new(),
            control_opens: Vec::new(),
            outbox: Vec::new(),
            inbound: BTreeMap::new(),
            reset_in_queue: BTreeSet::new(),
            last_checked_dial: None,
            last_capture: None,
            last_slot: None,
            pairs: BTreeMap::new(),
            far_end: BTreeMap::new(),
            vsock_ended: BTreeSet::new(),
            slots,
            readiness: BTreeSet::new(),
            release_reports: BTreeSet::new(),
            live_cookies: BTreeSet::new(),
            exhausted_sends: 0,
            requests: Vec::new(),
            taken_ends: VecDeque::new(),
        };
        Self { state: Arc::new(Mutex::new(state)), clock, fwd: Arc::new(GuestContext::open()) }
    }

    fn st(&self) -> MutexGuard<'_, SimState> {
        lock(&self.state)
    }

    /// The clock this seam reads.
    pub(crate) fn clock(&self) -> Arc<SimGuestClock> {
        Arc::clone(&self.clock)
    }

    // ----- QPROJ-7 / QPROJ-8 read surfaces --------------------------------

    /// QPROJ-7 arbiter: the guest's socket facts. `parked` lists, per slot,
    /// the origins of the datagrams the slot socket's kernel queue still holds
    /// (the newest `queued()` of the origins the seam recorded); `far_end`
    /// merges the far-end events the seam recorded at the guest's closes with
    /// the kernel's `peer_end` of every open outer half.
    pub(crate) fn guest_socket_observation(&self) -> SimGuestSocketObservation {
        let st = self.st();
        let mut far_end = st.far_end.clone();
        for sock in st.sockets.values().filter(|s| s.outer && !s.closed) {
            if let (Some(flow), Some(end)) =
                (sock.flow, sock.kernel.as_ref().and_then(SimSocket::peer_end))
            {
                far_end.entry(flow).or_insert_with(|| end.into());
            }
        }
        let parked = st
            .slots
            .iter()
            .filter_map(|(s, r)| {
                let queued = r.kernel.queued();
                let tags: BTreeSet<_> = r.origins.iter().rev().take(queued).copied().collect();
                (!tags.is_empty()).then_some((*s, tags))
            })
            .collect();
        SimGuestSocketObservation {
            queued_inbound: st
                .inbound
                .iter()
                .filter(|(_, rec)| rec.socket.is_none())
                .map(|(flow, _)| *flow)
                .collect(),
            reset_in_queue: st.reset_in_queue.clone(),
            readiness: st.readiness.clone(),
            parked,
            release_reports: st.release_reports.clone(),
            far_end,
        }
    }

    /// QPROJ-8 arbiter: `pair_state` of the guest's context (the port's own
    /// read). `None` once the context is released.
    pub(crate) fn forwarder_pair_state(&self, id: ForwardId) -> Option<PairState> {
        let ctx = lock(&self.fwd.ctx);
        ctx.as_ref().and_then(|c| c.pair_state(id).expect("the sim context is never in error"))
    }

    /// QPROJ-8 arbiter: `inventory` of the guest's context; `None` once the
    /// context is released.
    pub(crate) fn forwarder_inventory(&self) -> Option<ForwarderInventory> {
        let ctx = lock(&self.fwd.ctx);
        ctx.as_ref().map(|c| c.inventory().expect("the sim context is never in error"))
    }

    /// QPROJ-8 arbiter: the guest device's context table (open or released,
    /// and what each context still holds).
    pub(crate) fn forwarder_contexts(&self) -> Vec<SimContextState> {
        self.fwd.device.contexts()
    }

    /// Steps the guest forwarder's queued runs until none is queued (the
    /// kernel's work items; never run behind the test's back). Returns how
    /// many ran.
    pub(crate) fn run_forwarder(&self) -> u32 {
        self.fwd.with(SimFlowForwarder::run_until_idle)
    }

    /// Steps one bounded run of the direction of pair `id` whose source is
    /// `source`; `false` when none is queued.
    pub(crate) fn run_forwarder_once(&self, id: ForwardId, source: PairSide) -> bool {
        self.fwd.with(|c| c.run(id, source))
    }

    /// The directions with a queued run, `(id, source half)`.
    pub(crate) fn forwarder_pending_runs(&self) -> Vec<(ForwardId, PairSide)> {
        self.fwd.with(SimFlowForwarder::pending_runs)
    }

    // ----- delivery and quiescence (DG-5-Q) -------------------------------

    /// An event is released and not yet delivered (DG-5-Q).
    pub(crate) fn released_undelivered(&self) -> bool {
        !self.st().deliverable.is_empty()
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

    /// A listener's accept fails (`ListenerFailed`), released at once.
    pub(crate) fn listener_failed(&self, listener: GuestListener, kind: ErrorKind) {
        self.st().deliverable.push_back(GuestIoEvent::ListenerFailed { listener, kind });
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

    /// Releases the completion of `socket`'s connect. A C whose connect
    /// completes `Ok` gets its connected kernel TCP socket here (the
    /// application accepted), unless [`Self::accept_app_connect`] already
    /// made it.
    pub(crate) fn complete_connect(&self, socket: SocketId, result: Result<(), ErrorKind>) {
        let mut st = self.st();
        let sock = st.sock(socket);
        assert!(
            !sock.closed && sock.connect == Connect::InFlight,
            "driver defect: {socket:?} has no connect in flight"
        );
        sock.connect = Connect::Released;
        if sock.stage == Stage::App && result.is_ok() && sock.kernel.is_none() {
            sock.kernel = Some(SimSocket::tcp());
        }
        st.deliverable.push_back(GuestIoEvent::Connected { socket, result });
    }

    /// The application accepts the dial `flow`'s C: its kernel TCP socket is
    /// connected (the application may write, half-close or reset at once),
    /// and the completion stays held — the guest owner learns of it only when
    /// [`Self::complete_app_connect`] releases it. Returns C's kernel socket.
    pub(crate) fn accept_app_connect(&self, flow: FlowId) -> SimSocket {
        let socket = self
            .pending_connects()
            .into_iter()
            .find(|(_, t)| matches!(t, SimConnectTarget::App { flow: Some(f), .. } if *f == flow))
            .map_or_else(
                || panic!("driver defect: no application connect in flight for {flow:?}"),
                |(id, _)| id,
            );
        let mut st = self.st();
        st.sock(socket).kernel.get_or_insert_with(SimSocket::tcp).clone()
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
            .map_or_else(
                || panic!("driver defect: no {channel:?} connect in flight"),
                |(id, _)| id,
            );
        self.complete_connect(socket, result);
        socket
    }

    /// Releases the in-flight application connect (C) of the dial `flow`.
    pub(crate) fn complete_app_connect(
        &self,
        flow: FlowId,
        result: Result<(), ErrorKind>,
    ) -> SocketId {
        let socket = self
            .pending_connects()
            .into_iter()
            .find(|(_, t)| matches!(t, SimConnectTarget::App { flow: Some(f), .. } if *f == flow))
            .map_or_else(
                || panic!("driver defect: no application connect in flight for {flow:?}"),
                |(id, _)| id,
            );
        self.complete_connect(socket, result);
        socket
    }

    // ----- control session (host side) ------------------------------------------

    /// The live session socket (`Connected` delivered, not ended).
    pub(crate) fn session_socket(&self) -> Option<SocketId> {
        self.st().session
    }

    /// The session has ended (and no new one has connected since).
    pub(crate) fn session_ended(&self) -> bool {
        self.st().session_ended
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

    /// The messages the guest wrote and the host has not consumed.
    pub(crate) fn outbox(&self) -> Vec<ControlMessage> {
        self.st().outbox.clone()
    }

    /// The host consumes every message the guest wrote and returns them.
    pub(crate) fn take_outbox(&self) -> Vec<ControlMessage> {
        std::mem::take(&mut self.st().outbox)
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

    /// The host ends the control session now: a pending read completes at EOF
    /// (or with the error); with no read pending, the session socket reports
    /// `POLLRDHUP` (or `POLLERR`). K-A5: a host close is EOF / `EPOLLRDHUP`.
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
                SimSessionEnd::Eof => GuestHangup::ReadHangup,
                SimSessionEnd::Reset => GuestHangup::Error(ErrorKind::ConnectionReset),
            };
            st.condition(session, hangup);
        }
    }

    /// The host ends the control session and the guest has not observed it
    /// yet: nothing is delivered until [`Self::release_session_end`].
    pub(crate) fn host_end_session_deferred(&self, how: SimSessionEnd) {
        let mut st = self.st();
        assert!(st.session.is_some(), "driver defect: no live session to end");
        st.host_closed = Some(how);
    }

    /// The host has ended the session and the guest has not observed it.
    pub(crate) fn session_end_pending(&self) -> bool {
        self.st().host_closed.is_some()
    }

    /// Releases a deferred end to the guest.
    pub(crate) fn release_session_end(&self) {
        let how = self.st().host_closed.take().expect("driver defect: no deferred session end");
        self.end_session(how);
    }

    /// The guest owner process dies with its VM (the driver dropped the
    /// owner): the guest's forwarder context is released (every pair
    /// aborted, no record) and the kernel closes every socket it held.
    pub(crate) fn guest_owner_dies(&self) {
        self.fwd.release();
        let mut st = self.st();
        let open: Vec<SocketId> =
            st.sockets.iter().filter(|(_, s)| !s.closed).map(|(id, _)| *id).collect();
        for id in open {
            st.end_socket(id, true);
        }
        st.session = None;
        st.session_ended = true;
        st.host_closed = None;
        st.deliverable.clear();
    }

    /// The host owner dies (`serve` killed): its context's release shuts every
    /// host flow vsock both ways, which reaches each guest flow vsock as a
    /// `0x3` (V-31 item 2c): an attached V_g is classified by the guest
    /// forwarder as an abort; an unattached one reports its end; a V_g still
    /// queued is torn down there; then the session ends (EOF).
    pub(crate) fn host_owner_dies(&self) {
        let flows: Vec<FlowId> = {
            let st = self.st();
            let queued = st.inbound.iter().filter(|(_, r)| r.socket.is_none()).map(|(f, _)| *f);
            let open = st
                .sockets
                .values()
                .filter(|s| {
                    !s.closed
                        && !s.outer
                        && (matches!(s.stage, Stage::Inbound | Stage::Pair)
                            || s.channel == Some(HostChannel::FlowStream))
                })
                .filter_map(|s| s.flow);
            queued.chain(open).collect()
        };
        for flow in flows {
            self.host_vsock_end(flow);
        }
        self.end_session(SimSessionEnd::Eof);
    }

    /// A scripted error on `flow`'s unattached guest vsock (`POLLERR`).
    pub(crate) fn flow_vsock_error(&self, flow: FlowId, kind: ErrorKind) {
        let mut st = self.st();
        let id = st
            .flow_socket(flow, |s| !s.outer && matches!(s.stage, Stage::Host | Stage::Inbound))
            .expect("driver defect: no unattached vsock for that flow");
        st.condition(id, GuestHangup::Error(kind));
    }

    // ----- host-opened flows (TcpAccept) -------------------------------------------

    /// The host connects V_h to the guest (CID, 1242) and writes `request`:
    /// V_g waits in the guest accept queue with the request buffered.
    pub(crate) fn host_open_accept(&self, request: FlowRequest) {
        let FlowRequest::TcpAccept { flow, .. } = request else {
            panic!("driver defect: the host opens only TcpAccept flows on 1242");
        };
        self.st()
            .inbound
            .insert(flow, InboundRec { request: Some(request), socket: None, host_closed: false });
    }

    /// The host connects V_h to the guest for host-origin `flow` and writes no
    /// byte: V_g waits in the guest accept queue with nothing buffered (its
    /// request read completes at EOF before any byte).
    pub(crate) fn host_open_accept_without_request(&self, flow: FlowId) {
        self.st()
            .inbound
            .insert(flow, InboundRec { request: None, socket: None, host_closed: false });
    }

    /// Releases the guest's accept of `flow`'s V_g (`InboundAccepted`).
    pub(crate) fn release_inbound_accept(&self, flow: FlowId) {
        let mut st = self.st();
        let handle = st.handle();
        let id = handle.id;
        let rec = st.inbound.get_mut(&flow).expect("driver defect: no V_g queued for that flow");
        assert!(rec.socket.is_none(), "driver defect: V_g already accepted");
        rec.socket = Some(id);
        let host_closed = rec.host_closed;
        let mut sock = Sock::new(Stage::Inbound, Some(SimSocket::vsock_stream()));
        sock.flow = Some(flow);
        sock.connect = Connect::Delivered(Ok(()));
        sock.peer_ended = host_closed;
        st.sockets.insert(id, sock);
        st.deliverable.push_back(GuestIoEvent::InboundAccepted { conn: InboundConn(handle) });
    }

    /// Releases the pending read of `flow`'s request on V_g (EOF before any
    /// byte when the host wrote none).
    pub(crate) fn release_inbound_request(&self, flow: FlowId) {
        let mut st = self.st();
        let rec = st.inbound.get(&flow).expect("driver defect: no V_g for that flow");
        let id = rec.socket.expect("driver defect: V_g not accepted yet");
        let bytes = rec.request.map(|r| r.encode().to_vec()).unwrap_or_default();
        let sock = st.sock(id);
        assert!(
            sock.read_pending == Some(GuestRecord::FlowRequest),
            "driver defect: the owner has no request read pending on V_g"
        );
        sock.read_pending = None;
        st.deliverable.push_back(GuestIoEvent::Read { socket: id, result: Ok(bytes) });
    }

    /// The host closes V_h of `flow` (a dial's abort or deadline, a guest
    /// flow's refusal, the host owner's death): a V_g still in the guest
    /// accept queue is torn down there (K-A4); an accepted, unattached V_g
    /// reports `POLLRDHUP` (DG-5-E) and `inbound_ended` reads true; an
    /// unattached guest-opened flow V_g reports `POLLRDHUP`; an attached V_g
    /// receives the host's `0x3` (the release's `SHUT_RDWR`, EVMOD-CTL), which
    /// its pair's callback classifies in this call — the end record is held
    /// until [`Self::release_forward_end`].
    pub(crate) fn host_vsock_end(&self, flow: FlowId) {
        let attached = {
            let mut st = self.st();
            if let Some(rec) = st.inbound.get_mut(&flow) {
                rec.host_closed = true;
                if rec.socket.is_none() {
                    st.inbound.remove(&flow);
                    st.reset_in_queue.insert(flow);
                    return;
                }
            }
            let Some(id) = st.flow_socket(flow, |s| !s.outer) else { return };
            if st.sock(id).stage == Stage::Pair {
                Some(st.sock(id).kernel().clone())
            } else {
                st.sock(id).peer_ended = true;
                st.condition(id, GuestHangup::ReadHangup);
                None
            }
        };
        if let Some(vsock) = attached {
            vsock.peer_shutdown(SimShutdown::Both);
        }
    }

    /// The vsock flows the guest has closed (test support: the host's view of
    /// its V_h's end).
    pub(crate) fn vsock_ended(&self) -> BTreeSet<FlowId> {
        self.st().vsock_ended.clone()
    }

    // ----- guest-opened flows (TcpConnect) ----------------------------------------

    /// An application's `connect()` is captured: the TCP intake accepts its
    /// child I (unattached; the application's early bytes and FIN wait in its
    /// queue), released at once (`OutboundCaptured`). Returns I's socket id.
    pub(crate) fn app_connect(&self, cookie: SocketCookie, destination: SocketAddrV4) -> SocketId {
        let mut st = self.st();
        let handle = st.handle();
        let id = handle.id;
        let mut sock = Sock::new(Stage::Captured, Some(SimSocket::tcp()));
        sock.outer = true;
        sock.linger = true;
        sock.connect = Connect::Delivered(Ok(()));
        st.sockets.insert(id, sock);
        st.live_cookies.insert(cookie);
        st.deliverable.push_back(GuestIoEvent::OutboundCaptured {
            child: CapturedConn(handle),
            original_destination: destination,
            cookie,
        });
        id
    }

    /// The flow id the owner wrote in the request on `socket` (a flow V_g).
    pub(crate) fn flow_of(&self, socket: SocketId) -> Option<FlowId> {
        self.st().sockets.get(&socket).and_then(|s| s.flow)
    }

    /// Every flow request the guest wrote, in order.
    pub(crate) fn requests(&self) -> Vec<FlowRequest> {
        self.st().requests.clone()
    }

    /// The kernel socket of a guest socket the sim issued (I by its socket id,
    /// before the owner wrote any request).
    pub(crate) fn kernel_socket(&self, socket: SocketId) -> SimSocket {
        self.st().sock(socket).kernel().clone()
    }

    /// The application resets the captured connection `socket` (I) while it
    /// is unattached: its kernel half is reset and the seam reports
    /// `Hangup { Error }` (EVMOD-PAIR: an unattached intake child is observed
    /// only for its error).
    pub(crate) fn app_reset_captured(&self, socket: SocketId) {
        let kernel = self.kernel_socket(socket);
        kernel.peer_reset();
        self.st().condition(socket, GuestHangup::Error(ErrorKind::ConnectionReset));
    }

    // ----- kernel facts of a flow's sockets ---------------------------------------

    /// The pair the guest forwarder accepted for `flow`, if any.
    pub(crate) fn pair_of(&self, flow: FlowId) -> Option<ForwardId> {
        self.st().pair_of(flow)
    }

    /// The half of `flow`'s pair that V_g is (`A` for TcpAccept, `B` for
    /// TcpConnect and datagram associations, "Sides and pairs").
    pub(crate) fn vsock_side(&self, flow: FlowId) -> Option<PairSide> {
        let st = self.st();
        st.pair_of(flow).and_then(|id| st.pairs.get(&id)).map(|p| p.vsock_side)
    }

    /// The kernel socket of one half of `flow` (open or closed): V_g, or the
    /// outer leg I / C. The far side of a half is scripted on it (`peer_*`)
    /// and its far side's observations are read from it (`peer_received`,
    /// `peer_end`); test support and the drivers' stimulus.
    pub(crate) fn flow_socket(&self, flow: FlowId, half: SimHalf) -> Option<SimSocket> {
        let st = self.st();
        st.sockets
            .values()
            .filter(|s| s.flow == Some(flow) && s.outer == (half == SimHalf::Outer))
            .filter(|s| s.slot.is_none())
            .filter_map(|s| s.kernel.clone())
            .next_back()
    }

    /// The application resets `flow`'s outer leg (`TcpRst` at the guest): the
    /// kernel half's far peer resets (a hooked half's `sk_error_report` runs
    /// in this call); an unattached intake child also reports `Hangup {
    /// Error }`.
    pub(crate) fn app_reset(&self, flow: FlowId) {
        let (kernel, unattached) = {
            let st = self.st();
            let id = st
                .sockets
                .iter()
                .filter(|(_, s)| s.flow == Some(flow) && s.outer && !s.closed)
                .map(|(id, _)| *id)
                .next_back()
                .expect("driver defect: the flow has no open outer leg");
            let sock = &st.sockets[&id];
            (sock.kernel().clone(), (sock.stage == Stage::Captured).then_some(id))
        };
        kernel.peer_reset();
        if let Some(id) = unattached {
            self.st().condition(id, GuestHangup::Error(ErrorKind::ConnectionReset));
        } else {
            // An attached half's real `sk_error_report` runs the forwarder in
            // the reset's own kernel step; preserve that event boundary in
            // the sim rather than requiring a second driver stimulus.
            self.run_forwarder();
        }
    }

    /// Releases the guest context's oldest queued end record as a
    /// `ForwardEnded` event (`take_end` on the context's readiness). Returns
    /// the record, `None` when none is queued.
    pub(crate) fn release_forward_end(&self) -> Option<ForwardEnd> {
        let end = self.fwd.with(|c| c.take_end().expect("the sim context is never in error"))?;
        self.st().deliverable.push_back(GuestIoEvent::ForwardEnded { end });
        Some(end)
    }

    /// Releases every queued end record (test support).
    pub(crate) fn release_forward_ends(&self) -> Vec<ForwardEnd> {
        std::iter::from_fn(|| self.release_forward_end()).collect()
    }

    /// `take_end` on the guest context's readiness, the record held — not yet
    /// delivered to the owner — until [`Self::deliver_taken_forward_ends`]
    /// (a driver that steps the context's read and the owner's dispatch of it
    /// as two model steps). Returns the record, `None` when none is queued.
    pub(crate) fn take_forward_end(&self) -> Option<ForwardEnd> {
        let end = self.fwd.with(|c| c.take_end().expect("the sim context is never in error"))?;
        self.st().taken_ends.push_back(end);
        Some(end)
    }

    /// Delivers every record [`Self::take_forward_end`] holds as a
    /// `ForwardEnded` event; `false` when none is held.
    pub(crate) fn deliver_taken_forward_ends(&self) -> bool {
        let mut st = self.st();
        if st.taken_ends.is_empty() {
            return false;
        }
        while let Some(end) = st.taken_ends.pop_front() {
            st.deliverable.push_back(GuestIoEvent::ForwardEnded { end });
        }
        true
    }

    // ----- datagram slots -----------------------------------------------------------

    /// An application socket `cookie` sends one datagram to `destination`: the
    /// capture program assigns the slot this (socket, destination) owns, or
    /// claims a free one; with no free slot the send is refused `EPERM` and
    /// counted (`None`). The datagram is queued in the slot socket (its
    /// kernel queue; the forwarder's run carries it once the slot is
    /// attached); an unattached slot with no association raises a readiness
    /// event (held until released).
    pub(crate) fn app_send(
        &self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
    ) -> Option<SocketAddrV4> {
        let owned = {
            let st = self.st();
            st.slots.iter().find(|(_, r)| r.owner == Some((cookie, destination))).map(|(s, _)| *s)
        };
        let Some(slot) = owned.or_else(|| self.st().free_slot()) else {
            self.st().exhausted_sends += 1;
            return None;
        };
        self.app_send_on(cookie, destination, slot);
        Some(slot)
    }

    /// As [`Self::app_send`], on the slot `slot` the driver chose (the model's
    /// nondeterministic claim): the slot must be free or already owned by
    /// `(cookie, destination)`.
    pub(crate) fn app_send_on(
        &self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
        slot: SocketAddrV4,
    ) {
        let kernel = {
            let mut st = self.st();
            st.live_cookies.insert(cookie);
            let rec = st.slot_mut(slot);
            assert!(
                rec.owner.is_none() || rec.owner == Some((cookie, destination)),
                "driver defect: {slot} is owned by another (socket, destination)"
            );
            rec.owner = Some((cookie, destination));
            rec.origins.push_back((cookie, destination));
            let idle = rec.attached.is_none() && rec.vsock.is_none() && rec.flow.is_none();
            let kernel = rec.kernel.clone();
            if idle {
                st.readiness.insert(slot);
            }
            kernel
        };
        kernel.peer_send(&[1]);
    }

    /// Sends refused for lack of a free slot (`guest_udp_slot_exhausted`).
    pub(crate) fn exhausted_sends(&self) -> u64 {
        self.st().exhausted_sends
    }

    /// Releases `slot`'s readiness event (`SlotReady`).
    pub(crate) fn release_slot_ready(&self, slot: SocketAddrV4) {
        let mut st = self.st();
        assert!(st.readiness.remove(&slot), "driver defect: no readiness pending on {slot}");
        st.deliverable.push_back(GuestIoEvent::SlotReady { slot });
    }

    /// The kernel slot socket S of `slot`'s current association (test support
    /// and the drivers' stimulus; the application is its far peer).
    pub(crate) fn slot_socket(&self, slot: SocketAddrV4) -> SimSocket {
        self.st().slot_mut(slot).kernel.clone()
    }

    /// The V_g of `slot`'s current or most recently ended association (the
    /// host is its far peer; D-5 keeps the arbiter readable after close).
    pub(crate) fn slot_vsock(&self, slot: SocketAddrV4) -> Option<SimSocket> {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        let id = rec.vsock.or(rec.last_vsock)?;
        st.sockets.get(&id).and_then(|s| s.kernel.clone())
    }

    /// The host sends one datagram frame toward the guest on `slot`'s attached
    /// association (`frame` is the ADR-0149 frame the host forwarder wrote).
    pub(crate) fn host_datagram(&self, slot: SocketAddrV4, frame: &[u8]) {
        let vsock = self.slot_vsock(slot).expect("driver defect: the slot has no V_g");
        vsock.peer_send(frame);
    }

    /// The application socket `cookie` is released, its `sock_release` report
    /// queued (held) when `report`, lost otherwise.
    pub(crate) fn app_close_socket(&self, cookie: SocketCookie, report: bool) {
        let mut st = self.st();
        st.live_cookies.remove(&cookie);
        if report {
            st.release_reports.insert(cookie);
        }
    }

    /// Releases `cookie`'s `sock_release` report (`SocketReleased`).
    pub(crate) fn release_socket_report(&self, cookie: SocketCookie) {
        let mut st = self.st();
        assert!(st.release_reports.remove(&cookie), "driver defect: no report queued for it");
        st.deliverable.push_back(GuestIoEvent::SocketReleased { cookie });
    }

    /// Test-support view of `slot`.
    pub(crate) fn slot_state(&self, slot: SocketAddrV4) -> SimSlotState {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        SimSlotState {
            owner: rec.owner,
            flow: rec.flow,
            attached: rec.attached,
            connected: rec.connected,
            discarded: rec.discarded,
            ended: rec.ended,
        }
    }

    /// The datagram flow V_g of `slot`'s association (test support).
    pub(crate) fn slot_flow(&self, slot: SocketAddrV4) -> Option<FlowId> {
        self.st().slots.get(&slot).and_then(|r| r.flow)
    }

    /// The guest's workload address.
    pub(crate) fn workload_addr(&self) -> Ipv4Addr {
        self.st().workload_addr
    }
}

// ---------------------------------------------------------------------------
// The seam implementation
// ---------------------------------------------------------------------------

impl GuestOwnerIo for SimGuestOwnerIo {
    fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<Option<GuestIoEvent>> {
        let now = self.clock.now();
        let mut st = self.st();
        if let Some(kind) = st.event_wait_failure.take() {
            return Err(io::Error::from(kind));
        }
        if let Some(event) = st.deliverable.pop_front() {
            st.last_capture = None;
            st.last_slot = None;
            match &event {
                GuestIoEvent::OutboundCaptured { child, .. } => st.last_capture = Some(child.id()),
                GuestIoEvent::SlotReady { slot } => st.last_slot = Some(*slot),
                _ => {}
            }
            if let GuestIoEvent::Connected { socket, result } = &event {
                let (socket, result) = (*socket, *result);
                let sock = st.sock(socket);
                sock.connect = Connect::Delivered(result);
                let control = sock.channel == Some(HostChannel::Control);
                if control && result.is_ok() {
                    st.session = Some(socket);
                    st.session_ended = false;
                }
            }
            return Ok(Some(event));
        }
        match deadline {
            Some(d) if d <= now => Ok(None),
            _ => unreachable!(
                "driver defect (DG-5-Q): next_event called with nothing released and no deadline due"
            ),
        }
    }

    fn start_connect_host(&mut self, channel: HostChannel) -> io::Result<HostConn> {
        let mut st = self.st();
        st.scripted(SimGuestOp::StartConnectHost)?;
        let handle = st.handle();
        let id = handle.id;
        let mut sock = Sock::new(Stage::Host, Some(SimSocket::vsock_stream()));
        sock.channel = Some(channel);
        sock.connect = Connect::InFlight;
        if channel == HostChannel::FlowStream {
            if let Some(child) = st.last_capture.take() {
                sock.partner = Some(child);
            } else if let Some(slot) = st.last_slot.take() {
                sock.slot = Some(slot);
            }
        }
        st.sockets.insert(id, sock);
        if let Some(slot) = st.sockets[&id].slot {
            let rec = st.slot_mut(slot);
            rec.vsock = Some(id);
            rec.last_vsock = Some(id);
        }
        st.effect(SimGuestEffect::ConnectHost { socket: id, channel });
        Ok(HostConn(handle))
    }

    fn start_read_host(&mut self, conn: &HostConn, record: GuestRecord) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::StartReadHost)?;
        st.require_connected(id)?;
        st.sock(id).read_pending = Some(record);
        st.effect(SimGuestEffect::ReadHost { socket: id, record });
        Ok(())
    }

    fn start_read_inbound(&mut self, conn: &InboundConn) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::StartReadInbound)?;
        st.sock(id).read_pending = Some(GuestRecord::FlowRequest);
        let flow = st.sock(id).flow.expect("an accepted V_g carries its dial's flow");
        st.effect(SimGuestEffect::ReadInbound { socket: id, flow });
        Ok(())
    }

    fn write_host(&mut self, conn: &HostConn, bytes: &[u8]) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::WriteHost)?;
        st.require_connected(id)?;
        assert!(
            st.session == Some(id),
            "guest-owner seam: write_host on a socket other than the session"
        );
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
                let message =
                    ControlMessage::decode(&raw).expect("the guest writes valid messages");
                st.effect(SimGuestEffect::WriteControl { socket: id, message });
                // a write after the host ended the session reaches nobody
                if st.host_closed.is_none() {
                    st.outbox.push(message);
                }
            }
            n => panic!("guest-owner seam: a control-session write of {n} bytes is not a record"),
        }
        Ok(())
    }

    fn inbound_ended(&mut self, conn: &InboundConn) -> io::Result<bool> {
        let mut st = self.st();
        let id = st.check(&conn.0);
        st.scripted(SimGuestOp::InboundEnded)?;
        let ended = st.sock(id).peer_ended;
        st.effect(SimGuestEffect::InboundEnded { socket: id, ended });
        if !ended {
            st.last_checked_dial = st.sock(id).flow;
        }
        Ok(ended)
    }

    fn write_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&vsock.0);
        st.scripted(SimGuestOp::WriteRequest)?;
        st.require_connected(id)?;
        let request = FlowRequest::decode(request).expect("the guest writes valid requests");
        assert!(
            matches!(request, FlowRequest::TcpConnect { .. }),
            "guest-owner seam: write_request carries a TcpConnect request"
        );
        let flow = request.flow();
        st.sock(id).flow = Some(flow);
        if let Some(child) = st.sock(id).partner {
            st.sock(child).flow = Some(flow);
        }
        st.requests.push(request);
        st.effect(SimGuestEffect::WriteRequest { socket: id, request });
        Ok(())
    }

    fn attach_outbound(
        &mut self,
        child: CapturedConn,
        vsock: HostConn,
    ) -> Result<GuestPair, GuestAttachRefused> {
        let (c, v, a, b) = {
            let st = self.st();
            let c = st.check(&child.0);
            let v = st.check(&vsock.0);
            (c, v, st.sockets[&c].kernel().clone(), st.sockets[&v].kernel().clone())
        };
        // FlowForwarder::attach(I, V_g, Stream): I is half A, V_g half B
        let result = self.fwd.with(|ctx| ctx.attach(&a, &b, PairMode::Stream));
        let mut st = self.st();
        let flow = st.sock(v).flow;
        match result {
            Ok(id) => {
                for s in [c, v] {
                    st.sock(s).stage = Stage::Pair;
                    st.sock(s).attached = Some(id);
                    st.unwatch(s);
                }
                let rec = PairRec {
                    flow,
                    vsock: Some(v),
                    outer: Some(c),
                    slot: None,
                    vsock_side: PairSide::B,
                };
                st.pairs.insert(id, rec);
                st.effect(SimGuestEffect::AttachOutbound { flow, accepted: Some(id) });
                Ok(GuestPair {
                    id,
                    vsock: GuestPairSocket(st.same(v)),
                    outer: GuestPairSocket(st.same(c)),
                })
            }
            Err(error) => {
                st.effect(SimGuestEffect::AttachOutbound { flow, accepted: None });
                Err(GuestAttachRefused { error, vsock: vsock.into(), outer: child.into() })
            }
        }
    }

    fn start_connect_app(
        &mut self,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> io::Result<AppConn> {
        let mut st = self.st();
        st.scripted(SimGuestOp::StartConnectApp)?;
        let handle = st.handle();
        let id = handle.id;
        let mut sock = Sock::new(Stage::App, None);
        sock.app_target = Some((client, destination));
        sock.connect = Connect::InFlight;
        sock.outer = true;
        sock.linger = true;
        sock.flow = st.last_checked_dial.take();
        st.sockets.insert(id, sock);
        st.effect(SimGuestEffect::StartConnectApp { socket: id, client, destination });
        Ok(AppConn(handle))
    }

    fn attach_inbound(
        &mut self,
        vsock: InboundConn,
        app: AppConn,
    ) -> Result<GuestPair, GuestAttachRefused> {
        let (v, c, a, b) = {
            let st = self.st();
            let v = st.check(&vsock.0);
            let c = st.check(&app.0);
            (v, c, st.sockets[&v].kernel().clone(), st.sockets[&c].kernel().clone())
        };
        // FlowForwarder::attach(V_g, C, Stream): V_g is half A, C half B
        let result = self.fwd.with(|ctx| ctx.attach(&a, &b, PairMode::Stream));
        let mut st = self.st();
        let flow = st.sock(v).flow;
        match result {
            Ok(id) => {
                for s in [v, c] {
                    st.sock(s).stage = Stage::Pair;
                    st.sock(s).attached = Some(id);
                    st.unwatch(s);
                }
                let rec = PairRec {
                    flow,
                    vsock: Some(v),
                    outer: Some(c),
                    slot: None,
                    vsock_side: PairSide::A,
                };
                st.pairs.insert(id, rec);
                st.effect(SimGuestEffect::AttachInbound { flow, accepted: Some(id) });
                Ok(GuestPair {
                    id,
                    vsock: GuestPairSocket(st.same(v)),
                    outer: GuestPairSocket(st.same(c)),
                })
            }
            Err(error) => {
                st.effect(SimGuestEffect::AttachInbound { flow, accepted: None });
                Err(GuestAttachRefused { error, vsock: vsock.into(), outer: app.into() })
            }
        }
    }

    fn abort_forward(&mut self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        let result = self.fwd.with(|ctx| ctx.abort(id));
        let mut st = self.st();
        if matches!(result, Ok(AbortResult::Aborted))
            && let Some(flow) = st.pairs.get(&id).and_then(|pair| pair.flow)
        {
            st.vsock_ended.insert(flow);
        }
        st.effect(SimGuestEffect::AbortForward { id });
        result
    }

    fn pair_state(&mut self, id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        self.st().effect(SimGuestEffect::PairState { id });
        self.fwd.with(|ctx| ctx.pair_state(id))
    }

    fn inventory(&mut self) -> Result<ForwarderInventory, ForwarderError> {
        self.st().effect(SimGuestEffect::Inventory);
        self.fwd.with(FlowForwarder::inventory)
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
        let handle = match &socket {
            GuestOwnedSocket::Host(h) => &h.0,
            GuestOwnedSocket::Inbound(h) => &h.0,
            GuestOwnedSocket::Captured(h) => &h.0,
            GuestOwnedSocket::App(h) => &h.0,
            GuestOwnedSocket::Pair(h) => &h.0,
        };
        let id = st.check(handle);
        let flow = st.sock(id).flow;
        st.end_socket(id, true);
        st.effect(SimGuestEffect::Abort { socket: id, flow });
    }

    fn close(&mut self, socket: GuestOwnedSocket) {
        let mut st = self.st();
        let handle = match &socket {
            GuestOwnedSocket::Host(h) => &h.0,
            GuestOwnedSocket::Inbound(h) => &h.0,
            GuestOwnedSocket::Captured(h) => &h.0,
            GuestOwnedSocket::App(h) => &h.0,
            GuestOwnedSocket::Pair(h) => &h.0,
        };
        let id = st.check(handle);
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

    fn write_slot_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let mut st = self.st();
        let id = st.check(&vsock.0);
        st.scripted(SimGuestOp::WriteSlotRequest)?;
        st.require_connected(id)?;
        let request = FlowRequest::decode(request).expect("the guest writes valid requests");
        let FlowRequest::DatagramStream { flow, .. } = request else {
            panic!("guest-owner seam: write_slot_request carries a DatagramStream request");
        };
        // the association's slot: linked at the connect the owner issued in
        // that slot's `SlotReady` dispatch (slot ready → connect → request)
        let slot = st.sock(id).slot.expect("guest-owner seam: the V_g was opened for no slot");
        let rec = st.slot_mut(slot);
        rec.flow = Some(flow);
        rec.vsock = Some(id);
        rec.last_vsock = Some(id);
        st.sock(id).flow = Some(flow);
        st.requests.push(request);
        st.effect(SimGuestEffect::WriteSlotRequest { socket: id, request });
        Ok(())
    }

    fn attach_slot(
        &mut self,
        slot: SocketAddrV4,
        vsock: HostConn,
    ) -> Result<ForwardId, GuestSlotAttachRefused> {
        let (v, s_kernel, v_kernel) = {
            let mut st = self.st();
            let v = st.check(&vsock.0);
            let s_kernel = st.slot_mut(slot).kernel.clone();
            (v, s_kernel, st.sockets[&v].kernel().clone())
        };
        // FlowForwarder::attach(S, V_g, Datagram { udp: A })
        let result = self
            .fwd
            .with(|ctx| ctx.attach(&s_kernel, &v_kernel, PairMode::Datagram { udp: PairSide::A }));
        let mut st = self.st();
        match result {
            Ok(id) => {
                let flow = st.sock(v).flow;
                st.sock(v).stage = Stage::Pair;
                st.sock(v).attached = Some(id);
                st.unwatch(v);
                let rec = st.slot_mut(slot);
                rec.attached = Some(id);
                rec.connected = true;
                let pair = PairRec {
                    flow,
                    vsock: Some(v),
                    outer: None,
                    slot: Some(slot),
                    vsock_side: PairSide::B,
                };
                st.pairs.insert(id, pair);
                st.effect(SimGuestEffect::AttachSlot { slot, accepted: Some(id) });
                Ok(id)
            }
            Err(error) => {
                st.effect(SimGuestEffect::AttachSlot { slot, accepted: None });
                Err(GuestSlotAttachRefused { error, vsock })
            }
        }
    }

    fn end_association(&mut self, slot: SocketAddrV4) {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        // discard S's receive queue without reading payload, then disconnect S
        // (the next association connects a fresh S: a recycled slot carries
        // none of the previous association's datagrams)
        let mut discarded = 0;
        while let SimLocalRead::Bytes(_) = rec.kernel.local_read(usize::MAX) {
            discarded += 1;
        }
        rec.discarded += discarded;
        rec.ended += 1;
        rec.kernel = SimSocket::new(SimSocketKind::Udp, SimSocketSetup::Connected);
        rec.origins.clear();
        rec.connected = false;
        rec.attached = None;
        rec.flow = None;
        rec.vsock = None;
        st.readiness.remove(&slot);
        st.effect(SimGuestEffect::EndAssociation { slot });
    }

    fn release_slot(&mut self, slot: SocketAddrV4) {
        let mut st = self.st();
        let rec = st.slot_mut(slot);
        assert!(rec.flow.is_none(), "guest-owner seam: release of a slot with a live association");
        rec.owner = None;
        st.effect(SimGuestEffect::ReleaseSlot { slot });
    }

    fn slot_counters(&mut self, slot: SocketAddrV4) -> io::Result<(u64, u64)> {
        let mut st = self.st();
        st.scripted(SimGuestOp::SlotCounters)?;
        st.effect(SimGuestEffect::SlotCounters { slot });
        let attached = st.slot_mut(slot).attached;
        drop(st);
        let state = attached.and_then(|id| self.forwarder_pair_state(id));
        Ok(state.map_or((0, 0), |s| (s.a_to_b.moved, s.b_to_a.moved)))
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
}
