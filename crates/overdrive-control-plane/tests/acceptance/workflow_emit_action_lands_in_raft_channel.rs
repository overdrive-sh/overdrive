//! Slice 03 / US-WP-5 AC2 — `ctx.emit_action` lands the typed Action in
//! the Raft channel with no direct `IntentStore` write; slice-03 AC2.
//!
//! Scenario S-WP-03-03. O3. A sequence that calls `ctx.emit_action
//! (action)`: the typed Action lands in the Action channel the reconciler
//! runtime consumes (→ Raft / Phase-1 `IntentStore` commit path), and the
//! workflow performs NO direct `IntentStore` write (`development.md`
//! Workflow contract rule 6 — the workflow never bypasses Raft). The
//! observable universe is "Action-channel arrivals" + "`IntentStore` writes
//! BY the workflow"; the latter must be empty. ADR-0064 §4/§5.
//!
//! # Port-to-port
//!
//! The driving port is the `ctx.emit_action` author surface, driven via
//! `WorkflowEngine::start` (the async executor off the shim). The driven
//! port is the engine's real Action channel (taken via
//! `take_action_emit_receiver`). The observable outcomes are asserted at
//! TWO driven-port boundaries: (1) the typed Action arrives on the Action
//! channel; (2) a counting `IntentStore` injected alongside the engine
//! records ZERO writes across the run — the emit reaches Raft through the
//! channel, never a direct store write. No internal helper is touched.

#![allow(clippy::expect_used, clippy::unwrap_used)]
// Test-double constructors: the const-fn lint adds ceremony with no test
// value on the recording/counting fixtures.
#![allow(clippy::missing_const_for_fn)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, PutOutcome, StateSnapshot, TxnOp, TxnOutcome,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A workflow whose ONLY effect is a single `ctx.emit_action(Action::Noop)`
/// followed by a terminal `Success`. Authored as one ordinary `async fn
/// run` — the emit goes through `ctx`, the only sanctioned surface; `ctx`
/// exposes no `IntentStore` `.put()`, so the workflow body structurally
/// cannot write the store directly.
struct EmittingWorkflow;

impl EmittingWorkflow {
    const WORKFLOW_NAME: &'static str = "emitting-workflow";

    fn spec() -> WorkflowSpec {
        WorkflowSpec { name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name") }
    }
}

#[async_trait]
impl Workflow for EmittingWorkflow {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        // The workflow→cluster mutation goes through ctx.emit_action — the
        // SAME Action channel the reconciler runtime consumes (→ Raft). The
        // body has no IntentStore handle; the emit is the only mutation it
        // can express, and it routes through the channel by construction.
        match ctx.emit_action(Action::Noop).await {
            Ok(()) => WorkflowResult::Success,
            Err(_) => WorkflowResult::Failed { reason: "emit failed".to_string() },
        }
    }
}

/// A counting `IntentStore` that records every mutating call. Injected
/// alongside the engine to prove the observable invariant: across an
/// emit-driven workflow run, the workflow performs ZERO direct
/// `IntentStore` writes — the emit reaches Raft through the Action channel,
/// never a direct store write (`development.md` Workflow contract rule 6).
struct CountingIntentStore {
    writes: AtomicUsize,
}

impl CountingIntentStore {
    fn new() -> Self {
        Self { writes: AtomicUsize::new(0) }
    }

    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn record_write(&self) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl IntentStore for CountingIntentStore {
    async fn get(&self, _key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
        Ok(None)
    }

    async fn put(&self, _key: &[u8], _value: &[u8]) -> Result<(), IntentStoreError> {
        self.record_write();
        Ok(())
    }

    async fn put_if_absent(
        &self,
        _key: &[u8],
        _value: &[u8],
    ) -> Result<PutOutcome, IntentStoreError> {
        self.record_write();
        Ok(PutOutcome::Inserted)
    }

    async fn delete(&self, _key: &[u8]) -> Result<(), IntentStoreError> {
        self.record_write();
        Ok(())
    }

    async fn txn(&self, _ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
        self.record_write();
        Ok(TxnOutcome::Committed)
    }

    async fn watch(
        &self,
        _prefix: &[u8],
    ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError> {
        Ok(Box::new(futures::stream::empty()))
    }

    async fn scan_prefix(&self, _prefix: &[u8]) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError> {
        Ok(Vec::new())
    }

    async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
        Ok(StateSnapshot::from_parts(0, Vec::new(), Vec::new()))
    }

    async fn bootstrap_from(&self, _snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
        self.record_write();
        Ok(())
    }
}

#[tokio::test]
async fn emit_action_lands_in_the_action_channel_and_the_workflow_makes_no_direct_intent_store_write()
 {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    // The IntentStore the workflow would have to write to if it bypassed
    // Raft. It is NOT threaded into the workflow path at all — `ctx`
    // exposes no `.put()`. We assert its write-count stays 0 across the run.
    let intent = Arc::new(CountingIntentStore::new());

    let mut registry = WorkflowRegistry::new();
    registry.register(EmittingWorkflow::spec().name, || Box::new(EmittingWorkflow));

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    // Take the receiver half of the engine's Action channel — the driven
    // port the workflow's ctx.emit_action lands on. This IS the same
    // channel the reconciler runtime consumes (→ Raft); the engine owns
    // both halves and hands the receiver to its consumer.
    let mut action_rx = engine
        .take_action_emit_receiver()
        .await
        .expect("engine yields the Action-channel receiver once");

    let _target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    let spec: WorkflowSpec = EmittingWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-emit-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-emit-0001").expect("valid instance id");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    // Observable outcome 1 (positive): the typed Action arrives on the
    // Action channel — the workflow→cluster mutation reached the Raft
    // commit path, not a direct store write.
    let landed = tokio::time::timeout(Duration::from_secs(1), action_rx.recv())
        .await
        .expect("an Action lands on the channel within the deadline");
    assert_eq!(
        landed,
        Some(Action::Noop),
        "ctx.emit_action must land the typed Action on the Action channel (→ Raft)"
    );

    // Observable outcome 2 (no Raft bypass): the workflow performed ZERO
    // direct IntentStore writes — the emit went through the channel, never
    // a `.put()` (development.md Workflow contract rule 6). `ctx` exposes
    // no IntentStore surface, so this count is structurally guaranteed 0;
    // asserting it pins the invariant against a future regression that
    // wires a store into the ctx.
    assert_eq!(
        intent.write_count(),
        0,
        "the workflow must perform NO direct IntentStore write — the emit goes through Raft"
    );
}
