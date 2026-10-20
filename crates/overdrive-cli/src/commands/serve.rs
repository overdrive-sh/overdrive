//! `overdrive serve` — start the Phase 1 control-plane server.
//!
//! Wraps `overdrive_control_plane::run_server` into the CLI-facing shape:
//! the handler returns a [`ServeHandle`] whose `endpoint()` names the
//! actually-bound address (ephemeral port in tests) and whose
//! `shutdown()` drains in-flight connections before closing the listener.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, this is a plain `async fn` that
//! tests call directly. What happens after boot — the internal shutdown
//! request versus operator signals, the fail-stop outer bound, and the exit
//! status — is owned by [`crate::commands::serve_lifetime`], which `main.rs`
//! runs over the returned [`ServeHandle`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::node_adapters::NodeAdapters;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::guest_network::ServeShutdownRequest;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::traits::cgroup_fs::CgroupFs;
use overdrive_host::RealCgroupFs;
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
    /// The node-global host vsock listener ports (PORTS-CONFIG, ADR-0218),
    /// copied into `ServerConfig.vsock_ports`. The `overdrive` binary passes
    /// `NodeVsockPortConfig::default()`; a test passes
    /// `NodeVsockPortConfig::kernel_assigned()` and reads the bound ports
    /// back from the server.
    pub vsock_ports: NodeVsockPortConfig,
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

    /// Wait for the retained shared guest-network owner to request shutdown.
    pub async fn shutdown_requested(&mut self) -> ServeShutdownRequest {
        self.inner.shutdown_requested().await
    }

    /// Trigger graceful shutdown. In-flight requests complete within a
    /// 5-second deadline; new connections are refused immediately.
    /// Awaits the server task to completion.
    ///
    /// # Errors
    ///
    /// Returns the typed, one-shot mTLS owner result when authoritative
    /// userspace teardown does not converge. No public retry capability exists.
    pub async fn shutdown(self) -> Result<(), CliError> {
        self.inner
            .shutdown(DEFAULT_DRAIN_DEADLINE)
            .await
            .map_err(|source| CliError::ServerShutdown { source })
    }

    /// Killed mode of the serve lifetime port
    /// ([`crate::commands::serve_lifetime::ServeSignal::Kill`]): abandon the
    /// in-process `serve` owner without graceful drain or workload cleanup so
    /// a test can boot again against the unchanged durable directories. The
    /// lifetime owner is the only caller; see
    /// [`overdrive_control_plane::ServerHandle::kill_for_test`] for exactly
    /// which in-process cleanup still runs.
    #[cfg(feature = "integration-tests")]
    pub(crate) async fn kill_for_test(self) {
        let overdrive_control_plane::AbruptServerResidue = self.inner.kill_for_test().await;
    }
}

/// Start the Phase 1 control-plane server over the host adapters
/// (`NodeAdapters::host` with the production `SystemdCredsKeyring` KEK,
/// ADR-0063). A call of [`run_with_adapters`].
///
/// # Errors
///
/// As [`run_with_adapters`].
#[expect(
    clippy::result_large_err,
    reason = "the bundle builder returns the boot's typed `ControlPlaneError` unboxed, as `run_server` does"
)]
pub async fn run(args: ServeArgs) -> Result<ServeHandle, CliError> {
    run_with_adapters(args, Arc::new(RealCgroupFs::new()), |config, fs| {
        NodeAdapters::host(config, fs, Arc::new(overdrive_host::ca::SystemdCredsKeyring::new()))
    })
    .await
}

/// Start the control-plane server over the adapters `adapters` builds
/// (NO-OVERRIDE-SEAMS, PROBE-ROOT-ENV; ADR-0219).
///
/// Builds the `ServerConfig` from `args` (`bind`, `data_dir`,
/// `operator_config_dir` = `config_dir`, `vsock_ports`, the rest
/// `ServerConfig::new()`), then, in this order:
/// 1. probes `cgroup_fs` (ADR-0054); on `Err` emits `health.startup.refused`
///    (reason `cgroup_fs.probe`) and returns `CliError::ProbeRefused` —
///    `adapters` is never called, so a refused probe opens no store;
/// 2. calls `adapters(&config, cgroup_fs)` — the probed handle is the one
///    the bundle carries;
/// 3. `run_server(config, bundle)`.
///
/// # Errors
///
/// [`CliError::ProbeRefused`] when the cgroupfs probe fails;
/// [`CliError::Transport`] for every error of step 2 or 3 (a `ViewStoreBoot`
/// error emits `health.startup.refused` first).
pub async fn run_with_adapters<F>(
    args: ServeArgs,
    cgroup_fs: Arc<dyn CgroupFs>,
    adapters: F,
) -> Result<ServeHandle, CliError>
where
    F: FnOnce(&ServerConfig, Arc<dyn CgroupFs>) -> Result<NodeAdapters, ControlPlaneError>,
{
    let requested_endpoint = format!("https://{}", args.bind);

    // ADR-0054 § Composition root wiring — Earned-Trust probe, BEFORE the
    // bundle is built (so a refused probe opens no store) and before
    // `cgroup_preflight` (ADR-0028). The probed handle is the used handle.
    if let Err(probe_err) = cgroup_fs.probe().await {
        let cause = probe_err.to_string();
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "cgroup_fs.probe",
            cause = %cause,
            adapter = cgroup_fs.kind(),
            "CgroupFs probe refused; composition root will not start worker subsystem"
        );
        return Err(CliError::ProbeRefused { cause });
    }

    let config = ServerConfig {
        bind: args.bind,
        data_dir: args.data_dir,
        operator_config_dir: args.config_dir,
        vsock_ports: args.vsock_ports,
        ..ServerConfig::new()
    };
    let map_boot_error = |e: ControlPlaneError| {
        // ADR-0035 §5: a `ViewStore` boot failure surfaces as the typed
        // `ControlPlaneError::ViewStoreBoot`; branch on the variant, not on
        // `Display`, before mapping to `CliError::Transport`.
        if matches!(e, ControlPlaneError::ViewStoreBoot(_)) {
            tracing::error!(
                target: "overdrive::health",
                event = "health.startup.refused",
                cause = %e,
                "ViewStore boot failed; refusing to start"
            );
        }
        CliError::Transport {
            endpoint: requested_endpoint.clone(),
            cause: stripped_server_error(&e.to_string()),
        }
    };
    let bundle = adapters(&config, cgroup_fs).map_err(map_boot_error)?;
    let inner = run_server(config, bundle).await.map_err(map_boot_error)?;

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
