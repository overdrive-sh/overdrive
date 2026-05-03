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
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use overdrive_core::aggregate::{AggregateError, IntentKey, Job, JobSpecInput};
use overdrive_core::id::{ContentHash, JobId};
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::traits::observation_store::AllocStatusRow;
use overdrive_core::transition_reason::TerminalCondition;
use serde::Deserialize;

use crate::api::{
    AllocStateWire, RestartBudget, StopJobResponse, StopOutcome, TransitionRecord, TransitionSource,
};

use crate::AppState;
use crate::api;
use crate::error::ControlPlaneError;
use crate::eval_broker::Evaluation;

/// Enqueue a `(job-lifecycle, job/<id>)` evaluation onto the runtime
/// broker. Called from `submit_job` and `stop_job` after the
/// `IntentStore` write commits — the edge-triggered ingress half of
/// whitepaper §18 *Triggering Model — Hybrid by Design*. The
/// convergence-loop spawn in [`crate::run_server_with_obs_and_driver`]
/// drains the broker on the next tick and dispatches one
/// [`crate::reconciler_runtime::run_convergence_tick`] call per pending
/// evaluation.
///
/// Per `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2):
/// without this enqueue, `IntentStore` writes would commit but no
/// convergence would ever run — `cluster_status.broker.dispatched`
/// would permanently read 0.
fn enqueue_job_lifecycle_eval(state: &AppState, job_id: &JobId) -> Result<(), ControlPlaneError> {
    let reconciler = ReconcilerName::new("job-lifecycle")
        .map_err(|e| ControlPlaneError::internal("ReconcilerName::new(\"job-lifecycle\")", e))?;
    let target_string = format!("job/{job_id}");
    let target = TargetResource::new(&target_string)
        .map_err(|e| ControlPlaneError::internal("TargetResource::new(job/<id>)", e))?;
    state.runtime.broker().submit(Evaluation { reconciler, target });
    Ok(())
}

/// Parse a `JobId` from a path parameter, attaching `field = Some("id")` to
/// the validation error so HTTP clients can branch on the error origin.
///
/// The `field` discriminator is the contract that lets a client tell
/// path-parameter validation apart from request-body validation —
/// `AggregateError::Id`'s `#[from]` pass-through correctly leaves
/// `field = None` because it has no caller-side context to name. Handlers
/// that DO have caller-side context (the `OpenAPI` path parameter `id`)
/// attach it explicitly through this helper.
fn parse_job_id_path(job_id_str: &str) -> Result<JobId, ControlPlaneError> {
    JobId::new(job_id_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("id".to_owned()),
    })
}

impl From<overdrive_core::traits::observation_store::AllocStatusRow> for api::AllocStatusRowBody {
    fn from(row: overdrive_core::traits::observation_store::AllocStatusRow) -> Self {
        // Phase 1 the action shim writes Driver(Exec) for rows it
        // produced post-spawn; the reconciler emits its own progress
        // markers via Reconciler. Without the source-attribution
        // metadata on the row itself we default to Reconciler — Phase 2
        // adds explicit attribution per ADR-0033 §1.
        let last_transition = row.reason.clone().map(|reason| TransitionRecord {
            from: None,
            to: AllocStateWire::from(row.state),
            reason,
            source: TransitionSource::Reconciler,
            at: format!("(c={},w={})", row.updated_at.counter, row.updated_at.writer),
        });

        // The `started_at` field is populated for rows that have left
        // the Pending state at least once. Phase 1 detects this by the
        // observed state being non-Pending — Phase 2+ will track the
        // first-non-Pending logical timestamp explicitly.
        let started_at = match row.state {
            AllocState::Pending => None,
            _ => Some(format!("(c={},w={})", row.updated_at.counter, row.updated_at.writer)),
        };

        Self {
            alloc_id: row.alloc_id.to_string(),
            job_id: row.job_id.to_string(),
            node_id: row.node_id.to_string(),
            state: AllocStateWire::from(row.state),
            reason: row.reason,
            // Resources cannot be reconstructed from the row alone in
            // Phase 1 — the row schema does not carry the requested
            // envelope. The handler that knows the JobSpec overrides
            // this field; the bare conversion uses zeroes.
            resources: api::ResourcesBody { cpu_milli: 0, memory_bytes: 0 },
            started_at,
            exit_code: None,
            last_transition,
            error: row.detail,
        }
    }
}

/// Query parameters for `GET /v1/allocs`.
///
/// `job` selects the snapshot for a specific `JobId`. The query
/// parameter is REQUIRED — a missing `?job=` returns HTTP 400 with
/// `field = Some("job")`. The handler reads the `IntentStore` for the
/// named job, returns 404 if absent, then projects matching rows + the
/// `JobLifecycle` view-cache restart counts into the populated
/// envelope shape per ADR-0033 §1.
#[derive(Debug, Clone, Deserialize)]
pub struct AllocStatusQuery {
    /// Canonical `JobId` to filter on. Required. Missing → HTTP 400
    /// with `field = Some("job")`.
    pub job: Option<String>,
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
// Slice 02 step 02-02 — `POST /v1/jobs` 200 response is polymorphic
// on `Accept` per DESIGN [D6] / [D8]. The path declares both content
// types under a single 200 response — `application/json` returns a
// one-shot `SubmitJobResponse`; `application/x-ndjson` returns a
// stream of `SubmitEvent` lines. utoipa 5.x's `responses(..., content(
// (T1 = "mime1"), (T2 = "mime2") ))` group form is the multi-content-
// type shape (see utoipa-gen 5.4.0 src/path/response.rs §"content"
// branch).
#[utoipa::path(
    post,
    path = "/v1/jobs",
    request_body = api::SubmitJobRequest,
    responses(
        (status = 200, description = "Job accepted (Accept negotiates one-shot vs streaming)",
            content(
                (api::SubmitJobResponse = "application/json"),
                (api::SubmitEvent       = "application/x-ndjson"),
            )
        ),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 409, description = "Conflict at existing key", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn submit_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<api::SubmitJobRequest>,
) -> Result<Response, ControlPlaneError> {
    let want_streaming = wants_ndjson(&headers);
    // 1. Validate via the single aggregate constructor (ADR-0011 —
    //    THE-single-validating-constructor). The `Job::from_spec` call
    //    is the server-side defence-in-depth complement to the CLI's
    //    fast-fail (ADR-0014); both lanes route through the same
    //    constructor, so the new ADR-0031 §4 `exec.command` non-empty
    //    rule fires on both by construction.
    //
    //    Field-name preservation per ADR-0015: scalar-field validation
    //    failures (`AggregateError::Validation { field, message }`)
    //    flatten into the top-level `ControlPlaneError::Validation`
    //    variant with `field: Some(field.to_string())`. This keeps the
    //    typed Rust contract aligned with the wire shape — clients
    //    matching on the typed error see the same `field` token the
    //    HTTP layer renders into the RFC 7807 body.
    //
    //    Non-validation `AggregateError` shapes (`Id`, `Resources`)
    //    fall through the `#[from]` blanket conversion to
    //    `ControlPlaneError::Aggregate(_)` — `to_response` still maps
    //    them to HTTP 400 with `error: "validation"`.
    let job = Job::from_spec(request.spec).map_err(|e| match e {
        AggregateError::Validation { field, message } => {
            ControlPlaneError::Validation { field: Some(field.to_owned()), message }
        }
        other => ControlPlaneError::Aggregate(other),
    })?;

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
    let outcome = match state.store.put_if_absent(key.as_bytes(), archived.as_ref()).await? {
        PutOutcome::Inserted => {
            // Edge-triggered ingress per whitepaper §18: enqueue an
            // evaluation for the job-lifecycle reconciler so the
            // convergence-loop spawn picks it up on the next tick.
            // Per `fix-convergence-loop-not-spawned` Step 01-02.
            enqueue_job_lifecycle_eval(&state, &job.id)?;
            api::IdempotencyOutcome::Inserted
        }
        PutOutcome::KeyExists { existing } => {
            if existing.as_ref() == archived.as_ref() {
                // Byte-identical re-submission: 200 OK with
                // `outcome = Unchanged`; the `spec_digest` is stable
                // across N retries by construction (rkyv archival is
                // canonical). Enqueue an evaluation anyway: the
                // resubmit is operator intent to re-converge against
                // any drift, and the broker collapses duplicates per
                // §18 evaluation-broker semantics so a flapping
                // resubmit produces one pending eval, not N.
                enqueue_job_lifecycle_eval(&state, &job.id)?;
                api::IdempotencyOutcome::Unchanged
            } else {
                // Different spec at the same key — 409 Conflict.
                // Conflict is HTTP-status, never a wire `outcome`
                // value (ADR-0015 §4 amended by ADR-0020). No
                // evaluation enqueued — the intent did not change.
                return Err(ControlPlaneError::Conflict {
                    message: format!("a different spec is already registered at {}", key.as_str()),
                });
            }
        }
    };

    // Branch on Accept header per [D6] / [D8]. JSON lane preserves
    // the existing `SubmitJobResponse` shape unchanged (back-compat
    // S-CP-08). NDJSON lane delegates to `streaming_submit_loop` for
    // the per-line emit + cap timer + lagged-recovery.
    if want_streaming {
        let accepted =
            crate::streaming::build_accepted(spec_digest, key.as_str().to_owned(), outcome);
        let stream = crate::streaming::build_stream(state.clone(), job.id, accepted);
        let body = Body::from_stream(stream);
        let response = Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .map_err(|e| ControlPlaneError::internal("build streaming response", e))?;
        Ok(response)
    } else {
        let body = api::SubmitJobResponse { job_id: job.id.to_string(), spec_digest, outcome };
        Ok(Json(body).into_response())
    }
}

/// Returns `true` when the `Accept` header signals the
/// `application/x-ndjson` streaming lane. Missing header, `*/*`, and
/// `application/json` all fall through to the JSON back-compat lane.
///
/// Phase 1 uses simple `contains("application/x-ndjson")` — no
/// q-value parsing — because the CLI sends one explicit value at a
/// time. Phase 2+ may upgrade to RFC 7231 §5.3.2 q-value resolution
/// if multiple Accept values become common.
fn wants_ndjson(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("application/x-ndjson"))
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
    //    HTTP 400 with `field: Some("id")` via `parse_job_id_path`.
    let job_id = parse_job_id_path(&job_id_str)?;

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
    //    field-naming discipline as `describe_job` — see `parse_job_id_path`.
    let job_id = parse_job_id_path(&job_id_str)?;

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

    // Edge-triggered ingress per whitepaper §18: enqueue an
    // evaluation for the job-lifecycle reconciler so the
    // convergence-loop spawn drives the running allocations to
    // Terminated. Both Stopped and AlreadyStopped enqueue: a redundant
    // stop arriving while the prior stop has not yet converged is
    // collapsed by the broker's `(reconciler, target)` keying.
    // Per `fix-convergence-loop-not-spawned` Step 01-02.
    enqueue_job_lifecycle_eval(&state, &job_id)?;

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

/// `GET /v1/allocs?job=<id>` — observation read on `alloc_status`.
///
/// Reads through the `ObservationStore::alloc_status_rows` trait method
/// (not the concrete `SimObservationStore` type) so Phase 2's
/// `CorrosionStore` swap is a single trait-object replacement with no
/// handler changes.
///
/// The `job` query parameter is REQUIRED. The handler reads the
/// `IntentStore` for `<id>`'s `Job`, returns 404 if absent, then
/// projects matching rows + the `JobLifecycle` view-cache restart
/// counts into the populated envelope shape per ADR-0033 §1. A missing
/// `?job=` query parameter returns HTTP 400 with
/// `field = Some("job")`.
#[utoipa::path(
    get,
    path = "/v1/allocs",
    responses(
        (status = 200, description = "Allocation snapshot for the named job", body = api::AllocStatusResponse),
        (status = 400, description = "Validation error (missing or malformed job query)", body = api::ErrorBody),
        (status = 404, description = "Job not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn alloc_status(
    State(state): State<AppState>,
    Query(query): Query<AllocStatusQuery>,
) -> Result<Json<api::AllocStatusResponse>, ControlPlaneError> {
    // The `job` query parameter is required. A missing `?job=` is a
    // client error (HTTP 400), not a wildcard read — slice 01 step
    // 01-03 made `?job=<id>` the canonical shape per S-AS-09.
    let Some(ref job_str) = query.job else {
        return Err(ControlPlaneError::Validation {
            message: "missing required query parameter: job".to_owned(),
            field: Some("job".to_owned()),
        });
    };

    // Validate the JobId (returns 400 with field=Some("job") on bad
    // input — the path-param helper handles this naming).
    let job_id = JobId::new(job_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("job".to_owned()),
    })?;

    // Read the Job aggregate — 404 if absent.
    let key = IntentKey::for_job(&job_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await?
        .ok_or_else(|| ControlPlaneError::NotFound { resource: key.as_str().to_owned() })?;
    let archived = rkyv::access::<rkyv::Archived<Job>, rkyv::rancor::Error>(&bytes)
        .map_err(|e| ControlPlaneError::internal("rkyv access of ArchivedJob", e))?;
    let job: Job = rkyv::deserialize::<Job, rkyv::rancor::Error>(archived)
        .map_err(|e| ControlPlaneError::internal("rkyv deserialize of Job", e))?;
    let spec_digest = ContentHash::of(&bytes).to_string();

    // Filter rows to this job, project them, and stamp the requested
    // resource envelope from the JobSpec onto each row body (the bare
    // conversion zeroes resources; the handler is the only call site
    // that knows the spec).
    let resources_body = api::ResourcesBody {
        cpu_milli: job.resources.cpu_milli,
        memory_bytes: job.resources.memory_bytes,
    };
    let raw_rows = state
        .obs
        .alloc_status_rows()
        .await
        .map_err(|e| ControlPlaneError::internal("alloc_status_rows", e))?;
    let job_rows: Vec<AllocStatusRow> =
        raw_rows.into_iter().filter(|row| row.job_id == job_id).collect();

    // Per ADR-0037 §4: derive the RestartBudget from the durable
    // `AllocStatusRow.terminal` field rather than from a recomputed
    // projection over `view.restart_counts`. The reconciler is the
    // single writer of terminal claims; the durable row is the
    // single source of truth for "is this job's replica budget
    // exhausted?".
    let restart_budget = restart_budget_from_rows(&job_rows);

    let rows: Vec<api::AllocStatusRowBody> = job_rows
        .into_iter()
        .map(|row| {
            let mut body = api::AllocStatusRowBody::from(row);
            body.resources = resources_body;
            body
        })
        .collect();
    let replicas_running =
        u32::try_from(rows.iter().filter(|r| matches!(r.state, AllocStateWire::Running)).count())
            .unwrap_or(u32::MAX);

    Ok(Json(api::AllocStatusResponse {
        job_id: Some(job.id.to_string()),
        spec_digest: Some(spec_digest),
        replicas_desired: job.replicas.get(),
        replicas_running,
        rows,
        restart_budget: Some(restart_budget),
    }))
}

/// Phase 1 maximum restart attempts before a `JobLifecycle` reconciler
/// emits `Action::FinalizeFailed { terminal: BackoffExhausted }`.
/// Surfaced on the `RestartBudget.max` wire field so operators see the
/// configured budget alongside the durable `exhausted` derivation.
const RESTART_BUDGET_MAX_FOR_WIRE: u32 = 5;

/// Derive a [`RestartBudget`] from the durable
/// [`AllocStatusRow.terminal`] field per ADR-0037 §4. The budget is
/// exhausted iff any of the job's rows carries
/// `Some(TerminalCondition::BackoffExhausted { .. })` — the
/// reconciler-emitted terminal claim. Per § Persist inputs, not
/// derived state: `exhausted` is recomputed on every read from the
/// durable inputs (`row.terminal`) so a future per-job budget policy
/// change does not require a row migration.
///
/// `used` reflects the attempts count from the `BackoffExhausted`
/// variant when terminal, else 0; `max` is the Phase 1 wire constant.
fn restart_budget_from_rows(rows: &[AllocStatusRow]) -> RestartBudget {
    let exhausted_attempts = rows.iter().find_map(|row| match &row.terminal {
        Some(TerminalCondition::BackoffExhausted { attempts }) => Some(*attempts),
        _ => None,
    });
    let used = exhausted_attempts.unwrap_or(0);
    let exhausted = exhausted_attempts.is_some();
    RestartBudget { used, max: RESTART_BUDGET_MAX_FOR_WIRE, exhausted }
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
