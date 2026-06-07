//! `WorkflowEngine::start` is a no-op for an instance that is ALREADY live
//! (ADR-0064 §5 — the concurrent-start guard).
//!
//! Regression test for: *No concurrent-start guard — two drives can corrupt
//! the same journal.* `start` claimed `live_instances` with an
//! `insert(correlation)` whose `bool` return was discarded, then fell through
//! unconditionally to `tasks.spawn`. A second `start` for an
//! already-running correlation — reachable via a `ctx.emit_action`
//! `StartWorkflow`, or a re-emit racing the reconciler's *advisory*
//! `has_live_task` snapshot — therefore spawned a SECOND task driving the
//! SAME journal. Two `JournalCursorHandle`s built from different snapshots
//! interleave their `append`s, producing a command sequence the positional
//! check-then-record cursor cannot replay deterministically on the next boot.
//!
//! The fix claims `live_instances` ATOMICALLY (`BTreeSet::insert` returns
//! `false` iff already present) before any journal-mutating work or the
//! spawn, and early-returns `Ok(())` when the instance is already live.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`, invoked twice for one
//! correlation. The driven-port oracle is a shared run-entry counter
//! (`Arc<AtomicUsize>`): the engine resolves a fresh workflow instance per
//! drive, so the body's entry count equals the number of drives the engine
//! spawned. Exactly one drive must run for two `start` calls.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{TerminalError, Workflow, WorkflowCtx, WorkflowName, WorkflowStart};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

const WORKFLOW_NAME: &str = "counting-blocking-wf";

/// A workflow that records that its body was ENTERED (one increment of the
/// shared counter) and then parks forever under `SimClock` (never ticked),
/// so the drive never reaches terminal and never tears its live entry down.
/// The increment is the oracle: the engine resolves a fresh instance per
/// drive, so the counter equals the number of drives the engine spawned.
struct CountingBlockingWorkflow {
    entered: Arc<AtomicUsize>,
}

#[async_trait]
impl Workflow for CountingBlockingWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // Record the entry BEFORE the first `.await` — runs synchronously the
        // moment the spawned task is first polled, so a single runtime drain
        // (yield) deterministically reflects it.
        self.entered.fetch_add(1, Ordering::SeqCst);
        // Park forever: the SimClock is never ticked, so this never resolves
        // and the instance stays live.
        let _ = ctx.sleep(Duration::from_secs(3600)).await;
        Ok(())
    }
}

/// CBOR-encode the unit `Input` (an empty `Vec` would fail-fast as
/// `MalformedInput` and terminate the drive — defeating the "stays live" oracle).
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes).expect("CBOR-encode unit");
    bytes
}

fn spec() -> WorkflowStart {
    WorkflowStart {
        name: WorkflowName::new(WORKFLOW_NAME).expect("valid kebab name"),
        input: cbor_unit(),
    }
}

/// Drain the current-thread runtime so any spawned task is polled to its first
/// `.await`. `#[tokio::test]` defaults to the current-thread flavour: nothing
/// runs except when the test task yields, so a fixed number of `yield_now`s
/// deterministically lets a freshly-spawned drive reach its entry increment
/// (and then park at `ctx.sleep`). No wall-clock dependency.
async fn drain_runtime() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// ADR-0064 §5 concurrent-start guard: a second `start` for a correlation that
/// is ALREADY live must be a no-op — it must NOT spawn a second drive against
/// the same journal.
///
/// Without the guard, the second `start` falls through to `tasks.spawn` and a
/// second task enters the body (counter == 2). With the guard, the atomic
/// `live_instances` claim fails for the already-present correlation and `start`
/// early-returns, leaving exactly one drive (counter == 1).
#[tokio::test]
async fn second_start_for_live_instance_does_not_spawn_a_second_drive() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    // Shared run-entry counter, captured by the registry factory so every
    // instance the engine resolves increments the SAME counter.
    let entered = Arc::new(AtomicUsize::new(0));
    let factory_counter = Arc::clone(&entered);

    let spec = spec();
    let mut registry = WorkflowRegistry::new();
    registry.register(spec.name.clone(), move || CountingBlockingWorkflow {
        entered: Arc::clone(&factory_counter),
    });
    let engine =
        WorkflowEngine::new(Arc::clone(&journal), clock, transport, entropy, registry, obs);

    let correlation = CorrelationKey::derive(
        "wf-concurrent-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::for_correlation(&correlation);

    // First start — claims the instance and spawns the (parking) drive.
    engine.start(&spec, &correlation, &workflow_id).await.expect("first start succeeds");
    drain_runtime().await;
    assert_eq!(entered.load(Ordering::SeqCst), 1, "the first start must spawn exactly one drive");
    assert!(
        engine.live_instances().contains(&correlation),
        "the instance must be live after the first start"
    );

    // Second start for the SAME live correlation — must be a no-op. The engine
    // is already driving this instance's journal; a second drive would
    // interleave appends the positional cursor cannot replay.
    engine.start(&spec, &correlation, &workflow_id).await.expect("second start is a no-op Ok");
    drain_runtime().await;
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "a second start for an already-live instance must NOT spawn a second drive \
         (without the concurrent-start guard the body is entered twice)"
    );
}
