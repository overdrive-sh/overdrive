//! Step 02-04 / Slice 3B scenario 3.9 — walking-skeleton:
//! `job_stop_drives_running_to_terminated`.
//!
//! Submits a 1-replica job and drives convergence until alloc reaches
//! Running. Then writes the stop intent (`IntentKey::for_job_stop`)
//! and drives convergence again — alloc must transition Running →
//! Terminated.
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

#[tokio::test]
async fn job_stop_drives_running_to_terminated() {
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

    // Use a distinct job_id so the derived cgroup scope
    // (`alloc-stopper-0.scope`) does not collide with submit_to_running
    // (`alloc-payments-0.scope`) when both tests run in parallel under nextest.
    let job = Job::from_spec(JobSpecInput {
        id: "stopper".to_string(),
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

    let target = TargetResource::new("job/stopper").expect("valid target");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);

    // Drive until Running.
    let mut converged_running = false;
    for tick_n in 0..30_u64 {
        run_convergence_tick(
            &state,
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
    assert!(converged_running, "convergence loop must produce a Running alloc within 30 ticks");

    // Now write the stop intent and drive convergence again.
    let stop_key = IntentKey::for_job_stop(&job.id);
    state.store.put(stop_key.as_bytes(), b"").await.expect("put stop intent");

    let mut converged_terminated = false;
    for tick_n in 30..60_u64 {
        run_convergence_tick(
            &state,
            &target,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if rows.iter().all(|r| r.state == AllocState::Terminated) && !rows.is_empty() {
            converged_terminated = true;
            break;
        }
    }
    assert!(
        converged_terminated,
        "after stop intent is written, all allocs must converge to Terminated within 30 more ticks"
    );
}
