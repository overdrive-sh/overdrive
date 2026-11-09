//! R-68 unit assertion on `error::to_response` for the gateway status point
//! read: both `GatewayStatusReadError` variants map to the same fixed, redacted
//! 500 body carrying none of the underlying cause text.
//!
//! Activation (DELIVER gateway-status-read-redaction): once
//! `ControlPlaneError::GatewayStatusRead` exists, register this file in
//! `tests/acceptance.rs` (`mod gateway_status_read_error_mapping;`, beside
//! `route_set_error_mapping`) and remove the `#[ignore]` markers. The file is
//! unregistered until then because it names the variant and would otherwise
//! stop the acceptance binary compiling.

use overdrive_control_plane::api::ErrorBody;
use overdrive_control_plane::error::{ControlPlaneError, to_response};
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_gateway::application::GatewayStatusReadError;

const CAUSE_MARKER: &str = "status-read-cause-marker";

fn a_read_failure() -> ObservationStoreError {
    ObservationStoreError::Unreachable { peer: CAUSE_MARKER.to_owned() }
}

fn assert_redacted_status_read_body(status: u16, body: &ErrorBody) {
    assert_eq!(status, 500);
    assert_eq!(body.error, "gateway_status_read");
    assert_eq!(body.message, "gateway status could not be read", "fixed message, no cause text");
    assert_eq!(body.field, None);
    let rendered = serde_json::to_string(body).expect("the error body serialises");
    assert!(!rendered.contains(CAUSE_MARKER), "the body leaks the cause: {rendered}");
}

/// R-68: a failed Gateway Application status read is a redacted 500
/// `gateway_status_read`.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_failed_application_status_read_maps_to_a_redacted_500_gateway_status_read() {
    let (status, body) = to_response(ControlPlaneError::GatewayStatusRead(
        GatewayStatusReadError::Application(a_read_failure()),
    ));
    assert_redacted_status_read_body(status.as_u16(), &body);
}

/// R-68: a failed certified-key status read maps to the identical redacted 500.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_failed_certified_key_status_read_maps_to_the_same_redacted_500() {
    let (status, body) = to_response(ControlPlaneError::GatewayStatusRead(
        GatewayStatusReadError::CertifiedKey(a_read_failure()),
    ));
    assert_redacted_status_read_body(status.as_u16(), &body);
}

/// R-68: the two variants are indistinguishable on the wire.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn both_status_read_variants_produce_an_identical_body() {
    let (_, application) = to_response(ControlPlaneError::GatewayStatusRead(
        GatewayStatusReadError::Application(a_read_failure()),
    ));
    let (_, certified_key) = to_response(ControlPlaneError::GatewayStatusRead(
        GatewayStatusReadError::CertifiedKey(a_read_failure()),
    ));
    assert_eq!(
        serde_json::to_string(&application).expect("the error body serialises"),
        serde_json::to_string(&certified_key).expect("the error body serialises"),
    );
}
