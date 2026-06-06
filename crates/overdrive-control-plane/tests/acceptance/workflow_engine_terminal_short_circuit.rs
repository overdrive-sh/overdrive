//! Bug fix (`fix-workflow-terminal-redrive`) — `WorkflowEngine::start` must
//! SHORT-CIRCUIT when the loaded journal already holds a
//! `JournalCommand::Terminal`.
//!
//! # The bug
//!
//! `start` guards a duplicate `Started` (via `run_has_started`) but nothing
//! guards a pre-existing `Terminal`, and `JournalStore::append` is
//! append-only (no dedup). Under a PERSISTENT terminal observation-store
//! write failure, the in-memory `WorkflowTerminal` row is lost on every
//! attempt, the live-instance teardown still fires, and the
//! workflow-lifecycle reconciler re-emits `StartWorkflow` each tick. Every
//! re-drive re-runs the author body AND appends ANOTHER `Terminal` — the
//! journal (which has no GC) grows unboundedly.
//!
//! # The regression
//!
//! Two engines share ONE `SimJournalStore` (the restart/resume shape).
//! TWO terminal-write failures are queued on a single-peer
//! `SimObservationStore` via `inject_write_failure`, so the obs terminal
//! row stays ABSENT across BOTH starts (reproducing the persistent-failure
//! loop). After both starts + `join_all`, the journal must hold EXACTLY ONE
//! `Terminal` — the second start must detect the durable terminal and
//! short-circuit (no second `Terminal` append), not re-drive the body.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The driven-port assert is at
//! the `JournalStore::load_journal` boundary: exactly one `Terminal` command
//! in the loaded run. The companion assert proves the author body ran
//! exactly ONCE across both starts (a shared `AtomicUsize` the body bumps),
//! distinguishing a genuine short-circuit from mere journal dedup.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A workflow that returns `WorkflowResult::Success` immediately and bumps a
/// shared counter each time its `run` body executes. The counter is the
/// "body-ran-once" oracle: a genuine short-circuit on resume must NOT re-run
/// the body, so the counter is 1 across both starts (the bug re-drives it to
/// 2).
struct CountingSuccess {
    runs: Arc<AtomicUsize>,
}

impl CountingSuccess {
    const WORKFLOW_NAME: &'static str = "counting-success-wf";

    fn spec() -> WorkflowSpec {
        WorkflowSpec {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: Vec::new(),
        }
    }
}

#[async_trait]
impl Workflow for CountingSuccess {
    async fn run(&self, _ctx: &WorkflowCtx) -> WorkflowResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        WorkflowResult::Success
    }
}

/// Count the `Terminal` commands in a loaded run — payload-type-agnostic
/// (`{ .. }`) so it compiles against BOTH the current `String`-based
/// `Terminal` (RED) and the post-fix `WorkflowResult`-based one (GREEN).
fn terminal_count(loaded: &[LoadedEntry]) -> usize {
    loaded
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
        .count()
}

/// A restart over an already-terminal journal — under a PERSISTENT terminal
/// obs-write failure — must NOT append a second `Terminal`, and must NOT
/// re-run the author body. The second `start` detects the durable terminal
/// and short-circuits (re-publishing only the cheap idempotent obs row).
///
/// ADR-0064 §2/§5 + `docs/feature/fix-workflow-terminal-redrive/deliver/rca.md`.
#[tokio::test]
async fn restart_over_terminal_journal_does_not_append_second_terminal() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    // A single-peer observation store. Queue TWO terminal-write failures so
    // the WorkflowTerminal row stays absent across BOTH starts — the
    // persistent-failure loop that re-drives the engine.
    let obs_concrete =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    obs_concrete.inject_write_failure(ObservationStoreError::Io(io::Error::from(
        io::ErrorKind::Interrupted,
    )));
    obs_concrete.inject_write_failure(ObservationStoreError::Io(io::Error::from(
        io::ErrorKind::Interrupted,
    )));
    let obs: Arc<dyn ObservationStore> = obs_concrete;

    let runs = Arc::new(AtomicUsize::new(0));

    // Two independent engines share ONE journal store + ONE obs store — the
    // first start drives `run` to terminal (obs write #1 fails, journal gets
    // Terminal #1); a fresh engine over the same journal models the
    // reconciler re-emit / restart.
    let make_engine = || {
        let mut registry = WorkflowRegistry::new();
        let runs = Arc::clone(&runs);
        registry.register(CountingSuccess::spec().name, move || {
            Box::new(CountingSuccess { runs: Arc::clone(&runs) })
        });
        WorkflowEngine::new(
            Arc::clone(&journal),
            Arc::clone(&clock),
            Arc::clone(&transport),
            Arc::clone(&entropy),
            registry,
            Arc::clone(&obs),
        )
    };

    let spec: WorkflowSpec = CountingSuccess::spec();
    let correlation = CorrelationKey::derive(
        "wf-short-circuit-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-short-circuit-0001").expect("valid instance id");

    // --- First start: drives run to terminal; obs write #1 fails ---
    let engine = make_engine();
    engine.start(&spec, &correlation, &workflow_id).await.expect("first start succeeds");
    engine.join_all().await;

    // --- Resume drives start a second time over the SAME journal ---
    let resumed = make_engine();
    resumed.start(&spec, &correlation, &workflow_id).await.expect("resume start succeeds");
    resumed.join_all().await;

    // Observable outcome at the JournalStore boundary: exactly ONE Terminal.
    let run = journal.load_journal(&workflow_id).await.expect("load after resume");
    assert_eq!(
        terminal_count(&run),
        1,
        "a restart over an already-terminal journal must not append a second Terminal; \
         run was {run:?}"
    );

    // Companion oracle: the author body ran exactly ONCE across both starts.
    // == 1 after the fix (short-circuit skips the spawn); == 2 with the bug
    // (re-drive re-runs the body). Proves the body was short-circuited, not
    // merely that the journal was de-duplicated.
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the author body must run exactly once across both starts — the resume must \
         short-circuit on the durable Terminal, not re-drive run"
    );
}
