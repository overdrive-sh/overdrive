//! Host forwarder for guest flows over vsock — vsock attachment replacement
//! (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — host forwarder `overdrive-dataplane::guest_flow`
//! (D1, D4, D5, D7)". Every public item below is the one that section pins,
//! with the derives and constant values it pins.
//!
//! RED scaffold (DISTILL, Mandate 7): every function and method body panics
//! until DELIVER vsock new-step 4 ("host forwarder in `overdrive-dataplane` +
//! host programs in `overdrive-bpf`") replaces it. No surface the design does
//! not pin is added here; private state is a placeholder field the crafter
//! replaces.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 4).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lints self-trip when DELIVER lands the bodies"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::BorrowedFd;

use overdrive_core::net::InterfaceIndex;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The loaded host forwarding programs, maps and links. Not pinned: dropping
/// it detaches every link and frees every map and program.
pub struct GuestFlowForwarder {
    _scaffold: (),
}

/// Opaque handle naming one forwarded flow inside the forwarder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowPairKey(u32);

/// Opaque handle for a registered intake listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntakeListenerKey(u32);

/// An accepted intake child whose bytes are parked in a host cell.
#[must_use]
pub struct ParkedIntake {
    _scaffold: (),
}

/// A host vsock socket installed, awaiting its route.
#[must_use]
pub struct InstalledVsock {
    _scaffold: (),
}

/// A host peer socket armed for install at `ACTIVE_ESTABLISHED`.
#[must_use]
pub struct ArmedPeer {
    _scaffold: (),
}

/// A host-opened flow awaiting `Paired`.
#[must_use]
pub struct PendingInbound {
    _scaffold: (),
}

/// Capacity of the forwarder: the most flows it forwards at once, and the
/// size of its host parking-cell pool.
pub const GUEST_FLOW_MAX_PAIRS: u32 = 65_536;

/// Which way through a flow pair bytes travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowDirection {
    /// From the host peer toward the guest.
    TowardGuest,
    /// From the guest toward the host peer.
    FromGuest,
}

impl GuestFlowForwarder {
    /// Loads the verdict, strparser, egress unframe, `sock_ops` and
    /// drain-counter programs; creates maps and the host parking-cell pool
    /// sized from `GUEST_FLOW_MAX_PAIRS`; attaches every program as a BPF
    /// link — `sock_ops` to `owner_cgroup`, which must not be the root cgroup,
    /// and the egress unframe to `lo` only (other interfaces via
    /// `converge_unframe_interfaces`).
    pub fn load(_owner_cgroup: BorrowedFd<'_>) -> Result<Self, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Earned-Trust probe (§ Earned Trust).
    pub fn probe(&self) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// D5a, D5a-IFACE: at boot, attaches the egress unframe program (TCX
    /// egress) to every non-`lo` interface in `interfaces` (every interface
    /// present in the host's root network namespace). Each interface lands
    /// in exactly one of `attached` / `failed`, a failure keeping its cause
    /// (`UnframeAttachError`); a failure never stops the others. Never
    /// detaches anything. `Err` only when the call cannot run at all (the
    /// program is not loaded). The only unframe attach after `load`: every
    /// interface `serve` creates exists before it (D5a-SERVE-IFACE) and no
    /// interface registered after boot is configured (D5a-LATE, ADR-0068
    /// § 5), so none is attached later.
    pub fn converge_unframe_interfaces(
        &self,
        _interfaces: &BTreeSet<InterfaceIndex>,
    ) -> Result<UnframeConvergence, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- inbound (TcpAccept, host opener) ---

    /// Tags `listener` with an entry in the forwarder's dedicated
    /// clone-flagged socket-storage map (`BPF_F_CLONE` is set on the map at
    /// creation; INTAKE-TAG-MAP), so every child accepted on it carries a copy
    /// of the tag from creation; no other socket is ever stored in that map;
    /// `sock_ops` installs and parks at `PASSIVE_ESTABLISHED` exactly the
    /// children that carry it (M-1). Never keyed by port or address.
    pub fn register_intake_listener(
        &self,
        _listener: BorrowedFd<'_>,
    ) -> Result<IntakeListenerKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removes the tag of a listener registered by
    /// [`Self::register_intake_listener`].
    pub fn unregister_intake_listener(
        &self,
        _key: IntakeListenerKey,
    ) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Called right after `accept()`; re-arms the child. `IntakeNotParked` if
    /// no cell was free at establishment.
    pub fn adopt_intake(&self, _child: BorrowedFd<'_>) -> Result<ParkedIntake, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Installs `vsock` (connected, request not yet written) with route
    /// vsock → intake child.
    pub fn begin_inbound(
        &self,
        _intake: ParkedIntake,
        _vsock: BorrowedFd<'_>,
    ) -> Result<PendingInbound, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// On `Paired`: routes the cell → vsock and re-arms it.
    pub fn complete_inbound(
        &self,
        _pending: PendingInbound,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- outbound TCP (TcpConnect, host acceptor) ---

    /// Precondition: the caller has already read the 32-byte request from
    /// `vsock` in userspace (H-1) and matched its launch token (ATTR-TOKEN).
    /// Installing earlier would hand the request to the verdict, which drops
    /// it on the route miss.
    pub fn install_accepted_vsock(
        &self,
        _vsock: BorrowedFd<'_>,
    ) -> Result<InstalledVsock, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Must be called before `connect()` on `peer`.
    pub fn arm_peer(
        &self,
        _vsock: &InstalledVsock,
        _peer: BorrowedFd<'_>,
    ) -> Result<ArmedPeer, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// After `connect()` succeeded: verifies `peer` was installed at
    /// establishment (`PeerNotInstalled` otherwise) and routes vsock → peer.
    pub fn complete_outbound(
        &self,
        _vsock: InstalledVsock,
        _peer: ArmedPeer,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- datagram (host acceptor) ---

    /// `host_udp` must be a connected IPv4 UDP socket with `SO_NO_CHECK`;
    /// `stream` and `seqpacket` must have had their requests read already.
    /// Registers the unframe tuple from `host_udp`'s kernel-reported local
    /// address and peer (`getsockname` / `getpeername`, i.e. after any
    /// `connect4` VIP rewrite), never from the requested destination.
    pub fn install_datagram(
        &self,
        _stream: BorrowedFd<'_>,
        _seqpacket: BorrowedFd<'_>,
        _host_udp: BorrowedFd<'_>,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- end of flow ---

    /// K2: true when every byte received toward `direction`'s target has been
    /// forwarded and sent across every hop between them — including the
    /// parking cell, when the flow's path has one (L-6). Exact counters;
    /// never evicted.
    pub fn drained(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
    ) -> Result<bool, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removes routes, tuples, counters and members of `key` and releases its
    /// cell. Convergent: absent entries count as removed.
    pub fn remove_pair(&self, _key: FlowPairKey) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removes every pair, as [`Self::remove_pair`] does for one.
    pub fn remove_all(&self) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// The forwarder's current pairs, cells, intake listeners and links.
    pub fn inventory(&self) -> Result<GuestFlowInventory, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// The forwarder's drop and keep counters.
    pub fn counters(&self) -> Result<GuestFlowCounters, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
}

/// What the forwarder holds right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFlowInventory {
    /// Forwarded flow pairs.
    pub pairs: u32,
    /// Parking cells free in the pool.
    pub free_cells: u32,
    /// Registered intake listeners.
    pub intake_listeners: u32,
    /// Which program links are attached.
    pub links_attached: GuestFlowLinks,
    /// Non-`lo` interfaces currently carrying the egress unframe link.
    pub unframe_interfaces: BTreeSet<InterfaceIndex>,
}

/// Which of the forwarder's program links are attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the design pins one bool per link (feature-delta `GuestFlowLinks`)"
)]
pub struct GuestFlowLinks {
    /// The verdict program.
    pub verdict: bool,
    /// The `sock_ops` program on the owner's cgroup.
    pub sock_ops: bool,
    /// The egress unframe program on `lo`.
    pub unframe_lo: bool,
    /// The drain-counter program.
    pub drain_counter: bool,
}

/// The outcome of the boot attach. Every requested interface is in exactly
/// one of the two maps; a failure keeps its cause.
#[derive(Debug, Default)]
pub struct UnframeConvergence {
    /// Interfaces now carrying the egress unframe link.
    pub attached: BTreeSet<InterfaceIndex>,
    /// Interfaces whose attach failed, with the cause.
    pub failed: BTreeMap<InterfaceIndex, UnframeAttachError>,
}

/// D5a-IFACE-BOOT: why the egress unframe program could not be attached to
/// an interface present at boot.
///
/// Every variant refuses startup
/// (`GuestNetworkError::UnframeAttach`); the variant names the cause for the
/// operator, and nothing retries it. Classified from the errno carried by
/// the source (P-44): `ENOMEM` → `KernelMemory`; every other errno →
/// `Attach` — observed: `EEXIST` (already attached), `ENODEV` (interface
/// gone between enumeration and attach), `EMFILE` (descriptor limit, from
/// name resolution), `ERANGE` (the interface's TCX egress set is full).
/// The attach of a loaded program raises no `EPERM`.
#[derive(Debug, thiserror::Error)]
pub enum UnframeAttachError {
    /// The kernel could not allocate the TCX link (`ENOMEM`): node memory
    /// pressure at boot.
    #[error("the kernel could not allocate the egress unframe link on interface {ifindex}")]
    KernelMemory {
        /// The interface.
        ifindex: InterfaceIndex,
        /// The attach error.
        #[source]
        source: aya::programs::ProgramError,
    },
    /// Any other failure (`EEXIST`, `ENODEV`, `EMFILE`, `ERANGE`, or any
    /// errno not listed). On the appliance nothing but Overdrive attaches to
    /// these interfaces (A-31), so it is an Overdrive defect, reported with
    /// its cause.
    #[error("attaching the egress unframe program to interface {ifindex} failed")]
    Attach {
        /// The interface.
        ifindex: InterfaceIndex,
        /// The attach error.
        #[source]
        source: aya::programs::ProgramError,
    },
}

impl UnframeAttachError {
    /// The interface the attach failed on.
    pub fn ifindex(&self) -> InterfaceIndex {
        panic!("{SCAFFOLD}")
    }
}

/// The forwarder's drop and keep counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuestFlowCounters {
    /// Skbs dropped on a route miss.
    pub route_miss: u64,
    /// Redirects that failed.
    pub redirect_failed: u64,
    /// Zero-length (FIN-only) TCP skbs dropped by a verdict.
    pub fin_only_dropped: u64,
    /// Guest datagrams over the payload limit.
    pub datagram_oversize: u64,
    /// Guest datagrams whose frame did not parse.
    pub datagram_malformed: u64,
    /// Egress packets on no registered tuple.
    pub unframe_tuple_miss: u64,
    /// Skbs the egress program could not linearize.
    pub linearize_failed: u64,
    /// Intake children not parked at establishment.
    pub intake_not_parked: u64,
    /// D5a: frames the verdict kept (empty datagram).
    pub frame_kept_empty: u64,
    /// D5a: frames the verdict kept (frame-shaped payload).
    pub frame_kept_escaped: u64,
    /// D5a: later fragments moved back after a first-fragment unframe.
    pub unframe_fragment_shifted: u64,
    /// D5a: later fragments seen with no first-fragment record (passed
    /// unchanged).
    pub unframe_fragment_orphan: u64,
}

/// One of the forwarder's programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowProgram {
    /// The sockmap verdict.
    Verdict,
    /// The stream parser.
    StreamParser,
    /// The TCX egress unframe.
    Unframe,
    /// The `sock_ops` program.
    SockOps,
    /// The drain counter.
    DrainCounter,
}

/// One of the forwarder's maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowMap {
    /// The socket map.
    Sockets,
    /// Redirect routes.
    Routes,
    /// Armed sockets.
    Arms,
    /// The clone-flagged intake-tag storage.
    IntakeTags,
    /// Parking cells.
    Parking,
    /// Registered unframe tuples.
    Tuples,
    /// The fragment-tracking map.
    Fragments,
    /// Drain counters.
    DrainCounters,
    /// Drop and keep counters.
    Counters,
}

/// A stage of the forwarder's startup probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowProbeStage {
    /// The owner's cgroup is not the root cgroup.
    OwnerCgroupNotRoot,
    /// Registering the probe CID with vhost.
    VhostCidRegister,
    /// Every link attached.
    LinksAttached,
    /// Inserting a vsock socket into the socket map.
    SockmapVsockInsert,
    /// Parking at establishment.
    EstablishmentParking,
    /// A loopback stream round trip.
    LoopbackStreamRoundTrip,
    /// A half-close drain.
    HalfCloseDrain,
    /// An empty datagram.
    EmptyDatagram,
    /// A maximum-size datagram.
    MaxDatagram,
    /// A frame-shaped (escaped) datagram.
    EscapedFrame,
    /// An oversize datagram is dropped.
    OversizeDropped,
    /// A seqpacket record.
    SeqpacketRecord,
    /// Removing every probe entry.
    Cleanup,
}

/// The kind of socket a forwarder call requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowSocketKind {
    /// A vsock stream socket.
    VsockStream,
    /// A vsock seqpacket socket.
    VsockSeqpacket,
    /// An established TCPv4 socket.
    TcpV4Established,
    /// An unconnected TCPv4 socket.
    TcpV4Unconnected,
    /// A listening TCPv4 socket.
    TcpV4Listener,
    /// A connected UDPv4 socket.
    UdpV4Connected,
}

/// What a probe stage observed instead of its expected outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowProbeObservation {
    /// The bytes received differ from those sent.
    BytesDiffer,
    /// Nothing was received.
    NothingReceived,
    /// Early bytes were lost.
    EarlyBytesLost,
    /// The tail was truncated.
    TailTruncated,
    /// A datagram arrived split.
    DatagramSplit,
    /// An oversize datagram was delivered.
    OversizeDelivered,
    /// Probe entries remained after cleanup.
    ResidueRemains {
        /// Pairs remaining.
        pairs: u32,
    },
}

/// The role of a host socket the owner armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRole {
    /// An accepted intake child.
    InboundIntake,
    /// A host peer of an outbound flow.
    OutboundPeer,
}

/// Why a forwarder call failed.
#[derive(Debug, thiserror::Error)]
pub enum GuestFlowError {
    /// Loading a program failed.
    #[error("loading guest-flow program {program:?} failed")]
    Load {
        /// The program.
        program: GuestFlowProgram,
        /// The load error.
        #[source]
        source: aya::EbpfError,
    },
    /// Attaching a program failed.
    #[error("attaching guest-flow program {program:?} failed")]
    Attach {
        /// The program.
        program: GuestFlowProgram,
        /// The attach error.
        #[source]
        source: aya::programs::ProgramError,
    },
    /// The owner's cgroup is the root cgroup.
    #[error("the owner's cgroup is the root cgroup; sock_ops would apply to every host socket")]
    OwnerCgroupIsRoot,
    /// Updating a map failed.
    #[error("updating guest-flow map {map:?} failed")]
    MapUpdate {
        /// The map.
        map: GuestFlowMap,
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// Deleting from a map failed.
    #[error("deleting from guest-flow map {map:?} failed")]
    MapDelete {
        /// The map.
        map: GuestFlowMap,
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// Reading a socket cookie or `TCP_INFO` failed.
    #[error("reading a socket cookie or TCP_INFO failed")]
    SocketQuery {
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Re-arming a socket with `SO_RCVLOWAT` failed.
    #[error("re-arming a socket with SO_RCVLOWAT failed")]
    Rearm {
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A descriptor is not the socket kind the call requires.
    #[error("socket is not a {expected:?}")]
    UnsupportedSocket {
        /// The kind required.
        expected: GuestFlowSocketKind,
    },
    /// The intake child was not parked at establishment.
    #[error("intake child was not parked at establishment")]
    IntakeNotParked,
    /// The socket was not installed at establishment.
    #[error("{role:?} socket was not installed at establishment")]
    PeerNotInstalled {
        /// The socket's role.
        role: FlowRole,
    },
    /// Every pair slot is in use.
    #[error("guest-flow pair capacity {capacity} exhausted")]
    PairCapacity {
        /// The capacity.
        capacity: u32,
    },
    /// A probe stage failed.
    #[error("guest-flow probe stage {stage:?} failed")]
    Probe {
        /// The stage.
        stage: GuestFlowProbeStage,
        /// The failure.
        #[source]
        source: Box<Self>,
    },
    /// A probe stage observed something other than its expected outcome.
    #[error("guest-flow probe stage {stage:?} observed {observed:?}")]
    ProbeMismatch {
        /// The stage.
        stage: GuestFlowProbeStage,
        /// What it observed.
        observed: GuestFlowProbeObservation,
    },
    /// Probe I/O failed.
    #[error("probe I/O failed at stage {stage:?}")]
    ProbeIo {
        /// The stage.
        stage: GuestFlowProbeStage,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
}
