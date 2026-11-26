//! Shared guest-network application contract (GH #295).
//!
//! The host guest-flow owner, the guest address pool and their accepted
//! executable specifications live at this application boundary.
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Owner and provisioner", § "[REF] Typed error taxonomy and who
//! branches", § DG-2 rows 1–3 and 7–9, § DG-3.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::dial::GuestStreamDial;
use overdrive_core::guest_transport::wire::{FlowWireError, GUEST_FLOW_MAX_PAIRS};
use overdrive_core::guest_transport::{GuestCid, GuestFlowPorts, GuestTransportAssignment};
use overdrive_core::id::AllocationId;
use overdrive_core::net::{InterfaceIndex, ListenPort};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::{
    GuestAttachmentLease, GuestAttachmentObservation, GuestAttachmentView,
};
use overdrive_netlink::NetlinkError;

use crate::action_shim::reclamation::CorruptLeftoverRow;

use overdrive_core::guest_transport::forwarder::{ForwarderError, ForwarderGeneration};

mod dial;
pub(crate) mod flow_io;
mod flow_owner;
mod owner_runtime;

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

/// The shared components this owner repairs. The other components
/// (`LegF`, `LegC`, `IpRules`, `IpSets`, `Dns`, `Supervisor`) belong to the
/// mTLS worker, the DNS owner and the supervisor.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OwnerRepairComponent {
    /// The forwarder context and its inventory.
    Forwarder,
    /// The guest-flow stream listener and its accept task.
    FlowListeners,
    /// The per-VM control listener and its accept task.
    ControlListener,
}

impl From<OwnerRepairComponent> for SharedGuestNetworkComponent {
    fn from(component: OwnerRepairComponent) -> Self {
        match component {
            OwnerRepairComponent::Forwarder => Self::Forwarder,
            OwnerRepairComponent::FlowListeners => Self::FlowListeners,
            OwnerRepairComponent::ControlListener => Self::ControlListener,
        }
    }
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
    /// the same state. Nothing removes the route. It is boot and idempotent
    /// re-convergence of the whole owner only; a runtime recovery's repair is
    /// [`repair_shared`](Self::repair_shared).
    async fn converge_shared(&self) -> Result<()>;
    /// A runtime recovery's repair of exactly `component`. Never writes the
    /// gateway route and never reads or consumes state naming another
    /// component. `Forwarder` closes the old context before it opens and
    /// probes the new one (EVMOD-CTL); `FlowListeners` and `ControlListener`
    /// rebind that listener only and restart its accept task, leaving the
    /// other listener untouched. `Ok` means that component's repair ran and
    /// its postcondition read back; it is never returned for a component that
    /// was not repaired. Repairs of different components are independent. A
    /// call for a component whose repair is already in flight returns
    /// `GuestNetworkError::RepairInFlight` and changes no state. A repair
    /// cancelled mid-flight releases its claim and leaves no record.
    async fn repair_shared(&self, component: OwnerRepairComponent) -> Result<()>;
    /// PORTS-CONFIG (ADR-0218). `None` until this owner's first
    /// `converge_shared` returns `Ok`; from then on `Some` of the ports its
    /// stream and control listeners bound, the same value for the rest of
    /// the process (a listener repair keeps the held socket, so its port
    /// never changes). Reads only.
    fn flow_ports(&self) -> Option<GuestFlowPorts>;
    /// QHOLD: holds issued by this owner's `QuiescenceLedger` and dropped
    /// without a successful redeem since this owner started. Starts at 0,
    /// never decreases, and performs no I/O or state transition.
    #[must_use]
    fn leaked_quiescence_holds(&self) -> u32;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestNetworkFact {
    Forwarder { generation: ForwarderGeneration, live_pairs: u32, live_watches: u32 },
    FlowListener { port: overdrive_core::vm::config::VsockPort, bound: bool },
    ControlSession { alloc: AllocationId, cid: GuestCid, live: bool },
    AllocationForwarding { alloc: AllocationId, cid: GuestCid, pairs: u32 },
    CleanupComplement,
    SharedComponent { component: SharedGuestNetworkComponent, healthy: bool },
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
    /// A repair of this component is already in flight; nothing changed.
    #[error("a repair of shared guest-network component {component:?} is already in flight")]
    RepairInFlight { component: SharedGuestNetworkComponent },
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
    first_usable_offset: Option<u32>,
    last_usable_offset: Option<u32>,
    capacity: u32,
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

#[derive(Debug)]
struct GuestAddressPoolState {
    next_fit: GuestCid,
    leases: BTreeMap<AllocationId, PoolLease>,
    held_offsets: Vec<u64>,
    /// Retiring entries in `leases`; maintained to keep the admission check O(1).
    retiring: u32,
}

#[derive(Debug)]
struct PoolLease {
    /// The leased offset (`workload_addr - node_prefix.network()`).
    offset: u32,
    /// `None` for a reservation made by [`GuestAddressPool::reserve_leftover`]:
    /// a stuck VMM's lease carries no network or flow ports.
    plan: Option<GuestNetworkPlan>,
    state: GuestAttachmentLease,
}

impl GuestAddressPoolState {
    fn new(next_fit: GuestCid) -> Self {
        Self {
            next_fit,
            leases: BTreeMap::new(),
            held_offsets: vec![0; (overdrive_core::guest_transport::GUEST_CID_SPAN / 64) as usize],
            retiring: 0,
        }
    }

    fn offset_is_held(&self, offset: u32) -> bool {
        let word = self.held_offsets[(offset / 64) as usize];
        word & (1_u64 << (offset % 64)) != 0
    }

    fn hold_offset(&mut self, offset: u32) {
        self.held_offsets[(offset / 64) as usize] |= 1_u64 << (offset % 64);
    }

    fn release_offset(&mut self, offset: u32) {
        self.held_offsets[(offset / 64) as usize] &= !(1_u64 << (offset % 64));
    }
}

impl GuestAddressPool {
    /// Construct an empty pool over the node guest prefix and its gateway
    /// (the guest's DNS server address), with the entropy port that draws
    /// each lease's launch token (ATTR-TOKEN). The port is required:
    /// `OsEntropy` in production, `SimEntropy` under test (DG-2 row 1).
    #[doc(hidden)]
    #[must_use]
    pub fn new(node_prefix: Ipv4Net, gateway: Ipv4Addr, entropy: Arc<dyn Entropy>) -> Self {
        let network = u32::from(node_prefix.network());
        let broadcast = u32::from(node_prefix.broadcast());
        let (first_address, last_address) = if node_prefix.prefix_len() <= 30 {
            (network.saturating_add(1), broadcast.saturating_sub(1))
        } else {
            (network, broadcast)
        };
        let last_address = last_address
            .min(network.saturating_add(overdrive_core::guest_transport::GUEST_CID_SPAN - 1));
        let gateway_offset = u32::from(gateway).checked_sub(network);
        let address_range =
            (first_address <= last_address).then_some((first_address, last_address));
        let first_usable_offset = address_range.and_then(|(first, last)| {
            (first..=last)
                .map(|address| address - network)
                .find(|offset| Some(*offset) != gateway_offset)
        });
        let last_usable_offset = address_range.and_then(|(first, last)| {
            (first..=last)
                .rev()
                .map(|address| address - network)
                .find(|offset| Some(*offset) != gateway_offset)
        });
        let capacity = address_range.map_or(0, |(first, last)| {
            let gateway_in_range = gateway_offset
                .is_some_and(|offset| first <= network + offset && network + offset <= last);
            last - first + 1 - u32::from(gateway_in_range)
        });
        let initial_offset = first_usable_offset.unwrap_or(0);
        let next_fit =
            GuestCid::new(overdrive_core::guest_transport::GUEST_CID_BASE + initial_offset)
                .unwrap_or_else(|_| unreachable!("the initial CID cursor is in the leased range"));
        Self {
            node_prefix,
            gateway,
            first_usable_offset,
            last_usable_offset,
            capacity,
            entropy,
            held: Arc::new(parking_lot::Mutex::new(GuestAddressPoolState::new(next_fit))),
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
        listen_ports: &[ListenPort],
        flow_ports: GuestFlowPorts,
    ) -> Result<GuestNetworkPlan> {
        let mut state = self.held.lock();
        match state.leases.get(&alloc) {
            Some(PoolLease { state: GuestAttachmentLease::Retiring, .. }) => {
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
            Some(PoolLease { plan: Some(plan), state: GuestAttachmentLease::Admitted, .. }) => {
                return Ok(plan.clone());
            }
            // An Admitted lease always carries its plan: only `reserve_leftover`
            // makes a plan-less lease, and it makes it Retiring.
            Some(PoolLease { plan: None, state: GuestAttachmentLease::Admitted, .. }) => {
                unreachable!("an Admitted lease carries its plan")
            }
            None => {}
        }

        let occupancy = guest_pool_occupancy(&state);
        if occupancy.at_cap() {
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
        let Some(offset) = self.next_free_offset(state.next_fit, &state) else {
            drop(state);
            return Err(GuestNetworkError::PoolExhausted {
                held: occupancy.held,
                capacity: self.capacity,
            });
        };

        let workload_addr = Ipv4Addr::from(u32::from(self.node_prefix.network()) + offset);
        let cid = GuestCid::new(overdrive_core::guest_transport::GUEST_CID_BASE + offset)
            .unwrap_or_else(|_| unreachable!("the pool walks only leased CID offsets"));
        let launch = overdrive_core::guest_transport::LaunchToken::generate(self.entropy.as_ref());
        let mut listen_ports = listen_ports.to_vec();
        listen_ports.sort_unstable();
        listen_ports.dedup();
        let plan = GuestNetworkPlan {
            alloc: alloc.clone(),
            node_prefix: self.node_prefix,
            assignment: GuestTransportAssignment::new(
                workload_addr,
                self.gateway,
                cid,
                launch,
                flow_ports,
            ),
            listen_ports,
        };
        state.next_fit = self.cursor_after(offset);
        state.hold_offset(offset);
        state.leases.insert(
            alloc,
            PoolLease { offset, plan: Some(plan.clone()), state: GuestAttachmentLease::Admitted },
        );
        drop(state);
        Ok(plan)
    }

    fn next_free_offset(&self, cursor: GuestCid, state: &GuestAddressPoolState) -> Option<u32> {
        let (first, last) = (self.first_usable_offset?, self.last_usable_offset?);
        let cursor_offset = cursor.get() - overdrive_core::guest_transport::GUEST_CID_BASE;
        let start = cursor_offset.clamp(first, last);
        let gateway_offset =
            u32::from(self.gateway).checked_sub(u32::from(self.node_prefix.network()));
        let available =
            |offset: &u32| Some(*offset) != gateway_offset && !state.offset_is_held(*offset);
        let first_run = (start..=last).find(available);
        first_run.or_else(|| (start > first).then(|| (first..start).find(available)).flatten())
    }

    fn cursor_after(&self, offset: u32) -> GuestCid {
        let next = match self.last_usable_offset {
            Some(last) if offset < last => offset + 1,
            _ => self.first_usable_offset.unwrap_or(0),
        };
        GuestCid::new(overdrive_core::guest_transport::GUEST_CID_BASE + next)
            .unwrap_or_else(|_| unreachable!("the next-fit cursor is in the leased CID range"))
    }

    /// Returns the lease's offset to the free set. Its caller invokes it only
    /// after the allocation's VMM has been reaped (release-last).
    pub(crate) fn release(&self, alloc: &AllocationId) {
        {
            let mut state = self.held.lock();
            if let Some(lease) = state.leases.remove(alloc) {
                if lease.state == GuestAttachmentLease::Retiring {
                    state.retiring -= 1;
                }
                state.release_offset(lease.offset);
            }
        }
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<AllocationId, GuestNetworkPlan> {
        self.held
            .lock()
            .leases
            .iter()
            .filter_map(|(alloc, lease)| lease.plan.clone().map(|plan| (alloc.clone(), plan)))
            .collect()
    }

    /// Reserve the guest address a stuck VM left behind (ADR-0350): insert a
    /// Retiring lease for `alloc` at the offset of `held`, the `workload_addr`
    /// its row recorded. The lease counts toward the cap, makes
    /// [`Self::assign`] for `alloc` return `LeaseRetiring`, is reported by
    /// [`Self::observe`] and omitted from [`Self::snapshot`] (it carries no
    /// plan). The CID follows from the offset exactly as `assign` derives it.
    ///
    /// A corrupt row never panics: an address outside the leased range, or an
    /// offset another lease (or this allocation at another offset) already
    /// holds, is returned as [`CorruptLeftoverRow`]. A repeat for the same
    /// allocation and address changes nothing.
    pub(crate) fn reserve_leftover(
        &self,
        alloc: &AllocationId,
        held: Ipv4Addr,
    ) -> std::result::Result<(), CorruptLeftoverRow> {
        let Some(offset) = self.leased_offset(held) else {
            return Err(CorruptLeftoverRow::address_outside_prefix(alloc.clone(), held));
        };
        let mut state = self.held.lock();
        if let Some(existing) = state.leases.get(alloc) {
            return if existing.offset == offset {
                Ok(())
            } else {
                Err(CorruptLeftoverRow::offset_already_held(alloc.clone(), held, alloc.clone()))
            };
        }
        if state.offset_is_held(offset) {
            let Some((holder, _)) = state.leases.iter().find(|(_, lease)| lease.offset == offset)
            else {
                unreachable!("a held offset belongs to a lease")
            };
            let holder = holder.clone();
            return Err(CorruptLeftoverRow::offset_already_held(alloc.clone(), held, holder));
        }
        state.hold_offset(offset);
        state.retiring += 1;
        state.leases.insert(
            alloc.clone(),
            PoolLease { offset, plan: None, state: GuestAttachmentLease::Retiring },
        );
        drop(state);
        Ok(())
    }

    /// The offset of `address` when the pool leases it: inside the node
    /// prefix, within the usable offset range, and not the gateway.
    fn leased_offset(&self, address: Ipv4Addr) -> Option<u32> {
        let (first, last) = (self.first_usable_offset?, self.last_usable_offset?);
        let offset = u32::from(address).checked_sub(u32::from(self.node_prefix.network()))?;
        let leased = self.node_prefix.contains(&address)
            && (first..=last).contains(&offset)
            && u32::from(self.gateway).checked_sub(u32::from(self.node_prefix.network()))
                != Some(offset);
        leased.then_some(offset)
    }

    /// Move an Admitted lease to Retiring; `true` only for that transition.
    /// An absent or already-Retiring lease returns `false` with no change, and
    /// Retiring never returns to Admitted. The lease still counts against the
    /// cap until [`Self::release`].
    pub(crate) fn retire(&self, alloc: &AllocationId) -> bool {
        {
            let mut state = self.held.lock();
            match state.leases.get_mut(alloc) {
                Some(lease) if lease.state == GuestAttachmentLease::Admitted => {
                    lease.state = GuestAttachmentLease::Retiring;
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
            .filter_map(|alloc| state.leases.get(alloc).map(|lease| (alloc.clone(), lease.state)))
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
    ports: GuestFlowOwnerPorts,
    io: Arc<dyn flow_io::GuestFlowIo>,
    /// QHOLD (D-UP-4): this owner's one ledger; it issues and redeems the
    /// owner's holds and counts those dropped unredeemed.
    ledger: QuiescenceLedger,
    runtime: Arc<flow_owner::OwnerRuntime>,
}

#[cfg(test)]
#[derive(Default)]
struct SharedAuditCheckpoint {
    armed: parking_lot::Mutex<Option<SharedAuditCheckpointWait>>,
}

#[cfg(test)]
struct SharedAuditCheckpointWait {
    reached: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(all(test, feature = "integration-tests"))]
pub(crate) struct SharedAuditCheckpointControl {
    reached: tokio::sync::oneshot::Receiver<()>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

#[cfg(test)]
impl SharedAuditCheckpoint {
    #[cfg(all(test, feature = "integration-tests"))]
    fn arm(&self) -> SharedAuditCheckpointControl {
        let (reached, wait_reached) = tokio::sync::oneshot::channel();
        let (release, wait_release) = tokio::sync::oneshot::channel();
        let mut armed = self.armed.lock();
        assert!(armed.is_none(), "a shared-audit checkpoint is already armed");
        *armed = Some(SharedAuditCheckpointWait { reached, release: wait_release });
        drop(armed);
        SharedAuditCheckpointControl { reached: wait_reached, release: Some(release) }
    }

    async fn pause_if_armed(&self) {
        let checkpoint = { self.armed.lock().take() };
        let Some(checkpoint) = checkpoint else { return };
        let _reached_observed = checkpoint.reached.send(());
        let _released_or_cancelled = checkpoint.release.await;
    }
}

#[cfg(all(test, feature = "integration-tests"))]
impl SharedAuditCheckpointControl {
    #[allow(
        clippy::expect_used,
        reason = "test-only checkpoint: a dropped arm is a harness failure"
    )]
    pub(crate) async fn reached(&mut self) {
        (&mut self.reached)
            .await
            .expect("the armed shared audit reaches its post-snapshot checkpoint");
    }

    pub(crate) fn release(mut self) -> bool {
        self.release.take().is_some_and(|release| release.send(()).is_ok())
    }
}

impl std::fmt::Debug for HostSharedGuestNetworkOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostSharedGuestNetworkOwner").finish_non_exhaustive()
    }
}

impl Drop for HostSharedGuestNetworkOwner {
    fn drop(&mut self) {
        if let Some(tasks) = self.runtime.tasks.lock().as_ref() {
            tasks.shutdown.cancel();
        }
    }
}

impl HostSharedGuestNetworkOwner {
    /// A listener repair reports the owner's listeners as running only when
    /// both accept tasks are, so repairing one never asserts the other's state.
    fn refresh_flow_listeners_running(&self) {
        let running = self.runtime.tasks.lock().as_ref().is_some_and(|tasks| {
            [&tasks.stream_accept, &tasks.control_accept]
                .into_iter()
                .all(|task| task.as_ref().is_some_and(|task| !task.is_finished()))
        });
        self.runtime.state.lock().flow_listeners_running = running;
    }

    /// Cancel and join this composed owner's listener and per-connection task
    /// tree before the serve owner releases the adapters those tasks hold.
    pub(crate) async fn shutdown_tasks(&self) {
        owner_runtime::shutdown(self).await;
    }

    #[cfg(all(test, feature = "integration-tests"))]
    pub(crate) fn arm_shared_audit_post_snapshot_checkpoint(&self) -> SharedAuditCheckpointControl {
        self.runtime.shared_audit_checkpoint.arm()
    }
}

async fn wait_for_drained_refusal_readers(
    runtime: &Arc<flow_owner::OwnerRuntime>,
    readers: &[(GuestCid, GuestConnKey)],
) {
    loop {
        let notified = runtime.refusal_reader_capacity.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let pending = {
            let state = runtime.state.lock();
            readers.iter().any(|(cid, key)| {
                state
                    .cids
                    .get(cid)
                    .and_then(|entry| entry.flow_tasks.get(key))
                    .is_some_and(|task| task.is_refusal_reader() && task.task.is_none())
            })
        };
        if !pending {
            return;
        }
        notified.await;
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
        let runtime = Arc::new(flow_owner::OwnerRuntime::new(
            Arc::clone(&ports.clock),
            ports.vip_range.clone(),
        ));
        Self { ports, io, ledger: QuiescenceLedger::new(), runtime }
    }
}

/// Phase 1 of the host guest-flow owner's two-phase boot (D-8, § *Composition*
/// step 1): every `probe_startup` check over one `io.host_facts()` snapshot —
/// `GuestPrefixOverlapsVipRange` first, then `GuestPrefixOverlapsNodeAddress`,
/// `FdBudget` — then `io.probe_forwarder()`
/// (`GuestNetworkError::Forwarder { source }`). Runs before the owner exists
/// and before any boot effect, and leaves nothing behind: the probe's
/// forwarder context is released before return, on `Ok` and on `Err`.
/// [`HostSharedGuestNetworkOwner::probe_startup`] is this function over the
/// owner's own seam, `vip_range` and `node_prefix`, so the two cannot
/// disagree.
pub(crate) async fn probe_guest_flow_startup(
    io: &dyn flow_io::GuestFlowIo,
    vip_range: &overdrive_dataplane::allocators::VipRange,
    node_prefix: Ipv4Net,
) -> Result<()> {
    let facts = io.host_facts().await?;
    if let Some(vip_range) = vip_range.first_overlapping_range(node_prefix) {
        return Err(GuestNetworkError::GuestPrefixOverlapsVipRange {
            prefix: node_prefix,
            vip_range,
        });
    }
    for (interface, addresses) in &facts.interfaces {
        if let Some(address) =
            addresses.iter().find(|address| node_prefix.contains(&address.addr()))
        {
            return Err(GuestNetworkError::GuestPrefixOverlapsNodeAddress {
                prefix: node_prefix,
                interface: *interface,
                address: *address,
            });
        }
    }

    // ADMISSIONREFUSAL1 retains the accepted four-place-per-attachment node
    // baseline and adds one descriptor for each bounded refusal reader.
    let required =
        u64::from(GUEST_FLOW_MAX_PAIRS) * 4 + u64::from(GUEST_FLOW_MAX_PAIRS) + 16_384 + 1 + 32_767;
    if facts.nofile_limit < required {
        return Err(GuestNetworkError::FdBudget { required, available: facts.nofile_limit });
    }

    io.probe_forwarder().await.map_err(|source| GuestNetworkError::Forwarder { source })?;
    Ok(())
}

fn listener_io_error(
    operation: GuestNetworkOperation,
    error: flow_io::FlowIoError,
) -> GuestNetworkError {
    match error {
        flow_io::FlowIoError::Io { source } => GuestNetworkError::Io { operation, source },
    }
}

#[async_trait::async_trait]
#[allow(
    clippy::too_many_lines,
    reason = "the allocation owner operations share the locked lease and session lifecycle invariants"
)]
impl GuestNetworkProvisioner for HostSharedGuestNetworkOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let assignment = plan.assignment();
        let cid = assignment.cid();
        let mut state = self.runtime.state.lock();
        match state.cids.get(&cid) {
            Some(cid)
                if cid.alloc == *plan.alloc()
                    && cid.launch == assignment.launch()
                    && cid.phase != CidPhase::Retiring =>
            {
                return Ok(());
            }
            Some(_) | None => {}
        }
        let replace_retiring = state.cids.get(&cid).is_some_and(|old| {
            old.phase == CidPhase::Retiring
                && old.launch != assignment.launch()
                && old.session.is_none()
                && old.session_sender.is_none()
                && old.session_cancel.is_none()
                && old.guest_flows.is_empty()
                && old.flow_tasks.is_empty()
                && old.flow_ids.is_empty()
                && old.dials.is_empty()
                && old.dial_replies.is_empty()
                && old.dial_cancels.is_empty()
                && old.dial_requests.is_empty()
                && old.datagrams.is_empty()
                && old.control_tasks.len() == old.unread.len()
                && old.unread.iter().all(|key| {
                    // CONFALIGN-1: a retained task keeps the launch it was
                    // accepted under (its attribution identity), so any
                    // launch of this CID — not only `old.launch` — is valid.
                    old.control_tasks.get(key).is_some_and(|task| task.owner_launch.is_some())
                })
                && !state.pairs.values().any(|pair| pair.cid == cid)
                && !state.dial_watches.values().any(|(owner_cid, _)| *owner_cid == cid)
        });
        if state.cids.contains_key(&cid) && !replace_retiring {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::PoolAssign,
                expected: GuestNetworkFact::AllocationForwarding {
                    alloc: plan.alloc().clone(),
                    cid,
                    pairs: 0,
                },
                observed: None,
            });
        }

        let mut replacement = flow_owner::CidState::provisioned(
            plan.alloc().clone(),
            assignment.launch(),
            assignment.workload_addr(),
            plan.listen_ports().iter().copied(),
        );
        if replace_retiring {
            let mut retired = state
                .cids
                .remove(&cid)
                .unwrap_or_else(|| unreachable!("the checked Retiring CID remains present"));
            replacement.unread = std::mem::take(&mut retired.unread);
            replacement.control_tasks = std::mem::take(&mut retired.control_tasks);
            replacement.dial_outcomes = std::mem::take(&mut retired.dial_outcomes);
        }
        state.cids.insert(cid, replacement);
        drop(state);
        Ok(())
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<ForwardingActivation> {
        let mut state = self.runtime.state.lock();
        let cid = plan.assignment().cid();
        let observed = state.cids.get(&cid);
        let Some(observed) = observed
            .filter(|entry| entry.alloc == *plan.alloc() && entry.phase != CidPhase::Retiring)
        else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::ForwardingActivate,
                expected: GuestNetworkFact::AllocationForwarding {
                    alloc: plan.alloc().clone(),
                    cid,
                    pairs: 0,
                },
                observed: observed.map(|entry| GuestNetworkFact::AllocationForwarding {
                    alloc: entry.alloc.clone(),
                    cid,
                    pairs: u32::try_from(
                        entry.guest_flows.len() + entry.datagrams.len() + entry.dials.len(),
                    )
                    .unwrap_or(u32::MAX),
                }),
            });
        };
        if state.admission == flow_owner::GuestAdmission::Quiesced {
            return Ok(ForwardingActivation::QuiescenceLatched);
        }
        if observed.phase == CidPhase::Active {
            return Ok(ForwardingActivation::Activated);
        }
        if observed.session.is_none() {
            return Ok(ForwardingActivation::ControlSessionPending);
        }
        state
            .cids
            .get_mut(&cid)
            .unwrap_or_else(|| unreachable!("the activated CID remains present under the lock"))
            .phase = CidPhase::Active;
        drop(state);
        Ok(ForwardingActivation::Activated)
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let cid = plan.assignment().cid();
        let (session_cancel, control_tasks, flow_drain, dial_watches) = {
            let mut state = self.runtime.state.lock();
            let Some(owner) = state.cids.get_mut(&cid) else { return Ok(()) };
            if owner.alloc != *plan.alloc() {
                return Ok(());
            }
            owner.phase = CidPhase::Retiring;
            owner.session = None;
            owner.session_sender = None;
            let session_cancel = owner.session_cancel.take();
            let unread = owner.unread.clone();
            let mut control_tasks = Vec::new();
            for (key, task) in std::mem::take(&mut owner.control_tasks) {
                if unread.contains(&key) {
                    owner.control_tasks.insert(key, task);
                } else {
                    control_tasks.push(task);
                }
            }
            let flow_drain = owner.take_flow_tasks_for_drain();
            let mut watches = Vec::new();
            for (flow, dial) in &owner.dials {
                if matches!(dial.phase, flow_owner::DialPhase::AwaitingReply) {
                    owner.dial_outcomes.insert(
                        *flow,
                        overdrive_core::guest_transport::dial::GuestDialError::NotServing,
                    );
                }
                if let Some(cancel) = owner.dial_cancels.get(flow) {
                    cancel.cancel();
                }
                if let flow_owner::DialPhase::Paired { watch, .. } = dial.phase {
                    watches.push((*flow, watch));
                }
            }
            let snapshot = (session_cancel, control_tasks, flow_drain, watches);
            drop(state);
            snapshot
        };
        let mut tasks = control_tasks;
        tasks.extend(flow_drain.tasks);
        if let Some(cancel) = session_cancel {
            cancel.cancel();
        }
        for (_, cancel) in &flow_drain.refusal_readers {
            cancel.cancel();
        }
        for task in &tasks {
            task.cancel.cancel();
        }
        for mut task in tasks {
            if let Some(join) = task.task.take() {
                let _ = join.await;
            }
        }
        let drained_readers =
            flow_drain.refusal_readers.iter().map(|(key, _)| (cid, *key)).collect::<Vec<_>>();
        wait_for_drained_refusal_readers(&self.runtime, &drained_readers).await;

        for (flow, watch) in dial_watches {
            self.io.abort_forward(watch)?;
            self.runtime.state.lock().dial_watches.remove(&watch);
            if let Some(owner) = self.runtime.state.lock().cids.get_mut(&cid) {
                owner.dials.remove(&flow);
                owner.dial_replies.remove(&flow);
                owner.dial_cancels.remove(&flow);
                owner.dial_requests.remove(&flow);
            }
            tracing::info!(name: "guest_flow.closed", alloc = %plan.alloc(), cid = %cid, flow = flow.get(), kind = "host_dial", cause = FlowCloseCause::Teardown.as_str());
        }

        let pair_ids = {
            let state = self.runtime.state.lock();
            state
                .pairs
                .iter()
                .filter_map(|(id, pair)| (pair.cid == cid).then_some(*id))
                .collect::<Vec<_>>()
        };
        for id in pair_ids {
            let pair = {
                let mut state = self.runtime.state.lock();
                self.io.abort_forward(id)?;
                state.abort_causes.insert(id, FlowCloseCause::Teardown);
                let pair = state.pairs.remove(&id);
                if let Some(pair) = &pair
                    && let Some(owner) = state.cids.get_mut(&cid)
                {
                    owner.flow_ids.remove(&pair.flow);
                    owner.guest_flows.retain(|_, phase| {
                        !matches!(phase, flow_owner::GuestFlowPhase::Paired { pair: observed, .. } if *observed == id)
                    });
                    owner.datagrams.remove(&pair.flow);
                }
                drop(state);
                pair
            };
            if let Some(pair) = pair {
                self.io.close(pair.peer.into()).await;
                self.io.close(pair.vsock.into()).await;
            }
        }

        let mut state = self.runtime.state.lock();
        let Some(owner) = state.cids.get(&cid) else { return Ok(()) };
        let remaining = owner.session.is_some()
            || !owner.guest_flows.is_empty()
            || !owner.dials.is_empty()
            || !owner.datagrams.is_empty()
            || !owner.flow_tasks.is_empty()
            || owner.control_tasks.keys().any(|key| !owner.unread.contains(key))
            || state.pairs.values().any(|pair| pair.cid == cid);
        if remaining {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::PairAbort,
                expected: GuestNetworkFact::AllocationForwarding {
                    alloc: owner.alloc.clone(),
                    cid,
                    pairs: 0,
                },
                observed: Some(GuestNetworkFact::AllocationForwarding {
                    alloc: owner.alloc.clone(),
                    cid,
                    pairs: u32::try_from(owner.held_flows()).unwrap_or(u32::MAX),
                }),
            });
        }
        if owner.unread.is_empty() && owner.control_tasks.is_empty() {
            state.cids.remove(&cid);
        }
        drop(state);
        Ok(())
    }
}

// The boot, audit and counted quiescence operations of the guest-flow owner
// are composed by `run_server` and drive the same production owner instance.
#[allow(
    clippy::too_many_lines,
    reason = "the shared-owner port implementation keeps ordered convergence, audit, repair and hold transitions together"
)]
#[async_trait::async_trait]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        probe_guest_flow_startup(self.io.as_ref(), &self.ports.vip_range, self.ports.node_prefix)
            .await
    }

    async fn sweep_stale(&self) -> Result<()> {
        if self.io.stale_objects_present().await? {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::CleanupComplement,
                expected: GuestNetworkFact::CleanupComplement,
                observed: Some(GuestNetworkFact::CleanupComplement),
            });
        }
        Ok(())
    }

    async fn converge_shared(&self) -> Result<()> {
        if self.runtime.state.lock().flow_ports.is_some() {
            owner_runtime::start(self);
            return Ok(());
        }

        let generation = self.io.open_forwarder().await?;
        let inventory = self.io.inventory()?;
        {
            let mut state = self.runtime.state.lock();
            state.forwarder_generation = generation;
            state.forwarder_inventory = ForwarderInventoryRead::Read(inventory);
        }

        let stream = self
            .io
            .bind_listener(flow_io::FlowListener::Stream, self.ports.stream_port)
            .await
            .map_err(|error| listener_io_error(GuestNetworkOperation::FlowListenerBind, error))?;
        let control = self
            .io
            .bind_listener(flow_io::FlowListener::Control, self.ports.control_port)
            .await
            .map_err(|error| {
                listener_io_error(GuestNetworkOperation::ControlListenerBind, error)
            })?;
        let flow_ports =
            GuestFlowPorts::new(stream, control).map_err(|error| GuestNetworkError::Io {
                operation: GuestNetworkOperation::ControlListenerBind,
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
            })?;
        self.io.converge_gateway_route(self.ports.gateway).await?;

        let mut state = self.runtime.state.lock();
        state.flow_ports = Some(flow_ports);
        state.flow_listeners_running = true;
        state.gateway_converged = true;
        drop(state);
        owner_runtime::start(self);
        Ok(())
    }

    async fn repair_shared(&self, component: OwnerRepairComponent) -> Result<()> {
        let Some(_claim) = self.runtime.repairs.try_claim(component) else {
            return Err(GuestNetworkError::RepairInFlight { component: component.into() });
        };
        match component {
            OwnerRepairComponent::Forwarder => {
                self.io.close_forwarder();
                self.runtime.state.lock().forwarder_inventory = ForwarderInventoryRead::NoContext;
                let generation = self.io.open_forwarder().await?;
                let inventory = self.io.inventory()?;
                let mut state = self.runtime.state.lock();
                state.forwarder_generation = generation;
                state.forwarder_inventory = ForwarderInventoryRead::Read(inventory);
                drop(state);
                Ok(())
            }
            OwnerRepairComponent::FlowListeners => {
                self.io
                    .bind_listener(flow_io::FlowListener::Stream, self.ports.stream_port)
                    .await
                    .map_err(|error| {
                        listener_io_error(GuestNetworkOperation::FlowListenerBind, error)
                    })?;
                owner_runtime::restart_listener(self, flow_io::FlowListener::Stream);
                self.refresh_flow_listeners_running();
                Ok(())
            }
            OwnerRepairComponent::ControlListener => {
                self.io
                    .bind_listener(flow_io::FlowListener::Control, self.ports.control_port)
                    .await
                    .map_err(|error| {
                        listener_io_error(GuestNetworkOperation::ControlListenerBind, error)
                    })?;
                owner_runtime::restart_listener(self, flow_io::FlowListener::Control);
                self.refresh_flow_listeners_running();
                Ok(())
            }
        }
    }

    fn flow_ports(&self) -> Option<GuestFlowPorts> {
        self.runtime.state.lock().flow_ports
    }

    fn leaked_quiescence_holds(&self) -> u32 {
        self.ledger.leaked()
    }

    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        let repairs_in_flight = self.runtime.repairs.snapshot();
        let (expected_pairs, expected_watches) = {
            let state = self.runtime.state.lock();
            let pairs = u32::try_from(state.pairs.len()).unwrap_or(u32::MAX);
            let watches = u32::try_from(state.dial_watches.len()).unwrap_or(u32::MAX);
            let snapshot = (pairs, watches);
            drop(state);
            snapshot
        };
        let generation = self.runtime.state.lock().forwarder_generation;
        #[cfg(test)]
        self.runtime.shared_audit_checkpoint.pause_if_armed().await;
        if !repairs_in_flight.contains(&OwnerRepairComponent::Forwarder) {
            let inventory = self.io.inventory().map_err(|source| SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::Forwarder,
                source: GuestNetworkError::Forwarder { source },
            })?;
            // End records leave the live count when queued and remain in the
            // owner's table until the consumer reads them.  Count that
            // existing handoff state as occupied while auditing the owner;
            // otherwise a normal end-record delivery window is reported as
            // forwarder damage.
            let inventory_places = inventory
                .live_pairs
                .saturating_add(inventory.live_watches)
                .saturating_add(inventory.queued_ends);
            let expected_places = expected_pairs.saturating_add(expected_watches);
            let inventory_matches_snapshot = inventory_places == expected_places;
            let inventory_matches_current_owner = if inventory_matches_snapshot {
                true
            } else {
                let state = self.runtime.state.lock();
                !self.runtime.repairs.snapshot().contains(&OwnerRepairComponent::Forwarder)
                    && state.forwarder_generation == generation
                    && inventory_places
                        == u32::try_from(state.pairs.len()).unwrap_or(u32::MAX).saturating_add(
                            u32::try_from(state.dial_watches.len()).unwrap_or(u32::MAX),
                        )
            };
            if !inventory_matches_current_owner {
                return Err(SharedGuestNetworkAuditError {
                    component: SharedGuestNetworkComponent::Forwarder,
                    source: GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::SharedAudit,
                        expected: GuestNetworkFact::Forwarder {
                            generation,
                            live_pairs: expected_pairs,
                            live_watches: expected_watches,
                        },
                        observed: Some(GuestNetworkFact::Forwarder {
                            generation,
                            live_pairs: inventory.live_pairs,
                            live_watches: inventory.live_watches,
                        }),
                    },
                });
            }
        }
        let task_failure = {
            let tasks = self.runtime.tasks.lock();
            tasks.as_ref().and_then(|tasks| {
                if !tasks.shutdown.is_cancelled()
                    && !repairs_in_flight.contains(&OwnerRepairComponent::FlowListeners)
                    && tasks
                        .stream_accept
                        .as_ref()
                        .is_some_and(tokio::task::JoinHandle::is_finished)
                {
                    Some(SharedGuestNetworkComponent::FlowListeners)
                } else if !tasks.shutdown.is_cancelled()
                    && !repairs_in_flight.contains(&OwnerRepairComponent::ControlListener)
                    && tasks
                        .control_accept
                        .as_ref()
                        .is_some_and(tokio::task::JoinHandle::is_finished)
                {
                    Some(SharedGuestNetworkComponent::ControlListener)
                } else {
                    None
                }
            })
        };
        if let Some(component) = task_failure {
            return Err(SharedGuestNetworkAuditError {
                component,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::SharedAudit,
                    expected: GuestNetworkFact::SharedComponent { component, healthy: true },
                    observed: Some(GuestNetworkFact::SharedComponent { component, healthy: false }),
                },
            });
        }
        Ok(SharedGuestNetworkAudit::default())
    }

    async fn quiesce(&self) -> (QuiescenceHold, ForwardingQuiescence) {
        let hold = self.ledger.issue();
        let (pairs, tasks, refusal_readers, allocations, dials, first) = {
            let mut state = self.runtime.state.lock();
            state.quiescence_holds = state.quiescence_holds.saturating_add(1);
            let first = state.quiescence_holds == 1;
            if first {
                state.admission = flow_owner::GuestAdmission::Quiesced;
                let allocations = state
                    .cids
                    .iter()
                    .filter(|(_, cid)| cid.held_flows() > 0)
                    .map(|(guest_cid, cid)| {
                        (
                            cid.alloc.clone(),
                            *guest_cid,
                            u32::try_from(cid.held_flows()).unwrap_or(u32::MAX),
                        )
                    })
                    .collect::<Vec<_>>();
                let dials = state
                    .cids
                    .iter()
                    .flat_map(|(guest_cid, cid)| {
                        cid.dials.iter().filter_map(move |(flow, dial)| match dial.phase {
                            flow_owner::DialPhase::Paired { watch, .. } => Some((
                                *guest_cid,
                                *flow,
                                watch,
                                cid.alloc.clone(),
                                cid.session_sender.clone(),
                            )),
                            flow_owner::DialPhase::AwaitingReply => None,
                        })
                    })
                    .collect::<Vec<_>>();
                let mut tasks = Vec::new();
                let mut refusal_readers = Vec::new();
                for (guest_cid, cid) in &mut state.cids {
                    cid.guest_flows.clear();
                    cid.datagrams.clear();
                    for (flow, dial) in &cid.dials {
                        if matches!(dial.phase, flow_owner::DialPhase::AwaitingReply) {
                            cid.dial_outcomes.insert(
                                *flow,
                                overdrive_core::guest_transport::dial::GuestDialError::Quiesced,
                            );
                        }
                        if let Some(cancel) = cid.dial_cancels.get(flow) {
                            cancel.cancel();
                        }
                    }
                    cid.dials.clear();
                    cid.dial_replies.clear();
                    cid.dial_cancels.clear();
                    cid.dial_requests.clear();
                    cid.flow_ids.clear();
                    let drain = cid.take_flow_tasks_for_drain();
                    tasks.extend(drain.tasks);
                    refusal_readers.extend(
                        drain
                            .refusal_readers
                            .into_iter()
                            .map(|(key, cancel)| (*guest_cid, key, cancel)),
                    );
                }
                let watches = dials
                    .iter()
                    .map(|(_, _, watch, _, _)| *watch)
                    .collect::<std::collections::BTreeSet<_>>();
                state.dial_watches.retain(|watch, _| !watches.contains(watch));
                let pairs = std::mem::take(&mut state.pairs);
                let snapshot = (pairs, tasks, refusal_readers, allocations, dials, true);
                drop(state);
                snapshot
            } else {
                drop(state);
                (BTreeMap::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), false)
            }
        };
        let mut unconfirmed = BTreeMap::new();
        if first {
            for task in &tasks {
                task.cancel.cancel();
            }
            for (_, _, cancel) in &refusal_readers {
                cancel.cancel();
            }
            if self.io.abort_all().is_err() {
                for (allocation, cid, pairs) in allocations {
                    unconfirmed.insert(
                        allocation.clone(),
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::PairAbort,
                            expected: GuestNetworkFact::AllocationForwarding {
                                alloc: allocation.clone(),
                                cid,
                                pairs: 0,
                            },
                            observed: Some(GuestNetworkFact::AllocationForwarding {
                                alloc: allocation,
                                cid,
                                pairs,
                            }),
                        },
                    );
                }
            }
            for mut task in tasks {
                if let Some(join) = task.task.take() {
                    let _ = join.await;
                }
            }
            let drained_readers =
                refusal_readers.iter().map(|(cid, key, _)| (*cid, *key)).collect::<Vec<_>>();
            wait_for_drained_refusal_readers(&self.runtime, &drained_readers).await;
            for (_, pair) in pairs {
                tracing::info!(name: "guest_flow.closed", alloc = %pair.alloc, cid = %pair.cid, kind = if pair.mode == flow_io::PairMode::Stream { "tcp_connect" } else { "datagram" }, cause = FlowCloseCause::Quiesced.as_str());
                self.io.close(pair.peer.into()).await;
                self.io.close(pair.vsock.into()).await;
            }
            for (guest_cid, flow, _, allocation, sender) in dials {
                if let Some(sender) = sender {
                    let _ =
                        sender.send(overdrive_core::guest_transport::wire::ControlMessage::Abort {
                            flow,
                        });
                }
                tracing::info!(name: "guest_flow.closed", alloc = %allocation, cid = %guest_cid, flow = flow.get(), kind = "host_dial", cause = FlowCloseCause::Quiesced.as_str());
            }
        }
        (hold, ForwardingQuiescence { unconfirmed })
    }

    async fn restore(
        &self,
        hold: QuiescenceHold,
    ) -> std::result::Result<ForwardingRestore, QuiescenceRestoreError> {
        if let Err(hold) = self.ledger.redeem(hold) {
            drop(hold);
            return Err(QuiescenceRestoreError::ForeignHold);
        }
        let mut state = self.runtime.state.lock();
        if state.quiescence_holds == 0 {
            return Err(QuiescenceRestoreError::ForeignHold);
        }
        state.quiescence_holds -= 1;
        if state.quiescence_holds == 0 {
            state.admission = flow_owner::GuestAdmission::Open;
            let result = Ok(ForwardingRestore::Reopened);
            drop(state);
            result
        } else {
            let result = Ok(ForwardingRestore::StillQuiesced);
            drop(state);
            result
        }
    }
}
