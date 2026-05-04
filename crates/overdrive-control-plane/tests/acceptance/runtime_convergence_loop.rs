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
async fn build_converged_state(tmp: &TempDir, clock: Arc<SimClock>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver, clock)
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
    let clock = Arc::new(SimClock::new());
    let state = build_converged_state(&tmp, clock.clone()).await;

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
        reason: None,
        detail: None,
        terminal: None,
    };
    state.obs.write(ObservationRow::AllocStatus(alloc_row)).await.expect("seed Running alloc row");

    // --- Submit ONE evaluation. The convergence-tick loop in
    //     `lib.rs::run_server_with_obs_and_driver` drains the broker per
    //     tick and dispatches each drained Evaluation to exactly the
    //     reconciler named in `eval.reconciler` against the eval's
    //     target — N entries collapse to N dispatches per distinct
    //     `(reconciler, target)` key, per whitepaper §18 / ADR-0013 §8.
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
            run_convergence_tick(&state, &eval.reconciler, &eval.target, now, tick_n, deadline)
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
///   `JobLifecycle` and `NoopHeartbeat` against the `JobLifecycle`
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
async fn eval_dispatch_runs_only_the_named_reconciler() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = Arc::new(SimClock::new());

    // --- Build a converged AppState (same fixture shape as the test
    //     above; both reconcilers registered).
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let state = AppState::new(store, obs, Arc::new(runtime), driver, clock.clone());

    // --- Preload IntentStore with one converged Job (replicas=1).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let payments_intent_key = IntentKey::for_job(&job.id);
    state.store.put(payments_intent_key.as_bytes(), archived.as_ref()).await.expect("put job");

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
        reason: None,
        detail: None,
        terminal: None,
    };
    state.obs.write(ObservationRow::AllocStatus(alloc_row)).await.expect("seed Running alloc row");

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
        let _ = run_convergence_tick(&state, &eval.reconciler, &eval.target, now, tick_n, deadline)
            .await;
    }

    // --- Assertion (kills the bugged behaviour): only `job-lifecycle`
    //     ran against `job/payments`. Per ADR-0035 §5, the runtime now
    //     stashes per-reconciler-kind in-memory `BTreeMap<TargetResource,
    //     View>` maps; the JobLifecycle map gets an entry for `target`
    //     IFF the JobLifecycle reconciler ran against it. The
    //     NoopHeartbeat variant carries `View = ()` and never
    //     materialises a per-target row, so the cross-reconciler fan-out
    //     bug class manifests differently here: pre-fix the JobLifecycle
    //     map has an entry for `target` AND the NoopHeartbeat reconciler
    //     ALSO ran (broker dispatch fan-out), bumping the dispatched
    //     counter past one. Post-fix only the named reconciler runs and
    //     the dispatched counter is exactly one.
    let job_lifecycle_name = ReconcilerName::new("job-lifecycle").expect("name");
    let jl_views = state
        .runtime
        .loaded_job_lifecycle_views_for_test(&job_lifecycle_name)
        .expect("job-lifecycle map present");
    assert!(
        jl_views.contains_key(&target),
        "expected job-lifecycle to have run against {target} — got map keys {:?}",
        jl_views.keys().collect::<Vec<_>>()
    );
    // Broker dispatched counter: pre-fix would be ≥ 2 (both reconcilers
    // ran); post-fix is exactly 1 (the named reconciler only).
    assert_eq!(
        state.runtime.broker().counters().dispatched,
        1,
        "expected exactly one dispatch per submitted evaluation — \
         pre-fix value ≥ 2 indicates fan-out across both reconcilers"
    );
}

// ---------------------------------------------------------------------------
// fix-stop-branch-backoff-pending — RED regression scaffold (Step 01-01).
//
// Pins the §18 *Level-triggered inside the reconciler* contract for the
// Stop branch of `JobLifecycle::reconcile`: when a stop intent arrives
// while the only alloc is `Failed` mid-restart-backoff
// (`view.last_failure_seen_at` populated, `view.restart_counts < CEILING`),
// the reconciler must clear the transitional view state — otherwise
// `view_has_backoff_pending` keeps `has_work = true` and the broker
// self-re-enqueues every tick until the ceiling is reached
// (~5 s hot spin per the RCA at
// `docs/feature/fix-stop-branch-backoff-pending/deliver/rca.md`).
//
// Pre-fix (current `main`):
//   - Stop branch returns `(stop_actions, view.clone())`. With no
//     Running allocs, `stop_actions` is empty BUT
//     `view.last_failure_seen_at` still names the Failed alloc and
//     `restart_counts < CEILING` ⇒ `view_has_backoff_pending` returns
//     `true` ⇒ `has_work = true` ⇒ re-enqueue. Every subsequent tick
//     repeats — the eval is in the pending set every time the broker
//     is drained.
//   - Observable symptom on this fixture: across 10 ticks after the
//     stop intent is written, `dispatched` advances by ≥5 (the per-alloc
//     `RESTART_BACKOFF_CEILING` budget — see memory note 38682) and
//     `queued` stays at 1.
//
// Post-fix (the GREEN edit at step 01-02 in
// `crates/overdrive-core/src/reconciler.rs:1019-1027`):
//   - Stop branch clears `last_failure_seen_at` when
//     `stop_actions.is_empty()`, so the first post-stop tick sees
//     `view_has_backoff_pending = false`, `has_work = false`, no
//     re-enqueue. The broker drains and stays empty.
//
// This test is `#[ignore]`d in this commit so the lefthook
// pre-commit gate (`nextest-affected`) stays green between the RED
// scaffold commit and the GREEN fix commit. Step 01-02 lands the
// reconciler edit and removes the `#[ignore]` in the same commit; the
// test transitions skipped → executed-and-passing as the proof of the
// RED → GREEN flip.
//
// Tier classification: **Tier 1 DST** per `.claude/rules/testing.md`.
// Default unit lane (in-process sim adapters only).
// ---------------------------------------------------------------------------

/// RED — drives the runtime convergence loop through the
/// Failed-mid-backoff → Stop sequence and asserts the broker drains
/// after the stop intent lands.
///
/// Sequence:
///   1. Build sim `AppState`. `SimDriver` is configured to reject every
///      `start()` with `DriverError::StartRejected` so the alloc
///      transitions to `AllocState::Failed` on the first tick.
///   2. Submit the job intent and one `Evaluation { reconciler:
///      "job-lifecycle", target: "job/payments" }`.
///   3. Drive ticks until the cached view records the alloc's
///      `last_failure_seen_at` observation and `restart_counts == 1` —
///      proof that the reconciler emitted exactly one Restart action and
///      then sat back on the backoff. (We do NOT advance logical time
///      far enough to elapse the 1-second per-attempt backoff window;
///      this is the load-bearing precondition — the alloc is Failed
///      AND mid-backoff when the stop arrives.)
///   4. Submit the stop intent (`IntentKey::for_job_stop(<id>)`) and
///      capture `dispatched` at this moment.
///   5. Drive 10 more convergence ticks.
///   6. Assert: `queued == 0` (broker drained); `dispatched -
///      dispatched_at_stop_submit <= 2` (the stop converges in 1-2
///      ticks; pre-fix this hits ≥5 from the `RESTART_BACKOFF_CEILING`
///      hot spin).
#[tokio::test]
// DST harness setup (build sim AppState, preload IntentStore, drive warm-up
// ticks until Failed-mid-backoff state, submit stop intent, drive post-stop
// ticks, assert) is necessarily long; splitting would obscure the scenario
// shape and force shared-mutable-fixture indirection.
#[allow(clippy::too_many_lines)]
async fn stop_after_failed_alloc_drains_broker() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = Arc::new(SimClock::new());

    // --- Build AppState. Reject starts so the action shim writes
    //     `AllocState::Failed` and the reconciler enters the
    //     restart-with-backoff branch.
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(
        SimDriver::new(DriverType::Exec).fail_on_start_with("binary not found".to_string()),
    );
    let state = AppState::new(store, obs, Arc::new(runtime), driver, clock.clone());

    // --- Preload IntentStore: one Job. The driver will reject its
    //     start, the action shim writes `AllocState::Failed`, the
    //     reconciler emits one `RestartAllocation` then sits on the
    //     1-second backoff (`RESTART_BACKOFF_DURATION`).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/does/not/exist".to_string(),
            args: vec![],
        }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let job_key = IntentKey::for_job(&job.id);
    state.store.put(job_key.as_bytes(), archived.as_ref()).await.expect("put job");

    // --- Submit the seed evaluation.
    let target = TargetResource::new("job/payments").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    // --- Drive convergence until the view records the alloc's
    //     backoff deadline AND `restart_counts == 1`. We bound the
    //     loop at 30 ticks (>> than the 2-3 we expect — first tick
    //     emits StartAllocation, action shim writes Failed; second
    //     tick emits RestartAllocation, restart_counts becomes 1,
    //     deadline written; third tick sits on the backoff and
    //     re-enqueues).
    //
    //     Logical time advances by 100 ms per tick — well below
    //     `RESTART_BACKOFF_DURATION = 1 s`, so the alloc stays
    //     mid-backoff for the rest of the test.
    let setup_target = target.clone();
    let job_id = job.id.clone();
    let mut warm_up_ticks = 0_u64;
    while warm_up_ticks < 30 {
        let now = clock.now();
        let deadline = now + Duration::from_millis(100);
        let pending = {
            let mut broker = state.runtime.broker();
            broker.drain_pending()
        };
        for eval in pending {
            run_convergence_tick(
                &state,
                &eval.reconciler,
                &eval.target,
                now,
                warm_up_ticks,
                deadline,
            )
            .await
            .expect("convergence tick succeeds");
        }
        clock.tick(Duration::from_millis(100));
        warm_up_ticks += 1;

        // Check whether the cached view shows the desired
        // Failed-mid-backoff state.
        let view = state.runtime.view_for_job_lifecycle(&setup_target);
        let alloc_id =
            AllocationId::new(&format!("alloc-{}-0", job_id.as_str())).expect("derived alloc id");
        let count = view.restart_counts.get(&alloc_id).copied().unwrap_or(0);
        let has_deadline = view.last_failure_seen_at.contains_key(&alloc_id);
        if count >= 1 && has_deadline {
            break;
        }
    }
    assert!(
        warm_up_ticks < 30,
        "warm-up failed to reach Failed-mid-backoff state within 30 ticks; \
         test fixture is misconfigured (broker should self-re-enqueue \
         under both pre-fix and post-fix code while restart_counts \
         < CEILING and now < deadline)",
    );

    // --- Snapshot dispatched counter at the moment the stop intent
    //     is submitted. This isolates the stop-branch behaviour from
    //     the warm-up dispatch traffic.
    let dispatched_at_stop_submit = state.runtime.broker().counters().dispatched;

    // --- Submit the stop intent. Per ADR-0027, presence of the key
    //     is the signal — value is opaque (the runtime probes
    //     `state.store.get(stop_key.as_bytes())` and treats `Some(_)`
    //     as "stop intended"). A single zero byte is sufficient.
    let stop_key = IntentKey::for_job_stop(&job_id);
    state.store.put(stop_key.as_bytes(), &[0u8]).await.expect("put stop intent");

    // --- Re-submit the evaluation so the next tick re-evaluates the
    //     target with the new stop signal in scope. This mirrors what
    //     the production handler does on a stop submission.
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    // --- Drive 10 convergence ticks. Logical time still advances by
    //     100 ms per tick — well under `RESTART_BACKOFF_DURATION`. The
    //     Failed alloc is still mid-backoff for every one of these
    //     ticks; the only thing that changed is the stop intent.
    for tick_n in 0..10_u64 {
        let now = clock.now();
        let deadline = now + Duration::from_millis(100);
        let pending = {
            let mut broker = state.runtime.broker();
            broker.drain_pending()
        };
        for eval in pending {
            run_convergence_tick(
                &state,
                &eval.reconciler,
                &eval.target,
                now,
                warm_up_ticks + tick_n,
                deadline,
            )
            .await
            .expect("convergence tick succeeds");
        }
        clock.tick(Duration::from_millis(100));
    }

    // --- Assertion 1 (kills the bugged behaviour): the broker drained
    //     after the stop converged. Pre-fix:
    //     `view_has_backoff_pending` keeps re-enqueueing every tick →
    //     queued stays at 1. Post-fix: the Stop branch clears
    //     `last_failure_seen_at` so the predicate returns false on the
    //     first tick after the stop, no re-enqueue, queued == 0.
    let counters = state.runtime.broker().counters();
    assert_eq!(
        counters.queued, 0,
        "convergence must complete with no pending evaluations after stop; \
         got {} (pre-fix value: 1 — view_has_backoff_pending kept the \
         eval in the queue)",
        counters.queued
    );

    // --- Assertion 2 (kills the inverted-predicate / always-true
    //     mutation): dispatch traffic is bounded after stop. Post-fix
    //     the stop converges in 1-2 ticks; pre-fix the broker
    //     self-re-enqueues until `restart_counts` reaches the
    //     ceiling — observable as ≥5 extra dispatches.
    let dispatched_during_stop = counters.dispatched - dispatched_at_stop_submit;
    assert!(
        dispatched_during_stop <= 2,
        "post-stop dispatch traffic must be bounded; expected <= 2 \
         dispatches between stop submit and broker drain, got \
         {dispatched_during_stop} (pre-fix value: ≥5 — the broker \
         self-re-enqueues until restart_counts reaches \
         RESTART_BACKOFF_CEILING)",
    );
}

// ---------------------------------------------------------------------------
// Step 03-02 — issue #141 final gate: runtime convergence loop is idempotent
// across simulated control-plane restart.
//
// Pins the load-bearing property of "persist inputs, not derived state" at
// the runtime boundary: a freshly-rehydrated `JobLifecycleView` constructed
// from only the persisted *inputs* `(restart_counts, last_failure_seen_at)`
// produces a reconcile output bit-equivalent to the live in-memory view at
// the same `TickContext`, when `backoff_for_attempt` is unchanged.
//
// Step 02-02 pinned the same property at the `JobLifecycle::reconcile`
// boundary (`crates/overdrive-core/tests/acceptance/job_lifecycle_recompute_deadline.rs`
// `restart_survival_idempotence`). 03-02 is the runtime-boundary
// counterpart: drive the runtime convergence loop until the cached view
// has accumulated non-trivial backoff state, then prove that wiping the
// cache and reseeding from `(restart_counts, last_failure_seen_at)` —
// exactly the persist-inputs shape that libSQL hydrate would produce —
// yields an identical reconcile trajectory at the same TickContext.
//
// Why this matters: under the rejected alternative where
// `JobLifecycleView` persisted a precomputed `next_attempt_at` deadline,
// this property would still pass *today* but would silently no-op when
// `backoff_for_attempt` evolves (a future per-tenant backoff override,
// a tier-based schedule swap, a deferred attempt-count adjustment).
// The persist-inputs shape makes the property structural: same inputs +
// same policy + same tick → same output by purity, regardless of how
// the policy evolves between persistence and rehydration.
// ---------------------------------------------------------------------------

/// GIVEN the runtime has driven `JobLifecycle` into a Failed-mid-backoff
/// state where the cached `JobLifecycleView` carries
/// `restart_counts > 0` and `last_failure_seen_at` populated —
/// WHEN the cached view is replaced with a freshly-constructed
/// `JobLifecycleView` containing ONLY the persisted inputs from the
/// snapshot (simulating a control-plane restart that rehydrates view
/// state from libSQL columns) AND reconcile is invoked at an identical
/// `TickContext` —
/// THEN the resulting `Vec<Action>` and `NextView` are bit-equivalent
/// to the pre-restart pair.
///
/// This proves the persist-inputs design is structurally idempotent
/// across restart at the runtime boundary: the rehydration ceremony
/// (libSQL → fresh view → reconcile) is indistinguishable from a
/// continuous in-memory tick when the underlying inputs and policy
/// are unchanged.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn runtime_reconcile_is_idempotent_across_simulated_control_plane_restart() {
    use std::collections::BTreeMap;
    use std::time::Instant;

    use overdrive_core::UnixInstant;
    use overdrive_core::reconciler::{
        AnyReconciler, AnyReconcilerView, AnyState, JobLifecycle, JobLifecycleState,
        JobLifecycleView, TickContext,
    };
    use overdrive_core::traits::driver::Resources;

    let tmp = TempDir::new().expect("tempdir");
    let sim_clock = Arc::new(SimClock::new());

    // --- Build AppState with the SimClock injected. The runtime's
    //     `run_convergence_tick` reads `state.clock.unix_now()` to
    //     populate `tick.now_unix`, so the sim clock must be the
    //     authoritative wall-clock source for this test — passed in at
    //     construction so the test fails to compile if a future refactor
    //     forgets to thread it through (per `.claude/rules/development.md`
    //     § "Port-trait dependencies").
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(
        SimDriver::new(DriverType::Exec).fail_on_start_with("binary not found".to_string()),
    );
    let state = AppState::new(store, obs, Arc::new(runtime), driver, sim_clock.clone());

    // --- Preload a Job that the SimDriver will reject — this drives
    //     the alloc into Failed and exercises the restart-with-backoff
    //     branch where `JobLifecycleView` accumulates state.
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/does/not/exist".to_string(),
            args: vec![],
        }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let job_key = IntentKey::for_job(&job.id);
    state.store.put(job_key.as_bytes(), archived.as_ref()).await.expect("put job");

    // --- Submit the seed evaluation.
    let target = TargetResource::new("job/payments").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    // --- Warm up: drive ticks until the cached view has non-trivial
    //     state (restart_counts > 0 AND last_failure_seen_at populated).
    //     Mirror the warm-up loop from `stop_after_failed_alloc_drains_broker`
    //     above. Logical time advances by 100 ms per tick — well below
    //     `RESTART_BACKOFF_DURATION = 1 s`, so the alloc stays mid-backoff
    //     across the warm-up.
    let job_id = job.id.clone();
    let alloc_id =
        AllocationId::new(&format!("alloc-{}-0", job_id.as_str())).expect("derived alloc id");
    let mut warm_up_ticks = 0_u64;
    while warm_up_ticks < 30 {
        let now = sim_clock.now();
        let deadline = now + Duration::from_millis(100);
        let pending = {
            let mut broker = state.runtime.broker();
            broker.drain_pending()
        };
        for eval in pending {
            run_convergence_tick(
                &state,
                &eval.reconciler,
                &eval.target,
                now,
                warm_up_ticks,
                deadline,
            )
            .await
            .expect("convergence tick succeeds");
        }
        sim_clock.tick(Duration::from_millis(100));
        warm_up_ticks += 1;

        let view = state.runtime.view_for_job_lifecycle(&target);
        let count = view.restart_counts.get(&alloc_id).copied().unwrap_or(0);
        let has_seen_at = view.last_failure_seen_at.contains_key(&alloc_id);
        if count >= 1 && has_seen_at {
            break;
        }
    }
    assert!(
        warm_up_ticks < 30,
        "warm-up failed to populate JobLifecycleView with backoff inputs in 30 ticks; \
         test fixture is misconfigured (SimDriver should reject starts and the \
         action_shim should write Failed allocs that the reconciler then restarts)",
    );

    // --- Snapshot the cached view's persisted inputs. These are the
    //     ONLY two fields a libSQL hydrate would produce: `restart_counts`
    //     and `last_failure_seen_at`. The view holds nothing else — by
    //     construction (issue #141 §"Persist inputs, not derived state").
    let view_pre: JobLifecycleView = state.runtime.view_for_job_lifecycle(&target);
    assert!(
        !view_pre.restart_counts.is_empty() || !view_pre.last_failure_seen_at.is_empty(),
        "expected non-default JobLifecycle view after warm-up; got default"
    );
    let restart_counts_persisted: BTreeMap<AllocationId, u32> = view_pre.restart_counts.clone();
    let last_failure_seen_at_persisted: BTreeMap<AllocationId, UnixInstant> =
        view_pre.last_failure_seen_at.clone();

    // Sanity: the test fixture actually exercises the non-trivial path.
    // If the warm-up missed the backoff state we want to know loudly,
    // not silently pass on a vacuous BTreeMap-equality assertion.
    assert!(
        !restart_counts_persisted.is_empty(),
        "snapshot must carry at least one restart_counts entry; got empty map"
    );
    assert!(
        !last_failure_seen_at_persisted.is_empty(),
        "snapshot must carry at least one last_failure_seen_at entry; got empty map"
    );

    // --- Capture pre-restart Actions and NextView. We rebuild the
    //     desired/actual states inline (mirroring what
    //     `run_convergence_tick`'s private `hydrate_desired` /
    //     `hydrate_actual` helpers produce against the seeded
    //     IntentStore + ObservationStore) and call `reconcile` through
    //     the same `AnyReconciler` dispatch the runtime uses. This
    //     pins the runtime-boundary contract without dispatching
    //     actions through `action_shim` (which would mutate the
    //     ObservationStore and invalidate the post-restart comparison).
    let alloc_rows = state.obs.alloc_status_rows().await.expect("read alloc rows");
    let mut allocations = BTreeMap::new();
    for row in alloc_rows.into_iter().filter(|r| r.job_id == job_id) {
        allocations.insert(row.alloc_id.clone(), row);
    }
    let mut nodes = BTreeMap::new();
    let local_node =
        overdrive_core::aggregate::Node::new(overdrive_core::aggregate::NodeSpecInput {
            id: "local".to_string(),
            region: "local".to_string(),
            cpu_milli: 4_000,
            memory_bytes: 8 * 1024 * 1024 * 1024,
        })
        .expect("baseline node spec");
    let _ = Resources { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 }; // type used by job spec
    nodes.insert(local_node.id.clone(), local_node);

    let desired = AnyState::JobLifecycle(JobLifecycleState {
        job: Some(job.clone()),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    });
    let actual = AnyState::JobLifecycle(JobLifecycleState {
        job: None,
        desired_to_stop: false,
        nodes,
        allocations,
    });

    // Single TickContext shared across both reconcile calls — same
    // monotonic `now`, same wall-clock `now_unix`, same `tick`, same
    // `deadline`. The wall-clock comes from the SimClock so it is
    // deterministic across the test.
    let now: Instant = sim_clock.now();
    let now_unix = UnixInstant::from_clock(&*sim_clock);
    let tick = TickContext { now, now_unix, tick: 99, deadline: now + Duration::from_millis(100) };

    let reconciler = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
    let view_live = AnyReconcilerView::JobLifecycle(view_pre.clone());
    let (actions_pre, next_view_pre) = reconciler.reconcile(&desired, &actual, &view_live, &tick);

    // --- Simulate control-plane restart: drop the cached view (the
    //     in-memory derived state) and rehydrate a fresh
    //     `JobLifecycleView` from ONLY the persisted inputs. This is
    //     the exact rehydration shape libSQL would produce —
    //     `restart_counts` and `last_failure_seen_at` are the row
    //     columns; the view struct holds nothing else.
    state.runtime.drop_job_lifecycle_view_for_test(&target);
    let view_post = JobLifecycleView {
        restart_counts: restart_counts_persisted.clone(),
        last_failure_seen_at: last_failure_seen_at_persisted.clone(),
    };
    state.runtime.seed_job_lifecycle_view_for_test(&target, view_post.clone());

    // --- Run reconcile against the rehydrated view at the SAME
    //     TickContext. Same desired, same actual, same tick — the only
    //     difference is the view came from a "freshly bootstrapped
    //     process" rather than a continuous in-memory accumulation.
    let view_rehydrated = AnyReconcilerView::JobLifecycle(view_post);
    let (actions_post, next_view_post) =
        reconciler.reconcile(&desired, &actual, &view_rehydrated, &tick);

    // --- The load-bearing assertion: rehydration from persisted
    //     inputs at the same TickContext produces identical Actions
    //     and identical NextView. This is the runtime-boundary witness
    //     for issue #141's "no-op policy change in code produces
    //     identical reconcile output across restart" acceptance bullet.
    //
    //     Under the persist-inputs shape this is structural:
    //       same `(restart_counts, last_failure_seen_at)` inputs
    //       + same `backoff_for_attempt` policy
    //       + same TickContext (notably same `tick.now_unix`)
    //       → same recomputed deadline
    //       → same gate decision
    //       → same Actions and NextView by purity.
    //
    //     Under the rejected alternative (persisting a precomputed
    //     `next_attempt_at` deadline) the property would still hold
    //     today, but a future change to `backoff_for_attempt` between
    //     persistence and rehydration would make it fail — the
    //     persisted deadline would lock in stale policy, while the
    //     freshly-rehydrated view would recompute against the new
    //     policy. The persist-inputs design eliminates that failure
    //     mode by construction.
    assert_eq!(
        actions_pre, actions_post,
        "runtime reconcile against rehydrated view must produce identical Actions; \
         pre-restart={actions_pre:?}, post-restart={actions_post:?}"
    );
    assert_eq!(
        next_view_pre, next_view_post,
        "runtime reconcile against rehydrated view must produce identical NextView; \
         pre-restart={next_view_pre:?}, post-restart={next_view_post:?}"
    );
}

// ---------------------------------------------------------------------------
// Step 03-02 mutation-gate kill targets — `view_has_backoff_pending`
// boundary semantics at the runtime level.
//
// `view_has_backoff_pending` (private to `reconciler_runtime`) gates the
// runtime's self-re-enqueue when `reconcile` returns no actions but the
// view carries a Failed alloc still mid-backoff (`restart_counts < CEILING
// AND last_failure_seen_at populated`). Without this, the broker drains
// empty during the backoff window and the deadline never fires.
//
// The two scenarios below probe the function's boundary semantics
// indirectly through observable broker re-enqueue behaviour, killing
// the four mutants the diff-scoped mutation gate flagged on
// `reconciler_runtime.rs:436` (whole function → false) and
// `reconciler_runtime.rs:441` (`<` → `==`, `>`, `<=`):
//
//   * `view_below_ceiling_with_seen_at_re_enqueues` — count=1 is strictly
//     below CEILING (5); correct semantics return true → broker
//     re-enqueues → queued == 1. Kills `false`, `==`, `>`.
//   * `view_at_ceiling_with_seen_at_does_not_re_enqueue` — count=CEILING
//     means exhausted; correct semantics return false → broker stays
//     empty → queued == 0. Kills `<=` (which would still re-enqueue
//     at-ceiling and is the only mutant the first scenario does not
//     distinguish from `<`).
//
// Together the pair pins `<` strictly: any of `false`, `==`, `>`, or
// `<=` substituted for `<` flips at least one assertion.
// ---------------------------------------------------------------------------

/// Prepare a converged-shaped runtime + Failed alloc + cached
/// `JobLifecycleView` with the supplied `restart_counts` for the
/// alloc, and `last_failure_seen_at` very close to "now" (so the
/// backoff window has NOT elapsed regardless of attempt count). Drives
/// the broker to empty, then runs ONE convergence tick. Returns
/// `queued` after the tick — the load-bearing observable.
async fn run_one_tick_with_seeded_view(restart_counts_value: u32) -> u64 {
    use std::collections::BTreeMap;

    use overdrive_core::UnixInstant;
    use overdrive_core::reconciler::JobLifecycleView;

    let tmp = TempDir::new().expect("tempdir");
    let sim_clock = Arc::new(SimClock::new());

    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    // SimDriver doesn't matter — no Start/Restart action will be
    // dispatched in this test (we seed Failed directly + restart_counts
    // via the cached view to keep the reconcile output empty).
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let state = AppState::new(store, obs, Arc::new(runtime), driver, sim_clock.clone());

    // Seed Job (intent) so hydrate_desired returns Some(job).
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

    // Seed Failed alloc (observation) so hydrate_actual sees a Failed
    // alloc that needs restarting.
    //
    // Per ADR-0037 §4 the reconciler is idempotent on the row's
    // `terminal` field: an at-ceiling Failed row that already
    // carries `Some(BackoffExhausted)` is treated as already-finalised
    // — `reconcile` returns `(Vec::new(), view.clone())`, and the
    // ONLY remaining re-enqueue path is the `view_has_backoff_pending`
    // predicate this test pins. Seeding the terminal field ensures the
    // mutation-killing power of the test holds: the FinalizeFailed
    // path is short-circuited so the predicate's strict-`<` boundary
    // semantics are the load-bearing observable.
    let writer = NodeId::new("local").expect("writer node id");
    let alloc_id = AllocationId::new("alloc-payments-0").expect("valid alloc id");
    let seeded_terminal =
        if restart_counts_value >= overdrive_core::reconciler::RESTART_BACKOFF_CEILING {
            Some(overdrive_core::transition_reason::TerminalCondition::BackoffExhausted {
                attempts: restart_counts_value,
            })
        } else {
            None
        };
    let alloc_row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        job_id: job.id.clone(),
        node_id: writer.clone(),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 1, writer: writer.clone() },
        reason: None,
        detail: None,
        terminal: seeded_terminal,
    };
    state.obs.write(ObservationRow::AllocStatus(alloc_row)).await.expect("seed Failed alloc row");

    // Seed cached view with the supplied restart_counts and
    // last_failure_seen_at = now_unix (zero — the backoff window has
    // NOT elapsed for any non-trivial RESTART_BACKOFF_DURATION).
    let target = TargetResource::new("job/payments").expect("valid target");
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(alloc_id.clone(), restart_counts_value);
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(alloc_id, UnixInstant::from_clock(&*sim_clock));
    let view = JobLifecycleView { restart_counts, last_failure_seen_at };
    state.runtime.seed_job_lifecycle_view_for_test(&target, view);

    // Submit and drain the seed eval — without re-submitting, the
    // broker is empty going into the tick. After the tick, queued
    // reflects ONLY whether `has_work` re-enqueued.
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });
    let now = sim_clock.now();
    let deadline = now + Duration::from_millis(100);
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    for eval in pending {
        run_convergence_tick(&state, &eval.reconciler, &eval.target, now, 0, deadline)
            .await
            .expect("convergence tick succeeds");
    }

    state.runtime.broker().counters().queued
}

/// GIVEN a `JobLifecycleView` with `restart_counts={alloc→1}` (strictly
/// below the CEILING of 5) and `last_failure_seen_at` populated within
/// the unelapsed backoff window —
/// WHEN one convergence tick runs against a Failed alloc whose
/// `reconcile` output is empty (mid-backoff) —
/// THEN the broker is re-enqueued (queued == 1) because
/// `view_has_backoff_pending` returned true under the strict-`<`
/// boundary semantics.
///
/// Kills three of the four diff-scoped mutants on `view_has_backoff_pending`:
///   * the whole function → `false` (would make queued == 0)
///   * `<` → `==` (`1 == 5` is false → queued == 0)
///   * `<` → `>` (`1 > 5` is false → queued == 0)
#[tokio::test]
async fn view_below_ceiling_with_seen_at_re_enqueues() {
    let queued = run_one_tick_with_seeded_view(1).await;
    assert_eq!(
        queued, 1,
        "restart_counts=1 (below CEILING=5) with last_failure_seen_at populated \
         must keep the eval queued via view_has_backoff_pending; got queued={queued}"
    );
}

/// GIVEN a `JobLifecycleView` with `restart_counts={alloc→CEILING}`
/// (exactly the ceiling — backoff budget exhausted) and
/// `last_failure_seen_at` populated —
/// WHEN one convergence tick runs against a Failed alloc whose
/// `reconcile` output is empty —
/// THEN the broker is NOT re-enqueued (queued == 0) because
/// `view_has_backoff_pending` returned false under the strict-`<`
/// boundary semantics (the alloc is terminal-failed, no further
/// restart will fire, so the level-triggered re-enqueue is correctly
/// withheld).
///
/// Kills the fourth diff-scoped mutant on `view_has_backoff_pending`:
///   * `<` → `<=` (`5 <= 5` is true → would re-enqueue → queued == 1)
///
/// Together with `view_below_ceiling_with_seen_at_re_enqueues` above,
/// this pins the `<` semantics strictly at the boundary.
#[tokio::test]
async fn view_at_ceiling_with_seen_at_does_not_re_enqueue() {
    let queued =
        run_one_tick_with_seeded_view(overdrive_core::reconciler::RESTART_BACKOFF_CEILING).await;
    assert_eq!(
        queued, 0,
        "restart_counts=CEILING (terminal-failed) must NOT re-enqueue via \
         view_has_backoff_pending; got queued={queued}"
    );
}

// ---------------------------------------------------------------------------
// drop_job_lifecycle_view_for_test — mutation-gate kill target
//
// Kills the `replace drop_job_lifecycle_view_for_test with ()` mutation
// (reconciler_runtime.rs:499). Without this test, the mutation is invisible
// because the only existing call site immediately re-seeds the view, so the
// drop effect is fully masked.
// ---------------------------------------------------------------------------

/// Seeding a `JobLifecycleView` then dropping it via
/// `drop_job_lifecycle_view_for_test` must leave `view_for_job_lifecycle`
/// returning the default (empty) view. If `drop` is replaced with a no-op,
/// `view_for_job_lifecycle` would still return the previously-seeded
/// non-default view and the assertion below would fail.
#[tokio::test]
async fn drop_job_lifecycle_view_removes_seeded_view() {
    use overdrive_core::reconciler::{JobLifecycleView, TargetResource};
    use std::collections::BTreeMap;

    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let state = build_converged_state(&tmp, clock).await;

    let target = TargetResource::new("job/payments").expect("valid target");
    let alloc_id =
        overdrive_core::id::AllocationId::new("alloc-payments-0").expect("valid alloc id");

    // Seed a non-default view (restart_counts non-empty).
    let mut counts = BTreeMap::new();
    counts.insert(alloc_id.clone(), 2u32);
    let seeded = JobLifecycleView { restart_counts: counts, last_failure_seen_at: BTreeMap::new() };
    state.runtime.seed_job_lifecycle_view_for_test(&target, seeded);

    // Verify the seed is visible before drop.
    let before = state.runtime.view_for_job_lifecycle(&target);
    assert_eq!(
        before.restart_counts.get(&alloc_id).copied(),
        Some(2),
        "seeded view must be visible before drop"
    );

    // Drop the view — after this, view_for_job_lifecycle must return default().
    state.runtime.drop_job_lifecycle_view_for_test(&target);

    let after = state.runtime.view_for_job_lifecycle(&target);
    assert_eq!(
        after,
        JobLifecycleView::default(),
        "view_for_job_lifecycle must return default() after drop_job_lifecycle_view_for_test; \
         got {after:?}"
    );
}
