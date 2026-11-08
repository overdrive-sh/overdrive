//! S-PIG-140 / S-PIG-141 (R-67) unit assertions on `error::to_response`.
//!
//! NOT IN THE BUILD. These bodies name `ControlPlaneError::{RouteDecode, RouteEncode,
//! GatewayOwnerUnavailable}`, which do not exist until roadmap step 05-02 (key
//! `route-set-error-mapping`). The crafter, in the commit that adds those variants:
//!   1. moves this file to `crates/overdrive-control-plane/tests/acceptance/route_set_error_mapping.rs`;
//!   2. adds `mod route_set_error_mapping;` next to `mod error_mapping_exhaustive;` in
//!      `crates/overdrive-control-plane/tests/acceptance.rs`.
//! The bodies are not to be edited. `RouteEncode` has no scenario (reachable through no
//! production port, R-67); this unit assertion is its whole obligation.

use overdrive_control_plane::api::ErrorBody;
use overdrive_control_plane::error::{ControlPlaneError, to_response};
use overdrive_core::codec::EnvelopeError;
use overdrive_core::public_ingress::{PublicRouteSetV1, RouteCodecError};
use overdrive_core::traits::intent_store::IntentStoreError;

/// A real envelope decode failure, produced by the codec's own public decode.
fn an_envelope_error() -> EnvelopeError {
    match PublicRouteSetV1::from_store_bytes(&[], std::path::Path::new("<fixture>"), None) {
        Err(IntentStoreError::Envelope { source, .. }) => source,
        other => panic!("empty bytes are an envelope failure, got {other:?}"),
    }
}

fn assert_body(body: &ErrorBody, error: &str, message: &str) {
    assert_eq!(body.error, error);
    assert_eq!(body.message, message, "fixed message, no cause text");
    assert_eq!(body.field, None);
}

/// S-PIG-140: a stored Route that fails to decode is a 500 `route_decode` whose
/// body carries no cause text.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_route_decode_failure_maps_to_a_500_route_decode_without_cause_text() {
    let (status, body) =
        to_response(ControlPlaneError::RouteDecode(RouteCodecError::Envelope(an_envelope_error())));
    assert_eq!(status.as_u16(), 500);
    assert_body(&body, "route_decode", "stored public Route is undecodable");
}

/// S-PIG-140 (encode arm, no-obligation ruling): a Route-set write that fails to
/// serialise is a 500 `route_encode`, distinct from `route_decode`.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_route_encode_failure_maps_to_a_500_route_encode_distinct_from_route_decode() {
    let (status, body) = to_response(ControlPlaneError::RouteEncode(an_envelope_error()));
    assert_eq!(status.as_u16(), 500);
    assert_body(&body, "route_encode", "public Route set could not be serialised");
    assert_ne!(body.error, "route_decode");
}

/// S-PIG-141: a Route owner that is gone is a 503 `gateway_owner_unavailable`.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_gone_route_owner_maps_to_a_503_gateway_owner_unavailable() {
    let (status, body) = to_response(ControlPlaneError::GatewayOwnerUnavailable);
    assert_eq!(status.as_u16(), 503);
    assert_body(&body, "gateway_owner_unavailable", "public Route owner is unavailable");
}
