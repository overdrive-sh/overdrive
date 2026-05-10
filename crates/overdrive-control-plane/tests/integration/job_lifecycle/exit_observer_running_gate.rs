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
//! ## RED scaffold convention
//!
//! Per `.claude/rules/testing.md` § "Test-side scaffolds — `#[should_panic(
//! expected = \"RED scaffold\")]`": this test uses the sanctioned RED
//! shape. Today's assertions panic with a message containing
//! `"RED scaffold"` (the silent-drop race leaves the alloc stuck at
//! Running with no terminal LifecycleEvent), so the test PASSES under
//! nextest while structurally pinning the defect — the moment Solution
//! 1' (oneshot-gated watcher emission) lands and the assertions stop
//! firing, the `#[should_panic]` will trip and flag the test for
//! review at the GREEN transition (drop the attribute, the test goes
//! green by virtue of the post-condition now holding).
//!
//! Portable across Linux/macOS — `SimDriver` does not require a real
//! kernel — gated only on `#[cfg(feature = "integration-tests")]`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconciler::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
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
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let node_id = NodeId::new("local").expect("node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver.clone();

    let state = AppState::new(
        store,
        obs,
        Arc::new(runtime),
        driver,
        sim_clock.clone(),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
    );

    exit_observer::spawn(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        sim_clock.clone(),
    );

    let job = Job::from_spec(JobSpecInput {
        id: "running-gate".to_string(),
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

    let target = TargetResource::new("job/running-gate").expect("valid target");
    // Phase 1 alloc id derivation per ADR-0023 — `alloc-{job_id}-0`.
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

    Harness { state, sim_driver, target, alloc_id, sim_clock, ticker_handle }
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
#[should_panic(expected = "RED scaffold")]
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
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);

    let mut found_terminal_event: Option<AllocStateWire> = None;
    'outer: for tick_n in 0_u64..60 {
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
        "RED scaffold (step 01-01 / fix-exit-observer-running-gate): \
         alloc must reach Failed or Terminated in obs after sub-budget exit; \
         current rows = {:?}; \
         the silent-drop race in exit_observer.rs:225 (RetryOutcome::NoPriorRow) \
         is the defect this test pins. GREEN landing zone is Solution 1' \
         (oneshot-gated watcher emission) per RCA \
         docs/feature/fix-exit-observer-running-gate/deliver/rca.md",
        rows.iter().map(|r| (&r.alloc_id, &r.state)).collect::<Vec<_>>(),
    );

    // Bus assertion: a `LifecycleEvent` carrying a terminal `to`
    // state must be broadcast. Today FAILS — `RetryOutcome::NoPriorRow`
    // emits no event (`exit_observer.rs:225-228` is the empty arm).
    assert!(
        found_terminal_event.is_some(),
        "RED scaffold (step 01-01 / fix-exit-observer-running-gate): \
         LifecycleEvent with terminal `to` (Failed | Terminated) must be \
         broadcast on the lifecycle bus after sub-budget exit; the silent-drop \
         race in exit_observer.rs:225 (RetryOutcome::NoPriorRow) emits nothing. \
         GREEN landing zone is Solution 1' (oneshot-gated watcher emission) per \
         RCA docs/feature/fix-exit-observer-running-gate/deliver/rca.md",
    );
}
