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
#![allow(
    clippy::large_enum_variant,
    clippy::result_large_err,
    reason = "GH #295 exact accepted source-honest guest-network and intercept rollback errors retain complete observations"
)]
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
// Service backend dataplane configuration —
// `[dataplane]` config section parser per architecture.md § 5.1.
// Section presence + the two required interface bindings; refusal
// surfaces as `ControlPlaneError::Validation { field:
// Some("dataplane"), .. }`.
pub mod dataplane_config;
// dial-by-name-responder (ADR-0072, GH #243) — the in-agent name layer:
// the third reader of the `ObservationStore` `service_backends` surface,
// answering `<workload>.svc.overdrive.local` queries. Step 01-02 lands only the
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
/// Shared guest-network plan, ports, facts, and source-honest errors.
pub mod guest_network;
pub mod identity_mgr;
// IPv4 resolution via `getifaddrs(3)` for the operator-supplied
// `[dataplane] client_iface`. Production boot threads the resolved
// `Ipv4Addr` through `AppState.host_ipv4` to ServiceLifecycle's
// backend-row projection.
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
pub mod vm_reclamation_boot;
pub mod worker;
// `workflow_runtime` — the durable-async `WorkflowEngine` (ADR-0064 §1,
// §3, §5). Drives author `async fn run` futures off the action-shim with
// crash-safe journal-cursor replay. The `Workflow` trait + `WorkflowCtx`
// it drives live in `overdrive-core::workflow`; this is the tokio-holding
// executor that core's trait declaration delegates to.
pub mod workflow_runtime;

// GH #295 (DISTILL gap B-4): crate-private test-local implementations of the
// three ports this crate declares, for the source-local supervisor and
// admission cells (the `overdrive-sim` doubles implement a second compiled
// copy of these traits here).
#[cfg(test)]
mod shared_network_test_ports;

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::{get, post};
use axum_server::Handle as AxumHandle;
use axum_server::tls_rustls::RustlsConfig;
use futures::stream::{FuturesUnordered, StreamExt};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::{Driver, DriverRegistry};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::cgroup_manager::CgroupManager;
use tokio_util::sync::CancellationToken;

use crate::identity_mgr::IdentityMgr;
use crate::reconciler_runtime::{DEFAULT_TICK_CADENCE, run_convergence_tick};

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker, EvaluationEligibility};
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::reconcilers::{ReconcilerName, ResyncSchedule, TargetResource, resolve_scope};
use overdrive_core::traits::observation_store::{
    LagAwareSubscription, ObservationRow, ObservationRowKind, SubscriptionEvent,
};
use overdrive_reconcilers::AnyReconciler;

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
    /// Driver registry per ADR-0022 (pre-committed migration) / ADR-0083
    /// §D1 (GH #42, step 01-08): replaces the former single
    /// `driver: Arc<dyn Driver>` field. **Absence of a key is a
    /// first-class answer, not an error state** — a node with no
    /// `cloud-hypervisor` installed simply has no `DriverType::Vm` entry
    /// (SD-5's capability gate). Production composes the `VmDriver` only
    /// when the discover→probe Earned-Trust sequence succeeds; DST/test
    /// fixtures compose a registry holding whichever `Sim*`/test driver(s)
    /// they need.
    pub drivers: Arc<DriverRegistry>,
    /// The host-observation-driven port `VmReclamation` hydrates its
    /// `actual` half from (ADR-0083 §D7, brief.md §105a.2, GH #42).
    /// Composed **unconditionally** — never gated on [`DriverRegistry`]'s
    /// `Vm` entry — so a node that uninstalled `cloud-hypervisor` still
    /// observes and still reclaims (S-VM-30). Production wires
    /// `Arc::new(overdrive_host::RealVmHostState::new(..))`; the broad
    /// fixture surface ([`AppState::new`]) default-composes
    /// [`NoopVmHostState`] (observes nothing, ripple-free — mirrors the
    /// `mtls_worker: None` / empty-registry `workflow_engine` defaults on
    /// that constructor).
    pub vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState>,
    /// The one production shared guest-network owner used by boot and every
    /// allocation action path.
    pub(crate) shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    /// Alloc → driver-kind routing index (ADR-0083 §D2a(b), GH #42).
    /// `Action::StopAllocation` / `Action::FinalizeFailed` carry no
    /// `AllocationSpec`, so the action shim cannot re-derive which driver
    /// started a given allocation from the action alone — this in-memory,
    /// per-boot index is written on `StartAllocation` / `RestartAllocation`
    /// (where the payload IS in hand) and read on every stop/terminal arm.
    /// See `action_shim::AllocDriverIndex`'s own doc comment for the full
    /// lock discipline.
    pub alloc_drivers: Arc<action_shim::AllocDriverIndex>,
    /// Best-effort live broadcast channel for `LifecycleEvent`s emitted by the
    /// action shim after successful `obs.write()`. Per architecture.md
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
    /// ServiceLifecycle backend projection — every
    /// `service_backends` observation row it emits carries
    /// this address in `endpoint.host` so XDP reverse-NAT translation
    /// (Phase 2.3) can derive the per-host VIP. Per
    /// `.claude/rules/development.md` § "Port-trait dependencies" the
    /// dependency is mandatory at construction so tests cannot
    /// silently inherit a production loopback by forgetting to
    /// override.
    ///
    /// The dataplane configuration work lands this field; the placeholder
    /// `Ipv4Addr::LOCALHOST` previously
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
    /// `stop_alloc`).
    ///
    /// `Option` (the sanctioned `ProbeRunner` shape, NOT a port-trait
    /// dodge): `Some(worker)` ONLY on the production `run_server` boot
    /// (and the Tier-3 e2e), where a REAL `EbpfDataplane` +
    /// `HostMtlsEnforcement` + `MtlsDataplane` are composed AFTER
    /// `IdentityMgr`; `None` for every non-mTLS fixture and the
    /// `SimDataplane`-override boot (no real BPF to intercept on). The
    /// action-shim reads `state.mtls_worker` and fires `if let Some`.
    pub mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    /// The one EXEC gate shared with the VM driver and supervisor.
    #[allow(
        dead_code,
        reason = "threaded into the state now; action helpers consume it in DELIVER 06-04"
    )]
    pub(crate) guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
    /// The one node-wide guest address pool used by dispatch and hydration.
    #[allow(
        dead_code,
        reason = "threaded into the state now; admission consumes it in DELIVER 06-03"
    )]
    pub(crate) guest_pool: Arc<guest_network::GuestAddressPool>,
    /// DNS reply-source fallback allocator. It is retained only by the DNS
    /// responder's source-pinning adapter; guest attachment ownership lives in
    /// `SharedGuestNetworkOwner`.
    ///
    /// NOT an `Option`: unlike `mtls_worker`, the allocator is harmless on the
    /// non-mTLS fixture surface (it just hands out slots nobody provisions),
    /// so a non-optional `Default`-constructed field keeps every fixture
    /// ripple-free. The type is already `#[derive(Clone, Default)]` and holds
    /// its `Arc<Mutex<BTreeMap<…>>>` INTERNALLY (it self-shares on clone,
    /// exactly like `IdentityMgr`), so the field is a plain value — no outer
    /// `Arc<Mutex<…>>` wrapper. Ephemeral runtime state, never persisted:
    /// on a fresh process boot nothing is held (criterion 6).
    pub(crate) dns_slots: veth_provisioner::NetSlotAllocator,
    /// Legacy fixture-only compatibility storage; production guest
    /// attachment ownership never reads this value.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub net_slot_allocator: veth_provisioner::NetSlotAllocator,
    /// Per-host stable per-`<workload>` frontend-address allocator
    /// (dial-by-name-responder step 01-05; ADR-0072 REV-2/REV-3, GH #243).
    /// The SINGLE source of frontend truth (DDN-2): the ONE `Arc`-shared
    /// instance the deploy-time WRITER (the `submit_workload` Service arm +
    /// the boot rebuild) populates AND the `name_index` (01-03) / `by_frontend`
    /// (02-00) READERS observe. The 01-05 assign-on-declare writes the
    /// `<workload> → F` binding; 02-01 LATER injects the SAME cloned instance into
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

/// Shared default wall-clock cap on Job and Service streaming connections.
/// `AppState::streaming_cap` remains a construction/test override; no operator
/// configuration supplies this value.
pub const DEFAULT_STREAMING_CAP: Duration = Duration::from_secs(90);

/// Default [`overdrive_core::traits::vm_host_state::VmHostState`] for
/// [`AppState::new`]'s broad fixture callers (ripple-free —
/// mirrors the `mtls_worker: None` / empty-registry `workflow_engine`
/// defaults on that constructor). Observes nothing, refuses nothing;
/// correct for a fixture surface that never seeds VM host state.
///
/// Deliberately NOT `overdrive_sim::adapters::vm_host_state::SimVmHostState`
/// — `overdrive-sim` is a `[dev-dependencies]`-only crate
/// (`.claude/rules/development.md` § "Port-trait dependencies") and
/// cannot be named from this non-`#[cfg(test)]` production constructor.
#[derive(Debug, Clone, Copy, Default)]
struct NoopVmHostState;

#[async_trait::async_trait]
impl overdrive_core::traits::vm_host_state::VmHostState for NoopVmHostState {
    fn kind(&self) -> &'static str {
        "overdrive_control_plane::NoopVmHostState"
    }

    async fn probe(
        &self,
    ) -> std::result::Result<(), overdrive_core::traits::vm_host_state::VmHostStateProbeError> {
        Ok(())
    }

    async fn observe(
        &self,
    ) -> std::io::Result<overdrive_core::traits::vm_host_state::VmHostObservation> {
        Ok(overdrive_core::traits::vm_host_state::VmHostObservation::default())
    }

    async fn kill_scope(&self, _scope: &overdrive_core::cgroup::CgroupPath) -> std::io::Result<()> {
        Ok(())
    }

    async fn discard_artifacts(
        &self,
        _alloc: &overdrive_core::AllocationId,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

impl AppState {
    /// Build an `AppState` with a fresh `LifecycleEvent` broadcast
    /// channel of default capacity. Used by every test fixture and
    /// the production boot path.
    ///
    /// The shared Job/Service default `streaming_cap` is 90s.
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
        // `driver` stays a single `Arc<dyn Driver>` on this convenience
        // constructor — it is wrapped into a fresh single-entry
        // `DriverRegistry` below (ADR-0083 §D1, GH #42) so existing
        // single-driver fixture callers remain simple. Multi-driver
        // composition goes through [`Self::new_with_workflow_engine`]
        // directly with a caller-built `Arc<DriverRegistry>`.
        driver: Arc<dyn Driver>,
        clock: Arc<dyn Clock>,
        dataplane: Arc<dyn Dataplane>,
        ca: Arc<dyn Ca>,
        identity: Arc<IdentityMgr>,
        node_id: NodeId,
        allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
        listener_facts: Arc<tokio::sync::Mutex<crate::listener_facts::ListenerFactStore>>,
        host_ipv4: std::net::Ipv4Addr,
        mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
        shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
        guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
        guest_pool: Arc<guest_network::GuestAddressPool>,
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
        // Wrap the single driver into a fresh, single-entry `DriverRegistry`
        // (ADR-0083 §D1, GH #42) — the field-level migration this
        // convenience constructor absorbs so existing fixture callers stay
        // unchanged.
        let mut registry = DriverRegistry::new();
        registry.insert(driver);
        let drivers = Arc::new(registry);
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
            drivers,
            clock,
            dataplane,
            ca,
            identity,
            node_id,
            allocator,
            listener_facts,
            host_ipv4,
            workflow_engine,
            mtls_worker,
            // Fixture surface: a ripple-free no-op VmHostState (see
            // `NoopVmHostState`'s own doc comment) — fixture callers of this
            // convenience constructor need no change.
            Arc::new(NoopVmHostState),
            // Fixture surface: a fresh empty per-host frontend-address allocator
            // (ripple-free, same posture as the `None` mtls_worker default). The
            // PRODUCTION boot path (`run_server`) constructs the ONE shared
            // instance and passes it through `new_with_workflow_engine` so it is
            // shared with the re-keyed `MtlsResolve` + the `DnsResponder`.
            crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new(),
            shared_guest_network,
            guest_network_exec,
            guest_pool,
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
        // ADR-0083 §D1 (GH #42, step 01-08): the composed driver set —
        // replaces the former single `driver: Arc<dyn Driver>` parameter.
        // Callers that only ever run one driver build a single-entry
        // registry (see [`Self::new`]); the production boot path and any
        // VM-aware tests build a registry with the VM capability composed.
        drivers: Arc<DriverRegistry>,
        clock: Arc<dyn Clock>,
        dataplane: Arc<dyn Dataplane>,
        ca: Arc<dyn Ca>,
        identity: Arc<IdentityMgr>,
        node_id: NodeId,
        allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
        listener_facts: Arc<tokio::sync::Mutex<crate::listener_facts::ListenerFactStore>>,
        host_ipv4: std::net::Ipv4Addr,
        workflow_engine: Arc<workflow_runtime::WorkflowEngine>,
        mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
        // microvm-driver-cloud-hypervisor step 02-02 (ADR-0083 §D7,
        // brief.md §105a.2, GH #42): composed unconditionally, never
        // gated on `Vm` registry presence. See `AppState::vm_host_state`'s
        // own doc comment.
        vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState>,
        frontend_addr_allocator:
            crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator,
        shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
        guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
        guest_pool: Arc<guest_network::GuestAddressPool>,
    ) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(DEFAULT_LIFECYCLE_BROADCAST_CAPACITY);
        let tx = Arc::new(tx);
        Self {
            store,
            intent_redb_path,
            obs,
            runtime,
            drivers,
            vm_host_state,
            shared_guest_network,
            // Fresh, empty per-boot index (ADR-0083 §D2a(b), GH #42) — no
            // allocation has started yet at construction time.
            alloc_drivers: Arc::new(action_shim::AllocDriverIndex::default()),
            lifecycle_events: tx,
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
            guest_network_exec,
            guest_pool,
            // Default-construct the per-host slot allocator INSIDE the
            // constructor (transparent-mtls-enrollment D-TME-12 G3, step
            // 04-01) — NOT a constructor parameter. This is the same
            // ripple-avoidance `mtls_worker` (`None`) + `workflow_engine`
            // (empty registry) use: the ~42 non-mTLS fixtures and the
            // `reconciler_runtime`/`listener_facts` callers need no change.
            // On a fresh process boot nothing is held; still-Running allocs
            // re-assign on their next lifecycle pass (criterion 6).
            dns_slots: veth_provisioner::NetSlotAllocator::new(),
            #[cfg(any(test, feature = "integration-tests"))]
            net_slot_allocator: veth_provisioner::NetSlotAllocator::new(),
            // The per-host frontend-address allocator is now a CONSTRUCTOR
            // PARAMETER (02-01) — NOT default-constructed here. DDN-2 requires
            // ONE `Arc`-shared instance the composition root injects into BOTH
            // the re-keyed `MtlsResolve` (`by_frontend`) AND the `DnsResponder`
            // (`name_index`), so the `F` keyed in `by_frontend` is
            // byte-identical to the `F` DNS answers. `run_server` constructs the
            // ONE allocator before the `MtlsResolve` (which is moved into the
            // worker) and passes the SAME handle here so the responder built
            // after `AppState::new` shares it. The convenience [`Self::new`]
            // default-constructs a fresh empty allocator for the fixture surface
            // (ripple-free). It self-shares on clone, so a `.clone()` shares the
            // same held map.
            frontend_addr_allocator,
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
    /// [`ServerConfig::new`](Self::new) + the
    /// `..ServerConfig::new(kek, mtls_intercept, guest_dns)` rest-pattern
    /// instead.
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

    /// Required `[dataplane]` section. Carries the operator-supplied
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

    /// The required transparent-mTLS intercept port (D-295-R16). Production
    /// passes `HostMtlsIntercept`; tests inject `SimMtlsIntercept` or a
    /// test-local intercept. `run_server*` always composes the worker over it.
    ///
    /// RED scaffold (D-295-R16): consumed in DELIVER step 05-01; until then
    /// `run_server*` keep today's composition and do not read it.
    pub mtls_intercept: Arc<dyn overdrive_worker::mtls_intercept_port::MtlsIntercept>,

    /// The required shared-gateway DNS responder factory (D-295-R16).
    /// Production passes `HostGuestDnsFactory`; tests inject
    /// `SimGuestDnsFactory`.
    ///
    /// RED scaffold (D-295-R16): consumed in DELIVER step 05-01; until then
    /// `run_server*` keep today's composition and do not read it.
    pub guest_dns: Arc<dyn crate::dns_responder::GuestDnsFactory>,

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

    /// Test-only whole-port substitution seam for focused transparent-mTLS
    /// adapter tests (transparent-mtls-host-socket, step 06-03, criteria[1]).
    /// When `Some(read)`, boot composes `HostMtlsEnforcement` over THIS
    /// `IdentityRead` instead of the production `IdentityMgr`; its SVID must
    /// still chain to its supplied bundle and carry exactly one valid SPIFFE
    /// URI SAN because the production relying-party verifier remains active.
    ///
    /// Production walking-skeleton tests must leave this `None`: only the real
    /// `IdentityMgr` issuance/lifecycle path can prove production composition.
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

    /// Test-only adapter-substitution seam for the `Vmm` port (ADR-0083
    /// §D8, GH #42, step 01-09). When `Some(v)`, `compose_vm_driver` uses
    /// THIS adapter in place of discovering/constructing
    /// `CloudHypervisorVmm`, so a real in-process `overdrive serve` runs
    /// the SAME discover → probe → insert sequence against an adapter
    /// carrying an injected fault (a `SimVmm`), or against a REAL
    /// `CloudHypervisorVmm` constructed with a test-only builder override
    /// (e.g. `.with_image_dir(tmpfs_path)` — a genuinely non-reflink real
    /// substrate, no injection), rather than against the production
    /// discovery path. `None` in production; the field does not exist in
    /// a production binary.
    ///
    /// Shaped after `mtls_identity_override` above (a whole-**port**-
    /// implementation swap, `Arc<dyn Trait>`) — deliberately NOT after
    /// `dataplane_override`'s whole-**subsystem** gate (ADR-0083 §A10
    /// considers and rejects that shape by name). Every downstream
    /// consumer (`DriverRegistry`, the exit-observer loop, `VmDriver`)
    /// sees `Arc<dyn Vmm>` and is unaware the seam exists.
    /// `Vmm::probe()` (and `CgroupAccounting::probe()`) run
    /// UNCONDITIONALLY against whichever adapter is bound — Earned Trust
    /// is never skipped for the injected case.
    ///
    /// Gated behind `#[cfg(feature = "integration-tests")]` on both this
    /// field declaration and its one use site in `compose_vm_driver`,
    /// mirroring `mtls_identity_override`'s discipline.
    ///
    /// Excluded from [`Debug`] (beyond the adapter-presence marker) —
    /// `Arc<dyn Vmm>` is neither `Debug` nor `Default`.
    #[cfg(feature = "integration-tests")]
    pub vmm_override: Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>,
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
            )
            .field("mtls_intercept", &"<dyn MtlsIntercept>")
            .field("guest_dns", &"<dyn GuestDnsFactory>");
        #[cfg(feature = "integration-tests")]
        dbg.field("dataplane_probe_fault", &self.dataplane_probe_fault);
        #[cfg(feature = "integration-tests")]
        dbg.field("mtls_probe_fault", &self.mtls_probe_fault);
        #[cfg(feature = "integration-tests")]
        dbg.field(
            "mtls_identity_override",
            &self.mtls_identity_override.as_ref().map(|_| "<dyn IdentityRead>"),
        );
        #[cfg(feature = "integration-tests")]
        dbg.field("vmm_override", &self.vmm_override.as_ref().map(|_| "<dyn Vmm>"));
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
    /// Fixtures use `..ServerConfig::new(test_kek, intercept, dns)`: the
    /// rest-pattern still supplies every other field, while the three required
    /// ports are explicit, type-checked arguments.
    ///
    /// `bind`, `data_dir`, and `operator_config_dir` get sentinel values that
    /// callers MUST override (as under the prior `Default`). `tick_cadence`
    /// defaults to [`reconciler_runtime::DEFAULT_TICK_CADENCE`] (100ms) and
    /// `clock` to `Arc::new(SystemClock)` from the [`overdrive_host`] crate —
    /// the only crate permitted to instantiate `SystemClock` per CLAUDE.md
    /// "Repository structure". Tests that need a controllable clock override
    /// `clock` in the same struct literal.
    ///
    /// The intercept and DNS ports (D-295-R16) are equally required: a boot
    /// site that forgets either fails to compile. Production passes
    /// `HostMtlsIntercept` and `HostGuestDnsFactory`.
    ///
    /// [`Kek`]: overdrive_core::ca::kek::Kek
    #[must_use]
    pub fn new(
        kek: Arc<dyn overdrive_core::ca::kek::Kek>,
        mtls_intercept: Arc<dyn overdrive_worker::mtls_intercept_port::MtlsIntercept>,
        guest_dns: Arc<dyn crate::dns_responder::GuestDnsFactory>,
    ) -> Self {
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
            mtls_intercept,
            guest_dns,
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
            // ADR-0083 §D8 step 01-09: default no Vmm override; the
            // composition root discovers/constructs the production
            // `CloudHypervisorVmm`. S-VM-13/S-VM-75 set `Some(..)` to
            // exercise the probe-refusal fail-closed branch.
            #[cfg(feature = "integration-tests")]
            vmm_override: None,
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
    /// One `JoinHandle` PER REGISTRY ENTRY for the `worker::exit_observer`
    /// task — each consumes `ExitEvent`s from its OWN driver's watcher and
    /// writes `AllocStatusRow`s to the `ObservationStore`. Per
    /// `fix-exec-driver-exit-watcher` Step 01-02, widened to a `Vec` by
    /// ADR-0083 §D2a(a)/(a5) (GH #42, step 01-08): a naive per-driver loop
    /// would leak N-1 tasks (dropping a `JoinHandle` DETACHES, it does not
    /// abort) and — because the token was previously minted per spawn
    /// call — retain a cancel path for only the LAST driver. Every task
    /// shares ONE cloned `exit_observer_shutdown` token; `shutdown()`
    /// cancels it once and awaits every task in this `Vec`.
    ///
    /// Shutdown ordering: per RCA §Approved fix item 5 the convergence
    /// task is signalled to drain FIRST, then axum drains, THEN the
    /// observer's `exit_observer_shutdown` token is cancelled so every
    /// observer task's `tokio::select!` resolves and each task exits.
    ///
    /// The token-driven shutdown is the fallback path for the case
    /// where a watcher task is still alive at shutdown time (e.g. a
    /// `/bin/sleep` workload that did not reap before convergence was
    /// cancelled, or a `SimDriver`-backed test where `exit_tx` is held
    /// by the test's `Arc<dyn Driver>` until the test fn returns).
    /// Without this, awaiting these tasks would block indefinitely on
    /// `rx.recv()`. With it, shutdown is bounded.
    exit_observer_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// `JoinHandle` for the workflow emit-drain task — the production
    /// consumer of the [`workflow_runtime::WorkflowEngine`]'s Action
    /// channel. It takes the channel receiver once at boot and forwards
    /// every `ctx.emit_action`'d [`Action`](overdrive_core::reconcilers::Action)
    /// into [`action_shim::dispatch_with_workflow_intent`] (→ Raft). Per
    /// ADR-0064 §4 / brief.md §92; spawned in
    /// [`run_server_with_obs_and_driver`].
    emit_drain_task: tokio::task::JoinHandle<()>,
    /// `JoinHandle` for the interest-router task (ADR-0084 §5, Piece B) — the
    /// declarative event-interest fan-out spawned by
    /// [`run_server_with_obs_and_driver`]. List-then-Watch over the
    /// observation change-feed; submits an `Evaluation` per interested
    /// reconciler on every accepted `alloc_status` change. Held so
    /// [`Self::shutdown`] can cooperatively drain it, and so
    /// [`Self::interest_router_running`] can report it live (the S-266-01
    /// vertical-slice boot check).
    interest_router_task: tokio::task::JoinHandle<()>,
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
    /// Token observed by the interest-router task's `tokio::select!` loop.
    /// Cancelled in [`Self::shutdown`] alongside the convergence loop — the
    /// router holds an `Arc<dyn ObservationStore>` and a broker capability, so
    /// it must stop before `AppState` tears down. Cooperative shutdown is
    /// mandatory: the router parks in `subscription.next()`, which a bare
    /// `abort()` cannot interrupt (`.claude/rules/development.md`
    /// § "Concurrency & async").
    interest_router_shutdown: CancellationToken,
    /// Explicit owner for the worker's accept/enforce/pass-through task tree.
    /// Both graceful shutdown and abrupt test-owner loss invalidate and join
    /// userspace while leaving active allocation rules in the kernel for the
    /// replacement boot's reclaim-before-sweep boundary.
    mtls_worker_owner: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    /// Concrete private owner for the mTLS resolver's List/Watch drain. Kept
    /// outside the `MtlsResolve` domain port so both graceful and abrupt server
    /// boundaries can cancel and await the exact `JoinHandle`.
    mtls_resolve_owner: Arc<crate::mtls_resolve_adapter::ServiceBackendsResolve>,
    /// Sole retained shared-network supervisor owner. Step 03-03 supplies the
    /// retained task and classification behavior behind this exact field.
    shared_network_supervisor: SharedNetworkSupervisorHandle,
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code, reason = "D-295-DISTILL-8 RED scaffold is not composed before gate activation")]
pub(crate) enum SharedNetworkSupervisorError {
    #[error("guest-network convergence failed")]
    GuestNetwork(#[from] guest_network::GuestNetworkError),
    #[error("shared mTLS owner convergence failed")]
    MtlsOwner(#[from] overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError),
    #[error("DNS owner recovery failed")]
    Dns(#[from] crate::dns_responder::responder::DnsResponderError),
}

/// Period between the starts of two full shared-network audits while EXEC is
/// Open: ADR-0124's one-second security detection bound (D-295-R13). A slow
/// audit stretches the period instead of stacking calls.
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_AUDIT_PERIOD: Duration = Duration::from_secs(1);

/// Period between two bounded recovery attempts while EXEC is Recovering
/// (ADR-0124, D-295-R13).
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_RETRY_PERIOD: Duration = Duration::from_millis(250);

/// Recovery window from detection to the typed fail-stop (ADR-0124,
/// D-295-R13).
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_RECOVERY_DEADLINE: Duration = Duration::from_secs(5);

/// Completed recovery attempts after which the supervisor fail-stops
/// (ADR-0124, D-295-R13).
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_RECOVERY_ATTEMPTS: u32 = 20;

/// Bound on each owner call during detection; a call that misses it fails
/// with its owner's first component and cause `audit_timeout` (D-295-R13).
/// Rule: `max(1 s, 4 × L)`, L the largest single owner-call audit latency at
/// T1-PORT4 (M-ND295-E18). It holds the rule's floor until the measurement
/// sets it; the step that sets it records the measurement here.
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_AUDIT_CALL_BOUND: Duration = Duration::from_secs(1);

/// Bound on the one `quiesce_managed_taps` call, itself capped by the
/// remaining recovery deadline; a call that misses it leaves the unconfirmed
/// set undetermined (D-295-R14). Rule: `max(1 s, 4 × Q)`, Q the largest
/// quiescence wall time at T1-PORT4 (M-ND295-E18). It holds the rule's floor
/// until the measurement sets it; the step that sets it records the
/// measurement here.
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_QUIESCE_CALL_BOUND: Duration = Duration::from_secs(1);

/// Bound on each `cgroup.kill` write — per-VM and workloads-slice — in a
/// report's kill loop, measured on the injected clock (D-295-R14; kill loop,
/// user decision 1 of 2026-09-30). A write still pending at it counts as a
/// failed kill: per-VM, the workloads-slice kill then `VmKillFailed`; slice, its
/// recorded `vm_kill` outcome then the fail-stop. The recovery deadline does NOT
/// cap it — a report's kill loop runs to its end before any further owner call
/// or fail-stop. Rule: `max(1 s, 4 × W)`, W the largest single kill write at
/// T1-PORT4 (M-ND295-E18). It holds the rule's floor until the measurement sets
/// it; the step that sets it records the measurement here.
#[cfg_attr(not(test), allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01"))]
const SHARED_NETWORK_VM_KILL_CALL_BOUND: Duration = Duration::from_secs(1);

/// The ports the one runtime shared-network supervisor task owns (D-295-R13,
/// R14, R16). They replace `run_mtls_owner`'s parameters in DELIVER step
/// 09-01.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01")]
struct SharedNetworkSupervisorPorts {
    shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    dns: DnsServeTaskOwner,
    dns_factory: Arc<dyn dns_responder::GuestDnsFactory>,
    dns_deps: dns_responder::GuestDnsDeps,
    vm_kill: vm_kill::VmKillCapability,
}

/// Kill-only capability over VMM cgroups (review finding H3). The
/// `CgroupManager` field and the constructor are private to this child
/// module and the two kill methods are `pub(super)`, so the enclosing
/// supervisor module can call exactly `kill_allocation` and
/// `kill_workloads_slice` and reach no other `CgroupManager` surface (no
/// create, placement, limit, removal, or bootstrap authority) — "exposes
/// exactly two methods" holds structurally (review finding L4).
mod vm_kill {
    /// Writes `1` to VMM cgroups' `cgroup.kill`; nothing else.
    #[allow(
        clippy::redundant_pub_crate,
        reason = "the DESIGN pins `pub(super)`: visibility is stated relative to the supervisor module"
    )]
    pub(super) struct VmKillCapability {
        #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01")]
        cgroups: super::CgroupManager,
    }

    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01")]
    impl VmKillCapability {
        /// Wrap the manager over the same cgroup root and `CgroupFs` every
        /// composed `VmDriver` writes through.
        pub(super) const fn new(cgroups: super::CgroupManager) -> Self {
            Self { cgroups }
        }

        /// Write `1` to one allocation's scope `cgroup.kill`. An absent scope is
        /// `Ok`: the VMM can no longer execute.
        #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 09-01")]
        #[expect(
            clippy::unused_async,
            reason = "RED scaffold — DELIVER step 09-01 awaits the kill write"
        )]
        pub(super) async fn kill_allocation(
            &self,
            alloc: &super::AllocationId,
        ) -> std::io::Result<()> {
            let _ = alloc;
            todo!("RED scaffold: D-295-R14 VmKillCapability::kill_allocation — DELIVER step 09-01")
        }

        /// Write `1` to the workloads slice's `cgroup.kill`, killing every
        /// workload VMM.
        #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 09-01")]
        #[expect(
            clippy::unused_async,
            reason = "RED scaffold — DELIVER step 09-01 awaits the kill write"
        )]
        pub(super) async fn kill_workloads_slice(&self) -> std::io::Result<()> {
            todo!(
                "RED scaffold: D-295-R14 VmKillCapability::kill_workloads_slice — DELIVER step 09-01"
            )
        }
    }
}

struct SharedNetworkSupervisorHandle {
    request_rx: tokio::sync::mpsc::Receiver<overdrive_core::guest_network::ServeShutdownRequest>,
    task: Option<tokio::task::JoinHandle<std::result::Result<(), SharedNetworkSupervisorError>>>,
    exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
    shutdown: CancellationToken,
}

impl SharedNetworkSupervisorHandle {
    const fn new(
        request_rx: tokio::sync::mpsc::Receiver<
            overdrive_core::guest_network::ServeShutdownRequest,
        >,
        task: tokio::task::JoinHandle<std::result::Result<(), SharedNetworkSupervisorError>>,
        exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
        shutdown: CancellationToken,
    ) -> Self {
        Self { request_rx, task: Some(task), exec, shutdown }
    }

    async fn shutdown_requested(&mut self) -> overdrive_core::guest_network::ServeShutdownRequest {
        let Some(task) = self.task.as_mut() else {
            unreachable!("the retained supervisor has one retained task");
        };
        tokio::select! {
            biased;
            joined = task => {
                self.task.take();
                let cause = match joined {
                    Ok(Ok(())) => overdrive_core::guest_network::SharedGuestNetworkFailStopCause::SupervisorReturned,
                    Ok(Err(source)) => {
                        tracing::error!(
                            name: "guest_network.shared_owner_supervisor_failed",
                            error = %source,
                            "retained shared-network supervisor returned a typed failure"
                        );
                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::SupervisorFailed
                    }
                    Err(join_error) if join_error.is_panic() => {
                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::SupervisorPanicked
                    }
                    Err(join_error) if join_error.is_cancelled() => {
                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::SupervisorCancelled
                    }
                    Err(_) => unreachable!("Tokio JoinError has only panic and cancellation states"),
                };
                let Some(request) = self.exec.fail_stop(cause) else {
                    unreachable!("the retained supervisor writes its first fail-stop request");
                };
                overdrive_core::guest_network::ServeShutdownRequest::SharedGuestNetwork(request)
            }
            request = self.request_rx.recv() => {
                if let Some(request) = request {
                    request
                } else {
                    let Some(request) = self.exec.fail_stop(
                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::RequestChannelClosed,
                    ) else {
                        unreachable!("the retained supervisor writes its first fail-stop request");
                    };
                    overdrive_core::guest_network::ServeShutdownRequest::SharedGuestNetwork(request)
                }
            }
        }
    }

    /// The one runtime shared-network supervisor (D-295-R13, R14, R15, R16):
    /// detection, component-specific quiescence, bounded recovery, per-VM
    /// kill scope, restore, and typed fail-stop. Replaces `run_mtls_owner`
    /// in DELIVER step 09-01.
    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 09-01")]
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 09-01")]
    #[expect(
        clippy::unused_async,
        reason = "RED scaffold — DELIVER step 09-01 awaits the owner calls"
    )]
    async fn run_shared_network_supervisor(
        ports: SharedNetworkSupervisorPorts,
        exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
        clock: Arc<dyn Clock>,
        request_tx: tokio::sync::mpsc::Sender<overdrive_core::guest_network::ServeShutdownRequest>,
        shutdown: CancellationToken,
    ) -> std::result::Result<(), SharedNetworkSupervisorError> {
        let _ = (ports, exec, clock, request_tx, shutdown);
        todo!("RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01")
    }

    #[expect(
        clippy::collapsible_match,
        clippy::too_many_lines,
        reason = "RUN-295-B keeps component classification and the cadence/deadline state machine in one exact private owner future"
    )]
    async fn run_mtls_owner(
        shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
        mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
        exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
        clock: Arc<dyn Clock>,
        request_tx: tokio::sync::mpsc::Sender<overdrive_core::guest_network::ServeShutdownRequest>,
        shutdown: CancellationToken,
    ) -> std::result::Result<(), SharedNetworkSupervisorError> {
        let component_for =
            |error: &overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError| {
                match error {
                    overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::ListenerBind {
                        leg, ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::ListenerLocalAddr {
                        leg, ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::ListenerPostcondition {
                        leg, ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskReturned {
                        leg,
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskFailed {
                        leg, ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskPanicked {
                        leg,
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskCancelled {
                        leg,
                    } => match leg {
                        overdrive_worker::mtls_intercept::InterceptLeg::F => {
                            overdrive_core::guest_network::SharedGuestNetworkComponent::LegF
                        }
                        overdrive_worker::mtls_intercept::InterceptLeg::C => {
                            overdrive_core::guest_network::SharedGuestNetworkComponent::LegC
                        }
                    },
                    overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::Intercept {
                        source,
                    } => match source {
                        overdrive_worker::mtls_intercept::InterceptError::PostconditionMismatch {
                            ..
                        }
                        | overdrive_worker::mtls_intercept::InterceptError::NftSharedReplaceFailed {
                            ..
                        }
                        | overdrive_worker::mtls_intercept::InterceptError::NftSharedRollbackFailed {
                            ..
                        }
                        | overdrive_worker::mtls_intercept::InterceptError::NftSharedRollbackPostconditionMismatch {
                            ..
                        }
                        | overdrive_worker::mtls_intercept::InterceptError::NftSharedReplacementMismatchRolledBack {
                            ..
                        }
                        | overdrive_worker::mtls_intercept::InterceptError::NftSharedReplacementReadFailedRolledBack {
                            ..
                        } => overdrive_core::guest_network::SharedGuestNetworkComponent::IpRules,
                        _ => overdrive_core::guest_network::SharedGuestNetworkComponent::Supervisor,
                    },
                    overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::BootMemberClear {
                        ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::MemberMismatch {
                        ..
                    }
                    | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::MemberRepair {
                        ..
                    } => overdrive_core::guest_network::SharedGuestNetworkComponent::IpSets,
                    _ => overdrive_core::guest_network::SharedGuestNetworkComponent::Supervisor,
                }
            };

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(()),
                () = clock.sleep(Duration::from_secs(1)) => {}
            }

            let component = match mtls_worker.audit_shared_owner().await {
                Ok(()) => continue,
                Err(source) => component_for(&source),
            };
            if !exec.begin_recovery(component) {
                return Ok(());
            }
            tracing::warn!(
                name: "guest_network.shared_owner_unhealthy",
                component = ?component,
                "shared guest-network owner entered bounded recovery"
            );
            shared_guest_network.quiesce_managed_taps().await?;

            loop {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => return Ok(()),
                    () = clock.sleep(Duration::from_millis(250)) => {}
                }

                let first_remaining = match mtls_worker.converge_shared_owner().await {
                    Ok(()) => match mtls_worker.audit_shared_owner().await {
                        Ok(()) => None,
                        Err(source) => Some(component_for(&source)),
                    },
                    Err(source) => Some(component_for(&source)),
                };
                if first_remaining.is_none() {
                    if exec.complete_attempt(None) {
                        tracing::info!(
                            name: "guest_network.shared_owner_recovered",
                            component = ?component,
                            "shared guest-network owner recovered before the deadline"
                        );
                        break;
                    }
                    return Ok(());
                }
                let Some(component) = first_remaining else {
                    unreachable!("a failed recovery attempt always has a remaining component");
                };
                if !exec.complete_attempt(Some(component)) {
                    return Ok(());
                }
                let Some(progress) = exec.recovery_progress() else {
                    return Ok(());
                };
                tracing::warn!(
                    name: "guest_network.shared_owner_retry",
                    component = ?progress.component,
                    attempt = progress.attempts,
                    elapsed = ?progress.elapsed,
                    "shared guest-network owner retry remains unhealthy"
                );
                if progress.attempts >= 20 || progress.elapsed >= Duration::from_secs(5) {
                    tracing::error!(
                        name: "guest_network.shared_owner_fail_stop",
                        component = ?progress.component,
                        attempts = progress.attempts,
                        elapsed = ?progress.elapsed,
                        cleanup = "abandoned_to_shutdown",
                        "shared guest-network owner reached its bounded recovery deadline"
                    );
                    let Some(request) = exec.fail_stop(
                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
                    ) else {
                        return Ok(());
                    };
                    let _ = request_tx
                        .send(
                            overdrive_core::guest_network::ServeShutdownRequest::SharedGuestNetwork(
                                request,
                            ),
                        )
                        .await;
                    shutdown.cancelled().await;
                    return Ok(());
                }
            }
        }
    }

    async fn shutdown(mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsServeTaskExit {
    Returned,
    Panicked,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsServeTaskState {
    Running,
    Exited(DnsServeTaskExit),
    Replacing,
    ShuttingDown,
    Stopped,
}

#[derive(Debug, thiserror::Error)]
enum DnsServeTaskOwnerError {
    #[error("DNS task owner cannot replace from state {state:?}")]
    InvalidReplacementState { state: DnsServeTaskState },
}

struct DnsServeTaskOwner {
    state: DnsServeTaskState,
    responder: Option<Arc<dyn crate::dns_responder::GuestDns>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

#[expect(
    clippy::collapsible_if,
    reason = "DNS task owner keeps cooperative stop and bounded abort branches explicit"
)]
impl DnsServeTaskOwner {
    const fn new(
        responder: Arc<dyn crate::dns_responder::GuestDns>,
        task: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self { state: DnsServeTaskState::Running, responder: Some(responder), task: Some(task) }
    }

    async fn wait_failure(&mut self) -> DnsServeTaskExit {
        let Some(task) = self.task.take() else {
            return match self.state {
                DnsServeTaskState::Exited(exit) => exit,
                DnsServeTaskState::Stopped => DnsServeTaskExit::Cancelled,
                DnsServeTaskState::Running
                | DnsServeTaskState::Replacing
                | DnsServeTaskState::ShuttingDown => {
                    unreachable!("a live DNS owner has one task to observe")
                }
            };
        };
        let exit = match task.await {
            Ok(()) => DnsServeTaskExit::Returned,
            Err(error) if error.is_panic() => DnsServeTaskExit::Panicked,
            Err(error) if error.is_cancelled() => DnsServeTaskExit::Cancelled,
            Err(_) => unreachable!("Tokio JoinError has only panic and cancellation states"),
        };
        self.state = DnsServeTaskState::Exited(exit);
        exit
    }

    async fn replace(
        &mut self,
        replacement: Arc<dyn crate::dns_responder::GuestDns>,
        stop_bound: std::time::Duration,
        spawn: impl FnOnce(Arc<dyn crate::dns_responder::GuestDns>) -> tokio::task::JoinHandle<()>,
    ) -> std::result::Result<(), DnsServeTaskOwnerError> {
        if !matches!(self.state, DnsServeTaskState::Running | DnsServeTaskState::Exited(_)) {
            return Err(DnsServeTaskOwnerError::InvalidReplacementState { state: self.state });
        }
        self.state = DnsServeTaskState::Replacing;
        if let Some(responder) = self.responder.take() {
            responder.stop();
        }
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(stop_bound, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        self.responder = Some(Arc::clone(&replacement));
        self.task = Some(spawn(replacement));
        self.state = DnsServeTaskState::Running;
        Ok(())
    }

    async fn shutdown(&mut self, stop_bound: std::time::Duration) {
        if matches!(self.state, DnsServeTaskState::Stopped) {
            return;
        }
        self.state = DnsServeTaskState::ShuttingDown;
        if let Some(responder) = self.responder.take() {
            responder.stop();
        }
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(stop_bound, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        self.state = DnsServeTaskState::Stopped;
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "D-295-DISTILL-8 acceptance tables use exact Contract Shape markers and diagnostics"
)]
mod shared_network_task_owner_acceptance {
    //! Source-local supervisor lane (seeded-sim, default lane) of GH #295:
    //! S-ND295-19, 29A, 30A, 32, and the S-ND295-34 DNS task-owner bodies.
    //!
    //! The supervisor under test is the private
    //! `SharedNetworkSupervisorHandle::run_shared_network_supervisor(ports,
    //! exec, clock, request_tx, shutdown)` (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the private signature)). Its ports are the
    //! crate-private test-local owner and DNS doubles
    //! (`shared_network_test_ports`, because the `overdrive-sim` doubles
    //! implement a second compiled copy of this crate's traits), a real
    //! `MtlsInterceptWorker` over sim enforcement and resolve and the stateful
    //! test intercept below, and the kill-only `VmKillCapability` over a
    //! `CgroupManager` on one `SimCgroupFs`. Time is the injected `SimClock`;
    //! the supervisor future is polled through a pending-poll probe so every
    //! step waits for the supervisor to register its next wait.
    //!
    //! # SUT state machines
    //!
    //! ```text
    //! EXEC gate:   Open --detect(c)--> Recovering(c, n, t) --clean attempt--> Open
    //!                                  Recovering --20 attempts | 5 s--> FailStop
    //!              Open | Recovering --undetermined quiescence | kill write failed--> FailStop
    //! owner latch: clear --quiesce_managed_taps--> set --restore_quiesced_taps Ok--> clear
    //! allocation:  Active --reported unconfirmed or damaged + kill write Ok/NotFound--> Condemned
    //! ```
    //!
    //! Every observation point also asserts the E11 latch invariant (L9,
    //! FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the latch-set implies gate-not-Open invariant)): while the owner's latch is set, the gate is Recovering or
    //! a fail-stop request was received, and the gate admits no claim.
    //!
    //! # Universe
    //!
    //! Port-exposed observables only: the test-local owner's journal (with the
    //! `SimCgroupFs` snapshot taken as each call began), latch, and condemned
    //! set; the stateful intercept's call log (stamped with the owner journal
    //! length) and its published state; the DNS doubles' probe, audit, and stop
    //! records; the gate's `claim_release` and `recovery_progress`; the typed
    //! request from `ServerHandle::shutdown_requested`; and the supervisor's
    //! `guest_network.shared_owner_*` events.

    use std::collections::BTreeSet;
    use std::future::Future as _;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::num::NonZeroU16;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use futures::FutureExt as _;
    use tracing::field::{Field, Visit};
    use tracing::instrument::WithSubscriber as _;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

    use super::*;
    use crate::dns_responder::responder::Result as DnsResult;
    use crate::dns_responder::{GuestDns, GuestDnsDeps, GuestDnsFactory};
    use crate::shared_network_test_ports::{
        TestAuditMode, TestAuditOutcome, TestCallOutcome, TestGuestDns, TestGuestDnsFactory,
        TestGuestDnsServeExit, TestOwnerCall, TestQuiesceOutcome, TestQuiesceScript,
        TestSharedOwner,
    };
    use overdrive_core::guest_network::{
        GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
        ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStop,
        SharedGuestNetworkFailStopCause, SharedGuestNetworkRecovery,
    };
    use overdrive_core::id::{NodeId, SpiffeId};
    use overdrive_core::traits::IdentityRead;
    use overdrive_core::traits::cgroup_fs::CgroupFs as _;
    use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
    use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
    use overdrive_netlink::nft::SharedIpInterceptIdentity;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_sim::adapters::{
        SimAcceptScript, SimCgroupFs, SimEntry, SimIdentityRead, SimInterceptFault,
        SimMtlsIntercept, SimOp,
    };
    use overdrive_worker::mtls_intercept::{InterceptError, InterceptLeg, InterceptPostcondition};
    use overdrive_worker::mtls_intercept_port::{
        InterceptGuard, InterceptListener, InterceptMembers, InterceptState, MtlsIntercept,
    };
    use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
    use parking_lot::Mutex;

    #[derive(Clone, Copy)]
    enum SupervisorExitCase {
        Returned,
        Failed,
        Panicked,
        Cancelled,
        RequestChannelClosed,
    }

    const COMPONENTS: [SharedGuestNetworkComponent; 12] = [
        SharedGuestNetworkComponent::Bridge,
        SharedGuestNetworkComponent::LegF,
        SharedGuestNetworkComponent::LegC,
        SharedGuestNetworkComponent::Dns,
        SharedGuestNetworkComponent::TcxLink,
        SharedGuestNetworkComponent::EndpointMap,
        SharedGuestNetworkComponent::CounterMap,
        SharedGuestNetworkComponent::BpffsPin,
        SharedGuestNetworkComponent::BridgeGuard,
        SharedGuestNetworkComponent::IpRules,
        SharedGuestNetworkComponent::IpSets,
        SharedGuestNetworkComponent::Supervisor,
    ];

    /// The node-level components the shared guest-network owner repairs
    /// (runtime supervisor matrix, FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the component matrix)).
    const OWNER_COMPONENTS: [SharedGuestNetworkComponent; 6] = [
        SharedGuestNetworkComponent::Bridge,
        SharedGuestNetworkComponent::TcxLink,
        SharedGuestNetworkComponent::EndpointMap,
        SharedGuestNetworkComponent::CounterMap,
        SharedGuestNetworkComponent::BpffsPin,
        SharedGuestNetworkComponent::BridgeGuard,
    ];

    /// Seeds for every seeded supervisor schedule, printed on every verdict.
    const SUPERVISOR_SEEDS_ENV: &str = "OVERDRIVE_SUPERVISOR_SEEDS";
    const DEFAULT_SUPERVISOR_SEEDS: [u64; 2] = [0x2953_3000_0000_0001, 0x2953_3000_5eed_0002];

    /// The injected clock step that proves nothing fires one millisecond early.
    const ONE_MS: Duration = Duration::from_millis(1);

    const LIVE_ALLOCATION: &str = "nd295-live";
    const VM_A: &str = "nd295-vm-a";
    const VM_B: &str = "nd295-vm-b";
    const VM_C: &str = "nd295-vm-c";
    /// A managed-guest set member no registry record owns.
    const STALE_MEMBER: Ipv4Addr = Ipv4Addr::new(100, 95, 3, 7);

    /// `from - by`; every schedule subtracts a step shorter than its period.
    fn earlier(from: Duration, by: Duration) -> Duration {
        from.checked_sub(by).expect("a schedule step is shorter than its period")
    }

    fn supervisor_seeds() -> Vec<u64> {
        let Ok(raw) = std::env::var(SUPERVISOR_SEEDS_ENV) else {
            return DEFAULT_SUPERVISOR_SEEDS.to_vec();
        };
        raw.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                entry
                    .strip_prefix("0x")
                    .map_or_else(|| entry.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
                    .unwrap_or_else(|error| {
                        panic!("{SUPERVISOR_SEEDS_ENV} entry {entry:?} is not a u64: {error}")
                    })
            })
            .collect()
    }

    /// SplitMix64 over the printed seed: the seeded schedule's one entropy
    /// source (the crate has no `rand` dev-dependency).
    struct Schedule(u64);

    impl Schedule {
        const fn new(seed: u64, cell: u64) -> Self {
            Self(seed ^ cell.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        }

        const fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        const fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }

        fn attempts(&mut self, bound: u64) -> u32 {
            u32::try_from(self.below(bound)).expect("a small attempt count fits u32")
        }

        /// A detection-period offset in `1 ..= period - 1` milliseconds.
        fn offset_within(&mut self, period: Duration) -> Duration {
            let span = u64::try_from(period.as_millis()).expect("period fits u64");
            Duration::from_millis(1 + self.below(span.saturating_sub(1).max(1)))
        }
    }

    // -----------------------------------------------------------------------
    // Observations
    // -----------------------------------------------------------------------

    /// What `claim_release` would do right now, polled once and dropped
    /// (TS § *In-process observation*).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Admission {
        /// Pending: BootClosed or Recovering.
        Closed,
        /// `Some(claim)`: Open (the claim is dropped at once).
        Open,
        /// `None`: FailStop.
        FailStopped,
    }

    fn admission(gate: &GuestNetworkExecGate) -> Admission {
        match gate.claim_release().now_or_never() {
            None => Admission::Closed,
            Some(Some(claim)) => {
                drop(claim);
                Admission::Open
            }
            Some(None) => Admission::FailStopped,
        }
    }

    type CgroupSnapshot = BTreeMap<PathBuf, (SimEntry, Vec<u8>)>;

    /// One `guest_network.shared_owner_*` event with the port observations
    /// sampled as it was emitted.
    #[derive(Debug, Clone)]
    struct CapturedEvent {
        name: &'static str,
        fields: BTreeMap<String, String>,
        progress: Option<SharedGuestNetworkRecovery>,
        admission: Admission,
        owner_calls: usize,
        dns_built: usize,
        cgroups: CgroupSnapshot,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    #[derive(Default)]
    struct FieldCapture(BTreeMap<String, String>);

    impl Visit for FieldCapture {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }
    }

    /// The observation surfaces an event is stamped with.
    struct EventProbe {
        exec: Arc<GuestNetworkExecSupervisor>,
        gate: Arc<GuestNetworkExecGate>,
        owner: Arc<TestSharedOwner>,
        dns: Arc<StampedDnsFactory>,
        fs: SimCgroupFs,
        events: Mutex<Vec<CapturedEvent>>,
    }

    /// Captures the supervisor's `guest_network.shared_owner_*` events. It is
    /// installed on the supervisor future itself (`with_subscriber`), so it
    /// sees events on whichever runtime thread polls that future.
    struct SupervisorEventLayer(Arc<EventProbe>);

    impl<S: Subscriber> Layer<S> for SupervisorEventLayer {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let name = event.metadata().name();
            if !name.starts_with("guest_network.shared_owner") {
                return;
            }
            let mut fields = FieldCapture::default();
            event.record(&mut fields);
            let probe = &self.0;
            let captured = CapturedEvent {
                name,
                fields: fields.0,
                progress: probe.exec.recovery_progress(),
                admission: admission(&probe.gate),
                owner_calls: probe.owner.journal().len(),
                dns_built: probe.dns.built(),
                cgroups: probe.fs.snapshot(),
            };
            probe.events.lock().push(captured);
        }
    }

    // -----------------------------------------------------------------------
    // Stateful test intercept (TS § *Intercept listener and stop-error test
    // support*: `S19Intercept` delegates binding to an inner `SimMtlsIntercept`)
    // -----------------------------------------------------------------------

    /// The listener type `bind_transparent` returns.
    type BoundListener = Arc<dyn InterceptListener>;

    struct S19NodeGuard(Arc<AtomicUsize>);

    impl InterceptGuard for S19NodeGuard {}

    impl Drop for S19NodeGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct S19InertGuard;

    impl InterceptGuard for S19InertGuard {}

    /// One call on the stateful intercept.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum InterceptCall {
        Bind {
            requested: SocketAddrV4,
        },
        ConvergeShared {
            prior: bool,
            leg_f: SocketAddrV4,
            leg_c: SocketAddrV4,
            wrote: bool,
            refused: bool,
        },
        ObserveShared,
        ObserveSharedState,
        ConvergeMembers {
            expected: InterceptMembers,
            refused: bool,
        },
        RemoveMembers {
            source: Ipv4Addr,
        },
        InstallOutbound {
            source: Ipv4Addr,
        },
        InstallInbound {
            virt: SocketAddrV4,
        },
    }

    impl InterceptCall {
        /// True for every call that may change the published program, route,
        /// guard, members, or listeners.
        const fn writes(&self) -> bool {
            !matches!(self, Self::ObserveShared | Self::ObserveSharedState)
        }
    }

    /// One logged call, stamped with the test-local owner's journal length as
    /// it began (the cross-owner ordering point).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct InterceptEntry {
        call: InterceptCall,
        owner_calls: usize,
    }

    /// A stateful `MtlsIntercept` modelling the owned constant program, the
    /// policy route, the R18 mark guard, and the dynamic members with the
    /// accepted observation and repair contract (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the eight-method `MtlsIntercept` port through the netlink surface, the runtime member audit, and the runtime repair contract)): the
    /// program is observed by identity whatever the members; `converge_shared`
    /// refuses unless its observation equals `prior`, writes only when the
    /// identity differs, and ensures the route and guard; deleting the table
    /// deletes its sets.
    struct S19Intercept {
        sim: SimMtlsIntercept,
        owner: Arc<TestSharedOwner>,
        program: Mutex<Option<InterceptPostcondition>>,
        policy_route: AtomicBool,
        mark_guard: AtomicBool,
        members: Mutex<InterceptMembers>,
        listener_addresses: Mutex<Vec<SocketAddrV4>>,
        refuse_constant_repair: AtomicBool,
        refuse_member_repair: AtomicBool,
        log: Mutex<Vec<InterceptEntry>>,
        guard_drops: Arc<AtomicUsize>,
    }

    impl S19Intercept {
        fn new(owner: Arc<TestSharedOwner>) -> Self {
            Self {
                sim: SimMtlsIntercept::new(),
                owner,
                program: Mutex::new(None),
                policy_route: AtomicBool::new(false),
                mark_guard: AtomicBool::new(false),
                members: Mutex::new(InterceptMembers::default()),
                listener_addresses: Mutex::new(Vec::new()),
                refuse_constant_repair: AtomicBool::new(false),
                refuse_member_repair: AtomicBool::new(false),
                log: Mutex::new(Vec::new()),
                guard_drops: Arc::new(AtomicUsize::new(0)),
            }
        }

        /// The inner sim adapter every bind delegates to (its listener scripts
        /// are the listener-loss stimulus).
        const fn sim(&self) -> &SimMtlsIntercept {
            &self.sim
        }

        fn identity(leg_f: SocketAddrV4, leg_c: SocketAddrV4) -> InterceptPostcondition {
            let (table_and_chains, sets, prerouting, output) =
                SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
                    .expect("non-zero S19 targets form one canonical identity")
                    .normalized_parts();
            InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output }
        }

        fn record(&self, call: InterceptCall) {
            let owner_calls = self.owner.journal().len();
            self.log.lock().push(InterceptEntry { call, owner_calls });
        }

        /// A foreign actor rewrites one owned rule to a different leg-F target.
        fn publish_wrong_leg_f(&self, recorded_f: SocketAddrV4, recorded_c: SocketAddrV4) {
            let wrong_port = recorded_f.port().checked_add(1).unwrap_or(1);
            assert_ne!(wrong_port, recorded_f.port());
            *self.program.lock() =
                Some(Self::identity(SocketAddrV4::new(*recorded_f.ip(), wrong_port), recorded_c));
        }

        /// The owned table is deleted; its sets, and so every member, go with it.
        fn delete_program(&self) {
            *self.program.lock() = None;
            *self.members.lock() = InterceptMembers::default();
        }

        fn delete_policy_route(&self) {
            self.policy_route.store(false, Ordering::SeqCst);
        }

        fn delete_mark_guard(&self) {
            self.mark_guard.store(false, Ordering::SeqCst);
        }

        fn add_stale_member(&self, member: Ipv4Addr) {
            self.members.lock().managed_guest_ips.insert(member);
        }

        fn refuse_constant_repair(&self, refused: bool) {
            self.refuse_constant_repair.store(refused, Ordering::SeqCst);
        }

        fn refuse_member_repair(&self, refused: bool) {
            self.refuse_member_repair.store(refused, Ordering::SeqCst);
        }

        fn program(&self) -> Option<InterceptPostcondition> {
            self.program.lock().clone()
        }

        fn members(&self) -> InterceptMembers {
            self.members.lock().clone()
        }

        fn policy_route(&self) -> bool {
            self.policy_route.load(Ordering::SeqCst)
        }

        fn mark_guard(&self) -> bool {
            self.mark_guard.load(Ordering::SeqCst)
        }

        fn listener_addresses(&self) -> Vec<SocketAddrV4> {
            self.listener_addresses.lock().clone()
        }

        fn log(&self) -> Vec<InterceptEntry> {
            self.log.lock().clone()
        }

        fn log_since(&self, mark: usize) -> Vec<InterceptEntry> {
            self.log.lock()[mark..].to_vec()
        }

        fn writes_since(&self, mark: usize) -> Vec<InterceptEntry> {
            self.log_since(mark).into_iter().filter(|entry| entry.call.writes()).collect()
        }

        fn guard_drops(&self) -> usize {
            self.guard_drops.load(Ordering::SeqCst)
        }

        fn state(&self) -> Option<InterceptState> {
            let program = self.program.lock().clone()?;
            Some(InterceptState {
                program,
                policy_route: self.policy_route(),
                intercept_mark_guard: self.mark_guard(),
                members: self.members(),
            })
        }

        fn nft_refusal(op: &'static str) -> overdrive_netlink::NetlinkError {
            overdrive_netlink::NetlinkError::nft(
                op,
                std::io::Error::other("scripted repair refusal"),
            )
        }
    }

    impl MtlsIntercept for S19Intercept {
        fn bind_transparent(
            &self,
            addr: SocketAddrV4,
        ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
            self.record(InterceptCall::Bind { requested: addr });
            let listener = self.sim.bind_transparent(addr)?;
            let bound = listener
                .local_addr()
                .map_err(|source| InterceptError::TransparentListener { addr, source })?;
            self.listener_addresses.lock().push(bound);
            Ok(listener)
        }

        fn converge_shared(
            &self,
            prior: Option<&InterceptPostcondition>,
            leg_f: SocketAddrV4,
            leg_c: SocketAddrV4,
        ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            let requested = Self::identity(leg_f, leg_c);
            let observed = self.program.lock().clone();
            if observed.as_ref() != prior {
                self.record(InterceptCall::ConvergeShared {
                    prior: prior.is_some(),
                    leg_f,
                    leg_c,
                    wrote: false,
                    refused: true,
                });
                return Err(InterceptError::PostconditionMismatch {
                    expected: prior.cloned().unwrap_or(requested),
                    observed,
                });
            }
            if self.refuse_constant_repair.load(Ordering::SeqCst) {
                self.record(InterceptCall::ConvergeShared {
                    prior: prior.is_some(),
                    leg_f,
                    leg_c,
                    wrote: false,
                    refused: true,
                });
                return Err(InterceptError::NftSharedReplaceFailed {
                    prior: observed,
                    requested,
                    source: Self::nft_refusal("shared-replace"),
                });
            }
            let wrote = observed.as_ref() != Some(&requested);
            if wrote {
                *self.program.lock() = Some(requested);
            }
            self.policy_route.store(true, Ordering::SeqCst);
            self.mark_guard.store(true, Ordering::SeqCst);
            self.record(InterceptCall::ConvergeShared {
                prior: prior.is_some(),
                leg_f,
                leg_c,
                wrote,
                refused: false,
            });
            Ok(Box::new(S19NodeGuard(Arc::clone(&self.guard_drops))))
        }

        fn observe_shared(
            &self,
        ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
            self.record(InterceptCall::ObserveShared);
            Ok(self.program())
        }

        fn install_outbound(
            &self,
            source_addr: Ipv4Addr,
            _agent_leg_f_port: u16,
        ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            self.record(InterceptCall::InstallOutbound { source: source_addr });
            // The B-8 precondition; this double conflates the record with the
            // program it models.
            if self.program.lock().is_none() {
                return Err(InterceptError::SharedProgramNotConverged);
            }
            let mut members = self.members.lock();
            members.managed_guest_ips.insert(source_addr);
            members.outbound_sources.insert(source_addr);
            drop(members);
            Ok(Box::new(S19InertGuard))
        }

        fn install_inbound(
            &self,
            virt: SocketAddrV4,
            _agent_leg_c_port: u16,
        ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            self.record(InterceptCall::InstallInbound { virt });
            // The B-8 precondition; this double conflates the record with the
            // program it models.
            if self.program.lock().is_none() {
                return Err(InterceptError::SharedProgramNotConverged);
            }
            self.members.lock().inbound_destinations.insert(virt);
            Ok(Box::new(S19InertGuard))
        }

        fn observe_shared_state(
            &self,
        ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
            self.record(InterceptCall::ObserveSharedState);
            Ok(self.state())
        }

        fn converge_allocation_elements(
            &self,
            expected: &InterceptMembers,
        ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
            let refused = self.refuse_member_repair.load(Ordering::SeqCst);
            self.record(InterceptCall::ConvergeMembers { expected: expected.clone(), refused });
            if self.program.lock().is_none() {
                return Ok(None);
            }
            if refused {
                return Err(InterceptError::NftRuleInstallFailed {
                    op: "shared-member-converge",
                    source: Self::nft_refusal("shared-member-converge"),
                });
            }
            *self.members.lock() = expected.clone();
            Ok(self.state())
        }

        fn remove_allocation_elements(
            &self,
            source_addr: Ipv4Addr,
            destinations: &[SocketAddrV4],
        ) -> overdrive_worker::mtls_intercept::Result<InterceptState> {
            self.record(InterceptCall::RemoveMembers { source: source_addr });
            let mut members = self.members.lock();
            members.managed_guest_ips.remove(&source_addr);
            members.outbound_sources.remove(&source_addr);
            for destination in destinations {
                members.inbound_destinations.remove(destination);
            }
            drop(members);
            self.state().ok_or(InterceptError::SharedProgramNotConverged)
        }
    }

    fn s19_worker(intercept: Arc<S19Intercept>, clock: Arc<SimClock>) -> Arc<MtlsInterceptWorker> {
        let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
        let enforcement: Arc<dyn MtlsEnforcement> =
            Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
        let resolve: Arc<dyn MtlsResolve> = Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            BTreeMap::new(),
            MtlsResolution::NonMesh,
        ));
        Arc::new(MtlsInterceptWorker::new(enforcement, resolve, clock, intercept))
    }

    // -----------------------------------------------------------------------
    // Stamped DNS doubles over the test-local `TestGuestDnsFactory`
    // -----------------------------------------------------------------------

    /// One DNS audit: the responder (build order) and where the owner journal
    /// and the intercept call log stood as it began.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct DnsAuditStamp {
        responder: usize,
        owner_calls: usize,
        intercept_calls: usize,
    }

    #[derive(Default)]
    struct DnsRecord {
        /// Owner journal length at each probe and stop, per responder (build
        /// order).
        probes: Vec<(usize, usize)>,
        audits: Vec<DnsAuditStamp>,
        stops: Vec<usize>,
    }

    /// Wraps `TestGuestDnsFactory` so every responder's probe, audit, and stop
    /// is recorded against the owner journal and the intercept call log (the
    /// full-audit ordering points).
    struct StampedDnsFactory {
        inner: TestGuestDnsFactory,
        owner: Arc<TestSharedOwner>,
        intercept: Arc<S19Intercept>,
        record: Arc<Mutex<DnsRecord>>,
    }

    impl StampedDnsFactory {
        fn new(owner: Arc<TestSharedOwner>, intercept: Arc<S19Intercept>) -> Self {
            Self {
                inner: TestGuestDnsFactory::default(),
                owner,
                intercept,
                record: Arc::new(Mutex::new(DnsRecord::default())),
            }
        }

        /// Every DNS audit stamp from the `from`-th audit on.
        fn audit_stamps_since(&self, from: usize) -> Vec<DnsAuditStamp> {
            self.record.lock().audits[from..].to_vec()
        }

        fn built(&self) -> usize {
            self.inner.responders().len()
        }

        /// The most recently built responder: the live one after boot and
        /// after every completed replacement.
        fn live(&self) -> Arc<TestGuestDns> {
            self.inner.responders().pop().expect("the DNS owner holds at least the boot responder")
        }

        fn audits(&self) -> usize {
            self.record.lock().audits.len()
        }

        fn probes(&self) -> usize {
            self.record.lock().probes.len()
        }

        fn stops_of(&self, responder: usize) -> usize {
            self.record.lock().stops.iter().filter(|stopped| **stopped == responder).count()
        }

        fn audits_by(&self, responder: usize) -> usize {
            self.record.lock().audits.iter().filter(|stamp| stamp.responder == responder).count()
        }
    }

    impl GuestDnsFactory for StampedDnsFactory {
        fn responder(&self, deps: GuestDnsDeps) -> Arc<dyn GuestDns> {
            let index = self.built();
            Arc::new(StampedDns {
                inner: self.inner.responder(deps),
                index,
                owner: Arc::clone(&self.owner),
                intercept: Arc::clone(&self.intercept),
                record: Arc::clone(&self.record),
            })
        }
    }

    struct StampedDns {
        inner: Arc<dyn GuestDns>,
        index: usize,
        owner: Arc<TestSharedOwner>,
        intercept: Arc<S19Intercept>,
        record: Arc<Mutex<DnsRecord>>,
    }

    #[async_trait::async_trait]
    impl GuestDns for StampedDns {
        async fn probe(&self) -> DnsResult<()> {
            let owner_calls = self.owner.journal().len();
            self.record.lock().probes.push((self.index, owner_calls));
            self.inner.probe().await
        }

        async fn serve(self: Arc<Self>) {
            Arc::clone(&self.inner).serve().await;
        }

        async fn audit(&self) -> DnsResult<()> {
            let stamp = DnsAuditStamp {
                responder: self.index,
                owner_calls: self.owner.journal().len(),
                intercept_calls: self.intercept.log().len(),
            };
            self.record.lock().audits.push(stamp);
            self.inner.audit().await
        }

        fn stop(&self) {
            self.record.lock().stops.push(self.index);
            self.inner.stop();
        }
    }

    fn dns_deps(clock: &Arc<SimClock>) -> GuestDnsDeps {
        GuestDnsDeps {
            store: Arc::new(SimObservationStore::single_peer(
                NodeId::new("nd295-supervisor").expect("node id"),
                0,
            )),
            clock: Arc::clone(clock) as Arc<dyn Clock>,
            gateway: Ipv4Addr::new(100, 95, 0, 1),
            frontend: crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Fixture helpers
    // -----------------------------------------------------------------------

    fn alloc_id(name: &str) -> AllocationId {
        AllocationId::new(name).expect("allocation id")
    }

    fn guest_pool() -> guest_network::GuestAddressPool {
        guest_network::GuestAddressPool::new(
            ipnet::Ipv4Net::new(Ipv4Addr::new(100, 95, 0, 0), 16).expect("node guest prefix"),
            "ovd-gbr0".to_owned(),
            Ipv4Addr::new(100, 95, 0, 1),
            Ipv4Addr::new(100, 95, 0, 1),
        )
    }

    /// The spec of a shared (guest-network) allocation, as the action shim
    /// hands it to the worker after `provision`.
    fn shared_spec(plan: &guest_network::GuestNetworkPlan) -> AllocationSpec {
        let alloc = plan.alloc().clone();
        AllocationSpec {
            identity: SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/nd295/alloc/{alloc}"
            ))
            .expect("SPIFFE ID"),
            alloc,
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: PathBuf::from("/nd295/kernel"),
                rootfs: PathBuf::from("/nd295/rootfs.ext4"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            network: Some(plan.assignment().clone()),
            service_ports: vec![NonZeroU16::new(8080).expect("non-zero listener port")],
        }
    }

    /// The workloads slice every allocation scope lives under, spelled as the
    /// literal path the contract pins, never derived from the code under test
    /// (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the `SimCgroupFs::snapshot()` oracle: `<root>/overdrive.slice/workloads.slice/cgroup.kill`)).
    fn workloads_slice_dir(root: &Path) -> PathBuf {
        root.join("overdrive.slice/workloads.slice")
    }

    /// `<root>/overdrive.slice/workloads.slice/cgroup.kill`, literally.
    fn slice_kill(root: &Path) -> PathBuf {
        root.join("overdrive.slice/workloads.slice/cgroup.kill")
    }

    /// Whether an event field names `component`. The contract pins the field,
    /// not its rendering (`Debug` or a label), so case and separators are
    /// ignored.
    fn names_component(value: Option<&str>, component: SharedGuestNetworkComponent) -> bool {
        let normalize = |text: &str| {
            text.chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|character| character.to_ascii_lowercase())
                .collect::<String>()
        };
        value.is_some_and(|value| normalize(value) == normalize(&format!("{component:?}")))
    }

    /// Whether an event's `alloc` field names `alloc`, whatever its rendering
    /// (`Display` or `Debug` of the `AllocationId`).
    fn names_alloc(event: &CapturedEvent, alloc: &str) -> bool {
        event.field("alloc").is_some_and(|value| value.contains(alloc))
    }

    /// The one fixture allocation of `candidates` an event's `alloc` field
    /// names, if exactly one.
    fn named_alloc(event: &CapturedEvent, candidates: &[&'static str]) -> Option<&'static str> {
        let named: Vec<_> =
            candidates.iter().copied().filter(|alloc| names_alloc(event, alloc)).collect();
        match named.as_slice() {
            [alloc] => Some(alloc),
            _ => None,
        }
    }

    /// One allocation's scope, spelled as the literal the TS oracle pins
    /// (`<root>/overdrive.slice/workloads.slice/<alloc>.scope`), never derived
    /// through the `CgroupPath` constructors (DISTILL review H10: an oracle must not be computed
    /// by the code it checks).
    fn scope_dir(root: &Path, alloc: &str) -> PathBuf {
        workloads_slice_dir(root).join(format!("{alloc}.scope"))
    }

    fn scope_kill(root: &Path, alloc: &str) -> PathBuf {
        scope_dir(root, alloc).join("cgroup.kill")
    }

    fn killed(snapshot: &CgroupSnapshot, path: &Path) -> bool {
        snapshot.get(path) == Some(&(SimEntry::File, b"1\n".to_vec()))
    }

    fn untouched(snapshot: &CgroupSnapshot, path: &Path) -> bool {
        !snapshot.contains_key(path)
    }

    fn is_audit(call: &TestOwnerCall) -> bool {
        matches!(call, TestOwnerCall::AuditShared(_))
    }

    fn is_quiesce(call: &TestOwnerCall) -> bool {
        matches!(call, TestOwnerCall::Quiesce(_))
    }

    fn is_restore(call: &TestOwnerCall) -> bool {
        matches!(call, TestOwnerCall::Restore(_))
    }

    fn is_owner_converge(call: &TestOwnerCall) -> bool {
        matches!(call, TestOwnerCall::ConvergeShared(_))
    }

    const fn healthy_audit() -> TestOwnerCall {
        TestOwnerCall::AuditShared(TestAuditOutcome::Healthy)
    }

    fn quiesced_none() -> TestOwnerCall {
        TestOwnerCall::Quiesce(TestQuiesceOutcome::Unconfirmed(BTreeSet::new()))
    }

    // -----------------------------------------------------------------------
    // Losses: one per node-level component or task class (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the component matrix))
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Loss {
        /// A node-level part the shared owner audits fails its read-back.
        Owner(SharedGuestNetworkComponent),
        /// The owned nft table is deleted (members go with it).
        ProgramDeleted,
        /// The fwmark rule / table-100 local route is deleted.
        PolicyRouteDeleted,
        /// The R18 intercept-mark guard table is deleted (R18 is provisional).
        MarkGuardDeleted,
        /// A member no registry record owns appears in a shared set.
        StaleMember,
        /// A listener's accept task ends on a lost listener.
        ListenerLost(InterceptLeg),
        /// The DNS serve task returns.
        DnsServeReturned,
        /// The DNS serve task panics.
        DnsServePanicked,
        /// The DNS responder's read-back of its socket identities fails.
        DnsAuditFailed,
    }

    impl Loss {
        const fn component(self) -> SharedGuestNetworkComponent {
            match self {
                Self::Owner(component) => component,
                Self::ProgramDeleted | Self::PolicyRouteDeleted | Self::MarkGuardDeleted => {
                    SharedGuestNetworkComponent::IpRules
                }
                Self::StaleMember => SharedGuestNetworkComponent::IpSets,
                Self::ListenerLost(InterceptLeg::F) => SharedGuestNetworkComponent::LegF,
                Self::ListenerLost(InterceptLeg::C) => SharedGuestNetworkComponent::LegC,
                Self::DnsServeReturned | Self::DnsServePanicked | Self::DnsAuditFailed => {
                    SharedGuestNetworkComponent::Dns
                }
            }
        }

        /// Task exits are detected at once; everything else by the audit.
        const fn immediate(self) -> bool {
            matches!(self, Self::ListenerLost(_) | Self::DnsServeReturned | Self::DnsServePanicked)
        }

        const fn cause(self) -> &'static str {
            if self.immediate() { "task_exit" } else { "audit_mismatch" }
        }

        /// Kernel-path components quiesce TAPs; listener and DNS loss never do.
        const fn kernel_path(self) -> bool {
            !matches!(
                self.component(),
                SharedGuestNetworkComponent::LegF
                    | SharedGuestNetworkComponent::LegC
                    | SharedGuestNetworkComponent::Dns
            )
        }

        /// The owner whose repair heals this loss.
        const fn repair_owner(self) -> RepairOwner {
            match self {
                Self::Owner(_) => RepairOwner::Shared,
                Self::ProgramDeleted
                | Self::PolicyRouteDeleted
                | Self::MarkGuardDeleted
                | Self::StaleMember
                | Self::ListenerLost(_) => RepairOwner::Worker,
                Self::DnsServeReturned | Self::DnsServePanicked | Self::DnsAuditFailed => {
                    RepairOwner::Dns
                }
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RepairOwner {
        Shared,
        Worker,
        Dns,
    }

    fn every_loss() -> Vec<Loss> {
        OWNER_COMPONENTS
            .into_iter()
            .map(Loss::Owner)
            .chain([
                Loss::ProgramDeleted,
                Loss::PolicyRouteDeleted,
                Loss::MarkGuardDeleted,
                Loss::StaleMember,
                Loss::ListenerLost(InterceptLeg::F),
                Loss::ListenerLost(InterceptLeg::C),
                Loss::DnsServeReturned,
                Loss::DnsServePanicked,
                Loss::DnsAuditFailed,
            ])
            .collect()
    }

    /// Where every observation surface stood at one instant.
    #[derive(Debug, Clone, Copy)]
    struct Mark {
        owner_calls: usize,
        intercept_calls: usize,
        dns_built: usize,
        dns_probes: usize,
        dns_audits: usize,
    }

    // -----------------------------------------------------------------------
    // The rig
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, Default)]
    struct RigSetup {
        /// Allocations whose VMM scope directory exists under the workloads
        /// slice (a kill write under a missing parent is `NotFound`).
        scoped: Vec<&'static str>,
        /// Create the workloads slice directory.
        slice: bool,
        /// Start one live shared allocation through the worker, so the node
        /// holds dynamic members.
        live_allocation: bool,
    }

    /// How a rig's schedule is reproduced; printed on every verdict.
    #[derive(Debug, Clone)]
    enum Repro {
        /// A seeded schedule: rerun with `OVERDRIVE_SUPERVISOR_SEEDS=<seed>`.
        Seeded { seed: u64, cell: String },
        /// A fixed schedule with no entropy: its id names it.
        Deterministic { schedule: String },
    }

    impl Repro {
        fn verdict(&self) -> String {
            match self {
                Self::Seeded { seed, cell } => {
                    format!("[{SUPERVISOR_SEEDS_ENV}={seed:#018x} cell {cell}]")
                }
                Self::Deterministic { schedule } => format!("[deterministic schedule {schedule}]"),
            }
        }
    }

    /// One supervisor over the source-local ports, driven on the injected clock.
    struct Rig {
        repro: Repro,
        clock: Arc<SimClock>,
        root: PathBuf,
        fs: SimCgroupFs,
        owner: Arc<TestSharedOwner>,
        intercept: Arc<S19Intercept>,
        dns: Arc<StampedDnsFactory>,
        exec: Arc<GuestNetworkExecSupervisor>,
        gate: Arc<GuestNetworkExecGate>,
        probe: Arc<EventProbe>,
        pool: guest_network::GuestAddressPool,
        server: ServerHandle,
        pending_polls: tokio::sync::mpsc::UnboundedReceiver<()>,
        request: Option<ServeShutdownRequest>,
        request_cgroups: Option<CgroupSnapshot>,
        /// The supervisor's intentional-shutdown token, retained so a cell can
        /// cancel it mid-kill-loop (S-ND295-30A shutdown-mid-loop, user decision
        /// 1 of 2026-09-30). `run_shared_network_supervisor` and the handle hold
        /// their own clones.
        shutdown: CancellationToken,
    }

    /// What survives `Rig::finish` for after-shutdown assertions.
    struct Finished {
        owner: Arc<TestSharedOwner>,
        intercept: Arc<S19Intercept>,
        dns: Arc<StampedDnsFactory>,
    }

    impl Rig {
        /// A seeded rig: every verdict prints the seed and cell.
        async fn build(seed: u64, cell: impl Into<String>, setup: RigSetup) -> Self {
            Self::assemble(Repro::Seeded { seed, cell: cell.into() }, setup).await
        }

        /// A rig for a fixed schedule with no entropy: every verdict prints the
        /// schedule id and no seed.
        async fn build_deterministic(schedule: impl Into<String>, setup: RigSetup) -> Self {
            Self::assemble(Repro::Deterministic { schedule: schedule.into() }, setup).await
        }

        async fn assemble(repro: Repro, setup: RigSetup) -> Self {
            let verdict = repro.verdict();
            let clock = Arc::new(SimClock::new());
            let fs = SimCgroupFs::new();
            let root = PathBuf::from("/sys/fs/cgroup");
            if setup.slice {
                fs.create_dir(&workloads_slice_dir(&root)).await.unwrap_or_else(|error| {
                    panic!("{verdict}: create the workloads slice: {error}")
                });
            }
            for alloc in &setup.scoped {
                fs.create_dir(&scope_dir(&root, alloc)).await.unwrap_or_else(|error| {
                    panic!("{verdict}: create the scope of {alloc}: {error}")
                });
            }
            let owner = Arc::new(TestSharedOwner::with_cgroup_snapshots(
                fs.clone(),
                Arc::clone(&clock) as Arc<dyn Clock>,
            ));
            let intercept = Arc::new(S19Intercept::new(Arc::clone(&owner)));
            let worker = s19_worker(Arc::clone(&intercept), Arc::clone(&clock));
            worker.start_shared_owner().await.unwrap_or_else(|error| {
                panic!("{verdict}: publish the healthy two-listener owner: {error}")
            });
            let pool = guest_pool();
            if setup.live_allocation {
                let plan = pool.assign(alloc_id(LIVE_ALLOCATION)).unwrap_or_else(|error| {
                    panic!("{verdict}: lease the live allocation: {error}")
                });
                worker.start_alloc(&shared_spec(&plan)).await.unwrap_or_else(|error| {
                    panic!("{verdict}: the live shared allocation installs its members: {error}")
                });
            }

            let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
            let exec = wiring.supervisor();
            let gate = wiring.gate();
            assert!(
                exec.open_after_boot(),
                "{verdict}: the fixture's gate starts BootClosed; only its supervisor opens it"
            );

            let dns = Arc::new(StampedDnsFactory::new(Arc::clone(&owner), Arc::clone(&intercept)));
            let responder = dns.responder(dns_deps(&clock));
            responder.probe().await.unwrap_or_else(|error| {
                panic!("{verdict}: the boot responder probes clean: {error}")
            });
            let serve = tokio::spawn(Arc::clone(&responder).serve());
            let ports = SharedNetworkSupervisorPorts {
                shared_guest_network: Arc::clone(&owner)
                    as Arc<dyn guest_network::SharedGuestNetworkOwner>,
                mtls_worker: Arc::clone(&worker),
                dns: DnsServeTaskOwner::new(responder, serve),
                dns_factory: Arc::clone(&dns) as Arc<dyn GuestDnsFactory>,
                dns_deps: dns_deps(&clock),
                vm_kill: vm_kill::VmKillCapability::new(CgroupManager::new(
                    root.clone(),
                    Arc::new(fs.clone()),
                )),
            };

            let probe = Arc::new(EventProbe {
                exec: Arc::clone(&exec),
                gate: Arc::clone(&gate),
                owner: Arc::clone(&owner),
                dns: Arc::clone(&dns),
                fs: fs.clone(),
                events: Mutex::new(Vec::new()),
            });
            let dispatch = tracing::Dispatch::new(
                tracing_subscriber::registry().with(SupervisorEventLayer(Arc::clone(&probe))),
            );
            let shutdown = CancellationToken::new();
            let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
            let supervisor = SharedNetworkSupervisorHandle::run_shared_network_supervisor(
                ports,
                Arc::clone(&exec),
                Arc::clone(&clock) as Arc<dyn Clock>,
                request_tx,
                shutdown.clone(),
            )
            .with_subscriber(dispatch);
            let mut supervisor = Box::pin(supervisor);
            let (pending_poll_tx, pending_polls) = tokio::sync::mpsc::unbounded_channel();
            let poll_verdict = verdict.clone();
            let task = tokio::spawn(std::future::poll_fn(move |context| {
                let polled = supervisor.as_mut().poll(context);
                if polled.is_pending() {
                    pending_poll_tx.send(()).unwrap_or_else(|_| {
                        panic!(
                            "{poll_verdict}: the rig observes the supervisor cadence until \
                             shutdown"
                        )
                    });
                }
                polled
            }));
            let handle = SharedNetworkSupervisorHandle::new(
                request_rx,
                task,
                Arc::clone(&exec),
                shutdown.clone(),
            );
            let server = s19_server_handle(handle, Arc::clone(&worker));

            let mut rig = Self {
                repro,
                clock,
                root,
                fs,
                owner,
                intercept,
                dns,
                exec,
                gate,
                probe,
                pool,
                server,
                pending_polls,
                request: None,
                request_cgroups: None,
                shutdown,
            };
            rig.await_wake().await;
            rig
        }

        fn verdict(&self) -> String {
            self.repro.verdict()
        }

        /// Request intentional shutdown (SIGINT/SIGTERM): cancel the
        /// supervisor's token. Under user decision 1 of 2026-09-30 it cancels
        /// only BETWEEN kill loops — a loop in progress runs to its end first.
        fn request_intentional_shutdown(&self) {
            self.shutdown.cancel();
        }

        async fn drain(&mut self, quiet: Duration) {
            while tokio::time::timeout(quiet, self.pending_polls.recv()).await == Ok(Some(())) {}
        }

        /// Wait until the supervisor has woken and registered its next wait.
        async fn await_wake(&mut self) {
            let woke =
                tokio::time::timeout(Duration::from_secs(5), self.pending_polls.recv()).await;
            assert!(
                matches!(woke, Ok(Some(()))),
                "{}: the supervisor wakes and registers its next wait",
                self.verdict()
            );
            self.drain(Duration::from_millis(25)).await;
            self.check_invariants();
        }

        /// Advance to an instant at which the supervisor is due to wake.
        async fn advance(&mut self, by: Duration) {
            self.clock.tick(by);
            self.await_wake().await;
        }

        /// Advance to an instant at which nothing is due; any wake is settled.
        async fn advance_quiet(&mut self, by: Duration) {
            self.clock.tick(by);
            self.drain(Duration::from_millis(50)).await;
            self.check_invariants();
        }

        async fn attempt(&mut self) {
            self.advance(super::SHARED_NETWORK_RETRY_PERIOD).await;
        }

        /// Run until the supervisor's first audit, so every schedule is
        /// anchored on an audit start whatever the first audit's instant.
        async fn run_until_first_audit(&mut self) {
            for _ in 0..2 {
                if self.calls().iter().any(is_audit) {
                    break;
                }
                self.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            }
            assert_eq!(
                self.calls(),
                [healthy_audit()],
                "{}: one healthy audit anchors the schedule",
                self.verdict()
            );
            assert_eq!(self.exec.recovery_progress(), None, "{}", self.verdict());
            assert_eq!(self.admission(), Admission::Open, "{}", self.verdict());
            assert_eq!(self.dns.audits(), 1, "{}: the full audit reads DNS", self.verdict());
        }

        /// E11 latch invariant (L9), read from the owner's own latch at every
        /// observation point.
        fn check_invariants(&mut self) {
            let requested = self.poll_request().is_some();
            if self.owner.latched() {
                assert!(
                    self.exec.recovery_progress().is_some() || requested,
                    "{}: a set latch implies Recovering or a received fail-stop request",
                    self.verdict()
                );
                assert_ne!(
                    admission(&self.gate),
                    Admission::Open,
                    "{}: the gate admits no claim while the latch is set",
                    self.verdict()
                );
            }
        }

        /// The one fail-stop request, observed through the retained
        /// `ServerHandle` (polled once, never awaited).
        fn poll_request(&mut self) -> Option<&ServeShutdownRequest> {
            if self.request.is_none()
                && let Some(request) = self.server.shutdown_requested().now_or_never()
            {
                self.request_cgroups = Some(self.fs.snapshot());
                self.request = Some(request);
            }
            self.request.as_ref()
        }

        fn admission(&self) -> Admission {
            admission(&self.gate)
        }

        fn calls(&self) -> Vec<TestOwnerCall> {
            self.owner.journal().into_iter().map(|entry| entry.call).collect()
        }

        fn calls_since(&self, mark: usize) -> Vec<TestOwnerCall> {
            self.calls()[mark..].to_vec()
        }

        fn count_since(&self, mark: usize, pred: fn(&TestOwnerCall) -> bool) -> usize {
            self.calls_since(mark).iter().filter(|call| pred(call)).count()
        }

        fn events(&self, name: &str) -> Vec<CapturedEvent> {
            self.probe.events.lock().iter().filter(|event| event.name == name).cloned().collect()
        }

        fn unhealthy(&self) -> Vec<CapturedEvent> {
            self.events("guest_network.shared_owner_unhealthy")
        }

        fn vm_killed(&self) -> Vec<CapturedEvent> {
            self.events("guest_network.shared_owner_vm_killed")
        }

        fn mark(&self) -> Mark {
            Mark {
                owner_calls: self.owner.journal().len(),
                intercept_calls: self.intercept.log().len(),
                dns_built: self.dns.built(),
                dns_probes: self.dns.probes(),
                dns_audits: self.dns.audits(),
            }
        }

        fn leg_address(&self, leg: InterceptLeg) -> SocketAddrV4 {
            let addresses = self.intercept.listener_addresses();
            assert!(addresses.len() >= 2, "{}: both legs are bound", self.verdict());
            match leg {
                InterceptLeg::F => addresses[0],
                InterceptLeg::C => addresses[1],
            }
        }

        fn inject(&self, loss: Loss) {
            match loss {
                Loss::Owner(component) => {
                    self.owner.script_component_audit_failure(component, true);
                }
                Loss::ProgramDeleted => self.intercept.delete_program(),
                Loss::PolicyRouteDeleted => self.intercept.delete_policy_route(),
                Loss::MarkGuardDeleted => self.intercept.delete_mark_guard(),
                Loss::StaleMember => self.intercept.add_stale_member(STALE_MEMBER),
                Loss::ListenerLost(leg) => {
                    let at = self.leg_address(leg);
                    assert!(
                        self.intercept.sim().script_accept(
                            at,
                            SimAcceptScript::ListenerLost { errno: libc::EINVAL }
                        ),
                        "{}: the {leg:?} listener at {at} is a live port-owned listener",
                        self.verdict()
                    );
                }
                Loss::DnsServeReturned => self.dns.live().end_serve(TestGuestDnsServeExit::Return),
                Loss::DnsServePanicked => self.dns.live().end_serve(TestGuestDnsServeExit::Panic),
                Loss::DnsAuditFailed => self.dns.live().script_audit_failure(true),
            }
        }

        /// Keep the owning component's repair failing (`true`) or let it
        /// succeed (`false`). For a shared-owner component the standing audit
        /// slot is the loss itself, so releasing it heals the part.
        fn block_repair(&self, loss: Loss, blocked: bool) {
            match loss {
                Loss::Owner(component) => {
                    if !blocked {
                        self.owner.script_component_audit_failure(component, false);
                    }
                }
                Loss::ProgramDeleted | Loss::PolicyRouteDeleted | Loss::MarkGuardDeleted => {
                    self.intercept.refuse_constant_repair(blocked);
                }
                Loss::StaleMember => self.intercept.refuse_member_repair(blocked),
                Loss::ListenerLost(_) => {
                    if blocked {
                        self.intercept.sim().script_bind_fault(
                            SimInterceptFault::TransparentListener { errno: libc::EADDRINUSE },
                        );
                    } else {
                        self.intercept.sim().clear_faults();
                    }
                }
                Loss::DnsServeReturned | Loss::DnsServePanicked | Loss::DnsAuditFailed => {
                    self.dns.inner.script_probe_failure(blocked);
                }
            }
        }

        /// Inject `loss` `offset` after the last audit started and drive the
        /// supervisor to its detection; returns the mark taken just before.
        async fn detect(&mut self, loss: Loss, offset: Duration) -> Mark {
            self.advance_quiet(offset).await;
            let mark = self.mark();
            self.inject(loss);
            if loss.immediate() {
                self.await_wake().await;
            } else {
                self.advance(earlier(super::SHARED_NETWORK_AUDIT_PERIOD, offset)).await;
            }
            mark
        }

        /// EXEC closes before the one typed announcement of `component`.
        fn assert_detected(
            &self,
            component: SharedGuestNetworkComponent,
            cause: &str,
            since: Mark,
        ) {
            let verdict = self.verdict();
            assert_eq!(
                self.exec.recovery_progress(),
                Some(SharedGuestNetworkRecovery {
                    component,
                    attempts: 0,
                    elapsed: Duration::ZERO
                }),
                "{verdict}: detection begins recovery of the first failing component"
            );
            assert_eq!(self.admission(), Admission::Closed, "{verdict}: detection closes EXEC");
            let announced: Vec<_> = self
                .unhealthy()
                .into_iter()
                .filter(|event| event.owner_calls >= since.owner_calls)
                .collect();
            assert_eq!(
                announced.len(),
                1,
                "{verdict}: one announcement per detection: {announced:?}"
            );
            let event = &announced[0];
            assert!(
                names_component(event.field("component"), component),
                "{verdict}: the announcement names {component:?}: {event:?}"
            );
            assert_eq!(event.field("cause"), Some(cause), "{verdict}");
            assert_eq!(
                event.progress.as_ref().map(|progress| progress.component),
                Some(component),
                "{verdict}: begin_recovery precedes the announcement"
            );
            assert_eq!(
                event.admission,
                Admission::Closed,
                "{verdict}: EXEC closes before the announcement"
            );
        }

        /// The loss's own owner repaired it since `mark`, and no other owner
        /// was asked to.
        fn assert_repaired_through_owner(&self, loss: Loss, mark: Mark) {
            let verdict = self.verdict();
            let owner_converges = self.count_since(mark.owner_calls, is_owner_converge);
            let intercept_writes = self.intercept.writes_since(mark.intercept_calls);
            let replacements = self.dns.built() - mark.dns_built;
            match loss.repair_owner() {
                RepairOwner::Shared => {
                    assert!(owner_converges >= 1, "{verdict}: the shared owner converges its part");
                    assert!(intercept_writes.is_empty(), "{verdict}: {intercept_writes:?}");
                    assert_eq!(replacements, 0, "{verdict}: DNS is not replaced");
                }
                RepairOwner::Worker => {
                    assert_eq!(
                        owner_converges, 0,
                        "{verdict}: the shared owner had nothing to repair"
                    );
                    assert_eq!(replacements, 0, "{verdict}: DNS is not replaced");
                    let calls: Vec<_> =
                        intercept_writes.iter().map(|entry| entry.call.clone()).collect();
                    let repaired = match loss {
                        Loss::ProgramDeleted => calls.iter().any(|call| {
                            matches!(
                                call,
                                InterceptCall::ConvergeShared {
                                    prior: false,
                                    wrote: true,
                                    refused: false,
                                    ..
                                }
                            )
                        }),
                        Loss::PolicyRouteDeleted | Loss::MarkGuardDeleted => {
                            calls.iter().any(|call| {
                                matches!(
                                    call,
                                    InterceptCall::ConvergeShared {
                                        prior: true,
                                        wrote: false,
                                        refused: false,
                                        ..
                                    }
                                )
                            })
                        }
                        Loss::StaleMember => calls.iter().any(|call| {
                            matches!(call, InterceptCall::ConvergeMembers { refused: false, .. })
                        }),
                        Loss::ListenerLost(leg) => {
                            let recorded = self.leg_address(leg);
                            calls.contains(&InterceptCall::Bind { requested: recorded })
                        }
                        _ => unreachable!("worker-owned losses only"),
                    };
                    assert!(repaired, "{verdict}: the worker repaired {loss:?}: {calls:?}");
                }
                RepairOwner::Dns => {
                    assert_eq!(
                        owner_converges, 0,
                        "{verdict}: the shared owner had nothing to repair"
                    );
                    assert!(intercept_writes.is_empty(), "{verdict}: {intercept_writes:?}");
                    assert!(
                        replacements >= 1,
                        "{verdict}: a fresh responder replaces the lost one"
                    );
                    assert!(
                        self.dns.probes() > mark.dns_probes,
                        "{verdict}: the replacement is probed before it serves"
                    );
                }
            }
            if loss.repair_owner() == RepairOwner::Worker {
                assert!(
                    self.intercept.policy_route() && self.intercept.mark_guard(),
                    "{verdict}: the constant route and guard are present after repair"
                );
                assert_eq!(
                    self.intercept.program(),
                    Some(S19Intercept::identity(
                        self.leg_address(InterceptLeg::F),
                        self.leg_address(InterceptLeg::C)
                    )),
                    "{verdict}: the program names exactly the recorded listeners"
                );
            }
        }

        /// Recovery from `loss` with its repair blocked for `blocked` attempts
        /// reopens exactly once, through its owner, restoring quiesced TAPs.
        async fn recover(&mut self, loss: Loss, offset: Duration, blocked: u32) -> Mark {
            let before = self.mark();
            if blocked > 0 {
                self.block_repair(loss, true);
            }
            let detection = self.detect(loss, offset).await;
            self.assert_detected(loss.component(), loss.cause(), before);
            let verdict = self.verdict();
            assert_eq!(
                self.count_since(detection.owner_calls, is_quiesce),
                usize::from(loss.kernel_path()),
                "{verdict}: kernel-path components quiesce once; listener and DNS loss never do"
            );
            if loss.kernel_path() {
                assert!(self.owner.latched(), "{verdict}: quiescence sets the latch");
            }
            for attempt in 1..=blocked {
                self.attempt().await;
                let progress = self
                    .exec
                    .recovery_progress()
                    .unwrap_or_else(|| panic!("{verdict}: attempt {attempt} is still recovering"));
                assert_eq!(
                    progress.attempts, attempt,
                    "{verdict}: attempts count on the 250 ms cadence"
                );
                assert_eq!(progress.component, loss.component(), "{verdict}");
                assert_eq!(self.admission(), Admission::Closed, "{verdict}");
                assert_eq!(
                    self.count_since(detection.owner_calls, is_restore),
                    0,
                    "{verdict}: no TAP is restored while a component fails"
                );
            }
            self.block_repair(loss, false);
            self.attempt().await;
            assert_eq!(self.exec.recovery_progress(), None, "{verdict}: a clean attempt reopens");
            assert_eq!(self.admission(), Admission::Open, "{verdict}");
            self.assert_repaired_through_owner(loss, detection);
            let since = self.calls_since(detection.owner_calls);
            let restores: Vec<_> = since.iter().filter(|call| is_restore(call)).cloned().collect();
            if loss.kernel_path() {
                assert_eq!(
                    restores,
                    [TestOwnerCall::Restore(TestCallOutcome::Ok)],
                    "{verdict}: one restore after the clean audit"
                );
                let restore_at = since
                    .iter()
                    .position(is_restore)
                    .unwrap_or_else(|| panic!("{verdict}: the recovery restores"));
                assert_eq!(
                    since[restore_at - 1],
                    healthy_audit(),
                    "{verdict}: restore follows a clean shared audit"
                );
                assert!(!self.owner.latched(), "{verdict}: restore cleared the latch");
            } else {
                assert!(
                    restores.is_empty(),
                    "{verdict}: nothing was quiesced, so nothing is restored"
                );
            }
            detection
        }

        /// Reopen happened once: two more audit periods stay Open with no new
        /// announcement, quiescence, restore, or repair.
        async fn assert_single_reopen(&mut self) {
            let reopened = self.mark();
            let announcements = self.unhealthy().len();
            for _ in 0..2 {
                self.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
                let verdict = self.verdict();
                assert_eq!(self.exec.recovery_progress(), None, "{verdict}: stays Open");
                assert_eq!(self.admission(), Admission::Open, "{verdict}");
            }
            let verdict = self.verdict();
            assert_eq!(self.unhealthy().len(), announcements, "{verdict}: no second detection");
            let since = self.calls_since(reopened.owner_calls);
            assert_eq!(
                since,
                [healthy_audit(), healthy_audit()],
                "{verdict}: one audit per period and nothing else"
            );
            assert!(self.intercept.writes_since(reopened.intercept_calls).is_empty(), "{verdict}");
            assert_eq!(self.dns.built(), reopened.dns_built, "{verdict}");
        }

        async fn finish(self) -> Finished {
            let verdict = self.verdict();
            let Self { server, pending_polls, owner, intercept, dns, .. } = self;
            server.shutdown(Duration::from_millis(10)).await.unwrap_or_else(|error| {
                panic!(
                    "{verdict}: the sole terminal ServerHandle owner drains and joins every \
                     retained owner: {error}"
                )
            });
            drop(pending_polls);
            Finished { owner, intercept, dns }
        }
    }

    fn s19_inert_task(shutdown: &CancellationToken) -> tokio::task::JoinHandle<()> {
        let shutdown = shutdown.clone();
        tokio::spawn(async move { shutdown.cancelled().await })
    }

    fn s19_server_handle(
        shared_network_supervisor: SharedNetworkSupervisorHandle,
        mtls_worker_owner: Arc<MtlsInterceptWorker>,
    ) -> ServerHandle {
        let convergence_shutdown = CancellationToken::new();
        let emit_drain_shutdown = CancellationToken::new();
        let interest_router_shutdown = CancellationToken::new();
        let sim_store: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-server-handle").expect("node id"),
            0,
        ));
        let mtls_resolve_owner =
            Arc::new(crate::mtls_resolve_adapter::ServiceBackendsResolve::new(
                sim_store,
                crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new(),
            ));
        ServerHandle {
            inner: AxumHandle::new(),
            server_task: tokio::spawn(async { Ok::<(), std::io::Error>(()) }),
            convergence_task: s19_inert_task(&convergence_shutdown),
            exit_observer_tasks: Vec::new(),
            emit_drain_task: s19_inert_task(&emit_drain_shutdown),
            interest_router_task: s19_inert_task(&interest_router_shutdown),
            convergence_shutdown,
            exit_observer_shutdown: CancellationToken::new(),
            emit_drain_shutdown,
            interest_router_shutdown,
            mtls_worker_owner,
            mtls_resolve_owner,
            shared_network_supervisor,
        }
    }

    // -----------------------------------------------------------------------
    // S-ND295-19
    // -----------------------------------------------------------------------

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-19 — Live repair never redirects protection to a different listener
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Schedule `s19-wrong-leg-f-target` (deterministic `SimClock`, no seed).
    /// One owned rule names a different leg-F target. Detection is one full
    /// audit (the shared owner, then the worker's read of the program, then
    /// DNS) followed by one quiescence. Each of the twenty 250 ms attempts is,
    /// in this order: one worker repair (`converge_shared_owner` observes the
    /// program through `observe_shared_state` and refuses the differing
    /// identity without writing; the observation precedes the attempt's
    /// shared-owner audit), then one full audit of all three owners (the
    /// shared owner, the worker's read, then DNS). Nothing is restored; at
    /// five seconds exactly one `IpRules / RecoveryDeadlineExceeded / 20 /
    /// 5 s` request is sent and no twenty-first attempt runs. Terminal
    /// ownership stays with `ServerHandle::shutdown` (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the S19 consequence); FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the runtime member audit and the runtime repair contract)).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-19)"]
    async fn published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop()
     {
        assert_eq!(
            super::SHARED_NETWORK_RETRY_PERIOD * super::SHARED_NETWORK_RECOVERY_ATTEMPTS,
            super::SHARED_NETWORK_RECOVERY_DEADLINE,
            "ADR-0124: the twentieth attempt lands on the five-second deadline"
        );
        let mut rig = Rig::build_deterministic("s19-wrong-leg-f-target", RigSetup::default()).await;
        rig.run_until_first_audit().await;
        let verdict = rig.verdict();
        let leg_f = rig.leg_address(InterceptLeg::F);
        let leg_c = rig.leg_address(InterceptLeg::C);
        assert!(
            leg_f.port() != 0 && leg_c.port() != 0,
            "{verdict}: both legs are bound to real ports"
        );
        assert_eq!(
            rig.intercept.program(),
            Some(S19Intercept::identity(leg_f, leg_c)),
            "{verdict}"
        );
        rig.intercept.publish_wrong_leg_f(leg_f, leg_c);
        let wrong = rig
            .intercept
            .program()
            .unwrap_or_else(|| panic!("{verdict}: the canonical wrong target is present"));
        let start = rig.mark();

        rig.advance_quiet(earlier(super::SHARED_NETWORK_AUDIT_PERIOD, ONE_MS)).await;
        assert_eq!(rig.exec.recovery_progress(), None, "{verdict}: no detection before the audit");
        assert!(rig.calls_since(start.owner_calls).is_empty(), "{verdict}");
        assert!(rig.intercept.log_since(start.intercept_calls).is_empty(), "{verdict}");
        assert!(rig.poll_request().is_none(), "{verdict}");

        rig.advance(ONE_MS).await;
        rig.assert_detected(SharedGuestNetworkComponent::IpRules, "audit_mismatch", start);
        assert_eq!(
            rig.calls_since(start.owner_calls),
            [healthy_audit(), quiesced_none()],
            "{verdict}: detection is one full audit, then one quiescence"
        );
        assert_eq!(
            rig.intercept.log_since(start.intercept_calls),
            [InterceptEntry {
                call: InterceptCall::ObserveSharedState,
                owner_calls: start.owner_calls + 1,
            }],
            "{verdict}: the detection audit's worker read follows the shared-owner audit"
        );
        assert_eq!(
            rig.dns.audit_stamps_since(start.dns_audits),
            [DnsAuditStamp {
                responder: 0,
                owner_calls: start.owner_calls + 1,
                intercept_calls: start.intercept_calls + 1,
            }],
            "{verdict}: the full audit reads DNS last, after the worker"
        );

        let mut expected_calls = vec![healthy_audit(), quiesced_none()];
        for attempt in 1..super::SHARED_NETWORK_RECOVERY_ATTEMPTS {
            let prior = super::SHARED_NETWORK_RETRY_PERIOD * (attempt - 1);
            rig.advance_quiet(earlier(super::SHARED_NETWORK_RETRY_PERIOD, ONE_MS)).await;
            assert_eq!(
                rig.exec.recovery_progress(),
                Some(SharedGuestNetworkRecovery {
                    component: SharedGuestNetworkComponent::IpRules,
                    attempts: attempt - 1,
                    elapsed: earlier(prior + super::SHARED_NETWORK_RETRY_PERIOD, ONE_MS),
                }),
                "{verdict}: attempt {attempt} does not start early"
            );
            let at = rig.mark();
            rig.advance(ONE_MS).await;
            assert_eq!(
                rig.exec.recovery_progress(),
                Some(SharedGuestNetworkRecovery {
                    component: SharedGuestNetworkComponent::IpRules,
                    attempts: attempt,
                    elapsed: super::SHARED_NETWORK_RETRY_PERIOD * attempt,
                }),
                "{verdict}: attempt {attempt} completes on the 250 ms cadence"
            );
            assert_eq!(
                rig.intercept.log_since(at.intercept_calls),
                [
                    InterceptEntry {
                        call: InterceptCall::ObserveSharedState,
                        owner_calls: at.owner_calls,
                    },
                    InterceptEntry {
                        call: InterceptCall::ObserveSharedState,
                        owner_calls: at.owner_calls + 1,
                    },
                ],
                "{verdict}: attempt {attempt} is one worker repair (observe, refuse, no write) \
                 before the shared-owner audit, then the full audit's worker read after it"
            );
            assert_eq!(
                rig.calls_since(at.owner_calls),
                [healthy_audit()],
                "{verdict}: attempt {attempt} makes one shared-owner call, its audit"
            );
            assert_eq!(
                rig.dns.audit_stamps_since(at.dns_audits),
                [DnsAuditStamp {
                    responder: 0,
                    owner_calls: at.owner_calls + 1,
                    intercept_calls: at.intercept_calls + 2,
                }],
                "{verdict}: attempt {attempt}'s full audit reads DNS once, last"
            );
            expected_calls.push(healthy_audit());
            assert_eq!(
                rig.calls_since(start.owner_calls),
                expected_calls,
                "{verdict}: each attempt adds one shared audit and nothing else"
            );
            assert!(
                rig.intercept.writes_since(start.intercept_calls).is_empty(),
                "{verdict}: the differently targeted program is never rewritten"
            );
            assert_eq!(rig.intercept.program().as_ref(), Some(&wrong), "{verdict}");
            assert!(rig.poll_request().is_none(), "{verdict}: no request before the deadline");
        }

        rig.advance_quiet(earlier(super::SHARED_NETWORK_RETRY_PERIOD, ONE_MS)).await;
        assert!(rig.poll_request().is_none(), "{verdict}");
        let last = rig.mark();
        rig.advance(ONE_MS).await;
        assert_eq!(
            rig.intercept.log_since(last.intercept_calls),
            [
                InterceptEntry {
                    call: InterceptCall::ObserveSharedState,
                    owner_calls: last.owner_calls,
                },
                InterceptEntry {
                    call: InterceptCall::ObserveSharedState,
                    owner_calls: last.owner_calls + 1,
                },
            ],
            "{verdict}: attempt 20 is one worker repair, then the full audit"
        );
        assert_eq!(rig.dns.audits(), last.dns_audits + 1, "{verdict}: attempt 20 reads DNS once");
        let expected = ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
            component: SharedGuestNetworkComponent::IpRules,
            cause: SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
            attempts: super::SHARED_NETWORK_RECOVERY_ATTEMPTS,
            elapsed: super::SHARED_NETWORK_RECOVERY_DEADLINE,
        });
        assert_eq!(rig.poll_request(), Some(&expected), "{verdict}");
        expected_calls.push(healthy_audit());
        assert_eq!(
            rig.calls_since(start.owner_calls),
            expected_calls,
            "{verdict}: attempt 20 runs, then fail-stop"
        );
        assert!(
            rig.exec.fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded).is_none(),
            "{verdict}: the supervisor already entered FailStop exactly once"
        );
        assert_eq!(rig.exec.recovery_progress(), None, "{verdict}: FailStop consumes the snapshot");
        assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
        assert!(rig.intercept.writes_since(start.intercept_calls).is_empty(), "{verdict}");
        assert_eq!(rig.intercept.program().as_ref(), Some(&wrong), "{verdict}");
        assert!(
            rig.server
                .shared_network_supervisor
                .task
                .as_ref()
                .is_some_and(|task| !task.is_finished()),
            "{verdict}: the supervisor parks after its one request"
        );

        let terminal = rig.mark();
        rig.advance_quiet(super::SHARED_NETWORK_RECOVERY_DEADLINE).await;
        assert_eq!(rig.calls().len(), terminal.owner_calls, "{verdict}: no twenty-first attempt");
        assert_eq!(rig.intercept.log().len(), terminal.intercept_calls, "{verdict}");
        assert_eq!(rig.dns.audits(), terminal.dns_audits, "{verdict}");
        assert_eq!(rig.unhealthy().len(), 1, "{verdict}: one announcement in total");
        assert_eq!(rig.intercept.guard_drops(), 0, "{verdict}: the published guard is retained");

        let finished = rig.finish().await;
        assert_eq!(
            finished.owner.journal().len(),
            terminal.owner_calls,
            "{verdict}: shutdown adds no owner effect"
        );
        assert_eq!(finished.intercept.log().len(), terminal.intercept_calls, "{verdict}");
        assert_eq!(
            finished.intercept.guard_drops(),
            0,
            "{verdict}: sealed terminal relinquishment does not invoke the constant-program guard's Drop"
        );
        assert_eq!(finished.dns.audits(), terminal.dns_audits, "{verdict}");
    }

    // -----------------------------------------------------------------------
    // S-ND295-29A
    // -----------------------------------------------------------------------

    fn loss_code(loss: Loss) -> u64 {
        let index =
            every_loss().iter().position(|candidate| *candidate == loss).expect("known loss");
        u64::try_from(index).expect("index fits u64") + 1
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Every node-level component and task class (the six shared-owner parts;
    /// the program, policy route, and mark guard of `IpRules`; `IpSets`; each
    /// listener; DNS serve return, serve panic, and audit failure), each
    /// injected at a seeded instant within the audit period and kept failing
    /// for a seeded 0-2 attempts: detection within one audit period (at once
    /// for a task exit) closes EXEC before one announcement, the loss is
    /// repaired through its own owner, quiesced TAPs are restored only after a
    /// clean full audit, and EXEC reopens exactly once. The `Supervisor`
    /// component is the join classification of the retained exit matrix.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn every_component_loss_is_detected_and_recovered_through_its_owner() {
        for seed in supervisor_seeds() {
            for loss in every_loss() {
                let mut schedule = Schedule::new(seed, loss_code(loss));
                let blocked = schedule.attempts(3);
                let offset = schedule.offset_within(super::SHARED_NETWORK_AUDIT_PERIOD);
                let mut rig = Rig::build(
                    seed,
                    format!("{loss:?}/blocked-{blocked}/offset-{offset:?}"),
                    RigSetup::default(),
                )
                .await;
                rig.run_until_first_audit().await;
                rig.recover(loss, offset, blocked).await;
                rig.assert_single_reopen().await;
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// TAPs are quiesced once per recovery for a kernel-path component,
    /// however long it stays failing, and never for a listener or DNS loss;
    /// an attempt whose audit first reveals a kernel-path component during a
    /// listener or DNS recovery quiesces at that point, once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn kernel_path_components_quiesce_and_listener_or_dns_loss_never_does() {
        for seed in supervisor_seeds() {
            for loss in every_loss() {
                let mut schedule = Schedule::new(seed, loss_code(loss) + 100);
                let blocked = 2 + schedule.attempts(3);
                let mut rig = Rig::build(
                    seed,
                    format!("{loss:?}/long-outage-{blocked}"),
                    RigSetup::default(),
                )
                .await;
                rig.run_until_first_audit().await;
                let detection = rig.recover(loss, ONE_MS, blocked).await;
                assert_eq!(
                    rig.count_since(detection.owner_calls, is_quiesce),
                    usize::from(loss.kernel_path()),
                    "{}: one quiescence for the whole outage",
                    rig.verdict()
                );
                rig.finish().await;
            }

            for first in [
                Loss::ListenerLost(InterceptLeg::F),
                Loss::ListenerLost(InterceptLeg::C),
                Loss::DnsServeReturned,
                Loss::DnsAuditFailed,
            ] {
                let mut rig =
                    Rig::build(seed, format!("{first:?}-then-Bridge"), RigSetup::default()).await;
                rig.run_until_first_audit().await;
                let before = rig.mark();
                rig.block_repair(first, true);
                let detection = rig.detect(first, ONE_MS).await;
                rig.assert_detected(first.component(), first.cause(), before);
                let verdict = rig.verdict();
                assert_eq!(rig.count_since(detection.owner_calls, is_quiesce), 0, "{verdict}");
                rig.inject(Loss::Owner(SharedGuestNetworkComponent::Bridge));
                let revealing = rig.mark();
                rig.attempt().await;
                let since = rig.calls_since(revealing.owner_calls);
                assert_eq!(
                    since,
                    [
                        TestOwnerCall::AuditShared(TestAuditOutcome::NodeFailed(
                            SharedGuestNetworkComponent::Bridge
                        )),
                        quiesced_none(),
                    ],
                    "{verdict}: the attempt whose audit reveals a kernel-path loss quiesces there"
                );
                assert_eq!(
                    rig.exec.recovery_progress().map(|progress| progress.component),
                    Some(SharedGuestNetworkComponent::Bridge),
                    "{verdict}: Bridge is first in D8 order"
                );
                rig.attempt().await;
                assert_eq!(
                    rig.count_since(detection.owner_calls, is_quiesce),
                    1,
                    "{verdict}: still once"
                );
                rig.block_repair(first, false);
                rig.block_repair(Loss::Owner(SharedGuestNetworkComponent::Bridge), false);
                rig.attempt().await;
                assert_eq!(
                    rig.exec.recovery_progress(),
                    None,
                    "{verdict}: both repaired, EXEC reopens"
                );
                assert_eq!(rig.count_since(detection.owner_calls, is_restore), 1, "{verdict}");
                assert!(!rig.owner.latched(), "{verdict}");
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// An `IpRules`-only or `IpSets`-only loss is repaired by the worker alone,
    /// yet the TAPs the detection quiesced are restored after the clean audit,
    /// before EXEC reopens: the owner journal from detection is exactly one
    /// audit, one quiescence, one audit per attempt, and one successful
    /// restore, with no shared-owner converge.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn a_worker_only_repair_still_restores_quiesced_taps_and_reopens() {
        for seed in supervisor_seeds() {
            for loss in [
                Loss::ProgramDeleted,
                Loss::PolicyRouteDeleted,
                Loss::MarkGuardDeleted,
                Loss::StaleMember,
            ] {
                let mut schedule = Schedule::new(seed, loss_code(loss) + 200);
                let blocked = schedule.attempts(2);
                let mut rig = Rig::build(
                    seed,
                    format!("{loss:?}/worker-only-{blocked}"),
                    RigSetup::default(),
                )
                .await;
                rig.run_until_first_audit().await;
                let detection = rig.recover(loss, ONE_MS, blocked).await;
                let verdict = rig.verdict();
                let mut expected = vec![healthy_audit(), quiesced_none()];
                for _ in 0..=blocked {
                    expected.push(healthy_audit());
                }
                expected.push(TestOwnerCall::Restore(TestCallOutcome::Ok));
                assert_eq!(rig.calls_since(detection.owner_calls), expected, "{verdict}");
                let restore_at = detection.owner_calls + expected.len() - 1;
                let last_dns_audit =
                    rig.dns.record.lock().audits.last().map(|stamp| stamp.owner_calls);
                assert_eq!(
                    last_dns_audit,
                    Some(restore_at),
                    "{verdict}: the attempt's DNS audit completes the full audit before the restore"
                );
                let repair_stamp =
                    rig.intercept.writes_since(detection.intercept_calls).last().map_or_else(
                        || panic!("{verdict}: the worker repaired"),
                        |entry| entry.owner_calls,
                    );
                assert!(repair_stamp < restore_at, "{verdict}: repair precedes restore");
                rig.assert_single_reopen().await;
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With a live shared allocation holding members, losing the policy route
    /// or the mark guard under an intact program is repaired by one
    /// `converge_shared(Some(recorded), F, C)` that writes no program, the
    /// members stay exactly the live set, and the prior node guard is
    /// relinquished, never dropped (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the runtime repair contract)). A deleted table is recreated
    /// and its members reconverged to the live set.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn a_policy_route_loss_is_repaired_with_live_members_and_the_guard_is_relinquished() {
        for seed in supervisor_seeds() {
            for loss in [Loss::PolicyRouteDeleted, Loss::MarkGuardDeleted, Loss::ProgramDeleted] {
                let mut rig = Rig::build(
                    seed,
                    format!("{loss:?}/live-members"),
                    RigSetup { live_allocation: true, ..RigSetup::default() },
                )
                .await;
                rig.run_until_first_audit().await;
                let live_members = rig.intercept.members();
                assert!(
                    !live_members.managed_guest_ips.is_empty()
                        && !live_members.inbound_destinations.is_empty(),
                    "{}: the live allocation holds members",
                    rig.verdict()
                );
                let program = rig.intercept.program();
                let detection = rig.recover(loss, ONE_MS, 0).await;
                let verdict = rig.verdict();
                let converges: Vec<_> = rig
                    .intercept
                    .log_since(detection.intercept_calls)
                    .into_iter()
                    .filter(|entry| matches!(entry.call, InterceptCall::ConvergeShared { .. }))
                    .map(|entry| entry.call)
                    .collect();
                let leg_f = rig.leg_address(InterceptLeg::F);
                let leg_c = rig.leg_address(InterceptLeg::C);
                let expected_converge = InterceptCall::ConvergeShared {
                    prior: loss != Loss::ProgramDeleted,
                    leg_f,
                    leg_c,
                    wrote: loss == Loss::ProgramDeleted,
                    refused: false,
                };
                assert_eq!(
                    converges,
                    [expected_converge],
                    "{verdict}: one repair at the recorded targets"
                );
                assert_eq!(
                    rig.intercept.program(),
                    program,
                    "{verdict}: the same program identity"
                );
                assert_eq!(
                    rig.intercept.members(),
                    live_members,
                    "{verdict}: exactly the live members"
                );
                assert_eq!(
                    rig.intercept.guard_drops(),
                    0,
                    "{verdict}: the prior guard is relinquished, not dropped"
                );
                rig.assert_single_reopen().await;
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A shared-owner part and the program fail together and heal at seeded,
    /// different attempts: each attempt converges the shared owner before the
    /// worker, the recovery snapshot names the first failing component in D8
    /// order, no TAP is raised while either is failing, and the one restore
    /// follows the first fully clean audit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn a_double_failure_raises_no_tap_before_every_owner_is_repaired() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut schedule = Schedule::new(seed, 300);
            let bridge_heals_after = schedule.attempts(3);
            let program_heals_after = (bridge_heals_after + 1 + schedule.attempts(2)) % 4;
            let mut rig = Rig::build(
                seed,
                format!("Bridge+ProgramDeleted/heal-{bridge_heals_after}-{program_heals_after}"),
                RigSetup::default(),
            )
            .await;
            rig.run_until_first_audit().await;
            let before = rig.mark();
            rig.block_repair(Loss::ProgramDeleted, true);
            rig.inject(bridge);
            rig.inject(Loss::ProgramDeleted);
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            rig.assert_detected(SharedGuestNetworkComponent::Bridge, "audit_mismatch", before);
            let verdict = rig.verdict();
            assert_eq!(rig.count_since(before.owner_calls, is_quiesce), 1, "{verdict}");

            let last = bridge_heals_after.max(program_heals_after);
            for attempt in 0..=last {
                if attempt == bridge_heals_after {
                    rig.block_repair(bridge, false);
                }
                if attempt == program_heals_after {
                    rig.block_repair(Loss::ProgramDeleted, false);
                }
                let at = rig.mark();
                let bridge_failing = attempt <= bridge_heals_after;
                // The program fails every audit before the attempt whose
                // repair heals it, so the worker is due to repair in exactly
                // the attempts up to `program_heals_after`.
                let worker_due = attempt <= program_heals_after;
                rig.attempt().await;
                let calls = rig.calls_since(at.owner_calls);
                let worker_writes = rig.intercept.writes_since(at.intercept_calls);
                if bridge_failing {
                    assert!(
                        calls.first().is_some_and(is_owner_converge),
                        "{verdict}: attempt {attempt} converges the shared owner first: {calls:?}"
                    );
                    if worker_due {
                        assert!(
                            !worker_writes.is_empty(),
                            "{verdict}: attempt {attempt} repairs the worker too: {worker_writes:?}"
                        );
                        assert!(
                            worker_writes
                                .iter()
                                .all(|entry| entry.owner_calls == at.owner_calls + 1),
                            "{verdict}: the worker converges after the shared owner and before the audit: {worker_writes:?}"
                        );
                    } else {
                        assert!(
                            worker_writes.is_empty(),
                            "{verdict}: attempt {attempt}: a worker that passed the latest audit is not repaired: {worker_writes:?}"
                        );
                    }
                }
                if attempt < last {
                    assert_eq!(
                        rig.count_since(before.owner_calls, is_restore),
                        0,
                        "{verdict}: no TAP before every owner is repaired"
                    );
                    let expected_first = if attempt < bridge_heals_after {
                        SharedGuestNetworkComponent::Bridge
                    } else {
                        SharedGuestNetworkComponent::IpRules
                    };
                    assert_eq!(
                        rig.exec.recovery_progress().map(|progress| progress.component),
                        Some(expected_first),
                        "{verdict}: attempt {attempt} reports the first remaining component"
                    );
                }
            }
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: reopened after the last repair"
            );
            let since = rig.calls_since(before.owner_calls);
            assert_eq!(
                since.iter().filter(|call| is_restore(call)).count(),
                1,
                "{verdict}: exactly one restore"
            );
            assert_eq!(
                since.last(),
                Some(&TestOwnerCall::Restore(TestCallOutcome::Ok)),
                "{verdict}"
            );
            assert_eq!(
                since[since.len() - 2],
                healthy_audit(),
                "{verdict}: the restore follows a clean audit"
            );
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E11 re-quiescence after a part-way restore (user-approved 2026-09-30).
    /// Three allocations are `Active` on the owner. A seeded kernel-path loss
    /// quiesces once; its repair and a clean full audit are followed by a
    /// restore that raises one or two TAPs (seeded) and fails, which leaves
    /// `Bridge` first remaining and the latch set, and TAPs no longer count as
    /// quiesced. A seeded kernel-path component then fails: the next attempt's
    /// audit reveals it and the supervisor quiesces again before any
    /// successful restore, which sets down every TAP the restore raised. Once
    /// every fault clears, the next attempt repairs, audits clean, restores,
    /// and EXEC reopens exactly once. The latch is observed set at each step
    /// between the first quiescence and the successful restore, and the L9
    /// invariant holds at every observation point.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn a_kernel_path_failure_after_a_part_way_restore_quiesces_again_before_any_restore() {
        for seed in supervisor_seeds() {
            let mut schedule = Schedule::new(seed, 350);
            let kernel_path: Vec<Loss> =
                every_loss().into_iter().filter(|loss| loss.kernel_path()).collect();
            let bound = u64::try_from(kernel_path.len())
                .unwrap_or_else(|_| unreachable!("a Vec length fits u64"));
            let pick = |schedule: &mut Schedule| {
                kernel_path[usize::try_from(schedule.below(bound))
                    .unwrap_or_else(|_| unreachable!("an index below a Vec length fits usize"))]
            };
            let first = pick(&mut schedule);
            let second = pick(&mut schedule);
            let raised = if schedule.below(2) == 0 { 1 } else { 2 };
            let offset = schedule.offset_within(super::SHARED_NETWORK_AUDIT_PERIOD);
            let mut rig = Rig::build(
                seed,
                format!("requiesce/{first:?}-then-{second:?}/raised-{raised}/offset-{offset:?}"),
                RigSetup::default(),
            )
            .await;
            rig.run_until_first_audit().await;
            let verdict = rig.verdict();
            let vms = [VM_A, VM_B, VM_C];
            for vm in vms {
                let plan = rig
                    .pool
                    .assign(alloc_id(vm))
                    .unwrap_or_else(|error| panic!("{verdict}: lease {vm}: {error}"));
                let activation =
                    guest_network::GuestNetworkProvisioner::activate(rig.owner.as_ref(), &plan)
                        .await
                        .unwrap_or_else(|error| panic!("{verdict}: activate {vm}: {error}"));
                assert_eq!(activation, guest_network::TapActivation::Raised, "{verdict}: {vm}");
            }
            let every: BTreeSet<AllocationId> = vms.into_iter().map(alloc_id).collect();

            // A kernel-path loss quiesces once.
            let before = rig.mark();
            rig.block_repair(first, true);
            let detection = rig.detect(first, offset).await;
            rig.assert_detected(first.component(), first.cause(), before);
            assert_eq!(
                rig.count_since(detection.owner_calls, is_quiesce),
                1,
                "{verdict}: the detection quiesces once"
            );
            assert!(rig.owner.latched(), "{verdict}: the first quiescence sets the latch");
            assert!(rig.owner.active().is_empty(), "{verdict}: every Active TAP went down");

            // Repair, a clean full audit, then a restore that fails part-way.
            rig.owner.script_restore_failure_after(raised);
            rig.block_repair(first, false);
            let partial = rig.mark();
            rig.attempt().await;
            let since = rig.calls_since(partial.owner_calls);
            let restore_at = since
                .iter()
                .position(is_restore)
                .unwrap_or_else(|| panic!("{verdict}: the clean attempt restores: {since:?}"));
            assert_eq!(
                since[restore_at],
                TestOwnerCall::Restore(TestCallOutcome::Failed),
                "{verdict}: the restore fails part-way"
            );
            assert_eq!(
                since[restore_at - 1],
                healthy_audit(),
                "{verdict}: the restore follows a clean audit"
            );
            assert_eq!(
                rig.exec.recovery_progress().map(|progress| progress.component),
                Some(SharedGuestNetworkComponent::Bridge),
                "{verdict}: a restore failure leaves Bridge first remaining"
            );
            assert!(rig.owner.latched(), "{verdict}: a failed restore keeps the latch");
            assert_eq!(
                rig.owner.active().len(),
                raised,
                "{verdict}: the part-way restore left {raised} TAP(s) raised"
            );

            // A kernel-path component fails before any successful restore.
            rig.inject(second);
            let revealing = rig.mark();
            rig.attempt().await;
            let since = rig.calls_since(revealing.owner_calls);
            let quiesce_at = since.iter().position(is_quiesce).unwrap_or_else(|| {
                panic!(
                    "{verdict}: the attempt whose audit reveals {second:?} quiesces again: \
                     {since:?}"
                )
            });
            assert!(
                since[..quiesce_at].iter().any(is_audit),
                "{verdict}: the re-quiescence follows the audit that reveals the failure: {since:?}"
            );
            assert_eq!(
                since.iter().filter(|call| is_quiesce(call)).count(),
                1,
                "{verdict}: one quiescence in that attempt: {since:?}"
            );
            assert!(
                !since.iter().any(is_restore),
                "{verdict}: no restore while {second:?} fails: {since:?}"
            );
            assert!(rig.owner.latched(), "{verdict}: the re-quiescence keeps the latch set");
            assert!(
                rig.owner.active().is_empty(),
                "{verdict}: the re-quiescence set down every TAP the restore raised"
            );
            assert_eq!(
                rig.exec.recovery_progress().map(|progress| progress.component),
                Some(second.component()),
                "{verdict}: the revealing audit names the first remaining component"
            );
            assert_eq!(
                rig.admission(),
                Admission::Closed,
                "{verdict}: EXEC stays closed while TAPs are re-quiesced"
            );

            // Every fault clears: repair, a clean audit, one restore, reopen.
            rig.block_repair(second, false);
            rig.owner.script_restore_failure(false);
            let healing = rig.mark();
            rig.attempt().await;
            assert_eq!(rig.exec.recovery_progress(), None, "{verdict}: the clean attempt reopens");
            assert_eq!(rig.admission(), Admission::Open, "{verdict}");
            assert!(!rig.owner.latched(), "{verdict}: the successful restore cleared the latch");
            let since = rig.calls_since(healing.owner_calls);
            assert_eq!(
                since.last(),
                Some(&TestOwnerCall::Restore(TestCallOutcome::Ok)),
                "{verdict}: the attempt ends with one successful restore: {since:?}"
            );
            assert_eq!(
                since.iter().filter(|call| is_restore(call)).count(),
                1,
                "{verdict}: {since:?}"
            );
            assert_eq!(
                since[since.len() - 2],
                healthy_audit(),
                "{verdict}: the restore follows a clean audit"
            );
            let episode = rig.calls_since(detection.owner_calls);
            let restores: Vec<_> = episode.iter().filter(|call| is_restore(call)).collect();
            assert_eq!(
                restores,
                [
                    &TestOwnerCall::Restore(TestCallOutcome::Failed),
                    &TestOwnerCall::Restore(TestCallOutcome::Ok)
                ],
                "{verdict}: one part-way restore, then one successful restore"
            );
            let quiesces: Vec<_> =
                episode.iter().enumerate().filter(|(_, call)| is_quiesce(call)).collect();
            let failed_at = episode
                .iter()
                .position(|call| *call == TestOwnerCall::Restore(TestCallOutcome::Failed))
                .unwrap_or_else(|| panic!("{verdict}: the part-way restore is journaled"));
            let succeeded_at = episode
                .iter()
                .position(|call| *call == TestOwnerCall::Restore(TestCallOutcome::Ok))
                .unwrap_or_else(|| panic!("{verdict}: the successful restore is journaled"));
            assert_eq!(quiesces.len(), 2, "{verdict}: exactly two quiescences: {episode:?}");
            assert!(
                quiesces[0].0 < failed_at
                    && failed_at < quiesces[1].0
                    && quiesces[1].0 < succeeded_at,
                "{verdict}: quiesce, part-way restore, quiesce again, then the successful restore: \
                 {episode:?}"
            );
            assert_eq!(rig.owner.active(), every, "{verdict}: the restore raised every TAP again");
            rig.assert_single_reopen().await;
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH; OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// E17 seeded: a DNS serve return, serve panic, or audit failure closes
    /// EXEC before any replacement is built, never quiesces, and recovers
    /// through a freshly built, probed responder; a responder whose serve task
    /// is still running (the audit failure) is stopped before its replacement
    /// serves, while one whose task already exited has no stop pinned (D8's
    /// `replace` from `Exited`); a replacement whose probe fails keeps EXEC
    /// closed and the next attempt builds another.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn dns_task_loss_closes_new_commands_and_recovers_through_a_fresh_responder() {
        for seed in supervisor_seeds() {
            for loss in [Loss::DnsServeReturned, Loss::DnsServePanicked, Loss::DnsAuditFailed] {
                let mut schedule = Schedule::new(seed, loss_code(loss) + 400);
                let refused_probes = schedule.attempts(3);
                let mut rig = Rig::build(
                    seed,
                    format!("{loss:?}/refused-probes-{refused_probes}"),
                    RigSetup::default(),
                )
                .await;
                rig.run_until_first_audit().await;
                let detection = rig.recover(loss, ONE_MS, refused_probes).await;
                let verdict = rig.verdict();
                let announcement = rig
                    .unhealthy()
                    .into_iter()
                    .find(|event| event.owner_calls >= detection.owner_calls)
                    .unwrap_or_else(|| panic!("{verdict}: the DNS loss is announced"));
                assert_eq!(
                    announcement.dns_built, 1,
                    "{verdict}: EXEC closes before any replacement is built"
                );
                let refused_probes = usize::try_from(refused_probes)
                    .unwrap_or_else(|error| panic!("{verdict}: a small count fits usize: {error}"));
                assert_eq!(
                    rig.dns.built(),
                    2 + refused_probes,
                    "{verdict}: one fresh responder per attempt until one probes clean"
                );
                if loss == Loss::DnsAuditFailed {
                    assert!(
                        rig.dns.stops_of(0) >= 1,
                        "{verdict}: a still-running responder is stopped before it is replaced"
                    );
                }
                let live = rig.dns.built() - 1;
                assert_eq!(rig.dns.stops_of(live), 0, "{verdict}: the fresh responder serves");
                assert!(
                    rig.dns.audits_by(live) >= 1,
                    "{verdict}: the reopening audit read the fresh responder"
                );
                assert_eq!(rig.count_since(detection.owner_calls, is_quiesce), 0, "{verdict}");
                rig.assert_single_reopen().await;
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-29A — Every shared-network component follows one bounded recovery contract
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// L9 on every schedule: a seeded walk of losses, heals, restore
    /// failures, and clock steps, plus two fixed cells — a restore that fails
    /// twice keeps the latch and EXEC closed with `Bridge` first remaining,
    /// then reopens; a restore that never succeeds reaches the deadline with
    /// the latch set. The rig checks, at every observation point, that a set
    /// latch implies Recovering or a received request and that the gate
    /// admits no claim.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-29A)"]
    async fn the_gate_is_never_open_while_quiescence_is_latched() {
        for seed in supervisor_seeds() {
            // Fixed cell: a restore that fails twice.
            let mut rig = Rig::build(seed, "restore-fails-twice", RigSetup::default()).await;
            rig.run_until_first_audit().await;
            rig.owner.script_restore_failure(true);
            let detection = rig.detect(Loss::ProgramDeleted, ONE_MS).await;
            let verdict = rig.verdict();
            for failed in 1..=2 {
                rig.attempt().await;
                assert!(rig.owner.latched(), "{verdict}: a failed restore keeps the latch");
                assert_eq!(
                    rig.exec.recovery_progress().map(|progress| progress.component),
                    Some(SharedGuestNetworkComponent::Bridge),
                    "{verdict}: a restore failure leaves Bridge first remaining"
                );
                assert_eq!(
                    rig.count_since(detection.owner_calls, |call| {
                        *call == TestOwnerCall::Restore(TestCallOutcome::Failed)
                    }),
                    failed,
                    "{verdict}"
                );
            }
            rig.owner.script_restore_failure(false);
            rig.attempt().await;
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: the restore succeeds and EXEC reopens"
            );
            assert!(!rig.owner.latched(), "{verdict}");
            rig.finish().await;

            // Fixed cell: a restore that never succeeds.
            let mut rig = Rig::build(seed, "restore-never-succeeds", RigSetup::default()).await;
            rig.run_until_first_audit().await;
            rig.owner.script_restore_failure(true);
            rig.detect(Loss::StaleMember, ONE_MS).await;
            for _ in 0..super::SHARED_NETWORK_RECOVERY_ATTEMPTS {
                rig.attempt().await;
            }
            let verdict = rig.verdict();
            let request = rig
                .poll_request()
                .cloned()
                .unwrap_or_else(|| panic!("{verdict}: the deadline fail-stops"));
            let ServeShutdownRequest::SharedGuestNetwork(request) = request;
            assert_eq!(
                request.cause,
                SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
                "{verdict}"
            );
            assert_eq!(request.component, SharedGuestNetworkComponent::Bridge, "{verdict}");
            assert!(rig.owner.latched(), "{verdict}: the latch is still set at fail-stop");
            assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
            rig.finish().await;

            // Seeded walk. It opens with a seeded kernel-path loss detected by
            // the next audit, so every seed's walk reaches the latch at least
            // once and the invariant is exercised while it is set.
            let mut schedule = Schedule::new(seed, 500);
            let mut rig = Rig::build(seed, "latch-walk", RigSetup::default()).await;
            rig.run_until_first_audit().await;
            let verdict = rig.verdict();
            let losses = every_loss();
            let index = |schedule: &mut Schedule, len: usize| {
                let bound = u64::try_from(len)
                    .unwrap_or_else(|error| panic!("{verdict}: a length fits u64: {error}"));
                usize::try_from(schedule.below(bound))
                    .unwrap_or_else(|error| panic!("{verdict}: an index fits usize: {error}"))
            };
            let kernel_path: Vec<Loss> =
                losses.iter().copied().filter(|loss| loss.kernel_path()).collect();
            let opening = kernel_path[index(&mut schedule, kernel_path.len())];
            rig.inject(opening);
            let mut active: Vec<Loss> = vec![opening];
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            let mut latched_steps = usize::from(rig.owner.latched());
            for _ in 0..40 {
                if rig.poll_request().is_some() {
                    break;
                }
                match schedule.below(6) {
                    0 if active.is_empty() => {
                        let loss = losses[index(&mut schedule, losses.len())];
                        if !loss.immediate() || rig.exec.recovery_progress().is_none() {
                            rig.inject(loss);
                            active.push(loss);
                            if loss.immediate() {
                                rig.await_wake().await;
                            }
                        }
                    }
                    1 => {
                        for loss in std::mem::take(&mut active) {
                            rig.block_repair(loss, false);
                        }
                    }
                    2 => rig.owner.script_restore_failure(schedule.below(2) == 0),
                    3 => rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await,
                    _ => {
                        if rig.exec.recovery_progress().is_some() {
                            rig.attempt().await;
                        } else {
                            rig.advance_quiet(super::SHARED_NETWORK_RETRY_PERIOD).await;
                        }
                    }
                }
                rig.check_invariants();
                latched_steps += usize::from(rig.owner.latched());
            }
            assert!(
                latched_steps >= 1,
                "{verdict}: the walk observed the latch set at least once (opening loss \
                 {opening:?}), so the invariant was exercised while it held"
            );
            rig.finish().await;
        }
    }

    // -----------------------------------------------------------------------
    // S-ND295-30A
    // -----------------------------------------------------------------------

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (a) Quiescence reports A unconfirmed during a `Bridge` recovery: A's
    /// scope `cgroup.kill` holds `1\n` before the supervisor calls any owner
    /// again, B and the workloads slice are untouched, one
    /// `shared_owner_vm_killed { alloc: A, cause: "quiescence_unconfirmed" }`
    /// is emitted, recovery reopens for the rest, and no later audit period
    /// kills or announces A again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn an_unconfirmed_tap_stops_only_its_vm_and_recovery_reopens() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut schedule = Schedule::new(seed, 600);
            let offset = schedule.offset_within(super::SHARED_NETWORK_AUDIT_PERIOD);
            let mut rig = Rig::build(
                seed,
                format!("unconfirmed-A/offset-{offset:?}"),
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            rig.owner
                .script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([alloc_id(VM_A)])));
            let detection = rig.detect(bridge, offset).await;
            let verdict = rig.verdict();
            let journal = rig.owner.journal();
            let quiesce_at = journal[detection.owner_calls..]
                .iter()
                .position(|entry| is_quiesce(&entry.call))
                .map_or_else(
                    || panic!("{verdict}: the detection quiesces"),
                    |at| at + detection.owner_calls,
                );
            assert_eq!(
                journal[quiesce_at].call,
                TestOwnerCall::Quiesce(TestQuiesceOutcome::Unconfirmed(BTreeSet::from([
                    alloc_id(VM_A)
                ]))),
                "{verdict}"
            );
            let killed_events = rig.vm_killed();
            assert_eq!(killed_events.len(), 1, "{verdict}: {killed_events:?}");
            assert!(names_alloc(&killed_events[0], VM_A), "{verdict}: {killed_events:?}");
            assert_eq!(
                killed_events[0].field("cause"),
                Some("quiescence_unconfirmed"),
                "{verdict}"
            );
            assert!(
                killed(&killed_events[0].cgroups, &scope_kill(&rig.root, VM_A)),
                "{verdict}: kill, then announce"
            );
            assert!(
                rig.exec.recovery_progress().is_some(),
                "{verdict}: the kill does not end recovery"
            );

            rig.block_repair(bridge, false);
            rig.attempt().await;
            let journal = rig.owner.journal();
            let next = journal
                .get(quiesce_at + 1)
                .unwrap_or_else(|| panic!("{verdict}: the attempt calls the owner again"));
            let snapshot = next
                .cgroups
                .as_ref()
                .unwrap_or_else(|| panic!("{verdict}: the owner journals cgroup snapshots"));
            assert!(
                killed(snapshot, &scope_kill(&rig.root, VM_A)),
                "{verdict}: A is killed before the next owner call"
            );
            assert!(
                untouched(snapshot, &scope_kill(&rig.root, VM_B)),
                "{verdict}: B keeps running"
            );
            assert!(untouched(snapshot, &slice_kill(&rig.root)), "{verdict}: no whole-slice kill");
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: recovery reopens for the rest"
            );
            assert!(rig.poll_request().is_none(), "{verdict}");
            rig.assert_single_reopen().await;
            let killed_events = rig.vm_killed();
            assert_eq!(
                killed_events.len(),
                1,
                "{verdict}: no later audit period kills or announces A again: {killed_events:?}"
            );
            let final_snapshot = rig.fs.snapshot();
            assert!(untouched(&final_snapshot, &scope_kill(&rig.root, VM_B)), "{verdict}");
            assert!(untouched(&final_snapshot, &slice_kill(&rig.root)), "{verdict}");
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A netlink failure common to every TAP makes every `Active` allocation
    /// unconfirmed (DR-08 (b)-A): quiescence during a `Bridge` recovery names
    /// all of them (two or three, seeded). Each scope's `cgroup.kill` holds
    /// `1\n` in the snapshot of the first owner call after the report; the
    /// kills run in `AllocationId` order (each announcement's snapshot holds
    /// the kills of its own and every earlier allocation, and none of a later
    /// one); the workloads slice is never written, no fail-stop is requested,
    /// and recovery continues and reopens exactly once. The whole-call cells
    /// stay with `an_undetermined_quiescence_stops_every_workload_vm_then_fails_the_node`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn a_quiescence_naming_every_active_vm_stops_each_in_order_and_recovery_reopens() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut schedule = Schedule::new(seed, 650);
            let count = if schedule.below(2) == 0 { 2 } else { 3 };
            let offset = schedule.offset_within(super::SHARED_NETWORK_AUDIT_PERIOD);
            let vms: Vec<&'static str> = [VM_A, VM_B, VM_C][..count].to_vec();
            let mut rig = Rig::build(
                seed,
                format!("all-unconfirmed-{count}/offset-{offset:?}"),
                RigSetup { scoped: vms.clone(), slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            let verdict = rig.verdict();
            for vm in &vms {
                let plan = rig
                    .pool
                    .assign(alloc_id(vm))
                    .unwrap_or_else(|error| panic!("{verdict}: lease {vm}: {error}"));
                let raised =
                    guest_network::GuestNetworkProvisioner::activate(rig.owner.as_ref(), &plan)
                        .await
                        .unwrap_or_else(|error| panic!("{verdict}: activate {vm}: {error}"));
                assert_eq!(raised, guest_network::TapActivation::Raised, "{verdict}: {vm}");
            }
            let every: BTreeSet<AllocationId> = vms.iter().map(|vm| alloc_id(vm)).collect();
            rig.owner.script_quiesce(TestQuiesceScript::Unconfirmed(every.clone()));
            let detection = rig.detect(bridge, offset).await;
            let journal = rig.owner.journal();
            let quiesce_at = journal[detection.owner_calls..]
                .iter()
                .position(|entry| is_quiesce(&entry.call))
                .map_or_else(
                    || panic!("{verdict}: the detection quiesces"),
                    |at| at + detection.owner_calls,
                );
            assert_eq!(
                journal[quiesce_at].call,
                TestOwnerCall::Quiesce(TestQuiesceOutcome::Unconfirmed(every.clone())),
                "{verdict}: the report names every Active allocation"
            );

            let killed_events = rig.vm_killed();
            let announced: Vec<_> =
                killed_events.iter().map(|event| named_alloc(event, &vms)).collect();
            let expected: Vec<_> = vms.iter().map(|vm| Some(*vm)).collect();
            assert_eq!(
                announced, expected,
                "{verdict}: one announcement per VM, in AllocationId order: {killed_events:?}"
            );
            for (position, event) in killed_events.iter().enumerate() {
                assert_eq!(event.field("cause"), Some("quiescence_unconfirmed"), "{verdict}");
                for (other, vm) in vms.iter().enumerate() {
                    assert_eq!(
                        killed(&event.cgroups, &scope_kill(&rig.root, vm)),
                        other <= position,
                        "{verdict}: at the announcement of {}, {vm}'s kill is written exactly \
                         when it is not later in AllocationId order",
                        vms[position]
                    );
                }
                assert!(
                    untouched(&event.cgroups, &slice_kill(&rig.root)),
                    "{verdict}: no whole-slice kill"
                );
            }
            assert!(
                rig.exec.recovery_progress().is_some(),
                "{verdict}: the kills do not end recovery"
            );

            rig.block_repair(bridge, false);
            rig.attempt().await;
            let journal = rig.owner.journal();
            let next = journal
                .get(quiesce_at + 1)
                .unwrap_or_else(|| panic!("{verdict}: the attempt calls the owner again"));
            let snapshot = next
                .cgroups
                .as_ref()
                .unwrap_or_else(|| panic!("{verdict}: the owner journals cgroup snapshots"));
            for vm in &vms {
                assert!(
                    killed(snapshot, &scope_kill(&rig.root, vm)),
                    "{verdict}: {vm} is killed before the next owner call"
                );
            }
            assert!(untouched(snapshot, &slice_kill(&rig.root)), "{verdict}: no whole-slice kill");
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: recovery continues and reopens"
            );
            assert!(rig.poll_request().is_none(), "{verdict}: no fail-stop is requested");
            rig.assert_single_reopen().await;
            assert_eq!(
                rig.vm_killed().len(),
                vms.len(),
                "{verdict}: each VM is killed once and never again"
            );
            assert!(untouched(&rig.fs.snapshot(), &slice_kill(&rig.root)), "{verdict}");
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (b) A quiescence that fails, or that is still pending at
    /// `SHARED_NETWORK_QUIESCE_CALL_BOUND`, cannot name the affected TAPs:
    /// the workloads slice's `cgroup.kill` holds `1\n` when the one
    /// `TapQuiescenceUndetermined` request is received, no single scope is
    /// written, and the fail-stop event records the slice kill (S-ND295-32's
    /// quiescence bound cell).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn an_undetermined_quiescence_stops_every_workload_vm_then_fails_the_node() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            for (script, elapsed) in [
                (TestQuiesceScript::Fail, Duration::ZERO),
                (TestQuiesceScript::Hang, super::SHARED_NETWORK_QUIESCE_CALL_BOUND),
            ] {
                let mut rig = Rig::build(
                    seed,
                    format!("undetermined-{script:?}"),
                    RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
                )
                .await;
                rig.run_until_first_audit().await;
                rig.owner.script_quiesce(script.clone());
                rig.detect(bridge, ONE_MS).await;
                let verdict = rig.verdict();
                if script == TestQuiesceScript::Hang {
                    assert!(
                        rig.poll_request().is_none(),
                        "{verdict}: the quiescence call is still pending"
                    );
                    rig.advance_quiet(earlier(super::SHARED_NETWORK_QUIESCE_CALL_BOUND, ONE_MS))
                        .await;
                    assert!(
                        rig.poll_request().is_none(),
                        "{verdict}: not before the quiescence bound"
                    );
                    rig.advance(ONE_MS).await;
                }
                let request = rig
                    .poll_request()
                    .cloned()
                    .unwrap_or_else(|| panic!("{verdict}: an undetermined quiescence fail-stops"));
                assert_eq!(
                    request,
                    ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
                        component: SharedGuestNetworkComponent::Bridge,
                        cause: SharedGuestNetworkFailStopCause::TapQuiescenceUndetermined,
                        attempts: 0,
                        elapsed,
                    }),
                    "{verdict}"
                );
                let at_receipt = rig
                    .request_cgroups
                    .clone()
                    .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
                assert!(
                    killed(&at_receipt, &slice_kill(&rig.root)),
                    "{verdict}: every workload VM is killed"
                );
                assert!(untouched(&at_receipt, &scope_kill(&rig.root, VM_A)), "{verdict}");
                assert!(untouched(&at_receipt, &scope_kill(&rig.root, VM_B)), "{verdict}");
                assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
                assert!(rig.vm_killed().is_empty(), "{verdict}: no per-VM kill was attempted");
                let fail_stop = rig.events("guest_network.shared_owner_fail_stop");
                assert_eq!(fail_stop.len(), 1, "{verdict}");
                assert!(
                    fail_stop[0].fields.contains_key("vm_kill"),
                    "{verdict}: the slice kill outcome is recorded"
                );
                let calls = rig.calls();
                assert!(!calls.iter().any(is_owner_converge), "{verdict}: no repair attempt runs");
                assert!(!calls.iter().any(is_restore), "{verdict}");
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (c) A per-VM kill write for a known allocation fails other than with an
    /// absent scope — during a recovery's quiescence, and for damage found
    /// while Open: the workloads slice is killed and one `VmKillFailed`
    /// request is sent, carrying the recovery snapshot, or from Open the
    /// no-recovery values (`Supervisor`, 0, 0).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn a_failed_per_vm_stop_stops_every_workload_vm_then_fails_the_node() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            for during_recovery in [true, false] {
                let mut rig = Rig::build(
                    seed,
                    format!("kill-write-fails/during-recovery-{during_recovery}"),
                    RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
                )
                .await;
                rig.run_until_first_audit().await;
                rig.fs.inject_error(
                    SimOp::Write,
                    scope_kill(&rig.root, VM_A),
                    std::io::ErrorKind::Other,
                );
                let expected = if during_recovery {
                    rig.owner.script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([
                        alloc_id(VM_A),
                    ])));
                    rig.detect(bridge, ONE_MS).await;
                    SharedGuestNetworkFailStop {
                        component: SharedGuestNetworkComponent::Bridge,
                        cause: SharedGuestNetworkFailStopCause::VmKillFailed,
                        attempts: 0,
                        elapsed: Duration::ZERO,
                    }
                } else {
                    rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_A)]));
                    rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
                    SharedGuestNetworkFailStop {
                        component: SharedGuestNetworkComponent::Supervisor,
                        cause: SharedGuestNetworkFailStopCause::VmKillFailed,
                        attempts: 0,
                        elapsed: Duration::ZERO,
                    }
                };
                let verdict = rig.verdict();
                assert_eq!(
                    rig.poll_request().cloned(),
                    Some(ServeShutdownRequest::SharedGuestNetwork(expected)),
                    "{verdict}"
                );
                let at_receipt = rig
                    .request_cgroups
                    .clone()
                    .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
                assert!(
                    killed(&at_receipt, &slice_kill(&rig.root)),
                    "{verdict}: every workload VM is killed"
                );
                assert!(
                    untouched(&at_receipt, &scope_kill(&rig.root, VM_A)),
                    "{verdict}: A's write failed"
                );
                assert!(untouched(&at_receipt, &scope_kill(&rig.root, VM_B)), "{verdict}");
                assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (d) A reported allocation whose scope directory is already gone (the
    /// kill write under a missing parent is `NotFound`) counts as stopped:
    /// nothing is written at its scope or the slice, it is announced once and
    /// never again over the next audit periods, and — during a recovery or
    /// while Open — the node continues.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn an_already_removed_scope_counts_as_stopped() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            for during_recovery in [true, false] {
                let mut rig = Rig::build(
                    seed,
                    format!("scope-gone/during-recovery-{during_recovery}"),
                    RigSetup { scoped: vec![VM_B], slice: true, ..RigSetup::default() },
                )
                .await;
                rig.run_until_first_audit().await;
                let cause = if during_recovery {
                    rig.owner.script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([
                        alloc_id(VM_A),
                    ])));
                    rig.detect(bridge, ONE_MS).await;
                    rig.block_repair(bridge, false);
                    rig.attempt().await;
                    "quiescence_unconfirmed"
                } else {
                    rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_A)]));
                    rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
                    "attachment_damaged"
                };
                let verdict = rig.verdict();
                let killed_events = rig.vm_killed();
                assert_eq!(killed_events.len(), 1, "{verdict}: {killed_events:?}");
                assert!(names_alloc(&killed_events[0], VM_A), "{verdict}: {killed_events:?}");
                assert_eq!(killed_events[0].field("cause"), Some(cause), "{verdict}");
                let snapshot = rig.fs.snapshot();
                assert!(untouched(&snapshot, &scope_kill(&rig.root, VM_A)), "{verdict}");
                assert!(
                    untouched(&snapshot, &slice_kill(&rig.root)),
                    "{verdict}: no whole-slice kill"
                );
                assert!(untouched(&snapshot, &scope_kill(&rig.root, VM_B)), "{verdict}");
                assert!(
                    rig.poll_request().is_none(),
                    "{verdict}: an absent scope is a confirmed kill"
                );
                assert_eq!(rig.exec.recovery_progress(), None, "{verdict}");
                assert_eq!(rig.admission(), Admission::Open, "{verdict}");
                rig.assert_single_reopen().await;
                assert_eq!(
                    rig.vm_killed().len(),
                    1,
                    "{verdict}: A is never announced again over the next audit periods"
                );
                assert!(untouched(&rig.fs.snapshot(), &slice_kill(&rig.root)), "{verdict}");
                rig.finish().await;
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (e)-(g) Damage the owner attributes to allocations while every
    /// node-level part is healthy stops exactly those VMs, in `AllocationId`
    /// order, with no owner call between the audit and the kills, while EXEC
    /// stays Open; damage found by an attempt's audit during a recovery is
    /// killed before that attempt's restore. The table spans the damaged-set
    /// cardinalities (none, one, two); the kill decision does not depend on
    /// which part is damaged — per-part attribution is the owner's
    /// (S-ND295-50).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn every_damaged_per_vm_part_stops_only_that_vm_while_admission_stays_open() {
        for seed in supervisor_seeds() {
            for damaged in [vec![], vec![VM_B], vec![VM_A, VM_B]] {
                let mut rig = Rig::build(
                    seed,
                    format!("damage-while-open/{damaged:?}"),
                    RigSetup { scoped: vec![VM_A, VM_B, VM_C], slice: true, ..RigSetup::default() },
                )
                .await;
                rig.run_until_first_audit().await;
                rig.owner
                    .script_audit_damage(damaged.iter().map(|alloc| alloc_id(alloc)).collect());
                let before = rig.mark();
                rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
                let verdict = rig.verdict();
                assert_eq!(
                    rig.exec.recovery_progress(),
                    None,
                    "{verdict}: damage is not a node failure"
                );
                assert_eq!(rig.admission(), Admission::Open, "{verdict}: EXEC stays Open");
                assert!(rig.unhealthy().is_empty(), "{verdict}");
                assert_eq!(rig.count_since(before.owner_calls, is_quiesce), 0, "{verdict}");
                let killed_events = rig.vm_killed();
                let announced: Vec<_> = killed_events
                    .iter()
                    .map(|event| named_alloc(event, &[VM_A, VM_B, VM_C]))
                    .collect();
                let expected: Vec<_> = damaged.iter().map(|alloc| Some(*alloc)).collect();
                assert_eq!(
                    announced, expected,
                    "{verdict}: exactly the damaged VMs, in AllocationId order: {killed_events:?}"
                );
                for event in &killed_events {
                    assert_eq!(event.field("cause"), Some("attachment_damaged"), "{verdict}");
                    assert_eq!(
                        event.owner_calls,
                        before.owner_calls + 1,
                        "{verdict}: no owner call between the audit and the kills"
                    );
                }
                rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
                let journal = rig.owner.journal();
                let next = journal
                    .get(before.owner_calls + 1)
                    .unwrap_or_else(|| panic!("{verdict}: the next audit calls the owner"));
                let snapshot = next
                    .cgroups
                    .as_ref()
                    .unwrap_or_else(|| panic!("{verdict}: the owner journals cgroup snapshots"));
                for alloc in [VM_A, VM_B, VM_C] {
                    assert_eq!(
                        killed(snapshot, &scope_kill(&rig.root, alloc)),
                        damaged.contains(&alloc),
                        "{verdict}: {alloc}"
                    );
                }
                assert!(untouched(snapshot, &slice_kill(&rig.root)), "{verdict}");
                rig.finish().await;
            }

            // Damage revealed by an attempt's audit during a recovery.
            let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
            let mut rig = Rig::build(
                seed,
                "damage-during-recovery/B",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_B)]));
            let detection = rig.detect(bridge, ONE_MS).await;
            rig.block_repair(bridge, false);
            rig.attempt().await;
            let verdict = rig.verdict();
            let journal = rig.owner.journal();
            let restore = journal[detection.owner_calls..]
                .iter()
                .find(|entry| is_restore(&entry.call))
                .unwrap_or_else(|| panic!("{verdict}: the attempt restores"));
            let snapshot = restore
                .cgroups
                .as_ref()
                .unwrap_or_else(|| panic!("{verdict}: the owner journals cgroup snapshots"));
            assert!(
                killed(snapshot, &scope_kill(&rig.root, VM_B)),
                "{verdict}: B is killed before the restore"
            );
            assert!(untouched(snapshot, &scope_kill(&rig.root, VM_A)), "{verdict}");
            let killed_events = rig.vm_killed();
            assert_eq!(killed_events.len(), 1, "{verdict}: {killed_events:?}");
            assert!(names_alloc(&killed_events[0], VM_B), "{verdict}: {killed_events:?}");
            assert_eq!(killed_events[0].field("cause"), Some("attachment_damaged"), "{verdict}");
            assert_eq!(rig.exec.recovery_progress(), None, "{verdict}: the attempt reopens");
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A stopped VM is stopped once: over a later kernel-path recovery and
    /// further audits with the same standing reports, A is announced once, B is
    /// stopped when it is later reported, each one's scope `cgroup.kill` is
    /// written and the slice's never, A's and B's activations are refused
    /// while an unreported VM's raises, and recovery keeps reopening for the
    /// rest.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn a_killed_vm_leaves_every_later_audit_and_restore_universe() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "killed-once",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            let verdict = rig.verdict();
            let lease = |rig: &Rig, alloc: &str| {
                rig.pool
                    .assign(alloc_id(alloc))
                    .unwrap_or_else(|error| panic!("{verdict}: lease {alloc}: {error}"))
            };
            let plan_a = lease(&rig, VM_A);
            let plan_b = lease(&rig, VM_B);
            rig.owner
                .script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([alloc_id(VM_A)])));
            rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_A)]));
            rig.detect(bridge, ONE_MS).await;
            rig.block_repair(bridge, false);
            rig.attempt().await;
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            rig.detect(bridge, ONE_MS).await;
            rig.block_repair(bridge, false);
            rig.attempt().await;
            rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_A), alloc_id(VM_B)]));
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            let killed_events = rig.vm_killed();
            let announced: Vec<_> =
                killed_events.iter().map(|event| named_alloc(event, &[VM_A, VM_B])).collect();
            assert_eq!(
                announced,
                [Some(VM_A), Some(VM_B)],
                "{verdict}: each VM is stopped exactly once: {killed_events:?}"
            );
            let snapshot = rig.fs.snapshot();
            for alloc in [VM_A, VM_B] {
                assert!(
                    killed(&snapshot, &scope_kill(&rig.root, alloc)),
                    "{verdict}: {alloc}'s scope kill was written"
                );
            }
            assert!(untouched(&snapshot, &slice_kill(&rig.root)), "{verdict}: no whole-slice kill");
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: the node keeps recovering for the rest"
            );
            let refused =
                guest_network::GuestNetworkProvisioner::activate(rig.owner.as_ref(), &plan_a).await;
            assert!(refused.is_err(), "{verdict}: a stopped VM is never activated");
            let refused_b =
                guest_network::GuestNetworkProvisioner::activate(rig.owner.as_ref(), &plan_b).await;
            assert!(refused_b.is_err(), "{verdict}: B was stopped too");
            let other = lease(&rig, VM_C);
            let raised =
                guest_network::GuestNetworkProvisioner::activate(rig.owner.as_ref(), &other).await;
            assert!(
                matches!(raised, Ok(guest_network::TapActivation::Raised)),
                "{verdict}: an unreported VM raises"
            );
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (c2) A per-VM kill write that stays pending past
    /// `SHARED_NETWORK_VM_KILL_CALL_BOUND` on the injected clock counts as a
    /// failed kill: the workloads slice is killed and one `VmKillFailed` request
    /// is sent, with no owner call between the report and the slice kill (user
    /// decision 1 of 2026-09-30). A write that lands after its missed bound
    /// changes no outcome.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn a_per_vm_kill_write_pending_past_its_bound_fails_the_node() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "kill-write-bound-miss",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            // A's kill write parks past its own bound on the injected clock.
            rig.fs.park_until(
                SimOp::Write,
                scope_kill(&rig.root, VM_A),
                Arc::clone(&rig.clock) as Arc<dyn Clock>,
                super::SHARED_NETWORK_VM_KILL_CALL_BOUND * 2,
            );
            rig.owner
                .script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([alloc_id(VM_A)])));
            let detection = rig.detect(bridge, ONE_MS).await;
            let verdict = rig.verdict();
            // Not before the kill-write bound: no request yet.
            rig.advance_quiet(earlier(super::SHARED_NETWORK_VM_KILL_CALL_BOUND, ONE_MS)).await;
            assert!(rig.poll_request().is_none(), "{verdict}: not before the kill-write bound");
            rig.advance(ONE_MS).await;
            let request = rig
                .poll_request()
                .cloned()
                .unwrap_or_else(|| panic!("{verdict}: the missed kill-write bound fail-stops"));
            let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
            assert_eq!(fail_stop.component, SharedGuestNetworkComponent::Bridge, "{verdict}");
            assert_eq!(
                fail_stop.cause,
                SharedGuestNetworkFailStopCause::VmKillFailed,
                "{verdict}: a kill-write bound miss is VmKillFailed"
            );
            let at_receipt = rig
                .request_cgroups
                .clone()
                .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
            assert!(
                killed(&at_receipt, &slice_kill(&rig.root)),
                "{verdict}: every workload VM is killed"
            );
            assert_eq!(
                rig.count_since(detection.owner_calls, is_owner_converge),
                0,
                "{verdict}: no owner call between the report and the slice kill"
            );
            assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// (i) A report naming at least two allocations whose kill loop is still
    /// running when the recovery deadline passes completes — every reported
    /// scope's `cgroup.kill` written, in `AllocationId` order — before the one
    /// `RecoveryDeadlineExceeded` request, with no owner repair call between the
    /// report and that request (user decision 1 of 2026-09-30).
    /// `RecoveryDeadlineExceeded` never cuts the loop short.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn a_kill_loop_running_at_the_deadline_completes_before_the_one_request() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "loop-past-deadline",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            // A Bridge loss whose repair never succeeds drives recovery to the
            // 5 s deadline; both reported kill writes park (each within its own
            // bound) so a damage report near the deadline straddles it.
            rig.block_repair(bridge, true);
            let half = super::SHARED_NETWORK_VM_KILL_CALL_BOUND / 2;
            for alloc in [VM_A, VM_B] {
                rig.fs.park_until(
                    SimOp::Write,
                    scope_kill(&rig.root, alloc),
                    Arc::clone(&rig.clock) as Arc<dyn Clock>,
                    half,
                );
            }
            rig.detect(bridge, ONE_MS).await;
            let verdict = rig.verdict();
            // Drive failing attempts until one kill-loop budget remains before
            // the 5 s deadline.
            loop {
                let elapsed = rig
                    .exec
                    .recovery_progress()
                    .unwrap_or_else(|| panic!("{verdict}: recovery is running before the deadline"))
                    .elapsed;
                if elapsed + super::SHARED_NETWORK_VM_KILL_CALL_BOUND
                    >= super::SHARED_NETWORK_RECOVERY_DEADLINE
                {
                    break;
                }
                rig.attempt().await;
            }
            // The last attempt's audit before the deadline reports damage for
            // both A and B; their parked kill loop then straddles the deadline.
            let mark = rig.mark();
            rig.owner.script_audit_damage(BTreeSet::from([alloc_id(VM_A), alloc_id(VM_B)]));
            rig.advance(super::SHARED_NETWORK_RETRY_PERIOD).await;
            rig.advance_quiet(super::SHARED_NETWORK_VM_KILL_CALL_BOUND).await;
            let request = rig.poll_request().cloned().unwrap_or_else(|| {
                panic!("{verdict}: the deadline fail-stops once the kill loop completes")
            });
            let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
            assert_eq!(
                fail_stop.cause,
                SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
                "{verdict}: a kill loop running at the deadline does not change the cause"
            );
            let killed_events = rig.vm_killed();
            let order: Vec<_> = killed_events
                .iter()
                .filter_map(|event| named_alloc(event, &[VM_A, VM_B]))
                .collect();
            assert_eq!(
                order,
                [VM_A, VM_B],
                "{verdict}: both killed in AllocationId order: {killed_events:?}"
            );
            let at_receipt = rig
                .request_cgroups
                .clone()
                .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
            for alloc in [VM_A, VM_B] {
                assert!(
                    killed(&at_receipt, &scope_kill(&rig.root, alloc)),
                    "{verdict}: {alloc}'s kill was written before the request"
                );
            }
            assert!(
                untouched(&at_receipt, &slice_kill(&rig.root)),
                "{verdict}: no whole-slice kill — the loop ran to its end, not cut short"
            );
            assert!(
                !rig.calls_since(mark.owner_calls).iter().any(is_owner_converge),
                "{verdict}: no owner repair call between the report and the request"
            );
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// An intentional shutdown (SIGINT/SIGTERM) that arrives while a kill loop
    /// is in progress cancels the supervisor only between loops: every reported
    /// kill is written, in `AllocationId` order, before the supervisor observes
    /// cancellation — no slice kill, and intentional shutdown is not a fail-stop
    /// (user decision 1 of 2026-09-30).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-30A)"]
    async fn an_intentional_shutdown_mid_kill_loop_lets_the_loop_finish_first() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "shutdown-mid-loop",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            // Both reported VMs' kill writes park on the injected clock, so the
            // loop is unambiguously in progress when shutdown is requested.
            let half = super::SHARED_NETWORK_VM_KILL_CALL_BOUND / 2;
            for alloc in [VM_A, VM_B] {
                rig.fs.park_until(
                    SimOp::Write,
                    scope_kill(&rig.root, alloc),
                    Arc::clone(&rig.clock) as Arc<dyn Clock>,
                    half,
                );
            }
            rig.owner.script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([
                alloc_id(VM_A),
                alloc_id(VM_B),
            ])));
            rig.detect(bridge, ONE_MS).await;
            let verdict = rig.verdict();
            // Shutdown arrives mid-loop.
            rig.request_intentional_shutdown();
            rig.advance_quiet(super::SHARED_NETWORK_VM_KILL_CALL_BOUND).await;
            let killed_events = rig.vm_killed();
            let order: Vec<_> = killed_events
                .iter()
                .filter_map(|event| named_alloc(event, &[VM_A, VM_B]))
                .collect();
            assert_eq!(
                order,
                [VM_A, VM_B],
                "{verdict}: the loop runs to its end in AllocationId order: {killed_events:?}"
            );
            let snapshot = rig.fs.snapshot();
            for alloc in [VM_A, VM_B] {
                assert!(
                    killed(&snapshot, &scope_kill(&rig.root, alloc)),
                    "{verdict}: {alloc}'s kill was written before the supervisor observed shutdown"
                );
            }
            assert!(untouched(&snapshot, &slice_kill(&rig.root)), "{verdict}: no whole-slice kill");
            assert!(
                rig.poll_request().is_none(),
                "{verdict}: intentional shutdown is not a fail-stop"
            );
            rig.finish().await;
        }
    }

    // -----------------------------------------------------------------------
    // S-ND295-32
    // -----------------------------------------------------------------------

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A shared-owner audit that never answers fails that owner's first
    /// component, `Bridge`, with cause `audit_timeout`, exactly
    /// `SHARED_NETWORK_AUDIT_CALL_BOUND` after the audit began on the injected
    /// clock; recovery then proceeds as for any `Bridge` loss.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-32)"]
    async fn a_hung_audit_is_a_timeout_failure_of_its_owner() {
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(seed, "hung-shared-audit", RigSetup::default()).await;
            rig.run_until_first_audit().await;
            rig.owner.script_audit_mode(TestAuditMode::Hang);
            let before = rig.mark();
            rig.advance(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            let verdict = rig.verdict();
            assert_eq!(
                rig.calls_since(before.owner_calls),
                [TestOwnerCall::AuditShared(TestAuditOutcome::Hung)],
                "{verdict}: the audit began and is pending"
            );
            assert_eq!(rig.exec.recovery_progress(), None, "{verdict}");
            rig.advance_quiet(earlier(super::SHARED_NETWORK_AUDIT_CALL_BOUND, ONE_MS)).await;
            assert_eq!(rig.exec.recovery_progress(), None, "{verdict}: not before the call bound");
            assert_eq!(rig.admission(), Admission::Open, "{verdict}");
            rig.owner.script_audit_mode(TestAuditMode::Normal);
            rig.advance(ONE_MS).await;
            rig.assert_detected(SharedGuestNetworkComponent::Bridge, "audit_timeout", before);
            assert_eq!(
                rig.count_since(before.owner_calls, is_quiesce),
                1,
                "{verdict}: Bridge is kernel-path"
            );
            rig.attempt().await;
            assert_eq!(
                rig.exec.recovery_progress(),
                None,
                "{verdict}: an answering owner recovers"
            );
            assert_eq!(rig.count_since(before.owner_calls, is_restore), 1, "{verdict}");
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// An attempt whose full audit is still pending at the recovery deadline
    /// is abandoned and not counted: the one `RecoveryDeadlineExceeded`
    /// request carries the attempts completed before it and the full deadline
    /// as elapsed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-32)"]
    async fn a_call_pending_at_the_deadline_is_abandoned_uncounted() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        let completed = super::SHARED_NETWORK_RECOVERY_ATTEMPTS - 3;
        let hung_start = super::SHARED_NETWORK_RETRY_PERIOD * (completed + 1);
        assert!(
            hung_start < super::SHARED_NETWORK_RECOVERY_DEADLINE,
            "the hung attempt starts inside the window"
        );
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(seed, "pending-at-deadline", RigSetup::default()).await;
            rig.run_until_first_audit().await;
            rig.detect(bridge, ONE_MS).await;
            for _ in 0..completed {
                rig.attempt().await;
            }
            let verdict = rig.verdict();
            assert_eq!(
                rig.exec.recovery_progress().map(|progress| progress.attempts),
                Some(completed),
                "{verdict}"
            );
            rig.owner.script_audit_mode(TestAuditMode::Hang);
            rig.attempt().await;
            assert!(
                matches!(
                    rig.calls().last(),
                    Some(TestOwnerCall::AuditShared(TestAuditOutcome::Hung))
                ),
                "{verdict}: the attempt's audit is pending"
            );
            let remaining = earlier(super::SHARED_NETWORK_RECOVERY_DEADLINE, hung_start);
            rig.advance_quiet(earlier(remaining, ONE_MS)).await;
            assert!(rig.poll_request().is_none(), "{verdict}: nothing before the deadline");
            assert_eq!(
                rig.exec.recovery_progress().map(|progress| progress.attempts),
                Some(completed),
                "{verdict}: the pending attempt is not counted"
            );
            rig.advance(ONE_MS).await;
            assert_eq!(
                rig.poll_request().cloned(),
                Some(ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
                    component: SharedGuestNetworkComponent::Bridge,
                    cause: SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
                    attempts: completed,
                    elapsed: super::SHARED_NETWORK_RECOVERY_DEADLINE,
                })),
                "{verdict}"
            );
            rig.finish().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// G-295-5 row 4: a quiescence result arriving after its bound is
    /// ignored. The owner's quiescence resolves only after twice
    /// `SHARED_NETWORK_QUIESCE_CALL_BOUND` on the injected clock, naming A
    /// unconfirmed. At the bound the supervisor answers the miss: the
    /// workloads-slice `cgroup.kill` is already in the snapshot when the one
    /// `TapQuiescenceUndetermined` request is received. Past the late
    /// result's instant and further audit periods, no per-VM kill for A is
    /// ever written or announced, no repair or restore runs, and the gate
    /// never reopens.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-32)"]
    async fn a_quiescence_result_after_its_bound_is_ignored() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        let late = super::SHARED_NETWORK_QUIESCE_CALL_BOUND * 2;
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "late-quiescence-result",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            rig.owner.script_quiesce(TestQuiesceScript::Late {
                unconfirmed: BTreeSet::from([alloc_id(VM_A)]),
                after: late,
            });
            let detection = rig.detect(bridge, ONE_MS).await;
            let verdict = rig.verdict();
            assert_eq!(
                rig.count_since(detection.owner_calls, |call| {
                    *call == TestOwnerCall::Quiesce(TestQuiesceOutcome::Late)
                }),
                1,
                "{verdict}: the detection began the one quiescence call"
            );
            assert!(rig.poll_request().is_none(), "{verdict}: the call is still within its bound");
            rig.advance_quiet(earlier(super::SHARED_NETWORK_QUIESCE_CALL_BOUND, ONE_MS)).await;
            assert!(rig.poll_request().is_none(), "{verdict}: not before the quiescence bound");
            rig.advance(ONE_MS).await;

            let request = rig
                .poll_request()
                .cloned()
                .unwrap_or_else(|| panic!("{verdict}: the missed bound fail-stops"));
            let expected = ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
                component: SharedGuestNetworkComponent::Bridge,
                cause: SharedGuestNetworkFailStopCause::TapQuiescenceUndetermined,
                attempts: 0,
                elapsed: super::SHARED_NETWORK_QUIESCE_CALL_BOUND,
            });
            assert_eq!(request, expected, "{verdict}");
            let at_receipt = rig
                .request_cgroups
                .clone()
                .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
            assert!(
                killed(&at_receipt, &slice_kill(&rig.root)),
                "{verdict}: the workloads-slice kill already fired when the request arrived"
            );
            assert!(untouched(&at_receipt, &scope_kill(&rig.root, VM_A)), "{verdict}");

            // The late result's instant passes, then further audit periods.
            let after_bound = earlier(late, super::SHARED_NETWORK_QUIESCE_CALL_BOUND);
            rig.advance_quiet(after_bound).await;
            for _ in 0..2 {
                rig.advance_quiet(super::SHARED_NETWORK_AUDIT_PERIOD).await;
            }
            let snapshot = rig.fs.snapshot();
            assert!(
                untouched(&snapshot, &scope_kill(&rig.root, VM_A)),
                "{verdict}: no per-VM kill is written for the late result's set"
            );
            assert!(untouched(&snapshot, &scope_kill(&rig.root, VM_B)), "{verdict}");
            assert!(rig.vm_killed().is_empty(), "{verdict}: nothing is announced for the late set");
            assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}: never reopens");
            assert_eq!(rig.exec.recovery_progress(), None, "{verdict}");
            let calls = rig.calls();
            assert!(!calls.iter().any(is_owner_converge), "{verdict}: no repair attempt runs");
            assert!(!calls.iter().any(is_restore), "{verdict}: no restore runs");
            rig.finish().await;
        }
    }

    fn supervisor_handle(
        case: SupervisorExitCase,
        recovering: Option<SharedGuestNetworkComponent>,
    ) -> (
        SharedNetworkSupervisorHandle,
        Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
    ) {
        let clock = Arc::new(SimClock::new());
        let wiring = GuestNetworkExecWiring::new(clock.clone());
        let exec = wiring.supervisor();
        assert!(exec.open_after_boot());
        if let Some(component) = recovering {
            assert!(exec.begin_recovery(component));
            clock.tick(Duration::from_millis(250));
            assert!(exec.complete_attempt(Some(component)));
            clock.tick(Duration::from_millis(250));
            assert!(exec.complete_attempt(Some(component)));
            clock.tick(Duration::from_millis(750));
        }
        let shutdown = CancellationToken::new();
        let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
        let task = match case {
            SupervisorExitCase::Returned => {
                let request_owner = request_tx.clone();
                tokio::spawn(async move {
                    let _request_owner = request_owner;
                    Ok(())
                })
            }
            SupervisorExitCase::Failed => {
                let request_owner = request_tx.clone();
                tokio::spawn(async move {
                    let _request_owner = request_owner;
                    Err(SharedNetworkSupervisorError::GuestNetwork(
                        guest_network::GuestNetworkError::Io {
                            operation: guest_network::GuestNetworkOperation::BridgeObserve,
                            source: std::io::Error::other("scripted supervisor failure"),
                        },
                    ))
                })
            }
            SupervisorExitCase::Panicked => {
                let request_owner = request_tx.clone();
                tokio::spawn(async move {
                    let _request_owner = request_owner;
                    panic!("scripted supervisor panic");
                    #[allow(unreachable_code)]
                    Ok(())
                })
            }
            SupervisorExitCase::Cancelled => {
                let request_owner = request_tx.clone();
                let task = tokio::spawn(async move {
                    let _request_owner = request_owner;
                    std::future::pending::<()>().await;
                    Ok(())
                });
                task.abort();
                task
            }
            SupervisorExitCase::RequestChannelClosed => tokio::spawn(async {
                std::future::pending::<()>().await;
                Ok(())
            }),
        };
        drop(request_tx);
        (SharedNetworkSupervisorHandle::new(request_rx, task, Arc::clone(&exec), shutdown), exec)
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn actual_tokio_exit_matrix_fail_stops_before_returning_the_exact_snapshot() {
        let cases = [
            (SupervisorExitCase::Returned, SharedGuestNetworkFailStopCause::SupervisorReturned),
            (SupervisorExitCase::Failed, SharedGuestNetworkFailStopCause::SupervisorFailed),
            (SupervisorExitCase::Panicked, SharedGuestNetworkFailStopCause::SupervisorPanicked),
            (SupervisorExitCase::Cancelled, SharedGuestNetworkFailStopCause::SupervisorCancelled),
            (
                SupervisorExitCase::RequestChannelClosed,
                SharedGuestNetworkFailStopCause::RequestChannelClosed,
            ),
        ];

        for (case, cause) in cases {
            for recovering in std::iter::once(None).chain(COMPONENTS.map(Some)) {
                let (mut owner, exec) = supervisor_handle(case, recovering);
                let ServeShutdownRequest::SharedGuestNetwork(request) =
                    owner.shutdown_requested().await;
                assert_eq!(request.cause, cause);
                if let Some(component) = recovering {
                    assert_eq!(request.component, component);
                    assert_eq!(request.attempts, 2);
                    assert_eq!(request.elapsed, Duration::from_millis(1_250));
                } else {
                    assert_eq!(request.component, SharedGuestNetworkComponent::Supervisor);
                    assert_eq!(request.attempts, 0);
                    assert_eq!(request.elapsed, Duration::ZERO);
                }
                assert!(exec.fail_stop(cause).is_none(), "owner already wrote FailStop");
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn explicit_request_is_returned_unchanged_and_intentional_shutdown_is_not_failure() {
        let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
        let exec = wiring.supervisor();
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(async move {
            task_shutdown.cancelled().await;
            Ok(())
        });
        let expected = ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
            component: SharedGuestNetworkComponent::BridgeGuard,
            cause: SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
            attempts: 20,
            elapsed: Duration::from_secs(5),
        });
        request_tx.send(expected.clone()).await.expect("send explicit request");
        let mut owner = SharedNetworkSupervisorHandle::new(request_rx, task, exec, shutdown);
        assert_eq!(owner.shutdown_requested().await, expected);
        drop(request_tx);

        let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            let _request_owner = request_tx;
            task_shutdown.cancelled().await;
            Ok(())
        });
        SharedNetworkSupervisorHandle::new(request_rx, task, wiring.supervisor(), shutdown)
            .shutdown()
            .await;
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED
    /// S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A per-VM `cgroup.kill` write that never answers is a bounded wait: the
    /// supervisor abandons it at `SHARED_NETWORK_VM_KILL_CALL_BOUND` on the
    /// injected clock — not before — kills the workloads slice, and sends one
    /// `VmKillFailed` request (user decision 1 of 2026-09-30). The recovery
    /// deadline does not cap the write; its own bound does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "pending DELIVER step 09-01 (S-ND295-32)"]
    async fn a_per_vm_kill_write_is_bounded_by_its_own_bound() {
        let bridge = Loss::Owner(SharedGuestNetworkComponent::Bridge);
        for seed in supervisor_seeds() {
            let mut rig = Rig::build(
                seed,
                "kill-write-never-answers",
                RigSetup { scoped: vec![VM_A, VM_B], slice: true, ..RigSetup::default() },
            )
            .await;
            rig.run_until_first_audit().await;
            // A's kill write never answers within the window.
            rig.fs.park_until(
                SimOp::Write,
                scope_kill(&rig.root, VM_A),
                Arc::clone(&rig.clock) as Arc<dyn Clock>,
                super::SHARED_NETWORK_RECOVERY_DEADLINE * 4,
            );
            rig.owner
                .script_quiesce(TestQuiesceScript::Unconfirmed(BTreeSet::from([alloc_id(VM_A)])));
            rig.detect(bridge, ONE_MS).await;
            let verdict = rig.verdict();
            rig.advance_quiet(earlier(super::SHARED_NETWORK_VM_KILL_CALL_BOUND, ONE_MS)).await;
            assert!(rig.poll_request().is_none(), "{verdict}: the wait is not abandoned early");
            rig.advance(ONE_MS).await;
            let request = rig
                .poll_request()
                .cloned()
                .unwrap_or_else(|| panic!("{verdict}: the kill write is abandoned at its bound"));
            let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
            assert_eq!(
                fail_stop.cause,
                SharedGuestNetworkFailStopCause::VmKillFailed,
                "{verdict}: an abandoned kill write is VmKillFailed"
            );
            let at_receipt = rig
                .request_cgroups
                .clone()
                .unwrap_or_else(|| panic!("{verdict}: the rig snapshots at receipt"));
            assert!(killed(&at_receipt, &slice_kill(&rig.root)), "{verdict}: the slice is killed");
            assert_eq!(rig.admission(), Admission::FailStopped, "{verdict}");
            rig.finish().await;
        }
    }

    // -----------------------------------------------------------------------
    // S-ND295-34 — the DNS task owner over the `GuestDns` port
    // -----------------------------------------------------------------------

    /// One responder built through the test-local factory: the port handle the
    /// owner holds, and the concrete double the test scripts.
    fn test_dns(factory: &TestGuestDnsFactory) -> (Arc<dyn GuestDns>, Arc<TestGuestDns>) {
        let port = factory.responder(dns_deps(&Arc::new(SimClock::new())));
        let concrete = factory.responders().pop().expect("the factory records every responder");
        (port, concrete)
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH; OUT-ND295-BORN-CAPTURED
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands until a fresh responder is serving
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The owner holds `Arc<dyn GuestDns>`: a serve that returns or panics,
    /// and a cancelled task, are classified exactly; a replacement stops the
    /// live responder through the port and publishes one fresh one.
    #[tokio::test]
    async fn dns_task_owner_classifies_real_exits_and_never_overwrites_a_live_handle() {
        for expected in
            [DnsServeTaskExit::Returned, DnsServeTaskExit::Panicked, DnsServeTaskExit::Cancelled]
        {
            let factory = TestGuestDnsFactory::default();
            let (responder, live) = test_dns(&factory);
            let task = tokio::spawn(Arc::clone(&responder).serve());
            match expected {
                DnsServeTaskExit::Returned => live.end_serve(TestGuestDnsServeExit::Return),
                DnsServeTaskExit::Panicked => live.end_serve(TestGuestDnsServeExit::Panic),
                DnsServeTaskExit::Cancelled => task.abort(),
            }
            let mut owner = DnsServeTaskOwner::new(responder, task);
            assert_eq!(owner.wait_failure().await, expected);
            assert_eq!(owner.state, DnsServeTaskState::Exited(expected));
            assert!(owner.task.is_none());
        }

        let factory = TestGuestDnsFactory::default();
        let (old, _) = test_dns(&factory);
        let mut owner =
            DnsServeTaskOwner::new(Arc::clone(&old), tokio::spawn(Arc::clone(&old).serve()));
        let (replacement, _) = test_dns(&factory);
        let spawned = Arc::new(AtomicUsize::new(0));
        let spawned_for_closure = Arc::clone(&spawned);
        owner
            .replace(replacement, Duration::from_millis(10), move |responder| {
                spawned_for_closure.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(responder.serve())
            })
            .await
            .expect("the old task terminates before one replacement is published");
        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        assert_eq!(owner.state, DnsServeTaskState::Running);
        owner.shutdown(Duration::from_millis(10)).await;
        assert_eq!(owner.state, DnsServeTaskState::Stopped);
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH; OUT-ND295-BORN-CAPTURED
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands until a fresh responder is serving
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn dns_replacement_joins_the_old_task_before_spawning_from_live_and_exited_states() {
        struct ExitWitness(Arc<AtomicBool>);
        impl Drop for ExitWitness {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        for old_already_exited in [false, true] {
            let factory = TestGuestDnsFactory::default();
            let (old, live) = test_dns(&factory);
            let old_ended = Arc::new(AtomicBool::new(false));
            let old_ended_in_task = Arc::clone(&old_ended);
            let serving = Arc::clone(&old);
            let old_task = tokio::spawn(async move {
                let _witness = ExitWitness(old_ended_in_task);
                serving.serve().await;
            });
            let mut owner = DnsServeTaskOwner::new(old, old_task);
            if old_already_exited {
                live.end_serve(TestGuestDnsServeExit::Return);
                assert_eq!(owner.wait_failure().await, DnsServeTaskExit::Returned);
            }
            let (replacement, _) = test_dns(&factory);
            let spawns = Arc::new(AtomicUsize::new(0));
            let spawns_in_closure = Arc::clone(&spawns);
            let old_ended_at_spawn = Arc::clone(&old_ended);
            owner
                .replace(replacement, Duration::from_millis(10), move |responder| {
                    assert!(
                        old_ended_at_spawn.load(Ordering::SeqCst),
                        "the old JoinHandle is terminal before replacement publication"
                    );
                    spawns_in_closure.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(responder.serve())
                })
                .await
                .expect("Running and Exited are the only replacement origins");
            assert_eq!(spawns.load(Ordering::SeqCst), 1);
            assert_eq!(owner.state, DnsServeTaskState::Running);
            assert!(owner.task.is_some());
            owner.shutdown(Duration::from_millis(10)).await;
            assert_eq!(owner.state, DnsServeTaskState::Stopped);
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH; OUT-ND295-BORN-CAPTURED
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands until a fresh responder is serving
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Shutdown stops the responder through the port, so a serving task ends
    /// cooperatively; a task that ignores the stop is ended by the bounded
    /// abort backstop.
    #[tokio::test]
    async fn dns_shutdown_prefers_cooperative_stop_and_awaits_the_bounded_abort_backstop() {
        struct MarkEnded(Arc<AtomicBool>);
        impl Drop for MarkEnded {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        for cooperative in [true, false] {
            let factory = TestGuestDnsFactory::default();
            let (responder, _) = test_dns(&factory);
            let ended = Arc::new(AtomicBool::new(false));
            let ended_in_task = Arc::clone(&ended);
            let task = if cooperative {
                let serving = Arc::clone(&responder);
                tokio::spawn(async move {
                    serving.serve().await;
                    ended_in_task.store(true, Ordering::SeqCst);
                })
            } else {
                tokio::spawn(async move {
                    let _ended = MarkEnded(ended_in_task);
                    std::future::pending::<()>().await;
                })
            };
            let mut owner = DnsServeTaskOwner::new(responder, task);
            owner.shutdown(if cooperative { Duration::from_secs(1) } else { Duration::ZERO }).await;
            assert!(ended.load(Ordering::SeqCst));
            assert_eq!(owner.state, DnsServeTaskState::Stopped);
            assert!(owner.task.is_none());
            assert!(owner.responder.is_none());
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH; OUT-ND295-BORN-CAPTURED
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands until a fresh responder is serving
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn dns_replacement_is_refused_from_every_invalid_state_without_spawning() {
        for state in [
            DnsServeTaskState::Replacing,
            DnsServeTaskState::ShuttingDown,
            DnsServeTaskState::Stopped,
        ] {
            let factory = TestGuestDnsFactory::default();
            let mut owner = DnsServeTaskOwner { state, responder: None, task: None };
            let spawned = Arc::new(AtomicUsize::new(0));
            let spawned_for_closure = Arc::clone(&spawned);
            let (replacement, _) = test_dns(&factory);
            let error = owner
                .replace(replacement, Duration::from_millis(10), move |_| {
                    spawned_for_closure.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async {})
                })
                .await
                .expect_err("invalid state cannot replace");
            assert!(matches!(
                error,
                DnsServeTaskOwnerError::InvalidReplacementState { state: actual }
                    if actual == state
            ));
            assert_eq!(spawned.load(Ordering::SeqCst), 0);
            assert_eq!(owner.state, state);
            assert!(owner.task.is_none());
        }
    }
}

/// Typed failure returned when the server's userspace mTLS owner cannot
/// converge teardown. The worker is sealed and every userspace child has ended;
/// this error retains diagnostics only and exposes no retry capability.
#[derive(Debug)]
pub struct ServerShutdownError {
    source: overdrive_worker::mtls_intercept_worker::MtlsInterceptOwnerShutdownError,
}

impl std::fmt::Display for ServerShutdownError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "server shutdown mTLS teardown failed: {}", self.source)
    }
}

impl std::error::Error for ServerShutdownError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl ServerShutdownError {
    const fn new(
        source: overdrive_worker::mtls_intercept_worker::MtlsInterceptOwnerShutdownError,
    ) -> Self {
        Self { source }
    }

    /// The typed allocation-scoped teardown failures from the last attempt.
    #[must_use]
    pub const fn teardown_failure(
        &self,
    ) -> &overdrive_worker::mtls_intercept_worker::MtlsInterceptOwnerShutdownError {
        &self.source
    }
}

/// Zero-sized witness returned once [`ServerHandle::kill_for_test`] has
/// abandoned the server owner.
///
/// The workload and the kernel state the owner held deliberately survive;
/// every control-plane-owned task, socket, store handle, and listener has been
/// released before this value is returned.
#[doc(hidden)]
#[cfg(any(test, feature = "integration-tests"))]
#[derive(Debug)]
pub struct AbruptServerResidue;

/// Killed mode's final step: drop the mTLS worker as its LAST owner on a
/// dedicated thread that has relinquished every Linux capability, so the
/// worker's guard destructors cannot mutate kernel state the dead owner held.
///
/// Every `AppState` clone holds a worker reference; the caller has already
/// aborted and joined every task holding one, and lagging connection tasks
/// release theirs once they observe axum's immediate shutdown. A bounded wait
/// for sole ownership covers that lag. A degraded path (ownership never
/// becomes exclusive, the thread cannot be spawned, or capabilities cannot be
/// relinquished) is reported loudly; killed-mode callers prove the surviving
/// kernel residue independently by read-back.
#[cfg(any(test, feature = "integration-tests"))]
async fn release_killed_worker_without_host_authority(
    worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
) {
    const SOLE_OWNERSHIP_BOUND: Duration = Duration::from_secs(10);
    let deadline = std::time::Instant::now() + SOLE_OWNERSHIP_BOUND;
    while Arc::strong_count(&worker) > 1 {
        if std::time::Instant::now() >= deadline {
            tracing::error!(
                name: "serve.kill.worker_still_shared",
                strong_count = Arc::strong_count(&worker),
                "killed mode could not become the worker's last owner; its guard \
                 destructors will run elsewhere with host authority"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (released_tx, released_rx) = tokio::sync::oneshot::channel();
    let runtime = tokio::runtime::Handle::current();
    let spawned =
        std::thread::Builder::new().name("serve-kill-release".to_owned()).spawn(move || {
            let _entered = runtime.enter();
            let empty = rustix::thread::CapabilitySet::empty();
            let relinquished = rustix::thread::set_capabilities(
                None,
                rustix::thread::CapabilitySets {
                    effective: empty,
                    permitted: empty,
                    inheritable: empty,
                },
            );
            if let Err(error) = &relinquished {
                tracing::error!(
                    name: "serve.kill.capabilities_retained",
                    error = %error,
                    "killed mode could not relinquish capabilities before dropping the worker"
                );
            }
            drop(worker);
            let _ = released_tx.send(relinquished.is_ok());
        });
    if let Err(error) = spawned {
        tracing::error!(
            name: "serve.kill.release_thread_unavailable",
            error = %error,
            "killed mode could not spawn its release thread; the worker was dropped here"
        );
        return;
    }
    match released_rx.await {
        Ok(true) => tracing::info!(
            name: "serve.kill.worker_released",
            "killed mode dropped the mTLS worker without host authority"
        ),
        Ok(false) => tracing::error!(
            name: "serve.kill.worker_released_with_authority",
            "killed mode dropped the mTLS worker without relinquishing capabilities"
        ),
        Err(_) => tracing::error!(
            name: "serve.kill.worker_release_lost",
            "killed-mode release thread ended without reporting"
        ),
    }
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

    /// Wait for the retained shared-network owner to request process shutdown.
    pub async fn shutdown_requested(
        &mut self,
    ) -> overdrive_core::guest_network::ServeShutdownRequest {
        self.shared_network_supervisor.shutdown_requested().await
    }

    /// Killed mode: abandon this server owner the way process death would —
    /// no graceful drain, no workload stop, no worker teardown, no capability
    /// retirement — so a test can boot again against the unchanged durable
    /// directories.
    ///
    /// Rust cannot halt a task without dropping it, so this is what still runs,
    /// in order:
    ///
    /// 1. axum's immediate (non-graceful) shutdown notification, then abort and
    ///    join of the server, convergence, emit-drain, interest-router and
    ///    exit-observer tasks. Their futures are dropped on runtime threads:
    ///    listeners, connections, `AppState` clones and store handles are
    ///    released, and any effect still in flight runs its destructors there.
    /// 2. The resolver's List/Watch drain task is cancelled and joined
    ///    (userspace only).
    /// 3. The shared-network supervisor future is cancelled and joined; its
    ///    DNS owner stops the responder's blocking sockets so `:53` is
    ///    released as it would be at process death.
    /// 4. The mTLS worker — sole owner of every allocation's nft element
    ///    guards, TPROXY rule guards, and the shared constant-program guard —
    ///    is dropped as its last owner on a dedicated thread that first
    ///    relinquishes every Linux capability. Its destructors run, but the
    ///    kernel refuses their netlink/BPF requests for lack of
    ///    `CAP_NET_ADMIN`/`CAP_BPF` (the element guard logs
    ///    `health.mtls.shared_element_cleanup_failed`), so the kernel objects
    ///    the dead owner held survive as after `SIGKILL`, while its sockets,
    ///    listener threads, and store handles are released.
    ///
    /// Workload processes (spawned `kill_on_drop(false)`), cgroups, TAPs, the
    /// bridge, pinned TCX links and nft objects are never touched. The
    /// production graceful path ([`Self::shutdown`]) is unchanged.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub async fn kill_for_test(self) -> AbruptServerResidue {
        let Self {
            inner,
            server_task,
            convergence_task,
            exit_observer_tasks,
            emit_drain_task,
            interest_router_task,
            convergence_shutdown: _,
            exit_observer_shutdown: _,
            emit_drain_shutdown: _,
            interest_router_shutdown: _,
            mtls_worker_owner,
            mtls_resolve_owner,
            shared_network_supervisor,
        } = self;

        // The axum server owns one `AppState` clone and each accepted
        // connection task owns another.  Aborting only the server task leaves
        // those connection tasks detached; they retain the observation store
        // across the abrupt-owner boundary and a same-data-dir replacement
        // then fails redb's single-open lock.  Signal the non-graceful
        // shutdown first so every watcher drops its service, then retain the
        // existing abort/join fence for the server task itself.
        inner.shutdown();
        server_task.abort();
        convergence_task.abort();
        emit_drain_task.abort();
        interest_router_task.abort();
        for task in &exit_observer_tasks {
            task.abort();
        }

        let _ = server_task.await;
        let _ = convergence_task.await;
        let _ = emit_drain_task.await;
        let _ = interest_router_task.await;
        for task in exit_observer_tasks {
            let _ = task.await;
        }

        mtls_resolve_owner.shutdown().await;
        shared_network_supervisor.shutdown().await;
        release_killed_worker_without_host_authority(mtls_worker_owner).await;
        AbruptServerResidue
    }

    /// Whether the interest-router task (ADR-0084 §5, Piece B) is live — the
    /// structural boot check for the S-266-01 vertical slice. Returns `true`
    /// while the task spawned by [`run_server_with_obs_and_driver`] has not
    /// yet finished (it parks in `subscription.next()` between accepted
    /// changes, so it is `false` only after shutdown or a watch close).
    ///
    /// The teeth: a boot that omits the `spawn_interest_router` wiring has no
    /// task to report live — the mechanism would be dead code wearing a green
    /// suite (CLAUDE.md § "Build vertical slices through production entry
    /// points").
    #[must_use]
    pub fn interest_router_running(&self) -> bool {
        !self.interest_router_task.is_finished()
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
    pub async fn shutdown(self, drain_deadline: Duration) -> Result<(), ServerShutdownError> {
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

        // 1c. Cancel the interest-router loop and await its completion
        //     (ADR-0084 §5). It holds an `Arc<dyn ObservationStore>` and a
        //     broker capability, so — like the convergence loop — it must stop
        //     before axum tears down `AppState`. Cooperative: the router's
        //     `tokio::select!` resolves the cancellation branch even while
        //     parked on `subscription.next()`, so the join is bounded (a bare
        //     `abort()` on the parked recv would not interrupt it).
        self.interest_router_shutdown.cancel();
        let _ = self.interest_router_task.await;

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
        //    cleanly even when driver watcher tasks or test
        //    harness `Arc<dyn Driver>` refs still hold `exit_tx`
        //    clones. Without this token, a workload that did not
        //    reap before convergence was cancelled — or a SimDriver
        //    held by the test fn until its scope ends — would keep
        //    `rx.recv()` blocked indefinitely, deadlocking shutdown.
        //    Per `fix-exec-driver-exit-watcher` Step 01-02 follow-up.
        self.exit_observer_shutdown.cancel();
        for task in self.exit_observer_tasks {
            let _ = task.await;
        }

        // 5. Join the complete worker-owned userspace dataplane. This does
        // not stop workloads or author lifecycle state; it only releases the
        // listeners/connections whose lifetime is this serve owner's lifetime.
        // Active allocation rule guards are relinquished without deletion.
        let worker_failure =
            self.mtls_worker_owner.shutdown_owner().await.err().map(ServerShutdownError::new);
        self.mtls_resolve_owner.shutdown().await;
        self.shared_network_supervisor.shutdown().await;
        worker_failure.map_or(Ok(()), Err)
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
/// dataplane configuration contract.
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
    // production driver registry at this composition root, then delegate
    // to `run_server_with_obs_and_drivers`. The split
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
    // probe-runner Earned-Trust gate runs here, at the binary composition
    // root. Keep the one trusted runner returned by the gate and pass it to
    // the VM driver's existing lifecycle hooks.
    let clock: Arc<dyn Clock> = Arc::new(overdrive_host::SystemClock);
    let probe_runner = probe_runner_boot::compose_and_probe_runner_gate(
        Arc::new(overdrive_worker::probe_runner::TokioTcpProber::new()),
        Arc::new(overdrive_worker::probe_runner::HyperHttpProber::new()),
        Arc::clone(&clock),
        Arc::clone(&obs),
    )
    .await?;
    let guest_network_exec = GuestNetworkExecWiring::new(Arc::clone(&clock));

    let mut registry = DriverRegistry::new();

    // ADR-0082/ADR-0083 (GH #42, steps 01-08/01-09, §D3c): discover ->
    // probe -> insert `cloud-hypervisor`. UNCONDITIONAL — no `#[cfg]`, no
    // artifact precondition. Artifacts are per-allocation (§D3a), so the
    // ONLY thing that decides whether this node can run microVMs is
    // whether `Vmm::probe` passes, and "the registry IS the VM capability
    // gate" is finally true in production.
    //
    // Capability ABSENCE (the real, non-injected discover/probe sequence
    // failing — a host with no `cloud-hypervisor`, or SD-5's "no
    // genuinely lying host exists in the real test envelope for these
    // classes" case) is NOT a fault: the node boots with no `Vm` entry
    // and `[vm]` deploys fail at dispatch naming the absent capability
    // (`VmComposeError::NotAvailable`). A GENUINE substrate lie caught on
    // an adapter the composition root already knows is PRESENT — the real
    // adapter discovered successfully-but-then-failed-probe is not
    // reachable in this codebase's test envelope, so in practice this is
    // exclusively the injected `ServerConfig.vmm_override` path (ADR-0083
    // §D8) declaring presence — hard-refuses the whole boot with
    // `health.startup.refused` (`VmComposeError::Refused`).
    {
        let cgroup_accounting: Arc<
            dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting,
        > = Arc::new(overdrive_host::RealCgroupAccounting::new());
        #[cfg(feature = "integration-tests")]
        let vmm_override = config.vmm_override.clone();
        #[cfg(not(feature = "integration-tests"))]
        let vmm_override = None;
        match compose_vm_driver(
            cgroup_root_path.clone(),
            overdrive_core::vm::config::clone_index_dir(&config.data_dir),
            overdrive_core::vm::config::clone_staging_dir(&config.data_dir),
            Arc::clone(&clock),
            fs,
            cgroup_accounting,
            Arc::clone(&probe_runner),
            guest_network_exec.gate(),
            vmm_override,
        )
        .await
        {
            Ok(vm_driver) => registry.insert(Arc::new(vm_driver)),
            Err(VmComposeError::NotAvailable(cause)) => {
                tracing::info!(
                    name: "driver.vm.not_composed",
                    reason = %cause,
                    "cloud-hypervisor not composed; [vm] deploys will be rejected at admission"
                );
            }
            Err(VmComposeError::Refused(cause)) => {
                tracing::error!(
                    name: "health.startup.refused",
                    target: "overdrive::health",
                    reason = "vmm.probe",
                    cause = %cause,
                    "VM driver probe refused; composition root will not start"
                );
                return Err(error::ControlPlaneError::VmmBoot(cause));
            }
        }
    }

    let vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState> =
        Arc::new(overdrive_host::RealVmHostState::new(
            cgroup_root_path,
            std::path::PathBuf::from("/run/overdrive/vm"),
            overdrive_core::vm::config::clone_index_dir(&config.data_dir),
        ));

    run_server_with_obs_and_drivers(
        config,
        obs,
        Arc::new(registry),
        vm_host_state,
        Arc::new(guest_network::HostSharedGuestNetworkOwner::new()),
        guest_network_exec,
        bootstrap_manager,
    )
    .await
}

/// Outcome of a failed [`compose_vm_driver`] attempt — distinguishes
/// capability ABSENCE (soft: the node simply has no `Vm` entry) from a
/// genuine substrate lie caught by a probe against an adapter the
/// composition root (or an injected `ServerConfig.vmm_override` test
/// seam) already knows is present (hard: refuses the whole boot).
/// ADR-0083 §D8.
#[derive(Debug)]
enum VmComposeError {
    /// No override was injected, and the real discover/probe
    /// sequence failed. Capability absence is not a fault
    /// (SD-5) — the caller logs `driver.vm.not_composed` and continues
    /// booting with no `Vm` entry. Carries the typed
    /// [`error::VmmBootError`] cause (not a pre-flattened `String`) per
    /// `.claude/rules/development.md` § "Never flatten a typed error to
    /// `Internal(String)` at a composition boundary" — the
    /// `driver.vm.not_composed` log line renders it via `Display`
    /// (`%cause`), and a test can `matches!()` on the variant.
    NotAvailable(error::VmmBootError),
    /// An override WAS injected (`ServerConfig.vmm_override`) — the test
    /// is declaring "cloud-hypervisor IS present" — and its probe (or
    /// `CgroupAccounting`'s probe, or the kernel-header read/validate)
    /// failed. Unambiguously a genuine substrate lie, never absence: the
    /// caller refuses the whole boot with `health.startup.refused`,
    /// converting directly into `ControlPlaneError::VmmBoot` — the SAME
    /// typed [`error::VmmBootError`] value, no re-stringification at the
    /// boundary.
    Refused(error::VmmBootError),
}

/// Compose the production `VmDriver` — resolve `Vmm` (the injected
/// `vmm_override`, when present, or a freshly discovered
/// `CloudHypervisorVmm`) and run its Earned-Trust `.probe()` (and
/// `CgroupAccounting`'s, gated alongside it per ADR-0082 §D8's
/// "Composition" section). ADR-0082/ADR-0083 (GH #42, steps 01-08/01-09,
/// §§D3a/D3c).
///
/// Takes NO artifact argument and carries NO `#[cfg]`: the kernel and
/// rootfs are per-allocation (§D3a), so there is nothing node-level left
/// to configure, and the probe is the whole gate. What is proven here is
/// the HOST's microVM capability, once; what each allocation's own
/// artifacts are is proven per start by `VmDriver`'s `preflight_kernel`
/// and rootfs preflight (§D3b's table).
///
/// See [`VmComposeError`] for the two-way error split; see
/// `run_server`'s call site doc comment for the ABSENCE-vs-REFUSAL
/// framing.
/// The DEDICATED reserved, unprivileged NUMERIC identity the confined
/// hypervisor is spawned under (ADR-0082 §(e), sharpened by the 2026-08-18
/// fourth amendment (e-fix) M2). No `/etc/passwd`/`/etc/group` entry is
/// required — `setpriv --reuid/--regid` take raw numerics, which is precisely
/// what satisfies the "no appliance-image change" constraint.
///
/// `4200` is a value the appliance leaves unassigned: verified FREE against
/// the metal box's `/etc/passwd` + `/etc/group` (no user, no group at 4200/4200)
/// and unused by any base-OS daemon (highest in-use is the `ubuntu` login user
/// at 1000). It is deliberately NOT `65534` (`nobody`/`nogroup`) — the kernel's
/// overflow/anonymous id (`/proc/sys/kernel/overflowuid`, NFS id-squash, unmapped
/// user-namespace principals), a SHARED system identity the platform has not
/// reserved and which would NOT ptrace/signal/`/proc`-isolate the hypervisor from
/// every overflow-mapped principal on the host (M2 rejects it). Both the uid and
/// its primary gid are dedicated numerics.
///
/// The confined uid is SHARED across VMs and does NOT isolate siblings —
/// Landlock (the per-VM run-directory grant), the per-VM netns and the per-VM
/// cgroup do that; a per-VM uid is GH #258, not US-VM-7.
const OVERDRIVE_VMM_GID: u32 = 4_200;

/// Traverse-only permission mode for the platform-owned VM clone-staging root
/// (`0710`): owner (root) rwx, group (the confined gid) `--x` (traverse, not
/// read/list), other nothing. Applied ONCE at node setup with
/// `root:<confined-gid>` ownership so the confined identity can reach its OWN
/// chown'd clone by name without being able to LIST the staging root and
/// discover other allocations' clone names (ADR-0082 2026-08-18 fourth
/// amendment (c-fix.2)). Sibling-disk isolation is Landlock's job (P5), not
/// DAC's — a flat staging root with alloc-named files is correct.
const OVERDRIVE_VMM_STAGING_MODE: u32 = 0o710;

/// `RLIMIT_NOFILE` cap for the confined hypervisor. `256` is spike P5's proven
/// value (a full `--landlock` + vsock + disk + serial + api boot completed
/// under it) and is comfortably below any reasonable `overdrive serve` open-
/// files ceiling, so the confined limit is strictly below serve's (S-VM-49).
const OVERDRIVE_VMM_RLIMIT_NOFILE: u64 = 256;

#[allow(
    clippy::too_many_arguments,
    reason = "the composition helper receives the existing VM ports and the single trusted ProbeRunner without introducing a configuration wrapper"
)]
async fn compose_vm_driver(
    cgroup_root: std::path::PathBuf,
    clone_index_dir: std::path::PathBuf,
    clone_staging_dir: std::path::PathBuf,
    clock: Arc<dyn Clock>,
    fs: Arc<dyn overdrive_core::traits::cgroup_fs::CgroupFs>,
    cgroup_accounting: Arc<dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting>,
    probe_runner: Arc<overdrive_worker::probe_runner::ProbeRunner>,
    guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
    vmm_override: Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>,
) -> std::result::Result<overdrive_worker::vm_driver::VmDriver, VmComposeError> {
    use overdrive_core::traits::vmm::Vmm;
    use overdrive_core::vm::config::{HostArch, VmConfinement, VmmIdentity};
    use overdrive_worker::vm_driver::VmHostLayout;

    // §D8's resolution: the override, when present, replaces discovery
    // entirely. `injected` records WHICH branch was taken so a
    // subsequent probe failure is classified correctly — see
    // `VmComposeError`'s own docs.
    let injected = vmm_override.is_some();
    let vmm: Arc<dyn Vmm> = match vmm_override {
        Some(injected_vmm) => injected_vmm,
        None => Arc::new(overdrive_host::CloudHypervisorVmm::new()),
    };
    // Earned Trust is UNCONDITIONAL here: `.probe()` runs against
    // WHATEVER adapter is bound, production or injected — there is no
    // `if injected { skip probe }` branch anywhere in this function.
    if let Err(source) = vmm.probe().await {
        let cause = error::VmmBootError::Probe { source };
        return Err(if injected {
            VmComposeError::Refused(cause)
        } else {
            VmComposeError::NotAvailable(cause)
        });
    }
    // `CgroupAccounting` rides the SAME composition gate as `Vmm`
    // (ADR-0082 §D8 "Composition"): probed alongside it, refusing the
    // node on the same substrate-lie / capability-absence split.
    if let Err(source) = cgroup_accounting.probe().await {
        let cause = error::VmmBootError::CgroupAccountingProbe { source };
        return Err(if injected {
            VmComposeError::Refused(cause)
        } else {
            VmComposeError::NotAvailable(cause)
        });
    }

    // Host architecture — `cfg(target_arch)` at compile time, matching
    // the binary that is actually running (the same substrate `probe()`
    // just validated).
    let arch = if cfg!(target_arch = "aarch64") { HostArch::Aarch64 } else { HostArch::X86_64 };

    // Resolve the `kvm` group gid by stat-ing `/dev/kvm`'s group owner — the
    // same `metadata.gid()` the probe's `probe_kvm_reachable` reads (ADR-0082
    // §(e)). `/dev/kvm` is `0660 root:kvm`; the confined uid reaches it ONLY
    // via kvm-group membership, so this gid MUST land in `supplementary` or
    // the uid-drop denies `/dev/kvm` — the empty-`supplementary` bug the
    // placeholder carried. The probe above already opened `/dev/kvm` `O_RDWR`,
    // so a stat failure here is a TOCTOU anomaly: refuse the node fail-closed
    // rather than confine against an unknown kvm group.
    let kvm_gid = match tokio::fs::metadata("/dev/kvm").await {
        Ok(meta) => {
            use std::os::unix::fs::MetadataExt;
            meta.gid()
        }
        Err(source) => {
            let cause = error::VmmBootError::Probe {
                source: overdrive_core::traits::vmm::VmmProbeError::kvm_unreachable(
                    overdrive_core::vm::config::OVERDRIVE_VMM_UID,
                    OVERDRIVE_VMM_GID,
                    0,
                    source,
                ),
            };
            return Err(if injected {
                VmComposeError::Refused(cause)
            } else {
                VmComposeError::NotAvailable(cause)
            });
        }
    };

    // Node setup (once, idempotent — converge-on-boot): create the
    // platform-owned VM clone-staging root with the confined-identity
    // traverse posture (`0710 root:<confined-gid>`). Every per-launch rootfs
    // clone is FICLONE'd here (never beside the operator's master, so no
    // operator dir is ever widened — the B1 fix), and the confined uid reaches
    // its OWN chown'd clone via the group-execute traverse bit without being
    // able to LIST the directory (ADR-0082 2026-08-18 fourth amendment
    // (c-fix.2)). A failure here refuses the node fail-closed — no VM can be
    // confined without it — via the SAME injected/discovered split every
    // fallible step above uses.
    if let Err(source) = prepare_clone_staging_root(&clone_staging_dir, OVERDRIVE_VMM_GID).await {
        let cause = error::VmmBootError::Probe {
            source: overdrive_core::traits::vmm::VmmProbeError::run_dir_unusable(
                clone_staging_dir.clone(),
                source,
            ),
        };
        return Err(if injected {
            VmComposeError::Refused(cause)
        } else {
            VmComposeError::NotAvailable(cause)
        });
    }

    let layout = VmHostLayout {
        cgroup_root,
        run_dir_root: std::path::PathBuf::from("/run/overdrive/vm"),
        // ADR-0083 §§D3f-D3h: where `start` records each launch's clone as
        // a durable symlink BEFORE the FICLONE. The SAME expression
        // `RealVmHostState`'s `index_dir` is fed — `clone_index_dir` is the
        // one derivation both composition sites call (S-VM-84 criterion 4).
        clone_index_dir,
        // The platform-owned staging root just created above with the
        // confined-identity traverse posture; `RootfsPlan::for_alloc` stages
        // each clone here (ADR-0082 2026-08-18 fourth amendment, B1 fix).
        clone_staging_dir,
        arch,
        // Confined identity per ADR-0082 §(e) (gap-5 closure). A reserved,
        // unprivileged NUMERIC identity the platform runs the hypervisor
        // under — no `/etc/passwd`/`/etc/group` entry required (`setpriv`
        // takes raw numerics). The `kvm` group gid, discovered at compose
        // time by stat-ing `/dev/kvm`, rides in `supplementary` — WITHOUT
        // it the uid-drop denies `/dev/kvm` (the specific bug the prior
        // `supplementary: vec![]` placeholder carried). Root (0/0) is
        // deliberately avoided — it would defeat confinement outright.
        confinement: VmConfinement::confined(
            VmmIdentity {
                uid: overdrive_core::vm::config::OVERDRIVE_VMM_UID,
                gid: overdrive_core::vm::config::Gid::new(OVERDRIVE_VMM_GID),
                supplementary: vec![overdrive_core::vm::config::Gid::new(kvm_gid)],
            },
            OVERDRIVE_VMM_RLIMIT_NOFILE,
        ),
    };

    Ok(overdrive_worker::vm_driver::VmDriver::new(
        vmm,
        clock,
        fs,
        cgroup_accounting,
        probe_runner,
        guest_network_exec,
        layout,
    ))
}

/// Node-setup (once, idempotent) for the platform-owned VM clone-staging root:
/// create it, then set `root:<gid>` ownership and the `0710`
/// ([`OVERDRIVE_VMM_STAGING_MODE`]) traverse posture so the confined identity
/// can reach its OWN chown'd clone by name without listing the directory
/// (ADR-0082 2026-08-18 fourth amendment (c-fix.2)). Converge-on-boot: the
/// mkdir is `-p` and the chown/chmod are re-applied every boot, so a
/// pre-existing staging root from a prior boot converges rather than erroring.
/// `chown` needs root, which `overdrive serve` (and the Tier-3 harness) run as.
async fn prepare_clone_staging_root(dir: &std::path::Path, gid: u32) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::PermissionsExt;
        std::os::unix::fs::chown(&dir, Some(0), Some(gid))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(OVERDRIVE_VMM_STAGING_MODE))
    })
    .await
    .map_err(|join_err| {
        std::io::Error::other(format!("clone-staging-root setup task panicked: {join_err}"))
    })?
}

/// Start the control-plane server with caller-supplied observation store
/// and a SINGLE driver, wrapped into a fresh single-entry
/// [`DriverRegistry`] (ADR-0083 §D1, GH #42). Convenience sibling of
/// [`run_server_with_obs_and_drivers`] for callers that already own one
/// driver. Production composition and tests that need more than one
/// capability call [`run_server_with_obs_and_drivers`] directly with a
/// caller-built registry.
pub async fn run_server_with_obs_and_driver(
    config: ServerConfig,
    obs: Arc<dyn ObservationStore>,
    driver: Arc<dyn Driver>,
    vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState>,
    shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    guest_network_exec: GuestNetworkExecWiring,
    vm_cgroups: CgroupManager,
) -> Result<ServerHandle, error::ControlPlaneError> {
    let mut registry = DriverRegistry::new();
    registry.insert(driver);
    run_server_with_obs_and_drivers(
        config,
        obs,
        Arc::new(registry),
        vm_host_state,
        shared_guest_network,
        guest_network_exec,
        vm_cgroups,
    )
    .await
}

/// Start the control-plane server with caller-supplied observation
/// store and driver registry.
///
/// Per ADR-0022 (amended by ADR-0029) / ADR-0083 §D1 (GH #42), the
/// binary owns the composition: the CLI's `serve` subcommand composes the
/// available driver registry and threads it through this function.
///
/// Used by integration tests that need to retain a handle to the
/// observation store the server is reading from.
// `async` is kept to preserve the public-API shape: every caller
// invokes `run_server_with_obs_and_drivers(...).await`, and the function
// may grow real `.await` points as the boot sequence evolves
// (observation provisioning, lifecycle handshakes). Removing it now
// would churn every call site for no functional gain.
#[allow(clippy::unused_async, clippy::too_many_lines)]
pub async fn run_server_with_obs_and_drivers(
    config: ServerConfig,
    obs: Arc<dyn ObservationStore>,
    drivers: Arc<DriverRegistry>,
    vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState>,
    shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    guest_network_exec: GuestNetworkExecWiring,
    vm_cgroups: CgroupManager,
) -> Result<ServerHandle, error::ControlPlaneError> {
    // RED scaffold (D-295-R14): consumed in DELIVER step 09-01, which wraps it
    // in the supervisor's kill-only `VmKillCapability`.
    let _ = vm_cgroups;
    if let Err(cause) = shared_guest_network.probe_startup().await {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "guest_network.probe",
            cause = %cause,
            "shared guest-network startup probe refused"
        );
        return Err(error::ControlPlaneError::from(cause));
    }
    // ADR-0028 preflight, parent-slice delegation, and workloads-slice
    // bootstrap all run in `run_server` (the outer composition
    // boundary). Tests that compose `run_server_with_obs_and_driver`
    // directly are responsible for running these themselves if they
    // need real cgroupfs (typically they run on a properly configured
    // Lima VM — see the integration suite under
    // `crates/overdrive-control-plane/tests/integration/cgroup_isolation/`
    // and `crates/overdrive-worker/tests/integration/`).

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
    // ADR-0013 §9) and `workload-lifecycle` (the first real reconciler,
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
    // microvm-driver-cloud-hypervisor step 02-03 (ADR-0083 §D7, brief.md
    // §105a.7, GH #42) — the `vm-reclamation` reconciler. Registration is
    // INERT (probes the ViewStore + bulk-loads the field-less
    // `VmReclamationView` map; drives no tick) — `spawn_convergence_loop`,
    // spawned strictly AFTER the boot passes below, is the only production
    // driver of ticks, so no tick can interleave with the boot-epoch
    // `vm_reclamation_boot::converge` drive regardless of registration
    // order. Registered here (not gated on `Vm` driver composition) for
    // the same reason `state.vm_host_state` composes unconditionally
    // (S-VM-30): a node with no `Vm` registry entry still reclaims via
    // `Observed(∅)`.
    runtime.register(vm_reclamation()).await?;

    // Production boot threads the `ServerConfig.clock` into AppState
    // so the streaming submit handler's cap timer uses the same clock
    // as the convergence-loop spawn. The clock is required at
    // construction per `.claude/rules/development.md` § "Port-trait
    // dependencies"; there is no post-construction injection path.
    //
    // Dataplane adapter composition —
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
            if let Err(source) = veth_provisioner::provision(&plan).await {
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

    // Dataplane configuration —
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
    // Service-health-check-probes — the `ProbeRunner` Earned-Trust gate is
    // owned by `run_server`; this split function accepts a caller-supplied
    // registry and intentionally does not repeat the boot gate. Test callers
    // that need the gate call `probe_runner_boot::compose_and_probe_runner_gate`
    // at their own composition boundary.

    // UI-05 — register the downstream hydrator before ServiceLifecycle's
    // authoritative backend-row publisher. ServiceLifecycle emits the
    // explicit hydrator handoff after each changed row.
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

    // R16 composes the required mTLS worker on every serve boot, after
    // `IdentityMgr` exists and before `AppState` is constructed.

    // Construct the ONE per-host `FrontendAddrAllocator` (DDN-2 single-owner;
    // dial-by-name-responder step 02-01) BEFORE the `MtlsResolve` so the SAME
    // `Arc`-shared handle feeds BOTH the re-keyed resolve (`by_frontend`) below
    // AND the `DnsResponder` (`name_index`) + `AppState` after the worker move.
    // It self-shares on clone, so `frontend_addr_allocator.clone()` shares the
    // same held map — the `F` keyed in `by_frontend` is byte-identical to the
    // `F` DNS answers. Empty on boot; the converge-on-boot rebuild (01-05)
    // re-populates it from the declared-Service intent SSOT after `AppState`.
    let frontend_addr_allocator =
        crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new();
    let guest_pool = Arc::new(guest_network::GuestAddressPool::new(
        ipnet::Ipv4Net::new_assert(std::net::Ipv4Addr::new(100, 95, 0, 0), 16),
        "ovd-gbr0".to_owned(),
        std::net::Ipv4Addr::new(100, 95, 0, 1),
        std::net::Ipv4Addr::new(100, 95, 0, 1),
    ));

    // Retain the probed resolver through the later converge-on-boot frontend
    // rebuild. Its first List necessarily sees the allocator empty; after the
    // intent-backed rebuild lands, a second idempotent probe re-Lists the same
    // authoritative backend rows against the now-populated shared allocator.
    // Without this handoff a restart can answer DNS with F while the mTLS
    // resolver permanently lacks F -> backend until an unrelated backend-row
    // write happens.
    let mtls_resolve_after_frontend_rebuild;
    let mtls_resolve_owner;
    let mtls_worker = {
        // (1) construct the enforcement port over the held identity +
        // the F7 limits. `IdentityMgr` impls `IdentityRead`.
        //
        // Focused adapter tests may substitute the whole IdentityRead
        // port. The normal integration and production paths keep this
        // unset and therefore prove the real IdentityMgr issuance and
        // lifecycle path. Production builds compile the override out.
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
                source: overdrive_core::traits::mtls_enforcement::MtlsEnforcementError::Probe {
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
            return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::Probe { source }));
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
        let service_backends_resolve =
            Arc::new(crate::mtls_resolve_adapter::ServiceBackendsResolve::new(
                Arc::clone(&obs),
                // The SAME shared allocator the DNS `name_index` answers `F`
                // from — so the `by_frontend` `F` is byte-identical to the
                // `F` DNS answers (DDN-2 single-owner). The resolve's own
                // single-owner drain projects `by_frontend` as a pure reader
                // of this allocator's snapshot (REV-3 — never `assign`).
                frontend_addr_allocator.clone(),
            ));
        let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
            service_backends_resolve.clone();
        if let Err(source) = resolve.probe().await {
            tracing::warn!(
                name: "health.startup.refused",
                reason = "mtls.resolve.probe",
                error = %source,
                "transparent-mTLS resolve probe failed; refusing to boot (no cleartext fallback)"
            );
            return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::ResolveProbe {
                source,
            }));
        }
        mtls_resolve_after_frontend_rebuild = Arc::clone(&resolve);
        mtls_resolve_owner = service_backends_resolve;

        // (4) construct the worker with all four ports as REQUIRED params
        // (mandatory `new()`, no builder). `ServerConfig` owns the intercept
        // adapter: production supplies `HostMtlsIntercept`; in-process
        // compositions can supply `SimMtlsIntercept` through the same port.
        // The worker uses it for the node-shared listeners and program as
        // well as allocation element registration.
        Arc::new(overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::new(
            enforcement,
            resolve,
            config.clock.clone(),
            config.mtls_intercept.clone(),
        ))
    };

    // microvm-driver-cloud-hypervisor step 02-02 (ADR-0083 §D7, brief.md
    // §105a.2, GH #42): `VmHostState` is composed UNCONDITIONALLY — never
    // gated on the `Vm` registry entry or the `integration-tests` feature
    // — so a node that uninstalled `cloud-hypervisor` still observes and
    // still reclaims (S-VM-30).
    // `run_dir_root` mirrors `compose_vm_driver`'s own hardcoded
    // `/run/overdrive/vm` literal (the two composition sites must agree
    // on the same VM run root).
    //
    // `index_dir` is the platform-owned clone-index directory
    // (`clone_index_dir(&config.data_dir)` = `<data_dir>/vm/clone-index/`),
    // fed the SAME expression `compose_vm_driver` feeds
    // `VmHostLayout.clone_index_dir` — the one derivation both composition
    // sites call (ADR-0083 §§D3f-D3h, DWD-26; S-VM-84 criterion 4). It
    // supersedes the prior fixed `/run/overdrive/vm-rootfs-staging`: after
    // ADR-0083 §D3a made artifacts per-allocation, a clone lands beside the
    // operator's OWN rootfs master (§D3b — FICLONE is intra-filesystem), so
    // the sweep can no longer enumerate a single node-level staging root.
    // It now enumerates the durable index of symlinks `start` records, each
    // resolving (via `read_link`) to a clone wherever the operator's master
    // lives — and because the index is under `data_dir` (never `/run`) it
    // survives a restart that loses the in-memory `RootfsPlan` (S-VM-84
    // ending 3).
    let state: AppState = AppState::new_with_workflow_engine(
        store,
        store_path,
        obs,
        runtime,
        drivers,
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
        vm_host_state,
        // dial-by-name-responder step 02-01: the ONE shared per-host
        // `FrontendAddrAllocator` (DDN-2). Cloned (self-shares the held map) so
        // the original binding survives to construct the `DnsResponder` after
        // `AppState`; `state.frontend_addr_allocator` and the responder's handle
        // are the SAME allocator, and the SAME one already injected into the
        // re-keyed `MtlsResolve` above.
        frontend_addr_allocator.clone(),
        Arc::clone(&shared_guest_network),
        guest_network_exec.gate(),
        Arc::clone(&guest_pool),
    );

    // microvm-driver-cloud-hypervisor step 02-02 (ADR-0083 §D7, brief.md
    // §105a.6/§105a.10 AC3, GH #42): the `VmReclamation` boot-epoch drive
    // runs IMMEDIATELY BEFORE the shared-switch stale sweep below, so any
    // `rmdir` it issues via `kill_scope` has settled (succeeded or
    // NotFound) before that pass reads the same cgroup tree
    // (`VmHostState::kill_scope`'s own settle postcondition; S-VM-23).
    // Deliberately OUTSIDE the `state.mtls_worker.is_some()` gate below —
    // VM allocations exist whether or not mTLS is composed, and
    // `state.vm_host_state` is composed unconditionally (never gated on
    // the `Vm` registry entry either — S-VM-30).
    tracing::info!(
        name: "guest_network.shared_owner_boot_phase",
        node_id = %state.node_id,
        phase = "vm_reclamation",
        transition = "started",
    );
    vm_reclamation_boot::converge(&state).await.map_err(|source| {
        tracing::warn!(
            name: "health.startup.refused",
            reason = "vm_reclamation.boot",
            error = %source,
            "boot-epoch VmReclamation drive failed; refusing to boot"
        );
        error::ControlPlaneError::VmReclamationBoot(source)
    })?;
    tracing::info!(
        name: "guest_network.shared_owner_boot_phase",
        node_id = %state.node_id,
        phase = "vm_reclamation",
        transition = "completed",
    );
    tracing::info!(
        name: "guest_network.shared_owner_boot_phase",
        node_id = %state.node_id,
        phase = "stale_sweep",
        transition = "started",
    );
    if let Err(cause) = shared_guest_network.sweep_stale().await {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "guest_network.sweep",
            cause = %cause,
            "shared guest-network stale sweep refused"
        );
        return Err(error::ControlPlaneError::from(cause));
    }
    tracing::info!(
        name: "guest_network.shared_owner_boot_phase",
        node_id = %state.node_id,
        phase = "stale_sweep",
        transition = "completed",
    );
    if let Err(cause) = shared_guest_network.converge_shared().await {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "guest_network.converge",
            cause = %cause,
            "shared guest-network convergence refused"
        );
        return Err(error::ControlPlaneError::from(cause));
    }

    // Publish the one node-shared F/C listener owner only after the shared
    // guest switch has converged. Allocation start_alloc requires this owner;
    // refusing here preserves the accepted boot boundary instead of allowing
    // a Running allocation to discover an absent listener owner later.
    if let Err(source) = state.mtls_worker.start_shared_owner().await {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "mtls.shared_owner",
            cause = %source,
            "shared mTLS listener owner failed to start"
        );
        return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::SharedOwner {
            source,
        }));
    }

    let (dns_responder_owner, dns_factory, dns_responder_deps) = {
        // Ordinary shared-switch stale cleanup follows the boot-epoch VM
        // reclamation drive.
        // Reclamation has already killed every unsupervised non-terminal VM and
        // committed Platform Reclamation, so this pass does not reconstruct a live
        // survivor. It adopts any still-valid supervised ownership and garbage-
        // collects the dead VM's structural netns residue before ordinary
        // reconciliation can assign that slot again. A correlation conflict still
        // refuses boot via `health.startup.refused`, reason `netns.adopt`.
        // After VM reclamation, sweep the original per-workload rules whose
        // mark-first programs remained in the kernel while their serve-owner
        // listeners disappeared. Reclamation has already made the old VM
        // terminal, so removing those dead redirects cannot reopen a live
        // cleartext workload. A later RestartAllocation carries the
        // predecessor alloc_id separately from the fresh successor spec.alloc;
        // the successor outcome completes first, then one exact-old driver →
        // mTLS → structural-network cleanup attempt addresses only alloc_id.
        // Pinned boot order: reclamation → adopt/GC → sweep → serve.
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
        // pass re-derives every `<workload> → F` binding from the declared-Service intent
        // SSOT (`.claude/rules/reconcilers.md` § "Bar 1"; the same `workloads/`
        // intent scan as `ListenerFactStore::rebuild_from_intent`). It runs AFTER
        // the shared-switch sweep above (preserving the PINNED boot order
        // adopt → GC → sweep → rebuild → serve) and BEFORE the convergence loop /
        // responder serve spawn (so the `name_index` reader the responder reads —
        // once 02-01 injects the shared instance — never observes an
        // empty-but-trusted allocator).
        //
        // GATED on `state.mtls_worker.is_some()` — the SAME composition gate the
        // shared-switch sweep above uses, and (the load-bearing reason) the SAME gate the
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
        if let Err(source) = crate::dns_responder::boot_rebuild::rebuild_frontend_addrs_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.frontend_addr_allocator,
        )
        .await
        {
            return Err(source.into());
        }
        if let Err(source) = mtls_resolve_after_frontend_rebuild.probe().await {
            tracing::warn!(
                name: "health.startup.refused",
                reason = "mtls.resolve.frontend_rebuild",
                error = %source,
                "transparent-mTLS resolve refresh after frontend rebuild failed; refusing to boot"
            );
            mtls_resolve_owner.shutdown().await;
            return Err(error::ControlPlaneError::MtlsBoot(error::MtlsBootError::ResolveProbe {
                source,
            }));
        }

        // Dial-by-name `DnsResponder` — construct + probe + spawn (DDN-6,
        // dial-by-name-responder step 02-01, ADR-0072). Built AFTER the
        // converge-on-boot frontend rebuild (so its internal `NameIndex` reads a
        // populated allocator, never empty-but-trusted) and BEFORE the listener
        // binds, inside the SAME `mtls_worker.is_some()` real-dataplane block
        // that gates the netns / intercept path. It holds:
        //   - `state.obs` — the `service_backends` List/Watch source the
        //     internal `NameIndex` reads (the third sibling reader, DDN-1);
        //   - `config.clock` — the SOA SERIAL source for `wire::encode`;
        //   - the accepted fixed node-shared gateway — the singular fallback;
        //   - `state.frontend_addr_allocator` — the SAME shared
        //     `FrontendAddrAllocator` injected into the re-keyed `MtlsResolve`
        //     above, so the `F` the `name_index` answers is byte-identical to
        //     the `F` `by_frontend` recognizes (DDN-2 single-owner).
        // Earned-Trust (wire → probe → use): `probe()` binds `:53` (wildcard
        // first, exact shared-gateway fallback) AND List-seeds the `name_index`. A
        // bind / List-seed failure REFUSES the boot fail-closed with a
        // per-variant `health.startup.refused` reason (a responder that bound
        // lazily could start and THEN fail to answer — the silent-degradation
        // footgun). Mirrors the `MtlsResolve.probe()` refuse-boot block above.
        let dns_gateway = std::net::Ipv4Addr::new(100, 95, 0, 1);
        let dns_factory = Arc::clone(&config.guest_dns);
        let dns_responder_deps = (
            Arc::clone(&state.obs),
            config.clock.clone(),
            dns_gateway,
            state.frontend_addr_allocator.clone(),
        );
        let responder = dns_factory.responder(crate::dns_responder::GuestDnsDeps {
            store: Arc::clone(&dns_responder_deps.0),
            clock: Arc::clone(&dns_responder_deps.1),
            gateway: dns_responder_deps.2,
            frontend: dns_responder_deps.3.clone(),
        });
        if let Err(source) = responder.probe().await {
            // Per-variant refusal reason — the enum owns its own vocabulary
            // (`development.md` § "Label enums own their string representation"),
            // so the mapping is tested in-process (`boot_refusal_reason`) rather
            // than as an inline match a mutation gate cannot reach through the
            // Tier-3 probe path.
            let reason = source.boot_refusal_reason();
            tracing::warn!(
                name: "health.startup.refused",
                reason,
                error = %source,
                "dial-by-name DNS responder probe failed; refusing to boot \
                 (a lazily-bound responder would start and then fail to answer)"
            );
            return Err(error::ControlPlaneError::DnsResponderBoot(source));
        }
        // Probe Ok — spawn the source-pinned serve loop and hold both the task
        // handle AND the responder (so shutdown can `stop()` the SO_RCVTIMEO-
        // bounded serve loop before aborting).
        let owned_responder = Arc::clone(&responder);
        let serve_task = tokio::spawn(Arc::clone(&responder).serve());
        let dns_responder_owner = DnsServeTaskOwner::new(owned_responder, serve_task);
        (dns_responder_owner, dns_factory, dns_responder_deps)
    };

    // Spawn the exit-observer subsystem BEFORE the convergence loop so
    // the observer is already draining the driver's `ExitEvent`
    // channel when the first action-shim write happens. The observer
    // shares `state.obs` (so its writes appear in the same row stream
    // every reader consumes) and shares `state.runtime` (so the
    // observer can re-enqueue the workload-lifecycle reconciler after
    // each obs write — closes the latency between exit classification
    // and reconciler-driven recovery). Per
    // `fix-exec-driver-exit-watcher` Step 01-02 RCA §Approved fix
    // item 5.
    // ADR-0083 §D2a(a)/(a5) (GH #42, step 01-08): one observer task PER
    // registry entry — `spawn_with_runtime`'s early-return-on-`None`
    // contract already assumes exactly one observer per driver instance,
    // and `ExitEvent` carries no driver discriminator, so merging
    // receivers cannot recover provenance. ONE shutdown token, CLONED per
    // spawn (never re-minted) — a per-driver token would retain a cancel
    // path for only the last one.
    let exit_observer_shutdown = CancellationToken::new();
    let exit_observer_tasks: Vec<tokio::task::JoinHandle<()>> = state
        .drivers
        .kinds()
        .filter_map(|kind| state.drivers.get(kind).cloned())
        .map(|driver| {
            worker::exit_observer::spawn_with_runtime(
                state.obs.clone(),
                driver,
                state.lifecycle_events.clone(),
                config.clock.clone(),
                Some(state.runtime.clone()),
                exit_observer_shutdown.clone(),
            )
        })
        .collect();

    // Spawn the convergence-tick loop per `fix-convergence-loop-not-
    // spawned` Step 01-02 (RCA Option B2 broker-driven §18 wiring).
    // Each iteration admits eligible evaluations up to the bounded owner
    // capacity and drives them concurrently. Cancellation closes admission,
    // then waits for every admitted evaluation to complete before exit.
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

    // Spawn the interest-router (ADR-0084 §5, Piece B) — the declarative
    // event-interest fan-out. Build the interest table ONCE at registration
    // from the registered reconcilers' declared `interests()`, then SUBSCRIBE
    // FIRST (a `tokio::broadcast` subscriber does not see pre-subscription
    // sends — opening the watch before the router lists closes the boot-window
    // gap, S-266-15) and spawn the List-then-Watch router. Every accepted
    // `alloc_status` write now fans out to the interested reconcilers through
    // `broker.submit` — the SAME broker the convergence loop drains.
    //
    // The three current `alloc_status` consumers declare non-empty interests:
    // WorkloadLifecycle and SvidLifecycle declare
    // `&[ObservationRowKind::AllocStatus]`; ServiceLifecycle declares
    // `&[ObservationRowKind::AllocStatus, ObservationRowKind::ProbeResult]`.
    // The production table therefore wakes these consumers on accepted writes.
    // The router is wired here regardless (vertical slice: the production
    // entry spawns the mechanism, not a test) — S-266-01.
    let interest_table = build_interest_table(state.runtime.reconcilers_iter());
    let interest_subscription = state.obs.subscribe_all_events().await.map_err(|e| {
        error::ControlPlaneError::internal("subscribe_all_events for interest router", e)
    })?;
    let interest_router_shutdown = CancellationToken::new();
    let interest_router_task = spawn_interest_router(
        state.obs.clone(),
        interest_subscription,
        interest_table,
        InterestRouterBroker::from_runtime(state.runtime.clone(), config.clock.clone()),
        // Single-clock DST preserved (ADR-0084 § Amendment 2026-08-23): the
        // router reads the SAME `config.clock` instance the convergence loop
        // reads (one clock, two readers), so seed → bit-identical trajectory
        // holds. `INTEREST_ROUTER_RELIST_PERIOD` is the production relist
        // cadence — the row-backed level-triggered backstop.
        config.clock.clone(),
        INTEREST_ROUTER_RELIST_PERIOD,
        interest_router_shutdown.clone(),
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

    // Hold the worker's task tree at the same ownership boundary as the
    // server. The router and convergence tasks also hold AppState clones, but
    // this handle is the one both shutdown modes use to invalidate and JOIN
    // every worker child before returning.
    let mtls_worker_owner = Arc::clone(&state.mtls_worker);

    let supervisor_shutdown = CancellationToken::new();
    let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
    let supervisor_exec = guest_network_exec.supervisor();
    let supervisor_guest_network = Arc::clone(&shared_guest_network);
    let supervisor_clock = config.clock.clone();
    let supervisor_worker = mtls_worker_owner.clone();
    let supervisor_task_shutdown = supervisor_shutdown.clone();
    let supervisor_task = tokio::spawn(async move {
        let mut dns_owner = dns_responder_owner;
        let mut mtls_future = Box::pin(SharedNetworkSupervisorHandle::run_mtls_owner(
            supervisor_guest_network,
            supervisor_worker,
            supervisor_exec,
            supervisor_clock,
            request_tx,
            supervisor_task_shutdown.clone(),
        ));
        loop {
            tokio::select! {
                biased;
                () = supervisor_task_shutdown.cancelled() => {
                    let result = (&mut mtls_future).await;
                    dns_owner.shutdown(Duration::from_secs(1)).await;
                    break result;
                }
                result = &mut mtls_future => {
                    dns_owner.shutdown(Duration::from_secs(1)).await;
                    break result;
                }
                _exit = dns_owner.wait_failure() => {
                    let (store, clock, gateway, frontend) = &dns_responder_deps;
                    let replacement = dns_factory.responder(crate::dns_responder::GuestDnsDeps {
                        store: Arc::clone(store),
                        clock: Arc::clone(clock),
                        gateway: *gateway,
                        frontend: frontend.clone(),
                    });
                    if let Err(source) = replacement.probe().await {
                        break Err(SharedNetworkSupervisorError::Dns(source));
                    }
                    if let Err(source) = dns_owner
                        .replace(replacement, Duration::from_secs(1), |responder| {
                            tokio::spawn(responder.serve())
                        })
                        .await
                    {
                        break Err(SharedNetworkSupervisorError::Dns(
                            crate::dns_responder::responder::DnsResponderError::Probe {
                                reason: source.to_string(),
                            },
                        ));
                    }
                }
            }
        }
    });
    let shared_network_supervisor = SharedNetworkSupervisorHandle::new(
        request_rx,
        supervisor_task,
        guest_network_exec.supervisor(),
        supervisor_shutdown,
    );

    // The retained supervisor has completed the fresh-process owner
    // composition above; only now may the dependency-neutral EXEC capability
    // transition from BootClosed to Open.  Allocation start publishes
    // Running before it reaches the existing VmDriver EXEC-release hook, so
    // leaving this gate closed would strand the convergence evaluation after
    // its durable Running write and block a later public stop evaluation on
    // the same target.
    if !guest_network_exec.supervisor().open_after_boot() {
        return Err(error::ControlPlaneError::GuestNetworkBoot(
            guest_network::GuestNetworkError::ExecGateNotBootClosed,
        ));
    }

    // Assemble the router. Step 03-03 wires the real `alloc_status` and
    // `node_list` observation-read handlers; step 03-05 aligned the
    // `cluster_status` handler signature; step 05-03 wires it onto the
    // real route (previously a `stub` placeholder).
    let router = Router::new()
        .route("/v1/workloads", post(handlers::submit_workload))
        .route("/v1/workloads/:id", get(handlers::describe_workload))
        .route("/v1/workloads/:id/stop", post(handlers::stop_workload))
        .route("/v1/workloads/:id/restart", post(handlers::restart_workload))
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
        exit_observer_tasks,
        emit_drain_task,
        interest_router_task,
        convergence_shutdown,
        exit_observer_shutdown,
        emit_drain_shutdown,
        interest_router_shutdown,
        mtls_worker_owner,
        mtls_resolve_owner,
        shared_network_supervisor,
    })
}

// ---------------------------------------------------------------------------
// [cadence-loop-region-start] — ADR-0084 §4, Piece A: the per-reconciler
// next-wake table + pure cadence decision. After this change the
// convergence loop carries NO reconciler name, NO cadence constant, and
// NO hardcoded target scheme — only this generic machinery + the core
// `resolve_scope` over a `ResyncScope` enum it understands. The
// structural test `cadence_resync::loop_names_no_reconciler_or_cadence_
// constant` (S-266-05 companion) scans this region to keep it so.
// ---------------------------------------------------------------------------

/// Build the per-reconciler cadence table (ADR-0084 §4, Piece A) from
/// the registered reconcilers: exactly one entry per reconciler whose
/// [`Reconciler::resync_schedule`](overdrive_core::reconcilers::Reconciler::resync_schedule)
/// returns `Some`. Every Phase-1 production reconciler returns the
/// default `None`, so today this table is empty and the loop resyncs
/// nothing until a reconciler opts in (SD-6: `None` ⟺ resync-only-off).
///
/// Pure over the registry snapshot — reads no clock, holds no handle.
#[must_use]
pub fn build_cadence_table<'a>(
    reconcilers: impl Iterator<Item = &'a overdrive_reconcilers::AnyReconciler>,
) -> BTreeMap<ReconcilerName, ResyncSchedule> {
    reconcilers
        .filter_map(|r| r.resync_schedule().map(|schedule| (r.name().clone(), schedule)))
        .collect()
}

/// Arm the per-reconciler next-wake table (ADR-0084 §4, Piece A): each
/// scheduled reconciler's FIRST level-triggered resync fires one period
/// after `now` (registration time — `next_wake = now + period`). The
/// returned [`BTreeMap`] is the loop-owned mutable scheduling state that
/// [`due_resync_evaluations`] advances.
///
/// Pure over `(schedules, now)`. `BTreeMap` per `development.md`
/// § "Ordered-collection choice" — deterministic iteration across seeds.
#[must_use]
pub fn arm_next_wake(
    schedules: &BTreeMap<ReconcilerName, ResyncSchedule>,
    now: overdrive_core::UnixInstant,
) -> BTreeMap<ReconcilerName, overdrive_core::UnixInstant> {
    schedules.iter().map(|(name, schedule)| (name.clone(), now + schedule.period)).collect()
}

/// The pure cadence decision (ADR-0084 §4, Piece A). For every
/// reconciler whose next-wake instant is DUE (`next_wake <= now`),
/// resolve its [`ResyncScope`](overdrive_core::reconcilers::ResyncScope)
/// to the concrete broker target(s) via the core
/// [`resolve_scope`] and re-arm its next-wake ONE period forward
/// (`next_wake += period` — anchored to the prior wake, no drift). The
/// single `if` (not a `while`) plus the anchored re-arm is **C-A2**: a
/// due reconciler fires at most once per loop iteration and its schedule
/// never drifts. The returned evaluations are what the loop then routes
/// through `broker.submit` (**C-A1** — every resync coalesces through the
/// broker's LWW key-collapse, never a side channel).
///
/// Reads no clock and holds no handle: referentially transparent over
/// `(schedules, next_wake, now, node_id)`. The loop owns the clock
/// (`SimClock` under DST) and the local [`NodeId`]; the reconciler names
/// no target string. A schedule with no matching `next_wake` entry is
/// skipped (the two tables are armed together, so this cannot happen in
/// the loop — it keeps the function total).
#[must_use]
pub fn due_resync_evaluations(
    schedules: &BTreeMap<ReconcilerName, ResyncSchedule>,
    next_wake: &mut BTreeMap<ReconcilerName, overdrive_core::UnixInstant>,
    now: overdrive_core::UnixInstant,
    node_id: &NodeId,
) -> Vec<Evaluation> {
    let mut submits = Vec::new();
    for (name, schedule) in schedules {
        let Some(wake) = next_wake.get_mut(name) else {
            continue;
        };
        if *wake <= now {
            for target in resolve_scope(schedule.scope, node_id) {
                submits.push(Evaluation { reconciler: name.clone(), target });
            }
            // C-A2: re-arm anchored to the prior wake (`+= period`), so
            // the schedule never drifts and fires at most once per period.
            *wake = *wake + schedule.period;
        }
    }
    submits
}

/// Spawn the broker-driven convergence-tick loop.
///
/// Per `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2 §18
/// wiring), each iteration admits eligible evaluations from the
/// `EvaluationBroker` up to the fixed owner capacity and drives them as
/// concurrently-owned `run_convergence_tick` futures. Cancellation closes
/// admission and the owner drains every admitted evaluation before exit.
///
/// Piece A (ADR-0084 §4) adds a per-reconciler cadence phase ahead of the
/// drain: at registration the loop builds a next-wake table from every
/// reconciler that opted into a resync schedule; each iteration submits a
/// resync for every due reconciler through `broker.submit` (C-A1) and
/// re-arms it one period out (C-A2). Every Phase-1 reconciler returns the
/// default `None` EXCEPT `vm-reclamation`, which declares a 30s
/// `LocalNode` schedule — its host-backed, resync-only trigger (ADR-0084
/// §4, unifying the former hardcoded vm-reclamation sweep). So the cadence
/// phase drives `vm-reclamation` and is inert for the rest.
///
/// Without this spawn, `submit_workload` and `stop_workload` would only
/// write to the `IntentStore` — the broker would never be drained, no
/// allocations would ever be scheduled, and
/// `cluster_status.broker.dispatched` would permanently read 0. See
/// `docs/feature/fix-convergence-loop-not-spawned/bugfix-rca.md` for the
/// full root-cause chain.
///
/// The cadence sleep goes through the injected `Clock`: production
/// (`SystemClock`) parks on a real timer; DST (`SimClock`) parks until
/// the harness calls `sim_clock.tick(cadence)` to advance logical time
/// past the deadline. Either way the loop suspends between ticks
/// rather than busy-polling.
///
/// `vm-reclamation` (ADR-0083 §D7, brief.md §105a.8, GH #42) is the sole
/// Phase-1 cadence opt-in: its `resync_schedule` (30s / `LocalNode`,
/// ADR-0084 §4) makes the cadence phase submit one local-node
/// `vm-reclamation` `Evaluation` per period through the generic hook. The
/// broker is otherwise purely event-driven and nothing else would submit a
/// `vm-reclamation` evaluation, so S-VM-21's "a later steady-state tick,
/// WITHOUT restarting serve" claim now rides on that declaration rather
/// than a hardcoded sweep in this loop.
const CONVERGENCE_MAX_IN_FLIGHT: usize = 8;

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[allow(clippy::too_many_lines)]
fn spawn_convergence_loop(
    state: AppState,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    cadence: Duration,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Piece A (ADR-0084 §4). At registration build the per-reconciler
        // cadence table from every reconciler that opted into a resync
        // schedule, and arm its next-wake one period out. Today exactly one
        // reconciler opts in — `vm-reclamation` (a 30s `LocalNode`
        // schedule, its host-backed resync-only trigger); every other
        // reconciler returns the default `None`. The loop itself names NO
        // reconciler and bakes NO cadence constant: it carries only this
        // generic next-wake table + the core `resolve_scope`, and the
        // vm-reclamation cadence lives entirely in the reconciler's
        // `resync_schedule` declaration.
        let cadence_table = build_cadence_table(state.runtime.reconcilers_iter());
        let mut next_wake =
            arm_next_wake(&cadence_table, overdrive_core::UnixInstant::from_clock(&*clock));

        let mut tick_n: u64 = 0;
        let mut admission_closed = false;
        let mut admitted_at_close = 0usize;
        let mut completed_during_drain = 0usize;
        let mut drain_started = None;
        let mut active_targets = BTreeSet::new();
        let mut active = FuturesUnordered::new();

        loop {
            let now = clock.now();
            let now_unix = overdrive_core::UnixInstant::from_clock(&*clock);

            let capacity = CONVERGENCE_MAX_IN_FLIGHT.saturating_sub(active.len());
            let mut sleep_for = cadence;
            if !admission_closed && capacity > 0 {
                let pending = {
                    let mut broker = state.runtime.broker();
                    for eval in due_resync_evaluations(
                        &cadence_table,
                        &mut next_wake,
                        now_unix,
                        &state.node_id,
                    ) {
                        broker.submit(eval, now, EvaluationEligibility::Immediate);
                    }
                    broker.drain_pending(capacity, &active_targets, now, now_unix)
                };

                for (eval, queued_for) in pending {
                    let tick = tick_n;
                    tick_n = tick_n.saturating_add(1);
                    let deadline = now + cadence;
                    active_targets.insert(eval.target.clone());
                    let active_count = active.len().saturating_add(1);
                    tracing::info!(
                        name: "convergence.evaluation.admitted",
                        reconciler = %eval.reconciler,
                        target = %eval.target.as_str(),
                        tick,
                        queue_ms = duration_millis(queued_for),
                        active = active_count,
                        capacity = CONVERGENCE_MAX_IN_FLIGHT,
                    );
                    let state_for_eval = state.clone();
                    let eval_for_result = eval.clone();
                    active.push(async move {
                        let started = state_for_eval.clock.now();
                        let result = run_convergence_tick(
                            &state_for_eval,
                            &eval_for_result.reconciler,
                            &eval_for_result.target,
                            now,
                            tick,
                            deadline,
                        )
                        .await;
                        (eval_for_result, tick, started, result)
                    });
                }

                if active.len() < CONVERGENCE_MAX_IN_FLIGHT
                    && let Some(next_eligible_at) =
                        state.runtime.broker().next_eligible_at(&active_targets)
                {
                    let until_eligible = next_eligible_at
                        .as_unix_duration()
                        .saturating_sub(now_unix.as_unix_duration());
                    sleep_for = cadence.min(until_eligible);
                }
            }

            if admission_closed && active.is_empty() {
                let pending_at_exit = state.runtime.broker().counters().queued;
                let elapsed_ms = drain_started.map_or(0, |started: Instant| {
                    duration_millis(clock.now().saturating_duration_since(started))
                });
                tracing::info!(
                    name: "convergence.drain.completed",
                    elapsed_ms,
                    admitted_at_close,
                    completed_during_drain,
                    pending_at_exit,
                );
                break;
            }

            tokio::select! {
                biased;
                () = shutdown.cancelled(), if !admission_closed => {
                    admission_closed = true;
                    admitted_at_close = active.len();
                    completed_during_drain = 0;
                    drain_started = Some(clock.now());
                }
                Some((eval, tick, started, result)) = active.next(), if !active.is_empty() => {
                    active_targets.remove(&eval.target);
                    let elapsed_ms = duration_millis(clock.now().saturating_duration_since(started));
                    let outcome = if result.is_ok() { "ok" } else { "error" };
                    if admission_closed {
                        completed_during_drain = completed_during_drain.saturating_add(1);
                    }
                    let error = result.as_ref().err();
                    tracing::info!(
                        name: "convergence.evaluation.completed",
                        reconciler = %eval.reconciler,
                        target = %eval.target.as_str(),
                        tick,
                        elapsed_ms,
                        outcome,
                        error = tracing::field::debug(&error),
                    );
                }
                () = clock.sleep(sleep_for), if !admission_closed => {}
            }
        }
    })
}

// [cadence-loop-region-end]

// ---------------------------------------------------------------------------
// [interest-router-region-start] — ADR-0084 §5, Piece B: the declarative
// event-interest fan-out. A List-then-Watch task (modelled on
// `ServiceBackendsResolve`'s resolve index) subscribes to the observation
// change-feed, lists the interested snapshot families, and — for every
// accepted `alloc_status` change — submits an `Evaluation` per interested
// reconciler through a restricted broker handle. RN-2 = B-2: interests-only,
// NO warm cache (hydration stays per-tick, ADR-0036).
// ---------------------------------------------------------------------------

/// Production `relist_period` for the interest router's unconditional periodic
/// relist (ADR-0084 § Amendment 2026-08-23). A compile-time const — NOT a
/// config field / operator-tunable knob (that is a separate, approval-gated
/// decision). Mirrors the vm-reclamation sweep interval's role for the
/// host-backed partition; 30 s aligns with that sweep but is a deliberately
/// SEPARATE const (the row-backed relist and the host-backed cadence are
/// independent concerns even at the same value). Per-period cost =
/// O(interested targets) coalesced `broker.submit`s once per 30 s.
const INTEREST_ROUTER_RELIST_PERIOD: Duration = Duration::from_secs(30);

/// The restricted broker capability handed to the interest router
/// (ADR-0084 §5 — "injected capability is a restricted `EvaluationBroker`
/// handle (not a god-object)"). Its ENTIRE effect universe is
/// [`submit`](Self::submit): it cannot drain, reap, read counters, drive the
/// driver, write the store, or dispatch actions. Cloneable (`Arc` bump) so
/// the spawned router task owns its own handle.
///
/// Two constructors, one per composition site:
/// - [`from_runtime`](Self::from_runtime) — production: submit into the
///   runtime's shared broker, the SAME broker the convergence loop drains, so
///   a fan-out wake reaches dispatch. The runtime owns its broker inline
///   (`parking_lot::Mutex<EvaluationBroker>`), so the closure captures an
///   `Arc<ReconcilerRuntime>`; the router body still sees a submit-only
///   surface.
/// - [`from_shared_broker`](Self::from_shared_broker) — a standalone broker
///   any caller (a DST test driving `spawn_interest_router` directly) owns and
///   inspects, keeping the router-behaviour tests in the default lane while
///   driving the SAME router code path.
#[derive(Clone)]
pub struct InterestRouterBroker {
    submit: Arc<dyn Fn(Evaluation) + Send + Sync>,
}

impl InterestRouterBroker {
    /// Production capability: submit into the runtime's shared broker — the
    /// SAME broker the convergence loop drains, so a fan-out wake reaches
    /// dispatch (C-A1's fan-out sibling). The closure captures the runtime
    /// `Arc`; the router body still sees only [`submit`](Self::submit).
    #[must_use]
    pub fn from_runtime(
        runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            submit: Arc::new(move |eval| {
                runtime.broker().submit(eval, clock.now(), EvaluationEligibility::Immediate);
            }),
        }
    }

    /// Standalone-broker capability (DST tests + any caller holding its own
    /// broker): submit into the shared `EvaluationBroker` the caller owns and
    /// inspects.
    #[must_use]
    pub fn from_shared_broker(
        broker: Arc<parking_lot::Mutex<EvaluationBroker>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            submit: Arc::new(move |eval| {
                broker.lock().submit(eval, clock.now(), EvaluationEligibility::Immediate);
            }),
        }
    }

    /// Submit one evaluation — the router's whole effect universe.
    fn submit(&self, eval: Evaluation) {
        (self.submit)(eval);
    }
}

/// Route one observation row through the interest table (ADR-0084 §5): if any
/// reconciler declared interest in `row.kind()`, derive the broker target
/// INLINE from the row and submit one [`Evaluation`] per interested
/// reconciler. The watcher delivers a `Row` only for an ACCEPTED write (LWW
/// winner), so the fan-out fires on genuine changes only.
///
/// The inline `AllocStatus(row) → workload/<row.workload_id>` derivation is
/// the sole target-derivation site (the dropped `TargetFrom`/`derive_target`
/// indirection) — the mutation surface #3. Total: any row kind with no
/// interested reconcilers short-circuits. A `ProbeResult` event first
/// point-reads the current allocation row and routes only a current Service
/// allocation; the probe event itself carries no workload target and is not
/// persisted in generic row history.
async fn route_observation_row(
    obs: &Arc<dyn ObservationStore>,
    row: &ObservationRow,
    interest_table: &BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: &InterestRouterBroker,
) {
    let Some(reconcilers) = interest_table.get(&row.kind()).cloned() else {
        return;
    };
    let target = match row {
        ObservationRow::AllocStatus(r) => {
            match TargetResource::new(&format!("workload/{}", r.workload_id)) {
                Ok(target) => target,
                // A malformed target is dropped rather than panicking the
                // router task — an `AllocStatusRow` carries a validated
                // `WorkloadId`, so this arm is unreachable in practice.
                Err(_) => return,
            }
        }
        ObservationRow::ProbeResult(probe_row) => {
            let current = match obs.alloc_status_row(&probe_row.alloc_id).await {
                Ok(Some(current)) => current,
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!(
                        target: "overdrive::interest_router",
                        alloc_id = %probe_row.alloc_id,
                        ?error,
                        "interest-router probe-result point read failed; ignoring event",
                    );
                    return;
                }
            };
            if current.kind != overdrive_core::aggregate::WorkloadKind::Service {
                return;
            }
            match TargetResource::new(&format!("workload/{}", current.workload_id)) {
                Ok(target) => target,
                Err(_) => return,
            }
        }
        // Every other row kind has no Phase-1 consumer. Kept total (no
        // panic) should a caller provide an explicitly populated table.
        _ => return,
    };
    for reconciler in &reconcilers {
        broker.submit(Evaluation { reconciler: reconciler.clone(), target: target.clone() });
    }
}

/// The LIST leg of List-then-Watch (and the `Lagged`- / periodic-relist
/// recovery): read the interested `alloc_status` snapshot and route each row.
/// A snapshot read failure is logged and skipped — it is **not terminal**
/// because the unconditional periodic relist (Amendment 2026-08-23) retries
/// within `relist_period`; the router never faults on a transient store-read
/// error (there is no derived view to leave stale under B-2).
async fn list_and_route(
    obs: &Arc<dyn ObservationStore>,
    interest_table: &BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: &InterestRouterBroker,
) {
    match obs.alloc_status_rows().await {
        Ok(rows) => {
            for row in rows {
                route_observation_row(
                    obs,
                    &ObservationRow::AllocStatus(Box::new(row)),
                    interest_table,
                    broker,
                )
                .await;
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "overdrive::interest_router",
                ?e,
                "interest-router list/relist of alloc_status_rows failed; skipping this pass",
            );
        }
    }
}

/// Build the interest table (ADR-0084 §5) once at registration: invert every
/// reconciler's declared [`interests()`](AnyReconciler::interests) into a
/// `BTreeMap<ObservationRowKind, Vec<ReconcilerName>>`. A reconciler with the
/// default empty interests contributes nothing (SD-6: empty interests ⟺
/// host-backed ⟺ never event-woken). `BTreeMap` per `development.md`
/// § "Ordered-collection choice" — deterministic iteration across seeds.
///
/// Pure over the registry snapshot — reads no clock, holds no handle.
#[must_use]
pub fn build_interest_table<'a>(
    reconcilers: impl Iterator<Item = &'a AnyReconciler>,
) -> BTreeMap<ObservationRowKind, Vec<ReconcilerName>> {
    let mut table: BTreeMap<ObservationRowKind, Vec<ReconcilerName>> = BTreeMap::new();
    for reconciler in reconcilers {
        for kind in reconciler.interests() {
            table.entry(*kind).or_default().push(reconciler.name().clone());
        }
    }
    table
}

/// Spawn the interest-router task (ADR-0084 §5, Piece B) — List-then-Watch.
///
/// The caller MUST open `subscription` via
/// [`ObservationStore::subscribe_all_events`] BEFORE calling this (subscribe
/// FIRST, then this task lists): a `tokio::broadcast` subscriber never
/// observes sends that predate its subscription, so opening the watch before
/// the list closes the boot-window gap (S-266-15).
///
/// The task: (2) LISTs the interested snapshot family
/// (`alloc_status_rows()`) and submits an `Evaluation` per row's derived
/// target; (3) WATCHes the subscription — on `Row(row)` it routes by
/// `row.kind()`, on `Lagged` it RELISTs, and on every `relist_period` it
/// **unconditionally RELISTs** off the injected `clock` (Amendment
/// 2026-08-23). That periodic relist is the level-triggered backstop for the
/// row-backed partition (ADR-0084 SD-6): it closes the quiet-stream
/// boot-LIST-error liveness gap — on a `serve` restart `register` submits no
/// initial evaluation, so the boot LIST is the only boot-time wake, and if it
/// errors transiently on a quiet stream nothing else recovers until the next
/// relist tick (≤ `relist_period`). It also re-delivers any silently-missed
/// per-target edge each period, and subsumes the `Lagged`-relist as a special
/// case. The router's only effect is `broker.submit`; `clock` is a read
/// capability, not an effect. Shutdown is cooperative via `shutdown` (a bare
/// `abort()` on a parked task would not interrupt it —
/// `.claude/rules/development.md` § "Concurrency & async").
pub fn spawn_interest_router(
    obs: Arc<dyn ObservationStore>,
    mut subscription: LagAwareSubscription,
    interest_table: BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: InterestRouterBroker,
    clock: Arc<dyn Clock>,
    relist_period: Duration,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    use futures::StreamExt as _;
    tokio::spawn(async move {
        // (2) LIST — the caller opened `subscription` (step 1) BEFORE this
        // list, so no accepted write in the subscribe→list boot window is
        // missed (a write concurrent with boot is delivered on the watch,
        // coalesced with the list submit at the same broker key).
        list_and_route(&obs, &interest_table, &broker).await;

        // Arm the unconditional periodic relist (Amendment 2026-08-23) one
        // period out from the boot LIST. `next_relist_at` is an absolute
        // deadline on the injected `Clock` (`SimClock` under DST) — never
        // `tokio::time` — so the level-triggered backstop is DST-controllable
        // and single-clock-deterministic.
        let mut next_relist_at = clock.now() + relist_period;

        // (3) WATCH + periodic relist. Three-way `select!` racing shutdown, the
        // relist deadline, and the next stream item — `biased` so shutdown
        // wins ties. Shutdown is cooperative: `select!` resolves
        // `shutdown.cancelled()` even while parked (a bare `abort()` would not
        // interrupt a parked recv — `.claude/rules/development.md`
        // § "Concurrency & async").
        loop {
            // Sleep only the REMAINING time to the absolute deadline. Because
            // `next_relist_at` is fixed, re-deriving `remaining` each iteration
            // targets the SAME instant regardless of how many `Row` arrivals
            // re-enter the loop — so a `Row` never drifts the deadline. If the
            // deadline has already passed, `remaining` saturates to zero and the
            // relist arm fires immediately on the next poll.
            let remaining = next_relist_at.saturating_duration_since(clock.now());
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                () = clock.sleep(remaining) => {
                    // Unconditional periodic relist (ADR-0084 § Amendment
                    // 2026-08-23) — the level-triggered backstop for the
                    // row-backed partition: re-read the interested snapshot and
                    // re-route, so a transient boot-LIST error (log-and-skip) or
                    // a silently-missed per-target edge self-heals within one
                    // period. Re-arm the deadline a full period out — re-armed
                    // AT MOST once per period (the router analogue of Piece A's
                    // C-A2). The `Lagged`-relist below is a special case of this
                    // same relist path.
                    list_and_route(&obs, &interest_table, &broker).await;
                    next_relist_at = clock.now() + relist_period;
                }
                item = subscription.next() => match item {
                    Some(SubscriptionEvent::Row(row)) => {
                        // Edge wake — `next_relist_at` is UNCHANGED (a `Row`
                        // arrival does NOT reset the period: unconditional-
                        // periodic, not idle-debounce).
                        route_observation_row(&obs, &row, &interest_table, &broker).await;
                    }
                    Some(SubscriptionEvent::Lagged { .. }) => {
                        // Honour the mandatory `Lagged` contract
                        // (`observation_store.rs`): re-read the interested
                        // snapshot and re-route, so a dropped row's target is
                        // still woken (no warm cache to rebuild under B-2). A
                        // special case of the periodic relist above;
                        // `next_relist_at` is left UNCHANGED (the periodic
                        // cadence is independent of stream activity).
                        list_and_route(&obs, &interest_table, &broker).await;
                    }
                    // The broadcast sender was dropped — the watch closed. No
                    // further rows will arrive; exit cleanly.
                    None => break,
                },
            }
        }
    })
}

// [interest-router-region-end]

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
pub fn noop_heartbeat() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::{AnyReconciler, NoopHeartbeat};

    AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical())
}

/// Construct the `workload-lifecycle` reconciler.
///
/// The first real (non-proof-of-life) reconciler. Converges declared
/// replica count for a `Job` against the running `AllocStatusRow`
/// set, calling first-fit placement via
/// `overdrive_core::scheduler::schedule`.
///
/// Per US-03 (Slice 3 of phase-1-first-workload), this is registered
/// at boot alongside `noop-heartbeat`.
#[must_use]
pub fn workload_lifecycle() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::{AnyReconciler, WorkloadLifecycle};

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
pub fn workflow_lifecycle() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::{AnyReconciler, WorkflowLifecycle};

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
pub fn svid_lifecycle() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::{AnyReconciler, SvidLifecycle};

    AnyReconciler::SvidLifecycle(SvidLifecycle::canonical())
}

/// Construct the `service-lifecycle` reconciler per ADR-0055.
///
/// The Phase 1 Service-kind workload reconciler — converges
/// `Stable` / `StartupProbeFailed` / `EarlyExit` terminal conditions
/// against the running `AllocStatusRow` set + probe-result rows.
/// Registered at production boot alongside `noop-heartbeat` /
/// `workload-lifecycle` / `service-map-hydrator`.
///
/// Per service-health-check-probes step 01-03d this completes the
/// composition-root registration arc: the reconciler-core
/// (`ServiceLifecycleReconciler::reconcile`) landed in commit
/// `2fabf259` (step 01-03), the `AnyReconciler::ServiceLifecycle`
/// dispatch enum landed in commit `087bada4` (step 01-03b), and
/// this factory is the runtime-facing constructor invoked by
/// `run_server_with_obs_and_driver`.
#[must_use]
pub fn service_lifecycle() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::AnyReconciler;
    use overdrive_reconcilers::service_lifecycle::ServiceLifecycleReconciler;

    AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new())
}

/// Construct the `vm-reclamation` reconciler per ADR-0083 §D7 /
/// `brief.md` §105a.1.
///
/// SD-1's Bar-2 registered reconciler (`.claude/rules/reconcilers.md`):
/// converges `desired` (VM-driver allocations) against `actual` (the
/// three `VmHostState::observe()` surfaces + the supervision set),
/// emitting `Action::ReclaimAllocation` / `Action::DiscardStrandedArtifacts`.
/// Carries no per-node state of its own — `VmReclamation::new()` takes no
/// arguments; the node it observes comes from the `TargetResource`
/// (`node/<node_id>`) the runtime evaluates it against.
#[must_use]
pub fn vm_reclamation() -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::AnyReconciler;
    use overdrive_reconcilers::vm_reclamation::VmReclamation;

    AnyReconciler::VmReclamation(VmReclamation::new())
}

/// Construct the `service-map-hydrator` reconciler.
///
/// Activates J-PLAT-004 per ADR-0042 — converges
/// `service_hydration_results` rows by dispatching
/// `Action::DataplaneUpdateService` whenever a service's published
/// `(vip, backends)` fingerprint drifts from the last
/// confirmed-applied fingerprint persisted in the hydrator's `View`.
///
/// Registered at production boot BEFORE `service-lifecycle`
/// (the publisher re-enqueues this reconciler per the explicit
/// cross-reconciler handoff — see `Action::EnqueueEvaluation`). Order matters only
/// for `cluster_status`'s deterministic registration listing; the
/// runtime registers idempotently regardless of order.
#[must_use]
pub fn service_map_hydrator(host_ipv4: std::net::Ipv4Addr) -> overdrive_reconcilers::AnyReconciler {
    use overdrive_reconcilers::{AnyReconciler, ServiceMapHydrator};

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

    /// D1 review remediation (step 01-09, GH #42): proves `compose_vm_driver`
    /// wires each of its fallible steps into its OWN [`error::VmmBootError`]
    /// variant rather than a flattened `String` — a `matches!()` on the
    /// specific variant, not a `Display` substring, per
    /// `.claude/rules/development.md` § "Never flatten a typed error to
    /// `Internal(String)` at a composition boundary".
    ///
    /// The CLI-level walking-skeleton scenarios (S-VM-13, S-VM-75 in
    /// `overdrive-cli/tests/integration/vm_walking_skeleton.rs`) observe
    /// this same failure ONLY through
    /// `overdrive_cli::http_client::CliError::Transport`, whose
    /// `cause: String` field is a SEPARATE, pre-existing
    /// composition-boundary flattening at the CLI layer (`run_inner`'s
    /// `run_server(..).await.map_err(|e| CliError::Transport { cause:
    /// stripped_server_error(&e.to_string()), .. })` — out of this step's
    /// declared scope, and unconditional for every `ControlPlaneError`
    /// variant, not specific to `VmmBoot`). A `matches!()` is therefore
    /// unreachable from those integration tests without touching
    /// `overdrive-cli` production code; the structural proof lives here,
    /// at the layer where the typed enum is actually observable.
    #[cfg(feature = "integration-tests")]
    mod vm_compose_error_typing {
        use std::path::PathBuf;
        use std::sync::Arc;

        use overdrive_sim::adapters::clock::SimClock;
        use overdrive_sim::adapters::observation_store::SimObservationStore;
        use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
        use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm, SimVmmProbeFault};
        use overdrive_worker::probe_runner::ProbeRunner;

        use crate::error::VmmBootError;
        use crate::{VmComposeError, compose_vm_driver};

        #[allow(clippy::expect_used)]
        fn test_probe_runner() -> Arc<ProbeRunner> {
            Arc::new(ProbeRunner::new(
                Arc::new(SimTcpProber::new()),
                Arc::new(SimHttpProber::new()),
                Arc::new(SimClock::new()),
                Arc::new(SimObservationStore::single_peer(
                    overdrive_core::id::NodeId::new("vm-compose-errors").expect("valid node ID"),
                    0,
                )),
            ))
        }

        #[tokio::test]
        async fn injected_vmm_probe_failure_is_refused_with_typed_probe_variant() {
            let sim_vmm = SimVmm::new();
            sim_vmm.inject_probe_failure(SimVmmProbeFault::LandlockLsmAbsent);

            // `.err()` rather than binding the raw `Result`: the `Ok`
            // side is `VmDriver`, which does not implement `Debug`
            // (per `overdrive_worker::vm_driver::VmDriver`), so
            // formatting the whole `Result` on assertion failure would
            // not compile. Neither of these tests reaches `Ok`.
            let err = compose_vm_driver(
                PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
                overdrive_core::vm::config::clone_index_dir(&PathBuf::from("/srv/overdrive/data")),
                overdrive_core::vm::config::clone_staging_dir(&PathBuf::from(
                    "/srv/overdrive/data",
                )),
                Arc::new(SimClock::new()),
                Arc::new(SimCgroupFs::new()),
                Arc::new(SimCgroupAccounting::new()),
                test_probe_runner(),
                overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
                    SimClock::new(),
                ))
                .gate(),
                Some(Arc::new(sim_vmm)),
            )
            .await
            .err();

            assert!(
                matches!(
                    err,
                    Some(VmComposeError::Refused(VmmBootError::Probe {
                        source: overdrive_core::traits::vmm::VmmProbeError::LandlockLsmAbsent { .. }
                    }))
                ),
                "expected Refused(VmmBootError::Probe{{LandlockLsmAbsent}}), got {err:?}"
            );
        }

        /// `VmComposeError::NotAvailable` and `::Refused` share the SAME
        /// typed `VmmBootError` payload — the soft/hard split lives
        /// entirely in which wrapper holds the cause, never in a second,
        /// differently-shaped copy of it (enforced by the type
        /// declaration itself: both variants are declared as
        /// `(error::VmmBootError)`). Constructed directly — no adapter,
        /// no `.await` — because `compose_vm_driver`'s `injected` flag
        /// is DEFINED as `vmm_override.is_some()`: there is no way to
        /// reach `NotAvailable` through the function itself with a
        /// CONTROLLED fault. `vmm_override = None` always constructs the
        /// REAL, un-injectable `overdrive_host::CloudHypervisorVmm`
        /// (confirmed empirically: on the Lima dev box this genuinely
        /// fails probe with `ReflinkUnsupported` on `/srv/vm`, a real
        /// substrate fact of THIS host — exactly the kind of
        /// environment-dependent outcome a deterministic unit test must
        /// not assert a specific variant of).
        #[test]
        fn not_available_wraps_the_same_typed_boot_error_as_refused() {
            let cause = VmmBootError::Probe {
                source: overdrive_core::traits::vmm::VmmProbeError::landlock_lsm_absent(
                    "unit-test-constructed",
                ),
            };

            assert!(
                matches!(
                    VmComposeError::NotAvailable(cause),
                    VmComposeError::NotAvailable(VmmBootError::Probe {
                        source: overdrive_core::traits::vmm::VmmProbeError::LandlockLsmAbsent { .. }
                    })
                ),
                "VmComposeError::NotAvailable must carry the typed VmmBootError variant unchanged"
            );
        }

        /// The second fallible step — `CgroupAccounting`'s own probe —
        /// gets its OWN distinct variant, never collapsed into `Probe`.
        #[tokio::test]
        async fn injected_cgroup_accounting_probe_failure_is_refused_with_typed_variant() {
            // No fault injected on `sim_vmm` — its `.probe()` succeeds,
            // so control reaches `cgroup_accounting.probe()`.
            let sim_vmm = SimVmm::new();
            let sim_cgroup_accounting = SimCgroupAccounting::new();
            sim_cgroup_accounting
                .inject_probe_substrate_error(std::io::ErrorKind::PermissionDenied);

            let err = compose_vm_driver(
                PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
                overdrive_core::vm::config::clone_index_dir(&PathBuf::from("/srv/overdrive/data")),
                overdrive_core::vm::config::clone_staging_dir(&PathBuf::from(
                    "/srv/overdrive/data",
                )),
                Arc::new(SimClock::new()),
                Arc::new(SimCgroupFs::new()),
                Arc::new(sim_cgroup_accounting),
                test_probe_runner(),
                overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
                    SimClock::new(),
                ))
                .gate(),
                Some(Arc::new(sim_vmm)),
            )
            .await
            .err();

            assert!(
                matches!(
                    err,
                    Some(VmComposeError::Refused(VmmBootError::CgroupAccountingProbe {
                        source: overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::Substrate { .. }
                    }))
                ),
                "expected Refused(VmmBootError::CgroupAccountingProbe{{Substrate}}), got {err:?}"
            );
        }
    }
}
