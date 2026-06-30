//! backend-instance-replacement slice-01 step 01-03 / S-BIR-HANDLER-404
//! (US-BIR-1 AC5) — `restart_workload` on an unknown id is an honest 404
//! with ZERO mutation.
//!
//! Given no declared `workloads/<id>` aggregate, `restart_workload`
//! returns `Err(ControlPlaneError::NotFound { resource: "workloads/<id>" })`
//! AND commits NO `IntentStore` txn (no generation bump, no sentinel
//! delete — observed at the store boundary as the generation key staying
//! absent) AND enqueues NO job-lifecycle evaluation (the broker's pending
//! queue stays empty). Same 404 posture as `stop_workload`.
//!
//! # Port-to-port
//!
//! The driving port is the `restart_workload` axum handler, invoked
//! directly with a real `LocalIntentStore`-backed `AppState` (the
//! `submit_job_handler_rejects_empty_exec_command_with_400.rs` pattern —
//! no reqwest, no TLS, no port binding). The driven-port assertions are
//! taken at the `IntentStore` back-door read boundary (the generation key
//! is absent ⇒ no bump landed) and at the runtime broker's counter
//! snapshot (zero queued ⇒ no eval enqueued). No internal helper is
//! touched.
//!
//! Default-lane (in-process). The store is real redb over `TempDir` so a
//! generation bump WOULD be observable if the handler erroneously
//! committed one — the assertion is genuinely falsifiable.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::restart_workload;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::IntentKey;
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

#[tokio::test]
async fn restart_on_unknown_id_is_404_with_no_mutation_and_no_enqueue() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // Given: no `workloads/nonexistent` aggregate was ever declared.
    let workload_id = WorkloadId::new("nonexistent").expect("parse workload id");
    let gen_key = IntentKey::for_workload_generation(&workload_id);
    let stop_key = IntentKey::for_workload_stop(&workload_id);

    // Precondition sanity — both intent keys are absent before the call.
    assert!(
        state.store.get(gen_key.as_bytes()).await.expect("get gen").is_none(),
        "generation key must be absent before the call",
    );
    let pending_before = state.runtime.broker().counters().queued;

    // When: the operator restarts the never-deployed workload.
    let result = restart_workload(State(state.clone()), Path("nonexistent".to_owned())).await;

    // Then: an honest 404 naming `workloads/nonexistent`.
    match result {
        Err(ControlPlaneError::NotFound { resource }) => {
            assert_eq!(
                resource, "workloads/nonexistent",
                "404 resource must be the canonical `workloads/<id>` key (byte-identical to \
                 stop_workload's 404 shape); got {resource:?}",
            );
        }
        Err(other) => panic!(
            "expected ControlPlaneError::NotFound {{ resource: \"workloads/nonexistent\" }}; \
             got {other:?}",
        ),
        Ok(body) => panic!(
            "restart on an unknown id must 404, never bump-and-respond; handler returned \
             Ok({body:?})",
        ),
    }

    // And: NO IntentStore txn committed — the generation key stays absent
    // (a bump would have written the 8-byte BE `1`) and so does the stop
    // sentinel (no Delete on an absent aggregate).
    assert!(
        state.store.get(gen_key.as_bytes()).await.expect("get gen post").is_none(),
        "no generation bump may occur on an absent aggregate — the txn must not commit",
    );
    assert!(
        state.store.get(stop_key.as_bytes()).await.expect("get stop post").is_none(),
        "no stop-sentinel write/delete may occur on an absent aggregate",
    );

    // And: NO job-lifecycle evaluation was enqueued.
    let pending_after = state.runtime.broker().counters().queued;
    assert_eq!(
        pending_after, pending_before,
        "no evaluation may be enqueued on the 404 path; broker pending went from \
         {pending_before} to {pending_after}",
    );
}
