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
// netns-density-295 D-UP-16 — the one construction of the host guest-flow
// owner and the mTLS worker, each holding the other.
pub(crate) mod guest_network_composition;
pub(crate) use guest_network_composition::{GuestNetworkCompositionPorts, compose_guest_network};
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
pub mod node_adapters;
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

pub use node_adapters::{
    BootAdapters, CaAdapters, ComposedDataplane, DataplaneComposer, DataplanePlan,
    GuestFlowAdapter, HostDataplaneComposer, IntentStoreAdapter, MtlsAdapters, NodeAdapters,
    ProbeAdapters, StoreAdapters, VmAdapters, VmCapability,
};

// GH #295 (DISTILL gap B-4): crate-private test-local implementations of the
// three ports this crate declares, for the source-local supervisor and
// admission cells (the `overdrive-sim` doubles implement a second compiled
// copy of these traits here).
#[cfg(test)]
mod shared_network_test_ports;

// GH #295 QHOLD (ADR-0169, ADR-0184, ADR-0124): the source-local Quint
// conformance driver of `specs/quint/guest-flow-owner/quiescence.qnt`
// (QPROJ-1: the owner with the runtime shared-network supervisor) and the
// recovery acceptance bodies P6-S1..S6 / C2. Both boot the composition through
// `run_server_with_obs_and_drivers` (real HTTPS bind, real redb), so they run
// in the `integration-tests` lane.
#[cfg(all(test, feature = "integration-tests"))]
mod quint_quiescence;
#[cfg(all(test, feature = "integration-tests"))]
mod shared_network_recovery_acceptance;

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
use overdrive_core::guest_network::SharedGuestNetworkComponent;
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
    /// Required on every boot: mTLS composition is unconditional. The boot
    /// builds the worker after `IdentityMgr` over the intercept and
    /// enforcement of the composition root's `NodeAdapters` (`BootAdapters::
    /// mtls`, `MtlsAdapters`) and the dataplane its `DataplaneComposer`
    /// composed (NO-OVERRIDE-SEAMS, ADR-0219): host adapters under `overdrive
    /// serve`, sim adapters under a simulated test. The action-shim fires it
    /// on every allocation transition.
    pub mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    /// The one EXEC gate shared with the VM driver and supervisor.
    #[allow(
        dead_code,
        reason = "threaded into the state now; action helpers consume it in DELIVER 12-01"
    )]
    pub(crate) guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
    /// The one node-wide guest address pool used by dispatch and hydration.
    pub(crate) guest_pool: Arc<guest_network::GuestAddressPool>,
    /// Read-port projection backed by the same per-server pool used at dispatch.
    pub(crate) guest_attachment_view: guest_network::GuestPoolAttachmentView,
    /// PORTS-CONFIG: the guest-flow owner's bound stream and control
    /// listener ports, read once by the boot after step 9; the action shim
    /// passes them to `GuestAddressPool::assign` (GUEST-PORTS).
    pub(crate) guest_flow_ports:
        Arc<std::sync::OnceLock<overdrive_core::guest_transport::GuestFlowPorts>>,
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
        // The caller supplies the transparent-mTLS worker (required; the
        // fixture builds it over sim adapters). The production boot path
        // (`run_server`) uses [`Self::new_with_workflow_engine`] with the
        // worker composed over the `NodeAdapters` the composition root
        // passed.
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
            // (ripple-free, as the `NoopVmHostState` above). The
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
            guest_flow_ports: Arc::new(std::sync::OnceLock::new()),
            guest_attachment_view: guest_network::GuestPoolAttachmentView::new(Arc::clone(
                &guest_pool,
            )),
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
/// startup from CLI flags and environment. Configuration only
/// (NO-OVERRIDE-SEAMS, ADR-0219): no field carries or swaps a driven adapter
/// and no field injects a fault — every driven adapter comes from the
/// composition root in [`NodeAdapters`].
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
    /// Cadence between drains of the [`overdrive_core::eval_broker::EvaluationBroker`]
    /// in the convergence-loop spawn (see
    /// [`run_server_with_obs_and_driver`]). Default
    /// [`reconciler_runtime::DEFAULT_TICK_CADENCE`] (100ms) per
    /// ADR-0023. Tests inject a slower cadence with a [`SimClock`] to
    /// step through the loop deterministically.
    ///
    /// [`SimClock`]: overdrive_core::traits::clock::Clock
    pub tick_cadence: Duration,
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
    /// `run_server` refuses to start when this
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
    /// The node-global host vsock listener ports (PORTS-CONFIG, ADR-0218):
    /// the beacon, the guest-flow stream and the guest-flow control
    /// listener. Validated at construction (`NodeVsockPortConfig::new`), so
    /// the boot has no port-validation refusal. `ServerConfig::new()` sets
    /// the production default (1234 / 1240 / 1243); tests set
    /// `NodeVsockPortConfig::kernel_assigned()` and read the bound ports back
    /// ([`ServerHandle::vsock_ports`]).
    pub vsock_ports: overdrive_core::guest_transport::NodeVsockPortConfig,
}

impl ServerConfig {
    /// A `ServerConfig` with every field at its default: `bind`,
    /// `data_dir` and `operator_config_dir` get sentinel values callers
    /// MUST override; `tick_cadence` is
    /// [`reconciler_runtime::DEFAULT_TICK_CADENCE`] (100ms); the
    /// `[dataplane]` section is the veth-named single-node shape
    /// (ADR-0061 § 1); the pin directory and cgroup attach path are the
    /// production defaults; `vsock_ports` is
    /// `NodeVsockPortConfig::default()`. No `Default` impl: the rest-pattern
    /// `..ServerConfig::new()` is the construction shape.
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "ADR-0219: `ServerConfig` stays without `Default`; `..ServerConfig::new()` is the rest-pattern"
    )]
    pub fn new() -> Self {
        // 127.0.0.1:0 — IPv4 loopback, ephemeral port. Constructed
        // directly rather than via `parse()` so the constructor
        // is infallible and clippy's `expect_used` lint stays clean.
        let loopback = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
        Self {
            bind: loopback,
            data_dir: PathBuf::new(),
            operator_config_dir: PathBuf::new(),
            tick_cadence: DEFAULT_TICK_CADENCE,
            node: overdrive_worker::NodeConfig::default(),
            vip_range: VipRange::default(),
            // ADR-0061 § 1 (step 01-03): the veth-named single-node
            // `[dataplane]` shape (`ovd-veth-cli` / `ovd-veth-bk`).
            dataplane: Some(dataplane_config::DataplaneConfig::single_node_veth()),
            // `None` means the production default (`/sys/fs/bpf/overdrive`).
            dataplane_pin_dir: None,
            // ADR-0053 § 7: `None` means the production default
            // (`/sys/fs/cgroup/overdrive.slice`).
            dataplane_cgroup_attach_path: None,
            vsock_ports: overdrive_core::guest_transport::NodeVsockPortConfig::default(),
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
    /// PORTS-CONFIG: the guest-flow owner's bound listener ports, read once
    /// after boot step 9.
    flow_ports: Option<overdrive_core::guest_transport::GuestFlowPorts>,
    /// PORTS-CONFIG: the beacon intake's bound port; `None` when no `Vm`
    /// driver was composed or on the supplied-owner path.
    beacon_port: Option<overdrive_core::vm::config::VsockPort>,
}

/// The node-global host vsock listener ports a running server bound
/// (PORTS-CONFIG, ADR-0218), reported by [`ServerHandle::vsock_ports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundVsockPorts {
    /// `None` when `run_server` composed no `Vm` driver (`Optional`
    /// capability absent) or on the supplied-owner path.
    pub beacon: Option<overdrive_core::vm::config::VsockPort>,
    /// The guest-flow owner's stream and control listener ports.
    pub flow: overdrive_core::guest_transport::GuestFlowPorts,
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
#[cfg_attr(
    not(all(test, feature = "integration-tests")),
    allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")
)]
const SHARED_NETWORK_AUDIT_PERIOD: Duration = Duration::from_secs(1);

/// Period between two bounded recovery attempts while EXEC is Recovering
/// (ADR-0124, D-295-R13).
#[cfg_attr(
    not(all(test, feature = "integration-tests")),
    allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")
)]
const SHARED_NETWORK_RETRY_PERIOD: Duration = Duration::from_millis(250);

/// Recovery window from detection to the typed fail-stop (ADR-0124,
/// D-295-R13).
#[cfg_attr(
    not(all(test, feature = "integration-tests")),
    allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")
)]
const SHARED_NETWORK_RECOVERY_DEADLINE: Duration = Duration::from_secs(5);

/// Completed recovery attempts after which the supervisor fail-stops
/// (ADR-0124, D-295-R13).
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
const SHARED_NETWORK_RECOVERY_ATTEMPTS: u32 = 20;

/// Bound on each owner call during detection; a call that misses it fails
/// with its owner's first component and cause `audit_timeout` (D-295-R13).
/// Rule: `max(1 s, 4 × L)`, L the largest single owner-call audit latency at
/// T1-PORT4 (M-ND295-E18). It holds the rule's floor until the measurement
/// sets it; the step that sets it records the measurement here.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
const SHARED_NETWORK_AUDIT_CALL_BOUND: Duration = Duration::from_secs(1);

/// Bound on each recovery's `quiesce()` call, itself capped by the
/// remaining recovery deadline; a call that misses it leaves the unconfirmed
/// set undetermined (D-295-R14). Rule: `max(1 s, 4 × Q)`, Q the largest
/// quiescence wall time at T1-PORT4 (M-ND295-E18). It holds the rule's floor
/// until the measurement sets it; the step that sets it records the
/// measurement here.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
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
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
const SHARED_NETWORK_VM_KILL_CALL_BOUND: Duration = Duration::from_secs(1);

/// The ports the one runtime shared-network supervisor task owns (D-295-R13,
/// R14, R16). They replace `run_mtls_owner`'s parameters in DELIVER step
/// 12-01.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
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
        #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
        cgroups: super::CgroupManager,
    }

    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
    impl VmKillCapability {
        /// Wrap the manager over the same cgroup root and `CgroupFs` every
        /// composed `VmDriver` writes through.
        pub(super) const fn new(cgroups: super::CgroupManager) -> Self {
            Self { cgroups }
        }

        /// Write `1` to one allocation's scope `cgroup.kill`. An absent scope is
        /// `Ok`: the VMM can no longer execute.
        #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 12-01")]
        #[expect(
            clippy::unused_async,
            reason = "RED scaffold — DELIVER step 12-01 awaits the kill write"
        )]
        pub(super) async fn kill_allocation(
            &self,
            alloc: &super::AllocationId,
        ) -> std::io::Result<()> {
            let _ = alloc;
            todo!("RED scaffold: D-295-R14 VmKillCapability::kill_allocation — DELIVER step 12-01")
        }

        /// Write `1` to the workloads slice's `cgroup.kill`, killing every
        /// workload VMM.
        #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 12-01")]
        #[expect(
            clippy::unused_async,
            reason = "RED scaffold — DELIVER step 12-01 awaits the kill write"
        )]
        pub(super) async fn kill_workloads_slice(&self) -> std::io::Result<()> {
            todo!(
                "RED scaffold: D-295-R14 VmKillCapability::kill_workloads_slice — DELIVER step 12-01"
            )
        }
    }
}

/// QPROJ-6: one component's recovery episode at the runtime shared-network
/// supervisor (RUN-295-B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryEpisode {
    pub phase: RecoveryPhase,
    /// Failed attempts of this recovery: a failed repair, or a failed
    /// post-repair audit (D-295-R13, UP-7). Equals the `attempts` the EXEC
    /// gate's `recovery_progress()` reports for the same component.
    pub failed_attempts: u32,
}

/// QPROJ-6: where a component's recovery episode stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPhase {
    /// Quiesced under the recovery's `QuiescenceHold`; repair not yet clean.
    Held,
    /// The repair returned `Ok`; the post-repair audit is pending.
    Repaired,
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

    /// QPROJ-6: an owned copy of the supervisor's per-component episodes.
    ///
    /// - **Preconditions:** none. Infallible; callable at any time on a
    ///   spawned supervisor.
    /// - **Postconditions:** an owned copy of the supervisor's per-component
    ///   episodes, taken under the same gate lock that linearizes
    ///   `recovery_progress()`; no lock is held on return; nothing is
    ///   quiesced, repaired, audited or restored by the call.
    /// - **Consistency point:** between two supervisor steps (one quiesce,
    ///   one repair, one audit, one restore, one `complete_attempt`, one
    ///   fail-stop).
    /// - **Edge cases:** a component with no recovery in progress is absent;
    ///   the map is empty while the gate is Open, BootClosed or FailStop.
    /// - **Observable invariants:** the key set equals the set of components
    ///   `recovery_progress()` reports (the EXEC gate contract); at a
    ///   consistency point of both the supervisor and the owner, the number
    ///   of episodes plus the owner's
    ///   `flow_observation().leaked_quiescence_holds` equals its
    ///   `quiescence_holds` (QHOLD `CountCoversRecoveries`, D-UP-3: no
    ///   recovery drops its hold, so a dropped hold is not an episode); a
    ///   recovery's
    ///   `failed_attempts` equals the `attempts` of its `recovery_progress()`
    ///   entry; a repeated call with no intervening supervisor step returns
    ///   an equal value.
    ///
    /// RED scaffold: DELIVER lands it with the supervisor rework (12-01
    /// rework, inside the attachment cut), which keeps each recovery's
    /// episode beside the EXEC gate's per-recovery snapshots.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "RED scaffold — read by the quiescence conformance driver and the recovery acceptance bodies"
        )
    )]
    #[expect(
        clippy::unused_self,
        reason = "RED scaffold body panics; DELIVER 12-01 (09-01 rework) lands the episodes"
    )]
    pub fn recovery_observation(&self) -> BTreeMap<SharedGuestNetworkComponent, RecoveryEpisode> {
        panic!("Not yet implemented -- RED scaffold")
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
    /// in DELIVER step 12-01.
    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 12-01")]
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 12-01")]
    #[expect(
        clippy::unused_async,
        reason = "RED scaffold — DELIVER step 12-01 awaits the owner calls"
    )]
    async fn run_shared_network_supervisor(
        ports: SharedNetworkSupervisorPorts,
        exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
        clock: Arc<dyn Clock>,
        request_tx: tokio::sync::mpsc::Sender<overdrive_core::guest_network::ServeShutdownRequest>,
        shutdown: CancellationToken,
    ) -> std::result::Result<(), SharedNetworkSupervisorError> {
        let _ = (ports, exec, clock, request_tx, shutdown);
        todo!("RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 12-01")
    }

    fn listener_task_exit_leg(
        error: &overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError,
    ) -> Option<overdrive_worker::mtls_intercept::InterceptLeg> {
        match error {
            overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskReturned { leg }
            | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskFailed {
                leg,
                ..
            }
            | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskPanicked { leg }
            | overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError::TaskCancelled {
                leg,
            } => Some(*leg),
            _ => None,
        }
    }

    /// Consume the task owner's terminal event after its non-mutating audit
    /// reported the same dead listener task. The worker's exact-port repair
    /// replaces only a consumed terminal slot; leaving the event queued makes
    /// the completed task observer occupy that slot and turns a successful
    /// rebind into `TaskObserverClosed`.
    async fn consume_listener_task_exit(
        mtls_worker: &overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker,
        expected_leg: overdrive_worker::mtls_intercept::InterceptLeg,
        shutdown: &CancellationToken,
    ) -> std::result::Result<bool, overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError>
    {
        loop {
            let error = tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(false),
                error = mtls_worker.wait_shared_owner_failure() => error,
            };
            match Self::listener_task_exit_leg(&error) {
                Some(leg) if leg == expected_leg => return Ok(true),
                Some(_) => {}
                None => return Err(error),
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "RUN-295-B keeps the cadence/deadline state machine in one exact private owner future"
    )]
    async fn run_mtls_owner(
        shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
        mtls_worker: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
        exec: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
        clock: Arc<dyn Clock>,
        request_tx: tokio::sync::mpsc::Sender<overdrive_core::guest_network::ServeShutdownRequest>,
        shutdown: CancellationToken,
    ) -> std::result::Result<(), SharedNetworkSupervisorError> {
        let requires_quiescence =
            |component: overdrive_core::guest_network::SharedGuestNetworkComponent| {
                !matches!(
                    component,
                    overdrive_core::guest_network::SharedGuestNetworkComponent::LegF
                        | overdrive_core::guest_network::SharedGuestNetworkComponent::LegC
                        | overdrive_core::guest_network::SharedGuestNetworkComponent::Dns
                )
            };
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(()),
                () = clock.sleep(Duration::from_secs(1)) => {}
            }

            let (component, listener_task_exit) = match mtls_worker.audit_shared_owner().await {
                Ok(()) => continue,
                Err(source) => (source.component(), Self::listener_task_exit_leg(&source)),
            };
            if !exec.begin_recovery(component) {
                return Ok(());
            }
            tracing::warn!(
                name: "guest_network.shared_owner_unhealthy",
                component = ?component,
                "shared guest-network owner entered bounded recovery"
            );
            if let Some(leg) = listener_task_exit {
                match Self::consume_listener_task_exit(&mtls_worker, leg, &shutdown).await {
                    Ok(true) => {}
                    Ok(false) => return Ok(()),
                    Err(source) => return Err(source.into()),
                }
            }
            // QHOLD (ADR-0169): this recovery's counted, move-only hold. Kept
            // for the life of the recovery and consumed by `restore` after the
            // clean post-repair audit; dropped unrestored it raises the
            // owner's leaked-hold count and forwarding stays quiesced.
            let mut quiesced: Option<guest_network::QuiescenceHold> =
                if requires_quiescence(component) {
                    let (hold, _quiescence) = shared_guest_network.quiesce().await;
                    Some(hold)
                } else {
                    None
                };

            loop {
                let retry_deadline = clock.now() + Duration::from_millis(250);
                let mut supervisor_failure = None;
                loop {
                    let remaining = retry_deadline.saturating_duration_since(clock.now());
                    if remaining.is_zero() {
                        break;
                    }
                    tokio::select! {
                        biased;
                        () = shutdown.cancelled() => return Ok(()),
                        source = mtls_worker.wait_shared_owner_failure() => {
                            if Self::listener_task_exit_leg(&source).is_none() {
                                supervisor_failure = Some(source);
                                break;
                            }
                        }
                        () = clock.sleep(remaining) => break,
                    }
                }

                let first_remaining = if let Some(source) = supervisor_failure {
                    Some((source.component(), None))
                } else {
                    match mtls_worker.converge_shared_owner().await {
                        Ok(()) => match mtls_worker.audit_shared_owner().await {
                            Ok(()) => None,
                            Err(source) => {
                                Some((source.component(), Self::listener_task_exit_leg(&source)))
                            }
                        },
                        Err(source) => {
                            Some((source.component(), Self::listener_task_exit_leg(&source)))
                        }
                    }
                };
                if first_remaining.is_none() {
                    if exec.complete_attempt(BTreeSet::new()) {
                        if let Some(hold) = quiesced.take() {
                            match shared_guest_network.restore(hold).await {
                                Ok(restored) => tracing::info!(
                                    name: "guest_network.shared_owner_restored",
                                    component = ?component,
                                    restored = ?restored,
                                    "shared guest-network recovery restored its quiescence hold"
                                ),
                                // D-UP-4 / PD-1: a hold another ledger issued
                                // changes nothing at the owner; the
                                // composition is defective, so `serve`
                                // fail-stops at once (ADR-0124).
                                Err(error) => {
                                    tracing::error!(
                                        name: "guest_network.shared_owner_fail_stop",
                                        component = ?component,
                                        error = error.as_str(),
                                        "shared guest-network recovery restored a foreign quiescence hold"
                                    );
                                    let Some(request) = exec.fail_stop(
                                        overdrive_core::guest_network::SharedGuestNetworkFailStopCause::ForeignQuiescenceHold,
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
                        tracing::info!(
                            name: "guest_network.shared_owner_recovered",
                            component = ?component,
                            "shared guest-network owner recovered before the deadline"
                        );
                        break;
                    }
                    return Ok(());
                }
                let Some((component, listener_task_exit)) = first_remaining else {
                    unreachable!("a failed recovery attempt always has a remaining component");
                };
                if let Some(leg) = listener_task_exit {
                    match Self::consume_listener_task_exit(&mtls_worker, leg, &shutdown).await {
                        Ok(true) => {}
                        Ok(false) => return Ok(()),
                        Err(source) => return Err(source.into()),
                    }
                }
                if !exec.complete_attempt(BTreeSet::from([component])) {
                    return Ok(());
                }
                if quiesced.is_none() && requires_quiescence(component) {
                    let (hold, _quiescence) = shared_guest_network.quiesce().await;
                    quiesced = Some(hold);
                }
                let Some(progress) = exec.recovery_progress().into_iter().next() else {
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

// GH #295 — the source-local supervisor task-owner and DNS task-owner bodies
// that survive the vsock attachment replacement (S-ND295-32 exit matrix,
// S-ND295-34 DNS task owner). The bridge / TAP / drain-era cells of this
// module (per-TAP quiescence, per-VM kill scope over TAP sets, the R18
// guard, the TCX / endpoint / counter-map / bpffs-pin components) were
// deleted with the mechanism they defended; the QHOLD recovery cells live in
// `shared_network_recovery_acceptance` and the Quint `quiescence` driver.
#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    reason = "acceptance bodies use exact Contract Shape markers and fail at once on a broken precondition"
)]
mod shared_network_task_owner_acceptance {
    //! # Universe (port-exposed)
    //!
    //! The supervisor handle's typed `ServeShutdownRequest` and the EXEC
    //! gate's `fail_stop` / `recovery_progress`; the DNS task owner's state,
    //! its retained task and responder, the `GuestDns` port's serve/stop
    //! outcomes through the test-local `TestGuestDns` doubles
    //! (`shared_network_test_ports`). No production state is seeded.

    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use overdrive_core::guest_network::{
        GuestNetworkExecSupervisor, ServeShutdownRequest, SharedGuestNetworkFailStop,
        SharedGuestNetworkFailStopCause,
    };
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use super::*;
    use crate::dns_responder::{GuestDns, GuestDnsDeps, GuestDnsFactory};
    use crate::shared_network_test_ports::{
        TestGuestDns, TestGuestDnsFactory, TestGuestDnsServeExit,
    };

    #[derive(Clone, Copy)]
    enum SupervisorExitCase {
        Returned,
        Failed,
        Panicked,
        Cancelled,
        RequestChannelClosed,
    }

    /// The shared guest-network components (§ *Typed error taxonomy*), in
    /// their audit order.
    const COMPONENTS: [SharedGuestNetworkComponent; 9] = [
        SharedGuestNetworkComponent::Forwarder,
        SharedGuestNetworkComponent::FlowListeners,
        SharedGuestNetworkComponent::ControlListener,
        SharedGuestNetworkComponent::LegF,
        SharedGuestNetworkComponent::LegC,
        SharedGuestNetworkComponent::IpRules,
        SharedGuestNetworkComponent::IpSets,
        SharedGuestNetworkComponent::Dns,
        SharedGuestNetworkComponent::Supervisor,
    ];

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

    /// A spawned supervisor task ending as `case`, its handle, and the EXEC
    /// supervisor it fail-stops; with `recovering`, the gate is in a
    /// recovery of that component that has completed two failed attempts
    /// over 1.25 s of the injected clock.
    fn supervisor_handle(
        case: SupervisorExitCase,
        recovering: Option<SharedGuestNetworkComponent>,
    ) -> (SharedNetworkSupervisorHandle, Arc<GuestNetworkExecSupervisor>) {
        let clock = Arc::new(SimClock::new());
        let wiring = GuestNetworkExecWiring::new(clock.clone());
        let exec = wiring.supervisor();
        assert!(exec.open_after_boot());
        if let Some(component) = recovering {
            assert!(exec.begin_recovery(component));
            clock.tick(Duration::from_millis(250));
            assert!(exec.complete_attempt(BTreeSet::from([component])));
            clock.tick(Duration::from_millis(250));
            assert!(exec.complete_attempt(BTreeSet::from([component])));
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
                            operation: guest_network::GuestNetworkOperation::SharedAudit,
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
            component: SharedGuestNetworkComponent::IpRules,
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
    /// The running node's guest-flow owner's `flow_observation()` (D-15:
    /// R5-57, and the R5-41 / R5-54 / R5-63 cells through `serve`). `None`
    /// when the boot took the supplied-owner path
    /// (`run_server_with_obs_and_drivers`), whose owner exposes only
    /// `SharedGuestNetworkOwner`. Reads only; changes nothing.
    ///
    /// Reached without a CLI hook (D-17): the through-`serve` cells run in
    /// this crate's integration lane under the `kvm-tests` feature on native
    /// metal, which calls `run_server(config, adapters)` with
    /// `NodeAdapters::host(&config, cgroup_fs, kek)` and a hermetic `SimKek`,
    /// holds the returned `ServerHandle`, and deploys through the server's
    /// public API.
    #[cfg(feature = "integration-tests")]
    #[expect(
        clippy::unused_async,
        reason = "RED scaffold body panics; the attachment-cut step lands the observation"
    )]
    pub async fn guest_flow_observation(&self) -> Option<guest_network::GuestOwnerObservation> {
        // SCAFFOLD: true — netns-density-295 DISTILL (D-17; the attachment-cut
        // step holds the composed owner on the handle).
        panic!("Not yet implemented -- RED scaffold")
    }

    /// PORTS-CONFIG (ADR-0218): the ports this server's node-global vsock
    /// listeners bound — constant for the process. A test that configured
    /// `NodeVsockPortConfig::kernel_assigned()` reads them here and never
    /// assumes a number.
    #[must_use]
    pub fn vsock_ports(&self) -> BoundVsockPorts {
        // SCAFFOLD: true — netns-density-295 DISTILL (attachment-cut step 6):
        // the host owner binds its listeners and reports their ports there.
        let Some(flow) = self.flow_ports else { panic!("Not yet implemented -- RED scaffold") };
        BoundVsockPorts { beacon: self.beacon_port, flow }
    }

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
            flow_ports: _,
            beacon_port: _,
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

/// Start the control-plane server over the driven adapters the composition
/// root built (NO-OVERRIDE-SEAMS, STORES-IN; ADR-0219). `overdrive serve`
/// passes `NodeAdapters::host(..)`; a test passes the host bundle with the
/// fields it simulates replaced by name. Neither this function nor the
/// `boot_server` it delegates to constructs a driven adapter.
///
/// # Errors
///
/// The typed boot refusals of § *Composition* (cgroup preflight and
/// bootstrap, the guest-flow startup probe, the probe-runner gate, a
/// `Required` VM capability's probe refusal, and every refusal of
/// `boot_server`).
pub async fn run_server(
    config: ServerConfig,
    adapters: NodeAdapters,
) -> Result<ServerHandle, error::ControlPlaneError> {
    let NodeAdapters { boot, cgroup_fs: fs, probers, vm, guest_flow } = adapters;

    // Per ADR-0028 (as superseded in part by ADR-0034), run the cgroup
    // v2 delegation pre-flight at the start of the boot path — BEFORE
    // any on-disk side effects. Without this ordering, a misconfigured host
    // sees `WorkloadsBootstrap(WriteFailed: PermissionDenied)` instead of
    // the actionable `CgroupBootstrap(DelegationMissing)` message.
    cgroup_preflight::run_preflight().map_err(error::ControlPlaneError::from)?;

    // The parent slice delegates controllers BEFORE the workloads-slice
    // bootstrap below enables them (cgroup v2 kernel contract); the order is
    // load-bearing.
    cgroup_manager::create_and_enrol_control_plane_slice()
        .map_err(error::ControlPlaneError::from)?;

    // Delegate `+cpu +memory +io +pids` to the workloads slice BEFORE the
    // convergence loop accepts any allocation (fix-cgroup-subtree-control-
    // delegation, production fix #2).
    let cgroup_root_path = std::path::PathBuf::from(cgroup_preflight::DEFAULT_CGROUP_ROOT);
    let bootstrap_manager =
        overdrive_worker::cgroup_manager::CgroupManager::new(cgroup_root_path.clone(), fs.clone());
    bootstrap_manager
        .create_workloads_slice_with_controllers()
        .await
        .map_err(error::ControlPlaneError::from)?;

    // Boot step 1, phase 1 of the guest-flow owner's two-phase boot (D-8):
    // the owner's startup checks and the forwarder probe run over the
    // composition root's seam before the owner exists and before any boot
    // effect below. The owner is composed over this same seam at step 5.
    let GuestFlowAdapter(guest_flow_io) = guest_flow;
    probe_guest_flow_boot(guest_flow_io.as_ref(), &config.vip_range).await?;

    // ADR-0054 § 7 — the probe-runner Earned-Trust gate, over the root's
    // probers and the boot's one clock.
    let clock = Arc::clone(&boot.clock);
    // The boot's one `OwnerDial` (D-13): the VM probe runner dials guest
    // ports through it (DG-2 row 19) and the inbound enforcement dials leg S
    // through it (LEGS-DIAL). The owner is composed at boot step 5
    // (`compose_guest_network`, D-UP-16), which binds it.
    let owner_dial = Arc::new(guest_network_composition::OwnerDial::unbound());
    let guest_dial: Arc<dyn overdrive_core::guest_transport::dial::GuestStreamDial> =
        Arc::clone(&owner_dial) as _;
    let probe_runner = probe_runner_boot::compose_and_probe_runner_gate(
        probers.tcp,
        probers.http,
        Arc::clone(&clock),
        Arc::clone(&boot.observation),
        guest_dial,
    )
    .await?;
    let guest_network_exec = GuestNetworkExecWiring::new(Arc::clone(&clock));

    let mut registry = DriverRegistry::new();

    // ADR-0082/ADR-0083 (§D3c, § D8) and ADR-0219: discover -> probe ->
    // insert the `Vm` driver over the root's VM ports. The probes run
    // UNCONDITIONALLY; `vm.capability` decides what a failed probe means:
    // `Optional` — the node boots with no `Vm` entry and `[vm]` deploys fail
    // at dispatch (`driver.vm.not_composed`); `Required` — the boot refuses
    // (`health.startup.refused`, reason `vmm.probe`).
    let mut beacon_port = None;
    match compose_vm_driver(
        cgroup_root_path,
        overdrive_core::vm::config::clone_index_dir(&config.data_dir),
        overdrive_core::vm::config::clone_staging_dir(&config.data_dir),
        Arc::clone(&clock),
        fs,
        Arc::clone(&probe_runner),
        guest_network_exec.gate(),
        vm,
        config.vsock_ports.beacon(),
    )
    .await
    {
        Ok((vm_driver, bound)) => {
            beacon_port = Some(bound);
            registry.insert(Arc::new(vm_driver));
        }
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

    let mut handle = boot_server(
        config,
        boot,
        Arc::new(registry),
        GuestNetworkSource::Composed { io: guest_flow_io, dial: owner_dial },
        guest_network_exec,
        bootstrap_manager,
    )
    .await?;
    handle.beacon_port = beacon_port;
    Ok(handle)
}

/// Where a boot's guest-flow owner comes from.
enum GuestNetworkSource {
    /// The caller's owner ([`run_server_with_obs_and_drivers`]): the owner's
    /// driven port for owner-level doubles. The inbound enforcement takes it
    /// as its `guest_dial` (LEGS-DIAL); D-UP-16's postcondition is not
    /// claimed on this path.
    Supplied(Arc<dyn guest_network::SharedGuestNetworkOwner>),
    /// [`run_server`] and the in-crate composed rig: `io` is the seam phase 1
    /// checked; the owner and the mTLS worker are composed together at boot
    /// step 5 by [`compose_guest_network`] over `io` (D-UP-16, D-8), which
    /// binds `dial` — the boot's one `OwnerDial`, the VM probe runner's and
    /// the enforcement's guest dial — to the owner.
    Composed {
        io: Arc<dyn guest_network::flow_io::GuestFlowIo>,
        dial: Arc<guest_network_composition::OwnerDial>,
    },
}

/// Boot step 1, phase 1 (D-8): [`guest_network::probe_guest_flow_startup`]
/// over the boot's seam, before the owner exists and before any boot effect.
/// Refusal: `health.startup.refused`, reason `guest_network.probe`.
async fn probe_guest_flow_boot(
    io: &dyn guest_network::flow_io::GuestFlowIo,
    vip_range: &VipRange,
) -> Result<(), error::ControlPlaneError> {
    if let Err(cause) = guest_network::probe_guest_flow_startup(
        io,
        vip_range,
        overdrive_core::guest_transport::GUEST_NODE_PREFIX,
    )
    .await
    {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "guest_network.probe",
            cause = %cause,
            "shared guest-network startup probe refused"
        );
        return Err(error::ControlPlaneError::from(cause));
    }
    Ok(())
}

/// Boot step 1 on the supplied-owner path (§ *Composition*, "Entry points
/// and the supplied owner"): the supplied owner's startup probe, first in the
/// boot. Refusal: `health.startup.refused`, reason `guest_network.probe`.
async fn probe_guest_network_startup(
    owner: &dyn guest_network::SharedGuestNetworkOwner,
) -> Result<(), error::ControlPlaneError> {
    if let Err(cause) = owner.probe_startup().await {
        tracing::error!(
            name: "health.startup.refused",
            target: "overdrive::health",
            reason = "guest_network.probe",
            cause = %cause,
            "shared guest-network startup probe refused"
        );
        return Err(error::ControlPlaneError::from(cause));
    }
    Ok(())
}

/// Outcome of a failed [`compose_vm_driver`] attempt — distinguishes
/// capability ABSENCE (soft: the node simply has no `Vm` entry) from a
/// genuine substrate lie caught by a probe against an adapter the
/// composition root declared present with `VmCapability::Required` (hard:
/// refuses the whole boot). ADR-0083 §D8, ADR-0219.
#[derive(Debug)]
enum VmComposeError {
    /// The composition root passed `VmCapability::Optional` (as
    /// `overdrive serve` does), and a probe failed. Capability absence is
    /// not a fault
    /// (SD-5) — the caller logs `driver.vm.not_composed` and continues
    /// booting with no `Vm` entry. Carries the typed
    /// [`error::VmmBootError`] cause (not a pre-flattened `String`) per
    /// `.claude/rules/development.md` § "Never flatten a typed error to
    /// `Internal(String)` at a composition boundary" — the
    /// `driver.vm.not_composed` log line renders it via `Display`
    /// (`%cause`), and a test can `matches!()` on the variant.
    NotAvailable(error::VmmBootError),
    /// The composition root passed `VmCapability::Required` — it declares
    /// "cloud-hypervisor IS present" — and the VMM's probe (or
    /// `CgroupAccounting`'s probe, or the kernel-header read/validate)
    /// failed. Unambiguously a genuine substrate lie, never absence: the
    /// caller refuses the whole boot with `health.startup.refused`,
    /// converting directly into `ControlPlaneError::VmmBoot` — the SAME
    /// typed [`error::VmmBootError`] value, no re-stringification at the
    /// boundary.
    Refused(error::VmmBootError),
}

/// Compose the production `VmDriver` over the composition root's
/// `VmAdapters` (`CloudHypervisorVmm` and `VsockBeaconIo` from
/// `NodeAdapters::host`, or the doubles a test passes) and run the VMM's
/// Earned-Trust `.probe()` (and
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
    probe_runner: Arc<overdrive_worker::probe_runner::ProbeRunner>,
    guest_network_exec: Arc<overdrive_core::guest_network::GuestNetworkExecGate>,
    vm: VmAdapters,
    beacon_port: overdrive_core::guest_transport::VsockListenPort,
) -> std::result::Result<
    (overdrive_worker::vm_driver::VmDriver, overdrive_core::vm::config::VsockPort),
    VmComposeError,
> {
    use overdrive_core::vm::config::{HostArch, VmConfinement, VmmIdentity};
    use overdrive_worker::vm_driver::VmHostLayout;

    // ADR-0219: the composition root states what a failed probe means.
    // `injected` keeps its name for the classification below: `Required`
    // (the root constructed the VMM and declares the capability present) →
    // `Refused`; `Optional` (capability may be absent) → `NotAvailable`.
    let injected = vm.capability == VmCapability::Required;
    let VmAdapters { vmm, beacon: beacon_io, cgroup_accounting, capability: _ } = vm;
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

    // DG-4 boot step 2: after `Vmm::probe`, bind the node beacon intake on
    // the configured port (`BeaconIntake::bind`) and pass it to
    // `VmDriver::new`.
    #[allow(clippy::match_wild_err_arm, reason = "RED scaffold arm until the attachment-cut step")]
    let beacon = match overdrive_worker::BeaconIntake::bind(beacon_io, beacon_port).await {
        Ok(beacon) => beacon,
        // SCAFFOLD: true — netns-density-295 DISTILL (attachment-cut step 6):
        // the `vm_driver.beacon_bind` startup refusal is new behaviour, and no
        // `VmmBootError` / `ControlPlaneError` variant carrying
        // `BeaconIntakeError` is pinned (DG-4) — surfaced as a DISTILL blocker.
        Err(_source) => panic!("Not yet implemented -- RED scaffold"),
    };
    let bound = beacon.bound_port();

    Ok((
        overdrive_worker::vm_driver::VmDriver::new(
            vmm,
            clock,
            fs,
            cgroup_accounting,
            probe_runner,
            guest_network_exec,
            layout,
            beacon,
        ),
        bound,
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
    boot: BootAdapters,
    driver: Arc<dyn Driver>,
    shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    guest_network_exec: GuestNetworkExecWiring,
    vm_cgroups: CgroupManager,
) -> Result<ServerHandle, error::ControlPlaneError> {
    let mut registry = DriverRegistry::new();
    registry.insert(driver);
    run_server_with_obs_and_drivers(
        config,
        boot,
        Arc::new(registry),
        shared_guest_network,
        guest_network_exec,
        vm_cgroups,
    )
    .await
}

/// Start the control-plane server over a caller-supplied boot-wide adapter
/// group, driver registry and guest-flow owner (the supplied-owner path,
/// § *Composition*, "Entry points and the supplied owner").
///
/// `boot` replaces the former `obs` and `vm_host_state` parameters and
/// carries the stores and the CA (STORES-IN), so the boot opens and
/// constructs nothing. Preconditions: `drivers` and `guest_network_exec`
/// were built over `boot.clock` (the boot's one clock), and `boot.stores` /
/// `boot.observation` / `boot.vm_host_state` are over `config.data_dir` or
/// simulations. The inbound enforcement takes `shared_guest_network` as its
/// `GuestStreamDial` (LEGS-DIAL).
pub async fn run_server_with_obs_and_drivers(
    config: ServerConfig,
    boot: BootAdapters,
    drivers: Arc<DriverRegistry>,
    shared_guest_network: Arc<dyn guest_network::SharedGuestNetworkOwner>,
    guest_network_exec: GuestNetworkExecWiring,
    vm_cgroups: CgroupManager,
) -> Result<ServerHandle, error::ControlPlaneError> {
    boot_server(
        config,
        boot,
        drivers,
        GuestNetworkSource::Supplied(shared_guest_network),
        guest_network_exec,
        vm_cgroups,
    )
    .await
}

/// The boot sequence of § *Composition* over `guest_network`'s owner.
#[allow(clippy::too_many_lines)]
async fn boot_server(
    config: ServerConfig,
    boot: BootAdapters,
    drivers: Arc<DriverRegistry>,
    guest_network: GuestNetworkSource,
    guest_network_exec: GuestNetworkExecWiring,
    vm_cgroups: CgroupManager,
) -> Result<ServerHandle, error::ControlPlaneError> {
    // NO-OVERRIDE-SEAMS / STORES-IN (ADR-0219): every driven adapter of the
    // boot comes from the composition root; this function constructs none.
    let BootAdapters {
        clock,
        entropy,
        transport,
        kek,
        observation: obs,
        vm_host_state,
        stores,
        ca: ca_adapters,
        dataplane: dataplane_composer,
        mtls: mtls_adapters,
        guest_dns,
    } = boot;
    // RED scaffold (D-295-R14): consumed in DELIVER step 12-01, which wraps it
    // in the supervisor's kill-only `VmKillCapability`.
    let _ = vm_cgroups;
    // Boot step 1 for a supplied owner (D-8, "Entry points and the supplied
    // owner"). A composed owner's step 1 is phase 1, which its caller ran over
    // the seam before this function (`run_server`, the composed rig).
    if let GuestNetworkSource::Supplied(owner) = &guest_network {
        probe_guest_network_startup(owner.as_ref()).await?;
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
    overdrive_worker::start_local_node(&obs, &config.node, &clock)
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

    // The authoritative intent store the composition root opened
    // (STORES-IN); its path travels with it, so remediations name the file
    // that was opened.
    let store = Arc::clone(stores.intent.store());
    let store_path = stores.intent.path().to_path_buf();

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
    let view_store: Arc<dyn view_store::ViewStore> = Arc::clone(&stores.views);
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

    // The boot's one clock is `BootAdapters.clock`, threaded into
    // AppState so the streaming submit handler's cap timer uses the same
    // clock as the convergence-loop spawn. It is required at construction
    // per `.claude/rules/development.md` § "Port-trait dependencies"; there
    // is no post-construction injection path.
    //
    // Dataplane composition — the composition root's `DataplaneComposer`
    // (`BootAdapters.dataplane`) provisions, constructs and probes the
    // dataplane: `HostDataplaneComposer` (the real `EbpfDataplane`) from
    // `NodeAdapters::host`, or the simulation composer a test passes when
    // its subject is not the dataplane attach path. No branch here keys on
    // which composer it is (ADR-0219). Failure paths route through
    // `DataplaneBootError` per architecture.md § 5.3.
    let dataplane_cfg =
        config.dataplane.as_ref().ok_or_else(|| error::ControlPlaneError::Validation {
            message: "missing required [dataplane] section in overdrive.toml \
                      (client_iface + backend_iface)"
                .to_owned(),
            field: Some("dataplane".to_owned()),
        })?;
    // Boot step 3 (ADR-0052, ADR-0061, ADR-0219): the root's composer
    // provisions, constructs and probes the dataplane and resolves the
    // node's address on the client interface; its refusals are the boot's.
    let pin_dir = config
        .dataplane_pin_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(overdrive_dataplane::DEFAULT_PIN_DIR));
    let cgroup_attach_path = config.dataplane_cgroup_attach_path.clone().unwrap_or_else(|| {
        std::path::PathBuf::from(overdrive_dataplane::DEFAULT_CGROUP_ATTACH_PATH)
    });
    let ComposedDataplane { dataplane, host_ipv4 } = dataplane_composer
        .compose(DataplanePlan {
            config: dataplane_cfg,
            vip_range: &config.vip_range,
            pin_dir: &pin_dir,
            cgroup_attach_path: &cgroup_attach_path,
        })
        .await?;
    // Phase 2.2: production single-mode uses a placeholder node id;
    // Phase 2 introduces real node-bootstrap identity that will replace
    // this. The shim writes this into `LogicalTimestamp.writer` on
    // `service_hydration_results` rows.
    let node_id = overdrive_core::id::NodeId::new("local").map_err(|e| {
        error::ControlPlaneError::Internal(format!("placeholder NodeId rejected: {e}"))
    })?;

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
    let journal: Arc<dyn journal::JournalStore> = Arc::clone(&stores.journal);
    let workflow_engine = Arc::new(workflow_runtime::WorkflowEngine::new(
        journal,
        clock.clone(),
        transport,
        Arc::clone(&entropy),
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
    // STORES-IN (ADR-0219): the CA comes from the composition root as a
    // factory, built here exactly once over the boot's one entropy, so a
    // composition that replaces `BootAdapters.entropy` cannot leave the CA
    // keyed to another source.
    let CaAdapters { factory: ca_factory, root_key_codec } = ca_adapters;
    let ca = ca_factory.build(Arc::clone(&entropy));

    // The `Kek` provider is `BootAdapters.kek` (§ C1-AMEND), never constructed
    // here: `overdrive serve` passes `SystemdCredsKeyring::new()` through
    // `NodeAdapters::host`; tests pass a hermetic `SimKek::for_boot()`. Both
    // `boot_ca` and `bootstrap_node_intermediate` take `&dyn Kek`.
    let codec = root_key_codec;
    let kek_id = ca_boot::root_kek_id();
    let intent_store: Arc<dyn IntentStore> = Arc::clone(&store) as Arc<dyn IntentStore>;

    let _root = ca_boot::boot_ca(
        ca.as_ref(),
        kek.as_ref(),
        &kek_id,
        codec.as_ref(),
        &intent_store,
        &store_path,
    )
    .await?;
    let _intermediate = ca_boot::bootstrap_node_intermediate(
        ca.as_ref(),
        &node_id,
        &intent_store,
        kek.as_ref(),
        &kek_id,
        codec.as_ref(),
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
        overdrive_core::guest_transport::GUEST_NODE_PREFIX,
        overdrive_core::guest_transport::GUEST_NODE_GATEWAY,
        Arc::clone(&entropy),
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
    let (mtls_worker, shared_guest_network) = {
        // (1) the root's enforcement factory builds the enforcement port over
        // the held identity, the F7 limits, the guest prefix and the boot's
        // guest dial (NO-OVERRIDE-SEAMS, LEGS-DIAL). `IdentityMgr` impls
        // `IdentityRead`. Leg S to a VM backend dials the guest-flow owner —
        // the boot's one `OwnerDial` on the composed path, the supplied owner
        // on the supplied path.
        let enforcement = mtls_adapters.enforcement.enforcement(
            overdrive_core::traits::mtls_enforcement::MtlsEnforcementDeps {
                identity: Arc::clone(&identity) as Arc<dyn overdrive_core::traits::IdentityRead>,
                limits: overdrive_core::traits::mtls_enforcement::MtlsLimits::default(),
                guest_prefix: overdrive_core::guest_transport::GUEST_NODE_PREFIX,
                guest_dial: match &guest_network {
                    GuestNetworkSource::Supplied(owner) => Arc::clone(owner)
                        as Arc<dyn overdrive_core::guest_transport::dial::GuestStreamDial>,
                    GuestNetworkSource::Composed { dial, .. } => Arc::clone(dial)
                        as Arc<dyn overdrive_core::guest_transport::dial::GuestStreamDial>,
                },
            },
        );

        // (2) probe (Earned Trust): wire → probe → use. A failure refuses
        // to boot (no cleartext fallback).
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
                // The configured service VIP range — the same value the VIP
                // allocator is built from (D24a); unconsulted until step 11.
                config.vip_range.clone(),
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

        // (4) construct the worker with all its ports as REQUIRED params
        // (mandatory `new()`, no builder). `ServerConfig` owns the intercept
        // adapter: production supplies `HostMtlsIntercept`; in-process
        // compositions can supply `SimMtlsIntercept` through the same port.
        // The worker uses it for the node-shared listeners and program as
        // well as allocation element registration. LEGS-DIAL: leg S of a VM
        // backend is dialled by the enforcement (above), not the worker.
        match guest_network {
            GuestNetworkSource::Supplied(owner) => {
                let worker =
                    Arc::new(overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::new(
                        enforcement,
                        resolve,
                        clock.clone(),
                        Arc::clone(&mtls_adapters.intercept),
                    ));
                (worker, owner)
            }
            GuestNetworkSource::Composed { io, dial } => {
                // D-UP-16 / D-8 phase 2: the owner takes this worker, and the
                // enforcement the worker is built over reaches the owner
                // through `dial`, which the composition binds; one
                // construction, no other owner or worker.
                let (owner, worker) = compose_guest_network(
                    GuestNetworkCompositionPorts {
                        clock: clock.clone(),
                        resolve,
                        enforcement,
                        intercept: Arc::clone(&mtls_adapters.intercept),
                        vip_range: config.vip_range.clone(),
                        node_prefix: overdrive_core::guest_transport::GUEST_NODE_PREFIX,
                        gateway: overdrive_core::guest_transport::GUEST_NODE_GATEWAY,
                        stream_port: config.vsock_ports.flow_stream(),
                        control_port: config.vsock_ports.flow_control(),
                    },
                    io,
                    dial,
                );
                (worker, owner as Arc<dyn guest_network::SharedGuestNetworkOwner>)
            }
        }
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
        clock.clone(),
        dataplane,
        ca,
        identity,
        node_id,
        allocator,
        listener_facts,
        host_ipv4,
        workflow_engine,
        // transparent-mtls-host-socket step 06-03: the worker composed over
        // the `NodeAdapters`' mTLS adapters and the `DataplaneComposer`'s
        // dataplane — required on every boot.
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
    // PORTS-CONFIG (ADR-0218): from the first `Ok` converge the owner reports
    // the ports its stream and control listeners bound; the boot reads them
    // once into the action shim's state.
    // SCAFFOLD: true — netns-density-295 DISTILL (attachment-cut step 6): the
    // host owner binds no listener until that step and reports `None`; the
    // `unreachable!` on `None` after an `Ok` converge lands with it.
    let flow_ports = shared_guest_network.flow_ports();
    if let Some(ports) = flow_ports
        && state.guest_flow_ports.set(ports).is_err()
    {
        unreachable!("the boot sets the guest-flow ports once, after step 9");
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
        // binds, inside the same block that composes the netns / intercept
        // path. It holds:
        //   - `state.obs` — the `service_backends` List/Watch source the
        //     internal `NameIndex` reads (the third sibling reader, DDN-1);
        //   - the boot's one clock (`BootAdapters.clock`) — the SOA SERIAL
        //     source for `wire::encode`;
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
        let dns_factory = Arc::clone(&guest_dns);
        let dns_responder_deps = (
            Arc::clone(&state.obs),
            clock.clone(),
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
                clock.clone(),
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
        clock.clone(),
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
        InterestRouterBroker::from_runtime(state.runtime.clone(), clock.clone()),
        // Single-clock DST preserved (ADR-0084 § Amendment 2026-08-23): the
        // router reads the SAME `BootAdapters.clock` instance the convergence loop
        // reads (one clock, two readers), so seed → bit-identical trajectory
        // holds. `INTEREST_ROUTER_RELIST_PERIOD` is the production relist
        // cadence — the row-backed level-triggered backstop.
        clock.clone(),
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
        spawn_workflow_emit_drain(state.clone(), clock.clone(), emit_drain_shutdown.clone());

    // Hold the worker's task tree at the same ownership boundary as the
    // server. The router and convergence tasks also hold AppState clones, but
    // this handle is the one both shutdown modes use to invalidate and JOIN
    // every worker child before returning.
    let mtls_worker_owner = Arc::clone(&state.mtls_worker);

    let supervisor_shutdown = CancellationToken::new();
    let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
    let supervisor_exec = guest_network_exec.supervisor();
    let supervisor_guest_network = Arc::clone(&shared_guest_network);
    let supervisor_clock = clock.clone();
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
        flow_ports,
        beacon_port: None,
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

        use overdrive_core::guest_transport::VsockListenPort;

        use crate::error::VmmBootError;
        use crate::node_adapters::{VmAdapters, VmCapability};
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
                Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
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
                test_probe_runner(),
                overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
                    SimClock::new(),
                ))
                .gate(),
                VmAdapters {
                    beacon: Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(
                        sim_vmm.clone(),
                    )),
                    vmm: Arc::new(sim_vmm),
                    cgroup_accounting: Arc::new(SimCgroupAccounting::new()),
                    capability: VmCapability::Required,
                },
                VsockListenPort::KernelAssigned,
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
        /// `(error::VmmBootError)`).
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

        /// ADR-0219: the composition root's `VmCapability` states what a
        /// failed probe means. Under `Optional` the same injected probe
        /// fault is `NotAvailable` (the node boots without a `Vm` driver),
        /// carrying the same typed cause `Required` refuses with.
        #[tokio::test]
        async fn optional_capability_classifies_the_probe_failure_as_not_available() {
            let sim_vmm = SimVmm::new();
            sim_vmm.inject_probe_failure(SimVmmProbeFault::LandlockLsmAbsent);

            let err = compose_vm_driver(
                PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
                overdrive_core::vm::config::clone_index_dir(&PathBuf::from("/srv/overdrive/data")),
                overdrive_core::vm::config::clone_staging_dir(&PathBuf::from(
                    "/srv/overdrive/data",
                )),
                Arc::new(SimClock::new()),
                Arc::new(SimCgroupFs::new()),
                test_probe_runner(),
                overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
                    SimClock::new(),
                ))
                .gate(),
                VmAdapters {
                    beacon: Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(
                        sim_vmm.clone(),
                    )),
                    vmm: Arc::new(sim_vmm),
                    cgroup_accounting: Arc::new(SimCgroupAccounting::new()),
                    capability: VmCapability::Optional,
                },
                VsockListenPort::KernelAssigned,
            )
            .await
            .err();

            assert!(
                matches!(
                    err,
                    Some(VmComposeError::NotAvailable(VmmBootError::Probe {
                        source: overdrive_core::traits::vmm::VmmProbeError::LandlockLsmAbsent { .. }
                    }))
                ),
                "expected NotAvailable(VmmBootError::Probe{{LandlockLsmAbsent}}), got {err:?}"
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
                test_probe_runner(),
                overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(
                    SimClock::new(),
                ))
                .gate(),
                VmAdapters {
                    beacon: Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(
                        sim_vmm.clone(),
                    )),
                    vmm: Arc::new(sim_vmm),
                    cgroup_accounting: Arc::new(sim_cgroup_accounting),
                    capability: VmCapability::Required,
                },
                VsockListenPort::KernelAssigned,
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
