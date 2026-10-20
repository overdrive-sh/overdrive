//! Host owner effect seam `GuestFlowIo` — vsock attachment replacement
//! (GH #295, EVMOD).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § DG-3 ("Host owner effect seam `GuestFlowIo`", UI-C2, UI-COV-1), with the
//! forwarding methods' contract in § *[REF] Driven port — `FlowForwarder`*
//! (EVMOD-PORT). Every item below is the one DG-3 pins, crate-private
//! (`pub(crate)`) and compiled in every build. The host guest-flow owner
//! reaches every kernel, socket, forwarder and route effect through this one
//! seam; owner sockets are seam handles and the owner never holds a
//! descriptor.
//!
//! Socket handles are issued by the seam, move-only (no `Clone`, no `Copy`),
//! `#[must_use]`, meaningless to any other seam instance. Each type admits
//! exactly the operations its socket's stage allows, and a pair socket has no
//! read or write method: the "control plane never reads payload" rule is
//! enforced by these signatures (passing a `PairSocket` to `read_record` /
//! `write_record` does not compile). The types are crate-private, so no
//! `trybuild` fixture can name them; the DELIVER reviewer checks the
//! signatures against the design.
//!
//! Contract (every method):
//!
//! - **Preconditions are types.** A handle exists only between the call that
//!   issued it and the call that consumed it, so a stale or closed handle
//!   cannot be passed; a pair socket exists only after `attach` returned Ok,
//!   so no handle reaches the forwarder before its request was read in full
//!   and its peer connected. A handle of another seam instance is
//!   unreachable from the owner (`unreachable!` in both adapters). The owner
//!   ends every handle with `abort` / `close` or a consuming call; a handle
//!   dropped otherwise is an owner defect, and `SimGuestFlowIo` reports it at
//!   its own drop.
//! - **`attach` is all-or-nothing.** Ok: the forwarder holds both halves and
//!   has queued one run per direction. Err: the forwarder holds neither, and
//!   both handles come back in [`AttachRefused`] for the owner to close.
//! - **Postconditions and ordering.** A method's effect has happened when its
//!   future completes or it returns, never before; effects of one owner task
//!   happen in its call order. A dropped future of an effect method
//!   (anything but `accept`, `read_record`, `next_end`) leaves the effect
//!   either fully applied or not applied (except `connect_peer`); the owner's
//!   next call observes which.
//! - **No drain step.** The owner never stores a goal, arms a cell, polls a
//!   target or shuts a target's send: the forwarder carries each direction's
//!   end to its target after the last accepted byte, and the owner acts only
//!   on an end record or its own abort.
//! - **Edge cases.** `read_record` with `max = 0` returns `FlowIoError::Io`
//!   of kind `InvalidInput`. `abort_forward` of an id the seam's current
//!   context did not issue returns `Ok(AlreadyEnded)`. `close_forwarder` with
//!   no context does nothing; every forwarding method called with no context
//!   held is unreachable from the owner.
//! - **Blocking.** The production adapter never blocks a runtime thread:
//!   every procfs, sysfs and cgroup read in `host_facts` /
//!   `stale_objects_present` goes through `tokio::fs` or `spawn_blocking`
//!   (dst-lint async-fs rule), and the netlink client is opened at the first
//!   effect, so `new` touches no kernel state.
//! - **Boot ordering is structural.** The forwarding methods follow an
//!   `open_forwarder` Ok, because the owner calls them only after
//!   `converge_shared` returned Ok and admission opened (`open_after_boot`),
//!   and during a `Forwarder` repair only after the reopen. A call out of that
//!   order is unreachable from the owner; the adapters treat it as
//!   `unreachable!`, not as a runtime error.
//! - **Invariants.** Only `RecordSocket` carries bytes through the owner, and
//!   only the fixed records; the forwarding methods carry the
//!   `FlowForwarder` contract unchanged.
//!
//! RED scaffold (DISTILL, Mandate 7): the production adapter
//! [`HostGuestFlowIo`] panics until the attachment-cut step (7) lands it; the
//! owner composes it from `HostSharedGuestNetworkOwner::new(GuestFlowOwnerPorts)`
//! (DG-2 row 9).

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, DG-3).
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "RED scaffold — the attachment cut (DG-2 row 9) drives the seam from the owner"
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

// R5-59 (a), flow seam: `HostGuestFlowIo` against `SimGuestFlowIo` with a
// guest-kit microVM as the peer (native metal, kvm-tests).
#[cfg(all(test, feature = "integration-tests", feature = "kvm-tests"))]
mod accept_path_tier3;

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::dial::GuestStream;
use overdrive_core::guest_transport::wire::FlowId;
use overdrive_core::guest_transport::{GuestCid, VsockListenPort};
use overdrive_core::net::InterfaceIndex;
use overdrive_core::vm::config::VsockPort;

/// The forwarder's own types (§ *[REF] Driven port — `FlowForwarder`*): the
/// pair / watch id, the context generation, an end record, a pair's
/// read-only per-direction state, the context's inventory, the pair mode, the
/// abort result and the forwarder's error.
pub(crate) use overdrive_core::guest_transport::forwarder::{
    AbortResult, ForwardEnd, ForwardId, ForwarderError, ForwarderGeneration, ForwarderInventory,
    PairMode, PairState,
};

use super::{GuestConnKey, GuestNetworkError};

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

/// Accepted on 1240 / 1243, or a dial's `V_h`: fixed records only.
#[must_use]
pub(crate) struct RecordSocket(SeamHandle);

/// From `open_peer`: bound per its spec; TCP connected by `connect_peer`,
/// UDP by `connect_udp`.
#[must_use]
pub(crate) struct PeerSocket(SeamHandle);

/// Attached to the forwarder: linger, abort, close only.
#[must_use]
pub(crate) struct PairSocket(SeamHandle);

/// Every handle, for `abort` / `close`.
#[must_use]
pub(crate) enum OwnedSocket {
    /// A record socket.
    Record(RecordSocket),
    /// A host peer socket.
    Peer(PeerSocket),
    /// An attached pair socket.
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
impl From<PairSocket> for OwnedSocket {
    fn from(socket: PairSocket) -> Self {
        Self::Pair(socket)
    }
}

/// A pair the forwarder accepted: its id and the two attached halves.
pub(crate) struct AttachedPair {
    /// The pair's id in the seam's current context.
    pub(crate) id: ForwardId,
    /// The vsock half (`V_h`).
    pub(crate) vsock: PairSocket,
    /// The peer half (D, L or H).
    pub(crate) peer: PairSocket,
}

/// A refused attach: the forwarder's error, with the two halves handed back
/// unattached so the owner closes them as the failure projection pins
/// (`HalfEnded`: TCP halves with `SO_LINGER{1,0}`).
pub(crate) struct AttachRefused {
    /// Why the forwarder refused.
    pub(crate) error: ForwarderError,
    /// The vsock half, unattached.
    pub(crate) vsock: RecordSocket,
    /// The peer half, unattached.
    pub(crate) peer: PeerSocket,
}

/// The owner's host vsock listeners, bound on `VMADDR_CID_ANY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FlowListener {
    /// `GUEST_FLOW_STREAM_PORT` (1240), `SOCK_STREAM` (kinds 0, 1 and 2).
    Stream,
    /// `GUEST_FLOW_CONTROL_PORT` (1243), `SOCK_STREAM`.
    Control,
}

/// One snapshot of the host facts `probe_startup` checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestHostFacts {
    /// Every root-namespace interface's IPv4 addresses (D8-PREFIX-CFG).
    pub(crate) interfaces: BTreeMap<InterfaceIndex, BTreeSet<Ipv4Net>>,
    /// Soft `RLIMIT_NOFILE` of the `serve` process.
    pub(crate) nofile_limit: u64,
    /// The `serve` process's cgroup v2 path.
    pub(crate) owner_cgroup: PathBuf,
}

/// A host peer socket's transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerProtocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// How the seam creates a host peer socket (H-5, D26-PINS).
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
    /// Phase 1 (D-8): opens a forwarder context of its own
    /// (`KernelFlowForwarder::open()`), runs `FlowForwarder::probe` on it and
    /// releases it before return, on `Ok` and on `Err`; the `Cleanup` stage
    /// leaves its inventory zero. Does not touch the context the seam holds
    /// or the generation count. Err: the open's error, or the first stage
    /// that failed (`ForwarderError::Probe` / `ProbeMismatch`).
    async fn probe_forwarder(&self) -> Result<(), ForwarderError>;
    /// Boot step 9 and the `Forwarder` repair: opens the serving forwarder context
    /// (`KernelFlowForwarder::open()`) and runs its Earned-Trust probe
    /// (`FlowForwarder::probe`), replacing the context the seam holds, if
    /// any, only after dropping it (the old context is closed before the new
    /// one is opened). Ok: the seam holds the new context and returns its
    /// generation. Err: the seam holds no context.
    async fn open_forwarder(&self) -> Result<ForwarderGeneration, ForwarderError>;
    /// The `Forwarder` repair's first half: drops the context the seam holds
    /// (its release aborts every pair and watch and queues no record).
    /// Infallible; a seam holding none does nothing.
    fn close_forwarder(&self);
    /// Binds and listens `listener` on (`VMADDR_CID_ANY`, `port`); a
    /// `KernelAssigned` port binds `VMADDR_PORT_ANY` (PORTS-CONFIG). Ok: the
    /// port the bind took (`getsockname`), and `accept(listener)` yields its
    /// connections; the seam holds the socket for the process's life. On a
    /// listener the seam already holds (a repair after its accept task
    /// ended): `port` is not consulted; if the held socket still listens
    /// (`SO_ACCEPTCONN` = 1) Ok with its bound port, nothing closed or bound,
    /// and `accept(listener)` serves it again; otherwise Err (an Overdrive
    /// defect: nothing closes a held listener). Err: not bound.
    async fn bind_listener(
        &self,
        listener: FlowListener,
        port: VsockListenPort,
    ) -> Result<VsockPort, FlowIoError>;
    /// GW-LOCAL: converges the one Overdrive-tagged `local <gateway>/32`
    /// route on `lo` (keep identical / replace differing / add missing). Err:
    /// `GuestNetworkError::Netlink`, the route as it was.
    async fn converge_gateway_route(&self, gateway: Ipv4Addr) -> Result<(), GuestNetworkError>;
    /// `sweep_stale`: true iff a process-scoped owner object of a previous run
    /// is present. Never reports the gateway route.
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
    /// session, or a dial's 32-byte `TcpAccept` request on `V_h`), or fails
    /// with nothing the peer can parse as a record.
    async fn write_record(&self, socket: &RecordSocket, bytes: &[u8]) -> Result<(), FlowIoError>;
    /// A dial: opens `V_h` to (`cid`, `GUEST_INBOUND_PORT`). Non-blocking;
    /// completes when connected or failed (`ENODEV` / `ECONNRESET` when the
    /// VMM is gone → `GuestDialError::Unreachable`).
    async fn connect_guest(&self, cid: GuestCid) -> Result<RecordSocket, FlowIoError>;
    /// `FlowForwarder::watch` on the dial's connected `V_h`, before its
    /// request is written (EVMOD-STREAM-END). `HalfEnded` when the VMM has
    /// already gone (the dial returns `Unreachable { errno }`).
    fn watch(&self, vsock: &RecordSocket) -> Result<ForwardId, ForwarderError>;
    /// A dial's `V_h` after `Paired`: hands it to the dialer as the
    /// `GuestStream` of § *Driving port — guest stream dial*, with its watch.
    /// `V_h` is never attached to a pair. Production wraps the owned
    /// descriptor; the sim's stream is read and written by the driver as the
    /// guest application's peer. Consumes the handle. `target` is the
    /// address the dial was called with; the returned stream's `target()` is
    /// it (D-UP-11).
    #[allow(
        clippy::wrong_self_convention,
        reason = "DG-3 pins `into_guest_stream(&self, vsock, watch, flow, target)`: it consumes the handle, not the port"
    )]
    fn into_guest_stream(
        &self,
        vsock: RecordSocket,
        watch: ForwardId,
        flow: FlowId,
        target: SocketAddrV4,
    ) -> GuestStream;
    /// Creates a host peer socket per `spec`, bound to `spec.bind` when
    /// `Some` before return (so `local_addr` is final). A UDP socket gets
    /// `SO_NO_CHECK`.
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
    /// Non-blocking TCP connect of a peer; completes on readiness
    /// (established, refused, unreachable, or the kernel's SYN retries
    /// exhausted). Non-mesh destinations: the owner sets no timeout of its
    /// own (D15-R3). Leg-F: the owner bounds it by
    /// `FLOW_PAIRING_DEADLINE − FLOW_REQUEST_DEADLINE` by dropping the
    /// future; after a dropped call the peer's connect state is unspecified
    /// and the owner's only next step is `abort`.
    async fn connect_peer(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError>;
    /// The host's FIB route type for `destination` (D26 check 4). Pure read.
    async fn route_type(&self, destination: Ipv4Addr) -> Result<HostRouteType, FlowIoError>;

    // --- forwarding: the `FlowForwarder` calls, made on the seam's handles
    //     through the context the seam holds; same preconditions, results and
    //     errors ---

    /// `FlowForwarder::attach(vsock, peer, mode)` after the request was read
    /// in full, its token matched and the peer connected (`connect_peer` /
    /// `connect_udp` returned Ok). Consumes both handles: Ok returns them as
    /// attached halves; Err hands them back unattached (`HalfEnded`, or an
    /// owner defect), so the owner ends the flow by the failure projection.
    fn attach(
        &self,
        vsock: RecordSocket,
        peer: PeerSocket,
        mode: PairMode,
    ) -> Result<AttachedPair, AttachRefused>;
    /// `FlowForwarder::abort(id)`.
    fn abort_forward(&self, id: ForwardId) -> Result<AbortResult, ForwarderError>;
    /// `FlowForwarder::abort_all()` (quiescence's first hold, the `Forwarder`
    /// recovery's step 1).
    fn abort_all(&self) -> Result<u32, ForwarderError>;
    /// The next queued end record: waits on the context's readiness
    /// (`KernelFlowForwarder::readiness_fd`), then `take_end`.
    /// Cancellation-safe: a dropped call loses no record.
    async fn next_end(&self) -> Result<ForwardEnd, ForwarderError>;
    /// `FlowForwarder::pair_state(id)`; read-only.
    fn pair_state(&self, id: ForwardId) -> Result<Option<PairState>, ForwarderError>;
    /// `FlowForwarder::inventory()`; read-only.
    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError>;

    // --- ends of sockets ---

    /// Clears `SO_LINGER` on a completed TCP half before its close.
    async fn clear_linger(&self, socket: &PairSocket) -> Result<(), FlowIoError>;
    /// Closes with `SO_LINGER{1,0}` (reset). Consumes the handle.
    async fn abort(&self, socket: OwnedSocket);
    /// Closes. Consumes the handle.
    async fn close(&self, socket: OwnedSocket);
}

// ---------------------------------------------------------------------------
// Production adapter
// ---------------------------------------------------------------------------

/// The production `GuestFlowIo` (DG-3): holds the handle table (seam ids →
/// `OwnedFd` with `tokio::io::unix::AsyncFd` readiness), the
/// `KernelFlowForwarder` after `open_forwarder` (its `readiness_fd`
/// registered with the same reactor), and the netlink client; built only by
/// `HostSharedGuestNetworkOwner::new`. Each forwarding method calls
/// `KernelFlowForwarder` with the handles' descriptors as `&dyn AsFd`. `new`
/// touches no kernel state.
///
/// RED scaffold: every method panics until the attachment-cut step (7) lands
/// the adapter.
pub(crate) struct HostGuestFlowIo {
    _scaffold: (),
}

impl HostGuestFlowIo {
    /// The adapter with an empty handle table; touches no kernel state.
    /// `HostSharedGuestNetworkOwner::new` and `run_server`'s composition
    /// (`compose_guest_network`, D-UP-16) build it.
    pub(crate) const fn new() -> Self {
        Self { _scaffold: () }
    }
}

#[async_trait::async_trait]
impl GuestFlowIo for HostGuestFlowIo {
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError> {
        panic!("{SCAFFOLD}")
    }
    async fn probe_forwarder(&self) -> Result<(), ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    async fn open_forwarder(&self) -> Result<ForwarderGeneration, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    fn close_forwarder(&self) {
        panic!("{SCAFFOLD}")
    }
    async fn bind_listener(
        &self,
        _listener: FlowListener,
        _port: VsockListenPort,
    ) -> Result<VsockPort, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn converge_gateway_route(&self, _gateway: Ipv4Addr) -> Result<(), GuestNetworkError> {
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
    fn watch(&self, _vsock: &RecordSocket) -> Result<ForwardId, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    fn into_guest_stream(
        &self,
        _vsock: RecordSocket,
        _watch: ForwardId,
        _flow: FlowId,
        _target: SocketAddrV4,
    ) -> GuestStream {
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
        _peer: &PeerSocket,
        _destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    async fn route_type(&self, _destination: Ipv4Addr) -> Result<HostRouteType, FlowIoError> {
        panic!("{SCAFFOLD}")
    }
    fn attach(
        &self,
        _vsock: RecordSocket,
        _peer: PeerSocket,
        _mode: PairMode,
    ) -> Result<AttachedPair, AttachRefused> {
        panic!("{SCAFFOLD}")
    }
    fn abort_forward(&self, _id: ForwardId) -> Result<AbortResult, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    fn abort_all(&self) -> Result<u32, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    async fn next_end(&self) -> Result<ForwardEnd, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    fn pair_state(&self, _id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
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
