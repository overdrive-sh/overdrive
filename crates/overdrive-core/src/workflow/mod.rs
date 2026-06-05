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

use crate::id::CorrelationKey;
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

    /// The engine's journal-cursor handle failed to durably record a
    /// live `ctx` await-point (append + fsync + advance). Per ADR-0063
    /// §4 (fsync-then-suspend) the engine MUST surface this rather than
    /// continue against an unjournaled effect — a resume would re-fire
    /// the effect, breaking exactly-once.
    #[error("workflow journal record failed: {message}")]
    JournalRecord {
        /// Cause string from the engine's journal handle.
        message: String,
    },
}

/// The engine-owned **journal-cursor handle** the [`WorkflowCtx`] consults
/// at every await-point — the core-side surface of the durable replay
/// cursor (ADR-0064 §1, §3).
///
/// # Why this is a trait in `overdrive-core`
///
/// Per ADR-0064 §1 the `WorkflowCtx` *type* lives in core and carries "a
/// journal-cursor handle whose concrete async I/O is performed by the
/// engine in `overdrive-control-plane`". This trait IS that handle: a
/// **declaration only**. Its methods speak in core types
/// ([`CallResponse`], [`CorrelationKey`]) and its single concrete
/// implementation — over `Arc<dyn JournalStore>` + a per-instance cursor
/// — lives in `overdrive-control-plane::workflow_runtime`, where tokio +
/// the real journal I/O are allowed. The trait declaration pulls no
/// runtime into core (it uses `async_trait`, already a core dep; the
/// dst-lint gate finds no `Instant::now` / `rand::*` / `tokio::*` here).
///
/// # The check-then-record contract (ADR-0064 §3)
///
/// Every `ctx` await-op is a check-then-record point. For `ctx.call` the
/// cursor is consulted via [`replay_call`](Self::replay_call):
///
/// - **Replay (cursor < journal length):** the handle returns
///   `Some(recorded)` — the recorded [`CallResponse`] for this step. The
///   ctx returns it WITHOUT firing the transport effect (the exactly-once
///   guarantee — a resumed run re-derives the response from the journal,
///   never re-performs the effect). The cursor advances.
/// - **Live (cursor == journal length):** the handle returns `None`. The
///   ctx fires the real effect through the injected port, then calls
///   [`record_call`](Self::record_call) to append the result entry with
///   fsync BEFORE returning and advance the cursor.
///
/// A handle whose `replay_call` always returns `None` and whose
/// `record_call` is a no-op models a non-durable "always-live" execution
/// — the shape the core/sim tests inject when no real journal is wired
/// (see [`AlwaysLiveCursor`]).
#[async_trait]
pub trait JournalCursor: Send + Sync {
    /// Check the cursor for a recorded `ctx.call` at the current step.
    ///
    /// # Postconditions
    ///
    /// Returns `Some(response)` when replaying (a recorded entry exists at
    /// the cursor for `correlation`) — the caller MUST NOT fire the
    /// effect and MUST return the recorded response. Returns `None` when
    /// live (cursor at the journal end) — the caller fires the effect and
    /// then calls [`record_call`](Self::record_call). Implementations
    /// advance the cursor on a replay hit.
    async fn replay_call(&self, correlation: &CorrelationKey) -> Option<CallResponse>;

    /// Record a freshly-fired `ctx.call` result durably and advance the
    /// cursor (the live path).
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the response is durably journaled (append + fsync per
    /// ADR-0063 §4) and the cursor has advanced past this step, so a
    /// subsequent resume replays it via [`replay_call`](Self::replay_call)
    /// without re-firing.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::JournalRecord`] when the durable
    /// append/fsync fails — the engine surfaces this rather than continue
    /// against an unjournaled effect.
    async fn record_call(
        &self,
        correlation: &CorrelationKey,
        response: &CallResponse,
    ) -> Result<(), WorkflowCtxError>;
}

/// A trivial [`JournalCursor`] that never replays and never records — it
/// models a **non-durable, always-live** execution.
///
/// Used by the core author-surface acceptance test (S-WP-01-01) and any
/// caller that drives a [`Workflow`] without a real journal wired: every
/// `ctx.call` fires its effect (no replay short-circuit) and nothing is
/// persisted (no-op record). The durable handle — which DOES replay and
/// record — lives in `overdrive-control-plane::workflow_runtime`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysLiveCursor;

#[async_trait]
impl JournalCursor for AlwaysLiveCursor {
    async fn replay_call(&self, _correlation: &CorrelationKey) -> Option<CallResponse> {
        None
    }

    async fn record_call(
        &self,
        _correlation: &CorrelationKey,
        _response: &CallResponse,
    ) -> Result<(), WorkflowCtxError> {
        Ok(())
    }
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
    /// The engine-owned durable replay cursor (ADR-0064 §1, §3). Every
    /// `ctx` await-op consults it: replay short-circuits to the recorded
    /// result, live fires-then-records. The concrete impl over a real
    /// journal lives in `overdrive-control-plane`; core tests inject
    /// [`AlwaysLiveCursor`].
    journal: Arc<dyn JournalCursor>,
}

/// The `purpose` component of the [`CorrelationKey`] derived for a
/// `ctx.call` await-point. Stable so replay re-derives the identical key.
const CALL_PURPOSE: &str = "workflow-ctx-call";

impl WorkflowCtx {
    /// Construct a ctx over the injected ports + the engine's journal
    /// cursor. All are mandatory (no builder, no defaulting) per
    /// `.claude/rules/development.md` § "Port-trait dependencies" — a
    /// caller that forgets a port fails to compile rather than silently
    /// inheriting production behaviour.
    ///
    /// Drivers that run a workflow without a durable journal (the core
    /// author-surface test, S-WP-01-01) pass
    /// `Arc::new(AlwaysLiveCursor)`; the durable engine passes its
    /// real journal-cursor handle.
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        transport: Arc<dyn Transport>,
        entropy: Arc<dyn Entropy>,
        journal: Arc<dyn JournalCursor>,
    ) -> Self {
        Self { clock, transport, entropy, journal }
    }

    /// Perform one external effect through the injected [`Transport`]
    /// and return its observable result — the slice-01 await-surface
    /// (ADR-0064 §4), the only `ctx` method this slice ships.
    ///
    /// **Check-then-record (ADR-0064 §3).** The op first consults the
    /// engine's journal cursor:
    /// - **Replay:** if the cursor has a recorded response for this
    ///   step, it is returned WITHOUT firing the transport effect — the
    ///   exactly-once guarantee on resume (K1).
    /// - **Live:** otherwise the effect fires through the injected
    ///   [`Transport`], the result is durably recorded (append + fsync
    ///   per ADR-0063 §4) via the cursor BEFORE returning, and the
    ///   cursor advances.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::Transport`] when the underlying
    /// transport effect fails, or [`WorkflowCtxError::JournalRecord`]
    /// when the live-path durable record fails.
    pub async fn call(&self, request: CallRequest) -> Result<CallResponse, WorkflowCtxError> {
        // Correlation is deterministic across attempts/replays: derived
        // from (target, payload-digest, purpose) so a resumed run
        // re-derives the identical key and finds its recorded response
        // (ADR-0035 § Reconciler I/O rule 2; ADR-0064 §3).
        let payload_digest = crate::id::ContentHash::of(&request.payload);
        let correlation =
            CorrelationKey::derive(&request.target.to_string(), &payload_digest, CALL_PURPOSE);

        // Replay path — return the recorded response, never re-fire.
        if let Some(recorded) = self.journal.replay_call(&correlation).await {
            return Ok(recorded);
        }

        // Live path — fire the real effect, then durably record before
        // returning (fsync-then-suspend, ADR-0063 §4).
        let bytes_sent = self.transport.send_datagram(request.target, request.payload).await?;
        let response = CallResponse { bytes_sent };
        self.journal.record_call(&correlation, &response).await?;
        Ok(response)
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
