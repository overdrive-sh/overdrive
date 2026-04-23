//! axum route handlers for the Phase 1 control-plane API.
//!
//! One function per ADR-0008 endpoint:
//!
//! | Endpoint | Handler |
//! |---|---|
//! | `POST /v1/jobs` | `submit_job` |
//! | `GET /v1/jobs/{id}` | `describe_job` |
//! | `GET /v1/cluster/info` | `cluster_status` |
//! | `GET /v1/allocs` | `alloc_status` |
//! | `GET /v1/nodes` | `node_list` |
//!
//! Step 03-01 lands the `submit_job` body; the other four remain RED
//! scaffolds owned by subsequent deliver steps.

use axum::Json;
use axum::extract::State;
use overdrive_core::aggregate::{IntentKey, Job};
use overdrive_core::traits::intent_store::IntentStore;

use crate::AppState;
use crate::api;
use crate::error::ControlPlaneError;

/// `POST /v1/jobs` — validate, archive via rkyv, commit through the
/// intent store, return `(job_id, commit_index)`.
///
/// Idempotency contract (ADR-0015 §4):
///
/// * A spec whose rkyv-archived bytes are byte-identical to what is
///   already stored at the canonical `jobs/<JobId>` key returns HTTP
///   200 with the *current* `commit_index` — no additional write.
/// * A spec whose rkyv-archived bytes DIFFER from what is stored at
///   the same key returns HTTP 409 Conflict with an `ErrorBody`.
/// * An empty key returns a new commit with `commit_index` advanced.
#[utoipa::path(
    post,
    path = "/v1/jobs",
    request_body = api::SubmitJobRequest,
    responses(
        (status = 200, description = "Job accepted", body = api::SubmitJobResponse),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 409, description = "Conflict at existing key", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn submit_job(
    State(state): State<AppState>,
    Json(request): Json<api::SubmitJobRequest>,
) -> Result<Json<api::SubmitJobResponse>, ControlPlaneError> {
    // 1. Validate via the single aggregate constructor. Failures map
    //    to HTTP 400 via `ControlPlaneError::Aggregate` + `IntoResponse`.
    let job = Job::from_spec(request.spec)?;

    // 2. Archive canonically via rkyv. Two archivals of the same
    //    logical Job produce byte-identical bytes — this is what makes
    //    the idempotency check byte-equality instead of semantic-equality.
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job)
        .map_err(|e| ControlPlaneError::Internal(format!("rkyv archive of Job: {e}")))?;

    // 3. Derive the canonical intent key (`jobs/<JobId>`).
    let key = IntentKey::for_job(&job.id);

    // 4. Idempotency / conflict detection — read-then-write against the
    //    LocalStore (ADR-0015 §4 Phase 1 note).
    if let Some(existing) = state.store.get(key.as_bytes()).await? {
        if existing.as_ref() == archived.as_ref() {
            // Byte-identical re-submission: return the current
            // commit_index without writing again.
            return Ok(Json(api::SubmitJobResponse {
                job_id: job.id.to_string(),
                commit_index: state.store.commit_index(),
            }));
        }
        return Err(ControlPlaneError::Conflict {
            message: format!("a different spec is already registered at {}", key.as_str()),
        });
    }

    // 5. Commit. The counter advances post-commit inside `LocalStore::put`.
    state.store.put(key.as_bytes(), archived.as_ref()).await?;

    Ok(Json(api::SubmitJobResponse {
        job_id: job.id.to_string(),
        commit_index: state.store.commit_index(),
    }))
}

/// `GET /v1/jobs/{id}` — read via `IntentStore::get`, rkyv-access the
/// bytes, recompute `spec_digest = ContentHash::of(archived_bytes)`.
///
/// SCAFFOLD: true — owned by step 03-02.
#[utoipa::path(
    get,
    path = "/v1/jobs/{id}",
    params(
        ("id" = String, Path, description = "Canonical JobId"),
    ),
    responses(
        (status = 200, description = "Job description", body = api::JobDescription),
        (status = 404, description = "Job not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn describe_job(_id: String) -> Result<api::JobDescription, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/cluster/info` — mode, region, commit_index, reconciler
/// registry, broker counters.
///
/// SCAFFOLD: true — owned by step 03-05.
#[utoipa::path(
    get,
    path = "/v1/cluster/info",
    responses(
        (status = 200, description = "Cluster status", body = api::ClusterStatus),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "cluster",
)]
pub async fn cluster_status() -> Result<api::ClusterStatus, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/allocs` — observation read on `alloc_status`. Phase 1: zero
/// rows per US-03 AC.
///
/// SCAFFOLD: true — owned by step 03-03.
#[utoipa::path(
    get,
    path = "/v1/allocs",
    responses(
        (status = 200, description = "Allocation status rows", body = api::AllocStatusResponse),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn alloc_status() -> Result<api::AllocStatusResponse, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/nodes` — observation read on `node_health`. Phase 1: zero
/// rows per US-03 AC.
///
/// SCAFFOLD: true — owned by step 03-03.
#[utoipa::path(
    get,
    path = "/v1/nodes",
    responses(
        (status = 200, description = "Node rows", body = api::NodeList),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn node_list() -> Result<api::NodeList, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}
