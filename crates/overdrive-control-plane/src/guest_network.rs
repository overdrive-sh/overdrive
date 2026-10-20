//! Shared guest-network application contract (GH #295).
//!
//! The host guest-flow owner, the guest address pool and their accepted
//! executable specifications live at this application boundary.
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Owner and provisioner", § "[REF] Typed error taxonomy and who
//! branches", § DG-2 rows 1–3 and 7–9, § DG-3.

#![expect(
    clippy::use_self,
    reason = "exact accepted API scaffold precedes implementation and names GuestNetworkError explicitly"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::dial::GuestStreamDial;
use overdrive_core::guest_transport::wire::FlowWireError;
use overdrive_core::guest_transport::{GuestCid, GuestFlowPorts, GuestTransportAssignment};
use overdrive_core::id::AllocationId;
use overdrive_core::net::{InterfaceIndex, ListenPort};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::{
    GuestAttachmentLease, GuestAttachmentObservation, GuestAttachmentView,
};
use overdrive_netlink::NetlinkError;

use overdrive_core::guest_transport::forwarder::ForwarderError;
pub use overdrive_dataplane::guest_tcx::TcxAttachPoint;
pub use overdrive_netlink::nft::bridge::{
    BridgeGuardRuleExpression, BridgeGuardRuleFact, BridgeGuardRuleIdentity, BridgeGuardRuleProgram,
};

mod dial;
pub(crate) mod flow_io;
mod flow_owner;

/// Source-local pool acceptance bodies (CID-NOCLAIM, ATTR-TOKEN, G-V0).
#[cfg(test)]
mod pool_acceptance;

/// Source-local guest-prefix boot checks (D8-PREFIX-CFG/VIP, D24, fd budget).
#[cfg(test)]
mod prefix_boot_acceptance;

/// Source-local acceptance bodies over the production host guest-flow owner
/// (`with_flow_io` over `SimGuestFlowIo` and its `SimFlowForwarder` arbiter):
/// admission and attribution, flow ends from end records (EVMOD-REC), the
/// dial and its stream (INB-1, EVMOD-STREAM-END / -ABORT), datagram
/// associations (EVMOD-DGRAM), policy, quota, the closed error taxonomy and
/// the gateway route (GW-LOCAL).
#[cfg(test)]
mod owner_acceptance;

/// ITF decoding shared by every quint-connect driver of this crate (QPROJ-1,
/// D-UP-1): qualified-name variable scopes and value decoders.
#[cfg(all(test, feature = "integration-tests"))]
pub(crate) mod quint_itf;

#[cfg(all(test, feature = "integration-tests"))]
mod quint_cid_lease;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_flow_quota;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_fwd_pair;
/// Quint conformance drivers of the host guest-flow owner (QPROJ-1):
/// source-local, replaying `specs/quint/guest-flow-owner/*.qnt` (referenced in
/// place) on each module's recording main against the production owner over
/// `SimGuestFlowIo` and `SimFlowForwarder`.
#[cfg(all(test, feature = "integration-tests"))]
mod quint_owner_flows;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_udp_slots;

pub use flow_owner::{
    AssociationObservation, AssociationPhase, CidObservation, CidPhase, DialObservation, DialPhase,
    FlowCloseCause, ForwarderInventoryRead, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, GuestAdmission,
    GuestConnKey, GuestFlowOwnerPorts, GuestFlowPhase, GuestLeaseObservation, GuestLeaseView,
    GuestOwnerObservation, QuiescenceHold, QuiescenceLedger, QuiescenceRestoreError, RequestRead,
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
    /// The allocation → driver handoff: workload address, gateway, CID
    /// (`GUEST_CID_BASE + lease offset`) and launch token.
    #[must_use]
    pub fn assignment(&self) -> &GuestTransportAssignment {
        &self.assignment
    }
    /// The allocation's declared TCP listener ports (the PORT-295-C TCP
    /// projection), distinct and in ascending order: the ports
    /// `GuestStreamDial::dial` serves for the CID (INB-1).
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
    /// carrying any other token are closed unattributed. Binds nothing.
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()>;
    /// The only Provisioned → Active edge. Requires the CID's control
    /// session to be live; then marks the CID Active, after which
    /// guest-opened flows are admitted and `GuestStreamDial::dial` serves the
    /// CID's declared ports. Binds nothing. Idempotent on an Active CID
    /// (`Activated`). `Err(PostconditionMismatch { operation:
    /// ForwardingActivate, .. })` when the CID is unknown or Retiring.
    /// Serialized with quiescence by the existing sequencer, and with this
    /// CID's session events and dial admissions by the per-allocation order
    /// (M-7).
    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<ForwardingActivation>;
    /// Marks the CID Retiring, closes its control session, aborts every flow
    /// of the CID — guest-opened flows, datagram associations, in-flight
    /// dials and open `GuestStream`s — and converges on absence. Returns `Ok`
    /// only when no flow or control session of the CID remains; a failure to
    /// abort or remove a flow returns `Err` and teardown is retried by the
    /// existing reclaim path, the lease staying Retiring (ADR-0133,
    /// CleanupPending ADR-0141). `pool.release` runs only after `Ok` and
    /// after the VMM has exited.
    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()>;
}

/// The outcome of one `activate` call that did not fail.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingActivation {
    /// The CID is Active.
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

/// One owner for both allocation and node-scoped shared-network effects,
/// and the production `GuestStreamDial` (INB-1).
#[doc(hidden)]
#[async_trait::async_trait]
pub trait SharedGuestNetworkOwner: GuestNetworkProvisioner + GuestStreamDial {
    /// Owner placement (D24), forwarder probe, fd budget, and the guest
    /// prefix configuration checks (D8-PREFIX-CFG, D8-PREFIX-VIP): an address
    /// configured on any root-namespace interface inside the guest prefix →
    /// `GuestNetworkError::GuestPrefixOverlapsNodeAddress`; any range of the
    /// configured service VIP range overlapping the guest prefix →
    /// `GuestNetworkError::GuestPrefixOverlapsVipRange` (checked first, so a
    /// node that would fail both names the VIP range). Runs before any boot
    /// effect. Changes nothing.
    async fn probe_startup(&self) -> Result<()>;
    /// Verifies no process-scoped owner object (listener, link, map, socket)
    /// survives from a previous run. It does not inspect the gateway route
    /// (GW-LOCAL), a node-global object `converge_shared` converges (H-4).
    async fn sweep_stale(&self) -> Result<()>;
    /// At boot: opens and probes the forwarder context
    /// (`KernelFlowForwarder::open`, `probe`; EVMOD-CTL), binds the flow and
    /// control listeners, then converges the gateway route (GW-LOCAL): keep
    /// an identical Overdrive-tagged `local <gateway>/32 dev lo` route,
    /// replace a tagged one whose attributes differ, add a missing one. No
    /// other route covers the guest prefix and no other address of it is
    /// made local. Idempotent: re-running after a crash at any point reaches
    /// the same state. Nothing removes the route. When a runtime recovery
    /// calls it as its repair, it converges only that recovery's component
    /// and never writes the route; for the `Forwarder` component it closes
    /// the old context before it opens and probes the new one (EVMOD-CTL).
    async fn converge_shared(&self) -> Result<()>;
    /// PORTS-CONFIG (ADR-0218). `None` until this owner's first
    /// `converge_shared` returns `Ok`; from then on `Some` of the ports its
    /// stream and control listeners bound, the same value for the rest of
    /// the process (a listener repair keeps the held socket, so its port
    /// never changes). Reads only.
    fn flow_ports(&self) -> Option<GuestFlowPorts>;
    /// Non-repairing. Reports damage of the process-owned shared components
    /// and of the shared firewall state; the `Forwarder` component is
    /// damaged when its context fails `Io` or its `inventory()` disagrees
    /// with the owner's flow table. Not audited: the gateway route (only
    /// boot writes it).
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError>;
    /// QHOLD (ADR-0169). Increments the node's quiescence hold count and
    /// returns the new hold. The hold that takes the count from 0 to 1
    /// latches admission `Quiesced` (new flows refused `Quiesced`, dials
    /// answered `GuestDialError::Quiesced`) and aborts every live flow on
    /// the node — guest-opened flows, datagram associations, in-flight dials
    /// and open `GuestStream`s; a later hold only increments the count.
    /// `unconfirmed` reports per-allocation flow-abort failures at return.
    /// Infallible: per-allocation failures are classified, never returned.
    /// The hold is issued by this owner's own `QuiescenceLedger` (D-UP-4).
    async fn quiesce(&self) -> (QuiescenceHold, ForwardingQuiescence);
    /// QHOLD. Redeems `hold` at this owner's ledger and decrements the count.
    /// At zero: clears the latch (admission and dials reopen) → `Reopened`.
    /// Above zero: changes nothing else → `StillQuiesced`. Aborted flows are
    /// not restored. The caller (the supervisor) calls it only after its own
    /// repair and a clean audit. `Err(ForeignHold)` (D-UP-4, PD-1): `hold`
    /// was issued by another ledger; the count, the latch and this owner's
    /// leaked count are unchanged (fail-closed), and the foreign hold is
    /// dropped unredeemed and counts as leaked at the ledger that issued it.
    /// Never a panic.
    async fn restore(
        &self,
        hold: QuiescenceHold,
    ) -> std::result::Result<ForwardingRestore, QuiescenceRestoreError>;
}

/// The per-allocation result of one quiescence pass.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct ForwardingQuiescence {
    /// Each allocation whose flows could not be confirmed aborted, with the
    /// typed cause.
    pub unconfirmed: BTreeMap<AllocationId, GuestNetworkError>,
}

/// The outcome of one `restore` call.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingRestore {
    /// The count reached zero: admission and dials reopened.
    Reopened,
    /// Other holds remain; nothing reopened.
    StillQuiesced,
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

/// Guest-network operation carrying the original adapter failure (§ *Typed
/// error taxonomy*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestNetworkOperation {
    PoolAssign,
    StartupProbe,
    SharedAudit,
    CleanupComplement,
    ForwarderOpen,
    ForwarderInventory,
    FlowListenerBind,
    ControlListenerBind,
    ControlSessionAccept,
    /// The gateway route (GW-LOCAL).
    LocalRouteConverge,
    LocalDeliveryCheck,
    OwnerPlacementCheck,
    GuestPrefixConfigCheck,
    FlowAccept,
    FlowRequestRead,
    ControlMessageWrite,
    HostPeerConnect,
    GuestInboundConnect,
    PairAttach,
    PairAbort,
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

/// Source-honest orchestration error for all guest-network effects (§ *Typed
/// error taxonomy*, ADR-0122). Closed: no other variant escapes the owner or
/// the pool (C6c).
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
    /// The forwarder's errors (§ *Typed error taxonomy*): one `#[from]`
    /// variant, never flattened.
    #[error("guest flow forwarder operation failed")]
    Forwarder {
        #[from]
        source: ForwarderError,
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
}

/// Guest-network result alias.
pub type Result<T, E = GuestNetworkError> = std::result::Result<T, E>;

/// The server's process-local guest-address lease owner (D-295-R6). It is
/// intentionally not a repository. CID-NOCLAIM: the lease alone gives CID
/// uniqueness within one `serve` process (CID = `GUEST_CID_BASE` + lease
/// offset); no kernel claim is taken.
///
/// Doc-hidden public so fixtures outside this crate's source can construct the
/// one pool a server composes (F-02 precedent); its lease operations stay
/// crate-private.
#[doc(hidden)]
#[derive(Clone)]
pub struct GuestAddressPool {
    node_prefix: Ipv4Net,
    gateway: Ipv4Addr,
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
    /// (the guest's DNS server address), with the entropy port that draws
    /// each lease's launch token (ATTR-TOKEN). The port is required:
    /// `OsEntropy` in production, `SimEntropy` under test (DG-2 row 1).
    #[doc(hidden)]
    #[must_use]
    pub fn new(node_prefix: Ipv4Net, gateway: Ipv4Addr, entropy: Arc<dyn Entropy>) -> Self {
        Self {
            node_prefix,
            gateway,
            entropy,
            held: Arc::new(parking_lot::Mutex::new(GuestAddressPoolState::default())),
        }
    }

    /// Admit `alloc` with its declared TCP listener ports (the PORT-295-C TCP
    /// projection, distinct); the returned plan's `listen_ports()` is that
    /// set. Order, under one lock acquisition: an Admitted lease for this
    /// allocation returns its existing plan (same CID, same token); Retiring
    /// → `LeaseRetiring`; cap → `AdmissionCapReached`; then the next free
    /// offset in next-fit order from the cursor is Admitted, the cursor
    /// advances past it, and the plan's `GuestTransportAssignment` carries
    /// `cid = GUEST_CID_BASE + offset` and a token drawn by
    /// `LaunchToken::generate(entropy)` in the same lock acquisition. No free
    /// offset → `PoolExhausted`. No kernel call.
    pub(crate) fn assign(
        &self,
        alloc: AllocationId,
        _listen_ports: &[ListenPort],
        _flow_ports: GuestFlowPorts,
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

        // SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment-cut step, DG-2 row 2):
        // the next-fit offset walk (M-9), the CID derivation (CID-NOCLAIM) and the
        // launch-token draw (ATTR-TOKEN) are new behaviour.
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Returns the lease's offset to the free set. Its caller invokes it only
    /// after the allocation's VMM has been reaped (release-last).
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
/// node-shared guest-network effects and the production `GuestStreamDial`,
/// reaching every kernel, socket, forwarder and route effect through the
/// crate-private [`flow_io::GuestFlowIo`] seam. The forwarder context is held
/// by the seam on the owner's behalf (EVMOD-CTL: one context per owner, held
/// by value, replaced only by the `Forwarder` recovery).
pub struct HostSharedGuestNetworkOwner {
    // RED scaffold: read by the guest-flow owner the attachment cut lands.
    #[allow(dead_code, reason = "RED scaffold — the guest-flow owner reads its ports")]
    ports: GuestFlowOwnerPorts,
    #[allow(
        dead_code,
        reason = "RED scaffold — the guest-flow owner reaches every effect through it"
    )]
    io: Arc<dyn flow_io::GuestFlowIo>,
    /// QHOLD (D-UP-4): this owner's one ledger; it issues and redeems the
    /// owner's holds and counts those dropped unredeemed.
    #[allow(
        dead_code,
        reason = "RED scaffold — `quiesce` / `restore` use it at the attachment cut"
    )]
    ledger: QuiescenceLedger,
}

impl std::fmt::Debug for HostSharedGuestNetworkOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostSharedGuestNetworkOwner").finish_non_exhaustive()
    }
}

impl HostSharedGuestNetworkOwner {
    /// Production: `with_flow_io(ports, HostGuestFlowIo)`. Infallible;
    /// touches no kernel state (the effects start at `probe_startup`).
    #[must_use]
    pub fn new(ports: GuestFlowOwnerPorts) -> Self {
        Self::with_flow_io(ports, Arc::new(flow_io::HostGuestFlowIo::new()))
    }

    /// The same owner over `io`. Compiled in every build (D-UP-16): `new`
    /// and the in-crate composition call it; in-crate tests and conformance
    /// drivers pass `SimGuestFlowIo`.
    pub(crate) fn with_flow_io(
        ports: GuestFlowOwnerPorts,
        io: Arc<dyn flow_io::GuestFlowIo>,
    ) -> Self {
        Self { ports, io, ledger: QuiescenceLedger::new() }
    }
}

/// Phase 1 of the host guest-flow owner's two-phase boot (D-8, § *Composition*
/// step 1): every `probe_startup` check over one `io.host_facts()` snapshot —
/// `GuestPrefixOverlapsVipRange` first, then `GuestPrefixOverlapsNodeAddress`,
/// `OwnerCgroupPlacement`, `FdBudget` — then `io.probe_forwarder()`
/// (`GuestNetworkError::Forwarder { source }`). Runs before the owner exists
/// and before any boot effect, and leaves nothing behind: the probe's
/// forwarder context is released before return, on `Ok` and on `Err`.
/// [`HostSharedGuestNetworkOwner::probe_startup`] is this function over the
/// owner's own seam, `vip_range` and `node_prefix`, so the two cannot
/// disagree.
// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut; GW-LOCAL
// step 13): the four startup checks and the forwarder probe are new behaviour.
// Like the owner's boot methods below, the body returns `Ok(())` with no effect
// until that step, so `run_server` boots; the refusals are pinned by bodies
// pending the attachment-cut step and step 13, which fail on their assertions.
#[allow(
    clippy::unused_async,
    reason = "RED scaffold body; the attachment-cut step awaits the seam"
)]
pub(crate) async fn probe_guest_flow_startup(
    io: &dyn flow_io::GuestFlowIo,
    vip_range: &overdrive_dataplane::allocators::VipRange,
    node_prefix: Ipv4Net,
) -> Result<()> {
    let _ = (io, vip_range, node_prefix);
    Ok(())
}

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, DG-2 rows 7, 9):
// the guest-flow owner's provisioning, activation and teardown over `GuestFlowIo`
// are new behaviour; DELIVER lands them in the attachment-cut step (7).
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

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, DG-2 rows 8, 9;
// GW-LOCAL step 13): boot, audit and counted quiescence (QHOLD) of the
// guest-flow owner are new behaviour; DELIVER lands them in the attachment-cut step.
//
// The three boot methods return `Ok(())` with no effect until that step (the
// forwarder probe, the listener binds and the gateway route need the forwarder
// module, step 4), so `run_server` boots and every serve test that serves no
// VM runs. The RED stays on the guest-flow paths: provisioning, activation,
// teardown, the dial, audit and quiescence panic; the boot refusals are
// pinned by bodies pending the attachment-cut step and step 13, which fail on
// their assertions.
#[async_trait::async_trait]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        Ok(())
    }

    async fn sweep_stale(&self) -> Result<()> {
        Ok(())
    }

    async fn converge_shared(&self) -> Result<()> {
        Ok(())
    }

    fn flow_ports(&self) -> Option<GuestFlowPorts> {
        // SCAFFOLD: the listeners are bound in the attachment-cut step (6);
        // until then the owner has bound nothing to report.
        None
    }

    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn quiesce(&self) -> (QuiescenceHold, ForwardingQuiescence) {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn restore(
        &self,
        _hold: QuiescenceHold,
    ) -> std::result::Result<ForwardingRestore, QuiescenceRestoreError> {
        panic!("Not yet implemented -- RED scaffold")
    }
}
