//! Acceptance — `wire-exec-spec-end-to-end` server-side defence-in-depth.
//!
//! Per ADR-0011 / ADR-0015 / ADR-0031 §7: even when the CLI
//! pre-validates client-side, the server runs `Job::from_spec` again
//! on ingress. The new `exec.command` non-empty rule (ADR-0031 §4)
//! must fire on the server lane and surface as
//! `ControlPlaneError::Validation { field: Some("exec.command"), .. }`,
//! which the axum `IntoResponse` impl maps to HTTP 400 with the
//! ADR-0015 RFC 7807 body shape.
//!
//! Mirrors the in-process pattern from `submit_job_idempotency.rs` —
//! constructs an `AppState` from real `LocalIntentStore` over `TempDir`,
//! plus `SimObservationStore` + `SimDriver`, then calls the handler
//! directly. No reqwest, no TLS, no port binding.
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §8 *HTTP handler defence-in-depth*.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::SubmitJobRequest;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::submit_job;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("LocalIntentStore::open"),
    );
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver)
}

fn spec_with_command(command: &str) -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: command.to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
    }
}

#[tokio::test]
async fn submit_job_handler_rejects_empty_exec_command_with_validation_error_naming_field() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    // When the handler is invoked with a spec carrying empty exec.command.
    let result = submit_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(SubmitJobRequest { spec: spec_with_command("") }),
    )
    .await;

    // Then the result is the structured Validation variant naming the field.
    match result {
        Err(ControlPlaneError::Validation { field, message }) => {
            assert_eq!(
                field.as_deref(),
                Some("exec.command"),
                "field must name `exec.command` (the ADR-0011 typed-error contract \
                 the HTTP layer uses to render RFC 7807); got {field:?}",
            );
            assert!(!message.is_empty(), "validation message must be non-empty; got {message:?}");
        }
        Err(other) => panic!(
            "expected ControlPlaneError::Validation {{ field: Some(\"exec.command\"), .. }}; \
             got {other:?}",
        ),
        Ok(body) => panic!(
            "server-side defence-in-depth must reject empty exec.command BEFORE any \
             IntentStore put; handler returned Ok({body:?})",
        ),
    }

    // And no IntentStore put occurred — the key remains absent.
    let key = b"jobs/payments";
    let stored = state.store.get(key).await.expect("get must succeed");
    assert!(
        stored.is_none(),
        "no IntentStore put must occur on a validation rejection; key was populated",
    );
}

#[tokio::test]
async fn submit_job_handler_rejects_whitespace_only_exec_command_with_validation_error() {
    // Companion test — pinning the trim rule on the server lane, not
    // just on the constructor. Mutation that flips trim() away on the
    // server-side path would let "   " through; this test catches it.
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);

    let result = submit_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(SubmitJobRequest { spec: spec_with_command("   ") }),
    )
    .await;

    match result {
        Err(ControlPlaneError::Validation { field, .. }) => {
            assert_eq!(field.as_deref(), Some("exec.command"));
        }
        Err(other) => panic!(
            "expected ControlPlaneError::Validation {{ field: Some(\"exec.command\"), .. }}; \
             got {other:?}",
        ),
        Ok(_) => panic!("whitespace-only exec.command must be rejected on the server lane"),
    }

    // Defence-in-depth: no IntentStore put.
    let stored = state.store.get(b"jobs/payments").await.expect("get must succeed");
    assert!(stored.is_none(), "rejected spec must not reach the store");
}
