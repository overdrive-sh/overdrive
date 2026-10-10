//! Additive host guest-flow owner surface — vsock attachment replacement
//! (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Owner and provisioner" (`QuiescenceHolder`, `FlowCloseCause`),
//! § "[REF] Typed error taxonomy" (`InterfaceStacking`, `StackedLinkKind`),
//! § DG-3 ("Owner construction": `GuestFlowOwnerPorts`,
//! `HostSharedGuestNetworkOwner::with_flow_io`), § DG-13
//! (`IntakePortBlockConflict`) and § "Conformance observation surface"
//! QPROJ-2 (`GuestAddressPool::lease_observation`) and QPROJ-3
//! (`HostSharedGuestNetworkOwner::flow_observation`). Every item below is the
//! one those sections pin; the items are re-exported from
//! `overdrive_control_plane::guest_network`.
//!
//! The owner's constructors (`new(GuestFlowOwnerPorts)`, `with_flow_io`) live
//! beside its struct in `guest_network`. The observation accessors are RED
//! scaffolds; the owner they observe is the guest-flow owner the attachment
//! cut lands. The data types are plain
//! data and are written for real.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-steps 7, 9, 13, 15; attachment cut).

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::wire::FlowId;
use overdrive_core::guest_transport::{GuestCid, LaunchToken};
use overdrive_core::id::AllocationId;
use overdrive_core::net::{InterfaceIndex, ListenPort};
use overdrive_core::traits::GuestAttachmentLease;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_resolve::MtlsResolve;
use overdrive_core::vm::config::VsockPort;
use overdrive_dataplane::allocators::VipRange;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use super::{GuestAddressPool, HostSharedGuestNetworkOwner};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// Owner construction (DG-3)
// ---------------------------------------------------------------------------

/// Every input the owner needs; no field has a default (`development.md`
/// § "Port-trait dependencies").
pub struct GuestFlowOwnerPorts {
    /// Every deadline and the audit cadence read this clock.
    pub clock: Arc<dyn Clock>,
    /// The owner's single resolution of a guest TCP destination, with the VIP
    /// branch (D24a).
    pub resolve: Arc<dyn MtlsResolve>,
    /// The mTLS worker handle (mesh `register_forwarded_outbound`, D9).
    pub mtls: Arc<MtlsInterceptWorker>,
    /// The configured service VIP range — the same value the VIP allocator
    /// and `ServiceBackendsResolve::new` receive (D24a, D8-PREFIX-VIP).
    pub vip_range: VipRange,
    /// TCP ports `serve` binds on the unspecified address (INTAKE-BIND).
    pub wildcard_tcp_ports: BTreeSet<u16>,
    /// `GUEST_NODE_PREFIX` / `GUEST_NODE_GATEWAY` in production (DG-14).
    pub node_prefix: Ipv4Net,
    /// The guest gateway inside `node_prefix` (DG-14).
    pub gateway: Ipv4Addr,
}

impl HostSharedGuestNetworkOwner {
    /// QPROJ-3: an owned snapshot of the owner's per-CID tables, the holder
    /// set and the listener-task state.
    ///
    /// - **Preconditions:** none. Infallible; callable on a started owner at
    ///   any time, including while quiesced and after `teardown` of every
    ///   allocation.
    /// - **Postconditions:** the returned value is an owned copy of the
    ///   owner's per-CID tables, the holder set and the listener-task state.
    ///   No lock or queue slot is held after return. The call starts no step:
    ///   it opens no listener, registers no flow, sends no message and changes
    ///   no state. Dropping the future before it completes has no effect on
    ///   the owner (cancellation-safe).
    /// - **Consistency point:** a state between two steps of every CID's
    ///   serialized order (M-7) and of quiescence. The call is ordered through
    ///   each CID's queue and the quiescence section, so it never reports a
    ///   flow half-registered, a holder set that disagrees with `admission`
    ///   because a quiesce is mid-way, or a listener open for a port whose
    ///   steering entry is not yet removed. Steps that complete after the call
    ///   returns are not reflected.
    /// - **Edge cases:** an owner with no allocation returns empty `cids`,
    ///   empty `holders`, `admission = Open` and `flow_listeners_running =
    ///   true`. A flow, association or CID the owner no longer holds is
    ///   absent; a CID in `Retiring` stays until `teardown` returns `Ok`
    ///   (after which the pool, QPROJ-2, reports `Retiring` until `release`).
    ///   `admission = Quiesced` iff `holders` is non-empty.
    ///   `flow_listeners_running = false` after a listener-task exit until its
    ///   recovery repairs it.
    /// - **Observable invariants:** each `GuestConnKey` appears in at most one
    ///   of a CID's `unread`, `guest_flows` and `session`; `intake` and
    ///   `parked` are disjoint subsets of the CID's declared ports; `intake`
    ///   is empty while `holders` is non-empty (M-7); a repeated call with no
    ///   intervening owner step returns an equal value.
    #[expect(
        clippy::unused_async,
        reason = "RED scaffold body panics; DELIVER vsock new-step 7 lands the observation"
    )]
    #[expect(
        dead_code,
        reason = "RED scaffold — the owner_flows / intake / quiescence drivers read it at the attachment cut"
    )]
    pub async fn flow_observation(&self) -> GuestOwnerObservation {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// QPROJ-3 — host guest-flow owner observation
// ---------------------------------------------------------------------------

/// QPROJ-3 snapshot of the host guest-flow owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestOwnerObservation {
    /// D8a-HOLD holder set.
    pub holders: BTreeSet<QuiescenceHolder>,
    /// The admission latch, read apart from the holder set so that a
    /// divergence between them is visible.
    pub admission: GuestAdmission,
    /// The flow and control listeners' accept tasks are all running
    /// (`false` after a listener-task exit, until repaired).
    pub flow_listeners_running: bool,
    /// Per-CID tables.
    pub cids: BTreeMap<GuestCid, CidObservation>,
}

/// The owner's admission latch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAdmission {
    /// New flows are admitted.
    Open,
    /// New flows are refused `Quiesced` (D8a-HOLD).
    Quiesced,
}

/// One CID's state at the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CidObservation {
    /// The allocation the CID is leased to.
    pub alloc: AllocationId,
    /// The lease's launch token (ATTR-TOKEN).
    pub launch: LaunchToken,
    /// Provisioned | Active | Retiring.
    pub phase: CidPhase,
    /// Holder of the CID's `ClaimSet<GuestCid>` session claim.
    pub session: Option<GuestConnKey>,
    /// Control connections accepted on 1243 whose `ControlOpen` is not yet
    /// read (flow connections are registered at accept, U-1, and appear in
    /// `guest_flows` / `datagrams`).
    pub unread: BTreeSet<GuestConnKey>,
    /// Registered guest-opened flows (U-1), by connection.
    pub guest_flows: BTreeMap<GuestConnKey, GuestFlowPhase>,
    /// Host-opened `TcpAccept` flows, by host-origin flow id.
    pub host_flows: BTreeMap<FlowId, HostFlowPhase>,
    /// Datagram associations, by guest-origin flow id.
    pub datagrams: BTreeMap<FlowId, AssociationPhase>,
    /// The ports the guest's last `ListenState` reports listening.
    pub last_listen_state: BTreeSet<ListenPort>,
    /// Declared ports with an open (bound, listening) intake listener.
    pub intake: BTreeSet<ListenPort>,
    /// Serving ports a reported steer defect parked (D8a-CAP).
    pub parked: BTreeSet<ListenPort>,
}

/// The owner's per-CID lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CidPhase {
    /// Registered by `provision`; not yet activated.
    Provisioned,
    /// Activated: flows from this CID may be paired.
    Active,
    /// Teardown in progress.
    Retiring,
}

/// Kernel-reported identity of an accepted guest-opened host vsock connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuestConnKey {
    /// The host vsock listener port that accepted it.
    pub listener: VsockPort,
    /// The guest CID that opened it.
    pub peer_cid: GuestCid,
    /// The guest's source vsock port.
    pub peer_port: VsockPort,
}

/// A registered guest-opened flow's phase at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowPhase {
    /// Admitted and registered (U-1); the request is not yet complete. Its
    /// completion and the token comparison are one step: the flow becomes
    /// `Attributed`, or is closed.
    Registered {
        /// How much of the request the owner has read.
        request: RequestRead,
    },
    /// The request's token matched the live token (registration complete).
    Attributed,
    /// The flow vsock is installed in the forwarder.
    Installed,
    /// The non-blocking connect is issued; its completion is not processed.
    Connecting,
    /// Paired.
    Paired(FlowEndObservation),
}

/// How much of a flow request the owner has read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRead {
    /// No byte read.
    Nothing,
    /// Some, but not all, bytes read.
    Partial,
}

/// A host-opened `TcpAccept` flow's phase at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFlowPhase {
    /// V_h connected and the request written; `Paired` not yet received.
    /// `outer_eof`: the intake child read EOF before `Paired` (recorded).
    AwaitingPaired {
        /// The intake child read EOF before `Paired`.
        outer_eof: bool,
    },
    /// Paired.
    Paired(FlowEndObservation),
}

/// A datagram association's phase at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationPhase {
    /// Requested; `Paired` not yet sent.
    AwaitingPaired,
    /// Paired.
    Paired,
}

/// HALFCLOSE-SIGNAL state of one direction pair of a paired TCP flow, at
/// this owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowEndObservation {
    /// The outer leg read EOF.
    pub outer_eof: bool,
    /// This side's announcement of its direction.
    pub announce: HalfCloseAnnounce,
    /// This side acknowledged the peer's `HalfClose`.
    pub peer_announced: bool,
    /// This side shut its outer leg down for writing.
    pub outer_shut: bool,
}

/// This side's `HalfClose` announcement of its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfCloseAnnounce {
    /// Not announced.
    None,
    /// `HalfClose` sent; `HalfCloseAck` not yet received.
    Pending,
    /// `HalfCloseAck` received.
    Done,
}

// ---------------------------------------------------------------------------
// D8a-HOLD — quiescence holders
// ---------------------------------------------------------------------------

/// D8a-HOLD: who holds forwarding quiescence. One hold per holder at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuiescenceHolder {
    /// The runtime supervisor's recovery of one shared component.
    Recovery(SharedGuestNetworkComponent),
}

impl QuiescenceHolder {
    /// The holder's canonical label.
    ///
    /// RED scaffold: the label vocabulary is not pinned by the design
    /// (`SharedGuestNetworkComponent` carries no `as_str`); DELIVER lands it
    /// with the attachment cut's component set (DG-2 row 10).
    #[expect(
        clippy::unused_self,
        clippy::trivially_copy_pass_by_ref,
        reason = "RED scaffold body panics; the vocabulary lands with the attachment cut (the `&self` signature is pinned)"
    )]
    pub fn as_str(&self) -> &'static str {
        panic!("{SCAFFOLD}")
    }
}

/// D8a-HOLD: proof that `holder()` holds quiescence. Not `Clone`, not
/// constructible outside the owner; consumed only by `restore_forwarding`.
/// Dropping it without a restore leaves the holder registered (forwarding
/// stays quiesced; the supervisor's bounded recovery then fail-stops).
#[must_use]
#[derive(Debug)]
pub struct QuiescenceHold {
    pub(crate) holder: QuiescenceHolder,
}

impl QuiescenceHold {
    /// The holder this hold belongs to.
    pub const fn holder(&self) -> QuiescenceHolder {
        self.holder
    }
}

// ---------------------------------------------------------------------------
// Flow end (HALFCLOSE-SIGNAL, D26-PINS)
// ---------------------------------------------------------------------------

/// Why the owner closed a flow (`guest_flow.closed { cause }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowCloseCause {
    /// The far end read EOF and the flow closed cleanly.
    PeerEof,
    /// The far end reset.
    PeerReset,
    /// The flow was refused.
    Refused,
    /// The peer owner sent `Abort`.
    AbortReceived,
    /// Forwarding quiesced (D8a-HOLD).
    Quiesced,
    /// The allocation was torn down.
    Teardown,
    /// The owner aborted the flow.
    OwnerAbort,
    /// The request did not arrive within `FLOW_REQUEST_DEADLINE`.
    RequestTimeout,
    /// The flow did not pair within `FLOW_PAIRING_DEADLINE`.
    PairingTimeout,
    /// A datagram leg had no matching request.
    UnmatchedDatagram,
    /// The two datagram requests disagree.
    DatagramRequestMismatch,
    /// A direction did not drain within `FLOW_DRAIN_DEADLINE`.
    DrainTimeout,
    /// The control session was lost.
    ControlSessionLost,
    /// The control session carried a malformed message.
    ControlMalformed,
    /// The CID has no control session.
    NoControlSession,
    /// The record carried another launch's token (ATTR-TOKEN).
    StaleLaunch,
    /// HALFCLOSE-SIGNAL: the flow vsock ended for a direction whose
    /// `HalfClose` this side had not acknowledged — the peer owner is lost,
    /// or the peer aborted and its vsock end overtook its `Abort` message.
    UnannouncedVsockEnd,
    /// D26-PINS: the host-internal output rule rejected the transmit of a
    /// forwarded datagram (reported on the owner's UDP socket).
    DestinationUnreachable,
}

impl FlowCloseCause {
    /// The cause's telemetry label.
    ///
    /// RED scaffold: the label vocabulary is not pinned by the design;
    /// DELIVER vsock new-step 7 lands it with the owner flows.
    #[expect(
        clippy::unused_self,
        reason = "RED scaffold body panics; DELIVER vsock new-step 7 lands it"
    )]
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// ---------------------------------------------------------------------------
// probe_startup refusal vocabulary (INTAKE-BIND, IFACE-NO-STACK)
// ---------------------------------------------------------------------------

/// INTAKE-BIND (DG-13): what the intake host-port block meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntakePortBlockConflict {
    /// The block intersects `ip_local_port_range` = `low..=high`.
    EphemeralRange {
        /// The range's low end.
        low: u16,
        /// The range's high end.
        high: u16,
    },
    /// `serve` is configured to bind a TCP listener on `0.0.0.0:port`, a port
    /// of the block.
    HostListener {
        /// The configured wildcard listener port.
        port: u16,
    },
}

/// IFACE-NO-STACK: how a root-namespace interface is stacked on another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceStacking {
    /// The interface has a master (a bridge or bond port).
    Enslaved {
        /// The master interface.
        master: InterfaceIndex,
    },
    /// The interface is an upper device of a stacking link kind.
    Upper {
        /// The stacking link kind.
        kind: StackedLinkKind,
        /// The lower interface.
        lower: InterfaceIndex,
    },
}

/// IFACE-NO-STACK: the stacking link kinds `probe_startup` refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackedLinkKind {
    /// `vlan`.
    Vlan,
    /// `macvlan`.
    Macvlan,
    /// `macvtap`.
    Macvtap,
    /// `ipvlan`.
    Ipvlan,
    /// `ipvtap`.
    Ipvtap,
}

impl StackedLinkKind {
    /// The kernel's link kind name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vlan => "vlan",
            Self::Macvlan => "macvlan",
            Self::Macvtap => "macvtap",
            Self::Ipvlan => "ipvlan",
            Self::Ipvtap => "ipvtap",
        }
    }
}

// ---------------------------------------------------------------------------
// QPROJ-2 — pool lease observation
// ---------------------------------------------------------------------------

impl GuestAddressPool {
    /// QPROJ-2: an owned copy of the pool's lease table and cursor.
    ///
    /// - **Preconditions:** none. Infallible; callable at any time on a live
    ///   pool.
    /// - **Postconditions:** the returned value is an owned copy of the pool's
    ///   lease table and cursor taken under one acquisition of the pool lock.
    ///   The lock is released before return. `leases` holds exactly the
    ///   Admitted and Retiring leases at that instant; a released lease is
    ///   absent. `next_fit` is the cursor as stored, not normalised to a
    ///   usable offset. The call attempts no claim, runs no `assign` walk,
    ///   takes no lease and changes no state.
    /// - **Consistency point:** between two pool operations (`assign`,
    ///   `take_claim`, `retire`, `release`, the claim drop of a lease): never
    ///   inside an `assign` walk, whose per-offset `claim` calls happen under
    ///   the same lock.
    /// - **Edge cases:** an empty pool returns an empty `leases` and the
    ///   initial cursor. A lease whose claim was taken and passed to the VMM
    ///   reports `claim_in_slot = false`; one whose claim was dropped unused
    ///   also reports `false`. A Retiring lease keeps its CID, launch token
    ///   and reserved ports until `release`.
    /// - **Invariants:** `leases` has at most one entry per CID; the sum of
    ///   `reserved_ports` never exceeds the steering capacity (D8a-CAP); a
    ///   repeated call with no intervening pool operation returns an equal
    ///   value.
    #[expect(
        clippy::unused_self,
        reason = "RED scaffold body panics; DELIVER vsock new-step 15 lands the CID lease table"
    )]
    pub fn lease_observation(&self) -> GuestLeaseObservation {
        panic!("{SCAFFOLD}")
    }
}

/// QPROJ-2 snapshot of the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLeaseObservation {
    /// The CID at which the next `assign` walk starts (the cursor, M-9).
    pub next_fit: GuestCid,
    /// Every Admitted and Retiring lease.
    pub leases: BTreeMap<AllocationId, GuestLeaseView>,
}

/// One lease in a [`GuestLeaseObservation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLeaseView {
    /// The leased CID.
    pub cid: GuestCid,
    /// The existing lease state: Admitted | Retiring.
    pub state: GuestAttachmentLease,
    /// The lease's launch token (ATTR-TOKEN).
    pub launch: LaunchToken,
    /// The claim is still in the lease's take-once slot: not taken by the
    /// VM driver and not dropped.
    pub claim_in_slot: bool,
    /// Declared TCP ports this lease reserves (D8a-CAP).
    pub reserved_ports: u32,
}
