//! The `Workflow` primitive's author-facing surface — the §18 peer to
//! the pure-sync `Reconciler` (ADR-0035).
//!
//! A workflow is the one place in the codebase where `.await` on real
//! work is the *correct* shape (`.claude/rules/development.md`
//! § "Workflow contract"). Platform engineers author a workflow by
//! writing one ordinary `async fn run` against the [`Workflow`] trait —
//! no hand-written step enum, no transition match, no bespoke runtime.
//!
//! Per ADR-0064 §1 the **trait + ctx type + result + spec live in
//! `overdrive-core`** and pull **no `tokio`** into core: the async
//! signature is declared via `async_trait` (already a core dep, used by
//! `Driver` / `Transport` / `Llm`), and every source of non-determinism
//! flows through [`WorkflowCtx`]'s *injected port traits*
//! (`Arc<dyn Clock>` / `Arc<dyn Transport>` / `Arc<dyn Entropy>`) — the
//! same substitution the ports layer exists for. The *engine* that
//! actually drives `run`, polls the future, and writes the journal is
//! genuinely async, holds `tokio`, and lives in
//! `overdrive-control-plane` (later slices).
//!
//! `WorkflowCtx` is the workflow analogue of `TickContext`: a core-owned
//! bundle of injected non-determinism, DST-controllable, with no runtime
//! baked in. The dst-lint gate scans this module for `Instant::now` /
//! `rand::*` / `tokio::time::sleep` — the type definitions below contain
//! none; the ctx methods delegate to the injected traits.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

use crate::traits::transport::TransportError;
use crate::traits::{Clock, Entropy, Transport};

/// A durable-async workflow. The author writes one ordinary `async fn
/// run`; the engine (later slices) drives it to a terminal
/// [`WorkflowResult`], journaling each `ctx` await-point for
/// crash-resume.
///
/// The trait uses `async fn` via `async_trait` — declaring a
/// `Future`-returning signature does **not** require a runtime, so the
/// trait declaration is core-safe (ADR-0064 §1). All non-determinism in
/// the body MUST flow through [`WorkflowCtx`]; a body that reads
/// `Instant::now()` / `rand::*` / `tokio::time::sleep` directly breaks
/// journal replay and is rejected by the slice-01 dst-lint-style scan
/// (S-WP-01-03, later step).
#[async_trait]
pub trait Workflow: Send + Sync {
    /// Drive the workflow to a terminal [`WorkflowResult`]. Every
    /// non-deterministic input is read through `ctx`; the body contains
    /// no step cursor and no bespoke state machine.
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult;
}

/// A workflow's terminal value, returned from [`Workflow::run`].
///
/// **Distinct from `TerminalCondition`** (ADR-0037): that enum is the
/// *reconciler's* claim about an *allocation's* lifecycle, written onto
/// `AllocStatusRow`. `WorkflowResult` is the *workflow's own* return
/// value. They are related by composition (the workflow-lifecycle
/// reconciler may observe a `WorkflowResult` and emit a terminal claim
/// for the workflow instance's observation row) but model different
/// things and are **not substitutable** (ADR-0064 §2).
///
/// `#[non_exhaustive]` + the K8s-`Condition`-style SemVer convention
/// (well-known variants stable; new variants additive minor; renames
/// major) is inherited from ADR-0037 §5 — the *convention*, not the
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkflowResult {
    /// The workflow ran to a successful terminal.
    Success,

    /// The workflow ran to a failure terminal, carrying the cause.
    Failed {
        /// Operator-facing reason the workflow failed.
        reason: String,
    },

    /// The workflow was cancelled by an operator or its parent
    /// (forward-looking; the cancellation surface lands slice 03+).
    Cancelled,
}

/// The slice-01 request shape for [`WorkflowCtx::call`] — a single
/// external effect addressed at `target`, carrying `payload`. The
/// `ctx.call` op is the thinnest await-surface with a real,
/// non-idempotent-to-repeat effect (the ProvisionRecord write,
/// US-WP-1), performed through the injected [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRequest {
    /// Where the effect is addressed.
    pub target: SocketAddr,
    /// The request payload bytes.
    pub payload: Bytes,
}

/// The response returned by [`WorkflowCtx::call`]. Slice 01 carries the
/// observable result of the `Transport` effect — the number of bytes
/// the datagram delivered. Later slices grow this shape additively
/// alongside the journal `CallResult` entry (ADR-0064 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallResponse {
    /// Bytes delivered by the underlying transport effect.
    pub bytes_sent: usize,
}

/// Errors surfaced from [`WorkflowCtx`] await-ops.
#[derive(Debug, Error)]
pub enum WorkflowCtxError {
    /// The underlying [`Transport`] effect failed.
    #[error("workflow ctx.call transport error: {0}")]
    Transport(#[from] TransportError),
}

/// The injected non-determinism bundle handed to [`Workflow::run`] — the
/// workflow analogue of `TickContext`.
///
/// Carries the injected port traits only; no runtime, no wall-clock, no
/// RNG of its own. Every `ctx` await-op delegates to one of these
/// injected ports, which is exactly the substitution DST relies on
/// (production wires `Host*`; tests wire `Sim*`).
pub struct WorkflowCtx {
    clock: Arc<dyn Clock>,
    transport: Arc<dyn Transport>,
    entropy: Arc<dyn Entropy>,
}

impl WorkflowCtx {
    /// Construct a ctx over the injected ports. The ports are mandatory
    /// (no builder, no defaulting) per
    /// `.claude/rules/development.md` § "Port-trait dependencies" — a
    /// caller that forgets a port fails to compile rather than silently
    /// inheriting production behaviour.
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        transport: Arc<dyn Transport>,
        entropy: Arc<dyn Entropy>,
    ) -> Self {
        Self { clock, transport, entropy }
    }

    /// Perform one external effect through the injected [`Transport`]
    /// and return its observable result. The slice-01 await-surface
    /// (ADR-0064 §4) — the only `ctx` method this slice ships.
    ///
    /// In live execution (later slices) the engine appends a
    /// `CallResult` journal entry with fsync before returning; on replay
    /// it short-circuits to the recorded response without re-firing the
    /// effect (the exactly-once guarantee). This slice defines the
    /// surface; the journaling engine lands in `overdrive-control-plane`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::Transport`] when the underlying
    /// transport effect fails.
    pub async fn call(&self, request: CallRequest) -> Result<CallResponse, WorkflowCtxError> {
        let bytes_sent = self.transport.send_datagram(request.target, request.payload).await?;
        Ok(CallResponse { bytes_sent })
    }

    /// The injected clock. Workflow bodies read time only through this
    /// port (the `ctx.sleep` await-surface lands slice 02).
    #[must_use]
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// The injected entropy source. Workflow bodies read randomness only
    /// through this port.
    #[must_use]
    pub fn entropy(&self) -> &Arc<dyn Entropy> {
        &self.entropy
    }
}

/// Identity of a workflow to start. Kebab-case, `^[a-z][a-z0-9-]{0,62}$`
/// — the same shape as `ReconcilerName`, the peer primitive's identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowName(String);

/// Maximum length for a workflow name (1 lead + up to 62 interior).
const WORKFLOW_NAME_MAX: usize = 63;

impl WorkflowName {
    /// Validating constructor. Rejects empty, over-long, and
    /// non-`^[a-z][a-z0-9-]{0,62}$` inputs.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowNameError`] describing the first validation
    /// failure.
    pub fn new(raw: &str) -> Result<Self, WorkflowNameError> {
        if raw.is_empty() {
            return Err(WorkflowNameError::Empty);
        }
        if raw.len() > WORKFLOW_NAME_MAX {
            return Err(WorkflowNameError::TooLong { max: WORKFLOW_NAME_MAX });
        }
        let mut chars = raw.chars();
        let lead = chars.next().unwrap_or_else(|| {
            unreachable!("non-empty checked above guarantees at least one char")
        });
        if !lead.is_ascii_lowercase() {
            return Err(WorkflowNameError::BadShape);
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(WorkflowNameError::BadShape);
        }
        Ok(Self(raw.to_string()))
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validation failures for [`WorkflowName::new`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowNameError {
    /// The name was empty.
    #[error("workflow name must not be empty")]
    Empty,
    /// The name exceeded the length ceiling.
    #[error("workflow name too long (max {max})")]
    TooLong {
        /// The maximum permitted length.
        max: usize,
    },
    /// The name did not match `^[a-z][a-z0-9-]{0,62}$`.
    #[error("workflow name must match ^[a-z][a-z0-9-]{{0,62}}$")]
    BadShape,
}

/// The concrete workflow spec carried by `Action::StartWorkflow`
/// (ADR-0064 §1) — replaces the former unit placeholder at
/// `reconcilers/mod.rs`.
///
/// Slice 01 carries the workflow's identity; later slices grow the spec
/// additively (parameters, version) as the engine + lifecycle reconciler
/// land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSpec {
    /// Identity of the workflow to start.
    pub name: WorkflowName,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_name_accepts_canonical_kebab() {
        let name = WorkflowName::new("provision-record").expect("valid kebab name");
        assert_eq!(name.as_str(), "provision-record");
    }

    #[test]
    fn workflow_name_rejects_empty_uppercase_and_overlong() {
        assert!(matches!(WorkflowName::new(""), Err(WorkflowNameError::Empty)));
        assert!(matches!(WorkflowName::new("Provision"), Err(WorkflowNameError::BadShape)));
        assert!(matches!(
            WorkflowName::new(&"a".repeat(WORKFLOW_NAME_MAX + 1)),
            Err(WorkflowNameError::TooLong { .. })
        ));
    }
}
