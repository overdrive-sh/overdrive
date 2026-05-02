//! Step 01-01 — RED scaffold for the `exit_observer` obs-write failure
//! regression.
//!
//! Per `docs/feature/fix-exit-observer-write-retry/deliver/rca.md`:
//! when `obs.write` rejects the exit row, the current observer logs at
//! `warn!` and `continue`s the loop. The alloc's row in obs remains
//! `Running` while the process is dead. There is no retry, no
//! escalation, and no failure event on the lifecycle bus.
//!
//! Two scenarios pin the GREEN-step contract (step 01-02):
//!
//! 1. `transient_obs_write_recovers_on_retry` — inject one transient
//!    `Io(ErrorKind::Interrupted)` failure; the observer must retry
//!    and the alloc must reach `Failed`/`Terminated` in obs. A
//!    `LifecycleEvent { to: Failed }` must arrive on the bus.
//!
//! 2. `terminal_obs_write_escalates_via_lifecycle_event` — inject a
//!    `Io(ErrorKind::PermissionDenied)` failure (and re-inject on
//!    every subsequent attempt to model exhaustion). A degraded
//!    `LifecycleEvent` carrying `TransitionReason::DriverInternalError`
//!    with a detail string naming the obs-store error must arrive on
//!    the bus.
//!
//! Both scenarios PANIC RED on current `exit_observer.rs` (logs and
//! continues): (a) the row never reaches Failed because the prior
//! Running row stands; (b) no degraded event is ever broadcast.
//!
//! Portable across Linux/macOS — `SimDriver` does not require a real
//! kernel — so this file gates only on `#[cfg(feature =
//! "integration-tests")]` (no `target_os = "linux"`).

use std::io;
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
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Harness — mirrors `exit_observer.rs` integration test shape but holds
// the typed `Arc<SimObservationStore>` so the test body can call
// `inject_write_failure` directly.
// -----------------------------------------------------------------------

struct Harness {
    state: AppState,
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
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).expect("register noop");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let node_id = NodeId::new("local").expect("node id");
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs.clone();

    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver.clone();

    let state = AppState::new(store, obs, Arc::new(runtime), driver);

    exit_observer::spawn(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        sim_clock.clone(),
    );

    let job = Job::from_spec(JobSpecInput {
        id: "obswrite".to_string(),
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

    let target = TargetResource::new("job/obswrite").expect("valid target");
    let alloc_id = AllocationId::new("alloc-obswrite-0").expect("alloc id");

    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Harness { state, sim_obs, sim_driver, target, alloc_id, sim_clock, ticker_handle }
}

async fn drive_to_first_running(h: &Harness, start: Instant) {
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut tick_n = 0_u64;
    let mut reached_running = false;
    while tick_n < 30 && !reached_running {
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
        reached_running = rows.iter().any(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    assert!(reached_running, "alloc must reach Running before the test injects exit");
}

// -----------------------------------------------------------------------
// Scenario 1 — transient obs-write failure recovers on retry.
//
// Inject ONE transient `Io(ErrorKind::Interrupted)` write failure.
// The observer must retry and the alloc must reach `Failed` in obs.
// A `LifecycleEvent { to: Failed }` must arrive on the bus.
//
// FAILS RED on current code: `exit_observer.rs:188-196` logs and
// `continue`s; the row is dropped, the alloc stays at the prior
// `Running` row, and no Failed event is broadcast.
// -----------------------------------------------------------------------

#[tokio::test]
async fn transient_obs_write_recovers_on_retry() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();
    drive_to_first_running(&h, start).await;

    let mut events = h.state.lifecycle_events.subscribe();
    while events.try_recv().is_ok() {}

    // Inject ONE transient Io(Interrupted). The observer's first write
    // attempt (after handle_exit_event classifies the crash) gets the
    // injected error; on retry the queue is empty, the write succeeds,
    // and the alloc reaches Failed.
    h.sim_obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
        io::ErrorKind::Interrupted,
    )));

    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_millis(500),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut saw_failed_event = false;
    'outer: for tick_n in 30_u64..80 {
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
        // Yield generously — the inject task, the observer task, and
        // the retry's clock-sleep all need scheduling turns.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        while let Ok(ev) = events.try_recv() {
            if ev.alloc_id == h.alloc_id && ev.to == AllocStateWire::Failed {
                saw_failed_event = true;
                break 'outer;
            }
        }
    }
    assert!(
        saw_failed_event,
        "after one transient obs-write failure the observer must retry and \
         broadcast a Failed LifecycleEvent — the row must NOT be silently \
         dropped (this is the RCA defect)"
    );
}

// -----------------------------------------------------------------------
// Scenario 2 — terminal obs-write failure escalates via lifecycle event.
//
// Inject a `Io(ErrorKind::PermissionDenied)` failure and KEEP injecting
// it on every subsequent attempt to model retry exhaustion. The
// observer must surface a degraded `LifecycleEvent` carrying
// `TransitionReason::DriverInternalError` with a detail string naming
// the obs-store error so subscribers see the failure surface.
//
// FAILS RED on current code: same `continue` arm — no degraded event
// is ever broadcast.
// -----------------------------------------------------------------------

#[tokio::test]
async fn terminal_obs_write_escalates_via_lifecycle_event() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;
    let start = Instant::now();
    drive_to_first_running(&h, start).await;

    let mut events = h.state.lifecycle_events.subscribe();
    while events.try_recv().is_ok() {}

    // Inject a generous run of terminal failures so even an aggressive
    // retry budget exhausts itself. PermissionDenied is classified
    // non-retryable by `is_retryable`, so a single injection should
    // suffice — but stacking covers the case where the observer's
    // retry policy attempts a small number of writes regardless.
    for _ in 0..16 {
        h.sim_obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
    }

    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_millis(500),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut found_degraded_event = None;
    'outer: for tick_n in 30_u64..80 {
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
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        while let Ok(ev) = events.try_recv() {
            if ev.alloc_id == h.alloc_id
                && matches!(ev.reason, TransitionReason::DriverInternalError { .. })
            {
                found_degraded_event = Some(ev);
                break 'outer;
            }
        }
    }
    let ev = found_degraded_event.expect(
        "terminal obs-write failure must escalate via a degraded LifecycleEvent \
         carrying TransitionReason::DriverInternalError — silently logging at warn! \
         is the RCA defect this test defends",
    );
    let TransitionReason::DriverInternalError { detail } = ev.reason else {
        panic!("expected DriverInternalError after terminal obs-write failure");
    };
    assert!(
        !detail.is_empty(),
        "degraded LifecycleEvent's DriverInternalError.detail must name the \
         underlying obs-store error so operators can diagnose; empty detail \
         hides the cause"
    );
}
