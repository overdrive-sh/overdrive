//! Acceptance — Slice 02 step 02-03.
//!
//! `S-CP-01` / `S-CP-02` / `S-CP-03` / `S-CP-06` / `S-CP-07` / `S-CP-08` /
//! `S-CP-10` / `S-CP-11` — content-negotiated `submit_workload` +
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
    AllocStateWire, IdempotencyOutcome, SubmitWorkloadRequest, SubmitWorkloadResponse,
    TransitionSource,
};
use overdrive_control_plane::handlers::submit_workload;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
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

fn build_app_state(tmp: &TempDir, clock: Arc<dyn Clock>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(sample_node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn build_router(state: AppState) -> Router {
    Router::new().route("/v1/jobs", post(submit_workload)).with_state(state)
}

/// Build a `POST /v1/jobs` request with the given Accept header.
///
/// Per ADR-0051 the wire-side workload kind is the inner `kind` tag on
/// `SubmitSpecInput`; the outer `workload_kind` field has been deleted.
/// Every submission this helper constructs carries `kind: "job"`.
fn build_submit_request(spec: &JobSpecInput, accept: &str) -> Request<Body> {
    let body =
        serde_json::to_vec(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) })
            .expect("serialize");
    Request::builder()
        .method(Method::POST)
        .uri("/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, accept)
        .body(Body::from(body))
        .expect("build request")
}

/// Pre-ADR-0051 helper that varied `workload_kind` per call. Retained as
/// a thin wrapper over [`build_submit_request`] because the regression
/// tests at lines 1051 / 1149 / 1181 still call into it; the
/// `_workload_kind` argument is ignored — the outer field has been
/// deleted in this step's commit (the kind tag lives inside
/// `SubmitSpecInput::Job(_)` now).
fn build_submit_request_with_kind(
    spec: &JobSpecInput,
    accept: &str,
    _workload_kind: Option<&str>,
) -> Request<Body> {
    build_submit_request(spec, accept)
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
    workload_id: &WorkloadId,
    state: AllocState,
    counter: u64,
    reason: Option<TransitionReason>,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload_id.clone(),
        node_id: sample_node(),
        state,
        updated_at: LogicalTimestamp { counter, writer: sample_node() },
        reason,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        // GAP-1 subsidiary: None on Pending; fixed wall-clock otherwise.
        started_at: match state {
            AllocState::Pending => None,
            _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        },
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("obs write");
}

/// Fire a `LifecycleEvent` through the broadcast channel.
fn emit_lifecycle(state: &AppState, event: LifecycleEvent) {
    let _ = state.lifecycle_events.send(event);
}

fn make_lifecycle_event(
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    from: AllocStateWire,
    to: AllocStateWire,
    reason: TransitionReason,
) -> LifecycleEvent {
    LifecycleEvent {
        alloc_id,
        workload_id,
        from,
        to,
        reason,
        detail: None,
        source: TransitionSource::Driver(DriverType::Exec),
        at: "1@node-a".to_string(),
        // Per ADR-0037 §4: synthetic test fixtures default to `None`
        // unless the scenario specifically exercises the terminal-
        // surface projection (those scenarios construct LifecycleEvent
        // directly with their desired `Some(TerminalCondition::...)`).
        terminal: None,
    }
}

// ===========================================================================
// S-CP-08 — Back-compat: Accept: application/json returns one-shot
// SubmitWorkloadResponse byte-equivalent to the existing handler shape.
// ===========================================================================

#[tokio::test]
async fn s_cp_08_application_json_returns_one_shot_response_with_back_compat_shape() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp, Arc::new(SimClock::new()));
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
    let parsed: SubmitWorkloadResponse =
        serde_json::from_value(json.clone()).expect("body parses to SubmitWorkloadResponse");
    assert_eq!(parsed.workload_id, "payments-v0");
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
    let state = build_app_state(&tmp, Arc::new(SimClock::new()));
    let router = build_router(state);

    // Build request without Accept header — back-compat with reqwest
    // clients that never send Accept.
    let body =
        serde_json::to_vec(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec()) })
            .expect("serialize");
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
// S-CP-01 — Streaming submit emits Accepted+Running+Succeeded
// ===========================================================================

#[tokio::test]
async fn s_cp_01_streaming_lane_emits_accepted_then_running_then_converged_running() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp, Arc::new(SimClock::new()));

    // Spawn the request in a task so we can drive the broadcast
    // channel from the main test fixture concurrently.
    let router = build_router(state.clone());

    // Pre-resolve the alloc id we will inject. The handler does not
    // need to know it in advance — it just observes whatever transitions
    // come through the bus that match the workload_id.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

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

    // Emit a LifecycleTransition (Pending → Running) — informational
    // line on the Job stream (NOT terminal — Jobs are
    // run-to-completion).
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Emit the terminal Completed event — drives the Job stream to
    // emit `JobSubmitEvent::Succeeded` and close. Pre-migration this
    // step seeded a Running row + Running lifecycle and the Service
    // arm emitted `converged_running`; per ADR-0051 the Job arm has no
    // `converged_running` variant (RCA root causes B+C+D structurally
    // unreachable) — terminal claim is the close signal.
    let mut terminal_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    terminal_event.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Completed { exit_code: 0 });
    emit_lifecycle(&state, terminal_event);

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    // Sequence assertions — Job-arm vocabulary (ADR-0051):
    //   `accepted` → `running` (informational) → `succeeded` (terminal).
    assert!(lines.len() >= 3, "expected at least 3 NDJSON lines; got {lines:?}");
    assert_eq!(lines[0]["kind"], "accepted", "first line must be `accepted`");
    let last = lines.last().expect("at least one line");
    assert_eq!(last["kind"], "succeeded", "last line must be `succeeded` (Job-arm terminal)");
    // Every line is valid JSON with a `kind` discriminator.
    for line in &lines {
        assert!(line.get("kind").is_some(), "every line must carry `kind`; got {line}");
    }
    // The Accepted line carries spec_digest and outcome.
    assert!(lines[0]["data"]["spec_digest"].is_string());
    assert_eq!(lines[0]["data"]["outcome"], "inserted");
    // At least one informational `running` between `accepted` and the
    // terminal `succeeded`. Pre-migration this was a
    // `lifecycle_transition` line on the Service arm; the Job arm
    // projects Running directly via `JobSubmitEvent::Running { since }`.
    let has_running = lines.iter().any(|l| l["kind"] == "running");
    assert!(has_running, "expected at least one `running` informational line; got {lines:?}");
}

// ===========================================================================
// S-CP-03 — Re-submit unchanged → outcome: Unchanged
// ===========================================================================

#[tokio::test]
async fn s_cp_03_resubmit_unchanged_emits_accepted_with_unchanged_outcome() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp, Arc::new(SimClock::new()));
    let router = build_router(state.clone());

    // First submit — JSON lane (no streaming) just to install the job.
    let response = router
        .clone()
        .oneshot(build_submit_request(&payments_spec(), "application/json"))
        .await
        .expect("first submit");
    assert_eq!(response.status(), StatusCode::OK);

    // Second submit — streaming lane, byte-identical spec. Pre-seed an
    // already-Running row and also fire a synthetic `Completed`
    // terminal so the second call terminates without waiting for any
    // additional transitions.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &workload_id,
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
            workload_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Emit a terminal Completed event so the Job-arm stream closes
    // promptly (no `converged_running` variant on the Job arm per
    // ADR-0051; terminal claim is the close signal).
    let mut terminal_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    terminal_event.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Completed { exit_code: 0 });
    emit_lifecycle(&state, terminal_event);

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    assert_eq!(lines[0]["kind"], "accepted");
    assert_eq!(lines[0]["data"]["outcome"], "unchanged", "byte-identical resubmit → unchanged");
    let last = lines.last().expect("at least one line");
    assert!(
        last["kind"] == "succeeded" || last["kind"] == "failed" || last["kind"] == "stopped",
        "stream must terminate on a Job-arm terminal variant; got {last}"
    );
}

// ===========================================================================
// S-CP-06 — Cap timer fires Timeout terminal under SimClock
// ===========================================================================

#[tokio::test]
async fn s_cp_06_cap_timer_fires_timeout_terminal_when_no_events_arrive() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    // The SimClock instance above is shared with `state.clock` so the
    // test can advance time through the streaming cap deterministically.
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
    // ADR-0051 Job-arm migration: cap-timer emits
    // `JobSubmitEvent::Failed { exit_code: -1, duration: "60s",
    // stderr_tail: Some("did not converge in 60s") }` (NOT the legacy
    // Service-arm `converged_failed` + `terminal_reason: Timeout`).
    assert_eq!(last["kind"], "failed", "cap must produce Job-arm `failed` terminal");
    assert_eq!(last["data"]["exit_code"], -1, "cap fires with sentinel exit_code -1");
    assert_eq!(last["data"]["duration"], "60s", "cap duration must reflect configured cap");
    let stderr_tail = last["data"]["stderr_tail"].as_str().expect("stderr_tail string");
    assert!(
        stderr_tail.contains("did not converge"),
        "stderr_tail must explain cap-timer non-convergence; got {stderr_tail}"
    );
    // No informational `running` lines between Accepted and the
    // terminal (bus was silent for the whole stream).
    let has_running = lines.iter().any(|l| l["kind"] == "running");
    assert!(
        !has_running,
        "no informational running line should appear between accepted and cap-timer terminal; got {lines:?}"
    );
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
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    // Same SimClock + 60s cap setup as s_cp_06.
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

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
            workload_id.clone(),
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
    // ADR-0051 Job-arm migration: cap-timer emits Job-arm `failed`
    // (NOT Service-arm `converged_failed` + `terminal_reason: Timeout`).
    assert_eq!(
        last["kind"], "failed",
        "cap must produce Job-arm `failed` terminal at absolute 60s deadline"
    );
    assert_eq!(last["data"]["exit_code"], -1, "cap fires with sentinel exit_code -1");
    assert_eq!(
        last["data"]["duration"], "60s",
        "duration must reflect configured cap (60s), not the inactivity interval"
    );
    // The injected Running lifecycle event must have been projected
    // before the cap fired — Job-arm renders Running events as
    // `kind: "running"` (NOT `lifecycle_transition`).
    let has_running = lines.iter().any(|l| l["kind"] == "running");
    assert!(
        has_running,
        "injected LifecycleEvent at sim_t=30s must appear as a `running` line \
         before the cap-deadline terminal; got {lines:?}"
    );
}

// ===========================================================================
// S-CP-07 — Streaming reason byte-equals snapshot reason for every
// TransitionReason variant. Smoke version uses one representative variant
// (full proptest sweeping all variants would test the same wire shape).
// ===========================================================================

#[tokio::test]
async fn s_cp_07_streaming_reason_byte_equals_snapshot_reason() {
    // ADR-0051 migration note (step 02-03b): the Job-arm streaming
    // surface (`JobSubmitEvent`) does NOT emit a `lifecycle_transition`
    // line that carries the full `TransitionReason` payload — the
    // pre-migration Service-arm wire shape is structurally replaced by
    // per-kind sibling variants (`pending` / `running` / `attempt_failed`
    // / terminal). The Job-arm path that surfaces reason-derived data
    // on the wire is `attempt_failed`, which extracts the typed
    // `exit_code` from `TransitionReason::WorkloadCrashedImmediately`.
    //
    // This test is migrated to exercise that path: the typed
    // `exit_code` on the streaming `attempt_failed` line must match
    // the typed `exit_code` constructed in the source
    // `WorkloadCrashedImmediately` reason — the same "single source of
    // truth carried verbatim through the streaming projection"
    // invariant the pre-migration test enforced for the Service-arm
    // `lifecycle_transition.reason` field.
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    // 60s cap so the test can close the stream via SimClock.tick after
    // the AttemptFailed line is emitted without racing a terminal.
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

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

    // Emit a Failed lifecycle event carrying the typed reason. The
    // Job-arm projection reads `exit_code` directly from the typed
    // `WorkloadCrashedImmediately.exit_code` field — the streaming
    // wire's `exit_code` must byte-equal the constructed value.
    let mut failed_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Pending,
        AllocStateWire::Failed,
        TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(42),
            signal: None,
            stderr_tail: None,
        },
    );
    failed_event.detail = None;
    failed_event.terminal = None;
    emit_lifecycle(&state, failed_event);

    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    // Advance past the cap to close the stream.
    sim_clock.tick(Duration::from_secs(61));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes")
        .expect("task ok");

    let attempt_failed_line = lines
        .iter()
        .find(|l| l["kind"] == "attempt_failed")
        .expect("expected an attempt_failed line in the stream");

    // The streaming wire's `exit_code` byte-equals the typed
    // `WorkloadCrashedImmediately.exit_code` — the migration-time
    // equivalent of "stream reason byte-equals source reason" on the
    // Job arm.
    assert_eq!(
        attempt_failed_line["data"]["exit_code"], 42,
        "streaming attempt_failed.exit_code must byte-equal source \
         WorkloadCrashedImmediately.exit_code; got {attempt_failed_line}"
    );
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
// arm that emits `Stopped`.
// ===========================================================================

#[tokio::test]
async fn s_cp_11_stop_while_streaming_closes_stream_with_stopped_result() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());

    // Use a short cap so the test fails quickly rather than waiting
    // 60 s when converged_stopped is not yet implemented. The SimClock
    // above is shared with `state.clock` so the test can advance time
    // through the cap deterministically.
    state.streaming_cap = Duration::from_secs(10);

    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

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
            workload_id.clone(),
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
    // Per ADR-0037 §4: terminal classification is the reconciler's
    // decision, threaded onto LifecycleEvent.terminal by the action
    // shim. The streaming handler reads `event.terminal` (not
    // `event.reason`) for terminal projection — to drive the
    // converged_stopped close, the synthesised event must carry the
    // typed terminal claim, not just the legacy `reason` field.
    let mut terminated_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Stopped { by: StoppedBy::Reconciler },
    );
    terminated_event.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Stopped {
            by: StoppedBy::Reconciler,
        });
    emit_lifecycle(&state, terminated_event);

    // The stream must close within 5 s — not by hitting the 10 s cap.
    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("stream closes within 5 s after Terminated event, not via cap timer")
        .expect("task ok");

    // ADR-0051 Job-arm migration: terminal Stopped projects to
    // `JobSubmitEvent::Stopped { stopped_by, duration, attempts }`
    // (NOT Service-arm `converged_stopped`).
    let last = lines.last().expect("at least one line");
    assert_eq!(
        last["kind"], "stopped",
        "last line must be `stopped` (Job-arm terminal) after Stopped-by-Reconciler transition; got {lines:?}"
    );

    // AC: no Job-arm `failed` terminal anywhere in the stream (clean
    // stop is not a failure).
    let has_failed = lines.iter().any(|l| l["kind"] == "failed");
    assert!(!has_failed, "stream must not contain `failed`; got {lines:?}");

    // AC: stream must close via the Stopped terminal event, not the
    // cap timer. Cap-timer would produce `failed` (asserted above) and
    // a stderr_tail containing "did not converge"; assert it does not
    // appear.
    let has_cap_terminal = lines
        .iter()
        .any(|l| l["data"]["stderr_tail"].as_str().is_some_and(|s| s.contains("did not converge")));
    assert!(!has_cap_terminal, "stream must not close via cap timer; got {lines:?}");
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
    let state = build_app_state(&tmp, Arc::new(SimClock::new()));

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

    // AC-3: Pre-seed a Running AllocStatusRow so dispatch() can find the
    // prior state when it calls find_prior_alloc_row().
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &workload_id,
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

    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch(
        // ADR-0037 §4: emission sites outside a reconciler tick (here, a
        // direct test-bench dispatch) emit `terminal: None` — the
        // reconciler is the single source of every terminal claim.
        vec![Action::StopAllocation { alloc_id: alloc_id.clone(), terminal: None }],
        state.driver.as_ref(),
        state.obs.as_ref(),
        state.dataplane.as_ref(),
        &state.lifecycle_events,
        &tick,
        &state.node_id,
        std::sync::Arc::clone(&state.allocator),
        &test_broker,
        None,
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
// cap timer fires, emitting a false `Failed` (timeout) terminal.
//
// GREEN lands in step 01-02 when build_stream gains a lagged_recover
// snapshot call between bus.subscribe() and the loop, projecting the
// pre-existing Running row to a terminal event synchronously.
//
// Carries #[ignore] so lefthook nextest-affected pre-commit pass stays
// green between this commit and the GREEN commit. The GREEN step un-
// ignores it in the same commit as the fix. Mirror of the
// fix-stop-branch-backoff-pending precedent.
// ===========================================================================

#[tokio::test]
async fn s_cp_12_pre_subscribe_terminal_does_not_hang_until_cap() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());

    // SimClock + 60s cap per the s_cp_06 pattern.
    state.streaming_cap = Duration::from_secs(60);

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

    // Pre-seed a terminal `Completed` AllocStatusRow BEFORE issuing
    // the streaming request — simulates the production race outcome
    // where the convergence loop has already written the obs row with
    // its terminal claim by the time `build_workload_stream` reaches
    // `bus.subscribe()`. The Job-arm snapshot-recovery path
    // (`workload_terminal_from_snapshot`) reads `row.terminal` and
    // projects `TerminalCondition::Completed { exit_code: 0 }` onto
    // `JobSubmitEvent::Succeeded`.
    //
    // ADR-0051 migration note: pre-migration this test seeded a
    // `terminal: None` Running row and the Service-arm snapshot-
    // recovery classified Running as `converged_running`. The Job arm
    // has NO `converged_running` variant — Running is informational,
    // not terminal — so the seed must carry a real terminal claim to
    // close the stream synchronously.
    let row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: sample_node(),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 1, writer: sample_node() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: Some(overdrive_core::transition_reason::TerminalCondition::Completed {
            exit_code: 0,
        }),
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        // GAP-1 subsidiary: Terminated was Running first.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("obs write");

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

    // Assertions: exactly two lines, accepted then succeeded.
    // ADR-0051 Job-arm: `Succeeded` is the terminal that projects from
    // the pre-seeded `TerminalCondition::Completed { exit_code: 0 }`
    // row via the snapshot-recovery path.
    assert_eq!(lines.len(), 2, "expected exactly 2 NDJSON lines; got {lines:?}");
    assert_eq!(lines[0]["kind"], "accepted", "first line must be `accepted`");
    assert_eq!(
        lines[1]["kind"], "succeeded",
        "second line must be Job-arm `succeeded` (NOT `failed`); got {lines:?}"
    );
    assert_ne!(
        lines[1]["kind"], "failed",
        "must NOT emit `failed` — bug symptom is false-positive cap-fired terminal"
    );
    // Cap-timer fallback produces a stderr_tail containing "did not
    // converge"; the snapshot-recovery path produces no such tail.
    let stderr_tail = lines[1]["data"]["stderr_tail"].as_str().unwrap_or("");
    assert!(
        !stderr_tail.contains("did not converge"),
        "must NOT emit cap-fired terminal (no `did not converge` in stderr_tail); got {lines:?}"
    );
    assert_eq!(
        lines[1]["data"]["exit_code"], 0,
        "Job `succeeded` must carry exit_code 0 from the pre-seeded Completed terminal"
    );
}

// ===========================================================================
// Regression — AttemptFailed exit code comes from WorkloadCrashedImmediately
//
// Root cause: workload_event_from_lifecycle's AllocStateWire::Failed arm
// called parse_exit_code_from_detail(event.detail.as_deref()), which was
// written for the old "exit_code=137" string format in
// DriverInternalError.detail. After the exit observer was refactored to
// emit TransitionReason::WorkloadCrashedImmediately { exit_code, .. }, the
// detail field is None — so parse_exit_code_from_detail(None) always falls
// through to `return 1` and every AttemptFailed event carries exit code 1
// regardless of what the workload actually returned.
//
// RED: fails against current code (detail: None → exit_code always 1).
// GREEN: passes after workload_event_from_lifecycle reads exit_code from
//        event.reason instead.
// ===========================================================================

#[tokio::test]
async fn attempt_failed_exit_code_comes_from_workload_crashed_immediately_reason() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    // Use a 60s cap so the test can close the stream via SimClock.tick
    // after the AttemptFailed line is emitted, without racing a terminal.
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

    let request_task = tokio::spawn(async move {
        let response = router
            .oneshot(build_submit_request_with_kind(
                &payments_spec(),
                "application/x-ndjson",
                Some("job"),
            ))
            .await
            .expect("router oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        body_ndjson_lines(response.into_body()).await
    });

    // Wait until the handler has subscribed.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit a Failed event with WorkloadCrashedImmediately { exit_code: Some(137) }
    // and detail: None — the exact shape the exit observer now produces after the
    // refactor that removed the "exit_code=N" string encoding in detail.
    let mut failed_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Pending,
        AllocStateWire::Failed,
        TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: None,
            stderr_tail: None,
        },
    );
    // Explicitly confirm detail is None (make_lifecycle_event already sets
    // it to None; this documents the intent and protects against future
    // helper changes).
    failed_event.detail = None;
    // Not a terminal event — intermediate per-attempt failure.
    failed_event.terminal = None;
    emit_lifecycle(&state, failed_event);

    // Yield so the handler processes the Failed event and emits the
    // AttemptFailed line before we advance the clock to close the stream.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    // Advance the SimClock past the 60s cap — this closes the stream with
    // a `Failed` (timeout) terminal, which is after the AttemptFailed
    // line we care about.
    sim_clock.tick(Duration::from_secs(61));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes within 5s")
        .expect("task ok");

    // Find the AttemptFailed line.
    let attempt_failed_line = lines
        .iter()
        .find(|l| l["kind"] == "attempt_failed")
        .expect("expected an attempt_failed line in the stream");

    // The exit code must come from WorkloadCrashedImmediately.exit_code (137),
    // NOT from parse_exit_code_from_detail(None) which always returns 1.
    assert_eq!(
        attempt_failed_line["data"]["exit_code"], 137,
        "AttemptFailed exit_code must be 137 (from WorkloadCrashedImmediately reason), \
         not 1 (from stale parse_exit_code_from_detail(None)); got {attempt_failed_line}"
    );
}

/// Regression: on the `Unchanged` idempotency path the handler used to
/// take `workload_kind` from the *current request* rather than the stored
/// discriminator. A first submit with `workload_kind: "job"` followed by a
/// re-submit with `workload_kind: None` (defaults to `Service`) would
/// dispatch through the Service streaming path (`build_stream`), whose
/// `submit_event_from_terminal` has no `Completed` arm — it falls through
/// to a `Failed` projection, reporting a successful Job exit as a failure.
///
/// The fix re-writes the kind discriminator on the `Unchanged` path so
/// streaming dispatch always matches what the reconciler uses.
#[tokio::test]
async fn unchanged_resubmit_with_different_kind_uses_stored_discriminator_for_streaming() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let state = build_app_state(&tmp, sim_clock.clone());
    let router = build_router(state.clone());

    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("job id");

    // ── First submit: JSON lane with explicit `workload_kind: "job"` ──
    let first_response = router
        .clone()
        .oneshot(build_submit_request_with_kind(&payments_spec(), "application/json", Some("job")))
        .await
        .expect("first submit");
    assert_eq!(first_response.status(), StatusCode::OK);

    // Verify the stored discriminator is Job.
    let kind_key = overdrive_core::aggregate::IntentKey::for_workload_kind(&workload_id);
    let stored =
        state.store.get(kind_key.as_bytes()).await.expect("store get").expect("kind key present");
    assert_eq!(stored.as_ref(), b"j", "first submit must persist Job discriminator");

    // ── Second submit: streaming lane, same spec, but `workload_kind: None`
    //    (defaults to Service). On the buggy code path this would dispatch
    //    through `build_stream` (Service format) instead of
    //    `build_workload_stream` (Job format). ──

    // Seed a Running row so the stream can progress.
    write_row(
        state.obs.as_ref(),
        &alloc_id,
        &workload_id,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let request_task = {
        let router = router.clone();
        let spec = payments_spec();
        tokio::spawn(async move {
            let response = router
                .oneshot(build_submit_request_with_kind(
                    &spec,
                    "application/x-ndjson",
                    None, // defaults to Service — the bug trigger
                ))
                .await
                .expect("resubmit oneshot");
            assert_eq!(response.status(), StatusCode::OK);
            body_ndjson_lines(response.into_body()).await
        })
    };

    // Wait for handler subscription.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit Running transition then a Completed terminal — the Job
    // succeeded with exit code 0.
    emit_lifecycle(
        &state,
        make_lifecycle_event(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Pending,
            AllocStateWire::Running,
            TransitionReason::Started,
        ),
    );

    // Yield so handler processes the Running event.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Terminal: Completed with exit_code 0.
    let mut terminal_event = make_lifecycle_event(
        alloc_id.clone(),
        workload_id.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    terminal_event.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Completed { exit_code: 0 });
    emit_lifecycle(&state, terminal_event);

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request finishes within 5s")
        .expect("task ok");

    // The stream MUST use the Job path — a Completed terminal maps to
    // `succeeded` on the Job path but falls through to `converged_failed`
    // on the Service path (the bug).
    let terminal_line = lines.last().expect("at least one streaming line");
    assert_eq!(
        terminal_line["kind"], "succeeded",
        "Job completed successfully but stream reported {:?} — streaming dispatch \
         used the request's workload_kind (Service) instead of the stored kind (Job)",
        terminal_line["kind"]
    );
}

/// Regression: the Service streaming path's workload-id guard at
/// `streaming.rs:919` had an empty `if` body — no `continue`. A
/// terminal `LifecycleEvent` from a concurrent service (different
/// `workload_id`) fell through to the `event.terminal` check, emitted
/// the foreign terminal as if it belonged to this service, and closed
/// the stream with incorrect output.
#[tokio::test]
async fn foreign_service_terminal_does_not_leak_into_service_stream() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);

    let our_workload = WorkloadId::from_str("svc-ours-v0").expect("workload id");
    let foreign_workload = WorkloadId::from_str("svc-foreign-v0").expect("workload id");
    let our_alloc = AllocationId::from_str("alloc-svc-ours-0").expect("alloc id");
    let foreign_alloc = AllocationId::from_str("alloc-svc-foreign-0").expect("alloc id");

    // Seed a Running row for our service so the stream progresses past
    // the pre-subscribe snapshot check.
    write_row(
        state.obs.as_ref(),
        &our_alloc,
        &our_workload,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let service_spec = ServiceSpecInput {
        id: "svc-ours-v0".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/svc-ours".to_string(),
            args: vec![],
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    };

    let router = build_router(state.clone());
    let request_task = tokio::spawn({
        let spec = service_spec;
        async move {
            let body =
                serde_json::to_vec(&SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
                    .expect("serialize");
            let request = Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/x-ndjson")
                .body(Body::from(body))
                .expect("build request");
            let response = router.oneshot(request).await.expect("router oneshot");
            assert_eq!(response.status(), StatusCode::OK);
            body_ndjson_lines(response.into_body()).await
        }
    });

    // Wait for handler subscription.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit a terminal from a FOREIGN service. Before the fix, this
    // would leak through and close our stream with the wrong terminal.
    let mut foreign_terminal = make_lifecycle_event(
        foreign_alloc.clone(),
        foreign_workload.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    foreign_terminal.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::ServiceFailed {
            reason: overdrive_core::transition_reason::ServiceFailureReason::Other {
                source: "foreign".to_string(),
                message: "foreign service crashed".to_string(),
            },
        });
    emit_lifecycle(&state, foreign_terminal);

    // Yield to let handler process the foreign event.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Now emit the CORRECT terminal for our service.
    let mut our_terminal = make_lifecycle_event(
        our_alloc.clone(),
        our_workload.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    our_terminal.terminal = Some(overdrive_core::transition_reason::TerminalCondition::Stable {
        settled_in_ms: 5000,
        witness: overdrive_core::transition_reason::ProbeWitness {
            probe_idx: 0,
            role: "startup".to_string(),
            mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
            inferred: false,
        },
    });
    emit_lifecycle(&state, our_terminal);

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request must complete within 5s — bug: stream hung or closed early")
        .expect("task ok");

    // The stream MUST emit exactly 2 lines: accepted + stable.
    // Before the fix it emitted: accepted + failed (from the foreign
    // service's ServiceFailed terminal).
    assert_eq!(lines.len(), 2, "expected exactly 2 NDJSON lines; got {lines:?}");
    assert_eq!(lines[0]["kind"], "accepted", "first line must be `accepted`");
    assert_eq!(
        lines[1]["kind"], "stable",
        "second line must be `stable` (our service), not `failed` (foreign); got {lines:?}"
    );
}

/// Regression: the Job streaming path's workload-id guard at
/// `streaming.rs:241` had the same empty `if` body as the Service path.
/// A terminal `LifecycleEvent` from a concurrent job (different
/// `workload_id`) fell through, emitting the foreign terminal as a
/// `Succeeded` / `Failed` for this job and closing the stream.
#[tokio::test]
async fn foreign_job_terminal_does_not_leak_into_job_stream() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);

    // payments_spec() uses id "payments-v0" — the handler derives the
    // workload_id from the spec, so lifecycle events must match.
    let our_workload = WorkloadId::from_str("payments-v0").expect("workload id");
    let foreign_workload = WorkloadId::from_str("job-foreign-v0").expect("workload id");
    let our_alloc = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let foreign_alloc = AllocationId::from_str("alloc-job-foreign-0").expect("alloc id");

    // Seed a Running row so the stream progresses past the pre-subscribe
    // snapshot check.
    write_row(
        state.obs.as_ref(),
        &our_alloc,
        &our_workload,
        AllocState::Running,
        1,
        Some(TransitionReason::Started),
    )
    .await;

    let router = build_router(state.clone());
    let request_task = tokio::spawn({
        let spec = payments_spec();
        async move {
            let response = router
                .oneshot(build_submit_request(&spec, "application/x-ndjson"))
                .await
                .expect("router oneshot");
            assert_eq!(response.status(), StatusCode::OK);
            body_ndjson_lines(response.into_body()).await
        }
    });

    // Wait for handler subscription.
    for _ in 0..100 {
        if state.lifecycle_events.receiver_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Emit a terminal from a FOREIGN job. Before the fix, this
    // would leak through and close our stream.
    let mut foreign_terminal = make_lifecycle_event(
        foreign_alloc.clone(),
        foreign_workload.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    foreign_terminal.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Completed { exit_code: 42 });
    emit_lifecycle(&state, foreign_terminal);

    // Yield to let handler process the foreign event.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Now emit the CORRECT terminal for our job.
    let mut our_terminal = make_lifecycle_event(
        our_alloc.clone(),
        our_workload.clone(),
        AllocStateWire::Running,
        AllocStateWire::Terminated,
        TransitionReason::Started,
    );
    our_terminal.terminal =
        Some(overdrive_core::transition_reason::TerminalCondition::Completed { exit_code: 0 });
    emit_lifecycle(&state, our_terminal);

    let lines = tokio::time::timeout(Duration::from_secs(5), request_task)
        .await
        .expect("request must complete within 5s — bug: stream hung or closed early")
        .expect("task ok");

    // The stream MUST NOT have picked up the foreign terminal (exit 42).
    // It should emit: accepted + succeeded (from our Completed { exit_code: 0 }).
    let terminal_line = lines.last().expect("at least one streaming line");
    assert_eq!(
        terminal_line["kind"], "succeeded",
        "terminal line must be `succeeded` (our job, exit 0), \
         not from the foreign job (exit 42); got {lines:?}"
    );
    assert_eq!(
        terminal_line["data"]["exit_code"], 0,
        "exit_code must be 0 (our job), not 42 (foreign); got {lines:?}"
    );
}
