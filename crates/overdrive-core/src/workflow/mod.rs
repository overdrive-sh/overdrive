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

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
use crate::reconcilers::Action;
use crate::traits::{Clock, Entropy, Transport};

/// The Clock-park interval the live `ctx.wait_for_signal` block re-polls
/// the signal surface at while the signal is ABSENT (ADR-0064 §4).
///
/// This is a *deadline park*, not a busy-spin: under `SimClock` the
/// harness advances logical time past this interval to wake the park
/// (and writes the signal row in the same advance window); under
/// `SystemClock` it is a real Tokio timer. The value is the poll
/// granularity of the in-process single-node signal delivery — small
/// enough that a freshly-written signal is observed promptly, large
/// enough that an absent signal does not burn CPU.
const SIGNAL_POLL: Duration = Duration::from_millis(50);

/// A durable-async workflow. The author writes one ordinary `async fn
/// run` over a typed `Input` and `Output`; the engine (later slices)
/// drives it — via the object-safe [`ErasedWorkflow`] erasure — to a
/// terminal [`WorkflowStatus`], journaling each `ctx` await-point for
/// crash-resume (ADR-0065 §1).
///
/// # Object safety via the typed-edge / CBOR-erased-interior split
///
/// The associated `Input` / `Output` types make `Workflow` **not**
/// object-safe (a method's signature mentions `Self::Input` /
/// `Self::Output`). This is deliberate: the AUTHOR edge is typed (the
/// body returns a real `Self::Output`, not bytes), while the ENGINE
/// drives the object-safe [`ErasedWorkflow`] whose interior is `&[u8]` /
/// `Vec<u8>` CBOR. The generic [`ErasedWorkflowAdapter`] bridges them —
/// the SAME typed-edge / erased-interior split [`WorkflowCtx::run`]
/// already performs for step results. The composition root registers a
/// TYPED workflow; the adapter is applied internally, so the author
/// never writes the erasure.
///
/// # Behavior contract
///
/// - **Preconditions:** `input` is the typed `Self::Input` the start
///   intent's opaque CBOR bytes decoded to (the [`ErasedWorkflowAdapter`]
///   performs the decode; a decode failure never reaches the body — it
///   becomes a [`TerminalError::malformed_input`] before `run` is
///   called). Every non-deterministic input is read through `ctx`.
/// - **Postconditions:** `Ok(output)` is the workflow's typed terminal
///   value (the engine erases it to CBOR and projects to
///   [`WorkflowStatus::Completed`]); `Err(terminal)` is the body's
///   authored terminal FAILURE (projected to [`WorkflowStatus::Failed`]).
/// - **Edge cases:** an `Output` of `()` is the contentless terminal the
///   old contentless success modelled — it CBOR-encodes to a small fixed
///   value the consumer decodes back to `()`. A body that panics is
///   contained by the engine (`catch_unwind`) and converged to
///   `Failed { TerminalError::explicit(<deterministic panic detail>) }`
///   — the panic never escapes the engine task.
/// - **Invariants:** the body contains no step cursor and no bespoke
///   state machine; a body that reads `Instant::now()` / `rand::*` /
///   `tokio::time::sleep` directly breaks journal replay and is rejected
///   by the dst-lint-style scan (S-WP-01-03).
///
/// The trait uses `async fn` via `async_trait` — declaring a
/// `Future`-returning signature does **not** require a runtime, so the
/// trait declaration is core-safe (ADR-0064 §1).
#[async_trait]
pub trait Workflow: Send + Sync {
    /// The workflow's typed terminal-success value. CBOR-serialisable so
    /// the engine erases it to bytes for [`WorkflowStatus::Completed`].
    type Output: serde::Serialize + serde::de::DeserializeOwned + Send + Sync;

    /// The workflow's typed start input. CBOR-serialisable so the
    /// [`ErasedWorkflowAdapter`] decodes the start intent's opaque
    /// `input` bytes into it before calling [`Self::run`].
    type Input: serde::Serialize + serde::de::DeserializeOwned + Send + Sync;

    /// Drive the workflow over `input` to its typed terminal. Every
    /// non-deterministic input is read through `ctx`; the body contains
    /// no step cursor and no bespoke state machine. `Ok(Output)` is the
    /// terminal success value; `Err(TerminalError)` is the authored
    /// terminal failure (the explicit "do not retry; fail the workflow"
    /// signal — ADR-0065 §2).
    async fn run(
        &self,
        ctx: &WorkflowCtx,
        input: Self::Input,
    ) -> Result<Self::Output, TerminalError>;
}

/// The object-safe engine-facing surface of a [`Workflow`] (ADR-0065 §1).
///
/// `Workflow` is not object-safe (its associated `Input` / `Output`
/// appear in its method signature), so the engine cannot hold a
/// `Box<dyn Workflow>`. `ErasedWorkflow` is the erasure: a single
/// `run_erased` whose interior is concrete `&[u8]` / `Vec<u8>` CBOR —
/// object-safe, so `Box<dyn ErasedWorkflow>` compiles. The engine drives
/// THIS; the generic [`ErasedWorkflowAdapter`] is the sole bridge from a
/// typed `Workflow` to it.
///
/// # Behavior contract
///
/// - **Preconditions:** `input_bytes` are the start intent's opaque CBOR
///   `W::Input` bytes (recorded by the reconciler, replayed verbatim by
///   the engine). The engine never interprets them — only the adapter
///   decodes.
/// - **Postconditions:** `Ok(output_bytes)` is the CBOR encoding of the
///   body's `W::Output` (the engine projects it to
///   [`WorkflowStatus::Completed`] verbatim).
///   an `Err` of [`WorkflowDriveError::Terminal`] is the body's authored
///   failure, a decode failure ([`TerminalError::malformed_input`]), or an
///   encode failure ([`TerminalError::output_encode`]) (projected to
///   [`WorkflowStatus::Failed`]). An `Err` of [`WorkflowDriveError::Transient`]
///   is a [`WorkflowCtx::run`] step whose closure resolved to
///   `Err(StepError::Retryable)` — the engine ABSORBS and re-drives it
///   (ADR-0065 §4), and it NEVER becomes a durable terminal.
/// - **Edge cases:** undecodable `input_bytes` ⇒ `Terminal(malformed_input)`
///   and the typed body is NEVER entered; an `Output` whose serde impl fails
///   to encode ⇒ `Terminal(output_encode)`; a `ctx.run` step whose closure
///   resolved to `Err(StepError::Retryable)` ⇒ `Transient`.
/// - **Invariants:** `run_erased` is the only engine entry point; the
///   typed edge (decode in / encode out) is owned by the adapter, so the
///   engine stays type-agnostic. A transient is surfaced as
///   [`WorkflowDriveError::Transient`] HERE — never through the body's
///   `Result<Output, TerminalError>` (ADR-0065 §2/§4 Model Z). The body
///   cannot swallow-and-continue past a transient: a `ctx.run` transient
///   PARKS the body (its await never returns), and `run_erased` polls the
///   body, detects the recorded transient, and DROPS the parked body
///   (cancellation) — so a transient structurally pre-empts whatever the
///   body would have returned.
#[async_trait]
pub trait ErasedWorkflow: Send + Sync {
    /// Decode `input_bytes` into the workflow's typed `Input`, drive the
    /// body, and CBOR-encode its typed `Output`. See the trait contract
    /// for the precise pre/postconditions and error mapping (including the
    /// [`WorkflowDriveError::Transient`] re-drive channel).
    async fn run_erased(
        &self,
        ctx: &WorkflowCtx,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, WorkflowDriveError>;
}

/// The generic blanket bridge from a typed [`Workflow`] `W` to the
/// object-safe [`ErasedWorkflow`] (ADR-0065 §1).
///
/// Wraps a concrete `W: Workflow` and implements [`ErasedWorkflow`] by:
/// CBOR-decoding `input_bytes` into `W::Input` (a decode error becomes
/// [`TerminalError::malformed_input`], so the typed body is never
/// entered on undecodable input), calling `W::run`, and CBOR-encoding
/// `W::Output` (an encode error becomes [`TerminalError::output_encode`]).
/// This is the single site that crosses the typed author edge ↔ erased
/// engine interior — the author never writes it; the composition root
/// applies it when registering a typed workflow.
pub struct ErasedWorkflowAdapter<W: Workflow>(pub W);

#[async_trait]
impl<W: Workflow> ErasedWorkflow for ErasedWorkflowAdapter<W> {
    async fn run_erased(
        &self,
        ctx: &WorkflowCtx,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, WorkflowDriveError> {
        // One poll's outcome of driving the body (ADR-0065 §4 Model Z): either
        // the body resolved (`Body`) or a `ctx.run` transient was recorded this
        // poll (`Transient`). Declared at the top of the scope to satisfy
        // `clippy::items_after_statements`.
        enum DriveStep<O> {
            Body(Result<O, TerminalError>),
            Transient,
        }

        // Typed-edge IN: decode the opaque start input into W::Input. An
        // undecodable input is a malformed-input TERMINAL — the body is NEVER
        // entered (the bytes will not change on re-drive; ADR-0065 §2/§4).
        let input: W::Input = ciborium::from_reader(input_bytes)
            .map_err(|e| TerminalError::malformed_input(&e.to_string()))?;

        // Drive the typed body, watching for a PARKED TRANSIENT (ADR-0065 §4
        // Model Z). A `ctx.run` step whose closure resolved to
        // `Err(StepError::Retryable)` records a transient in the ctx and PARKS the
        // body (its await never returns on the transient path). We poll the body
        // and, the moment a transient is recorded — whether the body is Pending
        // (parked) or Ready — surface `Transient` and DROP the parked body
        // (cancellation); the engine re-drives. A genuine park (`ctx.sleep` /
        // `wait_for_signal` with no transient) stays Pending. The transient takes
        // precedence over a body that completed in the same poll, preserving the
        // prior "ctx transient overrides the body return" guarantee.
        //
        // LOAD-BEARING INVARIANT: every body `Pending` is either waker-registered
        // (a genuine `ctx.sleep` / `wait_for_signal` park registers `cx`'s waker, so
        // the Clock re-polls us) OR transient-flagged (caught here in the same poll).
        // A future returning `Pending` WITHOUT registering a waker AND WITHOUT setting
        // the transient slot would hang this drive forever — no `ctx` op does that,
        // and a `ctx.run` transient sets the slot synchronously before its `pending()`.
        let mut body = self.0.run(ctx, input);
        let step: DriveStep<W::Output> = std::future::poll_fn(|cx| {
            use std::task::Poll;
            match body.as_mut().poll(cx) {
                Poll::Ready(result) => {
                    if ctx.has_transient_step() {
                        Poll::Ready(DriveStep::Transient)
                    } else {
                        Poll::Ready(DriveStep::Body(result))
                    }
                }
                Poll::Pending => {
                    if ctx.has_transient_step() {
                        Poll::Ready(DriveStep::Transient)
                    } else {
                        Poll::Pending
                    }
                }
            }
        })
        .await;

        match step {
            DriveStep::Transient => {
                let transient = ctx
                    .take_transient_step()
                    .unwrap_or_else(|| unreachable!("has_transient_step() was true this poll"));
                Err(WorkflowDriveError::Transient(transient))
            }
            DriveStep::Body(body_result) => {
                // Body's `Err(TerminalError)` → `WorkflowDriveError::Terminal` (manual From; TerminalError is !Error).
                let output = body_result?;
                // Typed-edge OUT: encode W::Output to CBOR. An encode failure is a
                // programming error in the Output type's serde impl (output_encode).
                let mut output_bytes: Vec<u8> = Vec::new();
                ciborium::into_writer(&output, &mut output_bytes)
                    .map_err(|e| TerminalError::output_encode(&e.to_string()))?;
                Ok(output_bytes)
            }
        }
    }
}

/// Maximum byte length of a [`TerminalError`] `detail`, enforced at
/// construction (ADR-0065 §2).
///
/// Over-long detail is truncated to the largest UTF-8 char boundary at or
/// below this cap (see [`TerminalError::cap_detail`]). The cap closes the
/// free-text replay-determinism hazard ADR-0064 §3 identified: a
/// `TerminalError` rides in the durable journal `Terminal` command and the
/// terminal observation row as an INPUT, and an unbounded author-supplied
/// (or panic-derived) detail is a standing invitation to embed an
/// arbitrarily-large — and on the panic path, potentially non-deterministic
/// — value into the durable terminal. Bounding it at construction makes the
/// durable terminal's bytes stable and small.
///
/// 1 KiB is generous for an operator-facing reason string (the cause is the
/// structured [`TerminalErrorKind`]; `detail` is human context, not a
/// payload) while keeping the durable terminal compact.
pub const TERMINAL_ERROR_DETAIL_MAX: usize = 1024;

/// The terminal-failure channel of a workflow body (ADR-0065 §2).
///
/// A workflow body that returns `Err(TerminalError)` ALWAYS ends with a
/// **terminal failure** — the explicit "do not retry; fail the workflow"
/// signal (the Restate `TerminalError` / Temporal non-retryable
/// `ApplicationFailure` shape). It is PURELY TERMINAL: a `TerminalError` the
/// body returns is NEVER re-driven (ADR-0065 §2).
///
/// **RETRYABLE failures never construct this — the body cannot express
/// "retry me" through its return type.** A transient is signalled at the
/// STEP level: a [`WorkflowCtx::run`] step whose closure resolves
/// to an `Err` of [`StepError::Retryable`] surfaces a
/// [`WorkflowCtxError::TransientStep`] the engine ABSORBS and re-drives
/// (the [`WorkflowDriveError::Transient`] drive outcome; ADR-0065 §4
/// channel — "a `ctx.run` step whose `Err` is re-driven by the engine").
/// The transient lives in the `ctx` and the erased drive outcome, never in
/// the body's `Result<Output, TerminalError>` — which is why the four kinds
/// below are *all* genuinely terminal.
///
/// `TerminalError` is the workflow analogue of [`WorkflowCtxError`] but
/// models a different thing and is **not substitutable** with it:
/// `WorkflowCtxError` is an *engine-internal* await-op failure (journal
/// record failed, non-deterministic replay); `TerminalError` is the
/// *body's authored terminal-failure outcome*. It is `Serialize` /
/// `Deserialize` because it rides in the durable journal `Terminal` command
/// and the terminal observation row as an input. It deliberately does NOT
/// implement [`std::error::Error`] (its `Display` is hand-written, rendering
/// the structured kind plus the bounded detail) — the anyhow/eyre `!Error`
/// coherence trick (ADR-0065 Gap 1) that lets [`StepError`] carry both a
/// blanket `From<E: std::error::Error>` and a `From<TerminalError>` without
/// the two colliding. The hand-written `Display` keeps it composable with the
/// typed-error discipline the rest of core uses
/// (`.claude/rules/development.md` § Errors).
///
/// # Construction contract
///
/// Built ONLY through the validating constructors ([`Self::explicit`],
/// [`Self::malformed_input`], [`Self::output_encode`], and the engine-only
/// [`Self::budget_exhausted`]); the fields are private. Each constructor
/// length-caps `detail` at [`TERMINAL_ERROR_DETAIL_MAX`] deterministically
/// (per the newtype-completeness discipline) so an over-long input can never
/// reach the durable terminal.
///
/// # Consumed by the trait reshape (step 01-03)
///
/// The [`Workflow`] trait's `run` returns `Result<Self::Output,
/// TerminalError>`; the [`ErasedWorkflowAdapter`] mints `MalformedInput` /
/// `OutputEncode` at the CBOR erasure boundary, and the engine projects an
/// `Err(TerminalError)` to [`WorkflowStatus::Failed`] (ADR-0065 §1/§3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalError {
    /// The bounded, structured cause — NOT free-text. The replay-determinism
    /// hazard of a free-`String` reason (ADR-0064 §3) is closed by making
    /// the cause a typed [`TerminalErrorKind`].
    kind: TerminalErrorKind,
    /// Author-supplied (or, on the panic-containment path, deterministic
    /// panic-message-derived) detail. Length-capped at construction
    /// ([`TERMINAL_ERROR_DETAIL_MAX`]) and recorded as an INPUT in the
    /// durable terminal — deterministic because it is author-data, not
    /// engine-derived state. The structured cause is [`TerminalErrorKind`];
    /// this is human context, never a payload.
    detail: String,
}

impl TerminalError {
    /// Truncate `detail` to the largest UTF-8 char boundary at or below
    /// [`TERMINAL_ERROR_DETAIL_MAX`] bytes.
    ///
    /// Deterministic: identical input always yields identical output (a
    /// load-bearing property — the durable terminal must replay
    /// bit-identically). UTF-8-safe: truncation never splits a multi-byte
    /// scalar (a raw byte-index slice could panic / corrupt), so an over-long
    /// detail collapses to a valid prefix, not a torn one.
    fn cap_detail(detail: &str) -> String {
        if detail.len() <= TERMINAL_ERROR_DETAIL_MAX {
            return detail.to_string();
        }
        // Largest char boundary <= the cap. `floor_char_boundary` is not yet
        // stable, so walk char indices: the last index that starts at or
        // before the cap is the boundary to slice at.
        let boundary = detail
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|&idx| idx <= TERMINAL_ERROR_DETAIL_MAX)
            .last()
            .unwrap_or(0);
        detail[..boundary].to_string()
    }

    /// The author explicitly threw a terminal failure
    /// ([`TerminalErrorKind::Explicit`]). `detail` is length-capped at
    /// construction.
    #[must_use]
    pub fn explicit(detail: &str) -> Self {
        Self { kind: TerminalErrorKind::Explicit, detail: Self::cap_detail(detail) }
    }

    /// The start input could not be CBOR-decoded into the workflow's typed
    /// `Input` ([`TerminalErrorKind::MalformedInput`]) — the
    /// `ErasedWorkflowAdapter` decode failure. Not retryable (the bytes will
    /// not change on re-drive). `detail` is length-capped at construction.
    #[must_use]
    pub fn malformed_input(detail: &str) -> Self {
        Self { kind: TerminalErrorKind::MalformedInput, detail: Self::cap_detail(detail) }
    }

    /// The typed `Output` could not be CBOR-encoded
    /// ([`TerminalErrorKind::OutputEncode`]) — the `ErasedWorkflowAdapter`
    /// encode failure, a programming error in the `Output` type's serde
    /// impl. `detail` is length-capped at construction.
    #[must_use]
    pub fn output_encode(detail: &str) -> Self {
        Self { kind: TerminalErrorKind::OutputEncode, detail: Self::cap_detail(detail) }
    }

    /// The engine minted this terminal because the retry budget was
    /// exhausted ([`TerminalErrorKind::BudgetExhausted`], ADR-0065 §4).
    ///
    /// **Engine-minted, never body-authored.** A workflow body cannot
    /// produce `BudgetExhausted` through its own logic — it signals
    /// transients at the step level via [`WorkflowCtx::run`], and
    /// the ENGINE mints this once the journal-derived attempt count reaches
    /// `WORKFLOW_RETRY_BUDGET`.
    /// The ctor is `pub` so the engine (in `overdrive-control-plane`, a
    /// separate crate) can mint it on exhaustion; authors have no reason to
    /// call it and would only produce a misleading terminal. `detail` is
    /// length-capped at construction.
    #[must_use]
    pub fn budget_exhausted(detail: &str) -> Self {
        Self { kind: TerminalErrorKind::BudgetExhausted, detail: Self::cap_detail(detail) }
    }

    /// The structured cause of this terminal failure.
    #[must_use]
    pub const fn kind(&self) -> TerminalErrorKind {
        self.kind
    }

    /// The (length-capped) author-supplied detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Hand-written `Display` (ADR-0065 Gap 1). `TerminalError` deliberately does
/// NOT derive [`thiserror::Error`] / implement [`std::error::Error`]: it is the
/// anyhow/eyre `!Error` coherence trick that lets [`StepError`] carry BOTH a
/// blanket `From<E: std::error::Error>` (→ `Retryable`) AND a
/// `From<TerminalError>` (→ `Terminal`) without the two `From` impls colliding.
/// The structured cause is [`TerminalErrorKind`]; this renders it plus the
/// bounded detail (the same shape the prior `#[error(..)]` derive produced).
impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workflow terminal failure ({:?}): {}", self.kind, self.detail)
    }
}

/// The structured cause of a [`TerminalError`] (ADR-0065 §2).
///
/// `#[non_exhaustive]` — the well-known variants are stable; a new cause is
/// an additive minor change (the K8s-`Condition` / ADR-0037 SemVer
/// convention). Every match on `TerminalErrorKind` outside this crate must
/// carry a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TerminalErrorKind {
    /// The author explicitly threw a terminal failure (the Restate
    /// `TerminalError` / Temporal non-retryable `ApplicationFailure` shape).
    /// Constructed via [`TerminalError::explicit`].
    Explicit,
    /// The engine minted this terminal because the retry budget was
    /// exhausted (ADR-0065 §4). Author code never constructs this — the
    /// engine does, on exhaustion, via [`TerminalError::budget_exhausted`].
    BudgetExhausted,
    /// The start input could not be CBOR-decoded into the workflow's typed
    /// `Input` (the `ErasedWorkflowAdapter` decode failure). A
    /// malformed-input terminal — not retryable (the bytes will not change
    /// on re-drive). Constructed via [`TerminalError::malformed_input`].
    MalformedInput,
    /// The typed `Output` could not be CBOR-encoded (the
    /// `ErasedWorkflowAdapter` encode failure). A programming error in the
    /// `Output` type's serde impl. Constructed via
    /// [`TerminalError::output_encode`].
    OutputEncode,
}

/// A per-[`WorkflowCtx::run`] retry policy — the analogue of Restate's
/// `RunRetryPolicy` (ADR-0065 Amendment 2026-06-07, Gap 2).
///
/// Set on a step via the [`RunStep`] builder
/// (`ctx.run(name, fut).retry_policy(p).await?`); the FAILING step's policy
/// governs the engine's whole-workflow re-drive decision (the
/// re-drive-from-journal model of ADR-0064 is KEPT — the policy controls only
/// HOW MANY times and HOW LONG the engine re-drives, never WHETHER a transient
/// reaches the body, which it never does). The five fields mirror Restate's
/// `RunRetryPolicy` exactly.
///
/// # Not persisted — derived from the workflow CODE each drive
///
/// `RunRetryPolicy` is **NOT journaled** (per `.claude/rules/development.md`
/// § "Persist inputs, not derived state"). The policy is a function of the
/// body's builder call, so it is re-derived identically on every re-drive when
/// the body re-executes the failing step (the replay-equivalence contract
/// forbids a body that changes its `.retry_policy(..)` across drives). The
/// journal persists only the attempt INPUTS — the `RetryAttempted` command
/// count and the first attempt's `started_at_unix` — from which the engine
/// recomputes `attempts` and the elapsed window each drive. The engine learns
/// the failing step's policy because it RIDES the transient signal
/// (`WorkflowCtxError::TransientStep { .. policy }`), never read from a store.
///
/// # `Default` reproduces today's engine behaviour bit-for-bit
///
/// [`Self::default`] equals the pre-Gap-2 engine constants exactly:
/// `max_attempts == WORKFLOW_RETRY_BUDGET` (3) and an
/// `(initial_delay, exponentiation_factor, max_delay)` of `(50ms, 2.0, 200ms)`
/// whose computed per-attempt backoff
/// (`initial_delay * exponentiation_factor^attempts`, clamped to `max_delay`)
/// reproduces the engine's prior `backoff_for_attempt` schedule (`50 / 100 /
/// 200`ms, clamped) for every attempt index. `max_duration` defaults to
/// [`Duration::MAX`] so the elapsed-window gate NEVER fires under the default
/// (today has no duration bound; the attempt-count gate is what fires first).
/// The `WORKFLOW_RETRY_BUDGET` + `backoff_for_attempt` constants in
/// `overdrive-control-plane` are RETAINED as the default-policy SSOT (this
/// `Default` is observably equivalent to them; a control-plane unit test pins
/// the equivalence, since `overdrive-core` cannot reference the engine crate's
/// constant directly).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunRetryPolicy {
    /// The backoff window before the FIRST re-drive (attempt 0). The window
    /// before the `attempts`-th re-drive is
    /// `initial_delay * exponentiation_factor^attempts`, clamped to
    /// [`Self::max_delay`].
    pub initial_delay: Duration,
    /// The geometric growth factor applied to [`Self::initial_delay`] per
    /// attempt. `2.0` doubles the window each re-drive (the default).
    pub exponentiation_factor: f64,
    /// The ceiling the computed backoff window is clamped to — no re-drive
    /// parks longer than this regardless of attempt index.
    pub max_delay: Duration,
    /// The maximum number of transient RE-DRIVES before the engine mints
    /// [`TerminalError::budget_exhausted`]. The default is the engine's
    /// `WORKFLOW_RETRY_BUDGET` (3).
    pub max_attempts: u32,
    /// The maximum wall-clock duration the retry window may span (measured
    /// from the first attempt's journaled `started_at_unix`). Exhaustion on
    /// EITHER `max_attempts` OR `max_duration` mints `BudgetExhausted`. The
    /// default is [`Duration::MAX`] — effectively unbounded, so the
    /// attempt-count gate fires first (matching pre-Gap-2 behaviour, which had
    /// no duration bound).
    pub max_duration: Duration,
}

impl Default for RunRetryPolicy {
    /// The default policy reproduces the pre-Gap-2 engine behaviour exactly
    /// (the observable-equivalence requirement, ADR-0065 Gap 2): `max_attempts`
    /// == the engine's `WORKFLOW_RETRY_BUDGET` (3), and an `(initial_delay,
    /// exponentiation_factor, max_delay)` whose computed backoff reproduces the
    /// engine's prior `backoff_for_attempt` (`50 / 100 / 200`ms, clamped) for
    /// every attempt index. `max_duration` is unbounded so the duration gate
    /// never fires under the default.
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(50),
            exponentiation_factor: 2.0,
            max_delay: Duration::from_millis(200),
            max_attempts: 3,
            max_duration: Duration::MAX,
        }
    }
}

impl RunRetryPolicy {
    /// The backoff window to park before the re-drive AFTER `attempts`
    /// re-drives have already been recorded — `initial_delay *
    /// exponentiation_factor^attempts`, clamped to [`Self::max_delay`]
    /// (ADR-0065 Gap 2). `attempts` is the journal-derived `RetryAttempted`
    /// count (0-indexed: the window before the first re-drive is `attempts ==
    /// 0`).
    ///
    /// Deterministic and total: the geometric product is computed via
    /// [`Duration`]'s own `f64` helpers (no manual numeric casts), then clamped
    /// to `max_delay` in the `f64` domain BEFORE rebuilding the `Duration`, so a
    /// non-finite / overflowing product (huge `attempts` or
    /// `exponentiation_factor`) collapses to `max_delay` via the fallible
    /// [`Duration::try_from_secs_f64`] rather than panicking. With the
    /// [`Self::default`] policy this reproduces the engine's prior
    /// `backoff_for_attempt` schedule bit-for-bit (`50 / 100 / 200 / 200 / …`ms;
    /// pinned by the engine-side `default_policy_reproduces_engine_constants`
    /// test).
    #[must_use]
    pub fn backoff_window(&self, attempts: u32) -> Duration {
        // `powi` wants an `i32` exponent; a `u32` attempt count past `i32::MAX`
        // is absurd (and would only make the factor larger), so saturate.
        let exponent = i32::try_from(attempts).unwrap_or(i32::MAX);
        let factor = self.exponentiation_factor.powi(exponent);
        // A non-finite or negative factor (degenerate policy) collapses to the
        // ceiling — total, no panic.
        if !factor.is_finite() || factor < 0.0 {
            return self.max_delay;
        }
        let scaled_secs = self.initial_delay.as_secs_f64() * factor;
        let max_secs = self.max_delay.as_secs_f64();
        // Clamp in the f64 domain FIRST so `try_from_secs_f64` is always within
        // range (a huge/non-finite product collapses to `max_secs`); `min`
        // propagates NaN's right operand on most paths, so re-guard after.
        let clamped = scaled_secs.min(max_secs);
        if !clamped.is_finite() || clamped < 0.0 {
            return self.max_delay;
        }
        Duration::try_from_secs_f64(clamped).unwrap_or(self.max_delay)
    }
}

/// The externally-observable terminal status of a workflow INSTANCE — the
/// engine's projection of the body's `Result<Output, TerminalError>` PLUS
/// the engine-observed events the body cannot author (cancel, timeout)
/// (ADR-0065 §3).
///
/// Written to the `workflow_terminal` observation row keyed by the instance
/// `CorrelationKey`; the workflow-lifecycle reconciler observes it to
/// converge the instance. **Distinct** from the body's return type (the crux
/// of the ADR-0065 research finding — the body return and the control-plane
/// status are two different types) and from `TerminalCondition` (ADR-0037,
/// the reconciler's *allocation* claim — same SemVer convention, different
/// type; the `compile_fail/workflow_status_vs_terminal_condition.rs` fixture
/// structurally enforces the non-substitutability).
///
/// `#[non_exhaustive]` — the K8s-`Condition` / ADR-0037 SemVer convention
/// (well-known variants stable; new variants additive minor; renames major).
/// Every match outside this crate must carry a `_` arm.
///
/// # Variant ownership
///
/// `Completed` / `Failed` are the engine's projection of the body's
/// `Ok(Output)` / `Err(TerminalError)`. `Cancelled` / `TimedOut` are
/// **engine-authored** — the body can never return them. They are
/// **forward variants the Phase-1 engine never writes** (no cancel / deadline
/// surface yet); they are declared now so the projection's shape is honest
/// about what the control plane will record and the lifecycle reconciler's
/// match is exhaustive against them from day one.
///
/// # The durable terminal surface (step 01-03)
///
/// `ObservationRow::WorkflowTerminal` and the journal `Terminal` command
/// both carry a `WorkflowStatus` (the prior contentless terminal enum was
/// deleted in the same step). The engine projects the body's
/// `Result<Output, TerminalError>` here; the workflow-lifecycle reconciler
/// observes `Some(status)` to converge the instance (ADR-0065 §3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum WorkflowStatus {
    /// The body returned `Ok(Output)`. Carries the erased CBOR `Output` bytes
    /// (the workflow's real output — the contentful replacement for the
    /// pre-ADR-0065 contentless success terminal).
    Completed {
        /// The CBOR-encoded `Output` the body produced (opaque to the
        /// engine; the consumer decodes into the workflow's typed `Output`).
        output: Vec<u8>,
    },
    /// The body returned `Err(TerminalError)`, OR the engine minted a
    /// terminal on budget exhaustion. Carries the [`TerminalError`]
    /// (kind + detail).
    Failed {
        /// The terminal failure cause + detail.
        terminal: TerminalError,
    },
    /// The control plane cancelled the instance (delivered INTO the body as a
    /// terminal at the next await point — ADR-0065 §4 forward; the cancel
    /// surface is a later slice). Engine-authored; the body cannot return
    /// this.
    Cancelled,
    /// The instance exceeded its wall-clock deadline (engine-observed;
    /// forward — the deadline surface is a later slice). The body cannot
    /// return this.
    TimedOut,
}

/// The [`WorkflowCtx`] await-op surface a replay-path divergence was
/// detected on (ADR-0063 §2 journaled await-ops). Carried by
/// [`WorkflowCtxError::NonDeterministic`] so the rendered message names
/// the surface that actually diverged — `replay_sleep` /
/// `replay_signal` / `replay_emit` mismatches must NOT be reported with
/// the `ctx.run` prefix the variant once hard-coded for every site.
///
/// This is a label enum: it owns its `ctx`-method string representation
/// via [`as_str`](Self::as_str), per `.claude/rules/development.md`
/// § "Label enums own their string representation".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwaitOp {
    /// `ctx.run` — a journaled step result (`replay_run`).
    Run,
    /// `ctx.sleep` — a journaled durable timer (`replay_sleep`).
    Sleep,
    /// `ctx.wait_for_signal` — a journaled signal await (`replay_signal`).
    Signal,
    /// `ctx.emit_action` — a journaled cluster mutation (`replay_emit`).
    EmitAction,
}

impl AwaitOp {
    /// The author-facing `ctx` method name for this await-op — the
    /// canonical label rendered in [`WorkflowCtxError::NonDeterministic`].
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "ctx.run",
            Self::Sleep => "ctx.sleep",
            Self::Signal => "ctx.wait_for_signal",
            Self::EmitAction => "ctx.emit_action",
        }
    }
}

impl std::fmt::Display for AwaitOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Engine-internal workflow errors, RETAINED in two roles after the
/// Model Z amendment (ADR-0065 §4) — but **no longer a [`WorkflowCtx`]
/// await-op return type.**
///
/// The `WorkflowCtx` await-ops (`run` / `sleep` / `wait_for_signal` /
/// `emit_action`) now return `Result<_, TerminalError>`: an infra
/// `WorkflowCtxError` raised inside a ctx op is **projected** to
/// [`TerminalError::explicit`] at the ctx-op boundary (via
/// `infra_terminal`) so a workflow body composes the ops with a
/// clean `?` against its own `Result<Output, TerminalError>` return. This type
/// therefore survives only as:
///
/// - (a) the **transient-slot carrier** —
///   <code>[WorkflowDriveError::Transient]([WorkflowCtxError::TransientStep])</code>,
///   the channel that keeps a retryable step OFF the body's return type; and
/// - (b) the **internal infra-error type** the [`JournalCursor`] surface
///   returns, produced *inside* a ctx op and projected to [`TerminalError`]
///   before the body ever sees it.
///
/// It remains **non-substitutable** with [`TerminalError`] (corrected ADR-0065
/// §2): the infra projection is a deliberate one-way classification, not a
/// bidirectional `From`.
#[derive(Debug, Error)]
pub enum WorkflowCtxError {
    /// A `ctx.run` step's result could not be CBOR-serialised before
    /// recording it in the journal. The step's closure produced a value
    /// whose `Serialize` impl failed — surfaced rather than recording a
    /// truncated/garbled result.
    #[error("workflow ctx.run serialize failed: {message}")]
    Serialize {
        /// Cause string from the CBOR encoder.
        message: String,
    },

    /// A recorded `ctx.run` result could not be CBOR-deserialised back
    /// into the step's result type on the replay path. Indicates schema
    /// skew between the recorded bytes and the type the workflow body
    /// expects — surfaced rather than fabricating a default.
    #[error("workflow ctx.run deserialize failed: {message}")]
    Deserialize {
        /// Cause string from the CBOR decoder.
        message: String,
    },

    /// A replay-path await-op found a recorded command whose identity does
    /// not match the one the workflow body is replaying at this cursor
    /// position — a non-deterministic divergence between the recorded
    /// trajectory and the current run. Fail-closed: a workflow body that
    /// reorders / renames its await-ops cannot replay a journal recorded
    /// against the prior shape (journal replay must be bit-identical,
    /// `development.md` § "Workflow contract").
    ///
    /// `op` names the await-op surface the divergence was detected on
    /// (`replay_run` / `replay_sleep` / `replay_signal` / `replay_emit`)
    /// so the rendered message is honest about which `ctx` method
    /// diverged — it is NOT always `ctx.run`.
    #[error("workflow {op} non-deterministic: expected {expected:?}, got {actual:?}")]
    NonDeterministic {
        /// The await-op surface the divergence was detected on.
        op: AwaitOp,
        /// The command identity (step name / command kind / signal key)
        /// recorded in the journal at this cursor.
        expected: String,
        /// The command identity the replaying workflow body presented.
        actual: String,
    },

    /// A [`WorkflowCtx::run`] step's closure resolved to an `Err` of
    /// [`StepError::Retryable`] — a TRANSIENT step failure the engine
    /// ABSORBS and re-drives (ADR-0065 §4: "a `ctx.run` step whose `Err` is
    /// re-driven by the engine"). The author rarely names [`StepError`]:
    /// any [`std::error::Error`] propagated with `?` inside the closure is
    /// auto-absorbed into the retryable transient (anyhow-style — see
    /// [`StepError`]). This is the body's ONLY transient channel,
    /// and it is engine-internal: the engine projects it to
    /// [`WorkflowDriveError::Transient`] at the [`ErasedWorkflow::run_erased`]
    /// boundary and re-drives the body from the journal (the step's result
    /// was NOT journaled, so the step re-fires; completed steps replay
    /// byte-equal) until the transient clears (→ `Completed`) or the budget
    /// is exhausted (→ engine-minted [`TerminalError::budget_exhausted`]).
    ///
    /// It is a [`WorkflowCtxError`], NOT a [`TerminalError`]: the body cannot
    /// express "retry me" through its `Result<Output, TerminalError>` return
    /// type (ADR-0065 §2). Carries the step `name` and the closure's `detail`
    /// for diagnostics, plus the failing step's [`RunRetryPolicy`] (Gap 2) so
    /// the engine's re-drive decision consults the PER-STEP policy rather than a
    /// global constant. Neither `name` nor `detail` rides the durable terminal
    /// (a transient never becomes a terminal — the engine's minted
    /// `BudgetExhausted` does, with its own detail), and the `policy` is NOT
    /// persisted either (it is re-derived from the body each drive; ADR-0065
    /// Gap 2 sub-decision 3).
    #[error("workflow ctx.run step {name:?} failed transiently: {detail}")]
    TransientStep {
        /// The transient step's name (the `ctx.run` name argument).
        name: String,
        /// The closure-supplied transient detail
        /// ([`StepError::detail`] of the `Retryable` arm).
        detail: String,
        /// The failing step's per-step retry policy (ADR-0065 Gap 2). Rides
        /// the transient signal so the engine's re-drive decision
        /// (`redrive_decision`) consults THIS step's `max_attempts` /
        /// `max_duration` / backoff schedule rather than the global
        /// `WORKFLOW_RETRY_BUDGET` constant. The default
        /// ([`RunRetryPolicy::default`]) reproduces the pre-Gap-2 behaviour
        /// when the step set no explicit policy.
        policy: RunRetryPolicy,
    },

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

    /// An [`WorkflowCtx::emit_action`] could not hand the typed [`Action`]
    /// to the engine's Action channel (slice 03, ADR-0064 §4). The channel
    /// the reconciler runtime consumes (→ Raft) was closed or full — the
    /// engine surfaces this rather than drop the cluster mutation silently.
    #[error("workflow ctx.emit_action channel send failed: {message}")]
    ActionChannel {
        /// Cause string from the engine's Action-channel sender.
        message: String,
    },

    /// A [`WorkflowCtx::wait_for_signal`] could not read the typed signal
    /// surface (slice 03, ADR-0064 §4). The engine reads typed signal rows
    /// from the `ObservationStore`; an underlying read failure is surfaced
    /// rather than treated as "signal absent".
    #[error("workflow ctx.wait_for_signal failed: {message}")]
    Signal {
        /// Cause string from the engine's signal-read path.
        message: String,
    },
}

/// The error a `ctx.run` step closure may resolve to — a `retryable | terminal`
/// union (the analogue of Restate's `HandlerError`; ADR-0065 Gap 1). Named
/// `StepError` (not `HandlerError`) because in our system it is specifically the
/// `ctx.run` step-closure error, not a whole-handler error.
///
/// A step author rarely names it: any `std::error::Error` propagated with `?`
/// inside the closure folds into [`StepError::Retryable`] via the blanket
/// `From` below, so the natural shape is `Ok(op().await?)`. To fail a step
/// PERMANENTLY, return `Err(TerminalError::explicit(..).into())` (or
/// `StepError::terminal(..)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// Transient — the engine re-drives the workflow. NEVER reaches the body's
    /// `Result<Output, TerminalError>` return type.
    Retryable {
        /// Operator-facing transient detail (carried into the engine-minted
        /// `BudgetExhausted` terminal on exhaustion).
        detail: String,
    },
    /// Permanent — surfaces as `Err(TerminalError)` from `ctx.run(...).await`
    /// with NO retry and NO re-drive.
    Terminal(TerminalError),
}

impl StepError {
    /// Construct a retryable transient (replaces `RetryableStepError::new`).
    #[must_use]
    pub fn retryable(detail: &str) -> Self {
        Self::Retryable { detail: detail.to_string() }
    }

    /// Construct a terminal step failure (or use `TerminalError`'s `.into()`).
    #[must_use]
    pub const fn terminal(terminal: TerminalError) -> Self {
        Self::Terminal(terminal)
    }

    /// The human detail of either arm.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Retryable { detail } => detail,
            Self::Terminal(terminal) => terminal.detail(),
        }
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable { detail } => write!(f, "retryable step failure: {detail}"),
            Self::Terminal(terminal) => write!(f, "terminal step failure: {terminal}"),
        }
    }
}

/// Anyhow-style blanket: any [`std::error::Error`] propagated with `?` inside a
/// `ctx.run` closure folds into a RETRYABLE transient. Coherent ONLY because
/// both `StepError` and `TerminalError` are `!std::error::Error` (the
/// anyhow/eyre trick) — so this blanket neither collides with the reflexive
/// `From<StepError>` nor with `From<TerminalError>` below.
impl<E: std::error::Error> From<E> for StepError {
    fn from(err: E) -> Self {
        Self::Retryable { detail: err.to_string() }
    }
}

/// A `TerminalError` returned / `?`-propagated inside a `ctx.run` closure makes
/// the step TERMINAL (no retry). Coherent with the blanket above because
/// `TerminalError` is `!std::error::Error`.
impl From<TerminalError> for StepError {
    fn from(terminal: TerminalError) -> Self {
        Self::Terminal(terminal)
    }
}

/// The outcome of driving a workflow body once through the object-safe
/// [`ErasedWorkflow::run_erased`] edge — the engine's three-way
/// classification (ADR-0065 §3/§4).
///
/// `run_erased` returns `Result<Vec<u8>, WorkflowDriveError>`:
/// - `Ok(output_bytes)` — the body returned `Ok(Output)`; the engine
///   projects [`WorkflowStatus::Completed`].
/// - an `Err` of [`Self::Terminal`] — the body returned `Err(TerminalError)`,
///   or the adapter minted a decode/encode terminal; the engine projects
///   [`WorkflowStatus::Failed`] (a body-authored terminal is NEVER
///   re-driven).
/// - an `Err` of [`Self::Transient`] — a [`WorkflowCtx::run`] step whose
///   closure resolved to `Err(StepError::Retryable)`; the engine ABSORBS it
///   and re-drives the body from
///   the journal (budget-gated), minting `BudgetExhausted` on exhaustion.
///
/// This is the type that keeps the transient OFF the body's return type: the
/// transient is a `WorkflowDriveError::Transient`, distinct from a
/// `TerminalError`, surfaced by the engine's erased edge rather than the
/// author's typed `Result<Output, TerminalError>`.
#[derive(Debug, Error)]
pub enum WorkflowDriveError {
    /// A real terminal — projected to `WorkflowStatus::Failed`; never re-driven.
    ///
    /// Renders via [`TerminalError`]'s hand-written [`Display`](std::fmt::Display)
    /// (`#[error("{0}")]`, not `transparent`): `transparent` would require
    /// `TerminalError: std::error::Error`, which it deliberately is NOT under
    /// ADR-0065 Gap 1 (the `!Error` coherence trick for [`StepError`]). The
    /// `#[from]` is likewise replaced by the manual `From` impl below — a
    /// derived `#[from]` needs the source field to be an `Error`.
    #[error("{0}")]
    Terminal(TerminalError),
    /// A transient step the engine absorbs and re-drives (ADR-0065 §4).
    #[error(transparent)]
    Transient(#[from] WorkflowCtxError),
}

/// Manual `From<TerminalError>` (ADR-0065 Gap 1) — replaces the derived
/// `#[from]` on the `Terminal` arm, which `thiserror` cannot generate now that
/// `TerminalError` is `!std::error::Error`. Keeps the `?`-into-`WorkflowDriveError`
/// ergonomic in [`ErasedWorkflowAdapter::run_erased`].
impl From<TerminalError> for WorkflowDriveError {
    fn from(terminal: TerminalError) -> Self {
        Self::Terminal(terminal)
    }
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
/// **declaration only**. Its methods speak in core types (CBOR result
/// bytes + step names) and its single concrete implementation — over
/// `Arc<dyn JournalStore>` + a per-instance cursor — lives in
/// `overdrive-control-plane::workflow_runtime`, where tokio + the real
/// journal I/O are allowed. The trait declaration pulls no runtime into
/// core (it uses `async_trait`, already a core dep; the dst-lint gate
/// finds no `Instant::now` / `rand::*` / `tokio::*` here).
///
/// # The check-then-record contract (ADR-0064 §3)
///
/// Every `ctx` await-op is a check-then-record point. The durable handle
/// (`overdrive-control-plane::workflow_runtime`) partitions the loaded run
/// ONCE at construction into a positional **command** walk
/// (`Vec<JournalCommand>`) plus a `SignalKey`-correlated **notification**
/// lookup (`BTreeMap<SignalKey, JournalNotification>`) — D2 / ADR-0064 §3,
/// CA-5. Two contracts govern the two classes:
///
/// ## Command-advance contract (commands advance the cursor, by exactly 1)
///
/// **Post:** a replay hit at command-index N returns the recorded
/// [`JournalCommand`]'s result and advances the command-cursor by exactly
/// 1. A live op appends the command durably (append + fsync, ADR-0063 §4)
/// and advances by exactly 1.
///
/// **Invariant:** the cursor advances over `JournalCommand`s ONLY —
/// `Started`, `RunResult`, `SleepArmed`, `SignalAwaited`, `ActionEmitted`,
/// `Terminal`. A notification (`SignalSeen`) NEVER advances it; recording a
/// notification leaves the command-cursor where it was. There is no
/// `*cursor += 2` two-positional-entry walk — that conflation of an
/// armed-command with a satisfied-notification is RETIRED (the trap CA-5
/// closes).
///
/// ## Notification-lookup contract (`SignalSeen` resolved by key, never position)
///
/// **Post:** a `SignalSeen` is resolved by [`SignalKey`] lookup in the
/// `BTreeMap<SignalKey, JournalNotification>` — never by position. On a
/// `ctx.wait_for_signal` replay the cursor points at the `SignalAwaited`
/// COMMAND; the matching `SignalSeen` is found off the walk by its key,
/// wherever it landed in the interleaved on-disk stream, and the
/// command-cursor advances by exactly 1 (past the `SignalAwaited`). A
/// `SignalAwaited` command with NO matching `SignalSeen` notification (the
/// "crashed while still blocked" shape) is NOT a replay hit — `replay_signal`
/// returns `None` so the live path re-blocks on the SAME key.
///
/// **Invariant:** a notification is never consumed AS a command — it lives
/// off the positional command walk entirely, so it can never be mistaken
/// for a `SignalAwaited` (or any other command) at a cursor position.
///
/// For `ctx.run` the cursor is consulted via
/// [`replay_run`](Self::replay_run):
///
/// - **Replay (cursor < journal length):** the handle returns
///   `Ok(Some(recorded_bytes))` — the recorded CBOR result for this step.
///   The ctx CBOR-decodes them into the step's result type and returns it
///   WITHOUT polling the step's future (the exactly-once guarantee on the
///   replay path — a resumed run re-derives the result from the journal,
///   never re-performs the effect). The cursor advances. If the recorded
///   step's name does not match `name`, the handle returns
///   `Err(WorkflowCtxError::NonDeterministic { .. })` (fail-closed).
/// - **Live (cursor == journal length):** the handle returns `Ok(None)`.
///   The ctx polls the step's future, then calls
///   [`record_run`](Self::record_run) to append the result bytes with
///   fsync BEFORE returning and advance the cursor.
///
/// A handle whose `replay_run` always returns `Ok(None)` and whose
/// `record_run` is a no-op models a non-durable "always-live" execution
/// — the shape the core/sim tests inject when no real journal is wired
/// (see [`AlwaysLiveCursor`]).
///
/// ## Determinism fail-closed contract (D4, ADR-0064 §3 — Layers 1 + 2)
///
/// Every command-replay method (`replay_run`, `replay_sleep`,
/// `replay_signal`, `replay_emit`) gates the recorded command at the
/// command-cursor against the await-op being replayed, in two layers:
///
/// - **Layer 1 — type-at-index** (Restate RT0016 shape). The await-op names
///   its expected [`JournalCommand`] kind (`ctx.run` → `RunResult`,
///   `ctx.sleep` → `SleepArmed`, `ctx.wait_for_signal` → `SignalAwaited`,
///   `ctx.emit_action` → `ActionEmitted`). An IN-BOUNDS recorded command of
///   any OTHER kind at the cursor is a divergent trajectory.
/// - **Layer 2 — name within `RunResult`**. The variant matches but the
///   recorded `RunResult` name diverges from the replaying body's `ctx.run`
///   name.
///
/// **Post:** on a Layer-1 OR Layer-2 mismatch the cursor returns
/// [`WorkflowCtxError::NonDeterministic`] `{ expected, actual }`, does NOT
/// advance the command-cursor, and does NOT fall through to the live path
/// (no `Ok(None)` / `Ok(false)`). `expected`/`actual` are DETERMINISTIC: a
/// stable variant-kind label (an `as_str()`-style command-kind label, per
/// `.claude/rules/development.md` § "Label enums own their string
/// representation") or the recorded `RunResult` name — NEVER an
/// address-bearing `Debug` of the whole entry, so the trajectory stays
/// byte-identical across seeds (the DST replay-equivalence property).
///
/// **Invariant:** a divergent journal is an ERROR, never a silent
/// re-execution. An in-bounds cursor only ever resolves to a replay HIT
/// (matching kind, and matching name for `RunResult`) or a fail-closed
/// `NonDeterministic`; the live path is reached ONLY when the command-cursor
/// is PAST the loaded command walk. The one non-divergent in-bounds-yet-live
/// shape is `replay_signal`'s crashed-while-blocked case (a `SignalAwaited`
/// command with no matching `SignalSeen` notification), which re-blocks via
/// `Ok(None)` — the variant matched, so it is not a Layer-1 violation.
///
/// **Layer 3 (content/digest comparison) is OUT of scope for this contract**
/// — deferred to <https://github.com/overdrive-sh/overdrive/issues/214>. The
/// `result_digest` / `value_digest` / `action_digest` are recorded (for K4
/// replay-equivalence) but the cursor does NOT diff them at replay; Layers
/// 1 + 2 are the determinism gate.
#[async_trait]
pub trait JournalCursor: Send + Sync {
    /// Check the cursor for a recorded `ctx.run` step at the current
    /// position (POSITIONAL identity — the cursor index, like the sleep
    /// branch).
    ///
    /// # Postconditions
    ///
    /// Returns `Ok(Some(result_bytes))` when replaying (a recorded run
    /// entry exists at the cursor) — the caller MUST NOT poll the step's
    /// future and MUST decode + return the recorded result. Returns
    /// `Ok(None)` when live (cursor at the journal end) — the caller polls
    /// the step's future and then calls [`record_run`](Self::record_run).
    /// Implementations advance the cursor on a replay hit.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::NonDeterministic`] when a recorded run
    /// step exists at the cursor but its recorded `name` does not match
    /// the passed `name` — the workflow body diverged from the recorded
    /// trajectory (fail-closed; replay must be bit-identical).
    async fn replay_run(&self, name: &str) -> Result<Option<Vec<u8>>, WorkflowCtxError>;

    /// Record a freshly-resolved `ctx.run` result durably and advance the
    /// cursor (the live path). `name` is recorded for diagnostics + the
    /// replay-determinism check; `result_bytes` is the CBOR-encoded step
    /// result.
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the result bytes are durably journaled (append + fsync
    /// per ADR-0063 §4) and the cursor has advanced past this step, so a
    /// subsequent resume replays them via [`replay_run`](Self::replay_run)
    /// without re-polling the step's future.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::JournalRecord`] when the durable
    /// append/fsync fails — the engine surfaces this rather than continue
    /// against an unjournaled effect.
    async fn record_run(&self, name: &str, result_bytes: &[u8]) -> Result<(), WorkflowCtxError>;

    /// Check the cursor for a recorded `ctx.sleep` arm at the current
    /// step (the slice-02 await-surface, ADR-0064 §3 sleep branch).
    ///
    /// # Postconditions
    ///
    /// - **Replay (cursor < journal length):** returns
    ///   `Some(recorded_deadline_unix)` — the absolute wall-clock deadline
    ///   (an INPUT) recorded when the sleep was first armed
    ///   (`development.md` § "Persist inputs, not derived state"). The
    ///   caller recomputes the remaining wait as `recorded_deadline −
    ///   clock.unix_now()` and parks only for what remains (returning
    ///   immediately if the deadline has already passed). Implementations
    ///   advance the cursor on a replay hit.
    /// - **Live (cursor at journal end):** returns `None` — the caller
    ///   computes the deadline from `clock.unix_now() + duration`, records
    ///   it via [`record_sleep_armed`](Self::record_sleep_armed), and then
    ///   parks on the Clock deadline.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::NonDeterministic`] when an in-bounds
    /// recorded command at the cursor is NOT a `SleepArmed` — the replaying
    /// `ctx.sleep` await-op landed on a foreign command kind (Layer-1
    /// type-at-index fail-closed gate, D4). The cursor does NOT advance and
    /// does NOT fall through to live.
    async fn replay_sleep(&self) -> Result<Option<Duration>, WorkflowCtxError>;

    /// Record a freshly-armed `ctx.sleep` deadline durably and advance the
    /// cursor (the live path, ADR-0064 §3 sleep branch).
    ///
    /// `deadline_unix` is the ABSOLUTE wall-clock deadline (an input) —
    /// never a "remaining duration" cache. Resume reads it back via
    /// [`replay_sleep`](Self::replay_sleep) and recomputes the remaining
    /// wait against the live clock.
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the `SleepArmed { deadline_unix }` entry is durably
    /// journaled (append + fsync per ADR-0063 §4) and the cursor has
    /// advanced past this step, so a subsequent resume replays it via
    /// [`replay_sleep`](Self::replay_sleep) without re-arming.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::JournalRecord`] when the durable
    /// append/fsync fails — the engine surfaces this rather than continue
    /// against an unjournaled sleep.
    async fn record_sleep_armed(&self, deadline_unix: Duration) -> Result<(), WorkflowCtxError>;

    /// Check the cursor for a recorded `ctx.wait_for_signal` outcome
    /// (the slice-03 signal await-surface, ADR-0064 §4).
    ///
    /// Resolution is by the **notification-lookup contract** (D6 / CA-5):
    /// the `SignalSeen` is found by `signal_key` lookup in the
    /// `BTreeMap<SignalKey, JournalNotification>`, NEVER by position. The
    /// command-cursor advances over the `SignalAwaited` COMMAND only.
    ///
    /// # Postconditions
    ///
    /// - **Replay (command-cursor at a `SignalAwaited` command WITH a
    ///   matching `SignalSeen` notification):** returns
    ///   `Some(recorded_signal_value)` — the [`SignalValue`] recorded in the
    ///   `SignalSeen` notification when the signal was first observed
    ///   satisfied (`development.md` § "Persist inputs, not derived state";
    ///   the value is the input the workflow body received). The caller
    ///   returns it WITHOUT re-reading the signal surface. Implementations
    ///   advance the command-cursor by exactly 1 (past the `SignalAwaited`
    ///   command); the notification does NOT advance it (it is off the
    ///   walk).
    /// - **Crashed-while-blocked (command-cursor at a `SignalAwaited`
    ///   command with NO matching `SignalSeen` notification):** NOT a replay
    ///   hit — returns `None` so the live path re-blocks on the SAME signal
    ///   (the crash-safety contract proven by step 03-02). The command-cursor
    ///   does NOT advance here; `record_signal_awaited` advances past the
    ///   recorded `SignalAwaited`.
    /// - **Live (command-cursor past the loaded commands, or not at a
    ///   `SignalAwaited`):** returns `None` — the caller records
    ///   `SignalAwaited`, reads the typed signal surface, and on a hit
    ///   records `SignalSeen { value }` before returning the value.
    ///
    /// # Invariants
    ///
    /// - The `SignalSeen` is correlated by `signal_key`, never by position —
    ///   the retired `*cursor += 2` positional walk is gone (CA-5).
    /// - A notification never advances the command-cursor and is never
    ///   consumed as a command.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::NonDeterministic`] when an in-bounds
    /// recorded command at the cursor is NOT a `SignalAwaited` — the
    /// replaying `ctx.wait_for_signal` await-op landed on a foreign command
    /// kind (Layer-1 type-at-index fail-closed gate, D4). The cursor does NOT
    /// advance and does NOT fall through to live. The crashed-while-blocked
    /// case (a `SignalAwaited` with no matching `SignalSeen` notification) is
    /// NOT divergence — it stays `Ok(None)` so the live path re-blocks.
    async fn replay_signal(
        &self,
        signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError>;

    /// Durably record the `SignalAwaited` armed entry for `signal_key`
    /// (ADR-0063 §4 fsync-then-suspend) and advance the cursor — the FIRST
    /// half of the live `ctx.wait_for_signal` path, BEFORE the engine
    /// begins blocking.
    ///
    /// # Postconditions
    ///
    /// - **Live (command-cursor past the loaded commands):** appends a
    ///   `SignalAwaited { signal_key }` COMMAND durably and advances the
    ///   command-cursor by exactly 1, then returns `Ok(())`. The ctx then
    ///   enters its Clock-driven block on the signal surface.
    /// - **Crash-while-blocked replay (command-cursor at a `SignalAwaited`
    ///   command with no matching `SignalSeen` notification):** the command
    ///   is ALREADY recorded (the prior run crashed while blocked, having
    ///   recorded the `SignalAwaited` command but never the `SignalSeen`
    ///   notification — `replay_signal` returned `None` because no matching
    ///   notification exists in the lookup map), so this does NOT append a
    ///   duplicate — it advances the command-cursor PAST the recorded
    ///   `SignalAwaited` command and returns `Ok(())`. The ctx re-enters the
    ///   live block on the SAME `signal_key` (read from the recorded command).
    ///   This is the load-bearing crash-safety case (S-WP-03-01).
    ///
    /// # Errors
    ///
    /// - [`WorkflowCtxError::JournalRecord`] — the durable append failed.
    async fn record_signal_awaited(&self, signal_key: &SignalKey) -> Result<(), WorkflowCtxError>;

    /// Poll the engine's signal surface (the `ObservationStore`, in-process
    /// single-node per #207-OUT) for `signal_key` — the engine-internal
    /// block check the ctx loops on. Returns `Ok(Some(value))` when the
    /// signal is PRESENT, `Ok(None)` when it is still ABSENT (the ctx parks
    /// on the injected `Clock` and re-polls). This does NOT journal — it is
    /// engine-internal blocking, not a workflow await-point.
    ///
    /// # Errors
    ///
    /// - [`WorkflowCtxError::Signal`] — the signal surface read failed
    ///   (distinct from "signal absent", which is `Ok(None)`).
    async fn poll_signal(
        &self,
        signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError>;

    /// Durably record the `SignalSeen { value }` NOTIFICATION for
    /// `signal_key` (ADR-0063 §4) — the SECOND half of the live
    /// `ctx.wait_for_signal` path, AFTER the engine observed the signal
    /// present. Records the observed `value` as an input so a resumed run
    /// replays it via [`replay_signal`](Self::replay_signal) without
    /// re-reading the surface.
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the `SignalSeen { signal_key, value }` notification is
    /// durably journaled (appended as a `LoadedEntry::Notification`). Per the
    /// notification-lookup contract (D6) this does NOT advance the
    /// command-cursor — a notification lives off the positional command walk.
    /// The preceding `SignalAwaited` COMMAND (via
    /// [`record_signal_awaited`](Self::record_signal_awaited)) DID advance the
    /// cursor; a crash AFTER `SignalAwaited` advanced but BEFORE this
    /// notification is journaled leaves the `SignalAwaited` command with no
    /// matching `SignalSeen` notification — the re-block-on-resume shape.
    ///
    /// # Errors
    ///
    /// - [`WorkflowCtxError::JournalRecord`] — the durable append failed.
    async fn record_signal_seen(
        &self,
        signal_key: &SignalKey,
        value: &SignalValue,
    ) -> Result<(), WorkflowCtxError>;

    /// Check the cursor for a recorded `ctx.emit_action` at the current
    /// step (the slice-03 emit await-surface, ADR-0064 §4).
    ///
    /// # Postconditions
    ///
    /// - **Replay (an `ActionEmitted` is recorded at the cursor):** returns
    ///   `true` — the caller does NOT re-send the Action on the Action
    ///   channel (exactly-once *on the replay path*: once `ActionEmitted` is
    ///   journaled, resume replays it without re-sending). Implementations
    ///   advance the cursor on a replay hit.
    /// - **Live (cursor at journal end):** returns `false` — the caller
    ///   sends the Action on the engine's Action channel, then records
    ///   `ActionEmitted` durably before returning.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowCtxError::NonDeterministic`] when an in-bounds
    /// recorded command at the cursor is NOT an `ActionEmitted` — the
    /// replaying `ctx.emit_action` await-op landed on a foreign command kind
    /// (Layer-1 type-at-index fail-closed gate, D4). The cursor does NOT
    /// advance and does NOT fall through to live.
    async fn replay_emit(&self) -> Result<bool, WorkflowCtxError>;

    /// Send `action` on the engine's Action channel (→ Raft, the same
    /// channel the reconciler runtime consumes — NEVER a direct
    /// `IntentStore` write, `development.md` Workflow contract rule 6), then
    /// record the `ActionEmitted` entry durably and advance the cursor (the
    /// live path of `ctx.emit_action`). `action_digest` is the content
    /// digest of the emitted Action's inputs.
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the typed Action has been handed to the Action channel
    /// AND the `ActionEmitted` entry is durably journaled (append + fsync
    /// per ADR-0063 §4), so a subsequent resume replays it via
    /// [`replay_emit`](Self::replay_emit) without re-sending the Action.
    ///
    /// **At-least-once on the live path.** The send is BEFORE the durable
    /// record, so a crash (or an `Err(JournalRecord)`-then-crash) AFTER the
    /// send but BEFORE `ActionEmitted` is journaled leaves no `ActionEmitted`
    /// at the cursor — resume re-runs the live path and re-sends. Exactly-once
    /// holds only on the replay path (above). The ordering is deliberate:
    /// record-before-send would instead lose the mutation silently on a crash
    /// between record and send. Safety against the duplicate rests on the
    /// downstream action-shim dispatch being idempotent — the same
    /// at-least-once + downstream-idempotency contract reconciler-emitted
    /// Actions carry.
    ///
    /// # Errors
    ///
    /// - [`WorkflowCtxError::ActionChannel`] — the Action channel send
    ///   failed (channel closed / full).
    /// - [`WorkflowCtxError::JournalRecord`] — the durable record failed.
    async fn emit_action(&self, action: Action) -> Result<(), WorkflowCtxError>;
}

/// A trivial [`JournalCursor`] that never replays and never records — it
/// models a **non-durable, always-live** execution.
///
/// Used by the core author-surface acceptance test (S-WP-01-01) and any
/// caller that drives a [`Workflow`] without a real journal wired: every
/// `ctx.run` polls its future (no replay short-circuit) and nothing is
/// persisted (no-op record). The durable handle — which DOES replay and
/// record — lives in `overdrive-control-plane::workflow_runtime`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysLiveCursor;

#[async_trait]
impl JournalCursor for AlwaysLiveCursor {
    async fn replay_run(&self, _name: &str) -> Result<Option<Vec<u8>>, WorkflowCtxError> {
        Ok(None)
    }

    async fn record_run(&self, _name: &str, _result_bytes: &[u8]) -> Result<(), WorkflowCtxError> {
        Ok(())
    }

    async fn replay_sleep(&self) -> Result<Option<Duration>, WorkflowCtxError> {
        // Always-live: no recorded command at the cursor, so the Layer-1
        // gate never fires — the live path arms a fresh sleep.
        Ok(None)
    }

    async fn record_sleep_armed(&self, _deadline_unix: Duration) -> Result<(), WorkflowCtxError> {
        Ok(())
    }

    async fn replay_signal(
        &self,
        _signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError> {
        // Always-live: no recorded command at the cursor; the live path arms
        // a fresh wait.
        Ok(None)
    }

    async fn record_signal_awaited(&self, _signal_key: &SignalKey) -> Result<(), WorkflowCtxError> {
        // Non-durable always-live handle: nothing is journaled.
        Ok(())
    }

    async fn poll_signal(
        &self,
        _signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError> {
        // No signal surface is wired in the always-live handle; resolve to
        // the empty value immediately (present, no payload) so a
        // signalless execution does not block forever. The durable engine
        // handle (control-plane) polls the real signal row.
        Ok(Some(SignalValue::empty()))
    }

    async fn record_signal_seen(
        &self,
        _signal_key: &SignalKey,
        _value: &SignalValue,
    ) -> Result<(), WorkflowCtxError> {
        // Non-durable always-live handle: nothing is journaled.
        Ok(())
    }

    async fn replay_emit(&self) -> Result<bool, WorkflowCtxError> {
        // Always-live: no recorded command at the cursor; the live path
        // sends + records.
        Ok(false)
    }

    async fn emit_action(&self, _action: Action) -> Result<(), WorkflowCtxError> {
        // No Action channel is wired in the always-live handle; the emit is
        // dropped. The durable engine handle (control-plane) sends on the
        // real Action channel → Raft.
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
    /// The TRANSIENT-step carrier (ADR-0065 §4). A
    /// [`Self::run`] step whose closure resolved to an `Err` of
    /// [`StepError::Retryable`] records the resulting
    /// [`WorkflowCtxError::TransientStep`] here; the
    /// [`ErasedWorkflowAdapter`] reads it back via
    /// [`Self::take_transient_step`] AFTER the body returns and surfaces it
    /// as [`WorkflowDriveError::Transient`] for the engine to re-drive.
    ///
    /// Interior-mutable (`Mutex`) because every `ctx` op takes `&self` — the
    /// same shape `JournalCursorHandle`'s cursor uses. This is the channel
    /// that keeps the transient OFF the body's `Result<Output, TerminalError>`
    /// return type: the body cannot author a retry, only a `ctx.run`
    /// step can signal one, and the signal lives here, not in the body's
    /// return value (ADR-0065 §2).
    ///
    /// At most one transient per drive is retained (the FIRST `ctx.run`
    /// step that fails — the body short-circuits there in idiomatic use). A
    /// fresh ctx is built per drive by the engine, so this never carries a
    /// stale transient across re-drives.
    transient_step: std::sync::Mutex<Option<WorkflowCtxError>>,
}

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
        Self { clock, transport, entropy, journal, transient_step: std::sync::Mutex::new(None) }
    }

    /// Take (and clear) the transient-step signal recorded by a
    /// [`Self::run`] step this drive, if any (ADR-0065 §4).
    ///
    /// The [`ErasedWorkflowAdapter`] calls this AFTER the body returns: a
    /// `Some(WorkflowCtxError::TransientStep)` means a `ctx.run` step
    /// failed transiently and the engine must re-drive (it is surfaced as
    /// [`WorkflowDriveError::Transient`]); `None` means no transient fired
    /// and the body's typed `Result<Output, TerminalError>` is the outcome.
    /// This is the read side of the transient channel that keeps a retry OFF
    /// the body's return type (ADR-0065 §2).
    #[must_use]
    pub fn take_transient_step(&self) -> Option<WorkflowCtxError> {
        self.transient_step.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    /// Project an engine-internal ctx infra failure to a terminal (ADR-0065 §4
    /// "Model Z" amendment). A `WorkflowCtxError` from the journal cursor
    /// (non-deterministic replay, journal-record failure, signal-surface read,
    /// action-channel send) is a PERMANENT failure — re-driving against the same
    /// journal will not fix it — so it ends the workflow as `TerminalError::
    /// explicit`, the same observable terminal the body's prior hand-folding
    /// produced and consistent with the panic → `Failed { Explicit }` path.
    /// `WorkflowCtxError::TransientStep` is routed via the transient slot + body
    /// park, never through this projection.
    ///
    /// Takes `err` by value so it is usable point-free as
    /// `.map_err(Self::infra_terminal)` at every ctx-op boundary (the closure
    /// `map_err` expects receives `E` by value); the body only needs its
    /// `Display`, hence the `#[expect]`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "by-value signature is required for point-free `.map_err(Self::infra_terminal)`"
    )]
    fn infra_terminal(err: WorkflowCtxError) -> TerminalError {
        TerminalError::explicit(&err.to_string())
    }

    /// True if a transient step was recorded this drive (peek without clearing).
    /// `ErasedWorkflowAdapter::run_erased` consults this after each body poll to
    /// detect a parked transient and cancel the body (ADR-0065 §4 Model Z).
    fn has_transient_step(&self) -> bool {
        self.transient_step.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_some()
    }

    /// Begin one durable step `f`, named `name` — the general durable-step
    /// await-surface (the Restate `ctx.run` model). Returns a [`RunStep`]
    /// BUILDER (ADR-0065 Gap 2, the analogue of Restate's `RunFuture`) that
    /// implements [`IntoFuture`], so `ctx.run(name, fut).await?` works
    /// unchanged AND `ctx.run(name, fut).retry_policy(p).await?` sets a
    /// per-step [`RunRetryPolicy`].
    ///
    /// This is the ONE journaled-step primitive; a workflow body performs every
    /// effect (transport sends, future external calls) INSIDE a `ctx.run`
    /// closure so the result is journaled and replayed. The builder defaults to
    /// [`RunRetryPolicy::default`] (today's `WORKFLOW_RETRY_BUDGET` + backoff
    /// schedule) when [`RunStep::retry_policy`] is not called, so existing call
    /// sites behave identically.
    ///
    /// ```ignore
    /// // default policy (today's behaviour) — unchanged call site:
    /// let id = ctx.run("charge", fut).await?;
    /// // explicit per-step policy:
    /// let id = ctx.run("charge", fut)
    ///     .retry_policy(RunRetryPolicy { max_attempts: 10, ..Default::default() })
    ///     .await?;
    /// ```
    ///
    /// `name` stays POSITIONAL — the cosmetic `.name()` builder is out of scope
    /// (ADR-0065 Gap 2). The full step semantics (replay / journal-after-effect
    /// / the `retryable | terminal` union's two arms) are documented on the
    /// private [`Self::run_step`] the builder drives.
    pub fn run<'a, T, F>(&'a self, name: &str, f: F) -> RunStep<'a, T, F>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
        F: Future<Output = Result<T, StepError>> + Send,
    {
        RunStep {
            ctx: self,
            name: name.to_string(),
            f,
            policy: RunRetryPolicy::default(),
            _marker: PhantomData,
        }
    }

    /// Run one durable step `f`, named `name`, under `policy`, and return its
    /// result — the body the [`RunStep`] builder drives via [`IntoFuture`]
    /// (ADR-0065 Gap 1 + Gap 2). This is the ONE journaled-step primitive; a
    /// workflow body performs every effect (transport sends, future external
    /// calls) INSIDE a `ctx.run` closure so the result is journaled and
    /// replayed.
    ///
    /// The closure resolves to <code>Result&lt;T, [StepError]&gt;</code> — a
    /// `retryable | terminal` union (ADR-0065 Gap 1); the journaled step IS the
    /// retry unit (the Restate/Temporal shape). The author rarely names
    /// [`StepError`]: the natural shape is the anyhow `?` idiom — wrap the
    /// effect, propagate any inner error with `?` (folds to
    /// [`StepError::Retryable`]), return the success value; an explicit
    /// permanent failure is `Err(TerminalError::explicit(..).into())` (folds to
    /// [`StepError::Terminal`]):
    ///
    /// ```ignore
    /// let bytes = ctx.run("provision-write", async move {
    ///     Ok(transport.send_datagram(target, payload).await?)
    /// }).await?;
    /// ```
    ///
    /// The inner `?` works because [`StepError`] has a blanket
    /// `From<E: std::error::Error>`, so ANY [`std::error::Error`] is absorbed
    /// into a [`StepError::Retryable`] transient. The closure outcomes:
    /// - **`Ok(value)`** — the step succeeded. Its `value` is CBOR-journaled
    ///   (append + fsync per ADR-0063 §4) and returned; a resumed run replays
    ///   it WITHOUT re-polling `f` (exactly-once on the replay path).
    /// - **`Err(StepError::Retryable { .. })`** — the step failed TRANSIENTLY
    ///   (any [`std::error::Error`] propagated with `?` auto-folds here), the
    ///   ADR-0065 §4 retry channel. This is the body's ONLY way to request a
    ///   re-drive; a transient NEVER reaches the body's `Result<Output,
    ///   TerminalError>` return type (ADR-0065 §2). The result is NOT
    ///   journaled (the step did not durably complete, so on re-drive it
    ///   re-fires), the ctx RECORDS a [`WorkflowCtxError::TransientStep`]
    ///   (read back by the [`ErasedWorkflowAdapter`] and surfaced to the
    ///   engine as [`WorkflowDriveError::Transient`]), and the body PARKS. The
    ///   engine re-drives the body from the journal (completed steps replay
    ///   byte-equal; this step re-fires) up to [`WORKFLOW_RETRY_BUDGET`] —
    ///   then mints `BudgetExhausted`.
    /// - **`Err(StepError::Terminal(t))`** — the step failed PERMANENTLY
    ///   (ADR-0065 Gap 1; a `?`'d [`TerminalError`] folds here). `ctx.run(..).await`
    ///   returns `Err(t)` directly — NO transient-slot record, NO body-park, NO
    ///   re-drive — so the body's `?` observes the terminal and returns
    ///   `Err(TerminalError)`, projected to `WorkflowStatus::Failed`.
    ///
    /// [`WORKFLOW_RETRY_BUDGET`]: <https://docs.rs/overdrive-control-plane>
    ///
    /// **Check-then-record (ADR-0064 §3), POSITIONAL identity.** The op
    /// consults the engine's journal cursor at the current position:
    /// - **Replay:** if the cursor has a recorded result at this step, the
    ///   recorded CBOR bytes are decoded into `T` and returned WITHOUT
    ///   polling `f` — `f` is dropped unpolled, so the effect never
    ///   re-fires. This is the exactly-once guarantee on the replay path.
    ///   Only an `Ok` value is ever journaled (a transient is not recorded),
    ///   so a replay hit always decodes the cleared-step success: a transient
    ///   step that SUCCEEDED on a prior re-drive replays bit-identically.
    /// - **Live:** otherwise `f` is awaited; an `Ok` result is CBOR-encoded
    ///   and durably recorded (append + fsync per ADR-0063 §4) via the
    ///   cursor BEFORE returning, and the cursor advances. An
    ///   `Err(StepError::Retryable { .. })` parks BEFORE any journaling; an
    ///   `Err(StepError::Terminal(t))` returns `Err(t)` BEFORE any journaling.
    ///
    /// **Honest semantics:** the effect inside `f` is *at-least-once* (a
    /// crash after `f.await` but before the record is durable re-fires the
    /// effect on resume); the run await-point is *exactly-once on the
    /// replay path* (once the result is journaled, resume replays it
    /// without re-polling `f`). The journal-after-effect ordering is what
    /// makes the replay path exactly-once — it is NOT an unconditional
    /// exactly-once guarantee for the effect itself.
    ///
    /// # Why the closure error is the `StepError` union
    ///
    /// [`StepError`] is a `retryable | terminal` union (ADR-0065 Gap 1; the
    /// analogue of Restate's `HandlerError`) so a step's outcome is
    /// unambiguous: a transient ([`StepError::Retryable`], engine-absorbed +
    /// re-driven) and a permanent failure ([`StepError::Terminal`], surfaced as
    /// `Err(TerminalError)` from `ctx.run`, never re-driven) are disjoint arms,
    /// not conflated with a domain `Result` the body wants to inspect. A body
    /// that wants the step's success value AND to inspect a domain error itself
    /// returns a `Result<T, DomainErr>` as the step's own `T` (wrapped
    /// `Ok(Ok(..))` / `Ok(Err(..))`); a body that wants the engine to re-drive
    /// resolves to `StepError::Retryable`; a body that wants to fail the whole
    /// workflow resolves to `StepError::Terminal`.
    ///
    /// `name` is recorded for diagnostics and a replay-determinism check
    /// (a recorded step whose name diverges from the replaying body's
    /// `name` fails closed with [`WorkflowCtxError::NonDeterministic`]).
    /// Identity is the cursor position, not `name`.
    ///
    /// # Errors
    ///
    /// Returns a [`TerminalError`] when (a) the step's closure resolves to
    /// `Err(StepError::Terminal(t))` — the step's own permanent failure,
    /// returned verbatim (ADR-0065 Gap 1); or (b) an engine-internal infra
    /// failure occurs inside the journal cursor (non-deterministic replay,
    /// journal-record failure, deserialise failure) — projected to kind
    /// [`Explicit`](TerminalErrorKind::Explicit) at the ctx-op boundary via
    /// `infra_terminal` (ADR-0065 §4 Model Z); a CBOR
    /// serialise/deserialise failure on the step's own value is likewise
    /// projected. A `StepError::Retryable` TRANSIENT step is NOT an error the
    /// body observes — it PARKS the body and is surfaced by the engine as
    /// [`WorkflowDriveError::Transient`] (structurally invisible to the `?`).
    ///
    /// `policy` is the failing step's [`RunRetryPolicy`] (ADR-0065 Gap 2). It is
    /// NOT consulted on the success / replay / terminal paths; on the
    /// `Retryable` arm it RIDES the recorded `WorkflowCtxError::TransientStep`
    /// signal so the engine's re-drive decision consults THIS step's policy. It
    /// is not journaled (re-derived from the body each drive; Gap 2
    /// sub-decision 3).
    async fn run_step<T, F>(
        &self,
        name: &str,
        f: F,
        policy: RunRetryPolicy,
    ) -> Result<T, TerminalError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
        F: Future<Output = Result<T, StepError>> + Send,
    {
        // Replay path — decode the recorded SUCCESS, never poll `f`. A foreign
        // command kind at the cursor is a non-deterministic-replay INFRA failure
        // projected to a terminal (ADR-0065 §4 Model Z).
        if let Some(recorded_bytes) =
            self.journal.replay_run(name).await.map_err(Self::infra_terminal)?
        {
            let value: T = ciborium::from_reader(recorded_bytes.as_slice()).map_err(|e| {
                TerminalError::explicit(&format!("workflow ctx.run deserialize failed: {e}"))
            })?;
            return Ok(value);
        }

        // Live path — poll `f`.
        match f.await {
            Ok(value) => {
                // Success — durably record before returning (journal-after-effect).
                let mut bytes: Vec<u8> = Vec::new();
                ciborium::into_writer(&value, &mut bytes).map_err(|e| {
                    TerminalError::explicit(&format!("workflow ctx.run serialize failed: {e}"))
                })?;
                self.journal.record_run(name, &bytes).await.map_err(Self::infra_terminal)?;
                Ok(value)
            }
            Err(StepError::Terminal(terminal)) => {
                // Permanent step failure (ADR-0065 Gap 1) — propagate the terminal
                // directly. No transient-slot record, no body-park, no re-drive:
                // the body's `?` observes it and the body returns
                // `Err(TerminalError)`, projected to `WorkflowStatus::Failed`.
                Err(terminal)
            }
            Err(StepError::Retryable { detail }) => {
                // Transient — record the signal in the ctx and PARK the body (the
                // Model Z mechanism, unchanged). The engine (via
                // `ErasedWorkflowAdapter::run_erased`) observes the recorded
                // transient, cancels this parked body by dropping it, and
                // re-drives from the journal (ADR-0065 §4). The body's `?` NEVER
                // observes the transient — it is structurally invisible.
                self.record_transient_step(WorkflowCtxError::TransientStep {
                    name: name.to_string(),
                    detail,
                    // The failing step's policy rides the transient signal so
                    // the engine's re-drive decision consults THIS step's
                    // `max_attempts` / `max_duration` / backoff (ADR-0065 Gap 2),
                    // not the global `WORKFLOW_RETRY_BUDGET` constant.
                    policy,
                });
                // Park forever: never resolves, registers no waker. The engine
                // drops this future once the transient slot is observed.
                std::future::pending::<()>().await;
                unreachable!(
                    "ctx.run parked on a retryable step; the engine cancels the parked body and re-drives"
                )
            }
        }
    }

    /// Record the FIRST transient-step signal of this drive (ADR-0065 §4).
    /// Idempotent on a second call within the same drive — the first
    /// transient is the one the body short-circuits on, and the engine
    /// re-drives the whole body anyway, so a later transient (if the body
    /// kept going) is irrelevant. Read back by [`Self::take_transient_step`].
    fn record_transient_step(&self, transient: WorkflowCtxError) {
        let mut slot =
            self.transient_step.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(transient);
        }
    }

    /// Suspend the workflow for `duration` through the injected
    /// [`Clock`] — the slice-02 await-surface (ADR-0064 §3 sleep branch).
    ///
    /// **Check-then-record, deadline-as-input (ADR-0063 §2,
    /// `development.md` § "Persist inputs, not derived state").**
    ///
    /// - **Live:** the absolute deadline is computed as
    ///   `clock.unix_now() + duration`, durably recorded as a `SleepArmed
    ///   { deadline_unix }` entry (append + fsync per ADR-0063 §4) BEFORE
    ///   parking, and the ctx then parks on the Clock deadline. The
    ///   journal records the DEADLINE (an input), never a "remaining"
    ///   cache.
    /// - **Replay:** the cursor returns the recorded deadline; the ctx
    ///   recomputes the remaining wait as `recorded_deadline −
    ///   clock.unix_now()` and parks only for what remains — returning
    ///   immediately if the deadline has already passed.
    ///
    /// The same code path runs under `SimClock` (parks until the harness
    /// advances logical time) and `SystemClock` (parks on the Tokio
    /// timer) — no DST-only branch (`development.md` § "Production code is
    /// not shaped by simulation").
    ///
    /// # Errors
    ///
    /// Returns a [`TerminalError`] (kind [`Explicit`](TerminalErrorKind::Explicit))
    /// when an engine-internal infra failure occurs inside the journal cursor
    /// (a non-deterministic replay at this cursor, or the live-path durable
    /// record of the armed deadline failing) — projected at the ctx-op boundary
    /// via `infra_terminal` (ADR-0065 §4 Model Z). It no longer returns
    /// a `WorkflowCtxError`.
    pub async fn sleep(&self, duration: Duration) -> Result<(), TerminalError> {
        if let Some(deadline_unix) =
            self.journal.replay_sleep().await.map_err(Self::infra_terminal)?
        {
            let now = self.clock.unix_now();
            if let Some(remaining) = deadline_unix.checked_sub(now) {
                self.clock.sleep(remaining).await;
            }
            return Ok(());
        }
        let deadline_unix = self.clock.unix_now() + duration;
        self.journal.record_sleep_armed(deadline_unix).await.map_err(Self::infra_terminal)?;
        self.clock.sleep(duration).await;
        Ok(())
    }

    /// Wait for the typed signal `signal_key` to be present, returning its
    /// [`SignalValue`] — the slice-03 signal await-surface (ADR-0064 §4).
    /// The engine reads typed signal rows from the `ObservationStore`
    /// (in-process single-node delivery; cross-node-under-partition is
    /// #207-OUT). Cross-workflow coordination uses these typed signals,
    /// never an ad-hoc `IntentStore` write (whitepaper §18).
    ///
    /// **Check-then-record (ADR-0064 §4), POSITIONAL identity.**
    ///
    /// - **Replay:** if a `SignalSeen { value }` was recorded at this
    ///   cursor, the recorded value is returned WITHOUT re-reading the
    ///   signal surface (the workflow body received this exact value on the
    ///   live run). A `SignalAwaited` with no matching `SignalSeen` (crashed
    ///   while still blocked) re-blocks on the SAME signal on the live path.
    /// - **Live:** the ctx records `SignalAwaited`, then BLOCKS — it polls
    ///   the signal surface and, while the signal is ABSENT, parks on the
    ///   injected [`Clock`] (a deadline-park, NOT a busy-spin: under
    ///   `SimClock` the harness advances logical time and writes the signal
    ///   row, waking the park; under `SystemClock` the park is a Tokio
    ///   timer). When the signal is PRESENT it records `SignalSeen { value }`
    ///   durably (fsync per ADR-0063 §4) and returns the value. The
    ///   `SignalAwaited` and `SignalSeen` records are at DISTINCT cursor
    ///   positions, so a crash WHILE blocked leaves `SignalAwaited` with no
    ///   following `SignalSeen` — the re-block-on-resume shape
    ///   (S-WP-03-01).
    ///
    /// The block uses the injected `Clock` + `ObservationStore` ports only
    /// (`development.md` § "Production code is not shaped by simulation" —
    /// the same Clock-driven poll is the genuine in-process single-node
    /// production mechanism; there is no DST-only branch).
    ///
    /// # Errors
    ///
    /// Returns a [`TerminalError`] (kind [`Explicit`](TerminalErrorKind::Explicit))
    /// when an engine-internal infra failure occurs inside the journal cursor
    /// (the signal-surface read failing, a non-deterministic replay, or a
    /// durable record failing) — projected at the ctx-op boundary via
    /// `infra_terminal` (ADR-0065 §4 Model Z). It no longer returns a
    /// `WorkflowCtxError`.
    pub async fn wait_for_signal(
        &self,
        signal_key: SignalKey,
    ) -> Result<SignalValue, TerminalError> {
        if let Some(value) =
            self.journal.replay_signal(&signal_key).await.map_err(Self::infra_terminal)?
        {
            return Ok(value);
        }
        self.journal.record_signal_awaited(&signal_key).await.map_err(Self::infra_terminal)?;
        let value = loop {
            if let Some(value) =
                self.journal.poll_signal(&signal_key).await.map_err(Self::infra_terminal)?
            {
                break value;
            }
            self.clock.sleep(SIGNAL_POLL).await;
        };
        self.journal.record_signal_seen(&signal_key, &value).await.map_err(Self::infra_terminal)?;
        Ok(value)
    }

    /// Emit a typed cluster-mutation [`Action`] onto the SAME Action channel
    /// the reconciler runtime consumes (→ Raft) — the slice-03 emit
    /// await-surface (ADR-0064 §4; whitepaper §18 *Primitive Composition*).
    /// The workflow NEVER writes the `IntentStore` directly and `ctx`
    /// deliberately exposes no `.put()` surface (`development.md` Workflow
    /// contract rule 6 — no Raft bypass).
    ///
    /// **Check-then-record (ADR-0064 §4), POSITIONAL identity.**
    ///
    /// - **Replay:** if an `ActionEmitted` was recorded at this cursor, the
    ///   Action is NOT re-sent — exactly-once *on the replay path* (once
    ///   `ActionEmitted` is journaled, resume replays it without re-sending).
    /// - **Live:** the engine sends the Action on the Action channel, then
    ///   records `ActionEmitted` durably (fsync per ADR-0063 §4) before
    ///   returning, and advances the cursor.
    ///
    /// **Honest semantics:** the emit is *at-least-once* — a crash after the
    /// channel send but before `ActionEmitted` is durably recorded re-fires
    /// the emit on resume (no `ActionEmitted` is journaled at the cursor, so
    /// the live path runs again and re-sends). This is the SAME shape
    /// [`run`](Self::run) documents; the send-before-record ordering is
    /// deliberate. Safety against the duplicate rests on the downstream
    /// action-shim dispatch being idempotent (the at-least-once +
    /// downstream-idempotency contract reconciler-emitted Actions also carry).
    /// Reversing to record-before-send would trade the dedup-able duplicate
    /// for a SILENT lost mutation on a crash between record and send —
    /// strictly worse for a cluster mutation.
    ///
    /// # Errors
    ///
    /// Returns a [`TerminalError`] (kind [`Explicit`](TerminalErrorKind::Explicit))
    /// when an engine-internal infra failure occurs inside the journal cursor
    /// (the Action channel send failing — channel closed / full — a
    /// non-deterministic replay, or the durable record failing) — projected at
    /// the ctx-op boundary via `infra_terminal` (ADR-0065 §4 Model Z).
    /// It no longer returns a `WorkflowCtxError`.
    pub async fn emit_action(&self, action: Action) -> Result<(), TerminalError> {
        if self.journal.replay_emit().await.map_err(Self::infra_terminal)? {
            return Ok(());
        }
        self.journal.emit_action(action).await.map_err(Self::infra_terminal)
    }

    /// The injected clock. Workflow bodies read time only through this
    /// port (the `ctx.sleep` await-surface is [`Self::sleep`]).
    #[must_use]
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// The injected transport. Workflow bodies perform datagram / network
    /// effects only through this port — and only INSIDE a [`Self::run`]
    /// closure, so the effect's result is journaled and replayed
    /// (exactly-once on the replay path). Cloning the `Arc` into the
    /// closure is the idiomatic shape (the closure is `'static + Send`).
    #[must_use]
    pub fn transport(&self) -> &Arc<dyn Transport> {
        &self.transport
    }

    /// The injected entropy source. Workflow bodies read randomness only
    /// through this port.
    #[must_use]
    pub fn entropy(&self) -> &Arc<dyn Entropy> {
        &self.entropy
    }
}

/// The builder [`WorkflowCtx::run`] returns — the analogue of Restate's
/// `RunFuture` (ADR-0065 Amendment 2026-06-07, Gap 2).
///
/// Holds the durable step (`name` + closure `f`) plus its [`RunRetryPolicy`].
/// It implements [`IntoFuture`] resolving to `Result<T, TerminalError>`, so the
/// default-policy call site `ctx.run(name, fut).await?` is unchanged, while
/// `ctx.run(name, fut).retry_policy(p).await?` overrides the per-step policy
/// before awaiting. `name` is positional (the cosmetic `.name()` builder is out
/// of scope, Gap 2).
///
/// The builder is the ONLY public surface that constructs a step; the actual
/// step body lives in the private [`WorkflowCtx::run_step`] the [`IntoFuture`]
/// impl drives. The lifetime `'a` borrows the [`WorkflowCtx`] for the duration
/// of the step future (the body always awaits the step inline, so the borrow
/// never outlives the body).
pub struct RunStep<'a, T, F> {
    /// The ctx the step runs against (its journal cursor + injected ports).
    ctx: &'a WorkflowCtx,
    /// The `ctx.run` step name — diagnostics + the replay-determinism check.
    name: String,
    /// The step's closure future, resolving to `Result<T, StepError>`.
    f: F,
    /// The per-step retry policy (defaults to [`RunRetryPolicy::default`] —
    /// today's behaviour — unless [`Self::retry_policy`] overrides it).
    policy: RunRetryPolicy,
    /// `T` appears only in `F`'s `Output`; bind it so the struct is generic
    /// over the step's result type without an unused-parameter error.
    _marker: PhantomData<T>,
}

impl<T, F> RunStep<'_, T, F> {
    /// Override the per-step [`RunRetryPolicy`] (ADR-0065 Gap 2). Without this
    /// call the step uses [`RunRetryPolicy::default`] (today's
    /// `WORKFLOW_RETRY_BUDGET` + backoff schedule). Consumes and returns `self`
    /// so it chains before `.await`:
    /// `ctx.run(name, fut).retry_policy(p).await?`.
    #[must_use]
    pub const fn retry_policy(mut self, policy: RunRetryPolicy) -> Self {
        self.policy = policy;
        self
    }
}

impl<'a, T, F> IntoFuture for RunStep<'a, T, F>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + 'a,
    F: Future<Output = Result<T, StepError>> + Send + 'a,
{
    type Output = Result<T, TerminalError>;
    // TAIT (`type_alias_impl_trait`) is unstable on stable Rust, so the
    // associated future is a boxed trait object rather than a named opaque
    // type. The `'a` bound keeps the borrowed `ctx` alive for the future's
    // lifetime.
    type IntoFuture = Pin<Box<dyn Future<Output = Result<T, TerminalError>> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.ctx.run_step(&self.name, self.f, self.policy).await })
    }
}

/// The identity of a typed cross-workflow signal (slice 03, ADR-0064 §4).
///
/// A `ctx.wait_for_signal(key)` blocks on the typed signal named by this
/// key in the `ObservationStore`; a producer satisfies it by writing the
/// signal row keyed by the same `SignalKey`. Kebab-case,
/// `^[a-z][a-z0-9-]{0,126}$` — wider interior than `WorkflowName` since
/// signal keys may embed correlation suffixes.
///
/// STRICT newtype per `development.md` § "Newtypes": validating
/// constructor, `FromStr` / `Display` / `Serialize` / `Deserialize`
/// matching exactly. Serde is derived through the `String` form so the
/// wire shape equals `Display` / `FromStr`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalKey(String);

/// Maximum length for a signal key (1 lead + up to 126 interior).
const SIGNAL_KEY_MAX: usize = 127;

impl SignalKey {
    /// Validating constructor. Rejects empty, over-long, and
    /// non-`^[a-z][a-z0-9-]{0,126}$` inputs.
    ///
    /// # Errors
    ///
    /// Returns [`SignalKeyError`] naming the first validation failure.
    pub fn new(raw: &str) -> Result<Self, SignalKeyError> {
        if raw.is_empty() {
            return Err(SignalKeyError::Empty);
        }
        if raw.len() > SIGNAL_KEY_MAX {
            return Err(SignalKeyError::TooLong { max: SIGNAL_KEY_MAX });
        }
        let mut chars = raw.chars();
        let lead = chars.next().unwrap_or_else(|| {
            unreachable!("non-empty checked above guarantees at least one char")
        });
        if !lead.is_ascii_lowercase() {
            return Err(SignalKeyError::BadShape);
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(SignalKeyError::BadShape);
        }
        Ok(Self(raw.to_string()))
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SignalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SignalKey {
    type Err = SignalKeyError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl serde::Serialize for SignalKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SignalKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(serde::de::Error::custom)
    }
}

/// Validation failures for [`SignalKey::new`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignalKeyError {
    /// The key was empty.
    #[error("signal key must not be empty")]
    Empty,
    /// The key exceeded the length ceiling.
    #[error("signal key too long (max {max})")]
    TooLong {
        /// The maximum permitted length.
        max: usize,
    },
    /// The key did not match `^[a-z][a-z0-9-]{0,126}$`.
    #[error("signal key must match ^[a-z][a-z0-9-]{{0,126}}$")]
    BadShape,
}

/// The opaque payload a typed signal carries (slice 03, ADR-0064 §4).
///
/// A signal producer writes arbitrary bytes; a `ctx.wait_for_signal`
/// consumer receives them verbatim. The `value_digest` recorded in the
/// `SignalSeen` journal entry is the content digest of these bytes (an
/// input, per `development.md` § "Persist inputs, not derived state").
/// Any UTF-8 payload is valid — the value is opaque to the primitive — so
/// there is no rejecting constructor; `new` is infallible.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalValue(String);

impl SignalValue {
    /// Construct a signal value from its opaque payload. Infallible — the
    /// value is opaque to the primitive.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The empty signal value — the "present, no payload" sentinel a
    /// signalless live read resolves to.
    #[must_use]
    pub const fn empty() -> Self {
        Self(String::new())
    }

    /// The opaque payload bytes.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SignalValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SignalValue {
    type Err = std::convert::Infallible;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(raw))
    }
}

impl serde::Serialize for SignalValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SignalValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

/// Identity of a workflow to start. Kebab-case, `^[a-z][a-z0-9-]{0,62}$`
/// — the same shape as `ReconcilerName`, the peer primitive's identity.
///
/// Derives `rkyv::{Archive, Serialize, Deserialize}` because it is embedded
/// inline in the rkyv-archived [`WorkflowStartV1`] payload (the durable
/// `Action::StartWorkflow` intent crosses the rkyv `IntentStore` boundary
/// via [`WorkflowStart::archive_for_store`]). The inner `String` is a private
/// field; rkyv archives the validated canonical form and re-materialises it
/// without re-validating (the bytes were produced by `new()` at write time).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
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
/// (ADR-0064 §1, ADR-0065 §5) — the durable START INTENT for a workflow
/// instance, read back on every restart through the rkyv `IntentStore`
/// boundary.
///
/// # Why this is an rkyv versioned-envelope payload (ADR-0048 §4b)
///
/// `WorkflowStart` grew from identity-only (`{ name }`) to an input-bearing
/// durable aggregate (`{ name, input }`). It now crosses a durable-storage
/// boundary as the persisted `Action::StartWorkflow` intent, so per
/// `.claude/rules/development.md` § "rkyv schema evolution" it MUST be
/// wrapped in a per-type rkyv versioned envelope ([`WorkflowStartEnvelope`])
/// with a co-located typed codec ([`Self::archive_for_store`] /
/// [`Self::from_store_bytes`]) — the same shape the `Job` aggregate uses.
///
/// # `input` is an INPUT, opaque to the engine
///
/// `input` is the CBOR-encoded `W::Input` the workflow body receives —
/// persisted as the INPUT it is (`.claude/rules/development.md` § "Persist
/// inputs, not derived state"), NOT a derived value. The engine never
/// decodes it: the `ErasedWorkflowAdapter` (later slice) is the sole site
/// that CBOR-decodes it into the workflow's typed `Input`. The rkyv envelope
/// wraps the OUTER `WorkflowStart` only; the inner `input: Vec<u8>` stays
/// opaque CBOR — the rkyv-outer / CBOR-inner separation ("aggregate
/// envelopes wrap the outer type only").
///
/// # Alias-to-payload (UI-02)
///
/// `WorkflowStart` is a `pub type` alias to the V1 payload
/// ([`WorkflowStartV1`]) so call sites construct it with struct-literal
/// syntax (`WorkflowStart { name, input }`). The envelope enum is
/// codec-internal and NOT re-exported from `overdrive_core::lib.rs`
/// (ADR-0048 §2 Layer 1 — cross-crate writers reach it only via the verbose
/// `overdrive_core::workflow::WorkflowStartEnvelope` path, discouraged at
/// review; the load-bearing structural defense is the Layer 2 dst-lint
/// variant-construction scanner).
///
/// Slice 01 carries identity + input; later slices grow the spec additively
/// (version, scheduling) — an additive field appends to `WorkflowStartV1`
/// only until it breaks the archived layout, at which point a `V2` envelope
/// variant is minted per the version-bump procedure.
pub type WorkflowStart = WorkflowStartV1;

/// Documentation alias for "the latest [`WorkflowStart`] payload" — the shape
/// [`WorkflowStartEnvelope::into_latest`] projects to. Identical to
/// [`WorkflowStart`] today (both point at [`WorkflowStartV1`]); on a `V2` bump
/// both re-alias to the new payload in the same commit.
pub type WorkflowStartLatest = WorkflowStartV1;

/// The V1 payload of the [`WorkflowStart`] versioned envelope — the durable
/// workflow START intent (ADR-0048 §4b, ADR-0065 §5).
///
/// `pub` (not `pub(crate)`) because rustc E0446 rejects a `pub(crate)` type
/// referenced from the `pub` [`VersionedEnvelope`] trait's `type Latest`
/// assignment; ADR-0048 §2 Layer 1 is enforced instead by NON-RE-EXPORT from
/// `overdrive_core::lib.rs` plus the Layer 2 dst-lint scanner. The fields are
/// `pub` so callers use struct-literal `WorkflowStart { name, input }` via the
/// [`WorkflowStart`] alias.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct WorkflowStartV1 {
    /// Identity of the workflow to start.
    pub name: WorkflowName,
    /// The opaque CBOR-encoded `W::Input` (an INPUT — `.claude/rules/
    /// development.md` § "Persist inputs, not derived state"). The engine
    /// treats this as opaque bytes; only the `ErasedWorkflowAdapter` decodes
    /// it into the workflow's typed `Input`. May be empty (a workflow whose
    /// `Input` is `()` encodes to a 1-byte CBOR `null`, or to empty depending
    /// on the author's adapter — the codec does not interpret it).
    pub input: Vec<u8>,
}

/// The per-type rkyv versioned envelope for [`WorkflowStart`] (ADR-0048 §1).
///
/// Exactly one variant today (`V1`). Codec-internal: writers go through
/// [`VersionedEnvelope::latest`] (`WorkflowStartEnvelope::latest`), readers
/// through [`decode_envelope_bytes`] /
/// [`WorkflowStart::from_store_bytes`]. NOT re-exported from
/// `overdrive_core::lib.rs` (ADR-0048 §2 Layer 1).
///
/// On a `V2` bump: append `V2(WorkflowStartV2)` (NEVER reorder existing
/// variants — the rkyv discriminant tags are positional), re-alias
/// [`WorkflowStart`] / [`WorkflowStartLatest`] to the new payload, add a
/// `From<WorkflowStartV1> for WorkflowStartV2` impl, re-pin
/// [`Self::discriminant_offset_from_end`], and add a `FIXTURE_V2` golden-bytes
/// fixture — all in the same commit per the version-bump procedure.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum WorkflowStartEnvelope {
    /// The V1 payload — the only variant today.
    V1(WorkflowStartV1),
}

impl VersionedEnvelope for WorkflowStartEnvelope {
    type Latest = WorkflowStartV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `WorkflowStartEnvelope` archives, measured from
    /// the END of the archive bytes (rkyv 0.8 places the fixed-size root —
    /// including the outer enum discriminant byte — at the buffer tail, so
    /// the from-end offset is stable across all `input` / `name` lengths).
    ///
    /// Empirically pinned against the canonical V1 payload by the
    /// schema-evolution fixture's triangulation test
    /// (`workflow_start_envelope_discriminant_offset_triangulates`). Re-pin
    /// alongside `GOLDEN_DISCRIMINANT_OFFSET_V1` in lockstep at every variant
    /// or layout change, per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(WORKFLOW_START_DISCRIMINANT_OFFSET_FROM_END)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first variant).
        &[0]
    }

    fn type_name() -> &'static str {
        "WorkflowStartEnvelope"
    }
}

/// Empirically-pinned from-end discriminant offset for
/// [`WorkflowStartEnvelope`] V1 archives — the production-side half of the
/// two-source triangulation (the test side pins
/// `GOLDEN_DISCRIMINANT_OFFSET_V1` independently). Pinned in DELIVER Slice 01
/// (step 01-02): rkyv rejects a flip at `from_end == 20` of the canonical
/// archive with `invalid discriminant for enum 'ArchivedWorkflowStartEnvelope'`.
/// Update BOTH this constant and the test-side golden in the same commit on
/// any `V<N+1>` bump.
const WORKFLOW_START_DISCRIMINANT_OFFSET_FROM_END: usize = 20;

impl WorkflowStartV1 {
    /// Archive this [`WorkflowStart`] for persistence through the
    /// `IntentStore` (the co-located typed codec — ADR-0048 §4b). Wraps the
    /// payload in [`WorkflowStartEnvelope::V1`] via
    /// [`VersionedEnvelope::latest`] (the single write-side wrapping site)
    /// and rkyv-serialises.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv archive of
    /// `WorkflowStartEnvelope::latest(self.clone())`. Two archivals of the
    /// same logical spec produce byte-identical output (rkyv archives are
    /// canonical by construction). Callers pass `bytes.as_ref()` to the
    /// `IntentStore` byte-level write surface.
    ///
    /// # Observable invariants
    ///
    /// `WorkflowStart::from_store_bytes(&self.archive_for_store()?)` returns
    /// `Ok(self_owned)` bit-equivalent to `self` (the round-trip property the
    /// `workflow_start_archive_round_trips_through_store_codec` proptest pins).
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] if the rkyv serialiser fails
    /// (unreachable for valid payloads).
    pub fn archive_for_store(&self) -> Result<rkyv::util::AlignedVec, EnvelopeError> {
        let envelope = WorkflowStartEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`WorkflowStart`] (the co-located
    /// typed codec read side — ADR-0048 §4b). Runs the pre-decode
    /// known-variant probe, rkyv-deserialises into [`WorkflowStartEnvelope`],
    /// and projects via [`VersionedEnvelope::into_latest`].
    ///
    /// # Edge cases
    ///
    /// * Empty / truncated / corrupt `bytes` → [`EnvelopeError::Malformed`].
    /// * Future-binary `V<N+1>` bytes → [`EnvelopeError::UnknownVersion`].
    ///
    /// # Intent fail-fast precursor (ADR-0065 §5)
    ///
    /// This returns the codec-level [`EnvelopeError`] directly. The
    /// intent-store hydrate site (Slice 03, #217 engine discharge) wraps this
    /// in the `IntentStore` error surface AND emits the
    /// `health.startup.refused` event before refusing to start — the
    /// asymmetric-read policy (ADR-0048 §3, intent is SSOT) lives one layer up
    /// at the driving port, not in this codec primitive.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError`] when the bytes do not decode to a known
    /// variant that projects cleanly to [`WorkflowStartV1`].
    pub fn from_store_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        decode_envelope_bytes::<WorkflowStartEnvelope>(bytes)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // -----------------------------------------------------------------------
    // TerminalError + WorkflowStatus unit-level property coverage
    // (workflow-result-error-model step 01-01, ADR-0065 §2/§3).
    //
    // These live in `src/` (not `tests/acceptance/`) because the engine-only
    // `TerminalError::budget_exhausted()` ctor is `pub(crate)` — only an
    // in-crate test can exercise the `BudgetExhausted` variant. The
    // acceptance suite covers the publicly-constructible variants; this
    // suite extends to the full variant space plus the construction-time
    // length-cap truncation determinism (the property that closes the
    // ADR-0064 §3 free-text replay-determinism hazard).
    // -----------------------------------------------------------------------

    /// Author-supplied detail strategy bounded UNDER the construction-time
    /// length cap — generated detail survives verbatim, so the roundtrip
    /// asserts on the un-truncated shape. Over-cap truncation determinism is
    /// the separate `terminal_error_detail_length_cap_is_deterministic`
    /// property below.
    fn arb_short_detail() -> impl Strategy<Value = String> {
        "[A-Za-z0-9 ./:_-]{0,80}"
    }

    /// EVERY `TerminalError` variant, including the engine-only
    /// `BudgetExhausted` (reachable here via the `pub(crate)` ctor).
    fn arb_terminal_error() -> impl Strategy<Value = TerminalError> {
        prop_oneof![
            arb_short_detail().prop_map(|d| TerminalError::explicit(&d)),
            arb_short_detail().prop_map(|d| TerminalError::malformed_input(&d)),
            arb_short_detail().prop_map(|d| TerminalError::output_encode(&d)),
            arb_short_detail().prop_map(|d| TerminalError::budget_exhausted(&d)),
        ]
    }

    /// EVERY `WorkflowStatus` variant — `Completed` (opaque output bytes),
    /// `Failed` (embedded `TerminalError`, full variant space), `Cancelled`,
    /// `TimedOut`.
    fn arb_workflow_status() -> impl Strategy<Value = WorkflowStatus> {
        prop_oneof![
            prop::collection::vec(any::<u8>(), 0..=64)
                .prop_map(|output| WorkflowStatus::Completed { output }),
            arb_terminal_error().prop_map(|terminal| WorkflowStatus::Failed { terminal }),
            Just(WorkflowStatus::Cancelled),
            Just(WorkflowStatus::TimedOut),
        ]
    }

    fn cbor_round_trip<T>(value: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let mut bytes: Vec<u8> = Vec::new();
        ciborium::into_writer(value, &mut bytes).expect("encode to CBOR");
        ciborium::from_reader(bytes.as_slice()).expect("decode from CBOR")
    }

    /// A valid [`WorkflowStart`] across the observable input space: every
    /// kebab `name` shape the `WorkflowName` grammar accepts, paired with an
    /// arbitrary opaque `input` byte vector (the erased `W::Input`, including
    /// the empty case). `input` is generated as raw bytes — the codec treats
    /// it as opaque, so the property must hold for any byte content, not just
    /// well-formed CBOR.
    fn arb_workflow_start() -> impl Strategy<Value = WorkflowStart> {
        ("[a-z][a-z0-9-]{0,62}", prop::collection::vec(any::<u8>(), 0..=128)).prop_map(
            |(raw, input)| WorkflowStart {
                name: WorkflowName::new(&raw).expect("generator emits a valid kebab name"),
                input,
            },
        )
    }

    proptest! {
        /// `TerminalErrorRoundtrip` (AC#7) — for EVERY `TerminalError` value
        /// across all four `TerminalErrorKind` variants (including the
        /// engine-only `BudgetExhausted`), `encode → decode == original` and
        /// the decoded `kind()` matches. The serde shape is what the durable
        /// journal `Terminal` command + observation row depend on (ADR-0065
        /// §2).
        #[test]
        fn terminal_error_round_trips_for_every_variant(error in arb_terminal_error()) {
            let decoded = cbor_round_trip(&error);
            prop_assert_eq!(&decoded, &error, "TerminalError round-trips byte-equal");
            prop_assert_eq!(decoded.kind(), error.kind(), "kind() preserved across roundtrip");
        }

        /// `WorkflowStatusRoundtrip` (AC#7) — for EVERY `WorkflowStatus`
        /// variant, `encode → decode == original`. `Completed`'s opaque
        /// output bytes and `Failed`'s embedded `TerminalError` (full variant
        /// space) both survive byte-equal (ADR-0065 §3).
        #[test]
        fn workflow_status_round_trips_for_every_variant(status in arb_workflow_status()) {
            let decoded = cbor_round_trip(&status);
            prop_assert_eq!(&decoded, &status, "WorkflowStatus round-trips byte-equal");
        }

        /// Detail length-cap truncation determinism (AC#2; closes the
        /// ADR-0064 §3 free-text replay-determinism hazard). For ANY input —
        /// including arbitrary over-long input — the constructed detail is
        /// (a) never longer than [`TERMINAL_ERROR_DETAIL_MAX`] and (b) STABLE:
        /// constructing twice from the same input yields byte-identical
        /// detail. Determinism is the load-bearing property — a durable
        /// terminal that embedded a non-deterministic truncation would break
        /// bit-identical replay.
        #[test]
        fn terminal_error_detail_length_cap_is_deterministic(raw in ".{0,2048}") {
            let once = TerminalError::explicit(&raw);
            let twice = TerminalError::explicit(&raw);
            prop_assert_eq!(&once, &twice, "construction is deterministic for identical input");
            prop_assert!(
                once.detail().len() <= TERMINAL_ERROR_DETAIL_MAX,
                "capped detail never exceeds TERMINAL_ERROR_DETAIL_MAX ({} > {})",
                once.detail().len(),
                TERMINAL_ERROR_DETAIL_MAX
            );
            // Within-cap input is preserved verbatim (no spurious truncation).
            if raw.len() <= TERMINAL_ERROR_DETAIL_MAX {
                prop_assert_eq!(once.detail(), raw.as_str(), "within-cap detail preserved verbatim");
            }
        }

        /// `WorkflowStartCodecRoundtrip` (#217 / NEW-3 compensating PBT for the
        /// EXEMPT golden-bytes fixture) — the co-located typed codec is a
        /// symmetric pair: for EVERY `WorkflowStart` (every `name` shape, every
        /// opaque `input` including empty), `archive_for_store` →
        /// `from_store_bytes` yields a value byte-equal to the original
        /// (ADR-0048 §4b, the `Job` aggregate precedent). This is the
        /// observable invariant the `VersionedEnvelope` round-trip contract
        /// pins for the OUTER rkyv layer; the inner `input` bytes survive
        /// opaque (the rkyv-outer / CBOR-inner separation).
        #[test]
        fn workflow_start_archive_round_trips_through_store_codec(start in arb_workflow_start()) {
            let bytes = start.archive_for_store().expect("archive_for_store on a valid start intent");
            let decoded = WorkflowStart::from_store_bytes(bytes.as_ref())
                .expect("from_store_bytes on freshly-archived bytes");
            prop_assert_eq!(&decoded, &start, "WorkflowStart round-trips through the store codec");
            prop_assert_eq!(
                decoded.input.as_slice(),
                start.input.as_slice(),
                "opaque input bytes survive the rkyv outer round-trip verbatim",
            );
        }
    }

    /// Over-long detail is truncated to exactly the cap on a deterministic
    /// byte boundary, across every public + engine-only constructor (AC#2).
    /// Pins the canonical readable case the property generalises: a detail
    /// far longer than the cap collapses to `TERMINAL_ERROR_DETAIL_MAX` bytes
    /// and the same kind is reported.
    #[test]
    fn terminal_error_caps_over_long_detail_on_every_constructor() {
        let over_long = "x".repeat(TERMINAL_ERROR_DETAIL_MAX * 4);
        for (error, expected_kind) in [
            (TerminalError::explicit(&over_long), TerminalErrorKind::Explicit),
            (TerminalError::malformed_input(&over_long), TerminalErrorKind::MalformedInput),
            (TerminalError::output_encode(&over_long), TerminalErrorKind::OutputEncode),
            (TerminalError::budget_exhausted(&over_long), TerminalErrorKind::BudgetExhausted),
        ] {
            assert_eq!(
                error.detail().len(),
                TERMINAL_ERROR_DETAIL_MAX,
                "over-long detail capped to exactly TERMINAL_ERROR_DETAIL_MAX"
            );
            assert_eq!(error.kind(), expected_kind, "constructor sets the matching kind");
        }
    }

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

    /// `SignalKey` newtype completeness (`development.md` § "Newtypes"):
    /// the validating constructor accepts the canonical kebab form and its
    /// `FromStr` / `Display` / serde wire shape round-trip bit-equal. Each
    /// reject branch (empty / uppercase / overlong) maps to its own
    /// structured error — the mutation seam the validator must defend.
    #[test]
    fn signal_key_accepts_canonical_and_rejects_invalid_inputs() {
        // Driving port: the SignalKey::new validating constructor.
        let key = SignalKey::new("cert-ready-aa00").expect("valid kebab signal key");
        assert_eq!(key.as_str(), "cert-ready-aa00", "canonical form preserved verbatim");
        // FromStr is the same validation surface as new().
        assert_eq!("cert-ready-aa00".parse::<SignalKey>().expect("FromStr"), key);
        // Display round-trips through FromStr (canonical-form equivalence).
        assert_eq!(key.to_string().parse::<SignalKey>().expect("Display→FromStr"), key);
        // Each reject branch maps to its own structured variant.
        assert!(matches!(SignalKey::new(""), Err(SignalKeyError::Empty)));
        assert!(matches!(SignalKey::new("Cert-Ready"), Err(SignalKeyError::BadShape)));
        assert!(matches!(
            SignalKey::new(&"a".repeat(SIGNAL_KEY_MAX + 1)),
            Err(SignalKeyError::TooLong { .. })
        ));
    }

    /// `SignalKey` + `SignalValue` serde wire shape matches `Display` /
    /// `FromStr` exactly (`development.md` § "Newtype completeness"): both
    /// CBOR-round-trip bit-equal, the property the `SignalSeen` journal
    /// variant depends on (it carries a `SignalValue` and is CBOR-encoded
    /// per ADR-0063 §2). `SignalValue` is opaque — any payload round-trips.
    #[test]
    fn signal_key_and_value_cbor_round_trip_bit_equal() {
        let key = SignalKey::new("provision-done").expect("valid signal key");
        let mut key_bytes: Vec<u8> = Vec::new();
        ciborium::into_writer(&key, &mut key_bytes).expect("encode SignalKey");
        let decoded_key: SignalKey =
            ciborium::from_reader(key_bytes.as_slice()).expect("decode SignalKey");
        assert_eq!(decoded_key, key, "SignalKey round-trips through CBOR bit-equal");

        // SignalValue is opaque: an arbitrary (even empty) payload survives.
        for raw in ["", "ok", "0xDEADBEEF payload"] {
            let value = SignalValue::new(raw);
            let mut value_bytes: Vec<u8> = Vec::new();
            ciborium::into_writer(&value, &mut value_bytes).expect("encode SignalValue");
            let decoded_value: SignalValue =
                ciborium::from_reader(value_bytes.as_slice()).expect("decode SignalValue");
            assert_eq!(decoded_value, value, "SignalValue round-trips through CBOR bit-equal");
            assert_eq!(decoded_value.as_str(), raw, "opaque payload preserved verbatim");
        }
    }

    /// Regression: the `NonDeterministic` Display hard-coded a `ctx.run`
    /// prefix, so a divergence detected on ANY replay surface — including
    /// `ctx.sleep` / `ctx.wait_for_signal` / `ctx.emit_action` — rendered
    /// as "workflow ctx.run non-deterministic", naming a surface that was
    /// never involved. The message must name the await-op the divergence
    /// was actually detected on.
    #[test]
    fn non_deterministic_display_names_the_diverging_await_op() {
        // Each replay surface renders its OWN ctx-method label, never a
        // hard-coded `ctx.run` for the sleep / signal / emit sites (the
        // bug: a `ctx.sleep` landing on a journal `SignalAwaited` surfaced
        // "workflow ctx.run non-deterministic", naming a surface that was
        // never involved).
        let cases = [
            (AwaitOp::Run, "ctx.run"),
            (AwaitOp::Sleep, "ctx.sleep"),
            (AwaitOp::Signal, "ctx.wait_for_signal"),
            (AwaitOp::EmitAction, "ctx.emit_action"),
        ];
        for (op, label) in cases {
            assert_eq!(op.as_str(), label, "AwaitOp owns its ctx-method label");
            let err = WorkflowCtxError::NonDeterministic {
                op,
                expected: "RunResult".to_string(),
                actual: "SleepArmed".to_string(),
            };
            let rendered = err.to_string();
            assert!(
                rendered.contains(label),
                "the {op:?} divergence must render its own ctx label {label:?}: {rendered:?}"
            );
            // The non-run surfaces must NOT claim `ctx.run` (the bug).
            if op != AwaitOp::Run {
                assert!(
                    !rendered.contains("ctx.run"),
                    "a {op:?} divergence must not be rendered with a ctx.run prefix: {rendered:?}"
                );
            }
            // The determinism payload still survives verbatim.
            assert!(
                rendered.contains("RunResult") && rendered.contains("SleepArmed"),
                "expected/actual identities survive into the message: {rendered:?}"
            );
            // Brace-free: this detail is projected into a `TerminalError`
            // whose DST byte-identical-trajectory guard rejects whole-entry
            // `Debug` (`{` / `0x`). `op` renders via its `Display` label,
            // so no brace is introduced.
            assert!(!rendered.contains('{'), "rendered detail must stay brace-free: {rendered:?}");
        }
    }
}
