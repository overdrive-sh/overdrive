//! Regression test: submitting a `WorkloadIntent::Service` and running
//! a convergence tick must not panic.
//!
//! Prior to the fix, `read_job` contained an `unreachable!()` for the
//! `Service` and `Schedule` variants. When `submit_workload` accepted a
//! Service intent and enqueued a lifecycle eval, the convergence loop
//! called `hydrate_desired` → `read_job`, which panicked on the Service
//! variant — killing the entire `spawn_convergence_loop` task.
//!
//! The fix returns `Ok(None)` for non-Job variants. This test pins that
//! the convergence tick completes without panic for a Service workload.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{DriverInput, ExecInput, IntentKey, ResourcesInput, WorkloadKind};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::NodeId;
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

async fn build_state(tmp: &TempDir, clock: Arc<SimClock>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        NodeId::new("writer-1").unwrap(),
        allocator,
    )
}

/// Regression: `read_job` must return `Ok(None)` for Service workloads,
/// not panic via `unreachable!()`. A convergence tick against a stored
/// `WorkloadIntent::Service` must complete without panic.
#[tokio::test]
async fn service_workload_convergence_tick_does_not_panic() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = Arc::new(SimClock::new());
    let state = build_state(&tmp, clock.clone()).await;

    let svc = overdrive_core::aggregate::ServiceV1::from_submit(ServiceSpecInput {
        id: "web-frontend".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
    })
    .expect("valid service spec");

    let workload_id = svc.id.clone();
    let intent = overdrive_core::aggregate::WorkloadIntent::Service(svc);
    let archived = intent.archive_for_store().expect("rkyv archive");
    let key = IntentKey::for_workload(&workload_id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put service intent");

    let kind_key = IntentKey::for_workload_kind(&workload_id);
    state
        .store
        .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
        .await
        .expect("put workload kind");

    let target = TargetResource::new("job/web-frontend").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    let now = clock.now();
    let deadline = now + Duration::from_millis(100);
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };

    for eval in pending {
        run_convergence_tick(&state, &eval.reconciler, &eval.target, now, 0, deadline)
            .await
            .expect("convergence tick must not panic for Service workload");
    }
}
