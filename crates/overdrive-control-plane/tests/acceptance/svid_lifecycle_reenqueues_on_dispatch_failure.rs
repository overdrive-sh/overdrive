//! BUG-2 regression witness — a `svid-lifecycle` convergence tick whose
//! `IssueSvid` dispatch FAILS must still self-re-enqueue, so the persisted retry
//! memory re-drives on a later tick instead of stalling forever.
//!
//! Root cause (pre-fix): `run_convergence_tick` computed `has_work` pre-dispatch
//! and persisted the retry-bearing View, but then `?`-propagated the shim error
//! from `dispatch_with_workflow_intent(...).map_err(ConvergenceError::Shim)?` —
//! skipping the `yield_now` + `if has_work { submit }` self-re-enqueue below it.
//! The production loop (`lib.rs`) only LOGS the error, so the broker drained
//! empty and the reconciler was never re-ticked. `view_has_backoff_pending`
//! re-enqueues only once a tick actually RUNS, so the FIRST failed tick stalled:
//! its retry entry was persisted but no tick ever re-drove it.
//!
//! The fix captures the dispatch outcome and returns it LAST — after the
//! re-enqueue runs on all paths. The error is still propagated (so `lib.rs`
//! logs it); the re-enqueue is the self-heal.
//!
//! Port-to-port: the driving port is `run_convergence_tick` for `svid-lifecycle`
//! (driven with a test-only failing `Ca`); the observable outcome is whether a
//! `svid-lifecycle` eval is still pending in the broker after the failed tick
//! (asserted at the broker boundary). Pre-fix the broker is empty (stall);
//! post-fix it is pending.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, run_convergence_tick,
};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle;
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
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
const WORKLOAD_NAME: &str = "payments";
const NODE_NAME: &str = "host-0";

/// A test-only `Ca` whose `issue_svid` always fails — every other method
/// delegates to a real `SimCa`. This is a legitimate test double (a failing
/// driven-port adapter), NOT production surface: it drives the `IssueSvid`
/// action-shim executor onto its `Err` path so the convergence tick returns
/// `Err(ConvergenceError::Shim(_))`.
struct FailingIssueCa {
    delegate: SimCa,
}

impl FailingIssueCa {
    fn new() -> Self {
        Self { delegate: SimCa::new(Arc::new(SimEntropy::new(0))) }
    }
}

impl Ca for FailingIssueCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.delegate.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.delegate.issue_intermediate(node)
    }

    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        Err(CaError::signing_failed("test double: issuance fails to drive the Err path"))
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.delegate.trust_bundle()
    }
}

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
    // The failing CA — every IssueSvid dispatch returns Err, driving the
    // convergence tick onto its `Err(ConvergenceError::Shim(_))` path.
    let ca: Arc<dyn Ca> = Arc::new(FailingIssueCa::new());
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(SimDataplane::new()),
        ca,
        Arc::new(IdentityMgr::new(None)),
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

/// Is a `svid-lifecycle` eval currently pending in the broker (without
/// consuming it)? Drain-and-resubmit — the broker is LWW so re-submit is
/// idempotent.
fn svid_eval_pending(state: &AppState) -> bool {
    let mut broker = state.runtime.broker();
    let drained = broker.drain_pending();
    let present = drained.iter().any(|e| e.reconciler.as_str() == SVID_LIFECYCLE);
    for e in drained {
        broker.submit(e);
    }
    present
}

/// BUG-2 — the FIRST failed `IssueSvid` tick must re-enqueue. The tick returns
/// `Err(ConvergenceError::Shim(_))` (issuance failed); the test TOLERATES the
/// Err (it does NOT `.expect()` it — the cadence helper in the sibling test
/// expects-on-Err and would panic here), then asserts a `svid-lifecycle` eval is
/// still pending so the retry re-drives. Pre-fix the broker drained empty.
#[tokio::test]
async fn svid_lifecycle_reenqueues_when_issue_dispatch_fails() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    let state = build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs)).await;

    let workload = wid(WORKLOAD_NAME);
    let alloc = aid("payments-0");
    let target = svid_target(&workload);

    // One Running, unheld alloc, no retry seeded — the first-issue path. The
    // reconciler emits `IssueSvid`; the shim's executor calls the failing CA and
    // returns Err.
    write_running_alloc(&state, &workload, &alloc, 1).await;

    // Seed the FIRST enqueue (Shape C's job in production; here we submit
    // directly to isolate the failed-dispatch self-re-enqueue under test).
    state
        .runtime
        .broker()
        .submit(Evaluation { reconciler: svid_reconciler_name(), target: target.clone() });

    let now = std::time::Instant::now();
    let deadline = now + Duration::from_millis(100);

    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    assert!(
        pending.iter().any(|e| e.reconciler.as_str() == SVID_LIFECYCLE),
        "the seeded svid-lifecycle eval must be present to drive the failed tick"
    );

    // Run the failed tick. It MUST return Err (issuance failed) — we tolerate it
    // here rather than `.expect()` (the sibling cadence helper would panic).
    for eval in pending {
        if eval.reconciler.as_str() != SVID_LIFECYCLE {
            continue;
        }
        let result =
            run_convergence_tick(&state, &eval.reconciler, &eval.target, now, 0, deadline).await;
        assert!(
            matches!(result, Err(ConvergenceError::Shim(_))),
            "the IssueSvid dispatch must fail with a Shim error (failing CA), got {result:?}"
        );
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    // The alloc never got held (issuance failed).
    assert!(
        state.identity.held_snapshot().is_empty(),
        "a failed IssueSvid holds nothing — the alloc stays ¬held"
    );

    // BUG-2 witness: despite the Err propagating, the failed tick must have
    // re-enqueued svid-lifecycle so the persisted retry memory re-drives.
    // Pre-fix the early `?` skipped the re-enqueue and the broker drained empty.
    assert!(
        svid_eval_pending(&state),
        "BUG-2: a recoverable IssueSvid dispatch failure must still self-re-enqueue \
         svid-lifecycle (pre-fix the early `?` skipped the re-enqueue, stalling the retry)"
    );
}
