//! Step 02-03 / Slice 3A.3 scenario 3.8 — backoff exhaustion.
//!
//! Drives the runtime convergence tick loop against a `SimDriver` that
//! force-fails every `Driver::start` call. After M+1 ticks, the
//! `JobLifecycle` reconciler must STOP emitting `RestartAllocation`
//! actions for the persistently-failing alloc — the alloc state stays
//! Terminated (Phase 1's `Failed`-equivalent) and `Driver::start` is
//! NOT invoked again past the configured ceiling.
//!
//! Default-lane (in-memory). Enters via the runtime tick loop's public
//! API (`run_convergence_tick`) and asserts at the `ObservationStore`
//! boundary. Port-to-port: no internal class is touched by the test
//! harness; the `SimDriver` is the driven-port boundary fixture.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput};
use overdrive_core::id::{JobId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// SimDriver-shaped driver that records every `Driver::start` call and
/// rejects every one with `StartRejected`. Used by the backoff
/// exhaustion test to count how many start attempts the convergence
/// loop made.
struct AlwaysFailDriver {
    start_count: Arc<Mutex<u32>>,
}

impl AlwaysFailDriver {
    fn new() -> Self {
        Self { start_count: Arc::new(Mutex::new(0)) }
    }

    fn count_handle(&self) -> Arc<Mutex<u32>> {
        Arc::clone(&self.start_count)
    }
}

#[async_trait]
impl Driver for AlwaysFailDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Process
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        {
            let mut n = self.start_count.lock().expect("mutex");
            *n = n.saturating_add(1);
        }
        Err(DriverError::StartRejected {
            driver: DriverType::Process,
            reason: "deliberate failure injection for backoff test".to_string(),
        })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

fn build_state_with_driver(tmp: &TempDir, driver: Arc<dyn Driver>) -> AppState {
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    // Phase 1 single-node: the runtime tick loop's `hydrate_desired`
    // injects a hardcoded baseline `local` node with abundant capacity.
    // Tests do not seed node-registration intent — that would invert
    // the dependency direction.

    AppState::new(store, obs, Arc::new(runtime), driver)
}

#[tokio::test]
async fn repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying() {
    use overdrive_control_plane::reconciler_runtime::run_convergence_tick;

    let tmp = TempDir::new().expect("tempdir");
    let driver = Arc::new(AlwaysFailDriver::new());
    let count_handle = driver.count_handle();
    let driver_dyn: Arc<dyn Driver> = driver.clone();
    let state = build_state_with_driver(&tmp, driver_dyn);

    // Submit a 1-replica job. The submit goes through the IntentStore
    // directly (the test does not need the HTTP boundary here).
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

    // Drive the convergence tick loop M+1 times. The current ceiling
    // is 5 attempts (per `JobLifecycle::reconcile`'s
    // `RESTART_BACKOFF_CEILING`). At tick 6 the reconciler must
    // recognise the ceiling and emit no further StartAllocation.
    //
    // We use a target tick budget large enough to comfortably exceed
    // the ceiling; the assertion later pins the exact upper bound.
    let target = JobId::new("payments").expect("valid job id");
    let target_resource = overdrive_core::reconciler::TargetResource::new(&format!("job/{target}"))
        .expect("valid target");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);
    for tick_n in 0..20_u64 {
        run_convergence_tick(
            &state,
            &target_resource,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
    }

    // Assert: the alloc landed in Terminated state (Phase 1's
    // Failed-equivalent) and Driver::start was called at most
    // `RESTART_BACKOFF_CEILING + 1` times — once for the initial
    // start, and at most ceiling-many restarts before exhaustion.
    let rows = state.obs.alloc_status_rows().await.expect("read alloc rows");
    let payments_rows: Vec<_> = rows.iter().filter(|r| r.job_id == target).collect();
    assert!(!payments_rows.is_empty(), "alloc rows must exist for the submitted job");

    let final_count = *count_handle.lock().expect("mutex");
    assert!(
        final_count <= 6,
        "Driver::start must stop being invoked once backoff is exhausted; got {final_count} starts"
    );
    assert!(final_count >= 1, "Driver::start must be invoked at least once; got {final_count}");

    // Every alloc row for this job must be Terminated (no Running).
    for row in &payments_rows {
        assert_eq!(
            row.state,
            AllocState::Terminated,
            "all-failed allocs must converge to Terminated; got {row:?}"
        );
    }
}
