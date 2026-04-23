//! Reconciler primitive — the §18 pure-function contract.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0013 and whitepaper §18, a reconciler is a pure function over
//! `(desired, actual, db) -> Vec<Action>`. No `async fn`, no `.await`, no
//! `&dyn Clock` parameter, no direct store write, no wall-clock read. The
//! `Action::HttpCall` variant ships with the Phase 1 surface even though
//! the runtime shim that executes it lands Phase 3 (per development.md
//! §Reconciler I/O).
//!
//! The DELIVER crafter replaces every `panic!("Not yet implemented -- RED
//! scaffold")` body below with the real implementation.

use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;

use crate::id::CorrelationKey;

// ---------------------------------------------------------------------------
// Reconciler trait
// ---------------------------------------------------------------------------

/// The §18 reconciler trait. Synchronous by design — purity is load-bearing.
///
/// SCAFFOLD: true
pub trait Reconciler: Send + Sync {
    /// Canonical name. Used for libSQL path derivation and evaluation
    /// broker keying.
    fn name(&self) -> &ReconcilerName;

    /// Pure function over `(desired, actual, db) -> Vec<Action>`. See
    /// whitepaper §18 and `.claude/rules/development.md` §Reconciler I/O.
    fn reconcile(&self, desired: &State, actual: &State, db: &Db) -> Vec<Action>;
}

// ---------------------------------------------------------------------------
// State / Db placeholder handles
// ---------------------------------------------------------------------------

/// Opaque placeholder for the `desired` / `actual` state handed to a
/// reconciler. Phase 1 will flesh this out; for now the type exists so
/// the `Reconciler` trait surface compiles.
///
/// SCAFFOLD: true
pub struct State;

/// Opaque handle to a reconciler's private libSQL memory. Per ADR-0013,
/// one `&Db` handle per reconciler, exclusive to that reconciler,
/// provisioned by `libsql_provisioner::provision_db_path`.
///
/// SCAFFOLD: true
pub struct Db;

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// Actions a reconciler can emit. Phase 1 ships `Noop`, `HttpCall`, and a
/// `StartWorkflow` placeholder (workflow runtime lands Phase 3).
///
/// SCAFFOLD: true
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
        target: String, // placeholder for http::Uri — avoid new dep in core
        method: String, // placeholder for http::Method
        body: Bytes,
        timeout: Duration,
        idempotency_key: Option<String>,
    },

    /// Start a workflow. `WorkflowSpec` is a placeholder in Phase 1;
    /// workflow runtime lands Phase 3.
    StartWorkflow { spec: WorkflowSpec, correlation: CorrelationKey },
}

/// Placeholder for the workflow spec. Phase 3 replaces with real shape.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSpec;

// ---------------------------------------------------------------------------
// ReconcilerName newtype
// ---------------------------------------------------------------------------

/// Canonical reconciler name. Kebab-case, `^[a-z][a-z0-9-]{0,62}$`. The
/// strict character set lets the libSQL path provisioner safely
/// concatenate the name into a filesystem path without sanitisation.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReconcilerName(String);

impl ReconcilerName {
    /// Validating constructor. Rejects empty, uppercase, leading digit,
    /// path-traversal characters (`.`, `..`, `/`, `\`, `:`).
    ///
    /// SCAFFOLD: true
    pub fn new(_raw: &str) -> Result<Self, ReconcilerNameError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Canonical string form.
    ///
    /// SCAFFOLD: true
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ReconcilerName {
    type Err = ReconcilerNameError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

/// Errors from `ReconcilerName::new`.
///
/// SCAFFOLD: true
#[derive(Debug, thiserror::Error)]
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

/// Target-resource component of the evaluation broker's key.
///
/// The broker is keyed on `(ReconcilerName, TargetResource)` per
/// whitepaper §18. Phase 1 carries a canonical string form; Phase 2+
/// may refine into a typed sum over concrete resource kinds.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetResource(String);

impl TargetResource {
    /// Validating constructor. Accepts canonical forms like
    /// `job/<JobId>`, `node/<NodeId>`, `alloc/<AllocationId>`.
    ///
    /// SCAFFOLD: true
    pub fn new(_raw: &str) -> Result<Self, TargetResourceError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Canonical string form.
    ///
    /// SCAFFOLD: true
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TargetResource {
    type Err = TargetResourceError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

/// Errors from `TargetResource::new`.
///
/// SCAFFOLD: true
#[derive(Debug, thiserror::Error)]
pub enum TargetResourceError {
    #[error("empty target resource")]
    Empty,
    #[error("target resource has unknown shape: {raw}")]
    UnknownShape { raw: String },
}
