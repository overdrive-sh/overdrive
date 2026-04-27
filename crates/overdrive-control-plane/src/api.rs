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

/// Response for `POST /v1/jobs`. Carries `job_id`, the canonical
/// `spec_digest`, and the idempotency `outcome` per ADR-0008 and
/// ADR-0020.
///
/// Idempotency contract (ADR-0015 §4 amended by ADR-0020):
///
/// * Fresh insert → `outcome = IdempotencyOutcome::Inserted`, HTTP 200.
/// * Byte-identical resubmission of the same spec at the same key →
///   `outcome = IdempotencyOutcome::Unchanged`, HTTP 200. No write
///   occurred; `spec_digest` is stable across N retries.
/// * Different spec at the same key → HTTP 409 Conflict, no `outcome`
///   field on the wire (conflict is an HTTP-status concern, never an
///   enum value — see ADR-0015 §4 amendment via ADR-0020).
///
/// `job_id` is rendered as a `String` at the wire boundary; the server
/// converts back to `overdrive_core::id::JobId` in handlers.
/// `spec_digest` is the lowercase-hex SHA-256 of the canonical
/// rkyv-archived `Job` bytes (ADR-0002, development.md §Hashing); 64
/// characters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub spec_digest: String,
    pub outcome: IdempotencyOutcome,
}

/// Outcome of an idempotent `POST /v1/jobs` submission.
///
/// Distinguishes "your spec landed fresh" from "your spec was already
/// there." Conflict (different spec at same key) is an HTTP-status
/// concern (409), never an enumeration value here — see ADR-0015 §4
/// amendment via ADR-0020.
///
/// Wire shape: `"inserted"` | `"unchanged"` (lowercase JSON via
/// `#[serde(rename_all = "lowercase")]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum IdempotencyOutcome {
    /// The handler took the insert branch — `IntentStore::put_if_absent`
    /// returned `PutOutcome::Inserted`.
    Inserted,
    /// The handler took the idempotency branch —
    /// `IntentStore::put_if_absent` returned
    /// `PutOutcome::KeyExists { existing }` and the candidate bytes
    /// were byte-equal to `existing`. No write occurred.
    Unchanged,
}

/// Response for `GET /v1/jobs/{id}`. Carries the re-hydrated spec and
/// the canonical spec digest per ADR-0014 and US-03 AC (amended by
/// ADR-0020).
///
/// `spec` is typed (`JobSpecInput`), never `serde_json::Value` — the
/// CLI parses this response into a concrete type rather than a value
/// bag. `spec_digest` equals the lowercase-hex SHA-256 of the
/// rkyv-archived bytes pulled out of the `IntentStore` — i.e. the
/// same value the original `POST /v1/jobs` returned for this `job_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobDescription {
    pub spec: JobSpecInput,
    pub spec_digest: String,
}

/// Response for `GET /v1/cluster/info`.
///
/// Carries mode, region, the reconciler registry, and the broker
/// counters per ADR-0013 and US-04 AC (amended by ADR-0020 — the
/// `commit_index` field is dropped, no replacement).
///
/// Activity-rate observability is provided by `broker.dispatched`
/// (heartbeat reconciler ticks) plus the `reconcilers` list (the
/// "did the runtime register?" wiring witness). A dedicated metrics
/// endpoint covers cluster-level commit-rate signals starting in
/// Phase 5; the dropped in-memory counter was not a substitute for
/// it. See ADR-0020 §Considered alternatives §D for the full
/// rationale.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterStatus {
    pub mode: String,
    pub region: String,
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
///
/// `reason` is `Option<String>` — populated when the underlying state
/// carries actionable diagnostic context (currently only Pending rows
/// resulting from a `PlacementError::NoCapacity`). Other states leave
/// it `None`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub job_id: String,
    pub node_id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AllocStatusRowBody {
    /// Construct a Pending row body decorated with an actionable
    /// diagnostic `reason` — the `JobLifecycle` reconciler calls this
    /// shape when surfacing `PlacementError::NoCapacity` to the CLI.
    #[must_use]
    pub fn pending_with_reason(
        row: &overdrive_core::traits::observation_store::AllocStatusRow,
        reason: String,
    ) -> Self {
        Self {
            alloc_id: row.alloc_id.to_string(),
            job_id: row.job_id.to_string(),
            node_id: row.node_id.to_string(),
            state: row.state.to_string(),
            reason: Some(reason),
        }
    }
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
        IdempotencyOutcome,
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
