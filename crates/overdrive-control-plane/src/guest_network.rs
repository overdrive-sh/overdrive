//! Shared guest-network application contract (GH #295).
//!
//! The private host owner/pool implementation lands in DELIVER. DISTILL owns
//! this exact accepted API scaffold and its executable specifications.

// SCAFFOLD: true — netns-density-295 DISTILL.

#![expect(
    clippy::use_self,
    reason = "exact accepted API scaffold precedes implementation and names GuestNetworkError explicitly"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::GuestNetworkAssignment;
use overdrive_netlink::NetlinkError;

pub use overdrive_dataplane::guest_tcx::{GuestTcxError, TcxAttachPoint};
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
#[allow(dead_code, reason = "GH #295 exact RED scaffold is exercised only by pending tests")]
pub(crate) struct GuestAddressPool {
    node_prefix: Ipv4Net,
    bridge: String,
    gateway: Ipv4Addr,
    dns: Ipv4Addr,
    held: Arc<parking_lot::Mutex<BTreeMap<AllocationId, GuestNetworkPlan>>>,
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
            held: Arc::new(parking_lot::Mutex::new(BTreeMap::new())),
        }
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements the accepted pool")]
    pub(crate) fn assign(&self, _alloc: AllocationId) -> Result<GuestNetworkPlan> {
        panic!("Not yet implemented -- RED scaffold (GH #295 guest address assign)")
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements release-last")]
    pub(crate) fn release(&self, _alloc: &AllocationId) {
        panic!("Not yet implemented -- RED scaffold (GH #295 guest address release)")
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements detached snapshot")]
    pub(crate) fn snapshot(&self) -> BTreeMap<AllocationId, GuestNetworkPlan> {
        panic!("Not yet implemented -- RED scaffold (GH #295 guest address snapshot)")
    }
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

#[derive(Debug, Default)]
struct RealSharedGuestNetworkScratchIo;

#[async_trait::async_trait]
#[allow(
    clippy::panic,
    reason = "RED scaffold; DELIVER binds each approved typed real scratch effect"
)]
impl SharedGuestNetworkScratchIo for RealSharedGuestNetworkScratchIo {
    async fn apply_netlink(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _action: GuestNetworkScratchNetlinkAction,
    ) -> std::result::Result<(), NetlinkError> {
        panic!("Not yet implemented -- DELIVER binds typed overdrive-netlink scratch effects")
    }

    async fn apply_tcx(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _action: GuestNetworkScratchTcxAction,
    ) -> std::result::Result<(), GuestTcxError> {
        panic!("Not yet implemented -- DELIVER binds typed guest_tcx scratch effects")
    }

    fn close_loader_handles(&self, _plan: &GuestNetworkScratchPlan) {
        panic!("Not yet implemented -- DELIVER closes scratch loader handles")
    }

    fn release_adopted_handles(&self, _plan: &GuestNetworkScratchPlan) {
        panic!("Not yet implemented -- DELIVER releases adopted scratch handles")
    }

    async fn exercise(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _stage: GuestNetworkProbeStage,
    ) -> std::io::Result<bool> {
        panic!("Not yet implemented -- DELIVER binds classifier/socket scratch exercise")
    }

    async fn count_netlink(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _resource: GuestNetworkScratchNetlinkResource,
    ) -> std::result::Result<u32, NetlinkError> {
        panic!("Not yet implemented -- DELIVER binds typed overdrive-netlink scratch inventory")
    }

    async fn count_tcx(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _resource: GuestNetworkScratchTcxResource,
    ) -> std::result::Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- DELIVER binds typed guest_tcx scratch inventory")
    }
}

/// Private host owner; its startup algorithm is independently executable from
/// the still-pending native adapter bindings.
#[allow(dead_code, reason = "D5 exact RED owner field is activated in DELIVER")]
pub(crate) struct HostSharedGuestNetworkOwner {
    scratch_io: Arc<dyn SharedGuestNetworkScratchIo>,
}

impl std::fmt::Debug for HostSharedGuestNetworkOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostSharedGuestNetworkOwner").finish_non_exhaustive()
    }
}

#[allow(
    dead_code,
    clippy::panic,
    clippy::unused_async,
    reason = "D5 exact RED algorithm scaffolds remain inactive until DELIVER"
)]
impl HostSharedGuestNetworkOwner {
    pub(crate) fn new() -> Self {
        Self { scratch_io: Arc::new(RealSharedGuestNetworkScratchIo) }
    }

    #[cfg(test)]
    fn with_scratch_io(scratch_io: Arc<dyn SharedGuestNetworkScratchIo>) -> Self {
        Self { scratch_io }
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
        _plan: &GuestNetworkScratchPlan,
        _action: GuestNetworkScratchNetlinkAction,
        _operation: GuestNetworkOperation,
    ) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 scratch netlink projection)")
    }

    async fn tcx(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _action: GuestNetworkScratchTcxAction,
        _operation: GuestNetworkOperation,
    ) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 scratch TCX projection)")
    }

    async fn exercise(
        &self,
        _plan: &GuestNetworkScratchPlan,
        _stage: GuestNetworkProbeStage,
    ) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 scratch exercise projection)")
    }

    async fn run_probe(&self, _plan: &GuestNetworkScratchPlan) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 scratch probe algorithm)")
    }

    async fn cleanup(
        &self,
        _plan: &GuestNetworkScratchPlan,
    ) -> (Option<GuestNetworkError>, GuestNetworkScratchComplement) {
        panic!("Not yet implemented -- RED scaffold (GH #295 scratch cleanup algorithm)")
    }
}

#[async_trait::async_trait]
#[allow(clippy::panic, reason = "exact host-owner effects remain RED until DELIVER")]
impl GuestNetworkProvisioner for HostSharedGuestNetworkOwner {
    async fn provision(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 host provision)")
    }
    async fn teardown(&self, _plan: &GuestNetworkPlan) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 host teardown)")
    }
}

#[async_trait::async_trait]
#[allow(clippy::panic, reason = "exact host-owner lifecycle remains RED until DELIVER")]
impl SharedGuestNetworkOwner for HostSharedGuestNetworkOwner {
    async fn probe_startup(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 host startup probe)")
    }
    async fn sweep_stale(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 stale sweep)")
    }
    async fn converge_shared(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared converge)")
    }
    async fn audit_shared(&self) -> std::result::Result<(), SharedGuestNetworkAuditError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared audit)")
    }
    async fn quiesce_managed_taps(&self) -> Result<()> {
        panic!("Not yet implemented -- RED scaffold (GH #295 TAP quiesce)")
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
    #[ignore = "pending DELIVER step for GH #295 D5 host startup algorithm"]
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
    #[ignore = "pending DELIVER step for GH #295 D5 setup/probe cleanup algorithm"]
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
    #[ignore = "pending DELIVER step for GH #295 D5 semantic exercise projection"]
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
    #[ignore = "pending DELIVER step for GH #295 D5 cleanup aggregation"]
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
    #[ignore = "pending DELIVER step for GH #295 D5 complement residue classification"]
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
    #[ignore = "pending DELIVER step for GH #295 D5 primary-plus-cleanup preservation"]
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
            held: Arc::new(parking_lot::Mutex::new(BTreeMap::new())),
        }
    }

    proptest! {
        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        #[ignore = "pending DELIVER step for GH #295 guest address pool"]
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
    #[ignore = "full /16 pool exhaustion is a density acceptance case; run in the affected acceptance lane"]
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
    #[ignore = "pending DELIVER step for GH #295 exact pool value projection"]
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
