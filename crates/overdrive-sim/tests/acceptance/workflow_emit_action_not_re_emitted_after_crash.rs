//! Slice 03 / US-WP-5 AC3 — an emitted Action is not re-emitted after a
//! crash (idempotent emit); slice-03 AC3.
//!
//! Scenario S-WP-03-04. K1 (O1). A `ProvisionRecordWithSignalEmit`
//! instance that recorded `ActionEmitted` for a `ctx.emit_action` before
//! terminal is killed after the emit records but before terminal, and
//! restarted on the same node; the Action is NOT re-emitted on resume (the
//! `ActionEmitted` journal entry makes the emit idempotent) — exactly one
//! cluster mutation across the crash. ADR-0063 §2 (`ActionEmitted`),
//! ADR-0064 §4.
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart on one node.
//!
//! # Port-to-port
//!
//! The driving port is `ctx.emit_action` via the `WorkflowEngine`; the
//! observable outcome is asserted at the Action channel (driven port):
//! exactly ONE Action across the pre-crash run + the resume.
//!
//! # Falsifiability
//!
//! If the resume re-sent the recorded Action, the resume's Action channel
//! would carry a second emit — `exactly_one_emit_across_crash` would fail.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{
    ActionEmitReceiver, WorkflowEngine, WorkflowRegistry,
};
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::testing::workflow::ProvisionRecordWithSignalEmit;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::transport::Transport as TransportTrait;
use overdrive_core::workflow::SignalValue;

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

const SEED: u64 = 0x0302_e312_7a55_0004;

fn build_engine(
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
) -> WorkflowEngine {
    let transport: Arc<dyn TransportTrait> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(SEED));
    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecordWithSignalEmit::spec().name, || {
        Box::new(ProvisionRecordWithSignalEmit::new(
            ProvisionRecordWithSignalEmit::signal_key(),
            Action::Noop,
        ))
    });
    WorkflowEngine::new(journal, clock, transport, entropy, registry, obs)
}

async fn drain_emits(rx: &mut ActionEmitReceiver) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
        count += 1;
    }
    count
}

async fn drive_with_signal(
    engine: &WorkflowEngine,
    obs: &Arc<dyn ObservationStore>,
    clock: &Arc<SimClock>,
    spec: &overdrive_core::workflow::WorkflowSpec,
    correlation: &CorrelationKey,
    workflow_id: &WorkflowId,
) {
    obs.write(ObservationRow::Signal {
        key: ProvisionRecordWithSignalEmit::signal_key(),
        value: SignalValue::new("go"),
    })
    .await
    .expect("write signal");
    engine.start(spec, correlation, workflow_id).await.expect("start");
    let driver = Arc::clone(clock);
    let ticker = tokio::spawn(async move {
        for _ in 0..16 {
            tokio::task::yield_now().await;
            driver.tick(Duration::from_millis(100));
        }
    });
    engine.join_all().await;
    let _ = ticker.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_action_emitted_before_the_crash_is_not_re_emitted_on_resume() {
    let correlation = CorrelationKey::derive(
        "wf-emit-idempotent-0001",
        &ContentHash::of(ProvisionRecordWithSignalEmit::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-emit-idempotent-0001").expect("valid instance id");
    let spec = ProvisionRecordWithSignalEmit::spec();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0));

    // ---- (1) Pre-crash run: signal present, emit fires exactly once. ----
    let clock_a = Arc::new(SimClock::new());
    let engine_a = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_a.clone());
    let mut emits_a = engine_a.take_action_emit_receiver().await.expect("emit receiver");
    drive_with_signal(&engine_a, &obs, &clock_a, &spec, &correlation, &workflow_id).await;
    let pre_crash_emits = drain_emits(&mut emits_a).await;
    assert_eq!(pre_crash_emits, 1, "the live run emits the Action exactly once");

    let full = journal.load_journal(&workflow_id).await.unwrap_or_default();
    assert!(
        full.iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. }))),
        "the live emit records ActionEmitted: {full:?}"
    );
    drop(emits_a);
    drop(engine_a);
    drop(clock_a);

    // ---- (2) Crash AFTER ActionEmitted, BEFORE Terminal: truncate the
    //          journal at the Terminal boundary (ActionEmitted retained). ----
    let truncated: Vec<LoadedEntry> = full
        .iter()
        .take_while(|e| !matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
        .cloned()
        .collect();
    assert!(
        truncated
            .iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. }))),
        "the crash journal retains ActionEmitted (crash was AFTER the emit): {truncated:?}"
    );
    let resume_journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    for entry in &truncated {
        resume_journal.append(&workflow_id, entry).await.expect("seed resume journal");
    }

    // ---- (3) Resume: the recorded SignalSeen + ActionEmitted are replayed;
    //          the Action is NOT re-sent. (Signal still present in obs, but
    //          replay short-circuits both the wait and the emit.) ----
    let clock_b = Arc::new(SimClock::new());
    let engine_b = build_engine(Arc::clone(&resume_journal), Arc::clone(&obs), clock_b.clone());
    let mut emits_b = engine_b.take_action_emit_receiver().await.expect("emit receiver");
    engine_b.start(&spec, &correlation, &workflow_id).await.expect("resume start");
    let driver_b = Arc::clone(&clock_b);
    let ticker_b = tokio::spawn(async move {
        for _ in 0..16 {
            tokio::task::yield_now().await;
            driver_b.tick(Duration::from_millis(100));
        }
    });
    engine_b.join_all().await;
    let _ = ticker_b.await;

    // OBSERVABLE — the resume re-emitted ZERO Actions: the recorded
    // ActionEmitted made the emit idempotent. Exactly ONE cluster mutation
    // across the crash (pre-crash 1 + resume 0).
    let resume_emits = drain_emits(&mut emits_b).await;
    assert_eq!(
        resume_emits, 0,
        "the resumed run must NOT re-emit the recorded Action (idempotent emit)"
    );
    assert_eq!(
        pre_crash_emits + resume_emits,
        1,
        "exactly ONE cluster mutation across the crash (K1 / US-WP-5 AC3)"
    );
    // The resume did NOT append a second ActionEmitted — it replayed.
    let resumed = resume_journal.load_journal(&workflow_id).await.unwrap_or_default();
    let emitted_count = resumed
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. })))
        .count();
    assert_eq!(
        emitted_count, 1,
        "exactly one ActionEmitted — replay did not re-record the emit: {resumed:?}"
    );
}
