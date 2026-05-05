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
//!     // shape picks `()`; the first real reconciler (`JobLifecycle`)
//!     // picks `JobLifecycleState`.
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

use std::collections::BTreeMap;

use crate::SpiffeId;
use crate::aggregate::{Exec, Job, Node, WorkloadDriver};
use crate::id::{AllocationId, CorrelationKey, JobId, NodeId};
use crate::traits::driver::{AllocationSpec, Resources};
use crate::traits::observation_store::{AllocState, AllocStatusRow};
use crate::transition_reason::{StoppedBy, TerminalCondition};
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
    /// ()`; the first real reconciler (`JobLifecycle`) picks
    /// `type State = JobLifecycleState`.
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
/// - `JobLifecycle` — the first real reconciler's projection
///   (job + nodes + allocations). Lands in this DISTILL wave but
///   the `JobLifecycleState` body is RED scaffold.
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
    /// `JobLifecycle` reconciler's typed projection — see
    /// [`JobLifecycleState`].
    JobLifecycle(JobLifecycleState),
}

/// Desired/actual projection consumed by `JobLifecycle::reconcile`.
/// Hydrated by the runtime from `IntentStore` (job + nodes) and
/// `ObservationStore` (allocations) per ADR-0021.
///
/// The same struct serves both `desired` and `actual` — the
/// reconciler interprets `desired.job` as "what should exist" and
/// `actual.allocations` as "what is currently running." Field shapes
/// are pinned by ADR-0021 §1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLifecycleState {
    /// The target job. `None` when the desired-state read returned
    /// no row (job was deleted) or the actual-state read found no
    /// surviving row to project against.
    pub job: Option<Job>,
    /// Whether a stop intent has been recorded for this job (i.e.
    /// `IntentKey::for_job_stop(<id>)` is populated). When true and
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
    /// `job_id`. Empty when no allocations yet exist.
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,
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
    /// `JobLifecycle` reconciler when `desired.replicas >
    /// actual.replicas_running`.
    StartAllocation {
        /// Newly-minted allocation identifier (the reconciler reads
        /// this from its hydrated view; the view used the runtime's
        /// seeded `Entropy` port to mint it).
        alloc_id: AllocationId,
        /// Owning job.
        job_id: JobId,
        /// Placement decision from `overdrive-scheduler::schedule`.
        node_id: NodeId,
        /// Resources / command / args / identity for the workload. The action
        /// shim passes this directly to `Driver::start`.
        spec: AllocationSpec,
    },
    /// Stop a Running allocation. Emitted by the `JobLifecycle`
    /// reconciler when desired state is "stopped" (set by
    /// `IntentKey::for_job_stop`).
    ///
    /// Per ADR-0037 §4 the variant carries a typed
    /// [`TerminalCondition`] flag the action shim writes onto
    /// `AllocStatusRow.terminal` AND echoes onto `LifecycleEvent`.
    /// The reconciler is the *single source* of every terminal claim;
    /// emission sites outside a reconciler tick (the action-shim
    /// heartbeat, the exit observer) emit `terminal: None`. When a
    /// stop is operator-initiated (`desired.desired_to_stop` set
    /// by `IntentKey::for_job_stop`), the reconciler stamps
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
    /// Emitted by the `JobLifecycle` reconciler in crash-recovery
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
    },

    /// Finalize a failed allocation as terminal — the synthetic
    /// Failed-row action per ADR-0023 / ADR-0037 §4.
    ///
    /// Emitted by the `JobLifecycle` reconciler at the deciding tick
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
const CANONICAL_TARGET_PREFIXES: &[&str] = &["job/", "node/", "alloc/"];

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
/// `JobLifecycle` (the first real reconciler).
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// First real (non-proof-of-life) reconciler. Converges declared
    /// replica count for a `Job` — see [`JobLifecycle`].
    JobLifecycle(JobLifecycle),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            Self::JobLifecycle(r) => r.name(),
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
            Self::JobLifecycle(_) => <JobLifecycle as Reconciler>::NAME,
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
    /// Phase 02-02 lands the action shim and `JobLifecycle::reconcile`
    /// body; Phase 02-03 lands the runtime tick loop that builds
    /// `AnyState::JobLifecycle(...)` from the `IntentStore` /
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
            // JobLifecycle dispatch — types align by construction
            // when the runtime hydrates matching desired/actual/view
            // variants. Step 02-03 lands the runtime tick loop that
            // produces these triples; the body of `reconcile` itself
            // is fully implemented as of step 02-02.
            (
                Self::JobLifecycle(r),
                AnyState::JobLifecycle(desired),
                AnyState::JobLifecycle(actual),
                AnyReconcilerView::JobLifecycle(view),
            ) => {
                let (actions, next_view) = r.reconcile(desired, actual, view, tick);
                (actions, AnyReconcilerView::JobLifecycle(next_view))
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
/// phase-1-first-workload DISTILL added the `JobLifecycle` arm. Per
/// ADR-0035 §1 the runtime owns the cache (bulk-loaded at boot via
/// `ViewStore::bulk_load`, written through after each `reconcile`);
/// reconcilers see a typed `&Self::View`, never the erased
/// `AnyReconcilerView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyReconcilerView {
    /// The `View = ()` variant used by Phase 1 reconcilers
    /// (`NoopHeartbeat`).
    Unit,
    /// `JobLifecycle` reconciler's view — see [`JobLifecycleView`].
    JobLifecycle(JobLifecycleView),
}

// ---------------------------------------------------------------------------
// JobLifecycle reconciler — first real reconciler (US-03)
// ---------------------------------------------------------------------------

/// Maximum restart attempts before `JobLifecycle` gives up on an alloc.
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
pub struct JobLifecycle {
    name: ReconcilerName,
}

impl JobLifecycle {
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

impl Reconciler for JobLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "job-lifecycle";

    type State = JobLifecycleState;
    type View = JobLifecycleView;

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
            // explicit stops; an absent job with stale Running allocs
            // is TODO(#148) (cleanup reconciler) — for now we emit
            // nothing and pass the view through unchanged.
            None => (Vec::new(), view.clone()),
            // Run: a job is desired.
            Some(job) => {
                // Pure first-fit placement (inlined from
                // overdrive-scheduler::schedule). Pulled inline rather
                // than calling the scheduler crate because
                // overdrive-core cannot depend on overdrive-scheduler
                // (would invert the dependency direction).
                let allocs_vec: Vec<&AllocStatusRow> = actual.allocations.values().collect();

                // Is any allocation already Running for this job? If so
                // we are converged — emit nothing. Failed allocs flow
                // into the restart-with-backoff branch below.
                let running_alloc = allocs_vec.iter().find(|r| r.state == AllocState::Running);
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
                // a separate signal carried by `IntentKey::for_job_stop`
                // and handled at the Stop branch above. The
                // Operator-stopped row arrives via the watcher's
                // `intentional_stop` flag — set by `Driver::stop`
                // even when no `for_job_stop` intent exists, e.g.
                // by direct CLI / API operator action.)
                if allocs_vec.iter().any(|r| is_operator_stopped(r)) {
                    return (Vec::new(), view.clone());
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
                let failed_alloc = allocs_vec.iter().find(|r| is_restartable(r));
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
                        let alloc_id = mint_alloc_id(&job.id);
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
                            job_id: job.id.clone(),
                            node_id,
                            spec: AllocationSpec {
                                alloc: alloc_id,
                                identity,
                                command: command.clone(),
                                args: args.clone(),
                                resources: job.resources,
                            },
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

/// Mint a deterministic [`AllocationId`] for a job. Pure function over
/// the job id so two reconcile calls with the same desired/actual
/// produce the same alloc id (purity contract).
fn mint_alloc_id(job_id: &JobId) -> AllocationId {
    let raw = format!("alloc-{}-0", job_id.as_str());
    #[allow(clippy::expect_used)]
    AllocationId::new(&raw).expect("derived alloc id format is valid")
}

/// Mint a deterministic [`SpiffeId`] for an allocation.
fn mint_identity(job_id: &JobId, alloc_id: &AllocationId) -> SpiffeId {
    let raw =
        format!("spiffe://overdrive.local/job/{}/alloc/{}", job_id.as_str(), alloc_id.as_str());
    #[allow(clippy::expect_used)]
    SpiffeId::new(&raw).expect("derived SpiffeId is valid")
}

/// True iff the alloc row carries a terminal Operator-stop record.
///
/// Per `fix-exec-driver-exit-watcher` Step 01-02 RCA §Bug 3: a row
/// whose `reason` carries `Stopped { by: Operator }` is the terminal
/// record of an intentional stop. The reconciler MUST NOT restart it
/// or schedule a fresh allocation for the job — operator stop intent
/// is the load-bearing discriminator and a fresh schedule would undo
/// the operator's stop.
fn is_operator_stopped(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && matches!(
            row.reason,
            Some(crate::transition_reason::TransitionReason::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
            })
        )
}

/// True iff the alloc row is a candidate for a `RestartAllocation`
/// action — i.e. it sits in a restartable terminal state AND was NOT
/// stopped by the operator. Operator-stopped rows are explicitly
/// excluded; see `is_operator_stopped`.
fn is_restartable(row: &AllocStatusRow) -> bool {
    let restartable_state =
        matches!(row.state, AllocState::Terminated | AllocState::Draining | AllocState::Failed);
    restartable_state && !is_operator_stopped(row)
}

/// `JobLifecycle` reconciler's typed view — the libSQL-hydrated
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
pub struct JobLifecycleView {
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
}
