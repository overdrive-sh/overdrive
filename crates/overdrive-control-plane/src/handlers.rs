//! axum route handlers for the Phase 1 control-plane API.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
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
//! Bodies are `panic!` stubs. Each handler will gain its `#[utoipa::path]`
//! annotation when `utoipa` integration lands with Slice 3.

use crate::api;
use crate::error::ControlPlaneError;

/// `POST /v1/jobs` — validate, archive via rkyv, commit through the
/// intent store, return `(job_id, commit_index)`.
///
/// SCAFFOLD: true
pub async fn submit_job(
    _request: api::SubmitJobRequest,
) -> Result<api::SubmitJobResponse, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/jobs/{id}` — read via `IntentStore::get`, rkyv-access the
/// bytes, recompute `spec_digest = ContentHash::of(archived_bytes)`.
///
/// SCAFFOLD: true
pub async fn describe_job(_id: String) -> Result<api::JobDescription, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/cluster/info` — mode, region, commit_index, reconciler
/// registry, broker counters.
///
/// SCAFFOLD: true
pub async fn cluster_status() -> Result<api::ClusterStatus, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/allocs` — observation read on `alloc_status`. Phase 1: zero
/// rows per US-03 AC.
///
/// SCAFFOLD: true
pub async fn alloc_status() -> Result<api::AllocStatusResponse, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/nodes` — observation read on `node_health`. Phase 1: zero
/// rows per US-03 AC.
///
/// SCAFFOLD: true
pub async fn node_list() -> Result<api::NodeList, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}
