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
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    JournalCursor, SignalKey, SignalValue, TerminalError, TerminalErrorKind, WorkflowCtx,
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
/// Returns the raw ctx result so a record failure is observable. Under Model Z
/// (ADR-0065 §4) the ctx-op error channel is `TerminalError` (an infra failure
/// inside the cursor is projected to `TerminalError::explicit` at the ctx-op
/// boundary); the step's domain `Result<usize, String>` remains the success
/// type `T`.
async fn run_provision_step(
    ctx: &WorkflowCtx,
    target: SocketAddr,
) -> Result<Result<usize, String>, TerminalError> {
    let transport = StdArc::clone(ctx.transport());
    let payload = Bytes::from_static(PAYLOAD);
    ctx.run(STEP_NAME, async move {
        Ok(transport.send_datagram(target, payload).await.map_err(|e| e.to_string()))
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
    ctx.run(step_name, async move { Ok(value) }).await.expect("run named step")
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
    let replayed: Result<usize, String> = ctx
        .run("step-a", async move { Ok(Ok::<usize, String>(999)) })
        .await
        .expect("replayed step-a");
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
        ctx.run("step-a", async move { Ok(Ok::<usize, String>(0)) }).await.expect("replay step-a");
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
        ctx.run("step-b", async move { Ok(Ok::<usize, String>(0)) }).await.expect("replay step-b");
    assert_eq!(b, Ok(99), "step-b replays at command-index 2 — the cursor advanced by exactly 1");

    // The whole run was a replay — the cursor appended NOTHING (every step,
    // including the signal, was resolved from the loaded run).
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(entries.is_empty(), "a pure replay appends nothing; got {entries:?}");
}

/// Regression — two `ctx.wait_for_signal(x)` calls on the SAME `SignalKey`
/// in one instance must replay the two recorded `SignalSeen` values in
/// RECORDED ORDER (FIFO), not collapse to last-write-wins.
///
/// This is the `fix-duplicate-signal-key-replay` defect: the
/// `workflow-journal-command-notification-split` feature moved `SignalSeen`
/// off the positional command walk into a key-only
/// `BTreeMap<SignalKey, JournalNotification>` lookup, which silently dropped
/// the same-key disambiguation the retired `*cursor += 2` positional walk
/// provided. The command walk still disambiguates two `SignalAwaited{x}`
/// correctly (advance-by-1); only the notification side collapsed:
/// `BTreeMap::insert` overwrote `SignalSeen{x,v1}` with `SignalSeen{x,v2}`,
/// so a live run that produced `(v1, v2)` replays `(v2, v2)` — a silent
/// live-vs-replay divergence Layers 1/2 cannot detect (both
/// `SignalAwaited{x}` commands are byte-identical; the wrong datum is the
/// off-walk notification VALUE).
///
/// Loaded run (interleaved) and the partition it must produce:
///
/// ```text
/// Loaded Vec<LoadedEntry>:
///   [0] Command(SignalAwaited K)
///   [1] Notification(SignalSeen K = "v1")
///   [2] Command(SignalAwaited K)
///   [3] Notification(SignalSeen K = "v2")
///
/// Partitioned at the cursor (after Fix B — per-key FIFO):
///   replay_commands      = [SignalAwaited K, SignalAwaited K]
///   signal_notifications = { K -> [SignalSeen("v1"), SignalSeen("v2")] }
/// ```
///
/// The first wait pops `v1`, the second pops `v2`. Against current HEAD the
/// first wait returns `v2` (the last-write-wins bug).
///
/// Port-to-port: the driving port is `WorkflowCtx::wait_for_signal`; the
/// driven port is the `SimJournalStore` behind the cursor (observed via
/// `load_journal` — a pure replay appends nothing). Pure replay → 3-arg
/// `new` (obs = None); both waits are replay hits, no live path.
#[tokio::test]
async fn duplicate_signal_key_waits_replay_in_recorded_order() {
    let key = SignalKey::new("cert-ready").expect("valid signal key");
    let v1 = SignalValue::new("v1");
    let v2 = SignalValue::new("v2");

    let buffer = vec![
        // [0] command — the first armed wait.
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() }),
        // [1] notification — resolves the FIRST wait (FIFO position 0).
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: key.clone(),
            value_digest: ContentHash::of(v1.as_str().as_bytes()),
            value: v1.clone(),
        }),
        // [2] command — the second armed wait, SAME key.
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() }),
        // [3] notification — resolves the SECOND wait (FIFO position 1).
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: key.clone(),
            value_digest: ContentHash::of(v2.as_str().as_bytes()),
            value: v2.clone(),
        }),
    ];

    let (ctx, journal, workflow_id) = ctx_only(buffer);

    // Command-index 0 — the first wait resolves to the FIRST recorded
    // SignalSeen by FIFO order. Against HEAD this returns v2 (the bug).
    let first = ctx
        .wait_for_signal(key.clone())
        .await
        .expect("first wait resolves from recorded notification");
    assert_eq!(first, v1, "the FIRST wait resolves to the FIRST recorded SignalSeen value (FIFO)");

    // Command-index 1 — the second wait resolves to the SECOND recorded
    // SignalSeen. This only holds if the first wait POPPED v1 off the key's
    // FIFO queue, leaving v2 for the second.
    let second = ctx
        .wait_for_signal(key.clone())
        .await
        .expect("second wait resolves from recorded notification");
    assert_eq!(
        second, v2,
        "the SECOND wait resolves to the SECOND recorded SignalSeen value (FIFO)"
    );

    // The whole run was a replay — the cursor appended NOTHING.
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
            Ok(Ok::<usize, String>(1))
        })
        .await;

    // Under Model Z (ADR-0065 §4) the cursor's `NonDeterministic` infra
    // failure is PROJECTED to `TerminalError::explicit` at the ctx-op boundary;
    // the body's `?` observes a terminal, never a `WorkflowCtxError`. The
    // deterministic `expected`/`actual` labels survive inside the projected
    // detail (the `NonDeterministic` Display the projection renders).
    match layer1 {
        Err(terminal) => {
            assert_eq!(
                terminal.kind(),
                TerminalErrorKind::Explicit,
                "a divergent journal projects to an Explicit terminal at the ctx-op boundary"
            );
            let detail = terminal.detail();
            // Layer 1 is the type-at-index gate: BOTH `expected` and `actual`
            // are command-KIND labels in the same namespace — the recorded
            // command's kind (`expected`) vs the await-op's own expected kind
            // (`actual`), matching the sibling arms (replay_sleep →
            // "SleepArmed", replay_signal → "SignalAwaited", replay_emit →
            // "ActionEmitted"). The diverging step name is a Layer-2 concern
            // and MUST NOT appear in a Layer-1 detail.
            //
            // `expected` is the deterministic kind-label of the recorded
            // command at the cursor (a stable as_str()-style label), NOT an
            // address-bearing Debug of the whole entry.
            assert!(
                detail.contains("SleepArmed"),
                "detail names the recorded command KIND at the cursor (deterministic label): {detail:?}"
            );
            // `actual` reports the await-op's own command KIND (`RunResult`),
            // same namespace as `expected` — NOT the ctx.run step name.
            assert!(
                detail.contains("RunResult"),
                "Layer 1 reports the await-op's command KIND (RunResult), not the ctx.run step name: {detail:?}"
            );
            // The cross-namespace step name is a Layer-2 concern and must not
            // leak into a Layer-1 type-at-index detail.
            assert!(
                !detail.contains(STEP_NAME),
                "Layer 1 must NOT leak the cross-namespace step name; both fields are command-kind labels: {detail:?}"
            );
            // The projected detail carries no address-bearing whole-entry Debug:
            // the deterministic labels do not contain Rust pointer/struct syntax
            // that a `{:?}` of the entry would (DST trajectories must stay
            // byte-identical across seeds).
            assert!(
                !detail.contains("0x") && !detail.contains('{') && !detail.contains("deadline"),
                "detail must carry stable kind-labels, not a whole-entry Debug: {detail:?}"
            );
        }
        other => panic!(
            "Layer 1 type-at-index mismatch must fail closed with an Explicit terminal, got {other:?}"
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

    let layer2 = ctx2.run("body-name", async move { Ok(Ok::<usize, String>(1)) }).await;

    match layer2 {
        Err(terminal) => {
            assert_eq!(
                terminal.kind(),
                TerminalErrorKind::Explicit,
                "Layer 2 name divergence projects to an Explicit terminal at the ctx-op boundary"
            );
            let detail = terminal.detail();
            assert!(
                detail.contains("recorded-name"),
                "Layer 2 detail names the recorded RunResult name (deterministic): {detail:?}"
            );
            assert!(
                detail.contains("body-name"),
                "Layer 2 detail names the replaying body's ctx.run name (deterministic): {detail:?}"
            );
        }
        other => panic!(
            "Layer 2 name mismatch must fail closed with an Explicit terminal, got {other:?}"
        ),
    }

    let layer2_entries = journal2.load_journal(&workflow_id2).await.expect("load");
    assert!(
        layer2_entries.is_empty(),
        "Layer 2 fail-closed must not advance / append; got {layer2_entries:?}"
    );
}

/// ADR-0064 §3 fail-closed determinism gate (D4), Layer 2 for the
/// `SignalAwaited` branch — the key-within-variant gate, the sibling of the
/// `RunResult` name gate above.
///
/// The `SignalAwaited` replay branch had Layer 1 (type-at-index) but was
/// MISSING Layer 2 (signal-key-within-`SignalAwaited`), unlike `RunResult`.
/// Consequence on a crashed-while-blocked resume after the body's
/// `ctx.wait_for_signal` key changed at the same cursor:
///
/// 1. `replay_signal("key-b")` passes Layer 1 (variant is still
///    `SignalAwaited`), discards the recorded key, looks up the notification
///    map under the NEW key, misses, and returns `Ok(None)` — the legitimate
///    "crashed while blocked" shape.
/// 2. `record_signal_awaited("key-b")` matched the recorded
///    `SignalAwaited{"key-a"}` via an UNGUARDED `matches!(.., SignalAwaited)`,
///    advanced the cursor past it WITHOUT comparing keys, and returned
///    `Ok(())`.
/// 3. The recorded `SignalAwaited{"key-a"}` was silently consumed — no
///    `NonDeterministic`, then the live block resolved and a `SignalSeen` was
///    appended. Determinism-gate hole.
///
/// With Layer 2 in place a key divergence at the same cursor fails closed:
/// `replay_signal` returns `NonDeterministic { expected: "key-a", actual:
/// "key-b" }`, the cursor does NOT advance, and nothing is appended.
///
/// The buffer records `SignalAwaited{"key-a"}` with NO matching `SignalSeen`
/// (the crash-while-blocked shape); the replaying body issues
/// `ctx.wait_for_signal("key-b")`. Identity is POSITIONAL; the key is the
/// determinism guard, not the cursor identity.
///
/// Port-to-port: the driving port is `WorkflowCtx::wait_for_signal`; the
/// driven port is the `SimJournalStore` behind the cursor (observed via
/// `load_journal` — a fail-closed replay appends nothing). The buggy path
/// would have appended a `SignalSeen` notification.
#[tokio::test]
async fn signal_key_mismatch_in_crash_while_blocked_fails_closed_nondeterministic() {
    let recorded = SignalKey::new("key-a").expect("valid signal key");
    // Crash-while-blocked shape: a lone SignalAwaited command, NO matching
    // SignalSeen notification.
    let buffer =
        vec![LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: recorded.clone() })];
    let (ctx, journal, workflow_id) = ctx_only(buffer);

    // The replaying body re-blocks on a DIFFERENT key at the same cursor.
    let body_key = SignalKey::new("key-b").expect("valid signal key");
    let result = ctx.wait_for_signal(body_key).await;

    match result {
        Err(terminal) => {
            assert_eq!(
                terminal.kind(),
                TerminalErrorKind::Explicit,
                "a signal-key divergence projects to an Explicit terminal at the ctx-op boundary"
            );
            let detail = terminal.detail();
            assert!(
                detail.contains("key-a"),
                "detail names the recorded SignalAwaited key (deterministic, Display form): {detail:?}"
            );
            assert!(
                detail.contains("key-b"),
                "detail names the key the replaying body re-blocked on (deterministic): {detail:?}"
            );
        }
        other => panic!(
            "signal-key mismatch on crash-while-blocked resume must fail closed with \
             an Explicit terminal, got {other:?}"
        ),
    }

    // Fail-closed: the cursor did NOT advance and did NOT fall through to the
    // live block — nothing was appended. The buggy path silently consumed the
    // recorded SignalAwaited and then appended a SignalSeen notification.
    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(
        entries.is_empty(),
        "fail-closed must not advance / append (no SignalSeen); got {entries:?}"
    );
}

/// ADR-0064 §3 sleep branch, replay path. A recorded `SleepArmed` at the
/// cursor replays its recorded ABSOLUTE deadline WITHOUT re-arming (no
/// append), and advances the command-cursor by exactly 1 — so the trailing
/// `RunResult` replays at the next command position.
///
/// Falsifies three mutations at once:
/// - `replay_sleep -> Ok(None)` — would fall to the live path and append a
///   fresh `SleepArmed` (the journal would NOT be empty).
/// - `*cursor += 1` -> `*= 1` in `replay_sleep` — the cursor would not
///   advance, landing the trailing `ctx.run` on the `SleepArmed` at index 0
///   → Layer-1 `NonDeterministic` (the `expect` below panics).
/// - `*cursor += 1` -> `-= 1` in `replay_sleep` — usize underflow panic.
///
/// The recorded deadline is `Duration::ZERO` (already in the past relative
/// to `clock.unix_now()`), so the replay path returns immediately without a
/// `SimClock` park.
#[tokio::test]
async fn replay_sleep_returns_recorded_deadline_and_advances_so_next_command_replays() {
    let recorded_bytes = encode_run_result(&Ok(7usize));
    let buffer = vec![
        // [0] the armed sleep — an already-passed absolute deadline.
        LoadedEntry::Command(JournalCommand::SleepArmed { deadline_unix: Duration::ZERO }),
        // [1] the post-sleep durable step — replays only if the sleep
        //     advanced the cursor by exactly 1.
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "after-sleep".to_string(),
            result_digest: ContentHash::of(&recorded_bytes),
            result_bytes: recorded_bytes,
        }),
    ];
    let (ctx, journal, workflow_id) = ctx_only(buffer);

    // Command-index 0 — the sleep replays the recorded (already-passed)
    // deadline and returns immediately (the duration arg is irrelevant on
    // replay; `ZERO` keeps the live-path mutant from parking the SimClock).
    ctx.sleep(Duration::ZERO).await.expect("sleep replays the recorded deadline");

    // Command-index 1 — the post-sleep step replays, proving the cursor
    // advanced by exactly 1.
    let after: Result<usize, String> = ctx
        .run("after-sleep", async move { Ok(Ok::<usize, String>(0)) })
        .await
        .expect("post-sleep step replays at command-index 1");
    assert_eq!(after, Ok(7), "the post-sleep step replays its recorded result");

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(entries.is_empty(), "a pure replay appends nothing; got {entries:?}");
}

/// ADR-0064 §3 sleep branch, live path. With an empty replay buffer
/// `ctx.sleep` arms a fresh sleep: it records a `SleepArmed { deadline_unix }`
/// (append + fsync, ADR-0066 §4) carrying the ABSOLUTE wall-clock deadline
/// (an input), then parks on the Clock. Pins `record_sleep_armed` against the
/// `-> Ok(())` no-op mutation (which would append nothing).
///
/// `Duration::ZERO` so the `SimClock` park returns immediately without a
/// harness tick.
#[tokio::test]
async fn live_sleep_records_sleep_armed_deadline() {
    let (ctx, journal, workflow_id) = ctx_only(Vec::new());

    ctx.sleep(Duration::ZERO).await.expect("live sleep records the deadline and returns");

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(entries.len(), 1, "live sleep appends exactly one entry; got {entries:?}");
    match &entries[0] {
        LoadedEntry::Command(JournalCommand::SleepArmed { deadline_unix }) => {
            // The recorded deadline is an ABSOLUTE wall-clock input (>= the
            // UNIX epoch), never a zero/remaining cache.
            assert!(
                *deadline_unix > Duration::ZERO,
                "live sleep records an absolute wall-clock deadline, not a remaining cache"
            );
        }
        other => panic!("live sleep must append a SleepArmed command, got {other:?}"),
    }
}

/// ADR-0064 §4 signal branch, live path. With an empty replay buffer and no
/// `ObservationStore` wired (the 3-arg DST-harness handle, whose
/// `poll_signal` resolves to the empty value immediately), `ctx.wait_for_signal`
/// records a `SignalAwaited` COMMAND, then — once the signal resolves — a
/// `SignalSeen` NOTIFICATION. Pins two no-op mutations at once:
/// - `record_signal_awaited -> Ok(())` — would drop the `SignalAwaited`
///   command (the run would hold only the notification).
/// - `record_signal_seen -> Ok(())` — would drop the `SignalSeen`
///   notification (the run would hold only the command).
#[tokio::test]
async fn live_wait_for_signal_records_awaited_command_then_seen_notification() {
    let (ctx, journal, workflow_id) = ctx_only(Vec::new());
    let key = SignalKey::new("cert-ready").expect("valid signal key");

    let value = ctx.wait_for_signal(key.clone()).await.expect("live wait resolves");
    assert_eq!(
        value,
        SignalValue::empty(),
        "a no-obs handle resolves the signal to the present-empty value"
    );

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert_eq!(
        entries.len(),
        2,
        "live wait appends a SignalAwaited command THEN a SignalSeen notification; got {entries:?}"
    );
    assert_eq!(
        entries[0],
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() }),
        "the first append is the SignalAwaited command (an input — the key the body blocked on)"
    );
    match &entries[1] {
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key, value, ..
        }) => {
            assert_eq!(*signal_key, key, "the SignalSeen notification carries the awaited key");
            assert_eq!(*value, SignalValue::empty(), "the SignalSeen records the observed value");
        }
        other => panic!("the second append must be a SignalSeen notification, got {other:?}"),
    }
}

/// ADR-0064 §4 signal branch, crash-while-blocked resume (S-WP-03-01). A
/// recorded `SignalAwaited` with NO matching `SignalSeen` notification is the
/// "crashed while still blocked" shape: `replay_signal` returns `Ok(None)`
/// (re-block), and the live `record_signal_awaited` sees the recorded
/// `SignalAwaited` at the cursor and advances PAST it by exactly 1 WITHOUT a
/// duplicate append. The trailing `RunResult` then replays at the next
/// command position.
///
/// Pins the crash-block advance against both arithmetic mutations:
/// - `*cursor += 1` -> `*= 1` — the cursor would not advance, landing the
///   trailing `ctx.run` on the `SignalAwaited` at index 0 → Layer-1
///   `NonDeterministic` (the `expect` below panics).
/// - `*cursor += 1` -> `-= 1` — usize underflow panic.
#[tokio::test]
async fn crash_while_blocked_resume_advances_past_recorded_signal_awaited() {
    let key = SignalKey::new("cert-ready").expect("valid signal key");
    let recorded_bytes = encode_run_result(&Ok(42usize));
    let buffer = vec![
        // [0] the armed wait — recorded, but its SignalSeen never landed
        //     (the prior run crashed while blocked).
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() }),
        // [1] the post-signal durable step — replays only if the crash-block
        //     branch advanced the cursor by exactly 1.
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "after-signal".to_string(),
            result_digest: ContentHash::of(&recorded_bytes),
            result_bytes: recorded_bytes,
        }),
    ];
    let (ctx, _journal, _workflow_id) = ctx_only(buffer);

    // The resumed wait re-blocks on the SAME key (no matching SignalSeen),
    // then resolves on the live poll (no obs wired → present-empty).
    let value =
        ctx.wait_for_signal(key.clone()).await.expect("resumed wait re-blocks then resolves");
    assert_eq!(value, SignalValue::empty(), "the resumed wait resolves on the live poll");

    // Command-index 1 — the post-signal step replays, proving the crash-block
    // branch advanced the cursor by exactly 1.
    let after: Result<usize, String> = ctx
        .run("after-signal", async move { Ok(Ok::<usize, String>(0)) })
        .await
        .expect("post-signal step replays at command-index 1");
    assert_eq!(after, Ok(42), "the post-signal step replays its recorded result");
}

/// ADR-0064 §4 signal branch, engine-internal block poll. A handle with no
/// `ObservationStore` wired (the 3-arg DST-harness `new`) resolves
/// `poll_signal` to the present-empty value — NEVER `Ok(None)` (absent) — so
/// a signalless live wait does not block forever. Pins `poll_signal` against
/// the `-> Ok(None)` mutation, which would make every live wait spin on the
/// Clock park indefinitely. Driven directly on the handle (the ctx-level
/// effect of the mutation is an infinite park, not a clean assertion).
#[tokio::test]
async fn poll_signal_with_no_obs_resolves_to_present_empty_value() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let workflow_id = WorkflowId::new("wf-poll-signal").expect("valid id");
    let handle = JournalCursorHandle::new(Arc::clone(&journal), workflow_id, Vec::new());
    let key = SignalKey::new("cert-ready").expect("valid signal key");

    let polled = handle.poll_signal(&key).await.expect("poll");
    assert_eq!(
        polled,
        Some(SignalValue::empty()),
        "a no-obs handle resolves a signal poll to present-empty, never absent (Ok(None))"
    );
}

/// ADR-0064 §4 emit branch, replay path. A recorded `ActionEmitted` at the
/// cursor short-circuits `ctx.emit_action` — the Action is NOT re-sent and
/// nothing is appended (exactly-once on the replay path) — and advances the
/// command-cursor by exactly 1, so the trailing `RunResult` replays at the
/// next command position.
///
/// Falsifies three mutations at once:
/// - `replay_emit -> Ok(false)` — would fall to the live path and append a
///   fresh `ActionEmitted` (the journal would NOT be empty).
/// - `*cursor += 1` -> `*= 1` in `replay_emit` — the cursor would not
///   advance, landing the trailing `ctx.run` on the `ActionEmitted` at index
///   0 → Layer-1 `NonDeterministic` (the `expect` below panics).
/// - `*cursor += 1` -> `-= 1` in `replay_emit` — usize underflow panic.
#[tokio::test]
async fn replay_emit_short_circuits_and_advances_so_next_command_replays() {
    let recorded_bytes = encode_run_result(&Ok(5usize));
    let buffer = vec![
        // [0] the recorded emit — replayed, NOT re-sent.
        LoadedEntry::Command(JournalCommand::ActionEmitted {
            action_digest: ContentHash::of(b"recorded-action"),
        }),
        // [1] the post-emit durable step — replays only if the emit advanced
        //     the cursor by exactly 1.
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "after-emit".to_string(),
            result_digest: ContentHash::of(&recorded_bytes),
            result_bytes: recorded_bytes,
        }),
    ];
    let (ctx, journal, workflow_id) = ctx_only(buffer);

    // Command-index 0 — the emit replays (Action NOT re-sent; nothing
    // appended) and advances the cursor by exactly 1.
    ctx.emit_action(Action::Noop).await.expect("emit replays (Action not re-sent)");

    // Command-index 1 — the post-emit step replays, proving the cursor
    // advanced by exactly 1.
    let after: Result<usize, String> = ctx
        .run("after-emit", async move { Ok(Ok::<usize, String>(0)) })
        .await
        .expect("post-emit step replays at command-index 1");
    assert_eq!(after, Ok(5), "the post-emit step replays its recorded result");

    let entries = journal.load_journal(&workflow_id).await.expect("load");
    assert!(entries.is_empty(), "a pure replay appends nothing; got {entries:?}");
}
