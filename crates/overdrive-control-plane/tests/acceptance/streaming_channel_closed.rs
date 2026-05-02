//! Acceptance — `fix-terminal-reason-channel-closed` Slice 01 step 01-01 (RED).
//!
//! Companion to the RCA at
//! `docs/feature/fix-terminal-reason-channel-closed/deliver/rca.md`.
//!
//! # Scenario — `closed_lifecycle_channel_emits_stream_interrupted_terminal`
//!
//! When the streaming handler's lifecycle-events broadcast channel is
//! closed (i.e. every `Sender` clone drops) BEFORE a terminal event
//! arrives, the handler's `select!` arm at `streaming.rs:284-300`
//! reaches `Err(broadcast::error::RecvError::Closed)`. The Closed arm
//! is responsible for emitting a terminal `SubmitEvent::ConvergedFailed`
//! whose `terminal_reason` accurately classifies the failure as
//! "stream interrupted, no specific cause" — i.e.
//! `TerminalReason::StreamInterrupted`.
//!
//! ## Current (buggy) behaviour
//!
//! On the unfixed code path the Closed arm reaches for the only
//! payload-free variant available in the legacy enum and synthesises
//! `TerminalReason::Timeout { after_seconds: 0 }` — a sentinel value
//! that violates `.claude/rules/development.md` §"Sum types over
//! sentinels" and produces wrong CLI rendering / wrong operator hints
//! downstream (RCA Branch B).
//!
//! ## Fix scope (lands in step 01-02)
//!
//! - `streaming.rs:284-300` swaps `Timeout { after_seconds: 0 }` for
//!   the new `TerminalReason::StreamInterrupted` variant.
//! - `streaming.rs` drops the local `bus` Arc clone immediately after
//!   `let mut sub = bus.subscribe();` so the test below is mechanically
//!   able to reach the Closed arm. The current production code holds
//!   `bus` for the lifetime of the async-stream closure, which means
//!   the in-stream `Arc<Sender>` keeps the channel open even when every
//!   external `Sender` clone has dropped. Step 01-02 lands the
//!   `drop(bus);` along with the variant swap; the two are paired.
//!
//! ## Why this RED test exists
//!
//! Per the RCA Branch C ("the Closed arm code-path is only reachable
//! by dropping the broadcast sender mid-stream — a server-shutdown
//! scenario that no test fixture currently constructs"), there is
//! today no acceptance test that asserts what the Closed arm emits.
//! This file closes that gap.
//!
//! Until the GREEN commit (01-02) lands the paired fix, this test
//! fails — the assertion catches the legacy `Timeout { after_seconds: 0 }`
//! and prints the documented panic message.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]
#![allow(clippy::large_futures)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, header};
use axum::routing::post;
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{SubmitJobRequest, TerminalReason};
use overdrive_control_plane::handlers::submit_job;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt as _;

// ---------------------------------------------------------------------------
// Test fixtures (mirror of `streaming_submit.rs` shape)
// ---------------------------------------------------------------------------

fn sample_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments-v0".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/payments".to_string(),
            args: vec![],
        }),
    }
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("LocalIntentStore::open"),
    );
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(sample_node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver)
}

fn build_router(state: AppState) -> Router {
    Router::new().route("/v1/jobs", post(submit_job)).with_state(state)
}

fn build_submit_request(spec: &JobSpecInput, accept: &str) -> Request<Body> {
    let body = serde_json::to_vec(&SubmitJobRequest { spec: spec.clone() }).expect("serialize");
    Request::builder()
        .method(Method::POST)
        .uri("/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, accept)
        .body(Body::from(body))
        .expect("build request")
}

async fn body_ndjson_lines(body: Body) -> Vec<Value> {
    let bytes = to_bytes(body, usize::MAX).await.expect("body to bytes");
    let s = std::str::from_utf8(&bytes).expect("utf8 body");
    s.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect(&format!("valid json line: {l}")))
        .collect()
}

// ===========================================================================
// RED scaffold (step 01-01) — see file-level docstring for the GREEN
// fix that lands in step 01-02 (variant swap + `drop(bus);` after
// subscribe). Until then, this test fails with the documented panic
// asserting `TerminalReason::StreamInterrupted` against the legacy
// `Timeout { after_seconds: 0 }`.
// ===========================================================================

#[tokio::test]
async fn closed_lifecycle_channel_emits_stream_interrupted_terminal() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);

    // Inject SimClock + a small streaming_cap so that, even on the
    // legacy code path where the cap timer fires before Closed (the
    // in-stream `bus` Arc keeps the channel open until the closure
    // ends — see file docstring), the test still terminates promptly.
    // The terminal_reason on the cap path is
    // `Timeout { after_seconds: 1 }`, which the StreamInterrupted
    // assertion will catch with the documented panic message.
    let sim_clock = Arc::new(SimClock::new());
    state.clock = sim_clock.clone() as Arc<dyn Clock>;
    state.streaming_cap = Duration::from_secs(1);

    // Hold a clone of the lifecycle_events Arc so we can drop it
    // explicitly mid-stream — the post-fix code (01-02) drops its
    // own internal `bus` clone after subscribe, and this external
    // drop is what then causes the Sender refcount to hit zero,
    // triggering `RecvError::Closed` on the next poll.
    let lifecycle_events = state.lifecycle_events.clone();

    let router = build_router(state.clone());

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        body_ndjson_lines(response.into_body()).await
    });

    // Wait for the handler to subscribe to the broadcast bus (it has
    // emitted Accepted by then). Mirrors the s_cp_06 pattern.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Drop every external `Arc<Sender>` clone we hold. Once
    // step 01-02's paired `drop(bus);` lands inside `build_stream`,
    // this external drop closes the channel and the handler's
    // `sub.recv()` returns `RecvError::Closed`.
    drop(lifecycle_events);
    drop(state);

    // Tick past the streaming cap so the test does not hang on the
    // legacy code path (where the in-stream `bus` Arc keeps the
    // channel open and the cap timer fires instead). Mirrors s_cp_06.
    sim_clock.tick(Duration::from_secs(2));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes within outer wall-clock bound")
        .expect("task ok");

    let last = lines.last().expect("at least one NDJSON line on the wire");
    assert_eq!(
        last["kind"], "converged_failed",
        "channel-closed path must produce `converged_failed` terminal; got {lines:?}",
    );

    // Parse the terminal_reason structurally. The legacy code emits
    // `{"kind":"timeout","data":{"after_seconds":0}}` (or, on the
    // RED-test cap-fallback path, `{"kind":"timeout","data":{"after_seconds":1}}`).
    // The post-fix code emits `{"kind":"stream_interrupted"}`.
    let terminal_reason_json = &last["data"]["terminal_reason"];
    let terminal_reason: TerminalReason = serde_json::from_value(terminal_reason_json.clone())
        .expect(&format!(
            "terminal_reason must deserialise to TerminalReason; got {terminal_reason_json:?}",
        ));

    // The load-bearing RED assertion. On the unfixed code path this
    // panics with the documented message. The 01-02 GREEN commit
    // flips this to passing.
    assert!(
        matches!(terminal_reason, TerminalReason::StreamInterrupted),
        "RED scaffold (step 01-01): expected TerminalReason::StreamInterrupted \
         (channel-closed path); got {terminal_reason:?} — GREEN lands in step 01-02 \
         with the variant swap at streaming.rs:286 plus the paired drop(bus); after \
         subscribe. See docs/feature/fix-terminal-reason-channel-closed/deliver/rca.md",
    );
}
