//! `overdrive workload restart <id>` — backend-instance-replacement
//! slice 01, step 01-04.
//!
//! New top-level `workload` namespace (NOT under `job`, #220-aligned)
//! carrying the operator-facing restart verb that drives the
//! `POST /v1/jobs/{id}/restart` route shipped by step 01-03. Per ADR-0073
//! § "The six pinned signatures" item 1 this is the DESIGN-table-recorded
//! home for `RestartArgs` / `RestartOutput` / `restart` — a separate
//! module from `deploy.rs` because the restart handler is a distinct
//! workload-lifecycle concern (#220 will add `describe` here), not a
//! deploy/submit concern.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handler is a plain `async fn`
//! that integration tests call directly — no subprocess, no `println!`.
//! Rendering lives in `crate::render::workload_restart_accepted`.

use std::path::PathBuf;

use overdrive_control_plane::api::RestartOutcome;
use overdrive_core::id::WorkloadId;
use url::Url;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`restart`]. Mirrors `crate::commands::deploy::StopArgs`.
#[derive(Debug, Clone)]
pub struct RestartArgs {
    /// Canonical `WorkloadId` to restart. Validated client-side via
    /// `WorkloadId::new` before any HTTP call so operators see the
    /// offending byte without a round-trip.
    pub id: String,
    /// Path to the trust triple. Same conventions as
    /// `crate::commands::deploy::DeployArgs` — the recorded endpoint is
    /// where the POST is issued.
    pub config_path: PathBuf,
}

/// Typed output of `overdrive workload restart`.
///
/// Carries the server's echoed `workload_id`, the `outcome` (`Restarted`
/// vs `Resumed`), and the endpoint the POST was issued to. Mirrors
/// `crate::commands::deploy::StopOutput`.
#[derive(Debug, Clone)]
pub struct RestartOutput {
    /// Workload ID echoed by the server.
    pub workload_id: String,
    /// Restart outcome echoed by the control plane — `Restarted` when no
    /// live stop intent was on file, `Resumed` when an operator-stop
    /// sentinel was present at the check-exists read (ADR-0073 item 2).
    pub outcome: RestartOutcome,
    /// Endpoint the POST was issued to, echoed for operator clarity.
    pub endpoint: Url,
}

/// Replace a declared workload's backend instance with a fresh one by
/// driving `POST /v1/jobs/{id}/restart`.
///
/// Per ADR-0073: returns 200 OK with `outcome = Restarted` when no live
/// stop intent existed at the handler's check-exists read, and
/// `outcome = Resumed` when an operator-stop sentinel was present.
/// Returns 404 if the workload was never declared.
///
/// # Errors
///
/// * [`CliError::InvalidSpec`] — `id` does not parse as a canonical `WorkloadId`.
/// * [`CliError::ConfigLoad`] — trust triple unloadable.
/// * [`CliError::Transport`] — control plane unreachable.
/// * [`CliError::HttpStatus`] — server returned non-2xx (404 unknown,
///   with `body.error == "not_found"`).
/// * [`CliError::BodyDecode`] — 2xx body decode failed.
pub async fn restart(args: RestartArgs) -> Result<RestartOutput, CliError> {
    // Client-side validation — fail fast on malformed ids before any
    // HTTP call, same discipline as `commands::deploy::stop`.
    let _ = WorkloadId::new(&args.id)
        .map_err(|e| CliError::InvalidSpec { field: "id".to_string(), message: e.to_string() })?;

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client.restart_workload(&args.id).await?;

    Ok(RestartOutput { workload_id: resp.workload_id, outcome: resp.outcome, endpoint })
}
