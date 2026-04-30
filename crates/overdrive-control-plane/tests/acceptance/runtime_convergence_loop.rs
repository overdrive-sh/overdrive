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
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
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
async fn noop_heartbeat_against_converged_target_does_not_re_enqueue() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = SimClock::new();
    let state = build_converged_state(&tmp, &clock);

    // --- Preload IntentStore: one Job, replicas=1 (the converged
    //     desired state for `JobLifecycle` against `job/payments`).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
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

// ---------------------------------------------------------------------------
// fix-eval-reconciler-discarded — RED regression scaffold (Step 01-01).
//
// Pins the dispatch-routing contract that lives at
// `reconciler_runtime.rs::run_convergence_tick`: a drained
// `Evaluation { reconciler, target }` MUST dispatch ONLY the named
// reconciler against the target — not fan out across every registered
// reconciler. The current production loop (`for name in &registered`)
// ignores `eval.reconciler` entirely, so a single eval submitted at
// `(job-lifecycle, job/payments)` causes BOTH `JobLifecycle::hydrate_desired`
// AND `NoopHeartbeat::hydrate_desired` to read from the IntentStore — see
// `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md` §Defect.
//
// This test is written against the POST-FIX `run_convergence_tick`
// signature (`run_convergence_tick(state, reconciler_name, target, ...)`),
// so it WILL NOT COMPILE against current main — that compile failure IS
// the RED proof per `.claude/rules/testing.md` § "RED scaffolds and
// intentionally-failing commits". The `#[ignore]` attribute only skips
// runtime; cargo check still catches the arity mismatch, which is why
// this commit must land via `git commit --no-verify`.
//
// Step 01-02 lands the production fix in `run_convergence_tick`,
// updates the lib.rs caller and the cascade test sites, and removes
// the `#[ignore]` — the test transitions un-compiled-and-ignored →
// compiled-and-passing in one cohesive commit.
// ---------------------------------------------------------------------------

/// RED — drives the runtime convergence loop with a single
/// `Evaluation { reconciler: job-lifecycle, target: job/payments }` and
/// asserts that ONLY `JobLifecycle` is dispatched against the target.
///
/// Counting strategy: every reconciler that runs through
/// `run_convergence_tick` writes a `(reconciler_name, target_string)`
/// entry into `AppState::view_cache` via `store_cached_view`
/// (`reconciler_runtime.rs:248`). The cache is `pub` and observable from
/// the test:
///
/// * **Pre-fix**: the dispatch loop iterates every registered
///   reconciler (`for name in &registered`) and runs both
///   `JobLifecycle` and `NoopHeartbeat` against the JobLifecycle
///   target, so `view_cache` ends up with TWO entries —
///   `("job-lifecycle", "job/payments")` AND
///   `("noop-heartbeat", "job/payments")`. The latter entry is the
///   smoking gun: `NoopHeartbeat` was never named in the submitted
///   evaluation, yet it executed.
/// * **Post-fix**: the dispatch path looks up only the named
///   reconciler, so `view_cache` contains exactly ONE entry —
///   `("job-lifecycle", "job/payments")`.
///
/// Written against the post-fix `run_convergence_tick(state,
/// reconciler_name, target, now, tick_n, deadline)` signature — the
/// arity mismatch against current main is the proof-of-RED.
#[tokio::test]
#[ignore = "RED scaffold for fix-eval-reconciler-discarded — un-ignore in the GREEN step"]
async fn eval_dispatch_runs_only_the_named_reconciler() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = SimClock::new();

    // --- Build a converged AppState (same fixture shape as the test
    //     above; both reconcilers registered).
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").expect("NodeId"),
        0,
    ));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let state = AppState::new(store, obs, Arc::new(runtime), driver);

    // --- Preload IntentStore with one converged Job (replicas=1).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/true".to_string(),
            args: vec![],
        }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let payments_intent_key = IntentKey::for_job(&job.id);
    state
        .store
        .put(payments_intent_key.as_bytes(), archived.as_ref())
        .await
        .expect("put job");

    // --- Preload ObservationStore: one Running alloc against the same
    //     job so `JobLifecycle::reconcile` sees `desired ≈ actual` and
    //     emits no actions — keeps the assertion focused on the
    //     dispatch-routing defect rather than convergence work.
    let writer = NodeId::new("local").expect("writer node id");
    let alloc_row = AllocStatusRow {
        alloc_id: AllocationId::new("alloc-payments-0").expect("valid alloc id"),
        job_id: job.id.clone(),
        node_id: writer.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: writer.clone() },
    };
    state
        .obs
        .write(ObservationRow::AllocStatus(alloc_row))
        .await
        .expect("seed Running alloc row");

    // --- Submit ONE evaluation naming `job-lifecycle` only.
    let target = TargetResource::new("job/payments").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    // --- Drain and dispatch using the POST-FIX call shape. The
    //     compile error against current main is the RED proof: the
    //     present-day `run_convergence_tick` takes `(&state, &target,
    //     now, tick_n, deadline)` — this call site adds
    //     `&eval.reconciler` as the second arg and won't compile until
    //     the production fix in 01-02 lands the matching signature.
    let now = clock.now();
    let deadline = now + Duration::from_millis(100);
    let tick_n = 0_u64;
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    for eval in pending {
        let _ = run_convergence_tick(
            &state,
            &eval.reconciler,
            &eval.target,
            now,
            tick_n,
            deadline,
        )
        .await;
    }

    // --- Assertion (kills the bugged behaviour): only `job-lifecycle`
    //     ran against `job/payments`. `view_cache` is keyed on
    //     `(reconciler_name_string, target_string)`; every reconciler
    //     that runs through `run_convergence_tick` writes its entry
    //     via `store_cached_view`.
    //
    //     Pre-fix the cache has TWO entries — both reconcilers ran.
    //     Post-fix the cache has ONE entry — only the named reconciler.
    let cache = state.view_cache.lock().expect("view_cache mutex");
    let entries_for_target: Vec<&(String, String)> = cache
        .keys()
        .filter(|(_, t)| t == &target.to_string())
        .collect();
    assert_eq!(
        entries_for_target.len(),
        1,
        "expected exactly one reconciler to run against {} — \
         JobLifecycle only — got {} entries: {:?} \
         (pre-fix value 2 indicates fan-out across both reconcilers; \
         the smoking gun is the noop-heartbeat entry, which was never \
         named in the submitted evaluation)",
        target,
        entries_for_target.len(),
        entries_for_target
    );
    let only_entry = entries_for_target[0];
    assert_eq!(
        only_entry.0, "job-lifecycle",
        "expected the surviving cache entry to be `job-lifecycle` — \
         got `{}`",
        only_entry.0
    );
}
