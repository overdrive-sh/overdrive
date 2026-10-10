//! `SimGuestFlowIo` — the owner seam's simulation adapter (DG-3, QPROJ-7).
//!
//! `#[cfg(test)]`, crate-private, beside the seam (QPROJ-1 rule 3: it names the
//! owner crate's own types). It holds every effect the owner issues (install,
//! connect, bind, listen, steer, load, attach, pin, route write, close, …)
//! until the driver releases or fails it, and reports when every owner task
//! waiting on it is parked with nothing undelivered ([`SimGuestFlowIo::settled`],
//! on a paused current-thread runtime).
//!
//! Node-global steering state — the link (`None` / `Attached` / `Pinned`), the
//! program version it runs, the entries, the tagged route — and the host facts
//! live in a [`SimGuestNode`] shared by every `SimGuestFlowIo` built over it,
//! so they survive an owner's drop; an unpinned link and every handle die
//! with their seam ([`SimGuestFlowIo::exit_process`], or the seam's drop).
//! Closing an `IntakeListener` removes its steering entry in the same step
//! (K-L3). `converge_steering` is held as separate effects — `Load`, then
//! `Attach` (an adopted link: the update) and, for a first attach only,
//! `Pin` — so a driver can fail or crash between them.
//!
//! The read surfaces are the QPROJ-7 contract: [`SimGuestFlowIo::socket_observation`],
//! [`SimGuestFlowIo::steering_observation`], [`SimGuestFlowIo::steering_effects`].
//! Every other method (holding, releasing, failing, scripting, delivering
//! guest-side events) is DISTILL's test support.
//!
//! The sim enforces the real adapter's preconditions: a handle of another seam
//! instance, a forwarding / steering call before its boot step, and an
//! `accept` on an unbound listener are `unreachable!`; consuming calls are
//! all-or-nothing; a handle left open when the seam drops without a process
//! exit is reported (panic) as an owner defect.

#![allow(
    clippy::redundant_pub_crate,
    reason = "QPROJ-7 pins `pub(crate)` visibility for the sim's read surface"
)]
#![allow(
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    reason = "the sim applies each effect under one lock acquisition by design (atomic kernel steps)"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aya::programs::ProgramError;
use aya::sys::SyscallError;
use ipnet::Ipv4Net;
use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, ControlOpen, FLOW_REQUEST_LEN, FlowId,
    FlowRequest,
};
use overdrive_core::guest_transport::{GuestCid, LaunchToken};
use overdrive_core::net::InterfaceIndex;
use overdrive_dataplane::guest_flow::{
    GUEST_FLOW_MAX_PAIRS, GuestFlowCounters, GuestFlowError, GuestFlowInventory, GuestFlowLinks,
    UnframeAttachError, UnframeConvergence,
};
use overdrive_dataplane::guest_steering::{GUEST_PREFIX_STEERING_PIN_DIR, GuestSteeringError};
use overdrive_netlink::NetlinkError;
use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

use super::{
    ArmedPeer, DatagramPair, FlowDirection, FlowHangup, FlowIoError, FlowListener, GuestFlowIo,
    GuestHostFacts, HostRouteType, InstalledVsock, IntakeChild, IntakeListener, OwnedSocket,
    PairKey, PairSocket, ParkedChild, PeerProtocol, PeerSocket, PeerSocketSpec, PendingInbound,
    RecordSocket, SeamHandle, TcpPair,
};
use crate::guest_network::{GuestConnKey, GuestNetworkError, GuestNetworkOperation};

// ===========================================================================
// QPROJ-7 read surfaces (the contract)
// ===========================================================================

/// The steering program a sim binary loads (`bin` in `prefix_boot`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SimProgramVersion(String);

impl SimProgramVersion {
    /// A program labelled `label` (e.g. `"v1"`).
    pub(crate) fn new(label: &str) -> Self {
        Self(label.to_owned())
    }

    /// The label.
    pub(crate) fn label(&self) -> &str {
        &self.0
    }
}

/// The node's guest-prefix steering link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimSteeringLink {
    /// No link.
    None,
    /// A first link attached and not yet pinned (process-scoped).
    Attached,
    /// The link pinned under `GUEST_PREFIX_STEERING_PIN_DIR` (node-global).
    Pinned,
}

/// The guest prefix's Overdrive-tagged route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPrefixRoute {
    /// No tagged route.
    Absent,
    /// `local <prefix> dev lo src <gateway> proto GUEST_PREFIX_ROUTE_PROTOCOL`.
    Local,
}

/// The intake listener a steering entry names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SimListenerRef {
    /// The listener's bound address (`workload_addr:<intake host port>`).
    pub(crate) bound: SocketAddrV4,
    seam: u64,
    handle: u64,
}

/// The outcome of one `steer` the owner issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimSteerOutcome {
    /// The entry is present.
    Ok,
    /// `GuestSteeringError::KernelMemory`.
    KernelMemory,
    /// Any other failure (an Overdrive defect). No capacity outcome exists
    /// (D8a-CAP).
    Defect,
}

/// One steering or route effect the owner issued, with its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SimSteeringEffect {
    /// `GuestPrefixSteering::load` of this seam's program.
    Load {
        /// The program loaded.
        version: SimProgramVersion,
        /// The load returned `Ok`.
        ok: bool,
    },
    /// `GuestSteeringCandidate::attach`: the update of an adopted link, or a
    /// first attach.
    Attach {
        /// A pinned link was adopted (the update).
        adopted: bool,
        /// The attach returned `Ok`.
        ok: bool,
    },
    /// The pin of a first attach.
    Pin {
        /// The pin succeeded.
        ok: bool,
    },
    /// The prefix route write.
    Route {
        /// The write succeeded.
        ok: bool,
    },
    /// One `steer`.
    Steer {
        /// The intake `(workload_addr, declared port)`.
        intake: SocketAddrV4,
        /// Its outcome.
        outcome: SimSteerOutcome,
    },
}

/// The node-global steering state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimSteeringObservation {
    /// None | Attached | Pinned.
    pub(crate) link: SimSteeringLink,
    /// The program the link runs.
    pub(crate) program: Option<SimProgramVersion>,
    /// `(workload_addr, declared port)` → listener.
    pub(crate) entries: BTreeMap<SocketAddrV4, SimListenerRef>,
    /// Absent | Local.
    pub(crate) route: SimPrefixRoute,
}

/// The first terminal event an outer far end observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FarEndEvent {
    /// A clean FIN.
    Eof,
    /// A reset.
    Reset,
}

/// The socket facts of one seam (QPROJ-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimOwnerSocketObservation {
    /// Accept-queued connections and their buffered first record's token
    /// (`None`: nothing buffered).
    pub(crate) queued: BTreeMap<GuestConnKey, Option<LaunchToken>>,
    /// Connections the owner closed.
    pub(crate) closed_by_owner: BTreeSet<GuestConnKey>,
    /// Accepted; peer VM dead, not yet observed.
    pub(crate) peer_gone: BTreeSet<GuestConnKey>,
    /// Peer connect issued, completion not consumed.
    pub(crate) connects_in_flight: BTreeSet<GuestConnKey>,
    /// `V_h` the owner closed.
    pub(crate) host_flows_closed: BTreeSet<FlowId>,
    /// Written by the owner, not consumed by the peer.
    pub(crate) outbox: BTreeMap<GuestCid, Vec<ControlMessage>>,
    /// Control sessions that ended.
    pub(crate) session_ended: BTreeSet<GuestCid>,
    /// Flow-vsock ends the owner issued, not yet delivered to the peer.
    pub(crate) vsock_ends_in_flight: BTreeSet<FlowId>,
    /// Flow-vsock ends delivered to the peer.
    pub(crate) vsock_ended: BTreeSet<FlowId>,
    /// Each outer far end's first terminal event.
    pub(crate) far_end: BTreeMap<FlowId, FarEndEvent>,
}

// ===========================================================================
// Test-support vocabulary (DISTILL's)
// ===========================================================================

/// Identifies one held effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SimEffectId(u64);

/// The kind of an effect the owner issues through the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SimEffectKind {
    HostFacts,
    ProbeForwarder,
    LoadForwarder,
    BindListener,
    StaleObjects,
    SteeringLoad,
    SteeringAttach,
    SteeringPin,
    PrefixRoute,
    WriteRecord,
    ConnectGuest,
    OpenPeer,
    ConnectUdp,
    ConnectPeer,
    RouteType,
    BindIntake,
    Steer,
    InstallVsock,
    ArmPeer,
    RouteOutbound,
    InstallDatagram,
    RegisterIntake,
    UnregisterIntake,
    AdoptIntake,
    BeginInbound,
    WriteRequest,
    RouteInbound,
    Drained,
    RemovePair,
    Inventory,
    Counters,
    ShutdownWrite,
    ClearLinger,
    Abort,
    Close,
}

impl SimEffectKind {
    /// Every kind.
    pub(crate) const ALL: [Self; 35] = [
        Self::HostFacts,
        Self::ProbeForwarder,
        Self::LoadForwarder,
        Self::BindListener,
        Self::StaleObjects,
        Self::SteeringLoad,
        Self::SteeringAttach,
        Self::SteeringPin,
        Self::PrefixRoute,
        Self::WriteRecord,
        Self::ConnectGuest,
        Self::OpenPeer,
        Self::ConnectUdp,
        Self::ConnectPeer,
        Self::RouteType,
        Self::BindIntake,
        Self::Steer,
        Self::InstallVsock,
        Self::ArmPeer,
        Self::RouteOutbound,
        Self::InstallDatagram,
        Self::RegisterIntake,
        Self::UnregisterIntake,
        Self::AdoptIntake,
        Self::BeginInbound,
        Self::WriteRequest,
        Self::RouteInbound,
        Self::Drained,
        Self::RemovePair,
        Self::Inventory,
        Self::Counters,
        Self::ShutdownWrite,
        Self::ClearLinger,
        Self::Abort,
        Self::Close,
    ];

    /// Pure reads, released without a hold by default.
    const READS: [Self; 6] = [
        Self::HostFacts,
        Self::StaleObjects,
        Self::RouteType,
        Self::Drained,
        Self::Inventory,
        Self::Counters,
    ];
}

/// What one held effect targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimEffectTarget {
    /// Nothing beyond the kind.
    None,
    /// A flow listener.
    Listener(FlowListener),
    /// A guest CID.
    Cid(GuestCid),
    /// A socket address (bind, connect, steer intake).
    Addr(SocketAddrV4),
    /// An IPv4 destination.
    Ip(Ipv4Addr),
    /// A pair.
    Pair(PairKey),
    /// The steering attach: an adopted pinned link (`true`) or a first attach.
    Adopted(bool),
    /// A peer socket spec.
    Peer(PeerSocketSpec),
    /// The control session / flow connection a record write goes to.
    Conn(Option<GuestConnKey>),
}

/// One effect the owner issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimEffect {
    /// The effect's kind.
    pub(crate) kind: SimEffectKind,
    /// What it targets.
    pub(crate) target: SimEffectTarget,
}

/// A scripted steering failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimSteeringFailure {
    /// `load`: the kernel verifier rejects the program.
    Load,
    /// `load`: opening the pinned link failed.
    Query,
    /// `attach` of an adopted link: `BPF_LINK_UPDATE` failed.
    Update,
    /// A first `attach` failed.
    Attach,
    /// Pinning a first attach failed.
    Pin,
    /// `steer`: `ENOMEM`.
    KernelMemory,
    /// `steer`: another map update failure.
    MapUpdate,
    /// `steer`: reading the present entry failed.
    MapLookup,
    /// `steer`: the entry names another open listener.
    IntakeAlreadySteered,
}

/// A scripted forwarder failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimForwarderFailure {
    /// The owner's cgroup is the root cgroup.
    OwnerCgroupIsRoot,
    /// A socket query failed with this errno kind.
    SocketQuery(io::ErrorKind),
    /// Every pair slot is in use.
    PairCapacity,
}

/// A scripted egress unframe attach failure at boot (D5a-IFACE-BOOT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimUnframeFailure {
    /// `ENOMEM`.
    KernelMemory,
    /// Any other errno (`ERANGE`, `EEXIST`, `ENODEV`, …).
    Attach(i32),
}

/// A failure the driver gives a held (or scripts for an auto-released)
/// effect. It must match the effect's error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimFailure {
    /// A socket syscall (`FlowIoError::Io`) or, for `host_facts` /
    /// `stale_objects_present`, `GuestNetworkError::Io`.
    Io(io::ErrorKind),
    /// A steering failure (`converge_steering`, `steer`).
    Steering(SimSteeringFailure),
    /// A forwarder failure.
    Forwarder(SimForwarderFailure),
    /// The prefix route write (`GuestNetworkError::Netlink { LocalRouteConverge }`).
    Netlink,
}

/// A guest-opened connection's first record, buffered in the accept queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimFirstRecord(pub(crate) Vec<u8>);

/// How a peer ends a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPeerEnd {
    /// A clean end of stream.
    Eof,
    /// A reset.
    Reset,
}

/// Which leg of a pair a terminal event or a vsock end concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPairLeg {
    /// The TCP flow vsock.
    Vsock,
    /// The TCP outer leg.
    Outer,
    /// The datagram stream leg.
    Stream,
    /// The datagram seqpacket leg.
    Seqpacket,
    /// The datagram UDP socket.
    Udp,
}

/// Where a host-local client connect to the guest prefix lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimLanding {
    /// The steered intake listener's accept queue.
    Steered(SimListenerRef),
    /// Refused: no entry (every other lookup to the prefix is dropped).
    Refused,
}

// ===========================================================================
// Node-global state
// ===========================================================================

#[derive(Debug, Clone)]
struct NodeLink {
    pinned: bool,
    program: SimProgramVersion,
    attached_by: u64,
}

#[derive(Debug)]
struct NodeState {
    link: Option<NodeLink>,
    route: SimPrefixRoute,
    entries: BTreeMap<SocketAddrV4, SimListenerRef>,
    host_facts: GuestHostFacts,
    host_facts_failure: Option<io::ErrorKind>,
    stale_objects: bool,
}

/// Node-global objects shared by every [`SimGuestFlowIo`] over this node:
/// the pinned steering link, its program and entries, the tagged route, the
/// host facts. They survive an owner's drop (U-4, K-D3).
#[derive(Debug)]
pub(crate) struct SimGuestNode {
    state: Mutex<NodeState>,
    next_seam: AtomicU64,
}

impl SimGuestNode {
    /// A node never booted: no link, no route, default host facts
    /// (independent interfaces none, ephemeral range `32768..=60999`,
    /// `RLIMIT_NOFILE` 1,048,576, the owner in
    /// `/sys/fs/cgroup/overdrive.slice/control-plane.slice`).
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(NodeState {
                link: None,
                route: SimPrefixRoute::Absent,
                entries: BTreeMap::new(),
                host_facts: GuestHostFacts {
                    interfaces: BTreeMap::new(),
                    ephemeral_ports: 32_768..=60_999,
                    nofile_limit: 1_048_576,
                    owner_cgroup: PathBuf::from(
                        "/sys/fs/cgroup/overdrive.slice/control-plane.slice",
                    ),
                },
                host_facts_failure: None,
                stale_objects: false,
            }),
            next_seam: AtomicU64::new(1),
        })
    }

    /// Replaces the host facts `host_facts` reports.
    pub(crate) fn set_host_facts(&self, facts: GuestHostFacts) {
        self.state.lock().host_facts = facts;
    }

    /// Mutates the host facts `host_facts` reports.
    pub(crate) fn update_host_facts(&self, update: impl FnOnce(&mut GuestHostFacts)) {
        update(&mut self.state.lock().host_facts);
    }

    /// Makes the next `host_facts` read fail with `kind`.
    pub(crate) fn fail_host_facts(&self, kind: io::ErrorKind) {
        self.state.lock().host_facts_failure = Some(kind);
    }

    /// Whether a process-scoped owner object of a previous run is present.
    pub(crate) fn set_stale_objects(&self, present: bool) {
        self.state.lock().stale_objects = present;
    }

    /// The steering state, read across every seam over this node.
    fn steering(&self) -> SimSteeringObservation {
        let node = self.state.lock();
        SimSteeringObservation {
            link: match &node.link {
                None => SimSteeringLink::None,
                Some(link) if link.pinned => SimSteeringLink::Pinned,
                Some(_) => SimSteeringLink::Attached,
            },
            program: node.link.as_ref().map(|link| link.program.clone()),
            entries: node.entries.clone(),
            route: node.route,
        }
    }
}

// ===========================================================================
// Per-seam state
// ===========================================================================

struct Held {
    effect: SimEffect,
    decide: oneshot::Sender<Option<SimFailure>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteeringPhase {
    None,
    Converged { prefix: Ipv4Net, gateway: Ipv4Addr },
}

#[derive(Debug, Default)]
struct VsockListener {
    bound: bool,
    failed: Option<io::ErrorKind>,
    queue: VecDeque<QueuedConn>,
}

#[derive(Debug)]
struct QueuedConn {
    key: GuestConnKey,
    record: Vec<u8>,
    released: bool,
    /// The peer VM died while queued: buffered bytes, then this end.
    end: Option<SimPeerEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordOrigin {
    Accepted(FlowListener),
    HostOpened,
}

#[derive(Debug)]
struct RecordState {
    origin: RecordOrigin,
    conn: Option<GuestConnKey>,
    peer_cid: Option<GuestCid>,
    first_record: Vec<u8>,
    buffered: VecDeque<u8>,
    ready: VecDeque<u8>,
    end: Option<SimPeerEnd>,
    end_released: bool,
    peer_gone: bool,
}

#[derive(Debug, Clone, Copy)]
struct PeerState {
    spec: PeerSocketSpec,
    local: Option<SocketAddrV4>,
    peer: Option<SocketAddrV4>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairRole {
    Vsock,
    Outer { abortive: bool },
    Stream,
    Seqpacket,
    Udp,
}

#[derive(Debug)]
struct PairSocketState {
    pair: PairKey,
    role: PairRole,
    conn: Option<GuestConnKey>,
    label: Option<[u8; FLOW_REQUEST_LEN]>,
    hangup: Option<FlowHangup>,
    linger_cleared: bool,
    shut: bool,
}

#[derive(Debug)]
struct IntakeClient {
    client: SocketAddrV4,
    released: bool,
}

#[derive(Debug)]
struct IntakeListenerState {
    bound: SocketAddrV4,
    registered: bool,
    queue: VecDeque<IntakeClient>,
}

#[derive(Debug)]
enum HandleState {
    Record(RecordState),
    Peer(PeerState),
    Armed {
        peer: PeerState,
        conn: Option<GuestConnKey>,
        label: Option<[u8; FLOW_REQUEST_LEN]>,
    },
    Installed {
        conn: Option<GuestConnKey>,
        label: Option<[u8; FLOW_REQUEST_LEN]>,
    },
    Listener(IntakeListenerState),
    Child {
        client: SocketAddrV4,
        hangup: Option<FlowHangup>,
    },
    Parked {
        client: SocketAddrV4,
        hangup: Option<FlowHangup>,
    },
    Pending {
        client: SocketAddrV4,
        hangup: Option<FlowHangup>,
        cid: Option<GuestCid>,
        request: Option<[u8; FLOW_REQUEST_LEN]>,
    },
    Pair(PairSocketState),
}

#[derive(Debug, Default)]
struct PairEntry {
    toward_guest_drained: bool,
    from_guest_drained: bool,
}

#[derive(Debug)]
struct ForwarderState {
    attached: BTreeSet<InterfaceIndex>,
    failed: BTreeMap<InterfaceIndex, SimUnframeFailure>,
}

struct SimState {
    exited: bool,
    auto: BTreeSet<SimEffectKind>,
    scripts: BTreeMap<SimEffectKind, VecDeque<SimFailure>>,
    held: BTreeMap<SimEffectId, Held>,
    issued: Vec<SimEffectKind>,
    next_effect: u64,
    next_handle: u64,
    next_pair: u64,
    next_port: u16,
    forwarder: Option<ForwarderState>,
    unframe_failures: BTreeMap<InterfaceIndex, SimUnframeFailure>,
    steering: SteeringPhase,
    steering_effects: Vec<SimSteeringEffect>,
    listeners: BTreeMap<FlowListener, VsockListener>,
    handles: BTreeMap<u64, HandleState>,
    pairs: BTreeMap<PairKey, PairEntry>,
    route_types: BTreeMap<Ipv4Addr, HostRouteType>,
    connect4: BTreeMap<SocketAddrV4, SocketAddrV4>,
    counters: GuestFlowCounters,
    closed_by_owner: BTreeSet<GuestConnKey>,
    connects_in_flight: BTreeSet<GuestConnKey>,
    host_flows_closed: Vec<[u8; FLOW_REQUEST_LEN]>,
    outbox: BTreeMap<GuestCid, Vec<[u8; CONTROL_MESSAGE_LEN]>>,
    session_ended: BTreeSet<GuestCid>,
    vsock_ends_in_flight: BTreeMap<PairKey, [u8; FLOW_REQUEST_LEN]>,
    vsock_ended: Vec<[u8; FLOW_REQUEST_LEN]>,
    far_end: Vec<([u8; FLOW_REQUEST_LEN], FarEndEvent)>,
}

/// The owner seam's simulation adapter (DG-3).
pub(crate) struct SimGuestFlowIo {
    seam: u64,
    node: Arc<SimGuestNode>,
    binary: SimProgramVersion,
    state: Mutex<SimState>,
    changed: Notify,
}

impl std::fmt::Debug for SimGuestFlowIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimGuestFlowIo")
            .field("seam", &self.seam)
            .field("binary", &self.binary)
            .finish_non_exhaustive()
    }
}

fn io_error(kind: io::ErrorKind) -> FlowIoError {
    FlowIoError::Io { source: io::Error::from(kind) }
}

fn syscall(call: &'static str, errno: i32) -> SyscallError {
    SyscallError { call, io_error: io::Error::from_raw_os_error(errno) }
}

fn steering_error(failure: SimSteeringFailure, intake: SocketAddrV4) -> GuestSteeringError {
    match failure {
        SimSteeringFailure::Load => GuestSteeringError::Load {
            source: aya::EbpfError::ProgramError(ProgramError::SyscallError(syscall(
                "bpf_prog_load",
                libc::EACCES,
            ))),
        },
        SimSteeringFailure::Query => {
            GuestSteeringError::Query { source: io::Error::from_raw_os_error(libc::EIO) }
        }
        SimSteeringFailure::Update => {
            GuestSteeringError::Update { source: io::Error::from_raw_os_error(libc::EINVAL) }
        }
        SimSteeringFailure::Attach => GuestSteeringError::Attach {
            source: ProgramError::SyscallError(syscall("bpf_link_create", libc::EINVAL)),
        },
        SimSteeringFailure::Pin => GuestSteeringError::Pin {
            path: PathBuf::from(GUEST_PREFIX_STEERING_PIN_DIR),
            source: io::Error::from_raw_os_error(libc::ENOSPC),
        },
        SimSteeringFailure::KernelMemory => GuestSteeringError::KernelMemory {
            source: aya::maps::MapError::IoError(io::Error::from_raw_os_error(libc::ENOMEM)),
        },
        SimSteeringFailure::MapUpdate => GuestSteeringError::MapUpdate {
            source: aya::maps::MapError::IoError(io::Error::from_raw_os_error(libc::EINVAL)),
        },
        SimSteeringFailure::MapLookup => GuestSteeringError::MapLookup {
            source: aya::maps::MapError::IoError(io::Error::from_raw_os_error(libc::EIO)),
        },
        SimSteeringFailure::IntakeAlreadySteered => {
            GuestSteeringError::IntakeAlreadySteered { intake }
        }
    }
}

fn forwarder_error(failure: SimForwarderFailure) -> GuestFlowError {
    match failure {
        SimForwarderFailure::OwnerCgroupIsRoot => GuestFlowError::OwnerCgroupIsRoot,
        SimForwarderFailure::SocketQuery(kind) => {
            GuestFlowError::SocketQuery { source: io::Error::from(kind) }
        }
        SimForwarderFailure::PairCapacity => {
            GuestFlowError::PairCapacity { capacity: GUEST_FLOW_MAX_PAIRS }
        }
    }
}

fn unframe_error(ifindex: InterfaceIndex, failure: SimUnframeFailure) -> UnframeAttachError {
    match failure {
        SimUnframeFailure::KernelMemory => UnframeAttachError::KernelMemory {
            ifindex,
            source: ProgramError::SyscallError(syscall("bpf_link_create", libc::ENOMEM)),
        },
        SimUnframeFailure::Attach(errno) => UnframeAttachError::Attach {
            ifindex,
            source: ProgramError::SyscallError(syscall("bpf_link_create", errno)),
        },
    }
}

fn mismatched(effect: &str, failure: SimFailure) -> ! {
    panic!("test bug: scripted failure {failure:?} does not match the {effect} effect")
}

fn decode_flow(label: &[u8; FLOW_REQUEST_LEN]) -> FlowId {
    FlowRequest::decode(label)
        .unwrap_or_else(|error| panic!("sim flow label is not a canonical request: {error}"))
        .flow()
}

impl SimGuestFlowIo {
    /// A seam over `node` whose binary carries steering program `binary`.
    pub(crate) fn new(node: &Arc<SimGuestNode>, binary: SimProgramVersion) -> Arc<Self> {
        let seam = node.next_seam.fetch_add(1, Ordering::SeqCst);
        let auto = SimEffectKind::READS.into_iter().collect();
        Arc::new(Self {
            seam,
            node: Arc::clone(node),
            binary,
            state: Mutex::new(SimState {
                exited: false,
                auto,
                scripts: BTreeMap::new(),
                held: BTreeMap::new(),
                issued: Vec::new(),
                next_effect: 1,
                next_handle: 1,
                next_pair: 1,
                next_port: 40_000,
                forwarder: None,
                unframe_failures: BTreeMap::new(),
                steering: SteeringPhase::None,
                steering_effects: Vec::new(),
                listeners: BTreeMap::new(),
                handles: BTreeMap::new(),
                pairs: BTreeMap::new(),
                route_types: BTreeMap::new(),
                connect4: BTreeMap::new(),
                counters: GuestFlowCounters::default(),
                closed_by_owner: BTreeSet::new(),
                connects_in_flight: BTreeSet::new(),
                host_flows_closed: Vec::new(),
                outbox: BTreeMap::new(),
                session_ended: BTreeSet::new(),
                vsock_ends_in_flight: BTreeMap::new(),
                vsock_ended: Vec::new(),
                far_end: Vec::new(),
            }),
            changed: Notify::new(),
        })
    }

    // -----------------------------------------------------------------------
    // QPROJ-7 read surfaces
    // -----------------------------------------------------------------------

    /// The socket facts of this seam (QPROJ-7).
    ///
    /// A queued connection's token is `None` when nothing is buffered yet, and
    /// equally when its buffered first record is not canonical (a malformed
    /// `ControlOpen` / `FlowRequest`): a malformed record carries no token, so
    /// it projects like an empty buffer.
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
                    FlowListener::Stream | FlowListener::Seqpacket => conn
                        .record
                        .get(..FLOW_REQUEST_LEN)
                        .and_then(|bytes| <&[u8; FLOW_REQUEST_LEN]>::try_from(bytes).ok())
                        .and_then(|bytes| FlowRequest::decode(bytes).ok())
                        .map(|request| request.launch()),
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
        let outbox = st
            .outbox
            .iter()
            .map(|(cid, messages)| {
                let decoded = messages
                    .iter()
                    .map(|bytes| {
                        ControlMessage::decode(bytes)
                            .unwrap_or_else(|e| panic!("sim outbox message: {e}"))
                    })
                    .collect();
                (*cid, decoded)
            })
            .collect();
        SimOwnerSocketObservation {
            queued,
            closed_by_owner: st.closed_by_owner.clone(),
            peer_gone,
            connects_in_flight: st.connects_in_flight.clone(),
            host_flows_closed: st.host_flows_closed.iter().map(decode_flow).collect(),
            outbox,
            session_ended: st.session_ended.clone(),
            vsock_ends_in_flight: st.vsock_ends_in_flight.values().map(decode_flow).collect(),
            vsock_ended: st.vsock_ended.iter().map(decode_flow).collect(),
            far_end: {
                let mut far = BTreeMap::new();
                for (label, event) in &st.far_end {
                    far.entry(decode_flow(label)).or_insert(*event);
                }
                far
            },
        }
    }

    /// The node-global steering state (QPROJ-7).
    pub(crate) fn steering_observation(&self) -> SimSteeringObservation {
        self.node.steering()
    }

    /// Every steering and route effect this seam's owner issued, in order,
    /// with its outcome (QPROJ-7).
    pub(crate) fn steering_effects(&self) -> Vec<SimSteeringEffect> {
        self.state.lock().steering_effects.clone()
    }

    // -----------------------------------------------------------------------
    // Holding, releasing, failing, scripting
    // -----------------------------------------------------------------------

    /// Releases every later effect of `kind` without a hold.
    pub(crate) fn auto_release(&self, kind: SimEffectKind) {
        self.state.lock().auto.insert(kind);
    }

    /// Releases every later effect without a hold.
    pub(crate) fn auto_release_all(&self) {
        self.state.lock().auto.extend(SimEffectKind::ALL);
    }

    /// Holds every later effect of `kind` until released or failed.
    pub(crate) fn hold(&self, kind: SimEffectKind) {
        self.state.lock().auto.remove(&kind);
    }

    /// The next auto-released effect of `kind` fails with `failure`.
    pub(crate) fn script_failure(&self, kind: SimEffectKind, failure: SimFailure) {
        self.state.lock().scripts.entry(kind).or_default().push_back(failure);
    }

    /// Every effect kind the owner issued through this seam, in order (held
    /// or not, applied or not).
    pub(crate) fn issued(&self) -> Vec<SimEffectKind> {
        self.state.lock().issued.clone()
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

    /// Resolves when every owner task waiting on this seam is parked with
    /// nothing undelivered: no task of the runtime is ready to run.
    ///
    /// Idle detection is the runtime's own: the caller drives the owner on a
    /// current-thread tokio runtime whose time is paused
    /// (`#[tokio::test(flavor = "current_thread", start_paused = true)]`, or
    /// `Builder::new_current_thread().enable_time().start_paused(true)`). A
    /// paused runtime auto-advances its clock to the next timer only when no
    /// task is ready, so the sleep below completes exactly when every task —
    /// however many internal hand-offs it makes between seam calls — is
    /// parked. Every effect the sim holds wakes its waiter inside that
    /// runtime, so a parked owner is one waiting on the driver.
    ///
    /// The owner reads time only through its injected `Clock` port
    /// (`SimClock` here), never through tokio timers, so pausing tokio time
    /// does not change its behaviour. An owner that awaited a tokio timer
    /// directly would be a defect (`.claude/rules/development.md` §
    /// "Production code is not shaped by simulation"; the clock port): its
    /// timer would fire on the auto-advance here instead of on the injected
    /// clock.
    ///
    /// Precondition: the runtime's time is paused. On a running clock the
    /// sleep would elapse in real time.
    pub(crate) async fn settled(&self) {
        /// Far beyond any deadline the tests arm: the sleep is the idle
        /// signal, never a timeout that elapses while a task is ready.
        const IDLE_SIGNAL: std::time::Duration = std::time::Duration::from_secs(365 * 24 * 3600);
        tokio::time::sleep(IDLE_SIGNAL).await;
    }

    /// The `serve` process ended: every handle and every held effect dies
    /// with it, an unpinned (attached, not pinned) link detaches, and every
    /// steering entry naming one of its listeners is removed (K-L3, K-L4).
    /// Node-global objects stay (U-4).
    pub(crate) fn exit_process(&self) {
        let mut st = self.state.lock();
        if st.exited {
            return;
        }
        st.exited = true;
        st.held.clear();
        let handles = std::mem::take(&mut st.handles);
        for handle in handles.values() {
            if let HandleState::Record(record) = handle
                && record.origin == RecordOrigin::Accepted(FlowListener::Control)
                && let Some(cid) = record.peer_cid
            {
                st.session_ended.insert(cid);
            }
            if let HandleState::Pair(pair) = handle {
                Self::pair_ended_by_exit(&mut st, pair);
            }
        }
        drop(st);
        let mut node = self.node.state.lock();
        if node.link.as_ref().is_some_and(|link| !link.pinned && link.attached_by == self.seam) {
            node.link = None;
        }
        node.entries.retain(|_, listener| listener.seam != self.seam);
        drop(node);
        self.bump();
    }

    fn pair_ended_by_exit(st: &mut SimState, pair: &PairSocketState) {
        let Some(label) = pair.label else { return };
        match pair.role {
            PairRole::Vsock => {
                st.vsock_ends_in_flight.insert(pair.pair, label);
            }
            PairRole::Outer { abortive } => {
                // H-5: linger covers the far ends the dying owner holds.
                let event = if abortive && !pair.linger_cleared {
                    FarEndEvent::Reset
                } else {
                    FarEndEvent::Eof
                };
                st.far_end.push((label, event));
            }
            PairRole::Stream | PairRole::Seqpacket | PairRole::Udp => {}
        }
    }

    // -----------------------------------------------------------------------
    // Scripting host facts and kernel answers
    // -----------------------------------------------------------------------

    /// The next `load_forwarder` reports `ifindex` in `failed` with `failure`.
    pub(crate) fn script_unframe_failure(
        &self,
        ifindex: InterfaceIndex,
        failure: SimUnframeFailure,
    ) {
        self.state.lock().unframe_failures.insert(ifindex, failure);
    }

    /// `route_type(destination)` answers `route`.
    pub(crate) fn set_route_type(&self, destination: Ipv4Addr, route: HostRouteType) {
        self.state.lock().route_types.insert(destination, route);
    }

    /// A UDP `connect()` to `vip` is rewritten by `connect4` to `backend`.
    pub(crate) fn set_connect4_rewrite(&self, vip: SocketAddrV4, backend: SocketAddrV4) {
        self.state.lock().connect4.insert(vip, backend);
    }

    /// The forwarder's counters.
    pub(crate) fn set_counters(&self, counters: GuestFlowCounters) {
        self.state.lock().counters = counters;
    }

    // -----------------------------------------------------------------------
    // Guest-side and far-end events
    // -----------------------------------------------------------------------

    /// A guest opens a connection to `listener`; it waits in the accept queue
    /// with `record` buffered until [`Self::release_accept`].
    pub(crate) fn queue_connection(
        &self,
        listener: FlowListener,
        key: GuestConnKey,
        record: Option<SimFirstRecord>,
    ) {
        self.state.lock().listeners.entry(listener).or_default().queue.push_back(QueuedConn {
            key,
            record: record.map(|record| record.0).unwrap_or_default(),
            released: false,
            end: None,
        });
        self.bump();
    }

    /// Releases a queued connection to the owner's `accept`.
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

    /// The guest ends an accepted connection (after its buffered bytes).
    pub(crate) fn end_connection(&self, key: GuestConnKey, end: SimPeerEnd) {
        let mut st = self.state.lock();
        let record = Self::record_by_conn(&mut st, key);
        record.end = Some(end);
        if record.buffered.is_empty() {
            record.end_released = true;
        }
        drop(st);
        self.bump();
    }

    /// Bytes the guest writes on an accepted connection (a later control
    /// message), buffered until [`Self::release_read`].
    pub(crate) fn send_on_connection(&self, key: GuestConnKey, bytes: &[u8]) {
        let mut st = self.state.lock();
        Self::record_by_conn(&mut st, key).buffered.extend(bytes.iter().copied());
        drop(st);
        self.bump();
    }

    fn record_by_conn(st: &mut SimState, key: GuestConnKey) -> &mut RecordState {
        st.handles
            .values_mut()
            .find_map(|handle| match handle {
                HandleState::Record(record) if record.conn == Some(key) => Some(record),
                _ => None,
            })
            .unwrap_or_else(|| panic!("test bug: {key:?} is not an accepted connection"))
    }

    /// The VM of `cid` dies (K-A2′): accepted connections fail at their next
    /// I/O; queued ones keep their buffered record, then end; pair sockets of
    /// its flows hang up with a reset.
    pub(crate) fn vm_stop(&self, cid: GuestCid) {
        let mut st = self.state.lock();
        for listener in st.listeners.values_mut() {
            for conn in &mut listener.queue {
                if conn.key.peer_cid == cid {
                    conn.end = Some(SimPeerEnd::Reset);
                }
            }
        }
        for handle in st.handles.values_mut() {
            match handle {
                HandleState::Record(record) if record.peer_cid == Some(cid) => {
                    if record.origin == RecordOrigin::HostOpened {
                        record.end = Some(SimPeerEnd::Reset);
                        record.end_released = true;
                    } else {
                        record.peer_gone = true;
                    }
                }
                HandleState::Pair(pair)
                    if pair.conn.is_some_and(|conn| conn.peer_cid == cid)
                        && !matches!(pair.role, PairRole::Outer { .. } | PairRole::Udp) =>
                {
                    pair.hangup = Some(FlowHangup::Error(io::ErrorKind::ConnectionReset));
                }
                _ => {}
            }
        }
        drop(st);
        self.bump();
    }

    /// The listener's accept task fails with `kind` (the listener unbinds).
    pub(crate) fn fail_listener(&self, listener: FlowListener, kind: io::ErrorKind) {
        self.state.lock().listeners.entry(listener).or_default().failed = Some(kind);
        self.bump();
    }

    /// The guest consumes every control message the owner wrote to `cid`.
    pub(crate) fn consume_outbox(&self, cid: GuestCid) -> Vec<[u8; CONTROL_MESSAGE_LEN]> {
        let taken = self.state.lock().outbox.remove(&cid).unwrap_or_default();
        self.bump();
        taken
    }

    /// A host-local client at `client` connects to `intake` (a guest-prefix
    /// `(workload_addr, declared port)`): the steering decides (D8a-LOOKUP).
    pub(crate) fn connect_intake_client(
        &self,
        intake: SocketAddrV4,
        client: SocketAddrV4,
    ) -> SimLanding {
        let entry = self.node.state.lock().entries.get(&intake).copied();
        let Some(listener) = entry.filter(|listener| listener.seam == self.seam) else {
            return SimLanding::Refused;
        };
        let mut st = self.state.lock();
        if let Some(HandleState::Listener(state)) = st.handles.get_mut(&listener.handle) {
            state.queue.push_back(IntakeClient { client, released: false });
        }
        drop(st);
        self.bump();
        SimLanding::Steered(listener)
    }

    /// Releases a queued intake client to the owner's `accept_intake`.
    pub(crate) fn release_intake_accept(&self, client: SocketAddrV4) {
        let mut st = self.state.lock();
        let queued = st
            .handles
            .values_mut()
            .filter_map(|handle| match handle {
                HandleState::Listener(state) => Some(state),
                _ => None,
            })
            .flat_map(|state| state.queue.iter_mut())
            .find(|queued| queued.client == client)
            .unwrap_or_else(|| panic!("test bug: intake client {client} is not queued"));
        queued.released = true;
        drop(st);
        self.bump();
    }

    /// The intake client at `client` ends its connection before `Paired`.
    pub(crate) fn end_intake_client(&self, client: SocketAddrV4, hangup: FlowHangup) {
        let mut st = self.state.lock();
        for handle in st.handles.values_mut() {
            match handle {
                HandleState::Child { client: c, hangup: h }
                | HandleState::Parked { client: c, hangup: h }
                | HandleState::Pending { client: c, hangup: h, .. }
                    if *c == client =>
                {
                    *h = Some(hangup);
                }
                _ => {}
            }
        }
        drop(st);
        self.bump();
    }

    /// A terminal event on one leg of a routed pair.
    pub(crate) fn pair_hangup(&self, pair: PairKey, leg: SimPairLeg, hangup: FlowHangup) {
        let mut st = self.state.lock();
        for handle in st.handles.values_mut() {
            if let HandleState::Pair(socket) = handle
                && socket.pair == pair
                && Self::role_is_leg(socket.role, leg)
            {
                socket.hangup = Some(hangup);
            }
        }
        drop(st);
        self.bump();
    }

    const fn role_is_leg(role: PairRole, leg: SimPairLeg) -> bool {
        matches!(
            (role, leg),
            (PairRole::Vsock, SimPairLeg::Vsock)
                | (PairRole::Outer { .. }, SimPairLeg::Outer)
                | (PairRole::Stream, SimPairLeg::Stream)
                | (PairRole::Seqpacket, SimPairLeg::Seqpacket)
                | (PairRole::Udp, SimPairLeg::Udp)
        )
    }

    /// Sets the K2 drained answer of `pair` toward `direction`.
    pub(crate) fn set_drained(&self, pair: PairKey, direction: FlowDirection, drained: bool) {
        let mut st = self.state.lock();
        let entry = st.pairs.entry(pair).or_default();
        match direction {
            FlowDirection::TowardGuest => entry.toward_guest_drained = drained,
            FlowDirection::FromGuest => entry.from_guest_drained = drained,
        }
        drop(st);
        self.bump();
    }

    /// Delivers the flow-vsock end the owner issued on `pair` to the peer.
    pub(crate) fn deliver_vsock_end(&self, pair: PairKey) {
        let mut st = self.state.lock();
        let label = st
            .vsock_ends_in_flight
            .remove(&pair)
            .unwrap_or_else(|| panic!("test bug: no vsock end of {pair:?} in flight"));
        st.vsock_ended.push(label);
        drop(st);
        self.bump();
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn bump(&self) {
        self.changed.notify_waiters();
    }

    /// The effect gate: `None` applies the effect's natural outcome,
    /// `Some(failure)` fails it. A held effect waits for the driver; a call
    /// dropped while held never applies.
    async fn gate(&self, kind: SimEffectKind, target: SimEffectTarget) -> Option<SimFailure> {
        let (id, decided) = {
            let mut st = self.state.lock();
            assert!(!st.exited, "owner call after its seam's process exited");
            st.issued.push(kind);
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

    fn require_forwarder(st: &SimState) {
        if st.forwarder.is_none() {
            unreachable!("a forwarding call before load_forwarder returned Ok");
        }
    }

    fn require_steering(st: &SimState) -> (Ipv4Net, Ipv4Addr) {
        match st.steering {
            SteeringPhase::Converged { prefix, gateway } => (prefix, gateway),
            SteeringPhase::None => {
                unreachable!("a steering / route call before converge_steering returned Ok")
            }
        }
    }

    fn label_of_record(record: &RecordState) -> Option<[u8; FLOW_REQUEST_LEN]> {
        match record.origin {
            RecordOrigin::Accepted(FlowListener::Stream | FlowListener::Seqpacket) => record
                .first_record
                .get(..FLOW_REQUEST_LEN)
                .and_then(|bytes| <[u8; FLOW_REQUEST_LEN]>::try_from(bytes).ok()),
            RecordOrigin::Accepted(FlowListener::Control) | RecordOrigin::HostOpened => None,
        }
    }

    /// Applies the owner's end of one handle: `abortive` resets. A listener's
    /// steering entry is removed by the caller in the same step (K-L3).
    fn end_handle(st: &mut SimState, state: HandleState, abortive: bool) {
        match state {
            HandleState::Record(record) => {
                if let Some(conn) = record.conn {
                    st.closed_by_owner.insert(conn);
                }
                if record.origin == RecordOrigin::Accepted(FlowListener::Control)
                    && let Some(cid) = record.peer_cid
                {
                    st.session_ended.insert(cid);
                }
            }
            HandleState::Installed { conn, .. } | HandleState::Armed { conn, .. } => {
                if let Some(conn) = conn {
                    st.closed_by_owner.insert(conn);
                    st.connects_in_flight.remove(&conn);
                }
            }
            HandleState::Pending { request, .. } => {
                if let Some(request) = request {
                    st.host_flows_closed.push(request);
                }
            }
            HandleState::Pair(pair) => {
                if let Some(conn) = pair.conn
                    && pair.role != PairRole::Udp
                {
                    st.closed_by_owner.insert(conn);
                }
                if let Some(label) = pair.label {
                    match pair.role {
                        PairRole::Vsock if !pair.shut => {
                            st.vsock_ends_in_flight.insert(pair.pair, label);
                        }
                        PairRole::Outer { abortive: linger } => {
                            let event = if abortive || (linger && !pair.linger_cleared) {
                                FarEndEvent::Reset
                            } else {
                                FarEndEvent::Eof
                            };
                            st.far_end.push((label, event));
                        }
                        _ => {}
                    }
                }
            }
            HandleState::Peer(_)
            | HandleState::Listener(_)
            | HandleState::Child { .. }
            | HandleState::Parked { .. } => {}
        }
    }

    /// Ends `socket`'s handle (abort or close).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "ending a handle takes it from the owner, as the seam's `abort` / `close` do"
    )]
    fn end_owned(&self, socket: OwnedSocket, abortive: bool) {
        let handle = match socket {
            OwnedSocket::Record(RecordSocket(h))
            | OwnedSocket::Peer(PeerSocket(h))
            | OwnedSocket::Armed(ArmedPeer(h))
            | OwnedSocket::Installed(InstalledVsock(h))
            | OwnedSocket::Listener(IntakeListener(h))
            | OwnedSocket::Child(IntakeChild(h))
            | OwnedSocket::Parked(ParkedChild(h))
            | OwnedSocket::Pending(PendingInbound(h))
            | OwnedSocket::Pair(PairSocket(h)) => h,
        };
        let mut st = self.state.lock();
        let state = self.take(&mut st, handle);
        let listener_id = handle.id;
        let is_listener = matches!(state, HandleState::Listener(_));
        Self::end_handle(&mut st, state, abortive);
        drop(st);
        if is_listener {
            // K-L3: the entry naming this listener goes in the same step.
            self.node.state.lock().entries.retain(|_, listener| {
                !(listener.seam == self.seam && listener.handle == listener_id)
            });
        }
        self.bump();
    }

    /// Aborts every consumed handle state (a consuming call's `Err`).
    fn abort_states(st: &mut SimState, states: Vec<HandleState>) {
        for state in states {
            Self::end_handle(st, state, true);
        }
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
// The seam
// ===========================================================================

#[async_trait::async_trait]
impl GuestFlowIo for SimGuestFlowIo {
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError> {
        let failure = self.gate(SimEffectKind::HostFacts, SimEffectTarget::None).await;
        let mut node = self.node.state.lock();
        let failure = match failure {
            Some(SimFailure::Io(kind)) => Some(kind),
            Some(other) => mismatched("host_facts", other),
            None => node.host_facts_failure.take(),
        };
        if let Some(kind) = failure {
            return Err(GuestNetworkError::Io {
                operation: GuestNetworkOperation::StartupProbe,
                source: io::Error::from(kind),
            });
        }
        Ok(node.host_facts.clone())
    }

    async fn probe_forwarder(&self) -> Result<(), GuestFlowError> {
        match self.gate(SimEffectKind::ProbeForwarder, SimEffectTarget::None).await {
            None => Ok(()),
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("probe_forwarder", other),
        }
    }

    async fn load_forwarder(
        &self,
        interfaces: &BTreeSet<InterfaceIndex>,
    ) -> Result<UnframeConvergence, GuestFlowError> {
        let failure = self.gate(SimEffectKind::LoadForwarder, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        if let Some(forwarder) = &st.forwarder {
            return Ok(UnframeConvergence {
                attached: forwarder.attached.clone(),
                failed: forwarder
                    .failed
                    .iter()
                    .map(|(ifindex, failure)| (*ifindex, unframe_error(*ifindex, *failure)))
                    .collect(),
            });
        }
        match failure {
            None => {}
            Some(SimFailure::Forwarder(failure)) => return Err(forwarder_error(failure)),
            Some(other) => mismatched("load_forwarder", other),
        }
        let mut attached = BTreeSet::new();
        let mut failed = BTreeMap::new();
        for ifindex in interfaces {
            match st.unframe_failures.remove(ifindex) {
                Some(failure) => {
                    failed.insert(*ifindex, failure);
                }
                None => {
                    attached.insert(*ifindex);
                }
            }
        }
        let convergence = UnframeConvergence {
            attached: attached.clone(),
            failed: failed
                .iter()
                .map(|(ifindex, failure)| (*ifindex, unframe_error(*ifindex, *failure)))
                .collect(),
        };
        st.forwarder = Some(ForwarderState { attached, failed });
        Ok(convergence)
    }

    async fn bind_listener(&self, listener: FlowListener) -> Result<(), FlowIoError> {
        let failure =
            self.gate(SimEffectKind::BindListener, SimEffectTarget::Listener(listener)).await;
        let mut st = self.state.lock();
        let state = st.listeners.entry(listener).or_default();
        // A repair closes the earlier socket first.
        state.bound = false;
        state.failed = None;
        match failure {
            None => {
                state.bound = true;
                Ok(())
            }
            Some(SimFailure::Io(kind)) => Err(io_error(kind)),
            Some(other) => mismatched("bind_listener", other),
        }
    }

    async fn converge_steering(
        &self,
        prefix: Ipv4Net,
        gateway: Ipv4Addr,
    ) -> Result<(), GuestSteeringError> {
        if matches!(self.state.lock().steering, SteeringPhase::Converged { .. }) {
            unreachable!("converge_steering after it returned Ok in this seam's life");
        }
        let any = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        // Step 1 — Load: this binary's program, verifier-checked; nothing attached.
        let load = self.gate(SimEffectKind::SteeringLoad, SimEffectTarget::None).await;
        {
            let mut st = self.state.lock();
            let ok = load.is_none();
            st.steering_effects.push(SimSteeringEffect::Load { version: self.binary.clone(), ok });
            match load {
                None => {}
                Some(SimFailure::Steering(
                    failure @ (SimSteeringFailure::Load | SimSteeringFailure::Query),
                )) => return Err(steering_error(failure, any)),
                Some(other) => mismatched("steering load", other),
            }
        }
        // Step 2 — Swap: update an adopted pinned link, or a first attach.
        let adopted = self.node.state.lock().link.as_ref().is_some_and(|link| link.pinned);
        let attach =
            self.gate(SimEffectKind::SteeringAttach, SimEffectTarget::Adopted(adopted)).await;
        {
            let mut st = self.state.lock();
            st.steering_effects.push(SimSteeringEffect::Attach { adopted, ok: attach.is_none() });
            match attach {
                None => {
                    let mut node = self.node.state.lock();
                    if adopted {
                        // BPF_LINK_UPDATE: atomic; the new program's map is new and empty.
                        if let Some(link) = node.link.as_mut() {
                            link.program = self.binary.clone();
                        }
                    } else {
                        node.link = Some(NodeLink {
                            pinned: false,
                            program: self.binary.clone(),
                            attached_by: self.seam,
                        });
                    }
                    node.entries.clear();
                }
                Some(SimFailure::Steering(
                    failure @ (SimSteeringFailure::Update | SimSteeringFailure::Attach),
                )) => {
                    // The link keeps the earlier program (K-L9); nothing new attached.
                    return Err(steering_error(failure, any));
                }
                Some(other) => mismatched("steering attach", other),
            }
            if adopted {
                st.steering = SteeringPhase::Converged { prefix, gateway };
                return Ok(());
            }
        }
        // Step 2b — Pin, a first attach only.
        let pin = self.gate(SimEffectKind::SteeringPin, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        st.steering_effects.push(SimSteeringEffect::Pin { ok: pin.is_none() });
        let mut node = self.node.state.lock();
        match pin {
            None => {
                if let Some(link) = node.link.as_mut() {
                    link.pinned = true;
                }
                drop(node);
                st.steering = SteeringPhase::Converged { prefix, gateway };
                Ok(())
            }
            Some(SimFailure::Steering(SimSteeringFailure::Pin)) => {
                // The unpinned link detaches when its descriptor closes, on return.
                if node.link.as_ref().is_some_and(|link| link.attached_by == self.seam) {
                    node.link = None;
                }
                Err(steering_error(SimSteeringFailure::Pin, any))
            }
            Some(other) => mismatched("steering pin", other),
        }
    }

    async fn converge_prefix_route(
        &self,
        prefix: Ipv4Net,
        gateway: Ipv4Addr,
    ) -> Result<(), GuestNetworkError> {
        let converged = Self::require_steering(&self.state.lock());
        if converged != (prefix, gateway) {
            unreachable!("the prefix route converges the steering's prefix and gateway");
        }
        let failure = self.gate(SimEffectKind::PrefixRoute, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        st.steering_effects.push(SimSteeringEffect::Route { ok: failure.is_none() });
        match failure {
            None => {
                self.node.state.lock().route = SimPrefixRoute::Local;
                Ok(())
            }
            Some(SimFailure::Netlink) => Err(GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::LocalRouteConverge,
                source: NetlinkError::Connect {
                    source: io::Error::from_raw_os_error(libc::ENOBUFS),
                },
            }),
            Some(other) => mismatched("prefix route", other),
        }
    }

    async fn stale_objects_present(&self) -> Result<bool, GuestNetworkError> {
        match self.gate(SimEffectKind::StaleObjects, SimEffectTarget::None).await {
            None => Ok(self.node.state.lock().stale_objects),
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
            let state = st.listeners.entry(listener).or_default();
            if let Some(kind) = state.failed.take() {
                state.bound = false;
                return Some(Err(io_error(kind)));
            }
            if !state.bound {
                unreachable!("accept on a listener bind_listener has not bound");
            }
            let position = state.queue.iter().position(|conn| conn.released)?;
            let conn = state.queue.remove(position)?;
            let record = RecordState {
                origin: RecordOrigin::Accepted(listener),
                conn: Some(conn.key),
                peer_cid: Some(conn.key.peer_cid),
                first_record: conn.record.clone(),
                buffered: conn.record.into_iter().collect(),
                ready: VecDeque::new(),
                end: conn.end,
                end_released: false,
                peer_gone: false,
            };
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
            if record.peer_gone {
                record.peer_gone = false;
                return Some(Err(io_error(io::ErrorKind::ConnectionReset)));
            }
            if !record.ready.is_empty() {
                let take = max.min(record.ready.len());
                return Some(Ok(record.ready.drain(..take).collect()));
            }
            if record.end_released || (record.buffered.is_empty() && record.end.is_some()) {
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
        let mut st = self.state.lock();
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
            record.peer_gone = false;
            return Err(io_error(io::ErrorKind::BrokenPipe));
        }
        let Some(cid) = record.peer_cid else {
            unreachable!("a record socket always has a peer CID");
        };
        if let Ok(message) = <[u8; CONTROL_MESSAGE_LEN]>::try_from(bytes) {
            st.outbox.entry(cid).or_default().push(message);
        }
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
        let record = RecordState {
            origin: RecordOrigin::HostOpened,
            conn: None,
            peer_cid: Some(cid),
            first_record: Vec::new(),
            buffered: VecDeque::new(),
            ready: VecDeque::new(),
            end: None,
            end_released: false,
            peer_gone: false,
        };
        Ok(RecordSocket(self.issue(&mut st, HandleState::Record(record))))
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
                st.next_port = st.next_port.wrapping_add(1);
                SocketAddrV4::new(*bind.ip(), port)
            } else {
                bind
            }
        });
        Ok(PeerSocket(
            self.issue(&mut st, HandleState::Peer(PeerState { spec, local, peer: None })),
        ))
    }

    fn local_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let id = self.own(peer.0);
        match self.state.lock().handles.get(&id) {
            Some(HandleState::Peer(state)) => {
                state.local.ok_or_else(|| io_error(io::ErrorKind::NotConnected))
            }
            _ => unreachable!("local_addr on a handle that is not a live peer socket"),
        }
    }

    fn peer_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let id = self.own(peer.0);
        match self.state.lock().handles.get(&id) {
            Some(HandleState::Peer(state)) => {
                state.peer.ok_or_else(|| io_error(io::ErrorKind::NotConnected))
            }
            _ => unreachable!("peer_addr on a handle that is not a live peer socket"),
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
        let mut st = self.state.lock();
        let rewritten = st.connect4.get(&destination).copied().unwrap_or(destination);
        let port = st.next_port;
        let Some(HandleState::Peer(state)) = st.handles.get_mut(&id) else {
            unreachable!("connect_udp on a handle that is not a live peer socket");
        };
        if state.spec.protocol != PeerProtocol::Udp {
            unreachable!("connect_udp on a TCP peer socket");
        }
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("connect_udp", other),
            };
        }
        state.peer = Some(rewritten);
        if state.local.is_none() {
            state.local = Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
            st.next_port = st.next_port.wrapping_add(1);
        }
        Ok(())
    }

    async fn connect_peer(
        &self,
        peer: &ArmedPeer,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        let id = self.own(peer.0);
        let conn = match self.state.lock().handles.get(&id) {
            Some(HandleState::Armed { conn, .. }) => *conn,
            _ => unreachable!("connect_peer on a handle that is not a live armed peer"),
        };
        if let Some(conn) = conn {
            self.state.lock().connects_in_flight.insert(conn);
        }
        let failure =
            self.gate(SimEffectKind::ConnectPeer, SimEffectTarget::Addr(destination)).await;
        let mut st = self.state.lock();
        if let Some(conn) = conn {
            st.connects_in_flight.remove(&conn);
        }
        let port = st.next_port;
        let Some(HandleState::Armed { peer: state, .. }) = st.handles.get_mut(&id) else {
            unreachable!("connect_peer on a handle closed while its connect was held");
        };
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("connect_peer", other),
            };
        }
        state.peer = Some(destination);
        if state.local.is_none() {
            state.local = Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
            st.next_port = st.next_port.wrapping_add(1);
        }
        Ok(())
    }

    async fn route_type(&self, destination: Ipv4Addr) -> Result<HostRouteType, FlowIoError> {
        match self.gate(SimEffectKind::RouteType, SimEffectTarget::Ip(destination)).await {
            None => Ok(self
                .state
                .lock()
                .route_types
                .get(&destination)
                .copied()
                .unwrap_or(HostRouteType::Unicast)),
            Some(SimFailure::Io(kind)) => Err(io_error(kind)),
            Some(other) => mismatched("route_type", other),
        }
    }

    async fn bind_intake(&self, address: SocketAddrV4) -> Result<IntakeListener, FlowIoError> {
        let failure = self.gate(SimEffectKind::BindIntake, SimEffectTarget::Addr(address)).await;
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("bind_intake", other),
            };
        }
        let mut st = self.state.lock();
        let in_use = st
            .handles
            .values()
            .any(|handle| matches!(handle, HandleState::Listener(state) if state.bound == address));
        if in_use {
            return Err(io_error(io::ErrorKind::AddrInUse));
        }
        let state =
            IntakeListenerState { bound: address, registered: false, queue: VecDeque::new() };
        Ok(IntakeListener(self.issue(&mut st, HandleState::Listener(state))))
    }

    async fn accept_intake(
        &self,
        listener: &IntakeListener,
    ) -> Result<(IntakeChild, SocketAddrV4), FlowIoError> {
        let id = self.own(listener.0);
        self.wait_changed(|st| {
            let Some(HandleState::Listener(state)) = st.handles.get_mut(&id) else {
                unreachable!("accept_intake on a handle that is not a live intake listener");
            };
            let position = state.queue.iter().position(|queued| queued.released)?;
            let client = state.queue.remove(position)?.client;
            let child = self.issue(st, HandleState::Child { client, hangup: None });
            Some(Ok((IntakeChild(child), client)))
        })
        .await
    }

    async fn steer(
        &self,
        intake: SocketAddrV4,
        listener: &IntakeListener,
    ) -> Result<(), GuestSteeringError> {
        let id = self.own(listener.0);
        {
            let st = self.state.lock();
            Self::require_forwarder(&st);
            Self::require_steering(&st);
        }
        let failure = self.gate(SimEffectKind::Steer, SimEffectTarget::Addr(intake)).await;
        let st = self.state.lock();
        let (prefix, gateway) = Self::require_steering(&st);
        let Some(HandleState::Listener(state)) = st.handles.get(&id) else {
            unreachable!("steer on a handle that is not a live intake listener");
        };
        let bound = state.bound;
        drop(st);
        let reference = SimListenerRef { bound, seam: self.seam, handle: id };
        let result = if !prefix.contains(intake.ip()) || *intake.ip() == gateway {
            Err(GuestSteeringError::IntakeOutsideGuestPrefix { intake })
        } else if bound.ip() != intake.ip() {
            Err(GuestSteeringError::UnsupportedSocket)
        } else if let Some(SimFailure::Steering(failure)) = failure {
            Err(steering_error(failure, intake))
        } else if let Some(other) = failure {
            mismatched("steer", other)
        } else {
            let mut node = self.node.state.lock();
            match node.entries.get(&intake) {
                Some(existing) if *existing != reference => {
                    Err(GuestSteeringError::IntakeAlreadySteered { intake })
                }
                _ => {
                    node.entries.insert(intake, reference);
                    Ok(())
                }
            }
        };
        let outcome = match &result {
            Ok(()) => SimSteerOutcome::Ok,
            Err(GuestSteeringError::KernelMemory { .. }) => SimSteerOutcome::KernelMemory,
            Err(_) => SimSteerOutcome::Defect,
        };
        self.state.lock().steering_effects.push(SimSteeringEffect::Steer { intake, outcome });
        result
    }

    async fn install_vsock(&self, vsock: RecordSocket) -> Result<InstalledVsock, GuestFlowError> {
        Self::require_forwarder(&self.state.lock());
        let failure = self.gate(SimEffectKind::InstallVsock, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let state = self.take(&mut st, vsock.0);
        let HandleState::Record(record) = state else {
            unreachable!("install_vsock of a handle that is not a record socket");
        };
        match failure {
            None => {
                let label = Self::label_of_record(&record);
                let conn = record.conn;
                Ok(InstalledVsock(self.issue(&mut st, HandleState::Installed { conn, label })))
            }
            Some(SimFailure::Forwarder(failure)) => {
                Self::abort_states(&mut st, vec![HandleState::Record(record)]);
                Err(forwarder_error(failure))
            }
            Some(other) => mismatched("install_vsock", other),
        }
    }

    async fn arm_peer(
        &self,
        vsock: &InstalledVsock,
        peer: PeerSocket,
    ) -> Result<ArmedPeer, GuestFlowError> {
        let installed = self.own(vsock.0);
        let failure = self.gate(SimEffectKind::ArmPeer, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let (conn, label) = match st.handles.get(&installed) {
            Some(HandleState::Installed { conn, label }) => (*conn, *label),
            _ => unreachable!("arm_peer with a handle that is not an installed vsock"),
        };
        let state = self.take(&mut st, peer.0);
        let HandleState::Peer(peer_state) = state else {
            unreachable!("arm_peer of a handle that is not a peer socket");
        };
        match failure {
            None => Ok(ArmedPeer(
                self.issue(&mut st, HandleState::Armed { peer: peer_state, conn, label }),
            )),
            Some(SimFailure::Forwarder(failure)) => {
                Self::abort_states(&mut st, vec![HandleState::Peer(peer_state)]);
                Err(forwarder_error(failure))
            }
            Some(other) => mismatched("arm_peer", other),
        }
    }

    async fn route_outbound(
        &self,
        vsock: InstalledVsock,
        peer: ArmedPeer,
    ) -> Result<TcpPair, GuestFlowError> {
        let failure = self.gate(SimEffectKind::RouteOutbound, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let installed = self.take(&mut st, vsock.0);
        let armed = self.take(&mut st, peer.0);
        let (HandleState::Installed { conn, label }, HandleState::Armed { peer: peer_state, .. }) =
            (&installed, &armed)
        else {
            unreachable!(
                "route_outbound of handles that are not an installed vsock and an armed peer"
            );
        };
        let (conn, label, abortive) = (*conn, *label, peer_state.spec.abortive_linger);
        if let Some(failure) = failure {
            Self::abort_states(&mut st, vec![installed, armed]);
            return match failure {
                SimFailure::Forwarder(failure) => Err(forwarder_error(failure)),
                other => mismatched("route_outbound", other),
            };
        }
        let key = PairKey(st.next_pair);
        st.next_pair += 1;
        st.pairs.insert(key, PairEntry::default());
        let socket = |role| {
            HandleState::Pair(PairSocketState {
                pair: key,
                role,
                conn,
                label,
                hangup: None,
                linger_cleared: false,
                shut: false,
            })
        };
        let vsock = PairSocket(self.issue(&mut st, socket(PairRole::Vsock)));
        let outer = PairSocket(self.issue(&mut st, socket(PairRole::Outer { abortive })));
        Ok(TcpPair { key, vsock, outer })
    }

    async fn install_datagram(
        &self,
        stream: RecordSocket,
        seqpacket: RecordSocket,
        udp: PeerSocket,
    ) -> Result<DatagramPair, GuestFlowError> {
        Self::require_forwarder(&self.state.lock());
        let failure = self.gate(SimEffectKind::InstallDatagram, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let stream_state = self.take(&mut st, stream.0);
        let seqpacket_state = self.take(&mut st, seqpacket.0);
        let udp_state = self.take(&mut st, udp.0);
        let (HandleState::Record(stream_record), HandleState::Record(_), HandleState::Peer(_)) =
            (&stream_state, &seqpacket_state, &udp_state)
        else {
            unreachable!("install_datagram of handles that are not two records and a peer");
        };
        let (conn, label) = (stream_record.conn, Self::label_of_record(stream_record));
        if let Some(failure) = failure {
            Self::abort_states(&mut st, vec![stream_state, seqpacket_state, udp_state]);
            return match failure {
                SimFailure::Forwarder(failure) => Err(forwarder_error(failure)),
                other => mismatched("install_datagram", other),
            };
        }
        let key = PairKey(st.next_pair);
        st.next_pair += 1;
        st.pairs.insert(key, PairEntry::default());
        let socket = |role| {
            HandleState::Pair(PairSocketState {
                pair: key,
                role,
                conn,
                label,
                hangup: None,
                linger_cleared: false,
                shut: false,
            })
        };
        let stream = PairSocket(self.issue(&mut st, socket(PairRole::Stream)));
        let seqpacket = PairSocket(self.issue(&mut st, socket(PairRole::Seqpacket)));
        let udp = PairSocket(self.issue(&mut st, socket(PairRole::Udp)));
        Ok(DatagramPair { key, stream, seqpacket, udp })
    }

    async fn register_intake(&self, listener: &IntakeListener) -> Result<(), GuestFlowError> {
        let id = self.own(listener.0);
        Self::require_forwarder(&self.state.lock());
        let failure = self.gate(SimEffectKind::RegisterIntake, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let Some(HandleState::Listener(state)) = st.handles.get_mut(&id) else {
            unreachable!("register_intake with a handle that is not an intake listener");
        };
        match failure {
            None => {
                state.registered = true;
                Ok(())
            }
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("register_intake", other),
        }
    }

    async fn unregister_intake(&self, listener: &IntakeListener) -> Result<(), GuestFlowError> {
        let id = self.own(listener.0);
        Self::require_forwarder(&self.state.lock());
        let failure = self.gate(SimEffectKind::UnregisterIntake, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let Some(HandleState::Listener(state)) = st.handles.get_mut(&id) else {
            unreachable!("unregister_intake with a handle that is not an intake listener");
        };
        match failure {
            None => {
                state.registered = false;
                Ok(())
            }
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("unregister_intake", other),
        }
    }

    async fn adopt_intake(&self, child: IntakeChild) -> Result<ParkedChild, GuestFlowError> {
        Self::require_forwarder(&self.state.lock());
        let failure = self.gate(SimEffectKind::AdoptIntake, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let state = self.take(&mut st, child.0);
        let HandleState::Child { client, hangup } = state else {
            unreachable!("adopt_intake of a handle that is not an intake child");
        };
        match failure {
            None => Ok(ParkedChild(self.issue(&mut st, HandleState::Parked { client, hangup }))),
            Some(SimFailure::Forwarder(failure)) => {
                Self::abort_states(&mut st, vec![HandleState::Child { client, hangup }]);
                Err(forwarder_error(failure))
            }
            Some(other) => mismatched("adopt_intake", other),
        }
    }

    async fn begin_inbound(
        &self,
        child: ParkedChild,
        vsock: RecordSocket,
    ) -> Result<PendingInbound, GuestFlowError> {
        let failure = self.gate(SimEffectKind::BeginInbound, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let parked = self.take(&mut st, child.0);
        let record = self.take(&mut st, vsock.0);
        let (HandleState::Parked { client, hangup }, HandleState::Record(record_state)) =
            (&parked, &record)
        else {
            unreachable!("begin_inbound of handles that are not a parked child and V_h");
        };
        if record_state.origin != RecordOrigin::HostOpened {
            unreachable!("begin_inbound with a record socket that is not V_h");
        }
        let (client, hangup, cid) = (*client, *hangup, record_state.peer_cid);
        if let Some(failure) = failure {
            Self::abort_states(&mut st, vec![parked, record]);
            return match failure {
                SimFailure::Forwarder(failure) => Err(forwarder_error(failure)),
                other => mismatched("begin_inbound", other),
            };
        }
        let pending = HandleState::Pending { client, hangup, cid, request: None };
        Ok(PendingInbound(self.issue(&mut st, pending)))
    }

    async fn write_request(
        &self,
        pending: &PendingInbound,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> Result<(), FlowIoError> {
        let id = self.own(pending.0);
        let failure = self.gate(SimEffectKind::WriteRequest, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let Some(HandleState::Pending { request: written, .. }) = st.handles.get_mut(&id) else {
            unreachable!("write_request on a handle that is not a pending inbound flow");
        };
        if written.is_some() {
            unreachable!("write_request twice on one pending inbound flow");
        }
        match failure {
            None => {
                *written = Some(*request);
                Ok(())
            }
            Some(SimFailure::Io(kind)) => Err(io_error(kind)),
            Some(other) => mismatched("write_request", other),
        }
    }

    async fn pending_hangup(&self, pending: &PendingInbound) -> FlowHangup {
        let id = self.own(pending.0);
        self.wait_changed(|st| match st.handles.get(&id) {
            Some(HandleState::Pending { hangup, .. }) => *hangup,
            _ => unreachable!("pending_hangup on a handle that is not a pending inbound flow"),
        })
        .await
    }

    async fn route_inbound(&self, pending: PendingInbound) -> Result<TcpPair, GuestFlowError> {
        let failure = self.gate(SimEffectKind::RouteInbound, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let state = self.take(&mut st, pending.0);
        let HandleState::Pending { request, hangup, .. } = &state else {
            unreachable!("route_inbound of a handle that is not a pending inbound flow");
        };
        let (label, hangup) = (*request, *hangup);
        if let Some(failure) = failure {
            Self::abort_states(&mut st, vec![state]);
            return match failure {
                SimFailure::Forwarder(failure) => Err(forwarder_error(failure)),
                other => mismatched("route_inbound", other),
            };
        }
        let key = PairKey(st.next_pair);
        st.next_pair += 1;
        st.pairs.insert(key, PairEntry::default());
        let vsock = PairSocket(self.issue(
            &mut st,
            HandleState::Pair(PairSocketState {
                pair: key,
                role: PairRole::Vsock,
                conn: None,
                label,
                hangup: None,
                linger_cleared: false,
                shut: false,
            }),
        ));
        let outer = PairSocket(self.issue(
            &mut st,
            HandleState::Pair(PairSocketState {
                pair: key,
                role: PairRole::Outer { abortive: true },
                conn: None,
                label,
                hangup,
                linger_cleared: false,
                shut: false,
            }),
        ));
        Ok(TcpPair { key, vsock, outer })
    }

    async fn drained(
        &self,
        pair: PairKey,
        direction: FlowDirection,
    ) -> Result<bool, GuestFlowError> {
        match self.gate(SimEffectKind::Drained, SimEffectTarget::Pair(pair)).await {
            None => {
                let st = self.state.lock();
                Ok(st.pairs.get(&pair).is_none_or(|entry| match direction {
                    FlowDirection::TowardGuest => entry.toward_guest_drained,
                    FlowDirection::FromGuest => entry.from_guest_drained,
                }))
            }
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("drained", other),
        }
    }

    async fn remove_pair(&self, pair: PairKey) -> Result<(), GuestFlowError> {
        match self.gate(SimEffectKind::RemovePair, SimEffectTarget::Pair(pair)).await {
            None => {
                self.state.lock().pairs.remove(&pair);
                Ok(())
            }
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("remove_pair", other),
        }
    }

    async fn inventory(&self) -> Result<GuestFlowInventory, GuestFlowError> {
        match self.gate(SimEffectKind::Inventory, SimEffectTarget::None).await {
            None => {
                let st = self.state.lock();
                Self::require_forwarder(&st);
                let pairs = u32::try_from(st.pairs.len()).unwrap_or(u32::MAX);
                let intake_listeners = u32::try_from(
                    st.handles
                        .values()
                        .filter(|handle| {
                            matches!(handle, HandleState::Listener(state) if state.registered)
                        })
                        .count(),
                )
                .unwrap_or(u32::MAX);
                Ok(GuestFlowInventory {
                    pairs,
                    free_cells: GUEST_FLOW_MAX_PAIRS.saturating_sub(pairs),
                    intake_listeners,
                    links_attached: GuestFlowLinks {
                        verdict: true,
                        sock_ops: true,
                        unframe_lo: true,
                        drain_counter: true,
                    },
                    unframe_interfaces: st
                        .forwarder
                        .as_ref()
                        .map(|forwarder| forwarder.attached.clone())
                        .unwrap_or_default(),
                })
            }
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("inventory", other),
        }
    }

    async fn counters(&self) -> Result<GuestFlowCounters, GuestFlowError> {
        match self.gate(SimEffectKind::Counters, SimEffectTarget::None).await {
            None => Ok(self.state.lock().counters),
            Some(SimFailure::Forwarder(failure)) => Err(forwarder_error(failure)),
            Some(other) => mismatched("counters", other),
        }
    }

    async fn hangup(&self, socket: &PairSocket) -> FlowHangup {
        let id = self.own(socket.0);
        self.wait_changed(|st| match st.handles.get(&id) {
            Some(HandleState::Pair(pair)) => pair.hangup,
            _ => unreachable!("hangup on a handle that is not a live pair socket"),
        })
        .await
    }

    async fn shutdown_write(&self, socket: &PairSocket) -> Result<(), FlowIoError> {
        let id = self.own(socket.0);
        let failure = self.gate(SimEffectKind::ShutdownWrite, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let Some(HandleState::Pair(pair)) = st.handles.get_mut(&id) else {
            unreachable!("shutdown_write on a handle that is not a live pair socket");
        };
        if let Some(failure) = failure {
            return match failure {
                SimFailure::Io(kind) => Err(io_error(kind)),
                other => mismatched("shutdown_write", other),
            };
        }
        if pair.shut {
            return Ok(());
        }
        pair.shut = true;
        let (role, key, label) = (pair.role, pair.pair, pair.label);
        if let Some(label) = label {
            match role {
                PairRole::Vsock => {
                    st.vsock_ends_in_flight.insert(key, label);
                }
                PairRole::Outer { .. } => st.far_end.push((label, FarEndEvent::Eof)),
                PairRole::Stream | PairRole::Seqpacket | PairRole::Udp => {}
            }
        }
        Ok(())
    }

    async fn clear_linger(&self, socket: &PairSocket) -> Result<(), FlowIoError> {
        let id = self.own(socket.0);
        let failure = self.gate(SimEffectKind::ClearLinger, SimEffectTarget::None).await;
        let mut st = self.state.lock();
        let Some(HandleState::Pair(pair)) = st.handles.get_mut(&id) else {
            unreachable!("clear_linger on a handle that is not a live pair socket");
        };
        match failure {
            None => {
                pair.linger_cleared = true;
                Ok(())
            }
            Some(SimFailure::Io(kind)) => Err(io_error(kind)),
            Some(other) => mismatched("clear_linger", other),
        }
    }

    async fn abort(&self, socket: OwnedSocket) {
        if let Some(failure) = self.gate(SimEffectKind::Abort, SimEffectTarget::None).await {
            mismatched("abort", failure);
        }
        self.end_owned(socket, true);
    }

    async fn close(&self, socket: OwnedSocket) {
        if let Some(failure) = self.gate(SimEffectKind::Close, SimEffectTarget::None).await {
            mismatched("close", failure);
        }
        self.end_owned(socket, false);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "test fixtures assert their preconditions"
)]
mod tests;
