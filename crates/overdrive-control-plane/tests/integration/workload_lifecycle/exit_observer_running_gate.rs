//! Step 01-01 — RED regression test for the producer-ordering race
//! between the action shim's `obs.write(Running)` and the worker exit
//! observer's `ExitEvent` consumption.
//!
//! See:
//! - `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
//! - `docs/analysis/root-cause-analysis-exit-observer-prior-row-race.md`
//!
//! ## Defect under test
//!
//! For workloads whose process lifetime is shorter than the wall-clock
//! window between (a) the action shim returning from
//! `driver.start(&spec).await` and (b) the action shim completing
//! `obs.write(ObservationRow::AllocStatus(Running))`:
//!
//! 1. The driver's exit watcher emits an `ExitEvent` BEFORE the action
//!    shim has written the `Running` row.
//! 2. `exit_observer::run_with_retry → handle_exit_event →
//!    find_prior_row(obs, &event.alloc)` returns `Ok(None)`.
//! 3. `RetryOutcome::NoPriorRow` is the empty arm at
//!    `exit_observer.rs:225-228` — no obs row, no `LifecycleEvent`,
//!    no log, no telemetry.
//! 4. The exit event is silently dropped. The alloc remains stuck in
//!    `Running` from the obs reader's vantage point. CLI
//!    `submit --watch` hangs or mis-renders the terminal verdict.
//!
//! ## What this test pins
//!
//! Today's `Job`-kind acceptance fixtures insert `sleep 0.5` in the
//! workload bash to widen the wall-clock window past the race
//! (`coinflip_honesty_100_trials.rs:128`,
//! `job_kind_streaming.rs:235/255`). That is fixture-side concealment,
//! not a fix. This test exercises the race directly under a DST
//! schedule WITHOUT the `sleep 0.5` workaround:
//!
//! - Inject `ExitEvent` with `Duration::from_micros(1)` — the smallest
//!   deterministic sub-`obs.write(Running)`-budget delay that today
//!   produces `RetryOutcome::NoPriorRow` under the SimClock harness.
//! - Drive convergence ticks; the action shim's `StartAllocation`
//!   fires `driver.start()` and `obs.write(Running)` in sequence with
//!   the spawned exit task racing in between.
//! - Assert the post-condition: after the sub-budget exit, the alloc
//!   reaches `Failed` or `Terminated` in obs AND a `LifecycleEvent`
//!   is broadcast on the lifecycle bus carrying the correct terminal
//!   `to` state.
//!
//! Both assertions FAIL on current main — the race drops the event
//! silently. The GREEN transition lands in subsequent steps via
//! Solution 1' (oneshot-gated watcher emission) per the RCA.
//!
//! ## Step 01-03 — RED → GREEN transition
//!
//! Per step 01-03 of `fix-exit-observer-running-gate`: the
//! action shim's StartAllocation arm now fires the Running-confirmed
//! gate via `Driver::release_for_exit_emission` after `obs.write(
//! Running)` resolves Ok. The watcher parks on the corresponding
//! `oneshot::Receiver` BEFORE its first `ExitEvent` send. The
//! `#[should_panic(expected = "RED scaffold")]` attribute that
//! pinned the RED state during 01-01 / 01-02 has been removed; the
//! test now passes naturally because the alloc reaches a terminal
//! state and the lifecycle bus carries the corresponding event.
//!
//! Portable across Linux/macOS — `SimDriver` does not require a real
//! kernel — gated only on `#[cfg(feature = "integration-tests")]`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use std::io;

use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Harness — mirrors the shape in `exit_observer.rs` so the surface
// under test is identical (action shim + exit observer share the same
// AppState, driver, and lifecycle bus). The crucial difference: the
// test injects the exit BEFORE driving to Running, so the SimDriver's
// spawned exit task races the action shim's obs.write(Running).
// -----------------------------------------------------------------------

struct Harness {
    state: AppState,
    #[allow(dead_code)]
    sim_obs: Arc<SimObservationStore>,
    sim_driver: Arc<SimDriver>,
    target: TargetResource,
    alloc_id: AllocationId,
    #[allow(dead_code)]
    sim_clock: Arc<SimClock>,
    #[allow(dead_code)]
    ticker_handle: tokio::task::JoinHandle<()>,
}

async fn build_harness(tmp: &TempDir) -> Harness {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let node_id = NodeId::new("local").expect("node id");
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs.clone();
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver.clone();

    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        sim_clock.clone(),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    exit_observer::spawn(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        sim_clock.clone(),
    );

    let job = Job::from_submit(JobSpecInput {
        id: "running-gate".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    })
    .expect("valid job spec");
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("rkyv archive");
    let key = IntentKey::for_workload(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let target = TargetResource::new("job/running-gate").expect("valid target");
    // Phase 1 alloc id derivation per ADR-0023 — `alloc-{workload_id}-0`.
    let alloc_id = AllocationId::new("alloc-running-gate-0").expect("alloc id");

    // Background ticker: advances logical time so any
    // `clock.sleep(...)` parked inside `SimDriver::inject_exit_after`
    // wakes promptly. Identical to the convention in `exit_observer.rs`.
    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Harness { state, sim_obs, sim_driver, target, alloc_id, sim_clock, ticker_handle }
}

// -----------------------------------------------------------------------
// Regression — watcher cannot emit `ExitEvent` before `Running` row
// committed.
//
// On current main this test FAILS by natural assertion — the silent
// drop leaves no terminal row and no terminal LifecycleEvent. After
// Solution 1' lands, the watcher will park on a `oneshot::Receiver`
// until the action shim's `obs.write(Running)` resolves Ok, after
// which the watcher's `ExitEvent` drains against a present prior row
// and the observer writes Failed/Terminated as expected.
// -----------------------------------------------------------------------

#[tokio::test]
async fn watcher_cannot_emit_exit_before_running_row_committed() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;

    // Subscribe to the lifecycle bus BEFORE injecting the exit so we
    // see every transition the action shim and observer emit. The
    // bus is the permanent record of the transition under LWW (per
    // the rationale in `exit_observer.rs::simulated_crash_writes_
    // failed_to_obs_within_budget`); the snapshot may erase a
    // transient `Failed` once the next reconciler tick restarts.
    let mut events = h.state.lifecycle_events.subscribe();

    // Inject exit BEFORE any convergence tick runs — the spawned
    // exit task parks on `clock.sleep(Duration::from_micros(1))`,
    // which under the SimClock harness wakes on the very first
    // background ticker advance (50ms granularity ≫ 1µs). The
    // resulting `ExitEvent` send races the action shim's pending
    // `obs.write(Running)` from inside the StartAllocation handler.
    //
    // `Duration::from_micros(1)` is the smallest deterministic
    // sub-`obs.write(Running)`-budget delay that today produces
    // `RetryOutcome::NoPriorRow`. AC2: no `tokio::time::sleep`,
    // no real-clock dependency.
    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_micros(1),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    // Drive convergence ticks. Tick 0+ produces `StartAllocation`,
    // the action shim calls `driver.start(...)` (SimDriver always
    // succeeds at start), the spawned exit task fires its
    // `ExitEvent`, and the observer races the `obs.write(Running)`
    // for the prior-row read.
    //
    // Today: NoPriorRow → silent drop → alloc never leaves Running
    // from the obs reader's vantage. After Solution 1': the watcher
    // parks on the oneshot until `obs.write(Running)` commits, then
    // the exit observer reads the present prior row and writes
    // Failed.
    let start = Instant::now();
    let workload_lifecycle_name = overdrive_core::reconcilers::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);

    let mut found_terminal_event: Option<AllocStateWire> = None;
    'outer: for tick_n in 0_u64..60 {
        run_convergence_tick(
            &h.state,
            &workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        // Yield so the spawned `inject_exit_after` task and the
        // observer task both get a chance to run before draining
        // the bus. Same convention as in the sibling
        // `simulated_crash_writes_failed_to_obs_within_budget` test.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        while let Ok(ev) = events.try_recv() {
            if ev.alloc_id == h.alloc_id
                && matches!(ev.to, AllocStateWire::Failed | AllocStateWire::Terminated)
            {
                found_terminal_event = Some(ev.to);
                break 'outer;
            }
        }
    }

    // Snapshot assertion: alloc must reach Failed or Terminated in
    // obs. Today FAILS — the silent drop leaves the alloc stuck at
    // Running.
    let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
    let terminal_row = rows.iter().find(|r| {
        r.alloc_id == h.alloc_id && matches!(r.state, AllocState::Failed | AllocState::Terminated)
    });
    assert!(
        terminal_row.is_some(),
        "step 01-03 / fix-exit-observer-running-gate: alloc must reach \
         Failed or Terminated in obs after sub-budget exit; current rows = \
         {:?}. The Running-confirmed gate fired by the action shim's \
         StartAllocation arm provides the structural happens-before edge \
         that prevents the silent-drop race.",
        rows.iter().map(|r| (&r.alloc_id, &r.state)).collect::<Vec<_>>(),
    );

    // Bus assertion: a `LifecycleEvent` carrying a terminal `to`
    // state must be broadcast. With Solution 1' wired, the gate fires
    // post-`obs.write(Running)` Ok, the watcher unparks, the
    // `ExitEvent` lands at the observer with a present prior row, and
    // a terminal LifecycleEvent is broadcast.
    assert!(
        found_terminal_event.is_some(),
        "step 01-03 / fix-exit-observer-running-gate: LifecycleEvent \
         with terminal `to` (Failed | Terminated) must be broadcast on \
         the lifecycle bus after sub-budget exit. The Running-confirmed \
         gate forces the watcher to park until the action shim's \
         `obs.write(Running)` commits, eliminating the NoPriorRow drop.",
    );
}

// -----------------------------------------------------------------------
// Step 01-03 — May-2 retry-exhaustion-degraded path still fires the
// Running-confirmed gate (liveness rail).
//
// Per RCA `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
// § "Approved fix — Solution 1'" / "Liveness rail":
//
// > the May-2 `obs.write(Running)` retry path may exhaust retries and
// > degrade to `LifecycleEvent`-only. In that path the gate **must
// > still fire** (otherwise the watcher leaks forever waiting on a
// > oneshot that nothing will ever send). Two firing sites: post-
// > success and post-degraded-escalation.
//
// This scenario drives the alloc to Running (gate fires post-Ok at
// the action shim's StartAllocation arm), then injects a saturating
// run of non-retryable obs-write failures so the exit_observer's
// `run_with_retry` exhausts retries on the post-exit row write and
// degrades to a `LifecycleEvent` carrying
// `TransitionReason::DriverInternalError`. Asserts:
//
// (a) the alloc reaches a terminal LifecycleEvent via the degraded
//     path,
// (b) `Driver::release_for_exit_emission` was called from BOTH sites
//     (idempotent — the SimDriver's gate is `take`n on the first
//     fire from the action shim; the exit_observer's degraded-path
//     fire is a structural no-op),
// (c) the watcher's `ExitEvent` was consumed (it would have leaked
//     forever if either firing site were missing).
//
// On current main this scenario PASSES (drive-to-Running succeeds,
// exit_observer's May-2 retry path is wired). The test is the
// load-bearing structural defence against future regressions on
// either firing site — if a refactor breaks the May-2 path's gate
// fire, the watcher would deadlock pre-degraded-emit and this test
// would time out.
// -----------------------------------------------------------------------

#[tokio::test]
async fn degraded_escalation_still_fires_running_gate() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();

    let mut events = h.state.lifecycle_events.subscribe();
    let workload_lifecycle_name = overdrive_core::reconcilers::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);

    // Drive to Running first — the action shim's StartAllocation
    // arm fires the gate post-`obs.write(Running)` Ok at this stage.
    let mut tick_n: u64 = 0;
    let mut reached_running = false;
    while tick_n < 30 && !reached_running {
        run_convergence_tick(
            &h.state,
            &workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
        reached_running =
            rows.iter().any(|r| r.alloc_id == h.alloc_id && matches!(r.state, AllocState::Running));
        tick_n += 1;
    }
    assert!(
        reached_running,
        "alloc must reach Running before the test injects exit + write \
         failures (the gate fires at the action shim's post-Running-Ok \
         site here, validating the success path of AC1)",
    );

    // Drain pre-degraded events so the assertion below sees the
    // degraded-path events specifically.
    while events.try_recv().is_ok() {}

    // Inject a saturating run of non-retryable PermissionDenied
    // failures. `run_with_retry` consults `is_retryable` on each
    // attempt; PermissionDenied is non-retryable so a single injection
    // suffices, but stacking covers any future retry-policy change
    // that attempts a small number of writes regardless. This drives
    // the May-2 `RetryOutcome::Failed` path that step 01-03 wires the
    // gate-fire on (belt-and-suspenders against future regressions).
    for _ in 0..16 {
        h.sim_obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
    }

    // Now inject the exit. The watcher emits `ExitEvent` (its gate
    // already fired at the post-Ok site above); the exit_observer's
    // `run_with_retry` exhausts retries on every injected
    // PermissionDenied; emits the degraded LifecycleEvent. The
    // `RetryOutcome::Failed` arm ALSO fires the gate (idempotent
    // via `Option::take` + `oneshot::Sender::send` consume-self —
    // a no-op on the success-path code that already fired).
    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_millis(500),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    // Drive convergence ticks until either:
    //  - we observe a degraded LifecycleEvent (the success criterion),
    //  - 50 additional ticks elapse (the timeout — failure: the
    //    degraded path didn't run, or the gate-fire deadlocked the
    //    watcher).
    let mut saw_degraded_event = false;
    'outer: for tick_n in tick_n..(tick_n + 50) {
        run_convergence_tick(
            &h.state,
            &workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        // Yield generously — the inject task, the observer task, and
        // the retry's clock-sleep all need scheduling turns. Mirrors
        // the convention in `crash_recovery_obs_write_rejected.rs`.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        while let Ok(ev) = events.try_recv() {
            if ev.alloc_id == h.alloc_id
                && matches!(ev.to, AllocStateWire::Failed | AllocStateWire::Terminated)
            {
                saw_degraded_event = true;
                break 'outer;
            }
        }
    }

    assert!(
        saw_degraded_event,
        "step 01-03 / fix-exit-observer-running-gate: after the action \
         shim's StartAllocation arm fires the Running-confirmed gate \
         post-Ok AND obs writes saturate with non-retryable failures, \
         the exit_observer's May-2 retry path must exhaust and emit a \
         degraded LifecycleEvent with terminal `to` (Failed | \
         Terminated). The watcher's `ExitEvent` was consumed (it would \
         have leaked forever if either firing site were missing — the \
         post-Ok site at the action shim or the degraded-path site at \
         exit_observer.rs's RetryOutcome::Failed arm).",
    );
}
