//! Step 01-01 / `fix-noop-self-reenqueue` — RED regression scaffold.
//!
//! Pins the §18 level-triggered re-enqueue gate's *semantic* contract:
//! `NoopHeartbeat::reconcile` returns `vec![Action::Noop]` to signal
//! "nothing to do this tick" (proof-of-life), and `action_shim::dispatch`
//! treats `Action::Noop` as a no-op. The runtime's `has_work` predicate
//! must therefore honour that semantic and NOT re-enqueue purely on
//! `!actions.is_empty()`. The current production code (line 256 of
//! `reconciler_runtime.rs`) does the syntactic check and self-re-enqueues
//! `(noop-heartbeat, target)` perpetually — see
//! `docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md`.
//!
//! This test is `#[ignore]`d in this commit so the lefthook pre-commit
//! gate stays green. Step 01-02 lands the predicate fix and removes the
//! `#[ignore]` to prove the RED → GREEN transition.
//!
//! Tier classification: **Tier 1 DST** per `.claude/rules/testing.md`.
//! Default unit lane (no `#![cfg(feature = "integration-tests")]`) per
//! the `tests/acceptance.rs` entrypoint header — this crate's acceptance
//! suite is in-process serde + sim-adapter only.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::eval_broker::Evaluation;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Build an `AppState` whose runtime carries both production reconcilers
/// (`noop-heartbeat` and `job-lifecycle`) — matching the `run_server`
/// boot path. The `SimClock` is held by the caller so the test can
/// advance logical time between ticks.
fn build_converged_state(tmp: &TempDir, clock: &SimClock) -> AppState {
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let _ = clock; // explicit `clock` retained as the test's logical-time source
    AppState::new(store, obs, Arc::new(runtime), driver)
}

/// RED — drive the runtime convergence loop end-to-end against a fully
/// converged target. After the initial edge-triggered submit is drained
/// at tick 0, no further dispatches must occur for any reconciler whose
/// emitted actions are exclusively no-op sentinels.
///
/// With the bug present (production code at `reconciler_runtime.rs:256`
/// uses `!actions.is_empty()`): `noop-heartbeat` self-re-enqueues every
/// tick → `dispatched` reaches 10 and `queued` stays at 1.
///
/// With the fix landed: `dispatched == 1` (only the seed eval is drained)
/// and `queued == 0` (convergence is stable).
#[tokio::test]
#[ignore = "RED scaffold for fix-noop-self-reenqueue — un-ignore in the GREEN step"]
async fn noop_heartbeat_against_converged_target_does_not_re_enqueue() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = SimClock::new();
    let state = build_converged_state(&tmp, &clock);

    // --- Preload IntentStore: one Job, replicas=1 (the converged
    //     desired state for `JobLifecycle` against `job/payments`).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        cpu_milli: 100,
        memory_bytes: 256 * 1024 * 1024,
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let key = IntentKey::for_job(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    // --- Preload ObservationStore: one Running alloc against the same
    //     job (so `JobLifecycle::reconcile` sees `desired ≈ actual` and
    //     emits no actions, isolating the assertion to NoopHeartbeat).
    let writer = NodeId::new("local").expect("writer node id");
    let alloc_row = AllocStatusRow {
        alloc_id: AllocationId::new("alloc-payments-0").expect("valid alloc id"),
        job_id: job.id.clone(),
        node_id: writer.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: writer.clone() },
    };
    state.obs.write(ObservationRow::AllocStatus(alloc_row)).await.expect("seed Running alloc row");

    // --- Submit ONE evaluation. The convergence-tick loop in
    //     `lib.rs::run_server_with_obs_and_driver` drains the broker per
    //     tick and runs every registered reconciler against each drained
    //     target — we replicate that loop here without binding to TCP.
    let target = TargetResource::new("job/payments").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    // --- Drive 10 convergence ticks. Logical time is advanced by 100ms
    //     between ticks via `SimClock::tick` so the per-tick `now` and
    //     `deadline` snapshots remain monotonic and reproducible.
    for tick_n in 0..10_u64 {
        let now = clock.now();
        let deadline = now + Duration::from_millis(100);
        // Drop the MutexGuard before any `.await` per
        // `.claude/rules/development.md` § Concurrency & async.
        let pending = {
            let mut broker = state.runtime.broker();
            broker.drain_pending()
        };
        for eval in pending {
            run_convergence_tick(&state, &eval.target, now, tick_n, deadline)
                .await
                .expect("convergence tick succeeds");
        }
        clock.tick(Duration::from_millis(100));
    }

    // --- Assertion 1 (kills the bug): only the seed eval was drained.
    let counters = state.runtime.broker().counters();
    assert_eq!(
        counters.dispatched, 1,
        "noop-heartbeat against a converged target must not self-re-enqueue; \
         expected dispatched == 1, got {}",
        counters.dispatched
    );

    // --- Assertion 2 (kills the inverted-predicate mutation
    //     `!actions.iter().any(...)`): convergence is stable.
    assert_eq!(
        counters.queued, 0,
        "convergence must complete with no pending evaluations; got {}",
        counters.queued
    );
}
