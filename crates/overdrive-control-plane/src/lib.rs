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
//! | `handlers` | axum route handlers — submit_workload, describe_workload, cluster_status, alloc_status, node_list |
//! | `error` | `ControlPlaneError` enum + `to_response` mapping (ADR-0015) |
//! | `tls_bootstrap` | Ephemeral CA + trust triple + rustls config (ADR-0010) |
//! | `reconciler_runtime` | `ReconcilerRuntime` + registry (ADR-0013/ADR-0035) |
//! | `view_store` | Runtime-owned `ViewStore` port + `RedbViewStore` (ADR-0035) |
//! | `observation_wiring` | `LocalObservationStore` single-node wiring (ADR-0012, revised 2026-04-24) |

// Per ADR-0028, this crate's `cgroup_preflight` module calls
// `libc::geteuid` directly. It is a thin syscall wrapper with no
// preconditions, but it is `extern "C"` and therefore requires an
// `unsafe` block. We `deny(unsafe_code)` workspace-wide and
// `#[allow(unsafe_code)]` scope-locally on the call site that needs
// it; switching from `forbid` to `deny` is what enables the scoped
// allow. Every other module in this crate stays unsafe-free.
#![deny(unsafe_code)]
// Phase 2.2 RED scaffolds in `reconcilers/service_map_hydrator/*` carry
// short docstrings on draft type definitions. Per
// `.claude/rules/testing.md` § "Production-side scaffolds", crates with many
// concurrent scaffolds gate the relevant lints crate-level via `expect` (NOT
// `allow`) so the gate self-removes the moment every scaffold goes GREEN.
// Slice 08-01 closed the `action_shim::DataplaneUpdateService` `todo!()` —
// `clippy::todo` is therefore dropped from this expect block. Strip the rest
// once the remaining scaffolds go GREEN.
#![expect(
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::too_long_first_doc_paragraph,
    clippy::doc_lazy_continuation,
    reason = "Phase 2.2 RED scaffolds; lints will self-trip when scaffolds go GREEN"
)]

pub mod action_shim;
pub mod api;
pub mod cgroup_manager;
pub mod cgroup_preflight;
// backend-discovery-bridge-service-reachability step 02-01 —
// `[dataplane]` config section parser per architecture.md § 5.1.
// Section presence + the two required interface bindings; refusal
// surfaces as `ControlPlaneError::Validation { field:
// Some("dataplane"), .. }`.
pub mod dataplane_config;
pub mod error;
pub mod handlers;
// backend-discovery-bridge-service-reachability step 02-01 — host
// IPv4 resolution via `getifaddrs(3)` for the operator-supplied
// `[dataplane] client_iface`. Production boot threads the resolved
// `Ipv4Addr` through `AppState.host_ipv4` to the
// `BackendDiscoveryBridge` reconciler per architecture.md § 5.2.
pub mod iface;
pub mod observation_wiring;
// `cargo openapi-{gen,check}` library — pure deterministic YAML render
// + drift detection. Paired with the `openapi` binary in `src/bin/`.
// Lives here (not in xtask) per § "xtask is build / test / dev
// orchestration, NOT a runtime entry point" in
// `.claude/rules/development.md`.
pub mod openapi;
pub mod reconciler_runtime;
// Phase 2.2 reconcilers per DWD-3. Currently hosts only the
// `service_map_hydrator`; future Phase 2+ reconcilers will land
// alongside.
pub mod reconcilers;
pub mod streaming;
pub mod tls_bootstrap;
// reconciler-memory-redb step 01-03 — `ViewStore` port + error types
// per ADR-0035 §2. Wired into `ReconcilerRuntime` in step 01-06.
// service-vip-allocator step 02-02 — `[dataplane.vip_allocator]` TOML
// parser surface per ADR-0049 § 5b. Owns boot-time section presence,
// TOML deserialisation, delegation to `VipRange::new`, and the
// structured `health.startup.refused` event on refusal.
pub mod view_store;
pub mod vip_allocator_config;
pub mod worker;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
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
    /// Filesystem path of the intent redb file. Used by handlers that
    /// decode persisted bytes via `Job::from_store_bytes(bytes, path, key)`
    /// to produce operator-facing remediation messages naming the file
    /// the bytes were read from. Per ADR-0048 § 6 / UI-03 amendment.
    pub intent_redb_path: PathBuf,
    /// Eventually-consistent observation store. Unused by 03-01's
    /// `submit_workload` handler, but wired in so observation-reading
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
    /// Production [`Dataplane`] impl per architecture.md § 7. The
    /// action shim's `Action::DataplaneUpdateService` arm dispatches
    /// through this trait object; production wires
    /// `Arc<EbpfDataplane>` from `overdrive-dataplane`, tests wire
    /// `Arc<SimDataplane>`. Per `.claude/rules/development.md`
    /// § "Port-trait dependencies", the dependency is mandatory at
    /// construction so tests cannot silently inherit production
    /// kernel I/O behaviour by forgetting to override.
    pub dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane>,
    /// Identity of the node writing observation rows. The action
    /// shim populates `LogicalTimestamp.writer` from this value so
    /// LWW resolution across peers is deterministic per
    /// `docs/whitepaper.md` §4.
    pub node_id: NodeId,
    /// Persistent ServiceVip allocator per ADR-0049 (amended
    /// 2026-05-15). Bulk-loaded from the byte-level `IntentStore` at
    /// boot and write-through-persisted on every allocation. Wrapped
    /// in `Arc<tokio::sync::Mutex<...>>` because `allocate().await`
    /// (which lands in 02-03d on the Service-arm submit path) crosses
    /// an `.await` and serialises VIP issuance across concurrent
    /// submit handlers — `tokio::sync::Mutex` rather than
    /// `parking_lot` per `.claude/rules/development.md` §
    /// "Concurrency & async" → "Never hold a lock across `.await`".
    ///
    /// 02-03c lands this field and the boot-time construction. The
    /// Service-arm `submit_workload` / `alloc_status` consumers land in
    /// 02-03d alongside the six S-VIP acceptance scenarios.
    pub allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    /// Host's IPv4 address for the configured `[dataplane]
    /// client_iface`. Resolved once at boot by
    /// [`iface::resolve_iface_ipv4`] and threaded through to the
    /// `BackendDiscoveryBridge` reconciler — every
    /// `service_backends` observation row the bridge emits carries
    /// this address in `endpoint.host` so XDP reverse-NAT translation
    /// (Phase 2.3) can derive the per-host VIP. Per
    /// `.claude/rules/development.md` § "Port-trait dependencies" the
    /// dependency is mandatory at construction so tests cannot
    /// silently inherit a production loopback by forgetting to
    /// override.
    ///
    /// Step 02-01 of
    /// `backend-discovery-bridge-service-reachability` lands this
    /// field; the placeholder `Ipv4Addr::LOCALHOST` previously
    /// threaded through `run_server_with_obs_and_driver` (introduced
    /// in 01-04) is removed in the same commit per
    /// `feedback_single_cut_greenfield_migrations.md`.
    pub host_ipv4: std::net::Ipv4Addr,
}

/// Test-only helper: build the default `PersistentServiceVipAllocator`
/// (per ADR-0049 amendment 2026-05-15) wrapped in
/// `Arc<tokio::sync::Mutex<...>>` for the AppState shape. Used by the
/// crate's `tests/acceptance/*` and `tests/integration/*` fixtures so
/// the per-fixture boilerplate stays small. Production callers go
/// through `PersistentServiceVipAllocator::bulk_load` in
/// `run_server_with_obs_and_driver` directly — `new` skips the boot-time
/// replay and is only safe in fixtures that start against a fresh store.
#[must_use]
pub fn test_default_allocator(
    store: Arc<dyn IntentStore>,
) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> {
    Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        store,
    )))
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
    #[allow(
        clippy::too_many_arguments,
        reason = "Port-trait dependencies (Clock, Driver, Dataplane, ObservationStore, IntentStore) are required at construction per .claude/rules/development.md § Port-trait dependencies; bundling them into a builder would make individual deps optional and defeat the explicit-injection invariant."
    )]
    pub fn new(
        store: Arc<LocalIntentStore>,
        intent_redb_path: PathBuf,
        obs: Arc<dyn ObservationStore>,
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        driver: Arc<dyn Driver>,
        clock: Arc<dyn Clock>,
        dataplane: Arc<dyn Dataplane>,
        node_id: NodeId,
        allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
        host_ipv4: std::net::Ipv4Addr,
    ) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(DEFAULT_LIFECYCLE_BROADCAST_CAPACITY);
        Self {
            store,
            intent_redb_path,
            obs,
            runtime,
            driver,
            lifecycle_events: Arc::new(tx),
            streaming_cap: DEFAULT_STREAMING_CAP,
            clock,
            dataplane,
            node_id,
            allocator,
            host_ipv4,
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
    /// Cadence between drains of the [`overdrive_core::eval_broker::EvaluationBroker`]
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

    /// `[node]` config block per ADR-0025 (amended by ADR-0029).
    /// Carries the operator-supplied `id_override` (hostname fallback
    /// when `None`), `region` (Phase 1 default `"local"`), and
    /// declared `capacity`. Consumed by
    /// [`overdrive_worker::start_local_node`] at boot to write the
    /// local node's `NodeHealthRow` to the `ObservationStore` per
    /// ADR-0025 step 5.
    ///
    /// `NodeConfig: Default` ships sensible Phase 1 defaults so
    /// existing `..Default::default()` rest-pattern construction in
    /// test fixtures continues to work without touching every
    /// fixture's TOML.
    pub node: overdrive_worker::NodeConfig,

    /// Address-pool definition for [`PersistentServiceVipAllocator`]
    /// per ADR-0049 (amended 2026-05-15 — default-with-override
    /// posture). When the operator-supplied TOML carries
    /// `[dataplane.vip_allocator]`, the parser yields `Some(range)` and
    /// the binary substitutes it here; when the section is absent,
    /// [`VipRange::default()`] supplies the Phase 1 pinned default
    /// (`10.96.0.0/16` reserved `[.0, .1, .255.255]`).
    pub vip_range: VipRange,

    /// Required `[dataplane]` section per
    /// `backend-discovery-bridge-service-reachability` architecture.md
    /// § 5.1 (step 02-01). Carries the operator-supplied
    /// `client_iface` + `backend_iface` bindings the production XDP
    /// programs attach to (Phase 2.3) and from which
    /// [`iface::resolve_iface_ipv4`] derives `AppState.host_ipv4` at
    /// boot.
    ///
    /// `Option` shape rather than non-optional because production
    /// reads the value from a TOML file via
    /// [`dataplane_config::parse_dataplane_section`], whose return
    /// shape is `Result<DataplaneConfig, ControlPlaneError>`. The
    /// CLI threads `Some(parsed)` here; test fixtures default to
    /// `Some(DataplaneConfig::loopback())` via the `Default` impl
    /// below so existing `..Default::default()` rest-pattern
    /// construction continues to work without touching every
    /// fixture's TOML.
    ///
    /// `run_server_with_obs_and_driver` refuses to start when this
    /// field is `None` with
    /// `ControlPlaneError::Validation { field: Some("dataplane"), .. }`
    /// — the same shape the parser returns. See architecture.md
    /// § 5.2.
    pub dataplane: Option<dataplane_config::DataplaneConfig>,

    /// Optional override for the bpffs directory the `EbpfDataplane`
    /// pins SERVICE_MAP into. `None` (the production default) means
    /// `overdrive_dataplane::DEFAULT_PIN_DIR` = `/sys/fs/bpf/overdrive`.
    /// Tests pass a per-test tempdir under `/sys/fs/bpf/<name>` to
    /// avoid cross-test SERVICE_MAP pin collisions when many
    /// integration tests boot the server concurrently inside Lima.
    /// Per `feedback_single_cut_greenfield_migrations.md`, production
    /// `serve` always reads `None` (single-cut to EbpfDataplane); the
    /// field exists only for test isolation.
    pub dataplane_pin_dir: Option<PathBuf>,

    /// Optional override for the cgroup the `cgroup_connect4_service`
    /// program attaches to per ADR-0053 § 7. `None` (the production
    /// default) means
    /// `overdrive_dataplane::DEFAULT_CGROUP_ATTACH_PATH` =
    /// `/sys/fs/cgroup/overdrive.slice` — the slice
    /// `crates/overdrive-worker/src/cgroup_manager.rs` already
    /// manages. Tests may inject a per-test tempdir under
    /// `/sys/fs/cgroup/<name>` to avoid cross-test attachment
    /// collisions when multiple integration tests boot the server
    /// concurrently inside Lima.
    pub dataplane_cgroup_attach_path: Option<PathBuf>,

    /// Optional injected [`Dataplane`] adapter — for tests whose
    /// subject under test is NOT the dataplane attach path. When
    /// `Some(_)`, the boot path uses this adapter and SKIPS
    /// `EbpfDataplane::new`; when `None` (the production default),
    /// the boot path constructs `EbpfDataplane` from the
    /// `[dataplane]` config section per architecture.md § 5.2.
    ///
    /// Per architecture.md § 4.7, tests inject `SimDataplane` via
    /// this field; production binaries (the CLI's `serve` subcommand)
    /// pass `None` so the single-cut `EbpfDataplane` composition
    /// per `feedback_single_cut_greenfield_migrations.md` is the
    /// only production-reachable code path.
    ///
    /// Excluded from [`Debug`] / [`Default`] — `Arc<dyn Dataplane>`
    /// is neither `Debug` nor `Default`.
    ///
    /// [`Dataplane`]: overdrive_core::traits::dataplane::Dataplane
    pub dataplane_override: Option<Arc<dyn overdrive_core::traits::dataplane::Dataplane>>,

    /// Test-only failure-injection seam for the `EbpfDataplane::probe`
    /// Earned-Trust call site. When `Some(msg)`, the boot path
    /// constructs `DataplaneError::LoadFailed(msg)` and applies it to
    /// the constructed `EbpfDataplane` via
    /// [`overdrive_dataplane::EbpfDataplane::set_probe_fault`] BEFORE
    /// `.probe().await` runs; the probe short-circuits to the
    /// constructed error so the boot path exercises the
    /// `DataplaneBootError::Probe` mapping arm and the
    /// `health.startup.refused` emit per architecture.md § 5.4.
    ///
    /// The seam is `String`-shaped (not `DataplaneError`-shaped)
    /// because `DataplaneError` cannot derive `Clone` — its `Io`
    /// variant embeds `std::io::Error`. Storing the load-failure
    /// message and reconstructing `LoadFailed(msg)` at the boot
    /// boundary keeps the field `Clone` (and therefore `ServerConfig:
    /// Clone`) without broadening `DataplaneError`'s public surface.
    /// The S-BDB-14 fixture injects a verbatim "probe: round-trip
    /// mismatch ..." string per architecture.md § 5.4.
    ///
    /// Gated behind `#[cfg(feature = "integration-tests")]` on both
    /// this field declaration and its use site in the boot path.
    /// Production builds compile the field out entirely; the
    /// `..Default::default()` rest-pattern construction in production
    /// callers never names it. The `cfg(test)` arm is deliberately
    /// omitted — the boot-path call site forwards the feature into
    /// `overdrive-dataplane` via the Cargo.toml dep, and using
    /// `cfg(test)` here would let the control-plane's own `cargo
    /// test --no-run` enable the call site without enabling
    /// `set_probe_fault` on the dataplane dep.
    #[cfg(feature = "integration-tests")]
    pub dataplane_probe_fault: Option<String>,
}

impl std::fmt::Debug for ServerConfig {
    /// `Arc<dyn Clock>` is not [`Debug`], so the auto-derive on
    /// `ServerConfig` is replaced by a manual impl that elides the
    /// clock field.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("ServerConfig");
        dbg.field("bind", &self.bind)
            .field("data_dir", &self.data_dir)
            .field("operator_config_dir", &self.operator_config_dir)
            .field("tick_cadence", &self.tick_cadence)
            .field("clock", &"<dyn Clock>")
            .field("node", &self.node)
            .field("vip_range", &"<VipRange>")
            .field("dataplane", &self.dataplane)
            .field("dataplane_pin_dir", &self.dataplane_pin_dir)
            .field("dataplane_cgroup_attach_path", &self.dataplane_cgroup_attach_path)
            .field(
                "dataplane_override",
                &self.dataplane_override.as_ref().map(|_| "<dyn Dataplane>"),
            );
        #[cfg(feature = "integration-tests")]
        dbg.field("dataplane_probe_fault", &self.dataplane_probe_fault);
        dbg.finish()
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
            node: overdrive_worker::NodeConfig::default(),
            vip_range: VipRange::default(),
            // Per step 02-01: `Default` populates the loopback
            // `[dataplane]` shape so existing test fixtures using
            // `..Default::default()` rest-pattern construction
            // continue to work. Production callers go through
            // `parse_dataplane_section` and overwrite this with the
            // operator-supplied value.
            dataplane: Some(dataplane_config::DataplaneConfig::loopback()),
            // Step 02-02: `None` means production default
            // (`/sys/fs/bpf/overdrive`). Tests override to a per-test
            // tempdir for SERVICE_MAP pin isolation.
            dataplane_pin_dir: None,
            // ADR-0053 § 7: `None` means production default
            // (`/sys/fs/cgroup/overdrive.slice`). Tests inject a
            // per-test cgroup path for attachment isolation.
            dataplane_cgroup_attach_path: None,
            // Step 02-02: `None` means production default
            // (construct `EbpfDataplane` from the `[dataplane]`
            // section). Tests that exercise unrelated subsystems
            // inject `Some(Arc::new(SimDataplane::new()))` per
            // architecture.md § 4.7.
            dataplane_override: None,
            // Step 02-03: `None` means production default (probe
            // runs against the real BACKEND_MAP via the typed
            // `EbpfDataplane` handle). Tests inject
            // `Some(DataplaneError::...)` to exercise the
            // `DataplaneBootError::Probe` mapping arm (S-BDB-14).
            #[cfg(feature = "integration-tests")]
            dataplane_probe_fault: None,
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
/// Construct the persistent ServiceVip allocator per ADR-0049
/// (amended 2026-05-15). `bulk_load` replays any persisted allocator
/// entries from the byte-level `IntentStore` so VIP issuance resumes
/// from the post-crash counter rather than colliding with prior
/// allocations.
///
/// Per step 02-04 the `bulk_load` round-trip ALSO runs the Earned
/// Trust boot probe — every persisted VIP must project back within
/// the active [`VipRange`]. On probe failure (typically an operator
/// who narrowed the configured range after allocations were persisted
/// under a wider range), this fn emits a structured
/// `health.startup.refused` event naming the offending VIP and
/// propagates the typed [`overdrive_dataplane::allocators::PersistentAllocatorError`]
/// via the `#[from]` variant on `ControlPlaneError::VipAllocator` —
/// never flattened to `Internal(String)` per
/// `.claude/rules/development.md` § "Never flatten a typed error to
/// `Internal(String)` at a composition boundary".
///
/// Extracted from `run_server_with_obs_and_driver` to keep that fn
/// under the clippy `too_many_lines` ceiling — the construction is
/// otherwise a single logical step.
/// Validate the `[dataplane]` config section and resolve the host
/// IPv4 address for the configured `client_iface` per
/// `backend-discovery-bridge-service-reachability` architecture.md
/// § 5.1 / § 5.2 (step 02-01).
///
/// Two refusal shapes per
/// `.claude/rules/development.md` § Errors → "Distinct failure
/// modes get distinct error variants":
///
/// - Missing section → [`error::ControlPlaneError::Validation`] with
///   `field = Some("dataplane")` so the operator's CLI / log
///   surface can branch on the field without `Display`-grepping.
/// - `getifaddrs(3)` refusal → [`error::DataplaneBootError::
///   IfaceAddrResolution`] embedding the underlying `io::Error`
///   verbatim (NotFound vs Other) for programmatic inspection.
///
/// Extracted from `run_server_with_obs_and_driver` to keep that fn
/// under the clippy `too_many_lines` ceiling — the validation +
/// resolve sequence is otherwise a single logical step.
fn resolve_host_ipv4_from_dataplane_config(
    dataplane: Option<&dataplane_config::DataplaneConfig>,
) -> Result<std::net::Ipv4Addr, error::ControlPlaneError> {
    let dataplane_cfg = dataplane.ok_or_else(|| error::ControlPlaneError::Validation {
        message: "missing required [dataplane] section in overdrive.toml \
                  (client_iface + backend_iface)"
            .to_owned(),
        field: Some("dataplane".to_owned()),
    })?;
    let host_ipv4 = iface::resolve_iface_ipv4(&dataplane_cfg.client_iface).map_err(|source| {
        error::DataplaneBootError::IfaceAddrResolution {
            iface: dataplane_cfg.client_iface.clone(),
            source,
        }
    })?;
    Ok(host_ipv4)
}

async fn bulk_load_service_vip_allocator(
    vip_range: &VipRange,
    store: &Arc<LocalIntentStore>,
) -> Result<Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>, error::ControlPlaneError> {
    let intent_store: Arc<dyn IntentStore> = Arc::clone(store) as Arc<dyn IntentStore>;
    let allocator = PersistentServiceVipAllocator::bulk_load(vip_range.clone(), intent_store)
        .await
        .map_err(|err| {
            tracing::error!(
                target: "overdrive::health",
                event = "health.startup.refused",
                cause = %err,
                "ServiceVipAllocator bulk_load refused; control-plane will not start"
            );
            error::ControlPlaneError::from(err)
        })?;
    Ok(Arc::new(tokio::sync::Mutex::new(allocator)))
}

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
    // The control-plane crate calls the
    // `ExecDriver::new_with_default_fs` factory (which internally
    // wires the production cgroupfs adapter) rather than naming the
    // port trait or its concrete production binding here. This
    // preserves ADR-0029's invariant that the control-plane crate
    // does NOT name worker-internal port traits — and removes the
    // temporary cross-boundary host-cgroup-adapter construction
    // that step 01-05 introduced as a mechanical migration shim.
    //
    // The composition-root probe runs in `overdrive-cli`'s `serve`
    // subcommand BEFORE this `run_server` is invoked (ADR-0054
    // § Composition root wiring); `run_server` itself is the
    // in-process convenience used by integration tests that do not
    // exercise the probe path.
    let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new_with_default_fs(
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
#[allow(clippy::unused_async, clippy::too_many_lines)]
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
    cgroup_preflight::run_preflight().map_err(error::ControlPlaneError::from)?;
    cgroup_manager::create_and_enrol_control_plane_slice()
        .map_err(error::ControlPlaneError::from)?;
    // Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
    // § "Production fix #2": delegate `+cpu +memory +io +pids` to
    // `overdrive.slice/workloads.slice/cgroup.subtree_control` BEFORE
    // the convergence loop accepts any allocations. Without this, the
    // per-alloc `cpu.weight` / `memory.max` writes return EACCES on
    // real cgroupfs (the resource interface files do not exist on
    // children of a slice whose subtree_control is empty), silently
    // absorbed by the ADR-0026 D9 warn-and-continue disposition.
    // Order vs the control-plane init above does not matter — the two
    // touch disjoint slice paths.
    overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers(
        std::path::Path::new(cgroup_preflight::DEFAULT_CGROUP_ROOT),
    )
    .map_err(error::ControlPlaneError::from)?;

    // Per ADR-0025 step 5 (amended by ADR-0029): the worker subsystem
    // writes the local node's `NodeHealthRow` to the ObservationStore
    // BEFORE the listener binds. A failure here refuses the boot per
    // the ADR-0025 §3 step 5 contract — operators see the typed
    // `ControlPlaneError::NodeHealthWrite` variant at the CLI layer
    // rather than a silently-orphaned writer leaving `GET /v1/nodes`
    // empty. Per `.claude/rules/development.md` § "Never flatten a
    // typed error to `Internal(String)` at a composition boundary"
    // the conversion is `#[from]` — never
    // `ControlPlaneError::internal("context", e)`.
    overdrive_worker::start_local_node(&obs, &config.node, &config.clock)
        .await
        .map_err(error::ControlPlaneError::from)?;

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
    runtime.register(workload_lifecycle()).await?;

    // Production boot threads the `ServerConfig.clock` into AppState
    // so the streaming submit handler's cap timer uses the same clock
    // as the convergence-loop spawn. The clock is required at
    // construction per `.claude/rules/development.md` § "Port-trait
    // dependencies"; there is no post-construction injection path.
    //
    // backend-discovery-bridge-service-reachability step 02-02 —
    // wire `EbpfDataplane` as the single production `Dataplane`
    // adapter per architecture.md § 5.2. Single-cut migration from
    // `NoopDataplane` per `feedback_single_cut_greenfield_migrations.md`.
    // Failure paths route through `DataplaneBootError::Construct`
    // per architecture.md § 5.3.
    //
    // Tests whose subject under test is NOT the dataplane attach path
    // (CLI HTTPS handshake, trust-triple round-trip, action-shim
    // dispatch arms, observation-row read handlers) inject
    // `SimDataplane` via `config.dataplane_override` per architecture.md
    // § 4.7. Production binaries leave `dataplane_override = None` so
    // `EbpfDataplane::new` is the only production-reachable adapter.
    let dataplane_cfg =
        config.dataplane.as_ref().ok_or_else(|| error::ControlPlaneError::Validation {
            message: "missing required [dataplane] section in overdrive.toml \
                      (client_iface + backend_iface)"
                .to_owned(),
            field: Some("dataplane".to_owned()),
        })?;
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        if let Some(overridden) = config.dataplane_override.clone() {
            overridden
        } else {
            // ADR-0053 § 7: resolve cgroup_attach_path with the
            // production default when unset.
            let cgroup_attach_path: std::path::PathBuf =
                config.dataplane_cgroup_attach_path.clone().unwrap_or_else(|| {
                    std::path::PathBuf::from(overdrive_dataplane::DEFAULT_CGROUP_ATTACH_PATH)
                });
            #[allow(unused_mut)] // mut needed only under cfg(test|integration-tests)
            let mut ebpf_dataplane = config
                .dataplane_pin_dir
                .as_deref()
                .map_or_else(
                    || {
                        overdrive_dataplane::EbpfDataplane::new_with_pin_dir(
                            &dataplane_cfg.client_iface,
                            &dataplane_cfg.backend_iface,
                            std::path::Path::new(overdrive_dataplane::DEFAULT_PIN_DIR),
                            cgroup_attach_path.as_path(),
                        )
                    },
                    |pin_dir| {
                        overdrive_dataplane::EbpfDataplane::new_with_pin_dir(
                            &dataplane_cfg.client_iface,
                            &dataplane_cfg.backend_iface,
                            pin_dir,
                            cgroup_attach_path.as_path(),
                        )
                    },
                )
                .map_err(|source| error::DataplaneBootError::Construct {
                    client_iface: dataplane_cfg.client_iface.clone(),
                    backend_iface: dataplane_cfg.backend_iface.clone(),
                    source,
                })?;

            // Step 02-03: Apply the test-only probe-fault seam BEFORE
            // running the probe. Production builds compile both the
            // field and this branch out entirely. The seam is
            // `String`-shaped at the `ServerConfig` boundary (see the
            // field docstring) and reconstructed into
            // `DataplaneError::LoadFailed` here — the variant
            // S-BDB-14's assertion (`matches!(... LoadFailed(_))`)
            // expects.
            //
            // Gated on `feature = "integration-tests"` (NOT also
            // `cfg(test)`) so the gate matches the upstream
            // `EbpfDataplane::set_probe_fault` symbol's cfg —
            // `cargo test --no-run -p overdrive-control-plane`
            // without `--features integration-tests` would otherwise
            // enable this branch (test-of-control-plane sets
            // `cfg(test)` for control-plane) while leaving the
            // dataplane dep compiled without the feature, so the
            // method would be missing. The integration-tests feature
            // forwards via the Cargo.toml dep — see this crate's
            // `[features]` block.
            #[cfg(feature = "integration-tests")]
            if let Some(fault_msg) = config.dataplane_probe_fault.clone() {
                ebpf_dataplane.set_probe_fault(
                    overdrive_core::traits::dataplane::DataplaneError::LoadFailed(fault_msg),
                );
            }

            // Step 02-03: Earned-Trust probe per architecture.md § 5.4
            // and CLAUDE.md principle 12. Composition-root invariant
            // "wire then probe then use" — the probe runs AFTER
            // `new()` succeeds and BEFORE the first dataplane operation.
            // On failure we emit a structured `health.startup.refused`
            // event with `reason = "dataplane.probe"` and refuse to
            // boot. The `?` causes `ebpf_dataplane` to drop, which
            // detaches XDP and unlinks the SERVICE_MAP pin via the
            // `EbpfDataplane::Drop` impl.
            if let Err(source) = ebpf_dataplane.probe().await {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "dataplane.probe",
                    client_iface = %dataplane_cfg.client_iface,
                    backend_iface = %dataplane_cfg.backend_iface,
                    error = %source,
                    "Earned-Trust probe failed; refusing to boot"
                );
                return Err(error::ControlPlaneError::DataplaneBoot(
                    error::DataplaneBootError::Probe { source },
                ));
            }

            Arc::new(ebpf_dataplane)
        };
    // Phase 2.2: production single-mode uses a placeholder node id;
    // Phase 2 introduces real node-bootstrap identity that will replace
    // this. The shim writes this into `LogicalTimestamp.writer` on
    // `service_hydration_results` rows.
    let node_id = overdrive_core::id::NodeId::new("local").map_err(|e| {
        error::ControlPlaneError::Internal(format!("placeholder NodeId rejected: {e}"))
    })?;

    // backend-discovery-bridge-service-reachability step 02-01 —
    // require the `[dataplane]` config section per architecture.md
    // § 5.1 and resolve `host_ipv4` via `getifaddrs(3)` on the
    // operator-supplied `client_iface`. The `Ipv4Addr::LOCALHOST`
    // placeholder from 01-04 is removed in the same commit per
    // `feedback_single_cut_greenfield_migrations.md`.
    let host_ipv4 = resolve_host_ipv4_from_dataplane_config(config.dataplane.as_ref())?;
    runtime.register(backend_discovery_bridge(host_ipv4, node_id.clone())).await?;
    // UI-05 (`backend-discovery-bridge-service-reachability` step
    // 02-04 architectural remediation) — register the
    // `service-map-hydrator` at production boot. Prior to UI-05 this
    // was absent from the production wiring (architecture.md § 4.7
    // / § 6 carried `// existing` comments that did not reflect any
    // actual `runtime.register` call site); the bridge → hydrator
    // handoff failed silently in production. Registration MUST land
    // AFTER `backend_discovery_bridge` so the bridge's emitted
    // `Action::EnqueueEvaluation { reconciler: "service-map-hydrator",
    // .. }` resolves against a registered reconciler when the
    // broker first drains.
    runtime.register(service_map_hydrator(host_ipv4)).await?;
    let runtime = Arc::new(runtime);

    let allocator = bulk_load_service_vip_allocator(&config.vip_range, &store).await?;

    let state: AppState = AppState::new(
        store,
        store_path,
        obs,
        runtime,
        driver,
        config.clock.clone(),
        dataplane,
        node_id,
        allocator,
        host_ipv4,
    );

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
    // Without this spawn, `submit_workload` and `stop_workload` would only
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
        .route("/v1/jobs", post(handlers::submit_workload))
        .route("/v1/jobs/:id", get(handlers::describe_workload))
        .route("/v1/jobs/:id/stop", post(handlers::stop_workload))
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
/// Without this spawn, `submit_workload` and `stop_workload` would only write to
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
pub fn noop_heartbeat() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, NoopHeartbeat};

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
pub fn workload_lifecycle() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, WorkloadLifecycle};

    AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical())
}

/// Construct the `backend-discovery-bridge` reconciler per
/// `docs/feature/backend-discovery-bridge-service-reachability/
/// design/architecture.md` § 4.7 (boot composition).
///
/// The bridge converges `service_backends` observation rows for the
/// workload's declared listeners against the actual Running alloc
/// set, emitting `Action::WriteServiceBackendRow` on fingerprint
/// drift. Both `host_ipv4` and `writer_node_id` are mandatory per
/// `.claude/rules/development.md` § "Port-trait dependencies" — the
/// reconciler is constructed once at boot and the runtime composes
/// the same instance across every tick.
///
/// Phase 01 production boot threads `Ipv4Addr::LOCALHOST` as the
/// `host_ipv4` placeholder (step 01-04, single-commit transitional
/// shape); step 02-01 replaces this with the resolved interface
/// IPv4 from the dataplane config.
#[must_use]
pub fn backend_discovery_bridge(
    host_ipv4: std::net::Ipv4Addr,
    writer_node_id: overdrive_core::id::NodeId,
) -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::AnyReconciler;
    use overdrive_core::reconcilers::backend_discovery_bridge::BackendDiscoveryBridge;

    AnyReconciler::BackendDiscoveryBridge(BackendDiscoveryBridge::new(host_ipv4, writer_node_id))
}

/// Construct the `service-map-hydrator` reconciler.
///
/// Activates J-PLAT-004 per ADR-0042 — converges
/// `service_hydration_results` rows by dispatching
/// `Action::DataplaneUpdateService` whenever a service's bridge-written
/// `(vip, backends)` fingerprint drifts from the last
/// confirmed-applied fingerprint persisted in the hydrator's `View`.
///
/// Registered at production boot AFTER `backend-discovery-bridge`
/// (the bridge re-enqueues this reconciler per UI-05 cross-reconciler
/// handoff — see `Action::EnqueueEvaluation`). Order matters only
/// for `cluster_status`'s deterministic registration listing; the
/// runtime registers idempotently regardless of order.
#[must_use]
pub fn service_map_hydrator(
    host_ipv4: std::net::Ipv4Addr,
) -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, ServiceMapHydrator};

    AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(host_ipv4))
}
