//! Review D2 — populated-host `vm_reclamation` `hydrate_actual` equivalence.
//!
//! **CONTRACT_SHAPE: unbounded-preservation (equivalence).** The 02-03
//! characterization golden pins `vm_reclamation` at EMPTY `State` (the broad
//! Exec-only fixture never populates a `Vm` driver or a `VmHostState`
//! observation), and the sibling `hydration_move_equivalence` bar checks only
//! the wrapped `AnyState` *variant*, not its contents. So `VmReclamation`'s
//! non-trivial `hydrate_actual` — which reads `VmHostState::observe()` FIRST,
//! then joins the `Vm`-driver supervision set LAST into a `SupervisionSet`
//! (`service_lifecycle`-style two-surface projection, ADR-0083 §D7 /
//! `brief.md` §105a.2) — is otherwise unverified post-move.
//!
//! This test closes that gap: it seeds a populated `SimVmHostState` (all three
//! observation surfaces) plus a `Vm`-kind `SimDriver` holding ONE live
//! allocation, drives `AnyReconciler::hydrate_actual` for `vm_reclamation`
//! through a directly-built `HydrationContext` (the same borrow bundle the
//! runtime lends per tick), and asserts the hydrated `VmReclamationState`
//! equals an EXPLICIT, hand-pinned expected projection — never re-read from the
//! sim, so the assertion is not self-referential. The projection is
//! deliberately non-trivial: the host surfaces name three allocations
//! (`alloc-scope-0`, `alloc-live-0`, `alloc-stranded-0`) while only
//! `alloc-live-0` is supervised, proving the supervision set is derived from
//! the DRIVER's live set independently of what the host observes.

// `CONTRACT_SHAPE` prose in the module doc trips `doc_markdown`; match the
// sibling equivalence tests (`hydration_move_equivalence`) rather than churn it.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::id::{AllocationId, NodeId, SpiffeId};
use overdrive_core::reconcilers::{HydrationContext, TargetResource};
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, DriverType, ExecPayload, Resources,
};
use overdrive_core::traits::vm_host_state::{ScopeFacts, VmHostObservation};
use overdrive_reconcilers::{
    AnyReconciler, AnyState, SupervisionSet, VmReclamation, VmReclamationState,
};
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::{
    SimHeldSvidView, SimListenerFacts, SimServiceVipView, SimWorkflowLiveSet,
};
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

/// Minimal `AllocationSpec` for `SimDriver::start` (the payload kind is
/// irrelevant — `SimDriver` records supervision by `DriverType`, not payload).
fn vm_spec(name: &str) -> AllocationSpec {
    AllocationSpec {
        alloc: aid(name),
        identity: SpiffeId::from_str("spiffe://overdrive.local/test/vm").expect("valid SpiffeId"),
        driver: DriverPayload::Exec(ExecPayload { command: "/bin/true".to_owned(), args: vec![] }),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

/// S-ROH (review D2) — the populated-host `hydrate_actual` equivalence bar.
#[tokio::test]
async fn vm_reclamation_hydrate_actual_projects_host_observation_and_supervision_set() {
    let tmp = TempDir::new().expect("tmpdir");

    // --- intent + observation stores (unread by hydrate_actual, but the
    //     HydrationContext borrow bundle requires them) ---
    let intent_path = tmp.path().join("intent.redb");
    let intent = LocalIntentStore::open(&intent_path).expect("LocalIntentStore::open");
    let obs = SimObservationStore::single_peer(node_id("local"), 0);

    // --- the Vm driver: ONE live allocation supervised ---
    let vm_driver: Arc<SimDriver> = Arc::new(SimDriver::new(DriverType::Vm));
    vm_driver.start(&vm_spec("alloc-live-0")).await.expect("Vm driver start");
    let mut drivers = DriverRegistry::new();
    drivers.insert(Arc::clone(&vm_driver) as Arc<dyn Driver>);

    // --- the host observation: three surfaces, three DISTINCT allocations ---
    // (only `alloc-live-0` is BOTH host-present and supervised.)
    let sim_host = SimVmHostState::new();
    sim_host.set_scope(aid("alloc-scope-0"), BTreeSet::from([111u32, 222u32]));
    sim_host.set_run_dir(aid("alloc-live-0"));
    sim_host.set_run_dir(aid("alloc-stranded-0"));
    sim_host.set_clone(aid("alloc-live-0"), PathBuf::from("/sim/clones/alloc-live-0.img"));

    // --- the four narrow read-ports (unread by hydrate_actual — empty) ---
    let listener_facts = SimListenerFacts::new(BTreeMap::new());
    let service_vip_view = SimServiceVipView::new(BTreeMap::new());
    let workflow_live_set = SimWorkflowLiveSet::new(BTreeSet::new());
    let held_svid_view = SimHeldSvidView::new(BTreeMap::new());
    let node = node_id("local");

    let ctx = HydrationContext {
        intent_store: &intent,
        observation_store: &obs,
        drivers: &drivers,
        vm_host_state: &sim_host,
        listener_facts: &listener_facts,
        service_vip_view: &service_vip_view,
        workflow_live_set: &workflow_live_set,
        held_svid_view: &held_svid_view,
        node_id: &node,
        host_ipv4: std::net::Ipv4Addr::LOCALHOST,
        intent_redb_path: &intent_path,
    };

    let reconciler = AnyReconciler::VmReclamation(VmReclamation::new());
    let target = TargetResource::new("node/local").expect("valid target");
    let any_state = reconciler.hydrate_actual(&ctx, &target).await.expect("hydrate_actual");
    let AnyState::VmReclamation(vm_state) = any_state else {
        panic!("hydrate_actual for vm_reclamation must wrap into AnyState::VmReclamation");
    };

    // --- explicit, hand-pinned expected projection (NOT re-read from the sim) ---
    let expected = VmReclamationState {
        // `hydrate_actual` owns only the `actual` half; the desired-side
        // `allocations` join stays empty (it is the `hydrate_desired` arm's).
        allocations: BTreeMap::new(),
        host: VmHostObservation {
            scopes: BTreeMap::from([(
                aid("alloc-scope-0"),
                ScopeFacts { pids: BTreeSet::from([111u32, 222u32]) },
            )]),
            run_dirs: BTreeSet::from([aid("alloc-live-0"), aid("alloc-stranded-0")]),
            clones: BTreeMap::from([(
                aid("alloc-live-0"),
                PathBuf::from("/sim/clones/alloc-live-0.img"),
            )]),
        },
        // A SUCCESSFUL enumeration holding exactly the one started allocation —
        // NOT the host surfaces. `alloc-scope-0` / `alloc-stranded-0` are
        // host-present but unsupervised.
        supervision: SupervisionSet::Observed(BTreeSet::from([aid("alloc-live-0")])),
    };
    assert_eq!(
        vm_state, expected,
        "hydrate_actual must project observe() into `host` and the Vm-driver live set into \
         `supervision`, leaving `allocations` empty",
    );

    // Explicit supervision-projection semantics (the load-bearing kill
    // predicate, `brief.md` §105a.3): a supervised alloc is NOT authorised
    // for reclamation; an unsupervised host-present alloc IS.
    assert!(
        !vm_state.supervision.reclamation_authorised(&aid("alloc-live-0")),
        "the supervised live allocation must not be authorised for reclamation",
    );
    assert!(
        vm_state.supervision.reclamation_authorised(&aid("alloc-stranded-0")),
        "an unsupervised host-present allocation must be authorised for reclamation",
    );
}
