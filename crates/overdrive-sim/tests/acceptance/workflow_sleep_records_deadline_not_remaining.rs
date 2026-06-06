//! Slice 02 / AC4 — the sleep journal entry records the deadline (an
//! input), never a "remaining" cache.
//!
//! Scenario S-WP-02-03. O3/O6. Per `development.md` "Persist inputs, not
//! derived state": the `SleepArmed` entry carries `deadline_unix` (an
//! input) and no persisted "remaining duration" field — resume recomputes
//! `recorded_deadline − clock.now()`. ADR-0063 §2.
//!
//! # Port-to-port
//!
//! Driving port: `WorkflowCtx::sleep` (the slice-02 await-surface) over a
//! durable `JournalCursorHandle` whose driven port is the
//! `SimJournalStore`. The observable outcome is the `SleepArmed` entry the
//! live `ctx.sleep` appended — it carries `deadline_unix` (an input) and
//! exposes NO derived "remaining" slot — plus the resume behaviour: a
//! second boot whose `SimClock` is already past the recorded deadline
//! returns from `ctx.sleep` immediately (deadline-passed replay), and a
//! boot whose clock has NOT reached the deadline recomputes the remaining
//! wait from `recorded_deadline − clock.now()` (never a persisted
//! remaining cache).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use overdrive_core::traits::Clock;
use overdrive_core::workflow::{JournalCursor, WorkflowCtx};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::SimTransport;

/// The logical wait the workflow arms via `ctx.sleep`.
const SLEEP: Duration = Duration::from_secs(30);

/// Build a `WorkflowCtx` over a SHARED journal + a SHARED clock, with the
/// cursor seeded with `replay_buffer` (empty on first boot; the persisted
/// run on resume).
fn ctx_on(
    journal: &Arc<dyn JournalStore>,
    clock: Arc<dyn Clock>,
    workflow_id: &WorkflowId,
    replay_buffer: Vec<LoadedEntry>,
) -> WorkflowCtx {
    let cursor: Arc<dyn JournalCursor> =
        Arc::new(JournalCursorHandle::new(Arc::clone(journal), workflow_id.clone(), replay_buffer));
    WorkflowCtx::new(clock, Arc::new(SimTransport::new()), Arc::new(SimEntropy::new(0)), cursor)
}

#[tokio::test]
async fn sleep_armed_journal_entry_records_deadline_input_not_a_remaining_duration_cache() {
    let store = Arc::new(SimJournalStore::new());
    let journal: Arc<dyn JournalStore> = store.clone();
    let workflow_id = WorkflowId::new("wf-provision-sleep-0001").expect("valid workflow id");

    // --- First boot: arm the sleep on the live path. ---
    let clock = Arc::new(SimClock::new());
    let arm_deadline_unix = clock.unix_now() + SLEEP;
    {
        let ctx = ctx_on(&journal, clock.clone() as Arc<dyn Clock>, &workflow_id, Vec::new());
        // The live `ctx.sleep` writes a `SleepArmed` entry (deadline as an
        // input) then parks on the Clock deadline. Drive the park to
        // completion by advancing logical time past the deadline on a
        // concurrent task — the harness owns logical time.
        let driver = clock.clone();
        let sleeper = tokio::spawn(async move { ctx.sleep(SLEEP).await });
        // Yield so the sleeper appends `SleepArmed` and parks before we
        // advance; then advance past the deadline to wake it.
        tokio::task::yield_now().await;
        driver.tick(SLEEP);
        sleeper.await.expect("sleeper task").expect("live ctx.sleep arms and parks");
    }

    // Observable outcome 1 — the live path appended exactly one
    // `SleepArmed` command carrying `deadline_unix` (an INPUT). The
    // `JournalCommand` enum has NO "remaining_ms" / "remaining_duration"
    // slot AND no in-entry `step` (D5) by construction; we assert
    // positively on the recorded deadline.
    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(loaded.len(), 1, "live ctx.sleep appended exactly one SleepArmed entry");
    let recorded_deadline = match &loaded[0] {
        LoadedEntry::Command(JournalCommand::SleepArmed { deadline_unix }) => {
            assert_eq!(
                *deadline_unix, arm_deadline_unix,
                "SleepArmed records the absolute deadline (an input), not a remaining-duration cache",
            );
            *deadline_unix
        }
        other => panic!("first entry must be SleepArmed, got {other:?}"),
    };

    // --- Resume A: a boot whose clock is already PAST the recorded
    // deadline. `ctx.sleep` returns immediately (deadline-passed replay) —
    // no re-park, no second SleepArmed entry. ---
    let past_clock = Arc::new(SimClock::new());
    past_clock.tick(SLEEP + Duration::from_secs(5)); // now() > recorded deadline
    assert!(
        past_clock.unix_now() >= recorded_deadline,
        "resume-A clock is past the recorded deadline",
    );
    {
        let ctx =
            ctx_on(&journal, past_clock.clone() as Arc<dyn Clock>, &workflow_id, loaded.clone());
        // Must complete WITHOUT the harness advancing time — the replay
        // path sees the recorded deadline has passed and returns at once.
        tokio::time::timeout(Duration::from_secs(1), ctx.sleep(SLEEP))
            .await
            .expect("resume past-deadline ctx.sleep returns immediately, no re-park")
            .expect("replay ctx.sleep ok");
    }

    // Observable outcome 2 — the replay path neither re-armed nor mutated
    // the journal: still exactly the one original `SleepArmed` entry.
    let after_resume = journal.load_journal(&workflow_id).await.expect("reload journal");
    assert_eq!(
        after_resume, loaded,
        "deadline-passed replay re-records nothing — the recorded run is unchanged",
    );

    // --- Resume B: a boot whose clock has NOT reached the deadline. The
    // remaining wait is recomputed from `recorded_deadline − clock.now()`,
    // never read from a persisted remaining cache. We prove the recompute
    // by parking at a clock BEHIND the deadline and showing the sleep only
    // completes once logical time advances to the original deadline. ---
    let behind_clock = Arc::new(SimClock::new());
    behind_clock.tick(SLEEP / 3); // partway, still before the deadline
    assert!(behind_clock.unix_now() < recorded_deadline, "resume-B clock is before the deadline");
    {
        let ctx =
            ctx_on(&journal, behind_clock.clone() as Arc<dyn Clock>, &workflow_id, loaded.clone());
        let driver = behind_clock.clone();
        let sleeper = tokio::spawn(async move { ctx.sleep(SLEEP).await });
        tokio::task::yield_now().await;
        // NOTE: replay path recomputes remaining = deadline − now; the
        // SleepArmed entry is replayed (cursor advances) so no new entry.
        // Advance to exactly the recorded deadline (remaining recomputed
        // as deadline − now). The sleep resolves; if the engine had parked
        // on a fresh full `SLEEP` from this boot's now(), this advance
        // would be insufficient and the task would hang.
        let remaining = recorded_deadline.saturating_sub(driver.unix_now());
        driver.tick(remaining);
        tokio::time::timeout(Duration::from_secs(1), sleeper)
            .await
            .expect("resume-B sleep resolves at the original deadline (remaining recomputed)")
            .expect("sleeper task")
            .expect("replay ctx.sleep ok");
    }
}
