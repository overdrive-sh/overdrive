//! Step 01-01 — RED scaffold for the exit-event abstraction.
//!
//! Pins the public Driver/SimDriver/worker exit-observer surface that
//! Step 01-02 (GREEN) will land. Each test references symbols that do
//! NOT resolve against current main:
//!
//! * `overdrive_core::traits::driver::ExitEvent`
//! * `overdrive_core::traits::driver::ExitKind`
//! * `overdrive_sim::adapters::driver::SimDriver::inject_exit_after`
//! * `overdrive_control_plane::worker::exit_observer::spawn`
//!
//! The compile failure (`unresolved import`) IS the RED state — see
//! `.claude/rules/testing.md` §"RED scaffolds and intentionally-failing
//! commits". The production fix lands in Step 01-02; this step adds
//! tests only.
//!
//! Portable across Linux/macOS — `SimDriver` does not require a real
//! kernel — so this file gates only on `#[cfg(feature =
//! "integration-tests")]` (no `target_os = "linux"`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconciler::TargetResource;
use overdrive_core::traits::driver::{AllocationHandle, Driver, DriverType, ExitEvent, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// The crate path below is the load-bearing GREEN-step landing zone:
// the worker subsystem `exit_observer::spawn` is what consumes
// `ExitEvent`s from the driver and writes `Failed`/`Terminated` rows
// into obs. Step 01-02 will publish it under this exact path.
use overdrive_control_plane::worker::exit_observer;

// -----------------------------------------------------------------------
// Helpers — duplicated per the convention in `submit_to_running.rs` /
// `crash_recovery.rs` (each scenario file is self-contained). Builds a
// fully-wired AppState whose convergence loop is driven by `SimDriver`,
// while keeping a typed `Arc<SimDriver>` handle for `inject_exit_after`.
// -----------------------------------------------------------------------

struct Harness {
    state: AppState,
    sim_driver: Arc<SimDriver>,
    target: TargetResource,
    alloc_id: AllocationId,
    /// Shared with `sim_driver` so the background `_ticker` task
    /// advances logical time past `inject_exit_after`'s deadline.
    /// Per `.claude/rules/development.md` § "Production code is not
    /// shaped by simulation": `SimClock::sleep` parks until the
    /// harness ticks; `inject_exit_after`'s spawned task is no
    /// exception. Held on the harness so a future test that wants
    /// fine-grained tick control can read it; not currently consumed
    /// by any test body.
    #[allow(dead_code)]
    sim_clock: Arc<SimClock>,
    /// Background ticker task that advances `sim_clock` continuously
    /// so `inject_exit_after` deadlines fire promptly. Held in the
    /// harness so it lives for the duration of the test; dropped at
    /// test exit. Held by binding (not `_`-prefixed) so clippy's
    /// `used_underscore_binding` is satisfied while we keep the
    /// `JoinHandle` alive — dropping it cancels the spawned task.
    #[allow(dead_code)]
    ticker_handle: tokio::task::JoinHandle<()>,
}

async fn build_harness(tmp: &TempDir) -> Harness {
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).expect("register noop");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let node_id = NodeId::new("local").expect("node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    // SimDriver is the deterministic exit-event source under DST. The
    // production `ExecDriver` has its own watcher per RCA §Approved fix
    // items 1-3; the SimDriver injection API mirrors it for test code.
    // Share the SimClock with the harness so the test can drive
    // logical time past `inject_exit_after`'s deadline.
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver.clone();

    let state = AppState::new(store, obs, Arc::new(runtime), driver);

    // Spawn the worker-side exit observer. Step 01-02 wires this into
    // `run_server_with_obs_and_driver`; tests construct it directly so
    // the assertion surface is unambiguous about which subsystem wrote
    // the obs row. The signature below is the GREEN landing zone — the
    // exact parameter list may evolve, but the test author's view is:
    // "give me the obs sink and the driver's exit-event source, and
    // you'll write classified rows for me."
    exit_observer::spawn(state.obs.clone(), state.driver.clone(), state.lifecycle_events.clone());

    let job = Job::from_spec(JobSpecInput {
        id: "exitobs".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let key = IntentKey::for_job(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let target = TargetResource::new("job/exitobs").expect("valid target");
    // Phase 1 alloc id derivation per ADR-0023 — `alloc-{job_id}-0`.
    let alloc_id = AllocationId::new("alloc-exitobs-0").expect("alloc id");

    // Background ticker: advances logical time so any
    // `clock.sleep(...)` parked inside `SimDriver::inject_exit_after`
    // wakes promptly. The test never drops this handle until the
    // harness itself is dropped at end-of-test.
    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Harness { state, sim_driver, target, alloc_id, sim_clock, ticker_handle }
}

async fn drive_to_first_running(h: &Harness, start: Instant) -> AllocStatusRow {
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut tick_n = 0_u64;
    let mut running: Option<AllocStatusRow> = None;
    while tick_n < 30 && running.is_none() {
        run_convergence_tick(
            &h.state,
            &job_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
        running = rows.into_iter().find(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    running.expect("alloc must reach Running")
}

async fn drive_ticks(h: &Harness, start: Instant, range: std::ops::Range<u64>) {
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    for tick_n in range {
        run_convergence_tick(
            &h.state,
            &job_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
    }
}

// -----------------------------------------------------------------------
// Test 1 — simulated crash writes Failed within budget.
// -----------------------------------------------------------------------

#[tokio::test]
async fn simulated_crash_writes_failed_to_obs_within_budget() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();
    let prior_running_row = drive_to_first_running(&h, start).await;

    // Subscribe to the lifecycle event bus BEFORE injecting the
    // exit. The `Failed` row is transient under LWW: once the
    // observer writes it, the next tick's reconciler emits
    // `RestartAllocation` and the action shim writes a fresh
    // `Running` row at a higher counter, dominating the observer's
    // `Failed` and erasing it from the snapshot. The bus is the
    // permanent record of the transition — every consumer
    // (streaming `submit --watch`, metrics, audit) reads it instead
    // of the snapshot for transient state.
    //
    // The harness wires `exit_observer::spawn` against the same bus
    // the action shim emits on, so subscribers see ordered emissions
    // from both writers.
    let mut events = h.state.lifecycle_events.subscribe();

    // Drain any startup events emitted during `drive_to_first_running`
    // so the test only inspects post-injection emissions.
    while events.try_recv().is_ok() {}

    // Inject a non-zero exit (signal=None means it's a clean wait()
    // result, exit_code=Some(1) ⇒ Crashed classification).
    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_millis(500),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    // Drive ticks ONE AT A TIME and drain bus events between each.
    // The spawned `inject_exit_after` task fires during the tick's
    // awaits; the observer receives the event and broadcasts a
    // `LifecycleEvent { to: Failed }` after the obs write. The test
    // breaks as soon as that event arrives, BEFORE the next tick's
    // reconciler emits `RestartAllocation`.
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut found_failed_at: Option<String> = None;
    'outer: for tick_n in 30_u64..50 {
        run_convergence_tick(
            &h.state,
            &job_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        // Yield so the spawned `inject_exit_after` task and the
        // observer task both get a chance to run before we drain
        // the bus. Under single-threaded tokio the inject task
        // double-yields (`SimClock::sleep` → `yield_now`, then
        // `mpsc::send`); the observer needs another scheduling
        // turn to drain the receiver, write obs, and broadcast.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        while let Ok(ev) = events.try_recv() {
            if ev.alloc_id == h.alloc_id && ev.to == AllocStateWire::Failed {
                found_failed_at = Some(ev.at);
                break 'outer;
            }
        }
    }
    let at = found_failed_at.expect("watcher must emit Failed transition within 2s budget");
    // Strict LWW dominance: the Failed transition's counter must
    // strictly exceed the prior Running row's counter so observers
    // converge. The `at` field carries `counter@writer`.
    let counter: u64 = at
        .split_once('@')
        .and_then(|(c, _)| c.parse().ok())
        .expect("LifecycleEvent.at carries `counter@writer`");
    assert!(
        counter > prior_running_row.updated_at.counter,
        "Failed event counter ({counter}) must strictly dominate prior Running counter ({})",
        prior_running_row.updated_at.counter
    );
}

// -----------------------------------------------------------------------
// Test 2 — intentional stop classifies as Terminated, NOT Failed.
// -----------------------------------------------------------------------

#[tokio::test]
async fn simulated_intentional_stop_writes_terminated_to_obs() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();
    let _prior_running_row = drive_to_first_running(&h, start).await;

    // Stop is operator-driven: the action shim calls Driver::stop with
    // an AllocationHandle whose alloc field matches `alloc_id`. The
    // intentional_stop flag propagates through the watcher.
    let handle = AllocationHandle { alloc: h.alloc_id.clone(), pid: None };
    h.state.driver.stop(&handle).await.expect("driver stop");

    // Inject natural exit immediately AFTER stop — the watcher must
    // honour intentional_stop=true and classify as Terminated.
    h.sim_driver.inject_exit_after(&h.alloc_id, Duration::ZERO, ExitKind::CleanExit);

    drive_ticks(&h, start, 30_u64..50).await;

    let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
    let terminated = rows
        .iter()
        .find(|r| r.state == AllocState::Terminated)
        .expect("post-stop natural exit must classify as Terminated");
    assert_eq!(terminated.state, AllocState::Terminated);
    // No Failed row should ever appear for this alloc — the
    // intentional_stop flag is the entire defense.
    assert!(
        !rows.iter().any(|r| matches!(r.state, AllocState::Failed)),
        "no Failed row may appear after operator-stop; intentional_stop discriminator failed"
    );
}

// -----------------------------------------------------------------------
// Test 3 — DST-shaped invariant: crashed alloc eventually leaves Running.
// -----------------------------------------------------------------------

#[tokio::test]
async fn crashed_alloc_eventually_reaches_non_running() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();
    let prior = drive_to_first_running(&h, start).await;
    let prior_counter = prior.updated_at.counter;

    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(137), signal: None },
    );

    // RESTART_BACKOFF_CEILING (5) × RESTART_BACKOFF_DURATION (1s) plus
    // slack at 100ms-per-tick cadence ≈ 60 ticks. Any value strictly
    // greater than prior_counter that resolves to Failed or fresh
    // Running is acceptable — what is forbidden is staying stuck at
    // the original Running row's counter.
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut left_running = false;
    for tick_n in 30_u64..90 {
        run_convergence_tick(
            &h.state,
            &job_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
        let post = rows.iter().find(|r| r.alloc_id == h.alloc_id);
        if let Some(row) = post
            && row.updated_at.counter > prior_counter
            && (matches!(row.state, AllocState::Failed) || row.state == AllocState::Running)
        {
            left_running = true;
            break;
        }
    }
    assert!(
        left_running,
        "crashed alloc must reach Failed or fresh Running within 60 ticks; \
         stuck-at-Running is the bug this test defends"
    );
}

// -----------------------------------------------------------------------
// Test 4 — deterministic stop-vs-natural-exit race serialisation.
// -----------------------------------------------------------------------

#[tokio::test]
async fn intentional_stop_flag_serialises_with_natural_exit_race() {
    // Subcase A: stop FIRST, then natural exit. The watcher reads
    // intentional_stop=true and classifies as Terminated.
    {
        let tmp = TempDir::new().expect("tempdir");
        let h = build_harness(&tmp).await;
        let start = Instant::now();
        let _prior = drive_to_first_running(&h, start).await;

        let handle = AllocationHandle { alloc: h.alloc_id.clone(), pid: None };
        // Same logical tick: stop sets intentional_stop, exit fires.
        h.state.driver.stop(&handle).await.expect("driver stop");
        h.sim_driver.inject_exit_after(
            &h.alloc_id,
            Duration::ZERO,
            ExitKind::Crashed { exit_code: Some(1), signal: None },
        );

        drive_ticks(&h, start, 30_u64..50).await;
        let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
        assert!(
            rows.iter().any(|r| r.state == AllocState::Terminated),
            "operator-stop wins the race when stop precedes exit"
        );
        assert!(
            !rows.iter().any(|r| matches!(r.state, AllocState::Failed)),
            "no Failed row should appear when intentional_stop was set first"
        );
    }

    // Subcase B: natural exit FIRST, then stop. intentional_stop was
    // not yet set when the watcher read it — exit classifies as
    // Crashed → Failed. The post-hoc stop is idempotent.
    {
        let tmp = TempDir::new().expect("tempdir");
        let h = build_harness(&tmp).await;
        let start = Instant::now();
        let _prior = drive_to_first_running(&h, start).await;

        // Subscribe BEFORE injection so we capture every transition
        // the observer broadcasts. Same rationale as test 1: under
        // LWW, the reconciler's restart immediately after Failed
        // erases the Failed row from the snapshot, so the lifecycle
        // bus is the only stable observation surface for transient
        // transitions.
        let mut events = h.state.lifecycle_events.subscribe();
        while events.try_recv().is_ok() {}

        h.sim_driver.inject_exit_after(
            &h.alloc_id,
            Duration::ZERO,
            ExitKind::Crashed { exit_code: Some(1), signal: None },
        );

        let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
            .expect("job-lifecycle reconciler name");
        let deadline = start + Duration::from_secs(120);
        let mut saw_failed = false;
        'outer: for tick_n in 30_u64..50 {
            run_convergence_tick(
                &h.state,
                &job_lifecycle_name,
                &h.target,
                start + Duration::from_millis(tick_n.saturating_mul(100)),
                tick_n,
                deadline,
            )
            .await
            .expect("tick");
            // Yield so the spawned inject task and the observer
            // both get a chance to run before draining the bus
            // (see `simulated_crash_writes_failed_to_obs_within_budget`).
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            while let Ok(ev) = events.try_recv() {
                if ev.alloc_id == h.alloc_id && ev.to == AllocStateWire::Failed {
                    saw_failed = true;
                    break 'outer;
                }
            }
        }
        assert!(saw_failed, "natural exit before stop must classify as Failed (Crashed)");

        let handle = AllocationHandle { alloc: h.alloc_id.clone(), pid: None };
        let _ = h.state.driver.stop(&handle).await; // idempotent
    }

    // Pin the ExitEvent type symbol — Step 01-02 lands the concrete
    // shape; the import alone is enough to make this a RED reference.
    let _ = std::any::type_name::<ExitEvent>();
}
