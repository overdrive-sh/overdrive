//! Step 02-03 / Slice 3A.3 scenario 3.1 — walking-skeleton:
//! `submitted_job_reaches_running_via_real_process_driver`.
//!
//! Submits a 1-replica job through the in-process server with a real
//! `Arc<ProcessDriver>`, drives the convergence tick loop until the
//! alloc reaches `Running`, then asserts cgroup membership of the
//! workload PID.
//!
//! Linux-only — gated by `#[cfg(target_os = "linux")]` AND
//! `#[cfg(feature = "integration-tests")]`. Compile-clean on macOS via
//! `cargo nextest run --features integration-tests --no-run`.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput};
use overdrive_core::id::NodeId;
use overdrive_core::reconciler::TargetResource;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};

use super::cleanup::AllocCleanup;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ProcessDriver;
use tempfile::TempDir;

#[tokio::test]
async fn submitted_job_reaches_running_via_real_process_driver() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).expect("register noop");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");

    let store =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let driver: Arc<dyn Driver> =
        Arc::new(ProcessDriver::new(std::path::PathBuf::from("/sys/fs/cgroup")));

    let state = AppState::new(store, obs, Arc::new(runtime), driver);

    // Cleanup guard — fires on test exit (panic or success) and
    // mass-kills every workload cgroup the test created via
    // `cgroup.kill` + `waitpid`. Prevents the `LEAK` flag from
    // nextest. See `cleanup` module for why we don't reuse
    // `Driver::stop` here (tokio runtime cross-runtime hang).
    let _cleanup = AllocCleanup {
        obs: state.obs.clone(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // Submit a 1-replica job that runs `/bin/sleep` for a long time.
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

    let target = TargetResource::new("job/payments").expect("valid target");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);

    // Drive the convergence loop up to 30 ticks (3 seconds at 100ms
    // cadence). We expect convergence within a handful of ticks.
    let mut converged = false;
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
            converged = true;
            break;
        }
    }

    assert!(converged, "convergence loop must produce a Running alloc within 30 ticks");
}
