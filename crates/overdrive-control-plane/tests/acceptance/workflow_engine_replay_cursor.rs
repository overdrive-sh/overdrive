//! Slice 01 — the durable replay cursor (ADR-0064 §3), the canonical
//! Temporal/Restate/DBOS re-execute-from-top-and-short-circuit shape that
//! 01-06's DST `replay_equivalence_provision_record` invariant relies on.
//!
//! These unit tests pin the two halves of the check-then-record contract
//! the `JournalCursorHandle` implements behind `WorkflowCtx::call`:
//!
//! - **Replay** (cursor < buffer length): returns the recorded
//!   `CallResponse` WITHOUT firing the transport effect (K1 — exactly-once
//!   on resume). Observable proof: no datagram is delivered to the bound
//!   inbox, and nothing is appended to the journal.
//! - **Live** (cursor == buffer length): fires the real effect, then
//!   records a `CallResult` (append + fsync) carrying the byte-equal
//!   response, and advances the cursor.
//!
//! Port-to-port: the driving port is `WorkflowCtx::call`; the driven ports
//! are the injected `SimTransport` (observed via its bound inbox) and the
//! `SimJournalStore` behind the cursor handle (observed via `load_journal`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use overdrive_core::id::ContentHash;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{CallRequest, JournalCursor, WorkflowCtx};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const TARGET: &str = "127.0.0.1:9000";
const PAYLOAD: &[u8] = b"provision-record";

/// Build a `WorkflowCtx` whose transport is a real `SimTransport` with an
/// inbox bound at `TARGET`, and whose journal cursor wraps a
/// `SimJournalStore` seeded with `replay_buffer`. Returns the ctx, the
/// bound inbox (to observe delivered datagrams), the journal (to inspect
/// what the cursor recorded), and the instance id.
async fn ctx_with_buffer(
    replay_buffer: Vec<JournalEntry>,
) -> (WorkflowCtx, SimInbox, Arc<dyn JournalStore>, WorkflowId) {
    let target: SocketAddr = TARGET.parse().expect("addr");
    let sim_transport = SimTransport::new();
    let inbox = sim_transport.bind_inbox(target).await.expect("bind inbox");

    let transport: Arc<dyn Transport> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let workflow_id = WorkflowId::new("wf-cursor-unit").expect("valid id");
    let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
        Arc::clone(&journal),
        workflow_id.clone(),
        replay_buffer,
    ));

    let ctx = WorkflowCtx::new(clock, transport, entropy, cursor);
    (ctx, inbox, journal, workflow_id)
}

fn provision_request() -> CallRequest {
    CallRequest { target: TARGET.parse().expect("addr"), payload: Bytes::from_static(PAYLOAD) }
}

/// ADR-0064 §3 replay path: when the cursor has a recorded `CallResult`
/// for the step, `ctx.call` returns the recorded `CallResponse` WITHOUT
/// firing the transport effect (K1, exactly-once on resume). The proof
/// the effect did NOT fire is that NO datagram reaches the bound inbox.
#[tokio::test]
async fn replay_returns_recorded_response_without_firing_transport() {
    let recorded_bytes = 4242usize;
    let buffer = vec![JournalEntry::CallResult {
        step: 0,
        correlation: "x".to_string(),
        response_digest: ContentHash::of(recorded_bytes.to_le_bytes()),
        bytes_sent: recorded_bytes,
    }];
    let (ctx, mut inbox, journal, workflow_id) = ctx_with_buffer(buffer).await;

    let response = ctx.call(provision_request()).await.expect("replayed call");

    // 1. The replayed response IS the recorded value.
    assert_eq!(
        response.bytes_sent, recorded_bytes,
        "replay must return the recorded CallResponse, not a freshly-fired one"
    );

    // 2. The transport effect did NOT fire — no datagram delivered. A
    //    live fire would deliver PAYLOAD to the inbox.
    let delivered = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await;
    assert!(
        delivered.is_err(),
        "replay must NOT fire the transport effect; a datagram was delivered: {delivered:?}"
    );

    // 3. Replay short-circuits — it appends NOTHING (the run is already
    //    recorded).
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(entries.is_empty(), "replay must not append; got {entries:?}");
}

/// ADR-0064 §3 live path: with an empty replay buffer, `ctx.call` fires
/// the real transport effect, then records a `CallResult` (append + fsync)
/// carrying the byte-equal response, and advances the cursor. Proofs: the
/// delivered datagram AND the appended journal entry.
#[tokio::test]
async fn live_fires_effect_then_records_and_advances() {
    let (ctx, mut inbox, journal, workflow_id) = ctx_with_buffer(Vec::new()).await;

    let response = ctx.call(provision_request()).await.expect("live call");

    // 1. The effect fired — the datagram was delivered with PAYLOAD.
    let delivered = tokio::time::timeout(Duration::from_millis(50), inbox.recv())
        .await
        .expect("live call must deliver a datagram within the budget")
        .expect("inbox sender live");
    assert_eq!(delivered.payload.as_ref(), PAYLOAD, "live fire delivers the payload");

    // 2. The response reflects the real effect (bytes_sent == len).
    assert_eq!(response.bytes_sent, PAYLOAD.len(), "live response is the real byte count");

    // 3. The live path recorded exactly one CallResult carrying the
    //    byte-equal response — so a resume replays it without re-firing.
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(entries.len(), 1, "live path appends exactly one entry; got {entries:?}");
    match &entries[0] {
        JournalEntry::CallResult { step, bytes_sent, response_digest, .. } => {
            assert_eq!(*step, 0, "first live call records step 0");
            assert_eq!(
                *bytes_sent,
                PAYLOAD.len(),
                "recorded bytes_sent is the byte-equal response"
            );
            assert_eq!(
                *response_digest,
                ContentHash::of(PAYLOAD.len().to_le_bytes()),
                "recorded response_digest is the content hash of the response"
            );
        }
        other => panic!("live path must append a CallResult, got {other:?}"),
    }
}
