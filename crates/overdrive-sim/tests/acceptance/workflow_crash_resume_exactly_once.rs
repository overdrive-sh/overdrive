//! Slice 01 — WALKING SKELETON. US-WP-3 AC1 (O1) / AC2 (O4) / AC3 (O2
//! single-node) / AC4 (re-hydrate); slice-01 AC1/AC2/AC5.
//!
//! Scenario S-WP-01-06 — the headline durable-execution journey: kill the
//! process AFTER `ctx.call` records but BEFORE terminal, restart on the
//! SAME node, and the external effect is NOT repeated (`SimTransport` call
//! count == 1), the resumed `WorkflowResult` is byte-identical to the
//! uninterrupted run, and the `ObservationStore` carries a terminal-result
//! row keyed by `CorrelationKey`. This is the `WorkflowExactlyOnceEffect
//! OnResume` DST invariant (ADR-0064 §6). K1(O1), K3(O4), K2(O2
//! single-node).
//!
//! SINGLE-NODE SCOPE (D3 / #205): the kill-and-restart is process-local
//! on ONE node. No cross-node resume is claimed; the redb-journal design
//! does not preclude it but it is not demonstrated across nodes here.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` (the async executor driven
//! off the shim) for the uninterrupted + resumed runs; the "crash" between
//! them drives `WorkflowCtx::call` once via the engine's own
//! `JournalCursorHandle` and then drops the future before terminal — a
//! process-local kill modelled honestly (no production crash-hook). The
//! driven ports observed are the bound `SimInbox` (the `SimTransport`
//! effect: exactly one datagram delivered across all three runs) and the
//! `SimObservationStore` (the `WorkflowTerminal` row keyed by
//! `CorrelationKey`).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{
    JournalCursorHandle, WorkflowEngine, WorkflowRegistry,
};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationSubscription,
};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{CallRequest, JournalCursor, WorkflowCtx, WorkflowResult};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const TARGET: &str = "127.0.0.1:9000";

/// Build a `WorkflowEngine` over fresh `Sim*` ports, a SHARED journal +
/// observation store, and a freshly-bound transport inbox (so each "boot"
/// observes its OWN delivered-datagram count — the per-boot effect-fire
/// surface). The engine resolves `ProvisionRecord` addressed at `TARGET`.
async fn engine_on(
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> (WorkflowEngine, SimInbox) {
    let target: SocketAddr = TARGET.parse().expect("addr");
    let sim_transport = SimTransport::new();
    let inbox = sim_transport.bind_inbox(target).await.expect("bind inbox");

    let transport: Arc<dyn Transport> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || Box::new(ProvisionRecord::new(target)));

    let engine = WorkflowEngine::new(journal, clock, transport, entropy, registry, obs);
    (engine, inbox)
}

/// Drain the `WorkflowTerminal` row for `correlation` off a subscription
/// taken BEFORE the run drove to terminal. Returns the terminal result.
async fn terminal_result(
    subscription: &mut ObservationSubscription,
    correlation: &CorrelationKey,
) -> WorkflowResult {
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_secs(1), subscription.next()).await {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation: got, result }))
                if &got == correlation =>
            {
                return result;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    panic!("no WorkflowTerminal row arrived for {correlation:?}");
}

/// Count datagrams currently sitting in `inbox` without blocking past the
/// drain budget — the per-boot `SimTransport` effect-fire count.
async fn delivered_count(inbox: &mut SimInbox) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await {
        count += 1;
    }
    count
}

#[tokio::test]
async fn killing_after_step_records_does_not_repeat_the_effect_on_resume() {
    let correlation = CorrelationKey::derive(
        "wf-provision-0001",
        &ContentHash::of(ProvisionRecord::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-0001").expect("valid id");
    let spec = ProvisionRecord::spec();

    // ---- Run A: uninterrupted, capturing the terminal trajectory ----
    let journal_a: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs_a: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node"), 0));
    let (engine_a, mut inbox_a) = engine_on(Arc::clone(&journal_a), Arc::clone(&obs_a)).await;
    let mut sub_a = obs_a.subscribe_all().await.expect("subscribe");

    engine_a.start(&spec, &correlation, &workflow_id).await.expect("start A");
    engine_a.join_all().await;

    let uninterrupted_result = terminal_result(&mut sub_a, &correlation).await;
    let uninterrupted_journal = journal_a.load_journal(&workflow_id).await.expect("load A");
    let uninterrupted_fires = delivered_count(&mut inbox_a).await;
    assert_eq!(uninterrupted_fires, 1, "uninterrupted run fires the effect exactly once");

    // ---- Run B: crash — drive ctx.call once (records step 0), drop the
    //      future BEFORE terminal. A process-local kill modelled honestly:
    //      the journal carries the CallResult but NO Terminal entry. ----
    let journal_b: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs_b: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node"), 0));
    let crash_target: SocketAddr = TARGET.parse().expect("addr");
    let sim_transport_b = SimTransport::new();
    let mut inbox_b = sim_transport_b.bind_inbox(crash_target).await.expect("bind B");
    {
        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
            Arc::clone(&journal_b),
            workflow_id.clone(),
            Vec::new(),
        ));
        let ctx = WorkflowCtx::new(
            Arc::new(SimClock::new()),
            Arc::new(sim_transport_b) as Arc<dyn Transport>,
            Arc::new(SimEntropy::new(0x5eed)),
            cursor,
        );
        let request = CallRequest {
            target: crash_target,
            payload: Bytes::from_static(ProvisionRecord::PAYLOAD),
        };
        ctx.call(request).await.expect("crash-run records step 0");
        // <-- "crash": ctx + future dropped here, BEFORE the workflow
        //     would have returned terminal. No Terminal entry written.
    }
    let crash_fires = delivered_count(&mut inbox_b).await;
    assert_eq!(crash_fires, 1, "the pre-crash run fired the effect once (the recorded step)");

    let pre_resume_journal = journal_b.load_journal(&workflow_id).await.expect("load B");
    assert!(
        pre_resume_journal.iter().any(|e| matches!(e, JournalEntry::CallResult { .. })),
        "the crash left a recorded CallResult: {pre_resume_journal:?}"
    );
    assert!(
        !pre_resume_journal.iter().any(|e| matches!(e, JournalEntry::Terminal { .. })),
        "the crash happened BEFORE terminal — no Terminal entry: {pre_resume_journal:?}"
    );

    // ---- Run C: RESUME on the SAME node from the persisted journal_b.
    //      The engine load_journals the CallResult into the replay buffer;
    //      ctx.call short-circuits (replay) WITHOUT re-firing the effect. ----
    let (engine_c, mut inbox_c) = engine_on(Arc::clone(&journal_b), Arc::clone(&obs_b)).await;
    let mut sub_c = obs_b.subscribe_all().await.expect("subscribe C");

    engine_c.start(&spec, &correlation, &workflow_id).await.expect("start C (resume)");
    engine_c.join_all().await;

    // K1 / O1 — EXACTLY ONCE: the resumed boot fired the effect ZERO
    // additional times (replay short-circuited the recorded call). Across
    // all boots the effect fired exactly once (crash run's single fire).
    let resume_fires = delivered_count(&mut inbox_c).await;
    assert_eq!(
        resume_fires, 0,
        "resume must NOT re-fire the recorded ctx.call effect (exactly-once on resume)"
    );

    // K3 / O4 — the resumed WorkflowResult is byte-identical to the
    // uninterrupted run.
    let resumed_result = terminal_result(&mut sub_c, &correlation).await;
    assert_eq!(
        resumed_result, uninterrupted_result,
        "resumed terminal result must be byte-identical to the uninterrupted run"
    );
    assert_eq!(resumed_result, WorkflowResult::Success, "ProvisionRecord runs to Success");

    // AC5 / O2 — the resumed run reached terminal and the journal now
    // carries a Terminal entry alongside the (single, replayed) CallResult.
    let resumed_journal = journal_b.load_journal(&workflow_id).await.expect("load C");
    assert!(
        resumed_journal.iter().any(|e| matches!(e, JournalEntry::Terminal { .. })),
        "the resumed run reached terminal: {resumed_journal:?}"
    );
    let call_results =
        resumed_journal.iter().filter(|e| matches!(e, JournalEntry::CallResult { .. })).count();
    assert_eq!(
        call_results, 1,
        "resume must not append a second CallResult — the recorded one is replayed: {resumed_journal:?}"
    );

    // Cross-run journal byte-equality on the recorded call: the resumed
    // run's CallResult is the one the crash recorded (K2 — committed step
    // not lost), and matches the uninterrupted run's recorded call.
    let recorded_call = |run: &[JournalEntry]| -> Option<JournalEntry> {
        run.iter().find(|e| matches!(e, JournalEntry::CallResult { .. })).cloned()
    };
    assert_eq!(
        recorded_call(&resumed_journal),
        recorded_call(&uninterrupted_journal),
        "the replayed CallResult is byte-equal to the uninterrupted run's recorded call"
    );
}
