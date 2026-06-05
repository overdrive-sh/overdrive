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
//! enum-dispatched wrapper. Overdrive uses [`AnyReconciler`] for the
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
//! [`NoopHeartbeat`] shape. Returns one [`Action::Noop`] and an
//! unchanged `()` next-view. The `view` and `tick` parameters are
//! referenced explicitly to demonstrate how a real reconciler would
//! consume them.
//!
//! ```
//! use overdrive_core::reconcilers::{
//!     Action, Reconciler, ReconcilerName, TickContext,
//! };
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

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::aggregate::WorkloadKind;
use crate::id::{AllocationId, ContentHash, CorrelationKey, NodeId, WorkloadId};
use crate::traits::driver::AllocationSpec;
use crate::traits::observation_store::ServiceBackendRow;
use crate::transition_reason::TerminalCondition;
use crate::wall_clock::UnixInstant;

pub mod backend_discovery_bridge;
pub mod noop_heartbeat;
pub mod service_map_hydrator;
pub mod workload_lifecycle;

pub use backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
};
pub use noop_heartbeat::NoopHeartbeat;
pub use service_map_hydrator::{
    BackendAddressRejection, RetryMemory, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, classify_backend_address,
};
pub use workload_lifecycle::{
    RESTART_BACKOFF_CEILING, RESTART_BACKOFF_DURATION, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView, backoff_for_attempt, project_probe_descriptors,
};

// `ServiceLifecycleReconciler` lives in `overdrive_core::service_lifecycle`
// (NOT under this module) for cycle-breaking reasons documented at the
// `crate::service_lifecycle` module header. Re-import here so the
// dispatch enums (`AnyState`, `AnyReconciler`, `AnyReconcilerView`) can
// reference it without forcing every dispatcher to spell the full path.
use crate::service_lifecycle::{
    ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

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
/// hydrated into [`AnyState`] variants by the runtime; per-reconciler
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
    /// and constructs the matching [`AnyState`] variant on each tick.
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
}

// ---------------------------------------------------------------------------
// AnyState enum — per-reconciler typed `desired`/`actual` projection
// ---------------------------------------------------------------------------

/// Sum of every `desired`/`actual` shape consumed by a registered reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyState {
    /// `State = ()` variant for Phase 1 reconcilers that do not
    /// dereference their projection (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's typed projection — see
    /// [`WorkloadLifecycleState`].
    WorkloadLifecycle(WorkloadLifecycleState),
    /// `ServiceMapHydrator` reconciler's typed projection — see
    /// [`ServiceMapHydratorState`].
    ServiceMapHydrator(ServiceMapHydratorState),
    /// `BackendDiscoveryBridge` reconciler's typed projection — see
    /// [`backend_discovery_bridge::BackendDiscoveryBridgeState`].
    BackendDiscoveryBridge(BackendDiscoveryBridgeState),
    /// `ServiceLifecycle` reconciler's typed projection — see
    /// [`crate::service_lifecycle::ServiceLifecycleState`]. Per
    /// ADR-0055; landed by the `service-health-check-probes` feature.
    ServiceLifecycle(ServiceLifecycleState),
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

    /// Start a workflow. `WorkflowSpec` is a placeholder in Phase 1.
    StartWorkflow {
        /// The workflow to start.
        spec: WorkflowSpec,
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
    RestartAllocation {
        /// Allocation to restart.
        alloc_id: AllocationId,
        /// Resources / command / args / identity for the workload.
        spec: AllocationSpec,
        /// Workload-kind discriminator per ADR-0047 §1.
        kind: WorkloadKind,
        /// Why the reconciler decided to restart this alloc.
        ///
        /// `None` for the pre-existing `WorkloadLifecycle` crash-loop
        /// restart pathway (a Job/Service post-spawn crash with budget
        /// remaining — the restart cause is implicit in the prior
        /// alloc's terminal). `Some(_)` is the Service-lifecycle
        /// liveness-driven restart (step 03-02 / Slice 05): the
        /// `service-lifecycle` reconciler observed a liveness probe's
        /// consecutive-failure count reach its `failure_threshold` and
        /// stamps the cause so downstream surfaces (audit row,
        /// operator render) can name *why* the restart fired.
        ///
        /// Additive `Option` keeps the existing `WorkloadLifecycle`
        /// emit site + the `action_shim` consumer unchanged — they
        /// neither construct nor read the reason; the shim's
        /// stop+start semantics are identical regardless of cause per
        /// ADR-0023 §2 / ADR-0037 §4 (RestartAllocation is never a
        /// terminal claim).
        reason: Option<RestartReason>,
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
}

/// Why a reconciler emitted [`Action::RestartAllocation`].
///
/// Carried as `Option<RestartReason>` on the action so the
/// pre-existing `WorkloadLifecycle` crash-loop restart pathway can
/// keep emitting `reason: None` unchanged (the restart cause is
/// implicit in the prior alloc's terminal there). The
/// `service-lifecycle` reconciler stamps `Some(_)` so the
/// liveness-driven restart names its cause.
///
/// `#[non_exhaustive]` per ADR-0037 §5 / ADR-0055 §7 — future
/// restart causes (e.g. a Phase-2 `LivenessRestartGovernor`
/// rate-limit verdict) append at the tail; external `match` sites
/// carry a wildcard arm so adding a variant is a non-breaking
/// minor bump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestartReason {
    /// Service-lifecycle liveness probe at `probe_idx` reached its
    /// `failure_threshold` consecutive failures on a `Running` alloc.
    ///
    /// `consecutive_failures` is the observed count at the deciding
    /// tick (>= `threshold`); `threshold` is the live
    /// `failure_threshold` policy value the predicate was recomputed
    /// against this tick (per `.claude/rules/development.md`
    /// § "Persist inputs, not derived state" — the View persists the
    /// counter INPUT, never a `should_restart` bool). Per ADR-0055
    /// §7 / DDD-9 / P3-Q11 the Phase-1 reconciler emits the restart
    /// unconditionally — there is no cascading-restart governor;
    /// composition with the shared `RESTART_BACKOFF_CEILING` budget
    /// (`WorkloadLifecycle`) caps the crash loop and surfaces
    /// `BackoffExhausted` once the budget is spent.
    LivenessExhausted {
        /// 0-indexed liveness probe whose streak hit the threshold.
        probe_idx: u32,
        /// Observed consecutive-failure count at the deciding tick.
        consecutive_failures: u32,
        /// The live `failure_threshold` the predicate compared against.
        threshold: u32,
    },
}

/// Placeholder for the workflow spec. Phase 3 replaces with real shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSpec;

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
const CANONICAL_TARGET_PREFIXES: &[&str] = &["job/", "node/", "alloc/", "service/"];

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
// AnyReconciler — enum-dispatch replacement for Box<dyn Reconciler>
// ---------------------------------------------------------------------------

/// Enum-dispatched wrapper over every first-party reconciler kind.
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// First real (non-proof-of-life) reconciler.
    WorkloadLifecycle(WorkloadLifecycle),
    /// Phase 2 — `service-map-hydrator`.
    ServiceMapHydrator(ServiceMapHydrator),
    /// Phase 2.2 — `backend-discovery-bridge`.
    BackendDiscoveryBridge(BackendDiscoveryBridge),
    /// Service-health-check-probes — `service-lifecycle` per
    /// ADR-0055. See [`crate::service_lifecycle::ServiceLifecycleReconciler`].
    ServiceLifecycle(ServiceLifecycleReconciler),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            Self::WorkloadLifecycle(r) => r.name(),
            Self::ServiceMapHydrator(r) => r.name(),
            Self::BackendDiscoveryBridge(r) => r.name(),
            Self::ServiceLifecycle(r) => r.name(),
        }
    }

    /// Canonical name as the inner reconciler's `Self::NAME` const —
    /// a `&'static str` aliased to the binary's data segment.
    #[must_use]
    pub const fn static_name(&self) -> &'static str {
        match self {
            Self::NoopHeartbeat(_) => <NoopHeartbeat as Reconciler>::NAME,
            Self::WorkloadLifecycle(_) => <WorkloadLifecycle as Reconciler>::NAME,
            Self::ServiceMapHydrator(_) => <ServiceMapHydrator as Reconciler>::NAME,
            Self::BackendDiscoveryBridge(_) => <BackendDiscoveryBridge as Reconciler>::NAME,
            Self::ServiceLifecycle(_) => <ServiceLifecycleReconciler as Reconciler>::NAME,
        }
    }

    /// Pure compute phase — dispatches to the inner reconciler's
    /// `reconcile`.
    #[must_use]
    pub fn reconcile(
        &self,
        desired: &AnyState,
        actual: &AnyState,
        view: &AnyReconcilerView,
        tick: &TickContext,
    ) -> (Vec<Action>, AnyReconcilerView) {
        match (self, desired, actual, view) {
            (Self::NoopHeartbeat(r), AnyState::Unit, AnyState::Unit, AnyReconcilerView::Unit) => {
                let (actions, ()) = r.reconcile(&(), &(), &(), tick);
                (actions, AnyReconcilerView::Unit)
            }
            (
                Self::WorkloadLifecycle(r),
                AnyState::WorkloadLifecycle(desired),
                AnyState::WorkloadLifecycle(actual),
                AnyReconcilerView::WorkloadLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::WorkloadLifecycle(next_view))
            }
            (
                Self::ServiceMapHydrator(r),
                AnyState::ServiceMapHydrator(desired),
                AnyState::ServiceMapHydrator(actual),
                AnyReconcilerView::ServiceMapHydrator(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::ServiceMapHydrator(next_view))
            }
            (
                Self::BackendDiscoveryBridge(r),
                AnyState::BackendDiscoveryBridge(desired),
                AnyState::BackendDiscoveryBridge(actual),
                AnyReconcilerView::BackendDiscoveryBridge(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::BackendDiscoveryBridge(next_view))
            }
            (
                Self::ServiceLifecycle(r),
                AnyState::ServiceLifecycle(desired),
                AnyState::ServiceLifecycle(actual),
                AnyReconcilerView::ServiceLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::ServiceLifecycle(next_view))
            }
            _ => {
                panic!(
                    "AnyReconciler::reconcile dispatch mismatch — \
                    runtime supplied incompatible (reconciler, state, view) triple"
                )
            }
        }
    }
}

/// Sum of every per-reconciler `View` shape held by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyReconcilerView {
    /// The `View = ()` variant used by Phase 1 reconcilers
    /// (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's view.
    WorkloadLifecycle(WorkloadLifecycleView),
    /// `ServiceMapHydrator` reconciler's view.
    ServiceMapHydrator(ServiceMapHydratorView),
    /// `BackendDiscoveryBridge` reconciler's view.
    BackendDiscoveryBridge(BackendDiscoveryBridgeView),
    /// `ServiceLifecycle` reconciler's view per ADR-0055 § 3 / DDD-5.
    /// Carries inputs only (counters / once-only Stable-announcement
    /// set) — derived state (`Stable` predicate, deadlines) is
    /// recomputed every tick.
    ServiceLifecycle(ServiceLifecycleView),
}
