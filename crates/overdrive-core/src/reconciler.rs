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
//!     State, TargetResource, TickContext,
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
//!         _desired: &State,
//!         _actual: &State,
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

use crate::id::CorrelationKey;

// ---------------------------------------------------------------------------
// TickContext — time as injected input state
// ---------------------------------------------------------------------------

/// Time injected into `reconcile` as pure input.
///
/// The runtime constructs exactly one `TickContext` per evaluation by
/// snapshotting the injected `Clock` trait once — reconcilers must
/// read wall-clock via `tick.now` rather than calling `Instant::now()`
/// directly (dst-lint enforces this at PR time).
///
/// * `now` — the wall-clock instant the evaluation started.
/// * `tick` — a monotonic counter useful as a deterministic
///   tie-breaker across evaluations.
/// * `deadline` — the runtime's per-tick budget. Reconcilers that need
///   to checkpoint bounded work into their `NextView` consult this.
#[derive(Debug, Clone)]
pub struct TickContext {
    /// Wall-clock snapshot taken by the runtime at evaluation start.
    pub now: Instant,
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
/// `fn(&R, &State, &State, &R::View, &TickContext) -> (Vec<Action>, R::View)`
/// type assertion. A regression that makes `reconcile` `async fn` or
/// adds a `&dyn Clock` parameter fails that test at compile time.
pub trait Reconciler: Send + Sync {
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
        desired: &State,
        actual: &State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View);
}

// ---------------------------------------------------------------------------
// State placeholder
// ---------------------------------------------------------------------------

/// Opaque placeholder for the `desired` / `actual` state handed to a
/// reconciler.
///
/// Phase 2+ replaces with the real shape when a reconciler dereferences
/// it; Phase 1 reconcilers (just `NoopHeartbeat`) treat `State` as
/// opaque.
#[derive(Debug, Default)]
pub struct State;

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
        _desired: &State,
        _actual: &State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        (vec![Action::Noop], ())
    }
}

// ---------------------------------------------------------------------------
// HarnessNoopHeartbeat — DST canary-bug fixture (test-only)
// ---------------------------------------------------------------------------

/// Canary-bug reconciler used by the `overdrive-sim` DST harness to
/// prove the `ReconcilerIsPure` invariant actually catches divergences.
///
/// When compiled without the `canary-bug` feature, behaves exactly
/// like `NoopHeartbeat` — returns `vec![Action::Noop]`. When the
/// feature is enabled, the reconciler flips its output on every call
/// (even calls return one `Noop`, odd calls return two), which the
/// twin-invocation check MUST flag as a purity violation.
///
/// The type lives here — not in `overdrive-sim` — so `AnyReconciler`
/// can hold it in a conditionally-compiled variant. The mutants-skip
/// entry for `harness_purity_reconciler` (in `.cargo/mutants.toml`) is
/// updated to reference the new path below.
#[cfg(feature = "canary-bug")]
pub struct HarnessNoopHeartbeat {
    name: ReconcilerName,
}

#[cfg(feature = "canary-bug")]
impl HarnessNoopHeartbeat {
    /// Construct the canary-bug `noop-heartbeat` harness fixture.
    ///
    /// # Panics
    ///
    /// Never — see [`NoopHeartbeat::canonical`].
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new("noop-heartbeat")
            .expect("'noop-heartbeat' is a valid ReconcilerName by construction");
        Self { name }
    }
}

#[cfg(feature = "canary-bug")]
impl Reconciler for HarnessNoopHeartbeat {
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
        _desired: &State,
        _actual: &State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CALL: AtomicU64 = AtomicU64::new(0);
        let n = CALL.fetch_add(1, Ordering::SeqCst);
        if n % 2 == 0 { (vec![Action::Noop], ()) } else { (vec![Action::Noop, Action::Noop], ()) }
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
/// Phase 1 ships exactly one production variant: `NoopHeartbeat`. The
/// canary-bug feature adds `HarnessNoopHeartbeat` — available only
/// when the crate is compiled with the `canary-bug` feature enabled.
pub enum AnyReconciler {
    /// The Phase 1 proof-of-life reconciler. See [`NoopHeartbeat`].
    NoopHeartbeat(NoopHeartbeat),
    /// DST canary-bug fixture — deliberately non-deterministic when
    /// the `canary-bug` feature is enabled. See
    /// [`HarnessNoopHeartbeat`].
    #[cfg(feature = "canary-bug")]
    HarnessNoopHeartbeat(HarnessNoopHeartbeat),
}

impl AnyReconciler {
    /// Canonical name of the inner reconciler.
    #[must_use]
    pub fn name(&self) -> &ReconcilerName {
        match self {
            Self::NoopHeartbeat(r) => r.name(),
            #[cfg(feature = "canary-bug")]
            Self::HarnessNoopHeartbeat(r) => r.name(),
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
            #[cfg(feature = "canary-bug")]
            Self::HarnessNoopHeartbeat(r) => {
                r.hydrate(target, db).await.map(|()| AnyReconcilerView::Unit)
            }
        }
    }

    /// Pure compute phase — dispatches to the inner reconciler's
    /// `reconcile`. The caller supplies the matching view variant.
    ///
    /// Variant alignment is a compile-time invariant: the dispatch
    /// `match` below is exhaustive, and every arm pairs an
    /// `AnyReconciler` variant with its declared [`AnyReconcilerView`]
    /// counterpart. Adding a new reconciler variant whose `View` type
    /// does not line up with a matching `AnyReconcilerView` arm
    /// produces a non-exhaustive-match compile error, forcing the
    /// developer to extend the dispatch explicitly. There is no
    /// runtime fallback — a mismatched pair cannot be constructed in
    /// the first place.
    ///
    /// Phase 1 has only `View = ()`, so every arm routes through
    /// [`AnyReconcilerView::Unit`]; Phase 2+ widens the sum as new
    /// reconcilers land.
    #[must_use]
    pub fn reconcile(
        &self,
        desired: &State,
        actual: &State,
        view: &AnyReconcilerView,
        tick: &TickContext,
    ) -> (Vec<Action>, AnyReconcilerView) {
        match (self, view) {
            (Self::NoopHeartbeat(r), AnyReconcilerView::Unit) => {
                let (actions, ()) = r.reconcile(desired, actual, &(), tick);
                (actions, AnyReconcilerView::Unit)
            }
            #[cfg(feature = "canary-bug")]
            (Self::HarnessNoopHeartbeat(r), AnyReconcilerView::Unit) => {
                let (actions, ()) = r.reconcile(desired, actual, &(), tick);
                (actions, AnyReconcilerView::Unit)
            }
        }
    }
}

/// Sum of every view type produced by `AnyReconciler::hydrate`. Phase 1
/// only has `View = ()` so the sum carries a single `Unit` variant;
/// Phase 2+ adds real variants as new reconcilers land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyReconcilerView {
    /// The `View = ()` variant used by Phase 1 reconcilers.
    Unit,
}
