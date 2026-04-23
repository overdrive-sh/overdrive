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
//! Bodies remain `panic!` stubs until Slice 3 lands the real wiring —
//! the `#[utoipa::path]` annotations below walk the attribute only, so
//! the OpenAPI schema can be derived without exercising the bodies.

use crate::api;
use crate::error::ControlPlaneError;

/// `POST /v1/jobs` — validate, archive via rkyv, commit through the
/// intent store, return `(job_id, commit_index)`.
///
/// SCAFFOLD: true
#[utoipa::path(
    post,
    path = "/v1/jobs",
    request_body = api::SubmitJobRequest,
    responses(
        (status = 200, description = "Job accepted", body = api::SubmitJobResponse),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn submit_job(
    _request: api::SubmitJobRequest,
) -> Result<api::SubmitJobResponse, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// `GET /v1/jobs/{id}` — read via `IntentStore::get`, rkyv-access the
/// bytes, recompute `spec_digest = ContentHash::of(archived_bytes)`.
///
/// SCAFFOLD: true
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
/// SCAFFOLD: true
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
/// SCAFFOLD: true
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
/// SCAFFOLD: true
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
