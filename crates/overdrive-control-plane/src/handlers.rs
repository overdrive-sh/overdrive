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
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};

use crate::api::{StopJobResponse, StopOutcome};

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
            reason: None,
        }
    }
}

impl From<overdrive_core::traits::observation_store::NodeHealthRow> for api::NodeRowBody {
    fn from(row: overdrive_core::traits::observation_store::NodeHealthRow) -> Self {
        Self { node_id: row.node_id.to_string(), region: row.region.to_string() }
    }
}

/// `POST /v1/jobs` — validate, archive via rkyv, commit through the
/// intent store, return `{job_id, spec_digest, outcome}`.
///
/// Idempotency contract (ADR-0015 §4 amended by ADR-0020):
///
/// * A spec whose rkyv-archived bytes are absent at the canonical
///   `jobs/<JobId>` key returns HTTP 200 with `outcome =
///   IdempotencyOutcome::Inserted` and the canonical `spec_digest`
///   (`PutOutcome::Inserted` path).
/// * A spec whose rkyv-archived bytes are byte-identical to what is
///   already stored at the same key returns HTTP 200 with `outcome =
///   IdempotencyOutcome::Unchanged` and the same `spec_digest`. No
///   write occurred (`PutOutcome::KeyExists { existing }` path with
///   byte equality).
/// * A spec whose rkyv-archived bytes DIFFER from what is stored at
///   the same key returns HTTP 409 Conflict with an `ErrorBody`.
///   Conflict is an HTTP-status concern; the `outcome` field is
///   absent on the 409 path (ADR-0015 §4 amendment).
///
/// `spec_digest` is the lowercase-hex SHA-256 of the canonical
/// rkyv-archived bytes (ADR-0002, development.md §Hashing). On the
/// `Unchanged` path the digest is computed once over the candidate
/// bytes and used both for the byte-equality check (against the
/// `existing` bytes returned by `PutOutcome::KeyExists`) and for the
/// response body.
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
        .map_err(|e| ControlPlaneError::internal("rkyv archive of Job", e))?;

    // 3. Derive the canonical intent key (`jobs/<JobId>`).
    let key = IntentKey::for_job(&job.id);

    // 4. Compute the canonical `spec_digest` over the rkyv-archived
    //    bytes. Used both as the response field and (on the
    //    `KeyExists` branch) as the equality check against the
    //    bytes already stored at this key — see step 5.
    let spec_digest = ContentHash::of(archived.as_ref()).to_string();

    // 5. Atomic idempotency / conflict detection via `put_if_absent`.
    //    The existence check and the insert happen in a single store
    //    transaction — this closes the TOCTOU window that would open
    //    under a naive `get` (read txn) + `put` (write txn) pair,
    //    where two concurrent submitters for the same key could both
    //    see `None` on the read and both fall through to the write,
    //    silently clobbering the first spec.
    match state.store.put_if_absent(key.as_bytes(), archived.as_ref()).await? {
        PutOutcome::Inserted => Ok(Json(api::SubmitJobResponse {
            job_id: job.id.to_string(),
            spec_digest,
            outcome: api::IdempotencyOutcome::Inserted,
        })),
        PutOutcome::KeyExists { existing } => {
            if existing.as_ref() == archived.as_ref() {
                // Byte-identical re-submission: 200 OK with
                // `outcome = Unchanged`; the `spec_digest` is stable
                // across N retries by construction (rkyv archival is
                // canonical).
                Ok(Json(api::SubmitJobResponse {
                    job_id: job.id.to_string(),
                    spec_digest,
                    outcome: api::IdempotencyOutcome::Unchanged,
                }))
            } else {
                // Different spec at the same key — 409 Conflict.
                // Conflict is HTTP-status, never a wire `outcome`
                // value (ADR-0015 §4 amended by ADR-0020).
                Err(ControlPlaneError::Conflict {
                    message: format!("a different spec is already registered at {}", key.as_str()),
                })
            }
        }
    }
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
///
/// Post-ADR-0020: the response shape is `{spec, spec_digest}` only.
/// The `commit_index` field is dropped wholesale — there is no
/// store-wide nor per-entry index on the wire (see ADR-0020).
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
    //    identifier (non-ASCII, wrong length, bad charset) surfaces as
    //    HTTP 400 with `field: Some("id")` — the path-parameter name
    //    the OpenAPI spec declares.
    //
    //    Routing through `AggregateError::Id` here would lose the field
    //    name: that variant is a `#[from]` pass-through of `IdParseError`
    //    and the `to_response` mapping for `Aggregate(Id(_))` correctly
    //    leaves `field = None` (it has no caller-side context to name).
    //    The handler DOES have caller-side context — the path parameter
    //    is named `id` — so we attach it explicitly. Without this, a
    //    client branching on the `field` discriminator cannot tell
    //    path-parameter validation from request-body validation.
    let job_id = JobId::new(&job_id_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("id".to_owned()),
    })?;

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
        .map_err(|e| ControlPlaneError::internal("rkyv access of ArchivedJob", e))?;
    let job: Job = rkyv::deserialize::<Job, rkyv::rancor::Error>(archived)
        .map_err(|e| ControlPlaneError::internal("rkyv deserialize of Job", e))?;

    // 4. Canonical spec_digest — SHA-256 of the exact archived bytes we
    //    just read. This is what ADR-0002 calls "hash of canonical rkyv
    //    bytes"; no re-archival, no re-canonicalisation.
    let spec_digest = ContentHash::of(&bytes).to_string();

    Ok(Json(api::JobDescription { spec: JobSpecInput::from(&job), spec_digest }))
}

/// `POST /v1/jobs/{id}/stop` — record a stop intent for a previously-
/// submitted job. Per ADR-0027.
///
/// AIP-136 prescribes `POST /v1/jobs/{id}:stop`. axum 0.7 cannot
/// route the `:stop` verb suffix as a single path segment because its
/// matcher (matchit) treats `:` as the path-parameter prefix —
/// `/v1/jobs/:id:stop` is rejected. We use the path-subsegment form
/// `/v1/jobs/{id}/stop` which is industry-standard, semantically
/// equivalent, and free of framework conflict. ADR-0027's guidance
/// stands; only the URL form differs.
///
/// Idempotency: the handler writes `IntentKey::for_job_stop(<id>)`
/// via `IntentStore::put_if_absent` (atomic compare-and-set). A
/// second call sees `KeyExists` and returns
/// `outcome = AlreadyStopped` — no second write occurs.
///
/// 404 contract: a stop call against an `<id>` that was never
/// submitted (no `IntentKey::for_job(<id>)` row) returns HTTP 404.
/// The original spec key MUST exist before a stop intent can be
/// recorded — stopping a non-existent job is operator error, not an
/// idempotent no-op.
///
/// Empty request body. The response body is `{ job_id, outcome }`.
#[utoipa::path(
    post,
    path = "/v1/jobs/{id}/stop",
    params(
        ("id" = String, Path, description = "Canonical JobId"),
    ),
    responses(
        (status = 200, description = "Job stop recorded", body = StopJobResponse),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 404, description = "Job not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn stop_job(
    State(state): State<AppState>,
    Path(job_id_str): Path<String>,
) -> Result<axum::Json<StopJobResponse>, ControlPlaneError> {
    // 1. Parse the path parameter through the JobId newtype. Same
    //    field-naming discipline as `describe_job` — the validation
    //    error names the path parameter ("id") so clients can branch
    //    on field origin.
    let job_id = JobId::new(&job_id_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("id".to_owned()),
    })?;

    // 2. The job must exist before a stop can be recorded. Reading
    //    the canonical job key is the cheapest 404 check — if the
    //    row is absent we have no stop target, so 404 surfaces with
    //    the same `resource = jobs/<id>` shape as describe_job's
    //    NotFound path.
    let job_key = IntentKey::for_job(&job_id);
    let job_exists = state.store.get(job_key.as_bytes()).await?.is_some();
    if !job_exists {
        return Err(ControlPlaneError::NotFound { resource: job_key.as_str().to_owned() });
    }

    // 3. Atomic put_if_absent on the stop key. The empty value is
    //    deliberate — the key's existence IS the signal. A second
    //    stop call lands on the KeyExists branch and reports
    //    AlreadyStopped without a second write.
    let stop_key = IntentKey::for_job_stop(&job_id);
    let outcome = match state.store.put_if_absent(stop_key.as_bytes(), b"").await? {
        PutOutcome::Inserted => StopOutcome::Stopped,
        PutOutcome::KeyExists { .. } => StopOutcome::AlreadyStopped,
    };

    Ok(axum::Json(StopJobResponse { job_id: job_id.to_string(), outcome }))
}

/// `GET /v1/cluster/info` — mode, region, reconciler registry, broker
/// counters.
///
/// Post-ADR-0020 the response is the four-field shape
/// `{mode, region, reconcilers, broker}`. The `commit_index` field is
/// dropped wholesale (no rename to `writes_since_boot` — that
/// preserves the same race conditions and reset-on-restart gap under
/// a different name; see ADR-0020 §Considered alternatives §D).
/// Activity-rate observability is provided by `broker.dispatched`
/// plus the `reconcilers` list; cluster-level commit-rate signals
/// belong on Phase 5's metrics endpoint.
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
    // `ReconcilerRuntime::registered` already returns names in canonical
    // (Ord-sorted) order — the registry is a `BTreeMap` — so JSON wire
    // stability is by construction here. No `.sort()` needed.
    let reconcilers: Vec<String> =
        state.runtime.registered().into_iter().map(|n| n.to_string()).collect();

    Ok(Json(api::ClusterStatus {
        mode: "single".to_string(),
        region: "local".to_string(),
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
        .map_err(|e| ControlPlaneError::internal("alloc_status_rows", e))?
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
        .map_err(|e| ControlPlaneError::internal("node_health_rows", e))?
        .into_iter()
        .map(api::NodeRowBody::from)
        .collect();
    Ok(Json(api::NodeList { rows }))
}
