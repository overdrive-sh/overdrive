//! Engine terminal-label journaling + live-instance set registration
//! (ADR-0064 §2 / §3 / §5).
//!
//! Two driven-port surfaces the engine owns that the slice-01 obs-row test
//! (`workflow_engine_writes_terminal_row`) does NOT exercise:
//!
//! - **Terminal LABEL in the journal.** On `run` terminal the engine appends
//!   a `JournalCommand::Terminal { result }` whose string is
//!   `workflow_result_label(&result)` — the canonical, stable label a resumed
//!   run reads back. The obs-row carries the raw `WorkflowResult`; the JOURNAL
//!   carries the label string, so the label mapping is a distinct surface that
//!   must be pinned per-variant (`Success` / `Failed` / `Cancelled`).
//! - **Live-instance set registration.** `WorkflowEngine::start` inserts the
//!   instance `CorrelationKey` into the live set BEFORE spawning the task
//!   (ADR-0064 §5) — the set the workflow-lifecycle reconciler's
//!   `hydrate_actual` reads to derive `has_live_task`. A still-running
//!   instance must read as present.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The driven-port asserts are
//! at the `JournalStore::load_journal` boundary (the Terminal label) and the
//! engine's `live_instances()` accessor (the live set).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A workflow that returns `WorkflowResult::Failed` immediately — drives the
/// engine's `Failed` terminal-label path.
struct AlwaysFailed;

impl AlwaysFailed {
    const WORKFLOW_NAME: &'static str = "always-failed-wf";

    fn spec() -> WorkflowSpec {
        WorkflowSpec { name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name") }
    }
}

#[async_trait]
impl Workflow for AlwaysFailed {
    async fn run(&self, _ctx: &WorkflowCtx) -> WorkflowResult {
        WorkflowResult::Failed { reason: "deliberate test failure".to_string() }
    }
}

/// A workflow that returns `WorkflowResult::Cancelled` immediately — drives
/// the engine's `Cancelled` terminal-label path.
struct AlwaysCancelled;

impl AlwaysCancelled {
    const WORKFLOW_NAME: &'static str = "always-cancelled-wf";

    fn spec() -> WorkflowSpec {
        WorkflowSpec { name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name") }
    }
}

#[async_trait]
impl Workflow for AlwaysCancelled {
    async fn run(&self, _ctx: &WorkflowCtx) -> WorkflowResult {
        WorkflowResult::Cancelled
    }
}

/// A workflow that parks forever under `SimClock` (the harness never ticks),
/// so the engine task never reaches terminal and the live-instance entry
/// persists for the duration of the test.
struct BlockingWorkflow;

impl BlockingWorkflow {
    const WORKFLOW_NAME: &'static str = "blocking-wf";

    fn spec() -> WorkflowSpec {
        WorkflowSpec { name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name") }
    }
}

#[async_trait]
impl Workflow for BlockingWorkflow {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        // Park forever: the SimClock is never ticked, so this never resolves
        // and the task stays live. (The result below is unreachable in test.)
        let _ = ctx.sleep(Duration::from_secs(3600)).await;
        WorkflowResult::Success
    }
}

/// Build a fully-wired `WorkflowEngine` over a fresh shared journal, with
/// `registry` already populated. Returns the journal (to inspect the run) and
/// the engine.
fn engine_with(registry: WorkflowRegistry) -> (Arc<dyn JournalStore>, WorkflowEngine) {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let engine =
        WorkflowEngine::new(Arc::clone(&journal), clock, transport, entropy, registry, obs);
    (journal, engine)
}

/// The canonical instance correlation + journal id derived from `spec`.
fn instance(spec: &WorkflowSpec, target: &str) -> (CorrelationKey, WorkflowId) {
    let correlation = CorrelationKey::derive(
        target,
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::for_correlation(&correlation);
    (correlation, workflow_id)
}

/// The terminal-result label recorded in the loaded run, if any.
fn terminal_label(loaded: &[LoadedEntry]) -> Option<String> {
    loaded.iter().rev().find_map(|entry| match entry {
        LoadedEntry::Command(JournalCommand::Terminal { result }) => Some(result.clone()),
        _ => None,
    })
}

/// ADR-0064 §3: a `Failed` workflow journals a `Terminal` command whose label
/// is exactly `"Failed"`. Pins the `WorkflowResult::Failed` arm of
/// `workflow_result_label` against deletion (which would fold it to the
/// `"Unknown"` wildcard).
#[tokio::test]
async fn failed_workflow_journals_terminal_with_failed_label() {
    let spec = AlwaysFailed::spec();
    let mut registry = WorkflowRegistry::new();
    registry.register(spec.name.clone(), || Box::new(AlwaysFailed));
    let (journal, engine) = engine_with(registry);
    let (correlation, workflow_id) = instance(&spec, "wf-failed-0001");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    let loaded = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(
        terminal_label(&loaded).as_deref(),
        Some("Failed"),
        "a Failed workflow journals a Terminal command labelled \"Failed\", not the \
         \"Unknown\" wildcard; run was {loaded:?}"
    );
}

/// ADR-0064 §3: a `Cancelled` workflow journals a `Terminal` command whose
/// label is exactly `"Cancelled"`. Pins the `WorkflowResult::Cancelled` arm of
/// `workflow_result_label` against deletion (which would fold it to the
/// `"Unknown"` wildcard).
#[tokio::test]
async fn cancelled_workflow_journals_terminal_with_cancelled_label() {
    let spec = AlwaysCancelled::spec();
    let mut registry = WorkflowRegistry::new();
    registry.register(spec.name.clone(), || Box::new(AlwaysCancelled));
    let (journal, engine) = engine_with(registry);
    let (correlation, workflow_id) = instance(&spec, "wf-cancelled-0001");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    let loaded = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(
        terminal_label(&loaded).as_deref(),
        Some("Cancelled"),
        "a Cancelled workflow journals a Terminal command labelled \"Cancelled\", not the \
         \"Unknown\" wildcard; run was {loaded:?}"
    );
}

/// ADR-0064 §5: `WorkflowEngine::start` registers the instance correlation in
/// the live set BEFORE spawning the task, so a still-running instance reads as
/// present. The workflow parks forever under `SimClock` (never ticked), so the
/// task never reaches terminal and never tears the entry down — the entry is
/// observable WITHOUT a `join_all`.
///
/// Pins `live_instances()` against the `-> BTreeSet::new()` mutation, which
/// would always report the set empty and so falsely tell the lifecycle
/// reconciler `has_live_task = false` for a running instance (spurious
/// re-emit).
#[tokio::test]
async fn start_registers_instance_in_live_set_while_running() {
    let spec = BlockingWorkflow::spec();
    let mut registry = WorkflowRegistry::new();
    registry.register(spec.name.clone(), || Box::new(BlockingWorkflow));
    let (_journal, engine) = engine_with(registry);
    let (correlation, workflow_id) = instance(&spec, "wf-blocking-0001");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    // Deliberately NO join_all — the task parks forever, so the live entry
    // persists and the assertion does not race a teardown.

    let live = engine.live_instances();
    assert!(
        live.contains(&correlation),
        "start must register the running instance in the live set (a BTreeSet::new() getter \
         mutation reports it empty); live set was {live:?}"
    );
}
