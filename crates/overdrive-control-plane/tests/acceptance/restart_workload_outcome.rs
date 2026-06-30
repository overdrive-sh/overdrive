//! backend-instance-replacement slice-01 step 01-03 / DDD-11 —
//! `restart_workload`'s cosmetic outcome label is classified from the
//! single `/stop` check-exists read, BEFORE the bump txn.
//!
//!   * S-BIR-HANDLER-OUTCOME-RESUMED — a declared `payments` whose
//!     `/stop` sentinel IS present at the check-exists read ⇒
//!     `RestartWorkloadResponse.outcome == Resumed`.
//!   * S-BIR-HANDLER-OUTCOME-RESTARTED — a declared `coinflip` whose
//!     `/stop` sentinel is ABSENT ⇒ `outcome == Restarted`.
//!
//! The label is cosmetic — placement is the reconciler's generation
//! gate. The handler classifies it from the `/stop` presence observed at
//! the single point-in-time existence read; this test pins that
//! classification mapping (the present⇒Resumed / absent⇒Restarted decision
//! logic) so a mutation flipping the arms is killed.
//!
//! # Port-to-port
//!
//! The driving port is the `restart_workload` axum handler, invoked
//! directly with a real `LocalIntentStore`-backed `AppState`. The
//! observable outcome is the `outcome` field of the 200 response body —
//! a value returned from the driving port. No internal classifier helper
//! is touched.
//!
//! Default-lane (in-process), real redb over `TempDir` (Strategy C).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{RestartOutcome, RestartWorkloadResponse};
use overdrive_control_plane::handlers::restart_workload;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, WorkloadIntent,
};
use overdrive_core::id::{NodeId, WorkloadId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn spec_for(id: &str) -> JobSpecInput {
    JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

/// Seed a declared `workloads/<id>` aggregate by writing the same rkyv
/// archive bytes `submit_workload` would persist.
async fn seed_declared(state: &AppState, id: &str) {
    let workload_id = WorkloadId::new(id).expect("parse id");
    let job = Job::from_submit(spec_for(id)).expect("Job::from_submit");
    let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive");
    let job_key = IntentKey::for_workload(&workload_id);
    state.store.put(job_key.as_bytes(), archived.as_ref()).await.expect("seed aggregate");
}

async fn restart_outcome(state: &AppState, id: &str) -> RestartOutcome {
    let result = restart_workload(State(state.clone()), Path(id.to_owned())).await;
    let response: RestartWorkloadResponse = match result {
        Ok(axum::Json(body)) => body,
        Err(other) => panic!("restart on a declared workload must succeed; got {other:?}"),
    };
    response.outcome
}

#[tokio::test]
async fn present_stop_sentinel_classifies_outcome_as_resumed() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // Given: a declared `payments` whose `/stop` sentinel IS present at
    // the check-exists read (the operator-stopped origin).
    seed_declared(&state, "payments").await;
    let workload_id = WorkloadId::new("payments").expect("parse id");
    let stop_key = IntentKey::for_workload_stop(&workload_id);
    state.store.put(stop_key.as_bytes(), b"").await.expect("seed stop sentinel");

    // When/Then: the cosmetic outcome label is `Resumed`.
    let outcome = restart_outcome(&state, "payments").await;
    assert_eq!(
        outcome,
        RestartOutcome::Resumed,
        "a present `/stop` sentinel at the check-exists read classifies the outcome as Resumed; \
         got {outcome:?}",
    );
}

#[tokio::test]
async fn absent_stop_sentinel_classifies_outcome_as_restarted() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // Given: a declared `coinflip` with NO `/stop` sentinel (a
    // running/declared origin, never operator-stopped).
    seed_declared(&state, "coinflip").await;

    // When/Then: the cosmetic outcome label is `Restarted`.
    let outcome = restart_outcome(&state, "coinflip").await;
    assert_eq!(
        outcome,
        RestartOutcome::Restarted,
        "an absent `/stop` sentinel at the check-exists read classifies the outcome as \
         Restarted; got {outcome:?}",
    );
}
