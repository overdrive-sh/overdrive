//! Step 02-03 / Slice 3A.3 scenario 3.7 — walking-skeleton:
//! `killed_workload_is_restarted_with_fresh_alloc_id`.
//!
//! Submits a 1-replica job; waits until the alloc is Running; SIGKILLs
//! the workload externally; drives the convergence loop forward; and
//! asserts the alloc state transitions through Terminated → Running
//! again under the (deterministic, same) `alloc_id` (Phase 1 reuses
//! `mint_alloc_id(job_id)` per ADR-0023).
//!
//! The "fresh `alloc_id`" framing in the scenario name reflects the
//! Phase-2+ direction; in Phase 1 single-mode the alloc id is a pure
//! function of the job id (`alloc-{job_id}-0`), so observable rebirth
//! is the state transition Terminated → Running with a distinct PID
//! at the driver layer.
//!
//! Linux-only — gated by `#[cfg(target_os = "linux")]` AND
//! `#[cfg(feature = "integration-tests")]`. Compile-clean on macOS via
//! `cargo nextest run --features integration-tests --no-run`.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

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
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

use super::cleanup::AllocCleanup;

#[tokio::test]
async fn killed_workload_is_restarted_with_fresh_alloc_id() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).expect("register noop");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(std::path::PathBuf::from("/sys/fs/cgroup")));

    let state = AppState::new(store, obs, Arc::new(runtime), driver);

    // Cleanup guard — fires when the test exits (panic or success) and
    // mass-kills every workload cgroup the test created via
    // `cgroup.kill`. Prevents the `LEAK` flag from nextest without
    // depending on the `tokio::test` runtime that owns the `Child`
    // handles being alive at drop time.
    let _cleanup = AllocCleanup {
        obs: state.obs.clone(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // Use a distinct job_id so the derived cgroup scope
    // (`alloc-recovery-0.scope`) does not collide with the scope used by
    // submit_to_running (`alloc-payments-0.scope`) when both tests run in
    // parallel under nextest.
    let job = Job::from_spec(JobSpecInput {
        id: "recovery".to_string(),
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

    let target = TargetResource::new("job/recovery").expect("valid target");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    // Phase 1: drive to first Running.
    let mut tick_n = 0_u64;
    let mut first_running = false;
    while tick_n < 30 && !first_running {
        run_convergence_tick(
            &state,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        first_running = rows.iter().any(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    assert!(first_running, "alloc must reach Running before crash");

    // Phase 2: simulate crash by writing a Terminated row directly
    // into the observation store. This stands in for an external
    // SIGKILL detection path (kernel signal → node agent → obs
    // write). Phase 1 has no real crash detector wired yet — the
    // direct-write models the post-detection state the reconciler
    // would observe.
    //
    // The crash-row counter is `prior + 1` — just enough to win LWW
    // against the Phase-1 Running row. The action shim's
    // `timestamp_for(tick)` writes counter = `tick.tick + 1`, so for
    // the Phase-3 restart write to dominate the crash row we must also
    // skip `tick_n` forward by 1 below — otherwise the next shim write
    // would tie or lose. An earlier draft used `+100` which deadlocked
    // the test: subsequent shim writes (counter ≤ 60) all lost LWW
    // against a counter=~103 Terminated row, so Running was never
    // re-observable.
    let rows = state.obs.alloc_status_rows().await.expect("read rows");
    let prior = rows.into_iter().find(|r| r.state == AllocState::Running).expect("running row");
    let crashed_counter = prior.updated_at.counter.saturating_add(1);
    let crashed = overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: prior.alloc_id.clone(),
        job_id: prior.job_id.clone(),
        node_id: prior.node_id.clone(),
        state: AllocState::Terminated,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: crashed_counter,
            writer: prior.node_id.clone(),
        },
    };
    state
        .obs
        .write(overdrive_core::traits::observation_store::ObservationRow::AllocStatus(crashed))
        .await
        .expect("write crash");

    // Skip `tick_n` past the crash counter so the next shim write
    // (counter = `tick.tick + 1`) strictly dominates the crash row
    // under LWW. Without this, a tied (counter, writer) pair is a
    // no-op per the §4 LWW idempotency rule and the Running row is
    // silently dropped on every restart attempt.
    if tick_n < crashed_counter {
        tick_n = crashed_counter;
    }

    // Phase 3: drive the convergence loop forward and observe
    // Terminated → Running. The reconciler emits RestartAllocation
    // for the Failed alloc (within ceiling); the action shim
    // performs stop+start and writes a fresh Running row.
    let mut recovered = false;
    while tick_n < 60 && !recovered {
        run_convergence_tick(
            &state,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        recovered = rows.iter().any(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    assert!(
        recovered,
        "alloc must reach Running again after crash within the Phase-3 tick budget \
         (final tick_n={tick_n})"
    );
}
