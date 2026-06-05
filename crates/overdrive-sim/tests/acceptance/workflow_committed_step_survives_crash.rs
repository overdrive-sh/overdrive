//! Slice 01 / US-WP-3 AC2 — a committed step survives the crash (not
//! lost) on resume; slice-01 AC2.
//!
//! Scenario S-WP-01-07. K2 (O2, single-node). The recorded step's result
//! is read back from the redb journal on resume (committed step NOT
//! lost), and the resumed run continues from the first unrecorded await,
//! not from the top. ADR-0064 §3 (replay buffer; check-then-record).
//!
//! Cross-scenario consistency (journey steps 2↔3): the bytes read here
//! are the bytes S-WP-01-04 wrote to the real redb journal.
//!
//! # Port-to-port
//!
//! Driving port: `WorkflowCtx::call` (the slice-01 await-surface). Driven
//! ports: the bound `SimInbox` (the `SimTransport` effect — fires on the
//! live step, NOT on the replayed step) and the `SimJournalStore` behind
//! the cursor handle (the committed `CallResult` read back on resume).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use overdrive_core::traits::Transport;
use overdrive_core::workflow::{CallRequest, CallResponse, JournalCursor, WorkflowCtx};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const TARGET: &str = "127.0.0.1:9000";
const PAYLOAD: &[u8] = b"provision-record";

/// Build a `WorkflowCtx` over a SHARED journal, with a freshly-bound
/// transport inbox so each "boot" observes its own delivered-datagram
/// count. The cursor is seeded with `replay_buffer` (empty on first boot;
/// the persisted run on resume).
async fn ctx_on(
    journal: Arc<dyn JournalStore>,
    workflow_id: &WorkflowId,
    replay_buffer: Vec<JournalEntry>,
) -> (WorkflowCtx, SimInbox) {
    let target: SocketAddr = TARGET.parse().expect("addr");
    let sim_transport = SimTransport::new();
    let inbox = sim_transport.bind_inbox(target).await.expect("bind");
    let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
        Arc::clone(&journal),
        workflow_id.clone(),
        replay_buffer,
    ));
    let ctx = WorkflowCtx::new(
        Arc::new(SimClock::new()),
        Arc::new(sim_transport) as Arc<dyn Transport>,
        Arc::new(SimEntropy::new(0x5eed)),
        cursor,
    );
    (ctx, inbox)
}

fn request() -> CallRequest {
    CallRequest { target: TARGET.parse().expect("addr"), payload: Bytes::from_static(PAYLOAD) }
}

async fn delivered_once(inbox: &mut SimInbox) -> bool {
    tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await.is_ok()
}

#[tokio::test]
async fn committed_step_is_read_back_from_journal_and_run_resumes_from_first_unrecorded_await() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let workflow_id = WorkflowId::new("wf-committed-0001").expect("valid id");

    // ---- Boot 1: live ctx.call commits a CallResult to the journal,
    //      then "crash" (drop the ctx before any further await). ----
    let first_call: CallResponse = {
        let (ctx, mut inbox) = ctx_on(Arc::clone(&journal), &workflow_id, Vec::new()).await;
        let response = ctx.call(request()).await.expect("boot-1 live call");
        assert!(delivered_once(&mut inbox).await, "boot-1 live call fires the effect");
        response
    };

    // The committed step is durable: read back from the journal byte-equal.
    let committed = journal.load_journal(&workflow_id).await.expect("load after crash");
    assert_eq!(committed.len(), 1, "exactly the one committed step survives: {committed:?}");
    match &committed[0] {
        JournalEntry::CallResult { step, bytes_sent, .. } => {
            assert_eq!(*step, 0, "the committed step is step 0");
            assert_eq!(
                *bytes_sent, first_call.bytes_sent,
                "the journal read-back is byte-equal to the live response"
            );
        }
        other => panic!("committed step must be a CallResult, got {other:?}"),
    }

    // ---- Boot 2: RESUME. The engine load_journals the committed step into
    //      the replay buffer; ctx.call short-circuits (replay) and returns
    //      the read-back response WITHOUT re-firing the effect — the run
    //      resumes from the FIRST UNRECORDED await, not from the top. ----
    let (ctx, mut inbox) = ctx_on(Arc::clone(&journal), &workflow_id, committed.clone()).await;
    let replayed = ctx.call(request()).await.expect("boot-2 replayed call");

    // Committed step NOT lost: the replayed response is the read-back one.
    assert_eq!(
        replayed.bytes_sent, first_call.bytes_sent,
        "resume reads the committed step back from the journal — not lost"
    );
    // Resume continues past the recorded await WITHOUT re-performing it:
    // the effect did NOT re-fire (no datagram delivered to the fresh inbox).
    assert!(
        !delivered_once(&mut inbox).await,
        "resume must not re-fire the committed step's effect — it resumes past it"
    );
    // And nothing new was appended (the recorded step was replayed, not
    // re-recorded) — the journal still carries exactly the one step.
    let after_resume = journal.load_journal(&workflow_id).await.expect("load after resume");
    assert_eq!(
        after_resume, committed,
        "replay appends nothing; the journal is unchanged from the committed run"
    );
}
