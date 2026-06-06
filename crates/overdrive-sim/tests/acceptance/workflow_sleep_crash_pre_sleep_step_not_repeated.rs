//! Slice 02 / AC1 — a waiting sequence survives a crash spanning the
//! sleep window without repeating the pre-sleep step.
//!
//! Scenario S-WP-02-01. K1 (O1). Under DST a `ctx.run → ctx.sleep →
//! ctx.run` sequence is killed DURING the sleep window and restarted on
//! the same node; the pre-sleep `ctx.run` executes exactly once on
//! resume (`SimTransport` call count == 1) and the sequence resumes the
//! remaining wait, not the whole sleep. ADR-0064 §3/§4 (`SleepArmed`
//! check-then-record).
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart on one node.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` (the resumed run, driven
//! off the shim); the "crash" between the first boot and the resume drives
//! the author body once via the engine's own `JournalCursorHandle` and
//! drops the future mid-sleep-park — a process-local kill modelled
//! honestly. The driven ports observed are the two bound `SimInbox`es (the
//! pre-sleep / post-sleep `SimTransport` effects) and the `SimJournalStore`
//! (`RunResult` + `SleepArmed` entries). The pre-sleep effect fires exactly
//! once across all boots — the resume replays the recorded pre-sleep
//! `RunResult` WITHOUT re-firing.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{
    JournalCursorHandle, WorkflowEngine, WorkflowRegistry,
};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::testing::workflow::ProvisionRecordWithSleep;
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{JournalCursor, WorkflowCtx, WorkflowStatus};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const PRE_TARGET: &str = "127.0.0.1:9100";
const POST_TARGET: &str = "127.0.0.1:9101";
/// The logical wait `ctx.sleep` arms between the two `ctx.run` effects.
const SLEEP: Duration = Duration::from_secs(30);

/// Build a `WorkflowEngine` over fresh `Sim*` ports (returning the
/// `SimClock` so the harness can advance logical time past the sleep
/// deadline), a SHARED journal + observation store, and freshly-bound
/// pre/post transport inboxes (so each "boot" observes its OWN
/// delivered-datagram counts). The engine resolves
/// `ProvisionRecordWithSleep` addressed at `PRE_TARGET` / `POST_TARGET`.
async fn engine_on(
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> (WorkflowEngine, Arc<SimClock>, SimInbox, SimInbox) {
    let pre: SocketAddr = PRE_TARGET.parse().expect("pre addr");
    let post: SocketAddr = POST_TARGET.parse().expect("post addr");
    let sim_transport = SimTransport::new();
    let pre_inbox = sim_transport.bind_inbox(pre).await.expect("bind pre inbox");
    let post_inbox = sim_transport.bind_inbox(post).await.expect("bind post inbox");

    let sim_clock = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = sim_clock.clone();
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecordWithSleep::spec().name, move || {
        ProvisionRecordWithSleep::new(pre, post, SLEEP)
    });

    let engine = WorkflowEngine::new(journal, clock, transport, entropy, registry, obs);
    (engine, sim_clock, pre_inbox, post_inbox)
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

/// Drive the engine to terminal, advancing the `SimClock` past the sleep
/// deadline on a concurrent task so the live/replay sleep park resolves —
/// the harness owns logical time (it never auto-advances inside `sleep`).
async fn drive_to_terminal(engine: &WorkflowEngine, clock: &Arc<SimClock>) {
    let driver = Arc::clone(clock);
    let ticker = tokio::spawn(async move {
        // Advance well past the deadline in a few ticks; each `tick`
        // wakes any parked `SimClock` timer whose deadline has passed.
        for _ in 0..8 {
            tokio::task::yield_now().await;
            driver.tick(SLEEP);
        }
    });
    engine.join_all().await;
    ticker.await.expect("ticker task");
}

/// Drive the author body once via a raw ctx — the pre-sleep `ctx.run`
/// records (fires the pre effect once) and the `ctx.sleep` arms its
/// `SleepArmed` deadline and parks — then drop the future mid-park WITHOUT
/// advancing logical time. A process-local kill DURING the sleep window,
/// modelled honestly. Returns the pre/post effect-fire counts of this run
/// (the journal carries the pre-sleep `RunResult` + `SleepArmed`, no
/// post-sleep run, no `Terminal`).
async fn crash_during_sleep(
    journal: &Arc<dyn JournalStore>,
    workflow_id: &WorkflowId,
) -> (usize, usize) {
    let pre: SocketAddr = PRE_TARGET.parse().expect("pre addr");
    let post: SocketAddr = POST_TARGET.parse().expect("post addr");
    let crash_transport = SimTransport::new();
    let mut pre_inbox = crash_transport.bind_inbox(pre).await.expect("bind crash pre");
    let mut post_inbox = crash_transport.bind_inbox(post).await.expect("bind crash post");
    {
        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
            Arc::clone(journal),
            workflow_id.clone(),
            Vec::new(),
        ));
        let ctx = WorkflowCtx::new(
            Arc::new(SimClock::new()),
            Arc::new(crash_transport) as Arc<dyn Transport>,
            Arc::new(SimEntropy::new(0x5eed)),
            cursor,
        );
        // Pre-sleep durable step — the SAME `ctx.run` the author body runs
        // (records step 0, fires the pre effect once).
        let pre_transport = Arc::clone(ctx.transport());
        let pre_payload = Bytes::from_static(ProvisionRecordWithSleep::FIRST_PAYLOAD);
        let recorded: Result<usize, String> = ctx
            .run("provision-write-pre-sleep", async move {
                pre_transport.send_datagram(pre, pre_payload).await.map_err(|e| e.to_string())
            })
            .await
            .expect("crash-run records pre-sleep step");
        assert!(recorded.is_ok(), "the pre-sleep effect succeeded before the crash");

        // Arm the sleep + park, then "crash" mid-park: spawn the sleep so it
        // appends `SleepArmed` and parks (logical time NOT advanced), then
        // abort the task — the future is dropped while still parked, exactly
        // a kill DURING the sleep window.
        let sleeper = tokio::spawn(async move { ctx.sleep(SLEEP).await });
        tokio::task::yield_now().await;
        sleeper.abort();
        let _ = sleeper.await;
        // <-- "crash": the ctx + sleep future are gone, BEFORE the post-sleep
        //     run. The journal holds pre-sleep RunResult + SleepArmed only.
    }
    (delivered_count(&mut pre_inbox).await, delivered_count(&mut post_inbox).await)
}

#[tokio::test]
async fn crash_during_sleep_window_does_not_repeat_the_pre_sleep_step() {
    let correlation = CorrelationKey::derive(
        "wf-provision-sleep-0001",
        &ContentHash::of(ProvisionRecordWithSleep::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-sleep-0001").expect("valid id");
    let spec = ProvisionRecordWithSleep::spec();

    // ---- Crash run: pre-sleep `ctx.run` records + `ctx.sleep` arms its
    //      `SleepArmed` deadline and parks, then the future is dropped
    //      mid-park (a process-local kill DURING the sleep window). The
    //      journal carries the pre-sleep RunResult + SleepArmed, NO
    //      post-sleep RunResult, NO Terminal. ----
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node"), 0));
    let (crash_pre_fires, crash_post_fires) = crash_during_sleep(&journal, &workflow_id).await;
    assert_eq!(crash_pre_fires, 1, "the pre-crash run fired the pre-sleep effect once");
    assert_eq!(crash_post_fires, 0, "the crash happened DURING the sleep — post-sleep never fired");

    let pre_resume = journal.load_journal(&workflow_id).await.expect("load crash journal");
    assert!(
        pre_resume
            .iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::RunResult { .. }))),
        "the crash left a recorded pre-sleep RunResult: {pre_resume:?}"
    );
    assert!(
        pre_resume
            .iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::SleepArmed { .. }))),
        "the crash spanned the sleep window — SleepArmed is recorded: {pre_resume:?}"
    );
    assert!(
        !pre_resume
            .iter()
            .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. }))),
        "the crash happened BEFORE terminal — no Terminal entry: {pre_resume:?}"
    );

    // ---- Resume on the SAME node from the persisted journal. The engine
    //      load_journals the pre-sleep RunResult + SleepArmed into the
    //      replay buffer; the pre-sleep `ctx.run` short-circuits (replay,
    //      no re-fire), the sleep recomputes the remaining wait from the
    //      recorded deadline, and the post-sleep run fires live. ----
    let (engine, clock, mut resume_pre_inbox, mut resume_post_inbox) =
        engine_on(Arc::clone(&journal), Arc::clone(&obs)).await;
    let mut sub = obs.subscribe_all().await.expect("subscribe resume");
    engine.start(&spec, &correlation, &workflow_id).await.expect("start resume");
    drive_to_terminal(&engine, &clock).await;

    // K1 / O1 — the pre-sleep `ctx.run` executes EXACTLY ONCE across all
    // boots: the resumed boot re-fired the pre-sleep effect ZERO times (the
    // recorded pre-sleep RunResult was replayed). The crash run's single
    // fire is the only pre-sleep fire.
    let resume_pre_fires = delivered_count(&mut resume_pre_inbox).await;
    assert_eq!(
        resume_pre_fires, 0,
        "resume must NOT repeat the pre-sleep step — it is replayed from the journal (K1)"
    );
    // The post-sleep step is fresh-live on resume (recorded nowhere before
    // the crash), so it fires exactly once on this boot.
    let resume_post_fires = delivered_count(&mut resume_post_inbox).await;
    assert_eq!(
        resume_post_fires, 1,
        "the post-sleep step fires once on resume (it was never recorded)"
    );

    // The resumed run reached terminal Success — bounded progress across the
    // crash + remaining-wait resume.
    let resumed = journal.load_journal(&workflow_id).await.expect("load resumed");
    assert!(
        resumed.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. }))),
        "the resumed run reached terminal: {resumed:?}"
    );
    // No second pre-sleep RunResult was appended — the recorded one was
    // replayed (the structural witness of exactly-once on the pre-sleep step).
    let pre_sleep_runs = resumed
        .iter()
        .filter(|e| {
            matches!(
                e,
                LoadedEntry::Command(JournalCommand::RunResult { name, .. })
                    if name == "provision-write-pre-sleep"
            )
        })
        .count();
    assert_eq!(
        pre_sleep_runs, 1,
        "resume replayed the recorded pre-sleep RunResult — no second one appended: {resumed:?}"
    );

    // The terminal observation row records a Completed status (driven-port
    // outcome — `ProvisionRecordWithSleep`'s `Output = ()`).
    let mut terminal_seen = false;
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_secs(1), futures::StreamExt::next(&mut sub)).await
        {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation: got, status }))
                if got == correlation =>
            {
                assert!(
                    matches!(status, WorkflowStatus::Completed { .. }),
                    "the resumed run terminates Completed, got {status:?}"
                );
                terminal_seen = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    assert!(terminal_seen, "a WorkflowTerminal row arrived for the resumed instance");
}
