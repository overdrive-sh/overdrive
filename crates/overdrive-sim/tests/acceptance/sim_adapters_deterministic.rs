//! §6.1 — Every nondeterminism port has both a real and a sim
//! implementation available; each sim adapter is deterministic under a
//! fixed seed.
//!
//! Covers the six non-storage Sim adapters:
//!
//! * `SimClock` — logical time advances via a harness `tick`; `now()`
//!   reflects the cumulative logical time.
//! * `SimTransport` — an in-process echo-pair round-trips a datagram;
//!   partition blocks it; repair restores delivery.
//! * `SimEntropy` — a seeded `StdRng`; two independent instances produce
//!   identical first-N `u64()` draws on the same seed.
//! * `SimDataplane` — in-memory policy/service maps; `drain_flow_events`
//!   returns pre-queued synthetic events.
//! * `SimDriver` — in-memory allocation table with configurable failure
//!   modes.
//! * `SimLlm` — transcript replay; attempting a call with an empty
//!   transcript errors; a single-turn transcript round-trips cleanly;
//!   a tool-choice deviation returns a `TranscriptMismatch` error.

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;
use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::{Backend, Dataplane, FlowEvent, PolicyKey, Verdict};
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverError, DriverType, Resources};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::llm::{
    Completion, Llm, LlmError, Message, Prompt, Role, ToolCall, Usage,
};
use overdrive_core::traits::transport::Transport;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::llm::SimLlm;
use overdrive_sim::adapters::transport::SimTransport;

const STEP_SEED: u64 = 0x05_01_AA_AA_AA_AA_AA_AA;

fn spiffe(path: &str) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/{path}")).expect("valid SPIFFE ID")
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::from_str(id).expect("valid alloc id")
}

// ---------------------------------------------------------------------------
// SimClock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sim_clock_now_reflects_cumulative_harness_ticks() {
    let clock = SimClock::new();
    let t0 = clock.now();

    clock.tick(Duration::from_millis(200));
    let t1 = clock.now();

    clock.tick(Duration::from_millis(300));
    let t2 = clock.now();

    assert_eq!(
        t1.saturating_duration_since(t0),
        Duration::from_millis(200),
        "first tick must advance logical time by exactly 200ms"
    );
    assert_eq!(
        t2.saturating_duration_since(t0),
        Duration::from_millis(500),
        "two ticks must accumulate to 500ms of logical time"
    );
}

#[tokio::test]
async fn sim_clock_sleep_advances_logical_time() {
    let clock = SimClock::new();
    let t0 = clock.now();

    // `sleep` on a SimClock advances the logical clock in-place; no wall-
    // clock time passes. `tokio::time::sleep` is banned in core logic.
    clock.sleep(Duration::from_millis(750)).await;

    let t1 = clock.now();
    assert_eq!(
        t1.saturating_duration_since(t0),
        Duration::from_millis(750),
        "sim sleep must advance logical time deterministically"
    );
}

// ---------------------------------------------------------------------------
// SimTransport
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sim_transport_datagram_round_trips_between_endpoints() {
    let transport = SimTransport::new();
    let addr_b: SocketAddr = "127.0.0.1:9002".parse().expect("valid addr");

    // Bind B as a datagram sink.
    let mut inbox = transport.bind_inbox(addr_b).await.expect("bind succeeds");

    // A sends a datagram to B.
    let addr_a: SocketAddr = "127.0.0.1:9001".parse().expect("valid addr");
    let payload = Bytes::from_static(b"hello");
    let sent =
        transport.send_datagram_from(addr_a, addr_b, payload.clone()).await.expect("send succeeds");
    assert_eq!(sent, payload.len());

    let received = tokio::time::timeout(Duration::from_millis(500), inbox.recv())
        .await
        .expect("receive within deadline")
        .expect("datagram delivered");

    assert_eq!(received.payload, payload, "payload round-trips unchanged");
    assert_eq!(received.from, addr_a, "source address is preserved");
}

#[tokio::test]
async fn sim_transport_partition_blocks_delivery_repair_restores_it() {
    let transport = SimTransport::new();
    let addr_a: SocketAddr = "127.0.0.1:10001".parse().expect("valid addr");
    let addr_b: SocketAddr = "127.0.0.1:10002".parse().expect("valid addr");

    let mut inbox = transport.bind_inbox(addr_b).await.expect("bind succeeds");

    // Partition A → B. Sending is accepted but delivery is suppressed.
    transport.partition(addr_a, addr_b);
    transport
        .send_datagram_from(addr_a, addr_b, Bytes::from_static(b"drop-me"))
        .await
        .expect("partitioned send is accepted at the sender");

    let maybe = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await;
    assert!(maybe.is_err(), "partitioned datagram must not reach B");

    // Repair and resend.
    transport.repair(addr_a, addr_b);
    transport
        .send_datagram_from(addr_a, addr_b, Bytes::from_static(b"delivered"))
        .await
        .expect("repaired send succeeds");

    let delivered = tokio::time::timeout(Duration::from_millis(500), inbox.recv())
        .await
        .expect("receive within deadline")
        .expect("datagram delivered after repair");
    assert_eq!(delivered.payload, Bytes::from_static(b"delivered"));
}

// ---------------------------------------------------------------------------
// SimEntropy
// ---------------------------------------------------------------------------

#[test]
fn sim_entropy_u64_is_deterministic_under_fixed_seed() {
    let a = SimEntropy::new(STEP_SEED);
    let b = SimEntropy::new(STEP_SEED);

    let draws_a: Vec<u64> = (0..256).map(|_| a.u64()).collect();
    let draws_b: Vec<u64> = (0..256).map(|_| b.u64()).collect();

    assert_eq!(
        draws_a, draws_b,
        "two independent SimEntropy instances seeded with the same value \
         must produce identical u64 draws"
    );
    assert!(
        draws_a.iter().any(|&v| v != 0),
        "sanity: the RNG must actually produce some non-zero draws"
    );
}

#[test]
fn sim_entropy_fill_is_deterministic_under_fixed_seed() {
    let a = SimEntropy::new(STEP_SEED);
    let b = SimEntropy::new(STEP_SEED);

    let mut buf_a = [0u8; 32];
    let mut buf_b = [0u8; 32];
    a.fill(&mut buf_a);
    b.fill(&mut buf_b);

    assert_eq!(buf_a, buf_b, "fill must be deterministic under fixed seed");
}

// `proptest!` expands to a function that doesn't satisfy every pedantic
// clippy lint (`too_many_lines`, `similar_names` in shrinkers). Narrow the
// allow to the proptest! macro body.
proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: 64,
        .. proptest::prelude::ProptestConfig::default()
    })]

    /// Over arbitrary seeds and arbitrary draw-counts in `0..=1024`, two
    /// independent `SimEntropy` instances produce identical `u64()` draw
    /// sequences. This is the §21 "entropy is the seed, bit for bit"
    /// property — any deviation would hide DST non-determinism.
    #[test]
    fn sim_entropy_is_seed_deterministic_for_any_seed_and_draw_count(
        seed in proptest::prelude::any::<u64>(),
        draws in 0usize..=1024,
    ) {
        let a = SimEntropy::new(seed);
        let b = SimEntropy::new(seed);

        let draws_a: Vec<u64> = (0..draws).map(|_| a.u64()).collect();
        let draws_b: Vec<u64> = (0..draws).map(|_| b.u64()).collect();

        proptest::prop_assert_eq!(draws_a, draws_b);
    }
}

// ---------------------------------------------------------------------------
// SimDataplane
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sim_dataplane_stores_policy_and_service_state() {
    let dataplane = SimDataplane::new();

    let key = PolicyKey { src: spiffe("job/payments"), dst: spiffe("job/database") };
    dataplane.update_policy(key.clone(), Verdict::Allow).await.expect("update_policy succeeds");

    assert_eq!(
        dataplane.policy_verdict(&key),
        Some(Verdict::Allow),
        "stored policy verdict must be readable back"
    );

    let vip: Ipv4Addr = "10.0.0.7".parse().expect("valid ip");
    let backend = Backend {
        alloc: spiffe("job/payments/alloc/a1b2c3"),
        addr: "127.0.0.1:8080".parse().expect("valid addr"),
        weight: 100,
        healthy: true,
    };
    dataplane.update_service(vip, vec![backend.clone()]).await.expect("update_service succeeds");

    let stored = dataplane.service_backends(vip);
    assert_eq!(stored.as_deref(), Some(&[backend][..]));
}

#[tokio::test]
async fn sim_dataplane_drain_flow_events_returns_seeded_events() {
    let dataplane = SimDataplane::new();
    let event = FlowEvent {
        src: spiffe("job/a"),
        dst: spiffe("job/b"),
        verdict: Verdict::Allow,
        bytes_up: 128,
        bytes_down: 256,
    };
    dataplane.enqueue_flow_event(event.clone());

    let drained = dataplane.drain_flow_events().await.expect("drain succeeds");
    assert_eq!(drained, vec![event]);

    // Second drain returns an empty vec — `drain` empties the queue.
    let drained_again = dataplane.drain_flow_events().await.expect("drain succeeds");
    assert!(drained_again.is_empty(), "second drain must be empty");
}

// ---------------------------------------------------------------------------
// SimDriver
// ---------------------------------------------------------------------------

fn sample_spec() -> AllocationSpec {
    AllocationSpec {
        alloc: alloc("alloc-a1b2c3"),
        identity: spiffe("job/payments/alloc/a1b2c3"),
        image: "registry/payments:1.0".to_owned(),
        resources: Resources { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 },
    }
}

#[tokio::test]
async fn sim_driver_start_stop_status_round_trip() {
    let driver = SimDriver::new(DriverType::Exec);
    let spec = sample_spec();

    let handle = driver.start(&spec).await.expect("start succeeds");
    assert_eq!(handle.alloc, spec.alloc);

    let state = driver.status(&handle).await.expect("status after start");
    assert!(
        matches!(state, overdrive_core::traits::driver::AllocationState::Running),
        "status after start must be Running, got {state:?}"
    );

    driver.stop(&handle).await.expect("stop succeeds");
    let state = driver.status(&handle).await.expect("status after stop");
    assert!(
        matches!(state, overdrive_core::traits::driver::AllocationState::Terminated),
        "status after stop must be Terminated, got {state:?}"
    );
}

#[tokio::test]
async fn sim_driver_honours_configured_start_failure() {
    let driver = SimDriver::new(DriverType::MicroVm).fail_on_start_with("disk full".to_owned());
    let err = driver.start(&sample_spec()).await.expect_err("start must fail");

    match err {
        DriverError::StartRejected { driver, reason } => {
            assert_eq!(driver, DriverType::MicroVm);
            assert_eq!(reason, "disk full");
        }
        other => panic!("expected StartRejected, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SimLlm
// ---------------------------------------------------------------------------

fn sample_prompt() -> Prompt {
    Prompt {
        system: "sre investigation".to_owned(),
        messages: vec![Message { role: Role::User, content: "hello".to_owned() }],
    }
}

fn sample_completion() -> Completion {
    Completion {
        content: "investigating".to_owned(),
        tool_calls: vec![ToolCall {
            tool: "query_flows".to_owned(),
            input: serde_json::json!({"sql": "SELECT 1"}),
        }],
        usage: Usage { prompt_tokens: 42, completion_tokens: 7 },
    }
}

#[tokio::test]
async fn sim_llm_empty_transcript_rejects_any_call() {
    let llm = SimLlm::new(vec![]);
    let err = llm.complete(&sample_prompt(), &[]).await.expect_err("empty transcript rejects call");

    assert!(
        matches!(err, LlmError::TranscriptMismatch { .. }),
        "expected TranscriptMismatch on empty transcript, got {err:?}"
    );
}

#[tokio::test]
async fn sim_llm_single_turn_transcript_round_trips() {
    let expected = sample_completion();
    let llm = SimLlm::new(vec![expected.clone()]);

    let actual =
        llm.complete(&sample_prompt(), &[]).await.expect("first turn returns canned completion");
    assert_eq!(actual.content, expected.content);
    assert_eq!(actual.tool_calls.len(), expected.tool_calls.len());
    assert_eq!(actual.tool_calls[0].tool, expected.tool_calls[0].tool);
}

#[tokio::test]
async fn sim_llm_exhausted_transcript_rejects_next_call() {
    let llm = SimLlm::new(vec![sample_completion()]);
    llm.complete(&sample_prompt(), &[]).await.expect("first call uses the transcript");

    let err = llm
        .complete(&sample_prompt(), &[])
        .await
        .expect_err("second call must fail — transcript is exhausted");
    assert!(matches!(err, LlmError::TranscriptMismatch { .. }));
}

// ---------------------------------------------------------------------------
// Mutation-coverage tests — tightening the invariants of each adapter so
// small mutations (off-by-one, swap-return, flip-comparison) are caught.
// ---------------------------------------------------------------------------

#[test]
fn sim_llm_is_exhausted_transitions_from_false_to_true() {
    let llm = SimLlm::new(vec![sample_completion()]);
    assert!(!llm.is_exhausted(), "fresh transcript is not exhausted");

    // Drive the transcript forward to exhaustion.
    tokio::runtime::Runtime::new().expect("runtime").block_on(async {
        llm.complete(&sample_prompt(), &[]).await.expect("first call consumes the transcript");
    });

    assert!(llm.is_exhausted(), "after consuming the only entry, exhausted must be true");
}

#[test]
fn sim_llm_empty_transcript_is_immediately_exhausted() {
    let llm = SimLlm::new(vec![]);
    assert!(llm.is_exhausted(), "a transcript with no entries is exhausted from the start");
}

#[tokio::test]
async fn sim_llm_boundary_at_last_transcript_entry_succeeds() {
    // Guards against `<` → `<=` mutation in `complete`: the final index
    // must still return a completion, not error.
    let expected = sample_completion();
    let llm = SimLlm::new(vec![expected.clone(), expected.clone()]);

    let first = llm.complete(&sample_prompt(), &[]).await.expect("first call succeeds");
    let second = llm.complete(&sample_prompt(), &[]).await.expect("second (last) call succeeds");
    assert_eq!(first.content, expected.content);
    assert_eq!(second.content, expected.content);
}

#[tokio::test]
async fn sim_clock_unix_now_advances_with_logical_time() {
    let clock = SimClock::new();
    let before = clock.unix_now();

    clock.tick(Duration::from_millis(400));
    let after = clock.unix_now();

    assert_eq!(
        after.saturating_sub(before),
        Duration::from_millis(400),
        "unix_now must track logical-time advance exactly"
    );
}

#[tokio::test]
async fn sim_clock_clone_shares_logical_counter() {
    // Guards against `clone -> default` mutation: the clone must share the
    // same Arc-backed counter as the original, AND the same `Instant`
    // epoch, so that observations through the clone exactly match the
    // original. A fresh `Default::default()` would produce a new epoch
    // and a zero counter — the epoch mismatch falsifies this assertion.
    let clock = SimClock::new();
    let twin = clock.clone();

    let original_before = clock.now();
    let twin_before = twin.now();
    assert_eq!(original_before, twin_before, "clone must report the same `now` as the original");

    clock.tick(Duration::from_millis(250));

    let original_after = clock.now();
    let twin_after = twin.now();
    assert_eq!(original_after, twin_after, "clone must see advances made through the original");
    assert_eq!(
        twin_after.saturating_duration_since(twin_before),
        Duration::from_millis(250),
        "clone must reflect exactly 250ms of logical advance"
    );
}

#[tokio::test]
async fn sim_driver_resize_returns_error_for_unknown_allocation() {
    let driver = SimDriver::new(DriverType::Exec);
    let unknown_handle = overdrive_core::traits::driver::AllocationHandle {
        alloc: alloc("alloc-unknown"),
        pid: None,
    };

    let err = driver
        .resize(&unknown_handle, Resources { cpu_milli: 100, memory_bytes: 1024 })
        .await
        .expect_err("resize on an unknown allocation must fail");
    assert!(matches!(err, DriverError::NotFound { .. }));
}

#[tokio::test]
async fn sim_driver_resize_succeeds_for_running_allocation() {
    // Guards against `resize -> Ok(())` mutation AND the `!contains`
    // flip: a running allocation's resize must succeed; missing must
    // fail. Together with the previous test, both branches are covered.
    let driver = SimDriver::new(DriverType::Exec);
    let handle = driver.start(&sample_spec()).await.expect("start succeeds");

    driver
        .resize(&handle, Resources { cpu_milli: 1000, memory_bytes: 2048 })
        .await
        .expect("resize on a running allocation succeeds");
}

#[tokio::test]
async fn sim_transport_send_datagram_reports_payload_length() {
    // Guards against `send_datagram -> Ok(0)` / `Ok(1)` mutations: the
    // returned byte count must match the payload length exactly.
    let transport = SimTransport::new();
    let addr_b: SocketAddr = "127.0.0.1:11002".parse().expect("valid addr");
    let _inbox = transport.bind_inbox(addr_b).await.expect("bind succeeds");

    let payload = Bytes::from_static(b"XYZ12345");
    let sent = transport.send_datagram(addr_b, payload.clone()).await.expect("send succeeds");
    assert_eq!(sent, payload.len(), "send_datagram must return exact payload length");
}

#[test]
fn sim_entropy_distinct_seeds_produce_distinct_sequences() {
    // Guards against `u64 -> 1` / `fill -> ()` mutations: different
    // seeds must yield different values; a mutated `u64` that always
    // returns 1 would make the two sequences trivially equal.
    let a = SimEntropy::new(STEP_SEED);
    let b = SimEntropy::new(STEP_SEED ^ 0xDEAD_BEEF_DEAD_BEEF);

    let draws_a: Vec<u64> = (0..16).map(|_| a.u64()).collect();
    let draws_b: Vec<u64> = (0..16).map(|_| b.u64()).collect();
    assert_ne!(draws_a, draws_b, "distinct seeds must produce distinct draws");

    let mut buf_a = [0u8; 16];
    let mut buf_b = [0u8; 16];
    a.fill(&mut buf_a);
    b.fill(&mut buf_b);
    assert_ne!(buf_a, buf_b, "distinct seeds must fill distinct byte sequences");
}
