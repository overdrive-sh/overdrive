//! Slice 01 — `WorkflowCtx::run` (the durable-step await-surface) and the
//! `WorkflowCtx::sleep` await-surface, exercised through the driving port
//! against recording test doubles at the journal-cursor + clock boundaries.
//!
//! ADR-0064 §3 (check-then-record). These are the in-`overdrive-core`
//! port-to-port tests of the two ctx await-ops: the driving port is
//! `WorkflowCtx::{run, sleep}`; the driven ports are a recording
//! `JournalCursor` (observed via the entries it recorded) and a recording
//! `Clock` (observed via the durations it was asked to park for). The
//! control-plane `JournalCursorHandle` tests cover the durable-redb side;
//! these cover the ctx logic itself within the crate that owns it.

#![allow(clippy::expect_used)]
// Test-double constructors and lock-then-assert test patterns: the
// const-fn and drop-tightening lints add ceremony with no test value here.
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::significant_drop_tightening)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    JournalCursor, SignalKey, SignalValue, WorkflowCtx, WorkflowCtxError,
};

use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;

/// A recording `JournalCursor` that captures every `record_run` and
/// `record_sleep_armed` call and replays a pre-seeded run-result buffer.
/// Observing the recorded calls is how the tests assert what `ctx.run` /
/// `ctx.sleep` durably wrote (the live path) without a real journal.
#[derive(Default)]
struct RecordingCursor {
    /// Pre-seeded recorded run results to replay, in cursor order. `None`
    /// once exhausted → the live path.
    replay: Mutex<std::collections::VecDeque<(String, Vec<u8>)>>,
    /// Every `(name, bytes)` passed to `record_run`, in call order.
    recorded_runs: Mutex<Vec<(String, Vec<u8>)>>,
    /// Every absolute deadline passed to `record_sleep_armed`.
    recorded_sleeps: Mutex<Vec<Duration>>,
}

impl RecordingCursor {
    fn live() -> Self {
        Self::default()
    }

    fn replaying(entries: Vec<(String, Vec<u8>)>) -> Self {
        Self { replay: Mutex::new(entries.into_iter().collect()), ..Self::default() }
    }
}

#[async_trait]
impl JournalCursor for RecordingCursor {
    async fn replay_run(&self, name: &str) -> Result<Option<Vec<u8>>, WorkflowCtxError> {
        let mut replay = self.replay.lock().expect("lock");
        match replay.pop_front() {
            Some((recorded_name, bytes)) if recorded_name == name => Ok(Some(bytes)),
            Some((recorded_name, _)) => Err(WorkflowCtxError::NonDeterministic {
                expected: recorded_name,
                actual: name.to_string(),
            }),
            None => Ok(None),
        }
    }

    async fn record_run(&self, name: &str, result_bytes: &[u8]) -> Result<(), WorkflowCtxError> {
        self.recorded_runs.lock().expect("lock").push((name.to_string(), result_bytes.to_vec()));
        Ok(())
    }

    async fn replay_sleep(&self) -> Option<Duration> {
        None
    }

    async fn record_sleep_armed(&self, deadline_unix: Duration) -> Result<(), WorkflowCtxError> {
        self.recorded_sleeps.lock().expect("lock").push(deadline_unix);
        Ok(())
    }

    async fn replay_signal(&self, _signal_key: &SignalKey) -> Option<SignalValue> {
        // Slice-01/02 fixture: the signal + emit await-surfaces (slice 03)
        // are not exercised here — their port-to-port coverage lives in the
        // control-plane JournalCursorHandle tests + the sim DST invariants.
        // These methods exist only to satisfy the trait after the additive
        // surface landed; always-live / no-op behaviour.
        None
    }

    async fn record_signal_awaited(&self, _signal_key: &SignalKey) -> Result<(), WorkflowCtxError> {
        Ok(())
    }

    async fn poll_signal(
        &self,
        _signal_key: &SignalKey,
    ) -> Result<Option<SignalValue>, WorkflowCtxError> {
        // No signal surface wired in this fixture: resolve immediately
        // (always-live no-op) so a ctx that waits does not block forever.
        Ok(Some(SignalValue::empty()))
    }

    async fn record_signal_seen(
        &self,
        _signal_key: &SignalKey,
        _value: &SignalValue,
    ) -> Result<(), WorkflowCtxError> {
        Ok(())
    }

    async fn replay_emit(&self) -> bool {
        false
    }

    async fn emit_action(&self, _action: Action) -> Result<(), WorkflowCtxError> {
        Ok(())
    }
}

/// A recording `Clock` fixed at `now` that captures every `sleep`
/// duration. A fixed `unix_now` makes the recorded sleep deadline exactly
/// `now + duration`, so the `+`→`-` mutation in `ctx.sleep` is observable;
/// capturing the `sleep` duration makes a no-op `ctx.sleep` body
/// observable (a no-op records and parks nothing).
struct RecordingClock {
    unix: Duration,
    slept_for: Mutex<Vec<Duration>>,
}

impl RecordingClock {
    fn at(unix: Duration) -> Self {
        Self { unix, slept_for: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl Clock for RecordingClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_now(&self) -> Duration {
        self.unix
    }

    async fn sleep(&self, duration: Duration) {
        self.slept_for.lock().expect("lock").push(duration);
    }
}

fn ctx_with(cursor: Arc<dyn JournalCursor>, clock: Arc<dyn Clock>) -> WorkflowCtx {
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    WorkflowCtx::new(clock, transport, entropy, cursor)
}

/// CBOR-encode a value the way `ctx.run` records it — so the replay-path
/// test can seed byte-equal recorded results.
fn cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("encode");
    bytes
}

/// Live `ctx.run`: polls the step future, returns its value, and durably
/// records the CBOR-encoded result under the step name.
#[tokio::test]
async fn run_live_polls_step_and_records_cbor_result() {
    let cursor = Arc::new(RecordingCursor::live());
    let ctx = ctx_with(cursor.clone(), Arc::new(RecordingClock::at(Duration::ZERO)));

    let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let polled_in = polled.clone();
    let value: u64 = ctx
        .run("compute", async move {
            polled_in.store(true, std::sync::atomic::Ordering::SeqCst);
            7u64
        })
        .await
        .expect("live run");

    // 1. The returned value is the step's output (the live path polled `f`).
    assert_eq!(value, 7, "live run returns the polled step value");
    assert!(
        polled.load(std::sync::atomic::Ordering::SeqCst),
        "the live path polled the step future"
    );

    // 2. Exactly one durable record under the step name, byte-equal to the
    //    CBOR encoding of the result.
    let recorded = cursor.recorded_runs.lock().expect("lock");
    assert_eq!(recorded.len(), 1, "live run records exactly one result");
    assert_eq!(recorded[0].0, "compute", "the record carries the step name");
    assert_eq!(recorded[0].1, cbor(&7u64), "the record carries the CBOR-encoded result");
}

/// Replay `ctx.run`: returns the recorded result decoded from CBOR WITHOUT
/// polling the step future, and records nothing.
#[tokio::test]
async fn run_replay_returns_recorded_result_without_polling() {
    let recorded_bytes = cbor(&99u64);
    let cursor =
        Arc::new(RecordingCursor::replaying(vec![("compute".to_string(), recorded_bytes)]));
    let ctx = ctx_with(cursor.clone(), Arc::new(RecordingClock::at(Duration::ZERO)));

    let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let polled_in = polled.clone();
    let value: u64 = ctx
        .run("compute", async move {
            polled_in.store(true, std::sync::atomic::Ordering::SeqCst);
            7u64
        })
        .await
        .expect("replayed run");

    // The replayed value is the recorded one, the step future was NOT
    // polled (the effect never re-fires), and nothing new was recorded.
    assert_eq!(value, 99, "replay returns the recorded result, not the freshly-polled one");
    assert!(
        !polled.load(std::sync::atomic::Ordering::SeqCst),
        "replay must NOT poll the step future — the effect never re-fires"
    );
    assert!(cursor.recorded_runs.lock().expect("lock").is_empty(), "replay records nothing");
}

/// Replay `ctx.run` with a step-name divergence fails closed with
/// `NonDeterministic` — the replay-determinism guard.
#[tokio::test]
async fn run_replay_with_name_mismatch_fails_non_deterministic() {
    let cursor =
        Arc::new(RecordingCursor::replaying(vec![("recorded-step".to_string(), cbor(&1u64))]));
    let ctx = ctx_with(cursor, Arc::new(RecordingClock::at(Duration::ZERO)));

    let err = ctx
        .run::<u64, _>("different-step", async move { 1u64 })
        .await
        .expect_err("name mismatch must fail closed");

    match err {
        WorkflowCtxError::NonDeterministic { expected, actual } => {
            assert_eq!(expected, "recorded-step", "the recorded step name is reported as expected");
            assert_eq!(actual, "different-step", "the replaying body's name is reported as actual");
        }
        other => panic!("expected NonDeterministic, got {other:?}"),
    }
}

/// Live `ctx.sleep`: records the ABSOLUTE deadline (`now + duration`, an
/// input) and parks the clock for exactly `duration`. Kills both the
/// body-replacement (a no-op records/parks nothing) and the `+`→`-`
/// deadline-arithmetic mutation (the recorded deadline would be wrong).
#[tokio::test]
async fn sleep_live_records_absolute_deadline_and_parks_for_duration() {
    const NOW: Duration = Duration::from_secs(1_000);
    const WAIT: Duration = Duration::from_secs(30);

    let cursor = Arc::new(RecordingCursor::live());
    let clock = Arc::new(RecordingClock::at(NOW));
    let ctx = ctx_with(cursor.clone(), clock.clone());

    ctx.sleep(WAIT).await.expect("live sleep");

    // 1. The recorded deadline is the ABSOLUTE `now + duration` (an input),
    //    not `now - duration` — pins the `+` arithmetic in the live path.
    let sleeps = cursor.recorded_sleeps.lock().expect("lock");
    assert_eq!(sleeps.len(), 1, "live sleep records exactly one SleepArmed deadline");
    assert_eq!(
        sleeps[0],
        NOW + WAIT,
        "live sleep records the absolute deadline `now + duration` (an input)"
    );

    // 2. The clock was actually parked for `duration` — a no-op `ctx.sleep`
    //    body would park nothing.
    let parked = clock.slept_for.lock().expect("lock");
    assert_eq!(parked.as_slice(), &[WAIT], "live sleep parks the clock for exactly `duration`");
}

/// The `provision-write` shape end-to-end through `ctx.run`: the transport
/// effect runs inside the step, its `Result<usize, String>` result is
/// recorded, and the returned value reflects the real byte count.
#[tokio::test]
async fn run_wraps_a_transport_effect_and_records_its_result() {
    let target: SocketAddr = "127.0.0.1:9000".parse().expect("addr");
    let sim_transport = SimTransport::new();
    let mut inbox = sim_transport.bind_inbox(target).await.expect("bind");
    let transport: Arc<dyn Transport> = Arc::new(sim_transport);

    let cursor = Arc::new(RecordingCursor::live());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0));
    let clock: Arc<dyn Clock> = Arc::new(RecordingClock::at(Duration::ZERO));
    let ctx = WorkflowCtx::new(clock, transport, entropy, cursor.clone());

    let payload = Bytes::from_static(b"provision-record");
    let effect_transport = Arc::clone(ctx.transport());
    let sent: Result<usize, String> = ctx
        .run("provision-write", async move {
            effect_transport.send_datagram(target, payload).await.map_err(|e| e.to_string())
        })
        .await
        .expect("live run wrapping the transport effect");

    // The effect fired (datagram delivered) and the result is the real
    // byte count, recorded byte-equal as CBOR.
    assert_eq!(sent, Ok(b"provision-record".len()), "the step returns the real byte count");
    let delivered = tokio::time::timeout(Duration::from_millis(50), inbox.recv())
        .await
        .expect("datagram delivered")
        .expect("inbox live");
    assert_eq!(delivered.payload.as_ref(), b"provision-record", "the effect fired once");

    let recorded = cursor.recorded_runs.lock().expect("lock");
    assert_eq!(recorded.len(), 1, "the transport step records exactly one result");
    assert_eq!(recorded[0], ("provision-write".to_string(), cbor(&sent)));
}
