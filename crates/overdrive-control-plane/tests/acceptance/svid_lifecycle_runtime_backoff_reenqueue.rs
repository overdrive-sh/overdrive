//! Finding-1 runtime witness — the `svid-lifecycle` reconciler stays alive
//! across convergence cadences while an `IssueSvid` is mid-backoff, then re-ticks
//! at the deadline (ADR-0067 D8 retry memory + the §18 `view_has_backoff_pending`
//! self-re-enqueue gate).
//!
//! Pre-patch the `AnyReconcilerView::SvidLifecycle(_)` arm of
//! `view_has_backoff_pending` returned `false` with a stale comment ("the
//! retry-memory view + its backoff-pending arm land in step 03-01") — but the
//! retry-memory `View` HAD landed (`SvidLifecycleView { retry: BTreeMap<…,
//! IssueRetry> }`). So while a `running ∧ ¬held` alloc is mid-backoff the
//! reconciler suppresses the re-issue (emits a bare `Noop`), the §18
//! action-emitted gate (`has_work`) stays false, the broker drains empty, and the
//! reconciler is NEVER re-ticked at the deadline unless another event pokes it.
//!
//! This AT drives a REAL `ReconcilerRuntime` convergence loop with Sim adapters.
//! It SEEDS a retry entry whose backoff window has NOT yet elapsed (the
//! `tick.now_unix` sampled off the SHARED clock equals the seeded
//! `last_failure_seen_at`, so `now_unix < last_failure_seen_at + backoff`), for a
//! Running, `¬held` alloc — exactly the state a prior failed-then-recorded issue
//! leaves. The reconcile then emits a bare `Noop` (suppressed), so the only thing
//! that can keep the reconciler enqueued is the `view_has_backoff_pending`
//! predicate. It mirrors the GAP-9 `service_lifecycle_runtime_reenqueue.rs` shape
//! (drain → tick → assert still-pending), pointed at the svid retry-backoff seam.
//!
//! Port-to-port: the driving port is `run_convergence_tick` for the
//! `svid-lifecycle` reconciler; the observable outcome is whether the runtime
//! re-enqueues the eval (asserted at the broker boundary) while the alloc stays
//! suppressed-and-unheld.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::{IssueRetry, SvidLifecycle, SvidLifecycleView};
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use std::collections::BTreeMap;
use tempfile::TempDir;

const SVID_LIFECYCLE: &str = "svid-lifecycle";
const WORKLOAD_NAME: &str = "payments";
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
    // A normal SimCa — issuance succeeds, but the seeded backoff suppresses the
    // re-issue this tick so the CA is never reached (the alloc stays ¬held).
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

/// Drain the broker and run one convergence tick per pending eval. Returns
/// whether a `svid-lifecycle` eval was among the drained set.
async fn run_one_cadence(state: &AppState, now: std::time::Instant, tick_n: u64) -> bool {
    let deadline = now + Duration::from_millis(100);
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    let had_svid = pending.iter().any(|e| e.reconciler.as_str() == SVID_LIFECYCLE);
    for eval in pending {
        run_convergence_tick(state, &eval.reconciler, &eval.target, now, tick_n, deadline)
            .await
            .expect("convergence tick must not panic");
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }
    had_svid
}

/// Is a `svid-lifecycle` eval currently pending in the broker (without draining
/// it)? Drain-and-resubmit — the broker is LWW so re-submit is idempotent.
fn svid_eval_pending(state: &AppState) -> bool {
    let mut broker = state.runtime.broker();
    let drained = broker.drain_pending();
    let present = drained.iter().any(|e| e.reconciler.as_str() == SVID_LIFECYCLE);
    for e in drained {
        broker.submit(e);
    }
    present
}

/// Finding 1 — while a `running ∧ ¬held` alloc is mid-backoff (a recorded retry
/// entry whose window has not elapsed), the runtime self-re-enqueues
/// `svid-lifecycle` across cadences so the reconciler is re-ticked at the
/// deadline instead of being silently dropped. The reconcile emits a bare `Noop`
/// every suppressed tick, so the ONLY thing keeping the reconciler enqueued is
/// the `view_has_backoff_pending` predicate this fix corrects.
#[tokio::test]
async fn svid_lifecycle_reenqueues_while_issue_backoff_pending() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    let state = build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs)).await;

    let workload = wid(WORKLOAD_NAME);
    let alloc = aid("payments-0");
    let target = svid_target(&workload);

    // A Running alloc that is NOT yet held.
    write_running_alloc(&state, &workload, &alloc, 1).await;

    // Seed a retry entry whose backoff window has NOT elapsed: its
    // `last_failure_seen_at` equals `tick.now_unix` (sampled off the SHARED
    // clock, still at logical 0), so `now_unix < last_failure_seen_at + backoff`
    // and the reconcile SUPPRESSES the re-issue this tick (emits a bare Noop).
    // This is the state a prior failed-then-recorded `IssueSvid` leaves behind.
    let now_unix = UnixInstant::from_clock(&*clock);
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(alloc.clone(), IssueRetry { attempts: 1, last_failure_seen_at: now_unix });
    state.runtime.seed_svid_lifecycle_view_for_test(&target, SvidLifecycleView { retry });

    // Seed the FIRST enqueue (Shape C's job in production; here we submit
    // directly to isolate the self-re-enqueue under test).
    state
        .runtime
        .broker()
        .submit(Evaluation { reconciler: svid_reconciler_name(), target: target.clone() });

    let base = std::time::Instant::now();

    // -----------------------------------------------------------------
    // Cadence 1 — running ∧ ¬held, but mid-backoff → reconcile emits a bare
    // Noop (suppressed). The §18 action-emitted gate (`has_work`) is false,
    // so the runtime MUST self-re-enqueue via view_has_backoff_pending (the
    // retry map is non-empty) — pre-patch the broker drained empty here.
    // -----------------------------------------------------------------
    let ran_1 = run_one_cadence(&state, base, 0).await;
    assert!(ran_1, "cadence 1 must have run the seeded svid-lifecycle eval");

    // The alloc is still unheld (the issue was suppressed, not attempted).
    assert!(
        state.identity.held_snapshot().is_empty(),
        "the mid-backoff tick suppresses the re-issue — the alloc stays ¬held",
    );

    assert!(
        svid_eval_pending(&state),
        "Finding 1: while a retry entry is outstanding the runtime MUST re-enqueue \
         svid-lifecycle (pre-patch the broker drained empty here, stalling the retry)"
    );

    // -----------------------------------------------------------------
    // A few more cadences still inside the backoff window — the reconciler
    // emits Noop every tick, and the runtime must keep it enqueued via the
    // retry-memory predicate (the bug this test pins: a stale `false` arm
    // drops the eval and the backoff deadline is never re-evaluated).
    // -----------------------------------------------------------------
    for tick_n in 1..4 {
        let ran = run_one_cadence(&state, base, tick_n).await;
        assert!(ran, "cadence {tick_n}: svid-lifecycle must still be pending (mid-backoff)");
        assert!(
            svid_eval_pending(&state),
            "cadence {tick_n}: runtime must keep re-enqueueing while the retry entry is outstanding"
        );
    }

    // The retry entry is preserved across the suppressed cadences (still
    // attempts == 1 — a suppressed tick neither re-emits nor bumps).
    let view_mid = state
        .runtime
        .loaded_svid_lifecycle_views_for_test(&svid_reconciler_name())
        .expect("svid-lifecycle view map present")
        .get(&target)
        .cloned()
        .expect("view for target present");
    assert_eq!(
        view_mid.retry.get(&alloc).expect("retry entry preserved mid-backoff").attempts,
        1,
        "a suppressed (mid-backoff) tick neither re-emits nor bumps attempts"
    );
}
