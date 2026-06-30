//! backend-instance-replacement slice-01 step 01-03 / S-BIR-HANDLER-TXN
//! (US-BIR-1 AC3/4) — `restart_workload` on a declared workload commits
//! exactly ONE atomic bump+clear txn, retains the intent row, and
//! enqueues one job-lifecycle evaluation.
//!
//! Given a declared `workloads/payments` aggregate (and an optional
//! `/stop` sentinel), `restart_workload`:
//!   * commits exactly ONE `IntentStore::txn` carrying
//!     `[IncrementU64{for_workload_generation(payments)},
//!       Delete{for_workload_stop(payments)}]` — observed at the store
//!     boundary as the generation key bumping from absent (0) to exactly
//!     `1` (the BE-`u64` `1`) AND the `/stop` sentinel being cleared,
//!     atomically;
//!   * RETAINS `for_workload(payments)` (`Some` after — the intent stays
//!     declared, distinct from job-removal #211);
//!   * enqueues exactly ONE job-lifecycle evaluation (broker pending == 1);
//!   * returns 200 with `{ workload_id: "payments", outcome }`.
//!
//! # Port-to-port
//!
//! The driving port is the `restart_workload` axum handler, invoked
//! directly with a real `LocalIntentStore`-backed `AppState`. The
//! driven-port assertions are taken at the `IntentStore` back-door read
//! boundary (generation bumped to BE-`1`; stop cleared; aggregate
//! retained) and at the runtime broker's counter snapshot (one queued).
//! Asserting the resulting store state-delta is the falsifiable proxy for
//! "exactly one `[IncrementU64, Delete]` txn committed": absent the
//! production txn, the generation key would stay absent and the
//! assertions red.
//!
//! Default-lane (in-process), real redb over `TempDir` (Strategy C).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use bytes::Bytes;
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::RestartWorkloadResponse;
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

use axum::extract::{Path, State};

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

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

/// Seed a declared `workloads/payments` aggregate by writing the same
/// rkyv archive bytes `submit_workload` would persist. Returns the
/// archived bytes so the caller can assert byte-identical retention.
async fn seed_declared_payments(state: &AppState) -> Vec<u8> {
    let workload_id = WorkloadId::new("payments").expect("parse id");
    let job = Job::from_submit(payments_spec()).expect("Job::from_submit");
    let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive");
    let job_key = IntentKey::for_workload(&workload_id);
    state.store.put(job_key.as_bytes(), archived.as_ref()).await.expect("seed aggregate");
    archived.as_ref().to_vec()
}

/// Decode a port-observed value as a big-endian `u64` (absent / non-8
/// reads as `0`) per development.md § "Safe byte-slice access".
fn decode_be_u64(value: Option<Bytes>) -> u64 {
    value.and_then(|bytes| <[u8; 8]>::try_from(bytes.as_ref()).ok()).map_or(0, u64::from_be_bytes)
}

#[tokio::test]
async fn restart_commits_one_bump_clear_txn_retains_intent_and_enqueues_one_eval() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    let workload_id = WorkloadId::new("payments").expect("parse id");
    let gen_key = IntentKey::for_workload_generation(&workload_id);
    let stop_key = IntentKey::for_workload_stop(&workload_id);
    let job_key = IntentKey::for_workload(&workload_id);

    // Given: a declared aggregate and a present `/stop` sentinel (the
    // stopped-origin restart shape; the bump+clear must remove it).
    let expected_aggregate_bytes = seed_declared_payments(&state).await;
    state.store.put(stop_key.as_bytes(), b"").await.expect("seed stop sentinel");
    let pending_before = state.runtime.broker().counters().queued;

    // When: the operator restarts the declared workload.
    let result = restart_workload(State(state.clone()), Path("payments".to_owned())).await;

    // Then: 200 with `{ workload_id: "payments", outcome }`.
    let response: RestartWorkloadResponse = match result {
        Ok(axum::Json(body)) => body,
        Err(other) => panic!("restart on a declared workload must succeed; got {other:?}"),
    };
    assert_eq!(
        response.workload_id, "payments",
        "response workload_id must echo the canonical id; got {:?}",
        response.workload_id,
    );

    // And: the generation bumped from absent (0) to EXACTLY 1 — the
    // `IncrementU64` arm committed once.
    let generation = decode_be_u64(state.store.get(gen_key.as_bytes()).await.expect("get gen"));
    assert_eq!(
        generation, 1,
        "the restart must bump `workloads/payments/generation` from absent (0) to exactly 1 \
         via one IncrementU64 op; got {generation}",
    );

    // And: the `/stop` sentinel was cleared — atomically with the bump.
    assert!(
        state.store.get(stop_key.as_bytes()).await.expect("get stop").is_none(),
        "the same atomic txn must Delete the `/stop` sentinel",
    );

    // And: the aggregate intent is RETAINED byte-for-byte (distinct from
    // #211 job removal — restart keeps the workload declared).
    let retained = state
        .store
        .get(job_key.as_bytes())
        .await
        .expect("get aggregate")
        .expect("aggregate intent must be retained after restart");
    assert_eq!(
        retained.as_ref(),
        expected_aggregate_bytes.as_slice(),
        "restart must NOT delete or rewrite `workloads/payments` — the intent stays declared",
    );

    // And: exactly ONE job-lifecycle evaluation was enqueued.
    let pending_after = state.runtime.broker().counters().queued;
    assert_eq!(
        pending_after,
        pending_before + 1,
        "restart must enqueue exactly one job-lifecycle evaluation; broker pending went from \
         {pending_before} to {pending_after}",
    );
}
