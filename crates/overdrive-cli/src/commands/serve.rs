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
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::traits::cgroup_fs::CgroupFs;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_host::RealCgroupFs;
use url::Url;

use crate::http_client::CliError;

/// Test-only env var honoured by [`run_inner`] to override the
/// `RealCgroupFs` probe root. When unset (the production default),
/// the probe runs against `/sys/fs/cgroup` via
/// [`RealCgroupFs::new`]. When set, the probe runs under the supplied
/// path so integration tests can force probe-failure without
/// requiring privileged FS mutations on the real cgroup hierarchy.
///
/// TEST-HOOK-ONLY: this env var has NO documented use outside the
/// CLI integration test suite. It does NOT appear in `--help` and
/// the binary honours it without flagging — same convention as the
/// `OPENTELEMETRY_*` / `OVERDRIVE_CONFIG_DIR` env vars.
const PROBE_ROOT_ENV_VAR: &str = "OVERDRIVE_TEST_PROBE_ROOT";

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
    run_inner(args, None).await
}

/// Test-only sibling of [`run`].
///
/// Starts the server with an injected [`Dataplane`] adapter. Used by
/// integration tests whose subject under test is the CLI / HTTPS /
/// observation-row surface and which therefore inject
/// `Arc::new(SimDataplane::new())` instead of paying the
/// `CAP_NET_ADMIN` / `CAP_BPF` cost of constructing the production
/// `EbpfDataplane`.
///
/// Per architecture.md § 4.7 of
/// `backend-discovery-bridge-service-reachability`. Production
/// callers MUST use [`run`] — that path leaves
/// `ServerConfig.dataplane_override = None`, so production composition
/// goes through the single-cut `EbpfDataplane` per
/// `feedback_single_cut_greenfield_migrations.md`.
pub async fn run_with_dataplane(
    args: ServeArgs,
    dataplane: Arc<dyn Dataplane>,
) -> Result<ServeHandle, CliError> {
    run_inner(args, Some(dataplane)).await
}

async fn run_inner(
    args: ServeArgs,
    dataplane_override: Option<Arc<dyn Dataplane>>,
) -> Result<ServeHandle, CliError> {
    let requested_endpoint = format!("https://{}", args.bind);

    // ADR-0054 § Composition root wiring — Earned-Trust probe.
    //
    // Construct the production cgroupfs adapter and round-trip on the
    // kernel-managed `cgroup.subtree_control` pseudo-file BEFORE the
    // worker subsystem starts. On failure, emit a structured
    // `health.startup.refused` event whose `cause` field carries the
    // typed `ProbeError` Display rendering (NOT collapsed to
    // `Internal(String)` per `.claude/rules/development.md` § "Never
    // flatten a typed error to `Internal(String)` at a composition
    // boundary"), and return `CliError::ProbeRefused { cause }`.
    //
    // The probe runs BEFORE `run_server`, so it executes before
    // `cgroup_preflight` (ADR-0028) AND before any FS write the
    // worker subsystem would issue. When the probe fails, the
    // listener never binds, the convergence loop never spawns, and
    // no on-disk artefacts are created — operators see the typed
    // refusal rather than a downstream Transport failure that would
    // mask the substrate problem.
    //
    // The test-only `OVERDRIVE_TEST_PROBE_ROOT` env var swaps the
    // probe root from `/sys/fs/cgroup` to a caller-supplied path so
    // integration tests can force probe-failure deterministically.
    // Production callers never set the variable; the binary honours
    // it without --help advertisement (same convention as
    // `$OVERDRIVE_CONFIG_DIR`).
    let fs: Arc<dyn CgroupFs> = Arc::new(build_probe_adapter());
    if let Err(probe_err) = fs.probe().await {
        let cause = probe_err.to_string();
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "cgroup_fs.probe",
            cause = %cause,
            adapter = fs.kind(),
            "CgroupFs probe refused; composition root will not start worker subsystem"
        );
        return Err(CliError::ProbeRefused { cause });
    }
    // Same Arc cloned into run_server → ExecDriver::new — probed
    // substrate IS used substrate (Earned Trust per ADR-0054 §
    // Composition root). The CLI constructs `fs` ONCE; the probe
    // succeeded against THIS exact handle, and the worker subsystem
    // downstream calls methods on a clone of THIS same Arc. Threading
    // a different RealCgroupFs instance — even one that happens to
    // share `/sys/fs/cgroup` substrate semantics — would break the
    // invariant: probe-success is a property of the handle that was
    // probed, not of "some handle to the same substrate".

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
        dataplane_override,
        ..Default::default()
    };
    let inner = run_server(config, fs.clone()).await.map_err(|e| {
        // ADR-0035 §5 + reconciler-memory-redb step 01-06: any
        // `ViewStore` boot-time failure (open RedbViewStore, probe,
        // bulk_load) surfaces as the typed
        // `ControlPlaneError::ViewStoreBoot` variant. Emit a
        // structured `health.startup.refused` event by branching on
        // the variant before mapping to `CliError::Transport` —
        // matches on the type, not on `Display` output, so a future
        // rewording of the error message cannot silently break this
        // observability hook.
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

/// Construct the production [`RealCgroupFs`] for the Earned-Trust
/// probe, honouring the test-only [`PROBE_ROOT_ENV_VAR`] override.
/// Production callers leave the env var unset and the probe runs
/// against `/sys/fs/cgroup`.
fn build_probe_adapter() -> RealCgroupFs {
    match std::env::var(PROBE_ROOT_ENV_VAR) {
        Ok(path) if !path.is_empty() => RealCgroupFs::new().with_probe_root(PathBuf::from(path)),
        _ => RealCgroupFs::new(),
    }
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
