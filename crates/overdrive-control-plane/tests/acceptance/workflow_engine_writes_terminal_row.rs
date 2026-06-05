//! Slice 01 / AC5 (slice-01 AC5) — the engine writes a terminal-result
//! [`ObservationStore`] row via the sanctioned shim [`ObservationStore`]
//! write path on `run` terminal. ADR-0064 §2.
//!
//! Scenario companion to S-WP-01-11. When the engine drives the author's
//! `async fn run` to a `WorkflowResult`, it writes
//! `ObservationRow::WorkflowTerminal { correlation, result }` — keyed by
//! the instance `CorrelationKey` so the emitting workflow-lifecycle
//! reconciler finds the result deterministically next tick (ADR-0064 §2 /
//! `development.md` Reconciler I/O rule 2). The write goes through the
//! injected `ObservationStore` (the shim's write path), NOT a direct
//! engine bypass of the channels.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` (the async executor driven
//! off the shim). The driven port is the injected `ObservationStore`. The
//! observable outcome is asserted at the `ObservationStore` boundary: a
//! `WorkflowTerminal` row keyed by the instance correlation arrives on the
//! store's `subscribe_all` stream after the engine drove `run` to terminal.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationSubscription,
};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{WorkflowResult, WorkflowSpec};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

#[tokio::test]
async fn engine_writes_workflow_terminal_observation_row_on_run_terminal() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || Box::new(ProvisionRecord::new(target)));

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowSpec = ProvisionRecord::spec();
    let correlation = CorrelationKey::derive(
        "wf-provision-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-0001").expect("valid instance id");

    // Subscribe BEFORE driving so the terminal row is observed on the stream.
    let mut subscription: ObservationSubscription =
        obs.subscribe_all().await.expect("subscribe succeeds");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    // Observable outcome at the ObservationStore boundary: a
    // WorkflowTerminal row keyed by the instance correlation, carrying the
    // workflow's terminal result.
    let mut found: Option<(CorrelationKey, WorkflowResult)> = None;
    for _ in 0..8 {
        let next = tokio::time::timeout(Duration::from_secs(1), subscription.next()).await;
        match next {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation, result })) => {
                found = Some((correlation, result));
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }

    let (got_corr, got_result) =
        found.expect("engine must write a WorkflowTerminal observation row on run terminal");
    assert_eq!(
        got_corr, correlation,
        "the terminal row must be keyed by the instance CorrelationKey (ADR-0064 §2)"
    );
    assert_eq!(
        got_result,
        WorkflowResult::Success,
        "the terminal row must carry the workflow's terminal result"
    );
}
