//! Slice 02 / AC2 — the post-sleep step fires only at/after the original
//! deadline, regardless of crash timing.
//!
//! Scenario S-WP-02-02. K3 (O4). A sequence suspended on `ctx.sleep` with
//! a recorded deadline is crashed at an arbitrary point in the sleep
//! window and resumed (`SimClock` advances logical time); the post-sleep
//! `ctx.run` fires only at/after the ORIGINAL deadline, never earlier,
//! and the terminal result is unchanged by the crash timing. ADR-0063 §2
//! (`SleepArmed { deadline_unix }`), ADR-0064 §3.
//!
//! # Port-to-port
//!
//! The driving port is the author body run via the engine's own
//! `JournalCursorHandle` (the resume cursor seeded with the recorded
//! pre-sleep `RunResult` + `SleepArmed`). The observable outcome is at the
//! driven-port boundary: the post-sleep `SimInbox` receives ZERO datagrams
//! while the resume clock is held BEFORE the recorded deadline, and exactly
//! one once logical time reaches the original deadline. The recorded
//! `SleepArmed { deadline_unix }` is an INPUT (ADR-0063 §2); the resume
//! recomputes the remaining wait from `recorded_deadline − clock.now()`,
//! never from when the crash occurred.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use overdrive_core::testing::workflow::ProvisionRecordWithSleep;
use overdrive_core::traits::Clock;
use overdrive_core::workflow::{JournalCursor, WorkflowCtx, WorkflowResult};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const PRE_TARGET: &str = "127.0.0.1:9200";
const POST_TARGET: &str = "127.0.0.1:9201";
/// The logical wait `ctx.sleep` arms between the two `ctx.run` effects.
const SLEEP: Duration = Duration::from_secs(30);

/// Build a `WorkflowCtx` over a SHARED journal + a SHARED `SimClock`,
/// seeded with `replay_buffer` (empty on the arming boot; the persisted
/// pre-sleep run + `SleepArmed` on resume). Returns the bound pre/post
/// inboxes so the test observes each effect's fire count.
async fn ctx_on(
    journal: &Arc<dyn JournalStore>,
    clock: Arc<dyn Clock>,
    workflow_id: &WorkflowId,
    replay_buffer: Vec<JournalEntry>,
) -> (WorkflowCtx, SimInbox, SimInbox) {
    let pre: SocketAddr = PRE_TARGET.parse().expect("pre addr");
    let post: SocketAddr = POST_TARGET.parse().expect("post addr");
    let transport = SimTransport::new();
    let pre_inbox = transport.bind_inbox(pre).await.expect("bind pre");
    let post_inbox = transport.bind_inbox(post).await.expect("bind post");
    let cursor: Arc<dyn JournalCursor> =
        Arc::new(JournalCursorHandle::new(Arc::clone(journal), workflow_id.clone(), replay_buffer));
    let ctx =
        WorkflowCtx::new(clock, Arc::new(transport), Arc::new(SimEntropy::new(0x5eed)), cursor);
    (ctx, pre_inbox, post_inbox)
}

/// Drive `ProvisionRecordWithSleep`'s author body against `ctx`. Returns
/// the terminal `WorkflowResult` once the sleep park resolves.
async fn run_body(ctx: WorkflowCtx) -> WorkflowResult {
    use overdrive_core::workflow::Workflow;
    let pre: SocketAddr = PRE_TARGET.parse().expect("pre addr");
    let post: SocketAddr = POST_TARGET.parse().expect("post addr");
    let workflow = ProvisionRecordWithSleep::new(pre, post, SLEEP);
    workflow.run(&ctx).await
}

/// Count datagrams currently sitting in `inbox` without blocking past the
/// drain budget.
async fn delivered_count(inbox: &mut SimInbox) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await {
        count += 1;
    }
    count
}

#[tokio::test]
async fn post_sleep_step_fires_only_at_or_after_the_original_deadline_regardless_of_crash_timing() {
    let workflow_id = WorkflowId::new("wf-provision-sleep-deadline-0001").expect("valid id");
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());

    // ---- Arming boot: drive the body until it parks on `ctx.sleep`, then
    //      crash mid-park. Records the pre-sleep RunResult + SleepArmed
    //      (deadline = arm_now + SLEEP), no post-sleep run, no Terminal. ----
    let arm_clock = Arc::new(SimClock::new());
    let recorded_deadline = arm_clock.unix_now() + SLEEP;
    {
        let (ctx, mut arm_pre, mut arm_post) =
            ctx_on(&journal, arm_clock.clone() as Arc<dyn Clock>, &workflow_id, Vec::new()).await;
        let body = tokio::spawn(run_body(ctx));
        // Yield enough for the pre-sleep run to record and the sleep to arm
        // + park; logical time is NOT advanced, so the body stays parked.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        body.abort();
        let _ = body.await;
        assert_eq!(
            delivered_count(&mut arm_pre).await,
            1,
            "pre-sleep fired once on the arming boot"
        );
        assert_eq!(
            delivered_count(&mut arm_post).await,
            0,
            "post-sleep never fired before the crash"
        );
    }

    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    let deadline_in_journal = loaded
        .iter()
        .find_map(|e| match e {
            JournalEntry::SleepArmed { deadline_unix, .. } => Some(*deadline_unix),
            _ => None,
        })
        .expect("the arming boot recorded a SleepArmed deadline");
    assert_eq!(
        deadline_in_journal, recorded_deadline,
        "SleepArmed records the ORIGINAL absolute deadline (an input)"
    );
    assert!(
        !loaded.iter().any(|e| matches!(e, JournalEntry::Terminal { .. })),
        "crashed mid-sleep — no Terminal recorded yet"
    );

    // ---- Resume: a fresh boot whose `SimClock` starts at logical zero
    //      (NOT at the crash time). The replay recomputes the remaining
    //      wait as `recorded_deadline − clock.now()`. Hold the clock just
    //      SHORT of the original deadline and prove the post-sleep step
    //      does NOT fire; then advance to the original deadline and prove
    //      it fires exactly once. The crash timing (arbitrary) does not
    //      enter this computation — only the recorded deadline does. ----
    let resume_clock = Arc::new(SimClock::new());
    let (ctx, mut resume_pre, mut resume_post) =
        ctx_on(&journal, resume_clock.clone() as Arc<dyn Clock>, &workflow_id, loaded.clone())
            .await;
    let body = tokio::spawn(run_body(ctx));

    // Advance to ONE SECOND BEFORE the original deadline. The post-sleep
    // step must stay un-fired: the remaining wait is recomputed from the
    // recorded deadline, so it is not yet elapsed.
    let almost = recorded_deadline
        .saturating_sub(resume_clock.unix_now())
        .saturating_sub(Duration::from_secs(1));
    resume_clock.tick(almost);
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        delivered_count(&mut resume_post).await,
        0,
        "K3 — the post-sleep step does NOT fire before the ORIGINAL recorded deadline",
    );
    // The pre-sleep step is replayed on resume — it does not re-fire.
    assert_eq!(
        delivered_count(&mut resume_pre).await,
        0,
        "the pre-sleep step is replayed on resume, never re-fired",
    );

    // Advance the final second to reach the ORIGINAL deadline. Now the
    // remaining wait elapses and the post-sleep step fires exactly once.
    resume_clock.tick(Duration::from_secs(1));
    let terminal = tokio::time::timeout(Duration::from_secs(2), body)
        .await
        .expect("resume body resolves once logical time reaches the original deadline")
        .expect("body task");
    assert_eq!(terminal, WorkflowResult::Success, "the resumed run terminates Success");
    assert_eq!(
        delivered_count(&mut resume_post).await,
        1,
        "the post-sleep step fires exactly once, at the original deadline",
    );
}
