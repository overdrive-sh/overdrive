//! Slice 01 / US-WP-2 AC2 — the journal write does not advance the cursor
//! when the fsync fails (durability write-ordering); slice-01 AC2.
//!
//! Scenario S-WP-01-10. The `WorkflowJournalWriteOrdering` DST invariant
//! (ADR-0064 §6, mirroring ADR-0035 `WriteThroughOrdering`): under a
//! `SimJournalStore` configured to fail the fsync on the next `append`,
//! the engine does NOT advance the journal cursor and does NOT suspend
//! with an unrecorded step acknowledged; on the next boot the journal
//! carries no phantom half-written entry. fsync-then-suspend is
//! load-bearing (ADR-0063 §4).
//!
//! Two surfaces are asserted:
//!
//! 1. The named-invariant catalogue surface: `WorkflowJournalWriteOrdering`
//!    is a NAMED `Invariant` variant on the `cargo dst` critical path and
//!    its harness verdict is GREEN + seed-reproducible. The sibling
//!    `WorkflowExactlyOnceEffectOnResume` (US-WP-4 AC4) is also named and
//!    on the critical path.
//! 2. The behavioural surface (port-to-port): `WorkflowCtx::run` against a
//!    `JournalCursorHandle` whose `SimJournalStore` has an injected
//!    fsync-failure errors on the live-path record, the cursor does NOT
//!    advance (a subsequent retry is still a LIVE call, not a replay), and
//!    the journal carries no phantom entry.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use overdrive_core::traits::Transport;
use overdrive_core::workflow::{JournalCursor, TerminalError, TerminalErrorKind, WorkflowCtx};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};
use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;

const TARGET: &str = "127.0.0.1:9000";
const PAYLOAD: &[u8] = b"provision-record";
const STEP_NAME: &str = "provision-write";

/// Run the provision-write durable step through `ctx.run`; returns the raw
/// ctx result so a record failure surfaces. Under Model Z (ADR-0065 §4) a
/// journal-record infra failure is projected to `TerminalError::explicit` at
/// the ctx-op boundary. `T` is `Result<usize, String>` (the transport error
/// folds into the success type), so an infra record failure is distinguishable
/// from an effect failure.
async fn run_step(
    ctx: &WorkflowCtx,
    target: SocketAddr,
) -> Result<Result<usize, String>, TerminalError> {
    let transport = Arc::clone(ctx.transport());
    let payload = Bytes::from_static(PAYLOAD);
    ctx.run(STEP_NAME, async move {
        Ok(transport.send_datagram(target, payload).await.map_err(|e| e.to_string()))
    })
    .await
}

async fn delivered_count(inbox: &mut SimInbox) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await {
        count += 1;
    }
    count
}

/// The named-catalogue surface: `WorkflowJournalWriteOrdering` is a real
/// enum variant (no inline string literal), on the critical path, GREEN,
/// and seed-reproducible. The sibling `WorkflowExactlyOnceEffectOnResume`
/// (US-WP-4 AC4) is also a named variant on the critical path.
#[test]
fn write_ordering_and_exactly_once_are_named_critical_path_invariants() {
    const SEED: u64 = 0x0124_5678_9abc_def0;

    for (variant, name) in [
        (Invariant::WorkflowJournalWriteOrdering, "workflow-journal-write-ordering"),
        (Invariant::WorkflowExactlyOnceEffectOnResume, "workflow-exactly-once-effect-on-resume"),
    ] {
        // Named variant — no inline string literal; round-trips.
        assert_eq!(
            Invariant::from_str(name).expect("resolves by canonical name"),
            variant,
            "{name} maps to its named variant"
        );
        assert!(Invariant::ALL.contains(&variant), "{name} is on the critical path");

        let report_a = Harness::new().only(variant).run(SEED).expect("harness composes");
        let result_a = report_a
            .invariants
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} ran"));
        assert_eq!(result_a.status, InvariantStatus::Pass, "{name} GREEN: {:?}", result_a.cause);

        let report_b = Harness::new().only(variant).run(SEED).expect("harness composes (2nd)");
        let result_b =
            report_b.invariants.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(result_a, result_b, "{name} verdict is seed-reproducible");
    }
}

/// The behavioural surface (port-to-port). Under an injected fsync-failure,
/// the live-path `ctx.run` record FAILS with `JournalRecord`, the cursor
/// does NOT advance (the entry is unobservable + a retry is still LIVE),
/// and no phantom entry is in the journal. Mirrors ADR-0035
/// `WriteThroughOrdering` for the workflow journal.
#[tokio::test]
async fn fsync_failure_on_append_does_not_advance_cursor_or_suspend_with_unrecorded_step() {
    let store = Arc::new(SimJournalStore::new());
    let journal: Arc<dyn JournalStore> = Arc::clone(&store) as Arc<dyn JournalStore>;
    let workflow_id = WorkflowId::new("wf-ordering-0001").expect("valid id");

    let target: SocketAddr = TARGET.parse().expect("addr");
    let sim_transport = SimTransport::new();
    let mut inbox = sim_transport.bind_inbox(target).await.expect("bind");

    let cursor: Arc<dyn JournalCursor> =
        Arc::new(JournalCursorHandle::new(Arc::clone(&journal), workflow_id.clone(), Vec::new()));
    let ctx = WorkflowCtx::new(
        Arc::new(SimClock::new()),
        Arc::new(sim_transport) as Arc<dyn Transport>,
        Arc::new(SimEntropy::new(0x5eed)),
        cursor.clone(),
    );

    // Arm the fsync failure: the next live-path record (append) fails.
    store.inject_fsync_failure();

    let err = run_step(&ctx, target).await.expect_err("live record must fail under injected fsync");
    // Under Model Z (ADR-0065 §4) the cursor's JournalRecord infra failure is
    // PROJECTED to TerminalError::explicit at the ctx-op boundary; the
    // journal-record cause survives in the projected detail.
    assert_eq!(
        err.kind(),
        TerminalErrorKind::Explicit,
        "a journal-record infra failure projects to an Explicit terminal, got {err:?}"
    );
    assert!(
        err.detail().contains("journal record failed"),
        "the failed record surfaces as a journal-record terminal: {:?}",
        err.detail()
    );

    // The entry is UNOBSERVABLE — no phantom half-written entry persisted.
    let after_fail = journal.load_journal(&workflow_id).await.expect("load after failed append");
    assert!(
        after_fail.is_empty(),
        "a failed fsync leaves no observable entry (no phantom): {after_fail:?}"
    );

    // The transport effect is at-least-once by design: the failed
    // live-path call ALREADY fired its datagram before the append failed
    // (exactly-once is the replay/resume guarantee, not the within-boot
    // retry guarantee). Drain that pre-retry fire first.
    let fires_before_retry = delivered_count(&mut inbox).await;
    assert_eq!(
        fires_before_retry, 1,
        "the failed live-path call fired its datagram once before the append failed \
         (at-least-once transport)"
    );

    // The cursor did NOT advance: clear the failure and retry through the
    // SAME cursor handle. If the cursor had wrongly advanced, the retry
    // would be a REPLAY (cursor past buffer) and would fire ZERO datagrams.
    // Because the cursor stayed at step 0 and the buffer is empty, the
    // retry is a LIVE call that fires the effect once more and now records.
    store.clear_fsync_failure();
    let retry = run_step(&ctx, target).await.expect("retry after clear records live");
    assert_eq!(retry, Ok(PAYLOAD.len()), "the retry is a live fire (real byte count)");
    assert_eq!(
        delivered_count(&mut inbox).await,
        1,
        "the cursor did not advance — the retry is a LIVE call (fires once), not a replay (0)"
    );

    let after_clear = journal.load_journal(&workflow_id).await.expect("load after clear");
    assert_eq!(after_clear.len(), 1, "exactly one entry recorded after the successful retry");
}
