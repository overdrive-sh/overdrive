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
use std::sync::{Arc, OnceLock};

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::GuestNetworkAssignment;
use overdrive_netlink::NetlinkError;

use overdrive_dataplane::guest_tcx::{
    GuestTcxAttachment, GuestTcxEndpoint, GuestTcxInventoryIdentity, GuestTcxLink, GuestTcxProgram,
};
pub use overdrive_dataplane::guest_tcx::{GuestTcxError, TcxAttachPoint};
use overdrive_netlink::nft::bridge::{
    BridgeGuardError, BridgeGuardMutationOutcome, BridgeGuardObservation, BridgeGuardSpec,
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
    /// Provision the named attachment to its full read-back postcondition.
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()>;
    /// Tear down the named attachment to its empty complement.
    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()>;
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
    /// Non-repairing complete registered-inventory audit.
    async fn audit_shared(&self) -> std::result::Result<(), SharedGuestNetworkAuditError>;
    /// Return only after every managed TAP is observed administratively down.
    async fn quiesce_managed_taps(&self) -> Result<()>;
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
}

/// Guest-network result alias.
pub type Result<T, E = GuestNetworkError> = std::result::Result<T, E>;

/// Private process-local lease owner. It is intentionally not a repository.
#[derive(Debug, Clone)]
#[allow(dead_code, reason = "private pool is exercised by owner and source-local tests")]
pub(crate) struct GuestAddressPool {
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
    pub(crate) fn new(
        node_prefix: Ipv4Net,
        bridge: String,
        gateway: Ipv4Addr,
        dns: Ipv4Addr,
    ) -> Self {
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
    inventory: parking_lot::Mutex<Option<GuestTcxInventoryIdentity>>,
    adopted: parking_lot::Mutex<Option<overdrive_dataplane::guest_tcx::GuestTcxAdoptedState>>,
    pending_link: parking_lot::Mutex<Option<overdrive_dataplane::guest_tcx::GuestTcxLink>>,
}

impl RealSharedGuestNetworkScratchIo {
    fn new() -> Self {
        Self {
            program: parking_lot::Mutex::new(None),
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
}

#[async_trait::async_trait]
impl SharedGuestNetworkScratchIo for RealSharedGuestNetworkScratchIo {
    async fn apply_netlink(
        &self,
        plan: &GuestNetworkScratchPlan,
        action: GuestNetworkScratchNetlinkAction,
    ) -> std::result::Result<(), NetlinkError> {
        match action {
            GuestNetworkScratchNetlinkAction::ConvergeBridge => {
                let client = overdrive_netlink::Client::new()?;
                client.ensure_bridge(&plan.bridge).await?;
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
            GuestNetworkScratchTcxAction::UnpinLink => self
                .adopted
                .lock()
                .as_mut()
                .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                    family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxLink,
                })?
                .unpin_link()
                .map(|link| {
                    *self.pending_link.lock() = link;
                }),
            GuestNetworkScratchTcxAction::DetachLink => {
                let pending = self.pending_link.lock().take();
                if let Some(link) = pending { link.detach() } else { Ok(()) }
            }
            GuestNetworkScratchTcxAction::UnpinCounterMap => self
                .adopted
                .lock()
                .as_mut()
                .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                    family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::CounterMap,
                })?
                .unpin_counter_map(),
            GuestNetworkScratchTcxAction::UnpinEndpointMap => self
                .adopted
                .lock()
                .as_mut()
                .ok_or_else(|| GuestTcxError::CaptureUnavailable {
                    family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::EndpointMap,
                })?
                .unpin_endpoint_map(),
        }
    }

    fn close_loader_handles(&self, _plan: &GuestNetworkScratchPlan) {
        self.program.lock().take();
    }

    fn release_adopted_handles(&self, _plan: &GuestNetworkScratchPlan) {
        self.adopted.lock().take();
    }

    async fn exercise(
        &self,
        plan: &GuestNetworkScratchPlan,
        _stage: GuestNetworkProbeStage,
    ) -> std::io::Result<bool> {
        let ifindex = Self::ifindex(plan)?;
        Ok(self
            .program
            .lock()
            .as_ref()
            .and_then(|program| program.read_endpoint(ifindex).ok())
            .flatten()
            .is_some())
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
            _ => 0,
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
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "D12A exact private RED observation")]
enum GuestNetworkAllocationBridgeObservation {
    Absent { name: String },
    Present { name: String, ifindex: u32, kind: GuestLinkKind },
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
}

impl HostGuestNetworkAllocationIo {
    fn new(
        tcx: Arc<parking_lot::Mutex<Option<HostGuestTcxState>>>,
        guard: BridgeGuardSpec,
    ) -> Self {
        Self { tcx, guard, pending_links: parking_lot::Mutex::new(BTreeMap::new()) }
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

    fn missing_object() -> GuestTcxError {
        GuestTcxError::ObjectMissing {
            object: overdrive_dataplane::guest_tcx::GuestTcxObject::Classifier,
        }
    }
}

#[async_trait::async_trait]
impl GuestNetworkAllocationIo for HostGuestNetworkAllocationIo {
    async fn create_tap(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
        overdrive_netlink::create_persistent_tap(
            &plan.assignment().tap,
            overdrive_core::vm::config::OVERDRIVE_VMM_UID,
        )
    }
    async fn attach_tap_to_bridge(
        &self,
        plan: &GuestNetworkPlan,
    ) -> std::result::Result<(), NetlinkError> {
        let client = overdrive_netlink::Client::new()?;
        client.set_link_master(&plan.assignment().tap, plan.bridge()).await
    }
    async fn set_tap_up(&self, plan: &GuestNetworkPlan) -> std::result::Result<(), NetlinkError> {
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
        if let Some(link) = self.pending_links.lock().remove(plan.alloc()) {
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
struct HostGuestNetworkAllocationState {
    ifindex: u32,
    program_id: u32,
}

/// Private host owner; its startup algorithm is independently executable from
/// the still-pending native adapter bindings.
#[allow(dead_code, reason = "D5 exact RED owner field is activated in DELIVER")]
pub(super) struct HostSharedGuestNetworkOwner {
    scratch_io: Arc<dyn SharedGuestNetworkScratchIo>,
    allocation_io: Arc<dyn GuestNetworkAllocationIo>,
    tcx: Arc<parking_lot::Mutex<Option<HostGuestTcxState>>>,
    allocations: parking_lot::Mutex<BTreeMap<AllocationId, HostGuestNetworkAllocationState>>,
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
            other => GuestNetworkError::Io {
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
            owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
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
            .map(|(_, state)| format!("ovd-tp-{:04x}", state.ifindex.saturating_sub(293)))
            .collect::<BTreeSet<_>>();
        if let Some(plan) = extra {
            members.insert(plan.assignment().tap.clone());
        }
        members
    }

    fn guard_postcondition(
        &self,
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

    async fn rollback_provision(
        &self,
        plan: &GuestNetworkPlan,
        ifindex: Option<u32>,
    ) -> Option<GuestNetworkError> {
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
        if let Some(ifindex) = ifindex {
            attempt!(
                self.allocation_io.remove_endpoint(plan, ifindex).map_err(
                    |source| Self::tcx_error(GuestNetworkOperation::EndpointDelete, source)
                )
            );
        }
        attempt!(
            self.allocation_io
                .detach_pending_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxDetach, source))
        );
        attempt!(
            self.allocation_io
                .detach_pinned_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxDetach, source))
        );
        attempt!(
            self.allocation_io
                .set_tap_down(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapSetDown, source))
        );
        attempt!(
            self.allocation_io
                .delete_tap(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapDelete, source))
        );
        attempt!(
            self.allocation_io.delete_guard_member(plan).map_err(|error| Self::guard_error(
                GuestNetworkOperation::GuardMemberDelete,
                error
            ))
        );
        first
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
            original_destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 255, 254), 8443),
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
        self.exercise(plan, GuestNetworkProbeStage::Classifier).await?;
        self.exercise(plan, GuestNetworkProbeStage::OriginalDestination).await?;
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
impl GuestNetworkProvisioner for HostGuestNetworkProvisioner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> Result<()> {
        if self.allocations.lock().contains_key(plan.alloc()) {
            return Ok(());
        }
        let mut ifindex = None;
        let mut program_id = None;
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
                            owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
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
            let (_, observed) = Self::tap_fact(plan, &second_tap, false);
            let second_tap_for_final = match observed {
                None => None,
                Some(_) => Some(second_tap.clone()),
            };
            if let Some(second_tap) = &second_tap_for_final {
                let (_, second_observed) = Self::tap_fact(plan, second_tap, false);
                let valid_identity = matches!(
                    second_observed,
                    Some(GuestNetworkFact::Tap {
                        name,
                        ifindex: Some(observed_ifindex),
                        link_kind: GuestLinkKind::Tap,
                        persistent: true,
                        owner_uid: Some(observed_owner),
                        ..
                    }) if name == plan.assignment().tap
                        && observed_ifindex == tap_ifindex
                        && observed_owner == overdrive_core::vm::config::OVERDRIVE_VMM_UID
                );
                if valid_identity {
                    let second_master = match second_tap {
                        GuestNetworkAllocationTapObservation::Persistent {
                            master_ifindex, ..
                        }
                        | GuestNetworkAllocationTapObservation::Incompatible {
                            master_ifindex,
                            ..
                        } => *master_ifindex,
                        GuestNetworkAllocationTapObservation::Absent { .. } => None,
                    };
                    Self::ensure_master(tap_ifindex, bridge_ifindex, second_master.or(tap_master))?;
                }
            } else {
                Self::ensure_master(tap_ifindex, bridge_ifindex, tap_master)?;
            }

            self.allocation_io.insert_guard_member(plan).map_err(|error| {
                Self::guard_error(GuestNetworkOperation::GuardMemberInsert, error)
            })?;
            let expected_members = self.expected_guard_members(Some(plan), None);
            let guard = self.allocation_io.observe_guard(&expected_members).map_err(|error| {
                Self::guard_error(GuestNetworkOperation::GuardMemberInsert, error)
            })?;
            self.guard_postcondition(&expected_members, guard)?;

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
            let attached_program = self
                .allocation_io
                .pin_link(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))?;
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

            self.allocation_io
                .set_tap_up(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapSetUp, source))?;
            let final_bridge = self.observe_bridge(plan).await?;
            let final_tap =
                self.allocation_io.observe_tap(plan).await.map_err(|source| {
                    Self::netlink_error(GuestNetworkOperation::TapObserve, source)
                })?;
            let final_tap =
                if matches!(final_tap, GuestNetworkAllocationTapObservation::Absent { .. }) {
                    second_tap_for_final.clone().unwrap_or(final_tap)
                } else {
                    final_tap
                };
            let (mut expected, observed) = Self::tap_fact(plan, &final_tap, true);
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
            let (final_ifindex, final_master) = match final_tap {
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
                GuestNetworkAllocationTapObservation::Absent { .. } => (tap_ifindex, None),
            };
            if final_ifindex != tap_ifindex {
                return Err(GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::TapObserve,
                    expected: Self::master_fact(tap_ifindex, Some(final_bridge)),
                    observed: Some(Self::master_fact(final_ifindex, final_master)),
                });
            }
            Self::ensure_master(final_ifindex, final_bridge, final_master)?;
            Ok(())
        }
        .await;
        if let Err(error) = primary {
            let cleanup = self.rollback_provision(plan, ifindex).await;
            return Err(cleanup.unwrap_or(error));
        }
        let Some(ifindex) = ifindex else {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::TapObserve,
                expected: GuestNetworkFact::Tap {
                    name: plan.assignment().tap.clone(),
                    ifindex: None,
                    link_kind: GuestLinkKind::Tap,
                    persistent: true,
                    up: true,
                    owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
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
        self.allocations
            .lock()
            .insert(plan.alloc().clone(), HostGuestNetworkAllocationState { ifindex, program_id });
        Ok(())
    }
    async fn teardown(&self, plan: &GuestNetworkPlan) -> Result<()> {
        let Some(state) = self.allocations.lock().get(plan.alloc()).copied() else {
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
                .link_pin_present(plan)
                .map_err(|source| Self::tcx_error(GuestNetworkOperation::TcxLinkPin, source))
                .and_then(|present| {
                    if !present {
                        Ok(())
                    } else {
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
                    }
                })
        );
        attempt!(
            self.allocation_io
                .set_tap_down(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapSetDown, source))
        );
        attempt!(
            self.allocation_io
                .observe_tap(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapObserve, source))
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
        attempt!(
            self.allocation_io
                .delete_tap(plan)
                .await
                .map_err(|source| Self::netlink_error(GuestNetworkOperation::TapDelete, source))
        );
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
                .and_then(|observed| self.guard_postcondition(&expected_members, observed))
        );
        if let Some(error) = first {
            return Err(error);
        }
        self.allocations.lock().remove(plan.alloc());
        Ok(())
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        let plan = Self::scratch_plan();
        let primary = self.run_probe(&plan).await.err();
        let (cleanup, observed) = self.cleanup(&plan).await;
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
    async fn sweep_stale(&self) -> Result<()> {
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            let mut first_error = None;
            match std::fs::read_dir("/sys/class/net") {
                Ok(entries) => {
                    for entry in entries {
                        let entry = match entry {
                            Ok(entry) => entry,
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
                    }
                }
                Err(source) => {
                    first_error = Some(overdrive_netlink::NetlinkError::connect(source));
                }
            }
            for path in [
                "/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints",
                "/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters",
            ] {
                match std::fs::remove_file(path) {
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
            if let Ok(entries) = std::fs::read_dir(links) {
                for entry in entries {
                    let path = match entry {
                        Ok(entry) => entry.path(),
                        Err(source) => {
                            if first_error.is_none() {
                                first_error =
                                    Some(overdrive_netlink::NetlinkError::connect(source));
                            }
                            continue;
                        }
                    };
                    match std::fs::remove_file(path) {
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
            if let Err(error) =
                overdrive_netlink::nft::bridge::delete_owned_guard(&guard, &BTreeSet::new())
            {
                if first_error.is_none() {
                    first_error = Some(overdrive_netlink::NetlinkError::nft(
                        "stale-guard",
                        std::io::Error::other(error.to_string()),
                    ));
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
        overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            client.ensure_bridge(BRIDGE).await?;
            client.set_link_down(BRIDGE).await?;
            client.set_link_mac(BRIDGE, overdrive_core::dataplane::GUEST_BRIDGE_MAC).await?;
            client.converge_addr(BRIDGE, GATEWAY, 16).await?;
            client.set_link_up(BRIDGE).await?;
            Ok(())
        })
        .map_err(|source| GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::BridgeConverge,
            source,
        })?;
        let bridge_identity = overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            client.observe_link_identity(BRIDGE).await
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
        {
            return Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::BridgeObserve,
                expected: GuestNetworkFact::BridgeLinkIdentity {
                    name: BRIDGE.to_owned(),
                    ifindex: Some(bridge_identity.ifindex),
                    link_kind: GuestLinkKind::Bridge,
                },
                observed: Some(GuestNetworkFact::BridgeLinkIdentity {
                    name: bridge_identity.name,
                    ifindex: Some(bridge_identity.ifindex),
                    link_kind: observed_kind,
                }),
            });
        }
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
        Ok(())
    }
    async fn audit_shared(&self) -> std::result::Result<(), SharedGuestNetworkAuditError> {
        let bridge = overdrive_netlink::block_on_host_netlink(|| async {
            let client = overdrive_netlink::Client::new()?;
            client.observe_link_identity("ovd-gbr0").await
        })
        .map_err(|source| SharedGuestNetworkAuditError {
            component: SharedGuestNetworkComponent::Bridge,
            source: GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::BridgeObserve,
                source,
            },
        })?;
        let bridge_ok = bridge.as_ref().is_some_and(|identity| {
            matches!(identity.kind, overdrive_netlink::ObservedLinkKind::Bridge)
                && identity.mac == Some(overdrive_core::dataplane::GUEST_BRIDGE_MAC)
                && identity.up
        });
        if !bridge_ok {
            return Err(SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::Bridge,
                source: GuestNetworkError::PostconditionMismatch {
                    operation: GuestNetworkOperation::BridgeObserve,
                    expected: GuestNetworkFact::Bridge {
                        name: "ovd-gbr0".to_owned(),
                        ifindex: None,
                        link_kind: GuestLinkKind::Bridge,
                        mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                        up: true,
                        gateway: Some(Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 1), 16)),
                    },
                    observed: None,
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
        let expected_members = self.expected_guard_members(None, None);
        match overdrive_netlink::nft::bridge::observe(&guard, &expected_members).map_err(
            |error| SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::Bridge,
                source: GuestNetworkError::Io {
                    operation: GuestNetworkOperation::BridgeObserve,
                    source: std::io::Error::other(error.to_string()),
                },
            },
        )? {
            BridgeGuardObservation::Exact { .. } => {}
            BridgeGuardObservation::Absent { .. } | BridgeGuardObservation::Conflict { .. } => {
                return Err(SharedGuestNetworkAuditError {
                    component: SharedGuestNetworkComponent::Bridge,
                    source: GuestNetworkError::PostconditionMismatch {
                        operation: GuestNetworkOperation::BridgeObserve,
                        expected: GuestNetworkFact::BridgeLinkIdentity {
                            name: "ovd-gbr0".to_owned(),
                            ifindex: None,
                            link_kind: GuestLinkKind::Bridge,
                        },
                        observed: None,
                    },
                });
            }
        }
        let tcx = self.tcx.lock();
        let Some(tcx) = tcx.as_ref() else {
            return Err(SharedGuestNetworkAuditError {
                component: SharedGuestNetworkComponent::TcxLink,
                source: GuestNetworkError::Tcx {
                    operation: GuestNetworkOperation::TcxLoad,
                    source: GuestTcxError::CaptureUnavailable {
                        family: overdrive_dataplane::guest_tcx::GuestTcxInventoryFamily::TcxProgram,
                    },
                },
            });
        };
        for (component, observed) in [
            (SharedGuestNetworkComponent::EndpointMap, tcx.inventory.observe_endpoint_maps()),
            (SharedGuestNetworkComponent::CounterMap, tcx.inventory.observe_counter_maps()),
            (SharedGuestNetworkComponent::TcxLink, tcx.inventory.observe_tcx_links()),
        ] {
            if let Err(source) = observed {
                return Err(SharedGuestNetworkAuditError {
                    component,
                    source: GuestNetworkError::Tcx {
                        operation: GuestNetworkOperation::CleanupComplement,
                        source,
                    },
                });
            }
        }
        Ok(())
    }
    async fn quiesce_managed_taps(&self) -> Result<()> {
        let taps: Vec<String> = self
            .allocations
            .lock()
            .values()
            .map(|state| format!("ovd-tp-{:04x}", state.ifindex.saturating_sub(293)))
            .collect();
        for tap in taps {
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
        }
        Ok(())
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

    const SETUP_AND_PROBE: &[ScratchCall] = &[
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::ConvergeBridge),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::CreateTap),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::AttachTapToBridge),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::SetTapUp),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardTable),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardChain),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardSet),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::CreateGuardRules),
        ScratchCall::Netlink(GuestNetworkScratchNetlinkAction::InsertGuardMember),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::LoadProgramAndMaps),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::PinEndpointMap),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::PinCounterMap),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::InsertEndpoint),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::AttachLink),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::PinLink),
        ScratchCall::CloseLoader,
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::AdoptEndpointMap),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::AdoptCounterMap),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::AdoptLink),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::QueryLink),
        ScratchCall::Exercise(GuestNetworkProbeStage::Classifier),
        ScratchCall::Exercise(GuestNetworkProbeStage::OriginalDestination),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::UnpinLink),
        ScratchCall::Tcx(GuestNetworkScratchTcxAction::DetachLink),
        ScratchCall::Exercise(GuestNetworkProbeStage::DetachedLinkGuard),
    ];

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
    async fn healthy_probe_uses_the_exact_setup_probe_cleanup_and_inventory_order() {
        let io = Arc::new(ScriptedScratchIo::default());
        HostSharedGuestNetworkOwner::with_scratch_io(io.clone())
            .probe_startup()
            .await
            .expect("healthy probe plus empty observed complement");
        let mut expected = SETUP_AND_PROBE.to_vec();
        expected.extend_from_slice(CLEANUP);
        assert_eq!(io.calls(), expected);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_setup_or_probe_failure_preserves_primary_and_still_runs_complete_cleanup() {
        for (index, fail) in SETUP_AND_PROBE
            .iter()
            .copied()
            .filter(|call| *call != ScratchCall::CloseLoader)
            .enumerate()
        {
            let io = if let ScratchCall::Exercise(_) = fail {
                ScriptedScratchIo::with_semantic_failure(match fail {
                    ScratchCall::Exercise(stage) => stage,
                    _ => unreachable!(),
                })
            } else {
                ScriptedScratchIo::with_failure(fail)
            };
            let error = HostSharedGuestNetworkOwner::with_scratch_io(io.clone())
                .probe_startup()
                .await
                .expect_err("scripted primary failure");
            assert!(
                !matches!(error, GuestNetworkError::StartupProbeCleanup { .. }),
                "case {index} must preserve the original primary when cleanup is empty"
            );
            let calls = io.calls();
            let failure_index = calls.iter().position(|call| *call == fail).expect("failed call");
            assert_eq!(calls.get(failure_index + 1), Some(&ScratchCall::CloseLoader));
            assert!(
                calls.ends_with(&CLEANUP[1..]),
                "case {index} completes reverse cleanup and inventory"
            );
        }
    }

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
                residue: None,
                calls: Vec::new(),
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
#[allow(dead_code, clippy::doc_markdown, clippy::expect_used, clippy::too_many_lines)]
mod allocation_owner_acceptance {
    //! Allocation-owner model exercised here:
    //! `Unpublished -> Provisioning -> Published -> TeardownPending -> Absent`.
    //! Any setup/read-back failure returns to `Unpublished`; any teardown
    //! failure remains `TeardownPending` with the lease and publication held;
    //! retry on that same owner reaches `Absent`. Teardown from `Unpublished`
    //! or `Absent` is an idempotent self-loop. No other transition publishes.

    use super::*;
    use overdrive_netlink::nft::bridge::BridgeGuardInventory;
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
    }

    struct ScriptedAllocationIo {
        calls: parking_lot::Mutex<Vec<AllocationCall>>,
        tap_observations: parking_lot::Mutex<VecDeque<GuestNetworkAllocationTapObservation>>,
        bridge_observations: parking_lot::Mutex<VecDeque<GuestNetworkAllocationBridgeObservation>>,
        endpoint_reads: parking_lot::Mutex<VecDeque<Option<GuestTcxEndpoint>>>,
        attachment_reads: parking_lot::Mutex<VecDeque<GuestTcxAttachment>>,
        pin_reads: parking_lot::Mutex<VecDeque<bool>>,
        guard_expectations: parking_lot::Mutex<Vec<BTreeSet<String>>>,
        failures: parking_lot::Mutex<BTreeSet<(AllocationCall, usize)>>,
        call_counts: parking_lot::Mutex<BTreeMap<AllocationCall, usize>>,
        publication_probe: parking_lot::Mutex<Option<Arc<dyn Fn() -> bool + Send + Sync>>>,
        publication_trace: parking_lot::Mutex<Vec<bool>>,
    }

    impl ScriptedAllocationIo {
        fn healthy() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                tap_observations: parking_lot::Mutex::new(VecDeque::from([
                    GuestNetworkAllocationTapObservation::Persistent {
                        name: "ovd-tp-0002".to_owned(),
                        ifindex: 295,
                        up: false,
                        owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                        master_ifindex: Some(29),
                    },
                    GuestNetworkAllocationTapObservation::Persistent {
                        name: "ovd-tp-0002".to_owned(),
                        ifindex: 295,
                        up: true,
                        owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                        master_ifindex: Some(29),
                    },
                ])),
                bridge_observations: parking_lot::Mutex::new(VecDeque::from([
                    GuestNetworkAllocationBridgeObservation::Present {
                        name: "ovd-gbr0".to_owned(),
                        ifindex: 29,
                        kind: GuestLinkKind::Bridge,
                    },
                    GuestNetworkAllocationBridgeObservation::Present {
                        name: "ovd-gbr0".to_owned(),
                        ifindex: 29,
                        kind: GuestLinkKind::Bridge,
                    },
                ])),
                endpoint_reads: parking_lot::Mutex::new(VecDeque::new()),
                attachment_reads: parking_lot::Mutex::new(VecDeque::from([GuestTcxAttachment {
                    revision: 1,
                    program_ids: vec![2_950],
                }])),
                pin_reads: parking_lot::Mutex::new(VecDeque::from([true])),
                guard_expectations: parking_lot::Mutex::new(Vec::new()),
                failures: parking_lot::Mutex::new(BTreeSet::new()),
                call_counts: parking_lot::Mutex::new(BTreeMap::new()),
                publication_probe: parking_lot::Mutex::new(None),
                publication_trace: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn with_observations(
            taps: impl IntoIterator<Item = GuestNetworkAllocationTapObservation>,
            bridges: impl IntoIterator<Item = GuestNetworkAllocationBridgeObservation>,
        ) -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                tap_observations: parking_lot::Mutex::new(taps.into_iter().collect()),
                bridge_observations: parking_lot::Mutex::new(bridges.into_iter().collect()),
                endpoint_reads: parking_lot::Mutex::new(VecDeque::new()),
                attachment_reads: parking_lot::Mutex::new(VecDeque::from([GuestTcxAttachment {
                    revision: 1,
                    program_ids: vec![2_950],
                }])),
                pin_reads: parking_lot::Mutex::new(VecDeque::from([true])),
                guard_expectations: parking_lot::Mutex::new(Vec::new()),
                failures: parking_lot::Mutex::new(BTreeSet::new()),
                call_counts: parking_lot::Mutex::new(BTreeMap::new()),
                publication_probe: parking_lot::Mutex::new(None),
                publication_trace: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn for_teardown(failures: impl IntoIterator<Item = AllocationCall>) -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                tap_observations: parking_lot::Mutex::new(VecDeque::from([
                    GuestNetworkAllocationTapObservation::Persistent {
                        name: "ovd-tp-0002".to_owned(),
                        ifindex: 295,
                        up: false,
                        owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                        master_ifindex: Some(29),
                    },
                    GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
                ])),
                bridge_observations: parking_lot::Mutex::new(VecDeque::new()),
                endpoint_reads: parking_lot::Mutex::new(VecDeque::from([None])),
                attachment_reads: parking_lot::Mutex::new(VecDeque::from([GuestTcxAttachment {
                    revision: 1,
                    program_ids: Vec::new(),
                }])),
                pin_reads: parking_lot::Mutex::new(VecDeque::from([false])),
                guard_expectations: parking_lot::Mutex::new(Vec::new()),
                failures: parking_lot::Mutex::new(
                    failures.into_iter().map(|call| (call, 1)).collect(),
                ),
                call_counts: parking_lot::Mutex::new(BTreeMap::new()),
                publication_probe: parking_lot::Mutex::new(None),
                publication_trace: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn with_failure_occurrence(call: AllocationCall, occurrence: usize) -> Arc<Self> {
            let io = Self::for_teardown(std::iter::empty::<AllocationCall>());
            io.failures.lock().insert((call, occurrence));
            io
        }

        fn clear_failures(&self) {
            self.failures.lock().clear();
        }

        fn reset_teardown_observations(&self) {
            *self.tap_observations.lock() = VecDeque::from([
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
                    master_ifindex: Some(29),
                },
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
            ]);
            *self.endpoint_reads.lock() = VecDeque::from([None]);
            *self.attachment_reads.lock() =
                VecDeque::from([GuestTcxAttachment { revision: 1, program_ids: Vec::new() }]);
            *self.pin_reads.lock() = VecDeque::from([false]);
        }

        fn track_publication(
            &self,
            owner: &Arc<HostSharedGuestNetworkOwner>,
            allocation: AllocationId,
        ) {
            let owner = Arc::downgrade(owner);
            *self.publication_probe.lock() = Some(Arc::new(move || {
                owner
                    .upgrade()
                    .is_some_and(|owner| owner.allocations.lock().contains_key(&allocation))
            }));
        }

        fn record(&self, call: AllocationCall) -> bool {
            let published = self.publication_probe.lock().as_ref().is_some_and(|probe| probe());
            self.publication_trace.lock().push(published);
            self.calls.lock().push(call);
            let mut counts = self.call_counts.lock();
            let count = counts.entry(call).or_insert(0);
            *count += 1;
            let occurrence = *count;
            drop(counts);
            self.failures.lock().contains(&(call, occurrence))
        }

        fn calls(&self) -> Vec<AllocationCall> {
            self.calls.lock().clone()
        }

        fn guard_expectations(&self) -> Vec<BTreeSet<String>> {
            self.guard_expectations.lock().clone()
        }

        fn publication_trace(&self) -> Vec<bool> {
            self.publication_trace.lock().clone()
        }

        fn remaining_teardown_observations(&self) -> (usize, usize, usize, usize) {
            (
                self.tap_observations.lock().len(),
                self.endpoint_reads.lock().len(),
                self.attachment_reads.lock().len(),
                self.pin_reads.lock().len(),
            )
        }
    }

    fn netlink_failure() -> NetlinkError {
        NetlinkError::connect(std::io::Error::from_raw_os_error(libc::EBUSY))
    }

    fn tcx_failure() -> GuestTcxError {
        GuestTcxError::Io { source: std::io::Error::from_raw_os_error(libc::EIO) }
    }

    fn guard_inventory() -> BridgeGuardInventory {
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
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationTapObservation, NetlinkError> {
            if self.record(AllocationCall::ObserveTap) {
                return Err(netlink_failure());
            }
            Ok(self.tap_observations.lock().pop_front().unwrap_or_else(|| {
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() }
            }))
        }
        async fn observe_bridge(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestNetworkAllocationBridgeObservation, NetlinkError> {
            if self.record(AllocationCall::ObserveBridge) {
                return Err(netlink_failure());
            }
            Ok(self.bridge_observations.lock().pop_front().unwrap_or_else(|| {
                GuestNetworkAllocationBridgeObservation::Absent { name: "ovd-gbr0".to_owned() }
            }))
        }
        fn insert_guard_member(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            if self.record(AllocationCall::InsertGuard) {
                Err(BridgeGuardError::Netlink(netlink_failure()))
            } else {
                Ok(BridgeGuardMutationOutcome::Converged { observed: guard_inventory() })
            }
        }
        fn delete_guard_member(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<BridgeGuardMutationOutcome, BridgeGuardError> {
            if self.record(AllocationCall::DeleteGuard) {
                Err(BridgeGuardError::Netlink(netlink_failure()))
            } else {
                Ok(BridgeGuardMutationOutcome::Converged { observed: guard_inventory() })
            }
        }
        fn observe_guard(
            &self,
            expected_members: &BTreeSet<String>,
        ) -> std::result::Result<BridgeGuardObservation, BridgeGuardError> {
            self.guard_expectations.lock().push(expected_members.clone());
            if self.record(AllocationCall::ObserveGuard) {
                Err(BridgeGuardError::Netlink(netlink_failure()))
            } else {
                Ok(BridgeGuardObservation::Exact { inventory: guard_inventory() })
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
                Err(tcx_failure())
            } else {
                Ok(self.endpoint_reads.lock().pop_front().unwrap_or_else(|| {
                    Some(GuestTcxEndpoint {
                        source_ipv4: plan.assignment().address,
                        source_mac: plan.assignment().mac,
                        bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                    })
                }))
            }
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
            if self.record(AllocationCall::PinLink) { Err(tcx_failure()) } else { Ok(2_950) }
        }
        fn query_attachment(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
            if self.record(AllocationCall::QueryAttachment) {
                Err(tcx_failure())
            } else {
                Ok(self.attachment_reads.lock().pop_front().unwrap_or_else(|| GuestTcxAttachment {
                    revision: 1,
                    program_ids: vec![2_950],
                }))
            }
        }
        fn link_pin_present(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> std::result::Result<bool, GuestTcxError> {
            if self.record(AllocationCall::LinkPinPresent) {
                Err(tcx_failure())
            } else {
                Ok(self.pin_reads.lock().pop_front().unwrap_or(true))
            }
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
    }

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

    fn bridge_fact(name: &str, ifindex: Option<u32>, link_kind: GuestLinkKind) -> GuestNetworkFact {
        GuestNetworkFact::BridgeLinkIdentity { name: name.to_owned(), ifindex, link_kind }
    }

    fn master_fact(ifindex: u32, master_ifindex: Option<u32>) -> GuestNetworkFact {
        GuestNetworkFact::LinkMaster { ifindex, master_ifindex }
    }

    fn exposed_attachment_facts(
        plan: &GuestNetworkPlan,
        state: HostGuestNetworkAllocationState,
    ) -> Vec<GuestNetworkFact> {
        vec![
            tap_fact(
                &plan.assignment().tap,
                Some(state.ifindex),
                GuestLinkKind::Tap,
                true,
                true,
                Some(overdrive_core::vm::config::OVERDRIVE_VMM_UID),
            ),
            GuestNetworkFact::TcxAttachment {
                ifindex: state.ifindex,
                program_id: Some(state.program_id),
                attach_point: Some(TcxAttachPoint::Ingress),
            },
            GuestNetworkFact::EndpointMapEntry {
                ifindex: state.ifindex,
                value: Some(GuestEndpointFact {
                    source_ip: plan.assignment().address,
                    source_mac: plan.assignment().mac,
                    bridge_mac: overdrive_core::dataplane::GUEST_BRIDGE_MAC,
                }),
            },
        ]
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned expected/observed facts keep each table row self-contained"
    )]
    fn assert_mismatch(
        error: GuestNetworkError,
        operation: GuestNetworkOperation,
        expected: GuestNetworkFact,
        observed: Option<GuestNetworkFact>,
    ) {
        assert!(matches!(
            error,
            GuestNetworkError::PostconditionMismatch {
                operation: actual_operation,
                expected: actual_expected,
                observed: actual_observed,
            } if actual_operation == operation
                && actual_expected == expected
                && actual_observed == observed
        ));
    }

    #[derive(Debug, Clone, Copy)]
    enum CleanupFailureKind {
        Tcx,
        Netlink,
    }

    fn teardown_calls() -> [AllocationCall; 11] {
        [
            AllocationCall::RemoveEndpoint,
            AllocationCall::DetachPinnedLink,
            AllocationCall::ReadEndpoint,
            AllocationCall::QueryAttachment,
            AllocationCall::LinkPinPresent,
            AllocationCall::SetTapDown,
            AllocationCall::ObserveTap,
            AllocationCall::DeleteTap,
            AllocationCall::DeleteGuard,
            AllocationCall::ObserveTap,
            AllocationCall::ObserveGuard,
        ]
    }

    fn assert_cleanup_leaf_error(
        error: GuestNetworkError,
        operation: GuestNetworkOperation,
        kind: CleanupFailureKind,
    ) {
        match kind {
            CleanupFailureKind::Tcx => assert!(matches!(
                error,
                GuestNetworkError::Tcx {
                    operation: actual,
                    source: GuestTcxError::Io { source },
                } if actual == operation && source.raw_os_error() == Some(libc::EIO)
            )),
            CleanupFailureKind::Netlink => assert!(matches!(
                error,
                GuestNetworkError::Netlink {
                    operation: actual,
                    source: NetlinkError::Connect { source },
                } if actual == operation && source.raw_os_error() == Some(libc::EBUSY)
            )),
        }
    }

    /// S-ND295-11 — verified attachment precedes admission.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn provision_reads_every_attachment_fact_before_reporting_success() {
        let io = ScriptedAllocationIo::healthy();
        let owner = Arc::new(HostSharedGuestNetworkOwner::with_allocation_io(io.clone()));
        let plan = plan("nd295-s11", Ipv4Addr::new(100, 95, 0, 2));
        io.track_publication(&owner, plan.alloc().clone());
        assert!(!owner.allocations.lock().contains_key(plan.alloc()));
        owner.provision(&plan).await.expect("all leaf effects and exact read-backs succeed");

        assert_eq!(
            io.calls(),
            [
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
                AllocationCall::SetTapUp,
                AllocationCall::ObserveBridge,
                AllocationCall::ObserveTap,
            ]
        );
        assert_eq!(
            io.publication_trace(),
            vec![false; 16],
            "the allocation remains unpublished through the final TAP-up read-back call"
        );
        assert_eq!(
            owner.allocations.lock().get(plan.alloc()).copied(),
            Some(HostGuestNetworkAllocationState { ifindex: 295, program_id: 2_950 }),
            "publication occurs only after the final bridge refresh and TAP-up read-back"
        );
    }

    /// S-ND295-11 — incompatible TAP/bridge identity refuses publication.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_incompatible_tap_or_bridge_identity_refuses_owner_publication() {
        let uid = overdrive_core::vm::config::OVERDRIVE_VMM_UID;
        let bridge_29 = GuestNetworkAllocationBridgeObservation::Present {
            name: "ovd-gbr0".to_owned(),
            ifindex: 29,
            kind: GuestLinkKind::Bridge,
        };
        let valid_down = GuestNetworkAllocationTapObservation::Persistent {
            name: "ovd-tp-0002".to_owned(),
            ifindex: 295,
            up: false,
            owner_uid: Some(uid),
            master_ifindex: Some(29),
        };
        let valid_up = GuestNetworkAllocationTapObservation::Persistent {
            name: "ovd-tp-0002".to_owned(),
            ifindex: 295,
            up: true,
            owner_uid: Some(uid),
            master_ifindex: Some(29),
        };

        let first_tap_cases = [
            (
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
                tap_fact("ovd-tp-0002", None, GuestLinkKind::Tap, true, false, Some(uid)),
                None,
            ),
            (
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tun,
                    persistent: Some(true),
                    up: false,
                    owner_uid: Some(uid),
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tun,
                    true,
                    false,
                    Some(uid),
                )),
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
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, Some(uid)),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Other, false, false, None)),
            ),
            (
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tap,
                    persistent: Some(false),
                    up: false,
                    owner_uid: Some(uid),
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    false,
                    false,
                    Some(uid),
                )),
            ),
            (
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: Some(uid + 1),
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    true,
                    false,
                    Some(uid + 1),
                )),
            ),
            (
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: true,
                    owner_uid: Some(uid),
                    master_ifindex: None,
                },
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, false, Some(uid)),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid))),
            ),
        ];
        for (index, (actual, expected, observed)) in first_tap_cases.into_iter().enumerate() {
            let io = ScriptedAllocationIo::with_observations([actual], [bridge_29.clone()]);
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io);
            let plan = plan(&format!("nd295-s11-first-tap-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("first TAP checkpoint mismatch refuses publication");
            assert_mismatch(error, GuestNetworkOperation::TapObserve, expected, observed);
            assert!(!owner.allocations.lock().contains_key(plan.alloc()));
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
                    ifindex: 29,
                    kind: GuestLinkKind::Other,
                },
                bridge_fact("ovd-gbr0", Some(29), GuestLinkKind::Bridge),
                Some(bridge_fact("ovd-gbr0", Some(29), GuestLinkKind::Other)),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let io = ScriptedAllocationIo::with_observations([valid_down.clone()], [actual]);
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io);
            let plan =
                plan(&format!("nd295-s11-first-bridge-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("first bridge checkpoint mismatch refuses publication");
            assert_mismatch(error, GuestNetworkOperation::BridgeObserve, expected, observed);
            assert!(!owner.allocations.lock().contains_key(plan.alloc()));
        }

        let first_master_io = ScriptedAllocationIo::with_observations(
            [GuestNetworkAllocationTapObservation::Persistent {
                name: "ovd-tp-0002".to_owned(),
                ifindex: 295,
                up: false,
                owner_uid: Some(uid),
                master_ifindex: Some(30),
            }],
            [bridge_29.clone()],
        );
        let first_master_owner = HostSharedGuestNetworkOwner::with_allocation_io(first_master_io);
        let first_master_plan = plan("nd295-s11-first-master", Ipv4Addr::new(100, 95, 0, 2));
        let first_master_error = first_master_owner
            .provision(&first_master_plan)
            .await
            .expect_err("down-TAP master mismatch refuses publication");
        assert_mismatch(
            first_master_error,
            GuestNetworkOperation::TapObserve,
            master_fact(295, Some(29)),
            Some(master_fact(295, Some(30))),
        );
        assert!(!first_master_owner.allocations.lock().contains_key(first_master_plan.alloc()));

        let final_cases = [
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Absent { name: "ovd-tp-0002".to_owned() },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                None,
            ),
            (
                GuestNetworkAllocationBridgeObservation::Absent { name: "ovd-gbr0".to_owned() },
                valid_up.clone(),
                GuestNetworkOperation::BridgeObserve,
                bridge_fact("ovd-gbr0", None, GuestLinkKind::Bridge),
                None,
            ),
            (
                GuestNetworkAllocationBridgeObservation::Present {
                    name: "ovd-gbr0".to_owned(),
                    ifindex: 29,
                    kind: GuestLinkKind::Other,
                },
                valid_up.clone(),
                GuestNetworkOperation::BridgeObserve,
                bridge_fact("ovd-gbr0", Some(29), GuestLinkKind::Bridge),
                Some(bridge_fact("ovd-gbr0", Some(29), GuestLinkKind::Other)),
            ),
            (
                GuestNetworkAllocationBridgeObservation::Present {
                    name: "ovd-gbr0".to_owned(),
                    ifindex: 30,
                    kind: GuestLinkKind::Bridge,
                },
                valid_up.clone(),
                GuestNetworkOperation::TapObserve,
                master_fact(295, Some(30)),
                Some(master_fact(295, Some(29))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: true,
                    owner_uid: Some(uid),
                    master_ifindex: Some(30),
                },
                GuestNetworkOperation::TapObserve,
                master_fact(295, Some(29)),
                Some(master_fact(295, Some(30))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tun,
                    persistent: Some(true),
                    up: true,
                    owner_uid: Some(uid),
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tun, true, true, Some(uid))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Other,
                    persistent: None,
                    up: true,
                    owner_uid: None,
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Other, false, true, None)),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 296,
                    up: true,
                    owner_uid: Some(uid),
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact("ovd-tp-0002", Some(296), GuestLinkKind::Tap, true, true, Some(uid))),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: true,
                    owner_uid: Some(uid + 1),
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    true,
                    true,
                    Some(uid + 1),
                )),
            ),
            (
                bridge_29.clone(),
                GuestNetworkAllocationTapObservation::Incompatible {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    kind: GuestLinkKind::Tap,
                    persistent: Some(false),
                    up: true,
                    owner_uid: Some(uid),
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    false,
                    true,
                    Some(uid),
                )),
            ),
            (
                bridge_29,
                GuestNetworkAllocationTapObservation::Persistent {
                    name: "ovd-tp-0002".to_owned(),
                    ifindex: 295,
                    up: false,
                    owner_uid: Some(uid),
                    master_ifindex: Some(29),
                },
                GuestNetworkOperation::TapObserve,
                tap_fact("ovd-tp-0002", Some(295), GuestLinkKind::Tap, true, true, Some(uid)),
                Some(tap_fact(
                    "ovd-tp-0002",
                    Some(295),
                    GuestLinkKind::Tap,
                    true,
                    false,
                    Some(uid),
                )),
            ),
        ];
        for (index, (final_bridge, final_tap, operation, expected, observed)) in
            final_cases.into_iter().enumerate()
        {
            let io = ScriptedAllocationIo::with_observations(
                [valid_down.clone(), final_tap],
                [
                    GuestNetworkAllocationBridgeObservation::Present {
                        name: "ovd-gbr0".to_owned(),
                        ifindex: 29,
                        kind: GuestLinkKind::Bridge,
                    },
                    final_bridge,
                ],
            );
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io);
            let plan = plan(&format!("nd295-s11-final-{index}"), Ipv4Addr::new(100, 95, 0, 2));
            let error = owner
                .provision(&plan)
                .await
                .expect_err("final checkpoint mismatch refuses publication");
            assert_mismatch(error, operation, expected, observed);
            assert!(!owner.allocations.lock().contains_key(plan.alloc()));
        }
    }

    /// S-ND295-12 — teardown continues, returns the first source, and retries empty.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement()
    {
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
        // Separate dual-failure example: the later TAP failure must never replace
        // the genuine first endpoint-deletion source.
        let io = ScriptedAllocationIo::for_teardown([
            AllocationCall::RemoveEndpoint,
            AllocationCall::DeleteTap,
        ]);
        let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
        let named_state = HostGuestNetworkAllocationState { ifindex: 295, program_id: 2_950 };
        let unrelated_state = HostGuestNetworkAllocationState { ifindex: 296, program_id: 2_950 };
        owner.allocations.lock().insert(named.alloc().clone(), named_state);
        owner.allocations.lock().insert(unrelated.alloc().clone(), unrelated_state);
        let leases_before = pool.snapshot();
        let unrelated_plan_before =
            leases_before.get(unrelated.alloc()).expect("unrelated lease present").clone();
        let unrelated_facts_before = exposed_attachment_facts(&unrelated, unrelated_state);

        let error = owner
            .teardown(&named)
            .await
            .expect_err("primary and later cleanup failures retain the first exact source");
        assert!(matches!(
            error,
            GuestNetworkError::Tcx {
                operation: GuestNetworkOperation::EndpointDelete,
                source: GuestTcxError::Io { source },
            } if source.raw_os_error() == Some(libc::EIO)
        ));
        let first_attempt = io.calls();
        let endpoint_failure = first_attempt
            .iter()
            .position(|call| *call == AllocationCall::RemoveEndpoint)
            .expect("primary endpoint deletion attempted");
        let later_failure = first_attempt
            .iter()
            .position(|call| *call == AllocationCall::DeleteTap)
            .expect("later TAP deletion attempted");
        assert!(endpoint_failure < later_failure);
        assert!(first_attempt.contains(&AllocationCall::DeleteGuard));
        assert!(
            first_attempt.ends_with(&[AllocationCall::ObserveTap, AllocationCall::ObserveGuard,])
        );
        assert_eq!(owner.allocations.lock().get(named.alloc()).copied(), Some(named_state));
        assert_eq!(owner.allocations.lock().get(unrelated.alloc()).copied(), Some(unrelated_state));
        assert_eq!(pool.snapshot(), leases_before, "the same named lease remains held on failure");

        io.clear_failures();
        io.reset_teardown_observations();
        let retry_start = io.calls().len();
        owner
            .teardown(&named)
            .await
            .expect("the same retained owner/allocation retries to an exact empty complement");
        let retry_calls = &io.calls()[retry_start..];
        assert_eq!(
            retry_calls,
            [
                AllocationCall::RemoveEndpoint,
                AllocationCall::DetachPinnedLink,
                AllocationCall::ReadEndpoint,
                AllocationCall::QueryAttachment,
                AllocationCall::LinkPinPresent,
                AllocationCall::SetTapDown,
                AllocationCall::ObserveTap,
                AllocationCall::DeleteTap,
                AllocationCall::DeleteGuard,
                AllocationCall::ObserveTap,
                AllocationCall::ObserveGuard,
            ],
            "named empty complement is endpoint -> link/pin -> TAP-down -> guarded delete -> member delete -> final read-back"
        );
        assert!(!owner.allocations.lock().contains_key(named.alloc()));
        assert_eq!(
            io.remaining_teardown_observations(),
            (0, 0, 0, 0),
            "retry consumes the exact TAP/endpoint/attachment/pin empty-complement facts"
        );
        assert_eq!(owner.allocations.lock().get(unrelated.alloc()).copied(), Some(unrelated_state));
        assert_eq!(
            pool.snapshot().get(unrelated.alloc()),
            Some(&unrelated_plan_before),
            "the unrelated attachment lease remains byte-equal"
        );
        assert_eq!(
            exposed_attachment_facts(
                &unrelated,
                owner
                    .allocations
                    .lock()
                    .get(unrelated.alloc())
                    .copied()
                    .expect("unrelated publication remains present"),
            ),
            unrelated_facts_before,
            "the unrelated attachment's exposed facts remain byte-equal"
        );
        assert_eq!(
            io.guard_expectations().last(),
            Some(&BTreeSet::from([unrelated.assignment().tap.clone()])),
            "final guard read-back is the exact unrelated-TAP complement"
        );

        let calls_before_repeat = io.calls().len();
        owner
            .teardown(&named)
            .await
            .expect("repeating teardown after the named complement is empty is idempotent");
        assert_eq!(io.calls().len(), calls_before_repeat);

        pool.release(named.alloc());
        assert!(!pool.snapshot().contains_key(named.alloc()));
        assert_eq!(pool.snapshot().get(unrelated.alloc()), Some(&unrelated_plan_before));

        let absent = plan("nd295-s12-never-published", Ipv4Addr::new(100, 95, 0, 99));
        let calls_before_absent = io.calls().len();
        owner.teardown(&absent).await.expect("teardown of an unpublished allocation is idempotent");
        owner
            .teardown(&absent)
            .await
            .expect("repeated teardown of an unpublished allocation remains idempotent");
        assert_eq!(io.calls().len(), calls_before_absent);
        assert_eq!(owner.allocations.lock().get(unrelated.alloc()).copied(), Some(unrelated_state));

        // Exhaustive single-failure table: each accepted cleanup leaf retains
        // its exact operation/source while all later cleanup calls still run.
        let failure_rows = [
            (
                AllocationCall::RemoveEndpoint,
                1,
                GuestNetworkOperation::EndpointDelete,
                CleanupFailureKind::Tcx,
            ),
            (
                AllocationCall::DetachPinnedLink,
                1,
                GuestNetworkOperation::TcxDetach,
                CleanupFailureKind::Tcx,
            ),
            (
                AllocationCall::ReadEndpoint,
                1,
                GuestNetworkOperation::EndpointMapObserve,
                CleanupFailureKind::Tcx,
            ),
            (
                AllocationCall::QueryAttachment,
                1,
                GuestNetworkOperation::TcxQuery,
                CleanupFailureKind::Tcx,
            ),
            (
                AllocationCall::LinkPinPresent,
                1,
                GuestNetworkOperation::TcxLinkPin,
                CleanupFailureKind::Tcx,
            ),
            (
                AllocationCall::SetTapDown,
                1,
                GuestNetworkOperation::TapSetDown,
                CleanupFailureKind::Netlink,
            ),
            (
                AllocationCall::ObserveTap,
                1,
                GuestNetworkOperation::TapObserve,
                CleanupFailureKind::Netlink,
            ),
            (
                AllocationCall::DeleteTap,
                1,
                GuestNetworkOperation::TapDelete,
                CleanupFailureKind::Netlink,
            ),
            (
                AllocationCall::DeleteGuard,
                1,
                GuestNetworkOperation::GuardMemberDelete,
                CleanupFailureKind::Netlink,
            ),
            (
                AllocationCall::ObserveTap,
                2,
                GuestNetworkOperation::TapObserve,
                CleanupFailureKind::Netlink,
            ),
            (
                AllocationCall::ObserveGuard,
                1,
                GuestNetworkOperation::CleanupComplement,
                CleanupFailureKind::Netlink,
            ),
        ];
        for (index, (failing_leaf, occurrence, operation, kind)) in
            failure_rows.into_iter().enumerate()
        {
            let io = ScriptedAllocationIo::with_failure_occurrence(failing_leaf, occurrence);
            let owner = HostSharedGuestNetworkOwner::with_allocation_io(io.clone());
            let failing_plan = plan(
                &format!("nd295-s12-leaf-{index}"),
                Ipv4Addr::new(100, 95, 1, u8::try_from(index + 2).expect("small table index")),
            );
            let state = HostGuestNetworkAllocationState { ifindex: 295, program_id: 2_950 };
            owner.allocations.lock().insert(failing_plan.alloc().clone(), state);

            let error = owner
                .teardown(&failing_plan)
                .await
                .expect_err("each typed cleanup leaf failure remains visible");
            assert_cleanup_leaf_error(error, operation, kind);
            assert_eq!(
                io.calls(),
                teardown_calls(),
                "cleanup continues through the final guard observation after {failing_leaf:?} occurrence {occurrence}"
            );
            assert_eq!(
                owner.allocations.lock().get(failing_plan.alloc()).copied(),
                Some(state),
                "a cleanup failure retains the same allocation state for retry"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown, clippy::expect_used)]
mod pool_acceptance {
    use super::*;
    use overdrive_core::dataplane::GUEST_BRIDGE_MAC;
    use proptest::prelude::*;

    fn pool() -> GuestAddressPool {
        GuestAddressPool {
            node_prefix: "100.95.0.0/16".parse().expect("node prefix"),
            bridge: "ovd-gbr0".to_owned(),
            gateway: "100.95.0.1".parse().expect("gateway"),
            dns: "100.95.0.1".parse().expect("DNS"),
            held: Arc::new(parking_lot::Mutex::new(GuestAddressPoolState {
                plans: BTreeMap::new(),
                addresses: BTreeSet::new(),
                next_candidate: u32::from(Ipv4Addr::new(100, 95, 0, 0)).saturating_add(1),
            })),
        }
    }

    proptest! {
        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        fn assignment_replay_release_and_reuse_match_the_smallest_free_model(
            operations in prop::collection::vec((any::<bool>(), 0_u16..512), 1..256),
        ) {
            let pool = pool();
            let mut model = BTreeMap::<AllocationId, GuestNetworkPlan>::new();

            for (assign, key) in operations {
                let alloc = AllocationId::new(&format!("nd295-{key:04x}"))
                    .expect("generated allocation id");
                if assign {
                    let plan = pool.assign(alloc.clone()).expect("below-cap assignment");
                    if let Some(existing) = model.get(&alloc) {
                        prop_assert_eq!(&plan, existing, "replay is byte-equal");
                    } else {
                        let used = model
                            .values()
                            .map(|plan| plan.assignment.address)
                            .collect::<std::collections::BTreeSet<_>>();
                        let expected_host = (2_u32..=u16::MAX.into())
                            .find(|host| {
                                let octets = host.to_be_bytes();
                                !used.contains(&Ipv4Addr::new(100, 95, octets[2], octets[3]))
                            })
                            .expect("model has one free address");
                        let octets = expected_host.to_be_bytes();
                        prop_assert_eq!(
                            plan.assignment.address,
                            Ipv4Addr::new(100, 95, octets[2], octets[3])
                        );
                        let expected_tap = format!("ovd-tp-{expected_host:04x}");
                        prop_assert_eq!(plan.assignment.tap.as_str(), expected_tap.as_str());
                        prop_assert_eq!(
                            plan.assignment.mac,
                            [0x02, 0x00, 100, 95, octets[2], octets[3]]
                        );
                        prop_assert_ne!(plan.assignment.mac, GUEST_BRIDGE_MAC);
                        model.insert(alloc.clone(), plan);
                    }
                } else {
                    pool.release(&alloc);
                    model.remove(&alloc);
                    pool.release(&alloc);
                }
                prop_assert_eq!(pool.snapshot(), model.clone());
            }
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn slash_16_exhaustion_is_pool_drift_and_does_not_reuse_an_address() {
        let pool = pool();
        for index in 0_u32..65_533 {
            pool.assign(
                AllocationId::new(&format!("nd295-pool-{index:05}")).expect("allocation id"),
            )
            .expect("all non-reserved /16 addresses fit");
        }
        let before = pool.snapshot();
        let addresses = before
            .values()
            .map(|plan| plan.assignment.address)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(!addresses.contains(&Ipv4Addr::new(100, 95, 0, 0)));
        assert!(!addresses.contains(&Ipv4Addr::new(100, 95, 0, 1)));
        assert!(!addresses.contains(&Ipv4Addr::new(100, 95, 255, 255)));
        assert!(addresses.contains(&Ipv4Addr::new(100, 95, 255, 254)));
        assert_eq!(addresses.len(), 65_533);
        let error = pool
            .assign(AllocationId::new("nd295-pool-overflow").expect("allocation id"))
            .expect_err("full /16 returns typed drift without mutation");
        assert!(matches!(
            error,
            GuestNetworkError::PoolExhausted { held: 65_533, capacity: 65_533 }
        ));
        assert_eq!(pool.snapshot(), before);
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
