//! RED acceptance scaffolds for the two existing Service deploy lanes.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-23 — detached/JSON `deploy_service` forwards the selected VM driver
/// and all existing Service fields into exactly one `ServiceSpecInput`, with
/// unchanged HTTP request and acknowledgement behavior.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn detached_service_deploy_forwards_vm_driver_without_parallel_request_shape() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-23 / detached Service lane)");
}

/// S-SVM-24 — TTY/NDJSON `deploy_streaming_service` forwards the byte-equal
/// VM driver arm and preserves the existing Accepted/Stable/Failed rendering
/// and exit-code contract; the two lanes differ only in their existing wire
/// mode.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn streaming_service_deploy_has_driver_projection_parity_with_detached_lane() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-24 / streaming Service lane parity)");
}
