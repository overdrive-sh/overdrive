//! `WorkflowEngine` — the durable-async executor for the `Workflow`
//! primitive (ADR-0064 §1, §3, §5).
//!
//! The engine is to workflows what `Driver` is to allocations: the async
//! executor a *pure-sync* reconciler drives **off the action-shim**
//! (ADR-0023's sanctioned async boundary), NOT a reconciler itself. The
//! workflow-lifecycle reconciler (lands 01-06) emits
//! `Action::StartWorkflow`; the action-shim hands the instance to
//! [`WorkflowEngine::start`], which `load_journal`s, builds a
//! [`WorkflowCtx`] carrying a durable [`JournalCursor`] handle, and drives
//! the author's `async fn run` as a tracked `tokio` task. This is the
//! upheld two-primitive doctrine (R3): the reconciler manages WHICH
//! instances should exist; the engine manages HOW each instance's steps
//! execute between start and terminal.
//!
//! # The replay cursor (ADR-0064 §3)
//!
//! On (re)start the engine loads the instance's journal into a **replay
//! buffer** and constructs a [`JournalCursorHandle`] at step 0. Each
//! `ctx.run` durable step is check-then-record (POSITIONAL identity — the
//! cursor index):
//!
//! - **Replay (cursor < buffer length):** the handle returns the recorded
//!   CBOR result bytes WITHOUT polling the step's future — the
//!   exactly-once guarantee on the replay path (K1). The cursor advances.
//! - **Live (cursor == buffer length):** the handle returns `Ok(None)`;
//!   the ctx polls the step's future, then the handle appends a
//!   [`JournalEntry::RunResult`] with fsync BEFORE returning (ADR-0063
//!   §4 fsync-then-suspend) and advances the cursor.
//!
//! On `run` returning a [`WorkflowResult`], the engine appends a
//! [`JournalEntry::Terminal`] recording the canonical result string — the
//! durable terminal surface for slice 01.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    JournalCursor, SignalKey, SignalValue, Workflow, WorkflowCtx, WorkflowCtxError, WorkflowName,
    WorkflowResult, WorkflowSpec,
};

use crate::journal::{JournalEntry, JournalStore, WorkflowId};

/// The sender half of the engine's **Action channel** — the channel whose
/// receiver the production `spawn_workflow_emit_drain` task forwards into
/// the `action_shim` dispatch path (→ Raft commit path), exactly as a
/// reconciler-emitted Action reaches the shim. A `ctx.emit_action` hands
/// its typed [`Action`] to this sender (ADR-0064 §4; `development.md`
/// Workflow contract rule 6 — workflow→cluster mutations go through Raft,
/// never a direct `IntentStore` write).
pub type ActionEmitSender = mpsc::UnboundedSender<Action>;

/// The receiver half of the engine's Action channel. In production the
/// `spawn_workflow_emit_drain` task (the dedicated emit-drain task spawned
/// in `run_server`) takes this receiver and drains every item into
/// `action_shim::dispatch_with_workflow_intent`; a test harness may take it
/// instead. Every item is an [`Action`] a workflow emitted via
/// `ctx.emit_action`.
pub type ActionEmitReceiver = mpsc::UnboundedReceiver<Action>;

/// A factory producing a fresh [`Workflow`] trait object on demand. The
/// engine resolves a [`WorkflowSpec`]'s [`WorkflowName`] to one of these
/// and calls it to obtain a fresh instance to drive.
pub type WorkflowFactory = Box<dyn Fn() -> Box<dyn Workflow> + Send + Sync>;

/// Maps a [`WorkflowName`] (the workflow *kind*) to its author-supplied
/// [`Workflow`] factory. The composition root registers every first-party
/// workflow here at boot; the engine looks up `spec.name` on each
/// `StartWorkflow`.
///
/// `BTreeMap` per `.claude/rules/development.md` § "Ordered-collection
/// choice" — the registry is small and point-accessed, but keeping it
/// ordered costs nothing and avoids a `// dst-lint: hashmap-ok` waiver.
#[derive(Default)]
pub struct WorkflowRegistry {
    factories: BTreeMap<WorkflowName, WorkflowFactory>,
}

impl WorkflowRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self { factories: BTreeMap::new() }
    }

    /// Register `factory` under `name`. A later `StartWorkflow` carrying a
    /// spec with this name drives a fresh instance from `factory`.
    /// Re-registering the same name replaces the prior factory.
    pub fn register<F>(&mut self, name: WorkflowName, factory: F)
    where
        F: Fn() -> Box<dyn Workflow> + Send + Sync + 'static,
    {
        self.factories.insert(name, Box::new(factory));
    }

    /// Resolve a fresh [`Workflow`] for `name`, or `None` if unregistered.
    #[must_use]
    pub fn resolve(&self, name: &WorkflowName) -> Option<Box<dyn Workflow>> {
        self.factories.get(name).map(|factory| factory())
    }
}

/// Errors from the workflow engine's start path.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowEngineError {
    /// The `StartWorkflow` spec named a workflow kind not registered in
    /// the engine's [`WorkflowRegistry`]. The composition root must
    /// register every first-party workflow at boot.
    #[error("workflow kind not registered: {name}")]
    UnknownWorkflow {
        /// The unregistered workflow name.
        name: String,
    },

    /// Loading the instance's journal failed.
    #[error("load_journal failed: {0}")]
    LoadJournal(#[from] crate::journal::JournalStoreError),
}

/// The durable-async executor. Drives author `async fn run` futures as
/// tracked `tokio` tasks, each consulting a per-instance journal cursor
/// for crash-safe replay (ADR-0064 §3).
///
/// Holds `tokio` (the `JoinSet` task surface) — correctly, because the
/// engine is `adapter-host`-class control-plane code, NOT `core`. The
/// `Workflow` trait + `WorkflowCtx` it drives stay in `overdrive-core`
/// (ADR-0064 §1).
pub struct WorkflowEngine {
    journal: Arc<dyn JournalStore>,
    clock: Arc<dyn Clock>,
    transport: Arc<dyn Transport>,
    entropy: Arc<dyn Entropy>,
    /// The observation store the engine writes the terminal-result row to
    /// on `run` terminal (ADR-0064 §2). The sanctioned shim
    /// `ObservationStore::write` path — NOT a direct bypass of the
    /// channels. Mandatory at construction per
    /// `.claude/rules/development.md` § "Port-trait dependencies".
    obs: Arc<dyn ObservationStore>,
    /// The sender half of the **Action channel** (→ Raft) a workflow's
    /// `ctx.emit_action` sends on (ADR-0064 §4). Threaded into every
    /// instance's [`JournalCursorHandle`] so the live emit path hands the
    /// typed Action to the channel the production `spawn_workflow_emit_drain`
    /// task forwards into the SAME `action_shim` dispatch path a
    /// reconciler-emitted Action takes — NOT a direct `IntentStore` write.
    /// Mandatory at construction per `.claude/rules/development.md`
    /// § "Port-trait dependencies".
    action_emit: ActionEmitSender,
    registry: Arc<WorkflowRegistry>,
    /// Tracked task set for live instances — the engine owns it the same
    /// way the reconciler runtime owns its tick task (ADR-0023 §4).
    tasks: Mutex<JoinSet<()>>,
    /// The set of instance [`CorrelationKey`]s with a live (running, not
    /// yet terminal) engine task. Inserted on [`Self::start`], removed by
    /// the spawned task itself once `run` reaches terminal. This is the
    /// "engine live-task set" the workflow-lifecycle reconciler's
    /// `hydrate_actual` reads to populate
    /// `WorkflowInstanceState::has_live_task` (ADR-0064 §5): a
    /// running-in-intent instance with no live task and no terminal row
    /// is the re-emit trigger on restart.
    ///
    /// `Arc<Mutex<BTreeSet<..>>>` so the spawned task can drop its own
    /// entry on terminal without holding `&self`. `BTreeSet` for
    /// deterministic iteration per `.claude/rules/development.md`
    /// § "Ordered-collection choice".
    live_instances: Arc<Mutex<BTreeSet<CorrelationKey>>>,
    /// The receiver half of the Action channel, parked here until a
    /// consumer takes it via [`Self::take_action_emit_receiver`]. The
    /// engine owns BOTH halves so [`Self::new`]'s signature stays
    /// unchanged (the emit channel is an engine-internal wiring detail,
    /// not a constructor dependency); in production the dedicated
    /// `spawn_workflow_emit_drain` task takes the receiver once at boot
    /// and drains emitted Actions into the `action_shim` dispatch path (a
    /// test harness may take it instead). `Mutex<Option<..>>` so the take
    /// is `&self` and single-shot.
    action_emit_rx: Mutex<Option<ActionEmitReceiver>>,
}

impl WorkflowEngine {
    /// Construct an engine over the injected journal store + ports +
    /// workflow registry. Every dependency is mandatory (no builder, no
    /// defaulting) per `.claude/rules/development.md` § "Port-trait
    /// dependencies".
    #[must_use]
    pub fn new(
        journal: Arc<dyn JournalStore>,
        clock: Arc<dyn Clock>,
        transport: Arc<dyn Transport>,
        entropy: Arc<dyn Entropy>,
        registry: WorkflowRegistry,
        obs: Arc<dyn ObservationStore>,
    ) -> Self {
        // The engine owns BOTH halves of the Action channel. The sender is
        // threaded into every instance's JournalCursorHandle (the live
        // emit path); the receiver is parked until a consumer takes it via
        // `take_action_emit_receiver`. An UNBOUNDED channel because the
        // emit is on the workflow's async task (a bounded send could block
        // the task across the journal-fsync window); the consumer drains
        // promptly into the action_shim.
        let (action_emit, action_emit_rx) = mpsc::unbounded_channel();
        Self {
            journal,
            clock,
            transport,
            entropy,
            obs,
            action_emit,
            registry: Arc::new(registry),
            tasks: Mutex::new(JoinSet::new()),
            live_instances: Arc::new(Mutex::new(BTreeSet::new())),
            action_emit_rx: Mutex::new(Some(action_emit_rx)),
        }
    }

    /// Take the receiver half of the engine's Action channel. Single-shot:
    /// the first caller receives `Some(receiver)`, subsequent callers
    /// receive `None`. The consumer drains emitted Actions into the
    /// `action_shim` dispatch path (→ Raft). In production the consumer is
    /// the dedicated `spawn_workflow_emit_drain` task spawned in
    /// `run_server`; a test harness may take it instead. Per ADR-0064 §4
    /// the drain forwards each emitted Action into the SAME
    /// `action_shim::dispatch_with_workflow_intent` path a reconciler-emitted
    /// Action takes; `ctx.emit_action` reuses it rather than bypassing Raft.
    pub async fn take_action_emit_receiver(&self) -> Option<ActionEmitReceiver> {
        self.action_emit_rx.lock().await.take()
    }

    /// Snapshot the set of instance [`CorrelationKey`]s with a live
    /// (running, not-yet-terminal) engine task. The workflow-lifecycle
    /// reconciler's `hydrate_actual` reads this to mark
    /// `WorkflowInstanceState::has_live_task` (ADR-0064 §5).
    ///
    /// On a fresh process boot the set is empty — every
    /// previously-running instance reads as `has_live_task = false`,
    /// which is exactly the re-emit trigger the lifecycle reconciler
    /// needs to crash-resume a running-in-intent instance.
    #[must_use]
    pub async fn live_instances(&self) -> BTreeSet<CorrelationKey> {
        self.live_instances.lock().await.clone()
    }

    /// Start (or resume) the workflow instance `workflow_id` for `spec`,
    /// off the action-shim (ADR-0064 §5). Resolves `spec.name` to its
    /// registered [`Workflow`], `load_journal`s the instance's run (empty
    /// on first start; populated on resume), builds a [`WorkflowCtx`]
    /// carrying a durable [`JournalCursorHandle`], and spawns `run(&ctx)`
    /// as a tracked task.
    ///
    /// Returns once the task is *spawned* (the async body runs
    /// concurrently); callers awaiting completion use [`join_all`](Self::join_all).
    ///
    /// # Errors
    ///
    /// - [`WorkflowEngineError::UnknownWorkflow`] — `spec.name` is not
    ///   registered.
    /// - [`WorkflowEngineError::LoadJournal`] — the instance journal could
    ///   not be loaded.
    pub async fn start(
        &self,
        spec: &WorkflowSpec,
        correlation: &CorrelationKey,
        workflow_id: &WorkflowId,
    ) -> Result<(), WorkflowEngineError> {
        let workflow = self.registry.resolve(&spec.name).ok_or_else(|| {
            WorkflowEngineError::UnknownWorkflow { name: spec.name.as_str().to_string() }
        })?;

        let replay_buffer = self.journal.load_journal(workflow_id).await?;

        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new_with_channels(
            Arc::clone(&self.journal),
            workflow_id.clone(),
            replay_buffer,
            self.action_emit.clone(),
            Arc::clone(&self.obs),
        ));

        let ctx = WorkflowCtx::new(
            Arc::clone(&self.clock),
            Arc::clone(&self.transport),
            Arc::clone(&self.entropy),
            cursor,
        );

        let journal = Arc::clone(&self.journal);
        let obs = Arc::clone(&self.obs);
        let correlation = correlation.clone();
        let workflow_id = workflow_id.clone();

        // Mark this instance live BEFORE spawning so a hydrate_actual that
        // races the spawn sees the instance as running (has_live_task =
        // true) — the reconciler must NOT re-emit StartWorkflow for an
        // instance the engine is already driving (ADR-0064 §5).
        let live_instances = Arc::clone(&self.live_instances);
        live_instances.lock().await.insert(correlation.clone());

        // Spawn the author's async body as a tracked task (ADR-0064 §5 —
        // the engine owns a tokio task set, the same way the reconciler
        // runtime owns its tick task).
        let mut tasks = self.tasks.lock().await;
        tasks.spawn(async move {
            let result = workflow.run(&ctx).await;
            // Durable terminal record (slice-01 terminal surface,
            // ADR-0064 §2 / §3): append the canonical result string via
            // the sanctioned journal path. A real failure to append is
            // surfaced via tracing; the next resume re-drives `run`.
            let terminal =
                JournalEntry::Terminal { result: workflow_result_label(&result).to_string() };
            if let Err(err) = journal.append(&workflow_id, &terminal).await {
                tracing::error!(
                    target: "overdrive::workflow_engine",
                    workflow_id = %workflow_id,
                    err = %err,
                    "failed to append workflow Terminal journal entry",
                );
            }
            // Terminal-result OBSERVATION row (slice-01 AC5, ADR-0064 §2):
            // write the terminal through the sanctioned `ObservationStore`
            // write path — NOT a direct bypass of the channels — keyed by
            // the instance `CorrelationKey` so the workflow-lifecycle
            // reconciler finds the result deterministically next tick and
            // converges the instance. A write failure is surfaced via
            // tracing; the next resume re-drives `run` and re-writes the
            // row (the key is stable, so the re-write is idempotent).
            let row = ObservationRow::WorkflowTerminal { correlation: correlation.clone(), result };
            if let Err(err) = obs.write(row).await {
                tracing::error!(
                    target: "overdrive::workflow_engine",
                    workflow_id = %workflow_id,
                    err = %err,
                    "failed to write workflow terminal observation row",
                );
            }
            // Drop the live-task entry AFTER the terminal row is written
            // (ADR-0064 §5). Ordering is load-bearing: a hydrate_actual
            // that observes `has_live_task = false` MUST also be able to
            // observe the terminal row, otherwise the reconciler would
            // see "running-in-intent, no live task, no terminal" and
            // re-emit StartWorkflow — re-running a workflow that already
            // completed. Removing the live entry only after the terminal
            // write closes that window.
            live_instances.lock().await.remove(&correlation);
        });
        drop(tasks);
        Ok(())
    }

    /// Await every spawned workflow task to completion. Test/inner-loop
    /// helper so an acceptance test can observe the durable terminal after
    /// the engine drove `run` off the shim.
    pub async fn join_all(&self) {
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
    }
}

/// The canonical terminal-result string a [`WorkflowResult`] maps to in
/// the journal `Terminal` entry. Stable labels so a resumed run reads back
/// the same terminal (the engine maps these back to a `WorkflowResult` in
/// later slices).
const fn workflow_result_label(result: &WorkflowResult) -> &'static str {
    match result {
        WorkflowResult::Success => "Success",
        WorkflowResult::Failed { .. } => "Failed",
        WorkflowResult::Cancelled => "Cancelled",
        // `WorkflowResult` is `#[non_exhaustive]`; future variants get a
        // label when they land. Until then an unknown variant maps to a
        // conservative "Unknown" so the match stays total.
        _ => "Unknown",
    }
}

/// The durable [`JournalCursor`] implementation over an
/// `Arc<dyn JournalStore>` + a per-instance replay buffer and cursor
/// (ADR-0064 §3). This is the concrete handle the [`WorkflowCtx`] consults
/// at every await-point — the control-plane-side I/O the core trait
/// declaration delegates to.
pub struct JournalCursorHandle {
    journal: Arc<dyn JournalStore>,
    workflow_id: WorkflowId,
    /// The entries loaded at (re)start. The cursor reads recorded results
    /// from this buffer on replay; the live path appends to the journal.
    replay_buffer: Vec<JournalEntry>,
    /// The current await-point index — advanced on every replay hit and
    /// every live record. Interior-mutable so `&self` ctx ops can move it.
    cursor: Mutex<usize>,
    /// The sender half of the engine's Action channel (→ Raft). The live
    /// `ctx.emit_action` path sends the typed Action here — the channel the
    /// production `spawn_workflow_emit_drain` task forwards into the SAME
    /// `action_shim` dispatch path a reconciler-emitted Action takes, NOT a
    /// direct `IntentStore` write (ADR-0064 §4; `development.md` Workflow
    /// contract rule 6).
    ///
    /// `None` for the 3-arg [`new`](Self::new) handle used by the DST
    /// replay-equivalence harness (which drives `ctx.run` / `ctx.sleep`
    /// only, never `ctx.emit_action`); the engine wires it via
    /// [`new_with_channels`](Self::new_with_channels).
    action_emit: Option<ActionEmitSender>,
    /// The `ObservationStore` the live `ctx.wait_for_signal` path reads
    /// typed signal rows from (in-process single-node delivery; #207
    /// cross-node-under-partition is OUT). The full crash-safe signal
    /// delivery lands in step 03-02; this slice records the
    /// `SignalAwaited` / `SignalSeen` entries and reads the row surface.
    ///
    /// `None` for the 3-arg [`new`](Self::new) handle (the DST harness
    /// drives no `ctx.wait_for_signal`); the engine wires it via
    /// [`new_with_channels`](Self::new_with_channels).
    obs: Option<Arc<dyn ObservationStore>>,
}

impl JournalCursorHandle {
    /// Construct a handle over `journal` for `workflow_id`, seeded with the
    /// `replay_buffer` loaded at (re)start, cursor at step 0, with NO
    /// Action channel and NO signal surface wired.
    ///
    /// This is the handle the DST replay-equivalence harness
    /// (`overdrive-sim`) constructs — it drives `ctx.run` / `ctx.sleep`
    /// only, never `ctx.emit_action` / `ctx.wait_for_signal`. A workflow
    /// that emits / waits-for-signal against this handle gets the
    /// always-live degenerate behaviour (the emit is dropped, the signal
    /// resolves empty) exactly like [`AlwaysLiveCursor`]. The engine wires
    /// the real channels via [`new_with_channels`](Self::new_with_channels).
    #[must_use]
    pub fn new(
        journal: Arc<dyn JournalStore>,
        workflow_id: WorkflowId,
        replay_buffer: Vec<JournalEntry>,
    ) -> Self {
        Self {
            journal,
            workflow_id,
            replay_buffer,
            cursor: Mutex::new(0),
            action_emit: None,
            obs: None,
        }
    }

    /// Construct a handle with the engine's Action-channel sender (the live
    /// `ctx.emit_action` path) and the `ObservationStore` (the live
    /// `ctx.wait_for_signal` path) wired in addition to the journal +
    /// replay buffer. The engine uses this for every live instance so the
    /// emit reaches the Action channel (→ Raft) and the signal read reaches
    /// the observation surface (ADR-0064 §4).
    #[must_use]
    pub fn new_with_channels(
        journal: Arc<dyn JournalStore>,
        workflow_id: WorkflowId,
        replay_buffer: Vec<JournalEntry>,
        action_emit: ActionEmitSender,
        obs: Arc<dyn ObservationStore>,
    ) -> Self {
        Self {
            journal,
            workflow_id,
            replay_buffer,
            cursor: Mutex::new(0),
            action_emit: Some(action_emit),
            obs: Some(obs),
        }
    }

    /// Durably append a live-path await-point `entry` and advance the held
    /// cursor — the append + fsync + advance tail every `record_*` live
    /// path shares (ADR-0063 §4 fsync-then-suspend). On a durable-append
    /// failure the cursor does NOT advance (the engine must not continue
    /// against an unjournaled effect) and the error surfaces as
    /// [`WorkflowCtxError::JournalRecord`]. The caller holds `cursor` (the
    /// step index is `*cursor` at call time, already baked into `entry`),
    /// so the whole record stays inside the caller's lock window.
    async fn append_then_advance(
        &self,
        cursor: &mut usize,
        entry: &JournalEntry,
    ) -> Result<(), WorkflowCtxError> {
        self.journal
            .append(&self.workflow_id, entry)
            .await
            .map_err(|err| WorkflowCtxError::JournalRecord { message: err.to_string() })?;
        *cursor += 1;
        Ok(())
    }
}

#[async_trait]
impl JournalCursor for JournalCursorHandle {
    async fn replay_run(&self, name: &str) -> Result<Option<Vec<u8>>, WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // Replay only while the cursor is within the loaded run AND the
        // entry at the cursor is a RunResult. (`Started` / `Terminal`
        // entries are not ctx.run await-points; slice 01 records only
        // RunResult entries between Started and Terminal, but guarding on
        // the variant keeps the cursor honest if a future slice
        // interleaves other await entries.) A cursor past the buffer (or
        // at a non-run entry) is the live path → `Ok(None)`.
        let Some(JournalEntry::RunResult { name: recorded_name, result_bytes, .. }) =
            self.replay_buffer.get(*cursor)
        else {
            drop(cursor);
            return Ok(None);
        };
        // Replay-determinism check (POSITIONAL identity; `name` is the
        // diagnostic + determinism guard). A recorded step whose name
        // diverges from the replaying body's name at this cursor is a
        // non-deterministic trajectory — fail closed (ADR-0064 §3). Do NOT
        // advance the cursor on a mismatch.
        if recorded_name != name {
            let expected = recorded_name.clone();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic { expected, actual: name.to_string() });
        }
        let bytes = result_bytes.clone();
        *cursor += 1;
        drop(cursor);
        Ok(Some(bytes))
    }

    async fn record_run(&self, name: &str, result_bytes: &[u8]) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        // The result digest is the content hash of the CBOR-encoded step
        // result — slice 01 records both the digest (replay-equivalence)
        // and the bytes (for byte-equal replay).
        let result_digest = ContentHash::of(result_bytes);
        let entry = JournalEntry::RunResult {
            step,
            name: name.to_string(),
            result_digest,
            result_bytes: result_bytes.to_vec(),
        };
        // Append + fsync BEFORE returning (ADR-0063 §4). On failure the
        // cursor does NOT advance — the engine must not continue against
        // an unjournaled effect.
        self.append_then_advance(&mut cursor, &entry).await?;
        drop(cursor);
        Ok(())
    }

    async fn replay_sleep(&self) -> Option<Duration> {
        let mut cursor = self.cursor.lock().await;
        // Replay only while the cursor is within the loaded run AND the
        // entry at the cursor is a SleepArmed. A cursor past the buffer
        // (or at a non-sleep entry) is the live path → `None`. The
        // recorded `deadline_unix` is the absolute deadline (an input);
        // the ctx recomputes the remaining wait against the live clock.
        let deadline = match self.replay_buffer.get(*cursor) {
            Some(JournalEntry::SleepArmed { deadline_unix, .. }) => {
                *cursor += 1;
                Some(*deadline_unix)
            }
            _ => None,
        };
        drop(cursor);
        deadline
    }

    async fn record_sleep_armed(&self, deadline_unix: Duration) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        // Record the ABSOLUTE deadline (an input), never a remaining
        // cache (`development.md` § "Persist inputs, not derived state").
        let entry = JournalEntry::SleepArmed { step, deadline_unix };
        // Append + fsync BEFORE returning (ADR-0063 §4, fsync-then-park).
        // On failure the cursor does NOT advance — the engine must not
        // park against an unjournaled sleep.
        self.append_then_advance(&mut cursor, &entry).await?;
        drop(cursor);
        Ok(())
    }

    async fn replay_signal(&self, _signal_key: &SignalKey) -> Option<SignalValue> {
        let mut cursor = self.cursor.lock().await;
        // A `ctx.wait_for_signal` records a PAIR of entries at distinct
        // cursor positions: `SignalAwaited` (when blocking begins) then
        // `SignalSeen { value }` (when the signal is observed satisfied).
        // On replay the cursor points at the `SignalAwaited`:
        //
        // - **Completed wait** — `SignalAwaited` followed by `SignalSeen`:
        //   a replay HIT. Return the recorded value WITHOUT re-reading the
        //   surface and advance the cursor PAST BOTH entries (the live run
        //   received this exact value; ADR-0064 §4). [S-WP-03-02]
        // - **Crashed while blocked** — `SignalAwaited` with NO following
        //   `SignalSeen`: NOT a replay hit. Return None so the live path
        //   re-blocks on the SAME signal; `record_signal_awaited` then
        //   advances past the lone `SignalAwaited`. [S-WP-03-01]
        // - **Live / non-signal entry** — None (the live path arms a fresh
        //   wait).
        //
        // Identity is POSITIONAL — `signal_key` is carried for diagnostics;
        // the cursor index is the identity.
        if !matches!(self.replay_buffer.get(*cursor), Some(JournalEntry::SignalAwaited { .. })) {
            drop(cursor);
            return None;
        }
        let Some(JournalEntry::SignalSeen { value, .. }) = self.replay_buffer.get(*cursor + 1)
        else {
            // SignalAwaited with no following SignalSeen — crashed while
            // blocked. NOT a replay hit; re-block on the live path.
            drop(cursor);
            return None;
        };
        let value = value.clone();
        // Advance past BOTH the SignalAwaited and the SignalSeen.
        *cursor += 2;
        drop(cursor);
        Some(value)
    }

    async fn record_signal_awaited(&self, signal_key: &SignalKey) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // Crash-while-blocked replay: a SignalAwaited is ALREADY recorded
        // at the cursor (the prior run crashed while blocked, recording
        // SignalAwaited but never SignalSeen — replay_signal returned None
        // because there is no following SignalSeen). Do NOT append a
        // duplicate — advance PAST the recorded SignalAwaited and re-enter
        // the live block on the SAME key. This is the load-bearing
        // crash-safety case (S-WP-03-01). POSITIONAL identity — `signal_key`
        // is carried for diagnostics; the cursor index is the identity.
        if matches!(self.replay_buffer.get(*cursor), Some(JournalEntry::SignalAwaited { .. })) {
            *cursor += 1;
            drop(cursor);
            return Ok(());
        }
        // Live path — record the SignalAwaited armed entry (an input: the
        // key the body blocked on) durably before the ctx begins blocking
        // (ADR-0063 §4 fsync-then-suspend).
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        let awaited = JournalEntry::SignalAwaited { step, signal_key: signal_key.clone() };
        self.append_then_advance(&mut cursor, &awaited).await?;
        drop(cursor);
        Ok(())
    }

    async fn poll_signal(
        &self,
        signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError> {
        // Engine-internal block check — read the typed signal row from the
        // ObservationStore signal surface (in-process single-node delivery;
        // #207 cross-node-under-partition is OUT). Does NOT journal: this is
        // the engine's blocking poll, not a workflow await-point. A missing
        // row is `Ok(None)` (still blocked); a present row is its value. A
        // surface READ failure is surfaced as `Signal` (distinct from
        // "absent"). A handle with no obs wired (the 3-arg DST-harness
        // `new`) has no signal surface, so resolves to the empty value
        // (present, no payload) — degenerate always-live behaviour, never
        // reached by a signal-blocking workflow under the engine.
        let Some(obs) = self.obs.as_ref() else {
            return Ok(Some(SignalValue::empty()));
        };
        obs.workflow_signal(signal_key)
            .await
            .map_err(|err| WorkflowCtxError::Signal { message: err.to_string() })
    }

    async fn record_signal_seen(
        &self,
        signal_key: &SignalKey,
        value: &SignalValue,
    ) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // Record SignalSeen { value } durably at the NEXT cursor position
        // (ADR-0063 §4): the value_digest is the content digest of the
        // observed value's bytes (an input); the value itself is carried so
        // a resumed run replays it without re-reading the surface. Recorded
        // at a DISTINCT cursor position from SignalAwaited, so a crash
        // BETWEEN them leaves SignalAwaited with no SignalSeen — the
        // re-block-on-resume shape.
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        let value_digest = ContentHash::of(value.as_str().as_bytes());
        let seen = JournalEntry::SignalSeen {
            step,
            signal_key: signal_key.clone(),
            value_digest,
            value: value.clone(),
        };
        self.append_then_advance(&mut cursor, &seen).await?;
        drop(cursor);
        Ok(())
    }

    async fn replay_emit(&self) -> bool {
        let mut cursor = self.cursor.lock().await;
        // A replay hit requires a recorded ActionEmitted at the cursor: the
        // Action was already sent on a prior run, so it is NOT re-sent
        // (exactly one cluster mutation across a crash — ADR-0064 §4).
        let is_replay =
            matches!(self.replay_buffer.get(*cursor), Some(JournalEntry::ActionEmitted { .. }));
        if is_replay {
            *cursor += 1;
        }
        drop(cursor);
        is_replay
    }

    async fn emit_action(&self, action: Action) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // action_digest is the content digest of the emitted Action's
        // canonical inputs (deterministic over the Action's Debug form —
        // the enum derives only Debug/Clone/Eq, no Serialize; the Debug
        // form is a stable canonical projection of the inputs). Per
        // `development.md` § "Persist inputs, not derived state".
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        let action_digest = ContentHash::of(format!("{action:?}").as_bytes());
        // Send the typed Action on the Action channel (→ Raft) — the
        // channel the production `spawn_workflow_emit_drain` task forwards
        // into the SAME `action_shim` dispatch path a reconciler-emitted
        // Action takes, NEVER a direct
        // IntentStore write. The send is BEFORE the durable record so the
        // ActionEmitted entry implies the Action reached the channel. A
        // handle with no channel wired (the 3-arg DST-harness `new`) drops
        // the emit — degenerate always-live behaviour, never reached by an
        // emitting workflow under the engine.
        if let Some(sender) = &self.action_emit {
            sender
                .send(action)
                .map_err(|err| WorkflowCtxError::ActionChannel { message: err.to_string() })?;
        }
        // Record ActionEmitted durably before returning (ADR-0063 §4): a
        // resumed run sees this entry and does NOT re-send the Action.
        let entry = JournalEntry::ActionEmitted { step, action_digest };
        self.append_then_advance(&mut cursor, &entry).await?;
        drop(cursor);
        Ok(())
    }
}

// Unit tests for the replay cursor live in
// `tests/acceptance/workflow_engine_replay_cursor.rs` rather than an
// in-`src/` `#[cfg(test)] mod tests`. They exercise the cursor through
// real `Sim*` adapters from `overdrive-sim` (a dev-dependency); a
// `src/`-side unit test cannot use those, because `overdrive-sim`
// depends on `overdrive-control-plane` and the lib-test build would see
// `SimJournalStore` implementing a *separately-compiled*
// `JournalStore` (the dev-dep cycle), so `Arc<SimJournalStore> as
// Arc<dyn JournalStore>` fails to unify. The `tests/` target compiles
// against the published lib + dev-deps, where the trait identities match.
