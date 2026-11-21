//! workload-spiffe-identity D1 / D2 (ADR-0344, ADR-0345) — the SVID an
//! allocation holds names its WORKLOAD, and two allocations of one workload each
//! keep their own SVID under that one identity.
//!
//! Port-to-port: the driving port is `run_convergence_tick` for
//! `svid-lifecycle` (as `svid_lifecycle_actual_scoped_to_workload`); the
//! observable outcomes are the `IdentityMgr` held set (one entry per
//! `AllocationId`, carrying the identity) and the `issued_certificates` audit
//! rows (one row, one serial, per issuance). A serve restart is a fresh
//! `IdentityMgr` over the same observation store: the in-memory held set is
//! lost, the audit rows survive, and the boot convergence re-issues.
//!
//! RED today: an allocation's identity is the per-allocation
//! `spiffe://overdrive.local/workload/<w>/alloc/<a>`; the per-allocation SVID
//! (own serial) assertions already hold and are checked first, so the matched
//! failure is the identity.
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_reconcilers::svid_lifecycle::SvidLifecycle;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use overdrive_testing::placement;

placement!(host_pure);

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
    TargetResource::new(&format!("workload/{w}")).expect("valid target")
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
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
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
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
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
        last_terminated: None,
        restart_count: 0,
    };
    state
        .obs
        .write_alloc_lifecycle(
            row,
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("write alloc row");
}

/// Drive ONE convergence tick for `target` (svid-lifecycle), draining any other
/// pending evals first so this tick runs in isolation. Tolerates no Err — every
/// tick in this test issues or no-ops; none fail.
async fn tick_svid(state: &AppState, target: &TargetResource, now: std::time::Instant, n: u64) {
    let deadline = now + Duration::from_millis(100);
    // Drain whatever is pending so we control exactly which eval runs.
    {
        let mut broker = state.runtime.broker();
        let _ = broker.drain_pending(
            usize::MAX,
            &std::collections::BTreeSet::new(),
            now,
            overdrive_core::UnixInstant::from_unix_duration(std::time::Duration::ZERO),
        );
    }
    state.runtime.submit_evaluation(
        Evaluation { reconciler: svid_reconciler_name(), target: target.clone() },
        now,
        overdrive_core::eval_broker::EvaluationEligibility::Immediate,
    );
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending(
            usize::MAX,
            &std::collections::BTreeSet::new(),
            now,
            overdrive_core::UnixInstant::from_unix_duration(std::time::Duration::ZERO),
        )
    };
    for (eval, _) in pending {
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

const PAYMENTS_IDENTITY: &str = "spiffe://overdrive.local/workload/payments";

fn held_identities(
    identity: &IdentityMgr,
    allocs: &[&AllocationId],
) -> Vec<(AllocationId, String)> {
    let held = identity.held_snapshot();
    allocs
        .iter()
        .map(|alloc| {
            let facts = held
                .get(*alloc)
                .unwrap_or_else(|| panic!("allocation {alloc} must hold an SVID: {held:?}"));
            ((*alloc).clone(), facts.spiffe_id.as_str().to_owned())
        })
        .collect()
}

async fn serials(state: &AppState) -> std::collections::BTreeSet<String> {
    state
        .obs
        .issued_certificate_rows()
        .await
        .expect("read issued_certificates")
        .iter()
        .map(|row| row.serial.to_string())
        .collect()
}

/// Two Running allocations of `payments` are each issued an SVID: each holds
/// its own (distinct serial, one audit row per issuance), and the identity each
/// holds is the workload's.
#[tokio::test]
#[should_panic(expected = "an allocation's SVID names its workload")]
async fn two_allocations_of_one_workload_each_hold_their_own_svid_naming_the_workload() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    let identity = Arc::new(IdentityMgr::new(None));
    let state =
        build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs), Arc::clone(&identity)).await;
    let payments = wid("payments");
    let (first, second) = (aid("payments-0"), aid("payments-1"));
    write_running_alloc(&state, &payments, &first, 1).await;
    write_running_alloc(&state, &payments, &second, 1).await;

    tick_svid(&state, &svid_target(&payments), std::time::Instant::now(), 0).await;

    let held = held_identities(&identity, &[&first, &second]);
    assert_eq!(serials(&state).await.len(), 2, "each allocation holds its own SVID (own serial)");
    for (alloc, spiffe) in held {
        assert_eq!(spiffe, PAYMENTS_IDENTITY, "an allocation's SVID names its workload: {alloc}");
    }
}

/// A serve restart loses the in-memory held set but not the audit rows: the
/// boot convergence re-issues an SVID to BOTH allocations, again under the one
/// workload identity.
#[tokio::test]
#[should_panic(expected = "after a restart an allocation's SVID names its workload")]
async fn after_a_serve_restart_both_allocations_recover_their_own_svid_for_the_workload() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    let before = Arc::new(IdentityMgr::new(None));
    let state = build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs), Arc::clone(&before)).await;
    let payments = wid("payments");
    let (first, second) = (aid("payments-0"), aid("payments-1"));
    write_running_alloc(&state, &payments, &first, 1).await;
    write_running_alloc(&state, &payments, &second, 1).await;
    let base = std::time::Instant::now();
    tick_svid(&state, &svid_target(&payments), base, 0).await;
    assert_eq!(
        serials(&state).await.len(),
        2,
        "each allocation was issued its own SVID before the restart"
    );

    // The restart: a fresh node composition over the same observation store.
    let tmp_after = TempDir::new().expect("tmpdir");
    let after = Arc::new(IdentityMgr::new(None));
    let restarted =
        build_state(&tmp_after, Arc::clone(&clock), Arc::clone(&obs), Arc::clone(&after)).await;
    tick_svid(&restarted, &svid_target(&payments), base, 1).await;

    let held = held_identities(&after, &[&first, &second]);
    for (alloc, spiffe) in held {
        assert_eq!(
            spiffe, PAYMENTS_IDENTITY,
            "after a restart an allocation's SVID names its workload: {alloc}"
        );
    }
}
