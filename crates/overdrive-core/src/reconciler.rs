//! Reconciler primitive — the §18 pure-function contract with
//! pre-hydration + `TickContext` time injection per ADR-0013 (amended
//! 2026-04-24).
//!
//! A reconciler is a pure function over `(desired, actual, view, tick)`
//! that emits a list of [`Action`]s to converge the system toward the
//! desired state. Four patterns govern how an author writes one; each
//! is load-bearing for DST replay (whitepaper §21) and ESR verification
//! (whitepaper §18 / research §1.1, §10.5).
//!
//! # The pre-hydration pattern — ADR-0013 §2, §2b
//!
//! The trait splits into two methods with distinct purity contracts:
//!
//! * [`Reconciler::hydrate`] is `async` — the ONLY place a reconciler
//!   author touches libSQL. It reads the reconciler's private memory
//!   into an author-declared [`Reconciler::View`]. Free-form SQL lives
//!   here; so does schema management (CREATE TABLE IF NOT EXISTS, ALTER
//!   TABLE ADD COLUMN). No framework migrations in Phase 1.
//! * [`Reconciler::reconcile`] is sync and pure — no `.await`, no I/O,
//!   no direct store write. It operates only on its arguments. Two
//!   invocations with the same inputs MUST produce byte-identical
//!   output tuples.
//!
//! The runtime owns the `.await` on `hydrate`, the diff-and-persist of
//! the returned view, and the commit of emitted actions through Raft.
//!
//! # The time-injection pattern — ADR-0013 §2c
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
//! # The `AnyReconciler` enum-dispatch convention — ADR-0013 §2a
//!
//! `async fn` in traits is not dyn-compatible, and
//! [`Reconciler::View`] is an associated type — together they make
//! `Box<dyn Reconciler>` impossible. [`AnyReconciler`] is a hand-rolled
//! enum that dispatches each trait method via a match arm per variant.
//! Static dispatch, zero heap allocation on the hot path, compile-time
//! exhaustiveness across every registered reconciler kind. **Adding a
//! new first-party reconciler means adding one variant and one match
//! arm** in each of `name`, `hydrate`, and `reconcile`. Third-party
//! reconcilers land through the WASM extension path (whitepaper §18
//! "Extension Model") and do not go through `AnyReconciler`.
//!
//! # The `NextView` return convention — ADR-0013 §2b
//!
//! Reconcilers express writes as **data**, not side effects. The
//! [`Reconciler::reconcile`] signature returns `(Vec<Action>,
//! Self::View)`; the second element is the *next* view. The runtime
//! diffs it against the hydrated view and persists the delta back to
//! libSQL. Reconcilers never write libSQL directly — the
//! `&LibsqlHandle` is not passed to `reconcile` at all. Phase 1
//! convention is full-View replacement (`NextView = Self::View`); a
//! typed-diff shape is an additive future extension.
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
//!     Action, HydrateError, LibsqlHandle, Reconciler, ReconcilerName,
//!     TargetResource, TickContext,
//! };
//!
//! struct HelloReconciler {
//!     name: ReconcilerName,
//! }
//!
//! impl HelloReconciler {
//!     fn new() -> Self {
//!         Self {
//!             name: ReconcilerName::new("hello")
//!                 .expect("'hello' is a valid ReconcilerName"),
//!         }
//!     }
//! }
//!
//! impl Reconciler for HelloReconciler {
//!     // Per ADR-0021, every reconciler picks its own `State`
//!     // projection. A reconciler with no meaningful desired/actual
//!     // shape picks `()`; the first real reconciler (`JobLifecycle`)
//!     // picks `JobLifecycleState`.
//!     type State = ();
//!     // Phase 1 reconcilers carry no private memory — View is ().
//!     // Phase 2+ authors declare a struct decoded from libSQL rows
//!     // inside `hydrate`.
//!     type View = ();
//!
//!     fn name(&self) -> &ReconcilerName {
//!         &self.name
//!     }
//!
//!     // Schema-author hook (issue #139 step 02-02). Runs ONCE per
//!     // reconciler instance at register time, before any `hydrate` /
//!     // `persist` call. CREATE TABLE / ALTER TABLE lives here; failure
//!     // surfaces as `ControlPlaneError::Internal` at register time —
//!     // the runtime never carries a half-migrated handle into the
//!     // tick loop. Phase 1 `View = ()` reconcilers have no schema and
//!     // return `Ok(())`.
//!     async fn migrate(&self, _db: &LibsqlHandle) -> Result<(), HydrateError> {
//!         Ok(())
//!     }
//!
//!     // The ONLY place a reconciler author touches libSQL during a
//!     // tick. SELECT-only against the schema `migrate` materialised.
//!     // Phase 1 reconcilers hold no memory, so this is trivially
//!     // `Ok(())`.
//!     async fn hydrate(
//!         &self,
//!         _target: &TargetResource,
//!         _db: &LibsqlHandle,
//!     ) -> Result<Self::View, HydrateError> {
//!         Ok(())
//!     }
//!
//!     // Symmetric write side — Phase 1 convention `NextView =
//!     // Self::View` (full replacement). With `View = ()` there is
//!     // nothing to persist.
//!     async fn persist(
//!         &self,
//!         _view: &Self::View,
//!         _db: &LibsqlHandle,
//!     ) -> Result<(), HydrateError> {
//!         Ok(())
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
//!         // `view` carries the hydrated private memory. The returned
//!         // next-view (second element of the tuple) is diffed by the
//!         // runtime against this value and persisted back to libSQL.
//!         // Reconcilers never write libSQL directly.
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
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;

use std::collections::BTreeMap;

use crate::SpiffeId;
use crate::aggregate::{Exec, Job, Node, WorkloadDriver};
use crate::id::{AllocationId, CorrelationKey, JobId, NodeId};
use crate::traits::driver::{AllocationSpec, Resources};
use crate::traits::observation_store::{AllocState, AllocStatusRow};
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
// LibsqlHandle — opaque reconciler-memory handle
// ---------------------------------------------------------------------------

/// Opaque handle to a reconciler's private libSQL memory.
///
/// Per ADR-0013 §2b, one `&LibsqlHandle` per reconciler, exclusive to
/// that reconciler, provisioned by the runtime from the per-primitive
/// libSQL path. The handle wraps `Arc<libsql::Connection>` so cloning
/// is cheap and the underlying connection is shared safely across the
/// async hydrate path.
///
/// Two constructors:
///
/// * [`LibsqlHandle::open`] — file-backed, used by the production
///   runtime to open the per-reconciler libSQL file.
/// * [`LibsqlHandle::open_in_memory`] — `:memory:`-backed, used by
///   in-process tests that need a real connection without filesystem
///   I/O.
///
/// Both go through `libsql::Builder::new_local(...).build().await`
/// followed by `Database::connect()`; construction failure surfaces as
/// [`libsql::Error`].
///
/// The [`LibsqlHandle::connection`] accessor returns `&libsql::Connection`
/// so reconciler authors run free-form SQL through the underlying
/// primitive — `db.connection().execute(...)` / `db.connection()
/// .query(...)` — rather than through a constrained method surface on
/// the handle itself. Per `.claude/rules/development.md` § "Reconciler
/// I/O", `hydrate` is the ONLY place this access is permitted.
#[derive(Clone)]
pub struct LibsqlHandle {
    conn: Arc<libsql::Connection>,
}

impl fmt::Debug for LibsqlHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `libsql::Connection` is not `Debug`. We expose only the type
        // name — the connection's internals carry no information that
        // would be useful in a debug print and the surrounding handle
        // is intentionally opaque.
        f.debug_struct("LibsqlHandle").finish_non_exhaustive()
    }
}

impl LibsqlHandle {
    /// Open a file-backed libSQL database at `path` and return a
    /// handle wrapping the connection. The intended caller is the
    /// reconciler runtime in `overdrive-control-plane`, which derives
    /// `path` from the per-primitive libSQL provisioner.
    ///
    /// # Errors
    ///
    /// Returns [`libsql::Error`] if the libSQL builder rejects the
    /// path (filesystem permissions, malformed path, etc.) or if the
    /// subsequent `Database::connect()` call fails.
    pub async fn open(path: impl AsRef<std::path::Path>) -> Result<Self, libsql::Error> {
        let db = libsql::Builder::new_local(path.as_ref()).build().await?;
        let conn = db.connect()?;
        Ok(Self { conn: Arc::new(conn) })
    }

    /// Open an in-memory libSQL database and return a handle wrapping
    /// the connection. Used by in-process tests that need a real
    /// connection without filesystem I/O — production wiring uses
    /// [`LibsqlHandle::open`] with a real file path.
    ///
    /// # Errors
    ///
    /// Returns [`libsql::Error`] if the libSQL builder fails to
    /// construct the in-memory database or the subsequent
    /// `Database::connect()` call fails.
    pub async fn open_in_memory() -> Result<Self, libsql::Error> {
        let db = libsql::Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        Ok(Self { conn: Arc::new(conn) })
    }

    /// Borrow the underlying libSQL connection. Reconciler authors
    /// run free-form SQL through this — `handle.connection().execute(...)`,
    /// `handle.connection().query(...)`. Per ADR-0013 §2b and
    /// `.claude/rules/development.md` § "Reconciler I/O", this access
    /// is only permitted inside a `Reconciler::hydrate` body.
    #[must_use]
    pub fn connection(&self) -> &libsql::Connection {
        &self.conn
    }
}

// ---------------------------------------------------------------------------
// HydrateError — async read failure shape
// ---------------------------------------------------------------------------

/// Failure modes for `Reconciler::hydrate`.
///
/// Phase 1 ships exactly two variants:
///
/// * `Libsql` — underlying libsql error, wrapped via `#[from]` so
///   reconciler authors write `db.query(...)?` without per-call
///   `map_err`.
/// * `Schema` — the schema the reconciler expected is not present, or
///   does not match. Phase 1 schema management (CREATE TABLE IF NOT
///   EXISTS, ALTER TABLE ADD COLUMN) lives inline in `hydrate` per
///   development.md §Reconciler I/O; if the inline migration fails,
///   this is the error.
///
/// NO `Validation` variant — Phase 1 reconcilers do not validate
/// intra-DB invariants during hydrate. That arrives with the first
/// Phase 2+ reconciler author that needs it.
#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    /// Underlying libsql error.
    #[error("libsql error during hydrate: {0}")]
    Libsql(#[from] libsql::Error),
    /// Schema mismatch or migration failure.
    #[error("schema error: {message}")]
    Schema {
        /// Human-readable schema failure description.
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Reconciler trait
// ---------------------------------------------------------------------------

/// The §18 reconciler trait, pre-hydration + time-injected shape.
///
/// Per ADR-0013 §2 and §2c:
///
/// * `migrate` is async — runs ONCE per reconciler instance at register
///   time. Owns CREATE TABLE / ALTER TABLE; the runtime fails fast on
///   Err with `ControlPlaneError::Internal` and the registry contains
///   no partial slot for that reconciler. Issue #139 step 02-02.
/// * `hydrate` is async — runs every reconcile tick on the hot path.
///   The ONLY place a reconciler author runs SELECT against libSQL.
///   Assumes the schema is current (`migrate` already succeeded).
///   Returns the author-declared `View` type (typically a struct
///   decoded from a row set).
/// * `persist` is async — runs every reconcile tick after `reconcile`.
///   Reconciler-author-owned write phase (DELETE-then-INSERT under
///   Phase 1's `NextView = Self::View` convention). Assumes the schema
///   is current (`migrate` already succeeded).
/// * `reconcile` is pure and synchronous — no `.await`, no I/O, no
///   wall-clock read (only via `tick.now`), no direct store write. The
///   returned `(Vec<Action>, Self::View)` tuple carries actions the
///   runtime commits through Raft and the next-view the runtime diffs
///   against `view` and persists back to libSQL.
///
/// **Lifecycle invariant** (issue #139 step 02-02): by the time
/// `hydrate` or `persist` runs on a `LibsqlHandle`, `migrate` has
/// already returned `Ok(())` on that handle for this reconciler.
/// The runtime enforces this structurally — `ReconcilerRuntime::register`
/// calls `migrate` immediately after `LibsqlHandle::open` succeeds and
/// BEFORE the registry slot is finalised; there is no path that
/// reaches `hydrate`/`persist` without `migrate` having succeeded
/// at register time. Reconciler authors MAY rely on this invariant
/// to keep `hydrate` SELECT-only and `persist` DELETE-then-INSERT-only,
/// avoiding the per-tick CREATE TABLE IF NOT EXISTS overhead and
/// keeping the schema-author concern in exactly one place.
///
/// Compile-time enforcement: the acceptance test
/// `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
/// pins the signature via an
/// `fn(&R, &R::State, &R::State, &R::View, &TickContext) -> (Vec<Action>, R::View)`
/// type assertion. A regression that makes `reconcile` `async fn`,
/// adds a `&dyn Clock` parameter, or reverts the per-reconciler typed
/// `State` associated type (ADR-0021) fails that test at compile time.
pub trait Reconciler: Send + Sync {
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
    /// The runtime diffs the returned `NextView` against this view and
    /// persists the delta — reconcilers never write libSQL directly.
    type View: Send + Sync;

    /// Canonical name. Used for libSQL path derivation and evaluation
    /// broker keying.
    ///
    /// Per ADR-0013 §2 and §2a, the name is the [`AnyReconciler`]
    /// registry key; match arms in [`AnyReconciler::name`],
    /// [`AnyReconciler::hydrate`], and [`AnyReconciler::reconcile`]
    /// dispatch on the variant that holds this name.
    fn name(&self) -> &ReconcilerName;

    /// Schema-author lifecycle hook (issue #139 step 02-02). Runs ONCE
    /// per reconciler instance at register time, BEFORE any
    /// [`hydrate`](Self::hydrate) or [`persist`](Self::persist) call.
    ///
    /// CREATE TABLE / ALTER TABLE lives here. Future ALTER TABLE
    /// migrations land in the same body and follow the project's
    /// additive-only schema migration discipline (CLAUDE.md).
    /// `CREATE TABLE IF NOT EXISTS` is naturally idempotent so
    /// re-running migrate on a refreshed handle is safe.
    ///
    /// Pulling DDL off the per-tick `hydrate` hot path gives three
    /// properties: (1) `hydrate` stops re-parsing CREATE TABLE IF NOT
    /// EXISTS every tick; (2) WASM-loaded third-party reconcilers
    /// (whitepaper §18) get a clean module-load hook the runtime
    /// invokes without inspecting reconciler internals; (3) broken
    /// migrations surface at register time as
    /// `ControlPlaneError::Internal`, not on first reconcile — extends
    /// the boot-fast posture from step 01-02.
    ///
    /// Phase 1 ships no default impl: every reconciler MUST implement
    /// `migrate` explicitly. Reconcilers whose `View = ()` (e.g.
    /// `NoopHeartbeat`) implement it as `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns [`HydrateError::Libsql`] on underlying libsql failure
    /// (typically a malformed CREATE TABLE statement or a libsql build
    /// error against the underlying file). The `ReconcilerRuntime`
    /// maps this to `ControlPlaneError::Internal` and refuses to
    /// finalise the registry slot.
    fn migrate(
        &self,
        db: &LibsqlHandle,
    ) -> impl std::future::Future<Output = Result<(), HydrateError>> + Send;

    /// Async read phase. The ONLY place a reconciler author runs
    /// SELECT against libSQL. SELECT-only against the schema
    /// [`migrate`](Self::migrate) materialised — the lifecycle
    /// invariant guarantees the schema is current.
    ///
    /// Per ADR-0013 §2 and §2b, the runtime's tick loop is
    /// hydrate-then-reconcile: the runtime owns the `.await` on this
    /// method, hands the resulting [`Reconciler::View`] to
    /// [`Reconciler::reconcile`] as a pure input, and never exposes
    /// the `&LibsqlHandle` to `reconcile`.
    ///
    /// # Errors
    ///
    /// Returns [`HydrateError::Libsql`] on underlying libsql failure,
    /// or [`HydrateError::Schema`] on schema-level mismatch (e.g. a
    /// row whose textual content does not parse into the typed
    /// `View` field — newtype rejection, malformed `UnixInstant`).
    fn hydrate(
        &self,
        target: &TargetResource,
        db: &LibsqlHandle,
    ) -> impl std::future::Future<Output = Result<Self::View, HydrateError>> + Send;

    /// Async write phase — symmetric counterpart to [`Reconciler::hydrate`].
    ///
    /// Per ADR-0013 §2b Phase 1 convention `NextView = Self::View` (full
    /// replacement). The reconciler author owns the SQL on both the
    /// read side ([`hydrate`]) and the write side (this method); the
    /// runtime's tick loop calls `persist(next_view)` after `reconcile`
    /// returns.
    ///
    /// Phase 1 ships no default impl: every reconciler MUST implement
    /// `persist`. Reconcilers whose `View = ()` (e.g. `NoopHeartbeat`)
    /// implement it as `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns [`HydrateError::Libsql`] on underlying libsql failure,
    /// or [`HydrateError::Schema`] on schema-level mismatch.
    fn persist(
        &self,
        view: &Self::View,
        db: &LibsqlHandle,
    ) -> impl std::future::Future<Output = Result<(), HydrateError>> + Send;

    /// Pure function over `(desired, actual, view, tick) ->
    /// (Vec<Action>, NextView)`. See whitepaper §18, ADR-0013 §2 / §2b
    /// / §2c, and `.claude/rules/development.md` §Reconciler I/O.
    ///
    /// Per ADR-0013 §2b, `view` is the hydrated [`Reconciler::View`]
    /// and the second element of the returned tuple is the next-view
    /// — the runtime diffs it against `view` and persists the delta
    /// back to libSQL. Per ADR-0013 §2c, `tick` is the single pure
    /// time input constructed by the runtime once per evaluation;
    /// reading `Instant::now()` / `SystemTime::now()` inside this body
    /// is banned.
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
    StopAllocation {
        /// Target allocation. The action shim looks up the
        /// `AllocationHandle` via observation store.
        alloc_id: AllocationId,
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
    /// Never — `"noop-heartbeat"` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule. Failure
    /// would indicate a bug in the newtype constructor.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new("noop-heartbeat")
            .expect("'noop-heartbeat' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for NoopHeartbeat {
    // Per ADR-0021, reconcilers with no meaningful projection pick
    // `type State = ()`. `NoopHeartbeat` ignores `desired`/`actual`
    // entirely and always emits `Action::Noop`.
    type State = ();
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    async fn migrate(&self, _db: &LibsqlHandle) -> Result<(), HydrateError> {
        // `View = ()` carries no schema; the proof-of-life reconciler
        // has nothing to migrate.
        Ok(())
    }

    async fn hydrate(
        &self,
        _target: &TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        Ok(())
    }

    async fn persist(&self, _view: &Self::View, _db: &LibsqlHandle) -> Result<(), HydrateError> {
        // `View = ()` carries no rows; the proof-of-life reconciler
        // has nothing to persist.
        Ok(())
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
// TestStubReconciler — test-only reconciler with parameterised behaviour
// ---------------------------------------------------------------------------

/// Behaviour parameter for [`TestStubReconciler::migrate`].
///
/// **Test infrastructure**, kept hidden from rustdoc via `#[doc(hidden)]`.
/// Used to inject a controlled migrate outcome into tests that exercise
/// the runtime's eager-migrate-at-register path without modifying
/// production reconcilers. Constructible only via
/// [`TestStubReconciler::new`]; the only consumer is the
/// `register_runs_migrate_once_per_reconciler_and_fails_fast_on_broken_migration`
/// integration test in `overdrive-control-plane`.
///
/// The variant lives in the public surface (rather than feature-gated)
/// because resolver-v3 workspace builds compile the lib once with the
/// union of features any target requires; gating the variant produces
/// non-exhaustive-match errors in `cargo test --doc --workspace`. The
/// variant is harmless in production builds — `TestStubReconciler` does
/// no I/O outside of its `migrate` body, which production code never
/// constructs.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateBehavior {
    /// `migrate` returns `Ok(())`. Pinpoints the success path of the
    /// runtime's eager-migrate dispatch.
    Succeed,
    /// `migrate` returns `Err(HydrateError::Libsql(...))` once. The
    /// underlying error is a synthetic libsql failure produced by
    /// running an intentionally-malformed SQL statement against the
    /// supplied handle. Pinpoints the fail-fast path of the runtime's
    /// eager-migrate dispatch.
    FailOnce,
}

/// Test-only reconciler whose `migrate` outcome is parameterised by
/// [`MigrateBehavior`]. `hydrate`, `persist`, and `reconcile` are
/// trivial — `View = ()` and reconcile is a no-op — because the only
/// behaviour exercised by current tests is the migrate-at-register
/// fail-fast contract.
///
/// **Test infrastructure**, kept hidden from rustdoc via `#[doc(hidden)]`.
/// See [`MigrateBehavior`] for the cfg-gating rationale.
///
/// Use via the [`AnyReconciler::TestStub`] variant — the `TestStub`
/// variant is what `ReconcilerRuntime::register` accepts.
#[doc(hidden)]
pub struct TestStubReconciler {
    name: ReconcilerName,
    migrate_behavior: MigrateBehavior,
}

impl TestStubReconciler {
    /// Construct a `TestStubReconciler` with the given canonical name
    /// and migrate behaviour.
    #[must_use]
    pub const fn new(name: ReconcilerName, migrate_behavior: MigrateBehavior) -> Self {
        Self { name, migrate_behavior }
    }
}

impl Reconciler for TestStubReconciler {
    type State = ();
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    // mutants: skip — `TestStubReconciler` is test infrastructure
    // (kept out of public rustdoc via `#[doc(hidden)]`); replacing
    // `migrate` with `Ok(())` would land a `MigrateBehavior::FailOnce`
    // → `Ok(())` mutant the runtime test cannot catch by design,
    // because the test exercises exactly the FailOnce → Err path. The
    // value being defended is the `MigrateBehavior::FailOnce` arm
    // emitting `HydrateError::Libsql`, which is asserted in
    // `register_runs_migrate_once_per_reconciler_and_fails_fast_on_broken_migration`
    // in `overdrive-control-plane` integration tests — but that
    // assertion is on `ControlPlaneError::Internal`, not on the inner
    // error variant, so the cargo-mutants kill-rate tracker scoped to
    // this file does not see the catch.
    async fn migrate(&self, db: &LibsqlHandle) -> Result<(), HydrateError> {
        match self.migrate_behavior {
            MigrateBehavior::Succeed => Ok(()),
            MigrateBehavior::FailOnce => {
                // Run an intentionally-malformed statement against the
                // real handle so the surfaced `HydrateError::Libsql`
                // carries a real libsql diagnostic, not a fabricated
                // one. This exercises the same `#[from] libsql::Error`
                // conversion path the production reconciler authors
                // rely on, so the test catches regressions in error
                // mapping as well as registry-flow control.
                let _ = db.connection().execute("THIS IS NOT VALID SQL", ()).await?;
                Ok(())
            }
        }
    }

    async fn hydrate(
        &self,
        _target: &TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        Ok(())
    }

    async fn persist(&self, _view: &Self::View, _db: &LibsqlHandle) -> Result<(), HydrateError> {
        Ok(())
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
/// Replaces `Box<dyn Reconciler>` because the trait now carries an
/// associated type (`type View`) and an `async fn` in trait — both of
/// which break object safety. Adding a reconciler means adding a
/// variant here and a match arm in each of `name`, `hydrate`, and
/// `reconcile`.
///
/// Phase 1 ships exactly one proof-of-life variant: `NoopHeartbeat`.
/// The `phase-1-first-workload` DISTILL adds `JobLifecycle` as the
/// first real (non-proof-of-life) reconciler.
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// First real (non-proof-of-life) reconciler. Converges declared
    /// replica count for a `Job` — see [`JobLifecycle`].
    JobLifecycle(JobLifecycle),
    /// **Test infrastructure variant**, kept out of public rustdoc via
    /// `#[doc(hidden)]`. Carries a [`TestStubReconciler`] whose
    /// `migrate`, `hydrate`, and `persist` behaviour is parameterised
    /// at construction time. Used by the
    /// `register_runs_migrate_once_per_reconciler_and_fails_fast_on_broken_migration`
    /// integration test to inject a reconciler whose `migrate` returns
    /// Err WITHOUT adding a feature flag to a production reconciler
    /// (per issue #139 step 02-02 AC). Production code never
    /// constructs this variant. See [`TestStubReconciler`] for the
    /// gating rationale.
    #[doc(hidden)]
    TestStub(TestStubReconciler),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            Self::JobLifecycle(r) => r.name(),
            Self::TestStub(r) => r.name(),
        }
    }

    /// Schema-author lifecycle hook — dispatches to the inner
    /// reconciler's [`Reconciler::migrate`]. The runtime invokes this
    /// once at register time, immediately after `LibsqlHandle::open`
    /// and BEFORE the registry slot is finalised.
    ///
    /// # Errors
    ///
    /// Propagates [`HydrateError`] from the inner reconciler. The
    /// `ReconcilerRuntime::register` caller maps this to
    /// `ControlPlaneError::Internal` and refuses to insert a partial
    /// registry entry.
    pub async fn migrate(&self, db: &LibsqlHandle) -> Result<(), HydrateError> {
        match self {
            Self::NoopHeartbeat(r) => r.migrate(db).await,
            Self::JobLifecycle(r) => r.migrate(db).await,
            Self::TestStub(r) => r.migrate(db).await,
        }
    }

    /// Async read phase — dispatches to the inner reconciler's
    /// `hydrate`. Because every variant's `View` can differ, the
    /// caller receives a typed `AnyReconcilerView` sum.
    ///
    /// # Errors
    ///
    /// Propagates [`HydrateError`] from the inner reconciler.
    pub async fn hydrate(
        &self,
        target: &TargetResource,
        db: &LibsqlHandle,
    ) -> Result<AnyReconcilerView, HydrateError> {
        match self {
            Self::NoopHeartbeat(r) => r.hydrate(target, db).await.map(|()| AnyReconcilerView::Unit),
            Self::JobLifecycle(r) => {
                r.hydrate(target, db).await.map(AnyReconcilerView::JobLifecycle)
            }
            Self::TestStub(r) => r.hydrate(target, db).await.map(|()| AnyReconcilerView::Unit),
        }
    }

    /// Async write phase — dispatches to the inner reconciler's
    /// `persist`. The caller supplies the matching view variant.
    ///
    /// Variant alignment is a compile-time invariant: the dispatch
    /// `match` is exhaustive, and every arm pairs an `AnyReconciler`
    /// variant with its declared [`AnyReconcilerView`] counterpart.
    /// Adding a new reconciler variant whose `View` does not line up
    /// with a matching arm produces a non-exhaustive-match compile
    /// error, forcing the developer to extend the dispatch.
    ///
    /// # Errors
    ///
    /// Propagates [`HydrateError`] from the inner reconciler.
    pub async fn persist(
        &self,
        view: &AnyReconcilerView,
        db: &LibsqlHandle,
    ) -> Result<(), HydrateError> {
        match (self, view) {
            (Self::NoopHeartbeat(r), AnyReconcilerView::Unit) => r.persist(&(), db).await,
            (Self::JobLifecycle(r), AnyReconcilerView::JobLifecycle(view)) => {
                r.persist(view, db).await
            }
            // mutants: skip — test-infra dispatch arm; `TestStubReconciler::persist`
            // is `Ok(())`, so deleting the arm only changes behaviour when a test
            // explicitly asserts on the dispatch path's persist outcome — none does
            // (the integration test asserts on the runtime register flow, not on
            // post-register persist).
            (Self::TestStub(r), AnyReconcilerView::Unit) => r.persist(&(), db).await,
            // Cross-variant — the runtime tick loop only ever pairs a
            // reconciler with the View shape its own `hydrate`
            // produced, so this arm cannot be reached from a
            // correctly-wired caller. The match must remain exhaustive
            // so a future variant addition becomes a compile error.
            _ => panic!(
                "AnyReconciler::persist dispatch mismatch — \
                runtime supplied incompatible (reconciler, view) pair"
            ),
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
            // mutants: skip — test-infra dispatch arm; `TestStubReconciler::reconcile`
            // is a no-op returning `vec![Action::Noop]`. Deleting the arm only
            // changes behaviour when a test explicitly asserts on this dispatch
            // path's actions — none does. The runtime never constructs a
            // `TestStub` triple in production.
            (Self::TestStub(r), AnyState::Unit, AnyState::Unit, AnyReconcilerView::Unit) => {
                let (actions, ()) = r.reconcile(&(), &(), &(), tick);
                (actions, AnyReconcilerView::Unit)
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

/// Sum of every view type produced by `AnyReconciler::hydrate`. Phase 1
/// originally only had `View = ()` (the `Unit` variant); the
/// phase-1-first-workload DISTILL adds the `JobLifecycle` arm.
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
/// **Phase 1 is degenerate-constant**: every `attempt` value yields
/// the same [`RESTART_BACKOFF_DURATION`]. The function exists as a
/// stability anchor so call sites stay unchanged when
/// operator-configurable per-job policy lands in Phase 2+ (per issue
/// #141 'Out' section). The leading underscore on `_attempt` is
/// deliberate: the parameter is currently unused (degenerate policy
/// ignores attempt count) but lives in the signature so a future
/// progressive-backoff schedule (e.g. `RESTART_BACKOFF_DURATION *
/// 2_u32.pow(attempt)`) does not require a breaking API change.
///
/// Operator-configurable per-job policy is Phase 2+ scope and will
/// thread a `&JobBackoffPolicy` (or similar) through this signature
/// rather than relying on the workspace-global constant.
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
    /// Never — `"job-lifecycle"` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new("job-lifecycle")
            .expect("'job-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for JobLifecycle {
    type State = JobLifecycleState;
    type View = JobLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    async fn migrate(&self, db: &LibsqlHandle) -> Result<(), HydrateError> {
        // Per ADR-0013 §6 (schema-author clause), as refactored in
        // issue #139 step 02-02: the reconciler owns CREATE TABLE in
        // the dedicated `migrate` lifecycle hook called once per
        // reconciler instance at register time, NOT inline in
        // `hydrate`/`persist` on the per-tick hot path.
        //
        // Two tables, one column of identity + one column of payload
        // each — exactly the persist-inputs shape the View carries. NO
        // derived `next_attempt_at` column: the deadline is recomputed
        // every tick from `last_failure_seen_at + backoff_for_attempt(...)`
        // per `.claude/rules/development.md` § "Persist inputs, not
        // derived state".
        //
        // `last_failure_seen_at` stores `UnixInstant` as a TEXT column
        // carrying the canonical `<seconds>.<nanos>` Display form
        // (9-digit zero-padded nanos, see `wall_clock.rs`). TEXT was
        // chosen over INTEGER nanos so the full 64-bit seconds range
        // round-trips losslessly — INTEGER (i64) would refuse the
        // upper-half u64 seconds the proptest exercises.
        //
        // `CREATE TABLE IF NOT EXISTS` is naturally idempotent; future
        // ALTER TABLE migrations append here following the project's
        // additive-only schema migration discipline.
        let conn = db.connection();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS restart_counts (\
                alloc_id TEXT PRIMARY KEY NOT NULL, \
                count INTEGER NOT NULL\
            )",
            (),
        )
        .await?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS last_failure_seen_at (\
                alloc_id TEXT PRIMARY KEY NOT NULL, \
                ts TEXT NOT NULL\
            )",
            (),
        )
        .await?;
        Ok(())
    }

    async fn hydrate(
        &self,
        _target: &TargetResource,
        db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        // Schema is current by lifecycle invariant — `migrate` ran at
        // register time per the runtime contract (see
        // `ReconcilerRuntime::register`). This body is SELECT-only;
        // CREATE TABLE moved to `migrate` in issue #139 step 02-02 to
        // keep the per-tick hot path off the schema-management path.
        let conn = db.connection();

        let mut view = JobLifecycleView::default();

        let mut rows = conn.query("SELECT alloc_id, count FROM restart_counts", ()).await?;
        while let Some(row) = rows.next().await? {
            let alloc_id_raw: String = row.get(0)?;
            let count_raw: i64 = row.get(1)?;
            let alloc_id = AllocationId::new(&alloc_id_raw).map_err(|e| HydrateError::Schema {
                message: format!(
                    "restart_counts.alloc_id {alloc_id_raw:?} is not a valid AllocationId: {e}"
                ),
            })?;
            let count = u32::try_from(count_raw).map_err(|_| HydrateError::Schema {
                message: format!(
                    "restart_counts.count for {alloc_id_raw} ({count_raw}) does not fit in u32"
                ),
            })?;
            view.restart_counts.insert(alloc_id, count);
        }

        let mut rows = conn.query("SELECT alloc_id, ts FROM last_failure_seen_at", ()).await?;
        while let Some(row) = rows.next().await? {
            let alloc_id_raw: String = row.get(0)?;
            let ts_raw: String = row.get(1)?;
            let alloc_id = AllocationId::new(&alloc_id_raw).map_err(|e| {
                HydrateError::Schema {
                    message: format!(
                        "last_failure_seen_at.alloc_id {alloc_id_raw:?} is not a valid AllocationId: {e}"
                    ),
                }
            })?;
            let ts: UnixInstant = ts_raw.parse().map_err(|e| HydrateError::Schema {
                message: format!(
                    "last_failure_seen_at.ts {ts_raw:?} for {alloc_id_raw} does not parse as UnixInstant: {e}"
                ),
            })?;
            view.last_failure_seen_at.insert(alloc_id, ts);
        }

        Ok(view)
    }

    async fn persist(&self, view: &Self::View, db: &LibsqlHandle) -> Result<(), HydrateError> {
        // Per ADR-0013 §2b Phase 1 convention `NextView = Self::View`
        // — full replacement via DELETE-then-INSERT. Quadratic in view
        // size; fine for Phase 1 (single-node, single-job, view rows
        // bounded by the alloc cardinality of one job). Later phases
        // may optimise to true diffs.
        //
        // Schema is current by lifecycle invariant — `migrate` ran at
        // register time per the runtime contract (see
        // `ReconcilerRuntime::register`). This body is
        // DELETE-then-INSERT-only; CREATE TABLE moved to `migrate` in
        // issue #139 step 02-02 to keep the per-tick hot path off the
        // schema-management path.
        let conn = db.connection();
        let txn = conn.transaction().await?;
        txn.execute("DELETE FROM restart_counts", ()).await?;
        txn.execute("DELETE FROM last_failure_seen_at", ()).await?;

        for (alloc_id, count) in &view.restart_counts {
            txn.execute(
                "INSERT INTO restart_counts (alloc_id, count) VALUES (?1, ?2)",
                (alloc_id.as_str().to_owned(), i64::from(*count)),
            )
            .await?;
        }
        for (alloc_id, ts) in &view.last_failure_seen_at {
            txn.execute(
                "INSERT INTO last_failure_seen_at (alloc_id, ts) VALUES (?1, ?2)",
                (alloc_id.as_str().to_owned(), ts.to_string()),
            )
            .await?;
        }

        txn.commit().await?;
        Ok(())
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
            let stop_actions: Vec<Action> = actual
                .allocations
                .values()
                .filter(|r| r.state == AllocState::Running)
                .map(|r| Action::StopAllocation { alloc_id: r.alloc_id.clone() })
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
            // is a Phase 2+ concern (cleanup reconciler) — for now we
            // emit nothing and pass the view through unchanged.
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
                        // Backoff exhausted — alloc stays Terminated,
                        // no further actions emitted.
                        return (Vec::new(), view.clone());
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
/// per-primitive libSQL via the dedicated [`Reconciler::migrate`]
/// hook (issue #139 step 02-02) — schema is materialised at register
/// time, NOT inline in `hydrate`. The `UnixInstant` type is the
/// portable wall-clock representation chosen specifically so libSQL
/// can store and rehydrate the value across process restarts (cf.
/// `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobLifecycleView {
    /// How many times each alloc has been started under this
    /// reconciler's lifecycle. Reset by `alloc_id` when a new
    /// `alloc_id` is minted (per US-03 Domain Example 2).
    pub restart_counts: BTreeMap<AllocationId, u32>,
    /// Wall-clock observation timestamp of the last failure per alloc.
    /// The reconcile read site recomputes the backoff deadline as
    /// `seen_at + backoff_for_attempt(restart_count)` against
    /// `tick.now_unix` on every tick — the persisted *input*, not the
    /// derived deadline.
    pub last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>,
}

// ---------------------------------------------------------------------------
// LibsqlHandle constructor / accessor tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        AnyReconciler, AnyReconcilerView, JobLifecycle, JobLifecycleView, LibsqlHandle,
        NoopHeartbeat, Reconciler, TargetResource,
    };
    use crate::id::AllocationId;
    use crate::wall_clock::UnixInstant;

    fn alloc(raw: &str) -> AllocationId {
        AllocationId::new(raw).expect("valid alloc id")
    }

    fn target() -> TargetResource {
        TargetResource::new("job/payments").expect("valid target resource")
    }

    /// In-memory open + migrate ceremony. Mirrors the
    /// `ReconcilerRuntime::register` lifecycle — open the handle, then
    /// run `JobLifecycle::migrate` to materialise the schema before
    /// any `hydrate`/`persist` is invoked. Issue #139 step 02-02.
    async fn open_and_migrate_in_memory() -> LibsqlHandle {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        JobLifecycle::canonical().migrate(&handle).await.expect("migrate job-lifecycle schema");
        handle
    }

    /// AC 4 (negative case) — hydrating an empty (post-migrate)
    /// in-memory DB returns the default `JobLifecycleView`.
    #[tokio::test]
    async fn job_lifecycle_hydrate_empty_db_returns_default() {
        let handle = open_and_migrate_in_memory().await;
        let view = JobLifecycle::canonical().hydrate(&target(), &handle).await.expect("hydrate");
        assert_eq!(view, JobLifecycleView::default());
    }

    /// AC 5 — within-handle persist→hydrate round-trip. Complements
    /// the cross-handle integration test by exercising the same code
    /// path under in-process isolation.
    #[tokio::test]
    async fn job_lifecycle_persist_then_hydrate_roundtrip_in_memory() {
        let handle = open_and_migrate_in_memory().await;
        let reconciler = JobLifecycle::canonical();

        let mut view = JobLifecycleView::default();
        view.restart_counts.insert(alloc("alloc-x"), 2);
        view.restart_counts.insert(alloc("alloc-y"), 0);
        view.last_failure_seen_at.insert(
            alloc("alloc-x"),
            UnixInstant::from_unix_duration(Duration::new(1_700_000_000, 999_999_999)),
        );

        reconciler.persist(&view, &handle).await.expect("persist");
        let hydrated = reconciler.hydrate(&target(), &handle).await.expect("hydrate");
        assert_eq!(hydrated, view);
    }

    /// AC 4 — second persist replaces (not unions with) first. The
    /// in-memory variant of the integration test, so the kill-rate
    /// gate scopes to default-lane mutants too.
    #[tokio::test]
    async fn job_lifecycle_persist_replaces_prior_state_in_memory() {
        let handle = open_and_migrate_in_memory().await;
        let reconciler = JobLifecycle::canonical();

        let mut first = JobLifecycleView::default();
        first.restart_counts.insert(alloc("first-only"), 5);
        first
            .last_failure_seen_at
            .insert(alloc("first-only"), UnixInstant::from_unix_duration(Duration::new(100, 0)));
        reconciler.persist(&first, &handle).await.expect("persist 1");

        let second = JobLifecycleView::default();
        reconciler.persist(&second, &handle).await.expect("persist 2");

        let hydrated = reconciler.hydrate(&target(), &handle).await.expect("hydrate");
        assert_eq!(hydrated, JobLifecycleView::default());
        assert!(hydrated.restart_counts.is_empty());
        assert!(hydrated.last_failure_seen_at.is_empty());
    }

    /// AC 2 — `NoopHeartbeat::persist` is `Ok(())` regardless of the
    /// underlying DB state.
    #[tokio::test]
    async fn noop_heartbeat_persist_is_ok() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        NoopHeartbeat::canonical().persist(&(), &handle).await.expect("persist ok");
    }

    /// AC 3 — `AnyReconciler::persist` dispatches to the inner
    /// reconciler's `persist`. Verified by:
    ///
    /// * `NoopHeartbeat`-arm dispatch must succeed against an empty
    ///   DB (write nothing).
    /// * `JobLifecycle`-arm dispatch with a non-trivial view must
    ///   produce rows observable by a follow-up `hydrate` against the
    ///   same handle.
    #[tokio::test]
    async fn any_reconciler_persist_dispatches_correctly() {
        // NoopHeartbeat arm — no schema, so a bare in-memory handle is
        // sufficient (NoopHeartbeat::migrate is Ok(()) and its persist
        // writes nothing).
        {
            let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
            let any = AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical());
            any.persist(&AnyReconcilerView::Unit, &handle).await.expect("noop persist");
        }
        // JobLifecycle arm — write through Any, read back through the
        // concrete reconciler to prove the dispatch reached the right
        // impl. Schema must be materialised first via the Any-level
        // `migrate` dispatch so persist sees a current schema.
        {
            let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
            let any = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
            any.migrate(&handle).await.expect("any migrate");

            let mut view = JobLifecycleView::default();
            view.restart_counts.insert(alloc("dispatch-check"), 3);
            view.last_failure_seen_at.insert(
                alloc("dispatch-check"),
                UnixInstant::from_unix_duration(Duration::new(42, 0)),
            );
            any.persist(&AnyReconcilerView::JobLifecycle(view.clone()), &handle)
                .await
                .expect("any persist");

            let hydrated =
                JobLifecycle::canonical().hydrate(&target(), &handle).await.expect("hydrate");
            assert_eq!(hydrated, view);
        }
    }

    /// `open_in_memory()` produces a handle whose underlying connection
    /// can run a CREATE/INSERT/SELECT roundtrip. The accessor returns
    /// `&libsql::Connection` so reconciler authors can run free-form
    /// SQL without wrapping every call in a method on `LibsqlHandle`.
    #[tokio::test]
    async fn open_in_memory_returns_usable_connection() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        let conn = handle.connection();

        conn.execute("CREATE TABLE t (x INTEGER)", ()).await.expect("create table");
        conn.execute("INSERT INTO t (x) VALUES (1)", ()).await.expect("insert row");

        let mut rows = conn.query("SELECT x FROM t", ()).await.expect("select");
        let row = rows.next().await.expect("next row").expect("row present");
        let got: i64 = row.get(0).expect("x column");
        assert_eq!(got, 1);
    }

    // ---------------------------------------------------------------------------
    // Step 02-02 — `Reconciler::migrate` lifecycle hook
    // ---------------------------------------------------------------------------

    /// AC 2 — `JobLifecycle::migrate` materialises the reconciler's
    /// schema (`restart_counts` and `last_failure_seen_at`) on a fresh
    /// libSQL handle. Verified by issuing a `SELECT` against each
    /// table after `migrate` returns; an empty result set proves the
    /// table exists.
    #[tokio::test]
    async fn job_lifecycle_migrate_creates_schema_tables() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        JobLifecycle::canonical().migrate(&handle).await.expect("migrate ok");

        let conn = handle.connection();
        let mut rc =
            conn.query("SELECT alloc_id, count FROM restart_counts", ()).await.expect("select rc");
        assert!(rc.next().await.expect("rc next").is_none(), "fresh restart_counts is empty");

        let mut lf = conn
            .query("SELECT alloc_id, ts FROM last_failure_seen_at", ())
            .await
            .expect("select lf");
        assert!(lf.next().await.expect("lf next").is_none(), "fresh last_failure_seen_at is empty");
    }

    /// AC 2 (idempotence) — `JobLifecycle::migrate` is safe to re-run
    /// against the same handle. The runtime calls migrate once at
    /// register time, but the `CREATE TABLE IF NOT EXISTS` shape means
    /// re-running on a refreshed handle (e.g. tests that bootstrap and
    /// re-open) MUST also succeed without error.
    #[tokio::test]
    async fn job_lifecycle_migrate_is_idempotent() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        let reconciler = JobLifecycle::canonical();
        reconciler.migrate(&handle).await.expect("first migrate");
        reconciler.migrate(&handle).await.expect("second migrate");

        // Tables still queryable after re-migration.
        let conn = handle.connection();
        conn.query("SELECT alloc_id FROM restart_counts", ()).await.expect("rc still queryable");
        conn.query("SELECT alloc_id FROM last_failure_seen_at", ())
            .await
            .expect("lf still queryable");
    }

    /// AC 3 — `NoopHeartbeat::migrate` is `Ok(())` (no schema).
    #[tokio::test]
    async fn noop_heartbeat_migrate_is_ok() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        NoopHeartbeat::canonical().migrate(&handle).await.expect("migrate ok");
    }

    /// AC 4 — `AnyReconciler::migrate` dispatches to the inner
    /// reconciler's `migrate` per the existing `hydrate`/`persist`
    /// dispatch shape.
    ///
    /// * `NoopHeartbeat`-arm dispatch must succeed against an empty DB.
    /// * `JobLifecycle`-arm dispatch must produce a queryable schema —
    ///   a follow-up `SELECT` against `restart_counts` succeeds.
    #[tokio::test]
    async fn any_reconciler_migrate_dispatches_correctly() {
        // NoopHeartbeat arm.
        {
            let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
            let any = AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical());
            any.migrate(&handle).await.expect("noop migrate");
        }
        // JobLifecycle arm.
        {
            let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
            let any = AnyReconciler::JobLifecycle(JobLifecycle::canonical());
            any.migrate(&handle).await.expect("job-lifecycle migrate");

            // Prove the dispatch reached the right impl: the schema is
            // present.
            let conn = handle.connection();
            conn.query("SELECT alloc_id, count FROM restart_counts", ())
                .await
                .expect("schema present after dispatch");
        }
    }

    /// Lifecycle invariant — `JobLifecycle::hydrate` against an
    /// un-migrated DB returns `HydrateError::Libsql` because the
    /// `SELECT` against the missing `restart_counts` table fails. This
    /// pins the contract: hydrate assumes a current schema; migrate is
    /// the only place that materialises it. A regression that quietly
    /// re-introduced `CREATE TABLE IF NOT EXISTS` into hydrate would
    /// flip this test from passing to failing.
    #[tokio::test]
    async fn job_lifecycle_hydrate_against_unmigrated_db_returns_libsql_error() {
        let handle = LibsqlHandle::open_in_memory().await.expect("open in-memory");
        // Note: NO migrate() call.
        let result = JobLifecycle::canonical().hydrate(&target(), &handle).await;
        match result {
            Err(crate::reconciler::HydrateError::Libsql(_)) => {} // expected
            other => panic!(
                "hydrate against un-migrated DB must return HydrateError::Libsql, got {other:?}"
            ),
        }
    }

    /// `open(path)` writes to a file on disk. Re-opening the same path
    /// after dropping the handle observes the previously-inserted row,
    /// proving the constructor went through `Builder::new_local(path)`
    /// rather than an in-memory shortcut.
    #[tokio::test]
    async fn open_file_backed_round_trips_across_handles() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("memory.db");

        // First handle: create table, insert row, drop.
        {
            let handle = LibsqlHandle::open(&path).await.expect("first open");
            let conn = handle.connection();
            conn.execute("CREATE TABLE t (x INTEGER)", ()).await.expect("create");
            conn.execute("INSERT INTO t (x) VALUES (42)", ()).await.expect("insert");
        }

        // Second handle against the same path: the row must persist.
        let handle = LibsqlHandle::open(&path).await.expect("second open");
        let conn = handle.connection();
        let mut rows = conn.query("SELECT x FROM t", ()).await.expect("select");
        let row = rows.next().await.expect("next row").expect("row present");
        let got: i64 = row.get(0).expect("x column");
        assert_eq!(got, 42);
    }
}
