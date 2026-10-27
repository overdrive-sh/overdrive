//! `SimGuestFlowIo` — the owner seam's simulation adapter (DG-3, QPROJ-7,
//! QPROJ-8).
//!
//! `#[cfg(test)]`, crate-private, beside the seam (QPROJ-1 rule 3: it names
//! the owner crate's own types). It holds every async effect the owner
//! issues (connect, bind, the forwarder open, a route write, a close, …)
//! until the driver releases or fails it, and every input event (an accept, a
//! record's bytes, an end record on the forwarder context) until the driver
//! delivers it; it reports when every owner task waiting on it is parked with
//! nothing undelivered ([`SimGuestFlowIo::settled`], on a paused
//! current-thread runtime).
//!
//! **Arbiters.** The read surfaces are the QPROJ-7 contract:
//! [`SimGuestFlowIo::socket_observation`] ([`SimOwnerSocketObservation`],
//! field names and meanings pinned by the design). Every forwarding method
//! calls the `overdrive-sim` [`SimFlowForwarder`] context the seam holds,
//! opened from the seam's own [`SimForwarderDevice`]; that context is the
//! named arbiter of the forwarder's kernel facts (per-direction state, end
//! records, live pairs and watches, the context table; QPROJ-8), and the
//! [`SimSocket`]s the seam issues are the arbiters of the per-half socket
//! facts (queues, peer shutdown bits, `sk_err`, the far reader). The
//! `forwarder_*` read methods lend those facts to a driver's marked
//! projection. Their Tier-3 pair is the module's suite through
//! `KernelFlowForwarder` (V-EV-END, V-EV-CTL, V-EV-PAIR, V-EV-DGRAM) and, for
//! the accept path, R5-59(a).
//!
//! **Causes, not consequences** (`.claude/rules/testing.md` § "DISTILL —
//! prove composed system behavior"). A test never queues an end record: it
//! scripts the cause on a socket — the guest's `0x2` / `0x3` / `RST` on the
//! flow vsock, the destination's FIN or reset, a stalled reader, the device
//! release's `sk_err` — and the sim forwarder classifies it, runs the
//! directions and queues the record as `KernelFlowForwarder` would. With
//! auto-run (the default) every scripted cause and every attach is followed
//! by the forwarder's runs until idle; a driver that steps the runs itself
//! turns auto-run off ([`SimGuestFlowIo::set_auto_run`]).
//!
//! **Accept order (D-UP-19).** Pending connections wait outside the
//! listener's accept queue with their peer CID and buffered first record and
//! are released into it at the step that accepts them: one at a time by the
//! driver ([`SimGuestFlowIo::release_accept`]), or, with auto-delivery, in a
//! seeded order among those queued (the kernel's FIFO is one of the orders
//! the seed produces).
//!
//! **Streams.** The `GuestStream` a dial returns reads its dial's `V_h`
//! ([`SimSocket::local_read`], the holder's read of a watched vsock) and
//! resolves every end it observes from the facts the seam saw
//! (EVMOD-STREAM-END): the owner's own abort of the watch (`abort_forward` /
//! `abort_all`), else the watch's end record as the owner consumed it
//! (`next_end`), else it waits — the same facts `HostGuestFlowIo` sees in
//! production. The driver plays the guest application on the far side of
//! `V_h` ([`SimGuestFlowIo::stream_guest_write`],
//! [`SimGuestFlowIo::guest_ends_stream`]).
//!
//! The sim enforces the real adapter's preconditions: a handle of another
//! seam instance, a forwarding call with no context held, and an `accept` on
//! an unbound listener are `unreachable!`; `attach` is all-or-nothing; a
//! handle left open when the seam drops without a process exit is reported
//! (panic) as an owner defect.

#![allow(
    clippy::struct_excessive_bools,
    clippy::redundant_pub_crate,
    reason = "QPROJ-7 pins `pub(crate)` visibility for the sim's read surface"
)]
#![allow(
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]
#![allow(
    dead_code,
    reason = "test support shared by the default-lane bodies and the integration-tests conformance drivers; each lane uses a subset"
)]
#![allow(
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    clippy::missing_panics_doc,
    reason = "the sim applies each kernel step under one lock acquisition by design (atomic kernel steps)"
)]

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overdrive_core::guest_transport::dial::{GuestStream, GuestStreamEnd, GuestStreamIo};
use overdrive_core::guest_transport::forwarder::{
    FlowForwarder, ForwardOutcome, ForwarderOp, ForwarderProbeObservation, ForwarderProbeStage,
    PairSide,
};
use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, ControlOpen, FLOW_REQUEST_LEN, FlowId,
    FlowRequest, KernelErrno,
};
use overdrive_core::guest_transport::{GuestCid, LaunchToken, VsockListenPort};
use overdrive_core::vm::config::VsockPort;

/// The first port a kernel-assigned sim bind takes (above the reserved
/// range, as `VMADDR_PORT_ANY` binds are).
const SIM_FIRST_ASSIGNED_PORT: u32 = 49_152;
use overdrive_netlink::{FibRouteType, NetlinkError};
use overdrive_sim::adapters::{
    SimContextState, SimFarEnd, SimFlowForwarder, SimForwarderDevice, SimLocalRead, SimShutdown,
    SimSocket, SimSocketKind, SimVsockState,
};
use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

use super::{
    AbortResult, AttachRefused, AttachedPair, FlowIoError, FlowListener, ForwardEnd, ForwardId,
    ForwarderError, ForwarderGeneration, ForwarderInventory, GuestFlowIo, GuestHostFacts,
    OwnedSocket, PairMode, PairSocket, PairState, PeerProtocol, PeerSocket, PeerSocketSpec,
    RecordSocket, SeamHandle,
};
use crate::guest_network::{GuestConnKey, GuestNetworkError, GuestNetworkOperation};

// ===========================================================================
// QPROJ-7 read surface (the contract)
// ===========================================================================

/// An outer far end's first terminal event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FarEndEvent {
    /// A clean FIN (the forwarder carried the source's clean end, or the
    /// owner closed the socket with linger cleared).
    Eof,
    /// A reset (`SO_LINGER{1,0}`, or the forwarder's abort of the pair).
    Reset,
}

/// QPROJ-7: the socket facts of one seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimOwnerSocketObservation {
    /// Accept-queued connections and their buffered first record's token
    /// (`None`: nothing buffered).
    pub(crate) queued: BTreeMap<GuestConnKey, Option<LaunchToken>>,
    /// Connections the owner closed; no longer in `flow_observation()`.
    pub(crate) closed_by_owner: BTreeSet<GuestConnKey>,
    /// Accepted; peer VM dead, not yet observed.
    pub(crate) peer_gone: BTreeSet<GuestConnKey>,
    /// Each outer far end's first terminal event.
    pub(crate) far_end: BTreeMap<FlowId, FarEndEvent>,
}

// ===========================================================================
// Test-support vocabulary (DISTILL's)
// ===========================================================================

/// A vsock SHUTDOWN the host sent toward the guest on a dial's `V_h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VsockShutdownOp {
    /// Flags `0x2` (SEND only): the dialer's `shutdown_write`.
    HalfClose,
    /// Flags `0x3` (RCV | SEND): a close of `V_h`.
    Close,
}

/// Identifies one held effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SimEffectId(u64);

/// The kind of an effect the owner issues through the seam. The
/// synchronous forwarding calls (`CloseForwarder`, `Watch`, `Attach`,
/// `AbortForward`, `AbortAll`, `Inventory`) cannot be held; they are
/// recorded and may be scripted to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SimEffectKind {
    HostFacts,
    /// Phase 1's forwarder probe on a context of its own (D-8).
    ProbeForwarder,
    OpenForwarder,
    CloseForwarder,
    BindListener,
    GatewayRoute,
    StaleObjects,
    WriteRecord,
    ConnectGuest,
    Watch,
    OpenPeer,
    ConnectUdp,
    ConnectPeer,
    RouteType,
    Attach,
    AbortForward,
    AbortAll,
    /// `inventory()` (the owner's audit read; a scripted `ContextIo` damages
    /// the `Forwarder` component).
    Inventory,
    ClearLinger,
    Abort,
    Close,
}

impl SimEffectKind {
    /// The synchronous seam calls: recorded and scriptable, never held.
    const fn is_sync(self) -> bool {
        matches!(
            self,
            Self::CloseForwarder
                | Self::Watch
                | Self::Attach
                | Self::AbortForward
                | Self::AbortAll
                | Self::Inventory
        )
    }
}

/// What one effect targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimEffectTarget {
    /// Nothing beyond the kind.
    None,
    /// A flow listener.
    Listener(FlowListener),
    /// A guest CID.
    Cid(GuestCid),
    /// A socket address.
    Addr(SocketAddrV4),
    /// An IPv4 destination.
    Ip(Ipv4Addr),
    /// A peer socket spec.
    Peer(PeerSocketSpec),
    /// The connection a record write goes to (`None`: a dial's `V_h`).
    Conn(Option<GuestConnKey>),
    /// The flow a socket belongs to, when the sim knows it.
    Flow(Option<FlowId>),
    /// A forwarder id of the current context.
    Forward(ForwardId),
}

/// One effect the owner issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimEffect {
    /// The effect's kind.
    pub(crate) kind: SimEffectKind,
    /// What it targets.
    pub(crate) target: SimEffectTarget,
}

/// A scripted forwarder failure (UI-COV-1): the `ForwarderError` an
/// `open_forwarder`, `attach`, `watch` or context call returns. A
/// `HalfEnded` refusal is never scripted: it is the sim forwarder's answer
/// to a half whose scripted socket facts show it ended (EVMOD-ATTACH-END).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimForwarderFailure {
    /// `/dev/ovd_fwd` cannot be opened (the module is absent).
    DeviceAbsent,
    /// The module speaks another ABI.
    AbiMismatch,
    /// A probe stage observed the wrong behaviour.
    ProbeMismatch(ForwarderProbeStage, ForwarderProbeObservation),
    /// Owner defects at attach.
    UnsupportedSocket(PairSide),
    UpperLayerProtocol(PairSide),
    NotConnected(PairSide),
    AlreadyHooked(PairSide),
    BufferTooSmall(PairSide),
    /// A control call on the context failed (`Io`): the `Forwarder`
    /// component is damaged.
    ContextIo(ForwarderOp),
}

impl SimForwarderFailure {
    fn error(self) -> ForwarderError {
        match self {
            Self::DeviceAbsent => {
                ForwarderError::DeviceAbsent { source: io::Error::from_raw_os_error(libc::ENOENT) }
            }
            Self::AbiMismatch => ForwarderError::AbiMismatch { expected: 1, found: 2 },
            Self::ProbeMismatch(stage, observed) => {
                ForwarderError::ProbeMismatch { stage, observed }
            }
            Self::UnsupportedSocket(side) => ForwarderError::UnsupportedSocket { side },
            Self::UpperLayerProtocol(side) => ForwarderError::UpperLayerProtocol { side },
            Self::NotConnected(side) => ForwarderError::NotConnected { side },
            Self::AlreadyHooked(side) => ForwarderError::AlreadyHooked { side },
            Self::BufferTooSmall(side) => ForwarderError::BufferTooSmall { side },
            Self::ContextIo(op) => {
                ForwarderError::Io { op, source: io::Error::from_raw_os_error(libc::EIO) }
            }
        }
    }
}

/// A failure the driver gives a held (or scripts for an auto-released)
/// effect. It must match the effect's error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimFailure {
    /// A socket syscall (`FlowIoError::Io`) or, for `host_facts` /
    /// `stale_objects_present`, `GuestNetworkError::Io`.
    Io(io::ErrorKind),
    /// A forwarder failure.
    Forwarder(SimForwarderFailure),
    /// The gateway route write (`GuestNetworkError::Netlink { LocalRouteConverge }`).
    Netlink,
}

/// How a peer ends a connection the owner reads records on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPeerEnd {
    /// A clean end of stream.
    Eof,
    /// A reset.
    Reset,
}

/// The kernel-permitted result selected for one holder read after the local
/// owner shut a watched vsock both ways. Native V-EV-CTL observes both results
/// from this exact state; the choice is an input to the conformance step, not
/// persistent socket or model state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerShutdownRead {
    /// `recvmsg` returned zero while the socket was still established.
    Eof,
    /// `recvmsg` failed `ENOTCONN` after the local shutdown transition.
    NotConnected,
}

/// What a forwarder id the sim issued carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPairKind {
    /// A guest-opened TCP flow (TcpConnect): `V_h` ↔ D (or leg-F's L).
    Tcp,
    /// A datagram association: `V_h` ↔ H.
    Datagram,
    /// A dial's `V_h`, watched, never paired.
    Watch,
}

/// One forwarder id, as the sim names it (test support, never a projection
/// source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimPairInfo {
    /// What it carries.
    pub(crate) kind: SimPairKind,
    /// The flow it belongs to, from the request the sim saw.
    pub(crate) flow: Option<FlowId>,
    /// The guest CID of the flow.
    pub(crate) cid: GuestCid,
    /// The context generation it was issued in.
    pub(crate) generation: ForwarderGeneration,
}

/// A dial's `TcpAccept` request as the owner wrote it on `V_h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimDialRecord {
    /// The CID the owner connected `V_h` to.
    pub(crate) cid: GuestCid,
    /// The forwarder watch on `V_h`, once the owner made it.
    pub(crate) watch: Option<ForwardId>,
    /// The decoded request (`None` until written).
    pub(crate) request: Option<FlowRequest>,
    /// The owner closed or aborted `V_h` (or handed it off and the stream was
    /// dropped).
    pub(crate) closed: bool,
    /// `V_h` was handed to the dialer as a `GuestStream`.
    pub(crate) streamed: bool,
}

/// The guest's end of a dialled stream's direction, as the guest
/// application and its pair produce it on `V_g` (the far side of `V_h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimGuestStreamEnd {
    /// The application half-closed: `0x2` reaches `V_h`; the watch records
    /// `GuestHalfClose`.
    HalfClose,
    /// The guest pair aborted with a direction open: `0x3` reaches `V_h`;
    /// the watch records `Aborted { PeerAbort }`.
    Abort,
    /// The application's reset reached `V_h` as an `RST`.
    Reset,
}

/// The dialer side of a handed-off `GuestStream`, as the sim saw it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SimStreamObservation {
    /// Bytes the dialer wrote toward the guest.
    pub(crate) to_guest: Vec<u8>,
    /// The dialer shut its direction down.
    pub(crate) host_shut: bool,
    /// The dialer dropped the stream.
    pub(crate) dropped: bool,
}

// ===========================================================================
// Per-seam state
// ===========================================================================

struct Held {
    effect: SimEffect,
    decide: oneshot::Sender<Option<SimFailure>>,
}

#[derive(Debug, Default)]
struct ListenerState {
    bound: bool,
    /// The port the held socket bound (PORTS-CONFIG); kept for the
    /// process's life, so a repair reports it again.
    port: Option<VsockPort>,
    failed: Option<io::ErrorKind>,
    queue: VecDeque<QueuedConn>,
}

#[derive(Debug)]
struct QueuedConn {
    key: GuestConnKey,
    /// Trusted fixture identity of the VMM process that opened this socket.
    /// Kept separate from the untrusted first-record launch token.
    physical_launch: Option<LaunchToken>,
    record: Vec<u8>,
    released: bool,
    /// The peer VM died while queued: buffered bytes, then this end.
    end: Option<SimPeerEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordOrigin {
    Accepted(FlowListener),
    Dialed,
}

struct RecordState {
    origin: RecordOrigin,
    conn: Option<GuestConnKey>,
    peer_cid: GuestCid,
    /// [`QueuedConn::physical_launch`], preserved across accept.
    physical_launch: Option<LaunchToken>,
    first_record: Vec<u8>,
    buffered: VecDeque<u8>,
    ready: VecDeque<u8>,
    end: Option<SimPeerEnd>,
    end_released: bool,
    peer_gone: bool,
    /// The owner may observe `peer_gone` (auto-delivery, or released by
    /// [`SimGuestFlowIo::release_peer_gone`]).
    peer_gone_released: bool,
    /// A dial: its flow id, once its request was written.
    dial_flow: Option<FlowId>,
    /// The vsock socket the forwarder sees (after the record bytes).
    vsock: SimSocket,
}

struct PeerState {
    spec: PeerSocketSpec,
    local: Option<SocketAddrV4>,
    peer: Option<SocketAddrV4>,
    /// The flow the peer was attached for (learnt at `attach`).
    flow: Option<FlowId>,
    socket: SimSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairRole {
    Vsock,
    Tcp { linger: bool },
    Udp,
}

struct PairSocketState {
    id: ForwardId,
    generation: ForwarderGeneration,
    role: PairRole,
    conn: Option<GuestConnKey>,
    flow: Option<FlowId>,
    linger_cleared: bool,
    socket: SimSocket,
}

enum HandleState {
    Record(RecordState),
    Peer(PeerState),
    Pair(PairSocketState),
}

/// The context the seam holds.
struct Context {
    ctx: Arc<SimFlowForwarder>,
    generation: ForwarderGeneration,
}

/// The sockets of one flow, for scripting causes (test support).
#[derive(Debug, Clone)]
struct FlowSockets {
    cid: GuestCid,
    vsock: SimSocket,
    peer: Option<(PeerProtocol, SimSocket)>,
}

#[derive(Debug)]
struct StreamPipe {
    generation: ForwarderGeneration,
    watch: ForwardId,
    vsock: SimSocket,
    to_guest: Vec<u8>,
    host_shut: bool,
    dropped: bool,
}

struct SimState {
    exited: bool,
    auto: BTreeSet<SimEffectKind>,
    scripts: BTreeMap<SimEffectKind, VecDeque<SimFailure>>,
    held: BTreeMap<SimEffectId, Held>,
    issued: Vec<SimEffect>,
    next_effect: u64,
    next_handle: u64,
    next_port: u16,
    /// xorshift state choosing the auto-delivered accept order.
    rng: u64,
    /// Accept and record bytes reach the owner without a release.
    auto_deliver: bool,
    /// End records reach the owner without a release.
    auto_ends: bool,
    released_ends: usize,
    /// Every scripted cause and attach is followed by the forwarder's runs.
    auto_run: bool,
    host_facts: GuestHostFacts,
    stale_objects: bool,
    gateway_route: Option<Ipv4Addr>,
    /// This machine's `/dev/ovd_fwd`; its context table is the `oldCtx`
    /// arbiter (QPROJ-8).
    device: SimForwarderDevice,
    context: Option<Context>,
    /// The last generation opened (the next open is its `next()`).
    last_generation: Option<ForwarderGeneration>,
    listeners: BTreeMap<FlowListener, ListenerState>,
    /// The next port a kernel-assigned bind takes (the sim's stand-in for
    /// the kernel's `VMADDR_PORT_ANY` counter; deterministic per seam).
    next_assigned_port: u32,
    handles: BTreeMap<u64, HandleState>,
    route_types: BTreeMap<Ipv4Addr, FibRouteType>,
    connect4: BTreeMap<SocketAddrV4, SocketAddrV4>,
    vm_gone: BTreeSet<GuestCid>,
    closed_by_owner: BTreeSet<GuestConnKey>,
    /// Far-end events of the owner's own closes of sockets the forwarder
    /// never held (an unattached peer aborted or closed, a process exit).
    local_far_end: BTreeMap<FlowId, FarEndEvent>,
    to_guest: BTreeMap<FlowId, Vec<VsockShutdownOp>>,
    outbox: BTreeMap<GuestCid, Vec<ControlMessage>>,
    dials: BTreeMap<u64, SimDialRecord>,
    peer_connects: Vec<SocketAddrV4>,
    /// Every host vsock the sim made, by its guest CID and trusted physical
    /// launch (`None` for a host dial). A stepped device release reaches only
    /// the stopped launch; the low-level CID-wide helper reaches every row.
    cid_vsocks: Vec<(GuestCid, Option<LaunchToken>, SimSocket)>,
    flow_sockets: BTreeMap<FlowId, FlowSockets>,
    /// Live ids the sim issued, by (generation, id).
    pairs: BTreeMap<(ForwarderGeneration, ForwardId), SimPairInfo>,
    /// Ids the owner aborted (`abort_forward`, `abort_all`).
    owner_aborted: BTreeSet<(ForwarderGeneration, ForwardId)>,
    /// End records the owner consumed (`next_end`), by id.
    ends_taken: BTreeMap<(ForwarderGeneration, ForwardId), ForwardOutcome>,
    streams: BTreeMap<FlowId, Arc<Mutex<StreamPipe>>>,
}

impl std::fmt::Debug for SimState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimState").finish_non_exhaustive()
    }
}

/// The owner seam's simulation adapter (DG-3).
pub(crate) struct SimGuestFlowIo {
    seam: u64,
    state: Arc<Mutex<SimState>>,
    changed: Arc<Notify>,
}

impl std::fmt::Debug for SimGuestFlowIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimGuestFlowIo").field("seam", &self.seam).finish_non_exhaustive()
    }
}

static NEXT_SEAM: AtomicU64 = AtomicU64::new(1);

fn io_error(kind: io::ErrorKind) -> FlowIoError {
    FlowIoError::Io { source: io::Error::from(kind) }
}

fn errno_error(errno: KernelErrno) -> io::Error {
    io::Error::from_raw_os_error(i32::from(errno.get()))
}

fn mismatched(effect: &str, failure: SimFailure) -> ! {
    panic!("test bug: scripted failure {failure:?} does not match the {effect} effect")
}

fn request_of(bytes: &[u8]) -> Option<FlowRequest> {
    bytes
        .get(..FLOW_REQUEST_LEN)
        .and_then(|b| <&[u8; FLOW_REQUEST_LEN]>::try_from(b).ok())
        .and_then(|b| FlowRequest::decode(b).ok())
}

fn far_end_of(end: SimFarEnd) -> FarEndEvent {
    match end {
        SimFarEnd::Eof => FarEndEvent::Eof,
        SimFarEnd::Reset => FarEndEvent::Reset,
    }
}

impl SimGuestFlowIo {
    /// A fresh seam: no listener bound, no forwarder context, no route; host
    /// facts with no interface, `RLIMIT_NOFILE` 1,048,576 and the owner in
    /// `/sys/fs/cgroup/overdrive.slice/control-plane.slice`; every async
    /// effect auto-released; accepts, record bytes and end records delivered
    /// without a release, the forwarder run to idle after every cause, the
    /// accept order drawn from `seed`.
    pub(crate) fn new(seed: u64) -> Arc<Self> {
        let seam = NEXT_SEAM.fetch_add(1, Ordering::SeqCst);
        let auto = [
            SimEffectKind::HostFacts,
            SimEffectKind::OpenForwarder,
            SimEffectKind::BindListener,
            SimEffectKind::GatewayRoute,
            SimEffectKind::StaleObjects,
            SimEffectKind::WriteRecord,
            SimEffectKind::ConnectGuest,
            SimEffectKind::OpenPeer,
            SimEffectKind::ConnectUdp,
            SimEffectKind::ConnectPeer,
            SimEffectKind::RouteType,
            SimEffectKind::ClearLinger,
            SimEffectKind::Abort,
            SimEffectKind::Close,
        ]
        .into_iter()
        .collect();
        Arc::new(Self {
            seam,
            state: Arc::new(Mutex::new(SimState {
                exited: false,
                auto,
                scripts: BTreeMap::new(),
                held: BTreeMap::new(),
                issued: Vec::new(),
                next_effect: 1,
                next_handle: 1,
                next_port: 40_000,
                rng: seed | 1,
                auto_deliver: true,
                auto_ends: true,
                released_ends: 0,
                auto_run: true,
                host_facts: GuestHostFacts {
                    interfaces: BTreeMap::new(),
                    nofile_limit: 1_048_576,
                    owner_cgroup: PathBuf::from(
                        "/sys/fs/cgroup/overdrive.slice/control-plane.slice",
                    ),
                },
                stale_objects: false,
                gateway_route: None,
                device: SimForwarderDevice::new(),
                context: None,
                last_generation: None,
                listeners: BTreeMap::new(),
                next_assigned_port: SIM_FIRST_ASSIGNED_PORT,
                handles: BTreeMap::new(),
                route_types: BTreeMap::new(),
                connect4: BTreeMap::new(),
                vm_gone: BTreeSet::new(),
                closed_by_owner: BTreeSet::new(),
                local_far_end: BTreeMap::new(),
                to_guest: BTreeMap::new(),
                outbox: BTreeMap::new(),
                dials: BTreeMap::new(),
                peer_connects: Vec::new(),
                cid_vsocks: Vec::new(),
                flow_sockets: BTreeMap::new(),
                pairs: BTreeMap::new(),
                owner_aborted: BTreeSet::new(),
                ends_taken: BTreeMap::new(),
                streams: BTreeMap::new(),
            })),
            changed: Arc::new(Notify::new()),
        })
    }

    // -----------------------------------------------------------------------
    // QPROJ-7 / QPROJ-8 read surfaces
    // -----------------------------------------------------------------------

    /// QPROJ-7: the socket facts of this seam. A queued connection's token is
    /// `None` when nothing is buffered, and equally when its buffered first
    /// record is not canonical (a malformed record carries no token). A far
    /// end's first terminal event is the owner's own close of a socket the
    /// forwarder never held, else the far reader's first terminal event on
    /// the flow's host peer socket ([`SimSocket::peer_end`]).
    pub(crate) fn socket_observation(&self) -> SimOwnerSocketObservation {
        let st = self.state.lock();
        let mut queued = BTreeMap::new();
        for (listener, state) in &st.listeners {
            for conn in &state.queue {
                let token = match listener {
                    FlowListener::Control => conn
                        .record
                        .get(..CONTROL_OPEN_LEN)
                        .and_then(|bytes| <&[u8; CONTROL_OPEN_LEN]>::try_from(bytes).ok())
                        .and_then(|bytes| ControlOpen::decode(bytes).ok())
                        .map(|open| open.launch),
                    FlowListener::Stream => {
                        request_of(&conn.record).map(|request| request.launch())
                    }
                };
                queued.insert(conn.key, token);
            }
        }
        let peer_gone = st
            .handles
            .values()
            .filter_map(|handle| match handle {
                HandleState::Record(record) if record.peer_gone => record.conn,
                _ => None,
            })
            .collect();
        let mut far_end = st.local_far_end.clone();
        for (flow, sockets) in &st.flow_sockets {
            if let Some((PeerProtocol::Tcp, peer)) = &sockets.peer
                && let Some(end) = peer.peer_end()
            {
                far_end.entry(*flow).or_insert_with(|| far_end_of(end));
            }
        }
        SimOwnerSocketObservation {
            queued,
            closed_by_owner: st.closed_by_owner.clone(),
            peer_gone,
            far_end,
        }
    }

    /// QPROJ-8 arbiter: the current context's inventory
    /// (`FlowForwarder::inventory`), or the empty inventory when no context
    /// is held.
    pub(crate) fn forwarder_inventory(&self) -> ForwarderInventory {
        let ctx = self.state.lock().context.as_ref().map(|context| Arc::clone(&context.ctx));
        ctx.map(|ctx| ctx.inventory().expect("the sim forwarder's inventory is readable"))
            .unwrap_or_default()
    }

    /// QPROJ-8 arbiter: a live pair's per-direction state in the current
    /// context (`FlowForwarder::pair_state`).
    pub(crate) fn forwarder_pair_state(&self, id: ForwardId) -> Option<PairState> {
        let ctx = self.state.lock().context.as_ref().map(|context| Arc::clone(&context.ctx))?;
        ctx.pair_state(id).expect("the sim forwarder's pair state is readable")
    }

    /// QPROJ-8 arbiter (`oldCtx`): the device's context table, in open order
    /// — every context this seam opened, whether it is still open, and the
    /// pairs and watches each still holds.
    pub(crate) fn forwarder_contexts(&self) -> Vec<SimContextState> {
        self.state.lock().device.contexts()
    }

    /// The directions of the current context with a queued run.
    pub(crate) fn forwarder_pending_runs(&self) -> Vec<(ForwardId, PairSide)> {
        let ctx = self.state.lock().context.as_ref().map(|context| Arc::clone(&context.ctx));
        ctx.map(|ctx| ctx.pending_runs()).unwrap_or_default()
    }

    /// One bounded run of the current context's direction of `id` whose
    /// source is `source` (`false` when none is queued).
    pub(crate) fn run_forwarder_once(&self, id: ForwardId, source: PairSide) -> bool {
        let ctx = self.state.lock().context.as_ref().map(|context| Arc::clone(&context.ctx));
        let ran = ctx.is_some_and(|ctx| ctx.run(id, source));
        self.bump();
        ran
    }

    /// Runs the current context until no run is queued; returns how many
    /// ran.
    pub(crate) fn run_forwarder(&self) -> u32 {
        let ctx = self.state.lock().context.as_ref().map(|context| Arc::clone(&context.ctx));
        let ran = ctx.map_or(0, |ctx| ctx.run_until_idle());
        self.bump();
        ran
    }

    // -----------------------------------------------------------------------
    // Holding, releasing, failing, scripting (test support)
    // -----------------------------------------------------------------------

    /// Holds every later effect of `kind` until released or failed.
    pub(crate) fn hold(&self, kind: SimEffectKind) {
        assert!(!kind.is_sync(), "test bug: the synchronous {kind:?} call cannot be held");
        self.state.lock().auto.remove(&kind);
    }

    /// Releases every later effect of `kind` without a hold.
    pub(crate) fn auto_release(&self, kind: SimEffectKind) {
        self.state.lock().auto.insert(kind);
    }

    /// The next auto-released (or synchronous) effect of `kind` fails with
    /// `failure`.
    pub(crate) fn script_failure(&self, kind: SimEffectKind, failure: SimFailure) {
        self.state.lock().scripts.entry(kind).or_default().push_back(failure);
    }

    /// Every effect the owner issued through this seam, in order (held or
    /// not, applied or not).
    pub(crate) fn issued(&self) -> Vec<SimEffect> {
        self.state.lock().issued.clone()
    }

    /// How many effects of `kind` the owner issued.
    pub(crate) fn issued_count(&self, kind: SimEffectKind) -> usize {
        self.state.lock().issued.iter().filter(|effect| effect.kind == kind).count()
    }

    /// The effects currently held, in issue order.
    pub(crate) fn held(&self) -> Vec<(SimEffectId, SimEffect)> {
        self.state.lock().held.iter().map(|(id, held)| (*id, held.effect)).collect()
    }

    /// The oldest held effect of `kind`.
    pub(crate) fn held_of(&self, kind: SimEffectKind) -> Option<SimEffectId> {
        self.state.lock().held.iter().find(|(_, held)| held.effect.kind == kind).map(|(id, _)| *id)
    }

    /// Releases a held effect: it applies with its natural outcome.
    pub(crate) fn release(&self, id: SimEffectId) {
        self.decide(id, None);
    }

    /// Fails a held effect with `failure`.
    pub(crate) fn fail(&self, id: SimEffectId, failure: SimFailure) {
        self.decide(id, Some(failure));
    }

    fn decide(&self, id: SimEffectId, decision: Option<SimFailure>) {
        let held = self
            .state
            .lock()
            .held
            .remove(&id)
            .unwrap_or_else(|| panic!("test bug: effect {id:?} is not held"));
        // The owner future may have been dropped; then the effect never applies.
        let _not_applied_if_dropped = held.decide.send(decision);
        self.bump();
    }

    /// Accepts and record bytes reach the owner only on
    /// [`Self::release_accept`] / [`Self::release_read`] (`false`), or at
    /// once (`true`, the default).
    pub(crate) fn set_auto_deliver(&self, auto: bool) {
        self.state.lock().auto_deliver = auto;
        self.bump();
    }

    /// End records reach the owner's `next_end` only on
    /// [`Self::release_end`] (`false`), or at once (`true`, the default).
    pub(crate) fn set_auto_ends(&self, auto: bool) {
        self.state.lock().auto_ends = auto;
        self.bump();
    }

    /// Lets the owner's `next_end` take one more queued end record.
    pub(crate) fn release_end(&self) {
        self.state.lock().released_ends += 1;
        self.bump();
    }

    /// The forwarder runs to idle after every scripted cause and attach
    /// (`true`, the default), or only on [`Self::run_forwarder`] /
    /// [`Self::run_forwarder_once`] (`false`).
    pub(crate) fn set_auto_run(&self, auto: bool) {
        self.state.lock().auto_run = auto;
        self.bump();
    }

    /// Resolves when every owner task waiting on this seam is parked with
    /// nothing undelivered: no task of the runtime is ready to run.
    ///
    /// Idle detection is the runtime's own: the caller drives the owner on a
    /// current-thread tokio runtime whose time is paused. A paused runtime
    /// auto-advances its clock to the next timer only when no task is ready,
    /// so the sleep below completes exactly when every task is parked. The
    /// owner reads time only through its injected `Clock` port (`SimClock`),
    /// never through tokio timers, so pausing tokio time does not change its
    /// behaviour (`.claude/rules/development.md` § "Production code is not
    /// shaped by simulation").
    ///
    /// Precondition: the runtime's time is paused.
    pub(crate) async fn settled(&self) {
        /// Far beyond any deadline the tests arm: the sleep is the idle
        /// signal, never a timeout that elapses while a task is ready.
        const IDLE_SIGNAL: std::time::Duration = std::time::Duration::from_secs(365 * 24 * 3600);
        tokio::time::sleep(IDLE_SIGNAL).await;
    }

    /// The `serve` process ended: every handle and every held effect dies
    /// with it; the context's release aborts every pair (far ends reset,
    /// EVMOD-CTL); every unattached TCP peer the owner held with linger set
    /// is reset (H-5); every dial `V_h` ends with `0x3` toward the guest.
    pub(crate) fn exit_process(&self) {
        let mut st = self.state.lock();
        if st.exited {
            return;
        }
        st.exited = true;
        st.held.clear();
        if let Some(context) = st.context.take() {
            Self::release_context(&mut st, context);
        }
        let handles = std::mem::take(&mut st.handles);
        for handle in handles.into_values() {
            match handle {
                HandleState::Pair(_) => {}
                HandleState::Peer(peer) => {
                    if let (PeerProtocol::Tcp, true, Some(flow)) =
                        (peer.spec.protocol, peer.spec.abortive_linger, peer.flow)
                    {
                        st.local_far_end.entry(flow).or_insert(FarEndEvent::Reset);
                    }
                }
                HandleState::Record(record) => {
                    if let Some(flow) = record.dial_flow {
                        st.to_guest.entry(flow).or_default().push(VsockShutdownOp::Close);
                    }
                }
            }
        }
        drop(st);
        self.bump();
    }

    // -----------------------------------------------------------------------
    // Scripting host facts and kernel answers (test support)
    // -----------------------------------------------------------------------

    /// Mutates the host facts `host_facts` reports.
    pub(crate) fn update_host_facts(&self, update: impl FnOnce(&mut GuestHostFacts)) {
        update(&mut self.state.lock().host_facts);
    }

    /// Whether a process-scoped owner object of a previous run is present.
    pub(crate) fn set_stale_objects(&self, present: bool) {
        self.state.lock().stale_objects = present;
    }

    /// The gateway the converged route names, if converged.
    pub(crate) fn gateway_route(&self) -> Option<Ipv4Addr> {
        self.state.lock().gateway_route
    }

    /// The node starts with a gateway route already present (a route left
    /// by a previous boot, GW-LOCAL H-4).
    pub(crate) fn set_gateway_route(&self, gateway: Option<Ipv4Addr>) {
        self.state.lock().gateway_route = gateway;
    }

    /// `route_type(destination)` answers `route` (default `Unicast`).
    pub(crate) fn set_route_type(&self, destination: Ipv4Addr, route: FibRouteType) {
        self.state.lock().route_types.insert(destination, route);
    }

    /// A UDP `connect()` to `vip` is rewritten by `connect4` to `backend`.
    pub(crate) fn set_connect4_rewrite(&self, vip: SocketAddrV4, backend: SocketAddrV4) {
        self.state.lock().connect4.insert(vip, backend);
    }

    /// The live forwarder ids the sim issued in the current context.
    pub(crate) fn pairs(&self) -> BTreeMap<ForwardId, SimPairInfo> {
        let st = self.state.lock();
        let Some(generation) = st.context.as_ref().map(|context| context.generation) else {
            return BTreeMap::new();
        };
        st.pairs
            .iter()
            .filter(|((g, _), _)| *g == generation)
            .map(|((_, id), info)| (*id, *info))
            .collect()
    }

    /// The live pair or watch of `flow` in the current context.
    pub(crate) fn pair_of(&self, flow: FlowId) -> Option<ForwardId> {
        self.pairs().into_iter().find(|(_, info)| info.flow == Some(flow)).map(|(id, _)| id)
    }

    /// The forwarder's per-direction state of `flow`'s pair (the arbiter's
    /// `FlowForwarder::pair_state`).
    pub(crate) fn pair_state_of(&self, flow: FlowId) -> Option<PairState> {
        let id = self.pair_of(flow)?;
        self.forwarder_pair_state(id)
    }

    /// The vsock SHUTDOWN ops the host sent toward the guest on `flow`'s dial
    /// `V_h`, in order.
    pub(crate) fn sent_to_guest(&self, flow: FlowId) -> Vec<VsockShutdownOp> {
        self.state.lock().to_guest.get(&flow).cloned().unwrap_or_default()
    }

    /// Every TCP connect of a peer that completed, by destination (D9:
    /// leg-F; D14: the destination).
    pub(crate) fn peer_connects(&self) -> Vec<SocketAddrV4> {
        self.state.lock().peer_connects.clone()
    }

    /// Every dial the owner opened, in open order.
    pub(crate) fn dials(&self) -> Vec<SimDialRecord> {
        self.state.lock().dials.values().copied().collect()
    }

    /// Control messages the owner wrote on `cid`'s control session and the
    /// guest has not consumed.
    pub(crate) fn outbox(&self, cid: GuestCid) -> Vec<ControlMessage> {
        self.state.lock().outbox.get(&cid).cloned().unwrap_or_default()
    }

    /// The guest consumes every control message the owner wrote to `cid`.
    pub(crate) fn take_outbox(&self, cid: GuestCid) -> Vec<ControlMessage> {
        let taken = self.state.lock().outbox.remove(&cid).unwrap_or_default();
        self.bump();
        taken
    }

    // -----------------------------------------------------------------------
    // Causes on a flow's sockets (test support; the arbiter classifies them)
    // -----------------------------------------------------------------------

    /// The host vsock of `flow` (`V_h`), the guest being its far side.
    pub(crate) fn vsock_of(&self, flow: FlowId) -> SimSocket {
        self.flow_sockets(flow).vsock
    }

    /// The host peer socket of `flow` (D, L or H), the destination being its
    /// far side. Before attach, resolves the one unattached peer only when it
    /// is unambiguous; parallel in-flight peers are never guessed. Releasing a
    /// held connect makes its result eligible before the owner task next polls.
    pub(crate) fn peer_of(&self, flow: FlowId) -> SimSocket {
        let st = self.state.lock();
        if let Some((_, socket)) = st
            .flow_sockets
            .get(&flow)
            .unwrap_or_else(|| panic!("test bug: the sim made no socket of {flow:?}"))
            .peer
            .as_ref()
        {
            return socket.clone();
        }
        assert!(
            !st.held.values().any(|held| held.effect.kind == SimEffectKind::ConnectPeer),
            "test bug: {flow:?} cannot receive a destination cause while its connect is held"
        );
        let mut pending = st.handles.values().filter_map(|handle| match handle {
            HandleState::Peer(peer) if peer.flow.is_none() => Some(peer.socket.clone()),
            HandleState::Record(_) | HandleState::Peer(_) | HandleState::Pair(_) => None,
        });
        let peer = pending
            .next()
            .unwrap_or_else(|| panic!("test bug: {flow:?} has no host peer socket yet"));
        assert!(
            pending.next().is_none(),
            "test bug: {flow:?} cannot select among multiple unattached host peers"
        );
        peer
    }

    /// The shared `SimSocket` of `flow`'s host vsock half, keyed from the
    /// step the seam first carries the flow id on it (a guest-opened V_h: its
    /// request's read; a dial's V_h: its TcpAccept request's write), kept
    /// after the owner closed it, until the seam is dropped. `None` for a flow
    /// id the seam never saw (D-5; the `fwd_pair` row `vs(s)`).
    pub(crate) fn vsock_half(&self, flow: FlowId) -> Option<SimSocket> {
        self.state.lock().flow_sockets.get(&flow).map(|sockets| sockets.vsock.clone())
    }

    /// Select one native holder-read result from the exact state the Sim
    /// otherwise collapses after the local owner shuts a watched V_h both
    /// ways. Returns `None` for every other socket state so ordinary reads,
    /// peer shutdown, queued bytes and pending errors keep using
    /// [`SimSocket::local_read`] unchanged.
    pub(crate) fn owner_shutdown_read(
        &self,
        flow: FlowId,
        max: usize,
        selected: OwnerShutdownRead,
    ) -> Option<SimLocalRead> {
        let socket = self.vsock_half(flow)?;
        let collapsed = socket.kind() == SimSocketKind::VsockStream
            && socket.sent_shutdown() == Some(SimShutdown::Both)
            && socket.stream_input().is_empty()
            && socket.received_shutdown().is_none()
            && socket.sk_err().is_none()
            && socket.vsock_state() == SimVsockState::Established;
        if !collapsed {
            return None;
        }
        let observed = socket.local_read(max);
        assert_eq!(
            observed,
            SimLocalRead::WouldBlock,
            "the post-owner-SHUT_RDWR read selector only replaces the Sim's collapsed WouldBlock"
        );
        Some(match selected {
            OwnerShutdownRead::Eof => SimLocalRead::Eof,
            OwnerShutdownRead::NotConnected => SimLocalRead::Err(
                KernelErrno::new(107).unwrap_or_else(|_| unreachable!("ENOTCONN is non-zero")),
            ),
        })
    }

    /// [`Self::peer_of`], `None` before the flow's peer socket was attached.
    pub(crate) fn peer_of_opt(&self, flow: FlowId) -> Option<SimSocket> {
        self.state
            .lock()
            .flow_sockets
            .get(&flow)
            .and_then(|sockets| sockets.peer.as_ref().map(|(_, socket)| socket.clone()))
    }

    /// The socket of the most recently opened host peer socket the owner
    /// still holds unattached (D, L or H): the far end's facts before the
    /// sim learns the socket's flow at attach (a destination's bytes, FIN or
    /// reset that reach the socket before the owner attaches it).
    pub(crate) fn latest_peer_socket(&self) -> Option<SimSocket> {
        self.state.lock().handles.values().rev().find_map(|handle| match handle {
            HandleState::Peer(peer) => Some(peer.socket.clone()),
            _ => None,
        })
    }

    /// Wakes every owner task after a cause scripted directly on a
    /// [`SimSocket`] (the forwarder runs only with auto-run).
    pub(crate) fn caused(&self) {
        self.after_cause();
    }

    fn flow_sockets(&self, flow: FlowId) -> FlowSockets {
        self.state
            .lock()
            .flow_sockets
            .get(&flow)
            .cloned()
            .unwrap_or_else(|| panic!("test bug: the sim made no socket of {flow:?}"))
    }

    /// After a scripted cause: the forwarder's runs (with auto-run), then a
    /// wake of every owner task.
    fn after_cause(&self) {
        if self.state.lock().auto_run {
            self.run_forwarder();
        }
        self.bump();
    }

    /// The guest sends `bytes` toward the destination on `flow`'s vsock.
    pub(crate) fn guest_sends(&self, flow: FlowId, bytes: &[u8]) {
        self.vsock_of(flow).peer_send(bytes);
        self.after_cause();
    }

    /// The guest half-closes its direction of `flow` (vsock `0x2`).
    pub(crate) fn guest_half_closes(&self, flow: FlowId) {
        self.vsock_of(flow).peer_shutdown(SimShutdown::Send);
        self.after_cause();
    }

    /// The guest closes `flow`'s vsock (`0x3`): an abort while a direction is
    /// open, the end of a completed flow after both ended cleanly
    /// (CLEAN-FULL-END).
    pub(crate) fn guest_closes(&self, flow: FlowId) {
        self.vsock_of(flow).peer_shutdown(SimShutdown::Both);
        self.after_cause();
    }

    /// The guest resets `flow`'s vsock (`RST`).
    pub(crate) fn guest_resets(&self, flow: FlowId) {
        self.vsock_of(flow).peer_reset();
        self.after_cause();
    }

    /// The destination sends `bytes` (UDP: one datagram) toward the guest.
    pub(crate) fn destination_sends(&self, flow: FlowId, bytes: &[u8]) {
        self.peer_of(flow).peer_send(bytes);
        self.after_cause();
    }

    /// The destination ends its direction with a FIN.
    pub(crate) fn destination_fins(&self, flow: FlowId) {
        self.peer_of(flow).peer_shutdown(SimShutdown::Send);
        self.after_cause();
    }

    /// The destination resets the connection.
    pub(crate) fn destination_resets(&self, flow: FlowId) {
        self.peer_of(flow).peer_reset();
        self.after_cause();
    }

    /// The destination's reader stops reading (backpressure).
    pub(crate) fn destination_stops_reading(&self, flow: FlowId) {
        self.peer_of(flow).peer_stop_reading();
        self.after_cause();
    }

    /// The destination's reader reads again.
    pub(crate) fn destination_resumes_reading(&self, flow: FlowId) {
        self.peer_of(flow).peer_resume_reading();
        self.after_cause();
    }

    /// The host peer socket of `flow` reports `errno` (D26-PINS F-4: the
    /// forwarder's send on a datagram association's H rejected `EPERM` by
    /// the host-internal rule, or an ICMP refusal `ECONNREFUSED`).
    pub(crate) fn destination_socket_error(&self, flow: FlowId, errno: KernelErrno) {
        let peer = self.peer_of(flow);
        peer.set_sk_err(errno);
        peer.report_error();
        self.after_cause();
    }

    /// `flow`'s vsock leaves `ESTABLISHED` with no peer SHUTDOWN or `RST`
    /// recorded — the broken invariant EVMOD-END aborts `Unclassified`
    /// (an alarm; no production path produces it).
    pub(crate) fn vsock_ends_unclassified(&self, flow: FlowId) {
        self.vsock_of(flow).set_vsock_state(SimVsockState::Closing);
        self.after_cause();
    }

    // -----------------------------------------------------------------------
    // Guest-side events (test support)
    // -----------------------------------------------------------------------

    /// A guest opens a connection to `listener`; it waits outside the accept
    /// queue with `record` buffered (`None`: nothing buffered).
    /// `physical_launch` is trusted fixture metadata naming the VMM process
    /// that opened the socket, independent of the untrusted record bytes.
    pub(crate) fn queue_connection(
        &self,
        listener: FlowListener,
        key: GuestConnKey,
        record: Option<Vec<u8>>,
        physical_launch: Option<LaunchToken>,
    ) {
        let mut st = self.state.lock();
        let released = st.auto_deliver;
        st.listeners.entry(listener).or_default().queue.push_back(QueuedConn {
            key,
            physical_launch,
            record: record.unwrap_or_default(),
            released,
            end: None,
        });
        drop(st);
        self.bump();
    }

    /// Releases a pending connection into the accept queue (D-UP-19).
    pub(crate) fn release_accept(&self, key: GuestConnKey) {
        let mut st = self.state.lock();
        let conn = st
            .listeners
            .values_mut()
            .flat_map(|listener| listener.queue.iter_mut())
            .find(|conn| conn.key == key)
            .unwrap_or_else(|| panic!("test bug: {key:?} is not queued"));
        conn.released = true;
        drop(st);
        self.bump();
    }

    /// Releases up to `bytes` (`None`: all) buffered bytes of an accepted
    /// connection to `read_record`.
    pub(crate) fn release_read(&self, key: GuestConnKey, bytes: Option<usize>) {
        let mut st = self.state.lock();
        let record = Self::record_by_conn(&mut st, key);
        let take = bytes.unwrap_or(record.buffered.len()).min(record.buffered.len());
        let moved: Vec<u8> = record.buffered.drain(..take).collect();
        record.ready.extend(moved);
        if record.buffered.is_empty() && record.end.is_some() {
            record.end_released = true;
        }
        drop(st);
        self.bump();
    }

    /// The guest writes `bytes` on an accepted record connection (a later
    /// control message: `Paired`, `Refused`, `TimedOut`, `Abort`).
    pub(crate) fn send_on_connection(&self, key: GuestConnKey, bytes: &[u8]) {
        let mut st = self.state.lock();
        let auto = st.auto_deliver;
        let record = Self::record_by_conn(&mut st, key);
        if auto {
            record.ready.extend(bytes.iter().copied());
        } else {
            record.buffered.extend(bytes.iter().copied());
        }
        drop(st);
        self.bump();
    }

    /// The guest ends an accepted record connection (after its buffered
    /// bytes): the end a record read observes. Its flow vsock — the socket
    /// the forwarder would attach — sees the same end as a SHUTDOWN (`Eof`:
    /// `0x3` with the guest's send shut) or an `RST` (`Reset`), so an attach
    /// after it finds the half as the kernel leaves it.
    pub(crate) fn end_connection(&self, key: GuestConnKey, end: SimPeerEnd) {
        let mut st = self.state.lock();
        let auto = st.auto_deliver;
        let record = Self::record_by_conn(&mut st, key);
        record.end = Some(end);
        if record.buffered.is_empty() || auto {
            record.end_released = true;
        }
        let vsock = record.vsock.clone();
        drop(st);
        match end {
            SimPeerEnd::Eof => vsock.peer_shutdown(SimShutdown::Both),
            SimPeerEnd::Reset => vsock.peer_reset(),
        }
        self.after_cause();
    }

    fn record_by_conn(st: &mut SimState, key: GuestConnKey) -> &mut RecordState {
        st.handles
            .values_mut()
            .find_map(|handle| match handle {
                HandleState::Record(record) if record.conn == Some(key) => Some(record),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!("test bug: {key:?} is not an accepted, unattached connection")
            })
    }

    /// The VM of `cid` dies (K-A2′, RESET-EXIT): accepted connections fail
    /// at their next I/O; queued ones keep their buffered record, then end;
    /// the device release's orphan sweep sets `sk_err = ECONNRESET` on every
    /// host vsock of the CID and then runs its `sk_error_report`, so every
    /// attached pair and every watch of the CID queues its record. After the
    /// sweep, auto-delivery releases accepted-connection loss to their owner
    /// tasks. A new
    /// vsock connect to the CID fails `ENODEV` until [`Self::vm_start`].
    pub(crate) fn vm_stop(&self, cid: GuestCid) {
        self.vm_stop_before_callbacks_matching(cid, None);
        self.finish_sweep(cid);
        let mut st = self.state.lock();
        if st.auto_deliver {
            for handle in st.handles.values_mut() {
                if let HandleState::Record(record) = handle
                    && record.peer_cid == cid
                    && record.peer_gone
                {
                    record.peer_gone_released = true;
                }
            }
        }
        drop(st);
        self.bump();
    }

    /// The orphan sweep's first step only (`drivers/vhost/vsock.c:746-750 @
    /// v7.0`): `sk_err` and the state of every host vsock of `cid` are set
    /// and readable; the forwarder's `sk_error_report` has not run, so no
    /// record is queued yet (U-EV-11, the `VmmExit` action of `fwd_pair`).
    /// Accepted-connection loss also remains unreleased: a stepped driver may
    /// order it independently through [`Self::release_peer_gone`]. The
    /// trusted `stopped_launch` selects physical sockets and is never decoded
    /// from their untrusted first records.
    pub(crate) fn vm_stop_before_callbacks(&self, cid: GuestCid, stopped_launch: LaunchToken) {
        self.vm_stop_before_callbacks_matching(cid, Some(stopped_launch));
    }

    fn vm_stop_before_callbacks_matching(
        &self,
        cid: GuestCid,
        stopped_launch: Option<LaunchToken>,
    ) {
        let mut st = self.state.lock();
        st.vm_gone.insert(cid);
        for listener in st.listeners.values_mut() {
            for conn in &mut listener.queue {
                if conn.key.peer_cid == cid
                    && stopped_launch.is_none_or(|launch| conn.physical_launch == Some(launch))
                {
                    conn.end = Some(SimPeerEnd::Reset);
                }
            }
        }
        for handle in st.handles.values_mut() {
            if let HandleState::Record(record) = handle
                && record.peer_cid == cid
                && (record.origin == RecordOrigin::Dialed
                    || stopped_launch.is_none_or(|launch| record.physical_launch == Some(launch)))
            {
                if record.origin == RecordOrigin::Dialed {
                    record.end = Some(SimPeerEnd::Reset);
                    record.end_released = true;
                } else {
                    record.peer_gone = true;
                    record.peer_gone_released = false;
                }
            }
        }
        let vsocks: Vec<SimSocket> = st
            .cid_vsocks
            .iter()
            .filter(|(of, physical_launch, _)| {
                *of == cid
                    && stopped_launch.is_none_or(|launch| {
                        physical_launch.is_none() || *physical_launch == Some(launch)
                    })
            })
            .map(|(_, _, socket)| socket.clone())
            .collect();
        drop(st);
        for vsock in vsocks {
            vsock.set_sk_err(KernelErrno::ECONNRESET);
        }
        self.bump();
    }

    /// The orphan sweep's second step: each reset host vsock's
    /// `sk_error_report` runs — every pair and watch of `cid` queues its
    /// record (`Aborted { SocketError { ECONNRESET } }` on its vsock half).
    pub(crate) fn finish_sweep(&self, cid: GuestCid) {
        let vsocks: Vec<SimSocket> = self
            .state
            .lock()
            .cid_vsocks
            .iter()
            .filter(|(of, _, _)| *of == cid)
            .map(|(_, _, socket)| socket.clone())
            .collect();
        for vsock in vsocks {
            vsock.report_error();
        }
        self.after_cause();
    }

    /// The orphan sweep's second step for one flow only: `flow`'s host vsock
    /// runs its `sk_error_report` (a driver that orders the sweep's
    /// callbacks against other events of its trace).
    pub(crate) fn finish_sweep_of(&self, flow: FlowId) {
        self.vsock_of(flow).report_error();
        self.after_cause();
    }

    /// The dial `V_h` of `flow` fails with `errno` (a vsock I/O error before
    /// `Paired`): its `sk_error_report` runs, so the dial's watch queues its
    /// record (`SocketError`).
    pub(crate) fn dial_vsock_error(&self, flow: FlowId, errno: KernelErrno) {
        let vsock = self.vsock_of(flow);
        vsock.set_sk_err(errno);
        vsock.report_error();
        self.after_cause();
    }

    /// Lets the owner observe the death of an accepted connection's VM at its
    /// next I/O (with `set_auto_deliver(false)`; `HostDeadSessionLoss`).
    pub(crate) fn release_peer_gone(&self, key: GuestConnKey) {
        let mut st = self.state.lock();
        let record = Self::record_by_conn(&mut st, key);
        assert!(record.peer_gone, "test bug: {key:?} has no dead peer to report");
        record.peer_gone_released = true;
        drop(st);
        self.bump();
    }

    /// An I/O error on the `V_h` of the in-flight dial of `flow` (a failed
    /// vsock write or read): the owner's next I/O on it fails with a reset.
    pub(crate) fn fail_dial_vsock(&self, flow: FlowId) {
        let mut st = self.state.lock();
        let record = st
            .handles
            .values_mut()
            .find_map(|handle| match handle {
                HandleState::Record(record)
                    if record.origin == RecordOrigin::Dialed && record.dial_flow == Some(flow) =>
                {
                    Some(record)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("test bug: no in-flight dial V_h of {flow:?}"));
        record.end = Some(SimPeerEnd::Reset);
        record.end_released = true;
        drop(st);
        self.bump();
    }

    /// A new launch of `cid` runs: a vsock connect to it succeeds again.
    pub(crate) fn vm_start(&self, cid: GuestCid) {
        self.state.lock().vm_gone.remove(&cid);
        self.bump();
    }

    /// The listener's accept task fails with `kind` (the listener unbinds).
    pub(crate) fn fail_listener(&self, listener: FlowListener, kind: io::ErrorKind) {
        self.state.lock().listeners.entry(listener).or_default().failed = Some(kind);
        self.bump();
    }

    // -----------------------------------------------------------------------
    // Handed-off streams (test support)
    // -----------------------------------------------------------------------

    /// The guest application writes `bytes` toward the dialer of `flow`:
    /// they reach `V_h`'s receive queue.
    pub(crate) fn stream_guest_write(&self, flow: FlowId, bytes: &[u8]) {
        self.vsock_of(flow).peer_send(bytes);
        self.bump();
    }

    /// The guest ends its direction of `flow`'s stream: the watch's callback
    /// queues its record before the holder can read the end (EVMOD-END,
    /// "Watch"), and the holder then reads end of stream on `V_h`.
    pub(crate) fn guest_ends_stream(&self, flow: FlowId, end: SimGuestStreamEnd) {
        let vsock = self.vsock_of(flow);
        match end {
            SimGuestStreamEnd::HalfClose => vsock.peer_shutdown(SimShutdown::Send),
            SimGuestStreamEnd::Abort => vsock.peer_shutdown(SimShutdown::Both),
            SimGuestStreamEnd::Reset => vsock.peer_reset(),
        }
        self.bump();
    }

    /// The dialer side of `flow`'s stream.
    pub(crate) fn stream_observation(&self, flow: FlowId) -> SimStreamObservation {
        let pipe = self.stream_pipe(flow);
        let pipe = pipe.lock();
        SimStreamObservation {
            to_guest: pipe.to_guest.clone(),
            host_shut: pipe.host_shut,
            dropped: pipe.dropped,
        }
    }

    fn stream_pipe(&self, flow: FlowId) -> Arc<Mutex<StreamPipe>> {
        self.state
            .lock()
            .streams
            .get(&flow)
            .cloned()
            .unwrap_or_else(|| panic!("test bug: no stream was handed off for {flow:?}"))
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn bump(&self) {
        self.changed.notify_waiters();
    }

    /// The effect gate of an async effect: `None` applies the effect's
    /// natural outcome, `Some(failure)` fails it. A held effect waits for the
    /// driver; a call dropped while held never applies.
    async fn gate(&self, kind: SimEffectKind, target: SimEffectTarget) -> Option<SimFailure> {
        let (id, decided) = {
            let mut st = self.state.lock();
            assert!(!st.exited, "owner call after its seam's process exited");
            st.issued.push(SimEffect { kind, target });
            if st.auto.contains(&kind) {
                let scripted = st.scripts.get_mut(&kind).and_then(VecDeque::pop_front);
                drop(st);
                self.bump();
                return scripted;
            }
            let id = SimEffectId(st.next_effect);
            st.next_effect += 1;
            let (decide, decided) = oneshot::channel();
            st.held.insert(id, Held { effect: SimEffect { kind, target }, decide });
            (id, decided)
        };
        self.bump();
        let guard = HeldGuard { sim: self, id };
        let decision = decided.await;
        std::mem::forget(guard);
        self.bump();
        match decision {
            Ok(decision) => decision,
            // The process exited while the effect was held: the call never
            // returns (its task dies with the process).
            Err(_) => std::future::pending().await,
        }
    }

    /// The gate of a synchronous forwarding call: recorded, scripted.
    fn sync_gate(
        st: &mut SimState,
        kind: SimEffectKind,
        target: SimEffectTarget,
    ) -> Option<SimFailure> {
        assert!(!st.exited, "owner call after its seam's process exited");
        st.issued.push(SimEffect { kind, target });
        st.scripts.get_mut(&kind).and_then(VecDeque::pop_front)
    }

    fn issue(&self, st: &mut SimState, state: HandleState) -> SeamHandle {
        let id = st.next_handle;
        st.next_handle += 1;
        st.handles.insert(id, state);
        SeamHandle { seam: self.seam, id }
    }

    fn own(&self, handle: SeamHandle) -> u64 {
        if handle.seam != self.seam {
            unreachable!("a handle of another seam instance reached this seam");
        }
        handle.id
    }

    fn take(&self, st: &mut SimState, handle: SeamHandle) -> HandleState {
        let id = self.own(handle);
        st.handles
            .remove(&id)
            .unwrap_or_else(|| unreachable!("a stale or closed handle reached the seam"))
    }

    fn context(st: &SimState) -> (Arc<SimFlowForwarder>, ForwarderGeneration) {
        let Some(context) = st.context.as_ref() else {
            unreachable!("a forwarding call with no forwarder context held");
        };
        (Arc::clone(&context.ctx), context.generation)
    }

    /// Releases `context` (EVMOD-CTL): the sim forwarder's release aborts
    /// every pair and watch it holds without a record (TCP halves reset,
    /// vsock halves `SHUT_RDWR`, a watched `V_h` included).
    fn release_context(st: &mut SimState, context: Context) {
        let generation = context.generation;
        // The release is the owner's own end of every watch it still holds:
        // a stream whose read then returns 0 or fails `ENOTCONN` reports the
        // owner's abort (EVMOD-STREAM-END order (1)).
        let watches: Vec<ForwardId> = st
            .pairs
            .iter()
            .filter(|((g, _), info)| *g == generation && info.kind == SimPairKind::Watch)
            .map(|((_, id), _)| *id)
            .collect();
        for id in watches {
            if !st.ends_taken.contains_key(&(generation, id)) {
                st.owner_aborted.insert((generation, id));
            }
        }
        let ctx = Arc::try_unwrap(context.ctx).unwrap_or_else(|_| {
            unreachable!("a sim reader held the forwarder context across its release")
        });
        ctx.release();
        st.pairs.retain(|(g, _), _| *g != generation);
    }

    /// Applies the owner's end of one handle: `abortive` resets.
    fn end_handle(st: &mut SimState, state: HandleState, abortive: bool) {
        match state {
            HandleState::Record(record) => {
                if let Some(conn) = record.conn {
                    st.closed_by_owner.insert(conn);
                }
                record.vsock.close_from_owner(abortive);
                if record.origin == RecordOrigin::Dialed
                    && let Some(flow) = record.dial_flow
                {
                    for dial in st.dials.values_mut() {
                        if dial.request.map(|request| request.flow()) == Some(flow) {
                            dial.closed = true;
                        }
                    }
                    st.to_guest.entry(flow).or_default().push(VsockShutdownOp::Close);
                }
            }
            HandleState::Peer(peer) => {
                if let (PeerProtocol::Tcp, Some(flow)) = (peer.spec.protocol, peer.flow) {
                    let event = if abortive || peer.spec.abortive_linger {
                        FarEndEvent::Reset
                    } else {
                        FarEndEvent::Eof
                    };
                    st.local_far_end.entry(flow).or_insert(event);
                }
            }
            HandleState::Pair(pair) => {
                if let Some(conn) = pair.conn
                    && pair.role == PairRole::Vsock
                {
                    st.closed_by_owner.insert(conn);
                }
                // An attached half's far end already saw the forwarder's FIN
                // or reset; a close of a half whose far end saw neither (an
                // owner close before its pair ended) ends it here.
                if let (PairRole::Tcp { linger }, Some(flow)) = (pair.role, pair.flow)
                    && pair.socket.peer_end().is_none()
                {
                    let event = if abortive || (linger && !pair.linger_cleared) {
                        FarEndEvent::Reset
                    } else {
                        FarEndEvent::Eof
                    };
                    st.local_far_end.entry(flow).or_insert(event);
                }
                pair.socket.close_from_owner(abortive);
            }
        }
    }

    /// Ends `socket`'s handle (abort or close).
    fn end_owned(&self, socket: OwnedSocket, abortive: bool) {
        let handle = match socket {
            OwnedSocket::Record(RecordSocket(h))
            | OwnedSocket::Peer(PeerSocket(h))
            | OwnedSocket::Pair(PairSocket(h)) => h,
        };
        let mut st = self.state.lock();
        let state = self.take(&mut st, handle);
        Self::end_handle(&mut st, state, abortive);
        drop(st);
        self.bump();
    }

    async fn wait_changed<T>(&self, mut poll: impl FnMut(&mut SimState) -> Option<T>) -> T {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let polled = {
                let mut st = self.state.lock();
                if st.exited { None } else { Some(poll(&mut st)) }
            };
            match polled {
                // The process exited: the call never returns.
                None => return std::future::pending().await,
                Some(Some(value)) => {
                    self.bump();
                    return value;
                }
                Some(None) => notified.await,
            }
        }
    }

    fn next_random(st: &mut SimState) -> u64 {
        let mut x = st.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        st.rng = x;
        x
    }

    /// A new host vsock of `cid` (the device release's sweep reaches it).
    fn new_vsock(
        st: &mut SimState,
        cid: GuestCid,
        physical_launch: Option<LaunchToken>,
    ) -> SimSocket {
        let socket = SimSocket::vsock_stream();
        st.cid_vsocks.push((cid, physical_launch, socket.clone()));
        socket
    }
}

/// Removes a held effect if the owner's call is dropped while held (the
/// effect never applies).
struct HeldGuard<'a> {
    sim: &'a SimGuestFlowIo,
    id: SimEffectId,
}

impl Drop for HeldGuard<'_> {
    fn drop(&mut self) {
        self.sim.state.lock().held.remove(&self.id);
        self.sim.bump();
    }
}

impl Drop for SimGuestFlowIo {
    fn drop(&mut self) {
        let leaked = {
            let st = self.state.lock();
            if st.exited { 0 } else { st.handles.len() }
        };
        self.exit_process();
        assert!(
            leaked == 0 || std::thread::panicking(),
            "owner defect: {leaked} seam handle(s) dropped without abort / close or a consuming call"
        );
    }
}

// ===========================================================================
// The handed-off stream (sim side of `into_guest_stream`)
// ===========================================================================

/// The in-memory `GuestStreamIo` behind a `GuestStream` the sim hands off.
/// It reads the dial's `V_h` as its holder ([`SimSocket::local_read`]) and
/// resolves every end it observes from the seam's facts (EVMOD-STREAM-END,
/// WATCH-ABORT).
#[derive(Debug)]
struct SimGuestStreamIo {
    flow: FlowId,
    pipe: Arc<Mutex<StreamPipe>>,
    state: Arc<Mutex<SimState>>,
    changed: Arc<Notify>,
}

/// How the stream resolves an end it observed on `V_h` (or an owner abort).
enum Resolved {
    OwnerAbort,
    HalfClose,
    GuestAbort,
    Pending,
}

impl SimGuestStreamIo {
    fn resolve(&self, generation: ForwarderGeneration, watch: ForwardId) -> Resolved {
        let st = self.state.lock();
        if st.owner_aborted.contains(&(generation, watch)) {
            return Resolved::OwnerAbort;
        }
        match st.ends_taken.get(&(generation, watch)) {
            Some(ForwardOutcome::GuestHalfClose | ForwardOutcome::Completed) => Resolved::HalfClose,
            Some(ForwardOutcome::Aborted { .. }) => Resolved::GuestAbort,
            None => Resolved::Pending,
        }
    }

    fn owner_aborted(&self, generation: ForwarderGeneration, watch: ForwardId) -> bool {
        self.state.lock().owner_aborted.contains(&(generation, watch))
    }

    fn ids(&self) -> (ForwarderGeneration, ForwardId, SimSocket) {
        let pipe = self.pipe.lock();
        (pipe.generation, pipe.watch, pipe.vsock.clone())
    }
}

fn aborted() -> io::Error {
    io::Error::from(io::ErrorKind::ConnectionAborted)
}

fn reset() -> io::Error {
    io::Error::from(io::ErrorKind::ConnectionReset)
}

#[async_trait::async_trait]
impl GuestStreamIo for SimGuestStreamIo {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let waiter = self.changed.notified();
            tokio::pin!(waiter);
            waiter.as_mut().enable();
            let (generation, watch, vsock) = self.ids();
            if self.owner_aborted(generation, watch) {
                return Err(aborted());
            }
            match vsock.local_read(buf.len()) {
                SimLocalRead::Bytes(bytes) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    return Ok(n);
                }
                // The holder's read of an end is never the classification:
                // the end is resolved from the watch.
                SimLocalRead::Eof | SimLocalRead::Err(_) => match self.resolve(generation, watch) {
                    Resolved::OwnerAbort => return Err(aborted()),
                    Resolved::HalfClose => return Ok(0),
                    Resolved::GuestAbort => return Err(reset()),
                    Resolved::Pending => {}
                },
                SimLocalRead::WouldBlock => {}
            }
            waiter.await;
        }
    }

    async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (generation, watch, vsock) = self.ids();
        if self.owner_aborted(generation, watch) {
            return Err(aborted());
        }
        if self.pipe.lock().host_shut {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        // The guest stops receiving only by an abort: a write after the
        // guest's abort-class end (or the sweep's sk_err) fails, resolved
        // from the watch (EVMOD-STREAM-END).
        if vsock.sk_err().is_some() || vsock.vsock_state() == SimVsockState::Closing {
            loop {
                let waiter = self.changed.notified();
                tokio::pin!(waiter);
                waiter.as_mut().enable();
                match self.resolve(generation, watch) {
                    Resolved::OwnerAbort => return Err(aborted()),
                    Resolved::HalfClose | Resolved::GuestAbort => return Err(reset()),
                    Resolved::Pending => waiter.await,
                }
            }
        }
        self.pipe.lock().to_guest.extend_from_slice(buf);
        self.changed.notify_waiters();
        Ok(buf.len())
    }

    async fn shutdown_write(&mut self) -> io::Result<()> {
        let (generation, watch, vsock) = self.ids();
        if self.owner_aborted(generation, watch) {
            return Err(aborted());
        }
        let first = {
            let mut pipe = self.pipe.lock();
            let first = !pipe.host_shut;
            pipe.host_shut = true;
            first
        };
        if first {
            vsock.shutdown_send();
            self.state
                .lock()
                .to_guest
                .entry(self.flow)
                .or_default()
                .push(VsockShutdownOp::HalfClose);
        }
        self.changed.notify_waiters();
        Ok(())
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        let (generation, watch, _) = self.ids();
        match self.resolve(generation, watch) {
            Resolved::OwnerAbort | Resolved::GuestAbort => Some(GuestStreamEnd::Abort),
            Resolved::HalfClose => Some(GuestStreamEnd::HalfClose),
            Resolved::Pending => None,
        }
    }
}

impl Drop for SimGuestStreamIo {
    /// EVMOD-STREAM-ABORT: the drop aborts the watch (unless the owner
    /// already ended the stream), then closes `V_h` (`0x3` toward the
    /// guest), in one step. The watch's record (`Detached`) tells the owner
    /// to release the node slot and, for `Relay`, the quota place
    /// (EVMOD-AB).
    fn drop(&mut self) {
        let (generation, watch, vsock) = {
            let mut pipe = self.pipe.lock();
            pipe.dropped = true;
            (pipe.generation, pipe.watch, pipe.vsock.clone())
        };
        let mut st = self.state.lock();
        let live_context = st
            .context
            .as_ref()
            .filter(|context| context.generation == generation)
            .map(|context| Arc::clone(&context.ctx));
        if !st.owner_aborted.contains(&(generation, watch))
            && let Some(ctx) = live_context
        {
            let _record_queued_or_already_ended = ctx.abort(watch);
        }
        vsock.close_from_owner(false);
        st.to_guest.entry(self.flow).or_default().push(VsockShutdownOp::Close);
        for dial in st.dials.values_mut() {
            if dial.streamed && dial.request.map(|request| request.flow()) == Some(self.flow) {
                dial.closed = true;
            }
        }
        drop(st);
        self.changed.notify_waiters();
    }
}

// ===========================================================================
// The seam
// ===========================================================================

#[async_trait::async_trait]
impl GuestFlowIo for SimGuestFlowIo {
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError> {
        match self.gate(SimEffectKind::HostFacts, SimEffectTarget::None).await {
            None => Ok(self.state.lock().host_facts.clone()),
            Some(SimFailure::Io(kind)) => Err(GuestNetworkError::Io {
                operation: GuestNetworkOperation::StartupProbe,
                source: io::Error::from(kind),
            }),
            Some(other) => mismatched("host_facts", other),
        }
    }

    async fn probe_forwarder(&self) -> Result<(), ForwarderError> {
        let failure = self.gate(SimEffectKind::ProbeForwarder, SimEffectTarget::None).await;
        let st = self.state.lock();
        // A context of the probe's own: the serving context and the
        // generation count are untouched, and the context is released before
        // return on every outcome (D-8).
        let outcome = match failure {
            None => {
                let ctx = st.device.open();
                let probed = ctx.probe();
                ctx.release();
                probed
            }
            // The open itself fails: no context is opened.
            Some(SimFailure::Forwarder(
                failure @ (SimForwarderFailure::DeviceAbsent | SimForwarderFailure::AbiMismatch),
            )) => Err(failure.error()),
            // A stage observed the wrong behaviour on the opened context.
            Some(SimFailure::Forwarder(failure @ SimForwarderFailure::ProbeMismatch(..))) => {
                let ctx = st.device.open();
                ctx.release();
                Err(failure.error())
            }
            Some(other) => mismatched("probe_forwarder", other),
        };
        drop(st);
        self.bump();
        outcome
    }

    async fn open_forwarder(&self) -> Result<ForwarderGeneration, ForwarderError> {
        let failure = self.gate(SimEffectKind::OpenForwarder, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        // The old context is closed before the new one is opened (EVMOD-CTL).
        if let Some(old) = st.context.take() {
            Self::release_context(&mut st, old);
        }
        match failure {
            None => {
                let ctx = st.device.open();
                // Earned Trust: a context whose probe fails is released at
                // once; the seam then holds none.
                if let Err(error) = ctx.probe() {
                    ctx.release();
                    return Err(error);
                }
                let generation = match st.last_generation {
                    None => ForwarderGeneration::FIRST,
                    Some(last) => last.next().expect("fewer than u64::MAX contexts"),
                };
                st.last_generation = Some(generation);
                st.context = Some(Context { ctx: Arc::new(ctx), generation });
                drop(st);
                self.bump();
                Ok(generation)
            }
            Some(SimFailure::Forwarder(failure)) => Err(failure.error()),
            Some(other) => mismatched("open_forwarder", other),
        }
    }

    fn close_forwarder(&self) {
        let mut st = self.state.lock();
        if let Some(failure) =
            Self::sync_gate(&mut st, SimEffectKind::CloseForwarder, SimEffectTarget::None)
        {
            mismatched("close_forwarder (infallible)", failure);
        }
        if let Some(context) = st.context.take() {
            Self::release_context(&mut st, context);
        }
        drop(st);
        self.bump();
    }

    async fn bind_listener(
        &self,
        listener: FlowListener,
        port: VsockListenPort,
    ) -> Result<VsockPort, FlowIoError> {
        let failure =
            self.gate(SimEffectKind::BindListener, SimEffectTarget::Listener(listener)).await;
        let mut st = self.state.lock();
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("bind_listener", other),
            };
        }
        // A held listener (a repair after its accept task ended): `port` is
        // not consulted; the held socket still listens, so it serves again on
        // its bound port, nothing closed or bound.
        if let Some(state) = st.listeners.get_mut(&listener)
            && let Some(held) = state.port
        {
            state.bound = true;
            state.failed = None;
            return Ok(held);
        }
        let bound = match port {
            VsockListenPort::Fixed(port) => port,
            VsockListenPort::KernelAssigned => {
                let assigned = VsockPort::new(st.next_assigned_port);
                st.next_assigned_port += 1;
                assigned
            }
        };
        let state = st.listeners.entry(listener).or_default();
        state.bound = true;
        state.failed = None;
        state.port = Some(bound);
        Ok(bound)
    }

    async fn converge_gateway_route(&self, gateway: Ipv4Addr) -> Result<(), GuestNetworkError> {
        match self.gate(SimEffectKind::GatewayRoute, SimEffectTarget::Ip(gateway)).await {
            None => {
                self.state.lock().gateway_route = Some(gateway);
                Ok(())
            }
            Some(SimFailure::Netlink) => Err(GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::LocalRouteConverge,
                source: NetlinkError::Connect {
                    source: io::Error::from_raw_os_error(libc::ENOBUFS),
                },
            }),
            Some(other) => mismatched("converge_gateway_route", other),
        }
    }

    async fn stale_objects_present(&self) -> Result<bool, GuestNetworkError> {
        match self.gate(SimEffectKind::StaleObjects, SimEffectTarget::None).await {
            None => Ok(self.state.lock().stale_objects),
            Some(SimFailure::Io(kind)) => Err(GuestNetworkError::Io {
                operation: GuestNetworkOperation::CleanupComplement,
                source: io::Error::from(kind),
            }),
            Some(other) => mismatched("stale_objects_present", other),
        }
    }

    async fn accept(
        &self,
        listener: FlowListener,
    ) -> Result<(RecordSocket, GuestConnKey), FlowIoError> {
        self.wait_changed(|st| {
            let auto = st.auto_deliver;
            let state = st.listeners.entry(listener).or_default();
            if let Some(kind) = state.failed.take() {
                state.bound = false;
                return Some(Err(io_error(kind)));
            }
            if !state.bound {
                unreachable!("accept on a listener bind_listener has not bound");
            }
            let released: Vec<usize> = state
                .queue
                .iter()
                .enumerate()
                .filter(|(_, conn)| conn.released)
                .map(|(index, _)| index)
                .collect();
            if released.is_empty() {
                return None;
            }
            // D-UP-19: one connection enters the accept queue at the step
            // that accepts it; with auto-delivery the seed picks which.
            let pick = if auto && released.len() > 1 {
                let draw = Self::next_random(st);
                released[usize::try_from(draw % released.len() as u64).unwrap_or(0)]
            } else {
                released[0]
            };
            let state = st.listeners.entry(listener).or_default();
            let conn = state.queue.remove(pick)?;
            let vsock = Self::new_vsock(st, conn.key.peer_cid, conn.physical_launch);
            if listener == FlowListener::Stream
                && let Some(request) = request_of(&conn.record)
            {
                st.flow_sockets.insert(
                    request.flow(),
                    FlowSockets { cid: conn.key.peer_cid, vsock: vsock.clone(), peer: None },
                );
            }
            let mut record = RecordState {
                origin: RecordOrigin::Accepted(listener),
                conn: Some(conn.key),
                peer_cid: conn.key.peer_cid,
                physical_launch: conn.physical_launch,
                first_record: conn.record.clone(),
                buffered: VecDeque::new(),
                ready: VecDeque::new(),
                end: conn.end,
                end_released: false,
                peer_gone: false,
                peer_gone_released: false,
                dial_flow: None,
                vsock,
            };
            if auto {
                record.ready = conn.record.into_iter().collect();
                record.end_released = record.end.is_some();
            } else {
                record.buffered = conn.record.into_iter().collect();
            }
            let handle = self.issue(st, HandleState::Record(record));
            Some(Ok((RecordSocket(handle), conn.key)))
        })
        .await
    }

    async fn read_record(&self, socket: &RecordSocket, max: usize) -> Result<Vec<u8>, FlowIoError> {
        if max == 0 {
            return Err(io_error(io::ErrorKind::InvalidInput));
        }
        let id = self.own(socket.0);
        self.wait_changed(|st| {
            let Some(HandleState::Record(record)) = st.handles.get_mut(&id) else {
                unreachable!("read_record on a handle that is not a live record socket");
            };
            // K-A2′: an accepted connection whose VM died fails at its next I/O.
            if record.peer_gone && record.peer_gone_released {
                return Some(Err(io_error(io::ErrorKind::ConnectionReset)));
            }
            if !record.ready.is_empty() {
                let take = max.min(record.ready.len());
                return Some(Ok(record.ready.drain(..take).collect()));
            }
            if record.end_released {
                return match record.end {
                    Some(SimPeerEnd::Eof) | None => Some(Ok(Vec::new())),
                    Some(SimPeerEnd::Reset) => Some(Err(io_error(io::ErrorKind::ConnectionReset))),
                };
            }
            None
        })
        .await
    }

    async fn write_record(&self, socket: &RecordSocket, bytes: &[u8]) -> Result<(), FlowIoError> {
        let id = self.own(socket.0);
        let conn = match self.state.lock().handles.get(&id) {
            Some(HandleState::Record(record)) => record.conn,
            _ => unreachable!("write_record on a handle that is not a live record socket"),
        };
        let failure = self.gate(SimEffectKind::WriteRecord, SimEffectTarget::Conn(conn)).await;
        let mut guard = self.state.lock();
        let st = &mut *guard;
        let Some(HandleState::Record(record)) = st.handles.get_mut(&id) else {
            unreachable!("write_record on a handle closed while the write was held");
        };
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("write_record", other),
            };
        }
        if record.peer_gone || record.end == Some(SimPeerEnd::Reset) {
            return Err(io_error(io::ErrorKind::BrokenPipe));
        }
        let cid = record.peer_cid;
        match record.origin {
            RecordOrigin::Dialed => {
                let request = request_of(bytes);
                record.dial_flow = request.map(|request| request.flow());
                let flow = record.dial_flow;
                let vsock = record.vsock.clone();
                if let Some(flow) = flow {
                    st.flow_sockets.insert(flow, FlowSockets { cid, vsock, peer: None });
                }
                if let Some(dial) = st.dials.get_mut(&id) {
                    dial.request = request;
                    // The dial's watch learns its flow.
                    if let Some(watch) = dial.watch {
                        for ((_, pid), info) in &mut st.pairs {
                            if *pid == watch && info.kind == SimPairKind::Watch {
                                info.flow = flow;
                            }
                        }
                    }
                }
            }
            RecordOrigin::Accepted(_) => {
                if let Ok(bytes) = <&[u8; CONTROL_MESSAGE_LEN]>::try_from(bytes)
                    && let Ok(message) = ControlMessage::decode(bytes)
                {
                    st.outbox.entry(cid).or_default().push(message);
                }
            }
        }
        drop(guard);
        self.bump();
        Ok(())
    }

    async fn connect_guest(&self, cid: GuestCid) -> Result<RecordSocket, FlowIoError> {
        let failure = self.gate(SimEffectKind::ConnectGuest, SimEffectTarget::Cid(cid)).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("connect_guest", other),
            };
        }
        let mut st = self.state.lock();
        if st.vm_gone.contains(&cid) {
            // No device holds the CID: `drivers/vhost/vsock.c:290 @ v7.0`.
            return Err(FlowIoError::Io { source: io::Error::from_raw_os_error(libc::ENODEV) });
        }
        let vsock = Self::new_vsock(&mut st, cid, None);
        let record = RecordState {
            origin: RecordOrigin::Dialed,
            conn: None,
            peer_cid: cid,
            physical_launch: None,
            first_record: Vec::new(),
            buffered: VecDeque::new(),
            ready: VecDeque::new(),
            end: None,
            end_released: false,
            peer_gone: false,
            peer_gone_released: false,
            dial_flow: None,
            vsock,
        };
        let handle = self.issue(&mut st, HandleState::Record(record));
        st.dials.insert(
            handle.id,
            SimDialRecord { cid, watch: None, request: None, closed: false, streamed: false },
        );
        drop(st);
        self.bump();
        Ok(RecordSocket(handle))
    }

    fn watch(&self, vsock: &RecordSocket) -> Result<ForwardId, ForwarderError> {
        let id = self.own(vsock.0);
        let mut st = self.state.lock();
        let failure = Self::sync_gate(&mut st, SimEffectKind::Watch, SimEffectTarget::None);
        let (ctx, generation) = Self::context(&st);
        let (socket, cid, flow) = match st.handles.get(&id) {
            Some(HandleState::Record(record)) if record.origin == RecordOrigin::Dialed => {
                (record.vsock.clone(), record.peer_cid, record.dial_flow)
            }
            _ => unreachable!("watch on a handle that is not a dial's V_h"),
        };
        match failure {
            None => {}
            Some(SimFailure::Forwarder(failure)) => return Err(failure.error()),
            Some(other) => mismatched("watch", other),
        }
        // `HalfEnded` (the sweep already reset V_h) is the arbiter's answer.
        let watch = ctx.watch(&socket)?;
        st.pairs.insert(
            (generation, watch),
            SimPairInfo { kind: SimPairKind::Watch, flow, cid, generation },
        );
        if let Some(dial) = st.dials.get_mut(&id) {
            dial.watch = Some(watch);
        }
        drop(st);
        self.bump();
        Ok(watch)
    }

    fn into_guest_stream(
        &self,
        vsock: RecordSocket,
        watch: ForwardId,
        flow: FlowId,
        target: SocketAddrV4,
    ) -> GuestStream {
        let mut st = self.state.lock();
        let id = self.own(vsock.0);
        let HandleState::Record(record) = self.take(&mut st, vsock.0) else {
            unreachable!("into_guest_stream on a handle that is not a dial's V_h");
        };
        let (_, generation) = Self::context(&st);
        if let Some(dial) = st.dials.get_mut(&id) {
            assert_eq!(dial.watch, Some(watch), "into_guest_stream names the dial's own watch");
            dial.streamed = true;
        }
        let pipe = Arc::new(Mutex::new(StreamPipe {
            generation,
            watch,
            vsock: record.vsock,
            to_guest: Vec::new(),
            host_shut: false,
            dropped: false,
        }));
        st.streams.insert(flow, Arc::clone(&pipe));
        drop(st);
        self.bump();
        GuestStream::new(
            target,
            Box::new(SimGuestStreamIo {
                flow,
                pipe,
                state: Arc::clone(&self.state),
                changed: Arc::clone(&self.changed),
            }),
        )
    }

    async fn open_peer(&self, spec: PeerSocketSpec) -> Result<PeerSocket, FlowIoError> {
        let failure = self.gate(SimEffectKind::OpenPeer, SimEffectTarget::Peer(spec)).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("open_peer", other),
            };
        }
        let mut st = self.state.lock();
        let local = spec.bind.map(|bind| {
            if bind.port() == 0 {
                let port = st.next_port;
                st.next_port += 1;
                SocketAddrV4::new(*bind.ip(), port)
            } else {
                bind
            }
        });
        let socket = match spec.protocol {
            PeerProtocol::Tcp => SimSocket::tcp(),
            PeerProtocol::Udp => SimSocket::udp(),
        };
        let handle = self.issue(
            &mut st,
            HandleState::Peer(PeerState { spec, local, peer: None, flow: None, socket }),
        );
        Ok(PeerSocket(handle))
    }

    fn local_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let id = self.own(peer.0);
        match self.state.lock().handles.get(&id) {
            Some(HandleState::Peer(state)) => {
                state.local.ok_or_else(|| io_error(io::ErrorKind::NotConnected))
            }
            _ => unreachable!("local_addr on a handle that is not a peer socket"),
        }
    }

    fn peer_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let id = self.own(peer.0);
        match self.state.lock().handles.get(&id) {
            Some(HandleState::Peer(state)) => {
                state.peer.ok_or_else(|| io_error(io::ErrorKind::NotConnected))
            }
            _ => unreachable!("peer_addr on a handle that is not a peer socket"),
        }
    }

    async fn connect_udp(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        let id = self.own(peer.0);
        let failure =
            self.gate(SimEffectKind::ConnectUdp, SimEffectTarget::Addr(destination)).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("connect_udp", other),
            };
        }
        let mut st = self.state.lock();
        let rewritten = st.connect4.get(&destination).copied().unwrap_or(destination);
        let port = st.next_port;
        st.next_port += 1;
        match st.handles.get_mut(&id) {
            Some(HandleState::Peer(state)) => {
                if state.spec.protocol != PeerProtocol::Udp {
                    unreachable!("connect_udp on a TCP peer socket");
                }
                state.peer = Some(rewritten);
                if state.local.is_none() {
                    state.local = Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
                }
                Ok(())
            }
            _ => unreachable!("connect_udp on a handle that is not a peer socket"),
        }
    }

    async fn connect_peer(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        let id = self.own(peer.0);
        let failure =
            self.gate(SimEffectKind::ConnectPeer, SimEffectTarget::Addr(destination)).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("connect_peer", other),
            };
        }
        let mut st = self.state.lock();
        match st.handles.get_mut(&id) {
            Some(HandleState::Peer(state)) if state.spec.protocol == PeerProtocol::Tcp => {
                state.peer = Some(destination);
            }
            _ => unreachable!("connect_peer on a handle that is not a TCP peer socket"),
        }
        st.peer_connects.push(destination);
        drop(st);
        self.bump();
        Ok(())
    }

    async fn route_type(&self, destination: Ipv4Addr) -> Result<FibRouteType, NetlinkError> {
        match self.gate(SimEffectKind::RouteType, SimEffectTarget::Ip(destination)).await {
            None => Ok(self
                .state
                .lock()
                .route_types
                .get(&destination)
                .copied()
                .unwrap_or(FibRouteType::Unicast)),
            Some(SimFailure::Io(kind)) => Err(NetlinkError::connect(io::Error::from(kind))),
            Some(SimFailure::Netlink) => Err(NetlinkError::FibLookupMissing { destination }),
            Some(other) => mismatched("route_type", other),
        }
    }

    fn attach(
        &self,
        vsock: RecordSocket,
        peer: PeerSocket,
        mode: PairMode,
    ) -> Result<AttachedPair, AttachRefused> {
        let mut st = self.state.lock();
        let (conn, cid, flow, vsock_socket) = match st.handles.get(&self.own(vsock.0)) {
            Some(HandleState::Record(record)) => (
                record.conn,
                record.peer_cid,
                request_of(&record.first_record).map(|request| request.flow()),
                record.vsock.clone(),
            ),
            _ => unreachable!("attach with a vsock handle that is not a record socket"),
        };
        let failure = Self::sync_gate(&mut st, SimEffectKind::Attach, SimEffectTarget::Flow(flow));
        let (ctx, generation) = Self::context(&st);
        let (peer_socket, peer_spec) = match st.handles.get_mut(&self.own(peer.0)) {
            Some(HandleState::Peer(state)) => {
                // The sim learns the peer's flow here, whatever the outcome.
                state.flow = flow;
                (state.socket.clone(), state.spec)
            }
            _ => unreachable!("attach with a peer handle that is not a peer socket"),
        };
        if let Some(flow) = flow {
            st.flow_sockets
                .entry(flow)
                .or_insert_with(|| FlowSockets { cid, vsock: vsock_socket.clone(), peer: None })
                .peer = Some((peer_spec.protocol, peer_socket.clone()));
        }
        let attached = match failure {
            None => ctx.attach(&vsock_socket, &peer_socket, mode),
            Some(SimFailure::Forwarder(failure)) => Err(failure.error()),
            Some(other) => mismatched("attach", other),
        };
        let id = match attached {
            Ok(id) => id,
            Err(error) => {
                drop(st);
                self.bump();
                return Err(AttachRefused { error, vsock, peer });
            }
        };
        let _vsock_state = self.take(&mut st, vsock.0);
        let _peer_state = self.take(&mut st, peer.0);
        let (kind, peer_role) = match mode {
            PairMode::Stream => {
                (SimPairKind::Tcp, PairRole::Tcp { linger: peer_spec.abortive_linger })
            }
            PairMode::Datagram { .. } => (SimPairKind::Datagram, PairRole::Udp),
        };
        st.pairs.insert((generation, id), SimPairInfo { kind, flow, cid, generation });
        let vsock_half = self.issue(
            &mut st,
            HandleState::Pair(PairSocketState {
                id,
                generation,
                role: PairRole::Vsock,
                conn,
                flow,
                linger_cleared: false,
                socket: vsock_socket,
            }),
        );
        let peer_half = self.issue(
            &mut st,
            HandleState::Pair(PairSocketState {
                id,
                generation,
                role: peer_role,
                conn: None,
                flow,
                linger_cleared: false,
                socket: peer_socket,
            }),
        );
        drop(st);
        // The attach-time run forwards whatever waited in the halves' queues.
        self.after_cause();
        Ok(AttachedPair { id, vsock: PairSocket(vsock_half), peer: PairSocket(peer_half) })
    }

    fn abort_forward(&self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        let mut st = self.state.lock();
        let failure =
            Self::sync_gate(&mut st, SimEffectKind::AbortForward, SimEffectTarget::Forward(id));
        let (ctx, generation) = Self::context(&st);
        match failure {
            None => {}
            Some(SimFailure::Forwarder(failure)) => return Err(failure.error()),
            Some(other) => mismatched("abort_forward", other),
        }
        let result = ctx.abort(id)?;
        if result == AbortResult::Aborted {
            st.owner_aborted.insert((generation, id));
        }
        drop(st);
        self.bump();
        Ok(result)
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        let mut st = self.state.lock();
        let failure = Self::sync_gate(&mut st, SimEffectKind::AbortAll, SimEffectTarget::None);
        let (ctx, generation) = Self::context(&st);
        // The owner's call ends every stream of the context, whatever the
        // context returns (§ *Context lifetime and owner failure*, step 1):
        // a stream resolves the owner's own end of its watch first.
        let live: Vec<ForwardId> =
            st.pairs.keys().filter(|(g, _)| *g == generation).map(|(_, id)| *id).collect();
        for id in live {
            if !st.ends_taken.contains_key(&(generation, id)) {
                st.owner_aborted.insert((generation, id));
            }
        }
        let outcome = match failure {
            None => ctx.abort_all(),
            Some(SimFailure::Forwarder(failure)) => Err(failure.error()),
            Some(other) => mismatched("abort_all", other),
        };
        drop(st);
        self.bump();
        outcome
    }

    async fn next_end(&self) -> Result<ForwardEnd, ForwarderError> {
        self.wait_changed(|st| {
            let (ctx, generation) = match st.context.as_ref() {
                Some(context) => (Arc::clone(&context.ctx), context.generation),
                None => return None,
            };
            if !st.auto_ends && st.released_ends == 0 {
                return None;
            }
            let end = match ctx.take_end() {
                Ok(Some(end)) => end,
                Ok(None) => return None,
                Err(error) => return Some(Err(error)),
            };
            if !st.auto_ends {
                st.released_ends -= 1;
            }
            st.pairs.remove(&(generation, end.id));
            st.ends_taken.insert((generation, end.id), end.outcome);
            Some(Ok(end))
        })
        .await
    }

    fn pair_state(&self, id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        let st = self.state.lock();
        let (ctx, _) = Self::context(&st);
        drop(st);
        ctx.pair_state(id)
    }

    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
        let mut st = self.state.lock();
        let failure = Self::sync_gate(&mut st, SimEffectKind::Inventory, SimEffectTarget::None);
        let (ctx, _) = Self::context(&st);
        drop(st);
        match failure {
            None => ctx.inventory(),
            Some(SimFailure::Forwarder(failure)) => Err(failure.error()),
            Some(other) => mismatched("inventory", other),
        }
    }

    async fn clear_linger(&self, socket: &PairSocket) -> Result<(), FlowIoError> {
        let failure = self.gate(SimEffectKind::ClearLinger, SimEffectTarget::None).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("clear_linger", other),
            };
        }
        let mut st = self.state.lock();
        let id = self.own(socket.0);
        match st.handles.get_mut(&id) {
            Some(HandleState::Pair(pair)) if matches!(pair.role, PairRole::Tcp { .. }) => {
                pair.linger_cleared = true;
            }
            _ => unreachable!("clear_linger on a handle that is not an attached TCP half"),
        }
        Ok(())
    }

    async fn abort(&self, socket: OwnedSocket) {
        if let Some(failure) = self.gate(SimEffectKind::Abort, SimEffectTarget::None).await {
            mismatched("abort (infallible)", failure);
        }
        self.end_owned(socket, true);
    }

    async fn close(&self, socket: OwnedSocket) {
        if let Some(failure) = self.gate(SimEffectKind::Close, SimEffectTarget::None).await {
            mismatched("close (infallible)", failure);
        }
        self.end_owned(socket, false);
    }
}

/// The `io::Error` a record read reports for `errno` (test support).
pub(crate) fn errno_io(errno: KernelErrno) -> io::Error {
    errno_error(errno)
}
