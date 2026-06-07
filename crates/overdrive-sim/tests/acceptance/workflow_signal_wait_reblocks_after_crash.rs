//! Slice 03 / US-WP-5 AC1 — a sequence blocked on a signal re-blocks on
//! the SAME signal after a crash; slice-03 AC1.
//!
//! Scenario S-WP-03-01. K1 (O1). A `ProvisionRecordWithSignalEmit`
//! instance blocked on `ctx.wait_for_signal(key)` (signal NOT yet present
//! in the `ObservationStore`) is killed while blocked and restarted on the
//! same node; on resume it blocks on the SAME signal (neither lost nor
//! satisfied prematurely), then proceeds only once the signal arrives —
//! and no duplicate downstream effect occurs. ADR-0066 §2
//! (`SignalAwaited`), ADR-0064 §3/§4.
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart; in-process
//! single-node signal delivery via the `ObservationStore` (#207 defers
//! cross-node partition semantics).
//!
//! # Port-to-port
//!
//! The driving port is `ctx.wait_for_signal` via the `WorkflowEngine`; the
//! observable outcomes are asserted at the driven-port boundaries: the
//! `JournalStore` (a `SignalAwaited` with NO following `SignalSeen` after
//! the crash — the genuine "blocked, not satisfied" shape), the engine's
//! live-task set (the run future stays pending while the signal is
//! absent), and the Action channel (exactly ONE emit across the crash).
//!
//! # Falsifiability
//!
//! If `wait_for_signal` did NOT genuinely block on an absent signal (the
//! 03-01 placeholder that resolved immediately), the pre-crash run would
//! reach terminal and emit — the `did_not_terminate_while_blocked` and
//! `reblocks_on_the_same_signal` assertions would fail. Removing the block
//! reds this test.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};

/// Whether `run` contains a `SignalAwaited` command. Helper so the
/// assertions stay one-liners (the typed matcher would otherwise wrap
/// across three lines each, pushing the test body past the line cap).
fn has_signal_awaited(run: &[LoadedEntry]) -> bool {
    run.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::SignalAwaited { .. })))
}

/// Whether `run` contains a `SignalSeen` notification.
fn has_signal_seen(run: &[LoadedEntry]) -> bool {
    run.iter()
        .any(|e| matches!(e, LoadedEntry::Notification(JournalNotification::SignalSeen { .. })))
}
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::testing::workflow::ProvisionRecordWithSignalEmit;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::transport::Transport as TransportTrait;
use overdrive_core::workflow::{SignalValue, WorkflowStatus};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

const SEED: u64 = 0x0302_5165_a1ad_0001;

fn instance() -> (CorrelationKey, WorkflowId) {
    let correlation = CorrelationKey::derive(
        "wf-signal-reblock-0001",
        &ContentHash::of(ProvisionRecordWithSignalEmit::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-signal-reblock-0001").expect("valid instance id");
    (correlation, workflow_id)
}

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

async fn drain_emits(
    rx: &mut overdrive_control_plane::workflow_runtime::ActionEmitReceiver,
) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
        count += 1;
    }
    count
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_while_blocked_on_signal_reblocks_on_the_same_signal_on_resume() {
    let (correlation, workflow_id) = instance();
    let spec = ProvisionRecordWithSignalEmit::spec();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0));

    // ---- (1) Pre-crash run: start blocked on the ABSENT signal. ----
    let clock_a = Arc::new(SimClock::new());
    let engine_a = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_a.clone());
    let mut emits_a = engine_a.take_action_emit_receiver().await.expect("emit receiver");
    engine_a.start(&spec, &correlation, &workflow_id).await.expect("start");

    // Give the engine task time to record SignalAwaited and park on the
    // absent signal, advancing logical time (which must NOT satisfy the
    // wait — only a written signal row does).
    for _ in 0..8 {
        tokio::task::yield_now().await;
        clock_a.tick(Duration::from_millis(100));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // OBSERVABLE 1 — the run is STILL BLOCKED (the future is pending): the
    // engine reports the instance as live, no terminal row exists, and NO
    // Action was emitted. This is the genuine block on an absent signal.
    let live_while_blocked = engine_a.live_instances();
    assert!(
        live_while_blocked.contains(&correlation),
        "the run must STILL be blocked on the absent signal (engine task live)"
    );
    let terminals_while_blocked = obs.workflow_terminal_rows().await.expect("terminal rows");
    assert!(
        terminals_while_blocked.is_empty(),
        "a blocked-on-signal run must NOT have reached terminal: {terminals_while_blocked:?}"
    );
    let emits_while_blocked = drain_emits(&mut emits_a).await;
    assert_eq!(
        emits_while_blocked, 0,
        "a blocked-on-signal run must NOT have emitted its downstream Action yet"
    );

    // OBSERVABLE 2 — the journal carries SignalAwaited with NO SignalSeen
    // (blocked, not satisfied). This is the crash-while-blocked shape.
    let pre_crash = journal.load_journal(&workflow_id).await.unwrap_or_default();
    assert!(has_signal_awaited(&pre_crash), "blocking must record SignalAwaited: {pre_crash:?}");
    assert!(
        !has_signal_seen(&pre_crash),
        "a still-blocked run must have NO SignalSeen recorded: {pre_crash:?}"
    );

    // ---- (2) Crash: drop the engine (and its task) mid-block. ----
    drop(emits_a);
    drop(engine_a);
    drop(clock_a);

    // ---- (3) Resume on a fresh engine over the SAME journal + obs. ----
    let clock_b = Arc::new(SimClock::new());
    let engine_b = build_engine(Arc::clone(&journal), Arc::clone(&obs), clock_b.clone());
    let mut emits_b = engine_b.take_action_emit_receiver().await.expect("emit receiver");
    let mut sub = obs.subscribe_all().await.expect("subscribe");
    engine_b.start(&spec, &correlation, &workflow_id).await.expect("resume start");

    // Advance logical time on the resumed run — it must RE-BLOCK on the
    // SAME absent signal (not resolve prematurely, not lose the wait).
    for _ in 0..6 {
        tokio::task::yield_now().await;
        clock_b.tick(Duration::from_millis(100));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // OBSERVABLE 3 — RE-BLOCKED on the SAME signal: the resumed run is
    // still live, no terminal, no emit. The wait survived the crash.
    assert!(
        engine_b.live_instances().contains(&correlation),
        "on resume the run must RE-BLOCK on the same absent signal"
    );
    assert_eq!(
        drain_emits(&mut emits_b).await,
        0,
        "the resumed re-blocked run must NOT emit before the signal arrives"
    );

    // ---- (4) Satisfy the signal: now the resumed run completes once. ----
    obs.write(ObservationRow::Signal {
        key: ProvisionRecordWithSignalEmit::signal_key(),
        value: SignalValue::new("ready"),
    })
    .await
    .expect("write signal");
    // Advance time so the resumed run's poll observes the signal + proceeds.
    let driver = Arc::clone(&clock_b);
    let ticker = tokio::spawn(async move {
        for _ in 0..16 {
            tokio::task::yield_now().await;
            driver.tick(Duration::from_millis(100));
        }
    });
    engine_b.join_all().await;
    let _ = ticker.await;

    // OBSERVABLE 4 — the resumed run reached a Completed terminal, and
    // exactly ONE Action was emitted across the WHOLE history (pre-crash 0 +
    // resume 1). No duplicate downstream effect (K1 / US-WP-5 AC1).
    let terminal = drain_terminal(&mut sub, &correlation).await;
    assert!(
        matches!(terminal, Some(WorkflowStatus::Completed { .. })),
        "once the signal arrives the resumed run completes (Completed), got {terminal:?}"
    );
    let resumed = journal.load_journal(&workflow_id).await.unwrap_or_default();
    assert!(
        has_signal_seen(&resumed),
        "the satisfied wait records SignalSeen on resume: {resumed:?}"
    );
    // Exactly ONE SignalAwaited across the whole history — the resume did
    // NOT append a duplicate (it advanced past the recorded one).
    let awaited_count = resumed
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::SignalAwaited { .. })))
        .count();
    assert_eq!(
        awaited_count, 1,
        "resume re-blocks on the SAME SignalAwaited — no duplicate appended: {resumed:?}"
    );
    let total_emits = drain_emits(&mut emits_b).await + emits_while_blocked;
    assert_eq!(
        total_emits, 1,
        "exactly ONE downstream Action across the crash (no duplicate effect)"
    );
}

async fn drain_terminal(
    sub: &mut overdrive_core::traits::observation_store::ObservationSubscription,
    correlation: &CorrelationKey,
) -> Option<WorkflowStatus> {
    use futures::StreamExt;
    for _ in 0..64 {
        match tokio::time::timeout(Duration::from_millis(50), sub.next()).await {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation: got, status }))
                if &got == correlation =>
            {
                return Some(status);
            }
            Ok(Some(_) | None) | Err(_) => {}
        }
    }
    None
}
