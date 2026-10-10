//! Host owner effect seam `GuestFlowIo` — vsock attachment replacement
//! (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § DG-3 ("Host owner effect seam `GuestFlowIo`", UI-C2, UI-COV-1). Every
//! item below is the one that section pins, crate-private (`pub(crate)`) and
//! compiled in every build. The host guest-flow owner reaches every kernel,
//! socket, forwarder, steering and route effect through this one seam; owner
//! sockets are seam handles and the owner never holds a descriptor.
//!
//! Socket handles are issued by the seam, move-only (no `Clone`, no `Copy`),
//! `#[must_use]`, meaningless to any other seam instance. Each type admits
//! exactly the operations its socket's stage allows, so the typestate of the
//! forwarder survives the seam and a pair socket has no read or write method:
//! the "control plane never reads payload" rule is enforced by these
//! signatures. The types are crate-private, so no `trybuild` fixture can name
//! them; the DELIVER reviewer checks the signatures against the design.
//!
//! Contract (every method):
//!
//! - **Preconditions are types.** A handle exists only between the call that
//!   issued it and the call that consumed it, so a stale or closed handle
//!   cannot be passed; the per-flow order of the forwarder (install → arm →
//!   connect → route; adopt → begin → write request → route) is the only
//!   order the types admit. A handle of another seam instance is unreachable
//!   from the owner (`unreachable!` in both adapters). The owner ends every
//!   handle with `abort` / `close` or a consuming call; a handle dropped
//!   otherwise is an owner defect, and `SimGuestFlowIo` reports it at its own
//!   drop.
//! - **Consuming calls are all-or-nothing.** A call that consumes handles and
//!   returns `Err` has aborted (`SO_LINGER{1,0}`) every consumed socket and
//!   removed every entry it wrote before returning (the forwarder's "No
//!   partial pair").
//! - **Postconditions and ordering.** A method's effect has happened when its
//!   future completes, never before; effects of one owner task happen in its
//!   call order. A dropped future of an effect method (anything but `accept`,
//!   `accept_intake`, `read_record`, `hangup`, `pending_hangup`) leaves the
//!   effect either fully applied or not applied (except `connect_peer`); the
//!   owner's next call observes which.
//! - **Edge cases.** `read_record` with `max = 0` returns `FlowIoError::Io`
//!   of kind `InvalidInput`. `abort` of a pair socket before `remove_pair` is
//!   allowed (D20 orders it after). `drained` of a removed pair is
//!   `Ok(true)`.
//! - **Blocking.** The production adapter never blocks a runtime thread:
//!   every procfs, sysfs and cgroup read in `host_facts` /
//!   `stale_objects_present` goes through `tokio::fs` or `spawn_blocking`
//!   (dst-lint async-fs rule), and the netlink client is opened at the first
//!   effect, so `new` touches no kernel state.
//! - **Boot ordering is structural.** `converge_prefix_route` follows a
//!   `converge_steering` Ok, and `steer` / the forwarding methods follow a
//!   `load_forwarder` / `converge_steering` Ok, because the owner calls them
//!   only from `converge_shared` in that order and binds intake listeners
//!   only after the composition opened admission (`open_after_boot`). A call
//!   out of that order is unreachable from the owner; the adapters treat it
//!   as `unreachable!`, not as a runtime error.
//! - **Invariants.** Only `RecordSocket` and `PendingInbound` carry bytes
//!   through the owner, and only the fixed records; the forwarding methods
//!   carry the forwarder's contracts unchanged, and `steer` /
//!   `converge_steering` the steering's.
//!
//! RED scaffold (DISTILL, Mandate 7): the production adapter
//! [`HostGuestFlowIo`] panics until DELIVER (vsock new-steps 4, 7, 9 and 13)
//! lands it; the attachment cut composes it from
//! `HostSharedGuestNetworkOwner::new(GuestFlowOwnerPorts)` (DG-2 row 9).

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-steps 4, 7, 9, 13; attachment cut).
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "RED scaffold — the attachment cut (DG-2 row 9) composes the seam into the owner"
    )
)]
#![cfg_attr(
    test,
    allow(
        dead_code,
        reason = "RED scaffold — the attachment cut (DG-2 row 9) composes the production adapter"
    )
)]
#![allow(
    clippy::redundant_pub_crate,
    reason = "DG-3 pins `pub(crate)` visibility for the seam's items"
)]

#[cfg(test)]
pub(crate) mod sim;

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::GuestCid;
use overdrive_core::guest_transport::wire::FLOW_REQUEST_LEN;
use overdrive_core::net::InterfaceIndex;
use overdrive_dataplane::guest_flow::{
    GuestFlowCounters, GuestFlowError, GuestFlowInventory, UnframeConvergence,
};
use overdrive_dataplane::guest_steering::GuestSteeringError;

use super::{GuestConnKey, GuestNetworkError, InterfaceStacking};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// One handle issued by one seam instance. Private: the adapters name it;
/// the owner cannot build or inspect it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SeamHandle {
    /// The issuing seam instance.
    seam: u64,
    /// The handle's id within that instance.
    id: u64,
}

// ---------------------------------------------------------------------------
// Socket handles
// ---------------------------------------------------------------------------

/// Accepted on 1240 / 1241 / 1243, or `V_h`: fixed records only.
#[must_use]
pub(crate) struct RecordSocket(SeamHandle);

/// From `open_peer`: bound per its spec; UDP: connected by `connect_udp`.
#[must_use]
pub(crate) struct PeerSocket(SeamHandle);

/// `arm_peer` done; TCP connect allowed.
#[must_use]
pub(crate) struct ArmedPeer(SeamHandle);

/// Request read, token matched, installed.
#[must_use]
pub(crate) struct InstalledVsock(SeamHandle);

/// A bound, listening intake listener.
#[must_use]
pub(crate) struct IntakeListener(SeamHandle);

/// Accepted, `SO_LINGER{1,0}` set.
#[must_use]
pub(crate) struct IntakeChild(SeamHandle);

/// `adopt_intake` done.
#[must_use]
pub(crate) struct ParkedChild(SeamHandle);

/// `V_h` installed, request not yet written / `Paired` not received.
#[must_use]
pub(crate) struct PendingInbound(SeamHandle);

/// Routed: terminal readiness, shutdown, linger, abort, close only.
#[must_use]
pub(crate) struct PairSocket(SeamHandle);

/// Every handle, for `abort` / `close`.
#[must_use]
pub(crate) enum OwnedSocket {
    /// A record socket.
    Record(RecordSocket),
    /// A host peer socket.
    Peer(PeerSocket),
    /// An armed host peer socket.
    Armed(ArmedPeer),
    /// An installed flow vsock.
    Installed(InstalledVsock),
    /// An intake listener.
    Listener(IntakeListener),
    /// An accepted intake child.
    Child(IntakeChild),
    /// A parked intake child.
    Parked(ParkedChild),
    /// A pending inbound flow.
    Pending(PendingInbound),
    /// A routed pair socket.
    Pair(PairSocket),
}

impl From<RecordSocket> for OwnedSocket {
    fn from(socket: RecordSocket) -> Self {
        Self::Record(socket)
    }
}
impl From<PeerSocket> for OwnedSocket {
    fn from(socket: PeerSocket) -> Self {
        Self::Peer(socket)
    }
}
impl From<ArmedPeer> for OwnedSocket {
    fn from(socket: ArmedPeer) -> Self {
        Self::Armed(socket)
    }
}
impl From<InstalledVsock> for OwnedSocket {
    fn from(socket: InstalledVsock) -> Self {
        Self::Installed(socket)
    }
}
impl From<IntakeListener> for OwnedSocket {
    fn from(socket: IntakeListener) -> Self {
        Self::Listener(socket)
    }
}
impl From<IntakeChild> for OwnedSocket {
    fn from(socket: IntakeChild) -> Self {
        Self::Child(socket)
    }
}
impl From<ParkedChild> for OwnedSocket {
    fn from(socket: ParkedChild) -> Self {
        Self::Parked(socket)
    }
}
impl From<PendingInbound> for OwnedSocket {
    fn from(socket: PendingInbound) -> Self {
        Self::Pending(socket)
    }
}
impl From<PairSocket> for OwnedSocket {
    fn from(socket: PairSocket) -> Self {
        Self::Pair(socket)
    }
}

/// Names one installed pair for `drained` / `remove_pair`; no socket
/// capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PairKey(u64);

/// A routed TCP pair.
pub(crate) struct TcpPair {
    /// The pair's name.
    pub(crate) key: PairKey,
    /// The flow vsock.
    pub(crate) vsock: PairSocket,
    /// The outer leg (destination, leg-F, or intake child).
    pub(crate) outer: PairSocket,
}

/// A routed datagram association.
pub(crate) struct DatagramPair {
    /// The pair's name.
    pub(crate) key: PairKey,
    /// The 1240 stream leg.
    pub(crate) stream: PairSocket,
    /// The 1241 seqpacket leg.
    pub(crate) seqpacket: PairSocket,
    /// The host UDP socket.
    pub(crate) udp: PairSocket,
}

/// Direction of a TCP pair for `drained` (the forwarder's `FlowDirection`).
pub(crate) use overdrive_dataplane::guest_flow::FlowDirection;

/// The owner's host vsock listeners, bound on `VMADDR_CID_ANY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FlowListener {
    /// `GUEST_FLOW_STREAM_PORT` (1240), `SOCK_STREAM`.
    Stream,
    /// `GUEST_FLOW_DATAGRAM_PORT` (1241), `SOCK_SEQPACKET`.
    Seqpacket,
    /// `GUEST_FLOW_CONTROL_PORT` (1243), `SOCK_STREAM`.
    Control,
}

/// One snapshot of the host facts `probe_startup` checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestHostFacts {
    /// Every root-namespace interface.
    pub(crate) interfaces: BTreeMap<InterfaceIndex, InterfaceFacts>,
    /// `net.ipv4.ip_local_port_range`.
    pub(crate) ephemeral_ports: std::ops::RangeInclusive<u16>,
    /// Soft `RLIMIT_NOFILE` of the `serve` process.
    pub(crate) nofile_limit: u64,
    /// The `serve` process's cgroup v2 path.
    pub(crate) owner_cgroup: PathBuf,
}

/// One interface's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InterfaceFacts {
    /// Every IPv4 address configured on it.
    pub(crate) addresses: BTreeSet<Ipv4Net>,
    /// `Some` when the interface has a master or a stacking link kind
    /// (IFACE-NO-STACK).
    pub(crate) stacking: Option<InterfaceStacking>,
}

/// A host peer socket's transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerProtocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// How the seam creates a host peer socket (H-5, D26-PINS, USO-NO-SEGMENT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PeerSocketSpec {
    /// TCP or UDP.
    pub(crate) protocol: PeerProtocol,
    /// Carries `OWNER_EGRESS_MARK` (DG-9).
    pub(crate) owner_egress_mark: bool,
    /// Created with `SO_LINGER{1,0}`.
    pub(crate) abortive_linger: bool,
    /// Bound before `open_peer` returns: `Some(127.0.0.1:0)` for a connect
    /// to leg-F (its local address is registered before the connect, D9);
    /// `None` binds at connect.
    pub(crate) bind: Option<SocketAddrV4>,
}

/// Terminal readiness of a pair socket (never `EPOLLIN` / `EPOLLOUT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlowHangup {
    /// `EPOLLRDHUP`: the peer shut its write side.
    ReadHangup,
    /// `EPOLLHUP`.
    Hangup,
    /// `EPOLLERR` with the socket error.
    Error(std::io::ErrorKind),
}

/// The host's route type for a destination (D26 local-delivery check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostRouteType {
    /// `unicast`.
    Unicast,
    /// `local`.
    Local,
    /// `broadcast`.
    Broadcast,
    /// `multicast`.
    Multicast,
    /// `anycast`.
    Anycast,
    /// Any other route type.
    Other,
}

/// Why a socket operation of the seam failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FlowIoError {
    /// The socket's syscall failed; `source.kind()` is the classification the
    /// owner branches on (refused, unreachable, timed out, reset, broken pipe).
    #[error("flow socket operation failed: {source}")]
    Io {
        /// The syscall error.
        #[source]
        source: std::io::Error,
    },
}

/// The host guest-flow owner's effect seam (DG-3). See the module docs for
/// the contract every method carries.
#[async_trait::async_trait]
pub(crate) trait GuestFlowIo: Send + Sync {
    // --- startup and boot (probe_startup, sweep_stale, converge_shared) ---

    /// Reads every fact `probe_startup` checks, in one snapshot. Changes
    /// nothing. Err: a fact is unreadable (`GuestNetworkError::Io` /
    /// `Netlink`, operation `StartupProbe`); never a default value.
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError>;
    /// The forwarder's Earned-Trust probe on scratch objects
    /// (`GuestFlowForwarder::probe`'s stages). Leaves nothing loaded or
    /// attached on any return.
    async fn probe_forwarder(&self) -> Result<(), GuestFlowError>;
    /// Boot: loads the forwarder in the owner's cgroup
    /// (`GuestFlowForwarder::load`, unframe on `lo`), keeps it for the seam's
    /// life, then attaches the egress unframe program to every interface in
    /// `interfaces` (`converge_unframe_interfaces`, D5a-IFACE-BOOT). Ok: the
    /// forwarder is held, and every interface is in exactly one of
    /// `attached` / `failed`. Err: nothing held. A second call after Ok
    /// loads nothing and returns the earlier convergence.
    async fn load_forwarder(
        &self,
        interfaces: &BTreeSet<InterfaceIndex>,
    ) -> Result<UnframeConvergence, GuestFlowError>;
    /// Binds and listens `listener`. Ok: `accept(listener)` yields its
    /// connections. On a listener already bound (a repair after its accept
    /// task ended) the earlier socket is closed first. Err: not bound.
    async fn bind_listener(&self, listener: FlowListener) -> Result<(), FlowIoError>;
    /// D8a-LOAD-SWAP steps 1–2: `GuestPrefixSteering::load`, then
    /// `GuestSteeringCandidate::attach` (swap into the pinned link, or first
    /// attach then pin). Ok: the seam holds the `GuestPrefixSteering`.
    /// `Err(Load | Query)`: nothing attached. `Err(Update)`: the link runs
    /// the earlier program. `Err(Attach)`: nothing attached. `Err(Pin)`: the
    /// new link detaches on return. Never detaches a pinned link; the route
    /// is untouched on every return.
    async fn converge_steering(
        &self,
        prefix: Ipv4Net,
        gateway: Ipv4Addr,
    ) -> Result<(), GuestSteeringError>;
    /// D8a-ROUTE step 3: converge `local <prefix> dev lo src <gateway> proto
    /// GUEST_PREFIX_ROUTE_PROTOCOL` (keep / replace tagged / add). Err:
    /// `GuestNetworkError::Netlink { LocalRouteConverge }`, the route as it
    /// was.
    async fn converge_prefix_route(
        &self,
        prefix: Ipv4Net,
        gateway: Ipv4Addr,
    ) -> Result<(), GuestNetworkError>;
    /// `sweep_stale`: true iff a process-scoped owner object of a previous
    /// run is present. Never reports the route or the pinned steering.
    async fn stale_objects_present(&self) -> Result<bool, GuestNetworkError>;

    // --- connections and records ---

    /// Next connection accepted on `listener`, with its kernel-reported key.
    /// Cancellation-safe: a dropped call loses no connection. Err: the
    /// listener failed (its accept task ends; component `FlowListeners` /
    /// `ControlListener` damaged until `bind_listener` repairs it).
    async fn accept(
        &self,
        listener: FlowListener,
    ) -> Result<(RecordSocket, GuestConnKey), FlowIoError>;
    /// Reads 1..=`max` bytes of a fixed record or control message.
    /// `Ok(empty)`: end of stream before any byte. A queued connection whose
    /// peer died delivers its buffered bytes, then EOF or an error (P-37).
    /// Cancellation-safe: bytes are never lost to a dropped call.
    async fn read_record(&self, socket: &RecordSocket, max: usize) -> Result<Vec<u8>, FlowIoError>;
    /// Writes all of `bytes` (an 8-byte control message on the control
    /// session), or fails with nothing the peer can parse as a message.
    async fn write_record(&self, socket: &RecordSocket, bytes: &[u8]) -> Result<(), FlowIoError>;
    /// Opens `V_h` to (`cid`, `GUEST_INBOUND_PORT`). Non-blocking; completes
    /// when connected or failed.
    async fn connect_guest(&self, cid: GuestCid) -> Result<RecordSocket, FlowIoError>;
    /// Creates a host peer socket per `spec`, bound to `spec.bind` when
    /// `Some` before return (so `local_addr` is final). A UDP socket gets
    /// `SO_NO_CHECK` and never `UDP_SEGMENT`.
    async fn open_peer(&self, spec: PeerSocketSpec) -> Result<PeerSocket, FlowIoError>;
    /// `getsockname` as the kernel reports it.
    fn local_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError>;
    /// `getpeername` as the kernel reports it.
    fn peer_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError>;
    /// UDP `connect()` (sends nothing; a `connect4` VIP rewrite is visible in
    /// `peer_addr` afterwards).
    async fn connect_udp(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError>;
    /// Non-blocking TCP connect of an armed peer; completes on readiness
    /// (established, refused, unreachable, or the kernel's SYN retries
    /// exhausted). Non-mesh destinations: the owner sets no timeout of its
    /// own (D15-R3). Leg-F: the owner bounds it by
    /// `FLOW_PAIRING_DEADLINE − FLOW_REQUEST_DEADLINE` by dropping the
    /// future; after a dropped call the peer's connect state is unspecified
    /// and the owner's only next step is `abort`.
    async fn connect_peer(
        &self,
        peer: &ArmedPeer,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError>;
    /// The host's FIB route type for `destination` (D26 check 4). Pure read.
    async fn route_type(&self, destination: Ipv4Addr) -> Result<HostRouteType, FlowIoError>;

    // --- intake listeners (G-V8) ---

    /// Binds `address` (`workload_addr:guest_intake_host_port(i)`) and
    /// listens.
    async fn bind_intake(&self, address: SocketAddrV4) -> Result<IntakeListener, FlowIoError>;
    /// Next child of an intake listener (`SO_LINGER{1,0}` set) and its
    /// client's address. Cancellation-safe.
    async fn accept_intake(
        &self,
        listener: &IntakeListener,
    ) -> Result<(IntakeChild, SocketAddrV4), FlowIoError>;
    /// `GuestPrefixSteering::steer(intake, listener)`, with its contract.
    async fn steer(
        &self,
        intake: SocketAddrV4,
        listener: &IntakeListener,
    ) -> Result<(), GuestSteeringError>;

    // --- forwarding (the forwarder's contract and per-flow order, by type) ---

    /// `install_accepted_vsock`; the request was read and its token matched.
    async fn install_vsock(&self, vsock: RecordSocket) -> Result<InstalledVsock, GuestFlowError>;
    /// `arm_peer`; before the TCP connect.
    async fn arm_peer(
        &self,
        vsock: &InstalledVsock,
        peer: PeerSocket,
    ) -> Result<ArmedPeer, GuestFlowError>;
    /// `complete_outbound` after `connect_peer` returned Ok.
    async fn route_outbound(
        &self,
        vsock: InstalledVsock,
        peer: ArmedPeer,
    ) -> Result<TcpPair, GuestFlowError>;
    /// `install_datagram`: `udp` connected by `connect_udp`; both requests
    /// read.
    async fn install_datagram(
        &self,
        stream: RecordSocket,
        seqpacket: RecordSocket,
        udp: PeerSocket,
    ) -> Result<DatagramPair, GuestFlowError>;
    /// `register_intake_listener` (before `steer`).
    async fn register_intake(&self, listener: &IntakeListener) -> Result<(), GuestFlowError>;
    /// `unregister_intake_listener`.
    async fn unregister_intake(&self, listener: &IntakeListener) -> Result<(), GuestFlowError>;
    /// `adopt_intake`, right after `accept_intake`.
    async fn adopt_intake(&self, child: IntakeChild) -> Result<ParkedChild, GuestFlowError>;
    /// `begin_inbound` with `V_h` from `connect_guest`.
    async fn begin_inbound(
        &self,
        child: ParkedChild,
        vsock: RecordSocket,
    ) -> Result<PendingInbound, GuestFlowError>;
    /// Writes the 32-byte `TcpAccept` request on the pending `V_h`, once.
    async fn write_request(
        &self,
        pending: &PendingInbound,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> Result<(), FlowIoError>;
    /// Terminal readiness of the pending flow's intake child (outer-leg EOF
    /// before `Paired` is recorded, not an abort; § *Flow end*).
    async fn pending_hangup(&self, pending: &PendingInbound) -> FlowHangup;
    /// `complete_inbound` on `Paired`.
    async fn route_inbound(&self, pending: PendingInbound) -> Result<TcpPair, GuestFlowError>;
    /// K2 `drained` of a TCP pair.
    async fn drained(
        &self,
        pair: PairKey,
        direction: FlowDirection,
    ) -> Result<bool, GuestFlowError>;
    /// `remove_pair`, convergent (an absent pair counts as removed).
    async fn remove_pair(&self, pair: PairKey) -> Result<(), GuestFlowError>;
    /// The forwarder's inventory.
    async fn inventory(&self) -> Result<GuestFlowInventory, GuestFlowError>;
    /// The forwarder's counters.
    async fn counters(&self) -> Result<GuestFlowCounters, GuestFlowError>;

    // --- ends of sockets ---

    /// Next terminal readiness of a pair socket. Cancellation-safe.
    async fn hangup(&self, socket: &PairSocket) -> FlowHangup;
    /// `shutdown(SHUT_WR)` of a pair socket.
    async fn shutdown_write(&self, socket: &PairSocket) -> Result<(), FlowIoError>;
    /// Clears `SO_LINGER` of a pair socket (a clean close follows).
    async fn clear_linger(&self, socket: &PairSocket) -> Result<(), FlowIoError>;
    /// Closes with `SO_LINGER{1,0}` (reset). Consumes the handle.
    async fn abort(&self, socket: OwnedSocket);
    /// Closes. Closing an `IntakeListener` removes its steering entry in the
    /// same kernel step (K-L3). Consumes the handle.
    async fn close(&self, socket: OwnedSocket);
}

// ---------------------------------------------------------------------------
// Production adapter
// ---------------------------------------------------------------------------

/// The production `GuestFlowIo` (DG-3): holds the handle table (seam ids →
/// `OwnedFd` with `tokio::io::unix::AsyncFd` readiness), the
/// `GuestFlowForwarder` after `load_forwarder`, the `GuestPrefixSteering`
/// after `converge_steering`, and the netlink client; built only by
/// `HostSharedGuestNetworkOwner::new`. `new` touches no kernel state.
///
/// RED scaffold: every method panics until DELIVER lands the adapter.
pub(crate) struct HostGuestFlowIo {
    _scaffold: (),
}

impl HostGuestFlowIo {
    /// The adapter with an empty handle table; touches no kernel state.
    pub(super) const fn new() -> Self {
        Self { _scaffold: () }
    }
}

#[async_trait::async_trait]
impl GuestFlowIo for HostGuestFlowIo {
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError> {
        panic!("{SCAFFOLD}")
    }
    async fn probe_forwarder(&self) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn load_forwarder(
        &self,
        _interfaces: &BTreeSet<InterfaceIndex>,
    ) -> Result<UnframeConvergence, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn bind_listener(&self, _listener: FlowListener) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn converge_steering(
        &self,
        _prefix: Ipv4Net,
        _gateway: Ipv4Addr,
    ) -> Result<(), GuestSteeringError> {
        panic!("{SCAFFOLD}")
    }
    async fn converge_prefix_route(
        &self,
        _prefix: Ipv4Net,
        _gateway: Ipv4Addr,
    ) -> Result<(), GuestNetworkError> {
        panic!("{SCAFFOLD}")
    }
    async fn stale_objects_present(&self) -> Result<bool, GuestNetworkError> {
        panic!("{SCAFFOLD}")
    }
    async fn accept(
        &self,
        _listener: FlowListener,
    ) -> Result<(RecordSocket, GuestConnKey), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn read_record(
        &self,
        _socket: &RecordSocket,
        _max: usize,
    ) -> Result<Vec<u8>, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn write_record(&self, _socket: &RecordSocket, _bytes: &[u8]) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn connect_guest(&self, _cid: GuestCid) -> Result<RecordSocket, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn open_peer(&self, _spec: PeerSocketSpec) -> Result<PeerSocket, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    fn local_addr(&self, _peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    fn peer_addr(&self, _peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn connect_udp(
        &self,
        _peer: &PeerSocket,
        _destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn connect_peer(
        &self,
        _peer: &ArmedPeer,
        _destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn route_type(&self, _destination: Ipv4Addr) -> Result<HostRouteType, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn bind_intake(&self, _address: SocketAddrV4) -> Result<IntakeListener, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn accept_intake(
        &self,
        _listener: &IntakeListener,
    ) -> Result<(IntakeChild, SocketAddrV4), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn steer(
        &self,
        _intake: SocketAddrV4,
        _listener: &IntakeListener,
    ) -> Result<(), GuestSteeringError> {
        panic!("{SCAFFOLD}")
    }
    async fn install_vsock(&self, _vsock: RecordSocket) -> Result<InstalledVsock, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn arm_peer(
        &self,
        _vsock: &InstalledVsock,
        _peer: PeerSocket,
    ) -> Result<ArmedPeer, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn route_outbound(
        &self,
        _vsock: InstalledVsock,
        _peer: ArmedPeer,
    ) -> Result<TcpPair, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn install_datagram(
        &self,
        _stream: RecordSocket,
        _seqpacket: RecordSocket,
        _udp: PeerSocket,
    ) -> Result<DatagramPair, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn register_intake(&self, _listener: &IntakeListener) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn unregister_intake(&self, _listener: &IntakeListener) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn adopt_intake(&self, _child: IntakeChild) -> Result<ParkedChild, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn begin_inbound(
        &self,
        _child: ParkedChild,
        _vsock: RecordSocket,
    ) -> Result<PendingInbound, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn write_request(
        &self,
        _pending: &PendingInbound,
        _request: &[u8; FLOW_REQUEST_LEN],
    ) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn pending_hangup(&self, _pending: &PendingInbound) -> FlowHangup {
        panic!("{SCAFFOLD}")
    }
    async fn route_inbound(&self, _pending: PendingInbound) -> Result<TcpPair, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn drained(
        &self,
        _pair: PairKey,
        _direction: FlowDirection,
    ) -> Result<bool, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn remove_pair(&self, _pair: PairKey) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn inventory(&self) -> Result<GuestFlowInventory, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn counters(&self) -> Result<GuestFlowCounters, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
    async fn hangup(&self, _socket: &PairSocket) -> FlowHangup {
        panic!("{SCAFFOLD}")
    }
    async fn shutdown_write(&self, _socket: &PairSocket) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn clear_linger(&self, _socket: &PairSocket) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn abort(&self, _socket: OwnedSocket) {
        panic!("{SCAFFOLD}")
    }
    async fn close(&self, _socket: OwnedSocket) {
        panic!("{SCAFFOLD}")
    }
}
