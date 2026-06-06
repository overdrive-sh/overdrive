//! `ctx.emit_action` is **at-least-once on the live path** — a channel send
//! that fires but whose `ActionEmitted` record then fails (fsync failure /
//! crash) re-sends the Action on resume. This is the SEND-BEFORE-RECORD
//! contract `WorkflowCtx::emit_action` documents under "Honest semantics",
//! the same shape `ctx.run` carries.
//!
//! # Why this test exists
//!
//! The sibling `workflow_emit_action_not_re_emitted_after_crash` pins the
//! *replay-path* exactly-once case: a crash AFTER `ActionEmitted` is durable
//! is NOT re-emitted. That left the *live-path* window unguarded — the gap
//! between `sender.send(action)` and the `ActionEmitted` append resolving.
//! Three docstrings used to claim an unconditional "exactly one cluster
//! mutation across a crash"; the claim only holds once `ActionEmitted` is
//! journaled. This test pins the honest live-path semantics so the docs and
//! the code cannot silently diverge again.
//!
//! # What it structurally defends against
//!
//! The discriminating observable is **assertion (A)** below: with fsync
//! injection armed at the emit step, the Action still reaches the channel
//! BECAUSE the send precedes the (failing) record. A "fix" that reverses the
//! ordering to record-before-send — which would lose the mutation SILENTLY on
//! a crash between record and send (strictly worse for a cluster mutation) —
//! would make the failing append run FIRST and return `Err` BEFORE the send,
//! so the channel would see ZERO emits and (A) would fail. The assertion is
//! deliberately `== 1`/`== 2`, never `<= 1` — at-least-once is the INTENDED
//! contract, not a defect to be tightened toward exactly-once.
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart on one node.
//!
//! # Port-to-port
//!
//! The driving port is `ctx.emit_action` via the `WorkflowEngine`; the
//! observable outcome is asserted at the Action channel (driven port): the
//! Action reaches the channel on the injected-failure run AND again on the
//! resume — TWO sends across the crash (at-least-once).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};
use overdrive_control_plane::workflow_runtime::{
    ActionEmitReceiver, WorkflowEngine, WorkflowRegistry,
};
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::testing::workflow::ProvisionRecordWithSignalEmit;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::transport::Transport as TransportTrait;
use overdrive_core::workflow::SignalValue;

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

const SEED: u64 = 0x0302_e312_7a55_0009;

fn build_engine(
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
) -> WorkflowEngine {
    let transport: Arc<dyn TransportTrait> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(SEED));
    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecordWithSignalEmit::spec().name, || {
        ProvisionRecordWithSignalEmit::new(
            ProvisionRecordWithSignalEmit::signal_key(),
            Action::Noop,
        )
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

/// Drive `engine` to completion, advancing logical time so any clock-park in
/// the body progresses. `ProvisionRecordWithSignalEmit` replays its signal
/// wait and emits without parking, but the ticker matches the sibling test's
/// shape and is harmless.
async fn drive(engine: &WorkflowEngine, clock: &Arc<SimClock>) {
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
async fn a_send_whose_action_emitted_record_fails_re_emits_on_resume() {
    let correlation = CorrelationKey::derive(
        "wf-emit-at-least-once-0001",
        &ContentHash::of(ProvisionRecordWithSignalEmit::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-emit-at-least-once-0001").expect("valid instance id");
    let spec = ProvisionRecordWithSignalEmit::spec();
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0));

    // Pre-seed the journal with the recorded signal wait so `emit_action` is
    // the FIRST LIVE append. The body's `ctx.wait_for_signal` replays these
    // two entries (cursor advances past both without appending), so an armed
    // fsync failure targets exactly the `ActionEmitted` record — the precise
    // live-path window the defect names.
    let sim_journal = Arc::new(SimJournalStore::new());
    let journal: Arc<dyn JournalStore> = sim_journal.clone();
    let signal_key = ProvisionRecordWithSignalEmit::signal_key();
    let seen_value = SignalValue::new("go");
    journal
        .append(
            &workflow_id,
            &LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: signal_key.clone() }),
        )
        .await
        .expect("seed SignalAwaited");
    journal
        .append(
            &workflow_id,
            &LoadedEntry::Notification(JournalNotification::SignalSeen {
                signal_key,
                value_digest: ContentHash::of(seen_value.as_str().as_bytes()),
                value: seen_value,
            }),
        )
        .await
        .expect("seed SignalSeen");

    // ---- (1) Injected-failure run: the send fires, the ActionEmitted
    //          record fails (fsync injection). ----
    sim_journal.inject_fsync_failure();
    let clock_a = Arc::new(SimClock::new());
    let engine_a = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_a.clone());
    let mut emits_a = engine_a.take_action_emit_receiver().await.expect("emit receiver");
    engine_a.start(&spec, &correlation, &workflow_id).await.expect("start under injection");
    drive(&engine_a, &clock_a).await;
    let injected_run_emits = drain_emits(&mut emits_a).await;

    // ASSERTION (A) — the discriminating observable. The Action reached the
    // channel EVEN THOUGH the ActionEmitted record failed, because the send
    // precedes the record (send-before-record). A record-before-send
    // ordering would return Err from the failing append BEFORE the send and
    // this would be 0 — that regression is what this assertion catches.
    assert_eq!(
        injected_run_emits, 1,
        "send-before-record: the Action reaches the channel even though the \
         ActionEmitted record failed (a record-before-send ordering would be 0)"
    );

    // The failed append left NO ActionEmitted observable (ADR-0063 §4): the
    // journal is still just the two seeded signal entries.
    let after_injection = journal.load_journal(&workflow_id).await.unwrap_or_default();
    assert!(
        !after_injection
            .iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. }))),
        "the failed record left no ActionEmitted at the cursor: {after_injection:?}"
    );
    drop(emits_a);
    drop(engine_a);
    drop(clock_a);

    // ---- (2) Resume against the SAME journal (still lacking ActionEmitted):
    //          replay_emit misses, the live path re-sends. ----
    sim_journal.clear_fsync_failure();
    let clock_b = Arc::new(SimClock::new());
    let engine_b = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_b.clone());
    let mut emits_b = engine_b.take_action_emit_receiver().await.expect("emit receiver");
    engine_b.start(&spec, &correlation, &workflow_id).await.expect("resume start");
    drive(&engine_b, &clock_b).await;
    let resume_emits = drain_emits(&mut emits_b).await;

    // ASSERTION (B) — at-least-once: the resume re-sent the Action because no
    // ActionEmitted was journaled (the record had failed). Total across the
    // crash is TWO, NOT one. This is the INTENDED contract — downstream
    // action-shim idempotency makes the duplicate safe.
    assert_eq!(resume_emits, 1, "resume re-sends when ActionEmitted was never journaled");
    assert_eq!(
        injected_run_emits + resume_emits,
        2,
        "at-least-once across a failed-record crash: the cluster mutation is emitted TWICE \
         (NOT once) — see WorkflowCtx::emit_action \"Honest semantics\""
    );

    // The resume DID journal ActionEmitted this time (injection cleared) — a
    // further resume would now replay it exactly-once.
    let resumed = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let emitted_count = resumed
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. })))
        .count();
    assert_eq!(
        emitted_count, 1,
        "the successful resume records exactly one ActionEmitted: {resumed:?}"
    );
}
