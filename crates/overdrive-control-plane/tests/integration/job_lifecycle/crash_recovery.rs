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
    let job_lifecycle_name = overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle reconciler name");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    // Phase 1: drive to first Running.
    let mut tick_n = 0_u64;
    let mut first_running = false;
    while tick_n < 30 && !first_running {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
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

    // Phase 2: simulate crash by SIGKILLing the workload PID
    // externally. Step 01-02's worker exit-observer subsystem reads
    // the natural `child.wait()` resolution and writes
    // `AllocState::Failed` to obs — the synthetic-write workaround
    // that previously stood in here is gone.
    //
    // Read the workload PID from `cgroup.procs`. The action shim has
    // already written a `Running` row at this point, and `ExecDriver`
    // has placed the spawned `/bin/sleep` PID into the workload scope
    // (same pattern `AllocCleanup` uses for cleanup). The PID is the
    // SIGKILL target.
    let rows = state.obs.alloc_status_rows().await.expect("read rows");
    let prior = rows.into_iter().find(|r| r.state == AllocState::Running).expect("running row");
    let scope = std::path::PathBuf::from("/sys/fs/cgroup")
        .join("overdrive.slice/workloads.slice")
        .join(format!("{}.scope", prior.alloc_id));
    let procs_text =
        std::fs::read_to_string(scope.join("cgroup.procs")).expect("read cgroup.procs");
    let pid: libc::pid_t = procs_text
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .next()
        .expect("workload PID present in cgroup.procs");
    // SAFETY: SIGKILL on a child PID owned by this test. The PID was
    // minted by `ExecDriver::start` in this same test and resides in
    // the workload's cgroup scope; it is alive at the moment we read
    // it from `cgroup.procs`. `libc::kill` returns 0 on success and
    // -1 on error (with `errno` set); we ignore the return because
    // the assertion downstream is on the obs row the watcher writes,
    // not on the syscall result.
    let _ = unsafe { libc::kill(pid, libc::SIGKILL) };

    // Phase 3: drive the convergence loop forward. The exit-observer
    // subsystem must (i) classify the SIGKILL as a crash (no
    // `intentional_stop` flag was set — RCA §Approved fix item 4),
    // writing `AllocState::Failed`, and (ii) the reconciler must
    // re-enqueue and bring up a fresh Running row whose counter
    // strictly dominates the Failed row.
    let mut saw_failed = false;
    let mut failed_counter: u64 = 0;
    while tick_n < 90 && !saw_failed {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if let Some(row) = rows.iter().find(|r| matches!(r.state, AllocState::Failed { .. })) {
            saw_failed = true;
            failed_counter = row.updated_at.counter;
        }
        tick_n += 1;
    }
    assert!(saw_failed, "watcher must classify SIGKILL as Failed");

    let mut recovered = false;
    while tick_n < 150 && !recovered {
        run_convergence_tick(
            &state,
            &job_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        if let Some(row) = rows.iter().find(|r| r.state == AllocState::Running)
            && row.updated_at.counter > failed_counter
        {
            recovered = true;
        }
        tick_n += 1;
    }
    assert!(
        recovered,
        "alloc must reach a fresh Running (counter > Failed counter) after SIGKILL \
         within the Phase-3 tick budget (final tick_n={tick_n})"
    );
}
