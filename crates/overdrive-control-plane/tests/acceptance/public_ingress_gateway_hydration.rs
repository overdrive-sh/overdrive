//! Gateway-SVID hydration through the production `HydrationContext` composition.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "acceptance fixture preconditions and Contract Shape metadata"
)]

use std::sync::Arc;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::gateway_identity::GatewayIdentityEpoch;
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{HydrateError, HydrationContext, Reconciler, TargetResource};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_reconcilers::GatewaySvidLifecycle;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::{
    SimHeldSvidView, SimListenerFacts, SimServiceVipView, SimWorkflowLiveSet,
};
use overdrive_store_local::LocalIntentStore;

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER GatewaySvidLifecycle production hydration"]
async fn disabled_and_malformed_targets_hydrate_through_the_exact_gateway_identity_ports() {
    let root = tempfile::tempdir().expect("isolated hydration composition");
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(root.path()).expect("runtime");
    let intent_path = root.path().join("intent.redb");
    let intent = Arc::new(LocalIntentStore::open(&intent_path).expect("intent store"));
    let observation: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("writer-1").expect("node ID"), 51));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&intent) as Arc<dyn IntentStore>
    );
    let state = AppState::new(
        intent,
        intent_path.clone(),
        observation,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(51),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").expect("node ID"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    let listener = SimListenerFacts::new(std::collections::BTreeMap::new());
    let vip = SimServiceVipView::new(std::collections::BTreeMap::new());
    let workflows = SimWorkflowLiveSet::new(std::collections::BTreeSet::new());
    let held = SimHeldSvidView::new(std::collections::BTreeMap::new());
    let context = HydrationContext {
        intent_store: state.store.as_ref(),
        observation_store: state.obs.as_ref(),
        drivers: state.drivers.as_ref(),
        vm_host_state: state.vm_host_state.as_ref(),
        listener_facts: &listener,
        service_vip_view: &vip,
        workflow_live_set: &workflows,
        held_svid_view: &held,
        gateway_frontend_demand: state.gateway.demand().read(),
        gateway_identity_desired: state.gateway.identity_desired(),
        gateway_identity_current: state.gateway.identity_current(),
        node_id: &state.node_id,
        host_ipv4: state.host_ipv4,
        intent_redb_path: &intent_path,
    };
    let reconciler = GatewaySvidLifecycle::canonical();
    let target = TargetResource::new("node/writer-1").expect("gateway target");
    let desired = reconciler.hydrate_desired(&context, &target).await.expect("desired");
    let actual = reconciler.hydrate_actual(&context, &target).await.expect("actual");
    assert_eq!(desired.desired.epoch, GatewayIdentityEpoch::first());
    assert!(desired.desired.spiffe_id.is_none());
    assert!(desired.actual.is_none());
    assert!(actual.actual.is_none());
    assert!(!actual.ever_issued);

    let malformed = TargetResource::new("workload/writer-1").expect("wrong target kind");
    assert!(matches!(
        reconciler.hydrate_desired(&context, &malformed).await,
        Err(HydrateError::TargetShape(_))
    ));
    assert!(matches!(
        reconciler.hydrate_actual(&context, &malformed).await,
        Err(HydrateError::TargetShape(_))
    ));
}
