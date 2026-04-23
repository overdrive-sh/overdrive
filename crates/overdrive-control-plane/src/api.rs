//! Shared request/response types for the Phase 1 REST API.
//!
//! Per ADR-0014 (§Shared types), the CLI imports these same types
//! directly — they ARE the wire contract. The OpenAPI schema derived
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
/// `job_id` is rendered as a `String` at the wire boundary; the server
/// converts back to `overdrive_core::id::JobId` in handlers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub commit_index: u64,
}

/// Response for `GET /v1/jobs/{id}`. Carries the re-hydrated spec, the
/// commit index at which it was written, and the canonical spec
/// digest per ADR-0014 and US-03 AC.
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

/// Response for `GET /v1/cluster/info`. Carries mode, region,
/// commit_index, the reconciler registry, and the broker counters per
/// ADR-0013 and US-04 AC.
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

/// Response for `GET /v1/allocs`. Phase 1 always renders an empty
/// `rows` array per US-03 AC — the allocation-status path is owned by
/// Phase 2. The typed `rows` field is present so the CLI and external
/// clients can parse the response into a concrete shape rather than
/// `serde_json::Value`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AllocStatusResponse {
    pub rows: Vec<AllocStatusRowBody>,
}

/// Allocation-status row body. Empty in Phase 1 — the fields land in
/// Phase 2 alongside the observation-store schema. The type exists now
/// so downstream callers (CLI, openapi-gen) can reference a stable
/// name; future columns land additively.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {}

/// Response for `GET /v1/nodes`. Phase 1 always renders an empty
/// `rows` array per US-03 AC — node ingestion lands in a later phase.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct NodeList {
    pub rows: Vec<NodeRowBody>,
}

/// Node row body. Empty in Phase 1 — same forward-compatibility
/// rationale as [`AllocStatusRowBody`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct NodeRowBody {}

/// RFC-7807-compatible subset per ADR-0015. The three fields —
/// `error`, `message`, `field` — are pinned; renaming breaks the
/// contract surface the CLI and external clients depend on. `field` is
/// `Option<String>` because not every error class maps to a single
/// field (e.g. transport-layer errors).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    pub field: Option<String>,
}
