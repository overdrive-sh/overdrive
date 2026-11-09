//! D14-E2: the production Service submit handler preserves exact aggregate
//! causes and returns the approved HTTP 400 validation fields.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::post;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::error::{ControlPlaneError, to_response};
use overdrive_control_plane::handlers::submit_workload;
use overdrive_core::aggregate::{AggregateError, ProbeDescriptor, ProbeMechanic, Service};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::traits::intent_store::IntentStore;
use tower::ServiceExt as _;

use super::service_stream_support::{Node, service_spec};

const fn guest(role: ProbeRole) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic: ProbeMechanic::GuestListeners,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

/// D14-E2: raw JSON is admitted through the real handler, then rejected
/// before intent insertion. The nested typed cause and HTTP field agree.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER: D14-E2 typed Service ingress and HTTP field mapping"]
async fn invalid_guest_listeners_raw_service_is_rejected_at_http_ingress() {
    for (case, expected_field) in [
        ("readiness", "readiness_probes[0].mechanic"),
        ("liveness", "liveness_probes[0].mechanic"),
        ("startup-role", "startup_probes[0].role"),
        ("not-sole", "startup_probes"),
    ] {
        let node = Node::boot(1400).await;
        let mut spec = service_spec(case, 2, 30);
        spec.startup_probes.clear();
        match case {
            "readiness" => spec.readiness_probes.push(guest(ProbeRole::Readiness)),
            "liveness" => spec.liveness_probes.push(guest(ProbeRole::Liveness)),
            "startup-role" => spec.startup_probes.push(guest(ProbeRole::Readiness)),
            "not-sole" => {
                spec.startup_probes.push(guest(ProbeRole::Startup));
                spec.startup_probes.push(guest(ProbeRole::Startup));
            }
            _ => unreachable!(),
        }
        let before = node.state.store.scan_prefix(b"").await.expect("intent scan");
        let request_body = serde_json::to_vec(&SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(spec.clone()),
        })
        .expect("raw JSON body");
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/workloads")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(request_body))
            .expect("submit request");
        let router = Router::new()
            .route("/v1/workloads", post(submit_workload))
            .with_state(node.state.clone());
        let response = router.oneshot(request).await.expect("handler response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{case}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body");
        let wire: serde_json::Value = serde_json::from_slice(&bytes).expect("error body");
        assert_eq!(wire["error"], "validation", "{case}");
        assert_eq!(wire["field"], expected_field, "{case}");
        let after = node.state.store.scan_prefix(b"").await.expect("intent scan");
        assert_eq!(after, before, "{case}: rejected input has no intent-store delta");

        let cause = Service::from_submit(spec.clone()).expect_err("invalid raw Service");
        match (case, &cause) {
            (
                "readiness",
                AggregateError::GuestListenersInvalidRole {
                    vector_role: ProbeRole::Readiness,
                    descriptor_role: ProbeRole::Readiness,
                    index: 0,
                },
            )
            | (
                "liveness",
                AggregateError::GuestListenersInvalidRole {
                    vector_role: ProbeRole::Liveness,
                    descriptor_role: ProbeRole::Liveness,
                    index: 0,
                },
            )
            | (
                "startup-role",
                AggregateError::GuestListenersInvalidRole {
                    vector_role: ProbeRole::Startup,
                    descriptor_role: ProbeRole::Readiness,
                    index: 0,
                },
            )
            | ("not-sole", AggregateError::GuestListenersNotSoleStartup { count: 2 }) => {}
            _ => panic!("{case}: wrong typed cause: {cause:?}"),
        }
        let (typed_status, typed_body) = to_response(ControlPlaneError::Aggregate(cause));
        assert_eq!(typed_status, StatusCode::BAD_REQUEST);
        assert_eq!(typed_body.error, "validation");
        assert_eq!(typed_body.field.as_deref(), Some(expected_field));
    }
}
