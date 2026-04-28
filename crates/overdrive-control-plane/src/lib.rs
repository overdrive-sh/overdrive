//! Overdrive Phase 1 single-mode control-plane.
//!
//! This crate composes the intent-side `LocalIntentStore`, the observation-side
//! `LocalObservationStore` (Phase 1 production impl per ADR-0012, revised
//! 2026-04-24), the `axum` + `rustls` HTTP server (ADR-0008), the `rcgen`-minted ephemeral
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
//! | `observation_wiring` | `LocalObservationStore` single-node wiring (ADR-0012, revised 2026-04-24) |

// Per ADR-0028, this crate's `cgroup_preflight` and `cgroup_manager`
// modules call `libc::geteuid` / `libc::getpid` directly under
// `#[cfg(target_os = "linux")]`. Both are thin syscall wrappers with
// no preconditions, but they are `extern "C"` and therefore require
// an `unsafe` block. We `deny(unsafe_code)` workspace-wide and
// `#[allow(unsafe_code)]` scope-locally on the two call sites that
// need it; switching from `forbid` to `deny` is what enables the
// scoped allow. Every other module in this crate stays unsafe-free.
#![deny(unsafe_code)]

pub mod action_shim;
pub mod api;
pub mod cgroup_manager;
pub mod cgroup_preflight;
pub mod error;
pub mod eval_broker;
pub mod handlers;
pub mod libsql_provisioner;
pub mod observation_wiring;
pub mod reconciler_runtime;
pub mod tls_bootstrap;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalIntentStore;

/// Shared application state passed to every axum handler via
/// [`axum::extract::State`]. Cheap to clone — the inner handles are
/// `Arc`-shared.
///
/// * `store` — the authoritative [`IntentStore`] implementation
///   (`LocalIntentStore` in Phase 1 single mode).
/// * `obs` — the `ObservationStore` trait object. Phase 1 uses
///   `LocalObservationStore` (redb-backed, ADR-0012 revised 2026-04-24);
///   Phase 2 swaps in `CorrosionStore` via a single trait-object replacement.
///
/// [`IntentStore`]: overdrive_core::traits::intent_store::IntentStore
#[derive(Clone)]
pub struct AppState {
    /// Authoritative intent store — every write lands here.
    pub store: Arc<LocalIntentStore>,
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
    /// Production `Driver` impl per ADR-0022 (amended by ADR-0029):
    /// the action shim's reference to the workload driver. In Phase
    /// 1 single-mode this is `Arc<ProcessDriver>` from
    /// `overdrive-worker`; under DST tests it is `Arc<SimDriver>`.
    /// SCAFFOLD: true — every test caller (`run_server_with_obs`)
    /// is mechanically migrated by DELIVER to pass an
    /// `Arc<SimDriver>` value.
    pub driver: Arc<dyn Driver>,
    /// Per-`(ReconcilerName, TargetResource)` View cache. Phase 1
    /// reconcilers carry no libSQL memory yet (per-primitive libSQL
    /// is Phase 2+), but `JobLifecycle`'s `restart_counts` /
    /// `next_attempt_at` MUST persist across ticks for backoff
    /// exhaustion to be observable. The runtime tick loop reads the
    /// hydrated view from this cache before each `reconcile` call
    /// and writes back the returned `next_view`. Boxed-Any so a
    /// single map handles every reconciler kind's `View` type.
    ///
    /// Phase 2+: replaced by per-primitive libSQL diff-and-persist
    /// per ADR-0013 §2b.
    pub view_cache: Arc<Mutex<BTreeMap<(String, String), CachedView>>>,
}

/// A View persisted across ticks in [`AppState::view_cache`].
///
/// Phase 1 only stores the `JobLifecycle` view because no other
/// reconciler carries non-`()` memory. Adding a new reconciler with
/// non-trivial memory means adding a variant here.
#[derive(Debug, Clone)]
pub enum CachedView {
    /// Unit view — no actual memory. Stored for completeness so the
    /// cache is queried uniformly by reconciler.
    Unit,
    /// `JobLifecycle::View`.
    JobLifecycle(overdrive_core::reconciler::JobLifecycleView),
}

impl AppState {
    /// Build an `AppState` with a fresh empty view cache. Used by
    /// every test fixture and the production boot path.
    #[must_use]
    pub fn new(
        store: Arc<LocalIntentStore>,
        obs: Arc<dyn ObservationStore>,
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        driver: Arc<dyn Driver>,
    ) -> Self {
        Self { store, obs, runtime, driver, view_cache: Arc::new(Mutex::new(BTreeMap::new())) }
    }
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
    /// Storage root for the redb file (`<data_dir>/intent.redb`) and
    /// per-primitive libSQL files (`<data_dir>/reconciler-memory/...`).
    /// Per ADR-0013 §5 this is XDG `data_dir()/overdrive` in production.
    /// The operator trust triple does NOT live here — see
    /// [`Self::operator_config_dir`].
    pub data_dir: PathBuf,
    /// Operator-config base directory. The trust triple is written to
    /// `<operator_config_dir>/.overdrive/config` so the operator CLI
    /// reads the same file the server writes. Per whitepaper §8 and
    /// ADR-0019 this is `$HOME/.overdrive` (or
    /// `$OVERDRIVE_CONFIG_DIR`) in production. Decoupled from
    /// [`Self::data_dir`] per `fix-cli-cannot-reach-control-plane`:
    /// the data dir is a storage root; the operator config dir is an
    /// identity-artefact root, and conflating the two left the CLI
    /// pinning a stale CA on the production-default path.
    pub operator_config_dir: PathBuf,
    /// Per ADR-0028: when `true`, the server skips the cgroup v2
    /// delegation pre-flight, does NOT enrol its own PID into
    /// `overdrive.slice/control-plane.slice/`, and downstream
    /// drivers skip workload-cgroup operations. Logs a WARNING
    /// banner on boot. Default: `false` (production safe default).
    /// Set via `overdrive serve --allow-no-cgroups` (dev only).
    pub allow_no_cgroups: bool,
}

impl Default for ServerConfig {
    /// Production-safe default — `allow_no_cgroups: false` so a caller
    /// who builds a `ServerConfig` via `..Default::default()` does not
    /// accidentally bypass the pre-flight. `bind`, `data_dir`, and
    /// `operator_config_dir` get sentinel values that callers MUST
    /// override; the `Default` impl exists exclusively to make
    /// `..Default::default()` rest-pattern construction ergonomic for
    /// test fixtures that override the three required fields.
    fn default() -> Self {
        // 127.0.0.1:0 — IPv4 loopback, ephemeral port. Constructed
        // directly rather than via `parse()` so the `Default` impl
        // is infallible and clippy's `expect_used` lint stays clean.
        let loopback = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
        Self {
            bind: loopback,
            data_dir: PathBuf::new(),
            operator_config_dir: PathBuf::new(),
            allow_no_cgroups: false,
        }
    }
}

/// Handle to a running control-plane server.
///
/// Drop does NOT stop the server; call [`ServerHandle::shutdown`] to
/// drain in-flight requests and close the listener. The server task
/// runs until the handle is shut down or the process exits.
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

/// Start the control-plane server.
///
/// Mints a fresh ephemeral CA, writes the trust triple under
/// `<operator_config_dir>/.overdrive/config`, builds the
/// `rustls::ServerConfig` (HTTP/2 + HTTP/1.1 via ALPN), binds a TCP
/// listener on [`ServerConfig::bind`], and spawns the `axum_server`
/// serving task. Returns once the listener is bound — callers can
/// observe the actually-bound address via [`ServerHandle::local_addr`].
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the CA mint, TLS config
/// load, trust-triple write, or TCP bind fails. The server task itself
/// runs in the background; its errors are observable only via
/// [`ServerHandle::shutdown`] which awaits the task.
pub async fn run_server(config: ServerConfig) -> Result<ServerHandle, error::ControlPlaneError> {
    // Wire the Phase 1 observation store (`LocalObservationStore`
    // single-node per ADR-0012, revised 2026-04-24) internally and the
    // production `ProcessDriver` from the worker subsystem (ADR-0029),
    // then delegate to `run_server_with_obs_and_driver`. The split
    // exists so integration tests can hold a shared `Arc<dyn ObservationStore>`
    // handle for the canary-injection Fixture-Theater defence without
    // introducing a test-only hook into the production boot path.
    //
    // Per ADR-0029, this is the binary-composition boundary. The CLI's
    // `serve` subcommand may also call `run_server_with_obs_and_driver`
    // directly when it needs a non-default driver under tests.
    let obs: Arc<dyn ObservationStore> =
        Arc::from(observation_wiring::wire_single_node_observation(&config.data_dir)?);

    // Production default — `ProcessDriver` rooted at `/sys/fs/cgroup`.
    // Per ADR-0028, when `--allow-no-cgroups` is set we still construct
    // a ProcessDriver but flip its `allow_no_cgroups` flag so
    // `Driver::start` skips cgroup scope creation / PID placement /
    // limit writes; workloads run as ordinary child processes under
    // the running UID.
    let driver: Arc<dyn Driver> = Arc::new(
        overdrive_worker::ProcessDriver::new(std::path::PathBuf::from("/sys/fs/cgroup"))
            .with_allow_no_cgroups(config.allow_no_cgroups),
    );

    run_server_with_obs_and_driver(config, obs, driver).await
}

/// Start the control-plane server with caller-supplied observation
/// store and driver.
///
/// Per ADR-0022 (amended by ADR-0029), the binary owns the
/// composition: the CLI's `serve` subcommand instantiates
/// `Arc<ProcessDriver>` (Linux production) or `Arc<SimDriver>`
/// (non-Linux dev host) and threads it through this function.
/// Test callers pass `Arc::new(SimDriver::new(DriverType::Process))`.
///
/// Used by integration tests that need to retain a handle to the
/// observation store the server is reading from.
// `async` is kept to preserve the public-API shape: every caller
// invokes `run_server_with_obs_and_driver(...).await`, and the function
// may grow real `.await` points as the boot sequence evolves
// (observation provisioning, lifecycle handshakes). Removing it now
// would churn every call site for no functional gain.
#[allow(clippy::unused_async)]
pub async fn run_server_with_obs_and_driver(
    config: ServerConfig,
    obs: Arc<dyn ObservationStore>,
    driver: Arc<dyn Driver>,
) -> Result<ServerHandle, error::ControlPlaneError> {
    // Per ADR-0028, run the cgroup v2 delegation pre-flight at the
    // start of the boot path — BEFORE any on-disk side effects (no CA
    // mint, no IntentStore open, no listener bind). On failure, the
    // server refuses to start and produces no on-disk artefacts.
    //
    // Skip when:
    //   - `--allow-no-cgroups` was set (dev escape hatch); a WARNING
    //     banner names the disposition.
    //   - The host is not Linux (cgroup v2 is Linux-only by design;
    //     macOS / Windows dev hosts get a no-op).
    if config.allow_no_cgroups {
        tracing::warn!(
            target: "overdrive::cgroup",
            "!!! WARNING: --allow-no-cgroups set. Workloads run without cgroup \
             isolation; the control plane is not protected from workload CPU \
             bursts. Development use only — production deployments require \
             cgroup v2 delegation. !!!"
        );
    } else {
        #[cfg(target_os = "linux")]
        {
            cgroup_preflight::run_preflight().map_err(error::ControlPlaneError::from)?;
            cgroup_manager::create_and_enrol_control_plane_slice()
                .map_err(|e| error::ControlPlaneError::internal("create control-plane slice", e))?;
        }
    }

    // Install the rustls process-wide CryptoProvider (ring) exactly
    // once. The workspace enables only the `ring` feature, but rustls
    // still requires an explicit install when neither provider is the
    // sole compiled-in backend. Ignore the result: if the provider has
    // already been installed (e.g. a prior test in the same process),
    // that is a no-op success for our purposes.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Mint ephemeral CA + leafs per ADR-0010. The trust triple is
    // written AFTER `TcpListener::bind` so the recorded endpoint
    // names the resolved port (not the requested `config.bind`,
    // which may be `:0` under tests and dev flows).
    let material = tls_bootstrap::mint_ephemeral_ca()?;

    // Build the rustls::ServerConfig with ALPN h2/http1.1.
    let rustls_config = tls_bootstrap::load_server_tls_config(&material)?;
    let axum_rustls = RustlsConfig::from_config(Arc::new(rustls_config));

    // Open the authoritative intent store at <data_dir>/intent.redb.
    // `LocalIntentStore::open` creates the parent directory if missing,
    // so the boot path does not depend on caller ordering or a sibling
    // store's directory-creation side effect to satisfy this open.
    let store_path = config.data_dir.join("intent.redb");
    let store = Arc::new(
        LocalIntentStore::open(&store_path)
            .map_err(|e| error::ControlPlaneError::internal("open LocalIntentStore", e))?,
    );

    // Construct the reconciler runtime and register both Phase 1
    // reconcilers at boot: `noop-heartbeat` (proof-of-life,
    // ADR-0013 §9) and `job-lifecycle` (the first real reconciler,
    // US-03). Step 04-04 wired noop-heartbeat; step 02-02 adds
    // job-lifecycle alongside.
    let mut runtime = reconciler_runtime::ReconcilerRuntime::new(&config.data_dir)?;
    runtime.register(noop_heartbeat())?;
    runtime.register(job_lifecycle())?;
    let runtime = Arc::new(runtime);

    let state: AppState = AppState::new(store, obs, runtime, driver);

    // Assemble the router. Step 03-03 wires the real `alloc_status` and
    // `node_list` observation-read handlers; step 03-05 aligned the
    // `cluster_status` handler signature; step 05-03 wires it onto the
    // real route (previously a `stub` placeholder).
    let router = Router::new()
        .route("/v1/jobs", post(handlers::submit_job))
        .route("/v1/jobs/:id", get(handlers::describe_job))
        .route("/v1/jobs/:id/stop", post(handlers::stop_job))
        .route("/v1/allocs", get(handlers::alloc_status))
        .route("/v1/nodes", get(handlers::node_list))
        .route("/v1/cluster/info", get(handlers::cluster_status))
        .with_state(state);

    // Bind the listener synchronously so we can surface bind errors
    // before spawning the serve task.
    let std_listener = std::net::TcpListener::bind(config.bind)
        .map_err(|e| error::ControlPlaneError::internal(format!("bind {}", config.bind), e))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| error::ControlPlaneError::internal("set_nonblocking", e))?;

    // Write the trust triple using the RESOLVED listener address so
    // clients (tests, the CLI) load a config whose `endpoint` names
    // the actual bound port. Deferred until after bind: a failure
    // before this point leaves no stale config on disk.
    //
    // The triple goes under `operator_config_dir`, NOT `data_dir`:
    // `data_dir` is the storage root for redb + libSQL (ADR-0013 §5);
    // `operator_config_dir` is the operator-CLI read site
    // (whitepaper §8, ADR-0019). Pre-fix this used `config.data_dir`
    // and the resulting trust triple landed at
    // `<data_dir>/.overdrive/config`, which the CLI never read —
    // the production-default path was broken
    // (`fix-cli-cannot-reach-control-plane`).
    let bound = std_listener
        .local_addr()
        .map_err(|e| error::ControlPlaneError::internal("local_addr", e))?;
    let endpoint = format!("https://{bound}");
    tls_bootstrap::write_trust_triple(&config.operator_config_dir, &endpoint, &material)?;

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
///
/// Returns `AnyReconciler::NoopHeartbeat(NoopHeartbeat)` per the 04-07
/// migration — `Box<dyn Reconciler>` is no longer object-safe under
/// the trait's new `type View` + `async fn hydrate` shape.
#[must_use]
pub fn noop_heartbeat() -> overdrive_core::reconciler::AnyReconciler {
    use overdrive_core::reconciler::{AnyReconciler, NoopHeartbeat};

    AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical())
}

/// Construct the `job-lifecycle` reconciler.
///
/// The first real (non-proof-of-life) reconciler. Converges declared
/// replica count for a `Job` against the running `AllocStatusRow`
/// set, calling inline first-fit placement equivalent to
/// `overdrive_scheduler::schedule`.
///
/// Per US-03 (Slice 3 of phase-1-first-workload), this is registered
/// at boot alongside `noop-heartbeat`.
#[must_use]
pub fn job_lifecycle() -> overdrive_core::reconciler::AnyReconciler {
    use overdrive_core::reconciler::{AnyReconciler, JobLifecycle};

    AnyReconciler::JobLifecycle(JobLifecycle::canonical())
}
