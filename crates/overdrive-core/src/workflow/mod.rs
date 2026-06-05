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
/// Every `ctx` await-op is a check-then-record point. Identity is
/// POSITIONAL — the cursor index, exactly as the sleep branch already is.
/// `name` is carried for diagnostics and a replay-determinism check, not
/// for identity. For `ctx.run` the cursor is consulted via
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
