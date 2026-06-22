//! Slice 03 / AC1 (resume re-checks satisfaction) — a satisfied signal is
//! not re-waited on resume.
//!
//! Scenario S-WP-03-02. O1. A `ProvisionRecordWithSignalEmit` instance
//! that recorded `SignalSeen` for `key` before the crash is killed and
//! restarted on the same node; on resume it does NOT re-block on `key` —
//! it reads the recorded signal value and proceeds (check-then-record on
//! replay), EVEN AFTER the signal row is removed from the surface. ADR-0066
//! §2 (`SignalSeen { value }`), ADR-0064 §3.
//!
//! # Port-to-port
//!
//! The driving port is `ctx.wait_for_signal` via the `WorkflowEngine`. The
//! observable outcome: the resumed run reaches terminal Success WITHOUT a
//! live signal row present — proving the recorded `SignalSeen` value was
//! replayed, not re-read.
//!
//! # Falsifiability
//!
//! If replay re-read the live signal surface instead of the recorded
//! `SignalSeen`, the resumed run (with NO signal row present) would block
//! forever and never reach terminal — `replays_recorded_value` would fail.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::testing::workflow::ProvisionRecordWithSignalEmit;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::observation_store::{
    LagAwareSubscription, ObservationRow, ObservationStore, SubscriptionEvent,
};
use overdrive_core::traits::transport::Transport as TransportTrait;
use overdrive_core::workflow::{SignalValue, WorkflowStatus};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

const SEED: u64 = 0x0302_5ee2_b10c_0002;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signal_seen_before_the_crash_is_not_rewaited_on_resume() {
    let correlation = CorrelationKey::derive(
        "wf-signal-seen-0001",
        &ContentHash::of(ProvisionRecordWithSignalEmit::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-signal-seen-0001").expect("valid instance id");
    let spec = ProvisionRecordWithSignalEmit::spec();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0));

    // ---- (1) Pre-crash run: signal PRESENT, so the wait records
    //          SignalSeen. We crash AFTER SignalSeen but BEFORE terminal by
    //          using a registry whose workflow blocks forever AFTER the
    //          signal — simulated by recording the wait then dropping. We
    //          instead drive a normal run with the signal present, capture
    //          the journal AFTER SignalSeen records, then crash by reusing
    //          the journal on a fresh engine with the signal REMOVED. ----
    let signal_value = SignalValue::new("cert-pem-bytes");
    obs.write(ObservationRow::Signal {
        key: ProvisionRecordWithSignalEmit::signal_key(),
        value: signal_value.clone(),
    })
    .await
    .expect("write signal");

    let clock_a = Arc::new(SimClock::new());
    let engine_a = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_a.clone());
    let _emits_a = engine_a.take_action_emit_receiver().await.expect("emit receiver");
    engine_a.start(&spec, &correlation, &workflow_id).await.expect("start");
    let driver_a = Arc::clone(&clock_a);
    let ticker_a = tokio::spawn(async move {
        for _ in 0..16 {
            tokio::task::yield_now().await;
            driver_a.tick(Duration::from_millis(100));
        }
    });
    engine_a.join_all().await;
    let _ = ticker_a.await;

    // The signal was seen — the journal carries SignalSeen with the value.
    let after_run = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let seen = after_run.iter().find_map(|e| match e {
        LoadedEntry::Notification(JournalNotification::SignalSeen { value, .. }) => {
            Some(value.clone())
        }
        _ => None,
    });
    assert_eq!(
        seen,
        Some(signal_value.clone()),
        "the pre-crash run recorded SignalSeen with the observed value: {after_run:?}"
    );
    drop(engine_a);
    drop(clock_a);

    // ---- (2) Build a FRESH journal that ends right after SignalSeen (no
    //          Terminal): truncate the recorded journal to the point a
    //          crash-just-after-SignalSeen would leave it. ----
    let truncated: Vec<LoadedEntry> = after_run
        .iter()
        .take_while(|e| !matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
        .filter(|e| !matches!(e, LoadedEntry::Command(JournalCommand::ActionEmitted { .. })))
        .cloned()
        .collect();
    assert!(
        truncated.iter().any(|e| matches!(
            e,
            LoadedEntry::Notification(JournalNotification::SignalSeen { .. })
        )),
        "the truncated crash journal still carries SignalSeen: {truncated:?}"
    );
    let resume_journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    for entry in &truncated {
        resume_journal.append(&workflow_id, entry).await.expect("seed resume journal");
    }

    // ---- (3) Resume with the signal REMOVED from the surface. A re-read
    //          would block forever; replaying SignalSeen proceeds. ----
    let obs_no_signal: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0));
    let clock_b = Arc::new(SimClock::new());
    let engine_b =
        build_engine(Arc::clone(&resume_journal), Arc::clone(&obs_no_signal), clock_b.clone());
    let _emits_b = engine_b.take_action_emit_receiver().await.expect("emit receiver");
    let mut sub = obs_no_signal.subscribe_all_events().await.expect("subscribe");
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

    // OBSERVABLE — the resumed run reached a Completed terminal WITHOUT a
    // live signal row: the recorded SignalSeen value was replayed, not
    // re-read.
    let terminal = drain_terminal(&mut sub, &correlation).await;
    assert!(
        matches!(terminal, Some(WorkflowStatus::Completed { .. })),
        "resume replays the recorded SignalSeen and proceeds (Completed) — no re-block on absent \
         signal; got {terminal:?}"
    );
    // The resume did NOT append a second SignalAwaited/SignalSeen pair —
    // the recorded ones were replayed.
    let resumed = resume_journal.load_journal(&workflow_id).await.unwrap_or_default();
    let seen_count = resumed
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Notification(JournalNotification::SignalSeen { .. })))
        .count();
    assert_eq!(
        seen_count, 1,
        "exactly one SignalSeen — replay did not re-record the wait: {resumed:?}"
    );
}

async fn drain_terminal(
    sub: &mut LagAwareSubscription,
    correlation: &CorrelationKey,
) -> Option<WorkflowStatus> {
    use futures::StreamExt;
    for _ in 0..64 {
        match tokio::time::timeout(Duration::from_millis(50), sub.next()).await {
            Ok(Some(SubscriptionEvent::Row(ObservationRow::WorkflowTerminal {
                correlation: got,
                status,
            }))) if &got == correlation => {
                return Some(status);
            }
            // Single-workflow drain — lag is structurally impossible; surface it
            // loudly rather than skipping (a real lag would silently drop the
            // terminal row and fail the test for the wrong reason).
            Ok(Some(SubscriptionEvent::Lagged { missed })) => {
                panic!("subscription lagged ({missed}) draining a single workflow terminal row")
            }
            Ok(Some(SubscriptionEvent::Row(_)) | None) | Err(_) => {}
        }
    }
    None
}
