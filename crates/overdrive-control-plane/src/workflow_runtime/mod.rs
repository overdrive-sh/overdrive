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
//! # The replay cursor (ADR-0064 §3, the command/notification partition)
//!
//! On (re)start the engine `load_journal`s the instance's flat ordered
//! `Vec<LoadedEntry>` and constructs a [`JournalCursorHandle`], which
//! **partitions it ONCE at construction** (D2 / CA-5) into a positional
//! command walk (`Vec<JournalCommand>`) plus a `SignalKey`-correlated
//! notification lookup (`BTreeMap<SignalKey, JournalNotification>`). The
//! cursor walks the COMMANDS only, advancing by exactly 1 per replayed
//! command; notifications are resolved off the walk by key and never advance
//! the cursor. The retired `*cursor += 2` two-positional-entry signal walk is
//! gone. Each `ctx.run` durable step is check-then-record (the command-index
//! is the identity):
//!
//! - **Replay (command-cursor < command-walk length):** the handle returns
//!   the recorded CBOR result bytes WITHOUT polling the step's future — the
//!   exactly-once guarantee on the replay path (K1). The command-cursor
//!   advances by 1.
//! - **Live (command-cursor == command-walk length):** the handle returns
//!   `Ok(None)`; the ctx polls the step's future, then the handle appends a
//!   [`JournalCommand::RunResult`] (wrapped as a [`LoadedEntry::Command`])
//!   with fsync BEFORE returning (ADR-0063 §4 fsync-then-suspend) and
//!   advances the command-cursor by 1.
//!
//! A `ctx.wait_for_signal` resolves its `SignalSeen` by
//! `signal_notifications.get(signal_key)` — never by position; a
//! `SignalAwaited` command with no matching `SignalSeen` notification
//! re-blocks (the "crashed while blocked" shape, now structural).
//!
//! The engine drives the object-safe [`ErasedWorkflow::run_erased`] over
//! the start intent's opaque `input` bytes (the typed `Workflow` is
//! erased to it by the [`ErasedWorkflowAdapter`] in the registry). On a
//! terminal it projects the body's `Result<Vec<u8>, TerminalError>` to a
//! [`WorkflowStatus`] — `Ok(bytes)` → `Completed { output: bytes }`,
//! `Err(terminal)` → `Failed { terminal }` — and appends a
//! [`JournalCommand::Terminal`] recording that status (ADR-0065 §3), the
//! durable terminal surface for slice 01.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use parking_lot::Mutex as PlMutex;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    ErasedWorkflow, ErasedWorkflowAdapter, JournalCursor, SignalKey, SignalValue, TerminalError,
    Workflow, WorkflowCtx, WorkflowCtxError, WorkflowName, WorkflowStart, WorkflowStatus,
};

use crate::journal::{JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId};

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

/// A factory producing a fresh object-safe [`ErasedWorkflow`] on demand.
/// The engine resolves a [`WorkflowStart`]'s [`WorkflowName`] to one of
/// these and calls it to obtain a fresh erased instance to drive
/// (ADR-0065 §1).
///
/// The trait object is `ErasedWorkflow` (not [`Workflow`]) because
/// `Workflow`'s associated `Input` / `Output` make it not object-safe;
/// [`WorkflowRegistry::register`] wraps the author's TYPED workflow in an
/// [`ErasedWorkflowAdapter`] internally, so the author never writes the
/// erasure.
pub type WorkflowFactory = Box<dyn Fn() -> Box<dyn ErasedWorkflow> + Send + Sync>;

/// Maps a [`WorkflowName`] (the workflow *kind*) to its author-supplied
/// workflow factory. The composition root registers every first-party
/// workflow here at boot; the engine looks up `spec.name` on each
/// `StartWorkflow` and drives the resolved [`ErasedWorkflow`].
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

    /// Register a TYPED [`Workflow`] factory under `name`. A later
    /// `StartWorkflow` carrying a spec with this name drives a fresh
    /// instance from `factory`. Re-registering the same name replaces the
    /// prior factory.
    ///
    /// The caller hands a `Fn() -> W` for a concrete `W: Workflow`; this
    /// wraps each produced instance in an [`ErasedWorkflowAdapter`] so the
    /// engine drives the object-safe [`ErasedWorkflow`] surface (ADR-0065
    /// §1). The author NEVER writes the erasure — registering a typed
    /// workflow is the whole contract.
    pub fn register<W, F>(&mut self, name: WorkflowName, factory: F)
    where
        W: Workflow + 'static,
        F: Fn() -> W + Send + Sync + 'static,
    {
        self.factories.insert(name, Box::new(move || Box::new(ErasedWorkflowAdapter(factory()))));
    }

    /// Resolve a fresh [`ErasedWorkflow`] for `name`, or `None` if
    /// unregistered.
    #[must_use]
    pub fn resolve(&self, name: &WorkflowName) -> Option<Box<dyn ErasedWorkflow>> {
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
    /// `Arc<PlMutex<BTreeSet<..>>>` so the spawned task can drop its own
    /// entry on terminal without holding `&self`. A `parking_lot::Mutex`
    /// (not `tokio::sync::Mutex`) because the teardown is driven by a
    /// SYNC RAII drop guard ([`LiveInstanceGuard`]) whose `Drop` cannot
    /// `.await`; the set is point-accessed (insert / remove / clone) and
    /// never held across an `.await`, so a sync mutex is the correct fit
    /// (`development.md` § "Never hold a lock across `.await`"). `BTreeSet`
    /// for deterministic iteration per `.claude/rules/development.md`
    /// § "Ordered-collection choice".
    live_instances: Arc<PlMutex<BTreeSet<CorrelationKey>>>,
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
            live_instances: Arc::new(PlMutex::new(BTreeSet::new())),
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
    pub fn live_instances(&self) -> BTreeSet<CorrelationKey> {
        // `parking_lot::Mutex` — sync lock, no `.await`, so this is a plain
        // sync getter (it was `async` while the field was a
        // `tokio::sync::Mutex`).
        self.live_instances.lock().clone()
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
        spec: &WorkflowStart,
        correlation: &CorrelationKey,
        workflow_id: &WorkflowId,
    ) -> Result<(), WorkflowEngineError> {
        let workflow = self.registry.resolve(&spec.name).ok_or_else(|| {
            WorkflowEngineError::UnknownWorkflow { name: spec.name.as_str().to_string() }
        })?;

        let mut replay_buffer = self.journal.load_journal(workflow_id).await?;

        // TERMINAL SHORT-CIRCUIT (fix-workflow-terminal-redrive, RCA Option
        // 1). If the loaded run ALREADY holds a `JournalCommand::Terminal`,
        // the instance is COMPLETE. Without this guard, resume would:
        //   1. re-run the author body (it has no terminal awareness), AND
        //   2. append a SECOND `Terminal` — `JournalStore::append` is
        //      append-only with no dedup — so the GC-less journal grows
        //      unboundedly on every re-drive.
        // The re-drive is itself driven by a persistent terminal obs-write
        // failure: the in-memory `WorkflowTerminal` row is lost, the
        // workflow-lifecycle reconciler sees no terminal + no live task, and
        // re-emits `StartWorkflow` each tick.
        //
        // On the short-circuit we ONLY re-publish the terminal observation
        // row (idempotent under the instance `CorrelationKey`) from the
        // journal's FULL `WorkflowStatus` — losslessly, including a
        // `Failed`'s structured `TerminalError`. We do NOT write `Started`,
        // build the cursor, spawn the body, or insert into `live_instances`:
        // the instance is TERMINAL, not live, and the reconciler converges on
        // `terminal.is_some()` (never on `has_live_task` once a terminal
        // exists), so no live entry is needed and inserting one would leak.
        // The obs write carries the SAME non-fatal `tracing::error!`
        // discipline as the spawn-path terminal write — a failure is
        // surfaced and the cheap idempotent re-publish is retried next tick;
        // the journal is NOT touched (no append), so it halts at exactly one
        // `Terminal`.
        if let Some(status) = terminal_status(&replay_buffer) {
            let row = ObservationRow::WorkflowTerminal { correlation: correlation.clone(), status };
            if let Err(err) = self.obs.write(row).await {
                tracing::error!(
                    target: "overdrive::workflow_engine",
                    workflow_id = %workflow_id,
                    err = %err,
                    "failed to re-publish workflow terminal observation row on terminal short-circuit",
                );
            }
            return Ok(());
        }

        // CA-4 — write `Started` at command-index 0 on FIRST start; idempotent
        // on resume (ADR-0063 §2 / ADR-0064 §5). The instance has already
        // started iff its loaded run holds any command (on a genuine first
        // start the run is empty; on resume it already carries the `Started`
        // the first start wrote). We must NOT append a second `Started` (the
        // trap — a duplicated command-index-0 entry the positional cursor
        // would walk twice). The check is structural (any-command-present),
        // the same `WorkflowId` re-derived by `WorkflowId::for_correlation`
        // upstream targeting the same persisted run.
        if !run_has_started(&replay_buffer) {
            let (spec_digest, input_digest) = started_digests(spec);
            let started =
                LoadedEntry::Command(JournalCommand::Started { spec_digest, input_digest });
            // Append + fsync BEFORE building the cursor + spawning the author
            // body (ADR-0063 §4 fsync-then-suspend): a crash after this append
            // but before the spawn re-loads a run that already begins with
            // `Started` and resumes cleanly (the idempotent-resume path above).
            self.journal.append(workflow_id, &started).await?;
            // Reflect the just-appended entry in the in-memory replay buffer so
            // the cursor partitions a run that ALREADY begins with `Started` at
            // command-index 0 — the first author `await`-point then records at
            // command-index 1.
            replay_buffer.push(started);
        }

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
        // The opaque CBOR `input` bytes the erased body decodes into its
        // typed `Input` (ADR-0065 §1). Cloned into the spawned task; the
        // engine never interprets them — the `ErasedWorkflowAdapter` is the
        // sole decode site. On a malformed input the adapter returns
        // `Err(TerminalError::malformed_input)` and the typed body is never
        // entered (mapped to `Failed` like any other terminal failure).
        let input_bytes = spec.input.clone();

        // Mark this instance live BEFORE spawning so a hydrate_actual that
        // races the spawn sees the instance as running (has_live_task =
        // true) — the reconciler must NOT re-emit StartWorkflow for an
        // instance the engine is already driving (ADR-0064 §5).
        let live_instances = Arc::clone(&self.live_instances);
        live_instances.lock().insert(correlation.clone());

        // The RAII teardown guard. Its `Drop` removes the correlation from
        // `live_instances` UNCONDITIONALLY — even if the terminal-write code
        // below panics (a panic in `journal.append` / `obs.write`) the guard
        // still fires on unwind, closing the leak that stranded a panicked
        // instance. The guard is MOVED into the async block and drops at the
        // end of it, AFTER the terminal write, preserving the load-bearing
        // terminal-then-remove ordering (see the comment at the tail of the
        // spawned block).
        let teardown =
            LiveInstanceGuard { set: Arc::clone(&live_instances), key: correlation.clone() };

        // Spawn the author's async body as a tracked task (ADR-0064 §5 —
        // the engine owns a tokio task set, the same way the reconciler
        // runtime owns its tick task).
        let mut tasks = self.tasks.lock().await;
        tasks.spawn(async move {
            // Hold the teardown guard for the whole task body. `_teardown`
            // (leading underscore) keeps it alive until end-of-scope without
            // an "unused" warning; its `Drop` does the live-instance removal.
            let _teardown = teardown;
            // Drive the object-safe erased body over the opaque input bytes
            // and PROJECT the outcome to a `WorkflowStatus` (ADR-0065 §3):
            //   - `Ok(bytes)`            → `Completed { output: bytes }`
            //   - `Err(terminal)`        → `Failed { terminal }`  (the body's
            //                              authored failure OR the adapter's
            //                              MalformedInput / OutputEncode)
            //   - panic (catch_unwind)   → `Failed { TerminalError::explicit }`
            //
            // Contain a panic in the UNTRUSTED author `run` future. Without
            // this, a panic unwinds past the terminal-write below and the
            // JoinSet absorbs it (production never `join_next`s) — the
            // instance is left with no terminal row and (pre-guard) a leaked
            // live entry: the workflow-lifecycle reconciler then sees
            // "running-in-intent, no terminal" and cannot converge. Mapping
            // the panic to `Failed` runs the EXISTING terminal-write path, so
            // the reconciler converges. The detail is derived ONLY from the
            // deterministic downcast payload (the &str / String panic message,
            // NEVER the address-bearing raw box) so the durable terminal stays
            // byte-stable across runs — closing the ADR-0064 §3 hazard.
            let run = AssertUnwindSafe(workflow.run_erased(&ctx, &input_bytes)).catch_unwind();
            let status = match run.await {
                Ok(Ok(output)) => WorkflowStatus::Completed { output },
                Ok(Err(terminal)) => WorkflowStatus::Failed { terminal },
                Err(panic) => {
                    let detail = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "workflow panicked".to_string());
                    tracing::error!(
                        target: "overdrive::workflow_engine",
                        workflow_id = %workflow_id,
                        detail = %detail,
                        "workflow run panicked; converging instance to Failed terminal",
                    );
                    WorkflowStatus::Failed { terminal: TerminalError::explicit(&detail) }
                }
            };
            // Durable terminal record (slice-01 terminal surface,
            // ADR-0064 §2 / §3): append the FULL `WorkflowStatus` via the
            // sanctioned journal path — not a lossy label — so a resumed
            // run reads back the exact terminal (including a `Failed`'s
            // structured `TerminalError`) and the start-time short-circuit can
            // re-publish the terminal observation row losslessly without
            // re-running the body. A real failure to append is surfaced via
            // tracing.
            let terminal =
                LoadedEntry::Command(JournalCommand::Terminal { status: status.clone() });
            if let Err(err) = journal.append(&workflow_id, &terminal).await {
                tracing::error!(
                    target: "overdrive::workflow_engine",
                    workflow_id = %workflow_id,
                    err = %err,
                    "failed to append workflow Terminal journal entry",
                );
            }
            // Terminal-status OBSERVATION row (slice-01 AC5, ADR-0064 §2):
            // write the terminal through the sanctioned `ObservationStore`
            // write path — NOT a direct bypass of the channels — keyed by
            // the instance `CorrelationKey` so the workflow-lifecycle
            // reconciler finds the status deterministically next tick and
            // converges the instance. A write failure is surfaced via
            // tracing; the next resume re-drives `run` and re-writes the
            // row (the key is stable, so the re-write is idempotent).
            let row = ObservationRow::WorkflowTerminal { correlation: correlation.clone(), status };
            if let Err(err) = obs.write(row).await {
                tracing::error!(
                    target: "overdrive::workflow_engine",
                    workflow_id = %workflow_id,
                    err = %err,
                    "failed to write workflow terminal observation row",
                );
            }
            // The live-task entry is dropped AFTER the terminal row is
            // written (ADR-0064 §5) — NOT by an explicit `remove` here, but
            // by `_teardown`'s `Drop` at end-of-scope, which is reached only
            // after both terminal writes above. Ordering is load-bearing: a
            // hydrate_actual that observes `has_live_task = false` MUST also
            // be able to observe the terminal row, otherwise the reconciler
            // would see "running-in-intent, no live task, no terminal" and
            // re-emit StartWorkflow — re-running a workflow that already
            // completed. The guard dropping last closes that window AND
            // guarantees the entry is removed even if a terminal write above
            // panics (defense-in-depth backstop to the `catch_unwind`).
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

/// RAII teardown guard for a live workflow instance. Its `Drop` removes
/// the instance's [`CorrelationKey`] from the engine's `live_instances`
/// set UNCONDITIONALLY — on the normal terminal path AND on an unwind
/// through the spawned task body (e.g. a panic in the terminal-write code
/// that the `catch_unwind` around `run` does not cover).
///
/// This is the defense-in-depth half of the panic-containment fix: the
/// `catch_unwind` around the author `run` future converts a `run` panic to
/// a `Failed` terminal so the existing terminal-write path runs; this guard
/// guarantees the live-instance entry is torn down even if THAT path
/// panics. A stranded live entry is the bug this closes — it makes the
/// workflow-lifecycle reconciler's `hydrate_actual` derive
/// `has_live_task = true` forever, suppressing re-emit.
///
/// `Drop` is sync and acquires the `parking_lot::Mutex` directly (no
/// `.await`), which is why `live_instances` is a `parking_lot::Mutex`. A
/// double-remove (guard fires after some other removal) is a harmless
/// `BTreeSet` no-op.
struct LiveInstanceGuard {
    set: Arc<PlMutex<BTreeSet<CorrelationKey>>>,
    key: CorrelationKey,
}

impl Drop for LiveInstanceGuard {
    fn drop(&mut self) {
        self.set.lock().remove(&self.key);
    }
}

/// Compute the input-derived `Started` digests (`spec_digest`,
/// `input_digest`) for `spec` — the INPUTS the command-index-0 `Started`
/// entry records on first start (ADR-0063 §2; CA-4). Per
/// `.claude/rules/development.md` § "Persist inputs, not derived state":
/// these are content hashes over the workflow-kind identity and the start
/// input, NOT a pre-computed cache.
///
/// Slice 01's [`WorkflowStart`] carries only the workflow `name` (its
/// identity); the start input is the spec's identity bytes until a later
/// slice grows the spec with start parameters (the spec evolves additively,
/// `overdrive-core/src/workflow/mod.rs`). Both digests are derived here so
/// the engine — not the test — owns the derivation, matching the journal
/// characterization test's `ContentHash::of(spec.name…)` choice.
fn started_digests(spec: &WorkflowStart) -> (ContentHash, ContentHash) {
    let spec_digest = ContentHash::of(spec.name.as_str().as_bytes());
    // The start input. Slice 01 has no separate start-parameter surface on
    // `WorkflowStart`, so the input identity is the spec name bytes — the
    // same input the journal-store characterization test records via
    // `ProvisionRecord::PAYLOAD` (the workflow-kind constant).
    //
    // TODO(#217): when `WorkflowStart` grows a start-parameter surface,
    // hash the serialised parameter bytes here instead of the name bytes so
    // `spec_digest` and `input_digest` diverge as intended. The compiler will
    // NOT flag this site when the spec gains fields (both digests still
    // type-check against `spec.name`); this marker is the only obligation
    // record that the input digest must switch off the name bytes.
    let input_digest = ContentHash::of(spec.name.as_str().as_bytes());
    (spec_digest, input_digest)
}

/// Whether the loaded run has ALREADY started — the structural idempotency
/// guard for resume (CA-4). An instance has started iff its persisted run
/// holds at least one `LoadedEntry::Command`: on a genuine FIRST start the
/// run is empty, so the engine writes `Started` at command-index 0; on any
/// resume the run already carries the `Started` the first start wrote (plus
/// whatever await-points landed before the crash), so the engine must NOT
/// append a second `Started`.
///
/// The guard keys on "any command present," not "first command is
/// `Started`," deliberately: a run that already carries commands — whether
/// it begins with `Started` or with a mid-flight await-point — has started,
/// and appending a fresh `Started` at the END (a non-zero append position)
/// would corrupt the positional command walk (the cursor would later trip
/// the Layer-1 determinism gate on the stray trailing `Started`). The
/// trap (CA-4) is exactly "write `Started` once, on first start" — the
/// presence of any prior command is the structural proof the instance is
/// past first start.
fn run_has_started(loaded: &[LoadedEntry]) -> bool {
    loaded.iter().any(|entry| matches!(entry, LoadedEntry::Command(_)))
}

/// The full [`WorkflowStatus`] from a `JournalCommand::Terminal` in the
/// loaded run, if the instance has already reached a durable terminal.
///
/// This is the terminal-short-circuit guard for resume
/// (`docs/feature/fix-workflow-terminal-redrive/deliver/rca.md`, Option 1):
/// a run that already carries a `Terminal` command is COMPLETE — re-driving
/// it would re-run the author body AND (since `JournalStore::append` is
/// append-only, no dedup) append a SECOND `Terminal`, growing the
/// GC-less journal unboundedly. `start` short-circuits when this returns
/// `Some`. Returns the cloned full status so the obs terminal row can be
/// re-published losslessly (including a `Failed`'s structured
/// `TerminalError`) without reconstructing it from a lossy label.
fn terminal_status(loaded: &[LoadedEntry]) -> Option<WorkflowStatus> {
    loaded.iter().find_map(|entry| match entry {
        LoadedEntry::Command(JournalCommand::Terminal { status }) => Some(status.clone()),
        _ => None,
    })
}

/// The deterministic, address-free kind-label of a [`JournalCommand`] —
/// the stable `expected`/`actual` payload the fail-closed determinism gate
/// (D4, ADR-0064 §3) reports on a Layer-1 type-at-index mismatch.
///
/// A stable variant-kind label (an `as_str()`-style projection per
/// `.claude/rules/development.md` § "Label enums own their string
/// representation"), NEVER an address-bearing `Debug` of the whole entry: a
/// `WorkflowCtxError::NonDeterministic` carrying `{:?}` of the recorded
/// command would embed pointers / field values that vary across runs and
/// seeds, breaking the byte-identical-trajectory property DST replay relies
/// on (`.claude/rules/testing.md` § "Tier 1"). The label lives here, at the
/// cursor (its sole consumer), rather than on the `JournalCommand` enum:
/// the enum is defined in `journal/mod.rs`, outside this step's edit
/// boundary, and the cursor is the only site that needs the deterministic
/// label.
const fn command_kind(command: &JournalCommand) -> &'static str {
    match command {
        JournalCommand::Started { .. } => "Started",
        JournalCommand::RunResult { .. } => "RunResult",
        JournalCommand::SleepArmed { .. } => "SleepArmed",
        JournalCommand::SignalAwaited { .. } => "SignalAwaited",
        JournalCommand::ActionEmitted { .. } => "ActionEmitted",
        JournalCommand::Terminal { .. } => "Terminal",
    }
}

/// Partition the flat loaded run (the dumb-store ordered
/// `Vec<LoadedEntry>`) into the positional command walk plus the
/// `SignalKey`-correlated notification lookup — the D2 partition, performed
/// ONCE at [`JournalCursorHandle`] construction (ADR-0064 §3, CA-5).
///
/// Every [`LoadedEntry::Command`] lands in the returned `Vec<JournalCommand>`
/// in append order (its index there is its replay command-index, D3); every
/// [`LoadedEntry::Notification`]'s `SignalSeen` lands in the returned
/// `BTreeMap` keyed by its `SignalKey`. The store never classifies — this is
/// the cursor's job (D2). `BTreeMap`, not `HashMap`, per
/// `.claude/rules/development.md` § "Ordered-collection choice" (DST
/// determinism).
///
/// A duplicate `SignalSeen` for the same key (not expected for the
/// single-node Phase-1 one-notification model — D6) keeps the LAST observed
/// value via `BTreeMap::insert`'s overwrite semantics; the append-order
/// last write is the most recent observation.
fn partition_loaded_run(
    loaded: Vec<LoadedEntry>,
) -> (Vec<JournalCommand>, BTreeMap<SignalKey, JournalNotification>) {
    let mut commands = Vec::new();
    let mut notifications = BTreeMap::new();
    for entry in loaded {
        match entry {
            LoadedEntry::Command(command) => commands.push(command),
            LoadedEntry::Notification(notification) => {
                let JournalNotification::SignalSeen { ref signal_key, .. } = notification;
                notifications.insert(signal_key.clone(), notification);
            }
        }
    }
    (commands, notifications)
}

/// The initial command-cursor position for a partitioned command walk —
/// the command-index of the FIRST author await-point (CA-4, ADR-0064 §3).
///
/// `Started` is a real command-index-0 entry the engine writes on first
/// start, but it is **structural**, not an author await-op: no `ctx.run` /
/// `ctx.sleep` / `ctx.wait_for_signal` / `ctx.emit_action` maps to it. The
/// positional cursor must therefore begin PAST it — at command-index 1 —
/// so the first author await-point replays against command-index 1, not
/// against the `Started` entry (which would trip the Layer-1 type-at-index
/// determinism gate, since the author op's expected kind is never
/// `Started`).
///
/// A run that does NOT begin with `Started` (the DST replay-equivalence
/// harness's 3-arg [`JournalCursorHandle::new`] constructs runs of bare
/// `RunResult` / `SleepArmed` commands) starts at command-index 0 —
/// backward-compatible with every pre-CA-4 cursor consumer.
fn initial_command_cursor(commands: &[JournalCommand]) -> usize {
    usize::from(matches!(commands.first(), Some(JournalCommand::Started { .. })))
}

/// The durable [`JournalCursor`] implementation over an
/// `Arc<dyn JournalStore>` + a per-instance partitioned run and cursor
/// (ADR-0064 §3). This is the concrete handle the [`WorkflowCtx`] consults
/// at every await-point — the control-plane-side I/O the core trait
/// declaration delegates to.
///
/// The loaded run is partitioned ONCE at construction (via
/// [`partition_loaded_run`]) into a positional command walk
/// (`replay_commands`) plus a `SignalKey`-correlated notification lookup
/// (`signal_notifications`); the cursor walks commands ONLY and advances by
/// exactly 1 per replayed command. The retired `*cursor += 2`
/// two-positional-entry signal walk is gone — a `SignalSeen` is resolved by
/// key, off the walk (D2 / CA-5).
pub struct JournalCursorHandle {
    journal: Arc<dyn JournalStore>,
    workflow_id: WorkflowId,
    /// The replayable, **cursor-advancing** commands of the loaded run, in
    /// append order — the positional command walk (D2 / ADR-0064 §3,
    /// CA-5). Partitioned ONCE at construction from the flat
    /// `Vec<LoadedEntry>` the store returns: every `LoadedEntry::Command`
    /// lands here in order. The cursor walks THIS vector only and advances
    /// by exactly 1 per replayed command; notifications never advance it.
    replay_commands: Vec<JournalCommand>,
    /// The `SignalKey`-correlated notifications of the loaded run — the
    /// off-the-walk lookup map (D2 / D6 / ADR-0064 §4, CA-5). Partitioned
    /// ONCE at construction: every `LoadedEntry::Notification`'s
    /// `SignalSeen` lands here keyed by its `SignalKey`. `replay_signal`
    /// resolves a satisfied wait by `signal_notifications.get(signal_key)`
    /// — never by position; the retired `*cursor += 2` positional signal
    /// walk is gone.
    ///
    /// `BTreeMap`, not `HashMap`, per `.claude/rules/development.md`
    /// § "Ordered-collection choice" — the map is observed by the DST
    /// `replay_equivalence_provision_record` invariant (step 01-06) and
    /// must iterate deterministically across seeds.
    signal_notifications: BTreeMap<SignalKey, JournalNotification>,
    /// The current **command**-cursor index into [`replay_commands`] —
    /// advanced on every command replay hit and every live command record,
    /// by exactly 1. A notification record (`record_signal_seen`) does NOT
    /// advance it. Interior-mutable so `&self` ctx ops can move it.
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
        replay_buffer: Vec<LoadedEntry>,
    ) -> Self {
        let (replay_commands, signal_notifications) = partition_loaded_run(replay_buffer);
        let initial = initial_command_cursor(&replay_commands);
        Self {
            journal,
            workflow_id,
            replay_commands,
            signal_notifications,
            cursor: Mutex::new(initial),
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
        replay_buffer: Vec<LoadedEntry>,
        action_emit: ActionEmitSender,
        obs: Arc<dyn ObservationStore>,
    ) -> Self {
        let (replay_commands, signal_notifications) = partition_loaded_run(replay_buffer);
        let initial = initial_command_cursor(&replay_commands);
        Self {
            journal,
            workflow_id,
            replay_commands,
            signal_notifications,
            cursor: Mutex::new(initial),
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
        entry: &LoadedEntry,
    ) -> Result<(), WorkflowCtxError> {
        self.journal
            .append(&self.workflow_id, entry)
            .await
            .map_err(|err| WorkflowCtxError::JournalRecord { message: err.to_string() })?;
        // mutants: skip — `+= 1` -> `*= 1` is an equivalent mutant here.
        // `append_then_advance` is ONLY ever called from a live `record_*`
        // path, which the ctx reaches solely after a `replay_*` returned the
        // live sentinel — i.e. with `*cursor == replay_commands.len()` (the
        // command-walk end). Past the walk every subsequent `replay_*` resolves
        // to the live sentinel regardless of whether the cursor sits at `len`
        // or `len + k`, so the post-advance value is unobservable and `*= 1`
        // (identity at the boundary) cannot diverge from `+= 1`. The `-= 1`
        // mutant IS caught (usize underflow panic on the first live record).
        *cursor += 1;
        Ok(())
    }
}

#[async_trait]
impl JournalCursor for JournalCursorHandle {
    async fn replay_run(&self, name: &str) -> Result<Option<Vec<u8>>, WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // A command-cursor PAST the loaded command walk is the genuine live
        // path → `Ok(None)`. (Only an out-of-bounds cursor is live; an
        // in-bounds foreign variant is Layer-1 divergence, handled below.)
        let Some(command) = self.replay_commands.get(*cursor) else {
            drop(cursor);
            return Ok(None);
        };
        // LAYER 1 — type-at-index fail-closed gate (D4, ADR-0064 §3,
        // Restate RT0016 shape). The await-op being replayed is `ctx.run`,
        // whose expected command kind is `RunResult`. A recorded command of
        // ANY OTHER kind at this cursor (a `SleepArmed`, `SignalAwaited`,
        // `ActionEmitted`, `Started`, or `Terminal`) is a divergent
        // trajectory: return `NonDeterministic`, do NOT advance the cursor,
        // and do NOT fall through to the live path. This CLOSES the trap's
        // twin — the former `let ... else { Ok(None) }` that silently
        // fell to live on a variant mismatch, re-executing the effect.
        let JournalCommand::RunResult { name: recorded_name, result_bytes, .. } = command else {
            let expected = command_kind(command).to_string();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic { expected, actual: name.to_string() });
        };
        // LAYER 2 — name-within-`RunResult` fail-closed gate (D4). The
        // variant matches, but a recorded step whose name diverges from the
        // replaying body's `ctx.run` name at this cursor is still a
        // non-deterministic trajectory — fail closed. Do NOT advance the
        // cursor on a mismatch. (Identity is POSITIONAL; `name` is the
        // determinism guard, not the cursor identity.)
        if recorded_name != name {
            let expected = recorded_name.clone();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic { expected, actual: name.to_string() });
        }
        // LAYER 3 (content/digest comparison) is DEFERRED to
        // https://github.com/overdrive-sh/overdrive/issues/214 — slice 01 does
        // NOT compare `result_digest`/`result_bytes` against a re-derived
        // value at the cursor. Layers 1 + 2 are the determinism gate for this
        // step; the digest is recorded (for K4 replay-equivalence) but not
        // diffed here.
        let bytes = result_bytes.clone();
        *cursor += 1;
        drop(cursor);
        Ok(Some(bytes))
    }

    async fn record_run(&self, name: &str, result_bytes: &[u8]) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // The result digest is the content hash of the CBOR-encoded step
        // result — slice 01 records both the digest (replay-equivalence)
        // and the bytes (for byte-equal replay). No in-entry `step` —
        // identity is positional (D5).
        let result_digest = ContentHash::of(result_bytes);
        let entry = LoadedEntry::Command(JournalCommand::RunResult {
            name: name.to_string(),
            result_digest,
            result_bytes: result_bytes.to_vec(),
        });
        // Append + fsync BEFORE returning (ADR-0063 §4). On failure the
        // cursor does NOT advance — the engine must not continue against
        // an unjournaled effect.
        self.append_then_advance(&mut cursor, &entry).await?;
        drop(cursor);
        Ok(())
    }

    async fn replay_sleep(&self) -> Result<Option<Duration>, WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // A command-cursor PAST the loaded command walk is the genuine live
        // path → `Ok(None)`.
        let Some(command) = self.replay_commands.get(*cursor) else {
            drop(cursor);
            return Ok(None);
        };
        // LAYER 1 — type-at-index fail-closed gate (D4, ADR-0064 §3). The
        // await-op being replayed is `ctx.sleep`, whose expected command
        // kind is `SleepArmed`. A recorded command of any other kind at this
        // cursor is divergence: return `NonDeterministic`, do NOT advance,
        // do NOT fall through to live (the former `_ => None` arm silently
        // fell to live — that twin is now closed). The recorded
        // `deadline_unix` is the absolute deadline (an input); the ctx
        // recomputes the remaining wait against the live clock. Advance the
        // command-cursor by exactly 1 on a replay hit.
        let JournalCommand::SleepArmed { deadline_unix, .. } = command else {
            let expected = command_kind(command).to_string();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic {
                expected,
                actual: "SleepArmed".to_string(),
            });
        };
        let deadline = *deadline_unix;
        *cursor += 1;
        drop(cursor);
        Ok(Some(deadline))
    }

    async fn record_sleep_armed(&self, deadline_unix: Duration) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // Record the ABSOLUTE deadline (an input), never a remaining
        // cache (`development.md` § "Persist inputs, not derived state").
        // No in-entry `step` — identity is positional (D5).
        let entry = LoadedEntry::Command(JournalCommand::SleepArmed { deadline_unix });
        // Append + fsync BEFORE returning (ADR-0063 §4, fsync-then-park).
        // On failure the cursor does NOT advance — the engine must not
        // park against an unjournaled sleep.
        self.append_then_advance(&mut cursor, &entry).await?;
        drop(cursor);
        Ok(())
    }

    async fn replay_signal(
        &self,
        signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // The notification-lookup contract (D2 / D6 / CA-5). A
        // `ctx.wait_for_signal` records a `SignalAwaited` COMMAND (in the
        // command walk) and a `SignalSeen` NOTIFICATION (off the walk,
        // `SignalKey`-keyed). On replay the command-cursor points at the
        // `SignalAwaited` command; the `SignalSeen` is resolved by KEY
        // lookup, NEVER by position — the retired `*cursor += 2`
        // two-positional-entry walk is GONE.
        //
        // - **Live (cursor past the walk)** — `Ok(None)`: the live path arms
        //   a fresh wait.
        // - **Completed wait** — a `SignalAwaited` command at the cursor AND
        //   a matching `SignalSeen` notification in the lookup map: a replay
        //   HIT. Return the recorded value WITHOUT re-reading the surface and
        //   advance the command-cursor by EXACTLY 1 (past the `SignalAwaited`
        //   command only; the notification is off the walk and never advances
        //   the cursor; ADR-0064 §4). [S-WP-03-02]
        // - **Crashed while blocked** — a `SignalAwaited` command at the
        //   cursor with NO matching `SignalSeen` notification: NOT a replay
        //   hit. Return `Ok(None)` so the live path re-blocks on the SAME
        //   signal; `record_signal_awaited` then advances past the lone
        //   `SignalAwaited` command. [S-WP-03-01]
        let Some(command) = self.replay_commands.get(*cursor) else {
            drop(cursor);
            return Ok(None);
        };
        // LAYER 1 — type-at-index fail-closed gate (D4, ADR-0064 §3). The
        // await-op being replayed is `ctx.wait_for_signal`, whose expected
        // command kind is `SignalAwaited`. A recorded command of any other
        // kind at this cursor is divergence: return `NonDeterministic`, do
        // NOT advance, do NOT fall through to live (the former
        // `!matches!(..) { return None }` silently fell to live on a foreign
        // variant — that twin is now closed). NOTE: the
        // crashed-while-blocked case below (a `SignalAwaited` with no
        // matching notification) is NOT divergence — it is the
        // re-block-on-resume shape, which stays `Ok(None)`.
        let JournalCommand::SignalAwaited { signal_key: recorded_key } = command else {
            let expected = command_kind(command).to_string();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic {
                expected,
                actual: "SignalAwaited".to_string(),
            });
        };
        // LAYER 2 — key-within-`SignalAwaited` fail-closed gate (D4). The
        // variant matches, but a recorded `SignalAwaited` whose `signal_key`
        // diverges from the replaying body's `ctx.wait_for_signal` key at this
        // cursor is a non-deterministic trajectory — fail closed. Do NOT
        // advance the cursor on a mismatch. Mirrors the `RunResult` Layer-2
        // name check above. Without this, a key change at the same cursor
        // passes Layer 1, the notification lookup on the NEW key misses,
        // `replay_signal` returns `Ok(None)` as "crashed while blocked", and
        // `record_signal_awaited` silently consumes the recorded
        // `SignalAwaited{old}` with no `NonDeterministic`. (Identity is
        // POSITIONAL; the key is the determinism guard, not the cursor identity.)
        if recorded_key != signal_key {
            let expected = recorded_key.to_string();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic {
                expected,
                actual: signal_key.to_string(),
            });
        }
        // Correlated lookup — find the SignalSeen by its key, wherever it
        // landed in the interleaved on-disk stream (NOT at SignalAwaited+1).
        let Some(JournalNotification::SignalSeen { value, .. }) =
            self.signal_notifications.get(signal_key)
        else {
            // SignalAwaited command with no matching SignalSeen notification
            // — crashed while blocked. NOT a replay hit; re-block on the live
            // path. This is NOT a Layer-1 divergence (the variant matched).
            drop(cursor);
            return Ok(None);
        };
        let value = value.clone();
        // Advance past the SignalAwaited COMMAND by exactly 1 — the
        // notification is off the walk (it never advances the cursor).
        *cursor += 1;
        drop(cursor);
        Ok(Some(value))
    }

    async fn record_signal_awaited(&self, signal_key: &SignalKey) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // Crash-while-blocked replay: a SignalAwaited COMMAND is ALREADY at
        // the command-cursor (the prior run crashed while blocked, recording
        // the SignalAwaited command but never the SignalSeen notification —
        // replay_signal returned None because there is no matching SignalSeen
        // notification in the lookup map). Do NOT append a duplicate —
        // advance the command-cursor PAST the recorded SignalAwaited command
        // (by exactly 1) and re-enter the live block on the SAME key. This is
        // the load-bearing crash-safety case (S-WP-03-01).
        if let Some(JournalCommand::SignalAwaited { signal_key: recorded_key }) =
            self.replay_commands.get(*cursor)
        {
            // Crash-while-blocked: the recorded key MUST match the key the
            // body is re-blocking on. A divergent key is non-determinism —
            // already caught upstream by `replay_signal`'s Layer-2 gate, which
            // fails closed before this method is reached. This guard is
            // defense-in-depth: only advance past the recorded `SignalAwaited`
            // when the keys match; a mismatch fails closed rather than
            // silently consuming the recorded command.
            if recorded_key != signal_key {
                let expected = recorded_key.to_string();
                drop(cursor);
                return Err(WorkflowCtxError::NonDeterministic {
                    expected,
                    actual: signal_key.to_string(),
                });
            }
            *cursor += 1;
            drop(cursor);
            return Ok(());
        }
        // Live path — record the SignalAwaited armed command (an input: the
        // key the body blocked on) durably before the ctx begins blocking
        // (ADR-0063 §4 fsync-then-suspend). No in-entry `step` — identity
        // is positional (D5).
        let awaited =
            LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: signal_key.clone() });
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
        // Record SignalSeen { value } durably (ADR-0063 §4): the
        // value_digest is the content digest of the observed value's bytes
        // (an input); the value itself is carried so a resumed run replays it
        // by `SignalKey` lookup without re-reading the surface.
        //
        // SignalSeen is a NOTIFICATION (SignalKey-correlated), no in-entry
        // `step` (D1/D5). Per the notification-lookup contract (D2/D6) this
        // does NOT advance the command-cursor — a notification lives off the
        // positional command walk. The preceding SignalAwaited COMMAND (via
        // `record_signal_awaited`) already advanced the cursor; a crash AFTER
        // that advance but BEFORE this notification is durable leaves the
        // SignalAwaited command with no matching SignalSeen notification —
        // the re-block-on-resume shape. The append is therefore a plain
        // durable journal write with NO cursor mutation (the
        // `append_then_advance` helper is for commands only).
        let value_digest = ContentHash::of(value.as_str().as_bytes());
        let seen = LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: signal_key.clone(),
            value_digest,
            value: value.clone(),
        });
        self.journal
            .append(&self.workflow_id, &seen)
            .await
            .map_err(|err| WorkflowCtxError::JournalRecord { message: err.to_string() })?;
        Ok(())
    }

    async fn replay_emit(&self) -> Result<bool, WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // A command-cursor PAST the loaded command walk is the genuine live
        // path → `Ok(false)`: the live `emit_action` sends + records.
        let Some(command) = self.replay_commands.get(*cursor) else {
            drop(cursor);
            return Ok(false);
        };
        // LAYER 1 — type-at-index fail-closed gate (D4, ADR-0064 §3). The
        // await-op being replayed is `ctx.emit_action`, whose expected
        // command kind is `ActionEmitted`. A recorded command of any other
        // kind at this cursor is divergence: return `NonDeterministic`, do
        // NOT advance, do NOT fall through to live (the former
        // `matches!(..)`-then-`false` silently fell to live on a foreign
        // variant — that twin is now closed). A replay hit returns `Ok(true)`
        // — the Action was already sent on a prior run, so it is NOT re-sent
        // (exactly-once ON THE REPLAY PATH — ADR-0064 §4). The live path in
        // `emit_action` is at-least-once: a recorded ActionEmitted is what
        // makes resume idempotent, so a run that sent but failed to record it
        // (cursor past the walk → `Ok(false)` above) re-sends. Advance the
        // command-cursor by exactly 1 on a replay hit.
        let JournalCommand::ActionEmitted { .. } = command else {
            let expected = command_kind(command).to_string();
            drop(cursor);
            return Err(WorkflowCtxError::NonDeterministic {
                expected,
                actual: "ActionEmitted".to_string(),
            });
        };
        *cursor += 1;
        drop(cursor);
        Ok(true)
    }

    async fn emit_action(&self, action: Action) -> Result<(), WorkflowCtxError> {
        let mut cursor = self.cursor.lock().await;
        // action_digest is the content digest of the emitted Action's
        // canonical inputs (deterministic over the Action's Debug form —
        // the enum derives only Debug/Clone/Eq, no Serialize; the Debug
        // form is a stable canonical projection of the inputs). Per
        // `development.md` § "Persist inputs, not derived state".
        //
        // STABILITY — K4 replay-equivalence (load-bearing once Layer-3
        // digest comparison lands, #214): this digest is deterministic only
        // while `Action`'s Debug form is. Every collection-bearing `Action`
        // variant uses `BTreeMap`/`BTreeSet`, whose Debug iterates in `Ord`
        // order — never `HashMap`/`HashSet`, whose Debug order is
        // per-process-random and would make the same inputs hash differently
        // across runs. This precondition is mechanically enforced, not merely
        // convention: `Action` lives in `overdrive-core` (crate_class =
        // "core"), so a future variant introducing a `HashMap`/`HashSet`
        // fails the dst-lint gate at PR time (development.md §
        // "Ordered-collection choice") unless it carries a
        // `// dst-lint: hashmap-ok` waiver. The sharp hazard is therefore a
        // `hashmap-ok` waiver on an `Action` variant: it would pass the gate
        // while silently breaking this digest's cross-run stability. Do not
        // add one without first making the digest input canonical (e.g. an
        // explicit sorted projection of the variant's fields).
        let action_digest = ContentHash::of(format!("{action:?}").as_bytes());
        // Send the typed Action on the Action channel (→ Raft) — the
        // channel the production `spawn_workflow_emit_drain` task forwards
        // into the SAME `action_shim` dispatch path a reconciler-emitted
        // Action takes, NEVER a direct
        // IntentStore write. The send is BEFORE the durable record so the
        // ActionEmitted entry implies the Action reached the channel.
        //
        // SEND-BEFORE-RECORD ⇒ AT-LEAST-ONCE (deliberate). If the
        // `append_then_advance` below fails (or the process crashes) AFTER
        // this send but BEFORE ActionEmitted is durable, no ActionEmitted is
        // journaled at this cursor: a resume re-runs the live path and
        // re-sends. Exactly-once holds only on the replay path (`replay_emit`
        // returns true once ActionEmitted is recorded). This is the SAME
        // at-least-once window `WorkflowCtx::run` documents; safety against
        // the duplicate rests on the downstream `action_shim` dispatch being
        // idempotent. Do NOT "fix" this by recording before sending —
        // record-before-send loses the mutation SILENTLY on a crash between
        // the record and the send (strictly worse for a cluster mutation).
        //
        // A handle with no channel wired (the 3-arg DST-harness `new`) drops
        // the emit — degenerate always-live behaviour, never reached by an
        // emitting workflow under the engine.
        if let Some(sender) = &self.action_emit {
            sender
                .send(action)
                .map_err(|err| WorkflowCtxError::ActionChannel { message: err.to_string() })?;
        }
        // Record ActionEmitted durably before returning (ADR-0063 §4): a
        // resumed run sees this command and does NOT re-send the Action.
        // No in-entry `step` — identity is positional (D5).
        let entry = LoadedEntry::Command(JournalCommand::ActionEmitted { action_digest });
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
