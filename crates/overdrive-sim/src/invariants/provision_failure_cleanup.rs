//! Seeded VM provision-failure cleanup and slot-reuse invariant.
//!
//! This is the Tier-1 proof for ADR-0089 §6's post-assignment unwind. It
//! drives three VM allocations through the real action shim and its real
//! [`WorkloadNetworkProvisioner`] port:
//!
//! 1. a blocker starts successfully and holds slot 0;
//! 2. the target obtains slot 1, the Sim provisioner creates a seeded,
//!    non-empty subset of its allocation-owned logical network artifacts, and
//!    then returns the existing typed provision error;
//! 3. production dispatch invokes structural teardown, records the durable
//!    `Failed` outcome, and releases only after teardown succeeds; and
//! 4. a successor starts through the same production path and is observed by
//!    the provisioner on slot 1 (the smallest free slot), not slot 2.
//!
//! The checker states the three Tier-1 property classes explicitly:
//!
//! - **safety** — the failed allocation never reaches the driver, its durable
//!   reason remains `WorkloadNetnsProvisionFailed`, and no allocation-owned
//!   artifact survives the cleanup boundary;
//! - **liveness** — the failure reaches `Failed` and the successor reaches
//!   `Running` within three bounded dispatch ticks;
//! - **convergence** — teardown removes the exact created artifact set and the
//!   released slot is reused as the allocator's next smallest-free choice.
//!
//! The Sim adapter records only effects observed at the existing provisioner
//! boundary. It does not reproduce the action-shim cleanup algorithm. The
//! negative control removes one real `ArtifactRemoved` observation from a
//! healthy trace; the checker must turn red even though the final snapshot is
//! otherwise unchanged.

#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::action_shim::{
    AllocDriverIndex, WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_workload_netns_plan, responder_addr_for_slot,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, ObservationStore,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::adapters::ca::SimCa;
use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::driver::SimDriver;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::vm_host_state::SimVmHostState;
use crate::harness::{InvariantResult, InvariantStatus};

const NAME: &str = "vm-provision-failure-cleans-network-and-reuses-slot";
const HOST: &str = "host-0";
const FAILURE_CALL: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NetworkArtifact {
    Namespace(String),
    HostVeth(String),
    WorkloadVeth(String),
    ResolverDirectory(String),
    TransitRoute(String),
    GuestTap(String),
    GuestRoute(String),
    ReturnRoute(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NetworkEvent {
    ProvisionStarted { ordinal: usize, owner: String },
    ArtifactCreated { ordinal: usize, owner: String, artifact: NetworkArtifact },
    ProvisionFailed { ordinal: usize, owner: String },
    ProvisionSucceeded { ordinal: usize, owner: String },
    TeardownStarted { owner: String },
    ArtifactRemoved { owner: String, artifact: NetworkArtifact },
    TeardownCompleted { owner: String, remaining: BTreeSet<NetworkArtifact> },
}

#[derive(Debug, Default)]
struct NetworkState {
    provision_calls: usize,
    artifacts: BTreeMap<String, BTreeSet<NetworkArtifact>>,
    events: Vec<NetworkEvent>,
}

/// Sim implementation of the existing C3 network-provisioner boundary.
///
/// Calls 1 and 3 converge every logical artifact. Call 2 creates a seeded,
/// non-empty proper subset and then returns the same typed error family the
/// host adapter authors. Teardown removes by the allocation-keyed structural
/// owner supplied by the real production plan.
struct SimPartialFailureNetwork {
    entropy: SimEntropy,
    state: Mutex<NetworkState>,
}

impl SimPartialFailureNetwork {
    fn new(seed: u64) -> Self {
        Self { entropy: SimEntropy::new(seed), state: Mutex::new(NetworkState::default()) }
    }

    fn events(&self) -> Vec<NetworkEvent> {
        self.state.lock().events.clone()
    }

    fn artifacts_for(&self, owner: &str) -> BTreeSet<NetworkArtifact> {
        self.state.lock().artifacts.get(owner).cloned().unwrap_or_default()
    }

    fn failure() -> VethProvisionError {
        VethProvisionError::SysctlSetFailed {
            key: "net.ipv4.ip_forward".to_owned(),
            value: "1".to_owned(),
            path: "/sim/vm-provision-failure".to_owned(),
            source: std::io::Error::other("seeded post-assignment provision failure"),
        }
    }
}

impl WorkloadNetworkProvisioner for SimPartialFailureNetwork {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        let owner = workload.netns.as_str().to_owned();
        let mut all = network_artifacts(workload, vm_tap);
        let draw = self.entropy.u64();

        let mut state = self.state.lock();
        state.provision_calls += 1;
        let ordinal = state.provision_calls;
        state.events.push(NetworkEvent::ProvisionStarted { ordinal, owner: owner.clone() });

        if ordinal == FAILURE_CALL {
            // Seeded nondeterminism chooses both WHICH artifacts exist and how
            // many. The range excludes zero and the complete set, so this is
            // always an honest partial-provision state.
            let rotation = usize::try_from(draw % all.len() as u64)
                .unwrap_or_else(|_| unreachable!("draw modulo artifact count fits usize"));
            all.rotate_left(rotation);
            let partial_count = 1 + usize::try_from(draw.rotate_left(17) % (all.len() - 1) as u64)
                .unwrap_or_else(|_| unreachable!("partial artifact count fits usize"));
            all.truncate(partial_count);
        }

        for artifact in all {
            state.artifacts.entry(owner.clone()).or_default().insert(artifact.clone());
            state.events.push(NetworkEvent::ArtifactCreated {
                ordinal,
                owner: owner.clone(),
                artifact,
            });
        }

        if ordinal == FAILURE_CALL {
            state.events.push(NetworkEvent::ProvisionFailed { ordinal, owner });
            return Err(Self::failure());
        }

        state.events.push(NetworkEvent::ProvisionSucceeded { ordinal, owner });
        drop(state);
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        let owner = workload.netns.as_str().to_owned();
        let mut state = self.state.lock();
        state.events.push(NetworkEvent::TeardownStarted { owner: owner.clone() });
        let removed = state.artifacts.remove(&owner).unwrap_or_default();
        for artifact in removed {
            state.events.push(NetworkEvent::ArtifactRemoved { owner: owner.clone(), artifact });
        }
        let remaining = state.artifacts.get(&owner).cloned().unwrap_or_default();
        state.events.push(NetworkEvent::TeardownCompleted { owner, remaining });
        drop(state);
        Ok(())
    }
}

fn network_artifacts(
    workload: &WorkloadNetnsPlan,
    vm_tap: Option<&VmTapPlan>,
) -> Vec<NetworkArtifact> {
    let owner = workload.netns.as_str();
    let mut artifacts = vec![
        NetworkArtifact::Namespace(owner.to_owned()),
        NetworkArtifact::HostVeth(workload.host_veth.clone()),
        NetworkArtifact::WorkloadVeth(workload.workload_veth.clone()),
        NetworkArtifact::ResolverDirectory(format!("/etc/netns/{owner}")),
        NetworkArtifact::TransitRoute(workload.subnet.to_string()),
    ];
    if let Some(tap) = vm_tap {
        artifacts.extend([
            NetworkArtifact::GuestTap(tap.tap.clone()),
            NetworkArtifact::GuestRoute(tap.guest_network.to_string()),
            NetworkArtifact::ReturnRoute(format!(
                "{} via {} dev {}",
                tap.guest_network, workload.workload_addr, workload.host_veth
            )),
        ]);
    }
    artifacts
}

struct Fixture {
    _tempdir: TempDir,
    intent: Arc<dyn IntentStore>,
    observation: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    drivers: DriverRegistry,
    alloc_drivers: AllocDriverIndex,
    dataplane: SimDataplane,
    ca: SimCa,
    identity: IdentityMgr,
    events: tokio::sync::broadcast::Sender<overdrive_control_plane::action_shim::LifecycleEvent>,
    broker: Mutex<overdrive_core::eval_broker::EvaluationBroker>,
    slots: NetSlotAllocator,
    network: SimPartialFailureNetwork,
    host: SimVmHostState,
    node: NodeId,
}

impl Fixture {
    fn new(seed: u64, node: NodeId) -> Result<Self, String> {
        let tempdir = TempDir::new().map_err(|error| format!("tempdir: {error}"))?;
        let intent: Arc<dyn IntentStore> = Arc::new(
            LocalIntentStore::open(tempdir.path().join("intent.redb"))
                .map_err(|error| format!("open LocalIntentStore: {error}"))?,
        );
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let mut drivers = DriverRegistry::new();
        let driver_port: Arc<dyn Driver> = driver.clone();
        drivers.insert(driver_port);
        let (events, _receiver) = tokio::sync::broadcast::channel(16);

        Ok(Self {
            _tempdir: tempdir,
            intent: Arc::clone(&intent),
            observation: Arc::new(SimObservationStore::single_peer(node.clone(), seed)),
            clock,
            driver,
            drivers,
            alloc_drivers: AllocDriverIndex::default(),
            dataplane: SimDataplane::new(),
            ca: SimCa::new(Arc::new(SimEntropy::new(seed))),
            identity: IdentityMgr::new(None),
            events,
            broker: Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
            slots: NetSlotAllocator::new(),
            network: SimPartialFailureNetwork::new(seed),
            host: SimVmHostState::new(),
            node,
        })
    }

    async fn run_sequence(
        &self,
        allocations: &[(AllocationId, WorkloadId); 3],
    ) -> Result<BTreeSet<NetworkArtifact>, String> {
        let allocator = overdrive_control_plane::test_default_allocator(Arc::clone(&self.intent));
        let mut artifacts_after_failure = None;

        for (tick_number, (alloc, workload)) in (1_u64..).zip(allocations) {
            let now = self.clock.now();
            let tick = TickContext {
                now,
                now_unix: UnixInstant::from_unix_duration(self.clock.unix_now()),
                tick: tick_number,
                deadline: now + Duration::from_secs(1),
            };
            dispatch_with_network_provisioner(
                vec![Action::StartAllocation {
                    alloc_id: alloc.clone(),
                    workload_id: workload.clone(),
                    node_id: self.node.clone(),
                    spec: vm_spec(alloc, workload),
                    kind: WorkloadKind::Job,
                }],
                &self.drivers,
                &self.alloc_drivers,
                self.observation.as_ref(),
                &self.dataplane,
                &self.ca,
                self.clock.as_ref(),
                &self.identity,
                &self.events,
                &tick,
                &self.node,
                Arc::clone(&allocator),
                &self.broker,
                None,
                None,
                &self.slots,
                &self.network,
                &self.host,
            )
            .await
            .map_err(|error| format!("StartAllocation({alloc}) dispatch failed: {error}"))?;

            if tick_number == 2 {
                let events = self.network.events();
                let owner = provision_owner(&events, FAILURE_CALL)
                    .ok_or_else(|| "failed provision never reached the driven port".to_owned())?;
                artifacts_after_failure = Some(self.network.artifacts_for(&owner));
            }
        }

        artifacts_after_failure
            .ok_or_else(|| "failure-boundary artifact snapshot was not captured".to_owned())
    }
}

#[derive(Debug, Clone)]
struct Evidence {
    seed: u64,
    blocker_owner: String,
    failed_owner: String,
    successor_owner: String,
    failed_row: AllocStatusRow,
    failed_occurrences: Vec<AllocLifecycleOccurrenceRow>,
    successor_row: AllocStatusRow,
    driver_started: Vec<AllocationId>,
    failed_alloc: AllocationId,
    successor_alloc: AllocationId,
    events: Vec<NetworkEvent>,
    artifacts_after_failure: BTreeSet<NetworkArtifact>,
}

struct FixtureIds {
    blocker_alloc: AllocationId,
    blocker_workload: WorkloadId,
    failed_alloc: AllocationId,
    failed_workload: WorkloadId,
    successor_alloc: AllocationId,
    successor_workload: WorkloadId,
    node: NodeId,
}

fn fixture_ids(seed: u64) -> Result<FixtureIds, String> {
    let suffix = seed % 1_000_000;
    Ok(FixtureIds {
        blocker_alloc: AllocationId::new(&format!("alloc-net-blocker-{suffix}"))
            .map_err(|error| format!("blocker allocation id: {error:?}"))?,
        blocker_workload: WorkloadId::new(&format!("net-blocker-{suffix}"))
            .map_err(|error| format!("blocker workload id: {error:?}"))?,
        failed_alloc: AllocationId::new(&format!("alloc-net-failure-{suffix}"))
            .map_err(|error| format!("failed allocation id: {error:?}"))?,
        failed_workload: WorkloadId::new(&format!("net-failure-{suffix}"))
            .map_err(|error| format!("failed workload id: {error:?}"))?,
        successor_alloc: AllocationId::new(&format!("alloc-net-successor-{suffix}"))
            .map_err(|error| format!("successor allocation id: {error:?}"))?,
        successor_workload: WorkloadId::new(&format!("net-successor-{suffix}"))
            .map_err(|error| format!("successor workload id: {error:?}"))?,
        node: NodeId::new(&format!("node-net-failure-{suffix}"))
            .map_err(|error| format!("node id: {error:?}"))?,
    })
}

fn vm_spec(alloc: &AllocationId, workload: &WorkloadId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(workload, alloc),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/sim/kernel"),
            rootfs: PathBuf::from("/sim/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn provision_owner(events: &[NetworkEvent], ordinal: usize) -> Option<String> {
    events.iter().find_map(|event| match event {
        NetworkEvent::ProvisionStarted { ordinal: observed, owner } if *observed == ordinal => {
            Some(owner.clone())
        }
        _ => None,
    })
}

async fn drive(seed: u64) -> Result<Evidence, String> {
    let ids = fixture_ids(seed)?;
    let fixture = Fixture::new(seed, ids.node)?;

    // Calls 1-3: hold slot 0; create seeded partial slot-1 state and fail;
    // then observe the production allocator's next choice at the same port.
    // The fixture snapshots artifacts immediately after call 2, before the
    // successor can begin.
    let artifacts_after_failure = fixture
        .run_sequence(&[
            (ids.blocker_alloc, ids.blocker_workload),
            (ids.failed_alloc.clone(), ids.failed_workload),
            (ids.successor_alloc.clone(), ids.successor_workload),
        ])
        .await?;

    let events = fixture.network.events();
    let blocker_owner = provision_owner(&events, 1)
        .ok_or_else(|| "blocker provision never reached the driven port".to_owned())?;
    let failed_owner = provision_owner(&events, FAILURE_CALL)
        .ok_or_else(|| "failed provision was absent after the driven sequence".to_owned())?;
    let successor_owner = provision_owner(&events, 3)
        .ok_or_else(|| "successor provision never reached the driven port".to_owned())?;
    let failed_row = fixture
        .observation
        .alloc_status_row(&ids.failed_alloc)
        .await
        .map_err(|error| format!("read failed allocation row: {error}"))?
        .ok_or_else(|| "failed allocation row was not recorded".to_owned())?;
    let failed_occurrences = fixture
        .observation
        .alloc_lifecycle_occurrences(&ids.failed_alloc)
        .await
        .map_err(|error| format!("read failed allocation occurrences: {error}"))?;
    let successor_row = fixture
        .observation
        .alloc_status_row(&ids.successor_alloc)
        .await
        .map_err(|error| format!("read successor allocation row: {error}"))?
        .ok_or_else(|| "successor allocation row was not recorded".to_owned())?;
    let driver_started =
        fixture.driver.started_specs().into_iter().map(|spec| spec.alloc).collect();

    Ok(Evidence {
        seed,
        blocker_owner,
        failed_owner,
        successor_owner,
        failed_row,
        failed_occurrences,
        successor_row,
        driver_started,
        failed_alloc: ids.failed_alloc,
        successor_alloc: ids.successor_alloc,
        events,
        artifacts_after_failure,
    })
}

/// CONTRACT_SHAPE: pure-function.
fn check_evidence(evidence: &Evidence) -> Result<(), String> {
    let slot_zero = NetSlot::new(0)
        .unwrap_or_else(|_| unreachable!("slot zero is inside the declared slot domain"));
    let slot_one = NetSlot::new(1)
        .unwrap_or_else(|_| unreachable!("slot one is inside the declared slot domain"));
    let expected_blocker =
        derive_workload_netns_plan(slot_zero, responder_addr_for_slot(slot_zero))
            .netns
            .as_str()
            .to_owned();
    let expected_reused = derive_workload_netns_plan(slot_one, responder_addr_for_slot(slot_one))
        .netns
        .as_str()
        .to_owned();

    if evidence.blocker_owner != expected_blocker {
        return Err(format!(
            "seed {} safety precondition: first production allocation did not hold slot 0; owner={}",
            evidence.seed, evidence.blocker_owner
        ));
    }
    if evidence.failed_owner != expected_reused {
        return Err(format!(
            "seed {} safety precondition: failed production allocation did not receive smallest-free slot 1; owner={}",
            evidence.seed, evidence.failed_owner
        ));
    }

    if evidence.failed_row.state != AllocState::Failed
        || !matches!(
            &evidence.failed_row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { stage, detail })
                if stage == "netns_provision"
                    && detail == "workload netns provisioning failed"
        )
    {
        return Err(format!(
            "seed {} liveness: post-assignment provision failure did not converge to the durable Failed cause; row={:?}",
            evidence.seed, evidence.failed_row
        ));
    }
    if evidence.failed_occurrences.len() != 1
        || evidence.failed_occurrences[0].to != AllocState::Failed
        || evidence.failed_occurrences[0].reason != evidence.failed_row.reason
    {
        return Err(format!(
            "seed {} durability: Failed disposition was not recorded as exactly one accepted lifecycle occurrence; occurrences={:?}",
            evidence.seed, evidence.failed_occurrences
        ));
    }
    if evidence.driver_started.contains(&evidence.failed_alloc) {
        return Err(format!(
            "seed {} safety: the VM driver started the allocation whose network provision failed",
            evidence.seed
        ));
    }

    let created: BTreeSet<NetworkArtifact> = evidence
        .events
        .iter()
        .filter_map(|event| match event {
            NetworkEvent::ArtifactCreated { ordinal, owner, artifact }
                if *ordinal == FAILURE_CALL && owner == &evidence.failed_owner =>
            {
                Some(artifact.clone())
            }
            _ => None,
        })
        .collect();
    if created.is_empty() || created.len() >= 8 {
        return Err(format!(
            "seed {} safety precondition: injected failure did not leave a non-empty proper partial artifact set; created={created:?}",
            evidence.seed
        ));
    }
    let removed: BTreeSet<NetworkArtifact> = evidence
        .events
        .iter()
        .filter_map(|event| match event {
            NetworkEvent::ArtifactRemoved { owner, artifact }
                if owner == &evidence.failed_owner =>
            {
                Some(artifact.clone())
            }
            _ => None,
        })
        .collect();
    if removed != created {
        return Err(format!(
            "seed {} convergence: structural teardown did not remove the exact created artifact set; created={created:?}, removed={removed:?}",
            evidence.seed
        ));
    }
    if !evidence.artifacts_after_failure.is_empty() {
        return Err(format!(
            "seed {} convergence: allocation-owned artifacts survived the failure boundary: {:?}",
            evidence.seed, evidence.artifacts_after_failure
        ));
    }

    let failed_at = evidence
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                NetworkEvent::ProvisionFailed { ordinal, owner }
                    if *ordinal == FAILURE_CALL && owner == &evidence.failed_owner
            )
        })
        .ok_or_else(|| format!("seed {}: provision failure was not observed", evidence.seed))?;
    let teardown_started_at = evidence
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                NetworkEvent::TeardownStarted { owner } if owner == &evidence.failed_owner
            )
        })
        .ok_or_else(|| format!("seed {}: structural teardown was not invoked", evidence.seed))?;
    let teardown_completed_at = evidence
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                NetworkEvent::TeardownCompleted { owner, remaining }
                    if owner == &evidence.failed_owner && remaining.is_empty()
            )
        })
        .ok_or_else(|| {
            format!("seed {}: structural teardown did not complete cleanly", evidence.seed)
        })?;
    let successor_at = evidence
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                NetworkEvent::ProvisionStarted { ordinal: 3, owner }
                    if owner == &evidence.successor_owner
            )
        })
        .ok_or_else(|| format!("seed {}: successor never reached provisioning", evidence.seed))?;
    if !(failed_at < teardown_started_at
        && teardown_started_at < teardown_completed_at
        && teardown_completed_at < successor_at)
    {
        return Err(format!(
            "seed {} safety: observed order was not failure -> teardown -> cleanup complete -> successor; events={:?}",
            evidence.seed, evidence.events
        ));
    }

    if evidence.successor_owner != expected_reused
        || evidence.successor_owner != evidence.failed_owner
    {
        return Err(format!(
            "seed {} convergence: successor did not reuse released smallest-free slot 1; failed_owner={}, successor_owner={}",
            evidence.seed, evidence.failed_owner, evidence.successor_owner
        ));
    }
    if evidence.successor_row.state != AllocState::Running
        || !evidence.driver_started.contains(&evidence.successor_alloc)
    {
        return Err(format!(
            "seed {} liveness: successor did not complete the real allocator/provision/driver path; row={:?}, driver_started={:?}",
            evidence.seed, evidence.successor_row, evidence.driver_started
        ));
    }

    Ok(())
}

/// CONTRACT_SHAPE: pure-function.
fn remove_one_cleanup_fact(evidence: &Evidence) -> Evidence {
    let mut defective = evidence.clone();
    if let Some(index) = defective.events.iter().position(|event| {
        matches!(
            event,
            NetworkEvent::ArtifactRemoved { owner, .. } if owner == &defective.failed_owner
        )
    }) {
        defective.events.remove(index);
    }
    defective
}

/// Evaluate the registered seeded provisioning-failure invariant.
pub async fn evaluate(seed: u64) -> InvariantResult {
    let evidence = match drive(seed).await {
        Ok(evidence) => evidence,
        Err(cause) => return fail(cause),
    };
    if let Err(cause) = check_evidence(&evidence) {
        return fail(cause);
    }
    if check_evidence(&remove_one_cleanup_fact(&evidence)).is_ok() {
        return fail(format!(
            "seed {seed}: negative control removed one observed cleanup fact but the invariant still passed"
        ));
    }
    pass()
}

fn pass() -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Pass,
        tick: 3,
        host: HOST.to_owned(),
        cause: None,
    }
}

fn fail(cause: String) -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Fail,
        tick: 3,
        host: HOST.to_owned(),
        cause: Some(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn fixed_seed_converges_and_reuses_the_smallest_free_slot() {
        let result = evaluate(424_242).await;
        assert_eq!(result.status, InvariantStatus::Pass, "{:?}", result.cause);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn invariant_has_teeth_when_one_cleanup_fact_is_removed() {
        let evidence = match drive(424_242).await {
            Ok(evidence) => evidence,
            Err(cause) => panic!("fixed seed did not drive the production path: {cause}"),
        };
        if let Err(cause) = check_evidence(&evidence) {
            panic!("healthy production trace violated the invariant: {cause}");
        }
        let Err(cause) = check_evidence(&remove_one_cleanup_fact(&evidence)) else {
            panic!("removing one observed cleanup fact must turn the invariant red");
        };
        assert!(cause.contains("exact created artifact set"), "unexpected teeth failure: {cause}");
    }
}
