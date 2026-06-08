//! Acceptance — Step 01-03e3.
//!
//! `S-SHCP-WIRE-09` through `S-SHCP-WIRE-15` — handler dispatch wiring
//! for the Service-kind submit path. After this step, Service-kind
//! submits route through `streaming::build_service_stream` (the
//! sibling-event surface) instead of the legacy `build_stream`
//! (`SubmitEvent::Converged*`). The taxonomy infrastructure landed
//! in step 01-03e2 (commit `b4d3b411`); this step wires it into the
//! production `handlers.rs:498` dispatch path per ADR-0059 §Q6.
//!
//! Port-to-port discipline: every scenario enters through the
//! production handler entry point (`submit_workload`) via
//! `axum::Router::oneshot(...)` and asserts on the observable NDJSON
//! wire output. No hand-call into `build_service_stream`.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]
#![allow(clippy::large_futures)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::post;
use overdrive_control_plane::AppState;
use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::api::{AllocStateWire, SubmitWorkloadRequest, TransitionSource};
use overdrive_control_plane::handlers::submit_workload;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::transition_reason::{
    ProbeWitness, ServiceFailureReason, StoppedBy, TerminalCondition,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt as _;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sample_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn payments_service_spec() -> ServiceSpecInput {
    ServiceSpecInput {
        id: "payments-v0".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/payments".to_string(),
            args: vec![],
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn build_service_submit_request(spec: &ServiceSpecInput) -> Request<Body> {
    let body =
        serde_json::to_vec(&SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) })
            .expect("serialize");
    Request::builder()
        .method(Method::POST)
        .uri("/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/x-ndjson")
        .body(Body::from(body))
        .expect("build request")
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
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        sample_node(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn build_router(state: AppState) -> Router {
    Router::new().route("/v1/jobs", post(submit_workload)).with_state(state)
}

fn make_lifecycle_event_terminal(
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    to: AllocStateWire,
    reason: TransitionReason,
    terminal: Option<TerminalCondition>,
) -> LifecycleEvent {
    LifecycleEvent {
        alloc_id,
        workload_id,
        from: AllocStateWire::Pending,
        to,
        reason,
        detail: None,
        source: TransitionSource::Driver(DriverType::Exec),
        at: "1@node-a".to_string(),
        terminal,
    }
}

fn emit_lifecycle(state: &AppState, event: LifecycleEvent) {
    let _ = state.lifecycle_events.send(event);
}

/// Spawn a body-consumer task that decodes NDJSON line-by-line and
/// forwards each parsed `Value` to an mpsc channel so the test can
/// observe per-line progress. Returns both the channel receiver and a
/// [`tokio::task::JoinHandle`] that yields the complete list of parsed lines when the
/// stream closes.
fn spawn_response_consumer(
    response: axum::response::Response,
) -> (tokio::sync::mpsc::UnboundedReceiver<Value>, tokio::task::JoinHandle<Vec<Value>>) {
    use http_body_util::BodyExt as _;
    let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let handle = tokio::spawn(async move {
        let mut body = response.into_body();
        let mut lines = Vec::new();
        let mut buf = String::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.expect("body frame");
            if let Ok(data) = frame.into_data() {
                buf.push_str(std::str::from_utf8(&data).expect("utf8"));
                while let Some(pos) = buf.find('\n') {
                    let trimmed = buf[..pos].trim();
                    if !trimmed.is_empty() {
                        let value: Value = serde_json::from_str(trimmed)
                            .expect(&format!("valid json line: {trimmed}"));
                        let _ = line_tx.send(value.clone());
                        lines.push(value);
                    }
                    buf = buf[pos + 1..].to_string();
                }
            }
        }
        let remaining = buf.trim();
        if !remaining.is_empty() {
            let value: Value =
                serde_json::from_str(remaining).expect(&format!("valid json line: {remaining}"));
            let _ = line_tx.send(value.clone());
            lines.push(value);
        }
        lines
    });
    (line_rx, handle)
}

async fn wait_for_accepted(line_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(2), line_rx.recv())
        .await
        .expect("accepted line within 2s")
        .expect("channel open")
}

// ===========================================================================
// S-SHCP-WIRE-09 — Service submit dispatches to ServiceSubmitEvent
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_09_service_submit_dispatches_to_service_submit_event_accepted() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.starts_with("application/x-ndjson"),
        "streaming lane must serve application/x-ndjson; got {content_type}"
    );

    let (mut line_rx, _handle) = spawn_response_consumer(response);

    // Wait for Accepted (synchronous first line).
    let accepted = wait_for_accepted(&mut line_rx).await;
    assert_eq!(accepted["kind"], "accepted", "first wire line must be `accepted`; got {accepted}");
    let data = &accepted["data"];
    assert!(data["spec_digest"].is_string(), "Accepted must carry spec_digest; got {accepted}");
    assert!(data["intent_key"].is_string(), "Accepted must carry intent_key; got {accepted}");
    assert!(data["outcome"].is_string(), "Accepted must carry outcome; got {accepted}");
    // S-SHCP-WIRE-09 anti-shape check: legacy SubmitEvent::Accepted
    // carried a `vip: Option<String>` field. ServiceSubmitEvent::Accepted
    // does NOT.
    assert!(
        data.get("vip").is_none() || data["vip"].is_null(),
        "ServiceSubmitEvent::Accepted must NOT carry `vip` field; got {accepted}"
    );

    // Emit a terminal to close the stream so this scenario does not
    // dangle on the cap-timer.
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");
    let workload_id = WorkloadId::from_str("payments-v0").expect("workload id");
    let witness = ProbeWitness {
        probe_idx: 0,
        role: "startup".to_string(),
        mechanic_summary: "http".to_string(),
        inferred: false,
    };
    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id,
            workload_id,
            AllocStateWire::Running,
            TransitionReason::Started,
            Some(TerminalCondition::Stable { settled_in_ms: 100, witness }),
        ),
    );
}

// ===========================================================================
// S-SHCP-WIRE-10 — Terminal Stable projection
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_10_terminal_stable_projects_to_service_submit_event_stable() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let workload_id = WorkloadId::from_str("payments-v0").expect("workload id");
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    let witness = ProbeWitness {
        probe_idx: 2,
        role: "startup".to_string(),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
    };
    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Running,
            TransitionReason::Started,
            Some(TerminalCondition::Stable { settled_in_ms: 750, witness: witness.clone() }),
        ),
    );

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes")
        .expect("task ok");

    let last = lines.last().expect("at least one streaming line");
    assert_eq!(
        last["kind"], "stable",
        "Service-kind stream must terminate with `stable` on TerminalCondition::Stable; got {last}"
    );
    let data = &last["data"];
    assert_eq!(data["alloc_id"], "alloc-payments-0");
    assert_eq!(data["settled_in_ms"], 750);
    assert_eq!(data["witness"]["probe_idx"], 2);
    assert_eq!(data["witness"]["role"], "startup");
    assert_eq!(data["witness"]["mechanic_summary"], "tcp 0.0.0.0:8080");
    assert_eq!(data["witness"]["inferred"], false);

    let stable_count = lines.iter().filter(|l| l["kind"] == "stable").count();
    assert_eq!(stable_count, 1, "exactly one `stable` line expected; got {stable_count}");
}

// ===========================================================================
// S-SHCP-WIRE-11 — Terminal Failed projection (ServiceFailed reason)
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_11_terminal_service_failed_projects_to_service_submit_event_failed() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let workload_id = WorkloadId::from_str("payments-v0").expect("workload id");
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    let reason = ServiceFailureReason::StartupProbeFailed {
        probe_idx: 0,
        last_fail: "connection refused".to_string(),
        attempts: 3,
    };
    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Failed,
            TransitionReason::Started,
            Some(TerminalCondition::ServiceFailed { reason }),
        ),
    );

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes")
        .expect("task ok");

    let last = lines.last().expect("at least one streaming line");
    assert_eq!(
        last["kind"], "failed",
        "Service-kind stream must emit `failed` on TerminalCondition::ServiceFailed; got {last}"
    );
    let data = &last["data"];
    assert_eq!(data["alloc_id"], "alloc-payments-0");
    assert_eq!(
        data["reason"]["reason"], "startup_probe_failed",
        "reason must project ServiceFailureReason variant verbatim; got {data}"
    );

    let failed_count = lines.iter().filter(|l| l["kind"] == "failed").count();
    assert_eq!(failed_count, 1, "exactly one `failed` line expected");
}

// ===========================================================================
// S-SHCP-WIRE-12 — Terminal Stopped projection (sibling variant of Failed)
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_12_terminal_stopped_projects_to_service_submit_event_stopped_sibling() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let workload_id = WorkloadId::from_str("payments-v0").expect("workload id");
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Terminated,
            TransitionReason::Stopped { by: StoppedBy::Operator },
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        ),
    );

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes")
        .expect("task ok");

    let last = lines.last().expect("at least one streaming line");
    assert_eq!(
        last["kind"], "stopped",
        "Service-kind stream must emit `stopped` (NOT `failed`) on \
         TerminalCondition::Stopped per ADR-0059 Q1; got {last}"
    );
    let data = &last["data"];
    assert_eq!(data["alloc_id"], "alloc-payments-0");
    assert_eq!(data["by"], "operator");

    // Structural defense: NO `failed` line emitted.
    let failed_count = lines.iter().filter(|l| l["kind"] == "failed").count();
    assert_eq!(
        failed_count, 0,
        "Stopped must NOT fold into `failed` per ADR-0059 Q1 sibling-variant invariant"
    );
    let stopped_count = lines.iter().filter(|l| l["kind"] == "stopped").count();
    assert_eq!(stopped_count, 1, "exactly one `stopped` line expected");
}

// ===========================================================================
// S-SHCP-WIRE-13 — Cap timer synthesises Timeout
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_13_cap_timer_synthesises_service_failed_timeout() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(1);
    let clock_for_advance = sim_clock.clone();
    let router = build_router(state.clone());

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    // Advance SimClock past the cap; the handler's `clock.sleep(cap)`
    // future wakes and fires the cap-timer arm.
    clock_for_advance.tick(Duration::from_secs(2));

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes within timeout")
        .expect("task ok");

    let last = lines.last().expect("at least one streaming line");
    assert_eq!(last["kind"], "failed", "cap-timer must synthesise `failed`; got {last}");
    let data = &last["data"];
    assert!(
        data.get("alloc_id").is_none_or(Value::is_null)
            || !data.as_object().expect("object").contains_key("alloc_id"),
        "cap-timer Failed must have alloc_id = None; got {data}"
    );
    assert_eq!(
        data["reason"]["reason"], "timeout",
        "cap-timer must synthesise `ServiceFailureReason::Timeout`; got {data}"
    );
    assert!(
        data["reason"]["data"]["after_seconds"].is_number(),
        "Timeout must carry after_seconds; got {data}"
    );
}

// ===========================================================================
// S-SHCP-WIRE-14 — Broadcast closed synthesises StreamInterrupted
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_14_broadcast_closed_synthesises_stream_interrupted() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    // Drop every external Sender clone — the handler's `Receiver`
    // becomes orphaned, firing `RecvError::Closed` and triggering
    // the StreamInterrupted synthesis arm.
    drop(state);

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes")
        .expect("task ok");

    let last = lines.last().expect("at least one streaming line");
    assert_eq!(last["kind"], "failed", "broadcast Closed must synthesise `failed`; got {last}");
    let data = &last["data"];
    assert_eq!(
        data["reason"]["reason"], "stream_interrupted",
        "broadcast Closed must synthesise `ServiceFailureReason::StreamInterrupted`; got {data}"
    );
}

// ===========================================================================
// S-SHCP-WIRE-15 — No intermediate non-terminal lines on the wire
// ===========================================================================

#[tokio::test]
async fn s_shcp_wire_15_no_intermediate_non_terminal_lines_on_service_wire() {
    let tmp = TempDir::new().expect("tmpdir");
    let sim_clock = Arc::new(SimClock::new());
    let mut state = build_app_state(&tmp, sim_clock.clone());
    state.streaming_cap = Duration::from_secs(60);
    let router = build_router(state.clone());

    let workload_id = WorkloadId::from_str("payments-v0").expect("workload id");
    let alloc_id = AllocationId::from_str("alloc-payments-0").expect("alloc id");

    let response = router
        .oneshot(build_service_submit_request(&payments_service_spec()))
        .await
        .expect("router oneshot");
    let (mut line_rx, handle) = spawn_response_consumer(response);

    let _accepted = wait_for_accepted(&mut line_rx).await;

    // Emit a non-terminal Running event FIRST — the stream must
    // ignore it (no `lifecycle_transition` line, no `running` line).
    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id.clone(),
            workload_id.clone(),
            AllocStateWire::Running,
            TransitionReason::Started,
            None,
        ),
    );
    // Yield to give the handler a chance to process the non-terminal
    // (and discard it).
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Now emit a terminal Stable event to close the stream.
    let witness = ProbeWitness {
        probe_idx: 0,
        role: "startup".to_string(),
        mechanic_summary: "http".to_string(),
        inferred: false,
    };
    emit_lifecycle(
        &state,
        make_lifecycle_event_terminal(
            alloc_id,
            workload_id,
            AllocStateWire::Running,
            TransitionReason::Started,
            Some(TerminalCondition::Stable { settled_in_ms: 100, witness }),
        ),
    );

    let lines = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("stream completes")
        .expect("task ok");

    // Exactly 2 lines: Accepted then Stable. No intermediate.
    assert_eq!(
        lines.len(),
        2,
        "Service-kind wire must emit exactly {{Accepted, terminal}} — no intermediate \
         lines per ADR-0056 / S-SHCP-WIRE-15; got {} lines: {lines:?}",
        lines.len()
    );
    assert_eq!(lines[0]["kind"], "accepted");
    assert_eq!(lines[1]["kind"], "stable");

    let intermediate_kinds = ["lifecycle_transition", "running", "pending", "attempt_failed"];
    for kind in intermediate_kinds {
        let count = lines.iter().filter(|l| l["kind"] == kind).count();
        assert_eq!(count, 0, "Service-kind wire must NOT emit `{kind}` lines; got {count}");
    }
}
