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
//! | `reconciler_runtime` | `ReconcilerRuntime` + registry (ADR-0013/ADR-0035) |
//! | `eval_broker` | `EvaluationBroker` + cancelable-eval-set (ADR-0013) |
//! | `view_store` | Runtime-owned `ViewStore` port + `RedbViewStore` (ADR-0035) |
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
// Phase 2.2 service-hydration shim scaffold per
// `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
// DWD-3 / DWD-5. DELIVER's Slice 08 first GREEN commit moves this
// to the canonical path
// `action_shim/service_hydration.rs` (architecture.md § 9). The
// sibling shape here exists so the scaffold compiles before the
// directory-module conversion; the rename is non-substantive
// once DELIVER lands the dispatch body.
pub mod action_shim_service_hydration;
pub mod api;
pub mod cgroup_manager;
pub mod cgroup_preflight;
pub mod error;
pub mod eval_broker;
pub mod handlers;
pub mod observation_wiring;
pub mod reconciler_runtime;
// Phase 2.2 reconcilers per DWD-3. Currently hosts only the
// `service_map_hydrator`; future Phase 2+ reconcilers will land
// alongside.
pub mod reconcilers;
pub mod streaming;
pub mod tls_bootstrap;
// reconciler-memory-redb step 01-03 — `ViewStore` port + error types
// per ADR-0035 §2. Wired into `ReconcilerRuntime` in step 01-06.
pub mod view_store;
pub mod worker;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalIntentStore;
use tokio_util::sync::CancellationToken;

use crate::reconciler_runtime::{DEFAULT_TICK_CADENCE, run_convergence_tick};

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
    /// 1 single-mode this is `Arc<ExecDriver>` from
    /// `overdrive-worker`; under DST tests it is `Arc<SimDriver>`.
    /// SCAFFOLD: true — every test caller (`run_server_with_obs`)
    /// is mechanically migrated by DELIVER to pass an
    /// `Arc<SimDriver>` value.
    pub driver: Arc<dyn Driver>,
    /// Broadcast channel for `LifecycleEvent`s emitted by the action
    /// shim after every successful `obs.write()`. Per architecture.md
    /// §10 (cli-submit-vs-deploy-and-alloc-status DESIGN): this is
    /// the bus the slice 02 NDJSON streaming handler subscribes to;
    /// the channel is `tokio::sync::broadcast` so multiple
    /// concurrent `submit --watch` requests share a single emit.
    pub lifecycle_events: Arc<tokio::sync::broadcast::Sender<crate::action_shim::LifecycleEvent>>,
    /// Wall-clock cap on streaming `submit --watch` connections —
    /// after this duration, the streaming handler emits a
    /// `Timeout { after_seconds }` terminal event and closes the
    /// stream. Default 60s; configurable via
    /// `[server] streaming_submit_cap_seconds` per architecture.md §10.
    pub streaming_cap: Duration,
    /// Injected `Clock` used by the streaming submit handler for the
    /// cap timer. The dst-lint gate enforces that `tokio::time::sleep`
    /// is never used for this cap — the handler MUST go through
    /// `clock.sleep(cap)` so DST tests can advance time deterministically.
    /// Production wires `Arc::new(SystemClock)` from the `overdrive-host`
    /// crate (the only crate permitted to instantiate `SystemClock`);
    /// tests inject `Arc<SimClock>`.
    pub clock: Arc<dyn Clock>,
}

/// Default capacity for the lifecycle-event broadcast channel.
///
/// Phase 1 has at most one streaming subscriber per request, so 256
/// gives comfortable headroom for transient burstiness without OOM.
/// Lag handling (S-CP-10) is not in scope for this step.
pub const DEFAULT_LIFECYCLE_BROADCAST_CAPACITY: usize = 256;

/// Default wall-clock cap on streaming `submit --watch` connections.
/// Per architecture.md §10. Operators can override via
/// `[server] streaming_submit_cap_seconds`.
pub const DEFAULT_STREAMING_CAP: Duration = Duration::from_secs(60);

impl AppState {
    /// Build an `AppState` with a fresh `LifecycleEvent` broadcast
    /// channel of default capacity. Used by every test fixture and
    /// the production boot path.
    ///
    /// The default `streaming_cap` is 60s per architecture.md §10.
    /// Test fixtures that want a different cap construct `AppState`
    /// directly with the field set.
    ///
    /// The `clock` parameter is required at construction per
    /// `.claude/rules/development.md` § "Port-trait dependencies":
    /// types depending on a port trait take the implementation as an
    /// explicit constructor parameter so tests cannot silently inherit
    /// production wall-clock behaviour by forgetting to override.
    /// Production passes `Arc::new(overdrive_host::SystemClock)`; tests
    /// pass `Arc::new(overdrive_sim::adapters::clock::SimClock::new())`.
    #[must_use]
    pub fn new(
        store: Arc<LocalIntentStore>,
        obs: Arc<dyn ObservationStore>,
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        driver: Arc<dyn Driver>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(DEFAULT_LIFECYCLE_BROADCAST_CAPACITY);
        Self {
            store,
            obs,
            runtime,
            driver,
            lifecycle_events: Arc::new(tx),
            streaming_cap: DEFAULT_STREAMING_CAP,
            clock,
        }
    }
}

/// Configuration for the Phase 1 control-plane server. Populated at
/// startup from CLI flags and environment.
#[derive(Clone)]
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
    /// Cadence between drains of the [`crate::eval_broker::EvaluationBroker`]
    /// in the convergence-loop spawn (see
    /// [`run_server_with_obs_and_driver`]). Default
    /// [`reconciler_runtime::DEFAULT_TICK_CADENCE`] (100ms) per
    /// ADR-0023. Tests inject a slower cadence with a [`SimClock`] to
    /// step through the loop deterministically.
    ///
    /// [`SimClock`]: overdrive_core::traits::clock::Clock
    pub tick_cadence: Duration,
    /// Injected [`Clock`] used by the convergence-loop spawn for the
    /// per-tick `now()` snapshot, the `tick.deadline` budget, and the
    /// `clock.sleep(tick_cadence)` between drains. Production wires
    /// this to `Arc::new(SystemClock)` from the
    /// [`overdrive_host`] crate (the only crate permitted to
    /// instantiate `SystemClock` per CLAUDE.md "Repository
    /// structure"); DST tests inject `Arc<SimClock>` so the harness
    /// controls time.
    pub clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for ServerConfig {
    /// `Arc<dyn Clock>` is not [`Debug`], so the auto-derive on
    /// `ServerConfig` is replaced by a manual impl that elides the
    /// clock field.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("bind", &self.bind)
            .field("data_dir", &self.data_dir)
            .field("operator_config_dir", &self.operator_config_dir)
            .field("tick_cadence", &self.tick_cadence)
            .field("clock", &"<dyn Clock>")
            .finish()
    }
}

impl Default for ServerConfig {
    /// `bind`, `data_dir`, and `operator_config_dir` get sentinel
    /// values that callers MUST override; the `Default` impl exists
    /// exclusively to make `..Default::default()` rest-pattern
    /// construction ergonomic for test fixtures that override the
    /// three required fields.
    ///
    /// `tick_cadence` defaults to [`reconciler_runtime::DEFAULT_TICK_CADENCE`]
    /// (100ms) and `clock` defaults to `Arc::new(SystemClock)` from
    /// the [`overdrive_host`] crate — the only crate permitted to
    /// instantiate `SystemClock` per CLAUDE.md "Repository structure".
    /// Tests that need a controllable clock construct the
    /// `ServerConfig` directly with `clock: Arc::new(SimClock::new())`.
    fn default() -> Self {
        // 127.0.0.1:0 — IPv4 loopback, ephemeral port. Constructed
        // directly rather than via `parse()` so the `Default` impl
        // is infallible and clippy's `expect_used` lint stays clean.
        let loopback = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
        Self {
            bind: loopback,
            data_dir: PathBuf::new(),
            operator_config_dir: PathBuf::new(),
            tick_cadence: DEFAULT_TICK_CADENCE,
            clock: Arc::new(overdrive_host::SystemClock),
        }
    }
}

/// Handle to a running control-plane server.
///
/// Drop does NOT stop the server; call [`ServerHandle::shutdown`] to
/// drain in-flight requests, stop the convergence-loop spawn, and
/// close the listener. The server task runs until the handle is shut
/// down or the process exits.
#[derive(Debug)]
pub struct ServerHandle {
    inner: AxumHandle,
    server_task: tokio::task::JoinHandle<std::io::Result<()>>,
    /// `JoinHandle` for the convergence-tick spawn loop that drains
    /// the `EvaluationBroker` and dispatches actions through the
    /// action shim. See [`run_server_with_obs_and_driver`] for the
    /// spawn site. Per `fix-convergence-loop-not-spawned` Step 01-02.
    convergence_task: tokio::task::JoinHandle<()>,
    /// `JoinHandle` for the `worker::exit_observer` task — consumes
    /// `ExitEvent`s from the `Driver`'s watcher and writes
    /// `AllocStatusRow`s to the `ObservationStore`. Per
    /// `fix-exec-driver-exit-watcher` Step 01-02.
    ///
    /// Shutdown ordering: per RCA §Approved fix item 5 the convergence
    /// task is signalled to drain FIRST, then axum drains, THEN the
    /// observer's `exit_observer_shutdown` token is cancelled so the
    /// observer's `tokio::select!` resolves and the task exits.
    ///
    /// The token-driven shutdown is the fallback path for the case
    /// where a watcher task is still alive at shutdown time (e.g. a
    /// `/bin/sleep` workload that did not reap before convergence was
    /// cancelled, or a `SimDriver`-backed test where `exit_tx` is held
    /// by the test's `Arc<dyn Driver>` until the test fn returns).
    /// Without this, `await exit_observer_task` would block
    /// indefinitely on `rx.recv()`. With it, shutdown is bounded.
    exit_observer_task: tokio::task::JoinHandle<()>,
    /// Token observed by the convergence-tick spawn loop. Cancelled
    /// in [`Self::shutdown`] BEFORE axum graceful so reconciler tasks
    /// holding `Arc<dyn Driver>` references stop driving the driver
    /// before axum begins to tear down `AppState`.
    convergence_shutdown: CancellationToken,
    /// Token observed by the `exit_observer` task's `tokio::select!`
    /// loop. Cancelled in [`Self::shutdown`] AFTER the convergence
    /// task and axum task have drained, so any in-flight `ExitEvent`
    /// driven by an in-flight `Driver::stop` lands in obs before the
    /// observer is told to exit.
    exit_observer_shutdown: CancellationToken,
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
    /// requests complete; new connections are refused; the convergence
    /// loop stops draining the broker; the listener is dropped.
    /// Awaits the server task to completion.
    ///
    /// Ordering — convergence task FIRST, then axum graceful, then
    /// `server_task` join, then exit-observer task last. The
    /// convergence task holds `Arc<dyn Driver>` references; reversing
    /// this ordering risks reconciler tasks driving the driver while
    /// axum is tearing down `AppState`. Per
    /// `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2)
    /// and `fix-exec-driver-exit-watcher` Step 01-02 RCA §Approved
    /// fix item 5 (exit observer drains LAST so any in-flight
    /// `ExitEvent` lands in obs).
    pub async fn shutdown(self, drain_deadline: Duration) {
        // 1. Cancel the convergence loop and await its completion.
        //    The loop's `tokio::select!` resolves the cancellation
        //    branch on the next poll and `break`s; the join here
        //    waits for the active tick (if any) to finish through
        //    `action_shim::dispatch`.
        self.convergence_shutdown.cancel();
        let _ = self.convergence_task.await;

        // 2. Trigger axum graceful shutdown. In-flight requests
        //    complete within `drain_deadline`; new connections are
        //    refused.
        self.inner.graceful_shutdown(Some(drain_deadline));

        // 3. Wait for the axum task to drain and exit. We ignore the
        //    inner result here — this is the shutdown path;
        //    test-level assertions on server outcome happen before
        //    shutdown is called.
        let _ = self.server_task.await;

        // 4. Cancel the observer's shutdown token, then await the
        //    observer task. The observer's `tokio::select!`
        //    biased-resolves the cancellation branch and exits
        //    cleanly even when watcher tasks (production
        //    `ExecDriver` watchers awaiting `child.wait()`) or test
        //    harness `Arc<dyn Driver>` refs still hold `exit_tx`
        //    clones. Without this token, a workload that did not
        //    reap before convergence was cancelled — or a SimDriver
        //    held by the test fn until its scope ends — would keep
        //    `rx.recv()` blocked indefinitely, deadlocking shutdown.
        //    Per `fix-exec-driver-exit-watcher` Step 01-02 follow-up.
        self.exit_observer_shutdown.cancel();
        let _ = self.exit_observer_task.await;
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
    // production `ExecDriver` from the worker subsystem (ADR-0029),
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

    // Production default — `ExecDriver` rooted at `/sys/fs/cgroup`.
    let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new(
        std::path::PathBuf::from("/sys/fs/cgroup"),
        Arc::new(overdrive_host::SystemClock),
    ));

    run_server_with_obs_and_driver(config, obs, driver).await
}

/// Start the control-plane server with caller-supplied observation
/// store and driver.
///
/// Per ADR-0022 (amended by ADR-0029), the binary owns the
/// composition: the CLI's `serve` subcommand instantiates
/// `Arc<ExecDriver>` (Linux production) or `Arc<SimDriver>`
/// (non-Linux dev host) and threads it through this function.
/// Test callers pass `Arc::new(SimDriver::new(DriverType::Exec))`.
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
    // Per ADR-0028 (as superseded in part by ADR-0034), run the cgroup
    // v2 delegation pre-flight at the start of the boot path — BEFORE
    // any on-disk side effects (no CA mint, no IntentStore open, no
    // listener bind). On failure, the server refuses to start and
    // produces no on-disk artefacts.
    //
    // The host is not Linux on macOS / Windows dev hosts; cgroup v2 is
    // Linux-only by design, so the pre-flight is a no-op there. There
    // is no in-binary escape hatch (ADR-0034 deleted it); operators
    // running on macOS / Windows / non-delegated Linux dev boxes use
    // `cargo xtask lima run --` per `.claude/rules/testing.md`.
    #[cfg(target_os = "linux")]
    {
        cgroup_preflight::run_preflight().map_err(error::ControlPlaneError::from)?;
        cgroup_manager::create_and_enrol_control_plane_slice()
            .map_err(|e| error::ControlPlaneError::internal("create control-plane slice", e))?;
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

    // Construct the reconciler runtime against the production
    // `RedbViewStore` (ADR-0035 §4 — one redb file per node at
    // `<data_dir>/reconcilers/memory.redb`) and register both Phase 1
    // reconcilers at boot: `noop-heartbeat` (proof-of-life,
    // ADR-0013 §9) and `job-lifecycle` (the first real reconciler,
    // US-03).
    //
    // Per ADR-0035 §5 each `register` call probes the view store
    // (Earned-Trust handshake) and bulk-loads any persisted
    // `(target, view)` rows into the runtime's in-memory map before
    // the first tick fires. A probe failure short-circuits register
    // with `ControlPlaneError::Internal`; the surrounding `?` surfaces
    // it to the operator via the binary-layer error formatter
    // (`overdrive-cli` logs `health.startup.refused` and exits non-zero).
    let view_store: Arc<dyn view_store::ViewStore> =
        Arc::new(view_store::redb::RedbViewStore::open(&config.data_dir).map_err(|e| {
            error::ViewStoreBootError::Open {
                path: view_store::redb::RedbViewStore::resolve_path(&config.data_dir),
                source: e,
            }
        })?);
    let mut runtime = reconciler_runtime::ReconcilerRuntime::new(&config.data_dir, view_store)?;
    runtime.register(noop_heartbeat()).await?;
    runtime.register(job_lifecycle()).await?;
    let runtime = Arc::new(runtime);

    // Production boot threads the `ServerConfig.clock` into AppState
    // so the streaming submit handler's cap timer uses the same clock
    // as the convergence-loop spawn. The clock is required at
    // construction per `.claude/rules/development.md` § "Port-trait
    // dependencies"; there is no post-construction injection path.
    let state: AppState = AppState::new(store, obs, runtime, driver, config.clock.clone());

    // Spawn the exit-observer subsystem BEFORE the convergence loop so
    // the observer is already draining the driver's `ExitEvent`
    // channel when the first action-shim write happens. The observer
    // shares `state.obs` (so its writes appear in the same row stream
    // every reader consumes) and shares `state.runtime` (so the
    // observer can re-enqueue the job-lifecycle reconciler after
    // each obs write — closes the latency between exit classification
    // and reconciler-driven recovery). Per
    // `fix-exec-driver-exit-watcher` Step 01-02 RCA §Approved fix
    // item 5.
    let exit_observer_shutdown = CancellationToken::new();
    let exit_observer_task = worker::exit_observer::spawn_with_runtime(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        config.clock.clone(),
        Some(state.runtime.clone()),
        exit_observer_shutdown.clone(),
    );

    // Spawn the convergence-tick loop per `fix-convergence-loop-not-
    // spawned` Step 01-02 (RCA Option B2 broker-driven §18 wiring).
    // Each iteration drains the EvaluationBroker, dispatches one
    // `run_convergence_tick` per pending Evaluation, then sleeps
    // `tick_cadence` before re-draining. Cancellation via
    // `convergence_shutdown` is observed in `tokio::select!` between
    // ticks so an in-flight dispatch always completes before exit.
    //
    // Without this spawn, `submit_job` and `stop_job` would only
    // write to the IntentStore — the broker would never be drained,
    // no allocations would ever be scheduled, and
    // `cluster_status.broker.dispatched` would permanently read 0.
    // See `docs/feature/fix-convergence-loop-not-spawned/bugfix-rca.md`
    // for the full root-cause chain.
    let convergence_shutdown = CancellationToken::new();
    let convergence_task = spawn_convergence_loop(
        state.clone(),
        config.clock.clone(),
        config.tick_cadence,
        convergence_shutdown.clone(),
    );

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

    Ok(ServerHandle {
        inner: axum_handle,
        server_task,
        convergence_task,
        exit_observer_task,
        convergence_shutdown,
        exit_observer_shutdown,
    })
}

/// Construct the `noop-heartbeat` reconciler. Exposed as a public
/// factory so the DST harness and the server boot path register the
/// same canonical instance.
///
/// Per ADR-0013 §9, `noop-heartbeat` is Phase 1's proof-of-life
/// reconciler: its `reconcile` returns `vec![Action::Noop]`
/// deterministically, serving as the fixture against which the
/// `ReconcilerIsPure` invariant's twin-invocation check runs and as
/// Spawn the broker-driven convergence-tick loop.
///
/// Per `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2 §18
/// wiring), each iteration drains the `EvaluationBroker`, dispatches one
/// `run_convergence_tick` per pending `Evaluation`, then sleeps
/// `tick_cadence` before re-draining. Cancellation via `shutdown` is
/// observed in `tokio::select!` between ticks so an in-flight dispatch
/// always completes before exit.
///
/// Without this spawn, `submit_job` and `stop_job` would only write to
/// the `IntentStore` — the broker would never be drained, no allocations
/// would ever be scheduled, and `cluster_status.broker.dispatched` would
/// permanently read 0. See
/// `docs/feature/fix-convergence-loop-not-spawned/bugfix-rca.md` for
/// the full root-cause chain.
///
/// The cadence sleep goes through the injected `Clock`: production
/// (`SystemClock`) parks on a real timer; DST (`SimClock`) parks until
/// the harness calls `sim_clock.tick(cadence)` to advance logical time
/// past the deadline. Either way the loop suspends between ticks
/// rather than busy-polling.
fn spawn_convergence_loop(
    state: AppState,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    cadence: Duration,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick_n: u64 = 0;
        loop {
            let now = clock.now();
            let deadline = now + cadence;

            // Drain the broker into a local Vec — the
            // parking_lot::MutexGuard MUST be dropped before any
            // `.await` per `.claude/rules/development.md`
            // § Concurrency & async (no locks across `.await`).
            let pending = {
                let mut broker = state.runtime.broker();
                broker.drain_pending()
            };

            for eval in pending {
                if let Err(e) = run_convergence_tick(
                    &state,
                    &eval.reconciler,
                    &eval.target,
                    now,
                    tick_n,
                    deadline,
                )
                .await
                {
                    tracing::warn!(
                        target: "overdrive::reconciler",
                        ?e,
                        reconciler = %eval.reconciler,
                        target_name = %eval.target.as_str(),
                        "convergence tick error"
                    );
                }
            }

            tick_n = tick_n.saturating_add(1);

            tokio::select! {
                () = clock.sleep(cadence) => {},
                () = shutdown.cancelled() => break,
            }
        }
    })
}

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
