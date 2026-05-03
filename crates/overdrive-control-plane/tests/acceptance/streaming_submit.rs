//! Acceptance — Slice 02 step 02-03.
//!
//! `S-CP-01` / `S-CP-02` / `S-CP-03` / `S-CP-06` / `S-CP-07` / `S-CP-08` /
//! `S-CP-10` / `S-CP-11` — content-negotiated `submit_job` +
//! `streaming_submit_loop` with `select!` cap timer + lagged-recovery
//! fallback + stop-while-streaming closure.
//!
//! Driver: `axum::Router::oneshot(...)` against the production router
//! shape, with `Accept: application/x-ndjson` (streaming) or
//! `application/json` (back-compat) per [D6] / [D8].
//!
//! Per architecture.md §3 (happy path), §4 (broken-binary path), §5
//! (timeout path), and §10 (broadcast wiring).

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]
#![allow(clippy::large_futures)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::post;
use overdrive_control_plane::AppState;
use overdrive_control_plane::action_shim::{LifecycleEvent, dispatch};
use overdrive_control_plane::api::{
    AllocStateWire, IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse, TransitionSource,
};
use overdrive_control_plane::handlers::submit_job;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt as _;

// ---------------------------------------------------------------------------
// Test fixtures
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
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
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

/// Build a `POST /v1/jobs` request with the given Accept header.
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

/// Read the entire response body and split into NDJSON lines (each
/// line a `serde_json::Value`).
async fn body_ndjson_lines(body: Body) -> Vec<Value> {
    let bytes = to_bytes(body, usize::MAX).await.expect("body to bytes");
    let s = std::str::from_utf8(&bytes).expect("utf8 body");
    s.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect(&format!("valid json line: {l}")))
        .collect()
}

/// Read the entire response body and parse as a single JSON object.
async fn body_json(body: Body) -> Value {
    let bytes = to_bytes(body, usize::MAX).await.expect("body to bytes");
    serde_json::from_slice(&bytes).expect("valid json body")
}

/// Write a single `AllocStatusRow` to the observation store. Used by
/// scenarios that need to seed terminal-detection state for the handler
/// to observe.
async fn write_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    job_id: &JobId,
    state: AllocState,
    counter: u64,
    reason: Option<TransitionReason>,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        job_id: job_id.clone(),
        node_id: sample_node(),
        state,
        updated_at: LogicalTimestamp { counter, writer: sample_node() },
        reason,
        detail: None,
        terminal: None,
    };
    obs.write(ObservationRow::AllocStatus(row)).await.expect("obs write");
}

/// Fire a `LifecycleEvent` through the broadcast channel.
fn emit_lifecycle(state: &AppState, event: LifecycleEvent) {
    let _ = state.lifecycle_events.send(event);
}

fn make_lifecycle_event(
    alloc_id: AllocationId,
    job_id: JobId,
    from: AllocStateWire,
    to: AllocStateWire,
    reason: TransitionReason,
) -> LifecycleEvent {
    LifecycleEvent {
        alloc_id,
        job_id,
        from,
        to,
        reason,
        detail: None,
        source: TransitionSource::Driver(DriverType::Exec),
        at: "1@node-a".to_string(),
    }
}

// ===========================================================================
// S-CP-08 — Back-compat: Accept: application/json returns one-shot
// SubmitJobResponse byte-equivalent to the existing handler shape.
// ===========================================================================

#[tokio::test]
async fn s_cp_08_application_json_returns_one_shot_response_with_back_compat_shape() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let router = build_router(state);

    let response = router
        .oneshot(build_submit_request(&payments_spec(), "application/json"))
        .await
        .expect("router oneshot");

    assert_eq!(response.status(), StatusCode::OK, "back-compat lane must return 200");
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type set")
        .to_str()
        .expect("content-type ascii");
    assert!(
        content_type.starts_with("application/json"),
        "back-compat lane must serve application/json; got {content_type}"
    );

    let json = body_json(response.into_body()).await;
    let parsed: SubmitJobResponse =
        serde_json::from_value(json.clone()).expect("body parses to SubmitJobResponse");
    assert_eq!(parsed.job_id, "payments-v0");
    assert_eq!(parsed.outcome, IdempotencyOutcome::Inserted);
    assert_eq!(parsed.spec_digest.len(), 64);
    // No streaming-only fields leak into the JSON shape.
    assert!(
        json.get("kind").is_none(),
        "JSON lane must not carry the streaming `kind` discriminator"
    );
}

#[tokio::test]
async fn s_cp_08b_no_accept_header_defaults_to_json_back_compat() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let router = build_router(state);

    // Build request without Accept header — back-compat with reqwest
    // clients that never send Accept.
    let body = serde_json::to_vec(&SubmitJobRequest { spec: payments_spec() }).expect("serialize");
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("build request");

    let response = router.oneshot(request).await.expect("router oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type set")
        .to_str()
        .expect("ascii")
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "missing Accept must default to JSON back-compat; got {content_type}"
    );
}

// ===========================================================================
// S-CP-01 — Streaming submit emits Accepted+LifecycleTransition+ConvergedRunning
// ===========================================================================

#[tokio::test]
async fn s_cp_01_streaming_lane_emits_accepted_then_running_then_converged_running() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // Spawn the request in a task so we can drive the broadcast
    // channel from the main test fixture concurrently.
    let router = build_router(state.clone());

    // Pre-resolve the alloc id we will inject. The handler does not
    // need to know it in advance — it just observes whatever transitions
    // come through the bus that match the job_id.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    // Drive: send the request and read the body in a task.
    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content-type set")
            .to_str()
            .expect("ascii")
            .to_string();
        assert!(
            content_type.starts_with("application/x-ndjson"),
            "streaming lane must serve application/x-ndjson; got {content_type}"
        );
        body_ndjson_lines(response.into_body()).await
    });

    // Give the handler a moment to commit `Accepted` and subscribe.
    // We wait until the broadcast channel has at least one receiver.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit a LifecycleTransition (Pending → Running) — this should be
    // forwarded to the consumer.
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            job_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Write a Running row matching the desired replica count → handler
    // detects ConvergedRunning. Job has replicas=1, one Running row meets the bar.
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    // Sequence assertions
    assert!(lines.len() >= 3, "expected at least 3 NDJSON lines; got {lines:?}");
    assert_eq!(lines[0]["kind"], "accepted", "first line must be `accepted`");
    let last = lines.last().expect("at least one line");
    assert_eq!(last["kind"], "converged_running", "last line must be `converged_running`");
    // Every line is valid JSON with a `kind` discriminator.
    for line in &lines {
        assert!(line.get("kind").is_some(), "every line must carry `kind`; got {line}");
    }
    // The Accepted line carries spec_digest and outcome.
    assert!(lines[0]["data"]["spec_digest"].is_string());
    assert_eq!(lines[0]["data"]["outcome"], "inserted");
    // At least one LifecycleTransition between accepted and converged_running.
    let has_lt = lines.iter().any(|l| l["kind"] == "lifecycle_transition");
    assert!(has_lt, "expected at least one lifecycle_transition line; got {lines:?}");
}

// ===========================================================================
// S-CP-03 — Re-submit unchanged → outcome: Unchanged
// ===========================================================================

#[tokio::test]
async fn s_cp_03_resubmit_unchanged_emits_accepted_with_unchanged_outcome() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let router = build_router(state.clone());

    // First submit — JSON lane (no streaming) just to install the job.
    let response = router
        .clone()
        .oneshot(build_submit_request(&payments_spec(), "application/json"))
        .await
        .expect("first submit");
    assert_eq!(response.status(), StatusCode::OK);

    // Second submit — streaming lane, byte-identical spec. Pre-seed an
    // already-Running row and also fire a synthetic ConvergedRunning
    // path so the second call terminates without waiting for any
    // additional transitions.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("second submit");
        body_ndjson_lines(response.into_body()).await
    });

    // Give handler time to commit Accepted, then fire a transition so
    // it observes `Running` even if it dropped the pre-seeded row.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            job_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    assert_eq!(lines[0]["kind"], "accepted");
    assert_eq!(lines[0]["data"]["outcome"], "unchanged", "byte-identical resubmit → unchanged");
    let last = lines.last().expect("at least one line");
    assert!(
        last["kind"] == "converged_running" || last["kind"] == "converged_failed",
        "stream must terminate; got {last}"
    );
}

// ===========================================================================
// S-CP-06 — Cap timer fires Timeout terminal under SimClock
// ===========================================================================

#[tokio::test]
async fn s_cp_06_cap_timer_fires_timeout_terminal_when_no_events_arrive() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);
    // Inject a SimClock and a tiny cap so the test can advance through
    // the cap deterministically.
    let sim_clock = Arc::new(SimClock::new());
    state.clock = sim_clock.clone() as Arc<dyn Clock>;
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        body_ndjson_lines(response.into_body()).await
    });

    // Wait for the handler to subscribe (it has emitted Accepted by then).
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Advance the SimClock past the cap. SimClock::sleep returns
    // immediately and bumps the elapsed counter; the handler's cap
    // future should resolve on its next poll.
    sim_clock.tick(Duration::from_secs(61));
    // Yield enough times for the handler to observe the resolved sleep.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes within bound")
        .expect("task ok");

    let last = lines.last().expect("at least one line");
    assert_eq!(last["kind"], "converged_failed", "cap must produce converged_failed terminal");
    let terminal_reason = &last["data"]["terminal_reason"];
    assert_eq!(terminal_reason["kind"], "timeout", "cap must produce Timeout variant");
    assert_eq!(
        terminal_reason["data"]["after_seconds"], 60,
        "Timeout must carry the configured cap"
    );
    // No lifecycle_transition lines between Accepted and Timeout.
    let has_lt = lines.iter().any(|l| l["kind"] == "lifecycle_transition");
    assert!(!has_lt, "no transitions should appear between accepted and timeout; got {lines:?}");
}

// ===========================================================================
// S-CP-06b — RED regression: the streaming cap is documented (streaming.rs
// lines 22-25) as a wall-clock deadline from stream entry. The implementation
// recreates `clock.sleep(cap)` on every loop iteration, so any intervening
// `LifecycleEvent` resets the deadline — turning the cap into an inactivity
// timeout. SimClock semantics make this observable in pure DST: SimClock::sleep
// computes `deadline = elapsed + duration` AT CALL TIME. After tick(30s) and
// one event-induced loop iteration, the new clock.sleep(60s) registers
// deadline=90s. tick(31s) totalling 61s does NOT fire it; under the bug the
// stream hangs.
//
// This test injects exactly ONE non-terminal LifecycleEvent at sim_t=30s,
// then advances to sim_t=61s. Bug path: cap deadline=90s; stream hangs;
// outer wall-clock tokio::time::timeout(2s) fires. Future-fix path: pinned
// cap_future at deadline=60s; cap fires; stream emits Timeout terminal.
// ===========================================================================

#[tokio::test]
async fn s_cp_06b_cap_is_absolute_deadline_not_inactivity_timeout() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);
    // Same SimClock + 60s cap setup as s_cp_06.
    let sim_clock = Arc::new(SimClock::new());
    state.clock = sim_clock.clone() as Arc<dyn Clock>;
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        body_ndjson_lines(response.into_body()).await
    });

    // Wait for the handler to subscribe (cap_future registered at
    // SimClock-deadline=60s under the future fix; under the bug the
    // deadline is recomputed each loop iteration).
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Advance to sim_t=30s. cap_future at 60s does NOT fire.
    sim_clock.tick(Duration::from_secs(30));

    // Inject ONE non-terminal LifecycleEvent. We do NOT call write_row,
    // so the obs store stays empty and check_terminal returns None — the
    // loop iterates without emitting a terminal line. Under the bug,
    // this iteration recreates clock.sleep(60s) at sim_t=30s, registering
    // a NEW deadline at 30s+60s=90s.
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            job_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Yield enough times for the handler to process the event, emit the
    // LifecycleTransition line, run check_terminal → None, and re-enter
    // the select! arm.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    // Advance to sim_t=61s total. Bug path: deadline=90s > 61s, cap
    // does not fire; stream hangs. Future-fix path: deadline=60s ≤ 61s,
    // cap fires; stream emits Timeout terminal.
    sim_clock.tick(Duration::from_secs(31));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    // Wall-clock timeout (NOT SimClock). Under the bug, request_task
    // hangs forever waiting for a cap that was reset.
    let lines = tokio::time::timeout(Duration::from_secs(2), request_task)
        .await
        .expect(
            "stream must terminate within 2s after SimClock advances past cap (61s total) — \
             bug: cap deadline reset by intervening LifecycleEvent at 30s, never fires",
        )
        .expect("task ok");

    let last = lines.last().expect("at least one line");
    assert_eq!(
        last["kind"], "converged_failed",
        "cap must produce converged_failed terminal at absolute 60s deadline"
    );
    let terminal_reason = &last["data"]["terminal_reason"];
    assert_eq!(terminal_reason["kind"], "timeout", "cap must produce Timeout variant");
    assert_eq!(
        terminal_reason["data"]["after_seconds"], 60,
        "Timeout must carry the configured cap (60s), not the inactivity interval"
    );
    // The injected event must have been projected before the timeout fired.
    let has_lt = lines.iter().any(|l| l["kind"] == "lifecycle_transition");
    assert!(
        has_lt,
        "injected LifecycleEvent at sim_t=30s must appear as a lifecycle_transition \
         line before the cap-deadline timeout terminal; got {lines:?}"
    );
}

// ===========================================================================
// S-CP-07 — Streaming reason byte-equals snapshot reason for every
// TransitionReason variant. Smoke version uses one representative variant
// (full proptest sweeping all variants would test the same wire shape).
// ===========================================================================

#[tokio::test]
async fn s_cp_07_streaming_reason_byte_equals_snapshot_reason() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let router = build_router(state.clone());

    // Project a known TransitionReason variant onto both a row write
    // and a broadcast event, then assert the JSON shapes byte-equal
    // each other on the wire.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    let reason =
        TransitionReason::ExecBinaryNotFound { path: "/usr/local/bin/payments".to_string() };

    let req_state = state.clone();
    let req_reason = reason.clone();
    let req_alloc_id = alloc_id.clone();
    let req_job_id = job_id.clone();
    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        body_ndjson_lines(response.into_body()).await
    });

    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit the lifecycle event AFTER the subscriber is live, then write
    // the Running row that drives terminal classification. Mirrors the
    // s_cp_01 ordering — required so the post-subscribe snapshot does
    // not short-circuit the loop before the live event arrives. (The
    // test asserts on the wire shape of the projected
    // `LifecycleTransition`; that line only exists when the loop sees
    // a live event, which requires the subscriber to be live and the
    // obs row to not yet be terminal at subscribe-time.)
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            req_alloc_id.clone(),
            req_job_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            req_reason.clone(),
        ),
    );

    // Write a Running row matching the desired replica count → handler
    // detects ConvergedRunning. Job has replicas=1, one Running row meets the bar.
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    // Find the streaming `LifecycleTransition` with our reason.
    let lt_line = lines
        .iter()
        .find(|l| l["kind"] == "lifecycle_transition")
        .expect("at least one lifecycle_transition line");
    let stream_reason = &lt_line["data"]["reason"];

    // Serialize the same TransitionReason via the snapshot's path
    // (TransitionReason is the single source of truth used by both
    // surfaces).
    let snapshot_reason: Value = serde_json::to_value(&req_reason).expect("serialize reason");
    assert_eq!(
        *stream_reason, snapshot_reason,
        "streaming reason wire shape must byte-equal snapshot reason"
    );

    let _ = req_state;
}

// ===========================================================================
// S-CP-10 — Lagged subscriber recovery
//
// Marked #[ignore] per wave-decisions.md: complexity blew the per-step
// budget. The named test exists as a scaffold so the gap is visible
// and named, not silently dropped. Phase 2+ promotes this to a real
// scenario when multi-tenant streaming makes the case realistic.
// ===========================================================================

#[tokio::test]
#[ignore = "Phase 1 single-subscriber: Lagged is unrealistic; deferred per wave-decisions.md"]
async fn s_cp_10_lagged_subscriber_recovers_via_observation_snapshot() {
    // Scaffold: when promoted, the scenario builds a tiny-capacity
    // broadcast (capacity=4), sends 5 events before the handler reads,
    // observes the handler taking the Lagged(_) branch, falling back
    // to the obs snapshot, and resuming the broadcast subscription
    // until terminal.
}

// ===========================================================================
// S-CP-11 — Stop-while-streaming closes the stream with converged_stopped
//
// RED scaffold (step 01-01): streaming.rs has no Terminated-path in
// check_terminal(), so `converged_stopped` is never emitted. The test
// times out (or hits the 10s cap) and the final-line assertion fails.
// GREEN lands in step 01-02 when check_terminal() gains a Terminated
// arm that emits ConvergedStopped.
// ===========================================================================

#[tokio::test]
async fn s_cp_11_stop_while_streaming_closes_stream_with_stopped_result() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);

    // Use a SimClock with a short cap so the test fails quickly rather
    // than waiting 60 s when converged_stopped is not yet implemented.
    let sim_clock = Arc::new(SimClock::new());
    state.clock = sim_clock.clone() as Arc<dyn Clock>;
    state.streaming_cap = Duration::from_secs(10);

    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        body_ndjson_lines(response.into_body()).await
    });

    // Wait until the handler has subscribed to the broadcast channel
    // (it has emitted `accepted` by this point).
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // AC: Stream stays open after Running transition.
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            job_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Give the handler a moment to process the Running event (it should
    // NOT close the stream here — we haven't seeded a Running obs row so
    // the converged_running path only fires if the handler observes one
    // via the broadcast event directly; we need it to stay open for the
    // next Terminated event).
    //
    // NOTE: s_cp_01 uses write_row(Running) + emit Running to close the
    // stream. Here we intentionally omit the write_row so the stream
    // remains open after the Running event, allowing us to confirm it
    // only closes on the subsequent Terminated event.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // AC: Stream closes after Terminated { Stopped { by: Reconciler } }.
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            job_id.clone(),
            AllocStateWire::Running,
            AllocStateWire::Terminated,
            TransitionReason::Stopped { by: StoppedBy::Reconciler },
        ),
    );

    // The stream must close within 5 s — not by hitting the 10 s cap.
    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("stream closes within 5 s after Terminated event, not via cap timer")
        .expect("task ok");

    // AC: final line has kind == "converged_stopped".
    let last = lines.last().expect("at least one line");
    assert_eq!(
        last["kind"], "converged_stopped",
        "last line must be `converged_stopped` after Stopped-by-Reconciler transition; got {lines:?}"
    );

    // AC: no converged_failed line anywhere in the stream.
    let has_failed = lines.iter().any(|l| l["kind"] == "converged_failed");
    assert!(!has_failed, "stream must not contain `converged_failed`; got {lines:?}");

    // AC: no timeout terminal_reason anywhere in the stream.
    let has_timeout = lines.iter().any(|l| l["data"]["terminal_reason"]["kind"] == "timeout");
    assert!(!has_timeout, "stream must not close via cap timer; got {lines:?}");
}

// ===========================================================================
// S-LT-01 — LifecycleEvent.from reflects prior alloc state (regression)
//
// Exercises action_shim::dispatch() directly — NOT emit_lifecycle().
// This is the only test path that reaches build_lifecycle_event, where
// the bug lives: `from` is set to `to_wire` (the new state) instead of
// the prior state read from observation.
//
// RED phase: fails because build_lifecycle_event sets from: to_wire, so
// both from and to carry Terminated after StopAllocation.
// GREEN phase (step 01-02): passes after the fix reads prior obs state.
// ===========================================================================

#[tokio::test]
async fn s_lt_01_lifecycle_transition_from_reflects_prior_alloc_state() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    // AC-3: Pre-seed a Running AllocStatusRow so dispatch() can find the
    // prior state when it calls find_prior_alloc_row().
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    // AC-4: Subscribe BEFORE calling dispatch so the broadcast event
    // is not missed.
    let mut rx = state.lifecycle_events.subscribe();

    // AC-5: Construct a minimal TickContext and dispatch StopAllocation.
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 1,
        deadline: now + Duration::from_secs(5),
    };

    dispatch(
        // ADR-0037 §4: emission sites outside a reconciler tick (here, a
        // direct test-bench dispatch) emit `terminal: None` — the
        // reconciler is the single source of every terminal claim.
        vec![Action::StopAllocation { alloc_id: alloc_id.clone(), terminal: None }],
        state.driver.as_ref(),
        state.obs.as_ref(),
        &state.lifecycle_events,
        &tick,
    )
    .await
    .expect("dispatch succeeds");

    // Receive the broadcast event within 1 second — a missed event is a
    // test bug (subscribe-before-dispatch ordering was violated).
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("event received within 1s")
        .expect("channel not closed");

    // AC-6: from must reflect the prior (Running) state.
    assert_eq!(
        event.from,
        AllocStateWire::Running,
        "event.from must be Running (the prior state), got {:?}",
        event.from
    );

    // AC-7: to must reflect the new (Terminated) state.
    assert_eq!(
        event.to,
        AllocStateWire::Terminated,
        "event.to must be Terminated (the new state), got {:?}",
        event.to
    );

    // AC-8: The invariant the bug violates — from and to must differ.
    assert_ne!(
        event.from, event.to,
        "event.from ({:?}) must not equal event.to ({:?}): transition must carry prior state",
        event.from, event.to
    );
}

// ===========================================================================
// S-CP-12 — Pre-subscribe race: terminal already in obs store before subscribe
//
// RED scaffold (step 01-01): build_stream subscribes to lifecycle_events
// AFTER the upstream put_if_absent has already triggered the convergence
// loop. With the obs row pre-seeded and no LifecycleEvent broadcast
// (subscribe happens too late), the streaming loop hangs until the 60s
// cap timer fires, emitting a false ConvergedFailed { Timeout }.
//
// GREEN lands in step 01-02 when build_stream gains a lagged_recover
// snapshot call between bus.subscribe() and the loop, projecting the
// pre-existing Running row to ConvergedRunning synchronously.
//
// Carries #[ignore] so lefthook nextest-affected pre-commit pass stays
// green between this commit and the GREEN commit. The GREEN step un-
// ignores it in the same commit as the fix. Mirror of the
// fix-stop-branch-backoff-pending precedent.
// ===========================================================================

#[tokio::test]
async fn s_cp_12_pre_subscribe_terminal_does_not_hang_until_cap() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);

    // Inject SimClock + 60s cap per the s_cp_06 pattern.
    let sim_clock = Arc::new(SimClock::new());
    state.clock = sim_clock.clone() as Arc<dyn Clock>;
    state.streaming_cap = Duration::from_secs(60);

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let job_id = JobId::from_str("payments-v0").expect("job id");

    // Pre-seed a Running AllocStatusRow BEFORE issuing the streaming
    // request — simulates the production race outcome where the
    // convergence loop has already written the obs row by the time
    // build_stream reaches bus.subscribe().
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let router = build_router(state.clone());

    // Spawn the request. Do NOT poll receiver_count(). Do NOT call
    // emit_lifecycle. Do NOT tick the SimClock. The bus stays silent
    // for the entire test.
    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
            .await
            .expect("router oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        body_ndjson_lines(response.into_body()).await
    });

    // Wall-clock timeout (NOT SimClock) — fires before the 60s SimClock
    // cap could possibly be advanced. With the fix in place the request
    // completes synchronously within microseconds via the post-subscribe
    // snapshot path. Without the fix the request hangs (no events on
    // bus, SimClock cap never advances) and this timeout fires.
    let lines = tokio::time::timeout(Duration::from_millis(500), request_task)
        .await
        .expect("request must complete within 500ms wall-clock — bug: hangs until 60s cap")
        .expect("task ok");

    // Assertions: exactly two lines, accepted then converged_running.
    assert_eq!(lines.len(), 2, "expected exactly 2 NDJSON lines; got {lines:?}");
    assert_eq!(lines[0]["kind"], "accepted", "first line must be `accepted`");
    assert_eq!(
        lines[1]["kind"], "converged_running",
        "second line must be `converged_running` (NOT `converged_failed`); got {lines:?}"
    );
    assert_ne!(
        lines[1]["kind"], "converged_failed",
        "must NOT emit converged_failed — bug symptom is false-positive timeout"
    );
    assert_ne!(
        lines[1]["data"]["terminal_reason"]["kind"], "timeout",
        "must NOT emit timeout terminal_reason — bug symptom is cap-fired Timeout"
    );
    assert_eq!(
        lines[1]["data"]["alloc_id"], "alloc-payments-0",
        "converged_running must reference the pre-seeded alloc"
    );
}
