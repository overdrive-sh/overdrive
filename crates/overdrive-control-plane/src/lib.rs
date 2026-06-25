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
// built-in-ca (GH #28, ADR-0063 D2/D3/D8) — CA boot composition root:
// generate-or-load the persistent root + Earned-Trust probe + refuse-to-start.
pub mod ca_boot;
// built-in-ca (GH #28, ADR-0063 D6) — CA issuance + audit binding: every
// workload SVID issuance writes an `issued_certificates` observation row, bound
// so an audit-write failure refuses the issuance (no silent issuance).
pub mod ca_issuance;
pub mod cgroup_manager;
pub mod cgroup_preflight;
// backend-discovery-bridge-service-reachability step 02-01 —
// `[dataplane]` config section parser per architecture.md § 5.1.
// Section presence + the two required interface bindings; refusal
// surfaces as `ControlPlaneError::Validation { field:
// Some("dataplane"), .. }`.
pub mod dataplane_config;
// dial-by-name-responder (ADR-0072, GH #243) — the in-agent name layer:
// the third reader of the `ObservationStore` `service_backends` surface,
// answering `<job>.svc.overdrive.local` queries. Step 01-02 lands only the
// `wire` codec (DNS decode/encode behind the DDN-4/D-DBN-5 ACL boundary);
// `answer.rs` / `name_index.rs` / `responder.rs` are later slices.
pub mod dns_responder;
pub mod error;
pub mod handlers;
// workload-identity-manager step 01-03 (ADR-0067 D4) — `IdentityMgr`, the
// in-process held-SVID store + boot trust bundle. Ephemeral runtime state
// (neither intent nor observation); `held_snapshot` yields the `HeldSvidFacts`
// projection the `SvidLifecycle` reconciler reads as `actual`. The
// `IdentityRead` impl lands 02-01; the reconciler wiring 01-04.
pub mod identity_mgr;
// backend-discovery-bridge-service-reachability step 02-01 — host
// IPv4 resolution via `getifaddrs(3)` for the operator-supplied
// `[dataplane] client_iface`. Production boot threads the resolved
// `Ipv4Addr` through `AppState.host_ipv4` to the
// `BackendDiscoveryBridge` reconciler per architecture.md § 5.2.
pub mod iface;
// workflow-primitive step 01-03 — `JournalStore` port + `LoadedEntry`
// CBOR boundary sum (over `JournalCommand` / `JournalNotification`) +
// `WorkflowId` for the §18 workflow await-point journal (ADR-0066). A
// second redb table layout on the shared runtime substrate, distinct from
// `view_store`. Real `RedbJournalStore` adapter lands 01-04; engine wiring
// 01-05.
pub mod journal;
// reconciler-listener-fact-view step 01-01 — in-memory listener-fact
// projection (ADR-0062) replacing the `ServiceMapHydrator`'s O(S²)
// per-tick cluster scan with an O(1) keyed read off a maintained view.
pub mod listener_facts;
// transparent-mtls-enrollment step 01-03 (ADR-0071, GH #242) —
// `ServiceBackendsResolve`, the v1 host `MtlsResolve` adapter. Resolves
// `orig_dst` against an in-RAM, ownership-aware `addr → {service → Backend}`
// reverse index of the `running` `service_backends` set (C4), maintained by
// List-then-Watch over the `ObservationStore` `all_service_backends_rows` +
// `subscribe_all_events` surfaces; classifies into the 3-variant `MtlsResolution`
// (Mesh / NonMesh / MeshUnreachable). v1 SHELL: `expected_svid: None`, no
// `IdentityRead` (the identity join is #242). Earned-Trust `probe` refuses on
// an unreadable store. Composition-root probe wiring lands in step 04-02.
pub mod mtls_resolve_adapter;
pub mod observation_wiring;
// `cargo openapi-{gen,check}` library — pure deterministic YAML render
// + drift detection. Paired with the `openapi` binary in `src/bin/`.
// Lives here (not in xtask) per § "xtask is build / test / dev
// orchestration, NOT a runtime entry point" in
// `.claude/rules/development.md`.
pub mod openapi;
// service-health-check-probes step 01-03d — composition-root
// `ProbeRunner` Earned-Trust boot helper per ADR-0054 § 7.
pub mod probe_runner_boot;
pub mod reconciler_runtime;
pub mod streaming;
pub mod tls_bootstrap;
// single-node-dataplane-wiring step 01-02 — single-node veth provisioner
// (adapter-host) per ADR-0061 § 3. Pure `derive_veth_plan` (default
// lane) + idempotent `provision` production code (NOT
// integration-tests-gated). Wired into serve boot in step 01-03.
pub mod veth_provisioner;
// reconciler-memory-redb step 01-03 — `ViewStore` port + error types
// per ADR-0035 §2. Wired into `ReconcilerRuntime` in step 01-06.
// service-vip-allocator step 02-02 — `[dataplane.vip_allocator]` TOML
// parser surface per ADR-0049 § 5b. Owns boot-time section presence,
// TOML deserialisation, delegation to `VipRange::new`, and the
// structured `health.startup.refused` event on refusal.
pub mod view_store;
pub mod vip_allocator_config;
pub mod worker;
// `workflow_runtime` — the durable-async `WorkflowEngine` (ADR-0064 §1,
// §3, §5). Drives author `async fn run` futures off the action-shim with
// crash-safe journal-cursor replay. The `Workflow` trait + `WorkflowCtx`
// it drives live in `overdrive-core::workflow`; this is the tokio-holding
// executor that core's trait declaration delegates to.
pub mod workflow_runtime;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use overdrive_core::id::NodeId;
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_store_local::LocalIntentStore;
use tokio_util::sync::CancellationToken;

use crate::identity_mgr::IdentityMgr;
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
    /// In-memory listener-fact projection (ADR-0062 § Decision (1);
    /// feature-delta sub-decision 1). Mirrors the allocator's lifecycle
    /// — boot-rebuilt from the intent SSOT (via
    /// [`ListenerFactStore::rebuild_from_intent`]) and held here for the
    /// hydration layer's O(1) keyed read — MINUS persistence (the intent
    /// store is the SSOT; cold boot re-projects). Wrapped in
    /// `Arc<tokio::sync::Mutex<...>>` because it is acquired across
    /// `.await` in the async hydrate / submit-edge paths and the rebuild
    /// itself is async — `tokio::sync::Mutex` rather than `parking_lot`
    /// per `.claude/rules/development.md` § "Concurrency & async" →
    /// "Never hold a lock across `.await`".
    ///
    /// 01-02 lands this field and the boot-time construction (rebuilt
    /// immediately AFTER the allocator's `bulk_load` — ordering is
    /// load-bearing: the rebuild joins allocator-issued VIPs). The
    /// submit / stop edge maintenance lands in 01-03; the hydrator
    /// read-path switch lands in 01-04. After this step the store
    /// exists and is boot-rebuilt but is not yet mutated on the edge nor
    /// read by the hydrator.
    pub listener_facts: Arc<tokio::sync::Mutex<crate::listener_facts::ListenerFactStore>>,
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
    /// The durable-async workflow executor (ADR-0064 §1, §5). The
    /// reconciler-runtime's action-shim dispatch hands every committed
    /// `Action::StartWorkflow` to this engine off the shim — exactly as
    /// `Action::StartAllocation` → `Driver::start`. The engine is composed
    /// at boot over the real `RedbJournalStore` + injected ports + the
    /// `ObservationStore`; the workflow-lifecycle reconciler's
    /// `hydrate_actual` reads its live-task set
    /// ([`workflow_runtime::WorkflowEngine::live_instances`]) to mark an
    /// instance running.
    ///
    /// Mandatory at construction per `.claude/rules/development.md`
    /// § "Port-trait dependencies" — there is no `Option`/`None` shim
    /// (the 01-05/01-06 `dispatch(... None)` placeholder is gone). Tests
    /// inject an engine over `Sim*` ports.
    pub workflow_engine: Arc<workflow_runtime::WorkflowEngine>,
    /// The built-in certificate-authority driving port (ADR-0067 D3). The
    /// action shim's `Action::IssueSvid` arm dispatches through this trait
    /// object via `ca_issuance::issue_and_audit` (the ONE place workload-CA
    /// I/O happens). Production composes an EPHEMERAL `RcgenCa` at boot (fresh
    /// in-memory P-256 root each boot, NO KEK / NO persistence — the original
    /// Phase-2 plan; the persistent KEK-backed root + operator render are
    /// GH #215, blocked on #35). Tests inject `SimCa`.
    ///
    /// Mandatory at construction (`Arc<dyn Ca>`, NOT `Option`) per
    /// `.claude/rules/development.md` § "Port-trait dependencies": there IS a
    /// CA now, so the dependency is required at every call site.
    pub ca: Arc<dyn Ca>,
    /// The in-process held-SVID store + boot trust bundle (ADR-0067 D4). The
    /// action shim's `Action::IssueSvid` arm holds the minted `SvidMaterial`
    /// here after `issue_and_audit` succeeds; `Action::DropSvid` removes it.
    /// The `SvidLifecycle` reconciler (01-04) reads its `held_snapshot` as
    /// `actual`. Ephemeral runtime state — neither intent nor observation —
    /// rebuilt on restart by re-issuing for every still-Running alloc.
    pub identity: Arc<IdentityMgr>,
    /// The (β) transparent-mTLS intercept-and-enforce lifecycle component
    /// (transparent-mtls-host-socket, D-MTLS-16/17, GH #26; step 06-03).
    /// The action-shim fires it alongside the driver hooks
    /// (`on_alloc_running` → `start_alloc`, `on_alloc_terminal` →
    /// `stop_alloc`); `ExecDriver` is UNTOUCHED.
    ///
    /// `Option` (the sanctioned `ProbeRunner` shape, NOT a port-trait
    /// dodge): `Some(worker)` ONLY on the production `run_server` boot
    /// (and the Tier-3 e2e), where a REAL `EbpfDataplane` +
    /// `HostMtlsEnforcement` + `MtlsDataplane` are composed AFTER
    /// `IdentityMgr`; `None` for every non-mTLS fixture and the
    /// `SimDataplane`-override boot (no real BPF to intercept on). The
    /// action-shim reads `state.mtls_worker` and fires `if let Some`.
    pub mtls_worker: Option<Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>>,
    /// Per-host network-slot free-list (transparent-mtls-enrollment, D-TME-12
    /// G3; step 04-01). Hands out the host-unique, collision-free-by-
    /// construction [`veth_provisioner::NetSlot`] each live allocation's
    /// netns/veth/subnet is keyed from, at the action-shim C3 provision seam.
    ///
    /// NOT an `Option`: unlike `mtls_worker`, the allocator is harmless on the
    /// non-mTLS fixture surface (it just hands out slots nobody provisions),
    /// so a non-optional `Default`-constructed field keeps every fixture
    /// ripple-free. The type is already `#[derive(Clone, Default)]` and holds
    /// its `Arc<Mutex<BTreeMap<…>>>` INTERNALLY (it self-shares on clone,
    /// exactly like `IdentityMgr`), so the field is a plain value — no outer
    /// `Arc<Mutex<…>>` wrapper. Ephemeral runtime state, never persisted:
    /// on a fresh process boot nothing is held (criterion 6).
    pub net_slot_allocator: veth_provisioner::NetSlotAllocator,
    /// Per-host stable per-`<job>` frontend-address allocator
    /// (dial-by-name-responder step 01-05; ADR-0072 REV-2/REV-3, GH #243).
    /// The SINGLE source of frontend truth (DDN-2): the ONE `Arc`-shared
    /// instance the deploy-time WRITER (the `submit_workload` Service arm +
    /// the boot rebuild) populates AND the `name_index` (01-03) / `by_frontend`
    /// (02-00) READERS observe. The 01-05 assign-on-declare writes the
    /// `<job> → F` binding; 02-01 LATER injects the SAME cloned instance into
    /// the `DnsResponder` + re-keyed `MtlsResolve` readers.
    ///
    /// Plain `Clone` value field (mirrors `net_slot_allocator`), NOT an outer
    /// `Arc<Mutex<…>>`: [`crate::dns_responder::frontend_addr_allocator::
    /// FrontendAddrAllocator`] holds its `Arc<Mutex<BTreeMap<…>>>` INTERNALLY,
    /// so a `.clone()` shares the same held map — exactly how the single
    /// instance is shared across writer + readers. Ephemeral runtime state,
    /// NEVER persisted: empty on a fresh boot, re-populated by the
    /// converge-on-boot rebuild
    /// ([`crate::dns_responder::boot_rebuild::rebuild_frontend_addrs_from_intent`])
    /// from the declared-Service intent SSOT.
    pub frontend_addr_allocator:
        crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator,
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

/// Test-only helper: build an empty
/// [`listener_facts::ListenerFactStore`] wrapped in
/// `Arc<tokio::sync::Mutex<...>>` for the `AppState` shape. Used by the
/// crate's `tests/acceptance/*` and `tests/integration/*` fixtures that
/// seed intent AFTER constructing `AppState` — for those the boot
/// rebuild would project an empty store regardless, so a fresh empty
/// store is the correct construction-time value. Fixtures that
/// specifically exercise the boot-rebuild path call
/// [`listener_facts::ListenerFactStore::rebuild_from_intent`] explicitly
/// after seeding (mirroring the production wiring's post-`bulk_load`
/// rebuild). Production callers go through `rebuild_from_intent` in
/// `run_server_with_obs_and_driver` directly.
#[must_use]
pub fn test_empty_listener_facts() -> Arc<tokio::sync::Mutex<listener_facts::ListenerFactStore>> {
    Arc::new(tokio::sync::Mutex::new(listener_facts::ListenerFactStore::new()))
}

/// Test-only helper: build a default [`workflow_runtime::WorkflowEngine`]
/// for the `AppState` shape (ADR-0064 §5). Wires an empty
/// [`workflow_runtime::WorkflowRegistry`] over an in-memory
/// `RedbJournalStore`, host `TcpTransport` / `OsEntropy`, and the
/// supplied `clock` + `obs`.
///
/// Used by the broad fixture surface that constructs `AppState` but does
/// NOT exercise the workflow primitive — an empty registry means a
/// committed `StartWorkflow` would surface
/// `WorkflowEngineError::UnknownWorkflow`, but those fixtures never emit
/// one. Fixtures that DO drive a workflow end-to-end (the step-01-08 e2e)
/// build their own engine inline with the `ProvisionRecord` factory
/// registered + a real on-disk journal. Production callers go through the
/// real engine composition in `run_server_with_obs_and_driver`.
///
/// The journal uses redb's in-memory backend so the helper needs no
/// tempdir and leaves no on-disk residue — correct for a non-durable
/// fixture default (the durable path is exercised by the e2e + the
/// `RedbJournalStore` integration test).
#[must_use]
pub fn test_default_workflow_engine(
    obs: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
) -> Arc<workflow_runtime::WorkflowEngine> {
    #[allow(clippy::expect_used)]
    let db = redb::Database::builder()
        .create_with_backend(redb::backends::InMemoryBackend::new())
        .expect("in-memory redb journal for test engine");
    let journal: Arc<dyn journal::JournalStore> =
        Arc::new(journal::RedbJournalStore::new(Arc::new(db)));
    Arc::new(workflow_runtime::WorkflowEngine::new(
        journal,
        clock,
        Arc::new(overdrive_host::TcpTransport::default()),
        Arc::new(overdrive_host::OsEntropy),
        workflow_runtime::WorkflowRegistry::new(),
        obs,
    ))
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
    ///
    /// The `listener_facts` parameter is likewise required at
    /// construction (no default, no builder) per the same rule: the boot
    /// wiring MUST rebuild it from the intent SSOT after the allocator's
    /// `bulk_load`, and making it mandatory means a forgotten rebuild
    /// fails to compile. Test fixtures that seed intent AFTER
    /// construction pass an empty
    /// `Arc::new(tokio::sync::Mutex::new(ListenerFactStore::new()))`.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "Port-trait dependencies (Clock, Driver, Dataplane, ObservationStore, IntentStore) plus the boot-rebuilt allocator + listener-fact projections are required at construction per .claude/rules/development.md § Port-trait dependencies; bundling them into a builder would make individual deps optional and defeat the explicit-injection invariant."
    )]
    pub fn new(
        store: Arc<LocalIntentStore>,
        intent_redb_path: PathBuf,
        obs: Arc<dyn ObservationStore>,
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        driver: Arc<dyn Driver>,
        clock: Arc<dyn Clock>,
        dataplane: Arc<dyn Dataplane>,
        ca: Arc<dyn Ca>,
        identity: Arc<IdentityMgr>,
        node_id: NodeId,
        allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
        listener_facts: Arc<tokio::sync::Mutex<crate::listener_facts::ListenerFactStore>>,
        host_ipv4: std::net::Ipv4Addr,
    ) -> Self {
        // Default-compose a `WorkflowEngine` with an EMPTY registry over an
        // in-memory journal (ADR-0064 §5). `new` is the broad fixture
        // convenience constructor: the vast majority of `AppState` callers
        // do not exercise the workflow primitive, so an empty-registry
        // engine is the correct default (a committed `StartWorkflow` for an
        // unregistered kind surfaces `WorkflowEngineError::UnknownWorkflow`,
        // but those callers never emit one). The PRODUCTION boot path and
        // the workflow end-to-end test use [`Self::new_with_workflow_engine`]
        // to inject the real engine (real on-disk journal + registered
        // workflows). This keeps the engine field mandatory (no `Option`
        // shim) while the convenience constructor stays
        // ripple-free for the fixture surface.
        let workflow_engine = test_default_workflow_engine(Arc::clone(&obs), Arc::clone(&clock));
        // The broad fixture surface has no transparent-mTLS layer (no real
        // `EbpfDataplane` to intercept on) — `None` mirrors the
        // empty-registry `WorkflowEngine` default above. The PRODUCTION
        // boot path (`run_server`) and the Tier-3 e2e use
        // [`Self::new_with_workflow_engine`] to inject `Some(worker)`.
        Self::new_with_workflow_engine(
            store,
            intent_redb_path,
            obs,
            runtime,
            driver,
            clock,
            dataplane,
            ca,
            identity,
            node_id,
            allocator,
            listener_facts,
            host_ipv4,
            workflow_engine,
            None,
        )
    }

    /// The full constructor that injects the [`workflow_runtime::WorkflowEngine`]
    /// explicitly (ADR-0064 §5). The production boot path and the
    /// end-to-end composition test use this so the real engine (on-disk
    /// journal + registered workflows) is threaded into `AppState`; the
    /// convenience [`Self::new`] default-composes an empty-registry engine
    /// for the broad fixture surface.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "Every port-trait dependency plus the workflow engine is required at construction per .claude/rules/development.md § Port-trait dependencies; bundling into a builder would make individual deps optional."
    )]
    pub fn new_with_workflow_engine(
        store: Arc<LocalIntentStore>,
        intent_redb_path: PathBuf,
        obs: Arc<dyn ObservationStore>,
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        driver: Arc<dyn Driver>,
        clock: Arc<dyn Clock>,
        dataplane: Arc<dyn Dataplane>,
        ca: Arc<dyn Ca>,
        identity: Arc<IdentityMgr>,
        node_id: NodeId,
        allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
        listener_facts: Arc<tokio::sync::Mutex<crate::listener_facts::ListenerFactStore>>,
        host_ipv4: std::net::Ipv4Addr,
        workflow_engine: Arc<workflow_runtime::WorkflowEngine>,
        mtls_worker: Option<Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>>,
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
            listener_facts,
            host_ipv4,
            workflow_engine,
            ca,
            identity,
            mtls_worker,
            // Default-construct the per-host slot allocator INSIDE the
            // constructor (transparent-mtls-enrollment D-TME-12 G3, step
            // 04-01) — NOT a constructor parameter. This is the same
            // ripple-avoidance `mtls_worker` (`None`) + `workflow_engine`
            // (empty registry) use: the ~42 non-mTLS fixtures and the
            // `reconciler_runtime`/`listener_facts` callers need no change.
            // On a fresh process boot nothing is held; still-Running allocs
            // re-assign on their next lifecycle pass (criterion 6).
            net_slot_allocator: veth_provisioner::NetSlotAllocator::new(),
            // Default-construct the per-host frontend-address allocator INSIDE
            // the constructor (dial-by-name-responder step 01-05) — NOT a
            // constructor parameter, same ripple-avoidance as
            // `net_slot_allocator`: it self-shares on clone, so every fixture
            // gets its own fresh empty allocator with no signature change. On a
            // fresh process boot nothing is held; the production boot's
            // converge-on-boot rebuild re-populates it from the declared-Service
            // intent SSOT, and the `submit_workload` Service arm assigns on
            // every new declare.
            frontend_addr_allocator:
                crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new(),
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

    /// Injected [`Kek`] provider used by the workload-identity CA boot path
    /// (`boot_ca` / `bootstrap_node_intermediate`) to resolve the
    /// `overdrive-ca-root` key-encryption key that seals the persistent root
    /// at rest (ADR-0063 D3/D6).
    ///
    /// **Mandatory by design — no `Default`, no `Option`-override.** Production
    /// composes `SystemdCredsKeyring::new()` (the Linux kernel keyring binding)
    /// at the CLI `serve` boundary; tests inject a hermetic in-process double
    /// (`overdrive_sim::adapters::SimKek::for_boot()`) so the boot's KEK-resolve
    /// probe succeeds with no `$CREDENTIALS_DIRECTORY` / kernel-keyring
    /// dependency. The field is mandatory specifically so a boot site that
    /// forgets the KEK **fails to compile** rather than silently inheriting the
    /// production binding and refusing to start in a cold environment — the
    /// exact regression this seam closes. See `.claude/rules/development.md`
    /// § "Port-trait dependencies" and feature-delta § C1-AMEND. Because a
    /// mandatory `Arc<dyn Kek>` cannot be defaulted to a benign value,
    /// `ServerConfig` has **no `Default` impl**; use
    /// [`ServerConfig::new`](Self::new) + the `..ServerConfig::new(kek)`
    /// rest-pattern instead.
    ///
    /// Excluded from [`Debug`] — `Arc<dyn Kek>` is not [`Debug`].
    ///
    /// [`Kek`]: overdrive_core::ca::kek::Kek
    pub kek: Arc<dyn overdrive_core::ca::kek::Kek>,

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
    /// `Some(DataplaneConfig::single_node_veth())` via the `Default`
    /// impl below so existing `..Default::default()` rest-pattern
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

    /// Test-only fault-injection seam for the transparent-mTLS proxy
    /// Earned-Trust probe (transparent-mtls-host-socket, step 06-03,
    /// criteria[0]). When `Some(msg)`, the boot path forces
    /// `MtlsEnforcement::probe()` to fail with the carried message BEFORE
    /// the mTLS layer is declared usable, so the
    /// `MtlsBootError::Probe`/`health.startup.refused` fail-closed branch
    /// is exercised without needing a real kernel substrate failure.
    /// Mirrors `dataplane_probe_fault` above; gated behind
    /// `#[cfg(feature = "integration-tests")]` on both the field and its
    /// use site so production builds compile it out entirely.
    #[cfg(feature = "integration-tests")]
    pub mtls_probe_fault: Option<String>,

    /// Test-only PKI-injection seam for the transparent-mTLS layer
    /// (transparent-mtls-host-socket, step 06-03, criteria[1]). When
    /// `Some(read)`, the boot composes `HostMtlsEnforcement` over THIS
    /// `IdentityRead` instead of the production `IdentityMgr` — so the
    /// agent's leg-B client SVID (`svid_for`) AND the leg-B
    /// `TrustBundle` (`current_bundle`) both come from a shared
    /// `TestPki` the e2e also roots its `OutboundPeer` server cert on.
    /// Without this seam the production `IdentityMgr` owns a fresh
    /// ephemeral workload-CA root, and the test peer cannot present a
    /// server cert the agent's leg-B verifier (root anchor + SNI
    /// `peer.overdrive.local`) accepts, so the handshake never completes
    /// and no `0x17` reaches the peer wire.
    ///
    /// Sibling to the existing `SimKek::for_boot()` boot injection (the
    /// criteria[0] test uses that); gated behind
    /// `#[cfg(feature = "integration-tests")]` on both the field and its
    /// single use site so production builds compile it out entirely and
    /// the production `IdentityMgr` is the only reachable identity
    /// source. This is the WORKLOAD-identity path (`RcgenCa` /
    /// `IdentityMgr`), NOT the operator HTTPS CA.
    #[cfg(feature = "integration-tests")]
    pub mtls_identity_override: Option<Arc<dyn overdrive_core::traits::IdentityRead>>,
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
            .field("kek", &"<dyn Kek>")
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
        #[cfg(feature = "integration-tests")]
        dbg.field("mtls_probe_fault", &self.mtls_probe_fault);
        #[cfg(feature = "integration-tests")]
        dbg.field(
            "mtls_identity_override",
            &self.mtls_identity_override.as_ref().map(|_| "<dyn IdentityRead>"),
        );
        dbg.finish()
    }
}

impl ServerConfig {
    /// Construct a `ServerConfig` with the **mandatory** `kek` provider and
    /// every other field set to its prior `Default` value.
    ///
    /// Replaces the removed `impl Default for ServerConfig`: the [`Kek`] port
    /// binding MUST be supplied explicitly (production composes
    /// `SystemdCredsKeyring::new()` at the CLI `serve` boundary; tests inject a
    /// hermetic `SimKek::for_boot()`) so a boot site that forgets it fails to
    /// **compile**, never inherits the production binding and refuses to start
    /// in a cold environment. See feature-delta § C1-AMEND and
    /// `.claude/rules/development.md` § "Port-trait dependencies".
    ///
    /// Fixtures swap `..Default::default()` → `..ServerConfig::new(test_kek)`:
    /// the rest-pattern still supplies every non-`kek` field, while `kek` is
    /// now an explicit, type-checked argument.
    ///
    /// `bind`, `data_dir`, and `operator_config_dir` get sentinel values that
    /// callers MUST override (as under the prior `Default`). `tick_cadence`
    /// defaults to [`reconciler_runtime::DEFAULT_TICK_CADENCE`] (100ms) and
    /// `clock` to `Arc::new(SystemClock)` from the [`overdrive_host`] crate —
    /// the only crate permitted to instantiate `SystemClock` per CLAUDE.md
    /// "Repository structure". Tests that need a controllable clock override
    /// `clock` in the same struct literal.
    ///
    /// [`Kek`]: overdrive_core::ca::kek::Kek
    #[must_use]
    pub fn new(kek: Arc<dyn overdrive_core::ca::kek::Kek>) -> Self {
        // 127.0.0.1:0 — IPv4 loopback, ephemeral port. Constructed
        // directly rather than via `parse()` so the constructor
        // is infallible and clippy's `expect_used` lint stays clean.
        let loopback = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
        Self {
            bind: loopback,
            data_dir: PathBuf::new(),
            operator_config_dir: PathBuf::new(),
            tick_cadence: DEFAULT_TICK_CADENCE,
            clock: Arc::new(overdrive_host::SystemClock),
            kek,
            node: overdrive_worker::NodeConfig::default(),
            vip_range: VipRange::default(),
            // ADR-0061 § 1 (step 01-03): `Default` populates the
            // veth-named single-node `[dataplane]` shape (two DISTINCT
            // ifaces `ovd-veth-cli` / `ovd-veth-bk`, NOT `lo`/`lo`) so
            // existing test fixtures using `..Default::default()`
            // rest-pattern construction and the production boot default
            // both carry the shape the serve-boot provisioner expects.
            // Production callers reading an operator TOML go through
            // `parse_dataplane_section` and overwrite this value.
            dataplane: Some(dataplane_config::DataplaneConfig::single_node_veth()),
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
            // transparent-mtls-host-socket step 06-03: default no
            // mTLS-probe fault; the e2e criteria[0] test sets
            // `Some(..)` to exercise the `MtlsBootError::Probe`
            // fail-closed branch.
            #[cfg(feature = "integration-tests")]
            mtls_probe_fault: None,
            // transparent-mtls-host-socket step 06-03: default no
            // identity override; the criteria[1] e2e sets `Some(..)`
            // to a `TestPki`-rooted `IdentityRead` so the agent's
            // leg-B trusts the test peer's server cert.
            #[cfg(feature = "integration-tests")]
            mtls_identity_override: None,
        }
    }
}

/// Handle to a running control-plane server.
///
/// Drop does NOT stop the server; call [`ServerHandle::shutdown`] to
/// drain in-flight requests, stop the convergence-loop spawn, and
/// close the listener. The server task runs until the handle is shut
/// down or the process exits.
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
    /// `JoinHandle` for the workflow emit-drain task — the production
    /// consumer of the [`workflow_runtime::WorkflowEngine`]'s Action
    /// channel. It takes the channel receiver once at boot and forwards
    /// every `ctx.emit_action`'d [`Action`](overdrive_core::reconcilers::Action)
    /// into [`action_shim::dispatch_with_workflow_intent`] (→ Raft). Per
    /// ADR-0064 §4 / brief.md §92; spawned in
    /// [`run_server_with_obs_and_driver`].
    emit_drain_task: tokio::task::JoinHandle<()>,
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
    /// Token observed by the workflow emit-drain task's `tokio::select!`
    /// loop. Cancelled in [`Self::shutdown`] alongside the convergence
    /// loop — the drain task holds an `AppState` clone (and through it
    /// `Arc<dyn Driver>` references), so it must stop dispatching emitted
    /// Actions before axum tears down `AppState`.
    emit_drain_shutdown: CancellationToken,
}

impl std::fmt::Debug for ServerHandle {
    /// Manual `Debug` (the derive was dropped when the test-gated
    /// `mtls_worker` field — `Option<Arc<MtlsInterceptWorker>>`, not
    /// `Debug` — was added; step 06-03). Elides the task handles and the
    /// worker, mirroring the prior derived shape's information value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHandle").finish_non_exhaustive()
    }
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

        // 1b. Cancel the workflow emit-drain loop and await its
        //     completion. It holds an `AppState` clone (and through it
        //     `Arc<dyn Driver>` references), so — like the convergence
        //     loop — it must stop dispatching emitted Actions before axum
        //     tears down `AppState`. The join waits for any in-flight
        //     dispatch of an emitted Action to finish through the shim.
        self.emit_drain_shutdown.cancel();
        let _ = self.emit_drain_task.await;

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

/// Apply the override-aware `LOCALHOST` fallback to a `host_ipv4`
/// resolution result.
///
/// Production (no dataplane override) propagates a resolution failure
/// verbatim — the control plane refuses to boot on an absent or
/// unresolvable client iface. An injected dataplane override
/// (Sim/DST/CLI with no provisioned veth) falls back to
/// `Ipv4Addr::LOCALHOST` on resolution failure so those callers can
/// boot without real network state. A successful resolution is used
/// as-is on both branches — the override never overrides a real IP.
///
/// See ADR-0053 and
/// `docs/analysis/root-cause-analysis-bridge-hydrator-register-local-backend.md`
/// for why the override must NOT collapse a successful resolution to
/// loopback (doing so makes the bridge write a loopback backend the
/// hydrator's loopback guard then rejects).
fn host_ipv4_with_override_fallback(
    resolved: Result<std::net::Ipv4Addr, error::ControlPlaneError>,
    has_dataplane_override: bool,
) -> Result<std::net::Ipv4Addr, error::ControlPlaneError> {
    match resolved {
        Ok(ip) => Ok(ip),
        Err(_) if has_dataplane_override => Ok(std::net::Ipv4Addr::LOCALHOST),
        Err(source) => Err(source),
    }
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

pub async fn run_server(
    config: ServerConfig,
    fs: Arc<dyn overdrive_core::traits::cgroup_fs::CgroupFs>,
) -> Result<ServerHandle, error::ControlPlaneError> {
    // Wire the Phase 1 observation store (`LocalObservationStore`
    // single-node per ADR-0012, revised 2026-04-24) internally and the
    // production `ExecDriver` from the worker subsystem (ADR-0029),
    // then delegate to `run_server_with_obs_and_driver`. The split
    // exists so integration tests can hold a shared `Arc<dyn ObservationStore>`
    // handle for the canary-injection Fixture-Theater defence without
    // introducing a test-only hook into the production boot path.
    //
    // Per ADR-0029 / ADR-0054 § Composition root wiring, this is the
    // binary-composition boundary. The CLI's `serve` subcommand
    // constructs the production cgroupfs adapter at boot, probes it,
    // and threads the SAME `Arc<dyn CgroupFs>` through here — the
    // probed substrate IS the used substrate (Earned Trust invariant).
    // Tests pass either the production adapter (Lima integration
    // suite, real `/sys/fs/cgroup`) or the sim adapter (DST / sim
    // path). The trait name `CgroupFs` from `overdrive-core` is fine
    // in this signature — control-plane already depends on
    // `overdrive-core` for every other port trait (`Clock`,
    // `Driver`, `IntentStore`, `ObservationStore`); the concrete
    // production binding is NOT named here.
    let obs: Arc<dyn ObservationStore> =
        Arc::from(observation_wiring::wire_single_node_observation(&config.data_dir)?);

    // Per ADR-0028 (as superseded in part by ADR-0034), run the cgroup
    // v2 delegation pre-flight at the start of the boot path — BEFORE
    // any on-disk side effects. The preflight uses direct `std::fs`
    // reads (no `CgroupFs` port dependency) and must execute before
    // the workloads-slice bootstrap below, which creates directories
    // and writes `cgroup.subtree_control` on real cgroupfs. Without
    // this ordering, a misconfigured host sees
    // `WorkloadsBootstrap(WriteFailed: PermissionDenied)` instead of
    // the actionable `CgroupBootstrap(DelegationMissing)` message.
    cgroup_preflight::run_preflight().map_err(error::ControlPlaneError::from)?;

    // Per the cgroup v2 kernel contract, a parent's `subtree_control`
    // must delegate controllers BEFORE any child can enable them in
    // its own `subtree_control`. `create_and_enrol_control_plane_slice`
    // writes `+cpu +memory +io +pids` to
    // `overdrive.slice/cgroup.subtree_control` (step 2 of its
    // four-step sequence), which is a prerequisite for the
    // workloads-slice bootstrap below. Order is load-bearing: moving
    // this call after the workloads bootstrap produces ENOENT on the
    // child's `subtree_control` write because the kernel does not see
    // the controllers at the parent level.
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
    let cgroup_root_path = std::path::PathBuf::from(cgroup_preflight::DEFAULT_CGROUP_ROOT);
    let bootstrap_manager =
        overdrive_worker::cgroup_manager::CgroupManager::new(cgroup_root_path.clone(), fs.clone());
    bootstrap_manager
        .create_workloads_slice_with_controllers()
        .await
        .map_err(error::ControlPlaneError::from)?;

    // Service-health-check-probes step 01-03d / ADR-0054 § 7 — the
    // probe-runner Earned-Trust gate runs here, at the binary
    // composition root, and the resulting `Arc<ProbeRunner>` is
    // threaded into the production `ExecDriver` via the
    // `compose_production_driver` helper below. Acceptance test
    // `probe_runner_composition` drives the helper with `SimProber`
    // adapters to assert the threading structurally — closes
    // GAP-4 + GAP-5 from `.context/01-03-structural-gap-audit.md`.
    // The `ProbeRunner` half of the composed pair is intentionally
    // discarded here: the driver retains a clone of the `Arc` inside
    // its `with_probe_runner(...)` field, so the supervisor map stays
    // alive for the driver's lifetime. Destructuring the second
    // tuple slot with `_` (NOT `_probe_runner`) makes the discard
    // local + intentional and keeps the binary structurally distinct
    // from the pre-patch shape that the dst-lint
    // `underscore-binding-probe-runner` clause guards against.
    //
    // The driver shares the SAME cgroup root + probed `Arc<dyn CgroupFs>`
    // substrate the workloads-slice bootstrap above used (Earned Trust
    // invariant per ADR-0054 § Composition root wiring): `cgroup_root_path`
    // and `fs` are threaded through rather than re-deriving the literal
    // `/sys/fs/cgroup`.
    let (driver, _) = compose_production_driver(
        Arc::new(overdrive_worker::probe_runner::TokioTcpProber::new()),
        Arc::new(overdrive_worker::probe_runner::HyperHttpProber::new()),
        Arc::new(overdrive_worker::probe_runner::CgroupExecProber::new(Arc::clone(&fs))),
        cgroup_root_path,
        Arc::new(overdrive_host::SystemClock),
        fs,
        Arc::clone(&obs),
    )
    .await?;

    run_server_with_obs_and_driver(config, obs, driver).await
}

/// Compose the production `ExecDriver` with its Earned-Trust-vetted
/// `Arc<ProbeRunner>` already threaded via `with_probe_runner(...)`.
///
/// This is the single composition site for the production driver +
/// probe-runner threading per service-health-check-probes
/// step 01-03d / ADR-0054 § 7 + § 2. The Earned-Trust gate
/// (`probe_runner_boot::compose_and_probe_runner_gate`) runs FIRST;
/// on success the returned `Arc<ProbeRunner>` is cloned into the
/// driver builder so `ExecDriver::on_alloc_running` /
/// `on_alloc_terminal` fire the per-alloc supervisor lifecycle in
/// production. Without this threading the driver's lifecycle hooks
/// fall through to the trait-default no-op and the probe subsystem
/// is structurally dead — the failure mode `.context/01-03-structural-
/// gap-audit.md` GAP-4 + GAP-5 documents.
///
/// Returns `(Arc<dyn Driver>, Arc<ProbeRunner>)` so acceptance tests
/// can capture both halves of the composition and assert on
/// observable state (`runner.active_alloc_count()` before and after
/// `driver.on_alloc_running(...)` fires).
///
/// # Errors
///
/// Propagates `ControlPlaneError::ProbeRunnerBoot` from the
/// Earned-Trust gate — the helper emits the canonical
/// `health.startup.refused` tracing event before returning so the
/// CLI binary boundary surfaces a structured refusal.
pub async fn compose_production_driver(
    tcp_prober: Arc<dyn overdrive_core::traits::prober::TcpProber>,
    http_prober: Arc<dyn overdrive_core::traits::prober::HttpProber>,
    exec_prober: Arc<dyn overdrive_core::traits::prober::ExecProber>,
    cgroup_root: std::path::PathBuf,
    clock: Arc<dyn Clock>,
    fs: Arc<dyn overdrive_core::traits::cgroup_fs::CgroupFs>,
    observation_store: Arc<dyn ObservationStore>,
) -> Result<
    (Arc<dyn Driver>, Arc<overdrive_worker::probe_runner::ProbeRunner>),
    error::ControlPlaneError,
> {
    let probe_runner = probe_runner_boot::compose_and_probe_runner_gate(
        tcp_prober,
        http_prober,
        exec_prober,
        Arc::clone(&clock),
        observation_store,
    )
    .await?;

    let driver: Arc<dyn Driver> = Arc::new(
        overdrive_worker::ExecDriver::new(cgroup_root, clock, fs)
            .with_probe_runner(Arc::clone(&probe_runner)),
    );

    Ok((driver, probe_runner))
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
    // ADR-0028 preflight, parent-slice delegation, and workloads-slice
    // bootstrap all run in `run_server` (the outer composition
    // boundary). Tests that compose `run_server_with_obs_and_driver`
    // directly are responsible for running these themselves if they
    // need real cgroupfs (typically they run on a properly configured
    // Lima VM — see the integration suite under
    // `crates/overdrive-control-plane/tests/integration/cgroup_isolation/`
    // and `crates/overdrive-worker/tests/integration/exec_driver/`).

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
    // ADR-0064 §5 — the pure-sync workflow-lifecycle reconciler. Re-emits
    // `Action::StartWorkflow` for a running-in-intent instance with no live
    // engine task on restart; the engine (wired into dispatch below)
    // drives `run` off the shim. `ReconcilerIsPure` holds with it
    // registered (the reconcile body holds no `.await`).
    runtime.register(workflow_lifecycle()).await?;
    // ADR-0067 D1 — the pure-sync svid-lifecycle reconciler. Converges
    // `desired = running allocs` against `actual = the IdentityMgr held set`
    // and emits `Action::IssueSvid` / `Action::DropSvid`; the action-shim
    // executor (01-06) drives the CA I/O off the shim. `ReconcilerIsPure`
    // holds with it registered (the reconcile body holds no `.await`, reaches
    // for no CA / ObservationStore handle, and reads no wall-clock).
    runtime.register(svid_lifecycle()).await?;

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
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> = if let Some(overridden) =
        config.dataplane_override.clone()
    {
        overridden
    } else {
        // ADR-0061 § 3 (step 01-03): single-node serve-boot
        // auto-provisions the host-netns veth pair BEFORE
        // `EbpfDataplane::new`, extending the ADR-0052
        // parse->construct->probe->use sequence ADDITIVELY to
        // provision->construct->probe->use.
        //
        // CRITICAL GATING (ADR-0061 § 1 / feature-delta § 6.4): the
        // provisioner fires ONLY when the configured ifaces are the
        // DEFAULT veth names (`ovd-veth-cli` / `ovd-veth-bk`). When
        // an operator names REAL NICs (any other names) we SKIP
        // provision entirely — the existing two-NIC boot resolution
        // is unchanged and must not regress (an `ip addr add`
        // VIP-gateway onto a real NIC + `ip route add` over it would
        // be wrong). The default-veth-name check IS the implicit
        // gate; the explicit `[dataplane] provision = "veth"|"none"`
        // opt-out knob is deferred to issue #194.
        let is_default_veth = dataplane_cfg.client_iface == veth_provisioner::DEFAULT_CLIENT_IFACE
            && dataplane_cfg.backend_iface == veth_provisioner::DEFAULT_BACKEND_IFACE;
        if is_default_veth {
            let plan = veth_provisioner::derive_veth_plan(
                &dataplane_cfg.client_iface,
                &dataplane_cfg.backend_iface,
                config.vip_range.first_range(),
            );
            if let Err(source) = veth_provisioner::provision(&plan) {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "dataplane.provision",
                    client_iface = %dataplane_cfg.client_iface,
                    backend_iface = %dataplane_cfg.backend_iface,
                    error = %source,
                    "single-node veth provisioning failed; refusing to boot"
                );
                return Err(error::ControlPlaneError::DataplaneBoot(
                    error::DataplaneBootError::Provision { source },
                ));
            }
        }

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
    // operator-supplied `client_iface`.
    //
    // The loopback default is keyed on *iface-resolution failure*,
    // never on `dataplane_override` presence. The override knob is too
    // coarse a proxy for "Sim/loopback boot": it carries TWO cases that
    // need OPPOSITE host_ipv4 resolution (RCA
    // docs/analysis/root-cause-analysis-bridge-hydrator-register-local-backend.md):
    //
    //   1. Sim/DST/CLI boots inject a SimDataplane override and never
    //      provision the `client_iface` (`ovd-veth-cli` in the default
    //      config), or carry no `[dataplane]` section at all. Resolving
    //      the iface fails → `LOCALHOST` is correct (these boots only
    //      seed the bridge/hydrator local-backend socket addresses and
    //      do not exercise the real attach path).
    //   2. The S-BDB walking-skeletons inject a REAL `EbpfDataplane` on
    //      a REAL provisioned veth (`10.244.x.1`) through the SAME
    //      override field. Resolving the iface SUCCEEDS → the real IP is
    //      correct; collapsing it to `LOCALHOST` makes the bridge write
    //      a loopback backend that the hydrator's (correct, intended)
    //      loopback guard then rejects, so no `RegisterLocalBackend` is
    //      ever emitted.
    //
    // Resolve from `client_iface` first; fall back to `LOCALHOST` only
    // when resolution fails AND an override is present (the Sim/CLI/DST
    // condition). The production path (no override) still propagates the
    // error and refuses to boot on an absent/unresolvable iface.
    let host_ipv4 = host_ipv4_with_override_fallback(
        resolve_host_ipv4_from_dataplane_config(config.dataplane.as_ref()),
        config.dataplane_override.is_some(),
    )?;
    // Service-health-check-probes — the `ProbeRunner` Earned-Trust
    // gate and the threading of `Arc<ProbeRunner>` into the
    // production `ExecDriver` now live at the binary composition
    // root (`run_server`) per ADR-0054 § 7. This split function
    // accepts a caller-supplied driver and intentionally bypasses
    // the gate: production drivers come in already-wired (the
    // composition-root threaded the runner via
    // `ExecDriver::with_probe_runner(...)` before delegating here);
    // test callers that pass a non-`ExecDriver` (`SimDriver` etc.)
    // are not exercising the probe path. A test caller that
    // genuinely needs the gate's behaviour with a custom driver
    // calls `probe_runner_boot::compose_and_probe_runner_gate(...)`
    // itself and applies `.with_probe_runner(...)` before passing
    // the driver in.

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
    // Service-health-check-probes step 01-03d — register the
    // `service-lifecycle` reconciler via the `AnyReconciler::
    // ServiceLifecycle` dispatch enum landed in step 01-03b
    // (commit 087bada4). Phase-1 reconcile-body branches (Stable,
    // EarlyExit, StartupProbeFailed, idempotent-no-op) landed in
    // step 01-03 (commit 2fabf259); the readiness / liveness
    // arms land in slices 04 / 05.
    runtime.register(service_lifecycle()).await?;
    let runtime = Arc::new(runtime);

    let allocator = bulk_load_service_vip_allocator(&config.vip_range, &store).await?;

    // ORDERING IS LOAD-BEARING (ADR-0062 § Decision (1)): the
    // listener-fact projection is boot-rebuilt from the intent SSOT
    // IMMEDIATELY AFTER the allocator's `bulk_load`, because the rebuild
    // joins each Service's allocator-issued VIP — a Service whose VIP the
    // allocator has not yet issued is skipped. Building the store here,
    // before assembling `AppState`, lets it be threaded into the
    // constructor as a mandatory field (no default, no post-hoc set).
    let listener_facts = Arc::new(tokio::sync::Mutex::new(
        listener_facts::ListenerFactStore::rebuild_from_intent(&store, &store_path, &allocator)
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "overdrive::health",
                    event = "health.startup.refused",
                    cause = %e,
                    "listener-fact projection rebuild refused; control-plane will not start"
                );
                error::ControlPlaneError::ListenerFactRebuild(Box::new(e))
            })?,
    ));

    // ADR-0064 §5 — compose the durable-async WorkflowEngine into the
    // production AppState. The engine is built over the real
    // `RedbJournalStore` (one redb file per node at
    // `<data_dir>/workflow-journal.redb`, K5) + the injected `Clock`
    // (same one the convergence loop uses) + host `TcpTransport` /
    // `OsEntropy` for the workflow `ctx.call` / RNG await-surfaces +
    // the `ObservationStore` the engine writes terminal rows to.
    //
    // Phase 1 has no first-party production workflows (#206 CLI verb +
    // Phase-3 consumers are the future producers), so the registry is
    // empty: a committed `StartWorkflow` for an unregistered kind would
    // surface `WorkflowEngineError::UnknownWorkflow` rather than silently
    // no-op. The composition itself is what 01-08 makes real — the
    // engine is now reachable in the production binary, replacing the
    // 01-05/01-06 `dispatch(... None)` placeholder.
    let journal_path = config.data_dir.join("workflow-journal.redb");
    let journal_db = Arc::new(
        redb::Database::create(&journal_path)
            .map_err(|e| error::ControlPlaneError::internal("create workflow-journal redb", e))?,
    );
    let journal: Arc<dyn journal::JournalStore> =
        Arc::new(journal::RedbJournalStore::new(journal_db));
    let workflow_engine = Arc::new(workflow_runtime::WorkflowEngine::new(
        journal,
        config.clock.clone(),
        Arc::new(overdrive_host::TcpTransport::default()),
        Arc::new(overdrive_host::OsEntropy),
        workflow_runtime::WorkflowRegistry::new(),
        Arc::clone(&obs),
    ));

    // Persistent, KEK-sealed workload-identity root (#215 boot-side; closes
    // D-OC-4). Single-cut replacement of the prior EPHEMERAL per-boot root:
    // `boot_ca` runs the Earned-Trust probes (KEK-resolve (a), envelope-decrypt
    // (b)) and generate-or-adopt, then `bootstrap_node_intermediate` does the
    // same for the node intermediate — both threading the real redb store path
    // so a refuse-to-start remediation names the actual file to inspect/delete.
    // A boot failure propagates through `?` as the typed
    // `ControlPlaneError::CaBoot` (the `#[from]` landed in 02-01) — never
    // flattened to `Internal` — surfacing its cause-distinct message. The
    // adopted root/intermediate are cached inside the `RcgenCa`, so the trust
    // bundle below is built from the ADOPTED root (not a fresh per-boot one),
    // seeding `IdentityMgr` relying-party verification from boot. The `IssueSvid`
    // executor (the ONE place workload-CA I/O happens) mints leaves off this
    // adopted CA on demand. This is the WORKLOAD-identity CA only; the separate
    // operator/control-plane HTTPS CA (`mint_ephemeral_ca`) is ephemeral by
    // design and untouched.
    let ca_subject = overdrive_core::SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .unwrap_or_else(|e| unreachable!("CA trust-domain subject is a valid SPIFFE URI: {e}"));
    let ca: Arc<dyn Ca> =
        Arc::new(overdrive_host::ca::RcgenCa::new(Arc::new(overdrive_host::OsEntropy), ca_subject));

    // The `Kek` provider is INJECTED through `config.kek` (§ C1-AMEND), NOT
    // constructed inline. Production composes `SystemdCredsKeyring::new()` at
    // the CLI `serve` boundary; tests inject a hermetic `SimKek::for_boot()`.
    // Inline construction here (the prior `SystemdCredsKeyring::new()`) forced
    // the production kernel-keyring binding on every test fixture and refused
    // to boot in a cold environment — the regression this seam closes. Both
    // `boot_ca` and `bootstrap_node_intermediate` already take `&dyn Kek`, so
    // only the SOURCE of the `&dyn Kek` changes (REUSE-AS-IS).
    let codec = overdrive_host::ca::RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();
    let intent_store: Arc<dyn IntentStore> = Arc::clone(&store) as Arc<dyn IntentStore>;

    let _root = ca_boot::boot_ca(
        ca.as_ref(),
        config.kek.as_ref(),
        &kek_id,
        &codec,
        &intent_store,
        &store_path,
    )
    .await?;
    let _intermediate = ca_boot::bootstrap_node_intermediate(
        ca.as_ref(),
        &node_id,
        &intent_store,
        config.kek.as_ref(),
        &kek_id,
        &codec,
        &store_path,
    )
    .await?;

    // Bundle is built from the ADOPTED CA (the `RcgenCa` now holds the
    // persistent root/intermediate); `IdentityMgr` seeds relying-party
    // verification from boot.
    let bundle = ca.trust_bundle()?;
    let identity: Arc<IdentityMgr> = Arc::new(IdentityMgr::new(Some(bundle)));

    // transparent-mtls-host-socket (D-MTLS-16/17, GH #26; step 06-03) —
    // compose the production transparent-mTLS layer HERE, AFTER
    // `IdentityMgr` (so `HostMtlsEnforcement` can read the held identity)
    // and BEFORE `AppState`. This is the (3a) resequencing: the mTLS port
    // is NOT a `compose_production_driver` param (that runs before
    // `IdentityMgr`); instead a separate `MtlsInterceptWorker` is
    // constructed here with both ports as REQUIRED params and threaded
    // into `AppState` as the `Option` field the action-shim fires.
    //
    // GATED: `Some(worker)` on the production boot
    // (`dataplane_override.is_none()` — a real `EbpfDataplane` was
    // constructed above) OR, under `integration-tests`, when the test-only
    // `mtls_probe_fault` seam opts in. The mTLS BPF load is INDEPENDENT of
    // the LB `EbpfDataplane` (D-MTLS-17 item 1 — its OWN `aya::Ebpf`), so a
    // Tier-3 gate test MAY inject `SimDataplane` for the LB path (dodging
    // the `lo` XDP attach that DRV_MODE rejects under virtio) while STILL
    // composing the real `MtlsDataplane` to exercise the fail-closed
    // refusal. The 42 non-mTLS fixtures call `AppState::new` directly
    // (bypassing this boot path) and are unaffected either way.
    //
    // wire → probe → use (fail-closed): construct `HostMtlsEnforcement` +
    // `HostMtlsEnforcement::probe()`. On probe failure the node REFUSES to
    // boot with `health.startup.refused` — it does NOT degrade to a cleartext
    // path (the confidentiality invariant the feature rests on). As of step
    // 04-01 (ADR-0071 Path A) there is no `MtlsDataplane::load` step: the
    // OUTBOUND intercept is the per-veth egress nft-TPROXY rule installed
    // per-alloc by `start_alloc`, NOT a cgroup-attached BPF program, so the
    // worker holds no BPF object to load at boot. SERVICE_MAP is pinned
    // independently and earlier by `EbpfDataplane::new_with_pin_dir`.
    #[cfg(feature = "integration-tests")]
    let compose_mtls = config.dataplane_override.is_none() || config.mtls_probe_fault.is_some();
    #[cfg(not(feature = "integration-tests"))]
    let compose_mtls = config.dataplane_override.is_none();
    let mtls_worker: Option<Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>> =
        if compose_mtls {
            // (1) construct the enforcement port over the held identity +
            // the F7 limits. `IdentityMgr` impls `IdentityRead`.
            //
            // PKI-SEAM (transparent-mtls-host-socket step 06-03,
            // criteria[1]): when the test-only `mtls_identity_override`
            // is `Some`, the agent reads its leg-B SVID + `TrustBundle`
            // from THAT `IdentityRead` (a `TestPki`-rooted double the
            // e2e also roots its `OutboundPeer` server cert on) instead
            // of the production `IdentityMgr`, so the leg-B handshake
            // against the test peer completes. Production builds compile
            // the override out (the field is `cfg`-gated), so the
            // production `IdentityMgr` is the only reachable source.
            #[cfg(feature = "integration-tests")]
            let mtls_identity: Arc<dyn overdrive_core::traits::IdentityRead> =
                config.mtls_identity_override.clone().unwrap_or_else(|| {
                    Arc::clone(&identity) as Arc<dyn overdrive_core::traits::IdentityRead>
                });
            #[cfg(not(feature = "integration-tests"))]
            let mtls_identity: Arc<dyn overdrive_core::traits::IdentityRead> =
                Arc::clone(&identity) as Arc<dyn overdrive_core::traits::IdentityRead>;
            let enforcement: Arc<dyn overdrive_core::traits::mtls_enforcement::MtlsEnforcement> =
                Arc::new(overdrive_dataplane::mtls::HostMtlsEnforcement::new(
                    mtls_identity,
                    overdrive_core::traits::mtls_enforcement::MtlsLimits::default(),
                ));

            // (2) probe (Earned Trust): the test-only `mtls_probe_fault` seam
            // forces a probe failure so criteria[0] exercises the fail-closed
            // refusal without a real substrate fault; otherwise the real
            // `probe()` runs. Either failure → refuse to boot.
            #[cfg(feature = "integration-tests")]
            let forced_probe_fault = config.mtls_probe_fault.clone();
            #[cfg(not(feature = "integration-tests"))]
            let forced_probe_fault: Option<String> = None;

            if let Some(message) = forced_probe_fault {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "mtls.probe",
                    error = %message,
                    "transparent-mTLS proxy probe failed (injected fault); \
                     refusing to boot (no cleartext fallback)"
                );
                return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::Probe {
                source:
                    overdrive_core::traits::mtls_enforcement::MtlsEnforcementError::Probe {
                        which:
                            overdrive_core::traits::mtls_enforcement::ProbeSentinel::KtlsArmRoundTrip,
                        message,
                    },
            }));
            }
            if let Err(source) = enforcement.probe().await {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "mtls.probe",
                    error = %source,
                    "transparent-mTLS proxy probe failed; refusing to boot (no cleartext fallback)"
                );
                return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::Probe {
                    source,
                }));
            }

            // (3) construct the per-connection enrollment-resolve adapter
            // (`ServiceBackendsResolve`, ADR-0071 / D-TME-11) over the
            // `ObservationStore` and run its Earned-Trust probe BEFORE the
            // worker (and therefore before any connection is resolved). The
            // List-at-probe leg seeds the in-RAM addr→Backend index from the
            // authoritative `service_backends` snapshot (capturing rows written
            // before boot — e.g. on a control-plane restart) and opens the
            // single-owner watch; on an unreadable store the probe refuses to
            // boot fail-closed (`health.startup.refused`) rather than serve an
            // empty-but-trusted index that would degrade to silent cleartext.
            // wire → probe → use (principle 12).
            let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> = Arc::new(
                crate::mtls_resolve_adapter::ServiceBackendsResolve::new(Arc::clone(&obs)),
            );
            if let Err(source) = resolve.probe().await {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "mtls.resolve.probe",
                    error = %source,
                    "transparent-mTLS resolve probe failed; refusing to boot (no cleartext fallback)"
                );
                return Err(error::ControlPlaneError::MtlsBoot(
                    error::MtlsBootError::ResolveProbe { source },
                ));
            }

            // (4) construct the worker with all three ports as REQUIRED params
            // (mandatory `new()`, no builder). As of step 04-01 (ADR-0071 Path
            // A) the worker holds no `MtlsDataplane` and no cgroup root — the
            // OUTBOUND egress nft-TPROXY rule is installed per-alloc by
            // `start_alloc` against the host-veth NAME carried on
            // `AllocationSpec.host_veth` (set at the action-shim C3 provision
            // seam, JOIN-6). As of step 04-02 the worker also holds the
            // probed-Ok `MtlsResolve` adapter — the outbound accept loop
            // resolves each captured connection's recovered `orig_dst` through
            // it (the C1 3-arm decision).
            Some(Arc::new(overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::new(
                enforcement,
                resolve,
                config.clock.clone(),
            )))
        } else {
            None
        };

    let state: AppState = AppState::new_with_workflow_engine(
        store,
        store_path,
        obs,
        runtime,
        driver,
        config.clock.clone(),
        dataplane,
        ca,
        identity,
        node_id,
        allocator,
        listener_facts,
        host_ipv4,
        workflow_engine,
        // transparent-mtls-host-socket step 06-03: `Some(worker)` on the
        // production / Tier-3 boot (real dataplane), `None` under a
        // `SimDataplane` override.
        mtls_worker,
    );

    // Adopt-on-restart boot recovery (transparent-mtls-enrollment step 04-04,
    // D-TME-12 §1–§4). On a `serve` restart the in-RAM `NetSlotAllocator` map
    // is reconstructed EMPTY, but workloads SURVIVE in their old
    // `ovd-ns-<slot>` netns (setsid + kill_on_drop(false) + own cgroup scope —
    // SPIKE-A) and `WorkloadLifecycle::reconcile` does NOT re-drive a Running
    // survivor (SPIKE-B), so this dedicated boot pass is the ONLY trigger that
    // rebuilds the lost slot↔alloc map. It runs AFTER `AppState` construction
    // (so `state.net_slot_allocator` / `state.obs` are available) and BEFORE
    // the convergence loop / exit-observer spawn (so the first smallest-free
    // `assign` cannot hand a surviving slot to a new alloc — the cross-restart
    // B1 collision). Gated by `state.mtls_worker.is_some()` — the same
    // composition gate G1 uses — so it is a no-op on a non-mTLS boot where no
    // per-alloc netns exist. A `NetSlotAdoptConflict` (two survivors on one
    // slot — a fatal correlation bug) refuses the boot via
    // `health.startup.refused`, reason `netns.adopt`.
    if state.mtls_worker.is_some() {
        if let Err(source) = veth_provisioner::adopt_on_restart_recovery(
            state.obs.as_ref(),
            &state.net_slot_allocator,
            std::path::Path::new(cgroup_preflight::DEFAULT_CGROUP_ROOT),
        )
        .await
        {
            tracing::warn!(
                name: "health.startup.refused",
                reason = "netns.adopt",
                error = %source,
                "adopt-on-restart boot recovery failed; refusing to boot \
                 (a surviving slot↔alloc map could not be rebuilt)"
            );
            return Err(error::ControlPlaneError::NetnsRecovery(source));
        }

        // §5 (D-TME-12; folds 03-01 review finding D2): after the netns
        // adopt+GC, SWEEP every surviving per-workload nft-TPROXY rule from the
        // shared `overdrive-mtls prerouting` chain. Each per-workload rule was
        // appended once and is NEVER torn down per-workload, so it SURVIVES the
        // restart — but its in-RAM RAII guard was lost (the CP died; `Drop`
        // never ran), and the rule now redirects to a dead leg-C/leg-F listener.
        // Unlike the surviving netns (ADOPTED above — the workload lives in it),
        // the surviving rule is DEAD weight (it points at a dead listener), so
        // the boot pass REAPS it, leaving the shared infra (F5 exemption,
        // table+chain) untouched. The clean re-install at `start_alloc` then
        // appends exactly one rule per direction. PINNED order: adopt → GC →
        // sweep → serve. A sweep failure (a by-handle `nft delete` error)
        // refuses the boot, same fail-closed posture as the adopt conflict.
        match overdrive_worker::mtls_intercept::sweep_per_workload_tproxy_rules() {
            Ok(swept) => {
                tracing::info!(
                    name: "mtls.boot.swept_per_workload_rules",
                    swept,
                    "adopt-on-restart §5: swept {swept} surviving per-workload nft-TPROXY \
                     rule(s) from the shared chain (shared infra left intact)"
                );
            }
            Err(source) => {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "nft.sweep",
                    error = %source,
                    "adopt-on-restart §5 nft-rule sweep failed; refusing to boot \
                     (surviving per-workload TPROXY rules could not be reaped)"
                );
                return Err(error::ControlPlaneError::NftRuleSweep(source));
            }
        }

        // Converge-on-boot frontend-address rebuild (dial-by-name-responder step
        // 01-05; ADR-0072 REV-3, GH #243). The `FrontendAddrAllocator` is
        // reconstructed EMPTY on every fresh boot (ephemeral, no cross-restart
        // persistence — the `NetSlotAllocator` model). This Bar-1 converge-on-boot
        // pass re-derives every `<job> → F` binding from the declared-Service intent
        // SSOT (`.claude/rules/reconcilers.md` § "Bar 1"; the same `workloads/`
        // intent scan as `ListenerFactStore::rebuild_from_intent`). It runs AFTER
        // the netns adopt + nft sweep above (preserving the PINNED boot order
        // adopt → GC → sweep → rebuild → serve) and BEFORE the convergence loop /
        // responder serve spawn (so the `name_index` reader the responder reads —
        // once 02-01 injects the shared instance — never observes an
        // empty-but-trusted allocator).
        //
        // GATED on `state.mtls_worker.is_some()` — the SAME composition gate the
        // netns adopt above uses, and (the load-bearing reason) the SAME gate the
        // 02-01 responder + its `name_index` reader are themselves built behind
        // (feature-delta DDN-6; the responder is constructed inside this very
        // real-dataplane block). On a non-mTLS boot there is therefore NO
        // responder and NO reader to serve — populating the allocator there is
        // wasted work, not a fix. Re-gating restores the roadmap 01-05 pin
        // ("gated by the SAME `mtls_worker.is_some()` block") AND keeps the
        // rebuild and its only consumer behind one gate. A rebuild failure
        // (unreadable intent SSOT, or frontend-block exhaustion mid-rebuild)
        // refuses the boot fail-closed via the typed
        // `ControlPlaneError::FrontendRebuild` (never flattened to `Internal`).
        crate::dns_responder::boot_rebuild::rebuild_frontend_addrs_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.frontend_addr_allocator,
        )
        .await?;
    }

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

    // Spawn the workflow emit-drain task (ADR-0064 §4; brief.md §92).
    // This is the production CONSUMER of the engine's Action channel —
    // it takes the receiver once and forwards every `ctx.emit_action`'d
    // Action into the SAME `action_shim::dispatch_with_workflow_intent`
    // path a reconciler-emitted Action takes (→ Raft). Without this spawn,
    // an emitted Action would be undrained in production — sent on the
    // engine's channel but never reaching the commit path (the gap step
    // 03-03 closes). Phase 1 has no first-party emit producer (#206 CLI
    // verb + Phase-3 consumers are the future producers), so the drain is
    // idle until an emitting workflow runs; the wiring is what makes the
    // emit path complete end-to-end.
    let emit_drain_shutdown = CancellationToken::new();
    let emit_drain_task =
        spawn_workflow_emit_drain(state.clone(), config.clock.clone(), emit_drain_shutdown.clone());

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
        emit_drain_task,
        convergence_shutdown,
        exit_observer_shutdown,
        emit_drain_shutdown,
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

/// Spawn the workflow emit-drain task — the production consumer of the
/// [`WorkflowEngine`]'s Action channel (ADR-0064 §4; brief.md §92).
///
/// Step 03-01 built `ctx.emit_action` so an emitting workflow SENDS its
/// typed [`Action`] on the engine-owned Action channel, but the channel
/// receiver was never taken in production — so an emitted Action was
/// undrained, never reaching the action-shim/Raft commit path. This task
/// closes that gap: it takes the receiver ONCE (single-shot per
/// [`WorkflowEngine::take_action_emit_receiver`]) and, for each emitted
/// [`Action`], forwards it into the SAME production dispatch path a
/// reconciler-emitted Action takes —
/// [`action_shim::dispatch_with_workflow_intent`] threaded the real engine
/// from `state.workflow_engine`. NOT a direct `IntentStore` write, NOT a
/// parallel undrained channel: the emit reaches Raft through the action
/// shim exactly as `development.md` § "Workflow contract" rule 6 requires.
///
/// The per-Action [`TickContext`] is constructed from the SAME injected
/// [`Clock`](overdrive_core::traits::clock::Clock) the convergence loop
/// sources — `state.clock` — never `SystemTime::now()` (dst-lint
/// enforces). An emitted `StartWorkflow` therefore re-enters the shim's
/// `StartWorkflow` arm off the engine, an emitted cluster mutation reaches
/// the same write path, etc. — the emit is a first-class action on the
/// production commit path.
///
/// Cancellation via `shutdown` is observed in `tokio::select!` between
/// drained items so an in-flight dispatch always completes before exit;
/// the task also exits when the channel closes (the engine is dropped).
///
/// If the receiver was already taken (e.g. a test harness took it first),
/// the task is a no-op and returns immediately — the single-shot take
/// yields `None`.
pub fn spawn_workflow_emit_drain(
    state: AppState,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    use overdrive_core::UnixInstant;
    use overdrive_core::reconcilers::TickContext;
    tokio::spawn(async move {
        // Single-shot take of the engine's Action-channel receiver. The
        // engine owns BOTH halves; this is the production consumer taking
        // the receiver exactly once at boot.
        let Some(mut rx) = state.workflow_engine.take_action_emit_receiver().await else {
            // Already taken (no production producer wired, or a test took
            // it first). Nothing to drain.
            return;
        };
        let mut tick_n: u64 = 0;
        loop {
            tokio::select! {
                maybe_action = rx.recv() => {
                    let Some(action) = maybe_action else {
                        // Channel closed — the engine (and thus every
                        // sender clone) was dropped. No more emits will
                        // arrive; exit cleanly.
                        break;
                    };
                    // Build a fresh per-Action TickContext from the injected
                    // Clock — the same shape `run_convergence_tick` uses, so
                    // the forwarded Action sees a consistent wall-clock
                    // snapshot. `now_unix` recomputed each item.
                    let now = clock.now();
                    let now_unix = UnixInstant::from_clock(&*clock);
                    let tick = TickContext {
                        now,
                        now_unix,
                        tick: tick_n,
                        deadline: now + Duration::from_secs(1),
                    };
                    tick_n = tick_n.saturating_add(1);
                    // Forward the emitted Action into the SAME production
                    // dispatch path a reconciler-emitted Action takes (→
                    // Raft). A dispatch error is logged and the drain
                    // continues — one bad emit must not stall the channel.
                    if let Err(e) = action_shim::dispatch_with_workflow_intent(
                        vec![action],
                        &state,
                        &tick,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "overdrive::workflow_engine",
                            ?e,
                            "workflow emit-drain: dispatch of an emitted Action failed",
                        );
                    }
                }
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

/// Construct the `workflow-lifecycle` reconciler per ADR-0064 §5.
///
/// The pure-sync reconciler in the two-primitive doctrine: it manages
/// WHICH workflow instances should exist, re-emitting
/// `Action::StartWorkflow` for a running-in-intent instance with no live
/// engine task on restart (US-WP-3 AC4). The engine
/// ([`workflow_runtime::WorkflowEngine`]) is the async executor driven
/// off the shim; the reconciler NEVER `.await`s the workflow body
/// (`ReconcilerIsPure` holds). Registered at boot alongside the other
/// first-party reconcilers.
#[must_use]
pub fn workflow_lifecycle() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, WorkflowLifecycle};

    AnyReconciler::WorkflowLifecycle(WorkflowLifecycle::canonical())
}

/// Construct the `svid-lifecycle` reconciler per ADR-0067 D1.
///
/// The pure-sync workload-identity reconciler: it converges
/// `desired = the Running allocations for a workload` against
/// `actual = the in-process `IdentityMgr` held set`, emitting
/// [`Action::IssueSvid`](overdrive_core::reconcilers::Action::IssueSvid) for a
/// Running-but-unheld alloc and
/// [`Action::DropSvid`](overdrive_core::reconcilers::Action::DropSvid) for a
/// held-but-stopped alloc. CA I/O lives entirely in the action-shim executor
/// (01-06); the reconciler builds the `SpiffeId` purely and NEVER `.await`s or
/// reaches for the CA (`ReconcilerIsPure` holds). Registered at boot alongside
/// the other first-party reconcilers.
#[must_use]
pub fn svid_lifecycle() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, SvidLifecycle};

    AnyReconciler::SvidLifecycle(SvidLifecycle::canonical())
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

/// Construct the `service-lifecycle` reconciler per ADR-0055.
///
/// The Phase 1 Service-kind workload reconciler — converges
/// `Stable` / `StartupProbeFailed` / `EarlyExit` terminal conditions
/// against the running `AllocStatusRow` set + probe-result rows.
/// Registered at production boot alongside `noop-heartbeat` /
/// `workload-lifecycle` / `backend-discovery-bridge` /
/// `service-map-hydrator`.
///
/// Per service-health-check-probes step 01-03d this completes the
/// composition-root registration arc: the reconciler-core
/// (`ServiceLifecycleReconciler::reconcile`) landed in commit
/// `2fabf259` (step 01-03), the `AnyReconciler::ServiceLifecycle`
/// dispatch enum landed in commit `087bada4` (step 01-03b), and
/// this factory is the runtime-facing constructor invoked by
/// `run_server_with_obs_and_driver`.
#[must_use]
pub fn service_lifecycle() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::AnyReconciler;
    use overdrive_core::service_lifecycle::ServiceLifecycleReconciler;

    AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new())
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

    use crate::veth_provisioner::WORKLOAD_SUBNET_BASE;

    // Thread the SAME `WORKLOAD_SUBNET_BASE` the provisioner carves
    // per-allocation `/30`s from (one source, D-GATE-PRED) so the
    // hydrator gates Path-A/mesh backends out of BOTH LB paths.
    AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        host_ipv4,
        WORKLOAD_SUBNET_BASE,
    ))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::host_ipv4_with_override_fallback;
    use crate::error::ControlPlaneError;

    fn resolution_failure() -> ControlPlaneError {
        ControlPlaneError::Validation {
            message: "iface unresolvable".to_owned(),
            field: Some("dataplane.client_iface".to_owned()),
        }
    }

    // A successful resolution is used as-is regardless of the override
    // flag — the override must NEVER collapse a real IP to loopback
    // (the ADR-0053 bridge-hydrator bug). Asserted on both branches of
    // `has_dataplane_override` to pin that the flag is inert on `Ok`.
    #[test]
    fn successful_resolution_is_used_as_is_with_override() {
        let resolved = Ok(Ipv4Addr::new(10, 1, 2, 3));
        let result = host_ipv4_with_override_fallback(resolved, true);
        assert_eq!(result.ok(), Some(Ipv4Addr::new(10, 1, 2, 3)));
    }

    #[test]
    fn successful_resolution_is_used_as_is_without_override() {
        let resolved = Ok(Ipv4Addr::new(10, 1, 2, 3));
        let result = host_ipv4_with_override_fallback(resolved, false);
        assert_eq!(result.ok(), Some(Ipv4Addr::new(10, 1, 2, 3)));
    }

    // The fallback arm: resolution FAILED and an override is present
    // (Sim/DST/CLI with no provisioned veth) → boot with LOCALHOST.
    // This is precisely the arm the `is_some() -> false` match-guard
    // mutant breaks: with the guard forced to `false`, a present
    // override would fall through to `Err(source)` and this assertion
    // would fail.
    #[test]
    fn failed_resolution_with_override_falls_back_to_localhost() {
        let resolved = Err(resolution_failure());
        let result = host_ipv4_with_override_fallback(resolved, true);
        assert_eq!(result.ok(), Some(Ipv4Addr::LOCALHOST));
    }

    // The production arm: resolution FAILED and no override is present
    // → propagate the error and refuse to boot. Paired with the test
    // above, this is what makes flipping the guard to `false` fail:
    // the two branches must diverge on the same `Err` input.
    #[test]
    fn failed_resolution_without_override_propagates_error() {
        let resolved = Err(resolution_failure());
        let result = host_ipv4_with_override_fallback(resolved, false);
        assert!(
            matches!(result, Err(ControlPlaneError::Validation { .. })),
            "expected the resolution failure to propagate unchanged, got {result:?}",
        );
    }
}
