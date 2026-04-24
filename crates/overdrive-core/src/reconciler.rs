//! Reconciler primitive — the §18 pure-function contract.
//!
//! Per ADR-0013 and whitepaper §18, a reconciler is a pure function over
//! `(desired, actual, db) -> Vec<Action>`. No `async fn`, no `.await`, no
//! `&dyn Clock` parameter, no direct store write, no wall-clock read. The
//! `Action::HttpCall` variant ships with the Phase 1 surface even though
//! the runtime shim that executes it lands Phase 3 (per development.md
//! §Reconciler I/O).
//!
//! `State` and `Db` are opaque placeholder handles in Phase 1 — the real
//! shapes land with step 04-04. Their presence here pins the trait
//! signature so downstream reconcilers (`noop-heartbeat` in 04-02, the
//! runtime shim in 04-06) can implement against a stable surface.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;

use crate::id::CorrelationKey;

// ---------------------------------------------------------------------------
// Reconciler trait
// ---------------------------------------------------------------------------

/// The §18 reconciler trait. Synchronous by design — purity is load-bearing.
///
/// Per ADR-0013 §2 the trait is pure over `(desired, actual, db) ->
/// Vec<Action>`. It is NOT `async fn`, does NOT take `&dyn Clock` /
/// `&dyn Transport` / `&dyn Entropy` parameters, and does NOT perform
/// I/O. External calls flow through `Action::HttpCall`; the runtime
/// observes responses via the ObservationStore on the next tick.
///
/// Compile-time enforcement: the acceptance test
/// `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
/// pins the signature via an `fn(&R, &State, &State, &Db) -> Vec<Action>`
/// type assertion. A regression that makes `reconcile` `async fn` or
/// adds a `&dyn Clock` parameter fails that test at compile time.
pub trait Reconciler: Send + Sync {
    /// Canonical name. Used for libSQL path derivation and evaluation
    /// broker keying.
    fn name(&self) -> &ReconcilerName;

    /// Pure function over `(desired, actual, db) -> Vec<Action>`. See
    /// whitepaper §18 and `.claude/rules/development.md` §Reconciler I/O.
    ///
    /// Purity contract: two invocations with the same inputs MUST
    /// produce byte-identical action vectors. The ADR-0017
    /// `reconciler_is_pure` invariant will evaluate this as a
    /// `Predicate::PureState` twin-invocation check against the full
    /// reconciler registry.
    fn reconcile(&self, desired: &State, actual: &State, db: &Db) -> Vec<Action>;
}

// ---------------------------------------------------------------------------
// State / Db placeholder handles
// ---------------------------------------------------------------------------

/// Opaque placeholder for the `desired` / `actual` state handed to a
/// reconciler. Phase 1 step 04-04 replaces with the real shape; the type
/// exists here so the `Reconciler` trait surface compiles and downstream
/// reconcilers can implement against it today.
#[derive(Debug, Default)]
pub struct State;

/// Opaque handle to a reconciler's private libSQL memory. Per ADR-0013,
/// one `&Db` handle per reconciler, exclusive to that reconciler,
/// provisioned by `libsql_provisioner::provision_db_path`. Phase 1 step
/// 04-04 replaces with the real handle type.
#[derive(Debug, Default)]
pub struct Db;

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
        correlation: CorrelationKey,
        // `String` rather than `http::Uri` / `http::Method` per ADR-0013
        // §4 — avoid pulling a transport dep onto the core compile path.
        // The runtime shim parses these.
        target: String,
        method: String,
        body: Bytes,
        timeout: Duration,
        idempotency_key: Option<String>,
    },

    /// Start a workflow. `WorkflowSpec` is a placeholder in Phase 1;
    /// workflow runtime lands Phase 3.
    StartWorkflow { spec: WorkflowSpec, correlation: CorrelationKey },
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
        // SAFETY: checked `is_empty` above.
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
    #[error("empty reconciler name")]
    Empty,
    #[error("reconciler name too long: {got} > 63")]
    TooLong { got: usize },
    #[error("reconciler name contains forbidden character: {found:?}")]
    ForbiddenCharacter { found: char },
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    #[error("empty target resource")]
    Empty,
    #[error("target resource has unknown shape: {raw}")]
    UnknownShape { raw: String },
}
