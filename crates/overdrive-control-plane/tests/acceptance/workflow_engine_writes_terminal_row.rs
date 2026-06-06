//! Slice 01 / AC5 (slice-01 AC5) — the engine writes a terminal-result
//! [`ObservationStore`] row via the sanctioned shim [`ObservationStore`]
//! write path on `run` terminal. ADR-0064 §2.
//!
//! Scenario companion to S-WP-01-11. When the engine drives the author's
//! `async fn run` to a terminal, it projects the body's `Result<Output,
//! TerminalError>` to a `WorkflowStatus` and writes
//! `ObservationRow::WorkflowTerminal { correlation, status }` — keyed by
//! the instance `CorrelationKey` so the emitting workflow-lifecycle
//! reconciler finds the status deterministically next tick (ADR-0064 §2 /
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

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationSubscription,
};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{WorkflowStart, WorkflowStatus};

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
    registry.register(ProvisionRecord::spec().name, move || ProvisionRecord::new(target));

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = ProvisionRecord::spec();
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
    // workflow's terminal status.
    let mut found: Option<(CorrelationKey, WorkflowStatus)> = None;
    for _ in 0..8 {
        let next = tokio::time::timeout(Duration::from_secs(1), subscription.next()).await;
        match next {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation, status })) => {
                found = Some((correlation, status));
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }

    let (got_corr, got_status) =
        found.expect("engine must write a WorkflowTerminal observation row on run terminal");
    assert_eq!(
        got_corr, correlation,
        "the terminal row must be keyed by the instance CorrelationKey (ADR-0064 §2)"
    );
    // `ProvisionRecord` returns `Ok(())`, so the engine projects it to
    // `WorkflowStatus::Completed { output }` carrying the CBOR of `()`.
    let WorkflowStatus::Completed { output } = got_status else {
        panic!("the terminal row must carry a Completed status, got {got_status:?}");
    };
    let decoded: () = ciborium::from_reader(output.as_slice())
        .expect("the erased Completed output decodes back to the unit Output");
    assert_eq!(decoded, (), "ProvisionRecord's `Output = ()` round-trips through the terminal row");
}

/// Count the `Started` commands in a loaded run and return the index of
/// the first `LoadedEntry::Command` (the first command-walk position),
/// alongside the first `Started`'s digests if present.
fn started_facts(
    loaded: &[LoadedEntry],
) -> (usize, Option<usize>, Option<(ContentHash, ContentHash)>) {
    let mut started_count = 0usize;
    let mut first_command_pos: Option<usize> = None;
    let mut first_started: Option<(ContentHash, ContentHash)> = None;
    for (pos, entry) in loaded.iter().enumerate() {
        if let LoadedEntry::Command(command) = entry {
            if first_command_pos.is_none() {
                first_command_pos = Some(pos);
            }
            if let JournalCommand::Started { spec_digest, input_digest } = command {
                started_count += 1;
                if first_started.is_none() {
                    first_started = Some((*spec_digest, *input_digest));
                }
            }
        }
    }
    (started_count, first_command_pos, first_started)
}

/// CA-4 — the trap itself. `WorkflowEngine::start` writes `Started` at
/// command-index 0 on first start (the input-derived `spec_digest` /
/// `input_digest`), and is idempotent on resume: driving `start` a second
/// time over the persisted journal does NOT append a duplicate `Started`.
///
/// ADR-0063 §2 / ADR-0064 §5.
///
/// # Port-to-port
///
/// The driving port is `WorkflowEngine::start`. The driven port assert is
/// at the `JournalStore::load_journal` boundary: after the first start the
/// loaded run's first `Command` is `Started` at command-index 0; after a
/// resume drives `start` a second time over the SAME persisted journal,
/// exactly ONE `Started` command exists in the run (no duplicate).
#[tokio::test]
async fn start_writes_started_at_command_index_0_idempotent_on_resume() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let target: SocketAddr = "127.0.0.1:9100".parse().expect("valid addr");

    // Two independent engines share ONE journal store — the first start
    // persists `Started`; a fresh engine over the same journal models the
    // restart/resume (every previously-running instance reads back its
    // persisted run, exactly the crash-resume shape).
    let make_engine = || {
        let mut registry = WorkflowRegistry::new();
        registry.register(ProvisionRecord::spec().name, move || ProvisionRecord::new(target));
        WorkflowEngine::new(
            Arc::clone(&journal),
            Arc::clone(&clock),
            Arc::clone(&transport),
            Arc::clone(&entropy),
            registry,
            Arc::clone(&obs),
        )
    };

    let spec: WorkflowStart = ProvisionRecord::spec();
    let correlation = CorrelationKey::derive(
        "wf-provision-0002",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-0002").expect("valid instance id");

    // The input-derived digests the engine must record (ADR-0063 §2): the
    // spec's canonical identity and the start input. Per
    // `development.md` § "Persist inputs, not derived state" — INPUTS, not
    // a pre-computed cache. Mirrors the journal-store characterization
    // test's derivation so the engine's choice is pinned, not freeform.
    let expected_spec_digest = ContentHash::of(spec.name.as_str().as_bytes());
    let expected_input_digest = ContentHash::of(ProvisionRecord::PAYLOAD);

    // --- First start ---
    let engine = make_engine();
    engine.start(&spec, &correlation, &workflow_id).await.expect("first start succeeds");
    engine.join_all().await;

    let after_first = journal.load_journal(&workflow_id).await.expect("load after first start");
    let (started_count, first_command_pos, first_started) = started_facts(&after_first);

    assert_eq!(
        started_count, 1,
        "first start must write exactly one Started command (ADR-0064 §5), got run {after_first:?}"
    );
    assert_eq!(
        first_command_pos,
        Some(0),
        "Started is at command-index 0 — it is the FIRST entry in the run \
         (the engine writes it before the author body records anything), got {after_first:?}"
    );
    assert!(
        matches!(after_first.first(), Some(LoadedEntry::Command(JournalCommand::Started { .. }))),
        "the loaded run BEGINS with a Started command (command-index 0), got {after_first:?}"
    );
    assert_eq!(
        first_started,
        Some((expected_spec_digest, expected_input_digest)),
        "Started records the input-derived spec_digest / input_digest (ADR-0063 §2), \
         not a derived cache"
    );

    // --- Resume drives start a second time over the persisted journal ---
    let resumed = make_engine();
    resumed.start(&spec, &correlation, &workflow_id).await.expect("resume start succeeds");
    resumed.join_all().await;

    let after_resume = journal.load_journal(&workflow_id).await.expect("load after resume");
    let (resumed_started_count, resumed_first_command_pos, _) = started_facts(&after_resume);

    assert_eq!(
        resumed_started_count, 1,
        "resume is idempotent — it must NOT append a second Started (the trap, CA-4), \
         got run {after_resume:?}"
    );
    assert_eq!(
        resumed_first_command_pos,
        Some(0),
        "after resume the run still BEGINS with the original Started at command-index 0, \
         got {after_resume:?}"
    );
}
