//! Acceptance properties for the GH #295 control-plane guest-network contract.

#![allow(clippy::doc_markdown)]

use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::dispatch_with_guest_network_provisioner_for_test;
use overdrive_control_plane::guest_network::{
    GuestNetworkOperation, GuestNetworkScratchComplement, GuestNetworkScratchCount,
    SharedGuestNetworkOwner,
};
use overdrive_control_plane::{AppState, noop_heartbeat};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

fn scratch_complement(counts: &[Option<u32>]) -> GuestNetworkScratchComplement {
    assert_eq!(counts.len(), 15);
    let count = |index: usize| {
        counts[index]
            .map_or(GuestNetworkScratchCount::Unavailable, GuestNetworkScratchCount::Observed)
    };
    GuestNetworkScratchComplement {
        bridges: count(0),
        taps: count(1),
        endpoint_maps: count(2),
        counter_maps: count(3),
        endpoint_entries: count(4),
        tcx_programs: count(5),
        tcx_links: count(6),
        endpoint_map_pins: count(7),
        counter_map_pins: count(8),
        tcx_link_pins: count(9),
        bridge_guard_tables: count(10),
        bridge_guard_chains: count(11),
        bridge_guard_sets: count(12),
        bridge_guard_rules: count(13),
        bridge_guard_members: count(14),
    }
}

proptest! {
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER step for GH #295 scratch complement semantics"]
    fn scratch_complement_never_fabricates_zero(
        counts in prop::collection::vec(prop::option::of(0_u32..4), 15..=15),
    ) {
        let complement = scratch_complement(&counts);
        let all_observed = counts.iter().all(Option::is_some);
        let all_zero = counts.iter().all(|count| *count == Some(0));

        prop_assert_eq!(complement.is_fully_observed(), all_observed);
        prop_assert_eq!(complement.is_empty(), all_zero);
        prop_assert!(!complement.is_empty() || complement.is_fully_observed());
    }
}

async fn state(tmp: &TempDir, driver: Arc<SimDriver>, clock: Arc<SimClock>) -> AppState {
    let mut runtime =
        overdrive_control_plane::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
            tmp.path(),
        )
        .expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register reconciler");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("nd295-node").expect("node id"), 0));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver as Arc<dyn Driver>,
        clock,
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(295)))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("nd295-node").expect("node id"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn spec(name: &str) -> AllocationSpec {
    let alloc = AllocationId::new(name).expect("allocation id");
    AllocationSpec {
        alloc,
        identity: SpiffeId::new(&format!("spiffe://overdrive.local/workload/nd295/alloc/{name}"))
            .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/conformance/kernel"),
            rootfs: PathBuf::from("/conformance/rootfs.ext4"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: [8080_u16, 8080, 53]
            .into_iter()
            .map(|port| NonZeroU16::new(port).expect("non-zero listener port"))
            .collect(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn start_action(name: &str) -> Action {
    Action::StartAllocation {
        alloc_id: AllocationId::new(name).expect("allocation id"),
        workload_id: WorkloadId::new(&format!("workload-{name}")).expect("workload id"),
        node_id: NodeId::new("nd295-node").expect("node id"),
        spec: spec(name),
        kind: WorkloadKind::Service,
    }
}

fn assignment(spec: &AllocationSpec) -> GuestNetworkAssignment {
    GuestNetworkAssignment {
        address: spec.workload_addr.expect("canonical address"),
        tap: spec.guest_tap.clone().expect("guest TAP"),
        mac: spec.guest_mac.expect("guest MAC"),
        gateway: spec.guest_gateway.expect("guest gateway"),
        prefix: spec.guest_prefix_len.expect("guest prefix"),
        dns: spec.guest_dns.expect("guest DNS"),
    }
}

fn tick(counter: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(counter)),
        tick: counter,
        deadline: now + Duration::from_secs(1),
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step for GH #295 guest-network action-owner single cut"]
async fn provision_refusal_stops_before_driver_start_and_preserves_the_typed_owner_cause() {
    let tmp = TempDir::new().expect("tempdir");
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let state = state(&tmp, Arc::clone(&driver), Arc::new(SimClock::new())).await;
    let owner = SimSharedGuestNetworkOwner::default();
    owner.script_provision_failure(true);

    let error = dispatch_with_guest_network_provisioner_for_test(
        vec![start_action("nd295-provision-refused")],
        &state,
        &tick(1),
        &owner,
    )
    .await
    .expect_err("typed provision refusal reaches the production action owner");

    assert!(matches!(
        error,
        overdrive_control_plane::action_shim::ShimError::GuestNetwork(
            overdrive_control_plane::guest_network::GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapCreate,
                ..
            }
        )
    ));
    assert!(driver.started_specs().is_empty(), "the VMM driver is never entered");
    assert_eq!(driver.live_count(), 0);
    assert_eq!(owner.calls(), [GuestNetworkOperation::TapCreate]);
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step for GH #295 release-last action-owner single cut"]
async fn teardown_failure_holds_the_lease_until_retry_completes_then_allows_exact_address_reuse() {
    let tmp = TempDir::new().expect("tempdir");
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let state = state(&tmp, Arc::clone(&driver), Arc::new(SimClock::new())).await;
    let owner = SimSharedGuestNetworkOwner::default();
    let owner_port: &dyn SharedGuestNetworkOwner = &owner;

    dispatch_with_guest_network_provisioner_for_test(
        vec![start_action("nd295-predecessor")],
        &state,
        &tick(1),
        owner_port,
    )
    .await
    .expect("predecessor starts with one grouped network assignment");
    let predecessor = assignment(&driver.started_specs()[0]);

    owner.script_teardown_failure(true);
    let stop_error = dispatch_with_guest_network_provisioner_for_test(
        vec![Action::StopAllocation {
            alloc_id: AllocationId::new("nd295-predecessor").expect("allocation id"),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        }],
        &state,
        &tick(2),
        owner_port,
    )
    .await
    .expect_err("effect-first teardown refusal retains the lease");
    assert!(matches!(
        stop_error,
        overdrive_control_plane::action_shim::ShimError::GuestNetwork(
            overdrive_control_plane::guest_network::GuestNetworkError::Io {
                operation: GuestNetworkOperation::TapDelete,
                ..
            }
        )
    ));

    dispatch_with_guest_network_provisioner_for_test(
        vec![start_action("nd295-while-held")],
        &state,
        &tick(3),
        owner_port,
    )
    .await
    .expect("unrelated allocation remains startable");
    let while_held = assignment(&driver.started_specs()[1]);
    assert_ne!(while_held.address, predecessor.address);

    owner.script_teardown_failure(false);
    dispatch_with_guest_network_provisioner_for_test(
        vec![Action::StopAllocation {
            alloc_id: AllocationId::new("nd295-predecessor").expect("allocation id"),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        }],
        &state,
        &tick(4),
        owner_port,
    )
    .await
    .expect("retry completes the retained teardown before release");
    dispatch_with_guest_network_provisioner_for_test(
        vec![start_action("nd295-successor")],
        &state,
        &tick(5),
        owner_port,
    )
    .await
    .expect("successor starts after predecessor complement is empty");
    let successor = assignment(&driver.started_specs()[2]);
    assert_eq!(successor.address, predecessor.address);
    assert_eq!(successor.tap, predecessor.tap);
    assert_eq!(successor.mac, predecessor.mac);
}
