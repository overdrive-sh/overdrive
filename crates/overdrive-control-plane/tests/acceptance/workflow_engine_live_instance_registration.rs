//! `WorkflowEngine::start` registers a running instance in the live set
//! (ADR-0064 §5).
//!
//! Preserved from the deleted `workflow_engine_terminal_labels.rs` (whose
//! label tests went away with `workflow_result_label` per the
//! fix-workflow-terminal-redrive deletion). The live-instance registration
//! surface it also pinned is UNRELATED to the label and the production code
//! (`live_instances()`) survives, so its test does too — it is the only
//! guard against the `live_instances -> BTreeSet::new()` mutation.
//!
//! `start` inserts the instance `CorrelationKey` into the live set BEFORE
//! spawning the task — the set the workflow-lifecycle reconciler's
//! `hydrate_actual` reads to derive `has_live_task`. A still-running
//! instance must read as present, or the reconciler would see
//! `has_live_task = false` for a running instance and spuriously re-emit
//! `StartWorkflow`.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The driven-port assert is at
//! the engine's `live_instances()` accessor (the live set).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowStart,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A workflow that parks forever under `SimClock` (the harness never ticks),
/// so the engine task never reaches terminal and the live-instance entry
/// persists for the duration of the test.
struct BlockingWorkflow;

impl BlockingWorkflow {
    const WORKFLOW_NAME: &'static str = "blocking-wf";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: Vec::new(),
        }
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
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let spec = BlockingWorkflow::spec();
    let mut registry = WorkflowRegistry::new();
    registry.register(spec.name.clone(), || Box::new(BlockingWorkflow));
    let engine =
        WorkflowEngine::new(Arc::clone(&journal), clock, transport, entropy, registry, obs);

    let correlation = CorrelationKey::derive(
        "wf-blocking-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::for_correlation(&correlation);

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
