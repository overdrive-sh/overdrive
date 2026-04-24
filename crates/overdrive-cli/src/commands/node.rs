//! `overdrive node list`.
//!
//! Reads the observation-store `node_health` rows from the control
//! plane and returns a typed `NodeListOutput` carrying the rows plus an
//! explicit empty-state message referencing the
//! `phase-1-first-workload` onboarding step.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handler is a plain
//! `async fn` that integration tests call directly — no subprocess, no
//! stdout I/O. Rendering lives in `crate::render::node_list`.

use std::path::PathBuf;

use overdrive_control_plane::api::NodeRowBody;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`list`].
///
/// `config_path` locates the operator trust triple, which is the sole
/// source of the control-plane endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct ListArgs {
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the GET is issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `node list`. Carries the raw rows from
/// the control plane plus the empty-state message the render layer
/// emits when `rows` is empty.
#[derive(Debug, Clone)]
pub struct NodeListOutput {
    /// Raw rows from `GET /v1/nodes`, re-exported unchanged per
    /// ADR-0014 (no shadow types).
    pub rows: Vec<NodeRowBody>,
    /// Operator-facing empty-state message pointing at the
    /// `phase-1-first-workload` onboarding step. Rendered verbatim
    /// when `rows` is empty; ignored otherwise.
    pub empty_state_message: String,
}

/// Default empty-state message for `node list`. Referenced by the
/// render layer when `rows.is_empty()`.
const EMPTY_STATE_MESSAGE: &str =
    "no nodes registered — see `phase-1-first-workload` to register the first node";

/// List node-health rows from the observation store.
///
/// # Errors
///
/// Returns `CliError::ConfigLoad` if the trust triple cannot be loaded,
/// `CliError::Transport` if the control plane is unreachable, and
/// `CliError::HttpStatus` / `CliError::BodyDecode` on a malformed
/// server response.
pub async fn list(args: ListArgs) -> Result<NodeListOutput, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;
    let node_list = client.node_list().await?;
    Ok(NodeListOutput { rows: node_list.rows, empty_state_message: EMPTY_STATE_MESSAGE.to_owned() })
}
