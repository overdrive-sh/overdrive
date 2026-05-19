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
//! use overdrive_core::reconciler::{
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

use std::collections::{BTreeMap, BTreeSet};

use crate::SpiffeId;
use crate::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::id::{
    AllocationId, ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip, WorkloadId,
};
use crate::traits::dataplane::Backend;
use crate::traits::driver::{AllocationSpec, Resources};
use crate::traits::observation_store::{AllocState, AllocStatusRow, ServiceHydrationStatus};
use crate::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use crate::wall_clock::UnixInstant;

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
    ///
    /// Reconcilers with no meaningful projection pick `type State =
    /// ()`; the first real reconciler (`WorkloadLifecycle`) picks
    /// `type State = WorkloadLifecycleState`.
    type State: Send + Sync;

    /// Author-declared projection of the reconciler's private memory.
    /// Per ADR-0035 §1 the runtime owns persistence end-to-end: the
    /// `View` is bulk-loaded into an in-memory `BTreeMap` at boot via
    /// `ViewStore::bulk_load`, served from RAM on every tick, and
    /// written through to redb on every successful `reconcile` whose
    /// returned `next_view` differs from the in-memory value. The five
    /// bounds — `Serialize + DeserializeOwned + Default + Clone + Eq`
    /// plus the `Send + Sync` shared with the rest of the trait —
    /// give the runtime everything it needs to (a) persist on
    /// write-through, (b) materialise on bulk-load, (c) construct a
    /// fresh entry when a target has no persisted row, (d) hand the
    /// same value to multiple readers, and (e) skip the per-tick
    /// fsync via runtime Eq-diff when a reconciler returns an
    /// unchanged view (the additive future extension §1 anticipated).
    type View: Serialize + DeserializeOwned + Default + Clone + Eq + Send + Sync;

    /// Canonical name. Used for `ViewStore` table keying and
    /// evaluation broker lookup.
    ///
    /// Per ADR-0035 §1 + ADR-0036 the name is the [`AnyReconciler`]
    /// registry key; match arms in [`AnyReconciler::name`] and
    /// [`AnyReconciler::reconcile`] dispatch on the variant that
    /// holds this name.
    fn name(&self) -> &ReconcilerName;

    /// Pure function over `(desired, actual, view, tick) ->
    /// (Vec<Action>, NextView)`. See whitepaper §18, ADR-0035 §1, and
    /// `.claude/rules/development.md` §Reconciler I/O.
    ///
    /// `view` is the in-memory `View` value the runtime bulk-loaded at
    /// boot (or `Self::View::default()` when no persisted row exists
    /// for `target`). The second element of the returned tuple is the
    /// next-view — the runtime compares it against `view` for equality
    /// (`PartialEq` on `&Self::View`); when equal, the runtime skips
    /// both the `ViewStore::write_through` fsync and the in-memory
    /// map update. When the next-view differs, the runtime persists
    /// the full value via `ViewStore::write_through` and then
    /// installs it into the in-memory map. Per the `TickContext`
    /// shape, `tick` is the single pure time input constructed by the
    /// runtime once per evaluation; reading `Instant::now()` /
    /// `SystemTime::now()` inside this body is banned.
    ///
    /// Purity contract: two invocations with the same inputs MUST
    /// produce byte-identical `(actions, next_view)` tuples. The
    /// ADR-0017 `reconciler_is_pure` invariant evaluates this as a
    /// twin-invocation check against the full reconciler registry.
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
///
/// Per ADR-0021 (the State-shape decision), this enum mirrors the
/// existing `AnyReconciler` and `AnyReconcilerView` dispatch shape —
/// every reconciler kind has a typed `State`, a typed `View`, and is
/// dispatched by enum match.
///
/// Phase 1 ships two variants:
///
/// - `Unit` — carried by reconcilers whose `desired`/`actual`
///   projections are degenerate. `NoopHeartbeat` uses this.
/// - `WorkloadLifecycle` — the first real reconciler's projection
///   (job + nodes + allocations). Lands in this DISTILL wave but
///   the `WorkloadLifecycleState` body is RED scaffold.
///
/// Compile-time exhaustiveness: a new reconciler variant whose
/// `State` does not have a matching `AnyState` arm produces a
/// non-exhaustive-match compile error in `AnyReconciler::reconcile`,
/// matching the existing `AnyReconcilerView` discipline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyState {
    /// `State = ()` variant for Phase 1 reconcilers that do not
    /// dereference their projection (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's typed projection — see
    /// [`WorkloadLifecycleState`].
    WorkloadLifecycle(WorkloadLifecycleState),
    /// `ServiceMapHydrator` reconciler's typed projection — see
    /// [`ServiceMapHydratorState`]. Phase 2 (Slice 08; ASR-2.2-04).
    ServiceMapHydrator(ServiceMapHydratorState),
}

/// Desired/actual projection consumed by `WorkloadLifecycle::reconcile`.
/// Hydrated by the runtime from `IntentStore` (job + nodes) and
/// `ObservationStore` (allocations) per ADR-0021.
///
/// The same struct serves both `desired` and `actual` — the
/// reconciler interprets `desired.job` as "what should exist" and
/// `actual.allocations` as "what is currently running." Field shapes
/// are pinned by ADR-0021 §1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLifecycleState {
    /// Kind-agnostic workload identity, always available from the
    /// `TargetResource` at hydration time.
    pub workload_id: WorkloadId,
    /// The target job. `None` when the desired-state read returned
    /// no row (job was deleted) or the actual-state read found no
    /// surviving row to project against.
    pub job: Option<Job>,
    /// Whether a stop intent has been recorded for this job (i.e.
    /// `IntentKey::for_workload_stop(<id>)` is populated). When true and
    /// `job` is `Some`, the reconciler's Stop branch fires —
    /// emitting `Action::StopAllocation` for every Running alloc.
    /// Set false on the actual side; only the desired-side hydrator
    /// sets it. Per ADR-0027 / US-03 step 02-04.
    pub desired_to_stop: bool,
    /// Registered nodes with their declared capacity. Drives the
    /// scheduler input map. Phase 1 single-node has exactly one
    /// entry; the `BTreeMap` discipline holds at N=1.
    pub nodes: BTreeMap<NodeId, Node>,
    /// Current allocations belonging to this job, keyed by alloc id.
    /// Read from `ObservationStore::alloc_status_rows` filtered by
    /// `workload_id`. Empty when no allocations yet exist.
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,
    /// Workload kind discriminator per ADR-0047 §1 / ADR-0037 Amendment
    /// 2026-05-10. Drives the natural-exit branching in
    /// [`WorkloadLifecycle::reconcile`]:
    ///
    /// - `WorkloadKind::Job` — a terminal alloc (clean exit OR crash)
    ///   is the *natural* end of the workload. The reconciler emits
    ///   `Action::FinalizeFailed` carrying
    ///   `Some(TerminalCondition::Completed { exit_code: 0 })` for a
    ///   `Stopped { by: Process }` row, and
    ///   `Some(TerminalCondition::Failed { exit_code: N })` for a
    ///   crashed row. No restart attempts.
    /// - `WorkloadKind::Service` (and `WorkloadKind::Schedule` —
    ///   Phase 1 ships no schedule-firing reconciler logic) — preserves
    ///   the existing restart-budget semantics; a Failed alloc with
    ///   budget remaining flows through `RestartAllocation`, exhausting
    ///   the budget produces `FinalizeFailed { BackoffExhausted }`.
    ///
    /// Hydrated by `reconciler_runtime::hydrate_desired` from the
    /// active `WorkloadSpec` variant. Phase 1 default for legacy
    /// callers is `WorkloadKind::Service` (the kind-agnostic shape
    /// today's reconciler emulates).
    pub workload_kind: WorkloadKind,
    /// Content-addressed `spec_digest` for the workload (SHA-256 over
    /// the rkyv-archived `WorkloadIntent` payload per ADR-0050). Set
    /// to `Some(...)` by `reconciler_runtime::hydrate_desired` when
    /// the workload is a Service (`workload_kind == Service`); `None`
    /// for Job / Schedule kinds and for absent jobs.
    ///
    /// The reconciler reads this on the Service-arm release path:
    /// when an allocation has reached a terminal-state observation
    /// row and `service_spec_digest` is `Some(digest)`, the reconciler
    /// emits `Action::ReleaseServiceVip { spec_digest: digest, .. }`
    /// — gated by `view.released_for_terminal` so the emission is
    /// exactly-once per digest (per ADR-0049 amended 2026-05-15 +
    /// service-vip-allocator step 03-01).
    ///
    /// Set on BOTH the desired and actual sides by the runtime
    /// hydrator (the desired-side hydrator reads `WorkloadIntent`
    /// from `IntentStore`; the actual-side projection mirrors the
    /// desired-side value so the reconciler can read either).
    pub service_spec_digest: Option<crate::id::ContentHash>,
}

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// Actions a reconciler can emit. Phase 1 ships `Noop`, `HttpCall`, and a
/// `StartWorkflow` placeholder (workflow runtime lands Phase 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The reconciler has nothing to do this tick. The `noop-heartbeat`
    /// reconciler emits this always.
    Noop,

    /// An external HTTP call. The runtime shim executes this in Phase 3;
    /// Phase 1 reconcilers that emit this are responsible for reading
    /// the result from observation on the next tick per
    /// `development.md` §Reconciler I/O.
    HttpCall {
        /// Cause-to-response linkage. Derived from
        /// `(reconciliation_target, spec_hash, purpose)` per
        /// `development.md` §Reconciler I/O so the next tick's
        /// `hydrate` + `reconcile` pair can find the prior response
        /// deterministically.
        correlation: CorrelationKey,
        /// Target URL. `String` rather than `http::Uri` per ADR-0013 §4
        /// — the runtime shim parses this, keeping the transport dep
        /// off the core compile path.
        target: String,
        /// HTTP method. `String` rather than `http::Method` for the
        /// same reason as `target`.
        method: String,
        /// Request body bytes.
        body: Bytes,
        /// Per-attempt timeout.
        timeout: Duration,
        /// Idempotency key supplied to the remote API when supported.
        /// The runtime executes `HttpCall` at-least-once; remote-side
        /// idempotency is what makes the effect exactly-once per
        /// `development.md` §Reconciler I/O.
        idempotency_key: Option<String>,
    },

    /// Start a workflow. `WorkflowSpec` is a placeholder in Phase 1;
    /// workflow runtime lands Phase 3.
    StartWorkflow {
        /// The workflow to start. Phase 1 placeholder — see
        /// [`WorkflowSpec`].
        spec: WorkflowSpec,
        /// Cause-to-response linkage per [`Action::HttpCall`].
        correlation: CorrelationKey,
    },

    // -----------------------------------------------------------------
    // phase-1-first-workload — allocation-management variants
    // (US-03, ADR-0023). The action shim's
    // `dispatch(actions, ...)` consumes these and calls
    // `Driver::start` / `Driver::stop` per ADR-0023.
    // -----------------------------------------------------------------
    /// Start a fresh allocation for a job. Emitted by the
    /// `WorkloadLifecycle` reconciler when `desired.replicas >
    /// actual.replicas_running`.
    StartAllocation {
        /// Newly-minted allocation identifier (the reconciler reads
        /// this from its hydrated view; the view used the runtime's
        /// seeded `Entropy` port to mint it).
        alloc_id: AllocationId,
        /// Owning job.
        workload_id: WorkloadId,
        /// Placement decision from `overdrive-scheduler::schedule`.
        node_id: NodeId,
        /// Resources / command / args / identity for the workload. The action
        /// shim passes this directly to `Driver::start`.
        spec: AllocationSpec,
        /// Workload-kind discriminator per ADR-0047 §1 / step 02-02
        /// [D4]. The action shim denormalises this onto the emitted
        /// `AllocStatusRow.kind` field so the render layer can branch
        /// on kind without re-fetching intent. Sourced from
        /// [`WorkloadLifecycleState::workload_kind`] at emit time.
        kind: WorkloadKind,
    },
    /// Stop a Running allocation. Emitted by the `WorkloadLifecycle`
    /// reconciler when desired state is "stopped" (set by
    /// `IntentKey::for_workload_stop`).
    ///
    /// Per ADR-0037 §4 the variant carries a typed
    /// [`TerminalCondition`] flag the action shim writes onto
    /// `AllocStatusRow.terminal` AND echoes onto `LifecycleEvent`.
    /// The reconciler is the *single source* of every terminal claim;
    /// emission sites outside a reconciler tick (the action-shim
    /// heartbeat, the exit observer) emit `terminal: None`. When a
    /// stop is operator-initiated (`desired.desired_to_stop` set
    /// by `IntentKey::for_workload_stop`), the reconciler stamps
    /// `Some(TerminalCondition::Stopped { by: StoppedBy::Operator })`
    /// here — the by-source is already known from the desired state,
    /// so the action shim never re-derives it.
    StopAllocation {
        /// Target allocation. The action shim looks up the
        /// `AllocationHandle` via observation store.
        alloc_id: AllocationId,
        /// Reconciler-decided terminal claim per ADR-0037 §4. `None`
        /// when the stop is non-terminal (Phase 2+ cluster-driven
        /// drains may end up here); `Some(Stopped { by: Operator })`
        /// when the stop is operator-initiated.
        terminal: Option<TerminalCondition>,
    },
    /// Restart an allocation — semantically a `StopAllocation`
    /// followed by a fresh `StartAllocation` with the same `alloc_id`.
    /// Emitted by the `WorkloadLifecycle` reconciler in crash-recovery
    /// scenarios (per US-03 Domain Example 2).
    ///
    /// Per ADR-0031 §5 the variant carries a fully-populated
    /// `AllocationSpec` — mirroring `StartAllocation { spec }`. The
    /// reconciler has the live `Job` in scope at emit time, so the
    /// spec is constructed there (in pure code) and the action shim
    /// reads it straight off the action. The shim's
    /// `build_phase1_restart_spec`, `build_identity`, and
    /// `default_restart_resources` helpers are deleted in the same PR.
    RestartAllocation {
        /// Allocation to restart.
        alloc_id: AllocationId,
        /// Resources / command / args / identity for the workload —
        /// mirrors [`Action::StartAllocation::spec`]. The action shim
        /// passes this directly to `Driver::start`.
        spec: AllocationSpec,
        /// Workload-kind discriminator per ADR-0047 §1 / step 02-02
        /// [D4]. Mirrors [`Action::StartAllocation::kind`].
        kind: WorkloadKind,
    },

    /// Finalize a failed allocation as terminal — the synthetic
    /// Failed-row action per ADR-0023 / ADR-0037 §4.
    ///
    /// Emitted by the `WorkloadLifecycle` reconciler at the deciding tick
    /// when `attempts >= RESTART_BACKOFF_CEILING`: the reconciler has
    /// concluded the allocation will not be restarted and the row
    /// should be flipped to terminal-Failed. The action shim consumes
    /// this in step 02-02 to write `AllocStatusRow { state: Failed,
    /// terminal: Some(BackoffExhausted { attempts }), .. }` and to
    /// emit the matching `LifecycleEvent.terminal` broadcast — both
    /// surfaces carry the same value from the same dispatch site,
    /// per ADR-0037 §4.
    ///
    /// `terminal` is always `Some(...)` on this variant by
    /// construction (a `None` here would mean "finalize as failed
    /// but make no terminal claim", which is structurally
    /// nonsensical). The `Option` type is preserved for
    /// shape-uniformity with `StopAllocation` and to leave the door
    /// open for future non-`BackoffExhausted` finalisation paths
    /// (e.g. a Phase-2 right-sizing cap).
    FinalizeFailed {
        /// Allocation to finalize. The action shim writes the
        /// terminal Failed row against this id.
        alloc_id: AllocationId,
        /// Reconciler-decided terminal claim. Always `Some(...)` on
        /// emission today; the `Option` shape mirrors
        /// [`Action::StopAllocation::terminal`].
        terminal: Option<TerminalCondition>,
    },

    // -----------------------------------------------------------------
    // phase-2-xdp-service-map (Slice 08, US-08, ASR-2.2-04) — emitted
    // by the `service-map-hydrator` reconciler when the
    // `service_backends` ObservationStore rows for a `ServiceId`
    // produce a fingerprint distinct from the one persisted in the
    // reconciler's `View`.
    // -----------------------------------------------------------------
    /// Replace the backend set for a service VIP in the kernel-side
    /// `SERVICE_MAP` / `BACKEND_MAP` / `MAGLEV_MAP` tuple per
    /// `docs/feature/phase-2-xdp-service-map/design/architecture.md`
    /// § 7.
    ///
    /// The action shim consumes this variant, invokes
    /// `Dataplane::update_service(service_id, vip, backends)`,
    /// and writes the outcome into the `service_hydration_results`
    /// observation row. The next reconcile tick reads that row via
    /// `actual` and either advances (Completed) or retries on the
    /// next backend-set change (Failed).
    ///
    /// `Vec<Backend>` carries weighted backends in deterministic
    /// `BTreeMap<BackendId, Backend>::iter()` order — Maglev table
    /// generation is byte-deterministic across nodes given identical
    /// inputs (DISCUSS Decision 8 + architecture.md Constraint 6).
    DataplaneUpdateService {
        /// Identity of the service whose backend set is being
        /// rewritten. Maps 1:1 to a `MAGLEV_MAP` outer-map key.
        service_id: crate::id::ServiceId,
        /// Virtual IP the kernel-side XDP program matches incoming
        /// packets against. Carried explicitly (rather than re-derived
        /// from `service_id`) so the shim never needs to look back at
        /// `service_backends` to dispatch.
        vip: crate::id::ServiceVip,
        /// Backend set, in deterministic iteration order. The shim
        /// passes this slice straight into
        /// `Dataplane::update_service`; userspace Maglev permutation
        /// generation reads it in this exact order.
        backends: Vec<crate::traits::dataplane::Backend>,
        /// Cause-to-response linkage per the existing `HttpCall`
        /// pattern. Derived deterministically from
        /// `(target = "service-map-hydrator/<service_id>",
        ///   spec_hash = ContentHash::of(rkyv-archive of fingerprint),
        ///   purpose = "update-service")` so the next tick can locate
        /// the `service_hydration_results` row deterministically.
        correlation: CorrelationKey,
    },

    // -----------------------------------------------------------------
    // service-vip-allocator step 03-01 — ReleaseServiceVip per
    // ADR-0049 (amended 2026-05-15).
    //
    // Emitted by the `WorkloadLifecycle` reconciler when a Service-kind
    // workload's allocation reaches a terminal-state observation row
    // (i.e. `row.terminal.is_some()`) AND the workload's `spec_digest`
    // has not yet been recorded in `view.released_for_terminal`. The
    // gate is recomputed every tick from the persisted input ("we
    // already emitted release for this digest") — never cached as a
    // derived "needs release now" boolean, per
    // `.claude/rules/development.md` § "Persist inputs, not derived
    // state".
    //
    // The action shim's per-arm dispatch lands in step 03-02; the
    // end-to-end submit → terminal → release → reallocate flow lands
    // in step 03-03. This variant exists in step 03-01 so the
    // reconciler can emit it before the dispatch arm goes GREEN.
    // -----------------------------------------------------------------
    /// Release a VIP from the `ServiceVipAllocator` memo. Carries the
    /// content-addressed `spec_digest` the allocator uses as its memo
    /// key (per ADR-0049 amended 2026-05-15). The action shim invokes
    /// `ServiceVipAllocator::release(spec_digest)` on dispatch (step
    /// 03-02).
    ReleaseServiceVip {
        /// Content-addressed spec digest — SHA-256 over the rkyv-archived
        /// `WorkloadIntent` payload per ADR-0050. Used as the
        /// `ServiceVipAllocator` memo key.
        spec_digest: ContentHash,
        /// Cause-to-response linkage per the existing `HttpCall`
        /// pattern. Derived from
        /// `(target = "job-lifecycle/<workload_id>",
        ///   spec_hash = spec_digest,
        ///   purpose = "release-service-vip")` so the action shim
        /// (step 03-02) can correlate the dispatch with an observation
        /// row deterministically.
        correlation: CorrelationKey,
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

/// Canonical reconciler name. Kebab-case, `^[a-z][a-z0-9-]{0,62}$`. The
/// strict character set lets the libSQL path provisioner safely
/// concatenate the name into a filesystem path without sanitisation.
///
/// Per ADR-0013 §4 validation is hand-rolled char-by-char — no `regex`
/// crate dep on the core compile path. Path-traversal characters
/// (`.`, `/`, `\`, `:`) are rejected at the constructor, so any name
/// that parses here is safe to interpolate into a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReconcilerName(String);

impl ReconcilerName {
    /// Validating constructor. Rejects empty, uppercase, leading digit
    /// or hyphen, path-traversal characters (`.`, `/`, `\`, `:`), and
    /// any name longer than 63 bytes.
    pub fn new(raw: &str) -> Result<Self, ReconcilerNameError> {
        if raw.is_empty() {
            return Err(ReconcilerNameError::Empty);
        }
        if raw.len() > RECONCILER_NAME_MAX {
            return Err(ReconcilerNameError::TooLong { got: raw.len() });
        }

        let mut chars = raw.chars();
        // Safety: `raw.is_empty()` rejected above, so `.next()` is Some.
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
/// Exactly `[a-z0-9-]` — no uppercase, no dots, no slashes, no colons.
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
    /// Input contained a character outside `[a-z0-9-]`. Path-traversal
    /// characters (`.`, `/`, `\`, `:`) are rejected on this arm.
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

/// Canonical shapes accepted by `TargetResource::new`. Each variant
/// corresponds to one of the core aggregate identifier classes; any
/// other prefix is rejected with `UnknownShape`.
const CANONICAL_TARGET_PREFIXES: &[&str] = &["job/", "node/", "alloc/", "service/"];

/// Target-resource component of the evaluation broker's key.
///
/// The broker is keyed on `(ReconcilerName, TargetResource)` per
/// whitepaper §18. Phase 1 carries a canonical string form with prefix
/// validation; Phase 2+ may refine into a typed sum over concrete
/// resource kinds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetResource(String);

impl TargetResource {
    /// Validating constructor. Accepts canonical forms `job/<id>`,
    /// `node/<id>`, `alloc/<id>` with a non-empty id. Any other shape
    /// is rejected with `UnknownShape`.
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
    /// Input did not match any canonical prefix (`job/`, `node/`,
    /// `alloc/`) with a non-empty id component.
    #[error("target resource has unknown shape: {raw}")]
    UnknownShape {
        /// The rejected input, echoed back for diagnostics.
        raw: String,
    },
}

// ---------------------------------------------------------------------------
// NoopHeartbeat — Phase 1 proof-of-life reconciler
// ---------------------------------------------------------------------------

/// Phase 1 proof-of-life reconciler.
///
/// `NoopHeartbeat::reconcile` always emits `vec![Action::Noop]` and
/// an unchanged `()` next-view; `hydrate` is a trivial `Ok(())`. The
/// reconciler serves as the fixture against which the
/// `ReconcilerIsPure` invariant's twin-invocation check runs and as the
/// seed entry for the `AtLeastOneReconcilerRegistered` invariant.
///
/// The struct lives in `overdrive-core::reconciler` (rather than
/// in `overdrive-control-plane`) because `AnyReconciler` — the enum
/// that replaces `Box<dyn Reconciler>` — holds the concrete type in
/// its `NoopHeartbeat` variant.
pub struct NoopHeartbeat {
    name: ReconcilerName,
}

impl NoopHeartbeat {
    /// Construct the canonical `noop-heartbeat` instance. Named
    /// constructor rather than `Default` because the name is not
    /// defaultable — it carries the canonical string literal.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule. Failure
    /// would indicate a bug in the newtype constructor.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'noop-heartbeat' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for NoopHeartbeat {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "noop-heartbeat";

    // Per ADR-0021, reconcilers with no meaningful projection pick
    // `type State = ()`. `NoopHeartbeat` ignores `desired`/`actual`
    // entirely and always emits `Action::Noop`.
    type State = ();
    // Per ADR-0035 §1, `View` carries `Serialize + DeserializeOwned +
    // Default + Clone + Send + Sync`. `()` satisfies them trivially.
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        (vec![Action::Noop], ())
    }
}

// ---------------------------------------------------------------------------
// AnyReconciler — enum-dispatch replacement for Box<dyn Reconciler>
// ---------------------------------------------------------------------------

/// Enum-dispatched wrapper over every first-party reconciler kind.
///
/// Erases the per-reconciler `(State, View)` associated-type pair so
/// the runtime can hold a heterogeneous registry. Per ADR-0035 §1 the
/// trait itself is dyn-compatible for any *fixed* `(State, View)`
/// pair, but a registry with multiple kinds needs the enum dispatch.
/// Adding a reconciler means adding a variant here and a match arm in
/// each of `name` and `reconcile`.
///
/// Phase 1 ships two variants: `NoopHeartbeat` (proof-of-life) and
/// `WorkloadLifecycle` (the first real reconciler).
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// First real (non-proof-of-life) reconciler. Converges declared
    /// replica count for a `Job` — see [`WorkloadLifecycle`].
    WorkloadLifecycle(WorkloadLifecycle),
    /// Phase 2 (Slice 08; ASR-2.2-04) — `service-map-hydrator`.
    /// Activates J-PLAT-004 per ADR-0042. See [`ServiceMapHydrator`].
    ServiceMapHydrator(ServiceMapHydrator),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            Self::WorkloadLifecycle(r) => r.name(),
            Self::ServiceMapHydrator(r) => r.name(),
        }
    }

    /// Canonical name as the inner reconciler's `Self::NAME` const —
    /// a `&'static str` aliased to the binary's data segment.
    ///
    /// This is the surface the runtime hands to
    /// `ViewStore::{bulk_load_bytes, write_through_bytes, delete}`,
    /// whose `reconciler` parameter is typed `&'static str` per the
    /// `refactor-reconciler-static-name` RCA. Going through
    /// `name(&self).as_str()` instead would produce a `&str` borrowed
    /// from the inner `ReconcilerName`'s `String` — non-`'static` —
    /// and the redb `TableDefinition::new` call requires a static
    /// lifetime on the table name. The match arms below are
    /// exhaustive over `AnyReconciler` variants, so adding a new
    /// reconciler kind without declaring its `NAME` const fails to
    /// compile here, not silently at runtime.
    #[must_use]
    pub const fn static_name(&self) -> &'static str {
        match self {
            Self::NoopHeartbeat(_) => <NoopHeartbeat as Reconciler>::NAME,
            Self::WorkloadLifecycle(_) => <WorkloadLifecycle as Reconciler>::NAME,
            Self::ServiceMapHydrator(_) => <ServiceMapHydrator as Reconciler>::NAME,
        }
    }

    /// Pure compute phase — dispatches to the inner reconciler's
    /// `reconcile`. The caller supplies the matching state and view
    /// variants.
    ///
    /// Variant alignment is a compile-time invariant: the dispatch
    /// `match` below is exhaustive, and every arm pairs an
    /// `AnyReconciler` variant with its declared [`AnyState`] /
    /// [`AnyReconcilerView`] counterparts. Adding a new reconciler
    /// variant whose `State` or `View` type does not line up with a
    /// matching `AnyState` / `AnyReconcilerView` arm produces a
    /// non-exhaustive-match compile error, forcing the developer to
    /// extend the dispatch explicitly. There is no runtime fallback —
    /// a mismatched triple cannot be constructed in the first place
    /// once the runtime's hydrate-desired / hydrate-actual paths are
    /// wired (Phase 02-02+).
    ///
    /// Per ADR-0021, `state` is a single `&AnyState` parameter
    /// (replacing the prior `&State, &State` placeholder pair). The
    /// runtime hydrates the matching variant for both desired and
    /// actual; under the symmetric per-reconciler model, the inner
    /// reconciler receives two `&Self::State` references that live
    /// inside the same `AnyState` variant.
    ///
    /// **Phase 02-01 caller contract**: the runtime tick loop has not
    /// shipped yet, so callers (the sim invariant evaluator and the
    /// runtime acceptance test) construct `AnyState::Unit` directly.
    /// Phase 02-02 lands the action shim and `WorkloadLifecycle::reconcile`
    /// body; Phase 02-03 lands the runtime tick loop that builds
    /// `AnyState::WorkloadLifecycle(...)` from the `IntentStore` /
    /// `ObservationStore` reads.
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
            // WorkloadLifecycle dispatch — types align by construction
            // when the runtime hydrates matching desired/actual/view
            // variants. Step 02-03 lands the runtime tick loop that
            // produces these triples; the body of `reconcile` itself
            // is fully implemented as of step 02-02.
            (
                Self::WorkloadLifecycle(r),
                AnyState::WorkloadLifecycle(desired),
                AnyState::WorkloadLifecycle(actual),
                AnyReconcilerView::WorkloadLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::WorkloadLifecycle(next_view))
            }
            // Phase 2 — `service-map-hydrator` dispatch.
            (
                Self::ServiceMapHydrator(r),
                AnyState::ServiceMapHydrator(desired),
                AnyState::ServiceMapHydrator(actual),
                AnyReconcilerView::ServiceMapHydrator(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::ServiceMapHydrator(next_view))
            }
            // Cross-variant branches — statically impossible once the
            // runtime correctly hydrates matching state and view kinds.
            // The runtime tick loop ships in 02-03; until then these
            // arms cannot be reached from any caller, but the match
            // must remain exhaustive so a future variant addition is a
            // compile error rather than a silent runtime panic.
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
///
/// Phase 1 originally only had `View = ()` (the `Unit` variant); the
/// phase-1-first-workload DISTILL added the `WorkloadLifecycle` arm. Per
/// ADR-0035 §1 the runtime owns the cache (bulk-loaded at boot via
/// `ViewStore::bulk_load`, written through after each `reconcile`);
/// reconcilers see a typed `&Self::View`, never the erased
/// `AnyReconcilerView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyReconcilerView {
    /// The `View = ()` variant used by Phase 1 reconcilers
    /// (`NoopHeartbeat`).
    Unit,
    /// `WorkloadLifecycle` reconciler's view — see [`WorkloadLifecycleView`].
    WorkloadLifecycle(WorkloadLifecycleView),
    /// `ServiceMapHydrator` reconciler's view — see
    /// [`ServiceMapHydratorView`]. Phase 2 (Slice 08; ASR-2.2-04).
    ServiceMapHydrator(ServiceMapHydratorView),
}

// ---------------------------------------------------------------------------
// WorkloadLifecycle reconciler — first real reconciler (US-03)
// ---------------------------------------------------------------------------

/// Maximum restart attempts before `WorkloadLifecycle` gives up on an alloc.
///
/// Past this count the reconciler stops emitting `RestartAllocation`
/// for a persistently failing alloc. Per US-03 step 02-03 — the
/// ceiling exists to keep a repeatedly-crashing workload from
/// consuming infinite driver resources.
pub const RESTART_BACKOFF_CEILING: u32 = 5;

/// Backoff window between successive `RestartAllocation` emissions
/// for the same alloc.
///
/// Per US-03 Domain Example 2 (user-stories.md:421-424) the deadline
/// is `tick.now + initial_backoff` — singular, no progression. One
/// second balances transient-hiccup tolerance (slow startup,
/// dependency flap) against operator visibility within Phase 1's
/// single-node envelope: 1 s × `RESTART_BACKOFF_CEILING` = ~5 s
/// wall-clock to "Failed (backoff exhausted)".
pub const RESTART_BACKOFF_DURATION: Duration = Duration::from_secs(1);

/// Per-attempt restart backoff policy lookup.
///
/// **Today this is degenerate-constant**: every `attempt` value
/// yields the same [`RESTART_BACKOFF_DURATION`]. The function exists
/// as a stability anchor so call sites stay unchanged when
/// operator-configurable per-job policy lands — TODO(#137), deferred
/// from #141's 'Out' section. The leading underscore on `_attempt`
/// is deliberate: the parameter is currently unused (degenerate
/// policy ignores attempt count) but lives in the signature so a
/// future progressive-backoff schedule (e.g.
/// `RESTART_BACKOFF_DURATION * 2_u32.pow(attempt)`) does not require
/// a breaking API change.
///
/// TODO(#137): operator-configurable per-job policy will thread a
/// `&RestartPolicy` through this signature rather than relying on
/// the workspace-global constant.
///
/// Persist-inputs discipline: callers MUST persist the *attempt
/// count* (and a `last_failure_seen_at` timestamp), not the deadline
/// this function computes from them — see
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state". Recomputing on every read picks up future policy changes
/// without a schema migration.
#[must_use]
pub const fn backoff_for_attempt(_attempt: u32) -> Duration {
    RESTART_BACKOFF_DURATION
}

/// The Phase 1 first real reconciler. Converges declared replica
/// count for a `Job` against the running `AllocStatusRow` set.
///
/// Trait shape pinned by ADR-0021; convergence + backoff logic per
/// US-03 (phase-1-first-workload, slice 3).
///
/// The reconciler reads `desired.job` (the target job) and
/// `actual.allocations` (running set), calls
/// `overdrive_scheduler::schedule(...)` on `desired.nodes` +
/// `desired.job`, and emits `Action::StartAllocation` /
/// `Action::StopAllocation` to converge. Restart counts are tracked
/// in `view.restart_counts`; backoff is gated by recomputing the
/// deadline as `view.last_failure_seen_at + backoff_for_attempt(...)`
/// against `tick.now_unix` (NEVER `Instant::now()` /
/// `SystemTime::now()`). Per `.claude/rules/development.md` §
/// "Persist inputs, not derived state".
pub struct WorkloadLifecycle {
    name: ReconcilerName,
}

impl WorkloadLifecycle {
    /// Construct the canonical `job-lifecycle` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'job-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for WorkloadLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "job-lifecycle";

    type State = WorkloadLifecycleState;
    type View = WorkloadLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    // Per ADR-0023 + ADR-0037 §4 the reconcile body is the single
    // dispatch surface for every WorkloadLifecycle decision branch (Stop,
    // Absent, Run → {Running, Operator-stopped, Job-natural-exit,
    // Restart-with-budget, NoCapacity-fresh-schedule}). Splitting it
    // into N helper fns would require threading every read of
    // `desired` / `actual` / `view` / `tick` through arguments;
    // each branch is short and self-contained, and the line count is
    // dominated by the inline-comment audit trail per the project's
    // documentation discipline (`.claude/rules/development.md`).
    #[allow(clippy::too_many_lines)]
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        // service-vip-allocator step 03-01 — Service-arm VIP release
        // per ADR-0049 (amended 2026-05-15).
        //
        // When the workload is a Service AND we have a spec_digest in
        // scope AND any allocation has reached a terminal-state
        // observation row (i.e. `row.terminal.is_some()`) AND the
        // digest has NOT already been recorded in
        // `view.released_for_terminal`, emit
        // `Action::ReleaseServiceVip` exactly once and stamp the digest
        // onto `next_view.released_for_terminal` so the next tick's
        // gate short-circuits. Per `.claude/rules/development.md`
        // § "Persist inputs, not derived state": the recorded set is
        // the input "we already emitted release for this digest" —
        // never a derived "needs release now" boolean.
        //
        // The release decision is independent of (and additive to)
        // the existing Stop / Absent / Run branches below: a terminal
        // operator-stopped Service alloc flows through the Run-branch
        // operator-stopped short-circuit (returns no other actions)
        // AND ALSO emits the release here. The two-action shape is
        // intentional — the reconciler is the single source of every
        // terminal claim, and Service-VIP release is a terminal claim
        // per ADR-0049.
        let release_pair = service_vip_release_emission(desired, actual, view);

        let (mut actions, mut next_view) = Self::reconcile_inner(desired, actual, view, tick);
        if let Some((release_action, released_digest)) = release_pair {
            actions.push(release_action);
            next_view.released_for_terminal.insert(released_digest);
        }
        (actions, next_view)
    }
}

impl WorkloadLifecycle {
    // The original reconcile body, factored out so the Service-VIP
    // release branch can wrap it without duplicating every branch's
    // return tuple. The inner method's contract is unchanged from the
    // pre-step-03-01 shape; the wrapper above is the only Service-arm
    // augmentation. Associated function (no `&self`) because the
    // reconcile logic is purely a function of `(desired, actual, view,
    // tick)` — the reconciler instance carries only the name newtype.
    #[allow(clippy::too_many_lines)]
    fn reconcile_inner(
        desired: &WorkloadLifecycleState,
        actual: &WorkloadLifecycleState,
        view: &WorkloadLifecycleView,
        tick: &TickContext,
    ) -> (Vec<Action>, WorkloadLifecycleView) {
        // Per ADR-0021 + US-03 AC: handle Stop / Absent / Run branches.
        //
        // Stop: when a stop intent is recorded (`desired.desired_to_stop`)
        // AND a job spec exists, emit `Action::StopAllocation` for every
        // Running alloc. Allocs in any other state (Pending, Draining,
        // Terminated) require no action; the next tick's hydrate
        // re-evaluates. A stop intent against an absent job is a no-op
        // (the second `desired.job.is_some()` clause).
        //
        // Transitional-view-state contract (whitepaper §18 *Level-triggered
        // inside the reconciler* + `fix-stop-branch-backoff-pending` RCA):
        // when `stop_actions.is_empty()` the stop is complete — there is
        // nothing left for the runtime to do. Clearing
        // `last_failure_seen_at` is what tells the runtime's
        // `view_has_backoff_pending` predicate to stop re-enqueueing;
        // without it, a Failed-mid-backoff alloc keeps the predicate
        // `true` and the broker spins for ~5 s until `restart_counts`
        // reaches the ceiling. `restart_counts` is intentionally left
        // intact: the predicate only checks counts for entries that
        // exist in `last_failure_seen_at`, so clearing the
        // observation-timestamp map is sufficient — and the historical
        // record is preserved.
        if desired.desired_to_stop && desired.job.is_some() {
            // Per ADR-0037 §4: an operator-initiated stop is a
            // terminal moment. The reconciler stamps the
            // by-source onto every emitted StopAllocation — the
            // action shim writes the same value onto
            // AllocStatusRow.terminal AND echoes onto
            // LifecycleEvent.terminal in step 02-02.
            let operator_stop_terminal =
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
            let stop_actions: Vec<Action> = actual
                .allocations
                .values()
                .filter(|r| r.state == AllocState::Running)
                .map(|r| Action::StopAllocation {
                    alloc_id: r.alloc_id.clone(),
                    terminal: operator_stop_terminal.clone(),
                })
                .collect();
            // When nothing is Running, the stop is complete.
            // Clear backoff state so view_has_backoff_pending does not re-enqueue.
            let mut next_view = view.clone();
            if stop_actions.is_empty() {
                next_view.last_failure_seen_at.clear();
            }
            return (stop_actions, next_view);
        }
        match desired.job.as_ref() {
            // Absent: no desired job. The Stop branch above handles
            // explicit stops via `desired_to_stop`; the GC branch
            // here handles the case where the workload's intent has
            // been *withdrawn* entirely (hard-delete, multi-node
            // drain, crash-recovery surgery).
            //
            // GC branch (closes #148 per ADR-0037 Amendment
            // 2026-05-14): withdraw any non-terminal allocations by
            // stamping a system-GC terminal claim. Structural mirror
            // of the operator-Stop branch above (filter Running rows,
            // emit one StopAllocation per row, clear
            // `last_failure_seen_at` when no work remains).
            // `StoppedBy::SystemGc` is the load-bearing
            // discriminator: it lets the action shim, lifecycle
            // event consumers, and operator-facing surfaces
            // distinguish "the operator stopped this" from "the
            // system reaped this because no intent referenced it".
            //
            // Filter shape (architecture.md § 8 Open Q3): only
            // Running rows are stopped. A Pending row has no
            // driver-side runtime to stop; a Draining row is already
            // being torn down by the worker. Same shape as the
            // operator-Stop branch above; mutation tests pin the
            // filter at `state == Running`.
            //
            // Kind-agnostic: branches on `desired.job.is_none()`,
            // not on `desired.workload_kind`. An orphan-row scenario
            // can occur for any workload kind (architecture.md § 8
            // Open Q2).
            None => {
                let gc_terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc });
                let stop_actions: Vec<Action> = actual
                    .allocations
                    .values()
                    .filter(|r| r.state == AllocState::Running)
                    .map(|r| Action::StopAllocation {
                        alloc_id: r.alloc_id.clone(),
                        terminal: gc_terminal.clone(),
                    })
                    .collect();
                let mut next_view = view.clone();
                if stop_actions.is_empty() {
                    // No work left — clear backoff inputs so
                    // `view_has_backoff_pending` returns false and
                    // the broker stops re-enqueueing this target.
                    // Mirrors the Stop branch's view-cleanup shape;
                    // input clearance, not derived-deadline
                    // clearance, per `.claude/rules/development.md`
                    // § "Persist inputs, not derived state".
                    next_view.last_failure_seen_at.clear();
                }
                (stop_actions, next_view)
            }
            // Run: a job is desired.
            Some(job) => {
                // Pure first-fit placement (inlined from
                // overdrive-scheduler::schedule). Pulled inline rather
                // than calling the scheduler crate because
                // overdrive-core cannot depend on overdrive-scheduler
                // (would invert the dependency direction).
                let allocs_vec: Vec<&AllocStatusRow> = actual.allocations.values().collect();

                // Per workload-gc-absent-stale-allocs step 01-04: derive
                // a second view that excludes intentional-stop rows
                // (Operator OR SystemGc). Used by the running-alloc /
                // natural-exit / failed-alloc checks below so that a
                // re-submit after GC lands a fresh placement (the
                // architecture.md § 5 promise) rather than spuriously
                // restarting / finalizing the GC'd row. Operator-
                // stopped rows continue to short-circuit at the top
                // of the Run branch via the narrower
                // `is_operator_stopped` check (their semantics is
                // strictly stronger — see comment block below).
                let active_allocs_vec: Vec<&AllocStatusRow> =
                    allocs_vec.iter().filter(|r| !is_intentionally_stopped(r)).copied().collect();

                // Is any allocation already Running for this job? If so
                // we are converged — emit nothing. Failed allocs flow
                // into the restart-with-backoff branch below.
                let running_alloc =
                    active_allocs_vec.iter().find(|r| r.state == AllocState::Running);
                if running_alloc.is_some() {
                    return (Vec::new(), view.clone());
                }

                // Per `fix-exec-driver-exit-watcher` Step 01-02 RCA
                // §Bug 3: an Operator-stopped Terminated alloc is a
                // terminal intentional stop. The reconciler MUST NOT
                // schedule a fresh replacement allocation for the
                // job — operator stop intent is the load-bearing
                // discriminator and a fresh schedule would undo the
                // operator's stop. The alloc record remains in obs
                // as the terminal state until the operator explicitly
                // re-submits the job intent.
                //
                // (Distinct from `desired.desired_to_stop`, which is
                // a separate signal carried by `IntentKey::for_workload_stop`
                // and handled at the Stop branch above. The
                // Operator-stopped row arrives via the watcher's
                // `intentional_stop` flag — set by `Driver::stop`
                // even when no `for_job_stop` intent exists, e.g.
                // by direct CLI / API operator action.)
                //
                // **Asymmetry vs SystemGc** (workload-gc-absent-stale-
                // allocs step 01-04). SystemGc-stopped rows are NOT
                // short-circuited here — they are filtered out of
                // `active_allocs_vec` above so that the Run branch
                // falls through to fresh placement on resubmit. The
                // semantic difference: Operator stop is OVERRIDING
                // (operator's intent outranks the new submit; a
                // fresh schedule would undo the operator's stop),
                // while SystemGc stop is OVERRIDABLE (system stop
                // was system-initiated because intent disappeared;
                // a fresh submit IS the operator's overriding new
                // intent and should land a fresh allocation —
                // architecture.md § 5).
                if allocs_vec.iter().any(|r| is_operator_stopped(r)) {
                    return (Vec::new(), view.clone());
                }

                // Job-kind natural-exit handler per ADR-0037 Amendment
                // 2026-05-10 / ADR-0047 §1. A run-to-completion workload
                // (`WorkloadKind::Job`) terminates on the first observed
                // exit — clean OR crashed — and the reconciler emits
                // `Action::FinalizeFailed` carrying the typed terminal
                // claim. There are no restart attempts (the workload's
                // contract is "run once, until it exits"). Service-kind
                // (and Schedule-kind, Phase 1 no-op) flow through the
                // existing restart-budget branch below — preserves the
                // pre-feature kind-agnostic semantics that the Service
                // shape today emulates.
                //
                // Idempotency guard: if the row already carries a
                // Completed/Failed terminal claim the reconciler has
                // already finalised this alloc on a prior tick — do not
                // re-emit. Without this guard the action shim's
                // level-triggered re-enqueue would emit FinalizeFailed
                // every tick forever once the alloc reached terminal.
                if desired.workload_kind == WorkloadKind::Job {
                    if let Some(terminal_alloc) =
                        active_allocs_vec.iter().find(|r| is_natural_exit(r))
                    {
                        if matches!(
                            terminal_alloc.terminal,
                            Some(
                                TerminalCondition::Completed { .. }
                                    | TerminalCondition::Failed { .. }
                            )
                        ) {
                            return (Vec::new(), view.clone());
                        }
                        let typed = classify_natural_exit_terminal(terminal_alloc);
                        let action = Action::FinalizeFailed {
                            alloc_id: terminal_alloc.alloc_id.clone(),
                            terminal: Some(typed),
                        };
                        return (vec![action], view.clone());
                    }
                }

                // Failed alloc with attempt budget remaining and
                // backoff elapsed → emit RestartAllocation. Per US-03
                // the reconciler tracks restart attempts in
                // `view.restart_counts` and STOPS emitting
                // RestartAllocation once `RESTART_BACKOFF_CEILING` is
                // reached. The alloc then stays Terminated indefinitely
                // (backoff exhausted).
                // Per ADR-0032 §5 + slice 02 step 02-01: the action
                // shim now writes `AllocState::Failed` on driver
                // `StartRejected` (instead of `Terminated`) to
                // distinguish operator-stop from driver-could-not-
                // start. The restart-budget logic treats both states
                // identically — both are "this alloc is not Running
                // and the reconciler should consider restarting it"
                // — so the matcher includes both.
                //
                // Per `fix-exec-driver-exit-watcher` Step 01-02 RCA
                // §Bug 3: an alloc whose obs row carries
                // `reason: Some(Stopped { by: Operator })` is a
                // terminal intentional stop. The reconciler MUST NOT
                // restart it — operator stop intent is observed via
                // the watcher's `intentional_stop` flag (mapped to
                // `StoppedBy::Operator` by the worker `exit_observer`
                // subsystem) and is the load-bearing discriminator
                // distinguishing operator-driven termination from
                // crash. A reconciler that restarts an Operator-
                // stopped alloc would over-write the observer's
                // `Terminated` row with a fresh `Running`, masking
                // the operator's stop in obs and contradicting the
                // §intentional_stop ordering invariant on
                // `Driver::take_exit_receiver`.
                let failed_alloc = active_allocs_vec.iter().find(|r| is_restartable(r));
                if let Some(failed) = failed_alloc {
                    // Backoff exhaustion check — emit no further
                    // RestartAllocation past the ceiling. Pure check
                    // against `view.restart_counts`.
                    let attempts = view.restart_counts.get(&failed.alloc_id).copied().unwrap_or(0);
                    if attempts >= RESTART_BACKOFF_CEILING {
                        // Idempotency guard: if the row already carries
                        // a BackoffExhausted terminal claim the
                        // reconciler has already finalised this alloc on
                        // a prior tick — do not re-emit. Without this
                        // guard the action shim's level-triggered
                        // re-enqueue would emit FinalizeFailed every
                        // tick forever once the alloc reached ceiling.
                        if matches!(
                            failed.terminal,
                            Some(TerminalCondition::BackoffExhausted { .. })
                        ) {
                            return (Vec::new(), view.clone());
                        }
                        // Backoff exhausted — emit the synthetic
                        // FinalizeFailed action carrying the typed
                        // terminal claim per ADR-0037 §4. The action
                        // shim consumes this in step 02-02 to write
                        // AllocStatusRow.terminal AND echo onto
                        // LifecycleEvent.terminal — both surfaces
                        // populated from the same value at the same
                        // dispatch site (drift is structurally
                        // impossible). The reconciler is the single
                        // source of every terminal claim.
                        let action = Action::FinalizeFailed {
                            alloc_id: failed.alloc_id.clone(),
                            terminal: Some(TerminalCondition::BackoffExhausted { attempts }),
                        };
                        return (vec![action], view.clone());
                    }
                    // Persist-inputs read site (issue #141): recompute
                    // the backoff deadline on every tick from the
                    // persisted *inputs* (`last_failure_seen_at`,
                    // `restart_counts`) against the current policy
                    // (`backoff_for_attempt`). Mirrors the precedent at
                    // `crates/overdrive-control-plane/src/worker/exit_observer.rs:291`
                    // (`RETRY_BACKOFFS.get((attempts - 1) as usize)`).
                    // A future operator-configurable per-job
                    // `backoff_for_attempt` policy lands without a
                    // schema migration — every persisted row picks up
                    // the new policy on the next reconcile tick.
                    if let Some(seen_at) = view.last_failure_seen_at.get(&failed.alloc_id) {
                        let backoff = backoff_for_attempt(attempts);
                        if tick.now_unix < *seen_at + backoff {
                            // Backoff window not yet elapsed.
                            return (Vec::new(), view.clone());
                        }
                    }
                    // Per ADR-0031 §5 the Restart action carries the
                    // fully-populated `AllocationSpec` — mirroring the
                    // Start path. The reconciler has the live Job in
                    // scope; constructing the spec here is pure (two
                    // .clone() calls + identity derivation), and
                    // preserves the shim's stateless-dispatcher
                    // contract per ADR-0023.
                    let identity = mint_identity(&job.id, &failed.alloc_id);
                    // Per ADR-0031 Amendment 1: destructure the
                    // tagged-enum `WorkloadDriver` to project to the
                    // flat `AllocationSpec` (which stays flat per
                    // ADR-0030 §6). The destructure is irrefutable
                    // today (single Phase-1 variant); when Phase-2+
                    // adds variants it becomes a `match` and each arm
                    // projects to its per-driver-class spec.
                    let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
                    let action = Action::RestartAllocation {
                        alloc_id: failed.alloc_id.clone(),
                        spec: AllocationSpec {
                            alloc: failed.alloc_id.clone(),
                            identity,
                            command: command.clone(),
                            args: args.clone(),
                            resources: job.resources,
                        },
                        kind: desired.workload_kind,
                    };
                    let mut next_view = view.clone();
                    let count =
                        next_view.restart_counts.entry(failed.alloc_id.clone()).or_insert(0);
                    *count = count.saturating_add(1);
                    // Persist-inputs write site (issue #141): record
                    // the wall-clock observation timestamp of this
                    // failure (`tick.now_unix`) — NOT the precomputed
                    // deadline `tick.now + RESTART_BACKOFF_DURATION`,
                    // which would lock in the policy-at-write-time and
                    // break the "policy evolution is a no-op for the
                    // schema" guarantee. The deadline is recomputed at
                    // the read site on every tick from this seen_at +
                    // `backoff_for_attempt(restart_count)`.
                    next_view.last_failure_seen_at.insert(failed.alloc_id.clone(), tick.now_unix);
                    return (vec![action], next_view);
                }

                // No Running, no failed-needs-restart → schedule a
                // fresh allocation. Inline first-fit over BTreeMap.
                let placement = first_fit_place(&desired.nodes, job, &allocs_vec);
                placement.map_or_else(
                    || {
                        // NoCapacity — emit no action. The Pending row
                        // remains in obs (the renderer surfaces the
                        // reason at render time). Backoff is irrelevant
                        // here (nothing to back off from).
                        (Vec::new(), view.clone())
                    },
                    |node_id| {
                        // Fresh-id derivation per workload-gc-absent-
                        // stale-allocs step 01-04: index the new
                        // alloc by the number of pre-existing rows
                        // for this workload. With zero rows the
                        // suffix is `0` (preserves the prior shape);
                        // with a SystemGc-Terminated row already in
                        // `allocs_vec` (resubmit-after-GC), the
                        // suffix is `1` and the new alloc gets a
                        // distinct id. This makes the action shim's
                        // LWW write of the new `Running` row land
                        // on a NEW key rather than overwrite the
                        // prior SystemGc terminal stamp — making
                        // good on architecture.md § 5's
                        // `resubmit.preserves_prior_gc_terminal`
                        // promise.
                        let attempt = u32::try_from(allocs_vec.len()).unwrap_or(u32::MAX);
                        let alloc_id = mint_alloc_id(&job.id, attempt);
                        let identity = mint_identity(&job.id, &alloc_id);
                        // Per ADR-0031 §5 + Amendment 1: the Start
                        // action carries the operator-declared command
                        // + args projected from the tagged-enum
                        // `WorkloadDriver` field on `Job`. No more
                        // literal `/bin/sleep` / `["60"]`. The
                        // destructure is irrefutable today (single
                        // Phase-1 variant); future variants append.
                        let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
                        let action = Action::StartAllocation {
                            alloc_id: alloc_id.clone(),
                            workload_id: job.id.clone(),
                            node_id,
                            spec: AllocationSpec {
                                alloc: alloc_id,
                                identity,
                                command: command.clone(),
                                args: args.clone(),
                                resources: job.resources,
                            },
                            kind: desired.workload_kind,
                        };
                        (vec![action], view.clone())
                    },
                )
            }
        }
    }
}

/// Pure first-fit placement helper. Inlined here because
/// `overdrive-core` cannot depend on `overdrive-scheduler` (would
/// invert the dependency direction; the scheduler is a `core`-class
/// crate that depends on `overdrive-core`). The algorithm is the same
/// as `overdrive_scheduler::schedule`'s happy path: walk `nodes` in
/// `BTreeMap` order, return the first `NodeId` whose free capacity
/// covers the job's resource envelope.
fn first_fit_place(
    nodes: &BTreeMap<NodeId, Node>,
    job: &Job,
    current_allocs: &[&AllocStatusRow],
) -> Option<NodeId> {
    for (node_id, node) in nodes {
        let free = node_free_capacity(node, current_allocs, &job.resources);
        if free.cpu_milli >= job.resources.cpu_milli
            && free.memory_bytes >= job.resources.memory_bytes
        {
            return Some(node_id.clone());
        }
    }
    None
}

/// Free capacity of `node` after subtracting reserved envelope of
/// Running allocations targeting it. Inline counterpart to
/// `overdrive_scheduler::free_capacity`.
fn node_free_capacity(
    node: &Node,
    current_allocs: &[&AllocStatusRow],
    per_alloc: &Resources,
) -> Resources {
    let running_on_node: u64 = u64::try_from(
        current_allocs
            .iter()
            .filter(|alloc| alloc.node_id == node.id && alloc.state == AllocState::Running)
            .count(),
    )
    .unwrap_or(u64::MAX);
    let total_cpu_reserved = u64::from(per_alloc.cpu_milli).saturating_mul(running_on_node);
    let total_mem_reserved = per_alloc.memory_bytes.saturating_mul(running_on_node);
    let cpu_after = u64::from(node.capacity.cpu_milli).saturating_sub(total_cpu_reserved);
    Resources {
        cpu_milli: u32::try_from(cpu_after).unwrap_or(u32::MAX),
        memory_bytes: node.capacity.memory_bytes.saturating_sub(total_mem_reserved),
    }
}

/// Mint a deterministic [`AllocationId`] for a job at attempt index
/// `attempt`. Pure function over `(workload_id, attempt)` so two
/// reconcile calls with the same desired/actual produce the same
/// alloc id (purity contract).
///
/// **The `attempt` parameter is the count of pre-existing alloc
/// rows for the workload at placement time** (per workload-gc-
/// absent-stale-allocs step 01-04). With zero pre-existing rows the
/// suffix is `0` (preserves the pre-Phase-1.4 single-shot shape);
/// after a SystemGc stop leaves one Terminated row behind, a
/// resubmit's placement passes `attempt = 1` and mints
/// `alloc-{workload_id}-1` — distinct from the GC'd row's
/// `alloc-{workload_id}-0`. This is the structural defence against
/// the resurrection class where the action shim's LWW write of the
/// new `Running` row would otherwise overwrite the prior SystemGc
/// terminal stamp.
fn mint_alloc_id(workload_id: &WorkloadId, attempt: u32) -> AllocationId {
    let raw = format!("alloc-{}-{}", workload_id.as_str(), attempt);
    #[allow(clippy::expect_used)]
    AllocationId::new(&raw).expect("derived alloc id format is valid")
}

/// Mint a deterministic [`SpiffeId`] for an allocation.
fn mint_identity(workload_id: &WorkloadId, alloc_id: &AllocationId) -> SpiffeId {
    let raw = format!(
        "spiffe://overdrive.local/job/{}/alloc/{}",
        workload_id.as_str(),
        alloc_id.as_str()
    );
    #[allow(clippy::expect_used)]
    SpiffeId::new(&raw).expect("derived SpiffeId is valid")
}

/// service-vip-allocator step 03-01 — pure helper for the Service-arm
/// release-emission gate.
///
/// Returns `Some((action, digest))` when:
///
/// 1. `desired.workload_kind == WorkloadKind::Service`, AND
/// 2. `desired.service_spec_digest` is `Some(digest)`, AND
/// 3. at least one observed allocation in `actual.allocations` carries
///    a terminal claim (`row.terminal.is_some()`), AND
/// 4. `digest` is NOT already present in `view.released_for_terminal`.
///
/// Returns `None` otherwise — i.e. for non-Service kinds, when the
/// digest is absent (the runtime hydrator has not populated it), when
/// no terminal-state observation row exists yet, or when the digest
/// is already recorded as released.
///
/// The caller (the `WorkloadLifecycle::reconcile` wrapper) appends the
/// returned action to the inner reconcile's action list and stamps the
/// returned digest onto `next_view.released_for_terminal` so the next
/// tick short-circuits. Per ADR-0049 (amended 2026-05-15) +
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state".
fn service_vip_release_emission(
    desired: &WorkloadLifecycleState,
    actual: &WorkloadLifecycleState,
    view: &WorkloadLifecycleView,
) -> Option<(Action, crate::id::ContentHash)> {
    if desired.workload_kind != WorkloadKind::Service {
        return None;
    }
    let digest = desired.service_spec_digest?;
    if view.released_for_terminal.contains(&digest) {
        return None;
    }
    let terminal_observed = actual.allocations.values().any(|row| row.terminal.is_some());
    if !terminal_observed {
        return None;
    }
    let target = format!("job-lifecycle/{}", desired.workload_id.as_str());
    let correlation = CorrelationKey::derive(&target, &digest, "release-service-vip");
    Some((Action::ReleaseServiceVip { spec_digest: digest, correlation }, digest))
}

/// True iff the alloc row carries a terminal Operator-stop record.
///
/// Two writers produce operator-stop rows with different field shapes:
///
/// - **Exit observer** (direct observation): writes
///   `reason: Stopped { by: Operator }`, `terminal: None`.
/// - **Action shim** (ADR-0037 §4 SSOT): writes
///   `reason: Stopped { by: Reconciler }`,
///   `terminal: Stopped { by: Operator }`.
///
/// The function checks BOTH `terminal` (the ADR-0037 §4 SSOT) and
/// `reason` (the exit-observer path) so that operator-stop rows from
/// either writer are recognised. See GH #149 for the regression that
/// motivated the dual check.
///
/// **Narrow semantics — load-bearing.** This predicate matches
/// `StoppedBy::Operator` only and is used by the Run-branch's
/// short-circuit (`reconcile.rs:1294`-equivalent): an Operator-stopped
/// row preserves a stronger contract than the broader intentional-stop
/// class. Operator stop overrides re-submit (the operator's intent
/// outranks the new submit), so the Run branch returns no actions
/// even when desired intent is present. Use [`is_intentionally_stopped`]
/// for restart / natural-exit / placement-candidacy decisions where
/// `Operator` and `SystemGc` share the "don't restart, don't
/// finalize" semantics.
fn is_operator_stopped(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(crate::transition_reason::TerminalCondition::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
            })
        ) || matches!(
            row.reason,
            Some(crate::transition_reason::TransitionReason::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
            })
        ))
}

/// True iff the alloc row carries a terminal *intentional-stop class*
/// record — `state == Terminated` AND its `terminal` OR `reason`
/// carries `Stopped { by: ∈ {Operator, SystemGc} }`.
///
/// **Asymmetry vs [`is_operator_stopped`] — load-bearing.** This
/// predicate is the broader query covering both intentional-stop
/// sources; [`is_operator_stopped`] is the narrower Operator-only
/// query. The two predicates serve distinct call sites with distinct
/// semantics:
///
/// - **`is_operator_stopped`** (Run-branch top-of-branch short-circuit):
///   Operator stop is overriding — the Run branch returns
///   `(Vec::new(), view.clone())` even when desired intent is present
///   (`desired.job = Some(...)`). The operator's stop intent outranks
///   the new submit; a fresh schedule would undo the operator's stop.
/// - **`is_intentionally_stopped`** (filter for `active_allocs_vec`):
///   SystemGc-stopped rows are filtered out of placement-candidacy so
///   that a re-submit lands a fresh allocation (the operator's new
///   intent IS the override of the system's earlier GC withdrawal).
///   Operator-stopped rows would also be filtered out, but they
///   never reach this filter — the upstream `is_operator_stopped`
///   short-circuit fires first.
///
/// Use this predicate for restart / natural-exit / placement-
/// candidacy decisions where the question is "does this row
/// represent an intentional stop the reconciler should respect?"
/// (Operator OR SystemGc). Use [`is_operator_stopped`] for the
/// stricter "is this specifically an operator-driven stop?" check
/// (audit log gating, the Run-branch short-circuit, lifecycle event
/// payload classification).
fn is_intentionally_stopped(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(crate::transition_reason::TerminalCondition::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
                    | crate::transition_reason::StoppedBy::SystemGc
            })
        ) || matches!(
            row.reason,
            Some(crate::transition_reason::TransitionReason::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
                    | crate::transition_reason::StoppedBy::SystemGc
            })
        ))
}

/// True iff the alloc row is a candidate for a `RestartAllocation`
/// action — i.e. it sits in a restartable terminal state AND is NOT
/// part of the intentional-stop class (Operator OR SystemGc).
/// Intentional-stop rows are explicitly excluded; see
/// [`is_intentionally_stopped`].
fn is_restartable(row: &AllocStatusRow) -> bool {
    let restartable_state =
        matches!(row.state, AllocState::Terminated | AllocState::Draining | AllocState::Failed);
    restartable_state && !is_intentionally_stopped(row)
}

/// True iff the alloc row represents a *natural exit* the Job-kind
/// reconciler should finalize on — a terminal lifecycle state (Failed
/// OR Terminated) whose stop attribution (in either `terminal` or
/// `reason`) is NOT an intentional stop (Operator OR SystemGc). Per
/// ADR-0037 Amendment 2026-05-10 / ADR-0047 §1: Job kind terminates on
/// the first observed exit (clean OR crashed). Intentional-stop rows
/// are excluded — Operator-stopped rows short-circuit the entire Run
/// branch upstream via [`is_operator_stopped`], and SystemGc-stopped
/// rows are filtered out of `active_allocs_vec` so a re-submit lands
/// a fresh allocation rather than spuriously firing FinalizeFailed
/// against the prior GC'd row.
fn is_natural_exit(row: &AllocStatusRow) -> bool {
    let terminal_state = matches!(row.state, AllocState::Terminated | AllocState::Failed);
    terminal_state && !is_intentionally_stopped(row)
}

/// Classify a natural-exit alloc row into the typed
/// [`TerminalCondition::Completed { exit_code }`] / [`TerminalCondition::Failed { exit_code }`]
/// variant per ADR-0037 Amendment 2026-05-10.
///
/// Exit-code source per row shape:
///
/// - `state: Terminated`, `reason: Stopped { by: Process }` — clean
///   exit. Maps to `Completed { exit_code: 0 }`. The `Process` source
///   on `Stopped` IS the canonical signal that the workload exited
///   cleanly; `exit_code` is `0` by definition for this row shape
///   (the `ExitObserver`'s `CleanExit` path emits exactly this).
/// - `state: Failed`, `reason: WorkloadCrashedImmediately { exit_code, .. }` —
///   crash. The typed `exit_code` field is used directly; falls back
///   to `0` when `exit_code` is `None` (signal-only exits).
/// - Anything else — falls back to `Failed { exit_code: 0 }`. This
///   is structurally rare (`is_natural_exit` already filters
///   non-terminal states); the catch-all preserves total dispatch.
fn classify_natural_exit_terminal(row: &AllocStatusRow) -> TerminalCondition {
    if row.state == AllocState::Terminated
        && matches!(row.reason, Some(TransitionReason::Stopped { by: StoppedBy::Process }))
    {
        return TerminalCondition::Completed { exit_code: 0 };
    }
    if let Some(TransitionReason::WorkloadCrashedImmediately { exit_code, .. }) = row.reason {
        return TerminalCondition::Failed { exit_code: exit_code.unwrap_or(0) };
    }
    TerminalCondition::Failed { exit_code: 0 }
}

/// `WorkloadLifecycle` reconciler's typed view — the libSQL-hydrated
/// private memory.
///
/// Per US-03 AC and issue #141 (persist inputs, not derived state):
/// - `restart_counts: BTreeMap<AllocationId, u32>` — how many times
///   each alloc has been started in this incarnation. **Input.**
/// - `last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>` —
///   the wall-clock observation timestamp of the last failure
///   (`tick.now_unix` at the moment a Failed/Terminated alloc was
///   seen). **Input.** The backoff *deadline* is recomputed on every
///   read as `seen_at + backoff_for_attempt(restart_count)`; never
///   persisted as a derived value.
///
/// This is the `.claude/rules/development.md` § "Persist inputs, not
/// derived state" shape: a future operator-configurable per-job
/// `backoff_for_attempt` policy lands without a schema migration —
/// every persisted row picks up the new policy on the next reconcile
/// tick. Persisting a precomputed deadline would have been a stale
/// cache of `tick.now_unix + RESTART_BACKOFF_DURATION`; rotating the
/// policy would have silently no-op'd against in-flight rows until
/// they aged out.
///
/// Phase 1 hydrates this from the runtime's view cache
/// (`AppState::view_cache`); Phase 2+ migrates the cache to
/// per-primitive libSQL via `CREATE TABLE IF NOT EXISTS` inside
/// `hydrate` per ADR-0013 §2b. The `UnixInstant` type is the portable
/// wall-clock representation chosen specifically so libSQL can store
/// and rehydrate the value across process restarts (cf.
/// `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkloadLifecycleView {
    /// How many times each alloc has been started under this
    /// reconciler's lifecycle. Reset by `alloc_id` when a new
    /// `alloc_id` is minted (per US-03 Domain Example 2).
    #[serde(default)]
    pub restart_counts: BTreeMap<AllocationId, u32>,
    /// Wall-clock observation timestamp of the last failure per alloc.
    /// The reconcile read site recomputes the backoff deadline as
    /// `seen_at + backoff_for_attempt(restart_count)` against
    /// `tick.now_unix` on every tick — the persisted *input*, not the
    /// derived deadline.
    #[serde(default)]
    pub last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>,
    /// service-vip-allocator step 03-01 — set of `spec_digest`s for
    /// which `Action::ReleaseServiceVip` has already been emitted by
    /// this reconciler. The reconcile read site consults this set to
    /// gate re-emission: a terminal-state observation row whose
    /// workload `spec_digest` is already present produces NO
    /// `ReleaseServiceVip` action on this tick.
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this is the *input* "we already emitted release
    /// for this digest" — NOT a derived "needs release now" boolean.
    /// The release decision is recomputed every tick from
    /// `(any terminal alloc observed, released_for_terminal contains
    /// digest)` against the live workload state.
    ///
    /// `BTreeSet`, NOT `HashSet`, per § "Ordered-collection choice":
    /// the set is serialised via CBOR (the runtime-owned View
    /// persistence path per ADR-0035/0036) and iterated under DST
    /// harness assertions — iteration order must be deterministic
    /// across seeds.
    #[serde(default)]
    pub released_for_terminal: BTreeSet<crate::id::ContentHash>,
}

// ---------------------------------------------------------------------------
// ServiceMapHydrator reconciler — Phase 2 (Slice 08; ASR-2.2-04)
//
// Watches the `service_backends` ObservationStore rows for backend-set
// drift (the desired side) and the `service_hydration_results` rows
// for the dataplane's confirmed-state observation (the actual side).
// Emits one `Action::DataplaneUpdateService` per service whose
// fingerprint diverges, and reads the hydration-result row on the
// next tick to advance the state machine.
//
// Per ADR-0035/0036:
//
// - Sync `reconcile`. No `.await`, no `Instant::now()`, no DB handle.
//   Wall-clock only via `tick.now_unix`.
// - Typed `State` (desired+actual per `ServiceId`) and typed `View`
//   (per-service retry inputs only — `attempts`,
//   `last_failure_seen_at`, `last_attempted_fingerprint`). NEVER a
//   `next_attempt_at` field per `.claude/rules/development.md`
//   § "Persist inputs, not derived state".
//
// The struct lives here (rather than in `overdrive-control-plane`)
// because [`AnyReconciler`] holds the concrete type in its
// `ServiceMapHydrator` variant — same layering as `WorkloadLifecycle`.
// `overdrive-control-plane::reconcilers::service_map_hydrator`
// re-exports the public surface.
// ---------------------------------------------------------------------------

/// Desired-side projection for a single service. Sourced by the runtime's
/// `hydrate_desired` arm from the `service_backends` ObservationStore
/// table (see GH #160 for the upstream table addition pending) and
/// projected into [`ServiceMapHydratorState::desired`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDesired {
    /// Virtual IP the kernel-side XDP program matches incoming packets
    /// against. Wrapped from the `service_backends` row's `Ipv4Addr`
    /// at the runtime hydrate boundary (architecture.md § 8 lines
    /// 616-629).
    pub vip: ServiceVip,
    /// Backend set, in deterministic `BTreeMap<BackendId, Backend>`
    /// iteration order (architecture.md § 7). Maglev table generation
    /// is byte-deterministic across nodes given identical inputs.
    pub backends: Vec<Backend>,
    /// Content-hash of the `(vip, backends)` pair per
    /// [`crate::dataplane::fingerprint::fingerprint`]. Identifies a
    /// unique backend-set state for convergence detection.
    pub fingerprint: BackendSetFingerprint,
}

/// Hydrator state — split into `desired` and `actual` projections
/// merged by the runtime before `reconcile` per ADR-0036.
///
/// `BTreeMap` per `.claude/rules/development.md` § Ordered-collection
/// choice — deterministic iteration order is load-bearing for the
/// Maglev permutation generator that consumes the emitted action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceMapHydratorState {
    /// Per-service desired backend set. Hydrated from
    /// `service_backends` ObservationStore rows for the target
    /// `ServiceId`.
    pub desired: BTreeMap<ServiceId, ServiceDesired>,
    /// Per-service last-known hydration outcome from the
    /// `service_hydration_results` table. The hydrator observes the
    /// dataplane's confirmed state, not a next-action prediction.
    pub actual: BTreeMap<ServiceId, ServiceHydrationStatus>,
}

/// Per-service retry inputs — `attempts`,
/// `last_failure_seen_at`, `last_attempted_fingerprint` per
/// architecture.md § 8 *type View*. Per
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state" the View carries the inputs the next-attempt deadline is
/// computed from, NEVER the deadline itself — every tick recomputes
/// `last_failure_seen_at + backoff_for_attempt(attempts)` from these
/// inputs against `tick.now_unix`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetryMemory {
    /// Number of `Action::DataplaneUpdateService` dispatches emitted
    /// for this service. Increments only on dispatch (NOT every tick);
    /// reset to 0 on confirmed Completed observation. **Input.**
    #[serde(default)]
    pub attempts: u32,
    /// Wall-clock observation timestamp of the last failure
    /// (`tick.now_unix` at the moment a Failed status was recorded
    /// on dispatch). The backoff *deadline* is recomputed on every
    /// read as `seen_at + backoff_for_attempt(attempts)`; never
    /// persisted. **Input.**
    #[serde(default = "retry_memory_default_seen_at")]
    pub last_failure_seen_at: UnixInstant,
    /// Fingerprint of the most recently attempted backend set. Used
    /// to distinguish "same fingerprint failed; retry only when the
    /// backoff window elapses" from "fingerprint changed; dispatch
    /// immediately regardless of backoff." **Input.**
    #[serde(default)]
    pub last_attempted_fingerprint: Option<BackendSetFingerprint>,
}

/// Default `last_failure_seen_at` for serde — `UnixInstant` does not
/// implement `Default`, so we provide a sensible epoch-zero value
/// for new rows where no failure has been observed yet.
const fn retry_memory_default_seen_at() -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::ZERO)
}

impl Default for RetryMemory {
    fn default() -> Self {
        Self {
            attempts: 0,
            last_failure_seen_at: retry_memory_default_seen_at(),
            last_attempted_fingerprint: None,
        }
    }
}

/// `ServiceMapHydrator` reconciler memory — `BTreeMap<ServiceId,
/// RetryMemory>` persisted by the runtime via `RedbViewStore` per
/// ADR-0035.
///
/// `BTreeMap` per `.claude/rules/development.md` § Ordered-collection
/// choice. The retries map is iterated in the GC sweep at the end of
/// `reconcile`; deterministic iteration order keeps DST replay
/// bit-identical.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceMapHydratorView {
    /// Per-service retry inputs. Empty when no service has been
    /// dispatched for yet, or after every dispatched service has
    /// reached the converged-Completed branch (each Completed entry
    /// is removed by the convergence reset).
    #[serde(default)]
    pub retries: BTreeMap<ServiceId, RetryMemory>,
}

/// The Phase 2 hydrator reconciler. Activates J-PLAT-004 (per
/// ADR-0042). Watches `service_backends` and `service_hydration_results`
/// observation rows; emits one `Action::DataplaneUpdateService` per
/// service whose backend-set fingerprint has drifted from the
/// confirmed-applied fingerprint.
pub struct ServiceMapHydrator {
    name: ReconcilerName,
}

impl ServiceMapHydrator {
    /// Construct the canonical `service-map-hydrator` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'service-map-hydrator' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Default for ServiceMapHydrator {
    fn default() -> Self {
        Self::canonical()
    }
}

impl Reconciler for ServiceMapHydrator {
    const NAME: &'static str = "service-map-hydrator";

    type State = ServiceMapHydratorState;
    type View = ServiceMapHydratorView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions = Vec::new();
        let mut next_view = view.clone();

        // For each service in `desired`, decide whether to dispatch
        // based on (1) actual.fingerprint vs desired.fingerprint, and
        // (2) the retry-budget deadline recomputed from persisted
        // inputs (NEVER persisted as a derived value).
        for (service_id, desired_svc) in &desired.desired {
            let actual_status = actual.actual.get(service_id);
            let need_dispatch = should_dispatch(
                actual_status,
                desired_svc.fingerprint,
                view.retries.get(service_id),
                tick.now_unix,
            );

            if need_dispatch {
                let target_str = format!("service-map-hydrator/{service_id}");
                let spec_hash = ContentHash::of(desired_svc.fingerprint.to_le_bytes().as_slice());
                let correlation = CorrelationKey::derive(&target_str, &spec_hash, "update-service");

                actions.push(Action::DataplaneUpdateService {
                    service_id: *service_id,
                    vip: desired_svc.vip,
                    backends: desired_svc.backends.clone(),
                    correlation,
                });

                // Bump retry memory — record *inputs* per
                // `.claude/rules/development.md` § "Persist inputs,
                // not derived state". `attempts` and
                // `last_failure_seen_at` together drive the next
                // tick's deadline recomputation.
                let entry = next_view.retries.entry(*service_id).or_default();
                entry.attempts = entry.attempts.saturating_add(1);
                entry.last_failure_seen_at = tick.now_unix;
                entry.last_attempted_fingerprint = Some(desired_svc.fingerprint);
            } else if let Some(ServiceHydrationStatus::Completed { fingerprint, .. }) =
                actual_status
            {
                // Convergence: reset retry memory for this service.
                if *fingerprint == desired_svc.fingerprint {
                    next_view.retries.remove(service_id);
                }
            }
        }

        // GC: drop retry memory for services no longer in `desired`.
        next_view.retries.retain(|service_id, _| desired.desired.contains_key(service_id));

        (actions, next_view)
    }
}

/// Pure decision: dispatch a `DataplaneUpdateService` action this tick?
///
/// Encapsulates the four-arm decision tree per architecture.md § 8:
///
/// 1. No actual row yet (`None`) or `Pending` → dispatch.
/// 2. `Completed { fingerprint }` matches desired.fingerprint → no
///    dispatch (converged).
/// 3. `Completed { fingerprint }` differs → dispatch (fingerprint
///    drift, no backoff gate).
/// 4. `Failed { fingerprint }`:
///    - Different fingerprint than current desired → dispatch
///      immediately (drift overrides backoff).
///    - Same fingerprint → dispatch only when backoff window has
///      elapsed (`tick.now_unix >= seen_at + backoff_for_attempt`).
fn should_dispatch(
    actual_status: Option<&ServiceHydrationStatus>,
    desired_fingerprint: BackendSetFingerprint,
    retry: Option<&RetryMemory>,
    now: UnixInstant,
) -> bool {
    match actual_status {
        None | Some(ServiceHydrationStatus::Pending) => true,
        Some(ServiceHydrationStatus::Completed { fingerprint, .. }) => {
            *fingerprint != desired_fingerprint
        }
        Some(ServiceHydrationStatus::Failed { fingerprint, .. }) => {
            if *fingerprint != desired_fingerprint {
                // Backend set drifted while in Failed state — dispatch
                // the new fingerprint immediately.
                return true;
            }
            // Same fingerprint failed; gate on retry-budget deadline.
            // Per `.claude/rules/development.md` § "Persist inputs,
            // not derived state" the deadline is recomputed every
            // tick from inputs (`attempts`, `last_failure_seen_at`)
            // against the current backoff policy. Never persisted.
            retry.is_none_or(|r| now >= r.last_failure_seen_at + backoff_for_attempt(r.attempts))
        }
    }
}
