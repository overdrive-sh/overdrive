//! Shared guest-network application contract (GH #295).
//!
//! The private host owner/pool implementation and accepted executable
//! specifications live at this application boundary.

#![expect(
    clippy::use_self,
    reason = "exact accepted API scaffold precedes implementation and names GuestNetworkError explicitly"
)]

use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
#[cfg(target_os = "linux")]
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd as _;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::GuestNetworkAssignment;
use overdrive_core::traits::{GuestAttachmentObservation, GuestAttachmentView};
use overdrive_netlink::NetlinkError;

#[cfg(target_os = "linux")]
nix::ioctl_write_ptr_bad!(d14_tun_set_iff, libc::TUNSETIFF, libc::ifreq);

fn enable_bridge_nf_call_iptables(bridge: &str) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/sys/class/net/{bridge}/bridge/nf_call_iptables");
        let mut file = OpenOptions::new().write(true).open(&path)?;
        file.write_all(b"1\n")?;
        let observed = std::fs::read_to_string(path)?;
        if observed.trim() != "1" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bridge netfilter flag did not read back as enabled",
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = bridge;
        Ok(())
    }
}

use overdrive_dataplane::guest_tcx::{
    GuestTcxAttachment, GuestTcxCounter, GuestTcxEndpoint, GuestTcxInventoryIdentity, GuestTcxLink,
    GuestTcxProbeCounterObservation, GuestTcxProbeMark, GuestTcxProbeVerdict, GuestTcxProgram,
    GuestTcxTcpProbeInput, GuestTcxTcpProbeOutcome,
};
pub use overdrive_dataplane::guest_tcx::{GuestTcxError, TcxAttachPoint};
use overdrive_netlink::nft::bridge::{
    BridgeGuardChainDefinition, BridgeGuardChainHook, BridgeGuardChainPolicy, BridgeGuardChainType,
    BridgeGuardError, BridgeGuardMemberIdentity, BridgeGuardMutationOutcome,
    BridgeGuardObservation, BridgeGuardObservedFamily, BridgeGuardSpec, BridgeGuardTableFact,
};
pub use overdrive_netlink::nft::bridge::{
    BridgeGuardRuleExpression, BridgeGuardRuleFact, BridgeGuardRuleIdentity, BridgeGuardRuleProgram,
};

/// Opaque allocation-scoped network plan; only control-plane owners construct it.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestNetworkPlan {
    alloc: AllocationId,
    bridge: String,
    node_prefix: Ipv4Net,
    assignment: GuestNetworkAssignment,
}

impl GuestNetworkPlan {
    /// Allocation owning this plan.
    #[must_use]
    pub fn alloc(&self) -> &AllocationId {
        &self.alloc
    }
    /// Node-local bridge name.
    #[must_use]
    pub fn bridge(&self) -> &str {
        &self.bridge
    }
    /// Node guest prefix.
    #[must_use]
    pub fn node_prefix(&self) -> Ipv4Net {
        self.node_prefix
    }
    /// Complete grouped driver handoff.
    #[must_use]
    pub fn assignment(&self) -> &GuestNetworkAssignment {
        &self.assignment
    }
}

/// Allocation-scoped awaited network effects.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait GuestNetworkProvisioner: Send + Sync {
    /// Provision the named attachment to its full read-back postcondition,
    /// with the TAP administratively down. Never performs `TapSetUp`.
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()>;
    /// Activate the protected attachment after the allocation intercept is
    /// live: the only allocation operation allowed to set the TAP up.
    ///
    /// `Ok(TapActivation::Raised)` once the TAP is up and read back with its
    /// exact master (idempotent on a completed activation);
    /// `Ok(TapActivation::QuiescenceLatched)` with no mutation while runtime
    /// quiescence is latched. A condemned allocation, or a missing or
    /// non-matching allocation record, is the source-less
    /// `PostconditionMismatch`.
    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<TapActivation>;
    /// Tear down the named attachment to its empty complement. An absent part
    /// is already its postcondition; the final complement read-back alone
    /// decides success.
    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()>;
}

/// The outcome of one `activate` call that did not fail.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapActivation {
    /// The TAP is administratively up and read back with its exact master.
    Raised,
    /// Runtime quiescence is latched; nothing was mutated. The caller waits
    /// for recovery to reopen the EXEC gate and calls `activate` again.
    QuiescenceLatched,
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
    /// Exercise isolated scratch resources and always attempt cleanup.
    async fn probe_startup(&self) -> Result<()>;
    /// Remove prior-epoch owned attachment residue after VMM reclamation.
    async fn sweep_stale(&self) -> Result<()>;
    /// Converge the one production shared switch identity.
    async fn converge_shared(&self) -> Result<()>;
    /// Non-repairing audit. `Err` names the first failing node-level
    /// component; `Ok` carries every allocation whose own attachment parts
    /// failed.
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError>;
    /// Latch quiescence, then set every `Active` TAP down and read it back.
    async fn quiesce_managed_taps(&self) -> Result<TapQuiescence>;
    /// Raise every quiesced activation-complete TAP and read it back, then
    /// clear the quiescence latch.
    async fn restore_quiesced_taps(&self) -> Result<()>;
}

/// The per-TAP result of one quiescence pass.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct TapQuiescence {
    /// Each allocation whose TAP could not be confirmed administratively down,
    /// with the typed cause. Every other `Active` allocation was read back down
    /// and is now `QuiescedActive`.
    pub unconfirmed: BTreeMap<AllocationId, GuestNetworkError>,
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
    BridgeObserve,
    BridgeConverge,
    BridgeDelete,
    TapObserve,
    TapCreate,
    TapAttachBridge,
    TapSetDown,
    TapSetUp,
    TapDelete,
    GuardTableCreate,
    GuardTableDelete,
    GuardChainCreate,
    GuardChainDelete,
    GuardSetCreate,
    GuardSetDelete,
    GuardRulesCreate,
    GuardRulesDelete,
    GuardMemberInsert,
    GuardMemberDelete,
    EndpointInsert,
    EndpointDelete,
    EndpointMapObserve,
    CounterMapObserve,
    TcxLoad,
    TcxAttach,
    TcxQuery,
    TcxDetach,
    EndpointMapPin,
    EndpointMapAdopt,
    EndpointMapUnpin,
    CounterMapPin,
    CounterMapAdopt,
    CounterMapUnpin,
    TcxLinkPin,
    TcxLinkAdopt,
    TcxLinkUnpin,
    CleanupComplement,
    SharedAudit,
    TcxEgressAttach,
    TcxEgressLinkPin,
    TcxEgressQuery,
    TcxEgressDetach,
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
    #[error("guest TCX operation {operation:?} failed")]
    Tcx {
        operation: GuestNetworkOperation,
        #[source]
        source: GuestTcxError,
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
}

/// Guest-network result alias.
pub type Result<T, E = GuestNetworkError> = std::result::Result<T, E>;

/// The server's process-local guest-address lease owner (D-295-R6). It is
/// intentionally not a repository.
///
/// Doc-hidden public so fixtures outside this crate's source can construct the
/// one pool a server composes (F-02 precedent); its lease operations stay
/// crate-private.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct GuestAddressPool {
    node_prefix: Ipv4Net,
    bridge: String,
    gateway: Ipv4Addr,
    dns: Ipv4Addr,
    held: Arc<parking_lot::Mutex<GuestAddressPoolState>>,
}

#[derive(Debug, Default)]
struct GuestAddressPoolState {
    plans: BTreeMap<AllocationId, GuestNetworkPlan>,
    addresses: BTreeSet<u32>,
    next_candidate: u32,
}

#[allow(
    dead_code,
    clippy::unused_self,
    reason = "GH #295 exact accepted scaffold; production construction lands in DELIVER"
)]
impl GuestAddressPool {
    /// Construct an empty pool over the node guest prefix, the bridge the
    /// shared owner is composed with, and the gateway and DNS addresses every
    /// assignment hands the guest.
    #[doc(hidden)]
    #[must_use]
    pub fn new(node_prefix: Ipv4Net, bridge: String, gateway: Ipv4Addr, dns: Ipv4Addr) -> Self {
        Self {
            node_prefix,
            bridge,
            gateway,
            dns,
            held: Arc::new(parking_lot::Mutex::new(GuestAddressPoolState {
                plans: BTreeMap::new(),
                addresses: BTreeSet::new(),
                next_candidate: u32::from(node_prefix.network()).saturating_add(1),
            })),
        }
    }

    pub(crate) fn assign(&self, alloc: AllocationId) -> Result<GuestNetworkPlan> {
        let mut state = self.held.lock();
        if let Some(plan) = state.plans.get(&alloc) {
            let plan = plan.clone();
            return Ok(plan);
        }

        let network = u32::from(self.node_prefix.network());
        let broadcast = u32::from(self.node_prefix.broadcast());
        let capacity = broadcast.saturating_sub(network).saturating_sub(2);
        let first = state.next_candidate.clamp(network.saturating_add(1), broadcast);
        let mut address = None;
        for candidate in (first..broadcast).chain(network.saturating_add(1)..first) {
            if candidate != u32::from(self.gateway) && !state.addresses.contains(&candidate) {
                address = Some(candidate);
                break;
            }
        }
        let Some(address) = address else {
            let held_count = u32::try_from(state.plans.len()).unwrap_or(u32::MAX);
            return Err(GuestNetworkError::PoolExhausted { held: held_count, capacity });
        };

        let address = Ipv4Addr::from(address);
        let assignment = GuestNetworkAssignment {
            address,
            tap: format!("ovd-tp-{:04x}", u32::from(address) - network),
            mac: [
                0x02,
                0x00,
                address.octets()[0],
                address.octets()[1],
                address.octets()[2],
                address.octets()[3],
            ],
            gateway: self.gateway,
            prefix: self.node_prefix.prefix_len(),
            dns: self.dns,
        };
        let plan = GuestNetworkPlan {
            alloc: alloc.clone(),
            bridge: self.bridge.clone(),
            node_prefix: self.node_prefix,
            assignment,
        };
        let address_u32 = u32::from(address);
        state.next_candidate = if address_u32.saturating_add(1) >= broadcast {
            network.saturating_add(1)
        } else {
            address_u32.saturating_add(1)
        };
        state.addresses.insert(address_u32);
        state.plans.insert(alloc, plan.clone());
        drop(state);
        Ok(plan)
    }

    pub(crate) fn release(&self, alloc: &AllocationId) {
        let mut state = self.held.lock();
        if let Some(plan) = state.plans.remove(alloc) {
            let address = u32::from(plan.assignment.address);
            state.addresses.remove(&address);
            state.next_candidate = state.next_candidate.min(address);
        }
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<AllocationId, GuestNetworkPlan> {
        self.held.lock().plans.clone()
    }

    /// Move an Admitted lease to Retiring; `true` only for that transition.
    /// An absent or already-Retiring lease returns `false` with no change, and
    /// Retiring never returns to Admitted. The lease still counts against the
    /// cap until [`Self::release`].
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 06-03")]
    pub(crate) fn retire(&self, alloc: &AllocationId) -> bool {
        let _ = alloc;
        todo!("RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03")
    }

    /// Occupancy (Admitted plus Retiring held, and the Retiring subset) and the
    /// leases of `allocs`, read in one lock acquisition. Read-only.
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 06-03")]
    pub(crate) fn observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation {
        let _ = allocs;
        todo!("RED scaffold: D-295-R8 GuestAddressPool::observe — DELIVER step 06-03")
    }
}

/// The production [`GuestAttachmentView`] the reconciler runtime lends each
/// hydration context (D-295-R8, ADR-0134). Its `observe` delegates to the
/// pool's one-snapshot `observe`.
///
/// RED scaffold (D-295-R8): until DELIVER 05-01 gives `AppState` the server's
/// pool, it wraps the process's static action pool; nothing calls it before
/// DELIVER step 07-03 hydrates occupancy through it.
pub(crate) struct ActionPoolAttachmentView;

impl GuestAttachmentView for ActionPoolAttachmentView {
    fn observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation {
        action_pool().observe(allocs)
    }
}

static ACTION_POOL: OnceLock<GuestAddressPool> = OnceLock::new();

fn action_pool() -> &'static GuestAddressPool {
    ACTION_POOL.get_or_init(|| {
        GuestAddressPool::new(
            Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 0), 16),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        )
    })
}

pub(crate) fn assign_action_plan(alloc: AllocationId) -> Result<GuestNetworkPlan> {
    action_pool().assign(alloc)
}

pub(crate) fn release_action_plan(alloc: &AllocationId) {
    action_pool().release(alloc);
}

pub(crate) fn action_plan(alloc: &AllocationId) -> Option<GuestNetworkPlan> {
    action_pool().snapshot().remove(alloc)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestNetworkScratchPlan {
    bridge: String,
    tap: String,
    node_prefix: Ipv4Net,
    assignment: GuestNetworkAssignment,
    endpoint_map_pin: PathBuf,
    counter_map_pin: PathBuf,
    tcx_link_pin: PathBuf,
    guard_table: String,
    guard_chain: String,
    guard_set: String,
    original_destination: SocketAddrV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "D14A private production validator is exercised by source-local tables"
)]
enum GuestNetworkTcpProbeRequirement {
    Classifier,
    OriginalDestination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "D14A private production observation is exercised by source-local tables"
)]
struct GuestNetworkTcpProbeObservation {
    verdict: GuestTcxProbeVerdict,
    mark: GuestTcxProbeMark,
    source_mac: Option<[u8; 6]>,
    destination_mac: Option<[u8; 6]>,
    original_destination: Option<SocketAddrV4>,
    counters: [GuestTcxProbeCounterObservation; 8],
}

#[allow(dead_code, reason = "D14A production mapping is activated in DELIVER")]
impl GuestNetworkTcpProbeObservation {
    fn from_outcome(outcome: &GuestTcxTcpProbeOutcome) -> Self {
        Self {
            verdict: outcome.verdict(),
            mark: outcome.mark(),
            source_mac: outcome.source_mac(),
            destination_mac: outcome.destination_mac(),
            original_destination: outcome.original_destination(),
            counters: *outcome.counters(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "D14A exact private mismatch vocabulary precedes implementation")]
enum GuestNetworkTcpProbeMismatch {
    Verdict { observed: GuestTcxProbeVerdict },
    Mark { observed: GuestTcxProbeMark },
    SourceMac { expected: [u8; 6], observed: Option<[u8; 6]> },
    DestinationMac { expected: [u8; 6], observed: Option<[u8; 6]> },
    OriginalDestination { expected: SocketAddrV4, observed: Option<SocketAddrV4> },
    CounterIdentity { index: u8, expected: GuestTcxCounter, observed: GuestTcxCounter },
    CounterDecrease { counter: GuestTcxCounter, before: u64, after: u64 },
    CounterDelta { counter: GuestTcxCounter, expected: u64, observed: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "D14A private validation result precedes implementation")]
enum GuestNetworkTcpProbeValidation {
    Passed { intercept_before: u64, intercept_after: u64 },
    Mismatch(GuestNetworkTcpProbeMismatch),
}

fn validate_guest_tcx_tcp_probe(
    requirement: GuestNetworkTcpProbeRequirement,
    expected: &GuestTcxTcpProbeInput,
    observed: std::result::Result<GuestNetworkTcpProbeObservation, GuestTcxError>,
) -> std::io::Result<GuestNetworkTcpProbeValidation> {
    let observed = observed.map_err(std::io::Error::other)?;
    if observed.verdict != GuestTcxProbeVerdict::Accept {
        return Ok(GuestNetworkTcpProbeValidation::Mismatch(
            GuestNetworkTcpProbeMismatch::Verdict { observed: observed.verdict },
        ));
    }
    if observed.mark != GuestTcxProbeMark::Intercept {
        return Ok(GuestNetworkTcpProbeValidation::Mismatch(GuestNetworkTcpProbeMismatch::Mark {
            observed: observed.mark,
        }));
    }
    if observed.source_mac != Some(expected.source_mac) {
        return Ok(GuestNetworkTcpProbeValidation::Mismatch(
            GuestNetworkTcpProbeMismatch::SourceMac {
                expected: expected.source_mac,
                observed: observed.source_mac,
            },
        ));
    }
    if observed.destination_mac != Some(expected.bridge_mac) {
        return Ok(GuestNetworkTcpProbeValidation::Mismatch(
            GuestNetworkTcpProbeMismatch::DestinationMac {
                expected: expected.bridge_mac,
                observed: observed.destination_mac,
            },
        ));
    }
    if matches!(requirement, GuestNetworkTcpProbeRequirement::OriginalDestination)
        && observed.original_destination != Some(expected.original_destination)
    {
        return Ok(GuestNetworkTcpProbeValidation::Mismatch(
            GuestNetworkTcpProbeMismatch::OriginalDestination {
                expected: expected.original_destination,
                observed: observed.original_destination,
            },
        ));
    }
    let expected_counters = [
        GuestTcxCounter::GatewayHostPass,
        GuestTcxCounter::Intercept,
        GuestTcxCounter::EndpointMapMiss,
        GuestTcxCounter::SourceMacSpoof,
        GuestTcxCounter::SourceIpArpSpoof,
        GuestTcxCounter::DirectBypassDrop,
        GuestTcxCounter::ArpPass,
        GuestTcxCounter::MalformedDrop,
    ];
    for (index, (expected_counter, actual)) in
        expected_counters.into_iter().zip(observed.counters).enumerate()
    {
        let index = u8::try_from(index).unwrap_or_default();
        if actual.counter != expected_counter {
            return Ok(GuestNetworkTcpProbeValidation::Mismatch(
                GuestNetworkTcpProbeMismatch::CounterIdentity {
                    index,
                    expected: expected_counter,
                    observed: actual.counter,
                },
            ));
        }
        let Some(delta) = actual.after.checked_sub(actual.before) else {
            return Ok(GuestNetworkTcpProbeValidation::Mismatch(
                GuestNetworkTcpProbeMismatch::CounterDecrease {
                    counter: actual.counter,
                    before: actual.before,
                    after: actual.after,
                },
            ));
        };
        let expected_delta = u64::from(actual.counter == GuestTcxCounter::Intercept);
        if delta != expected_delta {
            return Ok(GuestNetworkTcpProbeValidation::Mismatch(
                GuestNetworkTcpProbeMismatch::CounterDelta {
                    counter: actual.counter,
                    expected: expected_delta,
                    observed: delta,
                },
            ));
        }
    }
    Ok(GuestNetworkTcpProbeValidation::Passed {
        intercept_before: observed.counters[1].before,
        intercept_after: observed.counters[1].after,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "D5 exact RED vocabulary is consumed by pending tests")]
enum GuestNetworkScratchNetlinkAction {
    ConvergeBridge,
    CreateTap,
    AttachTapToBridge,
    SetTapUp,
    CreateGuardTable,
    CreateGuardChain,
    CreateGuardSet,
    CreateGuardRules,
    InsertGuardMember,
    SetTapDown,
    DeleteTap,
    DeleteGuardMember,
    DeleteGuardRules,
    DeleteGuardSet,
    DeleteGuardChain,
    DeleteGuardTable,
    DeleteBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "D5 exact RED vocabulary is consumed by pending tests")]
enum GuestNetworkScratchTcxAction {
    LoadProgramAndMaps,
    PinEndpointMap,
    PinCounterMap,
    InsertEndpoint,
    AttachLink,
    PinLink,
    AdoptEndpointMap,
    AdoptCounterMap,
    AdoptLink,
    QueryLink,
    DeleteEndpoint,
    UnpinLink,
    DetachLink,
    UnpinCounterMap,
    UnpinEndpointMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "D5 exact RED vocabulary is consumed by pending tests")]
enum GuestNetworkScratchNetlinkResource {
    Bridge,
    Tap,
    BridgeGuardTable,
    BridgeGuardChain,
    BridgeGuardSet,
    BridgeGuardRule,
    BridgeGuardMember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "D5 exact RED vocabulary is consumed by pending tests")]
enum GuestNetworkScratchTcxResource {
    EndpointMap,
    CounterMap,
    EndpointEntry,
    TcxProgram,
    TcxLink,
    EndpointMapPin,
    CounterMapPin,
    TcxLinkPin,
}

#[async_trait::async_trait]
#[allow(dead_code, reason = "D5 exact RED effect seam is consumed by pending tests")]
trait SharedGuestNetworkScratchIo: Send + Sync {
    async fn apply_netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchNetlinkAction,
    ) -> std::result::Result<(), NetlinkError>;

    async fn apply_tcx(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchTcxAction,
    ) -> std::result::Result<(), GuestTcxError>;

    fn close_loader_handles(&self, plan: &GuestNetworkScratchPlan);
    fn release_adopted_handles(&self, plan: &GuestNetworkScratchPlan);
    fn probe_program_id(&self) -> Option<u32>;
    fn probe_attachment(
        &self,
        plan: &GuestNetworkScratchPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError>;

    async fn exercise(
        &self,
        plan: &GuestNetworkScratchPlan,
        stage: GuestNetworkProbeStage,
    ) -> std::io::Result<bool>;

    async fn count_netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchNetlinkResource,
    ) -> std::result::Result<u32, NetlinkError>;

    async fn count_tcx(
        &self,
        plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchTcxResource,
    ) -> std::result::Result<u32, GuestTcxError>;
}

struct RealSharedGuestNetworkScratchIo {
    program: parking_lot::Mutex<Option<GuestTcxProgram>>,
    probe_program_id: parking_lot::Mutex<Option<u32>>,
    inventory: parking_lot::Mutex<Option<GuestTcxInventoryIdentity>>,
    adopted: parking_lot::Mutex<Option<overdrive_dataplane::guest_tcx::GuestTcxAdoptedState>>,
    pending_link: parking_lot::Mutex<Option<overdrive_dataplane::guest_tcx::GuestTcxLink>>,
}

impl RealSharedGuestNetworkScratchIo {
    fn new() -> Self {
        Self {
            program: parking_lot::Mutex::new(None),
            probe_program_id: parking_lot::Mutex::new(None),
            inventory: parking_lot::Mutex::new(None),
            adopted: parking_lot::Mutex::new(None),
            pending_link: parking_lot::Mutex::new(None),
        }
    }

    fn guard_spec(plan: &GuestNetworkScratchPlan) -> Result<BridgeGuardSpec, BridgeGuardError> {
        BridgeGuardSpec::new(
            plan.guard_table.clone(),
            plan.guard_chain.clone(),
            plan.guard_set.clone(),
            -300,
            0x295a,
            0x295b,
        )
        .map_err(BridgeGuardError::Validation)
    }

    fn ifindex(plan: &GuestNetworkScratchPlan) -> std::io::Result<u32> {
        let value = std::fs::read_to_string(format!("/sys/class/net/{}/ifindex", plan.tap))?;
        value.trim().parse().map_err(std::io::Error::other)
    }

    #[cfg(target_os = "linux")]
    #[allow(
        unsafe_code,
        clippy::items_after_statements,
        clippy::redundant_closure,
        clippy::too_many_lines,
        clippy::unused_self,
        reason = "D14 private TAP ioctl/socket host-adapter boundary"
    )]
    fn detached_guard_packet(
        &self,
        plan: &GuestNetworkScratchPlan,
    ) -> std::io::Result<(u64, u64, u64, u64, u64, u64)> {
        let guard =
            Self::guard_spec(plan).map_err(|error| std::io::Error::other(error.to_string()))?;
        let expected_members = BTreeSet::from([plan.tap.clone()]);
        let counter = |observation: BridgeGuardObservation| {
            let BridgeGuardObservation::Exact { inventory } = observation else {
                return Err(std::io::Error::other("detached guard inventory is not exact"));
            };
            inventory
                .rules
                .into_iter()
                .find_map(|rule| {
                    (matches!(
                        rule.fact.identity,
                        overdrive_netlink::nft::bridge::BridgeGuardRuleIdentity::Owned(
                            overdrive_netlink::nft::bridge::BridgeGuardRuleKind::DefaultDrop
                        )
                    ))
                    .then(|| rule.counter)
                })
                .flatten()
                .map(|counter| (counter.packets, counter.bytes))
                .ok_or_else(|| std::io::Error::other("detached guard counter is unavailable"))
        };
        let udp = UdpSocket::bind(SocketAddrV4::new(plan.assignment.gateway, 0))?;
        udp.set_nonblocking(true)?;
        let interface = std::ffi::CString::new(plan.bridge.clone()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bridge contains NUL")
        })?;
        // SAFETY: the socket is owned by this function and the interface bytes
        // remain live for the complete setsockopt call.
        let bind_result = unsafe {
            libc::setsockopt(
                udp.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                interface.as_ptr().cast(),
                libc::socklen_t::try_from(interface.as_bytes_with_nul().len()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "interface name too long")
                })?,
            )
        };
        if bind_result < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let local_port = match udp.local_addr()? {
            std::net::SocketAddr::V4(address) if address.port() != 0 => address.port(),
            _ => return Err(std::io::Error::other("kernel did not allocate a UDP probe port")),
        };
        let before_guard = counter(
            overdrive_netlink::nft::bridge::observe(&guard, &expected_members)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
        )?;
        let before_classifier = overdrive_dataplane::guest_tcx::read_counter(
            &plan.counter_map_pin,
            GuestTcxCounter::Intercept,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut file = OpenOptions::new().read(true).write(true).open("/dev/net/tun")?;
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        let name = plan.tap.as_bytes();
        if name.is_empty() || name.len() >= libc::IFNAMSIZ {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid TAP name"));
        }
        // SAFETY: request is initialized storage and the validated name fits
        // the kernel ifreq name field.
        unsafe {
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                request.ifr_name.as_mut_ptr().cast::<u8>(),
                name.len(),
            );
            request.ifr_ifru.ifru_flags = libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI)
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid TAP flags")
                })?;
            d14_tun_set_iff(file.as_raw_fd(), &raw const request)
                .map_err(|errno| std::io::Error::from(errno))?;
        }
        const MARKER: &[u8] = b"nd295-d14-detached";
        let mut frame = vec![0_u8; 14 + 20 + 8 + MARKER.len()];
        frame[..6].copy_from_slice(&[0xff; 6]);
        frame[6..12].copy_from_slice(&plan.assignment.mac);
        frame[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
        frame[14] = 0x45;
        frame[16..18].copy_from_slice(
            &u16::try_from(20 + 8 + MARKER.len()).unwrap_or(u16::MAX).to_be_bytes(),
        );
        frame[22] = 64;
        frame[23] = 17;
        frame[26..30].copy_from_slice(&plan.assignment.address.octets());
        frame[30..34].copy_from_slice(&plan.assignment.gateway.octets());
        frame[34..36].copy_from_slice(&49_295_u16.to_be_bytes());
        frame[36..38].copy_from_slice(&local_port.to_be_bytes());
        frame[42..].copy_from_slice(MARKER);
        enable_bridge_nf_call_iptables(&plan.bridge)?;
        for _ in 0..64 {
            std::thread::yield_now();
        }
        file.write_all(&frame)?;
        enable_bridge_nf_call_iptables(&plan.bridge)?;
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let after_guard = counter(
                overdrive_netlink::nft::bridge::observe(&guard, &expected_members)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            )?;
            let after_classifier = overdrive_dataplane::guest_tcx::read_counter(
                &plan.counter_map_pin,
                GuestTcxCounter::Intercept,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            let mut datagram = [0_u8; 2048];
            match udp.recv(&mut datagram) {
                Ok(length) => {
                    if datagram[..length].windows(MARKER.len()).any(|window| window == MARKER) {
                        return Err(std::io::Error::other("detached marker reached host UDP"));
                    }
                    return Err(std::io::Error::other("unexpected host UDP datagram"));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            if after_classifier == before_classifier
                && after_guard.0 == before_guard.0 + 1
                && after_guard.1 > before_guard.1
            {
                return Ok((
                    before_classifier,
                    after_classifier,
                    before_guard.0,
                    after_guard.0,
                    before_guard.1,
                    after_guard.1,
                ));
            }
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "detached guard packet did not reach the exact drop transition",
                ));
            }
            std::thread::yield_now();
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn detached_guard_packet(
        &self,
        _plan: &GuestNetworkScratchPlan,
    ) -> std::io::Result<(u64, u64, u64, u64, u64, u64)> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "detached TAP packet probe requires Linux",
        ))
    }

    fn ensure_adopted_for_cleanup(
        &self,
        plan: &GuestNetworkScratchPlan,
    ) -> Result<(), GuestTcxError> {
        if self.adopted.lock().is_some() {
            return Ok(());
        }
        let identity = self.inventory.lock().clone().ok_or(GuestTcxError::CaptureUnavailable {
            family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
        })?;
        *self.adopted.lock() =
            Some(overdrive_dataplane::guest_tcx::GuestTcxAdoptedState::for_inventory(
                &identity,
                plan.tcx_link_pin.clone(),
            ));
        Ok(())
    }
}

#[async_trait::async_trait]
#[allow(
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    reason = "GH #295 staged owner algorithm: each port call is one D12A-ordered sequence; DELIVER 06-02/06-04 restructure these bodies to the R4/R5/R21/R22 contracts; the lifecycle and scratch-state guards serialize each whole owner operation by design (D12A); releasing them early would admit interleaved owner calls"
)]
impl SharedGuestNetworkScratchIo for RealSharedGuestNetworkScratchIo {
    async fn apply_netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchNetlinkAction,
    ) -> std::result::Result<(), NetlinkError> {
        match action {
            GuestNetworkScratchNetlinkAction::ConvergeBridge => {
                let client = overdrive_netlink::Client::new()?;
                client
                    .ensure_bridge(&plan.bridge, overdrive_core::dataplane::GUEST_BRIDGE_MAC)
                    .await?;
                client.set_link_down(&plan.bridge).await?;
                client
                    .set_link_mac(&plan.bridge, overdrive_core::dataplane::GUEST_BRIDGE_MAC)
                    .await?;
                client
                    .converge_addr(&plan.bridge, plan.assignment.gateway, plan.assignment.prefix)
                    .await?;
                client.set_link_up(&plan.bridge).await
            }
            GuestNetworkScratchNetlinkAction::CreateTap => {
                overdrive_netlink::create_persistent_tap(
                    &plan.tap,
                    overdrive_core::vm::config::OVERDRIVE_VMM_UID,
                )
            }
            GuestNetworkScratchNetlinkAction::AttachTapToBridge => {
                overdrive_netlink::Client::new()?.set_link_master(&plan.tap, &plan.bridge).await
            }
            GuestNetworkScratchNetlinkAction::SetTapUp => {
                overdrive_netlink::Client::new()?.set_link_up(&plan.tap).await
            }
            GuestNetworkScratchNetlinkAction::SetTapDown => {
                overdrive_netlink::Client::new()?.set_link_down(&plan.tap).await
            }
            GuestNetworkScratchNetlinkAction::DeleteTap => {
                overdrive_netlink::Client::new()?.del_link(&plan.tap).await
            }
            GuestNetworkScratchNetlinkAction::DeleteBridge => {
                overdrive_netlink::Client::new()?.del_link(&plan.bridge).await
            }
            GuestNetworkScratchNetlinkAction::CreateGuardTable => {
                enable_bridge_nf_call_iptables(&plan.bridge).map_err(NetlinkError::connect)?;
                overdrive_netlink::nft::bridge::converge_table(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft("guard-table", std::io::Error::other(error.to_string()))
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-table", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::CreateGuardChain => {
                overdrive_netlink::nft::bridge::converge_chain(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft("guard-chain", std::io::Error::other(error.to_string()))
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-chain", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::CreateGuardSet => {
                overdrive_netlink::nft::bridge::converge_set(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft("guard-set", std::io::Error::other(error.to_string()))
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-set", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::CreateGuardRules => {
                overdrive_netlink::nft::bridge::converge_rules(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft("guard-rules", std::io::Error::other(error.to_string()))
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-rules", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::InsertGuardMember => {
                overdrive_netlink::nft::bridge::insert_member(
                    &Self::guard_spec(plan).map_err(|error| {
                        NetlinkError::nft("guard-member", std::io::Error::other(error.to_string()))
                    })?,
                    &plan.tap,
                )
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-member", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::DeleteGuardMember => {
                overdrive_netlink::nft::bridge::delete_member(
                    &Self::guard_spec(plan).map_err(|error| {
                        NetlinkError::nft(
                            "guard-member-delete",
                            std::io::Error::other(error.to_string()),
                        )
                    })?,
                    &plan.tap,
                )
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft(
                        "guard-member-delete",
                        std::io::Error::other(error.to_string()),
                    )
                })
            }
            GuestNetworkScratchNetlinkAction::DeleteGuardRules => {
                overdrive_netlink::nft::bridge::delete_rules(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft(
                            "guard-rules-delete",
                            std::io::Error::other(error.to_string()),
                        )
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft(
                        "guard-rules-delete",
                        std::io::Error::other(error.to_string()),
                    )
                })
            }
            GuestNetworkScratchNetlinkAction::DeleteGuardSet => {
                overdrive_netlink::nft::bridge::delete_set(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft(
                            "guard-set-delete",
                            std::io::Error::other(error.to_string()),
                        )
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft("guard-set-delete", std::io::Error::other(error.to_string()))
                })
            }
            GuestNetworkScratchNetlinkAction::DeleteGuardChain => {
                overdrive_netlink::nft::bridge::delete_chain(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft(
                            "guard-chain-delete",
                            std::io::Error::other(error.to_string()),
                        )
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft(
                        "guard-chain-delete",
                        std::io::Error::other(error.to_string()),
                    )
                })
            }
            GuestNetworkScratchNetlinkAction::DeleteGuardTable => {
                overdrive_netlink::nft::bridge::delete_table(&Self::guard_spec(plan).map_err(
                    |error| {
                        NetlinkError::nft(
                            "guard-table-delete",
                            std::io::Error::other(error.to_string()),
                        )
                    },
                )?)
                .map(|_| ())
                .map_err(|error| {
                    NetlinkError::nft(
                        "guard-table-delete",
                        std::io::Error::other(error.to_string()),
                    )
                })
            }
        }
    }

    async fn apply_tcx(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchTcxAction,
    ) -> std::result::Result<(), GuestTcxError> {
        match action {
            GuestNetworkScratchTcxAction::LoadProgramAndMaps => {
                // The probe namespace is a fixed, owner-controlled scratch
                // namespace. A prior process can die after pinning a link or
                // map but before the reverse cleanup runs; reclaim those
                // exact pins before loading the next isolated probe. This is
                // bounded to the scratch paths and never touches the
                // allocation-owned shared-switch hierarchy.
                if plan.tcx_link_pin.exists() {
                    overdrive_dataplane::guest_tcx::detach_pinned_link(&plan.tcx_link_pin)?;
                }
                for pin in [&plan.endpoint_map_pin, &plan.counter_map_pin] {
                    match tokio::fs::remove_file(pin).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(source) => return Err(GuestTcxError::Io { source }),
                    }
                }
                let capture = GuestTcxInventoryIdentity::capture(
                    plan.endpoint_map_pin.clone(),
                    plan.counter_map_pin.clone(),
                );
                let (identity, disposition) = capture.into_parts();
                // Retain the identity before moving the one genuine capture
                // error.  The D5 cleanup complement must still observe every
                // failed domain as source-less unavailable.
                *self.inventory.lock() = Some(identity.clone());
                disposition?;
                let program = GuestTcxProgram::load(&identity)?;
                *self.program.lock() = Some(program);
                Ok(())
            }
            GuestNetworkScratchTcxAction::PinEndpointMap => self
                .program
                .lock()
                .as_mut()
                .ok_or_else(|| GuestTcxError::ObjectMissing {
                    object: overdrive_dataplane::guest_tcx::GuestTcxObject::EndpointMap,
                })?
                .pin_endpoint_map(&plan.endpoint_map_pin)
                .map(|_| ()),
            GuestNetworkScratchTcxAction::PinCounterMap => self
                .program
                .lock()
                .as_mut()
                .ok_or_else(|| GuestTcxError::ObjectMissing {
                    object: overdrive_dataplane::guest_tcx::GuestTcxObject::CounterMap,
                })?
                .pin_counter_map(&plan.counter_map_pin)
                .map(|_| ()),
            GuestNetworkScratchTcxAction::InsertEndpoint => {
                let ifindex = Self::ifindex(plan).map_err(|source| GuestTcxError::Io { source })?;
                self.program
                    .lock()
                    .as_mut()
                    .ok_or_else(|| GuestTcxError::ObjectMissing {
                        object: overdrive_dataplane::guest_tcx::GuestTcxObject::EndpointMap,
                    })?
                    .insert_endpoint(
                        ifindex,
                        GuestTcxEndpoint {
                            source_ipv4: plan.assignment.address,
                            source_mac: plan.assignment.mac,
                            bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                        },
                    )
            }
            GuestNetworkScratchTcxAction::AttachLink => {
                let link = self
                    .program
                    .lock()
                    .as_mut()
                    .ok_or_else(|| GuestTcxError::ObjectMissing {
                        object: overdrive_dataplane::guest_tcx::GuestTcxObject::Classifier,
                    })?
                    .attach_first_ingress(&plan.tap)?;
                *self.probe_program_id.lock() = Some(link.program_id());
                *self.pending_link.lock() = Some(link);
                Ok(())
            }
            GuestNetworkScratchTcxAction::PinLink => {
                let link = self.pending_link.lock().take().ok_or_else(|| {
                    GuestTcxError::ObjectMissing {
                        object: overdrive_dataplane::guest_tcx::GuestTcxObject::Classifier,
                    }
                })?;
                link.pin(&plan.tcx_link_pin)
            }
            GuestNetworkScratchTcxAction::AdoptEndpointMap
            | GuestNetworkScratchTcxAction::AdoptCounterMap
            | GuestNetworkScratchTcxAction::AdoptLink => {
                if self.adopted.lock().is_none() {
                    let identity = self.inventory.lock().clone().ok_or_else(|| {
                        GuestTcxError::CaptureUnavailable {
                            family:
                                overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
                        }
                    })?;
                    *self.adopted.lock() =
                        Some(overdrive_dataplane::guest_tcx::GuestTcxAdoptedState::for_inventory(
                            &identity,
                            plan.tcx_link_pin.clone(),
                        ));
                }
                let mut adopted = self.adopted.lock();
                let Some(adopted) = adopted.as_mut() else {
                    return Err(GuestTcxError::CaptureUnavailable {
                        family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
                    });
                };
                match action {
                    GuestNetworkScratchTcxAction::AdoptEndpointMap => {
                        adopted.adopt_endpoint_map().map(|_| ())
                    }
                    GuestNetworkScratchTcxAction::AdoptCounterMap => {
                        adopted.adopt_counter_map().map(|_| ())
                    }
                    _ => adopted.adopt_link(),
                }
            }
            GuestNetworkScratchTcxAction::QueryLink => {
                let attachment = overdrive_dataplane::guest_tcx::query_attachment(
                    &plan.tap,
                    TcxAttachPoint::Ingress,
                )?;
                if attachment.program_ids.is_empty() {
                    Err(GuestTcxError::InventoryAmbiguous {
                        family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
                    })
                } else {
                    Ok(())
                }
            }
            GuestNetworkScratchTcxAction::DeleteEndpoint => {
                overdrive_dataplane::guest_tcx::remove_endpoint(
                    &plan.endpoint_map_pin,
                    Self::ifindex(plan).map_err(|source| GuestTcxError::Io { source })?,
                )
            }
            GuestNetworkScratchTcxAction::UnpinLink => {
                let pin = &plan.tcx_link_pin;
                if !pin.exists() {
                    return Ok(());
                }
                if self.adopted.lock().is_none() {
                    return overdrive_dataplane::guest_tcx::detach_pinned_link(pin);
                }
                let result = self
                    .adopted
                    .lock()
                    .as_mut()
                    .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                        family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
                    })?
                    .unpin_link();
                match result {
                    Ok(Some(link)) => {
                        *self.pending_link.lock() = Some(link);
                        Ok(())
                    }
                    Ok(None) => overdrive_dataplane::guest_tcx::detach_pinned_link(pin),
                    Err(source) => Err(source),
                }
            }
            GuestNetworkScratchTcxAction::DetachLink => {
                let pending = self.pending_link.lock().take();
                pending.map_or_else(|| Ok(()), GuestTcxLink::detach)
            }
            GuestNetworkScratchTcxAction::UnpinCounterMap => {
                self.ensure_adopted_for_cleanup(plan)?;
                self.adopted
                    .lock()
                    .as_mut()
                    .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                        family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMap,
                    })?
                    .unpin_counter_map()
            }
            GuestNetworkScratchTcxAction::UnpinEndpointMap => {
                self.ensure_adopted_for_cleanup(plan)?;
                self.adopted
                    .lock()
                    .as_mut()
                    .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                        family:
                            overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMap,
                    })?
                    .unpin_endpoint_map()
            }
        }
    }

    fn close_loader_handles(&self, _plan: &GuestNetworkScratchPlan) {
        self.program.lock().take();
    }

    fn probe_program_id(&self) -> Option<u32> {
        (*self.probe_program_id.lock()).as_ref().copied()
    }

    fn probe_attachment(
        &self,
        plan: &GuestNetworkScratchPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
        overdrive_dataplane::guest_tcx::query_attachment(&plan.tap, TcxAttachPoint::Ingress)
    }

    fn release_adopted_handles(&self, _plan: &GuestNetworkScratchPlan) {
        self.adopted.lock().take();
    }

    async fn exercise(
        &self,
        plan: &GuestNetworkScratchPlan,
        stage: GuestNetworkProbeStage,
    ) -> std::io::Result<bool> {
        let ifindex = Self::ifindex(plan)?;
        match stage {
            GuestNetworkProbeStage::Classifier | GuestNetworkProbeStage::OriginalDestination => {
                let requirement = match stage {
                    GuestNetworkProbeStage::Classifier => {
                        GuestNetworkTcpProbeRequirement::Classifier
                    }
                    GuestNetworkProbeStage::OriginalDestination => {
                        GuestNetworkTcpProbeRequirement::OriginalDestination
                    }
                    GuestNetworkProbeStage::DetachedLinkGuard => unreachable!(),
                };
                let program_id = (*self.probe_program_id.lock())
                    .ok_or_else(|| std::io::Error::other("classifier identity is unavailable"))?;
                let source_mac = plan.assignment.mac;
                let peer_input = GuestTcxTcpProbeInput {
                    ingress_ifindex: ifindex,
                    source_ipv4: plan.assignment.address,
                    source_mac,
                    bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                    destination_mac: [0x02, 0x00, 100, 95, 255, 253],
                    original_destination: plan.original_destination,
                };
                let gateway_input = GuestTcxTcpProbeInput {
                    destination_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                    original_destination: SocketAddrV4::new(plan.assignment.gateway, 8443),
                    ..peer_input
                };
                let program = self.program.lock();
                let program = program
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("classifier loader handle is closed"))?;
                let peer = GuestNetworkTcpProbeObservation::from_outcome(
                    &program.probe_tcp_intercept(peer_input).map_err(std::io::Error::other)?,
                );
                let gateway = GuestNetworkTcpProbeObservation::from_outcome(
                    &program.probe_tcp_intercept(gateway_input).map_err(std::io::Error::other)?,
                );
                let peer_validation =
                    validate_guest_tcx_tcp_probe(requirement, &peer_input, Ok(peer))?;
                let gateway_validation =
                    validate_guest_tcx_tcp_probe(requirement, &gateway_input, Ok(gateway))?;
                let (
                    GuestNetworkTcpProbeValidation::Passed {
                        intercept_before: peer_before,
                        intercept_after: peer_after,
                    },
                    GuestNetworkTcpProbeValidation::Passed {
                        intercept_before: gateway_before,
                        intercept_after: gateway_after,
                    },
                ) = (peer_validation, gateway_validation)
                else {
                    return Ok(false);
                };
                tracing::info!(
                    event = "guest_network.shared_owner_startup_probe_tcp_stage_completed",
                    stage = match stage {
                        GuestNetworkProbeStage::Classifier => "classifier",
                        GuestNetworkProbeStage::OriginalDestination => "original_destination",
                        GuestNetworkProbeStage::DetachedLinkGuard => unreachable!(),
                    },
                    program_id,
                    peer_intercept_before = peer_before,
                    peer_intercept_after = peer_after,
                    gateway_intercept_before = gateway_before,
                    gateway_intercept_after = gateway_after,
                );
                Ok(true)
            }
            GuestNetworkProbeStage::DetachedLinkGuard => {
                let attachment = overdrive_dataplane::guest_tcx::query_attachment(
                    &plan.tap,
                    TcxAttachPoint::Ingress,
                )
                .map_err(|error| std::io::Error::other(error.to_string()))?;
                if !attachment.program_ids.is_empty() {
                    return Ok(false);
                }
                let (
                    classifier_before,
                    classifier_after,
                    guard_packets_before,
                    guard_packets_after,
                    guard_bytes_before,
                    guard_bytes_after,
                ) = self.detached_guard_packet(plan)?;
                let passed = classifier_before == classifier_after
                    && guard_packets_after == guard_packets_before + 1
                    && guard_bytes_after > guard_bytes_before;
                if passed {
                    let program_id = (*self.probe_program_id.lock()).unwrap_or_default();
                    tracing::info!(
                        event = "guest_network.shared_owner_startup_probe_detached_guard_completed",
                        program_id,
                        classifier_intercept_before = classifier_before,
                        classifier_intercept_after = classifier_after,
                        guard_packets_before,
                        guard_packets_after,
                        guard_bytes_before,
                        guard_bytes_after,
                        host_datagrams = 0_u64,
                    );
                }
                Ok(passed)
            }
        }
    }

    async fn count_netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchNetlinkResource,
    ) -> std::result::Result<u32, NetlinkError> {
        let client = overdrive_netlink::Client::new()?;
        let count = match resource {
            GuestNetworkScratchNetlinkResource::Bridge => {
                u32::from(client.observe_link(&plan.bridge).await?.is_some())
            }
            GuestNetworkScratchNetlinkResource::Tap => {
                u32::from(client.observe_link(&plan.tap).await?.is_some())
            }
            GuestNetworkScratchNetlinkResource::BridgeGuardTable
            | GuestNetworkScratchNetlinkResource::BridgeGuardChain
            | GuestNetworkScratchNetlinkResource::BridgeGuardSet
            | GuestNetworkScratchNetlinkResource::BridgeGuardRule
            | GuestNetworkScratchNetlinkResource::BridgeGuardMember => {
                let guard = Self::guard_spec(plan).map_err(|error| {
                    NetlinkError::nft("guard-observe", std::io::Error::other(error.to_string()))
                })?;
                let expected_members = BTreeSet::from([plan.tap.clone()]);
                let observation = overdrive_netlink::nft::bridge::observe(
                    &guard,
                    &expected_members,
                )
                .map_err(|error| {
                    NetlinkError::nft("guard-observe", std::io::Error::other(error.to_string()))
                })?;
                match observation {
                    BridgeGuardObservation::Absent { .. } => 0,
                    BridgeGuardObservation::Exact { inventory }
                    | BridgeGuardObservation::Conflict { inventory } => match resource {
                        GuestNetworkScratchNetlinkResource::BridgeGuardTable => {
                            u32::try_from(inventory.tables.len()).unwrap_or(u32::MAX)
                        }
                        GuestNetworkScratchNetlinkResource::BridgeGuardChain => {
                            u32::try_from(inventory.chains.len()).unwrap_or(u32::MAX)
                        }
                        GuestNetworkScratchNetlinkResource::BridgeGuardSet => {
                            u32::try_from(inventory.sets.len()).unwrap_or(u32::MAX)
                        }
                        GuestNetworkScratchNetlinkResource::BridgeGuardRule => {
                            u32::try_from(inventory.rules.len()).unwrap_or(u32::MAX)
                        }
                        GuestNetworkScratchNetlinkResource::BridgeGuardMember => {
                            u32::try_from(inventory.members.len()).unwrap_or(u32::MAX)
                        }
                        _ => 0,
                    },
                }
            }
        };
        Ok(count)
    }

    async fn count_tcx(
        &self,
        _plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchTcxResource,
    ) -> std::result::Result<u32, GuestTcxError> {
        let inventory =
            self.inventory.lock().clone().ok_or_else(|| GuestTcxError::CaptureUnavailable {
                family: match resource {
                    GuestNetworkScratchTcxResource::EndpointMap => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMap
                    }
                    GuestNetworkScratchTcxResource::CounterMap => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMap
                    }
                    GuestNetworkScratchTcxResource::EndpointEntry => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointEntry
                    }
                    GuestNetworkScratchTcxResource::TcxProgram => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxProgram
                    }
                    GuestNetworkScratchTcxResource::TcxLink => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink
                    }
                    GuestNetworkScratchTcxResource::EndpointMapPin => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMapPin
                    }
                    GuestNetworkScratchTcxResource::CounterMapPin => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMapPin
                    }
                    GuestNetworkScratchTcxResource::TcxLinkPin => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLinkPin
                    }
                },
            })?;
        match resource {
            GuestNetworkScratchTcxResource::EndpointMap => inventory.observe_endpoint_maps(),
            GuestNetworkScratchTcxResource::CounterMap => inventory.observe_counter_maps(),
            GuestNetworkScratchTcxResource::EndpointEntry => inventory.observe_endpoint_entries(),
            GuestNetworkScratchTcxResource::TcxProgram => inventory.observe_tcx_programs(),
            GuestNetworkScratchTcxResource::TcxLink => inventory.observe_tcx_links(),
            GuestNetworkScratchTcxResource::EndpointMapPin => inventory.observe_endpoint_map_pins(),
            GuestNetworkScratchTcxResource::CounterMapPin => inventory.observe_counter_map_pins(),
            GuestNetworkScratchTcxResource::TcxLinkPin => inventory.observe_tcx_link_pins(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "D12A exact private RED observation")]
enum GuestNetworkAllocationTapObservation {
    Absent {
        name: String,
    },
    Incompatible {
        name: String,
        ifindex: u32,
        kind: GuestLinkKind,
        persistent: Option<bool>,
        up: bool,
        owner_uid: Option<u32>,
        master_ifindex: Option<u32>,
    },
    Persistent {
        name: String,
        ifindex: u32,
        up: bool,
        owner_uid: Option<u32>,
        master_ifindex: Option<u32>,
        /// Projected from `ObservedLinkIdentity::mac` (D-295-R21).
        mac: Option<[u8; 6]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "D12A exact private RED observation")]
enum GuestNetworkAllocationBridgeObservation {
    Absent { name: String },
    Present { name: String, ifindex: u32, kind: GuestLinkKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestNetworkNodeTcxAuditRead {
    Programs,
    EndpointMap,
    CounterMap,
    EndpointMapPin,
    CounterMapPin,
    EndpointEntries,
}

#[async_trait::async_trait]
#[allow(dead_code, reason = "D12A exact private RED leaf boundary")]
trait GuestNetworkAllocationIo: Send + Sync {
    async fn create_tap(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError>;
    async fn attach_tap_to_bridge(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), NetlinkError>;
    async fn set_tap_up(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError>;
    async fn set_tap_down(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError>;
    async fn delete_tap(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError>;
    async fn observe_tap(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestNetworkAllocationTapObservation, NetlinkError>;
    async fn observe_bridge(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestNetworkAllocationBridgeObservation, NetlinkError>;
    async fn observe_shared_bridge(
        &self,
    ) -> std::result::Result<
        (Option<overdrive_netlink::ObservedLinkIdentity>, Option<bool>),
        NetlinkError,
    > {
        Err(NetlinkError::connect(std::io::Error::other(
            "shared bridge audit observation is unavailable on this allocation port",
        )))
    }
    fn insert_guard_member(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError>;
    fn delete_guard_member(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError>;
    fn observe_guard(
        &self,
        expected_members: &BTreeSet<String>,
    ) -> std::result::Result<BridgeGuardObservation, BridgeGuardError>;
    fn insert_endpoint(
        &self,
        _plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<(), GuestTcxError>;
    fn read_endpoint(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<Option<GuestTcxEndpoint>, GuestTcxError>;
    fn remove_endpoint(
        &self,
        _plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<(), GuestTcxError>;
    fn attach_first_ingress(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError>;
    fn pin_link(&self, plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError>;
    fn query_attachment(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError>;
    fn link_pin_present(&self, plan: &GuestNetworkPlan)
    -> std::result::Result<bool, GuestTcxError>;
    fn detach_pending_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError>;
    fn detach_pinned_link(&self, plan: &GuestNetworkPlan)
    -> std::result::Result<(), GuestTcxError>;
    fn attach_first_egress(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError>;
    fn pin_egress_link(&self, plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError>;
    fn query_egress_attachment(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError>;
    fn egress_link_pin_present(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<bool, GuestTcxError>;
    fn detach_pending_egress_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError>;
    fn detach_pinned_egress_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError>;
    /// The TAP's debug message mask, or `None` when the TAP no longer exists
    /// (the netlink read reports `ENODEV`). Used by provision and `activate`.
    async fn observe_tap_debug_msg_mask(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<Option<u32>, NetlinkError>;
    /// Every netdev's debug message mask keyed by ifindex, in one dump.
    /// Used once per audit pass.
    async fn observe_debug_msg_masks(
        &self,
    ) -> std::result::Result<BTreeMap<u32, u32>, NetlinkError>;
    fn observe_shared_tcx(
        &self,
        read: GuestNetworkNodeTcxAuditRead,
        managed_ifindices: &BTreeSet<u32>,
    ) -> std::result::Result<u32, GuestTcxError> {
        let _ = managed_ifindices;
        Err(GuestTcxError::CaptureUnavailable {
            family: match read {
                GuestNetworkNodeTcxAuditRead::Programs => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxProgram
                }
                GuestNetworkNodeTcxAuditRead::EndpointMap => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMap
                }
                GuestNetworkNodeTcxAuditRead::CounterMap => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMap
                }
                GuestNetworkNodeTcxAuditRead::EndpointMapPin => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMapPin
                }
                GuestNetworkNodeTcxAuditRead::CounterMapPin => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMapPin
                }
                GuestNetworkNodeTcxAuditRead::EndpointEntries => {
                    overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointEntry
                }
            },
        })
    }
}

#[allow(dead_code, reason = "D12 exact private RED owner state")]
struct HostGuestTcxState {
    program: GuestTcxProgram,
    inventory: GuestTcxInventoryIdentity,
}

struct HostGuestNetworkAllocationIo {
    tcx: Arc<parking_lot::Mutex<Option<HostGuestTcxState>>>,
    guard: BridgeGuardSpec,
    pending_links: parking_lot::Mutex<BTreeMap<AllocationId, GuestTcxLink>>,
    pending_egress_links: parking_lot::Mutex<BTreeMap<AllocationId, GuestTcxLink>>,
}

impl HostGuestNetworkAllocationIo {
    fn new(
        tcx: Arc<parking_lot::Mutex<Option<HostGuestTcxState>>>,
        guard: BridgeGuardSpec,
    ) -> Self {
        Self {
            tcx,
            guard,
            pending_links: parking_lot::Mutex::new(BTreeMap::new()),
            pending_egress_links: parking_lot::Mutex::new(BTreeMap::new()),
        }
    }

    fn endpoint_pin() -> PathBuf {
        PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints")
    }

    fn link_pin(plan: &GuestNetworkPlan) -> PathBuf {
        PathBuf::from(format!(
            "/sys/fs/bpf/overdrive/mtls-endpoints/links/{}-ingress",
            plan.assignment().tap
        ))
    }

    fn egress_link_pin(plan: &GuestNetworkPlan) -> PathBuf {
        PathBuf::from(format!(
            "/sys/fs/bpf/overdrive/mtls-endpoints/links/{}-egress",
            plan.assignment().tap
        ))
    }

    fn missing_object() -> GuestTcxError {
        GuestTcxError::ObjectMissing {
            object: overdrive_dataplane::guest_tcx::GuestTcxObject::Classifier,
        }
    }
}

#[async_trait::async_trait]
impl GuestNetworkAllocationIo for HostGuestNetworkAllocationIo {
    async fn create_tap(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
        overdrive_netlink::create_persistent_tap(&plan.assignment().tap, 0)
    }
    async fn attach_tap_to_bridge(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), NetlinkError> {
        let client = overdrive_netlink::Client::new()?;
        client.set_link_master(&plan.assignment().tap, plan.bridge()).await?;
        // The bridge's address is pinned at creation, so Linux does not adopt
        // this port's address. Keep the fixed classifier identity asserted
        // before TAP-up/read-back.
        client.set_link_mac(plan.bridge(), overdrive_core::dataplane::GUEST_BRIDGE_MAC).await
    }
    async fn set_tap_up(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
        overdrive_netlink::ethtool::disable_tx_offload(&plan.assignment().tap).await?;
        overdrive_netlink::Client::new()?.set_link_up(&plan.assignment().tap).await
    }
    async fn set_tap_down(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
        overdrive_netlink::Client::new()?.set_link_down(&plan.assignment().tap).await
    }
    async fn delete_tap(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
        overdrive_netlink::Client::new()?.del_link(&plan.assignment().tap).await
    }
    async fn observe_tap(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestNetworkAllocationTapObservation, NetlinkError> {
        match overdrive_netlink::Client::new()?
            .observe_persistent_tap_identity(&plan.assignment().tap)
            .await?
        {
            overdrive_netlink::PersistentTapIdentity::Absent { name } => {
                Ok(GuestNetworkAllocationTapObservation::Absent { name })
            }
            overdrive_netlink::PersistentTapIdentity::Incompatible {
                link,
                persistent,
                owner_uid,
            } => Ok(GuestNetworkAllocationTapObservation::Incompatible {
                name: link.name,
                ifindex: link.ifindex,
                kind: link_kind(link.kind),
                persistent,
                up: link.up,
                owner_uid,
                master_ifindex: link.master_ifindex,
            }),
            overdrive_netlink::PersistentTapIdentity::Persistent { link, owner_uid } => {
                Ok(GuestNetworkAllocationTapObservation::Persistent {
                    name: link.name,
                    ifindex: link.ifindex,
                    up: link.up,
                    owner_uid,
                    master_ifindex: link.master_ifindex,
                    mac: link.mac,
                })
            }
        }
    }
    async fn observe_bridge(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestNetworkAllocationBridgeObservation, NetlinkError> {
        match overdrive_netlink::Client::new()?.observe_link_identity(plan.bridge()).await? {
            None => Ok(GuestNetworkAllocationBridgeObservation::Absent {
                name: plan.bridge().to_owned(),
            }),
            Some(link) => Ok(GuestNetworkAllocationBridgeObservation::Present {
                name: link.name,
                ifindex: link.ifindex,
                kind: link_kind(link.kind),
            }),
        }
    }
    async fn observe_shared_bridge(
        &self,
    ) -> std::result::Result<
        (Option<overdrive_netlink::ObservedLinkIdentity>, Option<bool>),
        NetlinkError,
    > {
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            let identity = client.observe_link_identity("ovd-gbr0").await?;
            let gateway = if identity.is_some() {
                Some(client.observe_addr("ovd-gbr0", Ipv4Addr::new(100, 95, 0, 1), 16).await?)
            } else {
                None
            };
            Ok((identity, gateway))
        })
    }
    fn insert_guard_member(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        overdrive_netlink::nft::bridge::insert_member(&self.guard, &plan.assignment().tap)
    }
    fn delete_guard_member(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        overdrive_netlink::nft::bridge::delete_member(&self.guard, &plan.assignment().tap)
    }
    fn observe_guard(
        &self,
        expected_members: &BTreeSet<String>,
    ) -> std::result::Result<BridgeGuardObservation, BridgeGuardError> {
        overdrive_netlink::nft::bridge::observe(&self.guard, expected_members)
    }
    fn insert_endpoint(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<(), GuestTcxError> {
        let mut state = self.tcx.lock();
        state.as_mut().ok_or_else(Self::missing_object)?.program.insert_endpoint(
            ifindex,
            GuestTcxEndpoint {
                source_ipv4: plan.assignment().address,
                source_mac: plan.assignment().mac,
                bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
            },
        )
    }
    fn read_endpoint(
        &self,
        _plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<Option<GuestTcxEndpoint>, GuestTcxError> {
        self.tcx.lock().as_ref().ok_or_else(Self::missing_object)?.program.read_endpoint(ifindex)
    }
    fn remove_endpoint(
        &self,
        _plan: &GuestNetworkPlan,
        ifindex: u32,
    ) -> std::result::Result<(), GuestTcxError> {
        overdrive_dataplane::guest_tcx::remove_endpoint(Self::endpoint_pin(), ifindex)
    }
    fn attach_first_ingress(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let link = self
            .tcx
            .lock()
            .as_mut()
            .ok_or_else(Self::missing_object)?
            .program
            .attach_first_ingress(&plan.assignment().tap)?;
        self.pending_links.lock().insert(plan.alloc().clone(), link);
        Ok(())
    }
    fn pin_link(&self, plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError> {
        let link =
            self.pending_links.lock().remove(plan.alloc()).ok_or_else(Self::missing_object)?;
        let program_id = link.program_id();
        link.pin(&Self::link_pin(plan))?;
        Ok(program_id)
    }
    fn query_attachment(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
        overdrive_dataplane::guest_tcx::query_attachment(
            &plan.assignment().tap,
            TcxAttachPoint::Ingress,
        )
    }
    fn link_pin_present(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<bool, GuestTcxError> {
        Ok(Self::link_pin(plan).exists())
    }
    fn detach_pending_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let pending = self.pending_links.lock().remove(plan.alloc());
        if let Some(link) = pending {
            link.detach()?;
        }
        Ok(())
    }
    fn detach_pinned_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let pin = Self::link_pin(plan);
        if !pin.exists() {
            return Ok(());
        }
        overdrive_dataplane::guest_tcx::detach_pinned_link(pin)
    }
    fn attach_first_egress(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let link = {
            let mut tcx = self.tcx.lock();
            let state = tcx.as_mut().ok_or(GuestTcxError::ObjectMissing {
                object: overdrive_dataplane::guest_tcx::GuestTcxObject::EgressClassifier,
            })?;
            let link = state.program.attach_first_egress(&plan.assignment().tap)?;
            drop(tcx);
            link
        };
        self.pending_egress_links.lock().insert(plan.alloc().clone(), link);
        Ok(())
    }
    fn pin_egress_link(&self, plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError> {
        let link = self.pending_egress_links.lock().remove(plan.alloc()).ok_or_else(|| {
            GuestTcxError::ObjectMissing {
                object: overdrive_dataplane::guest_tcx::GuestTcxObject::EgressClassifier,
            }
        })?;
        let program_id = link.program_id();
        link.pin(&Self::egress_link_pin(plan))?;
        Ok(program_id)
    }
    fn query_egress_attachment(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
        overdrive_dataplane::guest_tcx::query_attachment(
            &plan.assignment().tap,
            TcxAttachPoint::Egress,
        )
    }
    fn egress_link_pin_present(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<bool, GuestTcxError> {
        Ok(Self::egress_link_pin(plan).exists())
    }
    fn detach_pending_egress_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let pending = self.pending_egress_links.lock().remove(plan.alloc());
        if let Some(link) = pending {
            link.detach()?;
        }
        Ok(())
    }
    fn detach_pinned_egress_link(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), GuestTcxError> {
        let pin = Self::egress_link_pin(plan);
        if !pin.exists() {
            return Ok(());
        }
        overdrive_dataplane::guest_tcx::detach_pinned_link(pin)
    }
    async fn observe_tap_debug_msg_mask(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<Option<u32>, NetlinkError> {
        match overdrive_netlink::ethtool::debug_msg_mask(&plan.assignment().tap).await {
            Ok(mask) => Ok(Some(mask)),
            Err(error) if error.errno() == Some(overdrive_netlink::error::NEG_ENODEV) => Ok(None),
            Err(error) => Err(error),
        }
    }
    async fn observe_debug_msg_masks(
        &self,
    ) -> std::result::Result<BTreeMap<u32, u32>, NetlinkError> {
        overdrive_netlink::ethtool::debug_msg_masks().await
    }
    fn observe_shared_tcx(
        &self,
        read: GuestNetworkNodeTcxAuditRead,
        managed_ifindices: &BTreeSet<u32>,
    ) -> std::result::Result<u32, GuestTcxError> {
        let _ = managed_ifindices;
        let tcx = self.tcx.lock();
        let inventory = tcx.as_ref().map(|state| state.inventory.clone()).ok_or_else(|| {
            GuestTcxError::CaptureUnavailable {
                family: match read {
                    GuestNetworkNodeTcxAuditRead::Programs => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxProgram
                    }
                    GuestNetworkNodeTcxAuditRead::EndpointMap => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMap
                    }
                    GuestNetworkNodeTcxAuditRead::CounterMap => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMap
                    }
                    GuestNetworkNodeTcxAuditRead::EndpointMapPin => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMapPin
                    }
                    GuestNetworkNodeTcxAuditRead::CounterMapPin => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMapPin
                    }
                    GuestNetworkNodeTcxAuditRead::EndpointEntries => {
                        overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointEntry
                    }
                },
            }
        })?;
        drop(tcx);
        match read {
            GuestNetworkNodeTcxAuditRead::Programs => inventory.observe_tcx_programs(),
            GuestNetworkNodeTcxAuditRead::EndpointMap => inventory.observe_endpoint_maps(),
            GuestNetworkNodeTcxAuditRead::CounterMap => inventory.observe_counter_maps(),
            GuestNetworkNodeTcxAuditRead::EndpointMapPin => inventory.observe_endpoint_map_pins(),
            GuestNetworkNodeTcxAuditRead::CounterMapPin => inventory.observe_counter_map_pins(),
            GuestNetworkNodeTcxAuditRead::EndpointEntries => inventory.observe_endpoint_entries(),
        }
    }
}

fn link_kind(kind: overdrive_netlink::ObservedLinkKind) -> GuestLinkKind {
    match kind {
        overdrive_netlink::ObservedLinkKind::Bridge => GuestLinkKind::Bridge,
        overdrive_netlink::ObservedLinkKind::Tap => GuestLinkKind::Tap,
        overdrive_netlink::ObservedLinkKind::Tun => GuestLinkKind::Tun,
        overdrive_netlink::ObservedLinkKind::Veth | overdrive_netlink::ObservedLinkKind::Other => {
            GuestLinkKind::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostGuestNetworkAllocationPhase {
    ProvisionedDown,
    Active,
    QuiescedActive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostGuestNetworkAllocationState {
    plan: GuestNetworkPlan,
    tap: String,
    ifindex: u32,
    program_id: u32,
    egress_program_id: u32,
    phase: HostGuestNetworkAllocationPhase,
}

#[derive(Debug, Default)]
struct HostGuestNetworkLifecycle {
    quiescing: bool,
    allocations: BTreeMap<AllocationId, HostGuestNetworkAllocationState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GuestNetworkRollbackLink {
    #[default]
    None,
    Pending,
    Pinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct GuestNetworkRollbackLinks {
    ingress: GuestNetworkRollbackLink,
    egress: GuestNetworkRollbackLink,
}

#[derive(Debug, Clone)]
struct GuestNetworkPendingRollback {
    plan: GuestNetworkPlan,
    ifindex: Option<u32>,
    links: GuestNetworkRollbackLinks,
    tap_removed: bool,
}

/// Private host owner; its startup algorithm is independently executable from
/// the still-pending native adapter bindings.
#[allow(dead_code, reason = "D5 exact RED owner field is activated in DELIVER")]
pub(super) struct HostSharedGuestNetworkOwner {
    scratch_io: Arc<dyn SharedGuestNetworkScratchIo>,
    allocation_io: Arc<dyn GuestNetworkAllocationIo>,
    tcx: Arc<parking_lot::Mutex<Option<HostGuestTcxState>>>,
    allocations: parking_lot::Mutex<BTreeMap<AllocationId, HostGuestNetworkAllocationState>>,
    allocation_lifecycle: tokio::sync::Mutex<HostGuestNetworkLifecycle>,
    // Failed partial provisions stay private and lease-correlated until the
    // existing teardown boundary proves the same empty complement; this is
    // process-local retry state, never a published allocation or persistence.
    rollback_pending: parking_lot::Mutex<BTreeMap<AllocationId, GuestNetworkPendingRollback>>,
}

// Allocation effects and node-shared effects intentionally have one concrete
// owner. This private alias names the inherited provisioner role without
// creating a second allocation or boot owner.
type HostGuestNetworkProvisioner = HostSharedGuestNetworkOwner;

impl std::fmt::Debug for HostSharedGuestNetworkOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostSharedGuestNetworkOwner").finish_non_exhaustive()
    }
}

impl HostSharedGuestNetworkOwner {
    pub(super) fn new() -> Self {
        let tcx = Arc::new(parking_lot::Mutex::new(None));
        let allocation_io =
            Arc::new(HostGuestNetworkAllocationIo::new(Arc::clone(&tcx), Self::guard_spec()));
        Self {
            scratch_io: Arc::new(RealSharedGuestNetworkScratchIo::new()),
            allocation_io,
            tcx,
            allocations: parking_lot::Mutex::new(BTreeMap::new()),
            allocation_lifecycle: tokio::sync::Mutex::new(HostGuestNetworkLifecycle::default()),
            rollback_pending: parking_lot::Mutex::new(BTreeMap::new()),
        }
    }

    #[cfg(test)]
    fn with_scratch_io(scratch_io: Arc<dyn SharedGuestNetworkScratchIo>) -> Self {
        let mut owner = Self::new();
        owner.scratch_io = scratch_io;
        owner
    }

    #[cfg(test)]
    fn with_allocation_io(allocation_io: Arc<dyn GuestNetworkAllocationIo>) -> Self {
        Self {
            scratch_io: Arc::new(RealSharedGuestNetworkScratchIo::new()),
            allocation_io,
            tcx: Arc::new(parking_lot::Mutex::new(None)),
            allocations: parking_lot::Mutex::new(BTreeMap::new()),
            allocation_lifecycle: tokio::sync::Mutex::new(HostGuestNetworkLifecycle::default()),
            rollback_pending: parking_lot::Mutex::new(BTreeMap::new()),
        }
    }

    fn guard_spec() -> BridgeGuardSpec {
        BridgeGuardSpec::new(
            "overdrive-mtls".to_owned(),
            "prerouting".to_owned(),
            "managed_taps".to_owned(),
            -300,
            0x295a,
            0x295b,
        )
        .unwrap_or_else(|_| unreachable!("the design-pinned bridge guard identity is valid"))
    }

    fn netlink_error(operation: GuestNetworkOperation, source: NetlinkError) -> GuestNetworkError {
        GuestNetworkError::Netlink { operation, source }
    }

    fn tcx_error(operation: GuestNetworkOperation, source: GuestTcxError) -> GuestNetworkError {
        GuestNetworkError::Tcx { operation, source }
    }

    fn guard_error(operation: GuestNetworkOperation, error: BridgeGuardError) -> GuestNetworkError {
        match error {
            BridgeGuardError::Netlink(source) => Self::netlink_error(operation, source),
            other @ BridgeGuardError::Validation(_) => GuestNetworkError::Io {
                operation,
                source: std::io::Error::other(other.to_string()),
            },
        }
    }

    fn tap_fact(
        plan: &GuestNetworkPlan,
        observation: &GuestNetworkAllocationTapObservation,
        expected_up: bool,
    ) -> (GuestNetworkFact, Option<GuestNetworkFact>) {
        let expected = GuestNetworkFact::Tap {
            name: plan.assignment().tap.clone(),
            ifindex: match observation {
                GuestNetworkAllocationTapObservation::Absent { .. } => None,
                GuestNetworkAllocationTapObservation::Incompatible { ifindex, .. }
                | GuestNetworkAllocationTapObservation::Persistent { ifindex, .. } => {
                    Some(*ifindex)
                }
            },
            link_kind: GuestLinkKind::Tap,
            persistent: true,
            up: expected_up,
            owner_uid: Some(0),
        };
        let observed = match observation {
            GuestNetworkAllocationTapObservation::Absent { .. } => None,
            GuestNetworkAllocationTapObservation::Incompatible {
                name,
                ifindex,
                kind,
                persistent,
                up,
                owner_uid,
                ..
            } => Some(GuestNetworkFact::Tap {
                name: name.clone(),
                ifindex: Some(*ifindex),
                link_kind: *kind,
                persistent: persistent.unwrap_or(false),
                up: *up,
                owner_uid: *owner_uid,
            }),
            GuestNetworkAllocationTapObservation::Persistent {
                name,
                ifindex,
                up,
                owner_uid,
                ..
            } => Some(GuestNetworkFact::Tap {
                name: name.clone(),
                ifindex: Some(*ifindex),
                link_kind: GuestLinkKind::Tap,
                persistent: true,
                up: *up,
                owner_uid: *owner_uid,
            }),
        };
        (expected, observed)
    }

    fn reserved_host_macs(
        lifecycle: &HostGuestNetworkLifecycle,
        additional: Option<&GuestNetworkPlan>,
    ) -> BTreeSet<[u8; 6]> {
        let mut reserved = lifecycle
            .allocations
            .values()
            .map(|state| state.plan.assignment().mac)
            .chain(additional.map(|plan| plan.assignment().mac))
            .collect::<BTreeSet<_>>();
        reserved.insert(overdrive_core::dataplane::GUEST_BRIDGE_MAC);
        reserved
    }

    fn tap_host_mac_facts(
        ifindex: u32,
        mac: Option<[u8; 6]>,
        reserved: &BTreeSet<[u8; 6]>,
    ) -> (GuestNetworkFact, Option<GuestNetworkFact>) {
        let expected =
            GuestNetworkFact::TapHostMac { ifindex, address: TapHostAddress::Unreserved };
        let observed = match mac {
            None => {
                Some(GuestNetworkFact::TapHostMac { ifindex, address: TapHostAddress::Missing })
            }
            Some(address) if reserved.contains(&address) => Some(GuestNetworkFact::TapHostMac {
                ifindex,
                address: TapHostAddress::Reserved(address),
            }),
            Some(_) => None,
        };
        (expected, observed)
    }

    fn bridge_fact(
        plan: &GuestNetworkPlan,
        observation: &GuestNetworkAllocationBridgeObservation,
    ) -> (GuestNetworkFact, Option<GuestNetworkFact>, Option<u32>) {
        let expected = GuestNetworkFact::BridgeLinkIdentity {
            name: plan.bridge().to_owned(),
            ifindex: match observation {
                GuestNetworkAllocationBridgeObservation::Absent { .. } => None,
                GuestNetworkAllocationBridgeObservation::Present { ifindex, .. } => Some(*ifindex),
            },
            link_kind: GuestLinkKind::Bridge,
        };
        let (observed, ifindex) = match observation {
            GuestNetworkAllocationBridgeObservation::Absent { .. } => (None, None),
            GuestNetworkAllocationBridgeObservation::Present { name, ifindex, kind } => (
                Some(GuestNetworkFact::BridgeLinkIdentity {
                    name: name.clone(),
                    ifindex: Some(*ifindex),
                    link_kind: *kind,
                }),
                (*kind == GuestLinkKind::Bridge).then_some(*ifindex),
            ),
        };
        (expected, observed, ifindex)
    }

    fn master_fact(ifindex: u32, master_ifindex: Option<u32>) -> GuestNetworkFact {
        GuestNetworkFact::LinkMaster { ifindex, master_ifindex }
    }

    fn expected_guard_members(
        &self,
        extra: Option<&GuestNetworkPlan>,
        exclude: Option<&AllocationId>,
    ) -> BTreeSet<String> {
        let mut members = self
            .allocations
            .lock()
            .iter()
            .filter(|(alloc, _)| exclude.is_none_or(|excluded| excluded != *alloc))
            .map(|(_, state)| state.tap.clone())
            .collect::<BTreeSet<_>>();
        if let Some(plan) = extra {
            members.insert(plan.assignment().tap.clone());
        }
        members
    }

    fn guard_postcondition(
        expected_members: &BTreeSet<String>,
        observed: BridgeGuardObservation,
    ) -> Result<()> {
        match observed {
            BridgeGuardObservation::Exact { .. } => Ok(()),
            BridgeGuardObservation::Absent { inventory }
            | BridgeGuardObservation::Conflict { inventory } => {
                Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::GuardMemberInsert,
                    expected: GuestNetworkFact::BridgeGuard {
                        tap: expected_members.iter().next().cloned().unwrap_or_default(),
                        member: true,
                        rules: Self::guard_spec().expected_rule_facts(),
                    },
                    observed: Some(GuestNetworkFact::BridgeGuard {
                        tap: expected_members.iter().next().cloned().unwrap_or_default(),
                        member: inventory.members.iter().any(|member| {
                            matches!(&member.identity, overdrive_netlink::nft::bridge::BridgeGuardMemberIdentity::Ifname(name) if expected_members.contains(name))
                        }),
                        rules: inventory.rules.into_iter().map(|rule| rule.fact).collect(),
                    }),
                })
            }
        }
    }

    async fn observe_bridge(&self, plan: &GuestNetworkPlan) -> Result<u32> {
        let observed =
            self.allocation_io.observe_bridge(plan).await.map_err(|source| {
                Self::netlink_error(GuestNetworkOperation::BridgeObserve, source)
            })?;
        let (expected, actual, ifindex) = Self::bridge_fact(plan, &observed);
        if expected != actual.clone().unwrap_or_else(|| expected.clone()) {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected,
                observed: actual,
            });
        }
        ifindex.ok_or_else(|| GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::BridgeObserve,
            expected,
            observed: actual,
        })
    }

    fn ensure_master(
        tap_ifindex: u32,
        expected_master: u32,
        actual_master: Option<u32>,
    ) -> Result<()> {
        if actual_master == Some(expected_master) {
            return Ok(());
        }
        Err(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::TapObserve,
            expected: Self::master_fact(tap_ifindex, Some(expected_master)),
            observed: Some(Self::master_fact(tap_ifindex, actual_master)),
        })
    }

    fn rollback_endpoint_absence(&self, plan: &GuestNetworkPlan, ifindex: u32) -> Result<()> {
        self.allocation_io
            .read_endpoint(plan, ifindex)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::EndpointMapObserve, source))
            .and_then(|value| {
                if value.is_some() {
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::EndpointMapObserve,
                        expected: GuestNetworkFact::EndpointMapEntry { ifindex, value: None },
                        observed: Some(GuestNetworkFact::EndpointMapEntry {
                            ifindex,
                            value: value.map(|endpoint| GuestEndpointFact {
                                source_ip: endpoint.source_ipv4,
                                source_mac: endpoint.source_mac,
                                bridge_mac: endpoint.bridge_mac,
                            }),
                        }),
                    })
                } else {
                    Ok(())
                }
            })
    }

    fn rollback_attachment_absence(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: Option<u32>,
    ) -> Result<()> {
        self.allocation_io
            .query_attachment(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxQuery, source))
            .and_then(|attachment| {
                if attachment.program_ids.is_empty() {
                    Ok(())
                } else {
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TcxQuery,
                        expected: GuestNetworkFact::TcxAttachment {
                            ifindex: ifindex.unwrap_or_default(),
                            program_id: None,
                            attach_point: None,
                        },
                        observed: Some(GuestNetworkFact::TcxAttachment {
                            ifindex: ifindex.unwrap_or_default(),
                            program_id: attachment.program_ids.first().copied(),
                            attach_point: Some(TcxAttachPoint::Ingress),
                        }),
                    })
                }
            })
    }

    fn rollback_link_pin_absence(&self, plan: &GuestNetworkPlan) -> Result<()> {
        self.allocation_io
            .link_pin_present(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))
            .and_then(|present| {
                if present {
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TcxLinkPin,
                        expected: GuestNetworkFact::BpfLinkPin {
                            path: PathBuf::new(),
                            link_id: None,
                        },
                        observed: Some(GuestNetworkFact::BpfLinkPin {
                            path: PathBuf::new(),
                            link_id: None,
                        }),
                    })
                } else {
                    Ok(())
                }
            })
    }

    fn rollback_egress_attachment_absence(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: Option<u32>,
    ) -> Result<()> {
        self.allocation_io
            .query_egress_attachment(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxEgressQuery, source))
            .and_then(|attachment| {
                if attachment.program_ids.is_empty() {
                    Ok(())
                } else {
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TcxEgressQuery,
                        expected: GuestNetworkFact::TcxAttachment {
                            ifindex: ifindex.unwrap_or_default(),
                            program_id: None,
                            attach_point: None,
                        },
                        observed: Some(GuestNetworkFact::TcxAttachment {
                            ifindex: ifindex.unwrap_or_default(),
                            program_id: attachment.program_ids.first().copied(),
                            attach_point: Some(TcxAttachPoint::Egress),
                        }),
                    })
                }
            })
    }

    fn rollback_egress_link_pin_absence(&self, plan: &GuestNetworkPlan) -> Result<()> {
        self.allocation_io
            .egress_link_pin_present(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxEgressLinkPin, source))
            .and_then(|present| {
                if present {
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TcxEgressLinkPin,
                        expected: GuestNetworkFact::BpfLinkPin {
                            path: PathBuf::new(),
                            link_id: None,
                        },
                        observed: Some(GuestNetworkFact::BpfLinkPin {
                            path: PathBuf::new(),
                            link_id: None,
                        }),
                    })
                } else {
                    Ok(())
                }
            })
    }

    async fn rollback_tap_absence(&self, plan: &GuestNetworkPlan) -> Result<bool> {
        self.allocation_io
            .observe_tap(plan)
            .await
            .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapObserve, source))
            .and_then(|observation| match observation {
                GuestNetworkAllocationTapObservation::Absent { .. } => Ok(true),
                other => {
                    let (expected, observed) = Self::tap_fact(plan, &other, false);
                    Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed,
                    })
                }
            })
    }

    fn rollback_guard_complement(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let expected_members = self.expected_guard_members(None, Some(plan.alloc()));
        self.allocation_io
            .observe_guard(&expected_members)
            .map_err(|error| Self::guard_error(GuestNetworkOperation::CleanupComplement, error))
            .and_then(|observed| Self::guard_postcondition(&expected_members, observed))
    }

    fn detach_rollback_links(
        &self,
        plan: &GuestNetworkPlan,
        links: GuestNetworkRollbackLinks,
    ) -> Result<()> {
        match links.ingress {
            GuestNetworkRollbackLink::None => {}
            GuestNetworkRollbackLink::Pending => self
                .allocation_io
                .detach_pending_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxDetach, source))?,
            GuestNetworkRollbackLink::Pinned => self
                .allocation_io
                .detach_pinned_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxDetach, source))?,
        }
        match links.egress {
            GuestNetworkRollbackLink::None => {}
            GuestNetworkRollbackLink::Pending => {
                self.allocation_io.detach_pending_egress_link(plan).map_err(|source| {
                    Self::tcx_error(GuestNetworkOperation::TcxEgressDetach, source)
                })?;
            }
            GuestNetworkRollbackLink::Pinned => {
                self.allocation_io.detach_pinned_egress_link(plan).map_err(|source| {
                    Self::tcx_error(GuestNetworkOperation::TcxEgressDetach, source)
                })?;
            }
        }
        Ok(())
    }

    async fn rollback_provision(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: Option<u32>,
        rollback_links: GuestNetworkRollbackLinks,
        tap_removed: bool,
    ) -> (Option<GuestNetworkError>, bool) {
        let mut first = None;
        macro_rules! attempt {
            ($result:expr) => {
                if let Err(error) = $result {
                    if first.is_none() {
                        first = Some(error);
                    }
                }
            };
        }
        let mut tap_removed = tap_removed;
        if !tap_removed {
            match self.allocation_io.observe_tap(plan).await {
                Ok(GuestNetworkAllocationTapObservation::Absent { .. }) => tap_removed = true,
                Ok(_) => {}
                Err(source) => {
                    if first.is_none() {
                        first =
                            Some(Self::netlink_error(GuestNetworkOperation::TapObserve, source));
                    }
                }
            }
        }
        if let Some(ifindex) = ifindex {
            attempt!(
                self.allocation_io.remove_endpoint(plan, ifindex).map_err(
                    |source| Self::tcx_error(GuestNetworkOperation::EndpointDelete, source)
                )
            );
        }
        attempt!(self.detach_rollback_links(plan, rollback_links));
        if !tap_removed {
            attempt!(
                self.allocation_io.set_tap_down(plan).await.map_err(|source| Self::netlink_error(
                    GuestNetworkOperation::TapSetDown,
                    source
                ))
            );
        }
        if let Some(ifindex) = ifindex {
            attempt!(self.rollback_endpoint_absence(plan, ifindex));
        }
        if !tap_removed && ifindex.is_some() {
            attempt!(self.rollback_attachment_absence(plan, ifindex));
            attempt!(self.rollback_egress_attachment_absence(plan, ifindex));
        }
        attempt!(self.rollback_link_pin_absence(plan));
        attempt!(self.rollback_egress_link_pin_absence(plan));
        if !tap_removed {
            let delete =
                self.allocation_io.delete_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapDelete, source)
                });
            if delete.is_ok() {
                tap_removed = true;
            }
            attempt!(delete);
        }
        attempt!(
            self.allocation_io.delete_guard_member(plan).map_err(|error| Self::guard_error(
                GuestNetworkOperation::GuardMemberDelete,
                error
            ))
        );
        match self.rollback_tap_absence(plan).await {
            Ok(absent) => tap_removed |= absent,
            Err(error) => {
                if first.is_none() {
                    first = Some(error);
                }
            }
        }
        attempt!(self.rollback_guard_complement(plan));
        (first, tap_removed)
    }

    #[expect(clippy::expect_used, reason = "the accepted scratch prefix is a static valid CIDR")]
    fn scratch_plan() -> GuestNetworkScratchPlan {
        let address = Ipv4Addr::new(100, 95, 255, 254);
        GuestNetworkScratchPlan {
            bridge: "ovd-gbr-probe".to_owned(),
            tap: "ovd-tp-probe".to_owned(),
            node_prefix: "100.95.0.0/16".parse().expect("static scratch prefix"),
            assignment: GuestNetworkAssignment {
                address,
                tap: "ovd-tp-probe".to_owned(),
                mac: [0x02, 0x00, 100, 95, 255, 254],
                gateway: Ipv4Addr::new(100, 95, 0, 1),
                prefix: 16,
                dns: Ipv4Addr::new(100, 95, 0, 1),
            },
            endpoint_map_pin: "/sys/fs/bpf/overdrive/probe/endpoints".into(),
            counter_map_pin: "/sys/fs/bpf/overdrive/probe/counters".into(),
            tcx_link_pin: "/sys/fs/bpf/overdrive/probe/link".into(),
            guard_table: "overdrive-probe".to_owned(),
            guard_chain: "ingress".to_owned(),
            guard_set: "members".to_owned(),
            original_destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 255, 253), 8443),
        }
    }

    async fn netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchNetlinkAction,
        operation: GuestNetworkOperation,
    ) -> Result<()> {
        self.scratch_io
            .apply_netlink(plan, action)
            .await
            .map_err(|source| GuestNetworkError::Netlink { operation, source })
    }

    async fn tcx(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchTcxAction,
        operation: GuestNetworkOperation,
    ) -> Result<()> {
        self.scratch_io
            .apply_tcx(plan, action)
            .await
            .map_err(|source| GuestNetworkError::Tcx { operation, source })
    }

    async fn exercise(
        &self,
        plan: &GuestNetworkScratchPlan,
        stage: GuestNetworkProbeStage,
    ) -> Result<()> {
        let passed = self.scratch_io.exercise(plan, stage).await.map_err(|source| {
            GuestNetworkError::Io { operation: GuestNetworkOperation::StartupProbe, source }
        })?;
        if passed {
            return Ok(());
        }
        Err(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::StartupProbe,
            expected: GuestNetworkFact::StartupProbe { stage, passed: true },
            observed: Some(GuestNetworkFact::StartupProbe { stage, passed: false }),
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "GH #295 staged owner algorithm: each port call is one D12A-ordered sequence; DELIVER 06-02/06-04 restructure these bodies to the R4/R5/R21/R22 contracts"
    )]
    async fn run_probe(&self, plan: &GuestNetworkScratchPlan) -> Result<()> {
        let netlink_actions = [
            (
                GuestNetworkScratchNetlinkAction::ConvergeBridge,
                GuestNetworkOperation::BridgeConverge,
            ),
            (GuestNetworkScratchNetlinkAction::CreateTap, GuestNetworkOperation::TapCreate),
            (
                GuestNetworkScratchNetlinkAction::AttachTapToBridge,
                GuestNetworkOperation::TapAttachBridge,
            ),
            (GuestNetworkScratchNetlinkAction::SetTapUp, GuestNetworkOperation::TapSetUp),
            (
                GuestNetworkScratchNetlinkAction::CreateGuardTable,
                GuestNetworkOperation::GuardTableCreate,
            ),
            (
                GuestNetworkScratchNetlinkAction::CreateGuardChain,
                GuestNetworkOperation::GuardChainCreate,
            ),
            (
                GuestNetworkScratchNetlinkAction::CreateGuardSet,
                GuestNetworkOperation::GuardSetCreate,
            ),
            (
                GuestNetworkScratchNetlinkAction::CreateGuardRules,
                GuestNetworkOperation::GuardRulesCreate,
            ),
            (
                GuestNetworkScratchNetlinkAction::InsertGuardMember,
                GuestNetworkOperation::GuardMemberInsert,
            ),
        ];
        for (action, operation) in netlink_actions {
            self.netlink(plan, action, operation).await?;
        }
        let tcx_actions = [
            (GuestNetworkScratchTcxAction::LoadProgramAndMaps, GuestNetworkOperation::TcxLoad),
            (GuestNetworkScratchTcxAction::PinEndpointMap, GuestNetworkOperation::EndpointMapPin),
            (GuestNetworkScratchTcxAction::PinCounterMap, GuestNetworkOperation::CounterMapPin),
            (GuestNetworkScratchTcxAction::InsertEndpoint, GuestNetworkOperation::EndpointInsert),
            (GuestNetworkScratchTcxAction::AttachLink, GuestNetworkOperation::TcxAttach),
            (GuestNetworkScratchTcxAction::PinLink, GuestNetworkOperation::TcxLinkPin),
        ];
        for (action, operation) in tcx_actions {
            self.tcx(plan, action, operation).await?;
        }
        self.exercise(plan, GuestNetworkProbeStage::Classifier).await?;
        self.exercise(plan, GuestNetworkProbeStage::OriginalDestination).await?;
        self.scratch_io.close_loader_handles(plan);
        for (action, operation) in [
            (
                GuestNetworkScratchTcxAction::AdoptEndpointMap,
                GuestNetworkOperation::EndpointMapAdopt,
            ),
            (GuestNetworkScratchTcxAction::AdoptCounterMap, GuestNetworkOperation::CounterMapAdopt),
            (GuestNetworkScratchTcxAction::AdoptLink, GuestNetworkOperation::TcxLinkAdopt),
            (GuestNetworkScratchTcxAction::QueryLink, GuestNetworkOperation::TcxQuery),
        ] {
            self.tcx(plan, action, operation).await?;
        }
        if let Some(program_id) = self.scratch_io.probe_program_id() {
            let attachment = self
                .scratch_io
                .probe_attachment(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxQuery, source))?;
            if attachment.program_ids != vec![program_id] {
                let ifindex =
                    tokio::fs::read_to_string(format!("/sys/class/net/{}/ifindex", plan.tap))
                        .await
                        .ok()
                        .and_then(|value| value.trim().parse::<u32>().ok())
                        .unwrap_or_default();
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TcxQuery,
                    expected: GuestNetworkFact::TcxAttachment {
                        ifindex,
                        program_id: Some(program_id),
                        attach_point: Some(TcxAttachPoint::Ingress),
                    },
                    observed: Some(GuestNetworkFact::TcxAttachment {
                        ifindex,
                        program_id: attachment.program_ids.first().copied(),
                        attach_point: Some(TcxAttachPoint::Ingress),
                    }),
                });
            }
            tracing::info!(
                event = "guest_network.shared_owner_startup_probe_attachment_reopened",
                program_id,
                revision = attachment.revision,
                program_count = attachment.program_ids.len(),
                ifindex = std::fs::read_to_string(format!("/sys/class/net/{}/ifindex", plan.tap))
                    .ok()
                    .and_then(|value| value.trim().parse::<u32>().ok())
                    .unwrap_or_default(),
            );
        }
        self.tcx(
            plan,
            GuestNetworkScratchTcxAction::UnpinLink,
            GuestNetworkOperation::TcxLinkUnpin,
        )
        .await?;
        self.tcx(plan, GuestNetworkScratchTcxAction::DetachLink, GuestNetworkOperation::TcxDetach)
            .await?;
        self.exercise(plan, GuestNetworkProbeStage::DetachedLinkGuard).await
    }

    #[allow(clippy::too_many_lines)]
    async fn cleanup(
        &self,
        plan: &GuestNetworkScratchPlan,
    ) -> (Option<GuestNetworkError>, GuestNetworkScratchComplement) {
        let mut first_error = None;
        macro_rules! attempt {
            ($future:expr) => {
                if let Err(error) = $future.await {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            };
        }
        self.scratch_io.close_loader_handles(plan);
        attempt!(self.tcx(
            plan,
            GuestNetworkScratchTcxAction::DeleteEndpoint,
            GuestNetworkOperation::EndpointDelete
        ));
        attempt!(self.tcx(
            plan,
            GuestNetworkScratchTcxAction::UnpinLink,
            GuestNetworkOperation::TcxLinkUnpin
        ));
        attempt!(self.tcx(
            plan,
            GuestNetworkScratchTcxAction::DetachLink,
            GuestNetworkOperation::TcxDetach
        ));
        attempt!(self.tcx(
            plan,
            GuestNetworkScratchTcxAction::UnpinCounterMap,
            GuestNetworkOperation::CounterMapUnpin
        ));
        attempt!(self.tcx(
            plan,
            GuestNetworkScratchTcxAction::UnpinEndpointMap,
            GuestNetworkOperation::EndpointMapUnpin
        ));
        self.scratch_io.release_adopted_handles(plan);
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::SetTapDown,
            GuestNetworkOperation::TapSetDown
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteTap,
            GuestNetworkOperation::TapDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteGuardMember,
            GuestNetworkOperation::GuardMemberDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteGuardRules,
            GuestNetworkOperation::GuardRulesDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteGuardSet,
            GuestNetworkOperation::GuardSetDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteGuardChain,
            GuestNetworkOperation::GuardChainDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteGuardTable,
            GuestNetworkOperation::GuardTableDelete
        ));
        attempt!(self.netlink(
            plan,
            GuestNetworkScratchNetlinkAction::DeleteBridge,
            GuestNetworkOperation::BridgeDelete
        ));

        let (bridges, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::Bridge).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (taps, error) = self.netlink_count(plan, GuestNetworkScratchNetlinkResource::Tap).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (bridge_guard_tables, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::BridgeGuardTable).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (bridge_guard_chains, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::BridgeGuardChain).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (bridge_guard_sets, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::BridgeGuardSet).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (bridge_guard_rules, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::BridgeGuardRule).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (bridge_guard_members, error) =
            self.netlink_count(plan, GuestNetworkScratchNetlinkResource::BridgeGuardMember).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (endpoint_maps, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::EndpointMap).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (counter_maps, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::CounterMap).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (endpoint_entries, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::EndpointEntry).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (tcx_programs, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::TcxProgram).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (tcx_links, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::TcxLink).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (endpoint_map_pins, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::EndpointMapPin).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (counter_map_pins, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::CounterMapPin).await;
        if first_error.is_none() {
            first_error = error;
        }
        let (tcx_link_pins, error) =
            self.tcx_count(plan, GuestNetworkScratchTcxResource::TcxLinkPin).await;
        if first_error.is_none() {
            first_error = error;
        }
        (
            first_error,
            GuestNetworkScratchComplement {
                bridges,
                taps,
                endpoint_maps,
                counter_maps,
                endpoint_entries,
                tcx_programs,
                tcx_links,
                endpoint_map_pins,
                counter_map_pins,
                tcx_link_pins,
                bridge_guard_tables,
                bridge_guard_chains,
                bridge_guard_sets,
                bridge_guard_rules,
                bridge_guard_members,
            },
        )
    }

    async fn netlink_count(
        &self,
        plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchNetlinkResource,
    ) -> (GuestNetworkScratchCount, Option<GuestNetworkError>) {
        match self.scratch_io.count_netlink(plan, resource).await {
            Ok(count) => (GuestNetworkScratchCount::Observed(count), None),
            Err(source) => (
                GuestNetworkScratchCount::Unavailable,
                Some(GuestNetworkError::Netlink {
                    operation: GuestNetworkOperation::CleanupComplement,
                    source,
                }),
            ),
        }
    }

    async fn tcx_count(
        &self,
        plan: &GuestNetworkScratchPlan,
        resource: GuestNetworkScratchTcxResource,
    ) -> (GuestNetworkScratchCount, Option<GuestNetworkError>) {
        match self.scratch_io.count_tcx(plan, resource).await {
            Ok(count) => (GuestNetworkScratchCount::Observed(count), None),
            Err(source) => (
                GuestNetworkScratchCount::Unavailable,
                Some(GuestNetworkError::Tcx {
                    operation: GuestNetworkOperation::CleanupComplement,
                    source,
                }),
            ),
        }
    }
}

#[async_trait::async_trait]
#[allow(
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    reason = "GH #295 staged owner algorithm: each port call is one D12A-ordered sequence; DELIVER 06-02/06-04 restructure these bodies to the R4/R5/R21/R22 contracts; the lifecycle and scratch-state guards serialize each whole owner operation by design (D12A); releasing them early would admit interleaved owner calls"
)]
impl GuestNetworkProvisioner for HostGuestNetworkProvisioner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let mut lifecycle = self.allocation_lifecycle.lock().await;
        if lifecycle.quiescing {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapCreate,
                expected: GuestNetworkFact::SharedComponent {
                    component: SharedGuestNetworkComponent::Bridge,
                    healthy: true,
                },
                observed: Some(GuestNetworkFact::SharedComponent {
                    component: SharedGuestNetworkComponent::Bridge,
                    healthy: false,
                }),
            });
        }
        if lifecycle.allocations.contains_key(plan.alloc()) {
            return Ok(());
        }
        let mut ifindex = None;
        let mut program_id = None;
        let mut egress_program_id = None;
        let mut rollback_links = GuestNetworkRollbackLinks::default();
        let primary = async {
            self.allocation_io
                .create_tap(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapCreate, source))?;

            let first_tap =
                self.allocation_io.observe_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            let (expected, observed) = Self::tap_fact(plan, &first_tap, false);
            if observed.is_none()
                || expected != observed.clone().unwrap_or_else(|| expected.clone())
            {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    observed,
                });
            }
            let (tap_ifindex, tap_master) = match first_tap {
                GuestNetworkAllocationTapObservation::Persistent {
                    ifindex,
                    master_ifindex,
                    ..
                }
                | GuestNetworkAllocationTapObservation::Incompatible {
                    ifindex,
                    master_ifindex,
                    ..
                } => (ifindex, master_ifindex),
                GuestNetworkAllocationTapObservation::Absent { .. } => {
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected: GuestNetworkFact::Tap {
                            name: plan.assignment().tap.clone(),
                            ifindex: None,
                            link_kind: GuestLinkKind::Tap,
                            persistent: true,
                            up: false,
                            owner_uid: Some(0),
                        },
                        observed: None,
                    });
                }
            };
            ifindex = Some(tap_ifindex);

            self.allocation_io.attach_tap_to_bridge(plan).await.map_err(|source| {
                Self::netlink_error(GuestNetworkOperation::TapAttachBridge, source)
            })?;
            let bridge_ifindex = self.observe_bridge(plan).await?;
            let second_tap =
                self.allocation_io.observe_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            let (expected_second, observed_second) = Self::tap_fact(plan, &second_tap, false);
            if observed_second.is_none()
                || expected_second
                    != observed_second.clone().unwrap_or_else(|| expected_second.clone())
            {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected: expected_second,
                    observed: observed_second,
                });
            }
            let second_master = match second_tap {
                GuestNetworkAllocationTapObservation::Persistent { master_ifindex, .. } => {
                    master_ifindex
                }
                GuestNetworkAllocationTapObservation::Incompatible { .. }
                | GuestNetworkAllocationTapObservation::Absent { .. } => {
                    let (expected, observed) = Self::tap_fact(plan, &second_tap, false);
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed,
                    });
                }
            };
            Self::ensure_master(tap_ifindex, bridge_ifindex, second_master.or(tap_master))?;

            self.allocation_io.insert_guard_member(plan).map_err(|error| {
                Self::guard_error(GuestNetworkOperation::GuardMemberInsert, error)
            })?;
            let expected_members = self.expected_guard_members(Some(plan), None);
            let guard = self.allocation_io.observe_guard(&expected_members).map_err(|error| {
                Self::guard_error(GuestNetworkOperation::GuardMemberInsert, error)
            })?;
            Self::guard_postcondition(&expected_members, guard)?;

            self.allocation_io
                .insert_endpoint(plan, tap_ifindex)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::EndpointInsert, source))?;
            let endpoint =
                self.allocation_io.read_endpoint(plan, tap_ifindex).map_err(|source| {
                    Self::tcx_error(GuestNetworkOperation::EndpointMapObserve, source)
                })?;
            let expected_endpoint = GuestTcxEndpoint {
                source_ipv4: plan.assignment().address,
                source_mac: plan.assignment().mac,
                bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
            };
            if endpoint != Some(expected_endpoint) {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::EndpointMapObserve,
                    expected: GuestNetworkFact::EndpointMapEntry {
                        ifindex: tap_ifindex,
                        value: Some(GuestEndpointFact {
                            source_ip: expected_endpoint.source_ipv4,
                            source_mac: expected_endpoint.source_mac,
                            bridge_mac: expected_endpoint.bridge_mac,
                        }),
                    },
                    observed: Some(GuestNetworkFact::EndpointMapEntry {
                        ifindex: tap_ifindex,
                        value: endpoint.map(|value| GuestEndpointFact {
                            source_ip: value.source_ipv4,
                            source_mac: value.source_mac,
                            bridge_mac: value.bridge_mac,
                        }),
                    }),
                });
            }

            self.allocation_io
                .attach_first_ingress(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxAttach, source))?;
            rollback_links.ingress = GuestNetworkRollbackLink::Pending;
            let attached_program = self
                .allocation_io
                .pin_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))?;
            rollback_links.ingress = GuestNetworkRollbackLink::Pinned;
            program_id = Some(attached_program);
            let attachment = self
                .allocation_io
                .query_attachment(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxQuery, source))?;
            if !attachment.program_ids.contains(&attached_program) {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TcxQuery,
                    expected: GuestNetworkFact::TcxAttachment {
                        ifindex: tap_ifindex,
                        program_id: Some(attached_program),
                        attach_point: Some(TcxAttachPoint::Ingress),
                    },
                    observed: Some(GuestNetworkFact::TcxAttachment {
                        ifindex: tap_ifindex,
                        program_id: attachment.program_ids.first().copied(),
                        attach_point: Some(TcxAttachPoint::Ingress),
                    }),
                });
            }
            if !self
                .allocation_io
                .link_pin_present(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))?
            {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TcxLinkPin,
                    expected: GuestNetworkFact::BpfLinkPin { path: PathBuf::new(), link_id: None },
                    observed: Some(GuestNetworkFact::BpfLinkPin {
                        path: PathBuf::new(),
                        link_id: None,
                    }),
                });
            }

            self.allocation_io.attach_first_egress(plan).map_err(|source| {
                Self::tcx_error(GuestNetworkOperation::TcxEgressAttach, source)
            })?;
            rollback_links.egress = GuestNetworkRollbackLink::Pending;
            let attached_egress_program =
                self.allocation_io.pin_egress_link(plan).map_err(|source| {
                    Self::tcx_error(GuestNetworkOperation::TcxEgressLinkPin, source)
                })?;
            rollback_links.egress = GuestNetworkRollbackLink::Pinned;
            egress_program_id = Some(attached_egress_program);
            let egress_attachment = self
                .allocation_io
                .query_egress_attachment(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxEgressQuery, source))?;
            if !egress_attachment.program_ids.contains(&attached_egress_program) {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TcxEgressQuery,
                    expected: GuestNetworkFact::TcxAttachment {
                        ifindex: tap_ifindex,
                        program_id: Some(attached_egress_program),
                        attach_point: Some(TcxAttachPoint::Egress),
                    },
                    observed: Some(GuestNetworkFact::TcxAttachment {
                        ifindex: tap_ifindex,
                        program_id: egress_attachment.program_ids.first().copied(),
                        attach_point: Some(TcxAttachPoint::Egress),
                    }),
                });
            }
            if !self.allocation_io.egress_link_pin_present(plan).map_err(|source| {
                Self::tcx_error(GuestNetworkOperation::TcxEgressLinkPin, source)
            })? {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TcxEgressLinkPin,
                    expected: GuestNetworkFact::BpfLinkPin { path: PathBuf::new(), link_id: None },
                    observed: Some(GuestNetworkFact::BpfLinkPin {
                        path: PathBuf::new(),
                        link_id: None,
                    }),
                });
            }

            let final_bridge = self.observe_bridge(plan).await?;
            let final_tap =
                self.allocation_io.observe_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            let (mut expected, observed) = Self::tap_fact(plan, &final_tap, false);
            if let GuestNetworkFact::Tap { ifindex, .. } = &mut expected {
                *ifindex = Some(tap_ifindex);
            }
            if observed.is_none()
                || expected != observed.clone().unwrap_or_else(|| expected.clone())
            {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    observed,
                });
            }
            let (final_ifindex, final_master, final_mac) = match &final_tap {
                GuestNetworkAllocationTapObservation::Persistent {
                    ifindex,
                    master_ifindex,
                    mac,
                    ..
                } => (*ifindex, *master_ifindex, *mac),
                GuestNetworkAllocationTapObservation::Incompatible { .. } => {
                    let (expected, observed) = Self::tap_fact(plan, &final_tap, false);
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed,
                    });
                }
                GuestNetworkAllocationTapObservation::Absent { .. } => (tap_ifindex, None, None),
            };
            if final_ifindex != tap_ifindex {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected: Self::master_fact(tap_ifindex, Some(final_bridge)),
                    observed: Some(Self::master_fact(final_ifindex, final_master)),
                });
            }
            Self::ensure_master(final_ifindex, final_bridge, final_master)?;
            let reserved = Self::reserved_host_macs(&lifecycle, Some(plan));
            let (expected_host_mac, observed_host_mac) =
                Self::tap_host_mac_facts(final_ifindex, final_mac, &reserved);
            if let Some(observed) = observed_host_mac {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected: expected_host_mac,
                    observed: Some(observed),
                });
            }
            let debug_mask =
                self.allocation_io.observe_tap_debug_msg_mask(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            match debug_mask {
                Some(0) => {}
                Some(mask) => {
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected: GuestNetworkFact::TapDebugMsgMask {
                            ifindex: final_ifindex,
                            mask: 0,
                        },
                        observed: Some(GuestNetworkFact::TapDebugMsgMask {
                            ifindex: final_ifindex,
                            mask,
                        }),
                    });
                }
                None => {
                    let (expected, _) = Self::tap_fact(plan, &final_tap, false);
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed: None,
                    });
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = primary {
            let (cleanup, tap_removed) =
                self.rollback_provision(plan, ifindex, rollback_links, false).await;
            if let Some(cleanup_error) = cleanup {
                self.rollback_pending.lock().insert(
                    plan.alloc().clone(),
                    GuestNetworkPendingRollback {
                        plan: plan.clone(),
                        ifindex,
                        links: rollback_links,
                        tap_removed,
                    },
                );
                return Err(cleanup_error);
            }
            return Err(error);
        }
        let Some(ifindex) = ifindex else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapObserve,
                expected: GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: false,
                    owner_uid: Some(0),
                },
                observed: None,
            });
        };
        let Some(program_id) = program_id else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TcxQuery,
                expected: GuestNetworkFact::TcxAttachment {
                    ifindex,
                    program_id: None,
                    attach_point: Some(TcxAttachPoint::Ingress),
                },
                observed: None,
            });
        };
        let Some(egress_program_id) = egress_program_id else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TcxEgressQuery,
                expected: GuestNetworkFact::TcxAttachment {
                    ifindex,
                    program_id: None,
                    attach_point: Some(TcxAttachPoint::Egress),
                },
                observed: None,
            });
        };
        self.allocations.lock().insert(
            plan.alloc().clone(),
            HostGuestNetworkAllocationState {
                plan: plan.clone(),
                tap: plan.assignment().tap.clone(),
                ifindex,
                program_id,
                egress_program_id,
                phase: HostGuestNetworkAllocationPhase::ProvisionedDown,
            },
        );
        lifecycle.allocations.insert(
            plan.alloc().clone(),
            HostGuestNetworkAllocationState {
                plan: plan.clone(),
                tap: plan.assignment().tap.clone(),
                ifindex,
                program_id,
                egress_program_id,
                phase: HostGuestNetworkAllocationPhase::ProvisionedDown,
            },
        );
        Ok(())
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> Result<TapActivation> {
        let mut lifecycle = self.allocation_lifecycle.lock().await;
        if lifecycle.quiescing {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapSetUp,
                expected: GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: true,
                    owner_uid: Some(0),
                },
                observed: Some(GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: false,
                    owner_uid: Some(0),
                }),
            });
        }
        let state = lifecycle.allocations.get(plan.alloc()).cloned().ok_or_else(|| {
            GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapSetUp,
                expected: GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: true,
                    owner_uid: Some(0),
                },
                observed: None,
            }
        })?;
        if matches!(state.phase, HostGuestNetworkAllocationPhase::Active) {
            return Ok(TapActivation::Raised);
        }

        let bridge_ifindex = self.observe_bridge(plan).await?;
        let tap = self
            .allocation_io
            .observe_tap(plan)
            .await
            .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapObserve, source))?;
        let (expected_tap, observed_tap) = Self::tap_fact(plan, &tap, false);
        if expected_tap != observed_tap.clone().unwrap_or_else(|| expected_tap.clone()) {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapObserve,
                expected: expected_tap,
                observed: observed_tap,
            });
        }
        let tap_ifindex = match tap {
            GuestNetworkAllocationTapObservation::Persistent {
                ifindex, master_ifindex, ..
            } => {
                Self::ensure_master(ifindex, bridge_ifindex, master_ifindex)?;
                ifindex
            }
            GuestNetworkAllocationTapObservation::Incompatible { .. }
            | GuestNetworkAllocationTapObservation::Absent { .. } => {
                let (expected, observed) = Self::tap_fact(plan, &tap, false);
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    observed,
                });
            }
        };
        let expected_members = self.expected_guard_members(None, None);
        let guard = self
            .allocation_io
            .observe_guard(&expected_members)
            .map_err(|error| Self::guard_error(GuestNetworkOperation::GuardMemberInsert, error))?;
        Self::guard_postcondition(&expected_members, guard)?;
        let endpoint = self
            .allocation_io
            .read_endpoint(plan, tap_ifindex)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::EndpointMapObserve, source))?;
        let expected_endpoint = GuestTcxEndpoint {
            source_ipv4: plan.assignment().address,
            source_mac: plan.assignment().mac,
            bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
        };
        if endpoint != Some(expected_endpoint) {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::EndpointMapObserve,
                expected: GuestNetworkFact::EndpointMapEntry {
                    ifindex: tap_ifindex,
                    value: Some(GuestEndpointFact {
                        source_ip: expected_endpoint.source_ipv4,
                        source_mac: expected_endpoint.source_mac,
                        bridge_mac: expected_endpoint.bridge_mac,
                    }),
                },
                observed: Some(GuestNetworkFact::EndpointMapEntry {
                    ifindex: tap_ifindex,
                    value: endpoint.map(|value| GuestEndpointFact {
                        source_ip: value.source_ipv4,
                        source_mac: value.source_mac,
                        bridge_mac: value.bridge_mac,
                    }),
                }),
            });
        }
        let attachment = self
            .allocation_io
            .query_attachment(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxQuery, source))?;
        if attachment.program_ids != vec![state.program_id] {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TcxQuery,
                expected: GuestNetworkFact::TcxAttachment {
                    ifindex: tap_ifindex,
                    program_id: Some(state.program_id),
                    attach_point: Some(TcxAttachPoint::Ingress),
                },
                observed: Some(GuestNetworkFact::TcxAttachment {
                    ifindex: tap_ifindex,
                    program_id: attachment.program_ids.first().copied(),
                    attach_point: Some(TcxAttachPoint::Ingress),
                }),
            });
        }
        if !self
            .allocation_io
            .link_pin_present(plan)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))?
        {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TcxLinkPin,
                expected: GuestNetworkFact::BpfLinkPin { path: PathBuf::new(), link_id: None },
                observed: Some(GuestNetworkFact::BpfLinkPin {
                    path: PathBuf::new(),
                    link_id: None,
                }),
            });
        }

        if let Err(source) = self.allocation_io.set_tap_up(plan).await {
            self.allocations.lock().entry(plan.alloc().clone()).and_modify(|state| {
                state.phase = HostGuestNetworkAllocationPhase::ProvisionedDown;
            });
            lifecycle
                .allocations
                .entry(plan.alloc().clone())
                .and_modify(|state| state.phase = HostGuestNetworkAllocationPhase::ProvisionedDown);
            return Err(Self::netlink_error(GuestNetworkOperation::TapSetUp, source));
        }
        let result = async {
            let final_bridge = self.observe_bridge(plan).await?;
            let final_tap =
                self.allocation_io.observe_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            let (expected, observed) = Self::tap_fact(plan, &final_tap, true);
            if expected != observed.clone().unwrap_or_else(|| expected.clone()) {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    observed,
                });
            }
            if let GuestNetworkAllocationTapObservation::Persistent {
                ifindex,
                master_ifindex,
                ..
            } = final_tap
            {
                Self::ensure_master(ifindex, final_bridge, master_ifindex)?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            let down =
                self.allocation_io.set_tap_down(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapSetDown, source)
                });
            self.allocations.lock().entry(plan.alloc().clone()).and_modify(|state| {
                state.phase = HostGuestNetworkAllocationPhase::ProvisionedDown;
            });
            lifecycle
                .allocations
                .entry(plan.alloc().clone())
                .and_modify(|state| state.phase = HostGuestNetworkAllocationPhase::ProvisionedDown);
            return match down {
                Ok(()) => Err(error),
                Err(cleanup) => Err(cleanup),
            };
        }
        self.allocations.lock().entry(plan.alloc().clone()).and_modify(|state| {
            state.phase = HostGuestNetworkAllocationPhase::Active;
        });
        lifecycle
            .allocations
            .entry(plan.alloc().clone())
            .and_modify(|state| state.phase = HostGuestNetworkAllocationPhase::Active);
        Ok(TapActivation::Raised)
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let mut lifecycle = self.allocation_lifecycle.lock().await;
        let pending_rollback = { self.rollback_pending.lock().get(plan.alloc()).cloned() };
        if let Some(pending) = pending_rollback {
            let (error, tap_removed) = self
                .rollback_provision(
                    &pending.plan,
                    pending.ifindex,
                    pending.links,
                    pending.tap_removed,
                )
                .await;
            if let Some(error) = error {
                self.rollback_pending
                    .lock()
                    .entry(plan.alloc().clone())
                    .and_modify(|state| state.tap_removed = tap_removed);
                return Err(error);
            }
            self.rollback_pending.lock().remove(plan.alloc());
            return Ok(());
        }
        let Some(state) = lifecycle
            .allocations
            .get(plan.alloc())
            .cloned()
            .or_else(|| self.allocations.lock().get(plan.alloc()).cloned())
        else {
            return Ok(());
        };
        let mut first = None;
        macro_rules! attempt {
            ($result:expr) => {
                if let Err(error) = $result {
                    if first.is_none() {
                        first = Some(error);
                    }
                }
            };
        }
        let tap_present = match self.allocation_io.observe_tap(plan).await {
            Ok(GuestNetworkAllocationTapObservation::Absent { .. }) => false,
            Ok(_) => true,
            Err(source) => {
                if first.is_none() {
                    first = Some(Self::netlink_error(GuestNetworkOperation::TapObserve, source));
                }
                true
            }
        };
        attempt!(
            self.allocation_io
                .remove_endpoint(plan, state.ifindex)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::EndpointDelete, source))
        );
        attempt!(
            self.allocation_io
                .detach_pinned_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxDetach, source))
        );
        attempt!(
            self.allocation_io
                .detach_pinned_egress_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxEgressDetach, source))
        );
        attempt!(
            self.allocation_io
                .read_endpoint(plan, state.ifindex)
                .map_err(|source| Self::tcx_error(
                    GuestNetworkOperation::EndpointMapObserve,
                    source
                ))
                .and_then(|value| {
                    if value.is_none() {
                        Ok(())
                    } else {
                        Err(GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::EndpointMapObserve,
                            expected: GuestNetworkFact::EndpointMapEntry {
                                ifindex: state.ifindex,
                                value: None,
                            },
                            observed: Some(GuestNetworkFact::EndpointMapEntry {
                                ifindex: state.ifindex,
                                value: value.map(|endpoint| GuestEndpointFact {
                                    source_ip: endpoint.source_ipv4,
                                    source_mac: endpoint.source_mac,
                                    bridge_mac: endpoint.bridge_mac,
                                }),
                            }),
                        })
                    }
                })
        );
        if tap_present {
            attempt!(
                self.allocation_io
                    .query_attachment(plan)
                    .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxQuery, source))
                    .and_then(|attachment| {
                        if attachment.program_ids.is_empty() {
                            Ok(())
                        } else {
                            Err(GuestNetworkError::PostconditionMismatch {
                                operation: GuestNetworkOperation::TcxQuery,
                                expected: GuestNetworkFact::TcxAttachment {
                                    ifindex: state.ifindex,
                                    program_id: None,
                                    attach_point: None,
                                },
                                observed: Some(GuestNetworkFact::TcxAttachment {
                                    ifindex: state.ifindex,
                                    program_id: attachment.program_ids.first().copied(),
                                    attach_point: Some(TcxAttachPoint::Ingress),
                                }),
                            })
                        }
                    })
            );
            attempt!(
                self.allocation_io
                    .query_egress_attachment(plan)
                    .map_err(|source| {
                        Self::tcx_error(GuestNetworkOperation::TcxEgressQuery, source)
                    })
                    .and_then(|attachment| {
                        if attachment.program_ids.is_empty() {
                            Ok(())
                        } else {
                            Err(GuestNetworkError::PostconditionMismatch {
                                operation: GuestNetworkOperation::TcxEgressQuery,
                                expected: GuestNetworkFact::TcxAttachment {
                                    ifindex: state.ifindex,
                                    program_id: None,
                                    attach_point: None,
                                },
                                observed: Some(GuestNetworkFact::TcxAttachment {
                                    ifindex: state.ifindex,
                                    program_id: attachment.program_ids.first().copied(),
                                    attach_point: Some(TcxAttachPoint::Egress),
                                }),
                            })
                        }
                    })
            );
        }
        attempt!(
            self.allocation_io
                .link_pin_present(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))
                .and_then(|present| {
                    if present {
                        Err(GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: None,
                            },
                            observed: Some(GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: Some(state.program_id),
                            }),
                        })
                    } else {
                        Ok(())
                    }
                })
        );
        attempt!(
            self.allocation_io
                .egress_link_pin_present(plan)
                .map_err(|source| {
                    Self::tcx_error(GuestNetworkOperation::TcxEgressLinkPin, source)
                })
                .and_then(|present| {
                    if present {
                        Err(GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxEgressLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: None,
                            },
                            observed: Some(GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: Some(state.egress_program_id),
                            }),
                        })
                    } else {
                        Ok(())
                    }
                })
        );
        if tap_present {
            attempt!(self.allocation_io.set_tap_down(plan).await.map_err(|source| {
                Self::netlink_error(GuestNetworkOperation::TapSetDown, source)
            }));
            attempt!(
                self.allocation_io
                    .observe_tap(plan)
                    .await
                    .map_err(|source| {
                        Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                    })
                    .and_then(|observation| {
                        let (expected, observed) = Self::tap_fact(plan, &observation, false);
                        if expected == observed.clone().unwrap_or_else(|| expected.clone()) {
                            Ok(())
                        } else {
                            Err(GuestNetworkError::PostconditionMismatch {
                                operation: GuestNetworkOperation::TapObserve,
                                expected,
                                observed,
                            })
                        }
                    })
            );
            attempt!(self.allocation_io.delete_tap(plan).await.map_err(|source| {
                Self::netlink_error(GuestNetworkOperation::TapDelete, source)
            }));
        }
        attempt!(
            self.allocation_io.delete_guard_member(plan).map_err(|error| Self::guard_error(
                GuestNetworkOperation::GuardMemberDelete,
                error
            ))
        );
        attempt!(
            self.allocation_io
                .observe_tap(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapObserve, source))
                .and_then(|observation| match observation {
                    GuestNetworkAllocationTapObservation::Absent { .. } => Ok(()),
                    other => {
                        let (expected, observed) = Self::tap_fact(plan, &other, false);
                        Err(GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TapObserve,
                            expected,
                            observed,
                        })
                    }
                })
        );
        let expected_members = self.expected_guard_members(None, Some(plan.alloc()));
        attempt!(
            self.allocation_io
                .observe_guard(&expected_members)
                .map_err(|error| Self::guard_error(GuestNetworkOperation::CleanupComplement, error))
                .and_then(|observed| Self::guard_postcondition(&expected_members, observed))
        );
        if let Some(error) = first {
            return Err(error);
        }
        self.allocations.lock().remove(plan.alloc());
        lifecycle.allocations.remove(plan.alloc());
        Ok(())
    }
}

#[async_trait::async_trait]
#[expect(
    clippy::too_many_lines,
    reason = "the approved host-owner port keeps startup, sweep, convergence, audit, and allocation lifecycle on one owner"
)]
#[allow(
    clippy::significant_drop_tightening,
    reason = "the lifecycle and scratch-state guards serialize each whole owner operation by design (D12A); releasing them early would admit interleaved owner calls"
)]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        let plan = Self::scratch_plan();
        let primary = self.run_probe(&plan).await.err();
        let (cleanup, observed) = self.cleanup(&plan).await;
        tracing::info!(
            event = "guest_network.shared_owner_startup_probe_cleanup_observed",
            primary_failed = primary.is_some(),
            cleanup_failed = cleanup.is_some(),
            fully_observed = observed.is_fully_observed(),
            empty = observed.is_empty(),
            bridges = ?observed.bridges,
            taps = ?observed.taps,
            endpoint_maps = ?observed.endpoint_maps,
            counter_maps = ?observed.counter_maps,
            endpoint_entries = ?observed.endpoint_entries,
            tcx_programs = ?observed.tcx_programs,
            tcx_links = ?observed.tcx_links,
            endpoint_map_pins = ?observed.endpoint_map_pins,
            counter_map_pins = ?observed.counter_map_pins,
            tcx_link_pins = ?observed.tcx_link_pins,
            bridge_guard_tables = ?observed.bridge_guard_tables,
            bridge_guard_chains = ?observed.bridge_guard_chains,
            bridge_guard_sets = ?observed.bridge_guard_sets,
            bridge_guard_rules = ?observed.bridge_guard_rules,
            bridge_guard_members = ?observed.bridge_guard_members,
        );
        if cleanup.is_none() && observed.is_empty() {
            return primary.map_or(Ok(()), Err);
        }
        let cleanup = cleanup.unwrap_or(GuestNetworkError::ScratchCleanupIncomplete);
        Err(GuestNetworkError::StartupProbeCleanup {
            primary: primary.map(Box::new),
            cleanup: Box::new(cleanup),
            observed,
        })
    }
    #[expect(
        clippy::collapsible_if,
        clippy::match_same_arms,
        clippy::too_many_lines,
        clippy::unnested_or_patterns,
        reason = "stale shared-guard cleanup keeps the exact owned-member recovery sequence explicit"
    )]
    async fn sweep_stale(&self) -> Result<()> {
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            let mut first_error = None;
            match tokio::fs::read_dir("/sys/class/net").await {
                Ok(mut entries) => loop {
                    let entry = match entries.next_entry().await {
                        Ok(Some(entry)) => entry,
                        Ok(None) => break,
                        Err(source) => {
                            if first_error.is_none() {
                                first_error =
                                    Some(overdrive_netlink::NetlinkError::connect(source));
                            }
                            continue;
                        }
                    };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("ovd-tp-") {
                        if let Err(error) = client.del_link(&name).await {
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                        }
                    }
                },
                Err(source) => {
                    first_error = Some(overdrive_netlink::NetlinkError::connect(source));
                }
            }
            for path in [
                "/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints",
                "/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters",
            ] {
                match tokio::fs::remove_file(path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        if first_error.is_none() {
                            first_error =
                                Some(overdrive_netlink::NetlinkError::nft("stale-pin", error));
                        }
                    }
                }
            }
            let links = std::path::Path::new("/sys/fs/bpf/overdrive/mtls-endpoints/links");
            if let Ok(mut entries) = tokio::fs::read_dir(links).await {
                loop {
                    let path = match entries.next_entry().await {
                        Ok(Some(entry)) => entry.path(),
                        Ok(None) => break,
                        Err(source) => {
                            if first_error.is_none() {
                                first_error =
                                    Some(overdrive_netlink::NetlinkError::connect(source));
                            }
                            continue;
                        }
                    };
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) if first_error.is_none() => {
                            first_error =
                                Some(overdrive_netlink::NetlinkError::nft("stale-link-pin", error));
                        }
                        Err(_) => {}
                    }
                }
            }
            let guard = BridgeGuardSpec::new(
                "overdrive-mtls".to_owned(),
                "prerouting".to_owned(),
                "managed_taps".to_owned(),
                -300,
                0x295a,
                0x295b,
            )
            .map_err(|error| {
                overdrive_netlink::NetlinkError::nft(
                    "stale-guard",
                    std::io::Error::other(error.to_string()),
                )
            })?;
            let expected_members = BTreeSet::new();
            match overdrive_netlink::nft::bridge::observe(&guard, &expected_members) {
                Ok(BridgeGuardObservation::Conflict { inventory }) => {
                    // A prior process can leave owned TAP names in the
                    // node-global managed set while its allocation owner is
                    // gone. Remove only the accepted Overdrive TAP-name
                    // members, then let delete_owned_guard retain any foreign
                    // rule/child conflict as a typed refusal.
                    for member in inventory.members {
                        if let overdrive_netlink::nft::bridge::BridgeGuardMemberIdentity::Ifname(
                            name,
                        ) = member.identity
                            && name.starts_with("ovd-tp-")
                            && let Err(error) =
                                overdrive_netlink::nft::bridge::delete_member(&guard, &name)
                            && first_error.is_none()
                        {
                            first_error = Some(overdrive_netlink::NetlinkError::nft(
                                "stale-guard-member",
                                std::io::Error::other(error.to_string()),
                            ));
                        }
                    }
                    if first_error.is_none()
                        && let Err(error) = overdrive_netlink::nft::bridge::delete_owned_guard(
                            &guard,
                            &expected_members,
                        )
                    {
                        first_error = Some(overdrive_netlink::NetlinkError::nft(
                            "stale-guard",
                            std::io::Error::other(error.to_string()),
                        ));
                    }
                }
                Ok(BridgeGuardObservation::Absent { .. })
                | Ok(BridgeGuardObservation::Exact { .. }) => {
                    if let Err(error) = overdrive_netlink::nft::bridge::delete_owned_guard(
                        &guard,
                        &expected_members,
                    ) {
                        if first_error.is_none() {
                            first_error = Some(overdrive_netlink::NetlinkError::nft(
                                "stale-guard",
                                std::io::Error::other(error.to_string()),
                            ));
                        }
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(overdrive_netlink::NetlinkError::nft(
                            "stale-guard",
                            std::io::Error::other(error.to_string()),
                        ));
                    }
                }
            }
            first_error.map_or(Ok(()), Err)
        })
        .map_err(|source| GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::TapDelete,
            source,
        })
    }
    async fn converge_shared(&self) -> Result<()> {
        const BRIDGE: &str = "ovd-gbr0";
        const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
        let mut lifecycle = self.allocation_lifecycle.lock().await;
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            client.ensure_bridge(BRIDGE, overdrive_core::dataplane::GUEST_BRIDGE_MAC).await?;
            client.set_link_down(BRIDGE).await?;
            client.set_link_mac(BRIDGE, overdrive_core::dataplane::GUEST_BRIDGE_MAC).await?;
            client.converge_addr(BRIDGE, GATEWAY, 16).await?;
            Ok(())
        })
        .map_err(|source| GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::BridgeConverge,
            source,
        })?;
        let (bridge_identity, gateway_present) =
            overdrive_netlink::block_on_host_netlink(|| async {
                let client = overdrive_netlink::Client::new()?;
                let identity = client.observe_link_identity(BRIDGE).await?;
                let gateway_present = if identity.is_some() {
                    client.observe_addr(BRIDGE, GATEWAY, 16).await?
                } else {
                    false
                };
                Ok((identity, gateway_present))
            })
            .map_err(|source| GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::BridgeObserve,
                source,
            })?;
        let Some(bridge_identity) = bridge_identity else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected: GuestNetworkFact::BridgeLinkIdentity {
                    name: BRIDGE.to_owned(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Bridge,
                },
                observed: None,
            });
        };
        let observed_kind = match bridge_identity.kind {
            overdrive_netlink::ObservedLinkKind::Bridge => GuestLinkKind::Bridge,
            overdrive_netlink::ObservedLinkKind::Tap => GuestLinkKind::Tap,
            overdrive_netlink::ObservedLinkKind::Tun => GuestLinkKind::Tun,
            overdrive_netlink::ObservedLinkKind::Veth
            | overdrive_netlink::ObservedLinkKind::Other => GuestLinkKind::Other,
        };
        if observed_kind != GuestLinkKind::Bridge
            || bridge_identity.mac != Some(overdrive_core::dataplane::GUEST_BRIDGE_MAC)
            || bridge_identity.up
            || !gateway_present
        {
            let expected_gateway = Ipv4Net::new_assert(GATEWAY, 16);
            let observed = match bridge_identity.mac {
                Some(mac) => GuestNetworkFact::Bridge {
                    name: bridge_identity.name,
                    ifindex: Some(bridge_identity.ifindex),
                    link_kind: observed_kind,
                    mac,
                    up: bridge_identity.up,
                    gateway: gateway_present.then_some(expected_gateway),
                },
                None => GuestNetworkFact::BridgeLinkIdentity {
                    name: bridge_identity.name,
                    ifindex: Some(bridge_identity.ifindex),
                    link_kind: observed_kind,
                },
            };
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected: GuestNetworkFact::Bridge {
                    name: BRIDGE.to_owned(),
                    ifindex: Some(bridge_identity.ifindex),
                    link_kind: GuestLinkKind::Bridge,
                    mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                    up: false,
                    gateway: Some(expected_gateway),
                },
                observed: Some(observed),
            });
        }
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            client.set_link_up(BRIDGE).await?;
            enable_bridge_nf_call_iptables(BRIDGE).map_err(NetlinkError::connect)?;
            Ok(())
        })
        .map_err(|source| GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::BridgeConverge,
            source,
        })?;
        let guard = overdrive_netlink::nft::bridge::BridgeGuardSpec::new(
            "overdrive-mtls".to_owned(),
            "prerouting".to_owned(),
            "managed_taps".to_owned(),
            -300,
            0x295a,
            0x295b,
        )
        .map_err(|error| GuestNetworkError::Io {
            operation: GuestNetworkOperation::BridgeConverge,
            source: std::io::Error::other(error.to_string()),
        })?;
        for result in [
            overdrive_netlink::nft::bridge::converge_table(&guard),
            overdrive_netlink::nft::bridge::converge_chain(&guard),
            overdrive_netlink::nft::bridge::converge_set(&guard),
            overdrive_netlink::nft::bridge::converge_rules(&guard),
        ] {
            result.map_err(|error| GuestNetworkError::Io {
                operation: GuestNetworkOperation::BridgeConverge,
                source: std::io::Error::other(error.to_string()),
            })?;
        }
        let guard_observation = overdrive_netlink::nft::bridge::observe(&guard, &BTreeSet::new())
            .map_err(|error| GuestNetworkError::Io {
            operation: GuestNetworkOperation::BridgeObserve,
            source: std::io::Error::other(error.to_string()),
        })?;
        let (guard_exact, guard_rules, guard_member) = match guard_observation {
            BridgeGuardObservation::Exact { inventory } => (
                true,
                inventory.rules.into_iter().map(|rule| rule.fact).collect(),
                !inventory.members.is_empty(),
            ),
            BridgeGuardObservation::Absent { inventory }
            | BridgeGuardObservation::Conflict { inventory } => (
                false,
                inventory.rules.into_iter().map(|rule| rule.fact).collect(),
                !inventory.members.is_empty(),
            ),
        };
        if !guard_exact {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected: GuestNetworkFact::BridgeGuard {
                    tap: String::new(),
                    member: false,
                    rules: guard.expected_rule_facts(),
                },
                observed: Some(GuestNetworkFact::BridgeGuard {
                    tap: String::new(),
                    member: guard_member,
                    rules: guard_rules,
                }),
            });
        }
        let endpoint_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints");
        let counter_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters");
        let capture = GuestTcxInventoryIdentity::capture(endpoint_pin.clone(), counter_pin.clone());
        let (identity, disposition) = capture.into_parts();
        disposition.map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLoad, source))?;
        let mut program = GuestTcxProgram::load(&identity)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLoad, source))?;
        program
            .pin_endpoint_map(&endpoint_pin)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::EndpointMapPin, source))?;
        program
            .pin_counter_map(&counter_pin)
            .map_err(|source| Self::tcx_error(GuestNetworkOperation::CounterMapPin, source))?;
        *self.tcx.lock() = Some(HostGuestTcxState { program, inventory: identity });
        if lifecycle.quiescing {
            let quiesced = lifecycle
                .allocations
                .iter()
                .filter(|(_, state)| {
                    matches!(state.phase, HostGuestNetworkAllocationPhase::QuiescedActive)
                })
                .map(|(alloc, state)| (alloc.clone(), state.clone()))
                .collect::<Vec<_>>();
            for (alloc, state) in quiesced {
                let plan = state.plan.clone();
                self.allocation_io.set_tap_up(&plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapSetUp, source)
                })?;
                let observed = self.allocation_io.observe_tap(&plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
                let (expected, actual) = Self::tap_fact(&plan, &observed, true);
                if expected != actual.clone().unwrap_or_else(|| expected.clone()) {
                    return Err(GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed: actual,
                    });
                }
                if let GuestNetworkAllocationTapObservation::Persistent {
                    ifindex,
                    master_ifindex,
                    ..
                } = observed
                {
                    let bridge_ifindex = self.observe_bridge(&plan).await?;
                    Self::ensure_master(ifindex, bridge_ifindex, master_ifindex)?;
                }
                if let Some(state) = lifecycle.allocations.get_mut(&alloc) {
                    state.phase = HostGuestNetworkAllocationPhase::Active;
                }
                self.allocations.lock().entry(alloc).and_modify(|state| {
                    state.phase = HostGuestNetworkAllocationPhase::Active;
                });
            }
            lifecycle.quiescing = false;
        }
        Ok(())
    }
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        const BRIDGE: &str = "ovd-gbr0";
        const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
        let lifecycle = self.allocation_lifecycle.lock().await;
        let (bridge, gateway_present) =
            self.allocation_io.observe_shared_bridge().await.map_err(|source| {
                SharedGuestNetworkAuditError {
                    component: SharedGuestNetworkComponent::Bridge,
                    source: GuestNetworkError::Netlink {
                        operation: GuestNetworkOperation::BridgeObserve,
                        source,
                    },
                }
            })?;
        let bridge_ok = bridge.as_ref().is_some_and(|identity| {
            matches!(identity.kind, overdrive_netlink::ObservedLinkKind::Bridge)
                && identity.mac == Some(overdrive_core::dataplane::GUEST_BRIDGE_MAC)
                && identity.up
                && gateway_present == Some(true)
        });
        if !bridge_ok {
            let expected_gateway = Ipv4Net::new_assert(GATEWAY, 16);
            let observed = bridge.as_ref().map(|identity| {
                let link_kind = match identity.kind {
                    overdrive_netlink::ObservedLinkKind::Bridge => GuestLinkKind::Bridge,
                    overdrive_netlink::ObservedLinkKind::Tap => GuestLinkKind::Tap,
                    overdrive_netlink::ObservedLinkKind::Tun => GuestLinkKind::Tun,
                    overdrive_netlink::ObservedLinkKind::Veth
                    | overdrive_netlink::ObservedLinkKind::Other => GuestLinkKind::Other,
                };
                identity.mac.map_or_else(
                    || GuestNetworkFact::BridgeLinkIdentity {
                        name: identity.name.clone(),
                        ifindex: Some(identity.ifindex),
                        link_kind,
                    },
                    |mac| GuestNetworkFact::Bridge {
                        name: identity.name.clone(),
                        ifindex: Some(identity.ifindex),
                        link_kind,
                        mac,
                        up: identity.up,
                        gateway: (gateway_present == Some(true)).then_some(expected_gateway),
                    },
                )
            });
            return Err(SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::Bridge,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::BridgeObserve,
                    expected: GuestNetworkFact::Bridge {
                        name: BRIDGE.to_owned(),
                        ifindex: None,
                        link_kind: GuestLinkKind::Bridge,
                        mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                        up: true,
                        gateway: Some(expected_gateway),
                    },
                    observed,
                },
            });
        }
        let guard = BridgeGuardSpec::new(
            "overdrive-mtls".to_owned(),
            "prerouting".to_owned(),
            "managed_taps".to_owned(),
            -300,
            0x295a,
            0x295b,
        )
        .map_err(|error| SharedGuestNetworkAuditError {
            component: SharedGuestNetworkComponent::Bridge,
            source: GuestNetworkError::Io {
                operation: GuestNetworkOperation::BridgeObserve,
                source: std::io::Error::other(error.to_string()),
            },
        })?;
        let managed_taps = lifecycle
            .allocations
            .values()
            .map(|state| state.plan.assignment().tap.clone())
            .collect::<BTreeSet<_>>();
        let guard_observation =
            self.allocation_io.observe_guard(&managed_taps).map_err(|error| {
                SharedGuestNetworkAuditError {
                    component: SharedGuestNetworkComponent::BridgeGuard,
                    source: Self::guard_error(GuestNetworkOperation::BridgeObserve, error),
                }
            })?;
        let guard_inventory = match guard_observation {
            BridgeGuardObservation::Absent { inventory }
            | BridgeGuardObservation::Exact { inventory }
            | BridgeGuardObservation::Conflict { inventory } => inventory,
        };
        let guard_table = BridgeGuardTableFact {
            family: BridgeGuardObservedFamily::Bridge,
            name: "overdrive-mtls".to_owned(),
        };
        let expected_guard_rules = guard.expected_rule_facts();
        let observed_guard_rules =
            guard_inventory.rules.iter().map(|rule| rule.fact.clone()).collect::<Vec<_>>();
        let guard_structure_is_exact = guard_inventory.tables == vec![guard_table.clone()]
            && guard_inventory.chains.len() == 1
            && guard_inventory.chains.first().is_some_and(|chain| {
                chain.table == guard_table
                    && chain.name == "prerouting"
                    && chain.definition
                        == BridgeGuardChainDefinition::Base {
                            chain_type: BridgeGuardChainType::Filter,
                            hook: BridgeGuardChainHook::Prerouting,
                            priority: -300,
                            policy: Some(BridgeGuardChainPolicy::Accept),
                        }
            })
            && guard_inventory.sets.len() == 1
            && guard_inventory.sets.first().is_some_and(|set| {
                set.table == guard_table
                    && set.name == "managed_taps"
                    && set.key_len == 16
                    && set.ifname_key
            })
            && guard_inventory.rules.len() == expected_guard_rules.len()
            && guard_inventory
                .rules
                .iter()
                .all(|rule| rule.table == guard_table && rule.chain == "prerouting")
            && observed_guard_rules == expected_guard_rules
            && guard_inventory.other_children.is_empty();
        let unmanaged_guard_member = guard_inventory.members.iter().any(|member| {
            if member.table != guard_table || member.set != "managed_taps" {
                return true;
            }
            match &member.identity {
                BridgeGuardMemberIdentity::Ifname(name) => !managed_taps.contains(name),
                BridgeGuardMemberIdentity::ForeignEncoding { .. } => true,
            }
        });
        if !guard_structure_is_exact || unmanaged_guard_member {
            return Err(SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::BridgeGuard,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::BridgeObserve,
                    expected: GuestNetworkFact::BridgeGuard {
                        tap: String::new(),
                        member: false,
                        rules: expected_guard_rules,
                    },
                    observed: Some(GuestNetworkFact::BridgeGuard {
                        tap: String::new(),
                        member: !guard_inventory.members.is_empty(),
                        rules: observed_guard_rules,
                    }),
                },
            });
        }
        let managed_ifindices =
            lifecycle.allocations.values().map(|state| state.ifindex).collect::<BTreeSet<_>>();
        for (read, component, expected) in [
            (GuestNetworkNodeTcxAuditRead::Programs, SharedGuestNetworkComponent::TcxLink, 2),
            (
                GuestNetworkNodeTcxAuditRead::EndpointMap,
                SharedGuestNetworkComponent::EndpointMap,
                1,
            ),
            (GuestNetworkNodeTcxAuditRead::CounterMap, SharedGuestNetworkComponent::CounterMap, 1),
            (
                GuestNetworkNodeTcxAuditRead::EndpointMapPin,
                SharedGuestNetworkComponent::BpffsPin,
                1,
            ),
            (GuestNetworkNodeTcxAuditRead::CounterMapPin, SharedGuestNetworkComponent::BpffsPin, 1),
        ] {
            let count = self.allocation_io.observe_shared_tcx(read, &managed_ifindices).map_err(
                |source| SharedGuestNetworkAuditError {
                    component,
                    source: GuestNetworkError::Tcx {
                        operation: GuestNetworkOperation::CleanupComplement,
                        source,
                    },
                },
            )?;
            if count != expected {
                return Err(SharedGuestNetworkAuditError {
                    component,
                    source: GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::CleanupComplement,
                        expected: GuestNetworkFact::SharedComponent { component, healthy: true },
                        observed: Some(GuestNetworkFact::SharedComponent {
                            component,
                            healthy: false,
                        }),
                    },
                });
            }
        }
        let endpoint_entries = self
            .allocation_io
            .observe_shared_tcx(GuestNetworkNodeTcxAuditRead::EndpointEntries, &managed_ifindices)
            .map_err(|source| SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::EndpointMap,
                source: GuestNetworkError::Tcx {
                    operation: GuestNetworkOperation::CleanupComplement,
                    source,
                },
            })?;
        if usize::try_from(endpoint_entries).unwrap_or(usize::MAX) > lifecycle.allocations.len() {
            let component = SharedGuestNetworkComponent::EndpointMap;
            return Err(SharedGuestNetworkAuditError {
                component,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::CleanupComplement,
                    expected: GuestNetworkFact::SharedComponent { component, healthy: true },
                    observed: Some(GuestNetworkFact::SharedComponent { component, healthy: false }),
                },
            });
        }
        let debug_msg_masks =
            self.allocation_io.observe_debug_msg_masks().await.map_err(|source| {
                SharedGuestNetworkAuditError {
                    component: SharedGuestNetworkComponent::Bridge,
                    source: Self::netlink_error(GuestNetworkOperation::TapObserve, source),
                }
            })?;
        let bridge_ifindex = bridge.as_ref().map_or(0, |identity| identity.ifindex);
        let reserved_macs = Self::reserved_host_macs(&lifecycle, None);
        let mut audit = SharedGuestNetworkAudit::default();
        for (alloc, state) in &lifecycle.allocations {
            let plan = &state.plan;
            let expected_up = matches!(state.phase, HostGuestNetworkAllocationPhase::Active);
            let observed = match self.allocation_io.observe_tap(plan).await {
                Ok(observed) => observed,
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::netlink_error(GuestNetworkOperation::TapObserve, source),
                    );
                    continue;
                }
            };
            let (mut expected_tap, observed_tap) = Self::tap_fact(plan, &observed, expected_up);
            if let GuestNetworkFact::Tap { ifindex, .. } = &mut expected_tap {
                *ifindex = Some(state.ifindex);
            }
            if observed_tap.is_none()
                || expected_tap != observed_tap.clone().unwrap_or_else(|| expected_tap.clone())
            {
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected: expected_tap,
                        observed: observed_tap,
                    },
                );
                continue;
            }
            let (ifindex, master_ifindex, mac) = match &observed {
                GuestNetworkAllocationTapObservation::Persistent {
                    ifindex,
                    master_ifindex,
                    mac,
                    ..
                } => (*ifindex, *master_ifindex, *mac),
                GuestNetworkAllocationTapObservation::Absent { .. }
                | GuestNetworkAllocationTapObservation::Incompatible { .. } => continue,
            };
            let (expected_host_mac, observed_host_mac) =
                Self::tap_host_mac_facts(ifindex, mac, &reserved_macs);
            if let Some(observed) = observed_host_mac {
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected: expected_host_mac,
                        observed: Some(observed),
                    },
                );
                continue;
            }
            let Some(mask) = debug_msg_masks.get(&ifindex).copied() else {
                let (expected, _) = Self::tap_fact(plan, &observed, expected_up);
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected,
                        observed: None,
                    },
                );
                continue;
            };
            if mask != 0 {
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::TapObserve,
                        expected: GuestNetworkFact::TapDebugMsgMask { ifindex, mask: 0 },
                        observed: Some(GuestNetworkFact::TapDebugMsgMask { ifindex, mask }),
                    },
                );
                continue;
            }
            if let Err(error) = Self::ensure_master(ifindex, bridge_ifindex, master_ifindex) {
                audit.damaged.insert(alloc.clone(), error);
                continue;
            }
            let ingress = match self.allocation_io.query_attachment(plan) {
                Ok(attachment) if attachment.program_ids == vec![state.program_id] => true,
                Ok(attachment) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxQuery,
                            expected: GuestNetworkFact::TcxAttachment {
                                ifindex,
                                program_id: Some(state.program_id),
                                attach_point: Some(TcxAttachPoint::Ingress),
                            },
                            observed: Some(GuestNetworkFact::TcxAttachment {
                                ifindex,
                                program_id: attachment.program_ids.first().copied(),
                                attach_point: Some(TcxAttachPoint::Ingress),
                            }),
                        },
                    );
                    false
                }
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::tcx_error(GuestNetworkOperation::TcxQuery, source),
                    );
                    false
                }
            };
            if !ingress {
                continue;
            }
            match self.allocation_io.link_pin_present(plan) {
                Ok(true) => {}
                Ok(false) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: Some(state.program_id),
                            },
                            observed: Some(GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: None,
                            }),
                        },
                    );
                    continue;
                }
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source),
                    );
                    continue;
                }
            }
            let egress = match self.allocation_io.query_egress_attachment(plan) {
                Ok(attachment) if attachment.program_ids == vec![state.egress_program_id] => true,
                Ok(attachment) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxEgressQuery,
                            expected: GuestNetworkFact::TcxAttachment {
                                ifindex,
                                program_id: Some(state.egress_program_id),
                                attach_point: Some(TcxAttachPoint::Egress),
                            },
                            observed: Some(GuestNetworkFact::TcxAttachment {
                                ifindex,
                                program_id: attachment.program_ids.first().copied(),
                                attach_point: Some(TcxAttachPoint::Egress),
                            }),
                        },
                    );
                    false
                }
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::tcx_error(GuestNetworkOperation::TcxEgressQuery, source),
                    );
                    false
                }
            };
            if !egress {
                continue;
            }
            match self.allocation_io.egress_link_pin_present(plan) {
                Ok(true) => {}
                Ok(false) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxEgressLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: Some(state.egress_program_id),
                            },
                            observed: Some(GuestNetworkFact::BpfLinkPin {
                                path: PathBuf::new(),
                                link_id: None,
                            }),
                        },
                    );
                    continue;
                }
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::tcx_error(GuestNetworkOperation::TcxEgressLinkPin, source),
                    );
                    continue;
                }
            }
            let endpoint = match self.allocation_io.read_endpoint(plan, ifindex) {
                Ok(endpoint) => endpoint,
                Err(source) => {
                    audit.damaged.insert(
                        alloc.clone(),
                        Self::tcx_error(GuestNetworkOperation::EndpointMapObserve, source),
                    );
                    continue;
                }
            };
            let expected_endpoint = GuestTcxEndpoint {
                source_ipv4: plan.assignment().address,
                source_mac: plan.assignment().mac,
                bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
            };
            if endpoint != Some(expected_endpoint) {
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::EndpointMapObserve,
                        expected: GuestNetworkFact::EndpointMapEntry {
                            ifindex,
                            value: Some(GuestEndpointFact {
                                source_ip: expected_endpoint.source_ipv4,
                                source_mac: expected_endpoint.source_mac,
                                bridge_mac: expected_endpoint.bridge_mac,
                            }),
                        },
                        observed: Some(GuestNetworkFact::EndpointMapEntry {
                            ifindex,
                            value: endpoint.map(|value| GuestEndpointFact {
                                source_ip: value.source_ipv4,
                                source_mac: value.source_mac,
                                bridge_mac: value.bridge_mac,
                            }),
                        }),
                    },
                );
                continue;
            }
            let has_guard_member = guard_inventory.members.iter().any(|member| {
                matches!(&member.identity, BridgeGuardMemberIdentity::Ifname(name) if name == &state.tap)
            });
            if !has_guard_member {
                audit.damaged.insert(
                    alloc.clone(),
                    GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::GuardMemberInsert,
                        expected: GuestNetworkFact::BridgeGuard {
                            tap: state.tap.clone(),
                            member: true,
                            rules: expected_guard_rules.clone(),
                        },
                        observed: Some(GuestNetworkFact::BridgeGuard {
                            tap: state.tap.clone(),
                            member: false,
                            rules: observed_guard_rules.clone(),
                        }),
                    },
                );
            }
        }
        Ok(audit)
    }
    async fn quiesce_managed_taps(&self) -> Result<TapQuiescence> {
        let mut lifecycle = self.allocation_lifecycle.lock().await;
        if lifecycle.quiescing {
            return Ok(TapQuiescence::default());
        }
        lifecycle.quiescing = true;
        let active = lifecycle
            .allocations
            .iter()
            .filter(|(_, state)| matches!(state.phase, HostGuestNetworkAllocationPhase::Active))
            .map(|(alloc, state)| (alloc.clone(), state.tap.clone()))
            .collect::<Vec<_>>();
        for (alloc, tap) in active {
            let down = overdrive_netlink::block_on_host_netlink(|| async {
                let client = overdrive_netlink::Client::new()?;
                client.set_link_down(&tap).await?;
                client.observe_link(&tap).await
            })
            .map_err(|source| GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::TapSetDown,
                source,
            })?;
            if down != Some(false) {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapSetDown,
                    expected: GuestNetworkFact::LinkUp { ifindex: 0, up: false },
                    observed: Some(GuestNetworkFact::LinkUp { ifindex: 0, up: true }),
                });
            }
            if let Some(state) = lifecycle.allocations.get_mut(&alloc) {
                state.phase = HostGuestNetworkAllocationPhase::QuiescedActive;
            }
            self.allocations.lock().entry(alloc).and_modify(|state| {
                state.phase = HostGuestNetworkAllocationPhase::QuiescedActive;
            });
        }
        Ok(TapQuiescence::default())
    }
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 06-04")]
    async fn restore_quiesced_taps(&self) -> Result<()> {
        todo!("RED scaffold: D-295-R14 restore_quiesced_taps — DELIVER step 06-04")
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::unnested_or_patterns
)]
mod scratch_probe_acceptance {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ScratchCall {
        Netlink(GuestNetworkScratchNetlinkAction),
        Tcx(GuestNetworkScratchTcxAction),
        CloseLoader,
        ReleaseAdopted,
        Exercise(GuestNetworkProbeStage),
        CountNetlink(GuestNetworkScratchNetlinkResource),
        CountTcx(GuestNetworkScratchTcxResource),
    }

    #[derive(Debug, Default)]
    struct Script {
        calls: Vec<ScratchCall>,
        fail: Option<ScratchCall>,
        fail_occurrence: usize,
        semantic_failure: Option<GuestNetworkProbeStage>,
        residue: Option<ScratchCall>,
    }

    #[derive(Debug, Default)]
    struct ScriptedScratchIo {
        script: parking_lot::Mutex<Script>,
    }

    impl ScriptedScratchIo {
        fn with_failure(fail: ScratchCall) -> Arc<Self> {
            Arc::new(Self {
                script: parking_lot::Mutex::new(Script {
                    fail: Some(fail),
                    fail_occurrence: 1,
                    ..Script::default()
                }),
            })
        }

        fn with_cleanup_failure(fail: ScratchCall) -> Arc<Self> {
            let occurrence = usize::from(matches!(
                fail,
                ScratchCall::Tcx(GuestNetworkScratchTcxAction::UnpinLink)
                    | ScratchCall::Tcx(GuestNetworkScratchTcxAction::DetachLink)
            )) + 1;
            Arc::new(Self {
                script: parking_lot::Mutex::new(Script {
                    fail: Some(fail),
                    fail_occurrence: occurrence,
                    ..Script::default()
                }),
            })
        }

        fn with_semantic_failure(stage: GuestNetworkProbeStage) -> Arc<Self> {
            Arc::new(Self {
                script: parking_lot::Mutex::new(Script {
                    semantic_failure: Some(stage),
                    ..Script::default()
                }),
            })
        }

        fn with_residue(resource: ScratchCall) -> Arc<Self> {
            Arc::new(Self {
                script: parking_lot::Mutex::new(Script {
                    residue: Some(resource),
                    ..Script::default()
                }),
            })
        }

        fn calls(&self) -> Vec<ScratchCall> {
            self.script.lock().calls.clone()
        }

        fn record(&self, call: ScratchCall) -> bool {
            let mut script = self.script.lock();
            let occurrence = script.calls.iter().filter(|prior| **prior == call).count() + 1;
            script.calls.push(call);
            script.fail == Some(call) && script.fail_occurrence == occurrence
        }
    }

    fn netlink_error() -> NetlinkError {
        NetlinkError::Connect { source: std::io::Error::other("scripted netlink failure") }
    }

    fn tcx_error() -> GuestTcxError {
        GuestTcxError::Io { source: std::io::Error::other("scripted TCX I/O failure") }
    }

    #[async_trait::async_trait]
    impl SharedGuestNetworkScratchIo for ScriptedScratchIo {
        async fn apply_netlink(
            &self,
            _plan: &GuestNetworkScratchPlan,
            action: GuestNetworkScratchNetlinkAction,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(ScratchCall::Netlink(action)) { Err(netlink_error()) } else { Ok(()) }
        }

        async fn apply_tcx(
            &self,
            _plan: &GuestNetworkScratchPlan,
            action: GuestNetworkScratchTcxAction,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(ScratchCall::Tcx(action)) { Err(tcx_error()) } else { Ok(()) }
        }

        fn close_loader_handles(&self, _plan: &GuestNetworkScratchPlan) {
            let _ = self.record(ScratchCall::CloseLoader);
        }

        fn release_adopted_handles(&self, _plan: &GuestNetworkScratchPlan) {
            let _ = self.record(ScratchCall::ReleaseAdopted);
        }

        fn probe_program_id(&self) -> Option<u32> {
            Some(297)
        }

        fn probe_attachment(
            &self,
            _plan: &GuestNetworkScratchPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            Ok(GuestTcxAttachment { revision: 1, program_ids: vec![297] })
        }

        async fn exercise(
            &self,
            _plan: &GuestNetworkScratchPlan,
            stage: GuestNetworkProbeStage,
        ) -> std::io::Result<bool> {
            let call = ScratchCall::Exercise(stage);
            if self.record(call) {
                return Err(std::io::Error::other("scripted exercise transport failure"));
            }
            Ok(self.script.lock().semantic_failure != Some(stage))
        }

        async fn count_netlink(
            &self,
            _plan: &GuestNetworkScratchPlan,
            resource: GuestNetworkScratchNetlinkResource,
        ) -> std::result::Result<u32, NetlinkError> {
            let call = ScratchCall::CountNetlink(resource);
            if self.record(call) {
                Err(netlink_error())
            } else if self.script.lock().residue == Some(call) {
                Ok(1)
            } else {
                Ok(0)
            }
        }

        async fn count_tcx(
            &self,
            _plan: &GuestNetworkScratchPlan,
            resource: GuestNetworkScratchTcxResource,
        ) -> std::result::Result<u32, GuestTcxError> {
            let call = ScratchCall::CountTcx(resource);
            if self.record(call) {
                Err(tcx_error())
            } else if self.script.lock().residue == Some(call) {
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }

    const CLEANUP: &[ScratchCall] = &[
        ScratchCall::CloseLoader,
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::DeleteEndpoint),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::UnpinLink),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::DetachLink),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::UnpinCounterMap),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::UnpinEndpointMap),
        ScratchCall::ReleaseAdopted,
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::SetTapDown),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteTap),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardMember),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardRules),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardSet),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardChain),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardTable),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteBridge),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::Bridge),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::Tap),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardTable),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardChain),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardSet),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardRule),
        ScratchCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardMember),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::EndpointMap),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::CounterMap),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::EndpointEntry),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::TcxProgram),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::TcxLink),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::EndpointMapPin),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::CounterMapPin),
        ScratchCall::CountTcx(GuestNetworkScratchTcxResource::TcxLinkPin),
    ];

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn exercise_transport_and_semantic_failures_are_distinct_for_every_probe_stage() {
        for stage in [
            GuestNetworkProbeStage::Classifier,
            GuestNetworkProbeStage::OriginalDestination,
            GuestNetworkProbeStage::DetachedLinkGuard,
        ] {
            let transport = ScriptedScratchIo::with_failure(ScratchCall::Exercise(stage));
            let error = HostSharedGuestNetworkOwner::with_scratch_io(transport)
                .probe_startup()
                .await
                .expect_err("transport failure");
            assert!(matches!(
                error,
                GuestNetworkError::Io { operation: GuestNetworkOperation::StartupProbe, .. }
            ));

            let semantic = ScriptedScratchIo::with_semantic_failure(stage);
            let error = HostSharedGuestNetworkOwner::with_scratch_io(semantic)
                .probe_startup()
                .await
                .expect_err("semantic failure");
            assert!(matches!(
                error,
                GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::StartupProbe,
                    expected: GuestNetworkFact::StartupProbe { stage: observed_stage, passed: true },
                    observed: Some(GuestNetworkFact::StartupProbe { stage: actual_stage, passed: false }),
                } if observed_stage == stage && actual_stage == stage
            ));
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_cleanup_or_inventory_failure_is_aggregated_after_the_remaining_cleanup() {
        for fail in CLEANUP
            .iter()
            .copied()
            .filter(|call| !matches!(call, ScratchCall::CloseLoader | ScratchCall::ReleaseAdopted))
        {
            let io = ScriptedScratchIo::with_cleanup_failure(fail);
            let error = HostSharedGuestNetworkOwner::with_scratch_io(io.clone())
                .probe_startup()
                .await
                .expect_err("cleanup failure");
            assert!(matches!(error, GuestNetworkError::StartupProbeCleanup { primary: None, .. }));
            assert!(io.calls().ends_with(&CLEANUP[1..]));
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_observed_residue_family_returns_incomplete_with_the_owner_built_complement() {
        for resource in CLEANUP
            .iter()
            .copied()
            .filter(|call| matches!(call, ScratchCall::CountNetlink(_) | ScratchCall::CountTcx(_)))
        {
            let io = ScriptedScratchIo::with_residue(resource);
            let error = HostSharedGuestNetworkOwner::with_scratch_io(io)
                .probe_startup()
                .await
                .expect_err("non-empty complement");
            assert!(matches!(
                error,
                GuestNetworkError::StartupProbeCleanup {
                    primary: None,
                    cleanup,
                    observed,
                } if matches!(*cleanup, GuestNetworkError::ScratchCleanupIncomplete)
                    && observed.is_fully_observed()
                    && !observed.is_empty()
            ));
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn primary_and_first_cleanup_failure_are_both_preserved_without_nested_aggregate() {
        let io = Arc::new(ScriptedScratchIo {
            script: parking_lot::Mutex::new(Script {
                fail: Some(ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteTap)),
                fail_occurrence: 1,
                semantic_failure: Some(GuestNetworkProbeStage::Classifier),
                ..Script::default()
            }),
        });
        let error = HostSharedGuestNetworkOwner::with_scratch_io(io)
            .probe_startup()
            .await
            .expect_err("primary plus cleanup aggregate");
        assert!(matches!(
            error,
            GuestNetworkError::StartupProbeCleanup {
                primary: Some(primary),
                cleanup,
                ..
            } if !matches!(*primary, GuestNetworkError::StartupProbeCleanup { .. })
                && !matches!(*cleanup, GuestNetworkError::StartupProbeCleanup { .. })
        ));
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    reason = "scripted packet-probe fixtures mutate their script under one lock per port call"
)]
mod scratch_probe_packet_acceptance {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ProbeTarget {
        Peer,
        Gateway,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PacketProbeCall {
        Netlink(GuestNetworkScratchNetlinkAction),
        Tcx(GuestNetworkScratchTcxAction),
        Probe(GuestNetworkProbeStage, ProbeTarget),
        DetachedGuard,
        CloseLoader,
        LazyAdoptForCleanup,
        ReleaseAdopted,
        CountNetlink(GuestNetworkScratchNetlinkResource),
        CountTcx(GuestNetworkScratchTcxResource),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ProbeFailure {
        TypedSource,
        Semantic,
    }

    #[derive(Debug, Default)]
    struct PacketProbeScript {
        calls: Vec<PacketProbeCall>,
        failure: Option<(GuestNetworkProbeStage, ProbeFailure)>,
        adopted: bool,
        lazy_adopted: bool,
    }

    #[derive(Debug, Default)]
    struct PacketProbeIo {
        script: parking_lot::Mutex<PacketProbeScript>,
    }

    impl PacketProbeIo {
        fn with_failure(stage: GuestNetworkProbeStage, failure: ProbeFailure) -> Arc<Self> {
            Arc::new(Self {
                script: parking_lot::Mutex::new(PacketProbeScript {
                    failure: Some((stage, failure)),
                    ..PacketProbeScript::default()
                }),
            })
        }

        fn calls(&self) -> Vec<PacketProbeCall> {
            self.script.lock().calls.clone()
        }

        fn record(&self, call: PacketProbeCall) {
            self.script.lock().calls.push(call);
        }
    }

    #[async_trait::async_trait]
    impl SharedGuestNetworkScratchIo for PacketProbeIo {
        async fn apply_netlink(
            &self,
            _plan: &GuestNetworkScratchPlan,
            action: GuestNetworkScratchNetlinkAction,
        ) -> std::result::Result<(), NetlinkError> {
            self.record(PacketProbeCall::Netlink(action));
            Ok(())
        }

        async fn apply_tcx(
            &self,
            _plan: &GuestNetworkScratchPlan,
            action: GuestNetworkScratchTcxAction,
        ) -> std::result::Result<(), GuestTcxError> {
            let mut script = self.script.lock();
            if matches!(
                action,
                GuestNetworkScratchTcxAction::UnpinLink
                    | GuestNetworkScratchTcxAction::UnpinCounterMap
                    | GuestNetworkScratchTcxAction::UnpinEndpointMap
            ) && !script.adopted
                && !script.lazy_adopted
            {
                script.calls.push(PacketProbeCall::LazyAdoptForCleanup);
                script.adopted = true;
                script.lazy_adopted = true;
            }
            script.calls.push(PacketProbeCall::Tcx(action));
            if matches!(
                action,
                GuestNetworkScratchTcxAction::AdoptEndpointMap
                    | GuestNetworkScratchTcxAction::AdoptCounterMap
                    | GuestNetworkScratchTcxAction::AdoptLink
            ) {
                script.adopted = true;
            }
            Ok(())
        }

        fn close_loader_handles(&self, _plan: &GuestNetworkScratchPlan) {
            self.record(PacketProbeCall::CloseLoader);
        }

        fn release_adopted_handles(&self, _plan: &GuestNetworkScratchPlan) {
            self.record(PacketProbeCall::ReleaseAdopted);
        }

        fn probe_program_id(&self) -> Option<u32> {
            Some(297)
        }

        fn probe_attachment(
            &self,
            _plan: &GuestNetworkScratchPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            Ok(GuestTcxAttachment { revision: 1, program_ids: vec![297] })
        }

        async fn exercise(
            &self,
            _plan: &GuestNetworkScratchPlan,
            stage: GuestNetworkProbeStage,
        ) -> std::io::Result<bool> {
            if stage == GuestNetworkProbeStage::DetachedLinkGuard {
                self.record(PacketProbeCall::DetachedGuard);
                return Ok(true);
            }
            self.record(PacketProbeCall::Probe(stage, ProbeTarget::Peer));
            self.record(PacketProbeCall::Probe(stage, ProbeTarget::Gateway));
            let failure = self.script.lock().failure;
            match failure {
                Some((failed_stage, ProbeFailure::TypedSource)) if failed_stage == stage => {
                    Err(std::io::Error::other(GuestTcxError::Io {
                        source: std::io::Error::from_raw_os_error(libc::EREMOTEIO),
                    }))
                }
                Some((failed_stage, ProbeFailure::Semantic)) if failed_stage == stage => Ok(false),
                _ => Ok(true),
            }
        }

        async fn count_netlink(
            &self,
            _plan: &GuestNetworkScratchPlan,
            resource: GuestNetworkScratchNetlinkResource,
        ) -> std::result::Result<u32, NetlinkError> {
            self.record(PacketProbeCall::CountNetlink(resource));
            Ok(0)
        }

        async fn count_tcx(
            &self,
            _plan: &GuestNetworkScratchPlan,
            resource: GuestNetworkScratchTcxResource,
        ) -> std::result::Result<u32, GuestTcxError> {
            self.record(PacketProbeCall::CountTcx(resource));
            Ok(0)
        }
    }

    const SETUP: &[PacketProbeCall] = &[
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::ConvergeBridge),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::CreateTap),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::AttachTapToBridge),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::SetTapUp),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardTable),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardChain),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardSet),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardRules),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::InsertGuardMember),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::LoadProgramAndMaps),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::PinEndpointMap),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::PinCounterMap),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::InsertEndpoint),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::AttachLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::PinLink),
    ];

    const POST_PROBE: &[PacketProbeCall] = &[
        PacketProbeCall::CloseLoader,
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::AdoptEndpointMap),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::AdoptCounterMap),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::AdoptLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::QueryLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::UnpinLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::DetachLink),
        PacketProbeCall::DetachedGuard,
    ];

    const CLEANUP: &[PacketProbeCall] = &[
        PacketProbeCall::CloseLoader,
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::DeleteEndpoint),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::UnpinLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::DetachLink),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::UnpinCounterMap),
        PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::UnpinEndpointMap),
        PacketProbeCall::ReleaseAdopted,
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::SetTapDown),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteTap),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardMember),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardRules),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardSet),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardChain),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteGuardTable),
        PacketProbeCall::Netlink(GuestNetworkScratchNetlinkAction::DeleteBridge),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::Bridge),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::Tap),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardTable),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardChain),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardSet),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardRule),
        PacketProbeCall::CountNetlink(GuestNetworkScratchNetlinkResource::BridgeGuardMember),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::EndpointMap),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::CounterMap),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::EndpointEntry),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::TcxProgram),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::TcxLink),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::EndpointMapPin),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::CounterMapPin),
        PacketProbeCall::CountTcx(GuestNetworkScratchTcxResource::TcxLinkPin),
    ];

    fn exact_probe_calls(stage: GuestNetworkProbeStage) -> [PacketProbeCall; 2] {
        [
            PacketProbeCall::Probe(stage, ProbeTarget::Peer),
            PacketProbeCall::Probe(stage, ProbeTarget::Gateway),
        ]
    }

    fn cleanup_after_preclose_failure() -> Vec<PacketProbeCall> {
        let mut cleanup = CLEANUP.to_vec();
        cleanup.insert(2, PacketProbeCall::LazyAdoptForCleanup);
        cleanup
    }

    fn d14_expected_input() -> GuestTcxTcpProbeInput {
        GuestTcxTcpProbeInput {
            ingress_ifindex: 295,
            source_ipv4: Ipv4Addr::new(100, 95, 255, 254),
            source_mac: [0x02, 0x00, 100, 95, 255, 254],
            bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
            destination_mac: [0x02, 0x00, 100, 95, 255, 253],
            original_destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 255, 253), 8443),
        }
    }

    fn d14_counter_order() -> [GuestTcxCounter; 8] {
        [
            GuestTcxCounter::GatewayHostPass,
            GuestTcxCounter::Intercept,
            GuestTcxCounter::EndpointMapMiss,
            GuestTcxCounter::SourceMacSpoof,
            GuestTcxCounter::SourceIpArpSpoof,
            GuestTcxCounter::DirectBypassDrop,
            GuestTcxCounter::ArpPass,
            GuestTcxCounter::MalformedDrop,
        ]
    }

    fn d14_valid_observation() -> GuestNetworkTcpProbeObservation {
        let counters = d14_counter_order();
        GuestNetworkTcpProbeObservation {
            verdict: GuestTcxProbeVerdict::Accept,
            mark: GuestTcxProbeMark::Intercept,
            source_mac: Some([0x02, 0x00, 100, 95, 255, 254]),
            destination_mac: Some(overdrive_core::dataplane::GUEST_BRIDGE_MAC),
            original_destination: Some(SocketAddrV4::new(Ipv4Addr::new(100, 95, 255, 253), 8443)),
            counters: std::array::from_fn(|index| GuestTcxProbeCounterObservation {
                counter: counters[index],
                before: u64::try_from(100 + index).expect("small finite table index"),
                after: u64::try_from(100 + index).expect("small finite table index")
                    + u64::from(counters[index] == GuestTcxCounter::Intercept),
            }),
        }
    }

    fn assert_d14_mismatch(
        observed: GuestNetworkTcpProbeObservation,
        requirement: GuestNetworkTcpProbeRequirement,
        expected_mismatch: GuestNetworkTcpProbeMismatch,
    ) {
        assert_eq!(
            validate_guest_tcx_tcp_probe(requirement, &d14_expected_input(), Ok(observed))
                .expect("semantic mismatch is not fabricated as I/O"),
            GuestNetworkTcpProbeValidation::Mismatch(expected_mismatch)
        );
    }

    /// S-ND295-00 — D14A's production validator exposes every mismatch and lower source.
    /// CONTRACT_SHAPE: pure-function.
    #[allow(clippy::too_many_lines, reason = "one finite D14A mismatch table is audited intact")]
    #[test]
    fn every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator() {
        let expected = d14_expected_input();
        let valid = d14_valid_observation();
        assert_eq!(
            validate_guest_tcx_tcp_probe(
                GuestNetworkTcpProbeRequirement::OriginalDestination,
                &expected,
                Ok(valid.clone()),
            )
            .expect("valid semantic observation"),
            GuestNetworkTcpProbeValidation::Passed { intercept_before: 101, intercept_after: 102 }
        );

        let mut classifier_ignores_original_destination = valid.clone();
        classifier_ignores_original_destination.original_destination = None;
        assert!(matches!(
            validate_guest_tcx_tcp_probe(
                GuestNetworkTcpProbeRequirement::Classifier,
                &expected,
                Ok(classifier_ignores_original_destination),
            ),
            Ok(GuestNetworkTcpProbeValidation::Passed { .. })
        ));

        let mut all_wrong = valid.clone();
        all_wrong.verdict = GuestTcxProbeVerdict::Drop;
        all_wrong.mark = GuestTcxProbeMark::Accepted;
        all_wrong.source_mac = None;
        assert_d14_mismatch(
            all_wrong,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::Verdict { observed: GuestTcxProbeVerdict::Drop },
        );

        for observed in [GuestTcxProbeVerdict::Drop, GuestTcxProbeVerdict::Unexpected] {
            let mut wrong = valid.clone();
            wrong.verdict = observed;
            assert_d14_mismatch(
                wrong,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::Verdict { observed },
            );
        }

        for observed in
            [GuestTcxProbeMark::None, GuestTcxProbeMark::Accepted, GuestTcxProbeMark::Unexpected]
        {
            let mut wrong = valid.clone();
            wrong.mark = observed;
            assert_d14_mismatch(
                wrong,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::Mark { observed },
            );
        }

        for observed in [None, Some([0x02, 0, 1, 2, 3, 4])] {
            let mut wrong = valid.clone();
            wrong.source_mac = observed;
            assert_d14_mismatch(
                wrong,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::SourceMac { expected: expected.source_mac, observed },
            );
        }
        for observed in [None, Some([0x02, 0, 4, 3, 2, 1])] {
            let mut wrong = valid.clone();
            wrong.destination_mac = observed;
            assert_d14_mismatch(
                wrong,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::DestinationMac {
                    expected: expected.bridge_mac,
                    observed,
                },
            );
        }
        for observed in [
            None,
            Some(SocketAddrV4::new(
                Ipv4Addr::new(100, 95, 255, 252),
                expected.original_destination.port(),
            )),
            Some(SocketAddrV4::new(
                *expected.original_destination.ip(),
                expected.original_destination.port() + 1,
            )),
            Some(SocketAddrV4::new(Ipv4Addr::new(100, 95, 255, 252), 9443)),
        ] {
            let mut wrong = valid.clone();
            wrong.original_destination = observed;
            assert_d14_mismatch(
                wrong,
                GuestNetworkTcpProbeRequirement::OriginalDestination,
                GuestNetworkTcpProbeMismatch::OriginalDestination {
                    expected: expected.original_destination,
                    observed,
                },
            );
        }

        let counter_order = d14_counter_order();
        let mut permuted_identity = valid.clone();
        let mut permutation = counter_order;
        permutation.rotate_left(1);
        for (observation, counter) in permuted_identity.counters.iter_mut().zip(permutation) {
            observation.counter = counter;
        }
        assert_d14_mismatch(
            permuted_identity,
            GuestNetworkTcpProbeRequirement::Classifier,
            GuestNetworkTcpProbeMismatch::CounterIdentity {
                index: 0,
                expected: counter_order[0],
                observed: counter_order[1],
            },
        );

        for index in 0..8 {
            let mut wrong_identity = valid.clone();
            let observed = counter_order[(index + 1) % counter_order.len()];
            wrong_identity.counters[index].counter = observed;
            assert_d14_mismatch(
                wrong_identity,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::CounterIdentity {
                    index: u8::try_from(index).expect("finite counter index"),
                    expected: counter_order[index],
                    observed,
                },
            );

            let mut wrong_delta = valid.clone();
            let expected_delta = u64::from(counter_order[index] == GuestTcxCounter::Intercept);
            let observed_delta = expected_delta + 1;
            wrong_delta.counters[index].after = wrong_delta.counters[index].before + observed_delta;
            assert_d14_mismatch(
                wrong_delta,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::CounterDelta {
                    counter: counter_order[index],
                    expected: expected_delta,
                    observed: observed_delta,
                },
            );

            for (before, after) in [(9, 8), (u64::MAX, 0)] {
                let mut decrease = valid.clone();
                decrease.counters[index].before = before;
                decrease.counters[index].after = after;
                assert_d14_mismatch(
                    decrease,
                    GuestNetworkTcpProbeRequirement::Classifier,
                    GuestNetworkTcpProbeMismatch::CounterDecrease {
                        counter: counter_order[index],
                        before,
                        after,
                    },
                );
            }
        }

        let mut unchanged_intercept = valid.clone();
        unchanged_intercept.counters[1].after = unchanged_intercept.counters[1].before;
        assert_d14_mismatch(
            unchanged_intercept,
            GuestNetworkTcpProbeRequirement::Classifier,
            GuestNetworkTcpProbeMismatch::CounterDelta {
                counter: GuestTcxCounter::Intercept,
                expected: 1,
                observed: 0,
            },
        );

        let mut verdict_before_mark = valid.clone();
        verdict_before_mark.verdict = GuestTcxProbeVerdict::Unexpected;
        verdict_before_mark.mark = GuestTcxProbeMark::None;
        assert_d14_mismatch(
            verdict_before_mark,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::Verdict { observed: GuestTcxProbeVerdict::Unexpected },
        );

        let mut mark_before_source = valid.clone();
        mark_before_source.mark = GuestTcxProbeMark::Accepted;
        mark_before_source.source_mac = None;
        assert_d14_mismatch(
            mark_before_source,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::Mark { observed: GuestTcxProbeMark::Accepted },
        );

        let mut source_before_destination = valid.clone();
        source_before_destination.source_mac = None;
        source_before_destination.destination_mac = None;
        assert_d14_mismatch(
            source_before_destination,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::SourceMac {
                expected: expected.source_mac,
                observed: None,
            },
        );

        let wrong_ip_same_port = SocketAddrV4::new(
            Ipv4Addr::new(100, 95, 255, 252),
            expected.original_destination.port(),
        );
        let mut destination_before_original = valid.clone();
        destination_before_original.destination_mac = None;
        destination_before_original.original_destination = Some(wrong_ip_same_port);
        assert_d14_mismatch(
            destination_before_original,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::DestinationMac {
                expected: expected.bridge_mac,
                observed: None,
            },
        );

        let mut classifier_destination_before_counter = valid.clone();
        classifier_destination_before_counter.destination_mac = None;
        classifier_destination_before_counter.counters[0].counter = counter_order[1];
        assert_d14_mismatch(
            classifier_destination_before_counter,
            GuestNetworkTcpProbeRequirement::Classifier,
            GuestNetworkTcpProbeMismatch::DestinationMac {
                expected: expected.bridge_mac,
                observed: None,
            },
        );

        let mut original_before_counters = valid.clone();
        original_before_counters.original_destination = Some(wrong_ip_same_port);
        original_before_counters.counters[0].counter = counter_order[1];
        assert_d14_mismatch(
            original_before_counters,
            GuestNetworkTcpProbeRequirement::OriginalDestination,
            GuestNetworkTcpProbeMismatch::OriginalDestination {
                expected: expected.original_destination,
                observed: Some(wrong_ip_same_port),
            },
        );

        for index in 0..7 {
            let lower_counter = counter_order[index];
            let expected_delta = u64::from(lower_counter == GuestTcxCounter::Intercept);
            let observed_delta = expected_delta + 1;
            let mut lower_record_before_next_identity = valid.clone();
            lower_record_before_next_identity.counters[index].after =
                lower_record_before_next_identity.counters[index].before + observed_delta;
            lower_record_before_next_identity.counters[index + 1].counter =
                counter_order[(index + 2) % counter_order.len()];
            assert_d14_mismatch(
                lower_record_before_next_identity,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::CounterDelta {
                    counter: lower_counter,
                    expected: expected_delta,
                    observed: observed_delta,
                },
            );
        }

        for index in 0..8 {
            let counter = counter_order[index];
            let observed_identity = counter_order[(index + 1) % counter_order.len()];
            let mut identity_before_decrease = valid.clone();
            identity_before_decrease.counters[index].counter = observed_identity;
            identity_before_decrease.counters[index].before = 9;
            identity_before_decrease.counters[index].after = 8;
            assert_d14_mismatch(
                identity_before_decrease,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::CounterIdentity {
                    index: u8::try_from(index).expect("finite counter index"),
                    expected: counter,
                    observed: observed_identity,
                },
            );

            let mut decrease_before_delta = valid.clone();
            decrease_before_delta.counters[index].before = u64::MAX;
            decrease_before_delta.counters[index].after = 0;
            assert_d14_mismatch(
                decrease_before_delta,
                GuestNetworkTcpProbeRequirement::Classifier,
                GuestNetworkTcpProbeMismatch::CounterDecrease {
                    counter,
                    before: u64::MAX,
                    after: 0,
                },
            );
        }

        let mut intercept_decrease_before_wrapping_delta = valid;
        intercept_decrease_before_wrapping_delta.counters[1].before = u64::MAX;
        intercept_decrease_before_wrapping_delta.counters[1].after = 1;
        assert_eq!(1_u64.wrapping_sub(u64::MAX), 2);
        assert_d14_mismatch(
            intercept_decrease_before_wrapping_delta,
            GuestNetworkTcpProbeRequirement::Classifier,
            GuestNetworkTcpProbeMismatch::CounterDecrease {
                counter: GuestTcxCounter::Intercept,
                before: u64::MAX,
                after: 1,
            },
        );

        let error = validate_guest_tcx_tcp_probe(
            GuestNetworkTcpProbeRequirement::Classifier,
            &expected,
            Err(GuestTcxError::Io { source: std::io::Error::from_raw_os_error(libc::EREMOTEIO) }),
        )
        .expect_err("lower dataplane source remains an I/O error");
        let typed = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<GuestTcxError>())
            .expect("GuestTcxError remains downcastable");
        assert!(matches!(
            typed,
            GuestTcxError::Io { source }
                if source.raw_os_error() == Some(libc::EREMOTEIO)
        ));
    }

    /// S-ND295-00 — D14A probes twice per stage before close and cleanup remains complete.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn classifier_runs_precede_close_and_each_stage_is_fresh() {
        let io = Arc::new(PacketProbeIo::default());
        HostSharedGuestNetworkOwner::with_scratch_io(io.clone())
            .probe_startup()
            .await
            .expect("four fresh classifier observations and distinct detached guard succeed");
        let calls = io.calls();
        let classifier = exact_probe_calls(GuestNetworkProbeStage::Classifier);
        let original = exact_probe_calls(GuestNetworkProbeStage::OriginalDestination);
        let classifier_index = calls
            .iter()
            .position(|call| *call == classifier[0])
            .expect("classifier peer probe is mandatory");
        let close_index = calls
            .iter()
            .position(|call| *call == PacketProbeCall::CloseLoader)
            .expect("normal loader close is mandatory");
        assert!(
            classifier_index < close_index,
            "D14 classifier exercise must precede normal loader closure"
        );

        let mut expected = SETUP.to_vec();
        expected.extend(classifier);
        expected.extend(original);
        expected.extend_from_slice(POST_PROBE);
        expected.extend_from_slice(CLEANUP);
        assert_eq!(calls, expected);
        let detach_index = calls
            .iter()
            .position(|call| {
                *call == PacketProbeCall::Tcx(GuestNetworkScratchTcxAction::DetachLink)
            })
            .expect("exact TCX detach");
        let guard_index = calls
            .iter()
            .position(|call| *call == PacketProbeCall::DetachedGuard)
            .expect("detached D9 guard stage");
        assert!(detach_index < guard_index, "D9 guard evidence stays after exact TCX detach");

        for stage in
            [GuestNetworkProbeStage::Classifier, GuestNetworkProbeStage::OriginalDestination]
        {
            for failure in [ProbeFailure::TypedSource, ProbeFailure::Semantic] {
                let io = PacketProbeIo::with_failure(stage, failure);
                let error = HostSharedGuestNetworkOwner::with_scratch_io(io.clone())
                    .probe_startup()
                    .await
                    .expect_err("a wrong or missing D14 result never fabricates startup success");
                match failure {
                    ProbeFailure::TypedSource => {
                        let GuestNetworkError::Io {
                            operation: GuestNetworkOperation::StartupProbe,
                            source,
                        } = error
                        else {
                            panic!("typed probe source must remain the StartupProbe I/O primary");
                        };
                        let typed = source
                            .get_ref()
                            .and_then(|source| source.downcast_ref::<GuestTcxError>())
                            .expect("typed GuestTcxError source remains downcastable");
                        assert!(matches!(
                            typed,
                            GuestTcxError::Io { source }
                                if source.raw_os_error() == Some(libc::EREMOTEIO)
                        ));
                    }
                    ProbeFailure::Semantic => assert!(matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::StartupProbe,
                            expected: GuestNetworkFact::StartupProbe {
                                stage: expected_stage,
                                passed: true,
                            },
                            observed: Some(GuestNetworkFact::StartupProbe {
                                stage: observed_stage,
                                passed: false,
                            }),
                        } if expected_stage == stage && observed_stage == stage
                    )),
                }

                let failure_marker = PacketProbeCall::Probe(stage, ProbeTarget::Gateway);
                let calls = io.calls();
                let failure_index = calls
                    .iter()
                    .position(|call| *call == failure_marker)
                    .expect("both peer and gateway observations run fresh before failure");
                let cleanup_start = calls
                    .iter()
                    .enumerate()
                    .skip(failure_index + 1)
                    .find_map(|(index, call)| {
                        (*call == PacketProbeCall::CloseLoader).then_some(index)
                    })
                    .expect("cleanup closes the live loader first");
                assert_eq!(&calls[cleanup_start..], cleanup_after_preclose_failure());
                assert_eq!(
                    calls
                        .iter()
                        .filter(|call| **call == PacketProbeCall::LazyAdoptForCleanup)
                        .count(),
                    1,
                    "the first unpin lazily creates exactly one handle-free adopted state"
                );
                assert!(!calls.contains(&PacketProbeCall::DetachedGuard));
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    clippy::significant_drop_in_scrutinee,
    clippy::option_if_let_else,
    clippy::struct_excessive_bools,
    reason = "source-local D12A owner tables: each expect() names the fixture precondition it establishes, the fake kernel holds its node lock only inside one leaf call, and its node model keeps each independent kernel fact as its own flag"
)]
mod allocation_owner_acceptance {
    //! The one host owner's allocation algorithms, driven through
    //! D-295-DISTILL-12A's module-private allocation leaf I/O
    //! (`with_allocation_io`). Allocation phases:
    //! `Unpublished -> ProvisionedDown -> Active -> QuiescedActive`, with
    //! `Condemned` reached from quiescence or audit, and every phase torn down
    //! to `Absent`. A failed provision stays `Unpublished`; a failed teardown
    //! keeps the allocation for retry.
    //!
    //! Two test doubles implement the private leaf trait:
    //!
    //! - `ScriptedAllocationIo` queues the TAP and bridge observations each
    //!   call returns, for the identity-partition and rollback tables whose
    //!   point is one scripted observation at one checkpoint;
    //! - `FakeAttachmentKernel` models the node (bridge, guard, TCX program and
    //!   maps, endpoint entries, TAPs, attachments, pins, debug masks); every
    //!   leaf reads and writes that model, and a fault is either an out-of-band
    //!   change to it (a removed TAP, a changed MAC) or a leaf failure armed on
    //!   a named call. It journals each call with the TAP it named and whether
    //!   it changed the node.
    //!
    //! Every precondition comes from the owner's own port calls (`provision`,
    //! `activate`, `quiesce_managed_taps`, `audit_shared`); no body writes the
    //! owner's private state.

    use super::*;
    use overdrive_dataplane::guest_tcx::GuestTcxObject;
    use overdrive_netlink::nft::bridge::{
        BridgeGuardChainDefinition, BridgeGuardChainHook, BridgeGuardChainOccurrence,
        BridgeGuardChainPolicy, BridgeGuardChainType, BridgeGuardInventory,
        BridgeGuardMemberIdentity, BridgeGuardMemberOccurrence, BridgeGuardObservedFamily,
        BridgeGuardRuleOccurrence, BridgeGuardSetFact, BridgeGuardTableFact,
    };
    use std::collections::VecDeque;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum AllocationCall {
        CreateTap,
        ObserveTap,
        AttachTap,
        ObserveBridge,
        InsertGuard,
        ObserveGuard,
        InsertEndpoint,
        ReadEndpoint,
        AttachFirstIngress,
        PinLink,
        QueryAttachment,
        LinkPinPresent,
        SetTapUp,
        RemoveEndpoint,
        DetachPendingLink,
        DetachPinnedLink,
        SetTapDown,
        DeleteTap,
        DeleteGuard,
        AttachFirstEgress,
        PinEgressLink,
        QueryEgressAttachment,
        EgressLinkPinPresent,
        DetachPendingEgressLink,
        DetachPinnedEgressLink,
        ObserveTapDebugMsgMask,
        ObserveDebugMsgMasks,
        ObserveTcxPrograms,
        ObserveEndpointMap,
        ObserveCounterMap,
        ObserveEndpointMapPin,
        ObserveCounterMapPin,
        ObserveEndpointEntries,
    }

    /// The TAP owner D-295-R4 requires: the root launcher.
    const ROOT_UID: Option<u32> = Some(0);
    /// Program id every ingress `pin_link` reports.
    const INGRESS_PROGRAM: u32 = 2_950;
    /// Program id every egress `pin_egress_link` reports.
    const EGRESS_PROGRAM: u32 = 2_951;
    /// The shared bridge's ifindex.
    const BRIDGE_IFINDEX: u32 = 29;
    /// The ifindex the fake kernel gives the first TAP it creates.
    const FIRST_TAP_IFINDEX: u32 = 295;

    /// Every leaf that can mutate kernel state.
    const MUTATIONS: [AllocationCall; 17] = [
        AllocationCall::CreateTap,
        AllocationCall::AttachTap,
        AllocationCall::SetTapUp,
        AllocationCall::SetTapDown,
        AllocationCall::DeleteTap,
        AllocationCall::InsertGuard,
        AllocationCall::DeleteGuard,
        AllocationCall::InsertEndpoint,
        AllocationCall::RemoveEndpoint,
        AllocationCall::AttachFirstIngress,
        AllocationCall::PinLink,
        AllocationCall::DetachPendingLink,
        AllocationCall::DetachPinnedLink,
        AllocationCall::AttachFirstEgress,
        AllocationCall::PinEgressLink,
        AllocationCall::DetachPendingEgressLink,
        AllocationCall::DetachPinnedEgressLink,
    ];

    /// Leaves that read one allocation's own attachment parts.
    const PER_ALLOCATION_READS: [AllocationCall; 7] = [
        AllocationCall::ObserveTap,
        AllocationCall::ReadEndpoint,
        AllocationCall::QueryAttachment,
        AllocationCall::LinkPinPresent,
        AllocationCall::QueryEgressAttachment,
        AllocationCall::EgressLinkPinPresent,
        AllocationCall::ObserveTapDebugMsgMask,
    ];

    fn netlink_failure() -> NetlinkError {
        NetlinkError::connect(std::io::Error::from_raw_os_error(libc::EBUSY))
    }

    fn tcx_failure() -> GuestTcxError {
        GuestTcxError::Io { source: std::io::Error::from_raw_os_error(libc::EIO) }
    }

    fn guard_failure() -> BridgeGuardError {
        BridgeGuardError::Netlink(netlink_failure())
    }

    /// What a TCX query of a vanished interface reports.
    fn absent_interface() -> GuestTcxError {
        GuestTcxError::Io { source: std::io::Error::from_raw_os_error(libc::ENODEV) }
    }

    const fn empty_guard_inventory() -> BridgeGuardInventory {
        BridgeGuardInventory {
            generation: 1,
            tables: Vec::new(),
            chains: Vec::new(),
            sets: Vec::new(),
            rules: Vec::new(),
            members: Vec::new(),
            other_children: Vec::new(),
        }
    }

    fn endpoint_for(plan: &GuestNetworkPlan) -> GuestTcxEndpoint {
        GuestTcxEndpoint {
            source_ipv4: plan.assignment().address,
            source_mac: plan.assignment().mac,
            bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
        }
    }

    // ---- queued-observation leaf -------------------------------------------

    /// D12A leaf whose TAP and bridge observations are queued per call (an
    /// empty queue observes absence). TCX, endpoint, and guard read-backs
    /// follow the calls already recorded, so a rollback reads back what it
    /// actually removed.
    struct ScriptedAllocationIo {
        calls: parking_lot::Mutex<Vec<AllocationCall>>,
        tap_observations: parking_lot::Mutex<VecDeque<GuestNetworkAllocationTapObservation>>,
        bridge_observations: parking_lot::Mutex<VecDeque<GuestNetworkAllocationBridgeObservation>>,
        failures: parking_lot::Mutex<BTreeSet<(AllocationCall, usize)>>,
    }

    impl ScriptedAllocationIo {
        fn with_observations(
            taps: impl IntoIterator<Item = GuestNetworkAllocationTapObservation>,
            bridges: impl IntoIterator<Item = GuestNetworkAllocationBridgeObservation>,
        ) -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                tap_observations: parking_lot::Mutex::new(taps.into_iter().collect()),
                bridge_observations: parking_lot::Mutex::new(bridges.into_iter().collect()),
                failures: parking_lot::Mutex::new(BTreeSet::new()),
            })
        }

        /// Fail the `occurrence`-th call of `call` (1-based).
        fn fail(&self, call: AllocationCall, occurrence: usize) {
            self.failures.lock().insert((call, occurrence));
        }

        fn record(&self, call: AllocationCall) -> bool {
            let occurrence = {
                let mut calls = self.calls.lock();
                calls.push(call);
                calls.iter().filter(|prior| **prior == call).count()
            };
            self.failures.lock().contains(&(call, occurrence))
        }

        fn calls(&self) -> Vec<AllocationCall> {
            self.calls.lock().clone()
        }

        /// `effect` happened and no `undo` followed its last occurrence.
        fn holds(&self, effect: AllocationCall, undo: &[AllocationCall]) -> bool {
            let calls = self.calls.lock();
            calls
                .iter()
                .rposition(|call| *call == effect)
                .is_some_and(|last| !calls[last..].iter().any(|call| undo.contains(call)))
        }
    }

    #[async_trait::async_trait]
    impl GuestNetworkAllocationIo for ScriptedAllocationIo {
        async fn create_tap(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(AllocationCall::CreateTap) { Err(netlink_failure()) } else { Ok(()) }
        }
        async fn attach_tap_to_bridge(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(AllocationCall::AttachTap) { Err(netlink_failure()) } else { Ok(()) }
        }
        async fn set_tap_up(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(AllocationCall::SetTapUp) { Err(netlink_failure()) } else { Ok(()) }
        }
        async fn set_tap_down(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(AllocationCall::SetTapDown) { Err(netlink_failure()) } else { Ok(()) }
        }
        async fn delete_tap(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            if self.record(AllocationCall::DeleteTap) { Err(netlink_failure()) } else { Ok(()) }
        }
        async fn observe_tap(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationTapObservation, NetlinkError> {
            if self.record(AllocationCall::ObserveTap) {
                return Err(netlink_failure());
            }
            Ok(self.tap_observations.lock().pop_front().unwrap_or_else(|| {
                GuestNetworkAllocationTapObservation::Absent { name: plan.assignment().tap.clone() }
            }))
        }
        async fn observe_bridge(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationBridgeObservation, NetlinkError> {
            if self.record(AllocationCall::ObserveBridge) {
                return Err(netlink_failure());
            }
            Ok(self.bridge_observations.lock().pop_front().unwrap_or_else(|| {
                GuestNetworkAllocationBridgeObservation::Absent { name: plan.bridge().to_owned() }
            }))
        }
        fn insert_guard_member(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            if self.record(AllocationCall::InsertGuard) {
                Err(guard_failure())
            } else {
                Ok(BridgeGuardMutationOutcome::Converged { observed: empty_guard_inventory() })
            }
        }
        fn delete_guard_member(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            if self.record(AllocationCall::DeleteGuard) {
                Err(guard_failure())
            } else {
                Ok(BridgeGuardMutationOutcome::Converged { observed: empty_guard_inventory() })
            }
        }
        fn observe_guard(
            &self,
            _expected_members: &BTreeSet<String>,
        ) -> std::result::Result<BridgeGuardObservation, BridgeGuardError> {
            if self.record(AllocationCall::ObserveGuard) {
                Err(guard_failure())
            } else {
                Ok(BridgeGuardObservation::Exact { inventory: empty_guard_inventory() })
            }
        }
        fn insert_endpoint(
            &self,
            _plan: &GuestNetworkPlan,
            _ifindex: u32,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::InsertEndpoint) { Err(tcx_failure()) } else { Ok(()) }
        }
        fn read_endpoint(
            &self,
            plan: &GuestNetworkPlan,
            _ifindex: u32,
        ) -> std::result::Result<Option<GuestTcxEndpoint>, GuestTcxError> {
            if self.record(AllocationCall::ReadEndpoint) {
                return Err(tcx_failure());
            }
            Ok(self
                .holds(AllocationCall::InsertEndpoint, &[AllocationCall::RemoveEndpoint])
                .then(|| endpoint_for(plan)))
        }
        fn remove_endpoint(
            &self,
            _plan: &GuestNetworkPlan,
            _ifindex: u32,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::RemoveEndpoint) { Err(tcx_failure()) } else { Ok(()) }
        }
        fn attach_first_ingress(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::AttachFirstIngress) {
                Err(tcx_failure())
            } else {
                Ok(())
            }
        }
        fn pin_link(&self, _plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError> {
            if self.record(AllocationCall::PinLink) {
                Err(tcx_failure())
            } else {
                Ok(INGRESS_PROGRAM)
            }
        }
        fn query_attachment(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            if self.record(AllocationCall::QueryAttachment) {
                return Err(tcx_failure());
            }
            let attached = self.holds(
                AllocationCall::AttachFirstIngress,
                &[AllocationCall::DetachPendingLink, AllocationCall::DetachPinnedLink],
            );
            Ok(GuestTcxAttachment {
                revision: 1,
                program_ids: if attached { vec![INGRESS_PROGRAM] } else { Vec::new() },
            })
        }
        fn link_pin_present(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<bool, GuestTcxError> {
            if self.record(AllocationCall::LinkPinPresent) {
                return Err(tcx_failure());
            }
            Ok(self.holds(AllocationCall::PinLink, &[AllocationCall::DetachPinnedLink]))
        }
        fn detach_pending_link(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::DetachPendingLink) { Err(tcx_failure()) } else { Ok(()) }
        }
        fn detach_pinned_link(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::DetachPinnedLink) { Err(tcx_failure()) } else { Ok(()) }
        }
        fn attach_first_egress(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::AttachFirstEgress) { Err(tcx_failure()) } else { Ok(()) }
        }
        fn pin_egress_link(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<u32, GuestTcxError> {
            if self.record(AllocationCall::PinEgressLink) {
                Err(tcx_failure())
            } else {
                Ok(EGRESS_PROGRAM)
            }
        }
        fn query_egress_attachment(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            if self.record(AllocationCall::QueryEgressAttachment) {
                return Err(tcx_failure());
            }
            let attached = self.holds(
                AllocationCall::AttachFirstEgress,
                &[AllocationCall::DetachPendingEgressLink, AllocationCall::DetachPinnedEgressLink],
            );
            Ok(GuestTcxAttachment {
                revision: 1,
                program_ids: if attached { vec![EGRESS_PROGRAM] } else { Vec::new() },
            })
        }
        fn egress_link_pin_present(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<bool, GuestTcxError> {
            if self.record(AllocationCall::EgressLinkPinPresent) {
                return Err(tcx_failure());
            }
            Ok(self.holds(AllocationCall::PinEgressLink, &[AllocationCall::DetachPinnedEgressLink]))
        }
        fn detach_pending_egress_link(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::DetachPendingEgressLink) {
                Err(tcx_failure())
            } else {
                Ok(())
            }
        }
        fn detach_pinned_egress_link(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            if self.record(AllocationCall::DetachPinnedEgressLink) {
                Err(tcx_failure())
            } else {
                Ok(())
            }
        }
        async fn observe_tap_debug_msg_mask(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<Option<u32>, NetlinkError> {
            if self.record(AllocationCall::ObserveTapDebugMsgMask) {
                Err(netlink_failure())
            } else {
                Ok(Some(0))
            }
        }
        async fn observe_debug_msg_masks(
            &self,
        ) -> std::result::Result<BTreeMap<u32, u32>, NetlinkError> {
            if self.record(AllocationCall::ObserveDebugMsgMasks) {
                Err(netlink_failure())
            } else {
                Ok(BTreeMap::from([(FIRST_TAP_IFINDEX, 0)]))
            }
        }
    }

    // ---- fake attachment kernel --------------------------------------------

    /// Deterministic host-side MAC the fake kernel gives a TAP it creates.
    fn host_mac(ifindex: u32) -> [u8; 6] {
        let [_, _, high, low] = ifindex.to_be_bytes();
        [0xfe, 0x95, 0x00, 0x00, high, low]
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeTap {
        ifindex: u32,
        kind: GuestLinkKind,
        persistent: bool,
        owner_uid: Option<u32>,
        up: bool,
        master: Option<u32>,
        mac: Option<[u8; 6]>,
        debug_mask: u32,
        /// Program ids attached at TCX ingress, ascending.
        ingress: Vec<u32>,
        /// Program ids attached at TCX egress, ascending.
        egress: Vec<u32>,
        /// Administrative set-up / set-down succeed but change nothing.
        admin_stuck: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeBridge {
        ifindex: u32,
        kind: GuestLinkKind,
        mac: [u8; 6],
        up: bool,
        gateway: bool,
    }

    /// The node state the fake kernel models. Node-level parts (bridge,
    /// guard structure, TCX program, map identity and pins) are modelled
    /// beside the per-allocation parts, so a node-level fault is a change to
    /// this state whatever private leaf the owner reads it through.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeNode {
        bridge: Option<FakeBridge>,
        guard_table: bool,
        guard_rules_intact: bool,
        guard_members: BTreeSet<String>,
        tcx_program_loaded: bool,
        endpoint_map_intact: bool,
        counter_map_intact: bool,
        endpoint_map_pinned: bool,
        counter_map_pinned: bool,
        endpoints: BTreeMap<u32, GuestTcxEndpoint>,
        taps: BTreeMap<String, FakeTap>,
        ingress_pins: BTreeSet<String>,
        egress_pins: BTreeSet<String>,
        pending_ingress: BTreeSet<String>,
        pending_egress: BTreeSet<String>,
        /// Ifindexes the debug-mask dump leaves out.
        mask_dump_omits: BTreeSet<u32>,
        next_ifindex: u32,
    }

    /// One attachment's parts as the node holds them.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct AttachmentParts {
        tap: Option<FakeTap>,
        endpoint: Option<GuestTcxEndpoint>,
        ingress_pin: bool,
        egress_pin: bool,
        pending_ingress: bool,
        pending_egress: bool,
        guard_member: bool,
    }

    impl AttachmentParts {
        const fn is_empty(&self) -> bool {
            self.tap.is_none()
                && self.endpoint.is_none()
                && !self.ingress_pin
                && !self.egress_pin
                && !self.pending_ingress
                && !self.pending_egress
                && !self.guard_member
        }
    }

    /// One part of an allocation's attachment that can vanish out of band.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AttachmentPart {
        Tap,
        IngressAttachment,
        EgressAttachment,
        IngressPin,
        EgressPin,
        EndpointEntry,
        GuardMember,
    }

    const EVERY_PART: [AttachmentPart; 7] = [
        AttachmentPart::Tap,
        AttachmentPart::IngressAttachment,
        AttachmentPart::EgressAttachment,
        AttachmentPart::IngressPin,
        AttachmentPart::EgressPin,
        AttachmentPart::EndpointEntry,
        AttachmentPart::GuardMember,
    ];

    impl FakeNode {
        fn healthy() -> Self {
            Self {
                bridge: Some(FakeBridge {
                    ifindex: BRIDGE_IFINDEX,
                    kind: GuestLinkKind::Bridge,
                    mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                    up: true,
                    gateway: true,
                }),
                guard_table: true,
                guard_rules_intact: true,
                guard_members: BTreeSet::new(),
                tcx_program_loaded: true,
                endpoint_map_intact: true,
                counter_map_intact: true,
                endpoint_map_pinned: true,
                counter_map_pinned: true,
                endpoints: BTreeMap::new(),
                taps: BTreeMap::new(),
                ingress_pins: BTreeSet::new(),
                egress_pins: BTreeSet::new(),
                pending_ingress: BTreeSet::new(),
                pending_egress: BTreeSet::new(),
                mask_dump_omits: BTreeSet::new(),
                next_ifindex: FIRST_TAP_IFINDEX,
            }
        }

        fn tap_mut(&mut self, tap: &str) -> &mut FakeTap {
            self.taps.get_mut(tap).expect("the fixture's TAP exists in the node")
        }

        fn parts(&self, tap: &str, ifindex: u32) -> AttachmentParts {
            AttachmentParts {
                tap: self.taps.get(tap).cloned(),
                endpoint: self.endpoints.get(&ifindex).copied(),
                ingress_pin: self.ingress_pins.contains(tap),
                egress_pin: self.egress_pins.contains(tap),
                pending_ingress: self.pending_ingress.contains(tap),
                pending_egress: self.pending_egress.contains(tap),
                guard_member: self.guard_members.contains(tap),
            }
        }

        /// Remove `part` out of band. A removed pin releases the link it held,
        /// so the attachment goes with it; a removed attachment leaves its pin
        /// behind (a defunct link); a removed TAP takes its attachments.
        fn remove_part(&mut self, part: AttachmentPart, tap: &str, ifindex: u32) {
            match part {
                AttachmentPart::Tap => {
                    self.taps.remove(tap);
                }
                AttachmentPart::IngressAttachment => {
                    if let Some(entry) = self.taps.get_mut(tap) {
                        entry.ingress.clear();
                    }
                }
                AttachmentPart::EgressAttachment => {
                    if let Some(entry) = self.taps.get_mut(tap) {
                        entry.egress.clear();
                    }
                }
                AttachmentPart::IngressPin => {
                    self.ingress_pins.remove(tap);
                    if let Some(entry) = self.taps.get_mut(tap) {
                        entry.ingress.clear();
                    }
                }
                AttachmentPart::EgressPin => {
                    self.egress_pins.remove(tap);
                    if let Some(entry) = self.taps.get_mut(tap) {
                        entry.egress.clear();
                    }
                }
                AttachmentPart::EndpointEntry => {
                    self.endpoints.remove(&ifindex);
                }
                AttachmentPart::GuardMember => {
                    self.guard_members.remove(tap);
                }
            }
        }

        fn guard_inventory(&self) -> BridgeGuardInventory {
            if !self.guard_table {
                return empty_guard_inventory();
            }
            let table = BridgeGuardTableFact {
                family: BridgeGuardObservedFamily::Bridge,
                name: "overdrive-mtls".to_owned(),
            };
            let mut rules = HostSharedGuestNetworkOwner::guard_spec().expected_rule_facts();
            if !self.guard_rules_intact {
                rules.pop();
            }
            BridgeGuardInventory {
                generation: 1,
                tables: vec![table.clone()],
                chains: vec![BridgeGuardChainOccurrence {
                    table: table.clone(),
                    name: "prerouting".to_owned(),
                    handle: Some(1),
                    definition: BridgeGuardChainDefinition::Base {
                        chain_type: BridgeGuardChainType::Filter,
                        hook: BridgeGuardChainHook::Prerouting,
                        priority: -300,
                        policy: Some(BridgeGuardChainPolicy::Accept),
                    },
                }],
                sets: vec![BridgeGuardSetFact {
                    table: table.clone(),
                    name: "managed_taps".to_owned(),
                    key_len: 16,
                    ifname_key: true,
                }],
                rules: rules
                    .into_iter()
                    .zip(2_u64..)
                    .map(|(fact, handle)| BridgeGuardRuleOccurrence {
                        table: table.clone(),
                        chain: "prerouting".to_owned(),
                        handle,
                        fact,
                        counter: None,
                    })
                    .collect(),
                members: self
                    .guard_members
                    .iter()
                    .map(|member| BridgeGuardMemberOccurrence {
                        table: table.clone(),
                        set: "managed_taps".to_owned(),
                        identity: BridgeGuardMemberIdentity::Ifname(member.clone()),
                    })
                    .collect(),
                other_children: Vec::new(),
            }
        }

        /// The D9 classification of the modelled guard against the owner's
        /// expected member set.
        fn classify_guard(&self, expected_members: &BTreeSet<String>) -> BridgeGuardObservation {
            let inventory = self.guard_inventory();
            if !self.guard_table {
                BridgeGuardObservation::Absent { inventory }
            } else if self.guard_rules_intact && self.guard_members == *expected_members {
                BridgeGuardObservation::Exact { inventory }
            } else {
                BridgeGuardObservation::Conflict { inventory }
            }
        }
    }

    /// One journaled leaf call: the leaf, the TAP it named (`None` for the
    /// node-wide guard read and debug-mask dump), and whether it changed the
    /// node.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct KernelCall {
        call: AllocationCall,
        tap: Option<String>,
        wrote: bool,
    }

    /// A leaf failure armed on a named call.
    #[derive(Debug, Clone)]
    struct LeafFault {
        call: AllocationCall,
        tap: Option<String>,
        /// Only once this call has been journaled since arming.
        after: Option<AllocationCall>,
        /// Only the nth matching call since arming (1-based).
        nth: Option<usize>,
        /// Keeps failing instead of failing once.
        standing: bool,
        armed_at: usize,
        spent: bool,
    }

    type NodeMutation = Box<dyn FnOnce(&mut FakeNode) + Send>;

    /// An out-of-band node change applied right after the nth journaled call
    /// of `after` since arming.
    struct NodeHook {
        after: AllocationCall,
        nth: usize,
        armed_at: usize,
        mutate: Option<NodeMutation>,
    }

    struct FakeAttachmentKernel {
        node: parking_lot::Mutex<FakeNode>,
        journal: parking_lot::Mutex<Vec<KernelCall>>,
        faults: parking_lot::Mutex<Vec<LeafFault>>,
        hooks: parking_lot::Mutex<Vec<NodeHook>>,
        guard_expectations: parking_lot::Mutex<Vec<BTreeSet<String>>>,
        /// Async leaves that never complete once reached (journaled first).
        hangs: parking_lot::Mutex<BTreeSet<AllocationCall>>,
    }

    impl FakeAttachmentKernel {
        fn healthy() -> Arc<Self> {
            Arc::new(Self {
                node: parking_lot::Mutex::new(FakeNode::healthy()),
                journal: parking_lot::Mutex::new(Vec::new()),
                faults: parking_lot::Mutex::new(Vec::new()),
                hooks: parking_lot::Mutex::new(Vec::new()),
                guard_expectations: parking_lot::Mutex::new(Vec::new()),
                hangs: parking_lot::Mutex::new(BTreeSet::new()),
            })
        }

        /// Every later `call` is journaled (as writing nothing) and then never
        /// completes: a leaf whose kernel request never returns.
        fn hang_always(&self, call: AllocationCall) {
            self.hangs.lock().insert(call);
        }

        /// Journal `call` and park forever when it is armed to hang; return
        /// when it is not.
        async fn hang_if_armed(&self, call: AllocationCall, tap: &str) {
            let hangs = self.hangs.lock().contains(&call);
            if hangs {
                self.journal.lock().push(KernelCall {
                    call,
                    tap: Some(tap.to_owned()),
                    wrote: false,
                });
                std::future::pending::<()>().await;
            }
        }

        fn node(&self) -> FakeNode {
            self.node.lock().clone()
        }

        fn with_node(&self, change: impl FnOnce(&mut FakeNode)) {
            change(&mut self.node.lock());
        }

        fn mark(&self) -> usize {
            self.journal.lock().len()
        }

        fn calls_since(&self, mark: usize) -> Vec<KernelCall> {
            self.journal.lock()[mark..].to_vec()
        }

        fn trace_since(&self, mark: usize) -> Vec<(AllocationCall, Option<String>)> {
            self.calls_since(mark).into_iter().map(|call| (call.call, call.tap)).collect()
        }

        fn mutations_since(&self, mark: usize) -> Vec<KernelCall> {
            self.calls_since(mark)
                .into_iter()
                .filter(|call| MUTATIONS.contains(&call.call))
                .collect()
        }

        fn guard_expectations(&self) -> Vec<BTreeSet<String>> {
            self.guard_expectations.lock().clone()
        }

        fn arm(
            &self,
            call: AllocationCall,
            tap: Option<&str>,
            after: Option<AllocationCall>,
            nth: Option<usize>,
            standing: bool,
        ) {
            let armed_at = self.mark();
            self.faults.lock().push(LeafFault {
                call,
                tap: tap.map(str::to_owned),
                after,
                nth,
                standing,
                armed_at,
                spent: false,
            });
        }

        fn fail_once(&self, call: AllocationCall) {
            self.arm(call, None, None, None, false);
        }

        fn fail_nth(&self, call: AllocationCall, nth: usize) {
            self.arm(call, None, None, Some(nth), false);
        }

        fn fail_always(&self, call: AllocationCall) {
            self.arm(call, None, None, None, true);
        }

        fn fail_once_for(&self, call: AllocationCall, tap: &str) {
            self.arm(call, Some(tap), None, None, false);
        }

        fn fail_once_after(&self, after: AllocationCall, call: AllocationCall) {
            self.arm(call, None, Some(after), None, false);
        }

        /// Fail the first `call` naming `tap` that follows an `after` call
        /// naming the same `tap` (for example the read-back that follows a
        /// TAP's own set-down).
        fn fail_once_for_after(&self, after: AllocationCall, call: AllocationCall, tap: &str) {
            self.arm(call, Some(tap), Some(after), None, false);
        }

        fn clear_faults(&self) {
            self.faults.lock().clear();
        }

        /// Change the node out of band right after the nth `after` call from
        /// now.
        fn after_call(
            &self,
            after: AllocationCall,
            nth: usize,
            mutate: impl FnOnce(&mut FakeNode) + Send + 'static,
        ) {
            let armed_at = self.mark();
            self.hooks.lock().push(NodeHook {
                after,
                nth,
                armed_at,
                mutate: Some(Box::new(mutate)),
            });
        }

        fn take_fault(&self, call: AllocationCall, tap: Option<&str>) -> bool {
            let journal = self.journal.lock().clone();
            let mut faults = self.faults.lock();
            for fault in faults.iter_mut() {
                if fault.spent
                    || fault.call != call
                    || fault.tap.as_deref().is_some_and(|wanted| Some(wanted) != tap)
                {
                    continue;
                }
                let since = &journal[fault.armed_at..];
                // A TAP-scoped fault waits for an `after` call on its own TAP.
                if fault.after.is_some_and(|after| {
                    !since.iter().any(|prior| {
                        prior.call == after
                            && fault
                                .tap
                                .as_deref()
                                .is_none_or(|wanted| prior.tap.as_deref() == Some(wanted))
                    })
                }) {
                    continue;
                }
                if let Some(nth) = fault.nth {
                    let occurrence = since
                        .iter()
                        .filter(|prior| {
                            prior.call == call
                                && fault
                                    .tap
                                    .as_deref()
                                    .is_none_or(|wanted| prior.tap.as_deref() == Some(wanted))
                        })
                        .count()
                        + 1;
                    if occurrence != nth {
                        continue;
                    }
                }
                if !fault.standing {
                    fault.spent = true;
                }
                return true;
            }
            false
        }

        fn run_hooks(&self) {
            let journal = self.journal.lock().clone();
            let due = self
                .hooks
                .lock()
                .iter_mut()
                .filter_map(|hook| {
                    let seen = journal[hook.armed_at..]
                        .iter()
                        .filter(|prior| prior.call == hook.after)
                        .count();
                    if seen == hook.nth { hook.mutate.take() } else { None }
                })
                .collect::<Vec<_>>();
            for mutate in due {
                mutate(&mut self.node.lock());
            }
        }

        /// Run one leaf: an armed failure returns `failure()` with no effect;
        /// otherwise `effect` runs against the node. The call is journaled
        /// with whether it changed the node, then due hooks run.
        fn leaf<T, E>(
            &self,
            call: AllocationCall,
            tap: Option<&str>,
            failure: impl FnOnce() -> E,
            effect: impl FnOnce(&mut FakeNode) -> std::result::Result<T, E>,
        ) -> std::result::Result<T, E> {
            let failing = self.take_fault(call, tap);
            let (result, wrote) = {
                let mut node = self.node.lock();
                let before = node.clone();
                let result = if failing { Err(failure()) } else { effect(&mut node) };
                let wrote = *node != before;
                (result, wrote)
            };
            self.journal.lock().push(KernelCall { call, tap: tap.map(str::to_owned), wrote });
            self.run_hooks();
            result
        }
    }

    #[async_trait::async_trait]
    impl GuestNetworkAllocationIo for FakeAttachmentKernel {
        async fn create_tap(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::CreateTap, Some(&tap), netlink_failure, |node| {
                if !node.taps.contains_key(&tap) {
                    let ifindex = node.next_ifindex;
                    node.next_ifindex += 1;
                    node.taps.insert(
                        tap.clone(),
                        FakeTap {
                            ifindex,
                            kind: GuestLinkKind::Tap,
                            persistent: true,
                            owner_uid: ROOT_UID,
                            up: false,
                            master: None,
                            mac: Some(host_mac(ifindex)),
                            debug_mask: 0,
                            ingress: Vec::new(),
                            egress: Vec::new(),
                            admin_stuck: false,
                        },
                    );
                }
                Ok(())
            })
        }
        async fn attach_tap_to_bridge(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            let tap = plan.assignment().tap.clone();
            let bridge = plan.bridge().to_owned();
            self.leaf(AllocationCall::AttachTap, Some(&tap), netlink_failure, |node| {
                let master = node
                    .bridge
                    .as_ref()
                    .map(|bridge| bridge.ifindex)
                    .ok_or_else(|| NetlinkError::link_absent(bridge))?;
                node.taps
                    .get_mut(&tap)
                    .ok_or_else(|| NetlinkError::link_absent(tap.clone()))?
                    .master = Some(master);
                Ok(())
            })
        }
        async fn set_tap_up(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::SetTapUp, Some(&tap), netlink_failure, |node| {
                let entry = node
                    .taps
                    .get_mut(&tap)
                    .ok_or_else(|| NetlinkError::link_absent(tap.clone()))?;
                if !entry.admin_stuck {
                    entry.up = true;
                }
                Ok(())
            })
        }
        async fn set_tap_down(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.hang_if_armed(AllocationCall::SetTapDown, &tap).await;
            self.leaf(AllocationCall::SetTapDown, Some(&tap), netlink_failure, |node| {
                let entry = node
                    .taps
                    .get_mut(&tap)
                    .ok_or_else(|| NetlinkError::link_absent(tap.clone()))?;
                if !entry.admin_stuck {
                    entry.up = false;
                }
                Ok(())
            })
        }
        async fn delete_tap(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DeleteTap, Some(&tap), netlink_failure, |node| {
                node.taps
                    .remove(&tap)
                    .map(drop)
                    .ok_or_else(|| NetlinkError::link_absent(tap.clone()))
            })
        }
        async fn observe_tap(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationTapObservation, NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::ObserveTap, Some(&tap), netlink_failure, |node| {
                Ok(match node.taps.get(&tap) {
                    None => GuestNetworkAllocationTapObservation::Absent { name: tap.clone() },
                    Some(entry) if entry.kind == GuestLinkKind::Tap && entry.persistent => {
                        GuestNetworkAllocationTapObservation::Persistent {
                            name: tap.clone(),
                            ifindex: entry.ifindex,
                            up: entry.up,
                            owner_uid: entry.owner_uid,
                            master_ifindex: entry.master,
                            mac: entry.mac,
                        }
                    }
                    Some(entry) => GuestNetworkAllocationTapObservation::Incompatible {
                        name: tap.clone(),
                        ifindex: entry.ifindex,
                        kind: entry.kind,
                        persistent: matches!(entry.kind, GuestLinkKind::Tap | GuestLinkKind::Tun)
                            .then_some(entry.persistent),
                        up: entry.up,
                        owner_uid: entry.owner_uid,
                        master_ifindex: entry.master,
                    },
                })
            })
        }
        async fn observe_bridge(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationBridgeObservation, NetlinkError> {
            let tap = plan.assignment().tap.clone();
            let name = plan.bridge().to_owned();
            self.leaf(AllocationCall::ObserveBridge, Some(&tap), netlink_failure, |node| {
                Ok(node.bridge.as_ref().map_or_else(
                    || GuestNetworkAllocationBridgeObservation::Absent { name: name.clone() },
                    |bridge| GuestNetworkAllocationBridgeObservation::Present {
                        name: name.clone(),
                        ifindex: bridge.ifindex,
                        kind: bridge.kind,
                    },
                ))
            })
        }
        async fn observe_shared_bridge(
            &self,
        ) -> std::result::Result<
            (Option<overdrive_netlink::ObservedLinkIdentity>, Option<bool>),
            NetlinkError,
        > {
            self.leaf(AllocationCall::ObserveBridge, None, netlink_failure, |node| {
                Ok(node.bridge.as_ref().map_or((None, None), |bridge| {
                    let kind = match bridge.kind {
                        GuestLinkKind::Bridge => overdrive_netlink::ObservedLinkKind::Bridge,
                        GuestLinkKind::Tap => overdrive_netlink::ObservedLinkKind::Tap,
                        GuestLinkKind::Tun => overdrive_netlink::ObservedLinkKind::Tun,
                        GuestLinkKind::Other => overdrive_netlink::ObservedLinkKind::Other,
                    };
                    (
                        Some(overdrive_netlink::ObservedLinkIdentity {
                            name: "ovd-gbr0".to_owned(),
                            ifindex: bridge.ifindex,
                            kind,
                            up: bridge.up,
                            master_ifindex: None,
                            mac: Some(bridge.mac),
                        }),
                        Some(bridge.gateway),
                    )
                }))
            })
        }
        fn observe_shared_tcx(
            &self,
            read: GuestNetworkNodeTcxAuditRead,
            managed_ifindices: &BTreeSet<u32>,
        ) -> std::result::Result<u32, GuestTcxError> {
            use overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily;

            let call = match read {
                GuestNetworkNodeTcxAuditRead::Programs => AllocationCall::ObserveTcxPrograms,
                GuestNetworkNodeTcxAuditRead::EndpointMap => AllocationCall::ObserveEndpointMap,
                GuestNetworkNodeTcxAuditRead::CounterMap => AllocationCall::ObserveCounterMap,
                GuestNetworkNodeTcxAuditRead::EndpointMapPin => {
                    AllocationCall::ObserveEndpointMapPin
                }
                GuestNetworkNodeTcxAuditRead::CounterMapPin => AllocationCall::ObserveCounterMapPin,
                GuestNetworkNodeTcxAuditRead::EndpointEntries => {
                    AllocationCall::ObserveEndpointEntries
                }
            };
            self.leaf(call, None, tcx_failure, |node| {
                Ok(match read {
                    GuestNetworkNodeTcxAuditRead::Programs => {
                        2 * u32::from(node.tcx_program_loaded)
                    }
                    GuestNetworkNodeTcxAuditRead::EndpointMap => {
                        u32::from(node.endpoint_map_intact)
                    }
                    GuestNetworkNodeTcxAuditRead::CounterMap => u32::from(node.counter_map_intact),
                    GuestNetworkNodeTcxAuditRead::EndpointMapPin => {
                        u32::from(node.endpoint_map_pinned)
                    }
                    GuestNetworkNodeTcxAuditRead::CounterMapPin => {
                        u32::from(node.counter_map_pinned)
                    }
                    GuestNetworkNodeTcxAuditRead::EndpointEntries => {
                        if node.endpoints.keys().any(|ifindex| !managed_ifindices.contains(ifindex))
                        {
                            return Err(GuestTcxError::InventoryAmbiguous {
                                family: GuestTcxInventoryFamily::EndpointEntry,
                            });
                        }
                        u32::try_from(node.endpoints.len()).map_err(|_| tcx_failure())?
                    }
                })
            })
        }
        fn insert_guard_member(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::InsertGuard, Some(&tap), guard_failure, |node| {
                if !node.guard_table {
                    return Err(BridgeGuardError::Netlink(NetlinkError::nft(
                        "bridge-insert-member",
                        std::io::Error::from_raw_os_error(libc::ENOENT),
                    )));
                }
                node.guard_members.insert(tap.clone());
                Ok(BridgeGuardMutationOutcome::Converged { observed: node.guard_inventory() })
            })
        }
        fn delete_guard_member(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DeleteGuard, Some(&tap), guard_failure, |node| {
                node.guard_members.remove(&tap);
                Ok(BridgeGuardMutationOutcome::Converged { observed: node.guard_inventory() })
            })
        }
        fn observe_guard(
            &self,
            expected_members: &BTreeSet<String>,
        ) -> std::result::Result<BridgeGuardObservation, BridgeGuardError> {
            self.guard_expectations.lock().push(expected_members.clone());
            self.leaf(AllocationCall::ObserveGuard, None, guard_failure, |node| {
                Ok(node.classify_guard(expected_members))
            })
        }
        fn insert_endpoint(
            &self,
            plan: &GuestNetworkPlan,
            ifindex: u32,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            let endpoint = endpoint_for(plan);
            self.leaf(AllocationCall::InsertEndpoint, Some(&tap), tcx_failure, |node| {
                node.endpoints.insert(ifindex, endpoint);
                Ok(())
            })
        }
        fn read_endpoint(
            &self,
            plan: &GuestNetworkPlan,
            ifindex: u32,
        ) -> std::result::Result<Option<GuestTcxEndpoint>, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::ReadEndpoint, Some(&tap), tcx_failure, |node| {
                Ok(node.endpoints.get(&ifindex).copied())
            })
        }
        fn remove_endpoint(
            &self,
            plan: &GuestNetworkPlan,
            ifindex: u32,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::RemoveEndpoint, Some(&tap), tcx_failure, |node| {
                node.endpoints.remove(&ifindex);
                Ok(())
            })
        }
        fn attach_first_ingress(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::AttachFirstIngress, Some(&tap), tcx_failure, |node| {
                let entry = node.taps.get_mut(&tap).ok_or_else(absent_interface)?;
                if !entry.ingress.contains(&INGRESS_PROGRAM) {
                    entry.ingress.push(INGRESS_PROGRAM);
                    entry.ingress.sort_unstable();
                }
                node.pending_ingress.insert(tap.clone());
                Ok(())
            })
        }
        fn pin_link(&self, plan: &GuestNetworkPlan) -> std::result::Result<u32, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::PinLink, Some(&tap), tcx_failure, |node| {
                if node.pending_ingress.remove(&tap) {
                    node.ingress_pins.insert(tap.clone());
                    Ok(INGRESS_PROGRAM)
                } else {
                    Err(GuestTcxError::ObjectMissing { object: GuestTcxObject::Classifier })
                }
            })
        }
        fn query_attachment(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::QueryAttachment, Some(&tap), tcx_failure, |node| {
                node.taps
                    .get(&tap)
                    .map(|entry| GuestTcxAttachment {
                        revision: 1,
                        program_ids: entry.ingress.clone(),
                    })
                    .ok_or_else(absent_interface)
            })
        }
        fn link_pin_present(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<bool, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::LinkPinPresent, Some(&tap), tcx_failure, |node| {
                Ok(node.ingress_pins.contains(&tap))
            })
        }
        fn detach_pending_link(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DetachPendingLink, Some(&tap), tcx_failure, |node| {
                if node.pending_ingress.remove(&tap)
                    && let Some(entry) = node.taps.get_mut(&tap)
                {
                    entry.ingress.retain(|program| *program != INGRESS_PROGRAM);
                }
                Ok(())
            })
        }
        fn detach_pinned_link(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DetachPinnedLink, Some(&tap), tcx_failure, |node| {
                if node.ingress_pins.remove(&tap)
                    && let Some(entry) = node.taps.get_mut(&tap)
                {
                    entry.ingress.retain(|program| *program != INGRESS_PROGRAM);
                }
                Ok(())
            })
        }
        fn attach_first_egress(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::AttachFirstEgress, Some(&tap), tcx_failure, |node| {
                let entry = node.taps.get_mut(&tap).ok_or_else(absent_interface)?;
                if !entry.egress.contains(&EGRESS_PROGRAM) {
                    entry.egress.push(EGRESS_PROGRAM);
                    entry.egress.sort_unstable();
                }
                node.pending_egress.insert(tap.clone());
                Ok(())
            })
        }
        fn pin_egress_link(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<u32, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::PinEgressLink, Some(&tap), tcx_failure, |node| {
                if node.pending_egress.remove(&tap) {
                    node.egress_pins.insert(tap.clone());
                    Ok(EGRESS_PROGRAM)
                } else {
                    Err(GuestTcxError::ObjectMissing { object: GuestTcxObject::EgressClassifier })
                }
            })
        }
        fn query_egress_attachment(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::QueryEgressAttachment, Some(&tap), tcx_failure, |node| {
                node.taps
                    .get(&tap)
                    .map(|entry| GuestTcxAttachment {
                        revision: 1,
                        program_ids: entry.egress.clone(),
                    })
                    .ok_or_else(absent_interface)
            })
        }
        fn egress_link_pin_present(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<bool, GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::EgressLinkPinPresent, Some(&tap), tcx_failure, |node| {
                Ok(node.egress_pins.contains(&tap))
            })
        }
        fn detach_pending_egress_link(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DetachPendingEgressLink, Some(&tap), tcx_failure, |node| {
                if node.pending_egress.remove(&tap)
                    && let Some(entry) = node.taps.get_mut(&tap)
                {
                    entry.egress.retain(|program| *program != EGRESS_PROGRAM);
                }
                Ok(())
            })
        }
        fn detach_pinned_egress_link(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<(), GuestTcxError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::DetachPinnedEgressLink, Some(&tap), tcx_failure, |node| {
                if node.egress_pins.remove(&tap)
                    && let Some(entry) = node.taps.get_mut(&tap)
                {
                    entry.egress.retain(|program| *program != EGRESS_PROGRAM);
                }
                Ok(())
            })
        }
        async fn observe_tap_debug_msg_mask(
            &self,
            plan: &GuestNetworkPlan,
        ) -> std::result::Result<Option<u32>, NetlinkError> {
            let tap = plan.assignment().tap.clone();
            self.leaf(AllocationCall::ObserveTapDebugMsgMask, Some(&tap), netlink_failure, |node| {
                Ok(node.taps.get(&tap).map(|entry| entry.debug_mask))
            })
        }
        async fn observe_debug_msg_masks(
            &self,
        ) -> std::result::Result<BTreeMap<u32, u32>, NetlinkError> {
            self.leaf(AllocationCall::ObserveDebugMsgMasks, None, netlink_failure, |node| {
                Ok(node
                    .taps
                    .values()
                    .filter(|entry| !node.mask_dump_omits.contains(&entry.ifindex))
                    .map(|entry| (entry.ifindex, entry.debug_mask))
                    .collect())
            })
        }
    }

    // ---- plans, fixtures, and oracles --------------------------------------

    /// The queued-observation plan: TAP `ovd-tp-0002` on the shared bridge.
    fn plan(name: &str, address: Ipv4Addr) -> GuestNetworkPlan {
        GuestNetworkPlan {
            alloc: AllocationId::new(name).expect("allocation id"),
            bridge: "ovd-gbr0".to_owned(),
            node_prefix: "100.95.0.0/16".parse().expect("node prefix"),
            assignment: GuestNetworkAssignment {
                address,
                tap: "ovd-tp-0002".to_owned(),
                mac: [0x02, 0x00, 100, 95, 0, 2],
                gateway: Ipv4Addr::new(100, 95, 0, 1),
                prefix: 16,
                dns: Ipv4Addr::new(100, 95, 0, 1),
            },
        }
    }

    /// A plan over a scratch TAP name outside `ovd-tp-`, so an owner path
    /// that still reaches the host never names a production TAP.
    fn scratch_plan(alloc: &str, tap: &str, host: u8) -> GuestNetworkPlan {
        GuestNetworkPlan {
            alloc: AllocationId::new(alloc).expect("allocation id"),
            bridge: "ovd-gbr0".to_owned(),
            node_prefix: "100.95.0.0/16".parse().expect("node prefix"),
            assignment: GuestNetworkAssignment {
                address: Ipv4Addr::new(100, 95, 0, host),
                tap: tap.to_owned(),
                mac: [0x02, 0x00, 100, 95, 0, host],
                gateway: Ipv4Addr::new(100, 95, 0, 1),
                prefix: 16,
                dns: Ipv4Addr::new(100, 95, 0, 1),
            },
        }
    }

    fn owner_over(kernel: &Arc<FakeAttachmentKernel>) -> HostSharedGuestNetworkOwner {
        HostSharedGuestNetworkOwner::with_allocation_io(kernel.clone())
    }

    async fn provisioned(owner: &HostSharedGuestNetworkOwner, plan: &GuestNetworkPlan) {
        owner.provision(plan).await.expect("fixture provisions the attachment down");
    }

    async fn activated(owner: &HostSharedGuestNetworkOwner, plan: &GuestNetworkPlan) {
        provisioned(owner, plan).await;
        assert_eq!(
            owner.activate(plan).await.expect("fixture activates the attachment"),
            TapActivation::Raised
        );
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "journal entries name an optional TAP; every comparison builds that same shape"
    )]
    fn tap_of(plan: &GuestNetworkPlan) -> Option<String> {
        Some(plan.assignment().tap.clone())
    }

    fn tap_fact(
        name: &str,
        ifindex: Option<u32>,
        link_kind: GuestLinkKind,
        persistent: bool,
        up: bool,
        owner_uid: Option<u32>,
    ) -> GuestNetworkFact {
        GuestNetworkFact::Tap {
            name: name.to_owned(),
            ifindex,
            link_kind,
            persistent,
            up,
            owner_uid,
        }
    }

    /// The exact persistent, root-owned TAP identity the owner expects.
    fn expected_tap(plan: &GuestNetworkPlan, ifindex: u32, up: bool) -> GuestNetworkFact {
        tap_fact(&plan.assignment().tap, Some(ifindex), GuestLinkKind::Tap, true, up, ROOT_UID)
    }

    fn bridge_fact(name: &str, ifindex: Option<u32>, link_kind: GuestLinkKind) -> GuestNetworkFact {
        GuestNetworkFact::BridgeLinkIdentity { name: name.to_owned(), ifindex, link_kind }
    }

    fn master_fact(ifindex: u32, master_ifindex: Option<u32>) -> GuestNetworkFact {
        GuestNetworkFact::LinkMaster { ifindex, master_ifindex }
    }

    /// The host-side MAC invariant's fact (D-295-R21): expected
    /// `Unreserved`, observed `Reserved(<mac>)` or `Missing`.
    const fn host_mac_fact(ifindex: u32, address: TapHostAddress) -> GuestNetworkFact {
        GuestNetworkFact::TapHostMac { ifindex, address }
    }

    fn attachment_fact(
        ifindex: u32,
        program_id: Option<u32>,
        point: TcxAttachPoint,
    ) -> GuestNetworkFact {
        GuestNetworkFact::TcxAttachment { ifindex, program_id, attach_point: Some(point) }
    }

    fn endpoint_fact(ifindex: u32, endpoint: GuestTcxEndpoint) -> GuestNetworkFact {
        GuestNetworkFact::EndpointMapEntry {
            ifindex,
            value: Some(GuestEndpointFact {
                source_ip: endpoint.source_ipv4,
                source_mac: endpoint.source_mac,
                bridge_mac: endpoint.bridge_mac,
            }),
        }
    }

    fn assert_mismatch(
        error: &GuestNetworkError,
        operation: GuestNetworkOperation,
        expected: &GuestNetworkFact,
        observed: Option<&GuestNetworkFact>,
    ) {
        assert!(
            matches!(
                error,
                GuestNetworkError::PostconditionMismatch {
                    operation: actual_operation,
                    expected: actual_expected,
                    observed: actual_observed,
                } if *actual_operation == operation
                    && actual_expected == expected
                    && actual_observed.as_ref() == observed
            ),
            "expected {operation:?} mismatch {expected:?} vs {observed:?}, got {error:?}"
        );
    }

    fn is_tcx_failure(error: &GuestNetworkError, operation: GuestNetworkOperation) -> bool {
        matches!(
            error,
            GuestNetworkError::Tcx { operation: actual, source: GuestTcxError::Io { source } }
                if *actual == operation && source.raw_os_error() == Some(libc::EIO)
        )
    }

    fn is_netlink_failure(error: &GuestNetworkError, operation: GuestNetworkOperation) -> bool {
        matches!(
            error,
            GuestNetworkError::Netlink { operation: actual, source: NetlinkError::Connect { source } }
                if *actual == operation && source.raw_os_error() == Some(libc::EBUSY)
        )
    }

    /// The owner does not hold the allocation: `activate` refuses it as a
    /// missing allocation record before any leaf call.
    async fn assert_unpublished(
        owner: &HostSharedGuestNetworkOwner,
        leaf_calls: impl Fn() -> usize,
        plan: &GuestNetworkPlan,
    ) {
        let before = leaf_calls();
        let refusal = owner.activate(plan).await;
        assert!(
            matches!(refusal, Err(GuestNetworkError::PostconditionMismatch { .. })),
            "an unpublished allocation is refused, got {refusal:?}"
        );
        assert_eq!(leaf_calls(), before, "the refusal reads and writes nothing");
    }

    /// The source-less refusal for a missing, non-matching, or condemned
    /// allocation record (D-295-R5 activation table).
    fn assert_refused_as_missing_record(error: &GuestNetworkError, plan: &GuestNetworkPlan) {
        let record =
            tap_fact(&plan.assignment().tap, None, GuestLinkKind::Tap, true, false, ROOT_UID);
        assert!(
            matches!(
                error,
                GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    ..
                } if *expected == record
            ),
            "expected the missing-record refusal, got {error:?}"
        );
    }

    /// The complete attachment `provision` leaves down in the node.
    fn provisioned_parts(plan: &GuestNetworkPlan, ifindex: u32) -> AttachmentParts {
        AttachmentParts {
            tap: Some(FakeTap {
                ifindex,
                kind: GuestLinkKind::Tap,
                persistent: true,
                owner_uid: ROOT_UID,
                up: false,
                master: Some(BRIDGE_IFINDEX),
                mac: Some(host_mac(ifindex)),
                debug_mask: 0,
                ingress: vec![INGRESS_PROGRAM],
                egress: vec![EGRESS_PROGRAM],
                admin_stuck: false,
            }),
            endpoint: Some(endpoint_for(plan)),
            ingress_pin: true,
            egress_pin: true,
            pending_ingress: false,
            pending_egress: false,
            guard_member: true,
        }
    }

    // ---- S-ND295-11 --------------------------------------------------------

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// `provision` performs exactly D12A's order — the egress attach, pin,
    /// query, and pin read-back as step 6; the bridge refresh, the TAP-down
    /// identity read-back (owner uid 0, host MAC) and the debug-mask read as
    /// step 7 — never raises the TAP, and leaves the complete attachment down.
    /// Only then does the owner hold the allocation: `activate` accepts it.
    #[tokio::test]
    async fn provision_reads_every_attachment_fact_before_reporting_success() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let plan = scratch_plan("nd295-s11", "t295-s11", 2);
        let tap = tap_of(&plan);

        owner.provision(&plan).await.expect("every leaf effect and read-back is exact");

        assert_eq!(
            kernel.trace_since(0),
            vec![
                (AllocationCall::CreateTap, tap.clone()),
                (AllocationCall::ObserveTap, tap.clone()),
                (AllocationCall::AttachTap, tap.clone()),
                (AllocationCall::ObserveBridge, tap.clone()),
                (AllocationCall::ObserveTap, tap.clone()),
                (AllocationCall::InsertGuard, tap.clone()),
                (AllocationCall::ObserveGuard, None),
                (AllocationCall::InsertEndpoint, tap.clone()),
                (AllocationCall::ReadEndpoint, tap.clone()),
                (AllocationCall::AttachFirstIngress, tap.clone()),
                (AllocationCall::PinLink, tap.clone()),
                (AllocationCall::QueryAttachment, tap.clone()),
                (AllocationCall::LinkPinPresent, tap.clone()),
                (AllocationCall::AttachFirstEgress, tap.clone()),
                (AllocationCall::PinEgressLink, tap.clone()),
                (AllocationCall::QueryEgressAttachment, tap.clone()),
                (AllocationCall::EgressLinkPinPresent, tap.clone()),
                (AllocationCall::ObserveBridge, tap.clone()),
                (AllocationCall::ObserveTap, tap.clone()),
                (AllocationCall::ObserveTapDebugMsgMask, tap),
            ],
            "D12A order: egress is step 6; bridge refresh, TAP-down read-back, and mask read are step 7"
        );
        assert_eq!(
            kernel.node().parts(&plan.assignment().tap, FIRST_TAP_IFINDEX),
            provisioned_parts(&plan, FIRST_TAP_IFINDEX),
            "the complete attachment is present, root-owned, mask 0, and down"
        );
        assert_eq!(
            kernel.guard_expectations(),
            vec![BTreeSet::from([plan.assignment().tap.clone()])],
            "the guard read-back expects the complete managed-TAP set"
        );

        assert_eq!(
            owner.activate(&plan).await.expect("the owner now holds the allocation"),
            TapActivation::Raised
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Every incompatible TAP identity at the first, second, and final
    /// checkpoints, every bridge absence or wrong kind, and every master
    /// mismatch refuses with the owner-built source-less mismatch, never
    /// raises the TAP, and publishes nothing. The expected TAP is persistent
    /// and owned by uid 0 (D-295-R4); a TAP still owned by the VMM uid is an
    /// incompatible owner.
    #[tokio::test]
    async fn every_incompatible_tap_or_bridge_identity_refuses_owner_publication() {
        let vmm_uid = Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID);
        let bridge_29 = GuestNetworkAllocationBridgeObservation::Present {
            name: "ovd-gbr0".to_owned(),
            ifindex: BRIDGE_IFINDEX,
            kind: GuestLinkKind::Bridge,
        };
        let valid_down = GuestNetworkAllocationTapObservation::Persistent {
            name: "ovd-tp-0002".to_owned(),
            ifindex: 295,
            up: false,
            owner_uid: ROOT_UID,
            master_ifindex: Some(BRIDGE_IFINDEX),
            mac: Some(host_mac(295)),
        };
        let first_tap_cases = [
            (
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
                tap_fact("ovd-tp-0002", None, GuestLinkKind::Tap, true, false, ROOT_UID),
                None,
            ),
            (
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tun,
                    persistent: Some(true),
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tun, true, false, ROOT_UID)),
            ),
            (
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Other,
                    persistent: None,
                    up: false,
                    owner_uid: None,
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Other, false, false, None)),
            ),
            (
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tap,
                    persistent: Some(false),
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    false,
                    false,
                    ROOT_UID,
                )),
            ),
            (
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: vmm_uid,
                    master_ifindex: None,
                    mac: Some(host_mac(295)),
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, vmm_uid)),
            ),
            (
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: true,
                    owner_uid: ROOT_UID,
                    master_ifindex: None,
                    mac: Some(host_mac(295)),
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, ROOT_UID)),
            ),
        ];
        for (index, (actual, expected, observed)) in first_tap_cases.into_iter().enumerate() {
            let io = ScriptedAllocationIo::with_observations([actual], [bridge_29.clone()]);
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
            let plan = plan(&format!("nd295-s11-first-tap-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("first TAP checkpoint mismatch refuses publication");
            assert_mismatch(
                &error,
                GuestNetworkOperation::TapObserve,
                &expected,
                observed.as_ref(),
            );
            assert!(!io.calls().contains(&AllocationCall::SetTapUp));
            assert_unpublished(&owner, || io.calls().len(), &plan).await;
        }

        for (index, (actual, expected, observed)) in [
            (
                GuestNetworkAllocationBridgeObservation::Absent { name: "ovd-gbr0".to_owned() },
                bridge_fact("ovd-gbr0", None, GuestLinkKind::Bridge),
                None,
            ),
            (
                GuestNetworkAllocationBridgeObservation::Present {
                    name: "ovd-gbr0".to_owned(),
                    ifindex: BRIDGE_IFINDEX,
                    kind: GuestLinkKind::Other,
                },
                bridge_fact("ovd-gbr0", Some(BRIDGE_IFINDEX), GuestLinkKind::Bridge),
                Some(bridge_fact("ovd-gbr0", Some(BRIDGE_IFINDEX), GuestLinkKind::Other)),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let io = ScriptedAllocationIo::with_observations([valid_down.clone()], [actual]);
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
            let plan =
                plan(&format!("nd295-s11-first-bridge-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("first bridge checkpoint mismatch refuses publication");
            assert_mismatch(
                &error,
                GuestNetworkOperation::BridgeObserve,
                &expected,
                observed.as_ref(),
            );
            assert!(!io.calls().contains(&AllocationCall::SetTapUp));
            assert_unpublished(&owner, || io.calls().len(), &plan).await;
        }

        let first_master_io = ScriptedAllocationIo::with_observations(
            [
                valid_down.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(30),
                    mac: Some(host_mac(295)),
                },
            ],
            [bridge_29.clone()],
        );
        let first_master_owner =
            HostSharedGuestNetworkOwner::with_allocation_io(first_master_io.clone());
        let first_master_plan = plan("nd295-s11-first-master", Ipv4Addr::new(100, 95, 0, 2));
        let first_master_error = first_master_owner
            .provision(&first_master_plan)
            .await
            .expect_err("down-TAP master mismatch refuses publication");
        assert_mismatch(
            &first_master_error,
            GuestNetworkOperation::TapObserve,
            &master_fact(295, Some(BRIDGE_IFINDEX)),
            Some(&master_fact(295, Some(30))),
        );
        assert_unpublished(
            &first_master_owner,
            || first_master_io.calls().len(),
            &first_master_plan,
        )
        .await;

        let final_cases = [
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                None,
            ),
            (
                GuestNetworkAllocationBridgeObservation::Absent { name: "ovd-gbr0".to_owned() },
                valid_down.clone(),
                GuestNetworkOperation::BridgeObserve,
                bridge_fact("ovd-gbr0", None, GuestLinkKind::Bridge),
                None,
            ),
            (
                GuestNetworkAllocationBridgeObservation::Present {
                    name: "ovd-gbr0".to_owned(),
                    ifindex: BRIDGE_IFINDEX,
                    kind: GuestLinkKind::Other,
                },
                valid_down.clone(),
                GuestNetworkOperation::BridgeObserve,
                bridge_fact("ovd-gbr0", Some(BRIDGE_IFINDEX), GuestLinkKind::Bridge),
                Some(bridge_fact("ovd-gbr0", Some(BRIDGE_IFINDEX), GuestLinkKind::Other)),
            ),
            (
                GuestNetworkAllocationBridgeObservation::Present {
                    name: "ovd-gbr0".to_owned(),
                    ifindex: 30,
                    kind: GuestLinkKind::Bridge,
                },
                valid_down.clone(),
                GuestNetworkOperation::TapObserve,
                master_fact(295, Some(30)),
                Some(master_fact(295, Some(BRIDGE_IFINDEX))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(30),
                    mac: Some(host_mac(295)),
                },
                GuestNetworkOperation::TapObserve,
                master_fact(295, Some(BRIDGE_IFINDEX)),
                Some(master_fact(295, Some(30))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tun,
                    persistent: Some(true),
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tun, true, false, ROOT_UID)),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Other,
                    persistent: None,
                    up: false,
                    owner_uid: None,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Other, false, false, None)),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 296,
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                    mac: Some(host_mac(296)),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(296), GuestLinkKind::Tap, true, false, ROOT_UID)),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: vmm_uid,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                    mac: Some(host_mac(295)),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, vmm_uid)),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tap,
                    persistent: Some(false),
                    up: false,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    false,
                    false,
                    ROOT_UID,
                )),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: true,
                    owner_uid: ROOT_UID,
                    master_ifindex: Some(BRIDGE_IFINDEX),
                    mac: Some(host_mac(295)),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, ROOT_UID),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, ROOT_UID)),
            ),
        ];
        for (index, (final_bridge, final_tap, operation, expected, observed)) in
            final_cases.into_iter().enumerate()
        {
            let mut tap_observations = vec![valid_down.clone(), valid_down.clone()];
            if operation == GuestNetworkOperation::TapObserve {
                tap_observations.push(final_tap.clone());
            }
            let io = ScriptedAllocationIo::with_observations(
                tap_observations,
                [bridge_29.clone(), final_bridge],
            );
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
            let plan = plan(&format!("nd295-s11-final-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("final checkpoint mismatch refuses publication");
            let calls = io.calls();
            assert!(
                calls.starts_with(&[
                    AllocationCall::CreateTap,
                    AllocationCall::ObserveTap,
                    AllocationCall::AttachTap,
                    AllocationCall::ObserveBridge,
                    AllocationCall::ObserveTap,
                    AllocationCall::InsertGuard,
                    AllocationCall::ObserveGuard,
                    AllocationCall::InsertEndpoint,
                    AllocationCall::ReadEndpoint,
                    AllocationCall::AttachFirstIngress,
                    AllocationCall::PinLink,
                    AllocationCall::QueryAttachment,
                    AllocationCall::LinkPinPresent,
                    AllocationCall::AttachFirstEgress,
                    AllocationCall::PinEgressLink,
                    AllocationCall::QueryEgressAttachment,
                    AllocationCall::EgressLinkPinPresent,
                    AllocationCall::ObserveBridge,
                ]),
                "final case {index} reaches the step-7 bridge refresh after the egress step: {calls:?}"
            );
            if operation == GuestNetworkOperation::TapObserve {
                assert_eq!(
                    calls.get(18),
                    Some(&AllocationCall::ObserveTap),
                    "final case {index} reaches the step-7 TAP-down read-back"
                );
            }
            assert!(!calls.contains(&AllocationCall::SetTapUp));
            assert_mismatch(&error, operation, &expected, observed.as_ref());
            assert_unpublished(&owner, || io.calls().len(), &plan).await;
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// F-28, extended to the egress link: a rollback that proves the TAP
    /// absent but fails its final guard read-back returns that cleanup error
    /// and keeps the rollback for retry; the retry, through `teardown`, never
    /// queries an ingress or egress TCX attachment on the removed interface,
    /// completes, and leaves nothing to retry.
    #[tokio::test]
    async fn rollback_retry_skips_attachment_query_after_tap_removal() {
        let bridge = GuestNetworkAllocationBridgeObservation::Present {
            name: "ovd-gbr0".to_owned(),
            ifindex: BRIDGE_IFINDEX,
            kind: GuestLinkKind::Bridge,
        };
        let valid_down = GuestNetworkAllocationTapObservation::Persistent {
            name: "ovd-tp-0002".to_owned(),
            ifindex: 295,
            up: false,
            owner_uid: ROOT_UID,
            master_ifindex: Some(BRIDGE_IFINDEX),
            mac: Some(host_mac(295)),
        };
        // The step-7 read-back finds the TAP gone; every later TAP read sees
        // it absent (the queue is then empty).
        let io = ScriptedAllocationIo::with_observations(
            [
                valid_down.clone(),
                valid_down,
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
            ],
            [bridge.clone(), bridge],
        );
        io.fail(AllocationCall::ObserveGuard, 2);

        let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
        let plan = plan("nd295-f28-post-delete", Ipv4Addr::new(100, 95, 0, 2));
        let error = owner
            .provision(&plan)
            .await
            .expect_err("the rollback's final guard read-back failure is returned");
        assert!(
            is_netlink_failure(&error, GuestNetworkOperation::CleanupComplement),
            "the first typed cleanup error is returned, got {error:?}"
        );
        let first_attempt = io.calls();
        assert!(
            first_attempt.contains(&AllocationCall::DetachPinnedEgressLink),
            "the rollback detaches the pinned egress link: {first_attempt:?}"
        );
        assert!(!first_attempt.contains(&AllocationCall::SetTapUp));

        let retry_start = io.calls().len();
        owner.teardown(&plan).await.expect("the same-owner retry converges after TAP absence");
        let retry_calls = io.calls()[retry_start..].to_vec();
        assert!(
            !retry_calls.contains(&AllocationCall::QueryAttachment)
                && !retry_calls.contains(&AllocationCall::QueryEgressAttachment),
            "no TCX query on an interface already observed absent: {retry_calls:?}"
        );
        assert!(retry_calls.contains(&AllocationCall::ObserveGuard));

        assert_unpublished(&owner, || io.calls().len(), &plan).await;
        let after_retry = io.calls().len();
        owner.teardown(&plan).await.expect("no rollback remains to retry");
        let later_calls = io.calls()[after_retry..].to_vec();
        assert!(
            !later_calls.contains(&AllocationCall::QueryAttachment)
                && !later_calls.contains(&AllocationCall::QueryEgressAttachment),
            "a completed retry leaves no rollback that reads the interface: {later_calls:?}"
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// F-28: an early failure with no TAP and no ifindex queries neither TCX
    /// attachment point, returns the owner-built absent-TAP mismatch (owner
    /// uid 0), and leaves nothing to retry.
    #[tokio::test]
    async fn early_provision_failure_without_tap_skips_attachment_query() {
        let io = ScriptedAllocationIo::with_observations(
            [GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() }],
            [],
        );
        io.fail(AllocationCall::QueryAttachment, 1);
        io.fail(AllocationCall::QueryEgressAttachment, 1);
        let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
        let plan = plan("nd295-f28-early", Ipv4Addr::new(100, 95, 0, 2));
        let error = owner
            .provision(&plan)
            .await
            .expect_err("an absent first TAP checkpoint refuses publication");
        assert_mismatch(
            &error,
            GuestNetworkOperation::TapObserve,
            &tap_fact("ovd-tp-0002", None, GuestLinkKind::Tap, true, false, ROOT_UID),
            None,
        );
        assert!(!io.calls().contains(&AllocationCall::QueryAttachment));
        assert!(!io.calls().contains(&AllocationCall::QueryEgressAttachment));

        let before_teardown = io.calls().len();
        owner.teardown(&plan).await.expect("no rollback remains to retry");
        assert!(
            !io.calls()[before_teardown..].contains(&AllocationCall::QueryAttachment),
            "the completed rollback leaves nothing that queries TCX"
        );
    }

    /// One egress-step or debug-mask provision fault.
    #[derive(Debug, Clone, Copy)]
    enum EgressOrMaskFault {
        AttachFails,
        PinFails,
        QueryFails,
        QueryShowsNoProgram,
        PinReadFails,
        PinAbsent,
        MaskNonZero,
        MaskReadFindsTapGone,
        MaskReadFails,
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Each egress attach, pin, query, and pin read-back failure returns its
    /// `TcxEgress*`-tagged error; a wrong egress attachment or a missing
    /// egress pin is the owner-built mismatch; a non-zero debug mask, a mask
    /// read that finds the TAP gone, and a sourced mask-read failure are the
    /// `TapObserve` outcomes of D-295-R22. Each refuses publication, never
    /// raises the TAP, and the rollback leaves no part of the attachment —
    /// the pending or pinned egress link included.
    #[tokio::test]
    async fn every_egress_and_debug_mask_provision_failure_refuses_publication() {
        for (index, fault) in [
            EgressOrMaskFault::AttachFails,
            EgressOrMaskFault::PinFails,
            EgressOrMaskFault::QueryFails,
            EgressOrMaskFault::QueryShowsNoProgram,
            EgressOrMaskFault::PinReadFails,
            EgressOrMaskFault::PinAbsent,
            EgressOrMaskFault::MaskNonZero,
            EgressOrMaskFault::MaskReadFindsTapGone,
            EgressOrMaskFault::MaskReadFails,
        ]
        .into_iter()
        .enumerate()
        {
            let kernel = FakeAttachmentKernel::healthy();
            let owner = owner_over(&kernel);
            let tap = format!("t295-em{index}");
            let plan = scratch_plan(&format!("nd295-s11-egress-mask-{index}"), &tap, 2);
            let ifindex = FIRST_TAP_IFINDEX;
            match fault {
                EgressOrMaskFault::AttachFails => {
                    kernel.fail_once(AllocationCall::AttachFirstEgress);
                }
                EgressOrMaskFault::PinFails => kernel.fail_once(AllocationCall::PinEgressLink),
                EgressOrMaskFault::QueryFails => {
                    kernel.fail_once(AllocationCall::QueryEgressAttachment);
                }
                EgressOrMaskFault::QueryShowsNoProgram => {
                    let tap = tap.clone();
                    kernel.after_call(AllocationCall::PinEgressLink, 1, move |node| {
                        node.tap_mut(&tap).egress.clear();
                    });
                }
                EgressOrMaskFault::PinReadFails => {
                    kernel.fail_once(AllocationCall::EgressLinkPinPresent);
                }
                EgressOrMaskFault::PinAbsent => {
                    let tap = tap.clone();
                    kernel.after_call(AllocationCall::QueryEgressAttachment, 1, move |node| {
                        node.remove_part(AttachmentPart::EgressPin, &tap, ifindex);
                    });
                }
                EgressOrMaskFault::MaskNonZero => {
                    let tap = tap.clone();
                    kernel.after_call(AllocationCall::CreateTap, 1, move |node| {
                        node.tap_mut(&tap).debug_mask = 0x10;
                    });
                }
                EgressOrMaskFault::MaskReadFindsTapGone => {
                    let tap = tap.clone();
                    kernel.after_call(AllocationCall::ObserveTap, 3, move |node| {
                        node.remove_part(AttachmentPart::Tap, &tap, ifindex);
                    });
                }
                EgressOrMaskFault::MaskReadFails => {
                    kernel.fail_once(AllocationCall::ObserveTapDebugMsgMask);
                }
            }

            let error = owner.provision(&plan).await.expect_err("the fault refuses publication");
            match fault {
                EgressOrMaskFault::AttachFails => assert!(
                    is_tcx_failure(&error, GuestNetworkOperation::TcxEgressAttach),
                    "{fault:?}: {error:?}"
                ),
                EgressOrMaskFault::PinFails | EgressOrMaskFault::PinReadFails => assert!(
                    is_tcx_failure(&error, GuestNetworkOperation::TcxEgressLinkPin),
                    "{fault:?}: {error:?}"
                ),
                EgressOrMaskFault::QueryFails => assert!(
                    is_tcx_failure(&error, GuestNetworkOperation::TcxEgressQuery),
                    "{fault:?}: {error:?}"
                ),
                EgressOrMaskFault::QueryShowsNoProgram => assert_mismatch(
                    &error,
                    GuestNetworkOperation::TcxEgressQuery,
                    &attachment_fact(ifindex, Some(EGRESS_PROGRAM), TcxAttachPoint::Egress),
                    Some(&attachment_fact(ifindex, None, TcxAttachPoint::Egress)),
                ),
                EgressOrMaskFault::PinAbsent => assert!(
                    matches!(
                        &error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxEgressLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin { .. },
                            ..
                        }
                    ),
                    "{fault:?}: {error:?}"
                ),
                EgressOrMaskFault::MaskNonZero => assert_mismatch(
                    &error,
                    GuestNetworkOperation::TapObserve,
                    &GuestNetworkFact::TapDebugMsgMask { ifindex, mask: 0 },
                    Some(&GuestNetworkFact::TapDebugMsgMask { ifindex, mask: 0x10 }),
                ),
                EgressOrMaskFault::MaskReadFindsTapGone => assert!(
                    matches!(
                        &error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TapObserve,
                            expected: GuestNetworkFact::Tap { name, .. },
                            observed: None,
                        } if *name == tap
                    ),
                    "{fault:?}: {error:?}"
                ),
                EgressOrMaskFault::MaskReadFails => assert!(
                    is_netlink_failure(&error, GuestNetworkOperation::TapObserve),
                    "{fault:?}: {error:?}"
                ),
            }
            assert!(
                !kernel.trace_since(0).iter().any(|(call, _)| *call == AllocationCall::SetTapUp),
                "{fault:?}: provision never raises the TAP"
            );
            assert!(
                kernel.node().parts(&tap, ifindex).is_empty(),
                "{fault:?}: the rollback leaves no attachment part: {:?}",
                kernel.node().parts(&tap, ifindex)
            );
            assert_unpublished(&owner, || kernel.mark(), &plan).await;
        }
    }

    // ---- S-ND295-12 --------------------------------------------------------

    /// Two attachments leased from one pool and activated on one owner over
    /// one fake kernel: the named one the body tears down and an unrelated
    /// one that must stay byte-equal.
    struct TwoAttachments {
        kernel: Arc<FakeAttachmentKernel>,
        owner: HostSharedGuestNetworkOwner,
        pool: GuestAddressPool,
        named: GuestNetworkPlan,
        named_ifindex: u32,
        unrelated: GuestNetworkPlan,
        unrelated_ifindex: u32,
    }

    impl TwoAttachments {
        async fn active() -> Self {
            let kernel = FakeAttachmentKernel::healthy();
            let owner = owner_over(&kernel);
            let pool = GuestAddressPool::new(
                "100.95.0.0/16".parse().expect("node prefix"),
                "ovd-gbr0".to_owned(),
                Ipv4Addr::new(100, 95, 0, 1),
                Ipv4Addr::new(100, 95, 0, 1),
            );
            let named = pool
                .assign(AllocationId::new("nd295-s12").expect("named allocation"))
                .expect("named lease");
            let unrelated = pool
                .assign(AllocationId::new("nd295-s12-unrelated").expect("unrelated allocation"))
                .expect("unrelated lease");
            activated(&owner, &named).await;
            activated(&owner, &unrelated).await;
            let node = kernel.node();
            let named_ifindex = node.taps[&named.assignment().tap].ifindex;
            let unrelated_ifindex = node.taps[&unrelated.assignment().tap].ifindex;
            Self { kernel, owner, pool, named, named_ifindex, unrelated, unrelated_ifindex }
        }

        fn named_parts(&self) -> AttachmentParts {
            self.kernel.node().parts(&self.named.assignment().tap, self.named_ifindex)
        }

        fn unrelated_parts(&self) -> AttachmentParts {
            self.kernel.node().parts(&self.unrelated.assignment().tap, self.unrelated_ifindex)
        }

        fn names_unrelated(&self, mark: usize) -> bool {
            let unrelated = tap_of(&self.unrelated);
            self.kernel.trace_since(mark).iter().any(|(_, tap)| *tap == unrelated)
        }
    }

    /// The typed error a single failure of `call` during teardown carries.
    fn assert_teardown_leaf_error(error: &GuestNetworkError, call: AllocationCall) {
        let tcx = |operation| (operation, true);
        let netlink = |operation| (operation, false);
        let (operation, is_tcx) = match call {
            AllocationCall::RemoveEndpoint => tcx(GuestNetworkOperation::EndpointDelete),
            AllocationCall::DetachPinnedLink => tcx(GuestNetworkOperation::TcxDetach),
            AllocationCall::DetachPinnedEgressLink => tcx(GuestNetworkOperation::TcxEgressDetach),
            AllocationCall::ReadEndpoint => tcx(GuestNetworkOperation::EndpointMapObserve),
            AllocationCall::QueryAttachment => tcx(GuestNetworkOperation::TcxQuery),
            AllocationCall::LinkPinPresent => tcx(GuestNetworkOperation::TcxLinkPin),
            AllocationCall::QueryEgressAttachment => tcx(GuestNetworkOperation::TcxEgressQuery),
            AllocationCall::EgressLinkPinPresent => tcx(GuestNetworkOperation::TcxEgressLinkPin),
            AllocationCall::SetTapDown => netlink(GuestNetworkOperation::TapSetDown),
            AllocationCall::ObserveTap => netlink(GuestNetworkOperation::TapObserve),
            AllocationCall::DeleteTap => netlink(GuestNetworkOperation::TapDelete),
            AllocationCall::DeleteGuard => netlink(GuestNetworkOperation::GuardMemberDelete),
            AllocationCall::ObserveGuard => netlink(GuestNetworkOperation::CleanupComplement),
            other => panic!("teardown reached a leaf outside the D12A teardown set: {other:?}"),
        };
        let matched = if is_tcx {
            is_tcx_failure(error, operation)
        } else {
            is_netlink_failure(error, operation)
        };
        assert!(matched, "a failed {call:?} returns its {operation:?} source, got {error:?}");
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-12 — Teardown leaves nothing behind and converges on parts already gone
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A healthy teardown mutates in exactly the order endpoint delete ->
    /// ingress detach -> egress detach -> TAP down (read back before delete)
    /// -> `RTM_DELLINK` -> guard-member delete, then reads the complement back,
    /// and leaves the named attachment empty and the unrelated one byte-equal.
    /// A failure of any single leaf of that teardown returns that leaf's typed
    /// error while every other teardown call still runs; the owner keeps the
    /// allocation, and a retry reaches the empty complement. Both the
    /// provisioned-down and the active phase tear down.
    #[tokio::test]
    async fn every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement()
    {
        let healthy_fixture = TwoAttachments::active().await;
        let named_tap = tap_of(&healthy_fixture.named);
        let unrelated_before = healthy_fixture.unrelated_parts();
        let leases_before = healthy_fixture.pool.snapshot();
        assert_eq!(
            leases_before.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                healthy_fixture.named.alloc().clone(),
                healthy_fixture.unrelated.alloc().clone()
            ]),
            "the precondition: both allocations hold their leases"
        );
        let mark = healthy_fixture.kernel.mark();
        healthy_fixture
            .owner
            .teardown(&healthy_fixture.named)
            .await
            .expect("a healthy teardown reaches the empty complement");
        let healthy = healthy_fixture.kernel.trace_since(mark);

        let mutations = healthy
            .iter()
            .filter(|(call, _)| MUTATIONS.contains(call))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                AllocationCall::RemoveEndpoint,
                AllocationCall::DetachPinnedLink,
                AllocationCall::DetachPinnedEgressLink,
                AllocationCall::SetTapDown,
                AllocationCall::DeleteTap,
                AllocationCall::DeleteGuard,
            ]
            .map(|call| (call, named_tap.clone()))
            .to_vec(),
            "teardown mutation order: endpoint -> ingress -> egress -> TAP down -> TAP delete -> guard member"
        );
        let position = |wanted: AllocationCall| {
            healthy.iter().position(|(call, _)| *call == wanted).expect("healthy teardown call")
        };
        let set_down = position(AllocationCall::SetTapDown);
        let delete = position(AllocationCall::DeleteTap);
        let guard_delete = position(AllocationCall::DeleteGuard);
        assert!(
            healthy[set_down..delete]
                .iter()
                .any(|entry| *entry == (AllocationCall::ObserveTap, named_tap.clone())),
            "the TAP is read back down before RTM_DELLINK: {healthy:?}"
        );
        let complement = &healthy[guard_delete..];
        assert!(
            complement.contains(&(AllocationCall::ObserveTap, named_tap.clone()))
                && complement.contains(&(AllocationCall::ObserveGuard, None)),
            "the TAP and guard complement are read back after the last delete: {healthy:?}"
        );
        assert!(healthy_fixture.named_parts().is_empty(), "the named complement is empty");
        assert_eq!(healthy_fixture.unrelated_parts(), unrelated_before, "unrelated is byte-equal");
        assert!(!healthy_fixture.names_unrelated(mark), "teardown never names the unrelated TAP");
        assert_eq!(
            healthy_fixture.kernel.guard_expectations().last(),
            Some(&BTreeSet::from([healthy_fixture.unrelated.assignment().tap.clone()])),
            "the final guard read-back expects exactly the unrelated TAP"
        );

        // Teardown releases no lease: releasing is its caller's, after `Ok`.
        assert_eq!(
            healthy_fixture.pool.snapshot(),
            leases_before,
            "the owner's teardown leaves both leases as they were"
        );

        let repeat = healthy_fixture.kernel.mark();
        healthy_fixture
            .owner
            .teardown(&healthy_fixture.named)
            .await
            .expect("a repeat is idempotent");
        let never = scratch_plan("nd295-s12-never", "t295-s12n", 99);
        healthy_fixture.owner.teardown(&never).await.expect("an unheld allocation is idempotent");
        assert!(
            healthy_fixture.kernel.calls_since(repeat).iter().all(|call| !call.wrote),
            "neither a repeat nor an unheld teardown writes"
        );
        assert_eq!(healthy_fixture.unrelated_parts(), unrelated_before);

        // A provisioned-down attachment tears down to the same empty complement.
        let down = scratch_plan("nd295-s12-down", "t295-s12d", 40);
        provisioned(&healthy_fixture.owner, &down).await;
        let down_ifindex = healthy_fixture.kernel.node().taps[&down.assignment().tap].ifindex;
        healthy_fixture.owner.teardown(&down).await.expect("a provisioned-down teardown");
        assert!(
            healthy_fixture.kernel.node().parts(&down.assignment().tap, down_ifindex).is_empty()
        );

        // One failure of each call the healthy teardown made.
        for (index, (call, _)) in healthy.iter().enumerate() {
            let occurrence = healthy[..=index].iter().filter(|(prior, _)| prior == call).count();
            let fixture = TwoAttachments::active().await;
            let unrelated_before = fixture.unrelated_parts();
            let mark = fixture.kernel.mark();
            fixture.kernel.fail_nth(*call, occurrence);

            let error = fixture
                .owner
                .teardown(&fixture.named)
                .await
                .expect_err("a leaf failure is returned");
            assert_teardown_leaf_error(&error, *call);
            assert_eq!(
                fixture.kernel.trace_since(mark),
                healthy,
                "cleanup continues through every teardown call after {call:?} #{occurrence}"
            );
            assert_eq!(fixture.unrelated_parts(), unrelated_before);

            fixture.kernel.clear_faults();
            fixture
                .owner
                .teardown(&fixture.named)
                .await
                .expect("the retained allocation retries to the empty complement");
            assert!(
                fixture.named_parts().is_empty(),
                "retry after {call:?} #{occurrence} empties the complement: {:?}",
                fixture.named_parts()
            );
            assert_eq!(fixture.unrelated_parts(), unrelated_before);
            assert!(!fixture.names_unrelated(mark));
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-12 — Teardown leaves nothing behind and converges on parts already gone
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With any one part of the named attachment already gone — its TAP, an
    /// ingress or egress attachment, an ingress or egress pin, its endpoint
    /// entry, or its guard member — and with all of them gone together,
    /// teardown returns `Ok`, the absent part is counted removed without a
    /// write, an absent TAP is never set down or deleted and no attachment
    /// point is queried on it, the complement ends empty, and the unrelated
    /// attachment is byte-equal and never named.
    #[tokio::test]
    async fn teardown_converges_on_every_absent_part_singly_and_together() {
        let rows = EVERY_PART
            .iter()
            .map(|part| vec![*part])
            .chain(std::iter::once(EVERY_PART.to_vec()))
            .collect::<Vec<_>>();
        for absent in rows {
            let fixture = TwoAttachments::active().await;
            let tap = fixture.named.assignment().tap.clone();
            let unrelated_before = fixture.unrelated_parts();
            fixture.kernel.with_node(|node| {
                for part in &absent {
                    node.remove_part(*part, &tap, fixture.named_ifindex);
                }
            });
            let mark = fixture.kernel.mark();

            fixture.owner.teardown(&fixture.named).await.unwrap_or_else(|error| {
                panic!("{absent:?} absent: teardown converges, got {error:?}")
            });

            let calls = fixture.kernel.calls_since(mark);
            let wrote =
                |wanted: AllocationCall| calls.iter().any(|call| call.call == wanted && call.wrote);
            let called = |wanted: AllocationCall| calls.iter().any(|call| call.call == wanted);
            for part in &absent {
                match part {
                    AttachmentPart::Tap => {
                        assert!(
                            !called(AllocationCall::SetTapDown)
                                && !called(AllocationCall::DeleteTap),
                            "{absent:?}: an absent TAP is never set down or deleted: {calls:?}"
                        );
                        assert!(
                            !called(AllocationCall::QueryAttachment)
                                && !called(AllocationCall::QueryEgressAttachment),
                            "{absent:?}: no attachment point is queried on an absent TAP: {calls:?}"
                        );
                    }
                    AttachmentPart::EndpointEntry => {
                        assert!(!wrote(AllocationCall::RemoveEndpoint));
                    }
                    AttachmentPart::GuardMember => assert!(!wrote(AllocationCall::DeleteGuard)),
                    AttachmentPart::IngressPin => assert!(!wrote(AllocationCall::DetachPinnedLink)),
                    AttachmentPart::EgressPin => {
                        assert!(!wrote(AllocationCall::DetachPinnedEgressLink));
                    }
                    AttachmentPart::IngressAttachment | AttachmentPart::EgressAttachment => {}
                }
            }
            if absent.len() == EVERY_PART.len() {
                assert!(
                    calls.iter().all(|call| !call.wrote),
                    "with every part gone, teardown writes nothing: {calls:?}"
                );
            }
            assert!(
                fixture.named_parts().is_empty(),
                "{absent:?}: the complement ends empty: {:?}",
                fixture.named_parts()
            );
            assert_eq!(
                fixture.unrelated_parts(),
                unrelated_before,
                "{absent:?}: unrelated is byte-equal"
            );
            assert!(!fixture.names_unrelated(mark), "{absent:?}: the unrelated TAP is never named");

            let repeat = fixture.kernel.mark();
            fixture.owner.teardown(&fixture.named).await.expect("a repeat is idempotent");
            assert!(fixture.kernel.calls_since(repeat).iter().all(|call| !call.wrote));
        }
    }

    // ---- S-ND295-50 --------------------------------------------------------

    /// One active and one provisioned-down attachment on one owner over one
    /// fake kernel: `active` holds ifindex 295, `down` holds 296.
    struct AuditFixture {
        kernel: Arc<FakeAttachmentKernel>,
        owner: HostSharedGuestNetworkOwner,
        active: GuestNetworkPlan,
        down: GuestNetworkPlan,
    }

    impl AuditFixture {
        async fn new(prefix: &str) -> Self {
            let kernel = FakeAttachmentKernel::healthy();
            let owner = owner_over(&kernel);
            let active = scratch_plan(&format!("{prefix}-active"), "t295-aa", 2);
            let down = scratch_plan(&format!("{prefix}-down"), "t295-ad", 3);
            activated(&owner, &active).await;
            provisioned(&owner, &down).await;
            Self { kernel, owner, active, down }
        }

        fn managed_taps(&self) -> [Option<String>; 2] {
            [tap_of(&self.active), tap_of(&self.down)]
        }

        /// Calls since `mark` that read one managed allocation's own parts.
        fn per_allocation_reads_since(&self, mark: usize) -> Vec<(AllocationCall, Option<String>)> {
            let managed = self.managed_taps();
            self.kernel
                .trace_since(mark)
                .into_iter()
                .filter(|(call, tap)| PER_ALLOCATION_READS.contains(call) && managed.contains(tap))
                .collect()
        }

        async fn audit(
            &self,
        ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
            self.owner.audit_shared().await
        }
    }

    const ACTIVE_IFINDEX: u32 = FIRST_TAP_IFINDEX;
    const DOWN_IFINDEX: u32 = FIRST_TAP_IFINDEX + 1;

    /// One node-level part the audit checks before any per-allocation part.
    #[derive(Debug, Clone, Copy)]
    enum NodeFault {
        BridgeAbsent,
        BridgeMacChanged,
        BridgeDown,
        BridgeGatewayLost,
        GuardTableAbsent,
        GuardRuleMissing,
        GuardMemberForUnmanagedTap,
        GuardReadFails,
        TcxProgramUnloaded,
        EndpointMapReplaced,
        CounterMapReplaced,
        EndpointMapPinRemoved,
        CounterMapPinRemoved,
        UnmanagedEndpointEntry,
        DebugMaskDumpFails,
    }

    impl NodeFault {
        const ALL: [Self; 15] = [
            Self::BridgeAbsent,
            Self::BridgeMacChanged,
            Self::BridgeDown,
            Self::BridgeGatewayLost,
            Self::GuardTableAbsent,
            Self::GuardRuleMissing,
            Self::GuardMemberForUnmanagedTap,
            Self::GuardReadFails,
            Self::TcxProgramUnloaded,
            Self::EndpointMapReplaced,
            Self::CounterMapReplaced,
            Self::EndpointMapPinRemoved,
            Self::CounterMapPinRemoved,
            Self::UnmanagedEndpointEntry,
            Self::DebugMaskDumpFails,
        ];

        /// The matrix component the audit names for this part.
        const fn component(self) -> SharedGuestNetworkComponent {
            match self {
                Self::BridgeAbsent
                | Self::BridgeMacChanged
                | Self::BridgeDown
                | Self::BridgeGatewayLost
                | Self::DebugMaskDumpFails => SharedGuestNetworkComponent::Bridge,
                Self::GuardTableAbsent
                | Self::GuardRuleMissing
                | Self::GuardMemberForUnmanagedTap
                | Self::GuardReadFails => SharedGuestNetworkComponent::BridgeGuard,
                Self::TcxProgramUnloaded => SharedGuestNetworkComponent::TcxLink,
                Self::EndpointMapReplaced | Self::UnmanagedEndpointEntry => {
                    SharedGuestNetworkComponent::EndpointMap
                }
                Self::CounterMapReplaced => SharedGuestNetworkComponent::CounterMap,
                Self::EndpointMapPinRemoved | Self::CounterMapPinRemoved => {
                    SharedGuestNetworkComponent::BpffsPin
                }
            }
        }

        fn inject(self, kernel: &FakeAttachmentKernel) {
            match self {
                Self::GuardReadFails => kernel.fail_always(AllocationCall::ObserveGuard),
                Self::DebugMaskDumpFails => {
                    kernel.fail_always(AllocationCall::ObserveDebugMsgMasks);
                }
                node_fault => kernel.with_node(|node| match node_fault {
                    Self::BridgeAbsent => node.bridge = None,
                    Self::BridgeMacChanged => {
                        node.bridge.as_mut().expect("bridge").mac = [0x02, 0xde, 0xad, 0, 0, 1];
                    }
                    Self::BridgeDown => node.bridge.as_mut().expect("bridge").up = false,
                    Self::BridgeGatewayLost => {
                        node.bridge.as_mut().expect("bridge").gateway = false;
                    }
                    Self::GuardTableAbsent => node.guard_table = false,
                    Self::GuardRuleMissing => node.guard_rules_intact = false,
                    Self::GuardMemberForUnmanagedTap => {
                        node.guard_members.insert("t295-foreign".to_owned());
                    }
                    Self::TcxProgramUnloaded => node.tcx_program_loaded = false,
                    Self::EndpointMapReplaced => node.endpoint_map_intact = false,
                    Self::CounterMapReplaced => node.counter_map_intact = false,
                    Self::EndpointMapPinRemoved => node.endpoint_map_pinned = false,
                    Self::CounterMapPinRemoved => node.counter_map_pinned = false,
                    Self::UnmanagedEndpointEntry => {
                        node.endpoints.insert(
                            4_242,
                            GuestTcxEndpoint {
                                source_ipv4: Ipv4Addr::new(100, 95, 9, 9),
                                source_mac: [0x02, 0x00, 100, 95, 9, 9],
                                bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                            },
                        );
                    }
                    Self::GuardReadFails | Self::DebugMaskDumpFails => {
                        unreachable!("leaf faults are armed above")
                    }
                }),
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-50 — The owner's audit tells node failures apart from one VM's damaged parts
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With one allocation's own parts also damaged, each failing node-level
    /// part — bridge identity, guard table / rules / an unmanaged member / a
    /// failed guard read, the TCX program, endpoint and counter map identity,
    /// their pins, an unmanaged endpoint entry, and a failed debug-mask dump —
    /// is reported as `Err` naming its matrix component, before any
    /// per-allocation part is read; the failed dump keeps its netlink source
    /// under `TapObserve`. The audit writes nothing and condemns nobody: once
    /// the node part is healthy again, the next audit names the damaged
    /// allocation.
    #[tokio::test]
    async fn every_node_level_audit_failure_names_its_matrix_component_first() {
        for fault in NodeFault::ALL {
            let fixture = AuditFixture::new("nd295-s50-node").await;
            fixture.kernel.with_node(|node| {
                AllocationDamage::HostMacReserved.inject(node, &fixture.active, ACTIVE_IFINDEX);
            });
            let damaged_only = fixture.kernel.node();
            fault.inject(&fixture.kernel);
            let with_fault = fixture.kernel.node();
            let mark = fixture.kernel.mark();

            let failure = fixture.audit().await.expect_err("a node-level failure is an Err");
            assert_eq!(
                failure.component,
                fault.component(),
                "{fault:?} names its matrix component"
            );
            if matches!(fault, NodeFault::DebugMaskDumpFails) {
                assert!(
                    is_netlink_failure(&failure.source, GuestNetworkOperation::TapObserve),
                    "the failed dump keeps its source: {:?}",
                    failure.source
                );
            }
            assert_eq!(
                fixture.per_allocation_reads_since(mark),
                Vec::new(),
                "{fault:?}: no per-allocation part is read after a node-level failure"
            );
            assert_eq!(
                fixture.kernel.mutations_since(mark),
                Vec::new(),
                "{fault:?}: the audit writes nothing"
            );
            assert_eq!(fixture.kernel.node(), with_fault, "{fault:?}: the node is unchanged");

            fixture.kernel.clear_faults();
            fixture.kernel.with_node(|node| *node = damaged_only.clone());
            let audit =
                fixture.audit().await.expect("a healthy node reports per-allocation damage");
            assert_eq!(
                audit.damaged.keys().cloned().collect::<Vec<_>>(),
                vec![fixture.active.alloc().clone()],
                "{fault:?}: the node-level failure condemned and hid nothing"
            );
            AllocationDamage::HostMacReserved.assert_named(
                &audit.damaged[fixture.active.alloc()],
                &fixture.active,
                ACTIVE_IFINDEX,
            );
        }
    }

    /// One per-allocation part the audit reads for an allocation it holds.
    #[derive(Debug, Clone, Copy)]
    enum AllocationDamage {
        TapDeleted,
        TapNotPersistent,
        TapOwnerChanged,
        HostMacReserved,
        HostMacMissing,
        DebugMaskNonZero,
        DebugMaskMissingFromDump,
        IfindexChanged,
        MasterLost,
        ActiveTapFoundDown,
        ProvisionedTapFoundUp,
        IngressDetached,
        IngressPinRemoved,
        EgressDetached,
        EgressPinRemoved,
        EndpointValueChanged,
        GuardMemberRemoved,
    }

    const CHANGED_IFINDEX: u32 = 301;

    impl AllocationDamage {
        const ALL: [Self; 17] = [
            Self::TapDeleted,
            Self::TapNotPersistent,
            Self::TapOwnerChanged,
            Self::HostMacReserved,
            Self::HostMacMissing,
            Self::DebugMaskNonZero,
            Self::DebugMaskMissingFromDump,
            Self::IfindexChanged,
            Self::MasterLost,
            Self::ActiveTapFoundDown,
            Self::ProvisionedTapFoundUp,
            Self::IngressDetached,
            Self::IngressPinRemoved,
            Self::EgressDetached,
            Self::EgressPinRemoved,
            Self::EndpointValueChanged,
            Self::GuardMemberRemoved,
        ];

        /// The damaged allocation: the provisioned-down one for the damage
        /// only a down attachment can show, the active one otherwise.
        const fn targets_down(self) -> bool {
            matches!(self, Self::ProvisionedTapFoundUp)
        }

        /// Damage the target's parts in isolation, so exactly one check can
        /// fail.
        fn inject(self, node: &mut FakeNode, plan: &GuestNetworkPlan, ifindex: u32) {
            let tap = plan.assignment().tap.as_str();
            match self {
                Self::TapDeleted => node.remove_part(AttachmentPart::Tap, tap, ifindex),
                Self::TapNotPersistent => node.tap_mut(tap).persistent = false,
                Self::TapOwnerChanged => {
                    node.tap_mut(tap).owner_uid =
                        Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID);
                }
                // A reserved address the target holds whatever else the owner
                // holds: its own guest MAC (the reserved set includes it).
                Self::HostMacReserved => node.tap_mut(tap).mac = Some(plan.assignment().mac),
                Self::HostMacMissing => node.tap_mut(tap).mac = None,
                Self::DebugMaskNonZero => node.tap_mut(tap).debug_mask = 0x10,
                Self::DebugMaskMissingFromDump => {
                    node.mask_dump_omits.insert(ifindex);
                }
                Self::IfindexChanged => node.tap_mut(tap).ifindex = CHANGED_IFINDEX,
                Self::MasterLost => node.tap_mut(tap).master = None,
                Self::ActiveTapFoundDown => node.tap_mut(tap).up = false,
                Self::ProvisionedTapFoundUp => node.tap_mut(tap).up = true,
                Self::IngressDetached => node.tap_mut(tap).ingress.clear(),
                Self::IngressPinRemoved => {
                    node.ingress_pins.remove(tap);
                }
                Self::EgressDetached => node.tap_mut(tap).egress.clear(),
                Self::EgressPinRemoved => {
                    node.egress_pins.remove(tap);
                }
                Self::EndpointValueChanged => {
                    node.endpoints.get_mut(&ifindex).expect("endpoint entry").source_mac =
                        [0x02, 0xde, 0xad, 0xbe, 0xef, 0x01];
                }
                Self::GuardMemberRemoved => {
                    node.remove_part(AttachmentPart::GuardMember, tap, ifindex);
                }
            }
        }

        /// The first failing check the audit names for this damage.
        fn assert_named(self, error: &GuestNetworkError, plan: &GuestNetworkPlan, ifindex: u32) {
            let tap = plan.assignment().tap.as_str();
            let up = !self.targets_down();
            match self {
                Self::TapDeleted | Self::DebugMaskMissingFromDump => assert!(
                    matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TapObserve,
                            expected: GuestNetworkFact::Tap { name, .. },
                            observed: None,
                        } if name == tap
                    ),
                    "{self:?}: the absent-TAP mismatch, got {error:?}"
                ),
                Self::TapNotPersistent => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &expected_tap(plan, ifindex, up),
                    Some(&tap_fact(tap, Some(ifindex), GuestLinkKind::Tap, false, up, ROOT_UID)),
                ),
                Self::TapOwnerChanged => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &expected_tap(plan, ifindex, up),
                    Some(&tap_fact(
                        tap,
                        Some(ifindex),
                        GuestLinkKind::Tap,
                        true,
                        up,
                        Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                    )),
                ),
                Self::HostMacReserved => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &host_mac_fact(ifindex, TapHostAddress::Unreserved),
                    Some(&host_mac_fact(ifindex, TapHostAddress::Reserved(plan.assignment().mac))),
                ),
                Self::HostMacMissing => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &host_mac_fact(ifindex, TapHostAddress::Unreserved),
                    Some(&host_mac_fact(ifindex, TapHostAddress::Missing)),
                ),
                Self::DebugMaskNonZero => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &GuestNetworkFact::TapDebugMsgMask { ifindex, mask: 0 },
                    Some(&GuestNetworkFact::TapDebugMsgMask { ifindex, mask: 0x10 }),
                ),
                Self::IfindexChanged => assert!(
                    matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TapObserve,
                            expected: GuestNetworkFact::Tap { ifindex: expected, .. },
                            observed: Some(GuestNetworkFact::Tap { ifindex: observed, .. }),
                        } if *expected == Some(ifindex) && *observed == Some(CHANGED_IFINDEX)
                    ),
                    "{self:?}: the recorded ifindex against the live one, got {error:?}"
                ),
                Self::MasterLost => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &master_fact(ifindex, Some(BRIDGE_IFINDEX)),
                    Some(&master_fact(ifindex, None)),
                ),
                Self::ActiveTapFoundDown | Self::ProvisionedTapFoundUp => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &expected_tap(plan, ifindex, up),
                    Some(&tap_fact(tap, Some(ifindex), GuestLinkKind::Tap, true, !up, ROOT_UID)),
                ),
                Self::IngressDetached => assert_mismatch(
                    error,
                    GuestNetworkOperation::TcxQuery,
                    &attachment_fact(ifindex, Some(INGRESS_PROGRAM), TcxAttachPoint::Ingress),
                    Some(&attachment_fact(ifindex, None, TcxAttachPoint::Ingress)),
                ),
                Self::IngressPinRemoved => assert!(
                    matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin { .. },
                            ..
                        }
                    ),
                    "{self:?}: the ingress pin mismatch, got {error:?}"
                ),
                Self::EgressDetached => assert_mismatch(
                    error,
                    GuestNetworkOperation::TcxEgressQuery,
                    &attachment_fact(ifindex, Some(EGRESS_PROGRAM), TcxAttachPoint::Egress),
                    Some(&attachment_fact(ifindex, None, TcxAttachPoint::Egress)),
                ),
                Self::EgressPinRemoved => assert!(
                    matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            operation: GuestNetworkOperation::TcxEgressLinkPin,
                            expected: GuestNetworkFact::BpfLinkPin { .. },
                            ..
                        }
                    ),
                    "{self:?}: the egress pin mismatch, got {error:?}"
                ),
                Self::EndpointValueChanged => assert_mismatch(
                    error,
                    GuestNetworkOperation::EndpointMapObserve,
                    &endpoint_fact(ifindex, endpoint_for(plan)),
                    Some(&endpoint_fact(
                        ifindex,
                        GuestTcxEndpoint {
                            source_mac: [0x02, 0xde, 0xad, 0xbe, 0xef, 0x01],
                            ..endpoint_for(plan)
                        },
                    )),
                ),
                Self::GuardMemberRemoved => assert!(
                    matches!(
                        error,
                        GuestNetworkError::PostconditionMismatch {
                            expected: GuestNetworkFact::BridgeGuard { member: true, .. },
                            ..
                        }
                    ),
                    "{self:?}: the missing guard member, got {error:?}"
                ),
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-50 — The owner's audit tells node failures apart from one VM's damaged parts
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With every node-level part healthy, each damage to one allocation's
    /// own parts — TAP deleted, non-persistent, owner changed, a host-side
    /// MAC that breaks the D-295-R21 invariant (a reserved address, here the
    /// TAP's own guest MAC, or no address), debug mask non-zero or missing
    /// from the dump, ifindex changed, master lost, administrative state
    /// against its phase, ingress or egress attachment detached or pin
    /// removed, endpoint value changed, guard member removed — returns `Ok`
    /// naming exactly that allocation with its first failing check; the
    /// other allocation is not named; two damaged allocations are both named.
    /// The audit writes nothing.
    #[tokio::test]
    async fn every_per_allocation_damage_is_named_only_when_the_node_is_healthy() {
        for damage in AllocationDamage::ALL {
            let fixture = AuditFixture::new("nd295-s50-alloc").await;
            let (target, ifindex) = if damage.targets_down() {
                (fixture.down.clone(), DOWN_IFINDEX)
            } else {
                (fixture.active.clone(), ACTIVE_IFINDEX)
            };
            fixture.kernel.with_node(|node| damage.inject(node, &target, ifindex));
            let damaged = fixture.kernel.node();
            let mark = fixture.kernel.mark();

            let audit = fixture.audit().await.unwrap_or_else(|failure| {
                panic!("{damage:?}: a healthy node reports per-allocation damage, got {failure:?}")
            });
            assert_eq!(
                audit.damaged.keys().cloned().collect::<Vec<_>>(),
                vec![target.alloc().clone()],
                "{damage:?}: exactly the damaged allocation is named"
            );
            damage.assert_named(&audit.damaged[target.alloc()], &target, ifindex);
            assert_eq!(
                fixture.kernel.mutations_since(mark),
                Vec::new(),
                "{damage:?}: the audit writes nothing"
            );
            assert_eq!(fixture.kernel.node(), damaged, "{damage:?}: the node is unchanged");
        }

        let fixture = AuditFixture::new("nd295-s50-both").await;
        fixture.kernel.with_node(|node| {
            AllocationDamage::HostMacReserved.inject(node, &fixture.active, ACTIVE_IFINDEX);
            AllocationDamage::ProvisionedTapFoundUp.inject(node, &fixture.down, DOWN_IFINDEX);
        });
        let audit = fixture.audit().await.expect("a healthy node reports both damaged allocations");
        assert_eq!(
            audit.damaged.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([fixture.active.alloc().clone(), fixture.down.alloc().clone()])
        );
        AllocationDamage::HostMacReserved.assert_named(
            &audit.damaged[fixture.active.alloc()],
            &fixture.active,
            ACTIVE_IFINDEX,
        );
        AllocationDamage::ProvisionedTapFoundUp.assert_named(
            &audit.damaged[fixture.down.alloc()],
            &fixture.down,
            DOWN_IFINDEX,
        );

        let healthy = AuditFixture::new("nd295-s50-none").await;
        let mark = healthy.kernel.mark();
        let audit = healthy.audit().await.expect("a healthy node");
        assert!(audit.damaged.is_empty(), "exact attachments are not named: {:?}", audit.damaged);
        let reads = healthy.per_allocation_reads_since(mark);
        for tap in healthy.managed_taps() {
            assert!(
                reads.contains(&(AllocationCall::ObserveTap, tap.clone())),
                "the audit reads the TAP of every held allocation ({tap:?}): {reads:?}"
            );
        }
        assert_eq!(healthy.kernel.mutations_since(mark), Vec::new());
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-50 — The owner's audit tells node failures apart from one VM's damaged parts
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// An allocation the audit names is condemned: later audits neither read
    /// nor name it though its damage persists; quiescence does not set it
    /// down; restore does not raise it; activation refuses it as a missing
    /// record; teardown still removes it. An allocation quiescence could not
    /// confirm down is excluded from later audits the same way. The healthy
    /// allocation stays in every universe.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-50)"]
    async fn a_condemned_allocation_leaves_every_later_audit_and_restore_universe() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let damaged = scratch_plan("nd295-s50-condemned-audit", "t295-ca", 2);
        let healthy = scratch_plan("nd295-s50-condemned-healthy", "t295-cb", 3);
        let unconfirmed = scratch_plan("nd295-s50-condemned-quiesce", "t295-cc", 4);
        for plan in [&damaged, &healthy, &unconfirmed] {
            activated(&owner, plan).await;
        }
        let damaged_ifindex = FIRST_TAP_IFINDEX;
        kernel.with_node(|node| {
            AllocationDamage::HostMacReserved.inject(node, &damaged, damaged_ifindex);
        });

        let first = owner.audit_shared().await.expect("a healthy node");
        assert_eq!(
            first.damaged.keys().cloned().collect::<Vec<_>>(),
            vec![damaged.alloc().clone()]
        );

        let mark = kernel.mark();
        let second = owner.audit_shared().await.expect("a healthy node");
        assert!(second.damaged.is_empty(), "a condemned allocation is never named again");
        let reads = kernel.trace_since(mark);
        assert!(
            !reads.iter().any(|(_, tap)| *tap == tap_of(&damaged)),
            "a condemned allocation's parts are not read: {reads:?}"
        );
        assert!(reads.contains(&(AllocationCall::ObserveTap, tap_of(&healthy))));

        kernel.with_node(|node| {
            node.remove_part(
                AttachmentPart::Tap,
                &unconfirmed.assignment().tap,
                FIRST_TAP_IFINDEX + 2,
            );
        });
        let quiesce = kernel.mark();
        let quiescence = owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<Vec<_>>(),
            vec![unconfirmed.alloc().clone()]
        );
        assert!(
            !kernel.trace_since(quiesce).iter().any(|(_, tap)| *tap == tap_of(&damaged)),
            "quiescence does not touch the audit-condemned allocation"
        );

        let restore = kernel.mark();
        owner.restore_quiesced_taps().await.expect("restore raises the quiesced allocation");
        let raised = kernel
            .trace_since(restore)
            .into_iter()
            .filter(|(call, _)| *call == AllocationCall::SetTapUp)
            .map(|(_, tap)| tap)
            .collect::<Vec<_>>();
        assert_eq!(raised, vec![tap_of(&healthy)], "restore raises only the quiesced healthy TAP");

        let third = kernel.mark();
        let audit = owner.audit_shared().await.expect("a healthy node");
        assert!(audit.damaged.is_empty());
        let reads = kernel.trace_since(third);
        assert!(
            !reads.iter().any(|(_, tap)| *tap == tap_of(&damaged) || *tap == tap_of(&unconfirmed)),
            "neither condemned allocation is read: {reads:?}"
        );
        assert!(reads.contains(&(AllocationCall::ObserveTap, tap_of(&healthy))));

        let refused = kernel.mark();
        let refusal =
            owner.activate(&damaged).await.expect_err("a condemned allocation is refused");
        assert_refused_as_missing_record(&refusal, &damaged);
        assert_eq!(kernel.mutations_since(refused), Vec::new());

        owner.teardown(&damaged).await.expect("teardown accepts a condemned allocation");
        assert!(kernel.node().parts(&damaged.assignment().tap, damaged_ifindex).is_empty());
        owner.teardown(&unconfirmed).await.expect("teardown accepts a condemned allocation");
        assert!(
            kernel.node().parts(&unconfirmed.assignment().tap, FIRST_TAP_IFINDEX + 2).is_empty()
        );
    }

    // ---- S-ND295-51 --------------------------------------------------------

    /// One protection fact activation re-reads before it raises the TAP.
    #[derive(Debug, Clone, Copy)]
    enum ProtectionFault {
        BridgeAbsent,
        BridgeReplaced,
        TapOwnerChanged,
        TapAlreadyUp,
        TapNotPersistent,
        HostMacReserved,
        HostMacMissing,
        DebugMaskNonZero,
        DebugMaskReadFails,
        GuardMemberRemoved,
        EndpointValueChanged,
        IngressDetached,
        IngressPinRemoved,
        EgressDetached,
        EgressPinRemoved,
    }

    impl ProtectionFault {
        const ALL: [Self; 15] = [
            Self::BridgeAbsent,
            Self::BridgeReplaced,
            Self::TapOwnerChanged,
            Self::TapAlreadyUp,
            Self::TapNotPersistent,
            Self::HostMacReserved,
            Self::HostMacMissing,
            Self::DebugMaskNonZero,
            Self::DebugMaskReadFails,
            Self::GuardMemberRemoved,
            Self::EndpointValueChanged,
            Self::IngressDetached,
            Self::IngressPinRemoved,
            Self::EgressDetached,
            Self::EgressPinRemoved,
        ];

        fn inject(self, kernel: &FakeAttachmentKernel, plan: &GuestNetworkPlan, ifindex: u32) {
            let tap = plan.assignment().tap.clone();
            match self {
                Self::DebugMaskReadFails => {
                    kernel.fail_once(AllocationCall::ObserveTapDebugMsgMask);
                }
                Self::BridgeAbsent => kernel.with_node(|node| node.bridge = None),
                Self::BridgeReplaced => {
                    kernel.with_node(|node| node.bridge.as_mut().expect("bridge").ifindex = 30);
                }
                Self::TapOwnerChanged => kernel.with_node(|node| {
                    AllocationDamage::TapOwnerChanged.inject(node, plan, ifindex);
                }),
                Self::TapAlreadyUp => kernel.with_node(|node| node.tap_mut(&tap).up = true),
                Self::TapNotPersistent => kernel.with_node(|node| {
                    AllocationDamage::TapNotPersistent.inject(node, plan, ifindex);
                }),
                Self::HostMacReserved => kernel.with_node(|node| {
                    AllocationDamage::HostMacReserved.inject(node, plan, ifindex);
                }),
                Self::HostMacMissing => kernel.with_node(|node| {
                    AllocationDamage::HostMacMissing.inject(node, plan, ifindex);
                }),
                Self::DebugMaskNonZero => kernel.with_node(|node| {
                    AllocationDamage::DebugMaskNonZero.inject(node, plan, ifindex);
                }),
                Self::GuardMemberRemoved => kernel.with_node(|node| {
                    AllocationDamage::GuardMemberRemoved.inject(node, plan, ifindex);
                }),
                Self::EndpointValueChanged => kernel.with_node(|node| {
                    AllocationDamage::EndpointValueChanged.inject(node, plan, ifindex);
                }),
                Self::IngressDetached => kernel.with_node(|node| {
                    AllocationDamage::IngressDetached.inject(node, plan, ifindex);
                }),
                Self::IngressPinRemoved => kernel.with_node(|node| {
                    AllocationDamage::IngressPinRemoved.inject(node, plan, ifindex);
                }),
                Self::EgressDetached => kernel.with_node(|node| {
                    AllocationDamage::EgressDetached.inject(node, plan, ifindex);
                }),
                Self::EgressPinRemoved => kernel.with_node(|node| {
                    AllocationDamage::EgressPinRemoved.inject(node, plan, ifindex);
                }),
            }
        }

        fn assert_refused(self, error: &GuestNetworkError, plan: &GuestNetworkPlan, ifindex: u32) {
            let tap = plan.assignment().tap.as_str();
            match self {
                Self::BridgeAbsent => assert_mismatch(
                    error,
                    GuestNetworkOperation::BridgeObserve,
                    &bridge_fact("ovd-gbr0", None, GuestLinkKind::Bridge),
                    None,
                ),
                Self::BridgeReplaced => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &master_fact(ifindex, Some(30)),
                    Some(&master_fact(ifindex, Some(BRIDGE_IFINDEX))),
                ),
                Self::TapAlreadyUp => assert_mismatch(
                    error,
                    GuestNetworkOperation::TapObserve,
                    &expected_tap(plan, ifindex, false),
                    Some(&tap_fact(tap, Some(ifindex), GuestLinkKind::Tap, true, true, ROOT_UID)),
                ),
                Self::DebugMaskReadFails => assert!(
                    is_netlink_failure(error, GuestNetworkOperation::TapObserve),
                    "{self:?}: the sourced mask-read failure, got {error:?}"
                ),
                Self::TapOwnerChanged => {
                    AllocationDamage::TapOwnerChanged.assert_named_down(error, plan, ifindex);
                }
                Self::TapNotPersistent => {
                    AllocationDamage::TapNotPersistent.assert_named_down(error, plan, ifindex);
                }
                Self::HostMacReserved => {
                    AllocationDamage::HostMacReserved.assert_named(error, plan, ifindex);
                }
                Self::HostMacMissing => {
                    AllocationDamage::HostMacMissing.assert_named(error, plan, ifindex);
                }
                Self::DebugMaskNonZero => {
                    AllocationDamage::DebugMaskNonZero.assert_named(error, plan, ifindex);
                }
                Self::GuardMemberRemoved => {
                    AllocationDamage::GuardMemberRemoved.assert_named(error, plan, ifindex);
                }
                Self::EndpointValueChanged => {
                    AllocationDamage::EndpointValueChanged.assert_named(error, plan, ifindex);
                }
                Self::IngressDetached => {
                    AllocationDamage::IngressDetached.assert_named(error, plan, ifindex);
                }
                Self::IngressPinRemoved => {
                    AllocationDamage::IngressPinRemoved.assert_named(error, plan, ifindex);
                }
                Self::EgressDetached => {
                    AllocationDamage::EgressDetached.assert_named(error, plan, ifindex);
                }
                Self::EgressPinRemoved => {
                    AllocationDamage::EgressPinRemoved.assert_named(error, plan, ifindex);
                }
            }
        }
    }

    impl AllocationDamage {
        /// `assert_named` for a TAP identity damage on a still-down TAP.
        fn assert_named_down(
            self,
            error: &GuestNetworkError,
            plan: &GuestNetworkPlan,
            ifindex: u32,
        ) {
            let tap = plan.assignment().tap.as_str();
            let observed = match self {
                Self::TapNotPersistent => {
                    tap_fact(tap, Some(ifindex), GuestLinkKind::Tap, false, false, ROOT_UID)
                }
                Self::TapOwnerChanged => tap_fact(
                    tap,
                    Some(ifindex),
                    GuestLinkKind::Tap,
                    true,
                    false,
                    Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                ),
                other => unreachable!("{other:?} is not a down-TAP identity damage"),
            };
            assert_mismatch(
                error,
                GuestNetworkOperation::TapObserve,
                &expected_tap(plan, ifindex, false),
                Some(&observed),
            );
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Activation re-reads every protection fact — bridge and master, TAP
    /// owner / down state / host-side MAC (the D-295-R21 invariant: a reserved
    /// or missing address refuses), debug mask, guard membership, endpoint,
    /// ingress program and pin, egress program and pin — once each, then
    /// raises the TAP once and reads the bridge and TAP back, and reports
    /// `Raised`; a repeat is idempotent. Each protection mismatch refuses with
    /// its pinned fact and writes nothing, leaving the attachment
    /// activatable. A `set_link_up` failure is `TapSetUp`. A failed read-back
    /// after the TAP went up sets it down again and returns the read-back
    /// failure, unless that set-down itself fails, whose `TapSetDown` error
    /// then takes precedence.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn activation_reads_every_protection_fact_before_reporting_success() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let plan = scratch_plan("nd295-s51-activate", "t295-v1", 2);
        let tap = tap_of(&plan);
        provisioned(&owner, &plan).await;
        let mark = kernel.mark();

        assert_eq!(
            owner.activate(&plan).await.expect("every protection fact reads back exactly"),
            TapActivation::Raised
        );
        let trace = kernel.trace_since(mark);
        let raises = trace.iter().filter(|(call, _)| *call == AllocationCall::SetTapUp).count();
        assert_eq!(raises, 1, "exactly one TAP-up: {trace:?}");
        let set_up = trace
            .iter()
            .position(|(call, _)| *call == AllocationCall::SetTapUp)
            .expect("one TAP-up");
        let mut protection_reads =
            trace[..set_up].iter().map(|(call, _)| *call).collect::<Vec<_>>();
        protection_reads.sort_unstable();
        let mut expected_reads = vec![
            AllocationCall::ObserveBridge,
            AllocationCall::ObserveTap,
            AllocationCall::ObserveTapDebugMsgMask,
            AllocationCall::ObserveGuard,
            AllocationCall::ReadEndpoint,
            AllocationCall::QueryAttachment,
            AllocationCall::LinkPinPresent,
            AllocationCall::QueryEgressAttachment,
            AllocationCall::EgressLinkPinPresent,
        ];
        expected_reads.sort_unstable();
        assert_eq!(
            protection_reads, expected_reads,
            "every protection fact is re-read once before TAP-up"
        );
        assert_eq!(
            trace[set_up + 1..].to_vec(),
            vec![(AllocationCall::ObserveBridge, tap.clone()), (AllocationCall::ObserveTap, tap)],
            "the bridge and the TAP are read back after TAP-up"
        );
        assert!(
            kernel.node().taps[&plan.assignment().tap].up,
            "the protected TAP is administratively up"
        );

        let repeat = kernel.mark();
        assert_eq!(
            owner.activate(&plan).await.expect("an idempotent repeat"),
            TapActivation::Raised
        );
        assert_eq!(
            kernel.mutations_since(repeat),
            Vec::new(),
            "a completed activation is not raised again"
        );

        for (index, fault) in ProtectionFault::ALL.into_iter().enumerate() {
            let kernel = FakeAttachmentKernel::healthy();
            let owner = owner_over(&kernel);
            let plan =
                scratch_plan(&format!("nd295-s51-protect-{index}"), &format!("t295-p{index}"), 2);
            provisioned(&owner, &plan).await;
            let provisioned_node = kernel.node();
            fault.inject(&kernel, &plan, FIRST_TAP_IFINDEX);
            let faulted_node = kernel.node();
            let mark = kernel.mark();

            let error =
                owner.activate(&plan).await.expect_err("a protection mismatch refuses activation");
            fault.assert_refused(&error, &plan, FIRST_TAP_IFINDEX);
            assert_eq!(kernel.mutations_since(mark), Vec::new(), "{fault:?}: nothing is written");
            assert_eq!(kernel.node(), faulted_node, "{fault:?}: the node is unchanged");

            kernel.clear_faults();
            kernel.with_node(|node| *node = provisioned_node.clone());
            assert_eq!(
                owner.activate(&plan).await.expect("the attachment stayed activatable"),
                TapActivation::Raised,
                "{fault:?}"
            );
        }

        // `set_link_up` fails.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let plan = scratch_plan("nd295-s51-set-up-fails", "t295-su", 2);
        provisioned(&owner, &plan).await;
        kernel.fail_once(AllocationCall::SetTapUp);
        let error = owner.activate(&plan).await.expect_err("a failed TAP-up");
        assert!(is_netlink_failure(&error, GuestNetworkOperation::TapSetUp), "{error:?}");
        assert!(!kernel.node().taps[&plan.assignment().tap].up);
        assert_eq!(owner.activate(&plan).await.expect("a retry raises"), TapActivation::Raised);

        // The read-back after TAP-up fails: the TAP is set down and read back.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let plan = scratch_plan("nd295-s51-readback-fails", "t295-rb", 2);
        let tap = tap_of(&plan);
        provisioned(&owner, &plan).await;
        kernel.fail_once_after(AllocationCall::SetTapUp, AllocationCall::ObserveBridge);
        let mark = kernel.mark();
        let error = owner.activate(&plan).await.expect_err("a failed final read-back");
        assert!(is_netlink_failure(&error, GuestNetworkOperation::BridgeObserve), "{error:?}");
        let trace = kernel.trace_since(mark);
        let set_up = trace
            .iter()
            .position(|entry| *entry == (AllocationCall::SetTapUp, tap.clone()))
            .expect("the TAP went up");
        let after_up = &trace[set_up..];
        let set_down = after_up
            .iter()
            .position(|entry| *entry == (AllocationCall::SetTapDown, tap.clone()))
            .expect("the owner sets the TAP down after a failed read-back");
        assert!(
            after_up[set_down..].contains(&(AllocationCall::ObserveTap, tap)),
            "the set-down is read back: {trace:?}"
        );
        assert!(!kernel.node().taps[&plan.assignment().tap].up, "the TAP is down again");
        assert_eq!(
            owner.activate(&plan).await.expect("the attachment returned to provisioned-down"),
            TapActivation::Raised
        );

        // The set-down after a failed read-back also fails: its error wins.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let plan = scratch_plan("nd295-s51-quiesce-fails", "t295-qf", 2);
        provisioned(&owner, &plan).await;
        kernel.fail_once_after(AllocationCall::SetTapUp, AllocationCall::ObserveBridge);
        kernel.fail_once_after(AllocationCall::SetTapUp, AllocationCall::SetTapDown);
        let error = owner.activate(&plan).await.expect_err("a failed set-down");
        assert!(is_netlink_failure(&error, GuestNetworkOperation::TapSetDown), "{error:?}");
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// While quiescence is latched, activation returns `QuiescenceLatched`
    /// and writes nothing; after restore it raises. An allocation condemned
    /// by quiescence or by the audit, and one the owner never held, are
    /// refused with the source-less missing-record mismatch and nothing is
    /// written.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn activation_under_a_latch_or_condemnation_changes_nothing() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let waiting = scratch_plan("nd295-s51-latch-waiting", "t295-l1", 2);
        let running = scratch_plan("nd295-s51-latch-running", "t295-l2", 3);
        provisioned(&owner, &waiting).await;
        activated(&owner, &running).await;
        let quiescence = owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert!(quiescence.unconfirmed.is_empty());

        let latched = kernel.node();
        for attempt in 0..2 {
            let mark = kernel.mark();
            assert_eq!(
                owner.activate(&waiting).await.expect("a latched activation is not a failure"),
                TapActivation::QuiescenceLatched,
                "attempt {attempt}"
            );
            assert_eq!(
                kernel.mutations_since(mark),
                Vec::new(),
                "a latched activation writes nothing"
            );
            assert_eq!(kernel.node(), latched);
        }
        owner.restore_quiesced_taps().await.expect("restore clears the latch");
        assert_eq!(
            owner.activate(&waiting).await.expect("activation after restore"),
            TapActivation::Raised
        );

        // Condemned by quiescence.
        let lost = scratch_plan("nd295-s51-condemned-quiesce", "t295-l3", 4);
        activated(&owner, &lost).await;
        let lost_ifindex = kernel.node().taps[&lost.assignment().tap].ifindex;
        kernel.with_node(|node| {
            node.remove_part(AttachmentPart::Tap, &lost.assignment().tap, lost_ifindex);
        });
        let quiescence = owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<Vec<_>>(),
            vec![lost.alloc().clone()]
        );
        owner.restore_quiesced_taps().await.expect("restore clears the latch");
        let before = kernel.node();
        let mark = kernel.mark();
        let refusal = owner.activate(&lost).await.expect_err("a condemned allocation is refused");
        assert_refused_as_missing_record(&refusal, &lost);
        assert_eq!(kernel.mutations_since(mark), Vec::new());
        assert_eq!(kernel.node(), before);

        // Condemned by the audit.
        let damaged = scratch_plan("nd295-s51-condemned-audit", "t295-l4", 5);
        provisioned(&owner, &damaged).await;
        let damaged_ifindex = kernel.node().taps[&damaged.assignment().tap].ifindex;
        kernel.with_node(|node| {
            AllocationDamage::HostMacReserved.inject(node, &damaged, damaged_ifindex);
        });
        let audit = owner.audit_shared().await.expect("a healthy node");
        assert_eq!(
            audit.damaged.keys().cloned().collect::<Vec<_>>(),
            vec![damaged.alloc().clone()]
        );
        let before = kernel.node();
        let mark = kernel.mark();
        let refusal =
            owner.activate(&damaged).await.expect_err("a condemned allocation is refused");
        assert_refused_as_missing_record(&refusal, &damaged);
        assert_eq!(kernel.mutations_since(mark), Vec::new());
        assert_eq!(kernel.node(), before);

        // Never held.
        let never = scratch_plan("nd295-s51-never-held", "t295-l5", 6);
        let mark = kernel.mark();
        let refusal = owner.activate(&never).await.expect_err("an unheld allocation is refused");
        assert_refused_as_missing_record(&refusal, &never);
        assert_eq!(
            kernel.calls_since(mark),
            Vec::new(),
            "a missing record is refused before any leaf call"
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Quiescence sets every active TAP down and reads it back, continuing
    /// past a TAP that is gone (`Netlink { TapSetDown }`) and one that stays
    /// up (`PostconditionMismatch`); it reports exactly those as unconfirmed
    /// and condemns them, confirms the rest, and never touches a
    /// provisioned-down TAP. A repeat while latched with no allocation still
    /// `Active` reports nothing and does no I/O. Restore raises only the
    /// confirmed TAPs; condemned allocations stay refused. Teardown accepts
    /// quiesced and condemned allocations. A netlink session failure
    /// (`NetlinkError::Connect`) on each TAP is classified per TAP (DR-08
    /// (b)-A): the pass returns `Ok`, every such TAP is unconfirmed with
    /// `Netlink { operation: TapSetDown, source: Connect }` and condemned, and
    /// the latch is set.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn quiescence_reports_every_unconfirmed_tap_and_condemns_it() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let confirmed = scratch_plan("nd295-s51-q-confirmed", "t295-q1", 2);
        let gone = scratch_plan("nd295-s51-q-gone", "t295-q2", 3);
        let stuck = scratch_plan("nd295-s51-q-stuck", "t295-q3", 4);
        let removed = scratch_plan("nd295-s51-q-removed", "t295-q4", 5);
        let waiting = scratch_plan("nd295-s51-q-waiting", "t295-q5", 6);
        for plan in [&confirmed, &gone, &stuck, &removed] {
            activated(&owner, plan).await;
        }
        provisioned(&owner, &waiting).await;
        let ifindex_of =
            |plan: &GuestNetworkPlan| kernel.node().taps[&plan.assignment().tap].ifindex;
        let gone_ifindex = ifindex_of(&gone);
        let stuck_ifindex = ifindex_of(&stuck);
        let removed_ifindex = ifindex_of(&removed);
        kernel.with_node(|node| {
            node.remove_part(AttachmentPart::Tap, &gone.assignment().tap, gone_ifindex);
            node.tap_mut(&stuck.assignment().tap).admin_stuck = true;
        });
        let mark = kernel.mark();

        let quiescence = owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([gone.alloc().clone(), stuck.alloc().clone()]),
            "exactly the TAPs that could not be confirmed down"
        );
        assert!(
            matches!(
                &quiescence.unconfirmed[gone.alloc()],
                GuestNetworkError::Netlink {
                    operation: GuestNetworkOperation::TapSetDown,
                    source: NetlinkError::LinkAbsent { .. },
                }
            ),
            "a gone TAP fails set-down: {:?}",
            quiescence.unconfirmed[gone.alloc()]
        );
        assert!(
            matches!(
                &quiescence.unconfirmed[stuck.alloc()],
                GuestNetworkError::PostconditionMismatch { .. }
            ),
            "a TAP read back up is unconfirmed: {:?}",
            quiescence.unconfirmed[stuck.alloc()]
        );
        let node = kernel.node();
        assert!(!node.taps[&confirmed.assignment().tap].up, "confirmed TAPs are down");
        assert!(!node.taps[&removed.assignment().tap].up, "confirmed TAPs are down");
        let trace = kernel.trace_since(mark);
        assert!(
            !trace.iter().any(|(_, tap)| *tap == tap_of(&waiting)),
            "a provisioned-down TAP is not touched: {trace:?}"
        );
        for plan in [&confirmed, &gone, &stuck, &removed] {
            assert!(
                trace.contains(&(AllocationCall::SetTapDown, tap_of(plan))),
                "every active TAP is set down, the pass continuing past failures: {trace:?}"
            );
        }
        for plan in [&confirmed, &stuck, &removed] {
            assert!(
                trace.contains(&(AllocationCall::ObserveTap, tap_of(plan))),
                "read back: {trace:?}"
            );
        }

        let repeat = kernel.mark();
        let again = owner.quiesce_managed_taps().await.expect("a repeat while latched");
        assert!(again.unconfirmed.is_empty(), "a repeat reports nothing");
        assert_eq!(kernel.calls_since(repeat), Vec::new(), "a repeat while latched does no I/O");
        assert_eq!(
            owner.activate(&waiting).await.expect("latched"),
            TapActivation::QuiescenceLatched
        );

        owner.teardown(&removed).await.expect("teardown accepts a quiesced allocation");
        assert!(kernel.node().parts(&removed.assignment().tap, removed_ifindex).is_empty());

        let restore = kernel.mark();
        owner.restore_quiesced_taps().await.expect("restore raises the confirmed TAP");
        let raised = kernel
            .trace_since(restore)
            .into_iter()
            .filter(|(call, _)| *call == AllocationCall::SetTapUp)
            .map(|(_, tap)| tap)
            .collect::<Vec<_>>();
        assert_eq!(
            raised,
            vec![tap_of(&confirmed)],
            "only the confirmed, still-held TAP is raised"
        );
        for plan in [&gone, &stuck] {
            let refusal =
                owner.activate(plan).await.expect_err("an unconfirmed allocation is condemned");
            assert_refused_as_missing_record(&refusal, plan);
        }
        assert_eq!(owner.activate(&waiting).await.expect("latch cleared"), TapActivation::Raised);

        kernel.with_node(|node| node.tap_mut(&stuck.assignment().tap).admin_stuck = false);
        owner
            .teardown(&gone)
            .await
            .expect("teardown accepts a condemned allocation with its TAP gone");
        owner.teardown(&stuck).await.expect("teardown accepts a condemned allocation");
        assert!(kernel.node().parts(&gone.assignment().tap, gone_ifindex).is_empty());
        assert!(kernel.node().parts(&stuck.assignment().tap, stuck_ifindex).is_empty());

        // A netlink session failure on every TAP (DR-08 (b)-A): each set-down
        // fails with the fake's `netlink_failure()`, `NetlinkError::Connect`.
        // It is that TAP's own failure, so the pass continues and returns `Ok`.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let first = scratch_plan("nd295-s51-q-session-first", "t295-w1", 2);
        let second = scratch_plan("nd295-s51-q-session-second", "t295-w2", 3);
        let pending = scratch_plan("nd295-s51-q-session-pending", "t295-w3", 4);
        activated(&owner, &first).await;
        activated(&owner, &second).await;
        provisioned(&owner, &pending).await;
        kernel.fail_always(AllocationCall::SetTapDown);
        let mark = kernel.mark();
        let quiescence = owner
            .quiesce_managed_taps()
            .await
            .expect("a netlink session failure is one TAP's outcome, never a whole-call Err");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([first.alloc().clone(), second.alloc().clone()]),
            "every Active TAP whose set-down could not obtain a session is unconfirmed"
        );
        for plan in [&first, &second] {
            let entry = &quiescence.unconfirmed[plan.alloc()];
            assert!(
                matches!(
                    entry,
                    GuestNetworkError::Netlink {
                        operation: GuestNetworkOperation::TapSetDown,
                        source: NetlinkError::Connect { .. },
                    }
                ),
                "{}: the entry is Netlink {{ operation: TapSetDown, source: Connect }}, got {entry:?}",
                plan.alloc()
            );
        }
        let trace = kernel.trace_since(mark);
        for plan in [&first, &second] {
            assert!(
                trace.contains(&(AllocationCall::SetTapDown, tap_of(plan))),
                "both set-downs are attempted: the pass continues past the first: {trace:?}"
            );
        }
        assert!(
            !trace.iter().any(|(_, tap)| *tap == tap_of(&pending)),
            "the provisioned-down TAP is untouched: {trace:?}"
        );
        assert_eq!(
            owner.activate(&pending).await.expect("a latched activation is not a failure"),
            TapActivation::QuiescenceLatched,
            "the latch is set"
        );

        kernel.clear_faults();
        let restore = kernel.mark();
        owner
            .restore_quiesced_taps()
            .await
            .expect("a restore with nothing quiesced clears the latch");
        assert_eq!(
            kernel.mutations_since(restore),
            Vec::new(),
            "neither condemned TAP is raised: nothing was confirmed down"
        );
        for plan in [&first, &second] {
            let before = kernel.node();
            let refused = kernel.mark();
            let refusal =
                owner.activate(plan).await.expect_err("an unconfirmed allocation is condemned");
            assert_refused_as_missing_record(&refusal, plan);
            assert_eq!(kernel.mutations_since(refused), Vec::new(), "the refusal writes nothing");
            assert_eq!(kernel.node(), before);
        }
        assert_eq!(
            owner.activate(&pending).await.expect("the latch is cleared"),
            TapActivation::Raised
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A netlink session failure is one TAP's failure wherever it strikes
    /// (DR-08 (b)-A). A set-down that lands but whose read-back cannot obtain
    /// a session (`NetlinkError::Connect`) leaves that TAP unconfirmed with
    /// `Netlink { operation: TapSetDown, source: Connect }` and condemned,
    /// while the other TAP is confirmed. In a mixed pass, a `Connect` set-down
    /// after another TAP's failure of a different kind (a TAP gone out of
    /// band, `LinkAbsent`) leaves both unconfirmed, each with its own cause,
    /// and the pass still confirms the TAP after them. Restore raises only
    /// the confirmed TAPs; the unconfirmed ones stay refused.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn a_netlink_session_failure_is_one_taps_unconfirmed_entry_and_the_pass_continues() {
        // The read-back of one TAP cannot obtain a session.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let lost = scratch_plan("nd295-s51-rb-a", "t295-rba", 2);
        let confirmed = scratch_plan("nd295-s51-rb-b", "t295-rbb", 3);
        activated(&owner, &lost).await;
        activated(&owner, &confirmed).await;
        kernel.fail_once_for_after(
            AllocationCall::SetTapDown,
            AllocationCall::ObserveTap,
            &lost.assignment().tap,
        );
        let mark = kernel.mark();
        let quiescence = owner
            .quiesce_managed_taps()
            .await
            .expect("a read-back session failure is one TAP's outcome");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<Vec<_>>(),
            vec![lost.alloc().clone()],
            "only the TAP whose read-back failed is unconfirmed"
        );
        let entry = &quiescence.unconfirmed[lost.alloc()];
        assert!(
            matches!(
                entry,
                GuestNetworkError::Netlink {
                    operation: GuestNetworkOperation::TapSetDown,
                    source: NetlinkError::Connect { .. },
                }
            ),
            "the read-back session failure is Netlink {{ operation: TapSetDown, source: Connect }}, got {entry:?}"
        );
        let trace = kernel.trace_since(mark);
        let set_down = trace
            .iter()
            .position(|entry| *entry == (AllocationCall::SetTapDown, tap_of(&lost)))
            .expect("the unconfirmed TAP was set down");
        assert!(
            trace[set_down..].contains(&(AllocationCall::ObserveTap, tap_of(&lost))),
            "the failed call is the read-back after the set-down: {trace:?}"
        );
        let node = kernel.node();
        assert!(
            !node.taps[&lost.assignment().tap].up,
            "the set-down itself landed; only its confirmation failed"
        );
        assert!(!node.taps[&confirmed.assignment().tap].up, "the other TAP is confirmed down");
        let restore = kernel.mark();
        owner.restore_quiesced_taps().await.expect("the confirmed TAP comes up");
        let raised = kernel
            .trace_since(restore)
            .into_iter()
            .filter(|(call, _)| *call == AllocationCall::SetTapUp)
            .map(|(_, tap)| tap)
            .collect::<Vec<_>>();
        assert_eq!(raised, vec![tap_of(&confirmed)], "only the confirmed TAP is raised");
        let refusal =
            owner.activate(&lost).await.expect_err("an unconfirmed allocation is condemned");
        assert_refused_as_missing_record(&refusal, &lost);

        // Mixed pass. In `AllocationId` order the `Connect` set-down follows
        // the gone TAP's failure, and a TAP still to be confirmed follows both.
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let gone = scratch_plan("nd295-s51-mix-a", "t295-mxa", 2);
        let session = scratch_plan("nd295-s51-mix-b", "t295-mxb", 3);
        let confirmed = scratch_plan("nd295-s51-mix-c", "t295-mxc", 4);
        for plan in [&gone, &session, &confirmed] {
            activated(&owner, plan).await;
        }
        let gone_ifindex = kernel.node().taps[&gone.assignment().tap].ifindex;
        kernel.with_node(|node| {
            node.remove_part(AttachmentPart::Tap, &gone.assignment().tap, gone_ifindex);
        });
        kernel.fail_once_for(AllocationCall::SetTapDown, &session.assignment().tap);
        let mark = kernel.mark();
        let quiescence = owner
            .quiesce_managed_taps()
            .await
            .expect("every failure of one TAP is that TAP's outcome");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([gone.alloc().clone(), session.alloc().clone()]),
            "both failing TAPs are unconfirmed"
        );
        let gone_entry = &quiescence.unconfirmed[gone.alloc()];
        assert!(
            matches!(
                gone_entry,
                GuestNetworkError::Netlink {
                    operation: GuestNetworkOperation::TapSetDown,
                    source: NetlinkError::LinkAbsent { .. },
                }
            ),
            "the gone TAP keeps its own cause: {gone_entry:?}"
        );
        let session_entry = &quiescence.unconfirmed[session.alloc()];
        assert!(
            matches!(
                session_entry,
                GuestNetworkError::Netlink {
                    operation: GuestNetworkOperation::TapSetDown,
                    source: NetlinkError::Connect { .. },
                }
            ),
            "the session failure keeps its own cause: {session_entry:?}"
        );
        let trace = kernel.trace_since(mark);
        for plan in [&gone, &session, &confirmed] {
            assert!(
                trace.contains(&(AllocationCall::SetTapDown, tap_of(plan))),
                "the pass continues past both failures: {trace:?}"
            );
        }
        assert!(
            !kernel.node().taps[&confirmed.assignment().tap].up,
            "the TAP after the failures is confirmed down"
        );
        let restore = kernel.mark();
        owner.restore_quiesced_taps().await.expect("the confirmed TAP comes up");
        let raised = kernel
            .trace_since(restore)
            .into_iter()
            .filter(|(call, _)| *call == AllocationCall::SetTapUp)
            .map(|(_, tap)| tap)
            .collect::<Vec<_>>();
        assert_eq!(raised, vec![tap_of(&confirmed)], "only the confirmed TAP is raised");
        for plan in [&gone, &session] {
            let refusal =
                owner.activate(plan).await.expect_err("an unconfirmed allocation is condemned");
            assert_refused_as_missing_record(&refusal, plan);
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A repeat quiescence while latched sets down what a part-way restore
    /// raised (the owner half of the re-quiescence after a part-way restore,
    /// user-approved 2026-09-30). Both `Active` TAPs are confirmed down; a
    /// restore whose second set-up fails raises the first again (`Active`),
    /// keeps the second `QuiescedActive`, and keeps the latch; the repeat
    /// quiescence sets the raised TAP down and reads it back, gives the
    /// still-quiesced TAP no I/O, reports nothing unconfirmed, and keeps the
    /// latch. The re-quiesced allocation is `QuiescedActive` again: the next
    /// restore raises both, in `AllocationId` order, and clears the latch.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn a_repeat_quiescence_while_latched_sets_down_what_a_partial_restore_raised() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let first = scratch_plan("nd295-s51-rq-a", "t295-rqa", 2);
        let second = scratch_plan("nd295-s51-rq-b", "t295-rqb", 3);
        let waiting = scratch_plan("nd295-s51-rq-waiting", "t295-rqw", 4);
        activated(&owner, &first).await;
        activated(&owner, &second).await;
        provisioned(&owner, &waiting).await;

        let quiescence = owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert!(quiescence.unconfirmed.is_empty(), "both TAPs are confirmed down");
        let node = kernel.node();
        assert!(
            !node.taps[&first.assignment().tap].up && !node.taps[&second.assignment().tap].up,
            "the precondition: both Active TAPs are quiesced"
        );

        kernel.fail_once_for(AllocationCall::SetTapUp, &second.assignment().tap);
        let error =
            owner.restore_quiesced_taps().await.expect_err("the second set-up fails the restore");
        assert!(is_netlink_failure(&error, GuestNetworkOperation::TapSetUp), "{error:?}");
        let node = kernel.node();
        assert!(node.taps[&first.assignment().tap].up, "the restore raised the first TAP again");
        assert!(!node.taps[&second.assignment().tap].up, "the second stays quiesced");
        assert_eq!(
            owner.activate(&waiting).await.expect("a latched activation is not a failure"),
            TapActivation::QuiescenceLatched,
            "the failed restore keeps the latch"
        );

        let repeat = kernel.mark();
        let again = owner.quiesce_managed_taps().await.expect("a repeat while latched");
        let trace = kernel.trace_since(repeat);
        assert!(
            again.unconfirmed.is_empty(),
            "the raised TAP is confirmed down, so nothing is unconfirmed: {:?}",
            again.unconfirmed
        );
        let set_down = trace
            .iter()
            .position(|entry| *entry == (AllocationCall::SetTapDown, tap_of(&first)))
            .expect("the repeat sets the TAP the restore raised down again");
        assert!(
            trace[set_down..].contains(&(AllocationCall::ObserveTap, tap_of(&first))),
            "and reads it back: {trace:?}"
        );
        assert!(
            !trace.iter().any(|(_, tap)| *tap == tap_of(&second)),
            "the still-quiesced TAP gets no I/O: {trace:?}"
        );
        assert!(
            !trace.iter().any(|(_, tap)| *tap == tap_of(&waiting)),
            "the provisioned-down TAP is untouched: {trace:?}"
        );
        assert!(!kernel.node().taps[&first.assignment().tap].up, "the raised TAP is down again");
        assert_eq!(
            owner.activate(&waiting).await.expect("a latched activation is not a failure"),
            TapActivation::QuiescenceLatched,
            "the latch stays set"
        );

        let restore = kernel.mark();
        owner.restore_quiesced_taps().await.expect("both quiesced TAPs come up");
        let raised = kernel
            .trace_since(restore)
            .into_iter()
            .filter(|(call, _)| *call == AllocationCall::SetTapUp)
            .map(|(_, tap)| tap)
            .collect::<Vec<_>>();
        assert_eq!(
            raised,
            vec![tap_of(&first), tap_of(&second)],
            "the re-quiesced allocation is QuiescedActive again, raised in AllocationId order"
        );
        assert_eq!(
            owner.activate(&waiting).await.expect("activation after the restore"),
            TapActivation::Raised,
            "the latch is cleared"
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The bound applies to every adapter (pinned 2026-09-30): the call makes
    /// no synchronous blocking wait on the task that awaits it. A quiescence
    /// whose set-down never completes stays pending in that leaf while the
    /// thread keeps running, so a caller racing it against its bound on a
    /// `SimClock` sees the bound fire and drops the pending call. The latch
    /// was set before that first mutation, and the dropped call leaves the
    /// owner answering: an activation of a provisioned-down allocation
    /// reports `QuiescenceLatched` and writes nothing. The race runs on its
    /// own current-thread runtime; a blocking implementation stalls that
    /// thread, so the bound can never fire, and the wall-clock watchdog turns
    /// the stall into a failure instead of a hung suite.
    #[test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    fn a_quiescence_whose_set_down_never_completes_is_a_bound_miss_not_a_blocked_caller() {
        /// The quiescence bound the caller races; any positive value.
        const BOUND: std::time::Duration = std::time::Duration::from_secs(1);
        /// Scheduler turns the ticker waits for the pass to reach its
        /// set-down before it fires the bound regardless.
        const YIELD_BUDGET: usize = 1_000;
        /// Harness guard: how long the awaiting thread may stay stalled.
        const WATCHDOG: std::time::Duration = std::time::Duration::from_secs(30);

        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let racer = std::thread::Builder::new()
            .name("nd295-s51-quiesce-bound".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("a current-thread runtime for the awaiting task");
                runtime.block_on(async {
                    let kernel = FakeAttachmentKernel::healthy();
                    let owner = owner_over(&kernel);
                    let active = scratch_plan("nd295-s51-nb-active", "t295-nba", 2);
                    let waiting = scratch_plan("nd295-s51-nb-waiting", "t295-nbw", 3);
                    activated(&owner, &active).await;
                    provisioned(&owner, &waiting).await;
                    kernel.hang_always(AllocationCall::SetTapDown);
                    let mark = kernel.mark();

                    let clock = Arc::new(overdrive_sim::adapters::clock::SimClock::new());
                    let ticker = {
                        let clock = Arc::clone(&clock);
                        let kernel = Arc::clone(&kernel);
                        tokio::spawn(async move {
                            for _ in 0..YIELD_BUDGET {
                                if kernel
                                    .trace_since(mark)
                                    .iter()
                                    .any(|(call, _)| *call == AllocationCall::SetTapDown)
                                {
                                    break;
                                }
                                tokio::task::yield_now().await;
                            }
                            clock.tick(BOUND);
                        })
                    };
                    let raced = {
                        let mut quiescence = owner.quiesce_managed_taps();
                        let bound =
                            overdrive_core::traits::clock::Clock::sleep(clock.as_ref(), BOUND);
                        tokio::select! {
                            biased;
                            result = &mut quiescence => Some(format!("{result:?}")),
                            () = bound => None,
                        }
                    };
                    ticker.await.expect("the ticker fires the bound");
                    assert_eq!(
                        raced, None,
                        "a set-down that never completes keeps the call pending until its bound"
                    );
                    let mutations = kernel.mutations_since(mark);
                    assert_eq!(
                        mutations,
                        vec![KernelCall {
                            call: AllocationCall::SetTapDown,
                            tap: tap_of(&active),
                            wrote: false,
                        }],
                        "the pass is pending in the Active TAP's set-down and nothing else ran"
                    );
                    let answered = kernel.mark();
                    assert_eq!(
                        owner
                            .activate(&waiting)
                            .await
                            .expect("the owner answers after the pending call is dropped"),
                        TapActivation::QuiescenceLatched,
                        "the latch was set before the first mutation"
                    );
                    assert_eq!(
                        kernel.mutations_since(answered),
                        Vec::new(),
                        "the latched activation writes nothing"
                    );
                });
                done_tx.send(()).expect("the watchdog waits for the verdict");
                runtime.shutdown_background();
            })
            .expect("spawn the awaiting thread");

        match done_rx.recv_timeout(WATCHDOG) {
            Ok(()) => racer.join().expect("the awaiting thread ends after its verdict"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
                "the quiescence call blocked the thread that awaits it for {WATCHDOG:?}: its \
                 bound could never fire"
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match racer.join() {
                Ok(()) => panic!("the awaiting thread ended without a verdict"),
                Err(payload) => std::panic::resume_unwind(payload),
            },
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With no latch, restore does no I/O. With the latch set, restore raises
    /// each quiesced TAP in `AllocationId` order, reading each back before the
    /// next, never raises a provisioned-down or condemned TAP, and clears the
    /// latch last. A failure — a set-up error or a TAP that stays down — stops
    /// the pass, keeps the latch and the remainder quiesced; a retry raises
    /// only the remainder.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-51)"]
    async fn restore_raises_only_quiesced_active_taps_in_order_and_clears_the_latch_last() {
        struct RestoreFixture {
            kernel: Arc<FakeAttachmentKernel>,
            owner: HostSharedGuestNetworkOwner,
            a: GuestNetworkPlan,
            b: GuestNetworkPlan,
            c: GuestNetworkPlan,
            waiting: GuestNetworkPlan,
        }
        async fn restore_fixture() -> RestoreFixture {
            let kernel = FakeAttachmentKernel::healthy();
            let owner = owner_over(&kernel);
            let c = scratch_plan("nd295-s51-r-c", "t295-rc", 2);
            let a = scratch_plan("nd295-s51-r-a", "t295-ra", 3);
            let b = scratch_plan("nd295-s51-r-b", "t295-rb", 4);
            let waiting = scratch_plan("nd295-s51-r-d", "t295-rd", 5);
            for plan in [&c, &a, &b] {
                activated(&owner, plan).await;
            }
            provisioned(&owner, &waiting).await;
            RestoreFixture { kernel, owner, a, b, c, waiting }
        }
        fn raised_since(kernel: &FakeAttachmentKernel, mark: usize) -> Vec<Option<String>> {
            kernel
                .trace_since(mark)
                .into_iter()
                .filter(|(call, _)| *call == AllocationCall::SetTapUp)
                .map(|(_, tap)| tap)
                .collect()
        }

        let fixture = restore_fixture().await;
        let condemned = scratch_plan("nd295-s51-r-e", "t295-re", 6);
        activated(&fixture.owner, &condemned).await;
        let mark = fixture.kernel.mark();
        fixture.owner.restore_quiesced_taps().await.expect("no latch");
        assert_eq!(fixture.kernel.calls_since(mark), Vec::new(), "no latch: restore does no I/O");

        fixture
            .kernel
            .with_node(|node| node.tap_mut(&condemned.assignment().tap).admin_stuck = true);
        let quiescence =
            fixture.owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        assert_eq!(
            quiescence.unconfirmed.keys().cloned().collect::<Vec<_>>(),
            vec![condemned.alloc().clone()]
        );
        fixture
            .kernel
            .with_node(|node| node.tap_mut(&condemned.assignment().tap).admin_stuck = false);

        let mark = fixture.kernel.mark();
        fixture.owner.restore_quiesced_taps().await.expect("every quiesced TAP comes up");
        assert_eq!(
            raised_since(&fixture.kernel, mark),
            vec![tap_of(&fixture.a), tap_of(&fixture.b), tap_of(&fixture.c)],
            "quiesced TAPs are raised in AllocationId order; provisioned-down and condemned never"
        );
        let trace = fixture.kernel.trace_since(mark);
        for plan in [&fixture.a, &fixture.b, &fixture.c] {
            let raise = trace
                .iter()
                .position(|entry| *entry == (AllocationCall::SetTapUp, tap_of(plan)))
                .expect("raised");
            let next_raise = trace[raise + 1..]
                .iter()
                .position(|(call, _)| *call == AllocationCall::SetTapUp)
                .map_or(trace.len(), |offset| raise + 1 + offset);
            assert!(
                trace[raise..next_raise].contains(&(AllocationCall::ObserveTap, tap_of(plan))),
                "each raised TAP is read back before the next is raised: {trace:?}"
            );
        }
        let node = fixture.kernel.node();
        for plan in [&fixture.a, &fixture.b, &fixture.c] {
            assert!(node.taps[&plan.assignment().tap].up);
        }
        assert!(!node.taps[&fixture.waiting.assignment().tap].up);
        assert_eq!(
            fixture.owner.activate(&fixture.waiting).await.expect("the latch is cleared"),
            TapActivation::Raised
        );

        // A set-up failure mid-list.
        let fixture = restore_fixture().await;
        fixture.owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        fixture.kernel.fail_once_for(AllocationCall::SetTapUp, &fixture.b.assignment().tap);
        let mark = fixture.kernel.mark();
        let error =
            fixture.owner.restore_quiesced_taps().await.expect_err("the failure is returned");
        assert!(is_netlink_failure(&error, GuestNetworkOperation::TapSetUp), "{error:?}");
        assert_eq!(
            raised_since(&fixture.kernel, mark),
            vec![tap_of(&fixture.a), tap_of(&fixture.b)],
            "the pass stops at the first failure"
        );
        let node = fixture.kernel.node();
        assert!(node.taps[&fixture.a.assignment().tap].up);
        assert!(
            !node.taps[&fixture.b.assignment().tap].up
                && !node.taps[&fixture.c.assignment().tap].up
        );
        assert_eq!(
            fixture.owner.activate(&fixture.waiting).await.expect("the latch is kept"),
            TapActivation::QuiescenceLatched
        );
        let retry = fixture.kernel.mark();
        fixture.owner.restore_quiesced_taps().await.expect("the retry resumes the remainder");
        assert_eq!(
            raised_since(&fixture.kernel, retry),
            vec![tap_of(&fixture.b), tap_of(&fixture.c)]
        );
        assert_eq!(
            fixture.owner.activate(&fixture.waiting).await.expect("the latch is cleared"),
            TapActivation::Raised
        );

        // A TAP that stays down after set-up.
        let fixture = restore_fixture().await;
        fixture.owner.quiesce_managed_taps().await.expect("per-TAP outcomes are known");
        fixture
            .kernel
            .with_node(|node| node.tap_mut(&fixture.a.assignment().tap).admin_stuck = true);
        let mark = fixture.kernel.mark();
        let error = fixture.owner.restore_quiesced_taps().await.expect_err("the read-back fails");
        assert!(matches!(error, GuestNetworkError::PostconditionMismatch { .. }), "{error:?}");
        assert_eq!(raised_since(&fixture.kernel, mark), vec![tap_of(&fixture.a)]);
        assert_eq!(
            fixture.owner.activate(&fixture.waiting).await.expect("the latch is kept"),
            TapActivation::QuiescenceLatched
        );
        fixture
            .kernel
            .with_node(|node| node.tap_mut(&fixture.a.assignment().tap).admin_stuck = false);
        let retry = fixture.kernel.mark();
        fixture.owner.restore_quiesced_taps().await.expect("the retry raises every quiesced TAP");
        assert_eq!(
            raised_since(&fixture.kernel, retry),
            vec![tap_of(&fixture.a), tap_of(&fixture.b), tap_of(&fixture.c)]
        );
    }

    // ---- S-ND295-72: the host-side MAC invariant (E22 (i1), (i2)) --------

    /// A host-side MAC outside the reserved set: neither `GUEST_BRIDGE_MAC`
    /// nor a guest MAC (`02:00:` followed by an IPv4 address).
    const UNRESERVED_HOST_MAC: [u8; 6] = [0xfe, 0x95, 0xde, 0xad, 0x00, 0x01];

    /// A second unreserved host-side MAC.
    const OTHER_UNRESERVED_HOST_MAC: [u8; 6] = [0x62, 0xa6, 0x95, 0x00, 0x00, 0x02];

    /// A third unreserved host-side MAC, shaped like a udev persistent address.
    const THIRD_UNRESERVED_HOST_MAC: [u8; 6] = [0xb6, 0xf2, 0x51, 0xad, 0x47, 0xae];

    /// The ifindex of the next TAP the fake kernel creates on an
    /// `AuditFixture` owner (after 295 and 296).
    const NEXT_TAP_IFINDEX: u32 = FIRST_TAP_IFINDEX + 2;

    /// One way a TAP's host-side MAC breaks the D-295-R21 invariant: each
    /// class of the reserved set, and a read that carries no address.
    #[derive(Debug, Clone, Copy)]
    enum InvariantBreak {
        /// The guest MAC of another allocation the owner holds `Active`.
        HeldActiveGuestMac,
        /// The guest MAC of another allocation the owner holds
        /// `ProvisionedDown`.
        HeldProvisionedDownGuestMac,
        /// The guest MAC of the TAP's own allocation.
        OwnGuestMac,
        /// The node bridge's address, `GUEST_BRIDGE_MAC`.
        BridgeMac,
        /// The read-back carries no 6-byte address.
        NoAddress,
    }

    impl InvariantBreak {
        const ALL: [Self; 5] = [
            Self::HeldActiveGuestMac,
            Self::HeldProvisionedDownGuestMac,
            Self::OwnGuestMac,
            Self::BridgeMac,
            Self::NoAddress,
        ];

        /// The address the TAP of `own` is given, and the observed fact the
        /// owner reports for it, over an owner holding `held`'s two
        /// allocations.
        fn stimulus(
            self,
            held: &AuditFixture,
            own: &GuestNetworkPlan,
        ) -> (Option<[u8; 6]>, TapHostAddress) {
            let mac = match self {
                Self::HeldActiveGuestMac => held.active.assignment().mac,
                Self::HeldProvisionedDownGuestMac => held.down.assignment().mac,
                Self::OwnGuestMac => own.assignment().mac,
                Self::BridgeMac => overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                Self::NoAddress => return (None, TapHostAddress::Missing),
            };
            (Some(mac), TapHostAddress::Reserved(mac))
        }
    }

    impl AuditFixture {
        /// Both held attachments' parts, as the node holds them.
        fn held_parts(&self) -> [AttachmentParts; 2] {
            let node = self.kernel.node();
            [
                node.parts(&self.active.assignment().tap, ACTIVE_IFINDEX),
                node.parts(&self.down.assignment().tap, DOWN_IFINDEX),
            ]
        }
    }

    /// The invariant's refusal: `PostconditionMismatch { operation:
    /// TapObserve, expected: TapHostMac { ifindex, Unreserved }, observed:
    /// Some(TapHostMac { ifindex, <observed> }) }`.
    fn assert_invariant_broken(
        error: &GuestNetworkError,
        ifindex: u32,
        observed: TapHostAddress,
        row: impl std::fmt::Debug,
    ) {
        assert!(
            matches!(
                error,
                GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected,
                    observed: Some(got),
                } if *expected == host_mac_fact(ifindex, TapHostAddress::Unreserved)
                    && *got == host_mac_fact(ifindex, observed)
            ),
            "{row:?}: expected TapObserve over TapHostMac {{ {ifindex}, Unreserved }} vs \
             {observed:?}, got {error:?}"
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i1) and (i2) at provision (FD § "[REF] Driven port — TAP egress
    /// guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the host-side
    /// MAC invariant; provision's final down read-back, D12A step 7)). On an
    /// owner that holds one `Active` and one `ProvisionedDown` allocation, a
    /// new TAP whose host-side MAC becomes another held allocation's guest
    /// MAC (either phase), its own allocation's guest MAC, `GUEST_BRIDGE_MAC`,
    /// or no address refuses publication with the invariant's `TapObserve`
    /// mismatch over `TapHostMac`. Each break is written twice over: from
    /// creation, and after the egress step (step 6) has run, so only the
    /// step-7 read-back can see it. Provision never raises the TAP, rolls back
    /// every part it made, and leaves both held attachments byte-equal. The
    /// contrast: a TAP whose address is unreserved, written at either point,
    /// is published and activates.
    #[tokio::test]
    async fn a_reserved_or_missing_host_side_address_refuses_publication() {
        for written_after in [AllocationCall::CreateTap, AllocationCall::EgressLinkPinPresent] {
            for broken in InvariantBreak::ALL {
                let row = (written_after, broken);
                let fixture = AuditFixture::new("nd295-s72-provision").await;
                let plan = scratch_plan("nd295-s72-provision-new", "t295-in", 4);
                let tap = plan.assignment().tap.clone();
                let (mac, observed) = broken.stimulus(&fixture, &plan);
                let written = tap.clone();
                fixture.kernel.after_call(written_after, 1, move |node| {
                    node.tap_mut(&written).mac = mac;
                });
                let held = fixture.held_parts();
                let mark = fixture.kernel.mark();

                let error =
                    fixture.owner.provision(&plan).await.expect_err(
                        "a host-side MAC that breaks the invariant refuses publication",
                    );
                assert_invariant_broken(&error, NEXT_TAP_IFINDEX, observed, row);
                let trace = fixture.kernel.trace_since(mark);
                assert!(
                    !trace.iter().any(|(call, _)| *call == AllocationCall::SetTapUp),
                    "{row:?}: provision never raises the TAP"
                );
                if written_after == AllocationCall::EgressLinkPinPresent {
                    for step_six in
                        [AllocationCall::AttachFirstEgress, AllocationCall::PinEgressLink]
                    {
                        assert!(
                            trace.contains(&(step_six, Some(tap.clone()))),
                            "{row:?}: the break lands after step 6 ran, so only the step-7 \
                             read-back sees it: {trace:?}"
                        );
                    }
                }
                assert!(
                    fixture.kernel.node().parts(&tap, NEXT_TAP_IFINDEX).is_empty(),
                    "{row:?}: the rollback leaves no attachment part: {:?}",
                    fixture.kernel.node().parts(&tap, NEXT_TAP_IFINDEX)
                );
                assert_eq!(
                    fixture.held_parts(),
                    held,
                    "{row:?}: the held attachments are untouched"
                );
                assert_unpublished(&fixture.owner, || fixture.kernel.mark(), &plan).await;
            }
        }

        for written_after in [AllocationCall::CreateTap, AllocationCall::EgressLinkPinPresent] {
            let fixture = AuditFixture::new("nd295-s72-provision-unreserved").await;
            let plan = scratch_plan("nd295-s72-provision-unreserved-new", "t295-iu", 4);
            let written = plan.assignment().tap.clone();
            fixture.kernel.after_call(written_after, 1, move |node| {
                node.tap_mut(&written).mac = Some(UNRESERVED_HOST_MAC);
            });
            fixture.owner.provision(&plan).await.unwrap_or_else(|error| {
                panic!("{written_after:?}: an unreserved host-side MAC is published, got {error:?}")
            });
            assert_eq!(
                fixture.owner.activate(&plan).await.expect("the published allocation activates"),
                TapActivation::Raised,
                "{written_after:?}"
            );
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i1) at `activate`: on an owner that holds one `Active` and one
    /// `ProvisionedDown` allocation, a provisioned-down TAP whose host-side
    /// MAC became another held allocation's guest MAC (either phase), its own
    /// guest MAC, `GUEST_BRIDGE_MAC`, or no address is refused with the
    /// invariant's `TapObserve` mismatch before any mutation, and the node is
    /// unchanged. With the address restored, the attachment still activates.
    /// The contrast (E22 (i2)): a TAP moved to an unreserved address
    /// activates.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-72)"]
    async fn a_reserved_or_missing_host_side_address_refuses_activation_before_any_change() {
        for broken in InvariantBreak::ALL {
            let fixture = AuditFixture::new("nd295-s72-activate").await;
            let plan = scratch_plan("nd295-s72-activate-new", "t295-iv", 4);
            provisioned(&fixture.owner, &plan).await;
            let provisioned_node = fixture.kernel.node();
            let (mac, observed) = broken.stimulus(&fixture, &plan);
            fixture.kernel.with_node(|node| node.tap_mut(&plan.assignment().tap).mac = mac);
            let broken_node = fixture.kernel.node();
            let mark = fixture.kernel.mark();

            let error = fixture
                .owner
                .activate(&plan)
                .await
                .expect_err("a host-side MAC that breaks the invariant refuses activation");
            assert_invariant_broken(&error, NEXT_TAP_IFINDEX, observed, broken);
            assert_eq!(
                fixture.kernel.mutations_since(mark),
                Vec::new(),
                "{broken:?}: the refusal writes nothing"
            );
            assert_eq!(fixture.kernel.node(), broken_node, "{broken:?}: the node is unchanged");

            fixture.kernel.with_node(|node| *node = provisioned_node.clone());
            assert_eq!(
                fixture.owner.activate(&plan).await.expect("the attachment stayed activatable"),
                TapActivation::Raised,
                "{broken:?}"
            );
        }

        let fixture = AuditFixture::new("nd295-s72-activate-unreserved").await;
        let plan = scratch_plan("nd295-s72-activate-unreserved-new", "t295-iw", 4);
        provisioned(&fixture.owner, &plan).await;
        fixture
            .kernel
            .with_node(|node| node.tap_mut(&plan.assignment().tap).mac = Some(UNRESERVED_HOST_MAC));
        assert_eq!(
            fixture.owner.activate(&plan).await.expect("an unreserved host-side MAC activates"),
            TapActivation::Raised
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i1) in the audit: on a healthy node holding one `Active` and one
    /// `ProvisionedDown` allocation, a TAP that holds the other held
    /// allocation's guest MAC (either phase), its own guest MAC,
    /// `GUEST_BRIDGE_MAC`, or no address is that allocation's damage alone,
    /// named with the invariant's `TapObserve` mismatch over `TapHostMac`.
    /// The other allocation is not named, and the audit writes nothing.
    #[tokio::test]
    async fn a_reserved_or_missing_host_side_address_is_that_allocations_audit_damage() {
        for broken in InvariantBreak::ALL {
            let fixture = AuditFixture::new("nd295-s72-audit").await;
            // The active allocation's guest MAC is taken by the provisioned-down
            // TAP; every other break is set on the active TAP.
            let (target, ifindex) = match broken {
                InvariantBreak::HeldActiveGuestMac => (fixture.down.clone(), DOWN_IFINDEX),
                _ => (fixture.active.clone(), ACTIVE_IFINDEX),
            };
            let (mac, observed) = broken.stimulus(&fixture, &target);
            fixture.kernel.with_node(|node| node.tap_mut(&target.assignment().tap).mac = mac);
            let broken_node = fixture.kernel.node();
            let mark = fixture.kernel.mark();

            let audit = fixture.audit().await.unwrap_or_else(|failure| {
                panic!("{broken:?}: TAP damage leaves the node healthy, got {failure:?}")
            });
            assert_eq!(
                audit.damaged.keys().cloned().collect::<Vec<_>>(),
                vec![target.alloc().clone()],
                "{broken:?}: exactly the allocation whose TAP breaks the invariant is named"
            );
            assert_invariant_broken(&audit.damaged[target.alloc()], ifindex, observed, broken);
            assert_eq!(
                fixture.kernel.mutations_since(mark),
                Vec::new(),
                "{broken:?}: the audit writes nothing"
            );
            assert_eq!(fixture.kernel.node(), broken_node, "{broken:?}: the node is unchanged");
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i2) in the audit: an unreserved host-side MAC is not damage, and
    /// nothing is recorded. The first pass audits the addresses provision
    /// left. Each later pass first moves both TAPs to unreserved addresses
    /// they did not carry at the previous audit (the provisioned-down TAP
    /// back to its provisioned address in the last pass), then audits. No
    /// pass names either allocation, each pass reads every TAP it judges, and
    /// no audit writes.
    #[tokio::test]
    async fn an_unreserved_host_side_address_is_not_audit_damage_whenever_it_changes() {
        let fixture = AuditFixture::new("nd295-s72-unreserved").await;
        let active_tap = fixture.active.assignment().tap.clone();
        let down_tap = fixture.down.assignment().tap.clone();
        for (pass, active_mac, down_mac) in [
            (0, host_mac(ACTIVE_IFINDEX), host_mac(DOWN_IFINDEX)),
            (1, UNRESERVED_HOST_MAC, OTHER_UNRESERVED_HOST_MAC),
            (2, THIRD_UNRESERVED_HOST_MAC, host_mac(DOWN_IFINDEX)),
        ] {
            fixture.kernel.with_node(|node| {
                node.tap_mut(&active_tap).mac = Some(active_mac);
                node.tap_mut(&down_tap).mac = Some(down_mac);
            });
            let changed = fixture.kernel.node();
            let mark = fixture.kernel.mark();
            let audit = fixture.audit().await.expect("a healthy node");
            assert!(
                audit.damaged.is_empty(),
                "pass {pass}: an unreserved address is not damage: {:?}",
                audit.damaged
            );
            let reads = fixture.per_allocation_reads_since(mark);
            for tap in fixture.managed_taps() {
                assert!(
                    reads.contains(&(AllocationCall::ObserveTap, tap.clone())),
                    "pass {pass}: the audit reads the TAP it judges ({tap:?}): {reads:?}"
                );
            }
            assert_eq!(fixture.kernel.mutations_since(mark), Vec::new(), "pass {pass}");
            assert_eq!(fixture.kernel.node(), changed, "pass {pass}");
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i2), the reserved set's `Condemned` exclusion: once the audit has
    /// named, and so condemned, an allocation, a TAP that holds that
    /// allocation's guest MAC is not damage. The later audit still reads the
    /// holder's TAP and names nothing.
    #[tokio::test]
    #[ignore = "pending DELIVER step 06-04 (S-ND295-72)"]
    async fn a_tap_holding_a_condemned_guests_address_is_not_audit_damage() {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let holder = scratch_plan("nd295-s72-condemned-holder", "t295-ih", 2);
        let condemned = scratch_plan("nd295-s72-condemned", "t295-ix", 3);
        activated(&owner, &holder).await;
        activated(&owner, &condemned).await;
        kernel.with_node(|node| {
            AllocationDamage::IngressDetached.inject(node, &condemned, FIRST_TAP_IFINDEX + 1);
        });
        let first = owner.audit_shared().await.expect("a healthy node");
        assert_eq!(
            first.damaged.keys().cloned().collect::<Vec<_>>(),
            vec![condemned.alloc().clone()],
            "the audit condemns the damaged allocation"
        );
        kernel.with_node(|node| {
            node.tap_mut(&holder.assignment().tap).mac = Some(condemned.assignment().mac);
        });
        let mark = kernel.mark();
        let second = owner.audit_shared().await.expect("a healthy node");
        assert!(
            second.damaged.is_empty(),
            "a condemned allocation's guest MAC is not reserved: {:?}",
            second.damaged
        );
        assert!(
            kernel.trace_since(mark).contains(&(AllocationCall::ObserveTap, tap_of(&holder))),
            "the audit reads the TAP that holds the condemned guest's MAC"
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (i1), the ordering case: a TAP that took a guest MAC before that
    /// guest's allocation was held passes every audit until the allocation is
    /// provisioned. The first audit after that names the TAP's allocation
    /// alone, with `Reserved(<the guest's MAC>)`; the newly provisioned
    /// allocation is not named.
    #[tokio::test]
    async fn a_guest_address_a_tap_took_early_is_damage_from_the_first_audit_after_that_guest_is_held()
     {
        let kernel = FakeAttachmentKernel::healthy();
        let owner = owner_over(&kernel);
        let holder = scratch_plan("nd295-s72-early-holder", "t295-ie", 2);
        let later = scratch_plan("nd295-s72-early-later", "t295-il", 3);
        activated(&owner, &holder).await;
        kernel.with_node(|node| {
            node.tap_mut(&holder.assignment().tap).mac = Some(later.assignment().mac);
        });
        for pass in 0..2 {
            let mark = kernel.mark();
            let audit = owner.audit_shared().await.expect("a healthy node");
            assert!(
                audit.damaged.is_empty(),
                "pass {pass}: a guest MAC no held allocation carries is not reserved: {:?}",
                audit.damaged
            );
            assert!(
                kernel.trace_since(mark).contains(&(AllocationCall::ObserveTap, tap_of(&holder))),
                "pass {pass}: the audit reads the holder's TAP"
            );
        }

        provisioned(&owner, &later).await;
        let audit = owner.audit_shared().await.expect("a healthy node");
        assert_eq!(
            audit.damaged.keys().cloned().collect::<Vec<_>>(),
            vec![holder.alloc().clone()],
            "the first audit after the guest is held names the TAP holding its MAC, alone"
        );
        assert_invariant_broken(
            &audit.damaged[holder.alloc()],
            FIRST_TAP_IFINDEX,
            TapHostAddress::Reserved(later.assignment().mac),
            "ordering",
        );
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
    //! returns the byte-equal plan; a Retiring replay is refused as retiring;
    //! both states count against the fixed cap until `release`; `release`
    //! alone frees the address; `observe` and `snapshot` are read-only.
    //!
    //! Leases are always produced by the pool's own `assign` / `retire` /
    //! `release`; no body writes the pool's private state.

    use super::*;
    use overdrive_core::dataplane::GUEST_BRIDGE_MAC;
    use overdrive_core::guest_network::{GuestAttachmentOccupancy, MAX_GUEST_NETWORK_ATTACHMENTS};
    use overdrive_core::traits::GuestAttachmentLease;
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

    fn pool() -> GuestAddressPool {
        GuestAddressPool::new(
            "100.95.0.0/16".parse().expect("node prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        )
    }

    fn alloc(key: u16) -> AllocationId {
        AllocationId::new(&format!("nd295-{key:04x}")).expect("generated allocation id")
    }

    /// The complete plan the pool hands out for `host` (the offset inside
    /// `100.95.0.0/16`): today's pool constants plus the address-derived TAP
    /// name and guest MAC.
    fn expected_plan(alloc: &AllocationId, host: u32) -> GuestNetworkPlan {
        let octets = host.to_be_bytes();
        GuestNetworkPlan {
            alloc: alloc.clone(),
            bridge: "ovd-gbr0".to_owned(),
            node_prefix: "100.95.0.0/16".parse().expect("node prefix"),
            assignment: GuestNetworkAssignment {
                address: Ipv4Addr::new(100, 95, octets[2], octets[3]),
                tap: format!("ovd-tp-{host:04x}"),
                mac: [0x02, 0x00, 100, 95, octets[2], octets[3]],
                gateway: Ipv4Addr::new(100, 95, 0, 1),
                prefix: 16,
                dns: Ipv4Addr::new(100, 95, 0, 1),
            },
        }
    }

    fn host_of(plan: &GuestNetworkPlan) -> u32 {
        u32::from(plan.assignment().address) - u32::from(Ipv4Addr::new(100, 95, 0, 0))
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

    /// The independent lease model: each held allocation's plan and lease.
    #[derive(Debug, Clone, Default)]
    struct LeaseModel {
        leases: BTreeMap<AllocationId, (GuestNetworkPlan, GuestAttachmentLease)>,
    }

    impl LeaseModel {
        /// Smallest free host offset: never the network (0), the gateway (1),
        /// or an address any held lease (Admitted or Retiring) occupies.
        fn smallest_free_host(&self) -> u32 {
            let used = self.leases.values().map(|(plan, _)| host_of(plan)).collect::<BTreeSet<_>>();
            (2_u32..0xffff).find(|host| !used.contains(host)).expect("model below /16 capacity")
        }

        /// Apply `op` by the lease-state contract alone (never by reading the
        /// pool), returning the outcome the pool must report.
        fn apply(&mut self, op: LeaseOp) -> ModelOutcome {
            match op {
                LeaseOp::Assign(key) => {
                    let alloc = alloc(key);
                    match self.leases.get(&alloc) {
                        Some((plan, GuestAttachmentLease::Admitted)) => {
                            ModelOutcome::Assigned(plan.clone())
                        }
                        Some((_, GuestAttachmentLease::Retiring)) => ModelOutcome::Retiring(alloc),
                        None => {
                            let plan = expected_plan(&alloc, self.smallest_free_host());
                            self.leases
                                .insert(alloc, (plan.clone(), GuestAttachmentLease::Admitted));
                            ModelOutcome::Assigned(plan)
                        }
                    }
                }
                LeaseOp::Retire(key) => {
                    let alloc = alloc(key);
                    let transitioned = match self.leases.get_mut(&alloc) {
                        Some((_, lease @ GuestAttachmentLease::Admitted)) => {
                            *lease = GuestAttachmentLease::Retiring;
                            true
                        }
                        Some((_, GuestAttachmentLease::Retiring)) | None => false,
                    };
                    ModelOutcome::Retired(transitioned)
                }
                LeaseOp::Release(key) => {
                    self.leases.remove(&alloc(key));
                    ModelOutcome::Released
                }
            }
        }

        fn plans(&self) -> BTreeMap<AllocationId, GuestNetworkPlan> {
            self.leases.iter().map(|(alloc, (plan, _))| (alloc.clone(), plan.clone())).collect()
        }

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

    #[derive(Debug)]
    enum ModelOutcome {
        Assigned(GuestNetworkPlan),
        Retiring(AllocationId),
        Retired(bool),
        Released,
    }

    /// Drive `op` through the pool's own operations and compare its outcome
    /// with the model's.
    fn apply_and_compare(
        pool: &GuestAddressPool,
        model: &mut LeaseModel,
        op: LeaseOp,
    ) -> std::result::Result<(), TestCaseError> {
        match (op, model.apply(op)) {
            (LeaseOp::Assign(key), ModelOutcome::Assigned(expected)) => {
                let plan = pool.assign(alloc(key));
                prop_assert!(
                    matches!(&plan, Ok(plan) if *plan == expected),
                    "assign({key}) returned {plan:?}, the model expects {expected:?}"
                );
                if let Ok(plan) = plan {
                    prop_assert_ne!(plan.assignment().mac, GUEST_BRIDGE_MAC);
                }
            }
            (LeaseOp::Assign(key), ModelOutcome::Retiring(retiring)) => {
                let refusal = pool.assign(alloc(key));
                prop_assert!(
                    matches!(&refusal, Err(GuestNetworkError::LeaseRetiring { alloc }) if *alloc == retiring),
                    "a Retiring replay of {key} returned {refusal:?}"
                );
            }
            (LeaseOp::Retire(key), ModelOutcome::Retired(expected)) => {
                prop_assert_eq!(pool.retire(&alloc(key)), expected, "retire({}) transition", key);
            }
            (LeaseOp::Release(key), ModelOutcome::Released) => {
                pool.release(&alloc(key));
                // A release of an absent lease is a no-op.
                pool.release(&alloc(key));
            }
            (op, outcome) => unreachable!("model outcome {outcome:?} does not answer {op:?}"),
        }
        Ok(())
    }

    fn every_key() -> Vec<AllocationId> {
        (0..KEYS).map(alloc).collect()
    }

    proptest! {
        /// Outcome anchor: OUT-ND295-SHARED-SWITCH
        /// S-ND295-04 — Lease replay, retirement, and release change only the named allocation
        /// CONTRACT_SHAPE: bounded-change.
        ///
        /// After every generated `assign` / `retire` / `release`, the pool's
        /// returned outcome, its `snapshot` (both lease states), and its
        /// `observe` over every key equal the lease-state model: smallest-free
        /// selection, a byte-equal Admitted replay, `LeaseRetiring` for a
        /// Retiring replay, `retire` true only for Admitted -> Retiring, and
        /// `held = Admitted + Retiring` with `retiring` exact.
        #[test]
        #[ignore = "pending DELIVER step 06-03 (S-ND295-04)"]
        fn assignment_replay_release_and_reuse_match_the_smallest_free_model(
            operations in prop::collection::vec(lease_op(), 1..256),
        ) {
            let pool = pool();
            let mut model = LeaseModel::default();
            let keys = every_key();
            for op in operations {
                apply_and_compare(&pool, &mut model, op)?;
                prop_assert_eq!(pool.snapshot(), model.plans(), "snapshot after {:?}", op);
                prop_assert_eq!(pool.observe(&keys), model.observation(&keys), "observe after {:?}", op);
            }
        }

        /// Outcome anchor: OUT-ND295-SHARED-SWITCH
        /// S-ND295-04 — Lease replay, retirement, and release change only the named allocation
        /// CONTRACT_SHAPE: bounded-change.
        ///
        /// Retirement happens once and never returns a lease to Admitted: a
        /// repeated `retire` and a refused replay leave it Retiring, it still
        /// counts in `held` and `snapshot`, and only `release` frees its
        /// address, after which a fresh `assign` of the same allocation takes
        /// the normal smallest-free path.
        #[test]
        #[ignore = "pending DELIVER step 06-03 (S-ND295-04)"]
        fn retirement_is_monotonic_and_a_retiring_lease_still_counts(
            admitted in 1_u16..24,
            retire_mask in any::<u32>(),
            repeats in 1_usize..4,
        ) {
            let pool = pool();
            let held = (0..admitted).map(alloc).collect::<Vec<_>>();
            let plans = held
                .iter()
                .map(|alloc| pool.assign(alloc.clone()).expect("below-cap assignment"))
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
                let refusal = pool.assign(alloc.clone());
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
                let position = held.iter().position(|alloc| *alloc == released).expect("held");
                pool.release(&released);
                let after_release = pool.observe(&held);
                prop_assert_eq!(after_release.occupancy.held, u32::from(admitted) - 1);
                prop_assert!(!after_release.leases.contains_key(&released));
                prop_assert!(!pool.snapshot().contains_key(&released));
                let reassigned = pool.assign(released.clone()).expect("a released id takes the normal path");
                prop_assert_eq!(
                    reassigned.assignment().address,
                    plans[position].assignment().address,
                    "only release freed that address, and it is the smallest free one"
                );
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
        /// mix of Retiring among them, the next allocation is assigned the
        /// smallest free address; at 16,384 held (again with every mix) a new
        /// allocation is refused with `AdmissionCapReached { held, retiring,
        /// cap: 16_384 }` and nothing changes, while an Admitted replay still
        /// returns its plan and a Retiring replay is refused as retiring. A
        /// held count of 16,385 cannot be produced through the pool's own
        /// operations, because `assign` refuses at 16,384.
        #[test]
        #[ignore = "pending DELIVER step 06-03 (S-ND295-05A)"]
        fn admission_refuses_at_the_cap_over_held_leases_for_every_retiring_mix(
            retiring in prop_oneof![Just(0_usize), Just(CAP - 1), 1_usize..CAP - 1],
            offset in 0_usize..CAP - 1,
            retire_the_last_admitted in any::<bool>(),
        ) {
            let ids = &*CAP_IDS;
            let pool = pool();
            let below_cap = CAP - 1;
            for id in &ids[..below_cap] {
                pool.assign(id.clone()).expect("below-cap assignment");
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

            let last = pool.assign(ids[below_cap].clone()).expect("16,383 held admits one more");
            prop_assert_eq!(last, expected_plan(&ids[below_cap], u32::try_from(CAP + 1).expect("cap host")));
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
            let refusal = pool.assign(ids[CAP].clone());
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
                let replay = pool.assign(ids[index].clone()).expect("an Admitted replay at the cap");
                prop_assert_eq!(Some(&replay), before.get(&ids[index]), "byte-equal replay");
            }
            let retiring_replay = retired
                .iter()
                .next()
                .copied()
                .or_else(|| retire_the_last_admitted.then_some(below_cap));
            if let Some(index) = retiring_replay {
                let replay = pool.assign(ids[index].clone());
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
        /// holds one (requested allocations without a lease and every
        /// unrequested allocation are omitted), and it changes nothing:
        /// `snapshot` is equal before and after and a second read is equal.
        #[test]
        #[ignore = "pending DELIVER step 07-03 (S-ND295-05B)"]
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
        let pool = GuestAddressPool::new(
            "100.95.0.0/30".parse().expect("small drift prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        );
        let first = pool
            .assign(AllocationId::new("nd295-below-cap-first").expect("allocation id"))
            .expect("the sole non-reserved address is initially free");
        let before = pool.snapshot();
        assert!(before.len() < 16_384);
        assert_eq!(first.assignment().address, Ipv4Addr::new(100, 95, 0, 2));

        let error = pool
            .assign(AllocationId::new("nd295-below-cap-overflow").expect("allocation id"))
            .expect_err("the small prefix is exhausted below the fixed admission cap");
        assert!(matches!(error, GuestNetworkError::PoolExhausted { held: 1, capacity: 1 }));
        assert_eq!(pool.snapshot(), before, "typed exhaustion does not mutate or reuse state");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn assignment_uses_the_exact_prefix_bridge_gateway_dns_and_boundary_addresses() {
        let pool = pool();
        let first = pool
            .assign(AllocationId::new("nd295-first").expect("allocation id"))
            .expect("first usable address");
        assert_eq!(first.bridge(), "ovd-gbr0");
        assert_eq!(first.node_prefix(), "100.95.0.0/16".parse().expect("prefix"));
        assert_eq!(first.assignment().address, Ipv4Addr::new(100, 95, 0, 2));
        assert_eq!(first.assignment().gateway, Ipv4Addr::new(100, 95, 0, 1));
        assert_eq!(first.assignment().dns, Ipv4Addr::new(100, 95, 0, 1));
        assert_eq!(first.assignment().prefix, 16);
        assert_eq!(first.assignment().tap, "ovd-tp-0002");

        let shared = pool.clone();
        assert_eq!(shared.snapshot(), pool.snapshot(), "held ownership is Arc-shared");
    }
}

/// S-ND295-72, the owner's real-kernel lane (E22 (d) and (e); feature delta
/// § "[REF] Managed-link identity independent of host link configuration
/// (fresh-host RCA) — pinned 2026-09-26; user rulings of 2026-09-28" (the
/// refusal names its cause; *Earned Trust*)). Lima root, `integration-tests`;
/// (e) also runs on the metal host as it is provisioned.
///
/// The production host owner (`HostSharedGuestNetworkOwner::new()`, its real
/// scratch and allocation I/O) converges the node's real shared state; faults
/// enter only as real out-of-band kernel mutations (`ip link set`) of links
/// the owner created. No body installs, reads, or requires a host
/// link-configuration file. A `NodeSharedStateSweep` removes every node-global
/// object the bodies can create — the bridge, the two TAPs, the bridge guard
/// table, and the owner's bpffs pins — before and after each body, so a panic
/// leaves nothing behind. These bodies mutate node-global names; the module is
/// in the `host-kernel-shared` nextest group (`.config/nextest.toml`).
#[cfg(all(test, feature = "integration-tests"))]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "Tier-3 source-local bodies: evidence goes to stderr and fixture preconditions panic"
)]
mod shared_owner_link_address_kernel {
    use std::process::Command;

    use super::*;

    const BRIDGE: &str = "ovd-gbr0";

    /// Production-shaped TAP names (`ovd-tp-<4hex>`, derived from the
    /// address) for two addresses no other fixture uses.
    const TAP_ADDRESS: Ipv4Addr = Ipv4Addr::new(100, 95, 242, 229);
    const TAP: &str = "ovd-tp-f2e5";
    const SECOND_TAP_ADDRESS: Ipv4Addr = Ipv4Addr::new(100, 95, 242, 230);
    const SECOND_TAP: &str = "ovd-tp-f2e6";

    /// An address written out of band. It is unreserved: neither
    /// `GUEST_BRIDGE_MAC` nor a guest MAC (`02:00:` followed by an IPv4
    /// address).
    const FOREIGN_MAC: &str = "02:95:72:00:00:0d";
    const FOREIGN_MAC_BYTES: [u8; 6] = [0x02, 0x95, 0x72, 0x00, 0x00, 0x0d];

    #[allow(unsafe_code, reason = "reading the effective uid is the test's root precondition")]
    fn require_root(test: &str) {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        assert_eq!(euid, 0, "{test} mutates node-global links and must run as root");
    }

    /// Run one diagnostic host command; its outcome is recorded, and
    /// `must_succeed` makes a failure fatal.
    fn run(program: &str, args: &[&str], must_succeed: bool) {
        let output = Command::new(program)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("spawning `{program} {args:?}`: {error}"));
        eprintln!(
            "[S-ND295-72] {program} {args:?} -> {:?} stderr={:?}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        if must_succeed {
            assert!(output.status.success(), "`{program} {args:?}` must succeed");
        }
    }

    /// Removes the node-global objects these bodies can create, when created
    /// and when dropped. A step that finds nothing is the clean outcome.
    struct NodeSharedStateSweep;

    impl NodeSharedStateSweep {
        fn fresh() -> Self {
            sweep();
            Self
        }
    }

    impl Drop for NodeSharedStateSweep {
        fn drop(&mut self) {
            sweep();
        }
    }

    fn sweep() {
        run("ip", &["link", "del", TAP], false);
        run("ip", &["link", "del", SECOND_TAP], false);
        run("ip", &["link", "del", BRIDGE], false);
        run("nft", &["delete", "table", "bridge", "overdrive-mtls"], false);
        run("rm", &["-rf", "/sys/fs/bpf/overdrive/mtls-endpoints"], false);
    }

    fn bridge_gateway() -> Ipv4Net {
        Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 1), 16)
    }

    fn live_identity(name: &str) -> overdrive_netlink::ObservedLinkIdentity {
        let owned = name.to_owned();
        overdrive_netlink::block_on_host_netlink(|| async move {
            overdrive_netlink::Client::new()?.observe_link_identity(&owned).await
        })
        .unwrap_or_else(|error| panic!("reading the identity of {name}: {error}"))
        .unwrap_or_else(|| panic!("{name} exists"))
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// E23 — A refused OS thread is a typed failure, never a panic.
    /// CONTRACT_SHAPE: unbounded-preservation.
    ///
    /// User decision 2 of 2026-09-30: `block_on_host_netlink`, whose host worker
    /// thread the OS refuses under a `pids.max` cap, returns
    /// `NetlinkError::Connect` and the process continues — it never aborts. The
    /// staged bridge uses `Scope::spawn`, which panics on a refused thread
    /// (Changed Assumption 40); DELIVER step 06-02 replaces it with
    /// `Builder::spawn_scoped`. RED-against-the-no-panic-baseline: before 06-02
    /// the refused thread panics/aborts and the body never reaches its assertion.
    #[cfg(target_os = "linux")]
    #[test]
    fn block_on_host_netlink_returns_connect_when_a_thread_is_refused() {
        require_root("block_on_host_netlink_returns_connect_when_a_thread_is_refused");
        let _refused = overdrive_testing::pids_max::refuse_thread_creation()
            .expect("the Lima substrate delegates the pids controller to the cgroup root");
        let result = overdrive_netlink::block_on_host_netlink(|| async move {
            overdrive_netlink::Client::new()?.observe_link_identity("lo").await
        });
        // Reaching here proves no abort/panic (user decision 2 of 2026-09-30). A
        // refused worker thread is `NetlinkError::Connect`; if a thread was
        // available the loopback identity is observed — still never a panic.
        match result {
            Ok(_) => {}
            Err(error) => assert!(
                matches!(error, overdrive_netlink::NetlinkError::Connect { .. }),
                "a refused host-netlink worker thread is NetlinkError::Connect, got {error:?}",
            ),
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// E23 — A refused OS thread is a typed failure, never a panic.
    /// CONTRACT_SHAPE: unbounded-preservation.
    ///
    /// User decision 2 of 2026-09-30: the host owner's `quiesce_managed_taps`
    /// under a `pids.max` cap returns a typed result and never aborts — the one
    /// `Active` TAP whose set-down could not get a netlink session is that TAP's
    /// `unconfirmed` entry carrying `NetlinkError::Connect` (DR-08 (b)-A), and
    /// the host still returns `Ok`, never `Err`. The per-TAP Connect→unconfirmed
    /// mapping over many TAPs is proven in the fake-kernel lane (S-ND295-51);
    /// this body proves the REAL quiescence composes a refused thread without
    /// aborting — which requires at least one managed `Active` TAP, so the pass
    /// has a per-TAP set-down to run. `converge_shared` alone registers no
    /// allocation, so the TAP is provisioned and activated before the cap;
    /// otherwise the pass quiesces an empty set and the refused-thread stimulus
    /// never fires.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "pending DELIVER step 06-04 (E23)"]
    async fn quiesce_managed_taps_never_aborts_when_a_thread_is_refused() {
        require_root("quiesce_managed_taps_never_aborts_when_a_thread_is_refused");
        let _sweep = NodeSharedStateSweep::fresh();
        let owner = HostSharedGuestNetworkOwner::new();
        owner.converge_shared().await.expect("the production owner converges a clean node");
        // One managed `Active` TAP, so the quiescence pass has a real per-TAP
        // set-down to run and the refused-thread stimulus bites.
        let plan = kernel_plan("nd295-e23-quiesce", TAP_ADDRESS, TAP);
        owner.provision(&plan).await.expect("the production owner provisions the managed TAP down");
        assert_eq!(
            owner.activate(&plan).await.expect("the production owner raises the managed TAP"),
            TapActivation::Raised,
        );
        let _refused = overdrive_testing::pids_max::refuse_thread_creation()
            .expect("the Lima substrate delegates the pids controller to the cgroup root");
        let quiescence = owner.quiesce_managed_taps().await;
        // Reaching here proves the owner's quiescence did not abort/panic under
        // thread refusal (user decision 2 of 2026-09-30). Per DR-08 (b)-A the
        // host returns no `Err`: the one `Active` TAP whose set-down could not
        // get a netlink worker thread is that allocation's `unconfirmed` entry
        // carrying `NetlinkError::Connect`, and the pass still returns `Ok`.
        let quiescence = quiescence.expect("the host returns per-TAP outcomes, never Err");
        for (alloc, error) in &quiescence.unconfirmed {
            assert!(
                matches!(
                    error,
                    GuestNetworkError::Netlink { source: NetlinkError::Connect { .. }, .. }
                ),
                "{alloc}'s unconfirmed entry is a Connect session failure, got {error:?}",
            );
        }
        assert!(
            quiescence.unconfirmed.contains_key(plan.alloc()),
            "the managed TAP whose set-down thread was refused is unconfirmed, got {:?}",
            quiescence.unconfirmed,
        );
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (d): a bridge identity mismatch reports the observed address and up
    /// state, not two equal facts. After the production owner converges the
    /// node, an out-of-band address write on `ovd-gbr0` makes `audit_shared`
    /// refuse with component `Bridge` and `PostconditionMismatch { operation:
    /// BridgeObserve }` whose expected fact is `Bridge { GUEST_BRIDGE_MAC, up,
    /// gateway 100.95.0.1/16 }` and whose observed fact is `Bridge` carrying the
    /// written address, the link's ifindex, and its up state. On a second
    /// fresh node, an out-of-band `down` is reported the same way with
    /// `up: false`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_bridge_identity_mismatch_names_the_observed_address_and_up_state() {
        require_root("a_bridge_identity_mismatch_names_the_observed_address_and_up_state");
        for (row, mutation, observed_mac, observed_up) in [
            (
                "changed address",
                vec!["link", "set", "dev", BRIDGE, "address", FOREIGN_MAC],
                FOREIGN_MAC_BYTES,
                true,
            ),
            (
                "set down",
                vec!["link", "set", "dev", BRIDGE, "down"],
                overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                false,
            ),
        ] {
            // Each row starts from a fresh node and a fresh owner: repeated
            // convergence on one owner is the runtime repair's (09-01), not
            // this row's subject.
            let _sweep = NodeSharedStateSweep::fresh();
            let owner = HostSharedGuestNetworkOwner::new();
            owner.converge_shared().await.expect("the production owner converges a clean node");
            let ifindex = live_identity(BRIDGE).ifindex;
            run("ip", &mutation, true);
            let refused =
                owner.audit_shared().await.expect_err("the damaged bridge fails the audit");
            eprintln!("[S-ND295-72 (d)][{row}] {refused:?}");
            assert_eq!(
                refused.component,
                SharedGuestNetworkComponent::Bridge,
                "[{row}] the bridge component fails"
            );
            let GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected,
                observed,
            } = refused.source
            else {
                panic!("[{row}] a bridge identity mismatch, got {:?}", refused.source);
            };
            assert!(
                matches!(
                    &expected,
                    GuestNetworkFact::Bridge { name, link_kind: GuestLinkKind::Bridge, mac, up: true, gateway, .. }
                        if name == BRIDGE
                            && *mac == overdrive_core::dataplane::GUEST_BRIDGE_MAC
                            && *gateway == Some(bridge_gateway())
                ),
                "[{row}] the expected fact is the fixed bridge identity, got {expected:?}"
            );
            assert_eq!(
                observed,
                Some(GuestNetworkFact::Bridge {
                    name: BRIDGE.to_owned(),
                    ifindex: Some(ifindex),
                    link_kind: GuestLinkKind::Bridge,
                    mac: observed_mac,
                    up: observed_up,
                    gateway: Some(bridge_gateway()),
                }),
                "[{row}] the observed fact carries the read-back address and up state"
            );
            assert_ne!(
                Some(expected),
                observed,
                "[{row}] the refusal never reports two equal facts"
            );
        }
    }

    /// A guest-network plan for `address` on the node bridge, with the
    /// production TAP name and guest MAC derivation.
    fn kernel_plan(alloc: &str, address: Ipv4Addr, tap: &str) -> GuestNetworkPlan {
        let [a, b, c, d] = address.octets();
        GuestNetworkPlan {
            alloc: AllocationId::new(alloc).expect("allocation id"),
            bridge: BRIDGE.to_owned(),
            node_prefix: "100.95.0.0/16".parse().expect("node prefix"),
            assignment: GuestNetworkAssignment {
                address,
                tap: tap.to_owned(),
                mac: [0x02, 0x00, a, b, c, d],
                gateway: Ipv4Addr::new(100, 95, 0, 1),
                prefix: 16,
                dns: Ipv4Addr::new(100, 95, 0, 1),
            },
        }
    }

    /// Wait until systemd-udevd, where this substrate runs it, has finished
    /// with `tap`; on a substrate without it there is nothing to wait for.
    /// Production never waits (feature delta § *Earned Trust*: no result waits
    /// on udev); the test waits so that "whatever address udev left" is the
    /// address udev left.
    fn udev_settled(tap: &str) {
        // The E22 metal lane records the host's systemd version
        // (`systemctl --version` prints what `systemd --version` prints).
        let version = Command::new("systemctl").arg("--version").output();
        eprintln!(
            "[S-ND295-72 (e)] host systemd: {:?}",
            version.as_ref().map(|output| String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned())
        );
        let ping = Command::new("udevadm").args(["control", "--ping", "--timeout=2"]).output();
        let running = ping.as_ref().is_ok_and(|output| output.status.success());
        eprintln!(
            "[S-ND295-72 (e)] systemd-udevd running on this substrate: {running} ({:?})",
            ping.map(|output| output.status.code())
        );
        if running {
            run("udevadm", &["wait", "--timeout=10", &format!("/sys/class/net/{tap}")], true);
        }
    }

    fn damaged_by_tap_host_mac(cause: &GuestNetworkError, ifindex: u32) -> Option<TapHostAddress> {
        match cause {
            GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapObserve,
                expected:
                    GuestNetworkFact::TapHostMac {
                        ifindex: expected,
                        address: TapHostAddress::Unreserved,
                    },
                observed: Some(GuestNetworkFact::TapHostMac { ifindex: observed, address }),
            } if *expected == ifindex && *observed == ifindex => Some(*address),
            _ => None,
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-72 — A managed link is correct whatever the host's link configuration.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E22 (e): the production owner provisions two allocations. After
    /// systemd-udevd, where the substrate runs it, has initialized the first
    /// TAP, the audit reports no damage, whatever address udev left on it.
    /// An out-of-band write of an unreserved address on that TAP is still no
    /// damage. An out-of-band write of the second allocation's guest MAC is
    /// that TAP's `TapHostMac` damage alone (observed `Reserved(<mac>)`), and
    /// the second allocation and the node stay healthy. No host link
    /// configuration is installed or required.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote() {
        require_root(
            "a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote",
        );
        let _sweep = NodeSharedStateSweep::fresh();
        let owner = HostSharedGuestNetworkOwner::new();
        owner.converge_shared().await.expect("the production owner converges a clean node");
        let first = kernel_plan("nd295-s72-e-first", TAP_ADDRESS, TAP);
        let second = kernel_plan("nd295-s72-e-second", SECOND_TAP_ADDRESS, SECOND_TAP);
        owner.provision(&first).await.expect("the production owner provisions the first TAP down");
        owner
            .provision(&second)
            .await
            .expect("the production owner provisions the second TAP down");
        udev_settled(TAP);
        let identity = live_identity(TAP);
        eprintln!(
            "[S-ND295-72 (e)] {TAP} ifindex {} after udev: mac={:?}",
            identity.ifindex, identity.mac
        );

        let settled = owner.audit_shared().await.expect("the node is healthy after provision");
        assert!(
            settled.damaged.is_empty(),
            "whatever address the host's link manager left, no allocation is damaged: {:?}",
            settled.damaged
        );

        run("ip", &["link", "set", "dev", TAP, "address", FOREIGN_MAC], true);
        assert_eq!(live_identity(TAP).mac, Some(FOREIGN_MAC_BYTES), "the unreserved write landed");
        let unreserved = owner.audit_shared().await.expect("the node stays healthy");
        assert!(
            unreserved.damaged.is_empty(),
            "an unreserved address is not damage: {:?}",
            unreserved.damaged
        );

        let stolen = second.assignment().mac;
        let stolen_text =
            stolen.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":");
        run("ip", &["link", "set", "dev", TAP, "address", &stolen_text], true);
        let reserved = owner.audit_shared().await.expect("TAP damage leaves the node healthy");
        assert_eq!(
            reserved.damaged.keys().cloned().collect::<Vec<_>>(),
            vec![first.alloc().clone()],
            "the TAP holding the second allocation's guest MAC is the only damage"
        );
        assert_eq!(
            damaged_by_tap_host_mac(&reserved.damaged[first.alloc()], identity.ifindex),
            Some(TapHostAddress::Reserved(stolen)),
            "the damage names the reserved address, got {:?}",
            reserved.damaged[first.alloc()]
        );

        owner.teardown(&first).await.expect("the production owner tears the first TAP down");
        owner.teardown(&second).await.expect("the production owner tears the second TAP down");
    }
}
