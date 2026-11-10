//! VM-path dispatch fixture: drives the action shim through the production
//! guest-network seam (`dispatch_with_guest_network_provisioner_for_test`)
//! over an `AppState` built the way the integration fixtures build it.
//!
//! Every admitted allocation is a VM allocation; the guest address pool and
//! the shared guest-network owner are the only network composition. The
//! fixture:
//!
//! - converges the owner (so its bound flow ports reach the shim's state),
//! - opens the EXEC gate (so activation is not withheld),
//! - exposes `state` so a test reads the lifecycle bus, broker, obs, store,
//!   identity and driver index through the same fields production reads.
//!
//! `#[path]`-included by `tests/acceptance.rs` and `tests/integration.rs` as
//! `vm_dispatch`; it relies on the sibling `serve_ports` module of each
//! entrypoint for the worker and the guest address pool.

#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]

use std::net::Ipv4Addr;
use std::sync::Arc;

use overdrive_control_plane::action_shim::{
    ShimError, dispatch_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::guest_network::SharedGuestNetworkOwner;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{
    AppState, noop_heartbeat, test_default_allocator, test_empty_listener_facts,
};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::{Driver, DriverRegistry};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// The ports a dispatch fixture is composed over. Every port defaults to its
/// `Sim*` adapter; a test replaces only the port whose behaviour it scripts.
pub struct VmDispatchPorts {
    pub driver: Arc<dyn Driver>,
    pub obs: Arc<dyn ObservationStore>,
    pub clock: Arc<dyn Clock>,
    pub dataplane: Arc<dyn Dataplane>,
    pub ca: Arc<dyn Ca>,
    pub identity: Arc<IdentityMgr>,
    pub node_id: NodeId,
    /// The one shared guest-network owner: the seam's provisioner and the
    /// `AppState` owner are this single instance.
    pub owner: Arc<dyn SharedGuestNetworkOwner>,
    pub vm_host_state: Arc<dyn VmHostState>,
}

impl VmDispatchPorts {
    /// Sim adapters for every port except `driver`, which the test supplies.
    #[must_use]
    pub fn sim(driver: Arc<dyn Driver>) -> Self {
        let node_id = NodeId::new("local").expect("valid node id");
        Self {
            driver,
            obs: Arc::new(SimObservationStore::single_peer(node_id.clone(), 0)),
            clock: Arc::new(SimClock::new()),
            dataplane: Arc::new(SimDataplane::new()),
            ca: Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            identity: Arc::new(IdentityMgr::new(None)),
            node_id,
            owner: Arc::new(SimSharedGuestNetworkOwner::default()),
            vm_host_state: Arc::new(SimVmHostState::new()),
        }
    }
}

/// One booted dispatch composition.
pub struct VmDispatchFixture {
    pub state: Arc<AppState>,
    /// The owner the seam provisions through (also `state.shared_guest_network`).
    pub owner: Arc<dyn SharedGuestNetworkOwner>,
    pub tmp: TempDir,
    _supervisor: Arc<GuestNetworkExecSupervisor>,
    _gate: Arc<GuestNetworkExecGate>,
}

impl VmDispatchFixture {
    /// Boot the composition over `ports`.
    pub async fn boot(ports: VmDispatchPorts) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(noop_heartbeat()).await.expect("register heartbeat");
        ports.owner.converge_shared().await.expect("the fixture's shared owner converges");
        let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        assert!(supervisor.open_after_boot(), "the fixture's gate leaves BootClosed once");
        // The node's shared mTLS listener owner is published once at boot, as
        // `run_server` does, so an admitted allocation can attach to it.
        let worker = super::serve_ports::worker();
        worker.start_shared_owner().await.expect("the fixture's shared mTLS owner publishes");
        let allocator = test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mut state = AppState::new(
            store,
            store_path,
            ports.obs,
            Arc::new(runtime),
            ports.driver,
            ports.clock,
            ports.dataplane,
            ports.ca,
            ports.identity,
            ports.node_id,
            allocator,
            test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&ports.owner),
            Arc::clone(&gate),
            super::serve_ports::pool(),
        );
        state.vm_host_state = ports.vm_host_state;
        Self {
            state: Arc::new(state),
            owner: ports.owner,
            tmp,
            _supervisor: supervisor,
            _gate: gate,
        }
    }

    /// Dispatch `actions` through the production action composition.
    pub async fn dispatch(
        &self,
        actions: Vec<Action>,
        tick: &TickContext,
    ) -> Result<(), ShimError> {
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            actions,
            self.state.as_ref(),
            tick,
            self.owner.as_ref(),
        ))
        .await
    }

    /// The registry of drivers the shim resolves, replaced wholesale for a
    /// composition that needs more than the single driver `boot` wraps.
    pub fn with_drivers(mut self, drivers: DriverRegistry) -> Self {
        let state = Arc::get_mut(&mut self.state).expect("fixture state is not yet shared");
        state.drivers = Arc::new(drivers);
        self
    }
}

/// A converged `Sim` shared guest-network owner plus an opened EXEC gate, for
/// tests that build their own `AppState` and drive the tick seam. The owner is
/// converged BEFORE `AppState::new` so its flow ports reach the state; the
/// gate is opened so guest activation is not withheld.
pub async fn converged_owner_and_open_gate()
-> (Arc<SimSharedGuestNetworkOwner>, Arc<GuestNetworkExecGate>, Arc<GuestNetworkExecSupervisor>) {
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    owner.converge_shared().await.expect("the fixture's shared owner converges");
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let gate = wiring.gate();
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot(), "the fixture's gate leaves BootClosed once");
    (owner, gate, supervisor)
}

/// Start the node-shared listener owner of `state`'s mTLS worker, the way the
/// boot composition does before any allocation registers. A tick-driven test
/// that builds its own `AppState` calls this once so guest allocations are not
/// refused with `shared mTLS owner unavailable`.
pub async fn start_shared_mtls_owner(state: &AppState) {
    state
        .mtls_worker
        .start_shared_owner()
        .await
        .expect("the fixture's shared mTLS listener owner starts");
}
