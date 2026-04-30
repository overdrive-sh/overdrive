//! Acceptance — Slice 02 step 02-03.
//!
//! `S-CP-01` / `S-CP-02` / `S-CP-03` / `S-CP-06` / `S-CP-07` / `S-CP-08` /
//! `S-CP-10` — content-negotiated `submit_job` + `streaming_submit_loop`
//! with `select!` cap timer + lagged-recovery fallback.
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
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::post;
use http_body_util::BodyExt as _;
use overdrive_control_plane::AppState;
use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::api::{
    AllocStateWire, IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse, TransitionSource,
};
use overdrive_control_plane::handlers::submit_job;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
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
// S-CP-02 — KPI-01 first-line ≤ 200ms (basic single-case smoke; full
// proptest version would sweep IntentStore latency).
// ===========================================================================

#[tokio::test]
async fn s_cp_02_first_ndjson_line_is_emitted_synchronously_under_200ms() {
    use std::time::Instant;

    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let router = build_router(state);

    let start = Instant::now();
    let response = router
        .oneshot(build_submit_request(&payments_spec(), "application/x-ndjson"))
        .await
        .expect("router oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    // Read the body stream and capture when the first chunk arrives.
    let mut body = response.into_body();
    let mut first_byte_at: Option<Duration> = None;
    while let Some(frame) = body.frame().await {
        let frame = frame.expect("frame");
        if let Ok(data) = frame.into_data() {
            if !data.is_empty() && first_byte_at.is_none() {
                first_byte_at = Some(start.elapsed());
                break;
            }
        }
    }
    let delta = first_byte_at.expect("at least one chunk arrived");
    assert!(
        delta < Duration::from_millis(200),
        "first NDJSON line must arrive within 200ms (KPI-01); was {delta:?}"
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

    // Pre-seed a Running row so the stream terminates on its own.
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &job_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

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
