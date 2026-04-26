//! `overdrive cluster status` — read the live control-plane mode,
//! region, Raft commit index, reconciler registry, and broker counters
//! over the operator trust triple at `<config_dir>/.overdrive/config`.
//!
//! Phase 1 has exactly one cert-minting site, and it is `serve` — the
//! `cluster init` verb that previously lived in this module was deleted
//! per `fix-remove-phase-1-cluster-init` (#81). RCA:
//! `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.
//! ADR-0010 §R5 (no cert persistence in the server process) makes
//! Phase 1 structurally incapable of honouring an init-produced cert,
//! so `cluster init` is a Phase 5 verb shipped early and tracked in
//! issue #81 for reintroduction alongside `op create` / `op revoke`.
//!
//! `default_operator_config_dir` and `default_operator_config_path`
//! remain — `serve` and `job submit` use them to compute the canonical
//! read/write target for the trust triple.
//!
//! The handler is a plain `async fn` so integration tests can call it
//! directly (per `crates/overdrive-cli/CLAUDE.md`). Errors surface as
//! `CliError` — the same typed enum used by the HTTP client in
//! `http_client.rs`, extended only as needed.

use std::path::PathBuf;

use overdrive_control_plane::api::BrokerCountersBody;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`status`].
///
/// `config_path` locates the operator trust triple, which is the sole
/// source of the control-plane endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct StatusArgs {
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the GET is issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `cluster status`. Carries the control
/// plane's self-reported mode, region, Raft commit index, the
/// reconciler registry, and the typed broker counters per ADR-0013.
#[derive(Debug, Clone)]
pub struct ClusterStatusOutput {
    /// Phase 1 control-plane mode — always `single` until HA lands.
    pub mode: String,
    /// Phase 1 region — always `local` until multi-region lands.
    pub region: String,
    /// Monotonic `IntentStore` commit counter. Zero on a fresh store.
    pub commit_index: u64,
    /// Alphabetically-sorted reconciler names registered with the
    /// runtime. Phase 1 must contain `noop-heartbeat` per ADR-0013 §9.
    pub reconcilers: Vec<String>,
    /// Evaluation-broker counters (queued / cancelled / dispatched).
    pub broker: BrokerCountersBody,
}

/// Read cluster status from the control plane.
///
/// # Errors
///
/// Returns `CliError::ConfigLoad` if the trust triple cannot be loaded,
/// `CliError::Transport` if the control plane is unreachable, and
/// `CliError::HttpStatus` / `CliError::BodyDecode` on a malformed
/// server response.
pub async fn status(args: StatusArgs) -> Result<ClusterStatusOutput, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;
    let cs = client.cluster_status().await?;
    Ok(ClusterStatusOutput {
        mode: cs.mode,
        region: cs.region,
        // ADR-0020: the API no longer surfaces a commit_index. The
        // CLI-internal field is dead-data carried until step 01-03
        // deletes the wire-render shape; populate with 0 so the
        // workspace compiles and the deletion in 01-03 is purely
        // mechanical.
        commit_index: 0,
        reconcilers: cs.reconcilers,
        broker: cs.broker,
    })
}

/// Canonical operator config BASE directory per ADR-0010 / ADR-0014 /
/// ADR-0019 and whitepaper §8.
///
/// Resolves from `$OVERDRIVE_CONFIG_DIR` first, then `$HOME`, then the
/// current directory as a last-resort fallback. Returns the BASE
/// directory only — the `.overdrive` segment and `config` filename are
/// NOT appended. `write_trust_triple` owns that suffix.
///
/// This is the single source of truth for the operator-config base
/// directory. `default_operator_config_path` delegates here for read
/// resolution; `serve::run` threads this value into
/// `ServerConfig::operator_config_dir` so the trust-triple write site
/// composes exactly the same path. Drift between read and write sites
/// is the bug class this function's existence prevents
/// (`fix-cli-cannot-reach-control-plane`,
/// `fix-overdrive-config-path-doubled`).
#[must_use]
pub fn default_operator_config_dir() -> PathBuf {
    std::env::var_os("OVERDRIVE_CONFIG_DIR")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Canonical operator config FILE path per ADR-0010 / ADR-0014 /
/// ADR-0019 and whitepaper §8.
///
/// Returns `<base>/.overdrive/config` where `<base>` is
/// [`default_operator_config_dir`]. Delegates to that helper so the
/// read-side computation cannot drift from the write-side base
/// resolution (`fix-cli-cannot-reach-control-plane` Fix step 7;
/// `fix-overdrive-config-path-doubled` Fix 3).
///
/// Callers that need the containing directory rather than the file
/// path call [`default_operator_config_dir`] directly.
#[must_use]
pub fn default_operator_config_path() -> PathBuf {
    default_operator_config_dir().join(".overdrive").join("config")
}
