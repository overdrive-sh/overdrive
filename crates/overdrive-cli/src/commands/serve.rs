//! `overdrive serve` — start the Phase 1 control-plane server.
//!
//! Wraps `overdrive_control_plane::run_server` into the CLI-facing shape:
//! the handler returns a [`ServeHandle`] whose `endpoint()` names the
//! actually-bound address (ephemeral port in tests) and whose
//! `shutdown()` drains in-flight connections before closing the listener.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, this is a plain `async fn` that
//! tests call directly; SIGINT handling lives in `main.rs` and delegates
//! into `ServeHandle::shutdown`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use url::Url;

use crate::http_client::CliError;

/// Default drain deadline for `ServeHandle::shutdown`. In-flight
/// requests complete within this window; new connections are refused
/// immediately. The window matches the 5s assertion in the integration
/// tests and leaves headroom for realistic request shapes.
const DEFAULT_DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// Arguments to [`run`].
#[derive(Debug, Clone)]
pub struct ServeArgs {
    /// Socket address to bind the HTTPS listener. Use `127.0.0.1:0` in
    /// tests to request an ephemeral port.
    pub bind: SocketAddr,
    /// Storage root for the redb file and per-primitive libSQL files
    /// (ADR-0013 §5). The trust triple does NOT live here — see
    /// [`Self::config_dir`].
    pub data_dir: PathBuf,
    /// Operator-config base directory. The trust triple is written to
    /// `<config_dir>/.overdrive/config` so the operator CLI reads the
    /// same file the server writes (whitepaper §8, ADR-0019). The
    /// binary wrapper in `main.rs` defaults this via
    /// `commands::cluster::default_operator_config_dir()`; tests pass
    /// an explicit subdirectory of their `TempDir`.
    pub config_dir: PathBuf,
}

/// Handle to a running control-plane server, owned by the CLI layer.
///
/// Wraps [`overdrive_control_plane::ServerHandle`] with the
/// CLI-specific URL shape returned by [`endpoint`]. Consumed by
/// [`shutdown`], which drains in-flight connections before closing the
/// listener.
///
/// [`endpoint`]: ServeHandle::endpoint
/// [`shutdown`]: ServeHandle::shutdown
#[derive(Debug)]
pub struct ServeHandle {
    inner: ServerHandle,
    endpoint: Url,
}

impl ServeHandle {
    /// The URL the server is actually listening on, including the
    /// ephemerally-resolved port when `bind` was `127.0.0.1:0`.
    #[must_use]
    pub const fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Trigger graceful shutdown. In-flight requests complete within a
    /// 5-second deadline; new connections are refused immediately.
    /// Awaits the server task to completion.
    ///
    /// # Errors
    ///
    /// Currently infallible — the future always resolves to `Ok(())`
    /// once the listener closes. The `Result` shape is reserved for a
    /// future deadline-exceeded variant.
    pub async fn shutdown(self) -> Result<(), CliError> {
        self.inner.shutdown(DEFAULT_DRAIN_DEADLINE).await;
        Ok(())
    }
}

/// Start the Phase 1 control-plane server. Wraps
/// [`overdrive_control_plane::run_server`] and converts
/// `ControlPlaneError` into [`CliError`] variants.
///
/// # Errors
///
/// Returns [`CliError::Transport`] when the listener cannot bind the
/// requested address (e.g. port already in use), or the TLS
/// configuration fails. `endpoint` names the requested bind address so
/// the operator can act on the error.
pub async fn run(args: ServeArgs) -> Result<ServeHandle, CliError> {
    let requested_endpoint = format!("https://{}", args.bind);

    // `..Default::default()` populates `tick_cadence`
    // (`reconciler_runtime::DEFAULT_TICK_CADENCE`, 100ms) and `clock`
    // (`Arc::new(SystemClock)` from `overdrive-host`). Per CLAUDE.md
    // "Repository structure" `overdrive-host` is the only crate
    // permitted to instantiate `SystemClock`, so the binding lives in
    // the `Default` impl of `ServerConfig` rather than this call site.
    // Per `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2).
    let config = ServerConfig {
        bind: args.bind,
        data_dir: args.data_dir,
        operator_config_dir: args.config_dir,
        ..Default::default()
    };
    let inner = run_server(config).await.map_err(|e| CliError::Transport {
        endpoint: requested_endpoint.clone(),
        cause: stripped_server_error(&e.to_string()),
    })?;

    let bound = inner.local_addr().await.ok_or_else(|| CliError::Transport {
        endpoint: requested_endpoint.clone(),
        cause: "server bound but did not report a local address".to_owned(),
    })?;

    let endpoint = Url::parse(&format!("https://{bound}")).map_err(|e| CliError::Transport {
        endpoint: requested_endpoint,
        cause: format!("parse bound endpoint: {e}"),
    })?;

    Ok(ServeHandle { inner, endpoint })
}

/// Condense a `ControlPlaneError::Internal(...)` rendering into an
/// operator-facing string that still names the offending address.
/// Preserves the bind address token so the Display output remains
/// actionable (step 05-02 test `serve_run_bind_failure_returns_cli_error`
/// asserts the rendered message contains the occupied port).
fn stripped_server_error(msg: &str) -> String {
    // `ControlPlaneError::Internal` renders as `internal: bind
    // 127.0.0.1:PORT: <os-specific detail>`. Keep the full message —
    // the port is the load-bearing token; OS-specific detail ("Address
    // already in use", "permission denied") is actionable context.
    msg.trim_start_matches("internal: ").to_owned()
}
