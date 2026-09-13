//! V1 golden for the redacted public Certified Key status envelope.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_core::public_ingress::{
    PublicCertifiedKeyStatusRowV1, PublicCertifiedKeyStatusState,
};

const FIXTURE_V1: &str = "6170692d6f726967696e676174657761792d6e6f6465000000000000000000008a000000e0ffffff0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000008c00000052ffffff";

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER typed public Certified Key status codec"]
fn public_certified_key_status_v1_decodes_through_the_production_codec() {
    let bytes = hex::decode(FIXTURE_V1).expect("pinned V1 hex");
    let decoded = PublicCertifiedKeyStatusRowV1::from_store_bytes(&bytes, Some("api-origin"))
        .expect("V1 key status remains readable");
    assert_eq!(decoded.certified_key_id.as_str(), "api-origin");
    assert_eq!(decoded.state, PublicCertifiedKeyStatusState::Absent);
    assert!(decoded.last_install_failure.is_none());
    assert_eq!(decoded.updated_at.counter, 1);
}
