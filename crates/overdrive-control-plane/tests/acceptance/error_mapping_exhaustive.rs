//! Acceptance: `ControlPlaneError::to_response` maps every variant to
//! the ADR-0015 `(status, error-kind)` pair.
//!
//! Step 03-05 — the contract lane every handler funnels into. The
//! match inside `to_response` is exhaustive at the enum level
//! (Rust's exhaustiveness check catches a missing arm as a compile
//! error). These tests enumerate the full variant surface and pin the
//! observable response shape (status code, `error` kind, `field`
//! population) so a silent drift in the mapping breaks the build.
//!
//! Per ADR-0015 §3 the response body is
//! `{ error: String, message: String, field: Option<String> }` —
//! the `error` field is the stable kind enum surface
//! (`"validation" | "not_found" | "conflict" | "internal"`).

use std::io;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use overdrive_control_plane::api::ErrorBody;
use overdrive_control_plane::ca_boot::CaBootError;
use overdrive_control_plane::error::{ControlPlaneError, to_response};
use overdrive_control_plane::tls_bootstrap::TlsBootstrapError;
use overdrive_core::aggregate::AggregateError;
use overdrive_core::ca::kek::KekError;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::traits::ca::CaError;
use overdrive_core::traits::intent_store::IntentStoreError;
use overdrive_core::traits::observation_store::ObservationStoreError;

// ---------------------------------------------------------------------------
// Variant × response mapping — exhaustive per ADR-0015 Table §3
// ---------------------------------------------------------------------------

#[test]
fn validation_error_renders_as_400_with_validation_kind_and_field() {
    let err = ControlPlaneError::Validation {
        field: Some("replicas".into()),
        message: "must be > 0".into(),
    };

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, "validation");
    assert_eq!(body.message, "must be > 0");
    assert_eq!(body.field.as_deref(), Some("replicas"));
}

#[test]
fn validation_error_without_field_renders_as_400_with_none_field() {
    let err = ControlPlaneError::Validation { field: None, message: "spec rejected".into() };

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, "validation");
    assert!(body.field.is_none());
}

#[test]
fn not_found_error_renders_as_404_with_not_found_kind() {
    let err = ControlPlaneError::NotFound { resource: "workloads/unknown-id".into() };

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body.error, "not_found");
    assert!(
        body.message.contains("workloads/unknown-id"),
        "message must name the missing resource, got {:?}",
        body.message,
    );
    assert!(body.field.is_none());
}

#[test]
fn conflict_error_renders_as_409_with_conflict_kind() {
    let err = ControlPlaneError::Conflict { message: "different spec at same key".into() };

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.error, "conflict");
    assert_eq!(body.message, "different spec at same key");
    assert!(body.field.is_none());
}

#[test]
fn intent_store_not_found_renders_as_404_with_not_found_kind() {
    let err = ControlPlaneError::Intent(IntentStoreError::NotFound);

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body.error, "not_found");
    assert!(body.field.is_none());
}

#[test]
fn intent_store_io_error_renders_as_500_with_internal_kind() {
    let err = ControlPlaneError::Intent(IntentStoreError::Io(io::Error::other("disk full")));

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert!(body.field.is_none());
}

#[test]
fn intent_store_busy_renders_as_500_with_internal_kind() {
    let err = ControlPlaneError::Intent(IntentStoreError::Busy);

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
}

#[test]
fn intent_store_conflict_renders_as_500_with_internal_kind() {
    // `IntentStoreError::Conflict` is a store-level transaction
    // conflict, distinct from the HTTP 409 ControlPlaneError::Conflict
    // that handlers raise on spec mismatch. The store-level variant
    // should not leak as HTTP 409 — it's an internal retry signal.
    let err = ControlPlaneError::Intent(IntentStoreError::Conflict);

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
}

#[test]
fn observation_store_error_renders_as_500_with_internal_kind() {
    let err = ControlPlaneError::Observation(ObservationStoreError::Unreachable {
        peer: "obs-2:8787".into(),
    });

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert!(body.field.is_none());
}

#[test]
fn aggregate_validation_error_renders_as_400_with_validation_kind_and_field() {
    let err = ControlPlaneError::Aggregate(AggregateError::Validation {
        field: "replicas",
        message: "must be > 0".into(),
    });

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, "validation");
    assert_eq!(
        body.field.as_deref(),
        Some("replicas"),
        "AggregateError::Validation.field must thread into ErrorBody.field",
    );
    assert!(body.message.contains("must be > 0"));
}

#[test]
fn aggregate_resources_error_renders_as_400_with_validation_kind() {
    let err =
        ControlPlaneError::Aggregate(AggregateError::Resources("cpu exceeds node capacity".into()));

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, "validation");
}

#[test]
fn tls_bootstrap_error_renders_as_500_with_internal_kind_and_preserves_chain() {
    // ADR-0015 §4: TLS bootstrap is infra failure → 500 internal.
    // Pass-through embedding (`Tls(#[from] TlsBootstrapError)`) MUST
    // preserve the structured chain in the rendered message — the
    // `MalformedMaterial.reason` text appears in `body.message` because
    // `to_response` calls `e.to_string()` on the embedded variant
    // rather than collapsing to a generic "tls failed" string.
    let err = ControlPlaneError::Tls(TlsBootstrapError::MalformedMaterial {
        reason: "server leaf PEM contained no certificates",
    });

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert!(
        body.message.contains("server leaf PEM contained no certificates"),
        "Tls(_) mapping must preserve the structured chain in the message; got {:?}",
        body.message,
    );
    assert!(body.field.is_none());
}

#[test]
fn cgroup_bootstrap_error_renders_as_500_with_internal_kind_and_preserves_chain() {
    let err = ControlPlaneError::CgroupBootstrap(
        overdrive_control_plane::error::CgroupBootstrapError::SubtreeControlWriteFailed {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "EACCES on subtree_control"),
        },
    );

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert!(
        body.message.contains("EACCES on subtree_control"),
        "CgroupBootstrap(_) mapping must preserve the structured chain in the message; got {:?}",
        body.message,
    );
    assert!(body.field.is_none());
}

#[test]
fn workloads_bootstrap_error_renders_as_500_with_internal_kind_and_preserves_chain() {
    let err = ControlPlaneError::WorkloadsBootstrap(
        overdrive_worker::cgroup_manager::WorkloadsBootstrapError::SubtreeControlBusy {
            source: io::Error::other("EBUSY on workloads.slice"),
        },
    );

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert!(
        body.message.contains("EBUSY on workloads.slice"),
        "WorkloadsBootstrap(_) mapping must preserve the structured chain in the message; got {:?}",
        body.message,
    );
    assert!(body.field.is_none());
}

#[test]
fn internal_error_renders_as_500_with_internal_kind() {
    let err = ControlPlaneError::Internal("store dropped mid-write".into());

    let (status, body) = to_response(err);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error, "internal");
    assert_eq!(body.message, "store dropped mid-write");
    assert!(body.field.is_none());
}

// ---------------------------------------------------------------------------
// CA-boot failures stay cause-distinguishable at the operator boundary
// (built-in-ca-operator-composition step 02-01 / D-OC-5)
// ---------------------------------------------------------------------------

/// Helper: build a `KekId` from a known-valid raw string.
fn kek(raw: &str) -> KekId {
    KekId::new(raw).expect("valid kek id fixture")
}

/// A CA-boot failure converts into `ControlPlaneError` through `?`/`From`
/// with NO string flattening — it stays matchable as the distinct
/// `ControlPlaneError::CaBoot(_)` variant at the composition root, each
/// cause renders a cause string that preserves its distinct underlying
/// cause (wrong-KEK and tampered render different strings), and the
/// variant maps to HTTP 500 for exhaustiveness (the boot path never
/// reaches HTTP).
///
/// Example-based over the finite, enumerated set of three boot causes —
/// a parametrized table is the right shape (the cause space is closed,
/// not a quantified domain). Asserts on matchability + rendered
/// cause-string distinctness through the public error boundary
/// (`From`/`?` + `to_response`), never private fields.
#[test]
fn ca_boot_error_causes_map_to_distinct_control_plane_ca_boot_variant() {
    // The three boot causes the operator must be able to tell apart.
    // absent-KEK     → KekUnavailable
    // wrong-KEK      → EnvelopeDecrypt(CaError::WrongKek)
    // tampered       → EnvelopeDecrypt(CaError::TamperedEnvelope)
    type CauseBuilder = fn() -> CaBootError;
    let causes: [(&str, CauseBuilder); 3] = [
        ("absent-kek", || CaBootError::KekUnavailable {
            kek_id: kek("overdrive-root-kek"),
            source: KekError::not_found(kek("overdrive-root-kek")),
        }),
        ("wrong-kek", || CaBootError::EnvelopeDecrypt {
            source: CaError::wrong_kek(kek("sealed-under-kek"), kek("supplied-kek")),
        }),
        ("tampered-envelope", || CaBootError::EnvelopeDecrypt {
            source: CaError::tampered_envelope(kek("overdrive-root-kek")),
        }),
    ];

    let mut rendered = std::collections::BTreeMap::new();

    for (label, build) in causes {
        // (a) converts via `?`/`From` and stays matchable as the distinct
        //     CaBoot variant — never flattened into `Internal(String)`.
        let cp: ControlPlaneError = build().into();
        assert!(
            matches!(cp, ControlPlaneError::CaBoot(_)),
            "{label}: CaBootError must convert to ControlPlaneError::CaBoot(_), \
             not be flattened; got {cp:?}",
        );

        // (c) maps to HTTP 500 for exhaustiveness (boot never reaches HTTP).
        let (status, body) = to_response(build().into());
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{label}: CaBoot maps to 500 for exhaustiveness",
        );
        assert_eq!(body.error, "internal", "{label}: CaBoot renders the internal kind");
        assert!(body.field.is_none(), "{label}: CaBoot carries no field");

        // The rendered message preserves the distinct underlying cause —
        // capture it for the cross-cause distinctness assertion below.
        rendered.insert(label, body.message);
    }

    // (b) wrong-KEK and tampered render DIFFERENT cause strings — the two
    //     are distinct underlying `CaError` variants and must NOT collapse
    //     to one operator-facing string.
    let wrong = &rendered["wrong-kek"];
    let tampered = &rendered["tampered-envelope"];
    assert_ne!(
        wrong, tampered,
        "wrong-KEK and tampered-envelope must render distinct cause strings; \
         both were {wrong:?}",
    );
    assert!(
        wrong.contains("cannot be opened with kek_id"),
        "wrong-KEK cause string must preserve the KEK-confusion cause; got {wrong:?}",
    );
    assert!(
        tampered.contains("corrupt or tampered"),
        "tampered-envelope cause string must preserve the integrity-failure cause; got {tampered:?}",
    );
}

// ---------------------------------------------------------------------------
// No-raw-stack-trace invariant
// ---------------------------------------------------------------------------

#[test]
fn error_response_body_does_not_leak_stack_trace() {
    // Seed an Internal error whose source carries rich formatting.
    // `to_response` must render only the sanitized `message` — never a
    // backtrace, never a nested `Caused by:` chain that could expose
    // internal file paths.
    let err = ControlPlaneError::Internal("database connection refused".into());

    let (_, body) = to_response(err);

    let serialised = serde_json::to_string(&body).expect("ErrorBody serialises");
    // Common backtrace markers. If any of these appear, something is
    // threading `{:?}` or a panic hook through the response path.
    let forbidden_markers = [
        "stack backtrace",
        "\n   0:",
        "\n   1:",
        "note: run with `RUST_BACKTRACE",
        "at ./",
        "at /Users/",
        "at /home/",
    ];
    for marker in forbidden_markers {
        assert!(
            !serialised.contains(marker),
            "response body leaks backtrace marker {marker:?}: {serialised}",
        );
    }
}

// ---------------------------------------------------------------------------
// `IntoResponse` round-trips through `to_response`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn into_response_round_trips_through_to_response() {
    // Confirm `IntoResponse::into_response` is not silently diverging
    // from `to_response` — status and body shape must match bit-for-bit.
    let err = ControlPlaneError::NotFound { resource: "workloads/abc".into() };
    let (expected_status, expected_body) =
        to_response(ControlPlaneError::NotFound { resource: "workloads/abc".into() });

    let response = err.into_response();

    assert_eq!(response.status(), expected_status);

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body collects");
    let parsed: ErrorBody =
        serde_json::from_slice(&body_bytes).expect("response body parses as ErrorBody");

    assert_eq!(parsed.error, expected_body.error);
    assert_eq!(parsed.message, expected_body.message);
    assert_eq!(parsed.field, expected_body.field);
}

#[tokio::test]
async fn into_response_renders_validation_with_field() {
    let err = ControlPlaneError::Validation {
        field: Some("replicas".into()),
        message: "must be > 0".into(),
    };

    let response = err.into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body collects");
    let parsed: ErrorBody =
        serde_json::from_slice(&body_bytes).expect("response body parses as ErrorBody");

    assert_eq!(parsed.error, "validation");
    assert_eq!(parsed.field.as_deref(), Some("replicas"));
    assert_eq!(parsed.message, "must be > 0");
}
