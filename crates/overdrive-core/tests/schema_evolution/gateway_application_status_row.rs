//! V1 golden for the redacted Gateway Application status envelope.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, GatewayApplicationUnavailableCause, GatewayConnectPathStatus,
    GatewayIdentityStatus, GatewayListenerStatus,
};

const FIXTURE_V1: &str = "676174657761792d6e6f646500000000676174657761792d6e6f64650000000000000000000000008c000000d8ffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f8feffff0000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000008c000000a0feffff";

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER typed Gateway Application status codec"]
fn gateway_application_status_v1_decodes_through_the_production_codec() {
    let bytes = hex::decode(FIXTURE_V1).expect("pinned V1 hex");
    let decoded = GatewayApplicationStatusRowV1::from_store_bytes(&bytes, Some("gateway-node"))
        .expect("V1 application status remains readable");
    assert_eq!(decoded.node_id.as_str(), "gateway-node");
    assert_eq!(decoded.listener, GatewayListenerStatus::Unbound);
    assert!(decoded.staged.is_none() && decoded.current.is_none() && decoded.draining.is_empty());
    assert_eq!(decoded.unavailable, Some(GatewayApplicationUnavailableCause::RouteAbsent));
    assert_eq!(decoded.gateway_identity, GatewayIdentityStatus::Absent);
    assert_eq!(decoded.connect_path, GatewayConnectPathStatus::Unavailable);
    assert_eq!(decoded.updated_at.counter, 2);
}
