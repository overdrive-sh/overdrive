//! Slice 01 — the durable replay cursor (ADR-0064 §3), the canonical
//! Temporal/Restate/DBOS re-execute-from-top-and-short-circuit shape that
//! 01-06's DST `replay_equivalence_provision_record` invariant relies on.
//!
//! These unit tests pin the two halves of the check-then-record contract
//! the `JournalCursorHandle` implements behind `WorkflowCtx::run`:
//!
//! - **Replay** (cursor < buffer length): returns the recorded step
//!   result WITHOUT polling the step future / firing the transport effect
//!   (K1 — exactly-once on the replay path). Observable proof: no datagram
//!   is delivered to the bound inbox, and nothing is appended to the
//!   journal.
//! - **Live** (cursor == buffer length): polls the step future (fires the
//!   real effect), then records a `RunResult` (append + fsync) carrying
//!   the byte-equal CBOR result, and advances the cursor.
//!
//! Port-to-port: the driving port is `WorkflowCtx::run`; the driven ports
//! are the injected `SimTransport` (observed via its bound inbox) and the
//! `SimJournalStore` behind the cursor handle (observed via `load_journal`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use std::sync::Arc as StdArc;

use overdrive_core::id::ContentHash;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{JournalCursor, WorkflowCtx, WorkflowCtxError};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::transport::{SimInbox, SimTransport};

const TARGET: &str = "127.0.0.1:9000";
const PAYLOAD: &[u8] = b"provision-record";
/// The `ctx.run` step name the provision-write durable step records under.
const STEP_NAME: &str = "provision-write";

/// CBOR-encode a `Result<usize, String>` step result — the shape the
/// provision-write `ctx.run` step records. Helper so the replay buffer's
/// recorded bytes match what `ctx.run` would have written.
fn encode_run_result(value: &Result<usize, String>) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("encode run result");
    bytes
}

/// Perform the provision-write durable step through `ctx.run` — the same
/// `Result<usize, String>` step the `ProvisionRecord` workflow body runs.
/// Returns the raw ctx result so a record failure is observable.
async fn run_provision_step(
    ctx: &WorkflowCtx,
    target: SocketAddr,
) -> Result<Result<usize, String>, WorkflowCtxError> {
    let transport = StdArc::clone(ctx.transport());
    let payload = Bytes::from_static(PAYLOAD);
    ctx.run(STEP_NAME, async move {
        transport.send_datagram(target, payload).await.map_err(|e| e.to_string())
    })
    .await
}

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

fn target() -> SocketAddr {
    TARGET.parse().expect("addr")
}

/// ADR-0064 §3 replay path: when the cursor has a recorded `RunResult`
/// for the step, `ctx.run` returns the recorded result WITHOUT polling the
/// step future / firing the transport effect (K1, exactly-once on the
/// replay path). The proof the effect did NOT fire is that NO datagram
/// reaches the bound inbox.
#[tokio::test]
async fn replay_returns_recorded_result_without_firing_transport() {
    let recorded: Result<usize, String> = Ok(4242usize);
    let recorded_bytes = encode_run_result(&recorded);
    let buffer = vec![JournalEntry::RunResult {
        step: 0,
        name: STEP_NAME.to_string(),
        result_digest: ContentHash::of(&recorded_bytes),
        result_bytes: recorded_bytes,
    }];
    let (ctx, mut inbox, journal, workflow_id) = ctx_with_buffer(buffer).await;

    let result = run_provision_step(&ctx, target()).await.expect("replayed run");

    // 1. The replayed result IS the recorded value (decoded from CBOR).
    assert_eq!(
        result, recorded,
        "replay must return the recorded run result, not a freshly-fired one"
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

/// ADR-0064 §3 live path: with an empty replay buffer, `ctx.run` polls the
/// step future (fires the real transport effect), then records a
/// `RunResult` (append + fsync) carrying the byte-equal CBOR result, and
/// advances the cursor. Proofs: the delivered datagram AND the appended
/// journal entry.
#[tokio::test]
async fn live_fires_effect_then_records_and_advances() {
    let (ctx, mut inbox, journal, workflow_id) = ctx_with_buffer(Vec::new()).await;

    let result = run_provision_step(&ctx, target()).await.expect("live run");

    // 1. The effect fired — the datagram was delivered with PAYLOAD.
    let delivered = tokio::time::timeout(Duration::from_millis(50), inbox.recv())
        .await
        .expect("live run must deliver a datagram within the budget")
        .expect("inbox sender live");
    assert_eq!(delivered.payload.as_ref(), PAYLOAD, "live fire delivers the payload");

    // 2. The result reflects the real effect (Ok(bytes_sent) == len).
    assert_eq!(result, Ok(PAYLOAD.len()), "live result is the real byte count");

    // 3. The live path recorded exactly one RunResult carrying the
    //    byte-equal CBOR result — so a resume replays it without re-firing.
    let expected_bytes = encode_run_result(&Ok(PAYLOAD.len()));
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(entries.len(), 1, "live path appends exactly one entry; got {entries:?}");
    match &entries[0] {
        JournalEntry::RunResult { step, name, result_bytes, result_digest } => {
            assert_eq!(*step, 0, "first live run records step 0");
            assert_eq!(name, STEP_NAME, "the recorded step name is the ctx.run name");
            assert_eq!(
                *result_bytes, expected_bytes,
                "recorded result_bytes is the byte-equal CBOR result"
            );
            assert_eq!(
                *result_digest,
                ContentHash::of(&expected_bytes),
                "recorded result_digest is the content hash of the result bytes"
            );
        }
        other => panic!("live path must append a RunResult, got {other:?}"),
    }
}

/// Run a trivial CBOR-serializable step under `step_name` through `ctx.run`
/// (no transport effect — exercises the cursor advance directly).
async fn run_named(ctx: &WorkflowCtx, step_name: &str, value: u64) -> u64 {
    ctx.run(step_name, async move { value }).await.expect("run named step")
}

/// ADR-0064 §3 cursor advance (live path): two sequential live `ctx.run`
/// steps record at ASCENDING step indices (0 then 1). Pins `record_run`'s
/// `*cursor += 1` advance — a non-advancing cursor (`*= 1` at cursor 0)
/// would record both steps at step 0.
#[tokio::test]
async fn two_live_runs_record_at_ascending_step_indices() {
    let (ctx, _inbox, journal, workflow_id) = ctx_with_buffer(Vec::new()).await;

    let first = run_named(&ctx, "step-a", 11).await;
    let second = run_named(&ctx, "step-b", 22).await;
    assert_eq!((first, second), (11, 22), "each live step returns its own value");

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    let steps: Vec<(u32, String)> = entries
        .iter()
        .map(|e| match e {
            JournalEntry::RunResult { step, name, .. } => (*step, name.clone()),
            other => panic!("expected RunResult, got {other:?}"),
        })
        .collect();
    assert_eq!(
        steps,
        vec![(0u32, "step-a".to_string()), (1u32, "step-b".to_string())],
        "two live runs record at ascending step indices — the cursor advanced past step 0"
    );
}

/// ADR-0064 §3 cursor advance (replay path): a recorded step at cursor 0 is
/// replayed (advancing the cursor), so the NEXT step is live and records at
/// step 1. Pins `replay_run`'s `*cursor += 1` advance — a non-advancing
/// cursor would re-replay step 0 (or mis-route the second step).
#[tokio::test]
async fn replay_advances_cursor_so_next_step_is_live_at_step_one() {
    let recorded_bytes = encode_run_result(&Ok(7usize));
    let buffer = vec![JournalEntry::RunResult {
        step: 0,
        name: "step-a".to_string(),
        result_digest: ContentHash::of(&recorded_bytes),
        result_bytes: recorded_bytes,
    }];
    let (ctx, _inbox, journal, workflow_id) = ctx_with_buffer(buffer).await;

    // Step 0: replays the recorded result (cursor advances past it).
    let replayed: Result<usize, String> =
        ctx.run("step-a", async move { Ok::<usize, String>(999) }).await.expect("replayed step-a");
    assert_eq!(replayed, Ok(7), "step-a is replayed from the recorded result");

    // Step 1: live (cursor advanced past the replayed entry) — records a
    // NEW entry at step 1.
    let live = run_named(&ctx, "step-b", 42).await;
    assert_eq!(live, 42, "step-b runs live");

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    // Only step-b was appended live (step-a was replayed, not re-recorded),
    // and it landed at step 1 — proving the replay advanced the cursor.
    assert_eq!(entries.len(), 1, "only the live step-b was appended; got {entries:?}");
    match &entries[0] {
        JournalEntry::RunResult { step, name, .. } => {
            assert_eq!(*step, 1, "the live step-b records at step 1 — replay advanced the cursor");
            assert_eq!(name, "step-b", "the appended entry is the live step-b");
        }
        other => panic!("expected RunResult, got {other:?}"),
    }
}
