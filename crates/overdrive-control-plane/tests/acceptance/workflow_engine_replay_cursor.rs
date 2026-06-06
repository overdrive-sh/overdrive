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

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};
use overdrive_control_plane::workflow_runtime::JournalCursorHandle;

use std::sync::Arc as StdArc;

use overdrive_core::id::ContentHash;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    JournalCursor, SignalKey, SignalValue, WorkflowCtx, WorkflowCtxError,
};

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
    replay_buffer: Vec<LoadedEntry>,
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
    let buffer = vec![LoadedEntry::Command(JournalCommand::RunResult {
        name: STEP_NAME.to_string(),
        result_digest: ContentHash::of(&recorded_bytes),
        result_bytes: recorded_bytes,
    })];
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
    // The single appended entry IS the run's first command (positional
    // identity, D5 — no in-entry `step` to assert).
    match &entries[0] {
        LoadedEntry::Command(JournalCommand::RunResult { name, result_bytes, result_digest }) => {
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
    // The cursor advance is now witnessed by POSITION in the loaded run
    // (D5 — identity is positional; there is no in-entry `step`). Pairing
    // each command with its index pins that step-a landed first (index 0)
    // and step-b second (index 1) — a non-advancing cursor would record
    // both at the same position (one entry, or step-b overwriting step-a).
    let steps: Vec<(usize, String)> = entries
        .iter()
        .enumerate()
        .map(|(index, e)| match e {
            LoadedEntry::Command(JournalCommand::RunResult { name, .. }) => (index, name.clone()),
            other => panic!("expected RunResult, got {other:?}"),
        })
        .collect();
    assert_eq!(
        steps,
        vec![(0usize, "step-a".to_string()), (1usize, "step-b".to_string())],
        "two live runs record at ascending positions — the cursor advanced past position 0"
    );
}

/// ADR-0064 §3 cursor advance (replay path): a recorded step at cursor 0 is
/// replayed (advancing the cursor), so the NEXT step is live and records at
/// step 1. Pins `replay_run`'s `*cursor += 1` advance — a non-advancing
/// cursor would re-replay step 0 (or mis-route the second step).
#[tokio::test]
async fn replay_advances_cursor_so_next_step_is_live_at_step_one() {
    let recorded_bytes = encode_run_result(&Ok(7usize));
    let buffer = vec![LoadedEntry::Command(JournalCommand::RunResult {
        name: "step-a".to_string(),
        result_digest: ContentHash::of(&recorded_bytes),
        result_bytes: recorded_bytes,
    })];
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
    // Only step-b was appended live (step-a was replayed, not re-recorded).
    // The replay advanced the cursor past the recorded step-a, so the live
    // step-b records at command-index 1 — but since step-a was NOT
    // re-appended, the loaded run for this instance holds ONLY step-b. The
    // proof the replay advanced the cursor is that step-b is the SOLE
    // appended entry (a non-advancing cursor would have re-recorded step-a
    // or mis-routed step-b). Identity is positional (D5) — no `step` field.
    assert_eq!(entries.len(), 1, "only the live step-b was appended; got {entries:?}");
    match &entries[0] {
        LoadedEntry::Command(JournalCommand::RunResult { name, .. }) => {
            assert_eq!(name, "step-b", "the appended entry is the live step-b");
        }
        other => panic!("expected RunResult, got {other:?}"),
    }
}

/// Build a `WorkflowCtx` over a `SimJournalStore` seeded with `replay_buffer`,
/// returning the ctx + the backing journal + the instance id. No transport
/// inbox is bound — this scenario drives replayed `ctx.run` / `ctx.wait_for_signal`
/// only (the replay path resolves both from the recorded run, never firing an
/// effect or reading a live signal surface).
fn ctx_only(replay_buffer: Vec<LoadedEntry>) -> (WorkflowCtx, Arc<dyn JournalStore>, WorkflowId) {
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let workflow_id = WorkflowId::new("wf-cursor-cmd-walk").expect("valid id");
    let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
        Arc::clone(&journal),
        workflow_id.clone(),
        replay_buffer,
    ));
    let ctx = WorkflowCtx::new(clock, transport, entropy, cursor);
    (ctx, journal, workflow_id)
}

/// ADR-0064 §3 / §4 (D2/D6, CA-5): the cursor walks COMMANDS ONLY and
/// resolves a `SignalSeen` by `SignalKey` lookup — NOT by position. The
/// `*cursor += 2` two-positional-entry signal walk is RETIRED: a
/// `SignalAwaited` command advances the command-cursor by exactly 1, and the
/// matching `SignalSeen` notification is found off the walk by its key,
/// wherever it lands in the interleaved on-disk stream.
///
/// The loaded run interleaves three commands and one notification, with the
/// `SignalSeen` placed AFTER a trailing `RunResult` command — so the
/// notification is NOT at `SignalAwaited_position + 1`. This is the
/// falsifying shape for the retired positional `+2` walk: the old code read
/// the entry at `SignalAwaited + 1` (a `RunResult`, here), found no
/// `SignalSeen` there, and treated the wait as "crashed while blocked"
/// (re-block), stranding the trailing command. Under correlated lookup the
/// notification resolves by key, the command-cursor advances exactly 1 past
/// the `SignalAwaited`, and the trailing `RunResult` replays at the next
/// command position.
///
/// Loaded run (interleaved) and the partition it produces:
///
/// ```text
/// Loaded Vec<LoadedEntry>:
///   [0] Command(RunResult "step-a")
///   [1] Command(SignalAwaited K)
///   [2] Command(RunResult "step-b")            <- NOT a SignalSeen
///   [3] Notification(SignalSeen K = "ack")     <- off the positional walk
///
/// Partitioned at the cursor:
///   replay_commands      = [RunResult a, SignalAwaited K, RunResult b]
///   signal_notifications = { K -> SignalSeen("ack") }
/// ```
///
/// Port-to-port: driving ports are `WorkflowCtx::run` and
/// `WorkflowCtx::wait_for_signal`; the driven port is the `SimJournalStore`
/// behind the cursor (observed via `load_journal` — replay appends nothing).
#[tokio::test]
async fn cursor_walks_commands_only_and_resolves_signal_seen_by_key_not_position() {
    let key = SignalKey::new("cert-ready").expect("valid signal key");
    let recorded_value = SignalValue::new("ack");

    let first_bytes = encode_run_result(&Ok(7usize));
    let second_bytes = encode_run_result(&Ok(99usize));

    let buffer = vec![
        // [0] command — replayed first (command-cursor 0 → 1)
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "step-a".to_string(),
            result_digest: ContentHash::of(&first_bytes),
            result_bytes: first_bytes,
        }),
        // [1] command — the armed wait (command-cursor 1 → 2 on resolve)
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() }),
        // [2] command — replayed after the signal resolves (command-cursor 2 → 3).
        //     Under the retired positional `+2` walk this sits where a
        //     SignalSeen would have to be; correlated lookup ignores its
        //     position entirely.
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "step-b".to_string(),
            result_digest: ContentHash::of(&second_bytes),
            result_bytes: second_bytes,
        }),
        // [3] NOTIFICATION — resolved by SignalKey lookup, off the walk. Its
        //     position (3, NOT SignalAwaited+1=2) is the load-bearing proof
        //     that resolution is correlated, not positional.
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: key.clone(),
            value_digest: ContentHash::of(recorded_value.as_str().as_bytes()),
            value: recorded_value.clone(),
        }),
    ];

    let (ctx, journal, workflow_id) = ctx_only(buffer);

    // Command-index 0 — step-a replays from the recorded run.
    let a: Result<usize, String> =
        ctx.run("step-a", async move { Ok::<usize, String>(0) }).await.expect("replay step-a");
    assert_eq!(a, Ok(7), "step-a replays the recorded result");

    // Command-index 1 — wait_for_signal resolves SignalSeen by SignalKey
    // lookup (NOT by reading position SignalAwaited+1, which holds step-b).
    // This is the retired `+2` proof: a positional walk would have read
    // step-b at SignalAwaited+1, missed the SignalSeen, and re-blocked.
    let value =
        ctx.wait_for_signal(key.clone()).await.expect("signal resolves from recorded notification");
    assert_eq!(value, recorded_value, "the wait resolves to the recorded SignalSeen value by key");

    // Command-index 2 — step-b MUST replay. It only does if wait_for_signal
    // advanced the command-cursor by exactly 1 (past the SignalAwaited), NOT
    // 2 (which would skip step-b) and NOT 0 (which would re-replay it as the
    // wait). This is the "advance by exactly 1" + "notification never
    // advances the cursor" proof.
    let b: Result<usize, String> =
        ctx.run("step-b", async move { Ok::<usize, String>(0) }).await.expect("replay step-b");
    assert_eq!(b, Ok(99), "step-b replays at command-index 2 — the cursor advanced by exactly 1");

    // The whole run was a replay — the cursor appended NOTHING (every step,
    // including the signal, was resolved from the loaded run).
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(entries.is_empty(), "a pure replay appends nothing; got {entries:?}");
}

/// ADR-0064 §3 fail-closed determinism gate (D4), Layers 1 + 2.
///
/// **Layer 1 (type-at-index, Restate RT0016 shape).** Every command-replay
/// method checks the recorded `JournalCommand` variant TYPE at the
/// command-cursor against the await-op being replayed. A `ctx.run` await
/// landing on a recorded `SleepArmed` command is a divergent trajectory:
/// the cursor returns `WorkflowCtxError::NonDeterministic { expected, actual }`,
/// does NOT advance the cursor, and does NOT fall through to the live path
/// (no `Ok(None)`). This is the trap's twin — the former silent fall-to-live
/// on a variant mismatch (the old `let ... else { Ok(None) }`), now CLOSED.
///
/// **Layer 2 (name within `RunResult`).** A recorded `RunResult` whose name
/// diverges from the replaying body's `ctx.run` name at this cursor is the
/// same fail-closed `NonDeterministic`.
///
/// Both halves assert: (a) `NonDeterministic` is returned, (b) the
/// `expected`/`actual` payload is a deterministic kind-label / recorded name
/// (never an address-bearing whole-entry `Debug`), and (c) the cursor did NOT
/// advance and did NOT fall through to live — proven by the journal staying
/// EMPTY (a fall-to-live would have polled the future, fired the effect, and
/// appended a `RunResult`).
///
/// Port-to-port: the driving port is `WorkflowCtx::run`; the driven port is
/// the `SimJournalStore` behind the cursor (observed via `load_journal` — a
/// fail-closed replay appends nothing).
#[tokio::test]
async fn type_at_index_mismatch_and_name_mismatch_both_fail_closed_nondeterministic() {
    // ----- Layer 1: type-at-index mismatch -----
    // The loaded run records a SleepArmed at command-cursor 0, but the
    // replaying body issues a `ctx.run` await — a divergent trajectory.
    let layer1_buffer = vec![LoadedEntry::Command(JournalCommand::SleepArmed {
        deadline_unix: Duration::from_secs(60),
    })];
    let (ctx, journal, workflow_id) = ctx_only(layer1_buffer);

    let layer1 = ctx
        .run(STEP_NAME, async move {
            // This future MUST NOT be polled: a fail-closed gate returns the
            // error WITHOUT reaching the live path. If it were polled the
            // effect below would fire and a RunResult would be appended.
            Ok::<usize, String>(1)
        })
        .await;

    match layer1 {
        Err(WorkflowCtxError::NonDeterministic { expected, actual }) => {
            // `expected` is the deterministic kind-label of the recorded
            // command at the cursor (a stable as_str()-style label), NOT an
            // address-bearing Debug of the whole entry.
            assert_eq!(
                expected, "SleepArmed",
                "expected names the recorded command KIND at the cursor (deterministic label)"
            );
            // `actual` names the await-op the body issued at this cursor —
            // here the `ctx.run` step name, equally deterministic.
            assert_eq!(
                actual, STEP_NAME,
                "actual names the await-op the body replayed (the ctx.run step name)"
            );
            // Neither value carries an address-bearing whole-entry Debug:
            // assert the stable labels do not contain Rust pointer/struct
            // syntax that a `{:?}` of the entry would (DST trajectories must
            // stay byte-identical across seeds).
            for value in [&expected, &actual] {
                assert!(
                    !value.contains("0x") && !value.contains('{') && !value.contains("deadline"),
                    "expected/actual must be a stable kind-label, not a whole-entry Debug: {value:?}"
                );
            }
        }
        other => panic!(
            "Layer 1 type-at-index mismatch must fail closed with NonDeterministic, got {other:?}"
        ),
    }

    // No advance, no fall-through: a fail-closed gate appends NOTHING. A
    // silent fall-to-live (the trap's twin) would have polled the future and
    // appended a RunResult here.
    let layer1_entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(
        layer1_entries.is_empty(),
        "Layer 1 fail-closed must not advance / fall through to live; got {layer1_entries:?}"
    );

    // ----- Layer 2: name-in-RunResult mismatch -----
    // The loaded run records a RunResult under a DIFFERENT name than the
    // replaying body's ctx.run step — divergence within the matching variant.
    let recorded_bytes = encode_run_result(&Ok(7usize));
    let layer2_buffer = vec![LoadedEntry::Command(JournalCommand::RunResult {
        name: "recorded-name".to_string(),
        result_digest: ContentHash::of(&recorded_bytes),
        result_bytes: recorded_bytes,
    })];
    let (ctx2, journal2, workflow_id2) = ctx_only(layer2_buffer);

    let layer2 = ctx2.run("body-name", async move { Ok::<usize, String>(1) }).await;

    match layer2 {
        Err(WorkflowCtxError::NonDeterministic { expected, actual }) => {
            assert_eq!(
                expected, "recorded-name",
                "Layer 2 expected is the recorded RunResult name (deterministic)"
            );
            assert_eq!(
                actual, "body-name",
                "Layer 2 actual is the replaying body's ctx.run name (deterministic)"
            );
        }
        other => {
            panic!("Layer 2 name mismatch must fail closed with NonDeterministic, got {other:?}")
        }
    }

    let layer2_entries = journal2.load_journal(&workflow_id2).await.expect("load");
    assert!(
        layer2_entries.is_empty(),
        "Layer 2 fail-closed must not advance / append; got {layer2_entries:?}"
    );
}
