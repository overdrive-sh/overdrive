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
//!     // The ONLY place a reconciler author touches libSQL. Phase 1
//!     // reconcilers hold no memory, so this is trivially Ok(()).
//!     async fn hydrate(
//!         &self,
//!         _target: &TargetResource,
//!         _db: &LibsqlHandle,
//!     ) -> Result<Self::View, HydrateError> {
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
/// Per ADR-0013, one `&LibsqlHandle` per reconciler, exclusive to that
/// reconciler, provisioned by the runtime from the per-primitive libSQL
/// path. Phase 1 reconcilers use `type View = ()` and do not touch the
/// handle; Phase 2+ reconcilers will gain public query/exec methods on
/// `LibsqlHandle` when a first concrete author needs them.
///
/// The type is real (not a unit-like empty placeholder) so the trait
/// signature is stable: `hydrate`'s async surface already takes a real
/// handle type, and downstream authors can implement against it today.
#[derive(Debug, Clone)]
pub struct LibsqlHandle {
    // Phase 1: the connection handle is `Option::None` because no
    // current reconciler opens its DB. The field exists so the newtype
    // is genuinely a wrapper around the eventual `Arc<libsql::Connection>`
    // shape — the crate-private constructor produces `None`; Phase 2+
    // wires the real connection.
    //
    // Typed as `Arc<()>` for now rather than `Arc<libsql::Connection>`
    // so the core crate does not pull libsql onto its compile graph
    // until a reconciler author actually needs a connection. The
    // architectural intent — one `Arc`-shared handle, cheap to clone,
    // opaque from the caller's perspective — is preserved.
    _handle: Option<Arc<()>>,
}

impl LibsqlHandle {
    /// Crate-private constructor. The runtime in
    /// `overdrive-control-plane::reconciler_runtime` is the intended
    /// caller; Phase 1 does not yet open any DB so the method is not
    /// reached from within this crate.
    ///
    /// Phase 1 produces an empty handle; Phase 2+ wires the real
    /// libsql connection.
    #[must_use]
    #[allow(dead_code)] // Reserved for the 04-09+ reconciler-runtime wiring.
    pub(crate) const fn empty() -> Self {
        Self { _handle: None }
    }

    /// Phase 1 default handle — no underlying libSQL connection. The
    /// runtime tick loop hands this to every `Reconciler::hydrate`
    /// call until Phase 2+ wires per-primitive libSQL files.
    /// Reconcilers that touch the handle in Phase 1 are a bug — every
    /// Phase 1 reconciler's `View = ()` (or carries no row data) and
    /// returns `Ok(default)` without using the handle.
    #[must_use]
    pub const fn default_phase1() -> Self {
        Self { _handle: None }
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
/// * `hydrate` is async — the ONLY place a reconciler author touches
///   libSQL. Returns the author-declared `View` type (typically a
///   struct decoded from a row set).
/// * `reconcile` is pure and synchronous — no `.await`, no I/O, no
///   wall-clock read (only via `tick.now`), no direct store write. The
///   returned `(Vec<Action>, Self::View)` tuple carries actions the
///   runtime commits through Raft and the next-view the runtime diffs
///   against `view` and persists back to libSQL.
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

    /// Async read phase. The ONLY place a reconciler author touches
    /// libSQL. Free-form SQL lives here; schema management (CREATE
    /// TABLE IF NOT EXISTS, ALTER TABLE ADD COLUMN) lives here too —
    /// no framework migrations in Phase 1.
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
    /// or [`HydrateError::Schema`] on schema-level mismatch.
    fn hydrate(
        &self,
        target: &TargetResource,
        db: &LibsqlHandle,
    ) -> impl std::future::Future<Output = Result<Self::View, HydrateError>> + Send;

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

    async fn hydrate(
        &self,
        _target: &TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
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
/// Maximum number of restart attempts before the `JobLifecycle`
/// reconciler stops emitting `RestartAllocation` for a persistently
/// failing alloc. Per US-03 step 02-03 — the ceiling exists to keep a
/// repeatedly-crashing workload from consuming infinite driver
/// resources.
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

    async fn hydrate(
        &self,
        _target: &TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        // Phase 1 02-02 carries the View shape; the libSQL hydrate
        // path itself (CREATE TABLE IF NOT EXISTS, SELECT decode)
        // lands in 02-03 alongside the runtime tick loop. For 02-02
        // a fresh empty View is sufficient — the convergence loop is
        // not yet driven, so the View has no rows to materialise.
        Ok(JobLifecycleView::default())
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
/// tick. The earlier shape (`next_attempt_at: Instant`) was a stale
/// cache of `Instant::now() + RESTART_BACKOFF_DURATION`; rotating the
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
