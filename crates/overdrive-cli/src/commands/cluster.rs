//! `overdrive cluster init` — mint a fresh ephemeral CA, write the
//! Talos-shape trust triple to `<config_dir>/.overdrive/config`, and
//! return the path and endpoint to the caller.
//!
//! Per ADR-0010 §R4, re-invoking `cluster init` on an existing config
//! always re-mints the triple. The `--force` flag is reserved for future
//! non-destructive modes and has no effect in Phase 1.
//!
//! The handler is a plain `async fn` so integration tests can call it
//! directly (per `crates/overdrive-cli/CLAUDE.md`). Errors surface as
//! `CliError` — the same typed enum used by the HTTP client in
//! `http_client.rs`, extended only as needed.

use std::path::PathBuf;

use overdrive_control_plane::tls_bootstrap::{mint_ephemeral_ca, write_trust_triple};
use url::Url;

use crate::http_client::CliError;

/// Arguments to [`init`]. `config_dir` overrides the default
/// `~/.overdrive/` location — integration tests pass a `TempDir` path so
/// each test gets its own clean state. `force` is reserved per
/// ADR-0010 §R4; Phase 1 re-mints unconditionally.
#[derive(Debug, Clone)]
pub struct InitArgs {
    /// Override the default config directory. When `None`, falls back
    /// to `$OVERDRIVE_CONFIG_DIR` then `~/.overdrive/` — the binary
    /// wrapper in `main.rs` resolves the default; tests pass an
    /// explicit path.
    pub config_dir: Option<PathBuf>,
    /// Reserved for future non-destructive modes (ADR-0010 §R4); has
    /// no effect in Phase 1 because re-init ALWAYS re-mints.
    pub force: bool,
}

/// Result of a successful `cluster init`. Binary wrapper renders
/// `config_path` and `endpoint` as the operator-facing summary; tests
/// assert on both fields directly.
#[derive(Debug, Clone)]
pub struct InitOutput {
    /// Absolute path to the written trust-triple file (typically
    /// `<config_dir>/.overdrive/config`).
    pub config_path: PathBuf,
    /// Default endpoint recorded in the trust triple
    /// (`https://127.0.0.1:7001` per ADR-0008).
    pub endpoint: Url,
}

/// Mint a fresh ephemeral CA and write the Talos-shape trust triple to
/// `<config_dir>/.overdrive/config`. Re-mints on every invocation per
/// ADR-0010 §R4.
///
/// # Errors
///
/// Returns [`CliError::ConfigLoad`] if the config directory cannot be
/// resolved, or if the CA mint / trust-triple write fails. The `path`
/// field names the resolved config directory so the operator can repair
/// it; `cause` is a short, stripped summary.
pub async fn init(args: InitArgs) -> Result<InitOutput, CliError> {
    // Reserved flag in Phase 1 — ADR-0010 §R4. Suppress the
    // unused-field warning without silently dropping the field.
    let _ = args.force;

    let config_dir = resolve_config_dir(args.config_dir)?;

    // ADR-0008: control-plane default endpoint is `127.0.0.1:7001`.
    // The trust triple records it so the CLI can reach the server
    // without a separate endpoint flag.
    let endpoint_str = "https://127.0.0.1:7001";

    let material = mint_ephemeral_ca().map_err(|e| CliError::ConfigLoad {
        path: config_dir.display().to_string(),
        cause: format!("mint ephemeral CA: {e}"),
    })?;

    write_trust_triple(&config_dir, endpoint_str, &material).map_err(|e| CliError::ConfigLoad {
        path: config_dir.display().to_string(),
        cause: format!("write trust triple: {e}"),
    })?;

    let config_path = config_dir.join(".overdrive").join("config");
    let endpoint = Url::parse(endpoint_str).map_err(|e| CliError::ConfigLoad {
        path: config_dir.display().to_string(),
        cause: format!("parse endpoint: {e}"),
    })?;

    Ok(InitOutput { config_path, endpoint })
}

/// Resolve the effective config directory. Explicit override wins;
/// otherwise fall back to `$OVERDRIVE_CONFIG_DIR` then `~/.overdrive/`.
fn resolve_config_dir(explicit: Option<PathBuf>) -> Result<PathBuf, CliError> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    if let Ok(env) = std::env::var("OVERDRIVE_CONFIG_DIR") {
        return Ok(PathBuf::from(env));
    }
    // Fall back to `$HOME`. The `dirs` crate is not in the workspace
    // graph (design principle 1: own your primitives — workspace deps
    // are explicit). `$HOME` is universally set on Unix; Windows gets
    // a separate code path if Phase 1 ever ships a Windows target.
    let home = std::env::var_os("HOME").ok_or_else(|| CliError::ConfigLoad {
        path: "<unresolved home directory>".to_owned(),
        cause: "home directory could not be resolved ($HOME unset); pass --config-dir explicitly"
            .to_owned(),
    })?;
    Ok(PathBuf::from(home).join(".overdrive"))
}
