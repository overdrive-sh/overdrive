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
use axum::extract::{Path, State};
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput};
use overdrive_core::id::{ContentHash, JobId};
use overdrive_core::traits::intent_store::IntentStore;

use crate::AppState;
use crate::api;
use crate::error::ControlPlaneError;

impl From<overdrive_core::traits::observation_store::AllocStatusRow> for api::AllocStatusRowBody {
    fn from(row: overdrive_core::traits::observation_store::AllocStatusRow) -> Self {
        Self {
            alloc_id: row.alloc_id.to_string(),
            job_id: row.job_id.to_string(),
            node_id: row.node_id.to_string(),
            state: row.state.to_string(),
        }
    }
}

impl From<overdrive_core::traits::observation_store::NodeHealthRow> for api::NodeRowBody {
    fn from(row: overdrive_core::traits::observation_store::NodeHealthRow) -> Self {
        Self { node_id: row.node_id.to_string(), region: row.region.to_string() }
    }
}

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
    //    LocalIntentStore (ADR-0015 §4 Phase 1 note).
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

    // 5. Commit. The counter advances post-commit inside `LocalIntentStore::put`.
    state.store.put(key.as_bytes(), archived.as_ref()).await?;

    Ok(Json(api::SubmitJobResponse {
        job_id: job.id.to_string(),
        commit_index: state.store.commit_index(),
    }))
}

/// `GET /v1/jobs/{id}` — read via `IntentStore::get`, rkyv-access the
/// bytes, recompute `spec_digest = ContentHash::of(archived_bytes)`.
///
/// Canonical-hashing contract (ADR-0002 + development.md §Hashing):
/// the `spec_digest` returned here is SHA-256 over the exact byte
/// sequence we pulled out of the `IntentStore` — i.e. the rkyv-archived
/// bytes of a validated `Job`. We deliberately do NOT re-canonicalise,
/// do NOT route through JCS, and do NOT hash `serde_json::to_string(&job)`
/// — any of those would break the byte-identity property the rest of
/// the platform depends on.
///
/// 404 contract (ADR-0015 §3): `IntentStore::get -> Ok(None)` maps to
/// `ControlPlaneError::NotFound { resource: <IntentKey::as_str()> }`,
/// which `to_response` renders as HTTP 404 with an `ErrorBody { error:
/// "not_found", ... }`.
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
pub async fn describe_job(
    State(state): State<AppState>,
    Path(job_id_str): Path<String>,
) -> Result<Json<api::JobDescription>, ControlPlaneError> {
    // 1. Parse the path parameter through the JobId newtype. A malformed
    //    identifier (non-ASCII, wrong length, bad charset) surfaces via
    //    `AggregateError::Id(..)` → HTTP 400 through `IntoResponse`.
    //    This is the same validation lane the submit path uses.
    let job_id = JobId::new(&job_id_str).map_err(overdrive_core::aggregate::AggregateError::Id)?;

    // 2. Derive the canonical intent key and read from the authoritative
    //    store. Missing key → NotFound → HTTP 404.
    //
    //    The `NotFound` `resource` string uses `IntentKey::as_str()`
    //    (the canonical `<prefix>/<id>` rendering) rather than
    //    hand-formatting the literal here — doing the latter would
    //    duplicate the job-prefix literal into a second production
    //    file, which the `intent_key_canonical` grep-gate in
    //    `overdrive-core` explicitly forbids.
    let key = IntentKey::for_job(&job_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await?
        .ok_or_else(|| ControlPlaneError::NotFound { resource: key.as_str().to_owned() })?;

    // 3. rkyv access + deserialise. Corruption / bit-rot in the redb
    //    file surfaces here; it maps to HTTP 500 via `Internal`.
    let archived = rkyv::access::<rkyv::Archived<Job>, rkyv::rancor::Error>(&bytes)
        .map_err(|e| ControlPlaneError::Internal(format!("rkyv access of ArchivedJob: {e}")))?;
    let job: Job = rkyv::deserialize::<Job, rkyv::rancor::Error>(archived)
        .map_err(|e| ControlPlaneError::Internal(format!("rkyv deserialize of Job: {e}")))?;

    // 4. Canonical spec_digest — SHA-256 of the exact archived bytes we
    //    just read. This is what ADR-0002 calls "hash of canonical rkyv
    //    bytes"; no re-archival, no re-canonicalisation.
    let spec_digest = ContentHash::of(&bytes).to_string();

    Ok(Json(api::JobDescription {
        spec: JobSpecInput::from(&job),
        commit_index: state.store.commit_index(),
        spec_digest,
    }))
}

/// `GET /v1/cluster/info` — mode, region, `commit_index`, reconciler
/// registry, broker counters.
#[utoipa::path(
    get,
    path = "/v1/cluster/info",
    responses(
        (status = 200, description = "Cluster status", body = api::ClusterStatus),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "cluster",
)]
pub async fn cluster_status(
    State(state): State<AppState>,
) -> Result<Json<api::ClusterStatus>, ControlPlaneError> {
    // Phase 1 scope — `mode` is always "single" (HA arrives Phase 2+)
    // and `region` is always "local" (multi-region arrives Phase 5+).
    // These are hard-pinned here rather than derived from config so a
    // stray config typo cannot put a non-canonical value on the wire.
    let counters = state.runtime.broker().counters();
    let mut reconcilers: Vec<String> =
        state.runtime.registered().into_iter().map(|n| n.to_string()).collect();
    // Sort for stable JSON output — `ReconcilerRuntime::registered`
    // returns HashMap keys, whose iteration order is unspecified. Wire
    // stability matters here because this body is serialised into an
    // OpenAPI-covered response; tests and clients expect deterministic
    // rendering.
    reconcilers.sort();

    Ok(Json(api::ClusterStatus {
        mode: "single".to_string(),
        region: "local".to_string(),
        commit_index: state.store.commit_index(),
        reconcilers,
        broker: api::BrokerCountersBody {
            queued: counters.queued,
            cancelled: counters.cancelled,
            dispatched: counters.dispatched,
        },
    }))
}

/// `GET /v1/allocs` — observation read on `alloc_status`.
///
/// Reads through the `ObservationStore::alloc_status_rows` trait method
/// (not the concrete `SimObservationStore` type) so Phase 2's
/// `CorrosionStore` swap is a single trait-object replacement with no
/// handler changes. Fresh store → HTTP 200 with explicit `{"rows": []}`
/// — honest empty state per K7; no fabrication, no hardcoded response.
#[utoipa::path(
    get,
    path = "/v1/allocs",
    responses(
        (status = 200, description = "Allocation status rows", body = api::AllocStatusResponse),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn alloc_status(
    State(state): State<AppState>,
) -> Result<Json<api::AllocStatusResponse>, ControlPlaneError> {
    let rows = state
        .obs
        .alloc_status_rows()
        .await
        .map_err(|e| ControlPlaneError::Internal(format!("alloc_status_rows: {e}")))?
        .into_iter()
        .map(api::AllocStatusRowBody::from)
        .collect();
    Ok(Json(api::AllocStatusResponse { rows }))
}

/// `GET /v1/nodes` — observation read on `node_health`.
///
/// Symmetric to `alloc_status` — reads through
/// `ObservationStore::node_health_rows`. Fresh store → HTTP 200 with
/// explicit `{"rows": []}`.
#[utoipa::path(
    get,
    path = "/v1/nodes",
    responses(
        (status = 200, description = "Node rows", body = api::NodeList),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn node_list(
    State(state): State<AppState>,
) -> Result<Json<api::NodeList>, ControlPlaneError> {
    let rows = state
        .obs
        .node_health_rows()
        .await
        .map_err(|e| ControlPlaneError::Internal(format!("node_health_rows: {e}")))?
        .into_iter()
        .map(api::NodeRowBody::from)
        .collect();
    Ok(Json(api::NodeList { rows }))
}
