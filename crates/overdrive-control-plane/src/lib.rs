//! Overdrive Phase 1 single-mode control-plane.
//!
//! This crate composes the intent-side `LocalStore`, the observation-side
//! `SimObservationStore` (Phase 1 production impl per ADR-0012), the
//! `axum` + `rustls` HTTP server (ADR-0008), the `rcgen`-minted ephemeral
//! CA (ADR-0010), the reconciler runtime (ADR-0013), and the shared
//! request/response types (ADR-0014) into the `overdrive serve` binary's
//! server loop.
//!
//! Module layout:
//!
//! | Module | Role |
//! |---|---|
//! | `api` | Shared request/response types (serde + utoipa) |
//! | `handlers` | axum route handlers — submit_job, describe_job, cluster_status, alloc_status, node_list |
//! | `error` | `ControlPlaneError` enum + `to_response` mapping (ADR-0015) |
//! | `tls_bootstrap` | Ephemeral CA + trust triple + rustls config (ADR-0010) |
//! | `reconciler_runtime` | `ReconcilerRuntime` + registry (ADR-0013) |
//! | `eval_broker` | `EvaluationBroker` + cancelable-eval-set (ADR-0013) |
//! | `libsql_provisioner` | Per-primitive libSQL path derivation (ADR-0013) |
//! | `observation_wiring` | `SimObservationStore` single-node wiring (ADR-0012) |

#![forbid(unsafe_code)]

pub mod api;
pub mod error;
pub mod eval_broker;
pub mod handlers;
pub mod libsql_provisioner;
pub mod observation_wiring;
pub mod reconciler_runtime;
pub mod tls_bootstrap;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalStore;

/// Shared application state passed to every axum handler via
/// [`axum::extract::State`]. Cheap to clone — the inner handles are
/// `Arc`-shared.
///
/// * `store` — the authoritative [`IntentStore`] implementation
///   (`LocalStore` in Phase 1 single mode).
/// * `obs` — the `ObservationStore` trait object. Phase 1 wraps
///   `SimObservationStore` (ADR-0012); Phase 2 swaps in `CorrosionStore`
///   via a single trait-object replacement.
///
/// [`IntentStore`]: overdrive_core::traits::intent_store::IntentStore
#[derive(Clone)]
pub struct AppState {
    /// Authoritative intent store — every write lands here.
    pub store: Arc<LocalStore>,
    /// Eventually-consistent observation store. Unused by 03-01's
    /// `submit_job` handler, but wired in so observation-reading
    /// handlers in later steps (03-03) can pick it up without
    /// restructuring the state shape.
    pub obs: Arc<dyn ObservationStore>,
    /// Reconciler runtime — registry of `Reconciler` trait objects
    /// and the `EvaluationBroker`. Step 04-04 threads this through
    /// `AppState` so the `cluster_status` handler can render the
    /// registry and broker counters without a side channel.
    pub runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
}

/// Configuration for the Phase 1 control-plane server. Populated at
/// startup from CLI flags and environment.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Socket address to bind the HTTPS listener. Default
    /// `127.0.0.1:7001` per ADR-0008. Use `127.0.0.1:0` in tests to
    /// request an ephemeral port; the bound port is observable via
    /// [`ServerHandle::local_addr`].
    pub bind: SocketAddr,
    /// Data directory — parent of the redb file, per-primitive libSQL
    /// files, and the trust triple config file.
    pub data_dir: PathBuf,
}

/// Handle to a running control-plane server. Drop does NOT stop the
/// server; call [`ServerHandle::shutdown`] to drain in-flight requests
/// and close the listener. The server task runs until the handle is
/// shut down or the process exits.
#[derive(Debug)]
pub struct ServerHandle {
    inner: AxumHandle,
    server_task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl ServerHandle {
    /// Return the socket address the server is actually listening on.
    /// When [`ServerConfig::bind`] specified port 0, this reveals the
    /// ephemeral port the OS chose. Awaits the server's "listening"
    /// notification; resolves as soon as the listener is bound.
    pub async fn local_addr(&self) -> Option<SocketAddr> {
        self.inner.listening().await
    }

    /// Trigger graceful shutdown with a drain deadline. In-flight
    /// requests complete; new connections are refused; the listener is
    /// dropped. Awaits the server task to completion.
    pub async fn shutdown(self, drain_deadline: Duration) {
        self.inner.graceful_shutdown(Some(drain_deadline));
        // The server task returns once the listener closes and in-flight
        // connections drain. We ignore the inner result here — this is
        // the shutdown path; test-level assertions on server outcome
        // happen before shutdown is called.
        let _ = self.server_task.await;
    }
}

/// Start the control-plane server. Mints a fresh ephemeral CA, writes
/// the trust triple under `<data_dir>/.overdrive/config`, builds the
/// `rustls::ServerConfig` (HTTP/2 + HTTP/1.1 via ALPN), binds a TCP
/// listener on [`ServerConfig::bind`], and spawns the `axum_server`
/// serving task. Returns once the listener is bound — callers can
/// observe the actually-bound address via
/// [`ServerHandle::local_addr`].
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the CA mint, TLS config
/// load, trust-triple write, or TCP bind fails. The server task itself
/// runs in the background; its errors are observable only via
/// [`ServerHandle::shutdown`] which awaits the task.
pub async fn run_server(config: ServerConfig) -> Result<ServerHandle, error::ControlPlaneError> {
    // Wire the Phase 1 observation store (`SimObservationStore`
    // single-peer per ADR-0012) internally, then delegate to
    // `run_server_with_obs`. The split exists so integration tests can
    // hold a shared `Arc<dyn ObservationStore>` handle — needed for the
    // 03-03 canary-injection Fixture-Theater defence — without
    // introducing a test-only hook into the production boot path.
    let obs: Arc<dyn ObservationStore> =
        Arc::from(observation_wiring::wire_single_node_observation(&config.data_dir)?);
    run_server_with_obs(config, obs).await
}

/// Start the control-plane server with a caller-supplied observation
/// store. Used by integration tests that need to retain a handle to
/// the observation store the server is reading from; the production
/// boot path calls [`run_server`], which wires the Phase 1 default.
pub async fn run_server_with_obs(
    config: ServerConfig,
    obs: Arc<dyn ObservationStore>,
) -> Result<ServerHandle, error::ControlPlaneError> {
    // Install the rustls process-wide CryptoProvider (ring) exactly
    // once. The workspace enables only the `ring` feature, but rustls
    // still requires an explicit install when neither provider is the
    // sole compiled-in backend. Ignore the result: if the provider has
    // already been installed (e.g. a prior test in the same process),
    // that is a no-op success for our purposes.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Mint ephemeral CA + leafs per ADR-0010.
    let material = tls_bootstrap::mint_ephemeral_ca()?;

    // Write the trust triple so clients (tests, the CLI) can load the
    // CA from a stable location. Endpoint is recorded even though the
    // test binds ephemeral — consumers use the field but must ignore
    // the port if they obtained a different one out-of-band.
    let endpoint = format!("https://{}", config.bind);
    tls_bootstrap::write_trust_triple(&config.data_dir, &endpoint, &material)?;

    // Build the rustls::ServerConfig with ALPN h2/http1.1.
    let rustls_config = tls_bootstrap::load_server_tls_config(&material)?;
    let axum_rustls = RustlsConfig::from_config(Arc::new(rustls_config));

    // Open the authoritative intent store at <data_dir>/intent.redb.
    // The parent directory is guaranteed to exist — callers pass a
    // tempdir or an operator-created data directory; we do not create
    // the directory ourselves here per `LocalStore::open`'s contract.
    let store_path = config.data_dir.join("intent.redb");
    let store = Arc::new(
        LocalStore::open(&store_path)
            .map_err(|e| error::ControlPlaneError::Internal(format!("open LocalStore: {e}")))?,
    );

    // Construct the reconciler runtime and register `noop_heartbeat` at
    // boot per ADR-0013 §9 — Phase 1's proof-of-life. Step 04-04 wires
    // this here so every server boot establishes the
    // `AtLeastOneReconcilerRegistered` invariant; step 05-05's walking
    // skeleton will assert it end-to-end.
    let mut runtime = reconciler_runtime::ReconcilerRuntime::new(&config.data_dir)?;
    runtime.register(noop_heartbeat())?;
    let runtime = Arc::new(runtime);

    let state = AppState { store, obs, runtime };

    // Assemble the router. Step 03-03 wires the real `alloc_status` and
    // `node_list` observation-read handlers; step 03-05 aligned the
    // `cluster_status` handler signature; step 05-03 wires it onto the
    // real route (previously a `stub` placeholder).
    let router = Router::new()
        .route("/v1/jobs", post(handlers::submit_job))
        .route("/v1/jobs/:id", get(handlers::describe_job))
        .route("/v1/allocs", get(handlers::alloc_status))
        .route("/v1/nodes", get(handlers::node_list))
        .route("/v1/cluster/info", get(handlers::cluster_status))
        .with_state(state);

    // Bind the listener synchronously so we can surface bind errors
    // before spawning the serve task.
    let std_listener = std::net::TcpListener::bind(config.bind)
        .map_err(|e| error::ControlPlaneError::Internal(format!("bind {}: {e}", config.bind)))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| error::ControlPlaneError::Internal(format!("set_nonblocking: {e}")))?;

    let axum_handle = AxumHandle::new();
    let server =
        axum_server::from_tcp_rustls(std_listener, axum_rustls).handle(axum_handle.clone());

    let server_task = tokio::spawn(async move { server.serve(router.into_make_service()).await });

    Ok(ServerHandle { inner: axum_handle, server_task })
}

/// Construct the `noop-heartbeat` reconciler. Exposed as a public
/// factory so the DST harness and the server boot path register the
/// same canonical instance.
///
/// Per ADR-0013 §9, `noop-heartbeat` is Phase 1's proof-of-life
/// reconciler: its `reconcile` returns `vec![Action::Noop]`
/// deterministically, serving as the fixture against which the
/// `ReconcilerIsPure` invariant's twin-invocation check runs and as
/// the seed entry for the `AtLeastOneReconcilerRegistered` invariant.
#[must_use]
pub fn noop_heartbeat() -> Box<dyn overdrive_core::reconciler::Reconciler> {
    use overdrive_core::reconciler::{Action, Db, Reconciler, ReconcilerName, State};

    struct NoopHeartbeat {
        name: ReconcilerName,
    }

    impl Reconciler for NoopHeartbeat {
        fn name(&self) -> &ReconcilerName {
            &self.name
        }

        fn reconcile(&self, _desired: &State, _actual: &State, _db: &Db) -> Vec<Action> {
            vec![Action::Noop]
        }
    }

    Box::new(NoopHeartbeat {
        name: ReconcilerName::new("noop-heartbeat")
            .expect("'noop-heartbeat' is a valid ReconcilerName by construction"),
    })
}
