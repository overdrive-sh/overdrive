//! Step 02-02 integration scenario — `terminal_condition_flows_from_
//! reconciler_through_action_shim_to_row_and_event`.
//!
//! Drives the full action-shim dispatch path under three terminal
//! shapes and asserts the reconciler-emitted `TerminalCondition` lands
//! byte-equal on BOTH `AllocStatusRow.terminal` AND
//! `LifecycleEvent.terminal` from the same dispatch call frame.
//!
//! Per ADR-0037 §4: drift between the two surfaces is structurally
//! impossible because the action shim threads the same Action-derived
//! value onto both writes.
//!
//! Three scenarios:
//!
//! 1. `terminal_backoff_exhausted_appears_on_alloc_status_and_streaming` —
//!    drive a JobLifecycle through the restart budget; both surfaces
//!    carry `Some(BackoffExhausted { attempts: ceiling })`.
//! 2. `terminal_stopped_appears_on_both_surfaces` — issue an explicit
//!    operator stop; both surfaces carry `Some(Stopped { by: Operator })`.
//! 3. `non_terminal_transitions_emit_none` — Pending → Running with
//!    budget remaining: every event/row carries `terminal: None`.
//!
//! Linux-only — gated by `#[cfg(target_os = "linux")]`. Compile-clean
//! on macOS via `cargo nextest run --features integration-tests --no-run`.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::NodeId;
use overdrive_core::reconciler::TargetResource;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;
use tokio::sync::broadcast;

/// Async bootstrap — wires runtime, store, obs, driver, and state with
/// a broadcast subscriber attached BEFORE any tick runs so no
/// `LifecycleEvent` is missed. Async because `runtime.register(...)` is
/// async.
async fn bootstrap_async(
    tmp: &TempDir,
) -> (AppState, broadcast::Receiver<LifecycleEvent>, Arc<overdrive_sim::adapters::clock::SimClock>)
{
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let sim_clock = Arc::new(overdrive_sim::adapters::clock::SimClock::new());
    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(std::path::PathBuf::from("/sys/fs/cgroup"), sim_clock.clone()));

    // SimClock is passed at construction so the convergence-tick's
    // `tick.now_unix` snapshot advances with simulation time. The
    // JobLifecycle backoff predicate compares `tick.now_unix <
    // last_failure_seen_at + backoff_for_attempt(attempts)`; under
    // SystemClock, a tight 200-tick loop completes in real ~ms so the
    // backoff window (1 s × CEILING) never elapses and restart_counts
    // never reaches CEILING. SimClock advances when the background
    // ticker calls `clock.tick(...)` so the backoff window crosses
    // and the reconciler emits FinalizeFailed within the test budget.
    let state = AppState::new(store, obs, Arc::new(runtime), driver, sim_clock.clone());
    let receiver = state.lifecycle_events.subscribe();
    (state, receiver, sim_clock)
}

/// Drain all currently-buffered events from the receiver into a Vec.
/// Used after convergence runs to inspect every emitted event.
fn drain(rx: &mut broadcast::Receiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => break,
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
        }
    }
    out
}

// ---------------------------------------------------------------------------
// AC #7 — terminal_backoff_exhausted_appears_on_alloc_status_and_streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn terminal_backoff_exhausted_appears_on_alloc_status_and_streaming() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, mut rx, sim_clock) = bootstrap_async(&tmp).await;

    // Background ticker — advances logical time so any clock.sleep(...)
    // parked inside the driver wakes promptly.
    let ticker_clock = sim_clock.clone();
    let _ticker = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(100));
            tokio::task::yield_now().await;
        }
    });

    // A job spec whose binary does not exist — every start attempt
    // fails with `StartRejected`, the reconciler increments
    // `restart_counts`, and after `RESTART_BACKOFF_CEILING (5)` the
    // JobLifecycle emits `Action::FinalizeFailed { terminal:
    // BackoffExhausted { attempts: 5 } }`.
    let job = Job::from_spec(JobSpecInput {
        id: "backoff-exhaust".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 50, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/nonexistent/binary".to_string(),
            args: vec![],
        }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let key = IntentKey::for_job(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let target = TargetResource::new("job/backoff-exhaust").expect("valid target");
    let job_lifecycle_name =
        overdrive_core::reconciler::ReconcilerName::new("job-lifecycle").expect("valid name");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(120);

    // Drive convergence until an alloc row carries a terminal claim.
    // The JobLifecycle needs ceiling+1 ticks (5 failed starts + 1 to
    // emit FinalizeFailed) plus headroom for backoff timer ticks.
    let mut terminal_row = None;
    for tick_n in 0..200_u64 {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if let Some(row) = rows.iter().find(|r| {
            r.job_id == job.id
                && matches!(r.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
        }) {
            terminal_row = Some(row.clone());
            break;
        }
    }

    let row = terminal_row.expect(
        "JobLifecycle must produce an AllocStatusRow with terminal = BackoffExhausted within 200 ticks",
    );

    // AC #1 / AC #7 — durable surface carries the terminal claim.
    let row_terminal = row.terminal.clone();
    assert!(
        matches!(row_terminal, Some(TerminalCondition::BackoffExhausted { .. })),
        "AllocStatusRow.terminal must be Some(BackoffExhausted), got {row_terminal:?}",
    );

    // AC #1 / AC #7 — broadcast surface carries byte-equal terminal
    // (constructed in the same dispatch call frame).
    let events = drain(&mut rx);
    let backoff_event = events.iter().find(|ev| {
        ev.alloc_id == row.alloc_id
            && matches!(ev.terminal, Some(TerminalCondition::BackoffExhausted { .. }))
    });
    let event = backoff_event.expect(
        "LifecycleEvent.terminal must carry BackoffExhausted on the same dispatch as the row",
    );

    assert_eq!(
        row.terminal, event.terminal,
        "AllocStatusRow.terminal and LifecycleEvent.terminal must be byte-equal — \
         both surfaces are written from the same Action.terminal in the same call frame",
    );
}

// ---------------------------------------------------------------------------
// AC #8 — terminal_stopped_appears_on_both_surfaces
// ---------------------------------------------------------------------------

#[tokio::test]
async fn terminal_stopped_appears_on_both_surfaces() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, mut rx, sim_clock) = bootstrap_async(&tmp).await;

    let ticker_clock = sim_clock.clone();
    let _ticker = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(100));
            tokio::task::yield_now().await;
        }
    });

    // Distinct job_id so cgroup scope name does not collide with
    // sibling tests under nextest.
    let job = Job::from_spec(JobSpecInput {
        id: "term-stop".to_string(),
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

    let target = TargetResource::new("job/term-stop").expect("valid target");
    let job_lifecycle_name =
        overdrive_core::reconciler::ReconcilerName::new("job-lifecycle").expect("valid name");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(120);

    // Drive until Running.
    let mut converged_running = false;
    for tick_n in 0..30_u64 {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if rows.iter().any(|r| r.state == AllocState::Running) {
            converged_running = true;
            break;
        }
    }
    assert!(converged_running, "must reach Running before issuing stop");

    // Issue operator stop.
    let stop_key = IntentKey::for_job_stop(&job.id);
    state.store.put(stop_key.as_bytes(), b"").await.expect("put stop intent");

    // Drain any pre-stop events; we only care about events from this
    // point forward (operator-stop terminal events).
    let _pre_stop = drain(&mut rx);

    // Drive convergence again — alloc must transition to Terminated
    // and the row + event must carry terminal=Stopped{by:Operator}.
    let mut terminal_row = None;
    for tick_n in 30..120_u64 {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if let Some(row) = rows.iter().find(|r| {
            r.job_id == job.id
                && r.state == AllocState::Terminated
                && matches!(
                    r.terminal,
                    Some(TerminalCondition::Stopped { by: StoppedBy::Operator })
                )
        }) {
            terminal_row = Some(row.clone());
            break;
        }
    }

    let row = terminal_row
        .expect("operator stop must produce a Terminated row with terminal=Stopped{Operator}");

    let events = drain(&mut rx);
    let stop_event = events.iter().find(|ev| {
        ev.alloc_id == row.alloc_id
            && matches!(ev.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::Operator }))
    });
    let event = stop_event
        .expect("LifecycleEvent.terminal must carry Stopped{Operator} from the same dispatch");

    assert_eq!(
        row.terminal, event.terminal,
        "AllocStatusRow.terminal and LifecycleEvent.terminal must be byte-equal on stop",
    );
}

// ---------------------------------------------------------------------------
// AC #9 — non_terminal_transitions_emit_none
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_terminal_transitions_emit_none() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, mut rx, sim_clock) = bootstrap_async(&tmp).await;

    let ticker_clock = sim_clock.clone();
    let _ticker = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(100));
            tokio::task::yield_now().await;
        }
    });

    // Distinct job_id; long-running /bin/sleep so we observe the
    // Pending → Running transition and stop early before any restart
    // budget would be consumed.
    let job = Job::from_spec(JobSpecInput {
        id: "non-term".to_string(),
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

    let target = TargetResource::new("job/non-term").expect("valid target");
    let job_lifecycle_name =
        overdrive_core::reconciler::ReconcilerName::new("job-lifecycle").expect("valid name");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);

    // Drive until Running, then stop early.
    let mut converged_running = false;
    for tick_n in 0..30_u64 {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if rows.iter().any(|r| r.state == AllocState::Running) {
            converged_running = true;
            break;
        }
    }
    assert!(converged_running, "must reach Running");

    // Every emitted event up to this point must carry terminal=None.
    let events = drain(&mut rx);
    assert!(!events.is_empty(), "at least one LifecycleEvent must have been emitted");
    for ev in &events {
        assert_eq!(
            ev.terminal, None,
            "non-terminal LifecycleEvent must carry terminal=None, got {:?} for alloc {}",
            ev.terminal, ev.alloc_id,
        );
    }

    // Every observation row must also carry terminal=None.
    let rows = state.obs.alloc_status_rows().await.expect("read rows");
    for row in &rows {
        assert_eq!(
            row.terminal, None,
            "non-terminal AllocStatusRow must carry terminal=None, got {:?} for alloc {}",
            row.terminal, row.alloc_id,
        );
    }
}
