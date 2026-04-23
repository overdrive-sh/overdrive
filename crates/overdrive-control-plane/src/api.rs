//! Shared request/response types for the Phase 1 REST API.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0014, the CLI imports these same types directly — they ARE the
//! wire contract. The OpenAPI schema derived via `utoipa` is a byproduct
//! of these types, not a parallel definition.
//!
//! Every type below is a `SCAFFOLD: true` placeholder. The DELIVER crafter
//! fills in field sets per ADR-0008 (endpoint table) and ADR-0015
//! (error body shape) as each slice lands.

use serde::{Deserialize, Serialize};

/// Body of `POST /v1/jobs`. Carries a serialised Job spec.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitJobRequest {
    // Field set lands with Slice 2 per ADR-0014.
}

/// Response for `POST /v1/jobs`. Carries `job_id` + `commit_index`
/// per journey step 1 and ADR-0008.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub commit_index: u64,
}

/// Response for `GET /v1/jobs/{id}`. Carries the re-hydrated spec,
/// the commit_index, and the spec_digest per ADR-0014 and US-03 AC.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescription {
    pub spec: serde_json::Value,
    pub commit_index: u64,
    pub spec_digest: String,
}

/// Response for `GET /v1/cluster/info`. Carries mode, region,
/// commit_index, and the reconciler registry + broker counters per
/// ADR-0013 and US-04 AC.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub mode: String,
    pub region: String,
    pub commit_index: u64,
    pub reconcilers: Vec<String>,
    pub broker: BrokerCountersBody,
}

/// Broker counters rendered by `GET /v1/cluster/info`.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct BrokerCountersBody {
    pub queued: u64,
    pub cancelled: u64,
    pub dispatched: u64,
}

/// Response for `GET /v1/allocs`. Empty row set in Phase 1.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AllocStatusResponse {
    pub rows: Vec<serde_json::Value>,
}

/// Response for `GET /v1/nodes`. Empty row set in Phase 1.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeList {
    pub rows: Vec<serde_json::Value>,
}

/// RFC-7807-compatible subset per ADR-0015. Shape:
/// `{error: <kind>, message: <human>, field: <optional>}`.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    pub field: Option<String>,
}
