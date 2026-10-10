//! Shared guest-network application contract (GH #295).
//!
//! The private host owner/pool implementation and accepted executable
//! specifications live at this application boundary.

#![expect(
    clippy::use_self,
    reason = "exact accepted API scaffold precedes implementation and names GuestNetworkError explicitly"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::wire::FlowWireError;
use overdrive_core::guest_transport::{
    GuestCid, GuestCidClaim, GuestCidClaimError, GuestTransportAssignment,
};
use overdrive_core::id::AllocationId;
use overdrive_core::net::{InterfaceIndex, ListenPort};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::{
    GuestAttachmentLease, GuestAttachmentObservation, GuestAttachmentView,
};
use overdrive_netlink::NetlinkError;

use overdrive_dataplane::guest_flow::{GuestFlowError, UnframeAttachError};
use overdrive_dataplane::guest_steering::GuestSteeringError;
pub use overdrive_dataplane::guest_tcx::TcxAttachPoint;
pub use overdrive_netlink::nft::bridge::{
    BridgeGuardRuleExpression, BridgeGuardRuleFact, BridgeGuardRuleIdentity, BridgeGuardRuleProgram,
};

mod flow_io;
mod flow_owner;
#[cfg(test)]
mod prefix_boot_acceptance;
#[cfg(all(test, feature = "integration-tests"))]
mod prefix_boot_conformance;

pub use flow_owner::{
    AssociationPhase, CidObservation, CidPhase, FlowCloseCause, FlowEndObservation, GuestAdmission,
    GuestConnKey, GuestFlowOwnerPorts, GuestFlowPhase, GuestLeaseObservation, GuestLeaseView,
    GuestOwnerObservation, HalfCloseAnnounce, HostFlowPhase, IntakePortBlockConflict,
    InterfaceStacking, QuiescenceHold, QuiescenceHolder, RequestRead, StackedLinkKind,
};

/// Opaque allocation-scoped network plan; only control-plane owners construct it.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestNetworkPlan {
    alloc: AllocationId,
    node_prefix: Ipv4Net,
    assignment: GuestTransportAssignment,
    listen_ports: Vec<ListenPort>,
}

impl GuestNetworkPlan {
    /// Allocation owning this plan.
    #[must_use]
    pub fn alloc(&self) -> &AllocationId {
        &self.alloc
    }
    /// Node guest prefix.
    #[must_use]
    pub fn node_prefix(&self) -> Ipv4Net {
        self.node_prefix
    }
    /// The allocation → driver handoff: workload address, gateway, launch
    /// token and the take-once CID claim.
    #[must_use]
    pub fn assignment(&self) -> &GuestTransportAssignment {
        &self.assignment
    }
    /// The allocation's declared TCP listener ports (the PORT-295-C TCP
    /// projection), distinct and in ascending order; a port's position is the
    /// index `guest_intake_host_port` takes (INTAKE-BIND).
    #[must_use]
    pub fn listen_ports(&self) -> &[ListenPort] {
        &self.listen_ports
    }
}

/// Allocation-scoped awaited network effects.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait GuestNetworkProvisioner: Send + Sync {
    /// Registers `plan.alloc`'s CID as Provisioned with the lease's launch
    /// token (`plan.assignment().launch()`, ATTR-TOKEN): its control session
    /// is accepted when its `ControlOpen` carries that token; its flow
    /// requests carrying that token are refused `NotActive`; connections
    /// carrying any other token are closed unattributed. Binds no listener.
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()>;
    /// The only Provisioned → Active edge. Requires the CID's control session
    /// to be live; then marks the CID Active and binds intake listeners for
    /// declared ports the guest reports listening (D23), steering each one
    /// (`GuestPrefixSteering::steer`) after it listens (D8a-LOOKUP).
    /// All or nothing (U-6): `Ok(Activated)` means Active together with a
    /// bound, steered listener for every such port; a bind, listen or steer
    /// failure returns `Err` with the CID still Provisioned and closes every
    /// listener it bound (closing removes its steering entry in the kernel, so
    /// the rollback cannot fail). Serialized with quiescence by the existing
    /// sequencer, and with this CID's `ListenState` and session events by the
    /// per-allocation order (M-7).
    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<ForwardingActivation>;
    /// Marks the CID Retiring, closes its intake listeners (each close removes
    /// its steering entry) and control session, aborts every flow of the CID,
    /// and converges on absence. Returns `Ok` only when no listener, flow or
    /// control session of the CID remains; a failure to abort or remove a flow
    /// returns `Err` and teardown is retried by the existing reclaim path, the
    /// lease staying Retiring (ADR-0133, CleanupPending ADR-0141).
    /// `pool.release` runs only after `Ok`.
    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()>;
}

/// The outcome of one `activate` call that did not fail.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingActivation {
    /// The CID is Active with a bound, steered listener for every declared
    /// port the guest reports listening.
    Activated,
    /// Node forwarding is quiesced; retry under the EXEC claim (ADR-0131).
    QuiescenceLatched,
    /// The guest's control session is not yet live; retry under the EXEC
    /// claim within the same budget as `QuiescenceLatched`.
    ControlSessionPending,
}

/// One source-honest failure from the shared owner's ordered component audit.
#[derive(Debug, thiserror::Error)]
#[error("shared guest-network component {component:?} audit failed")]
pub struct SharedGuestNetworkAuditError {
    pub component: SharedGuestNetworkComponent,
    #[source]
    pub source: GuestNetworkError,
}

/// One owner for both allocation and node-scoped shared-network effects.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait SharedGuestNetworkOwner: GuestNetworkProvisioner + Send + Sync {
    /// Owner placement (D24), forwarder probe, fd budget, and the guest
    /// prefix configuration checks (D8-PREFIX-CFG, D8-PREFIX-VIP): an address
    /// configured on any root-namespace interface inside the guest prefix →
    /// `GuestNetworkError::GuestPrefixOverlapsNodeAddress`; any range of the
    /// configured service VIP range overlapping the guest prefix →
    /// `GuestNetworkError::GuestPrefixOverlapsVipRange` (checked first); and
    /// the intake host-port block meeting `ip_local_port_range` or a
    /// configured wildcard TCP listener port →
    /// `GuestNetworkError::IntakePortBlockConflict` (INTAKE-BIND). Runs before
    /// any boot effect. Changes nothing.
    async fn probe_startup(&self) -> Result<()>;
    /// Verifies no process-scoped owner object (listener, link, map, socket)
    /// survives from a previous run. It does NOT inspect node-global objects
    /// that outlive a process — the shared `local` route and the pinned
    /// guest-prefix steering — which `converge_shared` converges.
    async fn sweep_stale(&self) -> Result<()>;
    /// Loads the forwarder, attaches the egress unframe program to every
    /// interface present (any failure → `GuestNetworkError::UnframeAttach`,
    /// startup refuses), binds the flow and control listeners, converges the
    /// guest-prefix steering (`load`, then `attach`; either failing →
    /// `GuestNetworkError::GuestPrefixSteeringConverge`, the route unwritten)
    /// and converges the shared `local` route. Idempotent: re-running after a
    /// crash at any point reaches the same state. Nothing removes the route
    /// at shutdown. When a runtime recovery calls it as its repair, it
    /// converges only that recovery's component and never writes the
    /// guest-prefix steering program or the prefix route.
    async fn converge_shared(&self) -> Result<()>;
    /// Non-repairing. Reports damage of the process-owned shared components
    /// and of the shared firewall state. Not audited: the guest-prefix
    /// steering program and the prefix route, and the forwarder's links.
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError>;
    /// D8a-HOLD (ADR-0169). Adds `holder` to the node's quiescence holders.
    /// If it is the first holder: refuses new flows with `Quiesced`, aborts
    /// every live flow, and closes every intake listener (each close removes
    /// its steering entry in the kernel). If other holders already hold,
    /// forwarding is already down and only the holder is added. `unconfirmed`
    /// reports per-allocation flow-abort failures at return.
    /// `Err(QuiescenceHolderBusy { holder })` with no change if `holder`
    /// already holds. Classifies per-allocation failures; never returns them
    /// as `Err`.
    async fn quiesce_forwarding(
        &self,
        holder: QuiescenceHolder,
    ) -> Result<(QuiescenceHold, ForwardingQuiescence)>;
    /// D8a-HOLD (ADR-0169). Ends exactly the presented hold. If other holders
    /// remain: returns `StillQuiesced { holders }` and changes nothing else.
    /// If none remains: reopens flow admission and, for each declared port
    /// whose last `ListenState` says listening, binds, listens and steers a
    /// listener; a bind, listen or steer failure is handled as at bring-up.
    /// The supervisor calls it only after its own repair and a clean audit.
    /// Aborted flows are not restored.
    async fn restore_forwarding(&self, hold: QuiescenceHold) -> Result<ForwardingRestore>;
}

/// The per-allocation result of one quiescence pass.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct ForwardingQuiescence {
    /// Each allocation whose flows could not be confirmed aborted, with the
    /// typed cause.
    pub unconfirmed: BTreeMap<AllocationId, GuestNetworkError>,
}

/// The outcome of one `restore_forwarding` call that did not fail.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardingRestore {
    /// No holder remains: admission reopened, listeners brought back.
    Reopened,
    /// Other holders still hold; nothing reopened.
    StillQuiesced { holders: BTreeSet<QuiescenceHolder> },
}

/// The per-allocation result of one audit that found every node-level
/// component healthy.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct SharedGuestNetworkAudit {
    /// Each allocation whose own attachment parts failed the audit, with the
    /// first failing check. Empty means every audited allocation is exact.
    pub damaged: BTreeMap<AllocationId, GuestNetworkError>,
}

/// Guest-network operation carrying the original adapter failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestNetworkOperation {
    PoolAssign,
    StartupProbe,
    SharedAudit,
    CleanupComplement,
    ForwarderLoad,
    ForwarderInventory,
    FlowListenerBind,
    ControlListenerBind,
    ControlSessionAccept,
    IntakeListenerBind,
    IntakeListenerClose,
    IntakeSteer,
    SteeringConverge,
    LocalRouteConverge,
    LocalDeliveryCheck,
    UnframeConverge,
    OwnerPlacementCheck,
    GuestPrefixConfigCheck,
    FlowAccept,
    FlowRequestRead,
    ControlMessageWrite,
    HostPeerConnect,
    GuestInboundConnect,
    PairInstall,
    PairRemove,
    ForwardingActivate,
    ForwardingQuiesce,
    ForwardingRestore,
}

/// Closed semantic startup-probe stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestNetworkProbeStage {
    Classifier,
    OriginalDestination,
    DetachedLinkGuard,
}

/// One resource-family observation; unavailable is never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestNetworkScratchCount {
    Observed(u32),
    Unavailable,
}

/// Complete scratch-resource observation; deliberately has no `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestNetworkScratchComplement {
    pub bridges: GuestNetworkScratchCount,
    pub taps: GuestNetworkScratchCount,
    pub endpoint_maps: GuestNetworkScratchCount,
    pub counter_maps: GuestNetworkScratchCount,
    pub endpoint_entries: GuestNetworkScratchCount,
    pub tcx_programs: GuestNetworkScratchCount,
    pub tcx_links: GuestNetworkScratchCount,
    pub endpoint_map_pins: GuestNetworkScratchCount,
    pub counter_map_pins: GuestNetworkScratchCount,
    pub tcx_link_pins: GuestNetworkScratchCount,
    pub bridge_guard_tables: GuestNetworkScratchCount,
    pub bridge_guard_chains: GuestNetworkScratchCount,
    pub bridge_guard_sets: GuestNetworkScratchCount,
    pub bridge_guard_rules: GuestNetworkScratchCount,
    pub bridge_guard_members: GuestNetworkScratchCount,
}

impl GuestNetworkScratchComplement {
    #[must_use]
    pub const fn is_fully_observed(&self) -> bool {
        matches!(
            self,
            Self {
                bridges: GuestNetworkScratchCount::Observed(_),
                taps: GuestNetworkScratchCount::Observed(_),
                endpoint_maps: GuestNetworkScratchCount::Observed(_),
                counter_maps: GuestNetworkScratchCount::Observed(_),
                endpoint_entries: GuestNetworkScratchCount::Observed(_),
                tcx_programs: GuestNetworkScratchCount::Observed(_),
                tcx_links: GuestNetworkScratchCount::Observed(_),
                endpoint_map_pins: GuestNetworkScratchCount::Observed(_),
                counter_map_pins: GuestNetworkScratchCount::Observed(_),
                tcx_link_pins: GuestNetworkScratchCount::Observed(_),
                bridge_guard_tables: GuestNetworkScratchCount::Observed(_),
                bridge_guard_chains: GuestNetworkScratchCount::Observed(_),
                bridge_guard_sets: GuestNetworkScratchCount::Observed(_),
                bridge_guard_rules: GuestNetworkScratchCount::Observed(_),
                bridge_guard_members: GuestNetworkScratchCount::Observed(_),
            }
        )
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(
            self,
            Self {
                bridges: GuestNetworkScratchCount::Observed(0),
                taps: GuestNetworkScratchCount::Observed(0),
                endpoint_maps: GuestNetworkScratchCount::Observed(0),
                counter_maps: GuestNetworkScratchCount::Observed(0),
                endpoint_entries: GuestNetworkScratchCount::Observed(0),
                tcx_programs: GuestNetworkScratchCount::Observed(0),
                tcx_links: GuestNetworkScratchCount::Observed(0),
                endpoint_map_pins: GuestNetworkScratchCount::Observed(0),
                counter_map_pins: GuestNetworkScratchCount::Observed(0),
                tcx_link_pins: GuestNetworkScratchCount::Observed(0),
                bridge_guard_tables: GuestNetworkScratchCount::Observed(0),
                bridge_guard_chains: GuestNetworkScratchCount::Observed(0),
                bridge_guard_sets: GuestNetworkScratchCount::Observed(0),
                bridge_guard_rules: GuestNetworkScratchCount::Observed(0),
                bridge_guard_members: GuestNetworkScratchCount::Observed(0),
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLinkKind {
    Bridge,
    Tap,
    Tun,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestBpfMapKind {
    Endpoint,
    Counter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestEndpointFact {
    pub source_ip: Ipv4Addr,
    pub source_mac: [u8; 6],
    pub bridge_mac: [u8; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestNetworkFact {
    Tap {
        name: String,
        ifindex: Option<u32>,
        link_kind: GuestLinkKind,
        persistent: bool,
        up: bool,
        owner_uid: Option<u32>,
    },
    Bridge {
        name: String,
        ifindex: Option<u32>,
        link_kind: GuestLinkKind,
        mac: [u8; 6],
        up: bool,
        gateway: Option<Ipv4Net>,
    },
    BridgeLinkIdentity {
        name: String,
        ifindex: Option<u32>,
        link_kind: GuestLinkKind,
    },
    LinkMaster {
        ifindex: u32,
        master_ifindex: Option<u32>,
    },
    LinkUp {
        ifindex: u32,
        up: bool,
    },
    TcxAttachment {
        ifindex: u32,
        program_id: Option<u32>,
        attach_point: Option<TcxAttachPoint>,
    },
    BpfMap {
        kind: GuestBpfMapKind,
        path: PathBuf,
        map_id: Option<u32>,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
    },
    BpfLinkPin {
        path: PathBuf,
        link_id: Option<u32>,
    },
    EndpointMapEntry {
        ifindex: u32,
        value: Option<GuestEndpointFact>,
    },
    BridgeGuard {
        tap: String,
        member: bool,
        rules: Vec<BridgeGuardRuleFact>,
    },
    CleanupComplement {
        taps: u32,
        tcx_links: u32,
        pins: u32,
        endpoint_entries: u32,
        guard_members: u32,
    },
    ScratchCleanupComplement {
        complement: GuestNetworkScratchComplement,
    },
    StartupProbe {
        stage: GuestNetworkProbeStage,
        passed: bool,
    },
    SharedComponent {
        component: SharedGuestNetworkComponent,
        healthy: bool,
    },
    /// The forwarding state of one allocation's CID (§ *Typed error
    /// taxonomy*).
    AllocationForwarding {
        alloc: AllocationId,
        cid: GuestCid,
        pairs: u32,
    },
    /// A managed TAP's host-side MAC, judged by the host-side MAC invariant
    /// (D-295-R21). The expected fact is `address: Unreserved`; the observed
    /// fact is `Reserved` or `Missing`.
    TapHostMac {
        ifindex: u32,
        address: TapHostAddress,
    },
    /// A TAP's ethtool debug message mask: expected is 0, observed is the
    /// live one (the ADR-0130 read-back set).
    TapDebugMsgMask {
        ifindex: u32,
        mask: u32,
    },
}

/// The standing of a managed TAP's host-side MAC under the invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapHostAddress {
    /// A 6-byte address outside the reserved set. It appears only as the
    /// expected fact; an unreserved address is never reported.
    Unreserved,
    /// A reserved address: `GUEST_BRIDGE_MAC`, or the guest MAC of an
    /// allocation the owner holds outside `Condemned`, the TAP's own included.
    /// The address itself names its holder: the bridge, or the guest whose
    /// IPv4 address it encodes.
    Reserved([u8; 6]),
    /// The read-back carried no 6-byte address.
    Missing,
}

/// Source-honest orchestration error for all guest-network effects.
#[derive(Debug, thiserror::Error)]
pub enum GuestNetworkError {
    #[error("guest address pool exhausted below fixed cap: held={held}, capacity={capacity}")]
    PoolExhausted { held: u32, capacity: u32 },
    #[error("guest-network boot requires the EXEC gate to remain BootClosed")]
    ExecGateNotBootClosed,
    #[error("guest network netlink/nft operation {operation:?} failed")]
    Netlink {
        operation: GuestNetworkOperation,
        #[source]
        source: NetlinkError,
    },
    #[error("guest flow operation {operation:?} failed")]
    Flow {
        operation: GuestNetworkOperation,
        #[source]
        source: GuestFlowError,
    },
    #[error("guest flow wire record of operation {operation:?} is malformed")]
    Wire {
        operation: GuestNetworkOperation,
        #[source]
        source: FlowWireError,
    },
    #[error("guest network I/O operation {operation:?} failed")]
    Io {
        operation: GuestNetworkOperation,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "guest network postcondition mismatch after {operation:?}: expected {expected:?}, observed {observed:?}"
    )]
    PostconditionMismatch {
        operation: GuestNetworkOperation,
        expected: GuestNetworkFact,
        observed: Option<GuestNetworkFact>,
    },
    #[error("guest-network scratch cleanup complement is non-empty")]
    ScratchCleanupIncomplete,
    #[error("guest-network startup probe cleanup failed")]
    StartupProbeCleanup {
        primary: Option<Box<GuestNetworkError>>,
        #[source]
        cleanup: Box<GuestNetworkError>,
        observed: GuestNetworkScratchComplement,
    },
    #[error("guest-network admission cap reached: {held} held ({retiring} retiring) of {cap}")]
    AdmissionCapReached { held: u32, retiring: u32, cap: u32 },
    #[error("allocation {alloc} holds a retiring guest-network lease")]
    LeaseRetiring { alloc: AllocationId },
    /// D8-PREFIX-CFG: the node's network configuration places an interface
    /// address inside the guest prefix; checked by `probe_startup` before any
    /// boot effect; startup refuses, naming the address and the interface.
    #[error("interface {interface} carries {address}, inside the guest prefix {prefix}")]
    GuestPrefixOverlapsNodeAddress { prefix: Ipv4Net, interface: InterfaceIndex, address: Ipv4Net },
    /// D8-PREFIX-VIP: a range of the configured service VIP range overlaps
    /// the guest prefix; checked by `probe_startup` before any boot effect;
    /// startup refuses, naming the range.
    #[error("the service VIP range {vip_range} overlaps the guest prefix {prefix}")]
    GuestPrefixOverlapsVipRange { prefix: Ipv4Net, vip_range: Ipv4Net },
    /// The `serve` process's `RLIMIT_NOFILE` is below the owner's budget.
    #[error("file-descriptor budget {required} exceeds RLIMIT_NOFILE {available}")]
    FdBudget { required: u64, available: u64 },
    /// D24: the owner's cgroup is not at or below the `connect4` attach path.
    #[error(
        "the owner cgroup {owner:?} is not at or below the connect4 attach point {attach_point:?}"
    )]
    OwnerCgroupPlacement { owner: PathBuf, attach_point: PathBuf },
    /// D5a-IFACE-BOOT: at boot the egress unframe program could not be
    /// attached to an interface present; startup refuses, naming it.
    #[error("the egress unframe program could not be attached at boot")]
    UnframeAttach {
        #[source]
        source: UnframeAttachError,
    },
    /// D8a-ROUTE, D8a-LOOKUP, D8a-LOAD-SWAP: at boot this binary's steering
    /// program could not be loaded or attached; the route was not written,
    /// the link runs the program an earlier boot attached, or none exists,
    /// and startup refuses.
    #[error("the guest-prefix steering could not be loaded or attached at boot")]
    GuestPrefixSteeringConverge {
        #[source]
        source: GuestSteeringError,
    },
    /// INTAKE-BIND: the intake host-port block meets the host's
    /// `ip_local_port_range` or a TCP port the configuration binds on the
    /// unspecified address; checked by `probe_startup` before any boot
    /// effect; startup refuses, naming the block and the conflict.
    #[error("the intake host-port block {first}..={last} conflicts: {conflict:?}")]
    IntakePortBlockConflict { first: u16, last: u16, conflict: IntakePortBlockConflict },
    /// IFACE-NO-STACK: a root-namespace interface is stacked on another;
    /// checked by `probe_startup` before any boot effect; startup refuses,
    /// naming the interface and how it is stacked.
    #[error("root-namespace interface {interface} is stacked: {}", describe_stacking(*stacking))]
    StackedInterface { interface: InterfaceIndex, stacking: InterfaceStacking },
    /// D8a-CAP: admitting `requested` declared TCP listener ports would take
    /// the node's reservations past the steering capacity.
    #[error(
        "guest intake capacity reached: {reserved} declared ports reserved, {requested} requested, capacity {capacity}"
    )]
    IntakeCapacityReached { reserved: u32, requested: u32, capacity: u32 },
    /// U-6: an activation steer failure (operation `IntakeSteer`).
    #[error("steering the intake listener {intake} of allocation {alloc} failed")]
    IntakeSteer {
        alloc: AllocationId,
        intake: SocketAddrV4,
        #[source]
        source: GuestSteeringError,
    },
    /// D16-CLAIM: every free CID's claim returned `InUse` during one `assign`.
    #[error("every free guest CID ({free}) is held by another vhost-vsock device instance")]
    GuestCidsHeldElsewhere { free: u32 },
    /// D16-CLAIM: a claim failed for a cause other than `InUse`.
    #[error("claiming guest CID {cid} failed")]
    GuestCidClaim {
        cid: GuestCid,
        #[source]
        source: GuestCidClaimError,
    },
    /// D8a-HOLD: a second quiesce by a holder that already holds.
    #[error("{holder:?} already holds forwarding quiescence")]
    QuiescenceHolderBusy { holder: QuiescenceHolder },
}

/// Guest-network result alias.
pub type Result<T, E = GuestNetworkError> = std::result::Result<T, E>;

/// The operator-facing rendering of how an interface is stacked: the link
/// kind through its pinned kernel name (`StackedLinkKind::as_str`).
fn describe_stacking(stacking: InterfaceStacking) -> String {
    match stacking {
        InterfaceStacking::Enslaved { master } => format!("enslaved to interface {master}"),
        InterfaceStacking::Upper { kind, lower } => {
            format!("a {} upper of interface {lower}", kind.as_str())
        }
    }
}

/// The server's process-local guest-address lease owner (D-295-R6). It is
/// intentionally not a repository.
///
/// Doc-hidden public so fixtures outside this crate's source can construct the
/// one pool a server composes (F-02 precedent); its lease operations stay
/// crate-private.
#[doc(hidden)]
#[derive(Clone)]
pub struct GuestAddressPool {
    node_prefix: Ipv4Net,
    gateway: Ipv4Addr,
    #[allow(dead_code, reason = "RED scaffold — the D16-CLAIM walk in `assign` claims through it")]
    cid_claim: Arc<dyn GuestCidClaim>,
    #[allow(dead_code, reason = "RED scaffold — `assign` draws each lease's launch token from it")]
    entropy: Arc<dyn Entropy>,
    held: Arc<parking_lot::Mutex<GuestAddressPoolState>>,
}

impl std::fmt::Debug for GuestAddressPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestAddressPool")
            .field("node_prefix", &self.node_prefix)
            .field("gateway", &self.gateway)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct GuestAddressPoolState {
    plans: BTreeMap<AllocationId, GuestNetworkPlan>,
    leases: BTreeMap<AllocationId, GuestAttachmentLease>,
    /// Retiring entries in `leases`; maintained to keep the admission check O(1).
    retiring: u32,
}

impl GuestAddressPool {
    /// Construct an empty pool over the node guest prefix and its gateway
    /// (the guest's DNS server address), with the CID claim port (D16-CLAIM)
    /// and the entropy port that draws each lease's launch token
    /// (ATTR-TOKEN). Both ports are required: `VhostVsockCidClaim` /
    /// `OsEntropy` in production, `SimGuestCidClaim` / `SimEntropy` under
    /// test.
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        node_prefix: Ipv4Net,
        gateway: Ipv4Addr,
        cid_claim: Arc<dyn GuestCidClaim>,
        entropy: Arc<dyn Entropy>,
    ) -> Self {
        Self {
            node_prefix,
            gateway,
            cid_claim,
            entropy,
            held: Arc::new(parking_lot::Mutex::new(GuestAddressPoolState::default())),
        }
    }

    /// Admit `alloc` with its declared TCP listener ports (the PORT-295-C TCP
    /// projection, distinct). An Admitted lease for this allocation returns
    /// its existing plan; Retiring → `LeaseRetiring`; cap →
    /// `AdmissionCapReached`; intake capacity → `IntakeCapacityReached`; then
    /// free offsets are claimed in next-fit order (D16-CLAIM): `Ok(claim)`
    /// admits the lease with a fresh launch token (ATTR-TOKEN), `InUse` tries
    /// the next free offset, any other failure is `GuestCidClaim`; every free
    /// offset `InUse` → `GuestCidsHeldElsewhere`; no free offset →
    /// `PoolExhausted`.
    pub(crate) fn assign(
        &self,
        alloc: AllocationId,
        _listen_ports: &[ListenPort],
    ) -> Result<GuestNetworkPlan> {
        let state = self.held.lock();
        if let Some(plan) = state.plans.get(&alloc) {
            if state.leases.get(&alloc) == Some(&GuestAttachmentLease::Retiring) {
                let occupancy = guest_pool_occupancy(&state);
                drop(state);
                tracing::info!(
                    name: "guest_network.admission_refused",
                    alloc = %alloc,
                    held = occupancy.held,
                    retiring = occupancy.retiring,
                    cap = overdrive_core::guest_network::MAX_GUEST_NETWORK_ATTACHMENTS,
                    "guest network admission refused"
                );
                return Err(GuestNetworkError::LeaseRetiring { alloc });
            }
            let plan = plan.clone();
            return Ok(plan);
        }

        let occupancy = guest_pool_occupancy(&state);
        if occupancy.held >= overdrive_core::guest_network::MAX_GUEST_NETWORK_ATTACHMENTS {
            drop(state);
            tracing::info!(
                name: "guest_network.admission_refused",
                alloc = %alloc,
                held = occupancy.held,
                retiring = occupancy.retiring,
                cap = overdrive_core::guest_network::MAX_GUEST_NETWORK_ATTACHMENTS,
                "guest network admission refused"
            );
            return Err(GuestNetworkError::AdmissionCapReached {
                held: occupancy.held,
                retiring: occupancy.retiring,
                cap: overdrive_core::guest_network::MAX_GUEST_NETWORK_ATTACHMENTS,
            });
        }
        drop(state);

        // SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 15, attachment cut):
        // the intake-capacity reservation (D8a-CAP), the next-fit CID claim walk
        // (D16-CLAIM) and the launch-token draw (ATTR-TOKEN) are new behaviour.
        panic!("Not yet implemented -- RED scaffold")
    }

    pub(crate) fn release(&self, alloc: &AllocationId) {
        {
            let mut state = self.held.lock();
            let lease = state.leases.remove(alloc);
            if lease == Some(GuestAttachmentLease::Retiring) {
                state.retiring -= 1;
            }
            state.plans.remove(alloc);
        }
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<AllocationId, GuestNetworkPlan> {
        self.held.lock().plans.clone()
    }

    /// Move an Admitted lease to Retiring; `true` only for that transition.
    /// An absent or already-Retiring lease returns `false` with no change, and
    /// Retiring never returns to Admitted. The lease still counts against the
    /// cap until [`Self::release`].
    pub(crate) fn retire(&self, alloc: &AllocationId) -> bool {
        {
            let mut state = self.held.lock();
            match state.leases.get_mut(alloc) {
                Some(lease) if *lease == GuestAttachmentLease::Admitted => {
                    *lease = GuestAttachmentLease::Retiring;
                    state.retiring += 1;
                    true
                }
                Some(_) | None => false,
            }
        }
    }

    /// Occupancy (Admitted plus Retiring held, and the Retiring subset) and the
    /// leases of `allocs`, read in one lock acquisition. Read-only.
    pub(crate) fn observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation {
        let state = self.held.lock();
        let occupancy = guest_pool_occupancy(&state);
        let leases = allocs
            .iter()
            .filter_map(|alloc| state.leases.get(alloc).map(|lease| (alloc.clone(), *lease)))
            .collect();
        drop(state);
        GuestAttachmentObservation { occupancy, leases }
    }
}

fn guest_pool_occupancy(
    state: &GuestAddressPoolState,
) -> overdrive_core::guest_network::GuestAttachmentOccupancy {
    overdrive_core::guest_network::GuestAttachmentOccupancy {
        held: u32::try_from(state.leases.len()).unwrap_or(u32::MAX),
        retiring: state.retiring,
    }
}

/// The production [`GuestAttachmentView`] the reconciler runtime lends each
/// hydration context (D-295-R8, ADR-0134). Its `observe` delegates to the
/// server's pool observation.
#[derive(Debug, Clone)]
pub(crate) struct GuestPoolAttachmentView {
    pool: Arc<GuestAddressPool>,
}

impl GuestPoolAttachmentView {
    pub(crate) fn new(pool: Arc<GuestAddressPool>) -> Self {
        Self { pool }
    }
}

impl GuestAttachmentView for GuestPoolAttachmentView {
    fn observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation {
        self.pool.observe(allocs)
    }
}

/// The host guest-flow owner (DG-3): one owner of the allocation-scoped and
/// node-shared guest-network effects, reaching every kernel, socket,
/// forwarder, steering and route effect through the crate-private
/// [`flow_io::GuestFlowIo`] seam.
pub struct HostSharedGuestNetworkOwner {
    // RED scaffold: read by the guest-flow owner the attachment cut lands.
    #[allow(dead_code, reason = "RED scaffold — the guest-flow owner reads its ports")]
    ports: GuestFlowOwnerPorts,
    #[allow(dead_code, reason = "RED scaffold — the guest-flow owner reaches every effect through it")]
    io: Arc<dyn flow_io::GuestFlowIo>,
}

impl std::fmt::Debug for HostSharedGuestNetworkOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostSharedGuestNetworkOwner").finish_non_exhaustive()
    }
}

impl HostSharedGuestNetworkOwner {
    /// Production: composes `HostGuestFlowIo`. Infallible; touches no kernel
    /// state (the effects start at `probe_startup`).
    #[must_use]
    pub fn new(ports: GuestFlowOwnerPorts) -> Self {
        Self { ports, io: Arc::new(flow_io::HostGuestFlowIo::new()) }
    }

    /// In-crate tests and conformance drivers: the same owner over `io`.
    #[cfg(test)]
    pub(crate) fn with_flow_io(ports: GuestFlowOwnerPorts, io: Arc<dyn flow_io::GuestFlowIo>) -> Self {
        Self { ports, io }
    }
}

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, DG-2 rows 7, 8, 9):
// the guest-flow owner's provisioning, activation and teardown over `GuestFlowIo`
// are new behaviour; DELIVER lands them in the attachment cut.
#[async_trait::async_trait]
impl GuestNetworkProvisioner for HostSharedGuestNetworkOwner {
    async fn provision(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn activate(&self, _plan: &GuestNetworkPlan) -> Result<ForwardingActivation> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn teardown(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, DG-2 rows 8, 9):
// boot, audit and holder-based quiescence of the guest-flow owner are new
// behaviour; DELIVER lands them in the attachment cut.
#[async_trait::async_trait]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn sweep_stale(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn converge_shared(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn quiesce_forwarding(
        &self,
        _holder: QuiescenceHolder,
    ) -> Result<(QuiescenceHold, ForwardingQuiescence)> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn restore_forwarding(&self, _hold: QuiescenceHold) -> Result<ForwardingRestore> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "source-local lease-state tables: each expect() names the fixture precondition it establishes"
)]
mod pool_acceptance {
    //! The lease-state model every pool body is compared against (D-295-R6,
    //! R7, R8): `absent -> Admitted -> Retiring -> absent`. An Admitted replay
    //! returns the plan the first `assign` returned; a Retiring replay is
    //! refused as retiring; both states count against the fixed cap until
    //! `release`; `observe` and `snapshot` are read-only.
    //!
    //! Leases are always produced by the pool's own `assign` / `retire` /
    //! `release`; no body writes the pool's private state. The pool claims
    //! CIDs through `SimGuestCidClaim` and draws launch tokens from
    //! `SimEntropy` (D16-CLAIM, ATTR-TOKEN).

    use super::*;
    use overdrive_core::guest_network::{GuestAttachmentOccupancy, MAX_GUEST_NETWORK_ATTACHMENTS};
    use overdrive_core::traits::GuestAttachmentLease;
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim;
    use proptest::prelude::*;
    use std::sync::LazyLock;

    /// The fixed admission cap as a lease count.
    const CAP: usize = MAX_GUEST_NETWORK_ATTACHMENTS as usize;

    /// Allocation-id key space of the generated operation sequences. Small, so
    /// sequences replay, retire, and release the same allocations often.
    const KEYS: u16 = 48;

    /// Ids for the at-cap body: `CAP` leases plus one refused newcomer.
    static CAP_IDS: LazyLock<Vec<AllocationId>> = LazyLock::new(|| {
        (0..=CAP)
            .map(|index| {
                AllocationId::new(&format!("nd295-cap-{index:05}")).expect("allocation id")
            })
            .collect()
    });

    fn pool_over(prefix: &str) -> GuestAddressPool {
        GuestAddressPool::new(
            prefix.parse().expect("node prefix"),
            Ipv4Addr::new(100, 95, 0, 1),
            Arc::new(SimGuestCidClaim::new()),
            Arc::new(SimEntropy::new(0x295)),
        )
    }

    fn pool() -> GuestAddressPool {
        pool_over("100.95.0.0/16")
    }

    fn alloc(key: u16) -> AllocationId {
        AllocationId::new(&format!("nd295-{key:04x}")).expect("generated allocation id")
    }

    /// One generated pool operation.
    #[derive(Debug, Clone, Copy)]
    enum LeaseOp {
        Assign(u16),
        Retire(u16),
        Release(u16),
    }

    fn lease_op() -> impl Strategy<Value = LeaseOp> {
        prop_oneof![
            3 => (0..KEYS).prop_map(LeaseOp::Assign),
            2 => (0..KEYS).prop_map(LeaseOp::Retire),
            1 => (0..KEYS).prop_map(LeaseOp::Release),
        ]
    }

    /// The independent lease model: each held allocation's lease, and the
    /// plan its first `assign` returned (an Admitted replay must equal it).
    #[derive(Debug, Clone, Default)]
    struct LeaseModel {
        leases: BTreeMap<AllocationId, (Option<GuestNetworkPlan>, GuestAttachmentLease)>,
    }

    impl LeaseModel {
        fn observation(&self, requested: &[AllocationId]) -> GuestAttachmentObservation {
            let held = u32::try_from(self.leases.len()).expect("small model");
            let retiring = u32::try_from(
                self.leases
                    .values()
                    .filter(|(_, lease)| *lease == GuestAttachmentLease::Retiring)
                    .count(),
            )
            .expect("small model");
            GuestAttachmentObservation {
                occupancy: GuestAttachmentOccupancy { held, retiring },
                leases: requested
                    .iter()
                    .filter_map(|alloc| {
                        self.leases.get(alloc).map(|(_, lease)| (alloc.clone(), *lease))
                    })
                    .collect(),
            }
        }
    }

    /// Drive `op` through the pool's own operations and compare its outcome
    /// with the model's.
    fn apply_and_compare(
        pool: &GuestAddressPool,
        model: &mut LeaseModel,
        op: LeaseOp,
    ) -> std::result::Result<(), TestCaseError> {
        match op {
            LeaseOp::Assign(key) => {
                let alloc = alloc(key);
                let outcome = pool.assign(alloc.clone(), &[]);
                match model.leases.get(&alloc) {
                    Some((first, GuestAttachmentLease::Admitted)) => {
                        prop_assert!(
                            matches!(&outcome, Ok(plan) if Some(plan) == first.as_ref()),
                            "an Admitted replay of {key} returned {outcome:?}"
                        );
                    }
                    Some((_, GuestAttachmentLease::Retiring)) => {
                        prop_assert!(
                            matches!(&outcome, Err(GuestNetworkError::LeaseRetiring { alloc: refused }) if *refused == alloc),
                            "a Retiring replay of {key} returned {outcome:?}"
                        );
                    }
                    None => {
                        let plan = outcome.expect("below-cap assignment");
                        prop_assert_eq!(plan.alloc(), &alloc);
                        model.leases.insert(alloc, (Some(plan), GuestAttachmentLease::Admitted));
                    }
                }
            }
            LeaseOp::Retire(key) => {
                let alloc = alloc(key);
                let expected = match model.leases.get_mut(&alloc) {
                    Some((_, lease @ GuestAttachmentLease::Admitted)) => {
                        *lease = GuestAttachmentLease::Retiring;
                        true
                    }
                    Some((_, GuestAttachmentLease::Retiring)) | None => false,
                };
                prop_assert_eq!(pool.retire(&alloc), expected, "retire({}) transition", key);
            }
            LeaseOp::Release(key) => {
                model.leases.remove(&alloc(key));
                pool.release(&alloc(key));
                // A release of an absent lease is a no-op.
                pool.release(&alloc(key));
            }
        }
        Ok(())
    }

    proptest! {
        /// Outcome anchor: OUT-ND295-SHARED-SWITCH
        /// S-ND295-04 — Lease replay, retirement, and release change only the named allocation
        /// CONTRACT_SHAPE: bounded-change.
        ///
        /// Retirement happens once and never returns a lease to Admitted: a
        /// repeated `retire` and a refused replay leave it Retiring, it still
        /// counts in `held` and `snapshot`, and only `release` frees it, after
        /// which a fresh `assign` of the same allocation admits it again.
        #[test]
        fn retirement_is_monotonic_and_a_retiring_lease_still_counts(
            admitted in 1_u16..24,
            retire_mask in any::<u32>(),
            repeats in 1_usize..4,
        ) {
            let pool = pool();
            let held = (0..admitted).map(alloc).collect::<Vec<_>>();
            let plans = held
                .iter()
                .map(|alloc| pool.assign(alloc.clone(), &[]).expect("below-cap assignment"))
                .collect::<Vec<_>>();
            let retiring = held
                .iter()
                .enumerate()
                .filter(|(index, _)| retire_mask & (1 << index) != 0)
                .map(|(_, alloc)| alloc.clone())
                .collect::<BTreeSet<_>>();

            for alloc in &retiring {
                prop_assert!(pool.retire(alloc), "Admitted -> Retiring reports the transition");
                for _ in 0..repeats {
                    prop_assert!(!pool.retire(alloc), "a Retiring lease never transitions again");
                }
            }
            prop_assert!(!pool.retire(&alloc(KEYS + 1)), "an absent lease has no transition");

            let before = pool.snapshot();
            let observed_before = pool.observe(&held);
            for alloc in &retiring {
                let refusal = pool.assign(alloc.clone(), &[]);
                prop_assert!(
                    matches!(&refusal, Err(GuestNetworkError::LeaseRetiring { alloc: refused }) if refused == alloc),
                    "a Retiring replay is refused as retiring, got {refusal:?}"
                );
            }
            prop_assert_eq!(&pool.snapshot(), &before, "refused replays change nothing");
            prop_assert_eq!(&pool.observe(&held), &observed_before);

            let expected_leases = held
                .iter()
                .map(|alloc| {
                    let lease = if retiring.contains(alloc) {
                        GuestAttachmentLease::Retiring
                    } else {
                        GuestAttachmentLease::Admitted
                    };
                    (alloc.clone(), lease)
                })
                .collect::<BTreeMap<_, _>>();
            prop_assert_eq!(
                observed_before,
                GuestAttachmentObservation {
                    occupancy: GuestAttachmentOccupancy {
                        held: u32::from(admitted),
                        retiring: u32::try_from(retiring.len()).expect("small set"),
                    },
                    leases: expected_leases,
                }
            );
            prop_assert_eq!(
                before,
                held.iter().cloned().zip(plans.iter().cloned()).collect::<BTreeMap<_, _>>(),
                "snapshot holds both states with their original plans"
            );

            if let Some(released) = retiring.iter().next().cloned() {
                pool.release(&released);
                let after_release = pool.observe(&held);
                prop_assert_eq!(after_release.occupancy.held, u32::from(admitted) - 1);
                prop_assert!(!after_release.leases.contains_key(&released));
                prop_assert!(!pool.snapshot().contains_key(&released));
                pool.assign(released.clone(), &[]).expect("a released id takes the normal path");
                prop_assert_eq!(
                    pool.observe(std::slice::from_ref(&released)).leases.get(&released).copied(),
                    Some(GuestAttachmentLease::Admitted)
                );
            }
        }

        /// Outcome anchor: OUT-ND295-SHARED-SWITCH
        /// S-ND295-05A — Admission refuses at the cap over held leases, one pool per server
        /// CONTRACT_SHAPE: bounded-change.
        ///
        /// Held leases are counted in both states. At 16,383 held, with any
        /// mix of Retiring among them, the next allocation is admitted; at
        /// 16,384 held (again with every mix) a new allocation is refused with
        /// `AdmissionCapReached { held, retiring, cap: 16_384 }` and nothing
        /// changes, while an Admitted replay still returns its plan and a
        /// Retiring replay is refused as retiring.
        #[test]
        fn admission_refuses_at_the_cap_over_held_leases_for_every_retiring_mix(
            retiring in prop_oneof![Just(0_usize), Just(CAP - 1), 1_usize..CAP - 1],
            offset in 0_usize..CAP - 1,
            retire_the_last_admitted in any::<bool>(),
        ) {
            let ids = &*CAP_IDS;
            let pool = pool();
            let below_cap = CAP - 1;
            for id in &ids[..below_cap] {
                pool.assign(id.clone(), &[]).expect("below-cap assignment");
            }
            let retired = (0..retiring)
                .map(|step| (offset + step) % below_cap)
                .collect::<BTreeSet<_>>();
            for index in &retired {
                prop_assert!(pool.retire(&ids[*index]));
            }
            let at_16383 = pool.observe(&[]);
            prop_assert_eq!(
                at_16383.occupancy,
                GuestAttachmentOccupancy {
                    held: MAX_GUEST_NETWORK_ATTACHMENTS - 1,
                    retiring: u32::try_from(retiring).expect("below cap"),
                }
            );

            let last = pool.assign(ids[below_cap].clone(), &[]).expect("16,383 held admits one more");
            prop_assert_eq!(last.alloc(), &ids[below_cap]);
            let mut retiring_at_cap = retiring;
            if retire_the_last_admitted {
                prop_assert!(pool.retire(&ids[below_cap]));
                retiring_at_cap += 1;
            }

            let before = pool.snapshot();
            let observed_before = pool.observe(&[ids[CAP].clone()]);
            prop_assert_eq!(
                observed_before.occupancy,
                GuestAttachmentOccupancy {
                    held: MAX_GUEST_NETWORK_ATTACHMENTS,
                    retiring: u32::try_from(retiring_at_cap).expect("at cap"),
                }
            );
            let refusal = pool.assign(ids[CAP].clone(), &[]);
            prop_assert!(
                matches!(
                    refusal,
                    Err(GuestNetworkError::AdmissionCapReached { held, retiring, cap })
                        if held == MAX_GUEST_NETWORK_ATTACHMENTS
                            && usize::try_from(retiring).expect("u32 fits") == retiring_at_cap
                            && cap == 16_384
                ),
                "at 16,384 held a new allocation is refused with its counts, got {refusal:?}"
            );

            let admitted_replay = (0..CAP)
                .find(|index| {
                    !(retired.contains(index) || (*index == below_cap && retire_the_last_admitted))
                });
            if let Some(index) = admitted_replay {
                let replay = pool.assign(ids[index].clone(), &[]).expect("an Admitted replay at the cap");
                prop_assert_eq!(Some(&replay), before.get(&ids[index]), "equal replay");
            }
            let retiring_replay = retired
                .iter()
                .next()
                .copied()
                .or_else(|| retire_the_last_admitted.then_some(below_cap));
            if let Some(index) = retiring_replay {
                let replay = pool.assign(ids[index].clone(), &[]);
                prop_assert!(
                    matches!(&replay, Err(GuestNetworkError::LeaseRetiring { alloc }) if *alloc == ids[index]),
                    "a Retiring replay at the cap is refused as retiring, got {replay:?}"
                );
            }

            prop_assert_eq!(pool.snapshot(), before, "no refusal or replay changes the lease map");
            prop_assert_eq!(pool.observe(&[ids[CAP].clone()]), observed_before);
        }

        /// Outcome anchor: OUT-ND295-SHARED-SWITCH
        /// S-ND295-05B — Placement reads the node's held attachments, not the workload's rows
        /// CONTRACT_SHAPE: bounded-change.
        ///
        /// Over any lease map the pool's own operations produce and any
        /// request set, `observe` returns exactly the whole map's held and
        /// retiring counts and the lease of each requested allocation that
        /// holds one, and it changes nothing: `snapshot` is equal before and
        /// after and a second read is equal.
        #[test]
        fn observe_reads_occupancy_and_requested_leases_in_one_snapshot_and_changes_nothing(
            operations in prop::collection::vec(lease_op(), 0..128),
            requested in prop::collection::vec(0_u16..KEYS + 16, 0..24),
        ) {
            let pool = pool();
            let mut model = LeaseModel::default();
            for op in operations {
                apply_and_compare(&pool, &mut model, op)?;
            }
            let requested = requested.into_iter().map(alloc).collect::<Vec<_>>();

            let before = pool.snapshot();
            let observed = pool.observe(&requested);
            prop_assert_eq!(&observed, &model.observation(&requested));
            prop_assert_eq!(pool.snapshot(), before, "observe is read-only");
            prop_assert_eq!(pool.observe(&requested), observed, "a repeated read is equal");
        }
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn below_cap_pool_exhaustion_is_typed_drift_and_preserves_state() {
        let pool = pool_over("100.95.0.0/30");
        pool.assign(AllocationId::new("nd295-below-cap-first").expect("allocation id"), &[])
            .expect("the sole non-reserved offset is initially free");
        let before = pool.snapshot();
        assert!(before.len() < 16_384);

        let error = pool
            .assign(AllocationId::new("nd295-below-cap-overflow").expect("allocation id"), &[])
            .expect_err("the small prefix is exhausted below the fixed admission cap");
        assert!(matches!(error, GuestNetworkError::PoolExhausted { held: 1, capacity: 1 }));
        assert_eq!(pool.snapshot(), before, "typed exhaustion does not mutate or reuse state");
    }
}
