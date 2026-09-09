//! ADR-0098 terminal cleanup through the production action-shim dispatch seam.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{
    AllocDriverIndex, WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_vm_tap_plan, derive_workload_netns_plan, responder_addr_for_slot,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::DriverRegistry;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

#[derive(Default)]
struct RecordingProvisioner {
    teardowns: parking_lot::Mutex<Vec<WorkloadNetnsPlan>>,
}

impl WorkloadNetworkProvisioner for RecordingProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.teardowns.lock().push(workload.clone());
        Ok(())
    }
}

async fn seed_running_row(
    obs: &dyn ObservationStore,
    alloc_id: &AllocationId,
    workload_addr: Ipv4Addr,
) {
    let node_id = NodeId::new("adr-0098-node").expect("valid node");
    obs.write_alloc_lifecycle(
        AllocStatusRow {
            alloc_id: alloc_id.clone(),
            workload_id: WorkloadId::new("adr-0098-service").expect("valid workload"),
            node_id: node_id.clone(),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: 0, writer: node_id },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
            workload_addr: Some(workload_addr),
            last_terminated: None,
            restart_count: 0,
        },
        TransitionSource::Reconciler,
    )
    .await
    .expect("seed Running row");
}

/// ADR-0098 D4: a genuine terminal with a missing ephemeral binding reclaims
/// only the exact plan whose prior Running address proves ownership. Both the
/// allocation-network Exec transit carve and VM guest carve reach this same
/// production dispatcher path.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn terminal_cleanup_recovers_only_exact_unheld_slot_from_prior_running_address() {
    for (suffix, workload_addr) in [("exec", false), ("vm", true)] {
        let slot = NetSlot::new(23).expect("valid slot");
        let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
        let assigned = if workload_addr {
            derive_vm_tap_plan(slot, responder_addr_for_slot(slot)).guest_addr
        } else {
            plan.workload_addr
        };
        let alloc_id = AllocationId::new(&format!("adr-0098-{suffix}")).expect("valid alloc");
        let obs = Arc::new(SimObservationStore::single_peer(
            NodeId::new("adr-0098-store").expect("valid node"),
            0,
        ));
        seed_running_row(obs.as_ref(), &alloc_id, assigned).await;

        let tempdir = TempDir::new().expect("tempdir");
        let intent: Arc<dyn IntentStore> = Arc::new(
            LocalIntentStore::open(tempdir.path().join("intent.redb")).expect("open intent"),
        );
        let vip_allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
            VipRange::default(),
            intent,
        )));
        let provisioner = RecordingProvisioner::default();
        let now = Instant::now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(2)),
            tick: 1,
            deadline: now + Duration::from_secs(1),
        };
        let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(4);
        let drivers = DriverRegistry::new();

        dispatch_with_network_provisioner(
            vec![Action::FinalizeFailed { alloc_id: alloc_id.clone(), terminal: None }],
            &drivers,
            &AllocDriverIndex::default(),
            obs.as_ref(),
            &SimDataplane::new(),
            &SimCa::new(Arc::new(SimEntropy::new(0))),
            &SimClock::new(),
            &IdentityMgr::new(None),
            &lifecycle_tx,
            &tick,
            &NodeId::new("adr-0098-writer").expect("valid node"),
            vip_allocator,
            &parking_lot::Mutex::new(EvaluationBroker::new()),
            None,
            None,
            &NetSlotAllocator::new(),
            &provisioner,
            &SimVmHostState::new(),
        )
        .await
        .expect("genuine terminal cleanup and write succeed");

        assert_eq!(provisioner.teardowns.lock().as_slice(), &[plan]);
        let row = obs
            .alloc_status_rows()
            .await
            .expect("read terminal row")
            .into_iter()
            .find(|row| row.alloc_id == alloc_id)
            .expect("terminal row exists");
        assert_eq!(row.state, AllocState::Failed);
    }
}
