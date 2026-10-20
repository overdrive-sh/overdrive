//! Host forwarder for guest flows over vsock — vsock attachment replacement
//! (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — host forwarder `overdrive-dataplane::guest_flow`".
//! Every public item in the first part of this file is the one that section
//! pins, with the derives, fields, variants and constant values it pins: the
//! verdict, `sock_ops`, `flow_end_classifier` (raw tracepoint on
//! `vsock:virtio_transport_recv_pkt`), `drain_sent_a` / `drain_sent_b`
//! (`fexit(skb_send_sock)`) and `drain_level_read` (`fentry(sock_poll)`)
//! programs, the `BPF_F_MMAPABLE` drain-cell array and `FLOW_EVENT_RING`
//! (R2-END, R2-DRAIN, UDP-EMPTY, UDP-SEQ).
//!
//! RED scaffold (DISTILL, Mandate 7): every function and method body panics
//! until DELIVER step 4 ("host forwarder in `overdrive-dataplane` + host
//! programs in `overdrive-bpf`") replaces it. Plain data (`DrainStep::as_str`)
//! is real. Private state is a placeholder field the crafter replaces.
//!
//! The items under "Removed by the 2026-10-11 rulings" at the end of the file
//! (the host egress unframe and the intake-steering surface) are the
//! feature-delta § *Roadmap impact* → *Bridge-era code for DELIVER to delete*
//! list D range of this file; DELIVER deletes them with the tests that still
//! defend them. They are not part of the current contract and nothing new
//! calls them.

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER step 4).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lints self-trip when DELIVER lands the bodies"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::BorrowedFd;

use overdrive_core::guest_transport::wire::{
    CellEpoch, FlowCell, FlowEvent, FlowEventKind, FlowWireError,
};
use overdrive_core::net::InterfaceIndex;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The loaded host forwarding programs, maps, drain cells, flow-event ring
/// and links. Not pinned: dropping it detaches every link and frees every
/// map and program.
pub struct GuestFlowForwarder {
    _scaffold: (),
}

/// Opaque handle naming one flow's slot inside the forwarder: its drain
/// cells, its classifier registration and, for an installed flow, its routes
/// and members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowPairKey(u32);

impl FlowPairKey {
    /// The flow's slot in the drain-cell array (cells `2 × slot` and
    /// `2 × slot + 1`); the `cell` field of every flow-event record of this
    /// flow.
    pub fn cell(self) -> FlowCell {
        panic!("{SCAFFOLD}")
    }
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

/// Node bound on concurrent flows (pair table, slots, fd budget term).
pub const GUEST_FLOW_MAX_PAIRS: u32 = 65_536;

/// Size of one drain cell; the cell array holds `2 × GUEST_FLOW_MAX_PAIRS`.
pub const GUEST_FLOW_CELL_LEN: usize = 64;

/// Host direction of a flow. `TowardGuest` = into the flow vsock (cell
/// `2 × slot`), `FromGuest` = out of the flow vsock (cell `2 × slot + 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowDirection {
    /// Into the flow vsock (cell `2 × slot`).
    TowardGuest,
    /// Out of the flow vsock (cell `2 × slot + 1`).
    FromGuest,
}

/// R2-DRAIN: the result of reading `sent` after `arm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainRead {
    /// `sent >= goal` and this call won `cmpxchg(fired, 0, 1)`: the direction
    /// is drained; no `DrainComplete` record is written for it.
    Complete,
    /// Either `sent < goal` (`short = true`), or a kernel program won
    /// `fired` (`short = false`; its `DrainComplete` record is or will be in
    /// the ring). The owner waits for the record.
    AwaitRecord {
        /// `sent < goal` at the read.
        short: bool,
    },
}

/// The userspace-visible fields of one drain cell, read atomically per field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowCellSnapshot {
    /// Bytes the direction's target accepted since the cell was initialised.
    pub sent: u64,
    /// The byte count at which the direction is drained.
    pub goal: u64,
    /// Bytes the verdict redirected from the direction's source.
    pub fwd: u64,
    /// The cell's epoch.
    pub epoch: CellEpoch,
    /// The goal is stored.
    pub armed: bool,
    /// Completion is reported.
    pub fired: bool,
    /// The direction's sending side has ended on the vsock.
    pub ended: bool,
    /// Redirects to the cell's target refused `SK_DROP` (A3).
    pub drops: u32,
    /// Lost `EndHalfClose` / `EndAbort` / `DrainComplete` / `RedirectRefused`
    /// records of this cell (A7).
    pub ring_lost: u32,
    /// Lost `EmptyDatagram` notices of this cell (UDP-EMPTY).
    pub empty_lost: u32,
}

/// `recursion_misses` of the two `fexit(skb_send_sock)` programs, read from
/// their program file descriptors' fdinfo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainRecursionMisses {
    /// `drain_sent_a`'s `recursion_misses`.
    pub sent_a: u64,
    /// `drain_sent_b`'s `recursion_misses`.
    pub sent_b: u64,
}

/// The pinned order of the drain sub-steps of one direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainStep {
    /// Step 1, `store_goal`.
    StoreGoal,
    /// Step 2, `arm`.
    Arm,
    /// Step 3, `read_sent`.
    ReadSent,
    /// Step 4 (conditional), `backstop_poll`.
    BackstopPoll,
    /// Step 5, `shutdown_write`.
    ShutdownWrite,
}

impl DrainStep {
    /// The step's label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StoreGoal => "store_goal",
            Self::Arm => "arm",
            Self::ReadSent => "read_sent",
            Self::BackstopPoll => "backstop_poll",
            Self::ShutdownWrite => "shutdown_write",
        }
    }
}

impl GuestFlowForwarder {
    /// Loads the verdict, `sock_ops`, `flow_end_classifier` (raw tracepoint,
    /// `tp_btf`, on `vsock:virtio_transport_recv_pkt`), `drain_sent_a` and
    /// `drain_sent_b` (each `fexit(skb_send_sock)`) and `drain_level_read`
    /// (`fentry(sock_poll)`) programs; creates the maps, the drain cell array
    /// (`BPF_ARRAY`, `BPF_F_MMAPABLE`, `2 × GUEST_FLOW_MAX_PAIRS` cells,
    /// mapped into this process) and `FLOW_EVENT_RING`; attaches every
    /// program as a BPF link — `sock_ops` to `owner_cgroup`. Precondition:
    /// owner placement was checked (`GuestNetworkError::OwnerCgroupPlacement`,
    /// D24), so `owner_cgroup` is at or below `overdrive.slice`, never the
    /// root cgroup.
    pub fn load(_owner_cgroup: BorrowedFd<'_>) -> Result<Self, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Earned-Trust probe (§ Earned Trust).
    pub fn probe(&self) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- outbound TCP (TcpConnect, host acceptor) ---

    /// Precondition: the caller has already read the 32-byte request from
    /// `vsock` in userspace (H-1) and matched its launch token (ATTR-TOKEN).
    /// Installing earlier would hand the request to the verdict, which drops
    /// it on the route miss. Takes a free slot, initialises both its cells
    /// (new epoch; every counter and flag zero) and registers the vsock with
    /// `flow_end_classifier`. `PairCapacity` when no slot is free.
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

    // --- host-opened flow (TcpAccept, GuestStreamDial) ---

    /// `vsock`: a connected host vsock STREAM to (CID, `GUEST_INBOUND_PORT`),
    /// before its request is written. Takes a free slot, initialises its
    /// cells and registers the vsock with `flow_end_classifier`. Installs
    /// nothing: the vsock is no `SockHash` member and no verdict runs on it;
    /// its bytes are read and written by the dialer (INB-1). Only
    /// `EndHalfClose` / `EndAbort` records name this slot.
    pub fn register_dialed_vsock(
        &self,
        _vsock: BorrowedFd<'_>,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- datagram (host acceptor) ---

    /// `host_udp` must be a connected IPv4 UDP socket with `SO_NO_CHECK`;
    /// `stream` and `seqpacket` must have had their requests read already.
    /// Installs `seqpacket` with route → `host_udp` (every record length-
    /// checked and stripped), `host_udp` with route → `stream` (framed) and
    /// `stream` as a target; takes a slot and initialises its cells, which
    /// carry the association's `EmptyDatagram` notices and loss counts.
    pub fn install_datagram(
        &self,
        _stream: BorrowedFd<'_>,
        _seqpacket: BorrowedFd<'_>,
        _host_udp: BorrowedFd<'_>,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    // --- end of flow (R2-END, R2-DRAIN; called in this order per direction) ---

    /// Step 1. Stores `goal` into the direction's cell. TCP source: the
    /// source's `tcpi_bytes_received − 1` read at its EOF (FIN counted).
    /// vsock source: the cell's own `fwd` at the classified `EndHalfClose`
    /// (T4: equals the peer's final-hop `sent`).
    pub fn store_goal(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
        _goal: u64,
    ) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Step 2. Sets `armed = 1`, sequentially consistent with the programs'
    /// read of `armed` after their `sent` update, so the owner's step 3 and
    /// every later program run cannot both miss the goal.
    pub fn arm(&self, _key: FlowPairKey, _direction: FlowDirection) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Step 3. Reads `sent`; when `sent >= goal`, attempts
    /// `cmpxchg(fired, 0, 1)`.
    pub fn read_sent(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
    ) -> Result<DrainRead, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Step 4 (conditional). A zero-event `poll(target, 0, 0)` that runs
    /// `drain_level_read` on `target`, the direction's target socket. Called
    /// once after an `AwaitRecord { short: true }` at arm time, and for every
    /// armed, incomplete direction when either value of
    /// `drain_recursion_misses` has risen at the owner's counter read.
    pub fn backstop_poll(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
        _target: BorrowedFd<'_>,
    ) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Step 5. `shutdown(SHUT_WR)` on `target`, the direction's target. When
    /// `target` is the flow vsock (`TowardGuest`), first sets the cell's
    /// `ended`. Precondition for an installed flow: the direction's `fired`
    /// is set. For a dialed vsock (no drain) it is the dialer's
    /// `shutdown(Write)` and has no precondition.
    pub fn shutdown_write(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
        _target: BorrowedFd<'_>,
    ) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Reads one direction's cell.
    pub fn cell(
        &self,
        _key: FlowPairKey,
        _direction: FlowDirection,
    ) -> Result<FlowCellSnapshot, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// The ring's readiness descriptor, for the owner's event loop.
    pub fn flow_event_fd(&self) -> BorrowedFd<'_> {
        panic!("{SCAFFOLD}")
    }

    /// Takes the next record of `FLOW_EVENT_RING`, `None` when empty.
    pub fn next_flow_event(&self) -> Result<Option<FlowEvent>, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// `recursion_misses` of `drain_sent_a` and `drain_sent_b`.
    pub fn drain_recursion_misses(&self) -> Result<DrainRecursionMisses, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removes routes, members, the classifier registration and the arms of
    /// `key` and frees its slot; the next flow on the slot gets a new epoch.
    /// Convergent: absent entries count as removed.
    pub fn remove_pair(&self, _key: FlowPairKey) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removes every pair, as [`Self::remove_pair`] does for one.
    pub fn remove_all(&self) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// The forwarder's current pairs, free slots and links (startup probe).
    pub fn inventory(&self) -> Result<GuestFlowInventory, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// The forwarder's global counters.
    pub fn counters(&self) -> Result<GuestFlowCounters, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
}

/// What the forwarder holds right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFlowInventory {
    /// Live flows.
    pub pairs: u32,
    /// Free slots.
    pub free_slots: u32,
    /// Which program links are attached.
    pub links_attached: GuestFlowLinks,
}

/// Which of the forwarder's program links are attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the design pins one bool per link (feature-delta `GuestFlowLinks`)"
)]
pub struct GuestFlowLinks {
    /// The verdict.
    pub verdict: bool,
    /// `sock_ops` on the owner's cgroup.
    pub sock_ops: bool,
    /// `flow_end_classifier` (raw tracepoint).
    pub flow_end_classifier: bool,
    /// `drain_sent_a` (`fexit(skb_send_sock)`).
    pub drain_sent_a: bool,
    /// `drain_sent_b` (`fexit(skb_send_sock)`).
    pub drain_sent_b: bool,
    /// `drain_level_read` (`fentry(sock_poll)`).
    pub drain_level_read: bool,
}

/// The forwarder's global counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuestFlowCounters {
    /// Skbs dropped on a route miss.
    pub route_miss: u64,
    /// Redirects refused `SK_DROP` (A3).
    pub redirect_refused: u64,
    /// FIN-only (zero-length) TCP skbs dropped.
    pub fin_only_dropped: u64,
    /// `bpf_skb_pull_data` failures before a UDP / SEQPACKET redirect.
    pub linearize_failed: u64,
    /// UDP-SEQ: `skb.len − 8 < declared`.
    pub datagram_short: u64,
    /// UDP-SEQ: `skb.len − 8 > declared`.
    pub datagram_long: u64,
    /// UDP-SEQ: `skb.len < 8`.
    pub datagram_lt8: u64,
    /// UDP-SEQ: magic ≠ `OVD1`.
    pub datagram_bad_magic: u64,
    /// UDP-SEQ: declared length > `MAX_GUEST_DATAGRAM_PAYLOAD`.
    pub datagram_oversize: u64,
    /// UDP-EMPTY: declared-length-0 records dropped with a notice.
    pub datagram_empty: u64,
    /// `bpf_ringbuf_output` failures, every kind.
    pub ring_dropped: u64,
}

/// One of the forwarder's programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowProgram {
    /// The verdict.
    Verdict,
    /// `sock_ops`.
    SockOps,
    /// `flow_end_classifier`.
    FlowEndClassifier,
    /// `drain_sent_a`.
    DrainSentA,
    /// `drain_sent_b`.
    DrainSentB,
    /// `drain_level_read`.
    DrainLevelRead,
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
    /// The drain-cell array.
    Cells,
    /// `flow_end_classifier` registrations.
    ClassifierRegistrations,
    /// `FLOW_EVENT_RING`.
    FlowEventRing,
    /// Global counters.
    Counters,
}

/// A stage of the forwarder's startup probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowProbeStage {
    /// Every link attached.
    LinksAttached,
    /// Inserting a vsock socket into the socket map.
    SockmapVsockInsert,
    /// A loopback stream round trip.
    LoopbackStreamRoundTrip,
    /// A half-close drain.
    HalfCloseDrain,
    /// A flow end classified.
    FlowEndClassified,
    /// An empty datagram.
    EmptyDatagram,
    /// A maximum-size datagram.
    MaxDatagram,
    /// A frame-shaped payload.
    FrameShapedPayload,
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
    /// The tail was truncated.
    TailTruncated,
    /// A datagram arrived split.
    DatagramSplit,
    /// An oversize datagram was delivered.
    OversizeDelivered,
    /// An expected flow-event record did not arrive.
    RecordMissing {
        /// The kind expected.
        kind: FlowEventKind,
    },
    /// A flow-event record arrived that the stage did not expect.
    RecordUnexpected {
        /// The kind received.
        kind: FlowEventKind,
    },
    /// Probe entries remained after cleanup.
    ResidueRemains {
        /// Pairs remaining.
        pairs: u32,
    },
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
    /// Reading a socket cookie, socket address or `TCP_INFO` failed.
    #[error("reading a socket cookie, socket address or TCP_INFO failed")]
    SocketQuery {
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
    /// The outbound peer socket was not installed at establishment.
    #[error("outbound peer socket was not installed at establishment")]
    PeerNotInstalled,
    /// Every slot is in use.
    #[error("guest-flow pair capacity {capacity} exhausted")]
    PairCapacity {
        /// The capacity.
        capacity: u32,
    },
    /// The key names no live flow.
    #[error("flow {key:?} is not live in the forwarder")]
    UnknownFlow {
        /// The key.
        key: FlowPairKey,
    },
    /// A drain sub-step was called out of the pinned order.
    #[error(
        "drain step {step:?} of flow {key:?} direction {direction:?} is out of the pinned order"
    )]
    DrainStepOutOfOrder {
        /// The flow.
        key: FlowPairKey,
        /// The direction.
        direction: FlowDirection,
        /// The step called.
        step: DrainStep,
    },
    /// A drain sub-step's syscall failed.
    #[error("drain step {step:?} failed")]
    DrainIo {
        /// The step.
        step: DrainStep,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Reading the flow-event ring failed.
    #[error("reading the flow-event ring failed")]
    FlowEventRing {
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// A flow-event record failed to decode.
    #[error("a flow-event record failed to decode")]
    FlowEventDecode {
        /// The decode error.
        #[source]
        source: FlowWireError,
    },
    /// Reading a program's statistics failed.
    #[error("reading program statistics of {program:?} failed")]
    ProgramStats {
        /// The program.
        program: GuestFlowProgram,
        /// The I/O error.
        #[source]
        source: std::io::Error,
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

// ---------------------------------------------------------------------------
// Removed by the 2026-10-11 rulings (feature-delta § *Roadmap impact* →
// *Bridge-era code for DELIVER to delete*, list D). Not part of the current
// contract; DELIVER deletes them with the tests that still defend them.
// ---------------------------------------------------------------------------

/// Opaque handle for a registered intake listener (removed mechanism).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntakeListenerKey(u32);

/// An accepted intake child whose bytes are parked (removed mechanism).
#[must_use]
pub struct ParkedIntake {
    _scaffold: (),
}

/// A host-opened flow awaiting `Paired` (removed mechanism).
#[must_use]
pub struct PendingInbound {
    _scaffold: (),
}

impl GuestFlowForwarder {
    /// Removed mechanism (host egress unframe).
    pub fn converge_unframe_interfaces(
        &self,
        _interfaces: &BTreeSet<InterfaceIndex>,
    ) -> Result<UnframeConvergence, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removed mechanism (intake steering).
    pub fn register_intake_listener(
        &self,
        _listener: BorrowedFd<'_>,
    ) -> Result<IntakeListenerKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removed mechanism (intake steering).
    pub fn unregister_intake_listener(
        &self,
        _key: IntakeListenerKey,
    ) -> Result<(), GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removed mechanism (intake steering).
    pub fn adopt_intake(&self, _child: BorrowedFd<'_>) -> Result<ParkedIntake, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removed mechanism (intake steering).
    pub fn begin_inbound(
        &self,
        _intake: ParkedIntake,
        _vsock: BorrowedFd<'_>,
    ) -> Result<PendingInbound, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }

    /// Removed mechanism (intake steering).
    pub fn complete_inbound(
        &self,
        _pending: PendingInbound,
    ) -> Result<FlowPairKey, GuestFlowError> {
        panic!("{SCAFFOLD}")
    }
}

/// The outcome of the boot unframe attach (removed mechanism).
#[derive(Debug, Default)]
pub struct UnframeConvergence {
    /// Interfaces now carrying the egress unframe link.
    pub attached: BTreeSet<InterfaceIndex>,
    /// Interfaces whose attach failed, with the cause.
    pub failed: BTreeMap<InterfaceIndex, UnframeAttachError>,
}

/// Why the egress unframe program could not be attached (removed mechanism).
#[derive(Debug, thiserror::Error)]
pub enum UnframeAttachError {
    /// `ENOMEM`.
    #[error("the kernel could not allocate the egress unframe link on interface {ifindex}")]
    KernelMemory {
        /// The interface.
        ifindex: InterfaceIndex,
        /// The attach error.
        #[source]
        source: aya::programs::ProgramError,
    },
    /// Any other errno.
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
