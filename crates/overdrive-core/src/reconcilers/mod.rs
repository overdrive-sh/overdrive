//! Reconciler primitive — the §18 pure-function contract with
//! `TickContext` time injection per ADR-0035 (supersedes ADR-0013 §2 /
//! §2a partial / §2b).
//!
//! A reconciler is a pure function over `(desired, actual, view, tick)`
//! that emits a list of [`Action`]s to converge the system toward the
//! desired state. Three patterns govern how an author writes one; each
//! is load-bearing for DST replay (whitepaper §21) and ESR verification
//! (whitepaper §18 / research §1.1, §10.5).
//!
//! # The single-method, sync-only trait — ADR-0035 §1
//!
//! The trait carries exactly one author-written method:
//!
//! * [`Reconciler::reconcile`] is sync and pure — no `.await`, no I/O,
//!   no direct store write, no wall-clock read except via `tick.now` /
//!   `tick.now_unix`. It operates only on its arguments.
//!
//! Two invocations with the same inputs MUST produce byte-identical
//! output tuples. Storage is the runtime's responsibility — there is
//! no `migrate`, no `hydrate`, and no `persist` on the trait. The
//! runtime owns:
//!
//! * Intent hydration via `IntentStore` (driven by the runtime's
//!   `hydrate_desired` path; the `AnyReconciler` enum projects to the
//!   matching `AnyState` variant).
//! * Observation hydration via `ObservationStore` (driven by the
//!   runtime's `hydrate_actual` path; same projection shape).
//! * Per-reconciler `View` persistence via `ViewStore` — bulk-loaded
//!   into an in-memory `BTreeMap<TargetResource, View>` at boot,
//!   write-through on every successful `reconcile`. See ADR-0035 §2.
//!
//! # The time-injection pattern — survives from ADR-0013 §2c
//!
//! [`TickContext::now`] is the only legitimate source of "now" inside
//! `reconcile`. The runtime snapshots the injected `Clock` trait once
//! per evaluation and passes the result as a pure input — the same
//! `SystemClock` in production and `SimClock` under simulation that
//! control every other non-determinism boundary (whitepaper §21).
//!
//! Reading `Instant::now()` or `SystemTime::now()` inside a `reconcile`
//! body breaks DST replay and ESR verification; dst-lint catches it at
//! PR time (see `.claude/rules/development.md` §Reconciler I/O).
//!
//! # The `AnyReconciler` enum-dispatch convention — ADR-0035 §1
//!
//! `Reconciler` carries associated types (`State`, `View`) so erased
//! dispatch *across heterogeneous reconciler kinds* requires either
//! a concrete `(State, View)` pair on the dyn-trait reference or an
//! enum-dispatched wrapper. Overdrive uses `AnyReconciler` for the
//! latter — a hand-rolled enum that dispatches each trait method via
//! a match arm per variant. Static dispatch, zero heap allocation on
//! the hot path, compile-time exhaustiveness across every registered
//! reconciler kind. **Adding a new first-party reconciler means adding
//! one variant and one match arm** in each of `name` and `reconcile`.
//! Third-party reconcilers land through the WASM extension path
//! (whitepaper §18 "Extension Model") and do not go through
//! `AnyReconciler`.
//!
//! # The `NextView` return convention — ADR-0035 §1
//!
//! Reconcilers express writes as **data**, not side effects. The
//! [`Reconciler::reconcile`] signature returns `(Vec<Action>,
//! Self::View)`; the second element is the *next* view. The runtime
//! compares it against the in-memory view (`PartialEq` on
//! `&Self::View`); when they are equal the runtime skips the
//! `ViewStore::write_through` fsync and the in-memory map update
//! both. When they differ the runtime persists the full `next_view`
//! through `ViewStore` (write-through), then installs it into the
//! in-memory map. Reconcilers never write storage directly. Phase 1
//! convention is full-`View` replacement (`NextView = Self::View`)
//! gated by runtime Eq-diff; a typed-delta shape (e.g. a
//! `ViewAction::{Noop, Update(V)}` enum at the reconciler return
//! site) is an additive future extension only if profiling later
//! shows the equality check is a measurable cost.
//!
//! # Example
//!
//! A minimal Phase 2+ author walkthrough, modeled on the Phase 1
//! `NoopHeartbeat` shape. Returns one [`Action::Noop`] and an
//! unchanged `()` next-view. The `view` and `tick` parameters are
//! referenced explicitly to demonstrate how a real reconciler would
//! consume them.
//!
//! ```
//! use overdrive_core::reconcilers::{Action, Reconciler, ReconcilerName, TickContext};
//!
//! struct HelloReconciler {
//!     name: ReconcilerName,
//! }
//!
//! impl HelloReconciler {
//!     fn new() -> Self {
//!         Self {
//!             name: ReconcilerName::new(<Self as Reconciler>::NAME)
//!                 .expect("'hello' is a valid ReconcilerName"),
//!         }
//!     }
//! }
//!
//! impl Reconciler for HelloReconciler {
//!     /// Canonical kebab-case name; single compile-time anchor.
//!     const NAME: &'static str = "hello";
//!
//!     // Per ADR-0021, every reconciler picks its own `State`
//!     // projection. A reconciler with no meaningful desired/actual
//!     // shape picks `()`; the first real reconciler (`WorkloadLifecycle`)
//!     // picks `WorkloadLifecycleState`.
//!     type State = ();
//!     // Per ADR-0035 §1, `View` carries the four serde + Default +
//!     // Clone bounds; `()` satisfies them trivially. Phase 2+
//!     // authors declare a struct that derives the four bounds; the
//!     // runtime owns persistence end-to-end.
//!     type View = ();
//!
//!     fn name(&self) -> &ReconcilerName {
//!         &self.name
//!     }
//!
//!     // Pure, synchronous. No `.await`, no I/O, no direct store
//!     // write. The signature IS the contract.
//!     fn reconcile(
//!         &self,
//!         _desired: &Self::State,
//!         _actual: &Self::State,
//!         view: &Self::View,
//!         tick: &TickContext,
//!     ) -> (Vec<Action>, Self::View) {
//!         // `tick.now` is the only legitimate source of "now" inside
//!         // reconcile. Phase 2+ reconcilers consult it for retry-
//!         // budget gates, backoff deadlines, and lease-renewal
//!         // decisions. NEVER call `Instant::now()` here — dst-lint
//!         // will reject the PR.
//!         let _now = tick.now;
//!
//!         // `view` carries the in-memory per-target view the runtime
//!         // bulk-loaded at boot. The returned next-view (second
//!         // element of the tuple) is diffed by the runtime against
//!         // this value and persisted via `ViewStore::write_through`.
//!         // Reconcilers never write storage directly.
//!         let next_view: Self::View = *view;
//!
//!         (vec![Action::Noop], next_view)
//!     }
//! }
//!
//! // Construction is plain — the runtime wraps the instance in
//! // `AnyReconciler::<Variant>` when registering.
//! let _reconciler = HelloReconciler::new();
//! ```

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::aggregate::WorkloadKind;
use crate::id::{AllocationId, ContentHash, CorrelationKey, NodeId, SpiffeId, WorkloadId};
use crate::traits::driver::AllocationSpec;
use crate::traits::observation_store::{ObservationRowKind, ServiceBackendRow};
use crate::transition_reason::TerminalCondition;
use crate::wall_clock::UnixInstant;

// reconcilers-own-hydration (ADR-0086 D1/D3/D5) — the `HydrationContext`
// borrow-bundle + `HydrateError` the async `hydrate_*` trait methods read
// through. STAYS in core (contract-in-core); the impl bodies land in step
// 02-04. The reconciler IMPLS + the three dispatch enums + `service_lifecycle`
// + the per-reconciler `State`/`View` types + pure helpers were extracted to
// the `overdrive-reconcilers` crate in step 02-02 (ADR-0086 D3) — depend on
// them via `overdrive_reconcilers::*`, NOT this module.
pub mod hydration;

pub use hydration::{HydrateError, HydrationContext};

// ---------------------------------------------------------------------------
// TickContext — time as injected input state
// ---------------------------------------------------------------------------

/// Time injected into `reconcile` as pure input.
///
/// The runtime constructs exactly one `TickContext` per evaluation by
/// snapshotting the injected `Clock` trait once — reconcilers must
/// read time via `tick.now` / `tick.now_unix` rather than calling
/// `Instant::now()` / `SystemTime::now()` directly (dst-lint enforces
/// this at PR time).
///
/// * `now` — the **monotonic, process-local** instant the evaluation
///   started. Use for in-process deadline arithmetic
///   (`tick.now < tick.deadline`) and for any comparison against
///   another `Instant` taken on the same process. Cannot be
///   persisted to libSQL, gossiped to a peer, or compared across
///   process restart — `Instant` is opaque.
/// * `now_unix` — the **wall-clock, persistable** snapshot. Use for
///   any deadline that must survive process restart or be persisted
///   to libSQL (per `.claude/rules/development.md` § "Reconciler
///   I/O" and `.claude/rules/development.md` § "Persist inputs, not
///   derived state"). Advances under DST alongside `now` per
///   `SimClock` discipline (both fields are snapshotted from the same
///   underlying logical-time counter).
/// * `tick` — a monotonic counter useful as a deterministic
///   tie-breaker across evaluations.
/// * `deadline` — the runtime's per-tick budget. Reconcilers that need
///   to checkpoint bounded work into their `NextView` consult this.
#[derive(Debug, Clone)]
pub struct TickContext {
    /// Monotonic, process-local wall-clock snapshot at evaluation
    /// start. Use for in-process deadline arithmetic; cannot be
    /// persisted.
    pub now: Instant,
    /// Wall-clock, persistable snapshot at evaluation start. Use for
    /// deadlines that must survive process restart or be persisted to
    /// libSQL.
    pub now_unix: UnixInstant,
    /// Monotonic tick counter.
    pub tick: u64,
    /// Per-tick deadline (`now + reconcile_budget`).
    pub deadline: Instant,
}

// ---------------------------------------------------------------------------
// Reconciler trait
// ---------------------------------------------------------------------------

/// The §18 reconciler trait, single-method sync shape.
///
/// Per ADR-0035 §1 (which supersedes ADR-0013 §2 / §2a partial / §2b):
///
/// * `reconcile` is pure and synchronous — no `.await`, no I/O, no
///   wall-clock read (only via `tick.now`), no direct store write. The
///   returned `(Vec<Action>, Self::View)` tuple carries actions the
///   runtime commits through Raft and the next-view the runtime diffs
///   against the in-memory cache and persists via `ViewStore`.
///
/// Per ADR-0036 the trait carries NO async hydrate / migrate / persist
/// surface. The runtime owns all hydration: intent + observation are
/// hydrated into `AnyState` variants by the runtime; per-reconciler
/// `View` memory is bulk-loaded at boot via `ViewStore::bulk_load` and
/// served from an in-memory `BTreeMap` thereafter, with write-through
/// after each `reconcile`.
///
/// Compile-time enforcement: the acceptance test
/// `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
/// pins the signature via an
/// `fn(&R, &R::State, &R::State, &R::View, &TickContext) -> (Vec<Action>, R::View)`
/// type assertion. A regression that makes `reconcile` `async fn`,
/// adds a `&dyn Clock` parameter, re-introduces a `&LibsqlHandle`
/// parameter, or reverts the per-reconciler typed `State` associated
/// type (ADR-0021) fails that test at compile time.
///
/// # Hydration methods — impure, async (ADR-0086 D1)
///
/// Per ADR-0086 (superseding-in-part ADR-0036 for the intent + observation
/// half) reconcilers own their hydration: [`hydrate_desired`](Reconciler::hydrate_desired)
/// and [`hydrate_actual`](Reconciler::hydrate_actual) are **impure, async**
/// methods reading through a [`HydrationContext`] borrow-bundle. They are the
/// ONLY impure surface on the trait; `reconcile` stays pure-sync. The trait
/// carries `todo!("RED scaffold")` defaults so core compiles and nothing calls
/// them until the per-reconciler bodies land in step 02-04 (VIEW hydration
/// stays runtime-owned per ADR-0035 §2 — unchanged). The async methods carry NO
/// `&dyn Clock` parameter (ADR-0086 D1; the compile-guard's additive assertion
/// lands in 02-05).
#[async_trait]
pub trait Reconciler: Send + Sync {
    /// Canonical kebab-case name as a single compile-time anchor.
    ///
    /// Per the `refactor-reconciler-static-name` RCA: the production
    /// `RedbViewStore::table_def` previously called `Box::leak` on a
    /// fresh `String` per invocation, leaking ~30 B per write-through
    /// per active target every tick. Threading a `const NAME: &'static
    /// str` through the `ViewStore` byte-level surface eliminates the
    /// leak class structurally — the `&'static` lifetime
    /// `redb::TableDefinition` requires is encoded in the type system,
    /// not recovered at runtime via `Box::leak` or an interner.
    ///
    /// Implementors MUST declare a string literal (or a `const`-fn
    /// derivation thereof) so `Self::NAME` aliases the binary's data
    /// segment — the regression test
    /// `tests/integration/redb_view_store_no_leak.rs` asserts the
    /// pointer-identity property mechanically.
    ///
    /// The declared value MUST satisfy `ReconcilerName::new`'s
    /// `^[a-z][a-z0-9-]{0,62}$` validator. A typo or invalid character
    /// is caught the first time `name(&self)` is constructed via
    /// `ReconcilerName::new(Self::NAME).expect(...)` — typically at
    /// `canonical()` construction time, before any `register` call.
    const NAME: &'static str;

    /// Author-declared projection of the reconciler's `desired` /
    /// `actual` cluster state. Per ADR-0021, every reconciler picks
    /// its own typed projection rather than sharing a single
    /// placeholder — the runtime owns hydrate-desired / hydrate-actual
    /// and constructs the matching `AnyState` variant on each tick.
    type State: Send + Sync;

    /// Author-declared projection of the reconciler's private memory.
    /// Per ADR-0035 §1 the runtime owns persistence end-to-end.
    type View: Serialize + DeserializeOwned + Default + Clone + Eq + Send + Sync;

    /// Canonical name.
    fn name(&self) -> &ReconcilerName;

    /// Pure function over `(desired, actual, view, tick) ->
    /// (Vec<Action>, NextView)`.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View);

    /// Hydrate this reconciler's `desired` projection (ADR-0086 D1).
    ///
    /// Impure + async: reads intent (and any other desired-side surface) for
    /// `target` through the [`HydrationContext`] borrow-bundle, returning the
    /// typed `Self::State`. This is one of the two impure surfaces on the trait
    /// (`reconcile` stays pure-sync); it carries NO `&dyn Clock` parameter
    /// (ADR-0086 D1).
    ///
    /// The default is a `todo!("RED scaffold")`: the per-reconciler body is
    /// ported off the central `hydrate_*_desired` free fn in step 02-04.
    /// Nothing calls it before then, and every impl inherits this default so
    /// core still compiles.
    #[expect(clippy::todo, reason = "RED scaffold; hydrate bodies land in step 02-04")]
    async fn hydrate_desired(
        &self,
        _ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        todo!("RED scaffold: Reconciler::hydrate_desired lands per-impl in step 02-04")
    }

    /// Hydrate this reconciler's `actual` projection (ADR-0086 D1).
    ///
    /// Impure + async: reads observation rows / host state / the injected
    /// read-ports for `target` through the [`HydrationContext`] borrow-bundle,
    /// returning the typed `Self::State`. The mirror of
    /// [`hydrate_desired`](Reconciler::hydrate_desired); same purity contract
    /// (impure/async, no `&dyn Clock`).
    ///
    /// The default is a `todo!("RED scaffold")`: the per-reconciler body is
    /// ported off the central `hydrate_*_actual` free fn in step 02-04.
    #[expect(clippy::todo, reason = "RED scaffold; hydrate bodies land in step 02-04")]
    async fn hydrate_actual(
        &self,
        _ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        todo!("RED scaffold: Reconciler::hydrate_actual lands per-impl in step 02-04")
    }

    /// Declarative level-triggered resync cadence — a safety net beside
    /// the edge-triggered broker (K8s `SyncPeriod` / `RequeueAfter`;
    /// kube-rs `Action::requeue_after`). Default `None` = edge-triggered
    /// only, no backstop (ADR-0084 §1, Piece A).
    ///
    /// PURE + object-safe: returns concrete data, reads NO clock, holds
    /// no handle. The convergence loop owns the clock (`SimClock` under
    /// DST), the local [`NodeId`], and scope→target resolution
    /// ([`resolve_scope`]). No associated types ⇒ one `AnyReconciler`
    /// forwarding arm; touches no `AnyState` / `AnyReconcilerView`. Adds
    /// no async surface and does not alter `reconcile`, so the
    /// compile-time guard
    /// `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
    /// still passes (ADR-0036 stands).
    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        None
    }

    /// Declarative event-interest: which observation-row changes wake this
    /// reconciler. Default `&[]` = **host-backed** (hydrates `actual` live
    /// from the host, never row-backed) ⇒ **resync-only**, never
    /// event-woken, with `resync_schedule` as its level-triggered backstop.
    /// The interest declaration IS the partition key (ADR-0084 §1, Piece B;
    /// Titan SD-6): non-empty ⟺ row-backed ⟺ event-woken, **with the
    /// interest-router's periodic relist as the level-triggered backstop**
    /// (ADR-0084 §5 / Amendment 2026-08-23) — NOT a per-reconciler
    /// `resync_schedule`, which is the backstop for the host-backed
    /// partition instead.
    ///
    /// PURE + object-safe: returns a borrowed `'static` slice of
    /// [`ObservationRowKind`] — a complete row-family discriminant, no
    /// payload, no severity, no occurrence semantics (contrast GH #265's
    /// outbound `ObservationEvent`), no clock, no I/O, no handle. No
    /// associated types ⇒ one `AnyReconciler` forwarding arm; touches no
    /// `AnyState` / `AnyReconcilerView`. Adds no async surface and does not
    /// alter `reconcile`, so the compile-time guard
    /// `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
    /// still passes (ADR-0036 stands).
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[]
    }
}

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// Actions a reconciler can emit. Phase 1 ships `Noop`, `HttpCall`, and a
/// `StartWorkflow` placeholder (workflow runtime lands Phase 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The reconciler has nothing to do this tick.
    Noop,

    /// An external HTTP call.
    HttpCall {
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
        /// Target URL.
        target: String,
        /// HTTP method.
        method: String,
        /// Request body bytes.
        body: Bytes,
        /// Per-attempt timeout.
        timeout: Duration,
        /// Idempotency key supplied to the remote API when supported.
        idempotency_key: Option<String>,
    },

    /// Start a workflow. Carries the durable [`WorkflowStart`] intent.
    StartWorkflow {
        /// The durable workflow START intent (identity + opaque input).
        start: WorkflowStart,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Start a fresh allocation for a job.
    StartAllocation {
        /// Newly-minted allocation identifier.
        alloc_id: AllocationId,
        /// Owning job.
        workload_id: WorkloadId,
        /// Placement decision.
        node_id: NodeId,
        /// Resources / command / args / identity for the workload.
        spec: AllocationSpec,
        /// Workload-kind discriminator per ADR-0047 §1.
        kind: WorkloadKind,
    },

    /// Stop a Running allocation.
    StopAllocation {
        /// Target allocation.
        alloc_id: AllocationId,
        /// Reconciler-decided terminal claim per ADR-0037 §4.
        terminal: Option<TerminalCondition>,
    },

    /// Restart an allocation.
    ///
    /// Carries no restart-cause field. Per ADR-0087 D4 the restart's
    /// cause is the prior observed alloc row's terminal (a crash
    /// terminal for a crash loop, `Stopped { by: LivenessProbe }` for a
    /// liveness kill) — `WorkloadLifecycle` is the sole restart
    /// authority and reads that terminal directly; the action-shim's
    /// stop+start semantics are identical regardless of cause per
    /// ADR-0023 §2 / ADR-0037 §4 (RestartAllocation is never a terminal
    /// claim).
    RestartAllocation {
        /// Allocation to restart.
        alloc_id: AllocationId,
        /// Resources / command / args / identity for the workload.
        spec: AllocationSpec,
        /// Workload-kind discriminator per ADR-0047 §1.
        kind: WorkloadKind,
    },

    /// Finalize a failed allocation as terminal.
    FinalizeFailed {
        /// Allocation to finalize.
        alloc_id: AllocationId,
        /// Reconciler-decided terminal claim.
        terminal: Option<TerminalCondition>,
    },

    /// Replace the backend set for a service frontend
    /// `(vip, port, proto)` in the kernel-side maps.
    DataplaneUpdateService {
        /// Identity of the service.
        service_id: crate::id::ServiceId,
        /// Virtual IP. Carried as `ServiceVip` (IPv6-admitting) so the
        /// action-shim performs the operator-visible IPv4 validation via
        /// `ServiceFrontend::new` (ADR-0060 D1a); the dataplane never
        /// sees an IPv6 VIP.
        vip: crate::id::ServiceVip,
        /// Service listener port. Sourced from a listener-bearing fact;
        /// projected to `BackendKey`'s `u16` via `.get()` at the adapter.
        port: std::num::NonZeroU16,
        /// L4 protocol. Sourced from a listener-bearing fact — NEVER
        /// defaulted to `Tcp` (ADR-0060 C3).
        proto: crate::dataplane::backend_key::Proto,
        /// Backend set, in deterministic iteration order.
        backends: Vec<crate::traits::dataplane::Backend>,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Release a VIP from the `ServiceVipAllocator` memo.
    ReleaseServiceVip {
        /// Content-addressed spec digest.
        spec_digest: ContentHash,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Write a `ServiceBackendRow` to the ObservationStore.
    WriteServiceBackendRow {
        /// The full `ServiceBackendRow` payload.
        row: ServiceBackendRow,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Enqueue a reconciliation evaluation for another reconciler.
    EnqueueEvaluation {
        /// Name of the downstream reconciler to enqueue.
        reconciler: ReconcilerName,
        /// Target the downstream reconciler should reconcile against.
        target: TargetResource,
    },

    /// Register the local backend for `(vip, vip_port, proto)`.
    RegisterLocalBackend {
        /// Identity of the service.
        service_id: crate::id::ServiceId,
        /// Virtual IP.
        vip: std::net::Ipv4Addr,
        /// VIP port the listener accepts on.
        vip_port: u16,
        /// L4 protocol the listener serves (ADR-0053 rev Amendment 3).
        /// Sourced from the listener-bearing fact, NEVER defaulted to
        /// `Tcp` (C3) — a service co-locating tcp/53 + udp/53 emits
        /// two `RegisterLocalBackend` with distinct proto.
        proto: crate::dataplane::backend_key::Proto,
        /// Resolved local backend `(IPv4, port)`.
        backend: std::net::SocketAddrV4,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Deregister the local backend for `(vip, vip_port, proto)`.
    DeregisterLocalBackend {
        /// Identity of the service.
        service_id: crate::id::ServiceId,
        /// VIP whose entry to remove.
        vip: std::net::Ipv4Addr,
        /// VIP port whose entry to remove.
        vip_port: u16,
        /// L4 protocol whose entry to remove (ADR-0053 rev Amendment 3).
        proto: crate::dataplane::backend_key::Proto,
        /// Resolved local backend `(IPv4, port)` whose reverse entry to
        /// remove. Caller-supplied so the reverse removal is retry-safe —
        /// it does not depend on a since-removed forward entry (GH #211).
        /// Mirrors `RegisterLocalBackend::backend`.
        backend: std::net::SocketAddrV4,
        /// Cause-to-response linkage.
        correlation: CorrelationKey,
    },

    /// Issue, re-issue, or ROTATE a workload SVID for a Running allocation
    /// (ADR-0067 D2; feature-delta D-OC-1). Emitted by the pure
    /// `SvidLifecycle` reconciler on two branches: `running ∧ ¬held`
    /// (first-issue / restart-recovery, `"issue-svid"` correlation) and
    /// `running ∧ held` near-expiry (rotation, `"rotate-svid"` correlation,
    /// feature-delta A1); dispatched by the action-shim `issue_svid` executor (01-06),
    /// which mints the leaf via `ca_issuance::issue_and_audit` and holds it
    /// in the in-process `IdentityMgr`. CA I/O lives entirely in the
    /// executor — never in `reconcile()`.
    IssueSvid {
        /// The allocation the SVID is issued for.
        alloc_id: AllocationId,
        /// The workload identity, built PURE by the reconciler via
        /// [`SpiffeId::for_allocation`] (ADR-0067 D5) — identity derivation
        /// is pure; identity issuance is the executor's.
        spiffe_id: SpiffeId,
        /// The node the SVID is issued on. Self-describing: this is the
        /// `issued_certificates` row's `node_id` and the
        /// `issue_and_audit(.., node, ..)` argument (#36-forward-compat).
        node_id: NodeId,
        /// Cause-to-response linkage. Derived via
        /// `CorrelationKey::derive("svid-lifecycle/<alloc>", spec_hash,
        /// purpose)` with `purpose` ∈ {`"issue-svid"`, `"rotate-svid"`} —
        /// `"issue-svid"` on the `running ∧ ¬held` branch, `"rotate-svid"` on
        /// the `running ∧ held` near-expiry rotation branch. NOT a per-attempt
        /// request id (ADR-0035 reconciler-I/O correlation discipline).
        correlation: CorrelationKey,
    },

    /// Drop a held workload SVID for an allocation that is no longer
    /// Running (ADR-0067 D2). Emitted by the pure `SvidLifecycle`
    /// reconciler (01-04) on the `¬running ∧ held` branch; dispatched by
    /// the action-shim executor (01-06), which calls
    /// `IdentityMgr::drop_svid` so the node-held leaf private key is no
    /// longer reachable in the held set (O2 — leak resistance on stop).
    DropSvid {
        /// The allocation whose held SVID is dropped.
        alloc_id: AllocationId,
        /// Cause-to-response linkage. Derived via
        /// `CorrelationKey::derive("svid-lifecycle/<alloc>", spec_hash,
        /// "drop-svid")` — NOT a per-attempt request id.
        correlation: CorrelationKey,
    },

    /// Author a Platform-Reclamation ending for `alloc_id` (ADR-0081 D1 /
    /// D2; SD-1). Emitted by the pure [`vm_reclamation::plan_reclamation`]
    /// diff when a non-terminal VM allocation's supervision is NOT held
    /// (`SupervisionSet::reclamation_authorised` is `true`); dispatched
    /// by a later step's `action_shim::reclamation::execute_reclaim_allocation`
    /// executor, which `kill_scope`s, `discard_artifacts`, and writes the
    /// terminal row (`brief.md` §105a.5).
    ///
    /// `alloc_id`-only payload, deliberately (DD-5, binding): no
    /// disposition parameter (the variant IS the class), no regime field
    /// (`boot_epoch` / `is_boot`) — the kill-authorising check reads the
    /// observed live-handle set, never a caller-declared boolean. The
    /// executor re-observes rather than trusting anything carried here;
    /// an observation carried from the diff into the plan goes stale
    /// between emit and execute.
    ReclaimAllocation {
        /// Target allocation.
        alloc_id: AllocationId,
    },

    /// Dispose of host state backing NO live instance of a non-terminal
    /// allocation (ADR-0081 D2 / D4 — Artifact Disposal, NOT an Ending
    /// Class: authors no ending, writes no row). Emitted by
    /// [`vm_reclamation::plan_reclamation`] for a terminal VM allocation
    /// (the disposal-not-reclamation exemption) or an unknown allocation
    /// on a VM-exclusive host surface whose supervision is not held.
    ///
    /// `alloc_id`-only payload, same DD-5 payload prohibitions as
    /// [`Action::ReclaimAllocation`]. Dispatched by a later step's
    /// `execute_discard_stranded_artifacts` executor, which `kill_scope`s
    /// and `discard_artifacts`s and NOTHING else — no row write, no
    /// evaluation submitted (the declared delta over the observation
    /// universe is structurally empty: the executor takes no
    /// `ObservationStore` and no broker parameter at all).
    DiscardStrandedArtifacts {
        /// Target allocation.
        alloc_id: AllocationId,
    },
}

// `WorkflowStart` is the concrete shape defined in `crate::workflow`
// (ADR-0064 §1, replacing the former unit placeholder). It is
// re-exported here so `Action::StartWorkflow` (above) and existing
// `reconcilers::WorkflowStart` references keep resolving against this
// path.
pub use crate::workflow::WorkflowStart;

// ---------------------------------------------------------------------------
// ReconcilerName newtype
// ---------------------------------------------------------------------------

/// Maximum length for a reconciler name, matching
/// `^[a-z][a-z0-9-]{0,62}$` (1 lead + up to 62 interior = 63 total).
const RECONCILER_NAME_MAX: usize = 63;

/// Canonical reconciler name. Kebab-case, `^[a-z][a-z0-9-]{0,62}$`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReconcilerName(String);

impl ReconcilerName {
    /// Validating constructor.
    pub fn new(raw: &str) -> Result<Self, ReconcilerNameError> {
        if raw.is_empty() {
            return Err(ReconcilerNameError::Empty);
        }
        if raw.len() > RECONCILER_NAME_MAX {
            return Err(ReconcilerNameError::TooLong { got: raw.len() });
        }

        let mut chars = raw.chars();
        #[allow(clippy::expect_used)]
        let lead = chars.next().expect("non-empty checked above");
        if !lead.is_ascii_lowercase() {
            return Err(ReconcilerNameError::InvalidLead);
        }

        for ch in chars {
            if !is_valid_interior_char(ch) {
                return Err(ReconcilerNameError::ForbiddenCharacter { found: ch });
            }
        }

        Ok(Self(raw.to_string()))
    }

    /// Canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Interior characters allowed after the leading lowercase letter.
#[inline]
const fn is_valid_interior_char(ch: char) -> bool {
    matches!(ch, 'a'..='z' | '0'..='9' | '-')
}

impl fmt::Display for ReconcilerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ReconcilerName {
    type Err = ReconcilerNameError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

/// Errors from `ReconcilerName::new`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReconcilerNameError {
    /// Empty input string.
    #[error("empty reconciler name")]
    Empty,
    /// Input longer than the 63-byte cap.
    #[error("reconciler name too long: {got} > 63")]
    TooLong {
        /// Observed length of the rejected input.
        got: usize,
    },
    /// Input contained a character outside `[a-z0-9-]`.
    #[error("reconciler name contains forbidden character: {found:?}")]
    ForbiddenCharacter {
        /// The offending character.
        found: char,
    },
    /// Input did not start with a lowercase ASCII letter.
    #[error("reconciler name must start with a lowercase letter")]
    InvalidLead,
}

// ---------------------------------------------------------------------------
// TargetResource — broker key component
// ---------------------------------------------------------------------------

/// Canonical shapes accepted by `TargetResource::new`.
///
/// `workflow/` is the workflow-lifecycle reconciler's target shape
/// (ADR-0064 §5). Unlike the per-resource reconcilers, the
/// workflow-lifecycle reconciler converges ALL instances each tick (its
/// hydrate scans the `workflows/` intent prefix), so the conventional
/// target is `workflow/all`; the `/all` id-part is non-empty and so
/// satisfies the shape rule.
const CANONICAL_TARGET_PREFIXES: &[&str] =
    &["workload/", "node/", "alloc/", "service/", "workflow/"];

/// Target-resource component of the evaluation broker's key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetResource(String);

impl TargetResource {
    /// Validating constructor.
    pub fn new(raw: &str) -> Result<Self, TargetResourceError> {
        if raw.is_empty() {
            return Err(TargetResourceError::Empty);
        }

        for prefix in CANONICAL_TARGET_PREFIXES {
            if let Some(id_part) = raw.strip_prefix(prefix) {
                if id_part.is_empty() {
                    return Err(TargetResourceError::UnknownShape { raw: raw.to_string() });
                }
                return Ok(Self(raw.to_string()));
            }
        }

        Err(TargetResourceError::UnknownShape { raw: raw.to_string() })
    }

    /// Canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for TargetResource {
    type Err = TargetResourceError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

/// Errors from `TargetResource::new`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetResourceError {
    /// Empty input string.
    #[error("empty target resource")]
    Empty,
    /// Input did not match any canonical prefix.
    #[error("target resource has unknown shape: {raw}")]
    UnknownShape {
        /// The rejected input, echoed back for diagnostics.
        raw: String,
    },
}

// ---------------------------------------------------------------------------
// Piece A — cadence declarations (ADR-0084 §2, pure data)
// ---------------------------------------------------------------------------

/// A declarative level-triggered resync cadence — the concrete data a
/// reconciler returns from [`Reconciler::resync_schedule`] to opt into a
/// periodic broker resync (ADR-0084 §2, Piece A).
///
/// Pure data: no clock, no handle. The convergence loop owns the clock
/// (`SimClock` under DST), the local [`NodeId`], and scope→target
/// resolution (see [`resolve_scope`]); the reconciler names no target
/// string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResyncSchedule {
    /// Minimum wall-clock period between level-triggered resyncs. The
    /// loop's per-reconciler next-wake table re-arms at most once per
    /// period.
    ///
    /// `Duration` is the same monotonic type [`TickContext`] already uses.
    pub period: Duration,
    /// Which target(s) each fire submits. The loop resolves this via
    /// [`resolve_scope`] — the reconciler never names a target string.
    pub scope: ResyncScope,
}

/// The target-set a resync fires against. Resolved by the loop from state
/// it owns (the local [`NodeId`]).
///
/// Phase 1 ships exactly one variant — `LocalNode` — the only shape any
/// current or incoming reconciler needs (ADR-0084 §2). A coarse
/// whole-set scope (`WholeManaged`) is deliberately NOT declared: its
/// resolver would need the managed-target-set source that is itself the
/// GH #270 bounding concern, so shipping it now would force an
/// unimplementable (`todo!`) resolver arm — the exact unused-surface
/// smell the project forbids. It is added additively (one enum variant +
/// one [`resolve_scope`] arm, in one change) the day a reconciler
/// declares it. Keeping the enum single-variant today means
/// [`resolve_scope`] is total and fully exercised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncScope {
    /// Resolves to exactly `node/<local_node_id>` (the loop supplies the
    /// id). The vm-reclamation motivating case.
    LocalNode,
}

impl ResyncScope {
    /// Canonical lowercase string form (label-enum rule per
    /// `.claude/rules/development.md` § "Label enums own their string
    /// representation").
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalNode => "local-node",
        }
    }
}

/// Resolve a [`ResyncScope`] to the concrete broker [`TargetResource`](s)
/// a resync fires against (ADR-0084 §4.4).
///
/// The convergence loop (step 01-02) owns and supplies the local
/// [`NodeId`]; this resolver is a pure function over `(scope, node_id)`.
/// It is TOTAL over the single-variant [`ResyncScope`] — no `todo!` /
/// `unreachable` arm on the scope match, which is the reason
/// `WholeManaged` is intentionally not shipped (ADR-0084 §2).
///
/// `LocalNode` resolves to exactly `[ node/<node_id> ]`. A [`NodeId`] is a
/// non-empty, slash-free label (`crates/overdrive-core/src/id.rs`
/// `validate_label`), so `node/<id>` always satisfies
/// [`TargetResource`]'s `node/` prefix rule — the construction cannot
/// fail, and the `unreachable!` documents that invariant rather than
/// papering over a real error path.
#[must_use]
pub fn resolve_scope(scope: ResyncScope, node_id: &NodeId) -> Vec<TargetResource> {
    match scope {
        ResyncScope::LocalNode => {
            let target =
                TargetResource::new(&format!("node/{}", node_id.as_str())).unwrap_or_else(|_| {
                    unreachable!(
                        "NodeId is a non-empty, slash-free label, so node/<id> always \
                         satisfies TargetResource's node/ prefix rule"
                    )
                });
            vec![target]
        }
    }
}

// ---------------------------------------------------------------------------
// Piece A — resolve_scope totality (S-266-07, co-located default-lane proptest)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod resolve_scope_tests {
    use proptest::prelude::*;

    use super::{NodeId, ResyncScope, TargetResource, resolve_scope};

    /// `ResyncScope` owns its canonical lowercase label (label-enum rule
    /// per `.claude/rules/development.md`). Pins the string so a mutated
    /// label (`as_str -> ""` / `-> "xyzzy"`) is caught.
    #[test]
    fn resync_scope_local_node_as_str_is_canonical_kebab_label() {
        assert_eq!(ResyncScope::LocalNode.as_str(), "local-node");
    }

    /// Strategy yielding an arbitrary VALID `NodeId`.
    ///
    /// Mirrors the `validate_label` contract
    /// (`crates/overdrive-core/src/id.rs`): non-empty, chars in
    /// `[a-z0-9-_.]`, and first/last char alphanumeric. First and last
    /// glyphs are drawn from the alphanumeric class; interior glyphs from
    /// the full label class.
    fn valid_node_id() -> impl Strategy<Value = NodeId> {
        let alnum: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
        let interior: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789-_.".chars().collect();
        (
            proptest::sample::select(alnum.clone()),
            proptest::collection::vec(proptest::sample::select(interior), 0..=16),
            proptest::sample::select(alnum),
        )
            .prop_map(|(first, mid, last)| {
                let mut raw = String::with_capacity(2 + mid.len());
                raw.push(first);
                raw.extend(mid);
                raw.push(last);
                NodeId::new(&raw).expect("generator yields only valid NodeIds")
            })
    }

    proptest! {
        /// S-266-07 — `resolve_scope(LocalNode, n)` is a TOTAL mapping over
        /// the single-variant `ResyncScope`, returning exactly
        /// `[ TargetResource("node/<n>") ]` for every valid `NodeId n`.
        ///
        /// Mutation target: the `node/<id>` scope→target derivation. A
        /// mutated prefix / dropped id / extra element must break the
        /// exact-vector equality below.
        #[test]
        fn local_node_scope_resolves_to_exactly_the_local_node_target(node in valid_node_id()) {
            let resolved = resolve_scope(ResyncScope::LocalNode, &node);

            let expected = TargetResource::new(&format!("node/{}", node.as_str()))
                .expect("node/<valid NodeId> is a canonical TargetResource");

            prop_assert_eq!(resolved, vec![expected]);
        }
    }
}
