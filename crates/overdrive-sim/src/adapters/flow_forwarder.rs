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
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;

use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, DirectionState, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome,
    ForwarderError, ForwarderInventory, ForwarderOp, PairMode, PairSide, PairState, SourceEnd,
};
use overdrive_core::guest_transport::wire::KernelErrno;

// ---------------------------------------------------------------------------
// The device and its context table
// ---------------------------------------------------------------------------

/// One machine's simulated `/dev/ovd_fwd`. Cloning shares the device.
#[derive(Debug, Clone)]
pub struct SimForwarderDevice {
    inner: Arc<Mutex<DeviceState>>,
}

impl Default for SimForwarderDevice {
    fn default() -> Self {
        Self::new()
    }
}

/// The device's private state: the context table, in open order. DELIVER
/// step 4 adds what the contexts' pairs and watches need.
#[derive(Debug, Default)]
struct DeviceState {
    contexts: Vec<ContextState>,
}

#[derive(Debug)]
struct ContextState {
    summary: SimContextState,
    next_id: u64,
    live: BTreeMap<ForwardId, SimItem>,
    ends: VecDeque<ForwardEnd>,
    runs: VecDeque<(ForwardId, PairSide)>,
    attached: u64,
    completed: u64,
    aborted: u64,
    unclassified_eof: u64,
    dgram_oversize: u64,
}

#[derive(Debug)]
enum SimItem {
    Pair(SimPair),
    Watch(SimSocket),
}

#[derive(Debug)]
struct SimPair {
    halves: [SimSocket; 2],
    mode: PairMode,
    a_to_b: PairDirection,
    b_to_a: PairDirection,
    abort: Option<AbortCause>,
}

#[derive(Debug)]
struct PairDirection {
    state: DirectionState,
    pending_frame: Option<(Vec<u8>, usize)>,
}

impl Default for PairDirection {
    fn default() -> Self {
        Self {
            state: DirectionState { source_end: None, target_shut: false, moved: 0 },
            pending_frame: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookKind {
    Pair,
    Watch,
}

#[derive(Debug, Clone)]
struct Hook {
    device: Weak<Mutex<DeviceState>>,
    context_index: usize,
    id: ForwardId,
    side: PairSide,
    kind: HookKind,
}

const SIM_SOCKET_BUFFER: usize = 64 * 1024;
const DEFAULT_VSOCK_BUFFER: u32 = 256 * 1024;

fn errno(raw: u16) -> KernelErrno {
    KernelErrno::new(raw).unwrap_or_else(|_| unreachable!("the simulated errno is non-zero"))
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
        state.contexts.push(ContextState {
            summary: SimContextState {
                open: true,
                live_pairs: 0,
                live_watches: 0,
                queued_ends: Vec::new(),
            },
            next_id: 1,
            live: BTreeMap::new(),
            ends: VecDeque::new(),
            runs: VecDeque::new(),
            attached: 0,
            completed: 0,
            aborted: 0,
            unclassified_eof: 0,
            dgram_oversize: 0,
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
        self.inner.lock().contexts.iter().map(|context| context.summary.clone()).collect()
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

fn io_error(op: ForwarderOp, errno: i32) -> ForwarderError {
    ForwarderError::Io { op, source: std::io::Error::from_raw_os_error(errno) }
}

fn refresh_summary(context: &mut ContextState) {
    context.summary.live_pairs = u32::try_from(
        context.live.values().filter(|item| matches!(item, SimItem::Pair(_))).count(),
    )
    .unwrap_or(u32::MAX);
    context.summary.live_watches = u32::try_from(
        context.live.values().filter(|item| matches!(item, SimItem::Watch(_))).count(),
    )
    .unwrap_or(u32::MAX);
}

fn queue_end(context: &mut ContextState, end: ForwardEnd) {
    context.ends.push_back(end);
    context.summary.queued_ends.push(end.id);
}

fn enqueue_run(context: &mut ContextState, id: ForwardId, source: PairSide) {
    if context.live.contains_key(&id) && !context.runs.contains(&(id, source)) {
        context.runs.push_back((id, source));
    }
}

fn unhook(socket: &SimSocket) {
    socket.inner.lock().hooked = None;
}

fn abort_pair_sockets(pair: &SimPair) {
    for socket in &pair.halves {
        let mut state = socket.inner.lock();
        match state.kind {
            SimSocketKind::Tcp if state.sk_err.is_none() => {
                state.peer_reset = true;
                state.peer_end.get_or_insert(SimFarEnd::Reset);
            }
            SimSocketKind::VsockStream => state.sent_shutdown = Some(SimShutdown::Both),
            SimSocketKind::Tcp | SimSocketKind::Udp => {}
        }
    }
}

fn finish_item(
    context: &mut ContextState,
    id: ForwardId,
    outcome: ForwardOutcome,
    queue_record: bool,
    release: bool,
) -> bool {
    let Some(item) = context.live.remove(&id) else {
        return false;
    };
    context.runs.retain(|(queued, _)| *queued != id);
    let end = match item {
        SimItem::Pair(pair) => {
            if matches!(outcome, ForwardOutcome::Aborted { .. }) {
                abort_pair_sockets(&pair);
            }
            for half in &pair.halves {
                unhook(half);
            }
            if outcome == ForwardOutcome::Completed {
                context.completed += 1;
            }
            if let ForwardOutcome::Aborted { cause } = outcome {
                context.aborted += 1;
                if matches!(cause, AbortCause::Unclassified { .. }) {
                    context.unclassified_eof += 1;
                }
            }
            ForwardEnd {
                id,
                outcome,
                moved_ab: pair.a_to_b.state.moved,
                moved_ba: pair.b_to_a.state.moved,
            }
        }
        SimItem::Watch(socket) => {
            if release {
                socket.inner.lock().sent_shutdown = Some(SimShutdown::Both);
            }
            unhook(&socket);
            if let ForwardOutcome::Aborted { cause } = outcome {
                context.aborted += 1;
                if matches!(cause, AbortCause::Unclassified { .. }) {
                    context.unclassified_eof += 1;
                }
            }
            ForwardEnd { id, outcome, moved_ab: 0, moved_ba: 0 }
        }
    };
    if queue_record {
        queue_end(context, end);
    }
    refresh_summary(context);
    true
}

fn record_socket_end(hook: &Hook, class: SourceEnd, cause: AbortCause) {
    let Some(device) = hook.device.upgrade() else {
        return;
    };
    let mut state = device.lock();
    let Some(context) = state.contexts.get_mut(hook.context_index) else {
        return;
    };
    if !context.summary.open {
        return;
    }
    match hook.kind {
        HookKind::Watch => {
            let outcome = match class {
                SourceEnd::Clean => ForwardOutcome::GuestHalfClose,
                SourceEnd::Abort => ForwardOutcome::Aborted { cause },
            };
            finish_item(context, hook.id, outcome, true, false);
        }
        HookKind::Pair => {
            if class == SourceEnd::Abort
                && let Some(SimItem::Pair(pair)) = context.live.get_mut(&hook.id)
            {
                pair.abort = Some(cause);
            }
            enqueue_run(context, hook.id, PairSide::A);
            enqueue_run(context, hook.id, PairSide::B);
        }
    }
    drop(state);
}

fn schedule_hook(hook: &Hook, source: PairSide) {
    if hook.kind != HookKind::Pair {
        return;
    }
    if let Some(device) = hook.device.upgrade() {
        let mut state = device.lock();
        if let Some(context) = state.contexts.get_mut(hook.context_index)
            && context.summary.open
        {
            enqueue_run(context, hook.id, source);
        }
        drop(state);
    }
}

fn checked_context(
    device: &mut DeviceState,
    index: usize,
    op: ForwarderOp,
) -> Result<&mut ContextState, ForwarderError> {
    let context = device.contexts.get_mut(index).ok_or_else(|| io_error(op, libc::EBADF))?;
    if !context.summary.open {
        return Err(io_error(op, libc::EBADF));
    }
    Ok(context)
}

fn allocate_id(context: &mut ContextState, op: ForwarderOp) -> Result<ForwardId, ForwarderError> {
    let raw = context.next_id;
    if raw == 0 {
        return Err(io_error(op, libc::EOVERFLOW));
    }
    context.next_id = raw.checked_add(1).unwrap_or(0);
    ForwardId::new(raw).map_err(|_| io_error(op, libc::EOVERFLOW))
}

fn install_hook(socket: &SimSocket, hook: Hook) {
    socket.inner.lock().hooked = Some(hook);
}

fn seed_vsock_class(socket: &SimSocket) {
    let mut state = socket.inner.lock();
    if state.kind != SimSocketKind::VsockStream {
        return;
    }
    state.recorded_class = match state.received_shutdown {
        Some(SimShutdown::Send) => Some(SourceEnd::Clean),
        Some(SimShutdown::Both) => Some(SourceEnd::Abort),
        None => None,
    };
}

fn validate_pair(a: &SimSocket, b: &SimSocket, mode: PairMode) -> Result<(), ForwarderError> {
    if Arc::ptr_eq(&a.inner, &b.inner) {
        return Err(io_error(ForwarderOp::Attach, libc::EINVAL));
    }
    let (a_kind, b_kind) = (a.kind(), b.kind());
    match mode {
        PairMode::Stream => {
            let valid = matches!(
                (a_kind, b_kind),
                (SimSocketKind::VsockStream, SimSocketKind::Tcp)
                    | (SimSocketKind::Tcp, SimSocketKind::VsockStream)
            );
            if !valid {
                let side = match (a_kind, b_kind) {
                    (SimSocketKind::Udp, _) | (SimSocketKind::Tcp, SimSocketKind::Tcp) => {
                        PairSide::A
                    }
                    (_, SimSocketKind::Udp)
                    | (SimSocketKind::VsockStream, SimSocketKind::VsockStream) => PairSide::B,
                    _ => unreachable!("the valid stream socket kinds were checked"),
                };
                return Err(ForwarderError::UnsupportedSocket { side });
            }
        }
        PairMode::Datagram { udp } => {
            let (udp_kind, other_kind) = match udp {
                PairSide::A => (a_kind, b_kind),
                PairSide::B => (b_kind, a_kind),
            };
            if udp_kind != SimSocketKind::Udp {
                return Err(ForwarderError::UnsupportedSocket { side: udp });
            }
            if other_kind != SimSocketKind::VsockStream {
                return Err(ForwarderError::UnsupportedSocket {
                    side: match udp {
                        PairSide::A => PairSide::B,
                        PairSide::B => PairSide::A,
                    },
                });
            }
        }
    }
    for (side, socket) in [(PairSide::A, a), (PairSide::B, b)] {
        let state = socket.inner.lock();
        if state.kind == SimSocketKind::Tcp && state.ulp {
            return Err(ForwarderError::UpperLayerProtocol { side });
        }
        if state.kind != SimSocketKind::Udp
            && let Some(errno) = state.sk_err
        {
            return Err(ForwarderError::HalfEnded { side, errno });
        }
        if state.kind == SimSocketKind::VsockStream && state.vsock_state == SimVsockState::Closing {
            return Err(ForwarderError::HalfEnded { side, errno: KernelErrno::ECONNRESET });
        }
        if state.setup != SimSocketSetup::Connected || state.sent_shutdown.is_some() {
            return Err(ForwarderError::NotConnected { side });
        }
        if state.hooked.is_some() {
            return Err(ForwarderError::AlreadyHooked { side });
        }
    }
    if let PairMode::Datagram { .. } = mode {
        let (side, socket) =
            if a_kind == SimSocketKind::VsockStream { (PairSide::A, a) } else { (PairSide::B, b) };
        if socket.inner.lock().buffer_size
            < overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_FRAME
        {
            return Err(ForwarderError::BufferTooSmall { side });
        }
    }
    Ok(())
}

fn validate_watch(vsock: &SimSocket) -> Result<(), ForwarderError> {
    let state = vsock.inner.lock();
    if state.kind != SimSocketKind::VsockStream {
        return Err(ForwarderError::UnsupportedSocket { side: PairSide::A });
    }
    if let Some(errno) = state.sk_err {
        return Err(ForwarderError::HalfEnded { side: PairSide::A, errno });
    }
    if state.vsock_state == SimVsockState::Closing {
        return Err(ForwarderError::HalfEnded {
            side: PairSide::A,
            errno: KernelErrno::ECONNRESET,
        });
    }
    if state.setup != SimSocketSetup::Connected || state.sent_shutdown.is_some() {
        return Err(ForwarderError::NotConnected { side: PairSide::A });
    }
    if state.hooked.is_some() {
        return Err(ForwarderError::AlreadyHooked { side: PairSide::A });
    }
    drop(state);
    Ok(())
}

const fn opposite(side: PairSide) -> PairSide {
    match side {
        PairSide::A => PairSide::B,
        PairSide::B => PairSide::A,
    }
}

const fn side_index(side: PairSide) -> usize {
    match side {
        PairSide::A => 0,
        PairSide::B => 1,
    }
}

fn socket_error(socket: &SimSocket, side: PairSide) -> Option<AbortCause> {
    let state = socket.inner.lock();
    state.sk_err.map(|errno| AbortCause::SocketError { side, errno })
}

fn receive_stream(socket: &SimSocket, max: usize) -> Vec<u8> {
    socket.inner.lock().stream_rx.iter().take(max).copied().collect()
}

fn consume_stream(socket: &SimSocket, bytes: usize) {
    let mut state = socket.inner.lock();
    let count = bytes.min(state.stream_rx.len());
    state.stream_rx.drain(..count);
}

fn accept_stream(socket: &SimSocket, offered: &[u8]) -> Result<usize, KernelErrno> {
    let mut state = socket.inner.lock();
    if let Some(errno) = state.sk_err {
        return Err(errno);
    }
    if state.peer_end == Some(SimFarEnd::Reset) {
        return Err(KernelErrno::ECONNRESET);
    }
    if state.kind == SimSocketKind::VsockStream
        && state.received_shutdown == Some(SimShutdown::Both)
    {
        return Err(errno(32));
    }
    if state.reader_active {
        state.peer_received.extend_from_slice(offered);
        drop(state);
        return Ok(offered.len());
    }
    let capacity = match state.kind {
        SimSocketKind::VsockStream => usize::try_from(state.buffer_size).unwrap_or(usize::MAX),
        SimSocketKind::Tcp | SimSocketKind::Udp => SIM_SOCKET_BUFFER,
    };
    let available = capacity.saturating_sub(state.stream_tx.len());
    let accepted = available.min(offered.len());
    state.stream_tx.extend(offered[..accepted].iter().copied());
    drop(state);
    Ok(accepted)
}

fn accept_datagram(socket: &SimSocket, datagram: Vec<u8>) -> Result<bool, KernelErrno> {
    let mut state = socket.inner.lock();
    if let Some(errno) = state.sk_err {
        return Err(errno);
    }
    if state.peer_end == Some(SimFarEnd::Reset) {
        return Err(KernelErrno::ECONNREFUSED);
    }
    if state.reader_active {
        state.peer_datagrams.push(datagram);
        drop(state);
        return Ok(true);
    }
    let capacity = SIM_SOCKET_BUFFER;
    let occupied = state.datagram_tx.iter().map(Vec::len).sum::<usize>();
    if occupied.saturating_add(datagram.len()) > capacity {
        drop(state);
        return Ok(false);
    }
    state.datagram_tx.push_back(datagram);
    drop(state);
    Ok(true)
}

fn clean_source_end(socket: &SimSocket) -> bool {
    let state = socket.inner.lock();
    match state.kind {
        SimSocketKind::Tcp => state.tcp_state == SimTcpState::CloseWait,
        SimSocketKind::VsockStream => state.recorded_class == Some(SourceEnd::Clean),
        SimSocketKind::Udp => false,
    }
}

fn source_abort(socket: &SimSocket, side: PairSide) -> Option<AbortCause> {
    if let Some(cause) = socket_error(socket, side) {
        return Some(cause);
    }
    let state = socket.inner.lock();
    if state.kind == SimSocketKind::VsockStream && state.recorded_class == Some(SourceEnd::Abort) {
        drop(state);
        return Some(AbortCause::PeerAbort { side });
    }
    drop(state);
    None
}

fn target_half_close(target: &SimSocket) {
    let mut state = target.inner.lock();
    state.sent_shutdown = Some(SimShutdown::Send);
    state.peer_end.get_or_insert(SimFarEnd::Eof);
}

enum DirectionRun {
    Blocked,
    Again,
    End(ForwardOutcome),
}

fn run_stream_direction(pair: &mut SimPair, source: PairSide) -> DirectionRun {
    let source_index = side_index(source);
    let target_index = side_index(opposite(source));
    let source_socket = pair.halves[source_index].clone();
    let target_socket = pair.halves[target_index].clone();
    let direction = match source {
        PairSide::A => &mut pair.a_to_b,
        PairSide::B => &mut pair.b_to_a,
    };
    if direction.state.source_end.is_some() {
        return DirectionRun::Blocked;
    }
    if let Some(cause) = source_abort(&source_socket, source) {
        return DirectionRun::End(ForwardOutcome::Aborted { cause });
    }
    let bytes = receive_stream(&source_socket, 1);
    if !bytes.is_empty() {
        match accept_stream(&target_socket, &bytes) {
            Ok(0) => DirectionRun::Blocked,
            Ok(accepted) => {
                consume_stream(&source_socket, accepted);
                direction.state.moved += u64::try_from(accepted).unwrap_or(u64::MAX);
                if accepted < bytes.len()
                    || source_socket.queued() > 0
                    || clean_source_end(&source_socket)
                {
                    DirectionRun::Again
                } else {
                    DirectionRun::Blocked
                }
            }
            Err(errno) => DirectionRun::End(ForwardOutcome::Aborted {
                cause: AbortCause::SocketError { side: opposite(source), errno },
            }),
        }
    } else if clean_source_end(&source_socket) {
        direction.state.source_end = Some(SourceEnd::Clean);
        target_half_close(&target_socket);
        direction.state.target_shut = true;
        let both_ended = pair.a_to_b.state.source_end == Some(SourceEnd::Clean)
            && pair.b_to_a.state.source_end == Some(SourceEnd::Clean);
        if both_ended {
            DirectionRun::End(ForwardOutcome::Completed)
        } else {
            DirectionRun::Blocked
        }
    } else {
        DirectionRun::Blocked
    }
}

fn frame_total(bytes: &[u8]) -> Result<Option<usize>, ()> {
    if bytes.len() < 8 {
        return Ok(None);
    }
    if bytes.get(..4) != Some(b"OVD1") {
        return Err(());
    }
    let length_bytes: [u8; 4] = bytes.get(4..8).ok_or(())?.try_into().map_err(|_| ())?;
    let payload_len = u32::from_be_bytes(length_bytes);
    if payload_len > overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_PAYLOAD {
        return Err(());
    }
    let total = 8_usize + usize::try_from(payload_len).map_err(|_| ())?;
    Ok((bytes.len() >= total).then_some(total))
}

fn run_udp_to_vsock(
    source_socket: &SimSocket,
    target_socket: &SimSocket,
    direction: &mut PairDirection,
    dgram_oversize: &mut u64,
    source: PairSide,
) -> DirectionRun {
    let (datagram, pending) = {
        let state = source_socket.inner.lock();
        (state.datagram_rx.front().cloned(), direction.pending_frame.clone())
    };
    let Some(datagram) = datagram else {
        return DirectionRun::Blocked;
    };
    if datagram.len() > overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_PAYLOAD as usize {
        source_socket.inner.lock().datagram_rx.pop_front();
        *dgram_oversize = dgram_oversize.saturating_add(1);
        return if source_socket.queued() > 0 {
            DirectionRun::Again
        } else {
            DirectionRun::Blocked
        };
    }
    let (frame, offset) = pending.unwrap_or_else(|| {
        let frame = overdrive_core::guest_transport::wire::DatagramFrame::new(&datagram)
            .unwrap_or_else(|_| unreachable!("the checked datagram is within the frame bound"))
            .encode();
        (frame, 0)
    });
    let unit_end = if offset < 8 { 8.min(frame.len()) } else { offset + 1 };
    let unit = &frame[offset..unit_end];
    let unit_len = unit.len();
    match accept_stream(target_socket, unit) {
        Ok(0) => {
            direction.pending_frame = Some((frame, offset));
            DirectionRun::Blocked
        }
        Ok(accepted) => {
            let next = offset + accepted;
            if next == frame.len() {
                source_socket.inner.lock().datagram_rx.pop_front();
                direction.pending_frame = None;
                direction.state.moved += u64::try_from(datagram.len()).unwrap_or(u64::MAX);
                if source_socket.queued() > 0 { DirectionRun::Again } else { DirectionRun::Blocked }
            } else {
                direction.pending_frame = Some((frame, next));
                if accepted == unit_len { DirectionRun::Again } else { DirectionRun::Blocked }
            }
        }
        Err(errno) => DirectionRun::End(ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: opposite(source), errno },
        }),
    }
}

fn run_vsock_to_udp(
    source_socket: &SimSocket,
    target_socket: &SimSocket,
    direction: &mut PairDirection,
    source: PairSide,
) -> DirectionRun {
    let bytes = receive_stream(source_socket, 65_536);
    if bytes.is_empty() {
        if clean_source_end(source_socket) {
            direction.state.source_end = Some(SourceEnd::Clean);
            direction.state.target_shut = true;
            return DirectionRun::End(ForwardOutcome::Completed);
        }
        return DirectionRun::Blocked;
    }
    let total = match frame_total(&bytes) {
        Ok(Some(total)) => total,
        Ok(None) if clean_source_end(source_socket) => {
            return DirectionRun::End(ForwardOutcome::Aborted {
                cause: AbortCause::FrameInvalid { side: source },
            });
        }
        Ok(None) => return DirectionRun::Blocked,
        Err(()) => {
            return DirectionRun::End(ForwardOutcome::Aborted {
                cause: AbortCause::FrameInvalid { side: source },
            });
        }
    };
    let frame_bytes = &bytes[..total];
    let Ok(frame) = overdrive_core::guest_transport::wire::DatagramFrame::decode(frame_bytes)
    else {
        return DirectionRun::End(ForwardOutcome::Aborted {
            cause: AbortCause::FrameInvalid { side: source },
        });
    };
    let payload = frame.payload().to_vec();
    let payload_len = payload.len();
    match accept_datagram(target_socket, payload) {
        Ok(false) => DirectionRun::Blocked,
        Ok(true) => {
            consume_stream(source_socket, total);
            direction.state.moved += u64::try_from(payload_len).unwrap_or(u64::MAX);
            if source_socket.queued() > 0 { DirectionRun::Again } else { DirectionRun::Blocked }
        }
        Err(errno) => DirectionRun::End(ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: opposite(source), errno },
        }),
    }
}

fn run_datagram_direction(
    pair: &mut SimPair,
    source: PairSide,
    dgram_oversize: &mut u64,
) -> DirectionRun {
    let source_socket = pair.halves[side_index(source)].clone();
    let target_socket = pair.halves[side_index(opposite(source))].clone();
    let direction = match source {
        PairSide::A => &mut pair.a_to_b,
        PairSide::B => &mut pair.b_to_a,
    };
    if direction.state.source_end.is_some() {
        return DirectionRun::Blocked;
    }
    if let Some(cause) = source_abort(&source_socket, source) {
        return DirectionRun::End(ForwardOutcome::Aborted { cause });
    }
    if source_socket.kind() == SimSocketKind::Udp {
        run_udp_to_vsock(&source_socket, &target_socket, direction, dgram_oversize, source)
    } else {
        run_vsock_to_udp(&source_socket, &target_socket, direction, source)
    }
}

fn run_pair(context: &mut ContextState, id: ForwardId, source: PairSide) -> bool {
    let Some(item) = context.live.remove(&id) else {
        return false;
    };
    let SimItem::Pair(mut pair) = item else {
        context.live.insert(id, item);
        return false;
    };
    let result = if let Some(cause) = pair.abort {
        DirectionRun::End(ForwardOutcome::Aborted { cause })
    } else {
        match pair.mode {
            PairMode::Stream => run_stream_direction(&mut pair, source),
            PairMode::Datagram { .. } => {
                run_datagram_direction(&mut pair, source, &mut context.dgram_oversize)
            }
        }
    };
    context.live.insert(id, SimItem::Pair(pair));
    match result {
        DirectionRun::End(outcome) => finish_item(context, id, outcome, true, false),
        DirectionRun::Again => {
            enqueue_run(context, id, source);
            true
        }
        DirectionRun::Blocked => true,
    }
}

impl SimFlowForwarder {
    /// The context's release — the last `close()` of its descriptor, or its
    /// owner's death: aborts every live pair and watch of the context (TCP
    /// halves reset, vsock halves `SHUT_RDWR`, a watched vsock included — the
    /// only operation that shuts a watched socket), queues no record, and
    /// returns once done. Another context's pairs are unaffected.
    pub fn release(self) {
        let mut device = self.device.inner.lock();
        let Some(context) = device.contexts.get_mut(self.index) else {
            return;
        };
        if !context.summary.open {
            return;
        }
        let ids: Vec<ForwardId> = context.live.keys().copied().collect();
        for id in ids {
            let _ = finish_item(
                context,
                id,
                ForwardOutcome::Aborted { cause: AbortCause::Detached },
                false,
                true,
            );
        }
        context.summary.open = false;
        context.runs.clear();
        context.ends.clear();
        refresh_summary(context);
        drop(device);
    }

    /// The directions with a queued run, as `(id, source half)`, in queue
    /// order.
    #[must_use]
    pub fn pending_runs(&self) -> Vec<(ForwardId, PairSide)> {
        self.device
            .inner
            .lock()
            .contexts
            .get(self.index)
            .map(|context| context.runs.iter().copied().collect())
            .unwrap_or_default()
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
    pub fn run(&self, id: ForwardId, source: PairSide) -> bool {
        let mut device = self.device.inner.lock();
        let Some(context) = device.contexts.get_mut(self.index) else {
            return false;
        };
        if !context.summary.open {
            return false;
        }
        let Some(position) = context.runs.iter().position(|run| *run == (id, source)) else {
            return false;
        };
        context.runs.remove(position);
        let ran = run_pair(context, id, source);
        drop(device);
        ran
    }

    /// Runs every queued run until none is queued; returns how many ran.
    pub fn run_until_idle(&self) -> u32 {
        let mut ran = 0_u32;
        loop {
            let mut device = self.device.inner.lock();
            let Some(context) = device.contexts.get_mut(self.index) else {
                return ran;
            };
            if !context.summary.open {
                return ran;
            }
            let Some((id, source)) = context.runs.pop_front() else {
                return ran;
            };
            let _ = run_pair(context, id, source);
            drop(device);
            ran = ran.saturating_add(1);
        }
    }
}

impl FlowForwarder<SimSocket> for SimFlowForwarder {
    fn probe(&self) -> Result<(), ForwarderError> {
        let mut device = self.device.inner.lock();
        checked_context(&mut device, self.index, ForwarderOp::Open).map(|_| ())
    }

    fn attach(
        &self,
        a: &SimSocket,
        b: &SimSocket,
        mode: PairMode,
    ) -> Result<ForwardId, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::Attach)?;
        let id = allocate_id(context, ForwarderOp::Attach)?;
        validate_pair(a, b, mode)?;
        let hook_a = Hook {
            device: Arc::downgrade(&self.device.inner),
            context_index: self.index,
            id,
            side: PairSide::A,
            kind: HookKind::Pair,
        };
        let hook_b = Hook { side: PairSide::B, ..hook_a.clone() };
        let a_shutdown = a.inner.lock().received_shutdown;
        let b_shutdown = b.inner.lock().received_shutdown;
        let initial_abort = match (a.kind(), a_shutdown, b.kind(), b_shutdown) {
            (SimSocketKind::VsockStream, Some(SimShutdown::Both), _, _) => {
                Some(AbortCause::PeerAbort { side: PairSide::A })
            }
            (_, _, SimSocketKind::VsockStream, Some(SimShutdown::Both)) => {
                Some(AbortCause::PeerAbort { side: PairSide::B })
            }
            _ => None,
        };
        install_hook(a, hook_a);
        install_hook(b, hook_b);
        seed_vsock_class(a);
        seed_vsock_class(b);
        context.live.insert(
            id,
            SimItem::Pair(SimPair {
                halves: [a.clone(), b.clone()],
                mode,
                a_to_b: PairDirection::default(),
                b_to_a: PairDirection::default(),
                abort: initial_abort,
            }),
        );
        context.attached += 1;
        refresh_summary(context);
        enqueue_run(context, id, PairSide::A);
        enqueue_run(context, id, PairSide::B);
        drop(device);
        Ok(id)
    }

    fn watch(&self, vsock: &SimSocket) -> Result<ForwardId, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::Watch)?;
        let id = allocate_id(context, ForwarderOp::Watch)?;
        validate_watch(vsock)?;
        install_hook(
            vsock,
            Hook {
                device: Arc::downgrade(&self.device.inner),
                context_index: self.index,
                id,
                side: PairSide::A,
                kind: HookKind::Watch,
            },
        );
        seed_vsock_class(vsock);
        let existing_class = vsock.inner.lock().recorded_class;
        context.live.insert(id, SimItem::Watch(vsock.clone()));
        refresh_summary(context);
        if let Some(class) = existing_class {
            let outcome = match class {
                SourceEnd::Clean => ForwardOutcome::GuestHalfClose,
                SourceEnd::Abort => {
                    ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
                }
            };
            finish_item(context, id, outcome, true, false);
        }
        drop(device);
        Ok(id)
    }

    fn abort(&self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::Abort)?;
        let result = finish_item(
            context,
            id,
            ForwardOutcome::Aborted { cause: AbortCause::Detached },
            true,
            false,
        );
        drop(device);
        Ok(if result { AbortResult::Aborted } else { AbortResult::AlreadyEnded })
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::AbortAll)?;
        let ids: Vec<ForwardId> = context.live.keys().copied().collect();
        let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        for id in ids {
            let _ = finish_item(
                context,
                id,
                ForwardOutcome::Aborted { cause: AbortCause::Detached },
                true,
                false,
            );
        }
        drop(device);
        Ok(count)
    }

    fn take_end(&self) -> Result<Option<ForwardEnd>, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::Read)?;
        let end = context.ends.pop_front();
        if end.is_some() && !context.summary.queued_ends.is_empty() {
            context.summary.queued_ends.remove(0);
        }
        drop(device);
        Ok(end)
    }

    fn pair_state(&self, id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::PairState)?;
        let state = match context.live.get(&id) {
            Some(SimItem::Pair(pair)) => {
                Some(PairState { a_to_b: pair.a_to_b.state, b_to_a: pair.b_to_a.state })
            }
            _ => None,
        };
        drop(device);
        Ok(state)
    }

    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
        let mut device = self.device.inner.lock();
        let context = checked_context(&mut device, self.index, ForwarderOp::Inventory)?;
        let inventory = ForwarderInventory {
            live_pairs: context.summary.live_pairs,
            live_watches: context.summary.live_watches,
            queued_ends: u32::try_from(context.ends.len()).unwrap_or(u32::MAX),
            attached: context.attached,
            completed: context.completed,
            aborted: context.aborted,
            unclassified_eof: context.unclassified_eof,
            dgram_oversize: context.dgram_oversize,
        };
        drop(device);
        Ok(inventory)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimTcpState {
    Established,
    CloseWait,
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent socket facts are the SimSocket arbiter's observable state"
)]
struct SocketState {
    kind: SimSocketKind,
    setup: SimSocketSetup,
    vsock_state: SimVsockState,
    buffer_size: u32,
    reader_active: bool,
    stream_rx: VecDeque<u8>,
    datagram_rx: VecDeque<Vec<u8>>,
    stream_tx: VecDeque<u8>,
    datagram_tx: VecDeque<Vec<u8>>,
    peer_received: Vec<u8>,
    peer_datagrams: Vec<Vec<u8>>,
    peer_end: Option<SimFarEnd>,
    peer_reset: bool,
    owner_closed: bool,
    received_shutdown: Option<SimShutdown>,
    sent_shutdown: Option<SimShutdown>,
    tcp_state: SimTcpState,
    sk_err: Option<KernelErrno>,
    ulp: bool,
    hooked: Option<Hook>,
    recorded_class: Option<SourceEnd>,
}

impl SimSocket {
    /// A socket of `kind` in `setup`. A connected vsock STREAM starts
    /// `ESTABLISHED` with the default buffer size (256 KiB, above
    /// `MAX_GUEST_DATAGRAM_FRAME`), no peer bits, no `sk_err`, a reading far
    /// side and nothing queued.
    #[must_use]
    pub fn new(kind: SimSocketKind, setup: SimSocketSetup) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SocketState {
                kind,
                setup,
                vsock_state: SimVsockState::Established,
                buffer_size: if kind == SimSocketKind::VsockStream {
                    DEFAULT_VSOCK_BUFFER
                } else {
                    0
                },
                reader_active: true,
                stream_rx: VecDeque::new(),
                datagram_rx: VecDeque::new(),
                stream_tx: VecDeque::new(),
                datagram_tx: VecDeque::new(),
                peer_received: Vec::new(),
                peer_datagrams: Vec::new(),
                peer_end: None,
                peer_reset: false,
                owner_closed: false,
                received_shutdown: None,
                sent_shutdown: None,
                tcp_state: SimTcpState::Established,
                sk_err: None,
                ulp: false,
                hooked: None,
                recorded_class: None,
            })),
        }
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
    pub fn peer_send(&self, bytes: &[u8]) {
        let (hook, side) = {
            let mut state = self.inner.lock();
            match state.kind {
                SimSocketKind::Udp => state.datagram_rx.push_back(bytes.to_vec()),
                SimSocketKind::VsockStream | SimSocketKind::Tcp => {
                    state.stream_rx.extend(bytes.iter().copied());
                }
            }
            let hook = state.hooked.clone();
            let side = hook.as_ref().map(|hook| hook.side);
            drop(state);
            (hook, side)
        };
        if let (Some(hook), Some(side)) = (hook, side) {
            schedule_hook(&hook, side);
        }
    }

    /// The far peer ends its direction: on a vsock STREAM a SHUTDOWN with
    /// `how`'s flags, recorded in `peer_shutdown` and classified by a hooked
    /// half's `sk_state_change` in this call; on TCP, `Send` is a FIN
    /// (`CLOSE_WAIT` once received). `Both` on TCP is a precondition
    /// violation.
    pub fn peer_shutdown(&self, how: SimShutdown) {
        let (hook, class) = {
            let mut state = self.inner.lock();
            match state.kind {
                SimSocketKind::Tcp => {
                    assert_eq!(how, SimShutdown::Send, "TCP stimulus is a FIN");
                    state.tcp_state = SimTcpState::CloseWait;
                    (state.hooked.clone(), None)
                }
                SimSocketKind::VsockStream => {
                    let old = state.received_shutdown;
                    let class = match (old, how) {
                        (None, SimShutdown::Send) => Some(SourceEnd::Clean),
                        (None, SimShutdown::Both) => Some(SourceEnd::Abort),
                        (Some(SimShutdown::Send), SimShutdown::Both)
                            if state.sent_shutdown.is_none() =>
                        {
                            Some(SourceEnd::Abort)
                        }
                        _ => None,
                    };
                    state.received_shutdown = match (old, how) {
                        (Some(SimShutdown::Both), _) | (_, SimShutdown::Both) => {
                            Some(SimShutdown::Both)
                        }
                        (_, SimShutdown::Send) => Some(SimShutdown::Send),
                    };
                    if how == SimShutdown::Both && state.stream_rx.is_empty() {
                        state.vsock_state = SimVsockState::Closing;
                    }
                    if let Some(class) = class
                        && state.hooked.is_some()
                        && state.recorded_class.is_none()
                    {
                        state.recorded_class = Some(class);
                    }
                    (state.hooked.clone(), class)
                }
                SimSocketKind::Udp => panic!("UDP has no shutdown end"),
            }
        };
        if let Some(hook) = hook {
            match class {
                Some(class) => {
                    record_socket_end(&hook, class, AbortCause::PeerAbort { side: hook.side });
                }
                None => schedule_hook(&hook, hook.side),
            }
        }
    }

    /// The far peer resets the connection: TCP — `sk_err` becomes
    /// `ECONNRESET`, or `EPIPE` in `CLOSE_WAIT`, reported through a hooked
    /// half's `sk_error_report` in this call; vsock — an `RST`, leaving
    /// `CLOSING` when nothing is queued and `ESTABLISHED` with every peer bit
    /// otherwise, classified in this call.
    pub fn peer_reset(&self) {
        match self.kind() {
            SimSocketKind::VsockStream => self.peer_shutdown(SimShutdown::Both),
            SimSocketKind::Tcp => {
                {
                    let mut state = self.inner.lock();
                    let errno = if state.tcp_state == SimTcpState::CloseWait {
                        errno(32)
                    } else {
                        KernelErrno::ECONNRESET
                    };
                    state.sk_err = Some(errno);
                }
                self.report_error();
            }
            SimSocketKind::Udp => {
                self.inner.lock().sk_err = Some(KernelErrno::ECONNREFUSED);
                self.report_error();
            }
        }
    }

    /// The far reader stops reading: once the far side's buffer is full the
    /// target accepts nothing more, and the sender blocks within its buffer.
    pub fn peer_stop_reading(&self) {
        self.inner.lock().reader_active = false;
    }

    /// The far reader reads again: everything queued toward it is delivered
    /// in this call (one far-side read), and queued runs resume.
    pub fn peer_resume_reading(&self) {
        let (hook, side) = {
            let mut state = self.inner.lock();
            state.reader_active = true;
            while let Some(byte) = state.stream_tx.pop_front() {
                state.peer_received.push(byte);
            }
            while let Some(datagram) = state.datagram_tx.pop_front() {
                state.peer_datagrams.push(datagram);
            }
            let hook = state.hooked.clone();
            let source = hook.as_ref().map(|hook| opposite(hook.side));
            drop(state);
            (hook, source)
        };
        if let (Some(hook), Some(source)) = (hook, side) {
            schedule_hook(&hook, source);
        }
    }

    // -- kernel facts of this socket (stimulus) ------------------------------

    /// Sets `sk_err` without reporting it — on a vsock, the first of the
    /// device release's orphan-sweep steps (with the state and peer bits);
    /// [`Self::report_error`] is the second.
    pub fn set_sk_err(&self, errno: KernelErrno) {
        let mut state = self.inner.lock();
        state.sk_err = Some(errno);
        if state.kind == SimSocketKind::VsockStream {
            state.received_shutdown = Some(SimShutdown::Both);
        }
    }

    /// Runs the socket's `sk_error_report` for its pending `sk_err` (a
    /// hooked half aborts its pair; a watch queues its record first).
    pub fn report_error(&self) {
        let (hook, errno) = {
            let state = self.inner.lock();
            (state.hooked.clone(), state.sk_err)
        };
        let (Some(hook), Some(errno)) = (hook, errno) else {
            return;
        };
        record_socket_end(
            &hook,
            SourceEnd::Abort,
            AbortCause::SocketError { side: hook.side, errno },
        );
    }

    /// Sets a vsock STREAM's state directly.
    pub fn set_vsock_state(&self, state: SimVsockState) {
        self.inner.lock().vsock_state = state;
    }

    /// This side shuts its own send direction (`SHUT_WR`; a vsock sends
    /// `0x2`). On a socket not yet attached it makes the socket an owner
    /// defect for attach (`NotConnected`).
    pub fn shutdown_send(&self) {
        let mut state = self.inner.lock();
        state.sent_shutdown = Some(SimShutdown::Send);
    }

    /// The local owner closes this socket. TCP exposes the first terminal
    /// event its far peer can observe (reset for abort/linger, EOF otherwise);
    /// a vsock STREAM sends `SHUTDOWN{S,R}`. A peer that already reset the TCP
    /// half cannot observe a later local reset.
    pub fn close_from_owner(&self, reset: bool) {
        let mut state = self.inner.lock();
        state.owner_closed = true;
        match state.kind {
            SimSocketKind::Tcp if state.sk_err.is_none() => {
                let end = if reset { SimFarEnd::Reset } else { SimFarEnd::Eof };
                state.peer_reset |= reset;
                state.peer_end.get_or_insert(end);
            }
            SimSocketKind::VsockStream => state.sent_shutdown = Some(SimShutdown::Both),
            SimSocketKind::Tcp | SimSocketKind::Udp => {}
        }
    }

    /// Sets a vsock STREAM's `buffer_size`.
    pub fn set_buffer_size(&self, bytes: u32) {
        self.inner.lock().buffer_size = bytes;
    }

    /// Installs an upper-layer protocol on a TCP socket (kTLS).
    pub fn set_ulp(&self) {
        self.inner.lock().ulp = true;
    }

    /// The socket's local holder reads up to `max` bytes (a watched vsock's
    /// holder, never the forwarder); on a UDP socket one datagram per call.
    pub fn local_read(&self, max: usize) -> SimLocalRead {
        let mut state = self.inner.lock();
        let result = if state.kind == SimSocketKind::Udp {
            state.datagram_rx.pop_front().map_or(SimLocalRead::WouldBlock, SimLocalRead::Bytes)
        } else if !state.stream_rx.is_empty() {
            let n = max.min(state.stream_rx.len());
            SimLocalRead::Bytes(state.stream_rx.drain(..n).collect())
        } else if let Some(errno) = state.sk_err {
            SimLocalRead::Err(errno)
        } else if state.kind == SimSocketKind::VsockStream
            && state.vsock_state == SimVsockState::Closing
        {
            SimLocalRead::Err(errno(107))
        } else if (state.kind == SimSocketKind::VsockStream && state.received_shutdown.is_some())
            || (state.kind == SimSocketKind::Tcp && state.tcp_state == SimTcpState::CloseWait)
        {
            SimLocalRead::Eof
        } else if state.setup != SimSocketSetup::Connected {
            SimLocalRead::Err(errno(107))
        } else {
            SimLocalRead::WouldBlock
        };
        drop(state);
        result
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
        self.inner.lock().peer_received.clone()
    }

    /// Stream bytes queued from the far peer for the local holder to read.
    #[must_use]
    pub fn stream_input(&self) -> Vec<u8> {
        self.inner.lock().stream_rx.iter().copied().collect()
    }

    /// Stream bytes accepted locally and queued for the far peer to read.
    #[must_use]
    pub fn stream_output(&self) -> Vec<u8> {
        self.inner.lock().stream_tx.iter().copied().collect()
    }

    /// The datagrams the far side has received, in order, empty ones
    /// included (UDP).
    #[must_use]
    pub fn peer_datagrams(&self) -> Vec<Vec<u8>> {
        self.inner.lock().peer_datagrams.clone()
    }

    /// The far end's first terminal event, once it has happened.
    #[must_use]
    pub fn peer_end(&self) -> Option<SimFarEnd> {
        self.inner.lock().peer_end
    }

    /// The local TCP half sent a reset its still-live far peer can observe,
    /// including a reset after an earlier EOF observation.
    #[must_use]
    pub fn peer_was_reset(&self) -> bool {
        self.inner.lock().peer_reset
    }

    /// The socket's owning userspace descriptor was closed, distinct from a
    /// forwarder abort that only shuts/resets the transport half.
    #[must_use]
    pub fn closed_by_owner(&self) -> bool {
        self.inner.lock().owner_closed
    }

    /// The SHUTDOWN this side sent on a vsock STREAM, the strongest once
    /// (`0x3` after `0x2` reports `Both`).
    #[must_use]
    pub fn sent_shutdown(&self) -> Option<SimShutdown> {
        self.inner.lock().sent_shutdown
    }

    /// The SHUTDOWN flags this vsock STREAM received from its peer
    /// (`peer_shutdown`), the strongest once: `Send` for `0x2`, `Both` once
    /// `RCV` is set too (a `0x3`, or an `RST` that set every peer bit).
    #[must_use]
    pub fn received_shutdown(&self) -> Option<SimShutdown> {
        self.inner.lock().received_shutdown
    }

    /// The class the forwarder recorded for the peer's end of this vsock
    /// half, once a transition was classified (at attach or by a callback).
    #[must_use]
    pub fn recorded_class(&self) -> Option<SourceEnd> {
        self.inner.lock().recorded_class
    }

    /// The pending `sk_err`.
    #[must_use]
    pub fn sk_err(&self) -> Option<KernelErrno> {
        self.inner.lock().sk_err
    }

    /// A vsock STREAM's state.
    #[must_use]
    pub fn vsock_state(&self) -> SimVsockState {
        self.inner.lock().vsock_state
    }

    /// This side has shut its own send direction.
    #[must_use]
    pub fn own_send_shut(&self) -> bool {
        self.inner.lock().sent_shutdown.is_some()
    }

    /// The socket is attached or watched by some context (`sk_user_data`
    /// set).
    #[must_use]
    pub fn is_hooked(&self) -> bool {
        self.inner.lock().hooked.is_some()
    }

    /// Bytes (UDP: datagrams) queued on this socket and not yet consumed.
    #[must_use]
    pub fn queued(&self) -> usize {
        let state = self.inner.lock();
        state.stream_rx.len()
            + state.stream_tx.len()
            + state.datagram_rx.iter().map(Vec::len).sum::<usize>()
            + state.datagram_tx.iter().map(Vec::len).sum::<usize>()
    }
}
