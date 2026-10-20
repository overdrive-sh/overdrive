//! `SimFlowForwarder` — the sim binding of the flow forwarder's driven port
//! [`FlowForwarder`] (netns-density-295 EVMOD-PORT), over [`SimSocket`]s.
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — `FlowForwarder` (EVMOD-PORT)" (the port's
//! preconditions, postconditions, edge cases and invariants, which this sim
//! honours exactly as `overdrive-fwd::KernelFlowForwarder` does), § "Sides
//! and pairs", § "End classification", § "Pair end", § "Datagram mode" and
//! § "Control interface" (the accepted-state table and the refusal
//! precedence), and § "Conformance observation surface and action map"
//! (the named-arbiter rows of `SimFlowForwarder`).
//!
//! The sim is the **named arbiter** of the forwarder's kernel facts: the
//! sockets' queues, each direction's source end and target shutdown, the
//! class each vsock transition was given, the queued end records, the live
//! pairs and watches, and the context table. Its Tier-3 pair is the module's
//! suite through `KernelFlowForwarder` (the shapes of V-EV-END, V-EV-CTL,
//! V-EV-PAIR and V-EV-DGRAM, P-69 to P-72). It models those facts and nothing
//! else.
//!
//! # Shape
//!
//! - [`SimForwarderDevice`] is one machine's `/dev/ovd_fwd`: it opens
//!   contexts and keeps the context table (open or released, and what each
//!   still holds).
//! - [`SimFlowForwarder`] is one context, the port's implementation. The
//!   kernel's work items are not run behind the test's back: a direction's
//!   run is queued by attach and by every socket event, and runs only when
//!   the harness calls [`SimFlowForwarder::run`] or
//!   [`SimFlowForwarder::run_until_idle`]. Callbacks (`sk_state_change`,
//!   `sk_error_report`) run synchronously inside the scripting call that
//!   triggers them, as the kernel runs them inside packet receipt: a watch's
//!   end record is queued before the scripting call returns.
//! - [`SimSocket`] is one socket. The far side of every socket is scripted
//!   by the harness (`peer_*`), so one owner is driven at a time, the other
//!   side's behaviour being stimulus. Its read surface reports what the far
//!   reader received and the far end's first terminal event.
//!
//! RED scaffold (DISTILL, Mandate 7; decision B-4): DELIVER step 4
//! ("Forwarder module and adapter") implements this sim. Construction is
//! real — [`SimForwarderDevice::open`] adds a row to the context table,
//! [`SimForwarderDevice::contexts`] reads it, and [`SimSocket::new`] makes a
//! socket of its kind — so a conformance driver or body built over this sim
//! reaches its first production call and goes RED there, not in the sim.
//! Every other method body panics until step 4; the enums and the
//! context-table record are plain data. The method set is DISTILL's
//! test-support surface (scripting and read-back of kernel facts); the
//! [`FlowForwarder`] impl is the pinned port.

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER step 4).
#![allow(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::new_without_default,
    reason = "RED scaffold bodies panic before reading their inputs; the lints self-trip when DELIVER step 4 lands the bodies"
)]

use std::sync::Arc;

use parking_lot::Mutex;

use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwarderError, ForwarderInventory,
    PairMode, PairSide, PairState, SourceEnd,
};
use overdrive_core::guest_transport::wire::KernelErrno;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// The device and its context table
// ---------------------------------------------------------------------------

/// One machine's simulated `/dev/ovd_fwd`. Cloning shares the device.
#[derive(Debug, Clone)]
pub struct SimForwarderDevice {
    inner: Arc<Mutex<DeviceState>>,
}

/// The device's private state: the context table, in open order. DELIVER
/// step 4 adds what the contexts' pairs and watches need.
#[derive(Debug, Default)]
struct DeviceState {
    contexts: Vec<SimContextState>,
}

impl SimForwarderDevice {
    /// A device with no context.
    #[must_use]
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(DeviceState::default())) }
    }

    /// Opens a new context (the `KernelFlowForwarder::open()` stand-in): a
    /// new open row at the end of the context table, holding nothing. Ids
    /// start at 1 in every context and are never reused within it.
    #[must_use]
    pub fn open(&self) -> SimFlowForwarder {
        let mut state = self.inner.lock();
        state.contexts.push(SimContextState {
            open: true,
            live_pairs: 0,
            live_watches: 0,
            queued_ends: Vec::new(),
        });
        let index = state.contexts.len() - 1;
        drop(state);
        SimFlowForwarder { device: self.clone(), index }
    }

    /// The context table, in open order: every context this device opened,
    /// whether it is still open, and the pairs and watches it still holds (a
    /// released context holds none), with the end records it queued and
    /// nobody took.
    #[must_use]
    pub fn contexts(&self) -> Vec<SimContextState> {
        self.inner.lock().contexts.clone()
    }
}

/// One row of the device's context table (QPROJ-7, D-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimContextState {
    /// The context has not been released.
    pub open: bool,
    /// Pairs the context holds.
    pub live_pairs: u32,
    /// Watches the context holds.
    pub live_watches: u32,
    /// The ids whose end record the context holds and nobody has taken, in
    /// queue order. A released context's row keeps the queue it held at its
    /// release: the kernel frees it unread, and the row is the fact that the
    /// records were produced (D-4).
    pub queued_ends: Vec<ForwardId>,
}

// ---------------------------------------------------------------------------
// One context — the port implementation
// ---------------------------------------------------------------------------

/// One forwarder context of a [`SimForwarderDevice`].
#[derive(Debug)]
pub struct SimFlowForwarder {
    device: SimForwarderDevice,
    /// This context's row in the device's context table.
    index: usize,
}

impl SimFlowForwarder {
    /// The context's release — the last `close()` of its descriptor, or its
    /// owner's death: aborts every live pair and watch of the context (TCP
    /// halves reset, vsock halves `SHUT_RDWR`, a watched vsock included — the
    /// only operation that shuts a watched socket), queues no record, and
    /// returns once done. Another context's pairs are unaffected.
    pub fn release(self) {
        let _ = (&self.device, self.index);
        panic!("{SCAFFOLD}")
    }

    /// The directions with a queued run, as `(id, source half)`, in queue
    /// order.
    #[must_use]
    pub fn pending_runs(&self) -> Vec<(ForwardId, PairSide)> {
        panic!("{SCAFFOLD}")
    }

    /// One step of the direction of `id` whose source is `source` — the
    /// conformance drivers map one `fwd_pair` hop to one call. A step moves
    /// one unit the target accepts (a stream: one byte; datagram mode: one
    /// frame item, the 8-byte header or one payload byte, and a UDP datagram
    /// is consumed from its source only once its whole frame was accepted),
    /// or, once the source queue is empty, carries out the source's
    /// classified end or ends the pair — never a unit and the end in one
    /// call. The module's `FWD_CHUNK` is a module constant, not contract, so
    /// the sim's unit differs from it. `false` when no run of that direction
    /// is queued.
    pub fn run(&self, _id: ForwardId, _source: PairSide) -> bool {
        panic!("{SCAFFOLD}")
    }

    /// Runs every queued run until none is queued; returns how many ran.
    pub fn run_until_idle(&self) -> u32 {
        panic!("{SCAFFOLD}")
    }
}

impl FlowForwarder<SimSocket> for SimFlowForwarder {
    fn probe(&self) -> Result<(), ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn attach(
        &self,
        _a: &SimSocket,
        _b: &SimSocket,
        _mode: PairMode,
    ) -> Result<ForwardId, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn watch(&self, _vsock: &SimSocket) -> Result<ForwardId, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn abort(&self, _id: ForwardId) -> Result<AbortResult, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn take_end(&self) -> Result<Option<ForwardEnd>, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn pair_state(&self, _id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// Sim sockets — scripted far sides, read-back of kernel facts
// ---------------------------------------------------------------------------

/// The kind of a [`SimSocket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimSocketKind {
    /// `AF_VSOCK` `SOCK_STREAM`.
    VsockStream,
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// Where a [`SimSocket`] is in its connection's life when it is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimSocketSetup {
    /// Connected (`ESTABLISHED`; UDP: `connect()`ed).
    Connected,
    /// Never connected.
    Unconnected,
    /// Listening.
    Listening,
}

/// A vsock SHUTDOWN the far peer sends, or this side sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimShutdown {
    /// Flags `0x2` (SEND only); on a TCP socket, a FIN.
    Send,
    /// Flags `0x3` (SEND and RCV).
    Both,
}

/// A vsock socket's connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimVsockState {
    /// `ESTABLISHED`.
    Established,
    /// `CLOSING` (set by an `RST`, or a `0x3` with an empty queue).
    Closing,
}

/// The far end's first terminal event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimFarEnd {
    /// The far reader read end-of-stream after every byte.
    Eof,
    /// The far end was reset.
    Reset,
}

/// What a read by the socket's local holder returns (a watched vsock is read
/// by its holder, not by the forwarder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimLocalRead {
    /// Queued bytes, in order.
    Bytes(Vec<u8>),
    /// End-of-stream.
    Eof,
    /// The read failed with this errno (`ENOTCONN`, `ECONNRESET`).
    Err(KernelErrno),
    /// Nothing queued and no end (`EAGAIN`).
    WouldBlock,
}

/// One simulated socket. Cloning shares the socket: the owner holds one
/// handle as its descriptor, the harness another to script and observe it.
#[derive(Debug, Clone)]
pub struct SimSocket {
    inner: Arc<Mutex<SocketState>>,
}

/// The socket's private state: its kind and setup. DELIVER step 4 adds the
/// queues, the peer bits, `sk_err`, the vsock state and the far side.
#[derive(Debug)]
struct SocketState {
    kind: SimSocketKind,
    setup: SimSocketSetup,
}

impl SimSocket {
    /// A socket of `kind` in `setup`. A connected vsock STREAM starts
    /// `ESTABLISHED` with the default buffer size (256 KiB, above
    /// `MAX_GUEST_DATAGRAM_FRAME`), no peer bits, no `sk_err`, a reading far
    /// side and nothing queued.
    #[must_use]
    pub fn new(kind: SimSocketKind, setup: SimSocketSetup) -> Self {
        Self { inner: Arc::new(Mutex::new(SocketState { kind, setup })) }
    }

    /// A connected vsock STREAM.
    #[must_use]
    pub fn vsock_stream() -> Self {
        Self::new(SimSocketKind::VsockStream, SimSocketSetup::Connected)
    }

    /// A connected TCP socket.
    #[must_use]
    pub fn tcp() -> Self {
        Self::new(SimSocketKind::Tcp, SimSocketSetup::Connected)
    }

    /// A connected UDP socket.
    #[must_use]
    pub fn udp() -> Self {
        Self::new(SimSocketKind::Udp, SimSocketSetup::Connected)
    }

    // -- the far side (stimulus) --------------------------------------------

    /// The far peer sends `bytes` (UDP: one datagram, an empty one included);
    /// they are queued on this socket and its direction's run is queued.
    pub fn peer_send(&self, _bytes: &[u8]) {
        let _ = &self.inner;
        panic!("{SCAFFOLD}")
    }

    /// The far peer ends its direction: on a vsock STREAM a SHUTDOWN with
    /// `how`'s flags, recorded in `peer_shutdown` and classified by a hooked
    /// half's `sk_state_change` in this call; on TCP, `Send` is a FIN
    /// (`CLOSE_WAIT` once received). `Both` on TCP is a precondition
    /// violation.
    pub fn peer_shutdown(&self, _how: SimShutdown) {
        panic!("{SCAFFOLD}")
    }

    /// The far peer resets the connection: TCP — `sk_err` becomes
    /// `ECONNRESET`, or `EPIPE` in `CLOSE_WAIT`, reported through a hooked
    /// half's `sk_error_report` in this call; vsock — an `RST`, leaving
    /// `CLOSING` when nothing is queued and `ESTABLISHED` with every peer bit
    /// otherwise, classified in this call.
    pub fn peer_reset(&self) {
        panic!("{SCAFFOLD}")
    }

    /// The far reader stops reading: once the far side's buffer is full the
    /// target accepts nothing more, and the sender blocks within its buffer.
    pub fn peer_stop_reading(&self) {
        panic!("{SCAFFOLD}")
    }

    /// The far reader reads again: everything queued toward it is delivered
    /// in this call (one far-side read), and queued runs resume.
    pub fn peer_resume_reading(&self) {
        panic!("{SCAFFOLD}")
    }

    // -- kernel facts of this socket (stimulus) ------------------------------

    /// Sets `sk_err` without reporting it — on a vsock, the first of the
    /// device release's orphan-sweep steps (with the state and peer bits);
    /// [`Self::report_error`] is the second.
    pub fn set_sk_err(&self, _errno: KernelErrno) {
        panic!("{SCAFFOLD}")
    }

    /// Runs the socket's `sk_error_report` for its pending `sk_err` (a
    /// hooked half aborts its pair; a watch queues its record first).
    pub fn report_error(&self) {
        panic!("{SCAFFOLD}")
    }

    /// Sets a vsock STREAM's state directly.
    pub fn set_vsock_state(&self, _state: SimVsockState) {
        panic!("{SCAFFOLD}")
    }

    /// This side shuts its own send direction (`SHUT_WR`; a vsock sends
    /// `0x2`). On a socket not yet attached it makes the socket an owner
    /// defect for attach (`NotConnected`).
    pub fn shutdown_send(&self) {
        panic!("{SCAFFOLD}")
    }

    /// Sets a vsock STREAM's `buffer_size`.
    pub fn set_buffer_size(&self, _bytes: u32) {
        panic!("{SCAFFOLD}")
    }

    /// Installs an upper-layer protocol on a TCP socket (kTLS).
    pub fn set_ulp(&self) {
        panic!("{SCAFFOLD}")
    }

    /// The socket's local holder reads up to `max` bytes (a watched vsock's
    /// holder, never the forwarder); on a UDP socket one datagram per call.
    pub fn local_read(&self, _max: usize) -> SimLocalRead {
        panic!("{SCAFFOLD}")
    }

    // -- read-back (arbiter facts) -----------------------------------------

    /// The socket's kind.
    #[must_use]
    pub fn kind(&self) -> SimSocketKind {
        let state = self.inner.lock();
        let _ = state.setup;
        state.kind
    }

    /// The bytes the far reader has received, in order (stream sockets).
    #[must_use]
    pub fn peer_received(&self) -> Vec<u8> {
        panic!("{SCAFFOLD}")
    }

    /// The datagrams the far side has received, in order, empty ones
    /// included (UDP).
    #[must_use]
    pub fn peer_datagrams(&self) -> Vec<Vec<u8>> {
        panic!("{SCAFFOLD}")
    }

    /// The far end's first terminal event, once it has happened.
    #[must_use]
    pub fn peer_end(&self) -> Option<SimFarEnd> {
        panic!("{SCAFFOLD}")
    }

    /// The SHUTDOWN this side sent on a vsock STREAM, the strongest once
    /// (`0x3` after `0x2` reports `Both`).
    #[must_use]
    pub fn sent_shutdown(&self) -> Option<SimShutdown> {
        panic!("{SCAFFOLD}")
    }

    /// The SHUTDOWN flags this vsock STREAM received from its peer
    /// (`peer_shutdown`), the strongest once: `Send` for `0x2`, `Both` once
    /// `RCV` is set too (a `0x3`, or an `RST` that set every peer bit).
    #[must_use]
    pub fn received_shutdown(&self) -> Option<SimShutdown> {
        panic!("{SCAFFOLD}")
    }

    /// The class the forwarder recorded for the peer's end of this vsock
    /// half, once a transition was classified (at attach or by a callback).
    #[must_use]
    pub fn recorded_class(&self) -> Option<SourceEnd> {
        panic!("{SCAFFOLD}")
    }

    /// The pending `sk_err`.
    #[must_use]
    pub fn sk_err(&self) -> Option<KernelErrno> {
        panic!("{SCAFFOLD}")
    }

    /// A vsock STREAM's state.
    #[must_use]
    pub fn vsock_state(&self) -> SimVsockState {
        panic!("{SCAFFOLD}")
    }

    /// This side has shut its own send direction.
    #[must_use]
    pub fn own_send_shut(&self) -> bool {
        panic!("{SCAFFOLD}")
    }

    /// The socket is attached or watched by some context (`sk_user_data`
    /// set).
    #[must_use]
    pub fn is_hooked(&self) -> bool {
        panic!("{SCAFFOLD}")
    }

    /// Bytes (UDP: datagrams) queued on this socket and not yet consumed.
    #[must_use]
    pub fn queued(&self) -> usize {
        panic!("{SCAFFOLD}")
    }
}
