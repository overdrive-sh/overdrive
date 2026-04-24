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

use overdrive_control_plane::api::BrokerCountersBody;
use overdrive_control_plane::tls_bootstrap::{mint_ephemeral_ca, write_trust_triple};
use url::Url;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`init`].
///
/// `config_dir` overrides the default `~/.overdrive/` location —
/// integration tests pass a `TempDir` path so each test gets its own
/// clean state. `force` is reserved per ADR-0010 §R4; Phase 1 re-mints
/// unconditionally.
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
// `async` is kept to match the CLI command-handler contract documented
// in `crates/overdrive-cli/CLAUDE.md` — every handler is a plain
// `async fn` that tests call directly as `handler(args).await`. The
// shape is uniform across the commands module even when a specific
// handler happens to have no `.await` points yet (`init` will grow
// them as `mint_ephemeral_ca` and `write_trust_triple` gain async
// boundaries for file I/O).
#[allow(clippy::unused_async)]
pub async fn init(args: InitArgs) -> Result<InitOutput, CliError> {
    // Reserved flag in Phase 1 — ADR-0010 §R4. Suppress the
    // unused-field warning without silently dropping the field.
    let _ = args.force;

    let config_dir = resolve_config_dir(args.config_dir)?;

    // ADR-0008: control-plane default endpoint is `127.0.0.1:7001`.
    // The trust triple records it so the CLI can reach the server
    // without a separate endpoint flag.
    let endpoint_str = "https://127.0.0.1:7001";

    // Every failure in this function surfaces the same `path` (the
    // config dir we resolved above) with a per-operation cause — the
    // closure factors the three copies out.
    let config_load_err = |cause_prefix: &str, err: &dyn std::fmt::Display| CliError::ConfigLoad {
        path: config_dir.display().to_string(),
        cause: format!("{cause_prefix}: {err}"),
    };

    let material = mint_ephemeral_ca().map_err(|e| config_load_err("mint ephemeral CA", &e))?;

    write_trust_triple(&config_dir, endpoint_str, &material)
        .map_err(|e| config_load_err("write trust triple", &e))?;

    let config_path = config_dir.join(".overdrive").join("config");
    let endpoint = Url::parse(endpoint_str).map_err(|e| config_load_err("parse endpoint", &e))?;

    Ok(InitOutput { config_path, endpoint })
}

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
        commit_index: cs.commit_index,
        reconcilers: cs.reconcilers,
        broker: cs.broker,
    })
}

/// Resolve the effective config directory. Explicit override wins;
/// otherwise fall back to `$OVERDRIVE_CONFIG_DIR` then `$HOME`. The
/// returned path is always a BASE directory — `write_trust_triple`
/// owns the `.overdrive/config` suffix and appends it exactly once
/// (see `overdrive-control-plane/src/tls_bootstrap.rs::write_trust_triple`).
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
    Ok(PathBuf::from(home))
}

/// Canonical operator config file path per ADR-0010 / ADR-0014 /
/// ADR-0019 and whitepaper §8.
///
/// Resolves the base directory from `$OVERDRIVE_CONFIG_DIR` first, then
/// `$HOME`, then the current directory as a last-resort fallback. The
/// `.overdrive` segment and `config` filename are always appended
/// exactly once — callers MUST pass bare base-dir env-var values and
/// MUST NOT pre-suffix. Returns the full path to the file (for example
/// `~/.overdrive/config`), not the containing directory.
///
/// This is the single source of truth for where the operator CLI
/// reads and writes its trust triple. Both `main.rs::default_config_path`
/// (read side) and the HOME fallback of `resolve_config_dir` +
/// `write_trust_triple` (write side) compose around this function so
/// the two sites cannot drift — the drift between them is the bug this
/// function's existence prevents (`fix-overdrive-config-path-doubled`).
#[must_use]
pub fn default_operator_config_path() -> PathBuf {
    let base = std::env::var_os("OVERDRIVE_CONFIG_DIR")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join(".overdrive").join("config")
}
