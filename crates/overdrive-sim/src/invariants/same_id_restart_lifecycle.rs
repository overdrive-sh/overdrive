//! Same-ID restart ordering across the mTLS lifecycle, network, identity, and driver ports.
//!
//! The evaluator drives the real action shim. Its local doubles only record
//! the existing driven-port completions; they do not recreate dispatch or
//! cleanup policy. `SimMtlsInterceptLifecycle` remains socket-free and models
//! the lifecycle owner's exact retry states.

#![allow(clippy::doc_markdown, clippy::missing_errors_doc, clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_control_plane::action_shim::{
    AllocDriverIndex, MtlsInterceptLifecycle, WorkloadNetworkProvisioner,
    dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::{
    NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverRegistry, DriverStartClass, DriverStartFailure, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::vm_host_state::SimVmHostState;
use crate::adapters::{SimCa, SimMtlsInterceptLifecycle, SimMtlsInterceptLifecycleState};
use crate::harness::{InvariantResult, InvariantStatus};

const NAME: &str = "same-id-restart-removes-prior-protection-before-replacement-provision";
const HOST: &str = "host-0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TraceEvent {
    DriverStop,
    LifecycleStopCompleted,
    NetworkTeardownAndSlotRelease,
    ReplacementProvision,
    DriverStartCompleted { identity_present: bool },
    LifecycleStartCompleted,
}

#[derive(Default)]
struct Trace(Mutex<Vec<TraceEvent>>);

impl Trace {
    fn record(&self, event: TraceEvent) {
        self.0.lock().push(event);
    }

    fn snapshot(&self) -> Vec<TraceEvent> {
        self.0.lock().clone()
    }

    fn clear(&self) {
        self.0.lock().clear();
    }
}

struct TraceLifecycle {
    inner: Arc<SimMtlsInterceptLifecycle>,
    trace: Arc<Trace>,
}

#[async_trait]
impl MtlsInterceptLifecycle for TraceLifecycle {
    async fn start_alloc(
        &self,
        spec: &AllocationSpec,
    ) -> Result<(), overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError> {
        self.inner.start_alloc(spec).await?;
        self.trace.record(TraceEvent::LifecycleStartCompleted);
        Ok(())
    }

    async fn stop_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<(), overdrive_worker::mtls_intercept_worker::MtlsInterceptStopError> {
        self.inner.stop_alloc(alloc_id).await?;
        self.trace.record(TraceEvent::LifecycleStopCompleted);
        Ok(())
    }
}

#[derive(Default)]
struct NetworkState {
    fail_teardown_once: bool,
    fail_provision_once: bool,
}

struct TraceNetwork {
    trace: Arc<Trace>,
    state: Mutex<NetworkState>,
}

impl TraceNetwork {
    fn new(trace: Arc<Trace>) -> Self {
        Self { trace, state: Mutex::new(NetworkState::default()) }
    }

    fn fail_teardown_once(&self) {
        self.state.lock().fail_teardown_once = true;
    }

    fn fail_provision_once(&self) {
        self.state.lock().fail_provision_once = true;
    }

    fn failure(stage: &'static str) -> VethProvisionError {
        VethProvisionError::SysctlSetFailed {
            key: "net.ipv4.ip_forward".to_owned(),
            value: "1".to_owned(),
            path: format!("/sim/{stage}"),
            source: std::io::Error::other(format!("seeded {stage} failure")),
        }
    }
}

impl WorkloadNetworkProvisioner for TraceNetwork {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        let mut state = self.state.lock();
        if std::mem::take(&mut state.fail_provision_once) {
            return Err(Self::failure("replacement-provision"));
        }
        drop(state);
        self.trace.record(TraceEvent::ReplacementProvision);
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        let mut state = self.state.lock();
        if std::mem::take(&mut state.fail_teardown_once) {
            return Err(Self::failure("network-teardown"));
        }
        drop(state);
        self.trace.record(TraceEvent::NetworkTeardownAndSlotRelease);
        Ok(())
    }
}

struct TraceDriver {
    live: Mutex<BTreeSet<AllocationId>>,
    fail_next_start: Mutex<bool>,
    identity: Arc<IdentityMgr>,
    trace: Arc<Trace>,
}

impl TraceDriver {
    const fn new(identity: Arc<IdentityMgr>, trace: Arc<Trace>) -> Self {
        Self {
            live: Mutex::new(BTreeSet::new()),
            fail_next_start: Mutex::new(false),
            identity,
            trace,
        }
    }

    fn fail_next_start(&self) {
        *self.fail_next_start.lock() = true;
    }
}

#[async_trait]
impl Driver for TraceDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        if std::mem::take(&mut *self.fail_next_start.lock()) {
            return Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Unclassified { driver: DriverType::Vm },
                    detail: "seeded replacement driver start failure".to_owned(),
                },
            });
        }
        let identity_present = self.identity.held_snapshot().contains_key(&spec.alloc);
        self.live.lock().insert(spec.alloc.clone());
        self.trace.record(TraceEvent::DriverStartCompleted { identity_present });
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.trace.record(TraceEvent::DriverStop);
        self.live.lock().remove(&handle.alloc);
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        if self.live.lock().contains(&handle.alloc) {
            Ok(AllocationState::Running)
        } else {
            Err(DriverError::NotFound { alloc: handle.alloc.clone() })
        }
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Err(DriverError::ResizeUnsupported {
            driver: DriverType::Vm,
            alloc: handle.alloc.clone(),
            detail: "not used by restart invariant".to_owned(),
        })
    }
}

struct RejectingIssueCa {
    inner: SimCa,
}

impl RejectingIssueCa {
    fn new(seed: u64) -> Self {
        Self { inner: SimCa::new(Arc::new(SimEntropy::new(seed))) }
    }
}

impl Ca for RejectingIssueCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.inner.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.inner.issue_intermediate(node)
    }

    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        Err(CaError::SigningFailed { reason: "seeded identity issuance failure".to_owned() })
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.inner.trust_bundle()
    }
}

struct Fixture {
    _tempdir: TempDir,
    intent: Arc<dyn IntentStore>,
    observation: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    driver: Arc<TraceDriver>,
    drivers: DriverRegistry,
    alloc_drivers: AllocDriverIndex,
    dataplane: SimDataplane,
    ca: SimCa,
    identity: Arc<IdentityMgr>,
    events: tokio::sync::broadcast::Sender<overdrive_control_plane::action_shim::LifecycleEvent>,
    broker: Mutex<overdrive_core::eval_broker::EvaluationBroker>,
    slots: NetSlotAllocator,
    network: TraceNetwork,
    host: SimVmHostState,
    lifecycle: TraceLifecycle,
    trace: Arc<Trace>,
    alloc: AllocationId,
    workload: WorkloadId,
    node: NodeId,
}

impl Fixture {
    fn new(seed: u64) -> Result<Self, String> {
        let suffix = seed % 1_000_000;
        let node = NodeId::new(&format!("node-restart-{suffix}"))
            .map_err(|error| format!("node id: {error:?}"))?;
        let alloc = AllocationId::new(&format!("alloc-restart-{suffix}"))
            .map_err(|error| format!("allocation id: {error:?}"))?;
        let workload = WorkloadId::new(&format!("restart-{suffix}"))
            .map_err(|error| format!("workload id: {error:?}"))?;
        let tempdir = TempDir::new().map_err(|error| format!("tempdir: {error}"))?;
        let intent: Arc<dyn IntentStore> = Arc::new(
            LocalIntentStore::open(tempdir.path().join("intent.redb"))
                .map_err(|error| format!("intent store: {error}"))?,
        );
        let trace = Arc::new(Trace::default());
        let identity = Arc::new(IdentityMgr::new(None));
        let driver = Arc::new(TraceDriver::new(Arc::clone(&identity), Arc::clone(&trace)));
        let mut drivers = DriverRegistry::new();
        let driver_port: Arc<dyn Driver> = driver.clone();
        drivers.insert(driver_port);
        let lifecycle_inner = Arc::new(SimMtlsInterceptLifecycle::new());
        let (events, _receiver) = tokio::sync::broadcast::channel(16);

        Ok(Self {
            _tempdir: tempdir,
            intent,
            observation: Arc::new(SimObservationStore::single_peer(node.clone(), seed)),
            clock: Arc::new(SimClock::new()),
            driver,
            drivers,
            alloc_drivers: AllocDriverIndex::default(),
            dataplane: SimDataplane::new(),
            ca: SimCa::new(Arc::new(SimEntropy::new(seed))),
            identity,
            events,
            broker: Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new()),
            slots: NetSlotAllocator::new(),
            network: TraceNetwork::new(Arc::clone(&trace)),
            host: SimVmHostState::new(),
            lifecycle: TraceLifecycle { inner: lifecycle_inner, trace: Arc::clone(&trace) },
            trace,
            alloc,
            workload,
            node,
        })
    }

    fn spec(&self) -> AllocationSpec {
        AllocationSpec {
            alloc: self.alloc.clone(),
            identity: SpiffeId::for_allocation(&self.workload, &self.alloc),
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
            workload_addr: None,
            guest_tap: None,
            guest_mac: None,
            guest_gateway: None,
            guest_prefix_len: None,
            guest_dns: None,
            service_ports: Vec::new(),
        }
    }

    fn tick(&self, count: u64) -> TickContext {
        let now = self.clock.now();
        TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(self.clock.unix_now()),
            tick: count,
            deadline: now + Duration::from_secs(1),
        }
    }

    async fn dispatch(&self, action: Action, count: u64, ca: &dyn Ca) -> Result<(), String> {
        let allocator = overdrive_control_plane::test_default_allocator(Arc::clone(&self.intent));
        dispatch_with_network_provisioner(
            vec![action],
            &self.drivers,
            &self.alloc_drivers,
            self.observation.as_ref(),
            &self.dataplane,
            ca,
            self.clock.as_ref(),
            self.identity.as_ref(),
            &self.events,
            &self.tick(count),
            &self.node,
            allocator,
            &self.broker,
            None,
            Some(&self.lifecycle),
            &self.slots,
            &self.network,
            &self.host,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn start(&self) -> Result<(), String> {
        self.dispatch(
            Action::StartAllocation {
                alloc_id: self.alloc.clone(),
                workload_id: self.workload.clone(),
                node_id: self.node.clone(),
                spec: self.spec(),
                kind: WorkloadKind::Job,
            },
            1,
            &self.ca,
        )
        .await
    }

    async fn restart(&self, count: u64, ca: &dyn Ca) -> Result<(), String> {
        self.dispatch(
            Action::RestartAllocation {
                alloc_id: self.alloc.clone(),
                spec: self.spec(),
                kind: WorkloadKind::Job,
            },
            count,
            ca,
        )
        .await
    }

    fn lifecycle_state(&self) -> Option<SimMtlsInterceptLifecycleState> {
        self.lifecycle.inner.snapshot().allocations.get(&self.alloc).copied()
    }

    fn slot_is_retained(&self) -> bool {
        self.slots.snapshot().contains_key(&self.alloc)
    }
}

/// CONTRACT_SHAPE: pure-function.
fn check_order(trace: &[TraceEvent]) -> Result<(), String> {
    let find = |event| {
        trace.iter().position(|observed| {
            matches!(
                (event, observed),
                (0, TraceEvent::DriverStop)
                    | (1, TraceEvent::LifecycleStopCompleted)
                    | (2, TraceEvent::NetworkTeardownAndSlotRelease)
                    | (3, TraceEvent::ReplacementProvision)
                    | (4, TraceEvent::DriverStartCompleted { identity_present: true })
                    | (5, TraceEvent::LifecycleStartCompleted)
            )
        })
    };
    let order: Vec<usize> = (0..6)
        .map(|event| {
            find(event).ok_or_else(|| format!("missing restart effect {event}; trace={trace:?}"))
        })
        .collect::<Result<_, _>>()?;
    if order.windows(2).all(|pair| pair[0] < pair[1]) {
        Ok(())
    } else {
        Err(format!("restart completion order violated: order={order:?}; trace={trace:?}"))
    }
}

/// CONTRACT_SHAPE: pure-function.
fn without_lifecycle_stop(trace: &[TraceEvent]) -> Vec<TraceEvent> {
    trace.iter().copied().filter(|event| *event != TraceEvent::LifecycleStopCompleted).collect()
}

async fn clean(seed: u64) -> Result<(), String> {
    let fixture = Fixture::new(seed)?;
    fixture.start().await?;
    if fixture.lifecycle_state() != Some(SimMtlsInterceptLifecycleState::Live) {
        return Err("initial production StartAllocation did not produce lifecycle Live".to_owned());
    }
    fixture.trace.clear();
    fixture.restart(2, &fixture.ca).await?;
    let trace = fixture.trace.snapshot();
    check_order(&trace)
        .map_err(|cause| format!("{cause}; lifecycle={:?}", fixture.lifecycle.inner.snapshot()))?;
    if check_order(&without_lifecycle_stop(&trace)).is_ok() {
        return Err("negative control deleting lifecycle stop completion stayed green".to_owned());
    }
    if fixture.lifecycle_state() != Some(SimMtlsInterceptLifecycleState::Live) {
        return Err("clean restart did not converge lifecycle back to Live".to_owned());
    }
    Ok(())
}

async fn transient_lifecycle_stop(seed: u64) -> Result<(), String> {
    let fixture = Fixture::new(seed)?;
    fixture.start().await?;
    fixture.trace.clear();
    fixture
        .lifecycle
        .inner
        .inject_stop_failure_once(fixture.alloc.clone(), "seeded lifecycle stop fault");
    if fixture.restart(2, &fixture.ca).await.is_ok() {
        return Err("one-shot lifecycle stop fault did not surface through dispatch".to_owned());
    }
    if fixture.lifecycle_state() != Some(SimMtlsInterceptLifecycleState::TeardownPending)
        || !fixture.slot_is_retained()
        || fixture.trace.snapshot() != vec![TraceEvent::DriverStop]
    {
        return Err(
            "lifecycle stop fault did not retain TeardownPending ownership and old slot".to_owned()
        );
    }
    fixture.trace.clear();
    fixture.restart(3, &fixture.ca).await?;
    check_order(&fixture.trace.snapshot())?;
    if fixture.lifecycle_state() != Some(SimMtlsInterceptLifecycleState::Live) {
        return Err("one retry after lifecycle stop fault did not converge Live".to_owned());
    }
    Ok(())
}

async fn transient_network_teardown(seed: u64) -> Result<(), String> {
    let fixture = Fixture::new(seed)?;
    fixture.start().await?;
    fixture.trace.clear();
    fixture.network.fail_teardown_once();
    if fixture.restart(2, &fixture.ca).await.is_ok() {
        return Err("one-shot network teardown fault did not surface through dispatch".to_owned());
    }
    if fixture.lifecycle_state().is_some()
        || !fixture.slot_is_retained()
        || fixture.trace.snapshot()
            != vec![TraceEvent::DriverStop, TraceEvent::LifecycleStopCompleted]
    {
        return Err(
            "network teardown fault did not retain only the structural slot owner".to_owned()
        );
    }
    fixture.trace.clear();
    fixture.restart(3, &fixture.ca).await?;
    check_order(&fixture.trace.snapshot())?;
    Ok(())
}

async fn replacement_provision_failure(seed: u64) -> Result<(), String> {
    let fixture = Fixture::new(seed)?;
    fixture.start().await?;
    fixture.trace.clear();
    fixture.network.fail_provision_once();
    fixture.restart(2, &fixture.ca).await?;
    let trace = fixture.trace.snapshot();
    if trace.iter().any(|event| {
        matches!(
            event,
            TraceEvent::DriverStartCompleted { .. } | TraceEvent::LifecycleStartCompleted
        )
    }) {
        return Err(format!("replacement provision failure reached a later effect: {trace:?}"));
    }
    Ok(())
}

async fn identity_and_driver_failures(seed: u64) -> Result<(), String> {
    let identity_fixture = Fixture::new(seed)?;
    identity_fixture.start().await?;
    identity_fixture.identity.drop_svid(&identity_fixture.alloc);
    identity_fixture.trace.clear();
    let rejecting_ca = RejectingIssueCa::new(seed);
    if identity_fixture.restart(2, &rejecting_ca).await.is_ok() {
        return Err("identity issuance failure did not surface through dispatch".to_owned());
    }
    if identity_fixture.trace.snapshot().iter().any(|event| {
        matches!(
            event,
            TraceEvent::DriverStartCompleted { .. } | TraceEvent::LifecycleStartCompleted
        )
    }) {
        return Err("identity failure reached replacement driver or lifecycle start".to_owned());
    }

    let driver_fixture = Fixture::new(seed)?;
    driver_fixture.start().await?;
    driver_fixture.trace.clear();
    driver_fixture.driver.fail_next_start();
    driver_fixture.restart(2, &driver_fixture.ca).await?;
    if driver_fixture
        .trace
        .snapshot()
        .iter()
        .any(|event| matches!(event, TraceEvent::LifecycleStartCompleted))
    {
        return Err("driver start failure reached replacement lifecycle start".to_owned());
    }
    Ok(())
}

/// Evaluate the canonical seeded BTR-3 lifecycle-port invariant.
pub async fn evaluate(seed: u64) -> InvariantResult {
    let result: Result<(), String> = async {
        clean(seed).await?;
        transient_lifecycle_stop(seed).await?;
        transient_network_teardown(seed).await?;
        replacement_provision_failure(seed).await?;
        identity_and_driver_failures(seed).await
    }
    .await;
    match result {
        Ok(()) => InvariantResult {
            name: NAME.to_owned(),
            status: InvariantStatus::Pass,
            tick: 3,
            host: HOST.to_owned(),
            cause: None,
        },
        Err(cause) => InvariantResult {
            name: NAME.to_owned(),
            status: InvariantStatus::Fail,
            tick: 3,
            host: HOST.to_owned(),
            cause: Some(format!("seed {seed}: {cause}")),
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test fixtures fail immediately on impossible construction or driven-port failures"
)]
mod tests {
    use super::*;
    use crate::adapters::SimMtlsInterceptLifecycleEvent;

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn fixed_seed_exercises_every_btr3_partition() {
        let result = evaluate(424_242).await;
        assert_eq!(result.status, InvariantStatus::Pass, "{:?}", result.cause);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn teardown_pending_is_not_treated_as_absence() {
        let fixture = Fixture::new(424_242).expect("fixture builds");
        fixture.start().await.expect("initial start");
        fixture.lifecycle.inner.inject_stop_failure_once(fixture.alloc.clone(), "retain owner");
        fixture.restart(2, &fixture.ca).await.expect_err("fault must surface");
        assert_eq!(
            fixture.lifecycle_state(),
            Some(SimMtlsInterceptLifecycleState::TeardownPending)
        );
        assert!(fixture.slot_is_retained());
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn lifecycle_adapter_preserves_absence_and_prior_teardown_event_shapes() {
        let fixture = Fixture::new(424_242).expect("fixture builds");
        let lifecycle = &fixture.lifecycle.inner;

        lifecycle.inject_stop_failure_once(fixture.alloc.clone(), "scripted prior teardown fault");
        lifecycle.stop_alloc(&fixture.alloc).await.expect("absent stop is idempotent");
        assert!(matches!(
            lifecycle.snapshot().events.as_slice(),
            [SimMtlsInterceptLifecycleEvent::StopCompleted { prior: None, .. }]
        ));

        lifecycle.start_alloc(&fixture.spec()).await.expect("first start is live");
        lifecycle
            .start_alloc(&fixture.spec())
            .await
            .expect_err("repeat start must surface prior teardown failure");
        let snapshot = lifecycle.snapshot();
        assert_eq!(
            snapshot.allocations.get(&fixture.alloc),
            Some(&SimMtlsInterceptLifecycleState::TeardownPending)
        );
        assert!(matches!(
            snapshot.events.last(),
            Some(SimMtlsInterceptLifecycleEvent::StartPriorTeardownFailed { alloc_id, failures })
                if alloc_id == &fixture.alloc && failures == &["scripted prior teardown fault"]
        ));
        assert!(
            !matches!(
                snapshot.events.last(),
                Some(SimMtlsInterceptLifecycleEvent::StopFailed { .. })
            ),
            "repeat start emits only its StartPriorTeardownFailed outcome"
        );
    }
}
