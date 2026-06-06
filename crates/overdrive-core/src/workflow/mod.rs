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

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

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

/// Errors surfaced from [`WorkflowCtx`] await-ops.
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

    /// A replay-path `ctx.run` found a recorded step whose name does not
    /// match the name the workflow body is replaying at this cursor
    /// position — a non-deterministic divergence between the recorded
    /// trajectory and the current run. Fail-closed: a workflow body that
    /// reorders / renames its steps cannot replay a journal recorded
    /// against the prior shape (journal replay must be bit-identical,
    /// `development.md` § "Workflow contract").
    #[error("workflow ctx.run non-deterministic: expected step {expected:?}, got {actual:?}")]
    NonDeterministic {
        /// The step name recorded in the journal at this cursor.
        expected: String,
        /// The step name the replaying workflow body presented.
        actual: String,
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
    async fn replay_sleep(&self) -> Option<Duration>;

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
    async fn replay_signal(&self, signal_key: &SignalKey) -> Option<SignalValue>;

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
    async fn replay_emit(&self) -> bool;

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

    async fn replay_sleep(&self) -> Option<Duration> {
        None
    }

    async fn record_sleep_armed(&self, _deadline_unix: Duration) -> Result<(), WorkflowCtxError> {
        Ok(())
    }

    async fn replay_signal(&self, _signal_key: &SignalKey) -> Option<SignalValue> {
        None
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

    async fn replay_emit(&self) -> bool {
        false
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
        Self { clock, transport, entropy, journal }
    }

    /// Run one durable step `f`, named `name`, and return its result —
    /// the general durable-step await-surface (the Restate `ctx.run`
    /// model). This is the slice-01 await-surface; a workflow body
    /// performs every effect (transport sends, future external calls)
    /// INSIDE a `ctx.run` closure so the result is journaled and replayed.
    ///
    /// **Check-then-record (ADR-0064 §3), POSITIONAL identity.** The op
    /// consults the engine's journal cursor at the current position:
    /// - **Replay:** if the cursor has a recorded result at this step, the
    ///   recorded CBOR bytes are decoded into `T` and returned WITHOUT
    ///   polling `f` — `f` is dropped unpolled, so the effect never
    ///   re-fires. This is the exactly-once guarantee on the replay path.
    /// - **Live:** otherwise `f` is awaited, its result is CBOR-encoded
    ///   and durably recorded (append + fsync per ADR-0063 §4) via the
    ///   cursor BEFORE returning, and the cursor advances.
    ///
    /// **Honest semantics:** the effect inside `f` is *at-least-once* (a
    /// crash after `f.await` but before the record is durable re-fires the
    /// effect on resume); the run await-point is *exactly-once on the
    /// replay path* (once the result is journaled, resume replays it
    /// without re-polling `f`). The journal-after-effect ordering is what
    /// makes the replay path exactly-once — it is NOT an unconditional
    /// exactly-once guarantee for the effect itself.
    ///
    /// `name` is recorded for diagnostics and a replay-determinism check
    /// (a recorded step whose name diverges from the replaying body's
    /// `name` fails closed with [`WorkflowCtxError::NonDeterministic`]).
    /// Identity is the cursor position, not `name`.
    ///
    /// # Errors
    ///
    /// - [`WorkflowCtxError::Serialize`] — the live result could not be
    ///   CBOR-encoded.
    /// - [`WorkflowCtxError::Deserialize`] — a recorded result could not
    ///   be CBOR-decoded into `T` on replay.
    /// - [`WorkflowCtxError::NonDeterministic`] — a recorded step's name
    ///   diverges from `name` at this cursor position.
    /// - [`WorkflowCtxError::JournalRecord`] — the live-path durable
    ///   record failed.
    pub async fn run<T, F>(&self, name: &str, f: F) -> Result<T, WorkflowCtxError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
        F: std::future::Future<Output = T> + Send,
    {
        // Replay path — decode the recorded result, never poll `f` (the
        // effect inside `f` never re-fires on the replay path).
        if let Some(recorded_bytes) = self.journal.replay_run(name).await? {
            let value: T = ciborium::from_reader(recorded_bytes.as_slice())
                .map_err(|e| WorkflowCtxError::Deserialize { message: e.to_string() })?;
            return Ok(value);
        }

        // Live path — poll `f`, then durably record before returning
        // (journal-after-effect, ADR-0063 §4). The effect is at-least-once;
        // the record makes the replay path exactly-once.
        let result = f.await;
        let mut bytes: Vec<u8> = Vec::new();
        ciborium::into_writer(&result, &mut bytes)
            .map_err(|e| WorkflowCtxError::Serialize { message: e.to_string() })?;
        self.journal.record_run(name, &bytes).await?;
        Ok(result)
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
    /// Returns [`WorkflowCtxError::JournalRecord`] when the live-path
    /// durable record of the armed deadline fails.
    pub async fn sleep(&self, duration: Duration) -> Result<(), WorkflowCtxError> {
        // Replay path — recompute remaining wait from the recorded
        // absolute deadline (an input), never a persisted remaining cache.
        if let Some(deadline_unix) = self.journal.replay_sleep().await {
            let now = self.clock.unix_now();
            if let Some(remaining) = deadline_unix.checked_sub(now) {
                self.clock.sleep(remaining).await;
            }
            // deadline already passed → return immediately (no re-park).
            return Ok(());
        }

        // Live path — compute the absolute deadline, durably record it
        // (fsync-then-park, ADR-0063 §4), then park on the Clock deadline.
        let deadline_unix = self.clock.unix_now() + duration;
        self.journal.record_sleep_armed(deadline_unix).await?;
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
    /// - [`WorkflowCtxError::Signal`] — the signal surface read failed.
    /// - [`WorkflowCtxError::JournalRecord`] — a durable record failed.
    pub async fn wait_for_signal(
        &self,
        signal_key: SignalKey,
    ) -> Result<SignalValue, WorkflowCtxError> {
        // Replay path — return the recorded SignalSeen value, never
        // re-read the signal surface. A SignalAwaited with no following
        // SignalSeen is NOT a replay hit (replay_signal returns None), so
        // the live block below re-enters on the SAME signal.
        if let Some(value) = self.journal.replay_signal(&signal_key).await {
            return Ok(value);
        }
        // Live path, first half — record SignalAwaited durably (fsync per
        // ADR-0063 §4) and advance the cursor. On a crash-while-blocked
        // resume the SignalAwaited is already at the cursor: this advances
        // past it WITHOUT appending a duplicate (see
        // `record_signal_awaited`).
        self.journal.record_signal_awaited(&signal_key).await?;

        // Live path, block — poll the signal surface; while ABSENT, park on
        // the injected Clock and re-poll. This is a genuine block on an
        // absent signal (the run future stays pending), not an immediate
        // resolve. Under SimClock the harness advances logical time and
        // writes the signal row; under SystemClock the park is a real
        // timer. No busy-spin: each absent poll parks for SIGNAL_POLL.
        let value = loop {
            if let Some(value) = self.journal.poll_signal(&signal_key).await? {
                break value;
            }
            self.clock.sleep(SIGNAL_POLL).await;
        };

        // Live path, second half — record SignalSeen { value } durably at
        // the NEXT cursor position and return the value.
        self.journal.record_signal_seen(&signal_key, &value).await?;
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
    /// - [`WorkflowCtxError::ActionChannel`] — the Action channel send
    ///   failed (channel closed / full).
    /// - [`WorkflowCtxError::JournalRecord`] — the durable record failed.
    pub async fn emit_action(&self, action: Action) -> Result<(), WorkflowCtxError> {
        // Replay path — the Action was already emitted on a prior run; do
        // NOT re-send it (exactly-once on the replay path; the live path
        // below is at-least-once — see the rustdoc "Honest semantics").
        if self.journal.replay_emit().await {
            return Ok(());
        }
        // Live path — the engine sends on the Action channel then records
        // ActionEmitted durably before returning.
        self.journal.emit_action(action).await
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
}
