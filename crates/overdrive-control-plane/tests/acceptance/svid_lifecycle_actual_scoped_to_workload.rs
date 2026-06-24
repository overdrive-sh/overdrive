//! BUG-1 regression witness — the `svid-lifecycle` reconciler's `actual` (held)
//! side is scoped to the TARGET workload, so a convergence tick for one workload
//! never drops a DIFFERENT workload's still-live SVID.
//!
//! Root cause (pre-fix): `svid-lifecycle` is enqueued with a workload-scoped
//! target `job/<workload_id>` (ADR-0067 D5b). The DESIRED side hydrates only that
//! workload's Running allocs, but the ACTUAL (held) side read the GLOBAL
//! `IdentityMgr::held_snapshot()` unfiltered. The reconciler's `¬running ∧ held →
//! DropSvid` loop then dropped every held alloc absent from that ONE workload's
//! desired set — so a `payments` tick emitted `DropSvid` for `inventory`'s
//! still-live SVID. The pure reconciler is correct given correctly-scoped inputs;
//! the bug was the runtime feeding it a global `actual` against a workload-scoped
//! `desired`.
//!
//! The fix filters the held snapshot to entries whose SPIFFE id equals
//! `SpiffeId::for_allocation(target_workload, alloc_id)` — symmetry with the
//! desired side.
//!
//! Port-to-port: the driving port is `run_convergence_tick` for `svid-lifecycle`;
//! the observable outcome is the `IdentityMgr` held set after a `job/payments`
//! tick (asserted at the held-snapshot boundary). Pre-fix `inventory`'s alloc is
//! dropped from the held set; post-fix it survives.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle;
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

const SVID_LIFECYCLE: &str = "svid-lifecycle";
const NODE_NAME: &str = "host-0";

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn wid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn svid_target(w: &WorkloadId) -> TargetResource {
    TargetResource::new(&format!("job/{w}")).expect("valid target")
}
fn svid_reconciler_name() -> ReconcilerName {
    ReconcilerName::new(<SvidLifecycle as Reconciler>::NAME).expect("valid reconciler name")
}

async fn build_state(
    tmp: &TempDir,
    clock: Arc<SimClock>,
    obs: Arc<dyn ObservationStore>,
    identity: Arc<IdentityMgr>,
) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(svid_lifecycle()).await.expect("register svid-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    // A normal SimCa — issuance SUCCEEDS, so the first svid-lifecycle tick for an
    // unheld Running alloc emits IssueSvid → shim mints + holds.
    let ca: Arc<dyn Ca> = Arc::new(SimCa::new(Arc::new(SimEntropy::new(0))));
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(SimDataplane::new()),
        ca,
        identity,
        nid(NODE_NAME),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

async fn write_running_alloc(state: &AppState, w: &WorkloadId, a: &AllocationId, counter: u64) {
    let row = AllocStatusRow {
        alloc_id: a.clone(),
        workload_id: w.clone(),
        node_id: nid(NODE_NAME),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: nid(NODE_NAME) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
}

/// Drive ONE convergence tick for `target` (svid-lifecycle), draining any other
/// pending evals first so this tick runs in isolation. Tolerates no Err — every
/// tick in this test issues or no-ops; none fail.
async fn tick_svid(state: &AppState, target: &TargetResource, now: std::time::Instant, n: u64) {
    let deadline = now + Duration::from_millis(100);
    // Drain whatever is pending so we control exactly which eval runs.
    {
        let mut broker = state.runtime.broker();
        let _ = broker.drain_pending();
    }
    state
        .runtime
        .broker()
        .submit(Evaluation { reconciler: svid_reconciler_name(), target: target.clone() });
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    for eval in pending {
        if eval.reconciler.as_str() != SVID_LIFECYCLE {
            continue;
        }
        run_convergence_tick(state, &eval.reconciler, &eval.target, now, n, deadline)
            .await
            .expect("convergence tick must not panic");
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
}

/// BUG-1 — a convergence tick for `job/payments` must NOT drop `inventory`'s
/// still-live SVID. Both workloads each hold an SVID; re-ticking `payments`
/// (both still Running) leaves `inventory`'s held entry intact.
#[tokio::test]
async fn payments_tick_does_not_drop_inventory_held_svid() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    // ONE IdentityMgr holds the node's GLOBAL held set — both workloads' SVIDs
    // land here. The bug is that the actual-side hydration read this whole set
    // unfiltered against a single workload's desired set.
    let identity = Arc::new(IdentityMgr::new(None));
    let state =
        build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs), Arc::clone(&identity)).await;

    let payments = wid("payments");
    let inventory = wid("inventory");
    let payments_alloc = aid("payments-0");
    let inventory_alloc = aid("inventory-0");
    let payments_target = svid_target(&payments);
    let inventory_target = svid_target(&inventory);

    // Both workloads have one Running alloc.
    write_running_alloc(&state, &payments, &payments_alloc, 1).await;
    write_running_alloc(&state, &inventory, &inventory_alloc, 1).await;

    let base = std::time::Instant::now();

    // Drive issuance so BOTH workloads hold an SVID: a Running + unheld +
    // no-retry alloc emits IssueSvid → shim → IdentityMgr holds it.
    tick_svid(&state, &payments_target, base, 0).await;
    tick_svid(&state, &inventory_target, base, 1).await;

    let held_after_issue = state.identity.held_snapshot();
    assert!(
        held_after_issue.contains_key(&payments_alloc),
        "payments alloc must hold an SVID after its issuance tick"
    );
    assert!(
        held_after_issue.contains_key(&inventory_alloc),
        "inventory alloc must hold an SVID after its issuance tick"
    );

    // Re-run a svid-lifecycle tick for `job/payments` ONLY. Both allocs are
    // STILL Running, so nothing should be dropped. Pre-fix: the actual side read
    // the global held set, saw `inventory-0` absent from `payments`'s desired set,
    // and emitted DropSvid for it.
    tick_svid(&state, &payments_target, base, 2).await;

    let held_after_payments_tick = state.identity.held_snapshot();
    assert!(
        held_after_payments_tick.contains_key(&inventory_alloc),
        "BUG-1: a payments-scoped tick must NOT drop inventory's still-live SVID \
         (pre-fix the global actual fed the DropSvid loop every other workload's held entry)"
    );
    // And payments' own SVID is unaffected.
    assert!(
        held_after_payments_tick.contains_key(&payments_alloc),
        "the payments tick must leave payments' own held SVID intact"
    );
}
