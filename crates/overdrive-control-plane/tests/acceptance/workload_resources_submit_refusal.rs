//! GH #372 (DD-8, AC-1.3) — the real submit handler refuses a memory that is not a whole number of
//! MiB as a `400 validation` on `memory` and stores nothing (the one composed-path obligation of this
//! feature, host-pure like `guest_listeners_http_rejection`). Integration-gated with its siblings
//! because it boots the shared `service_stream_support::Node` fixture. Pending body is complete and
//! marked `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::{get, post};
use overdrive_control_plane::handlers::{describe_workload, submit_workload};
use overdrive_testing::placement;
use serde_json::json;
use tower::ServiceExt as _;

use super::service_stream_support::Node;

placement!(host_pure);

/// DD-8 / AC-1.3 (composed-path obligation): a submit whose `memory` is not a whole number of MiB
/// goes through the real `submit_workload` handler and is refused `400 validation` with
/// `field: "memory"`, the rule and the received canonical value; the intent store is exactly as it
/// was and `describe` answers 404 afterwards. The same body with a whole-MiB memory is the healthy
/// control the aggregate matrix in `workload_resources_spec` covers. CONTRACT_SHAPE:
/// unbounded-preservation (the refusal leaves the store as it was).
#[tokio::test]
async fn sad_memory_that_is_not_a_whole_number_of_mib_is_refused_and_nothing_is_stored() {
    let node = Node::boot(372).await;
    let router = Router::new()
        .route("/v1/workloads", post(submit_workload))
        .route("/v1/workloads/:id", get(describe_workload))
        .with_state(node.state.clone());
    let before = node.state.store.scan_prefix(b"").await.expect("intent scan");

    let body = json!({ "spec": {
        "kind": "job", "id": "payments", "replicas": 1,
        "resources": { "cpu": "500m", "memory": "1000B" },
        "vm": { "command": "/bin/true", "args": [], "kernel": "/kernel", "rootfs": "/rootfs" }
    }});
    let submit = Request::builder()
        .method(Method::POST)
        .uri("/v1/workloads")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("body")))
        .expect("submit request");
    let response = router.clone().oneshot(submit).await.expect("handler response");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the submit must be refused as a 400 validation naming the memory field"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body");
    let wire: serde_json::Value = serde_json::from_slice(&bytes).expect("error body");
    assert_eq!(wire["error"], "validation");
    assert_eq!(wire["field"], "memory");
    let message = wire["message"].as_str().expect("message text");
    assert!(message.contains("whole number of MiB"), "the refusal must name the rule: {message}");
    assert!(
        message.contains("1000B"),
        "the refusal must name the received canonical value: {message}"
    );

    let after = node.state.store.scan_prefix(b"").await.expect("intent scan");
    assert_eq!(after, before, "a refused spec must leave the intent store as it was");
    let describe = Request::builder()
        .method(Method::GET)
        .uri("/v1/workloads/payments")
        .body(Body::empty())
        .expect("describe request");
    let response = router.oneshot(describe).await.expect("handler response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "a refused spec must store nothing");
}
