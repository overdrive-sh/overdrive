//! Acceptance tests for `handlers::submit_workload` — the byte-equality
//! idempotency check at `existing.as_ref() == archived.as_ref()`.
//!
//! A mutation flipping `==` to `!=` in the idempotency branch swaps
//! the two outcomes: byte-identical re-submits would 409, and
//! semantically-different specs at the same `WorkloadId` would return 200.
//! The integration suite catches this over real HTTP (the
//! `idempotent_resubmit.rs` case asserts `outcome == Unchanged` and
//! `spec_digest` equality), but the default mutation run does not
//! compile the integration lane.
//!
//! These acceptance tests call `submit_workload` directly against a live
//! `AppState` (`LocalIntentStore` over `TempDir` + Sim observation)
//! and assert on the typed `Result<Json<SubmitWorkloadResponse>,
//! ControlPlaneError>` return — no network, no TLS, no reqwest. The
//! byte-equality contract is pinned in the default lane, and the
//! `ControlPlaneError::Conflict` variant is asserted directly (no HTTP
//! status round-trip).
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the per-write
//! witness is `outcome` + `spec_digest`, not `commit_index`. See
//! `redesign-drop-commit-index/design/upstream-changes.md` §7.

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{
    IdempotencyOutcome, SubmitWorkloadRequest, SubmitWorkloadResponse,
};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::submit_workload;

/// Helper: invoke the content-negotiated `submit_workload` handler with no
/// `Accept` header (back-compat JSON lane) and parse the response body
/// into the typed `SubmitWorkloadResponse`. Slice 02 step 02-03 made
/// `submit_workload` content-negotiate; the existing acceptance tests
/// continue to assert on the JSON shape via this shim.
async fn submit_json(
    state: AppState,
    request: SubmitWorkloadRequest,
) -> Result<SubmitWorkloadResponse, ControlPlaneError> {
    let response: Response = submit_workload(State(state), HeaderMap::new(), Json(request)).await?;
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body to bytes");
    Ok(serde_json::from_slice(&bytes).expect("JSON SubmitWorkloadResponse"))
}
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::id::NodeId;
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
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
        allocator,
    )
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

fn payments_spec_alt_replicas() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 7,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

// ---------------------------------------------------------------------------
// Byte-identical re-submit returns Ok with `outcome = Unchanged` and
// the ORIGINAL spec_digest — pins the idempotency branch of the `==`
// check.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn byte_identical_resubmit_returns_ok_with_unchanged_outcome_and_same_digest() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let spec = payments_spec();

    // First submit — Ok, outcome = Inserted.
    let first: SubmitWorkloadResponse = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) },
    )
    .await
    .expect("first submit must be Ok");
    assert_eq!(first.workload_id, "payments");
    assert_eq!(
        first.outcome,
        IdempotencyOutcome::Inserted,
        "first successful put must report `outcome = Inserted`",
    );
    assert_eq!(
        first.spec_digest.len(),
        64,
        "first submit must return a 64-char SHA-256 spec_digest; got {} chars",
        first.spec_digest.len(),
    );

    // Second submit — byte-identical spec. Under original `==` this
    // takes the idempotency branch and returns Ok with the SAME
    // spec_digest and `outcome = Unchanged`. Under mutation `!=` this
    // takes the conflict branch and returns ControlPlaneError::Conflict.
    let second = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) },
    )
    .await;

    match second {
        Ok(body) => {
            assert_eq!(
                body.outcome,
                IdempotencyOutcome::Unchanged,
                "byte-identical re-submit MUST report `outcome = Unchanged` \
                 (idempotency branch); got {:?}",
                body.outcome,
            );
            assert_eq!(
                body.spec_digest, first.spec_digest,
                "byte-identical re-submit MUST return the ORIGINAL spec_digest \
                 (idempotency branch); a mutation of `==` to `!=` would either \
                 return a Conflict or compute a different digest",
            );
            assert_eq!(body.workload_id, first.workload_id);
        }
        Err(ControlPlaneError::Conflict { message }) => panic!(
            "byte-identical re-submit MUST NOT return Conflict — mutation of the \
             byte-equality check has flipped the branch. message = {message}",
        ),
        Err(other) => panic!("unexpected error on byte-identical re-submit: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Different spec at occupied key returns ControlPlaneError::Conflict —
// pins the conflict branch of the `==` check.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn different_spec_at_occupied_key_returns_conflict_variant() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // Prime with canonical spec.
    let primed: SubmitWorkloadResponse = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec()) },
    )
    .await
    .expect("prime submit");
    assert_eq!(primed.outcome, IdempotencyOutcome::Inserted);

    // Submit a DIFFERENT spec at the same `WorkloadId`. Under original `==`
    // this takes the conflict branch and returns Conflict. Under
    // mutation `!=` this takes the idempotency branch and returns
    // Ok.
    let outcome = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec_alt_replicas()) },
    )
    .await;

    match outcome {
        Err(ControlPlaneError::Conflict { message }) => {
            // Per `idempotent_resubmit.rs` AC (e), the conflict
            // message must name the canonical intent key path
            // (`jobs/payments`). Mutation that returned empty string
            // would be caught here.
            assert!(
                message.contains("workloads/payments"),
                "Conflict message must name the intent-key path `jobs/payments`; \
                 got: {message}",
            );
        }
        Err(other) => {
            panic!("different-spec submit at occupied key MUST return Conflict; got {other:?}")
        }
        Ok(_) => panic!(
            "different-spec submit at occupied key MUST NOT return Ok — mutation of \
             the byte-equality check has flipped the branch",
        ),
    }

    // The stored spec must remain the original — the conflict branch
    // does not call `put`. A back-door read of the intent key returns
    // the canonical (replicas=3) bytes; a mutation that called `put`
    // either way would surface here as drifted bytes.
    let key = b"workloads/payments";
    let stored = state.store.get(key).await.expect("get must succeed");
    let bytes = stored.expect("intent key must remain populated after a Conflict");
    let canonical_job =
        overdrive_core::aggregate::Job::from_submit(payments_spec()).expect("Job::from_submit");
    let canonical_bytes = overdrive_core::aggregate::WorkloadIntent::Job(canonical_job)
        .archive_for_store()
        .expect("rkyv archive of canonical job");
    assert_eq!(
        bytes.as_ref(),
        canonical_bytes.as_ref(),
        "stored bytes must remain the ORIGINAL canonical archive after a \
         Conflict — a mutation that called put on the conflict branch \
         would leave the rejected (replicas=7) bytes here",
    );
}

// ---------------------------------------------------------------------------
// Fresh (empty) key: successful submit returns `outcome = Inserted` and
// the canonical spec_digest. Kills a mutation that hardcodes `existing`
// to Some with wrong data.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fresh_submit_on_empty_key_returns_inserted_and_persists_spec() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    let resp: SubmitWorkloadResponse = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec()) },
    )
    .await
    .expect("submit");
    assert_eq!(resp.workload_id, "payments");
    assert_eq!(
        resp.outcome,
        IdempotencyOutcome::Inserted,
        "first put on a fresh key must report `outcome = Inserted`",
    );
    assert_eq!(
        resp.spec_digest.len(),
        64,
        "spec_digest must be 64 hex chars (SHA-256); got {} chars",
        resp.spec_digest.len(),
    );

    // The key must be populated (get returns Some) — proves the put
    // actually fired. Per ADR-0020 `IntentStore::get` returns
    // `Option<Bytes>`.
    let key = b"workloads/payments";
    let stored = state.store.get(key).await.expect("get must succeed");
    let bytes = stored.expect(
        "after successful submit the intent key must be populated — \
         a mutation that bypassed the put would leave it empty",
    );
    assert!(!bytes.is_empty(), "stored bytes must be non-empty");
}

// ---------------------------------------------------------------------------
// Regression: empty kind-key bytes must not panic on the Unchanged path.
// Before the fix, `stored_kind_bytes[0]` panicked on `Bytes::new()`.
// After the fix, `.first().copied()` gracefully skips the assignment.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unchanged_path_with_empty_kind_bytes_does_not_panic() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let spec = payments_spec();

    // First submit — plants the spec at the canonical key.
    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) },
    )
    .await
    .expect("first submit");
    assert_eq!(first.outcome, IdempotencyOutcome::Inserted);

    // Overwrite the kind key with an empty value — simulates a
    // corrupted or truncated store entry.
    let workload_id = overdrive_core::WorkloadId::from_str("payments").expect("WorkloadId");
    let kind_key = overdrive_core::aggregate::IntentKey::for_workload_kind(&workload_id);
    state.store.put(kind_key.as_bytes(), b"").await.expect("overwrite kind key with empty bytes");

    // Re-submit the identical spec. Before the fix this panicked with
    // "index out of bounds: the len is 0 but the index is 0".
    let second = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) },
    )
    .await
    .expect("re-submit with empty kind bytes must not panic");

    assert_eq!(
        second.outcome,
        IdempotencyOutcome::Unchanged,
        "byte-identical re-submit must still return Unchanged even when \
         the kind key holds empty bytes",
    );
}
