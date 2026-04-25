//! Shared request/response types for the Phase 1 REST API.
//!
//! Per ADR-0014 (§Shared types), the CLI imports these same types
//! directly — they ARE the wire contract. The `OpenAPI` schema derived
//! via `utoipa` (ADR-0009) is a byproduct of these types, not a
//! parallel definition.
//!
//! The shapes pinned here are:
//! - Step 02-03 AC — exact field sets; renaming breaks the contract.
//! - ADR-0008 — endpoint table (`POST /v1/jobs`, `GET /v1/jobs/{id}`,
//!   `GET /v1/cluster/info`, `GET /v1/allocs`, `GET /v1/nodes`).
//! - ADR-0015 — `ErrorBody` shape `{error, message, field}`.
//!
//! `JobSpecInput` is re-used from `overdrive_core::aggregate` so there
//! is exactly one definition of the spec shape on the wire. The CLI
//! will construct `JobSpecInput` from its TOML input; the server will
//! deserialise the same type out of JSON; both route through
//! `Job::from_spec` for validation.

// The `utoipa::OpenApi` derive on `OverdriveApi` below expands to code
// using `.for_each(...)` on the collected schemas. The lint fires on
// the macro expansion rather than any source we wrote, and outer
// `#[allow]` attributes do not propagate into the derive. Scope the
// allow to this module, which contains exactly one `utoipa` derive.
#![allow(clippy::needless_for_each)]

use overdrive_core::aggregate::JobSpecInput;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Body of `POST /v1/jobs`. Carries the operator-submitted job spec
/// verbatim; the server routes it through `Job::from_spec` to validate
/// and derive the intent key / digest.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobRequest {
    pub spec: JobSpecInput,
}

/// Response for `POST /v1/jobs`. Carries `job_id` + `commit_index` per
/// journey step 1 and ADR-0008.
///
/// **Per-entry semantics.** `commit_index` here is the index at which
/// THIS submission's bytes were written — captured atomically inside
/// the `IntentStore` write transaction via `PutOutcome::Inserted` /
/// `PutOutcome::KeyExists`. A byte-identical resubmission returns the
/// index of the original insert (idempotent); a different spec at the
/// same key returns 409 Conflict without a fresh index. This is the
/// same per-entry value [`JobDescription::commit_index`] returns for
/// the same job, regardless of how many other writes have landed
/// since.
///
/// Distinct from [`ClusterStatus::commit_index`], which is the
/// store-wide commit counter — see that field's docstring.
///
/// `job_id` is rendered as a `String` at the wire boundary; the server
/// converts back to `overdrive_core::id::JobId` in handlers.
///
/// [`JobDescription::commit_index`]: JobDescription::commit_index
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub commit_index: u64,
}

/// Response for `GET /v1/jobs/{id}`. Carries the re-hydrated spec, the
/// commit index at which it was written, and the canonical spec
/// digest per ADR-0014 and US-03 AC.
///
/// **Per-entry semantics.** `commit_index` here is the index at which
/// THIS specific job's bytes were committed — read alongside the
/// stored value via `IntentStore::get`'s `(bytes, commit_index)`
/// tuple. Stable across the job's lifetime: subsequent writes to
/// other keys do NOT advance the value returned here. Equal to the
/// `commit_index` the original `POST /v1/jobs` returned for this
/// `job_id`.
///
/// Distinct from [`ClusterStatus::commit_index`], which is the
/// store-wide commit counter — see that field's docstring.
///
/// `spec` is typed (`JobSpecInput`), never `serde_json::Value` — the
/// CLI parses this response into a concrete type rather than a value
/// bag.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobDescription {
    pub spec: JobSpecInput,
    pub commit_index: u64,
    pub spec_digest: String,
}

/// Response for `GET /v1/cluster/info`.
///
/// Carries mode, region, `commit_index`, the reconciler registry, and
/// the broker counters per ADR-0013 and US-04 AC.
///
/// **Store-wide cursor semantics.** `commit_index` here is the
/// store-wide commit counter — total successful writes since the
/// process started, advancing on every `put` / `delete` / `txn`
/// commit. This is intentionally NOT a per-entry index: aggregate
/// observability ("how many writes has this control plane committed?")
/// is the use case, not "what index was assigned to a specific
/// entry?".
///
/// Distinct from [`SubmitJobResponse::commit_index`] and
/// [`JobDescription::commit_index`], which carry per-entry indices —
/// the index at which a specific job's bytes were written. Use those
/// for optimistic-concurrency / watch-resume / audit-ordering against
/// a single entry; use this for cluster-level commit-rate signals.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterStatus {
    pub mode: String,
    pub region: String,
    pub commit_index: u64,
    pub reconcilers: Vec<String>,
    pub broker: BrokerCountersBody,
}

/// Broker counters rendered inside `ClusterStatus`. Tracks the
/// evaluation-broker ingress / cancel / dispatch shape from ADR-0013.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, ToSchema)]
pub struct BrokerCountersBody {
    pub queued: u64,
    pub cancelled: u64,
    pub dispatched: u64,
}

/// Response for `GET /v1/allocs`.
///
/// Phase 1 always renders an empty `rows` array per US-03 AC — the
/// allocation-status path is owned by Phase 2. The typed `rows` field
/// is present so the CLI and external clients can parse the response
/// into a concrete shape rather than `serde_json::Value`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AllocStatusResponse {
    pub rows: Vec<AllocStatusRowBody>,
}

/// Allocation-status row body.
///
/// Phase 1 shape mirrors the observation `AllocStatusRow` projected to
/// the wire — minimal fields matching the whitepaper §4 schema
/// (`alloc_id`, `job_id`, `node_id`, `state`). Phase 2+ adds columns
/// additively.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub job_id: String,
    pub node_id: String,
    pub state: String,
}

/// Response for `GET /v1/nodes`. Phase 1 always renders an empty
/// `rows` array per US-03 AC — node ingestion lands in a later phase.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct NodeList {
    pub rows: Vec<NodeRowBody>,
}

/// Node row body. Phase 1 shape mirrors the observation `NodeHealthRow`
/// projected to the wire — minimal fields (`node_id`, `region`). Phase
/// 2+ adds columns additively.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct NodeRowBody {
    pub node_id: String,
    pub region: String,
}

/// RFC-7807-compatible subset per ADR-0015.
///
/// The three fields — `error`, `message`, `field` — are pinned;
/// renaming breaks the contract surface the CLI and external clients
/// depend on. `field` is `Option<String>` because not every error class
/// maps to a single field (e.g. transport-layer errors).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    pub field: Option<String>,
}

/// Root `OpenAPI` document per ADR-0009.
///
/// Every ADR-0008 handler path is listed in `paths(...)` and every
/// request/response DTO in `components(schemas(...))`. The schema is
/// derived by `utoipa` at compile time; `cargo xtask openapi-gen`
/// writes the YAML rendering of `OverdriveApi::openapi()` to
/// `api/openapi.yaml`; `cargo xtask openapi-check` diffs the live
/// render against the checked-in copy and fails on drift.
///
/// Adding a handler requires adding its path here; adding a DTO
/// requires adding its schema. Drift between code and the `OpenAPI`
/// doc is caught by the CI gate, not in review.
#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "Overdrive Control Plane",
        description = "Phase 1 single-mode control-plane REST API (ADR-0008).",
        version = "0.0.0",
    ),
    paths(
        crate::handlers::submit_job,
        crate::handlers::describe_job,
        crate::handlers::cluster_status,
        crate::handlers::alloc_status,
        crate::handlers::node_list,
    ),
    components(schemas(
        SubmitJobRequest,
        SubmitJobResponse,
        JobDescription,
        ClusterStatus,
        BrokerCountersBody,
        AllocStatusResponse,
        AllocStatusRowBody,
        NodeList,
        NodeRowBody,
        ErrorBody,
        JobSpecInput,
    )),
    tags(
        (name = "jobs", description = "Job lifecycle endpoints"),
        (name = "cluster", description = "Cluster status endpoints"),
        (name = "observation", description = "Observation-store read endpoints"),
    ),
)]
pub struct OverdriveApi;
