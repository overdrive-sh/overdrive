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
//! `ctx.call` is check-then-record:
//!
//! - **Replay (cursor < buffer length):** the handle returns the recorded
//!   [`CallResponse`] WITHOUT firing the transport effect — the
//!   exactly-once guarantee on resume (K1). The cursor advances.
//! - **Live (cursor == buffer length):** the handle returns `None`; the
//!   ctx fires the real effect, then the handle appends a
//!   [`JournalEntry::CallResult`] with fsync BEFORE returning (ADR-0063
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
use tokio::task::JoinSet;

use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    CallResponse, JournalCursor, Workflow, WorkflowCtx, WorkflowCtxError, WorkflowName,
    WorkflowResult, WorkflowSpec,
};

use crate::journal::{JournalEntry, JournalStore, WorkflowId};

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
        Self {
            journal,
            clock,
            transport,
            entropy,
            obs,
            registry: Arc::new(registry),
            tasks: Mutex::new(JoinSet::new()),
            live_instances: Arc::new(Mutex::new(BTreeSet::new())),
        }
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

        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
            Arc::clone(&self.journal),
            workflow_id.clone(),
            replay_buffer,
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
}

impl JournalCursorHandle {
    /// Construct a handle over `journal` for `workflow_id`, seeded with the
    /// `replay_buffer` loaded at (re)start, cursor at step 0.
    #[must_use]
    pub fn new(
        journal: Arc<dyn JournalStore>,
        workflow_id: WorkflowId,
        replay_buffer: Vec<JournalEntry>,
    ) -> Self {
        Self { journal, workflow_id, replay_buffer, cursor: Mutex::new(0) }
    }
}

#[async_trait]
impl JournalCursor for JournalCursorHandle {
    async fn replay_call(&self, _correlation: &CorrelationKey) -> Option<CallResponse> {
        let mut cursor = self.cursor.lock().await;
        // Replay only while the cursor is within the loaded run AND the
        // entry at the cursor is a CallResult. (`Started` / `Terminal`
        // entries are not ctx.call await-points; slice 01 records only
        // CallResult entries between Started and Terminal, but guarding on
        // the variant keeps the cursor honest if a future slice
        // interleaves other await entries.) A cursor past the buffer (or
        // at a non-call entry) is the live path → `None`.
        let response = match self.replay_buffer.get(*cursor) {
            Some(JournalEntry::CallResult { bytes_sent, .. }) => {
                *cursor += 1;
                Some(CallResponse { bytes_sent: *bytes_sent })
            }
            _ => None,
        };
        drop(cursor);
        response
    }

    async fn record_call(
        &self,
        correlation: &CorrelationKey,
        response: &CallResponse,
    ) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        let step = u32::try_from(*cursor).unwrap_or(u32::MAX);
        // The response digest is the content hash of the observable
        // result — slice 01 records both the digest (replay-equivalence)
        // and the value (`bytes_sent`, for byte-equal replay).
        let response_digest = ContentHash::of(response.bytes_sent.to_le_bytes());
        let entry = JournalEntry::CallResult {
            step,
            correlation: correlation.as_str().to_string(),
            response_digest,
            bytes_sent: response.bytes_sent,
        };
        // Append + fsync BEFORE returning (ADR-0063 §4). On failure the
        // cursor does NOT advance — the engine must not continue against
        // an unjournaled effect.
        self.journal
            .append(&self.workflow_id, &entry)
            .await
            .map_err(|err| WorkflowCtxError::JournalRecord { message: err.to_string() })?;
        *cursor += 1;
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
        self.journal
            .append(&self.workflow_id, &entry)
            .await
            .map_err(|err| WorkflowCtxError::JournalRecord { message: err.to_string() })?;
        *cursor += 1;
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
