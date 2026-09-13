//! V1 golden for the protected public Certified Key envelope.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_core::UnixInstant;
use overdrive_core::public_ingress::{CertifiedKeyProvenance, PublicCertifiedKeyV1};
use std::path::Path;
use std::time::Duration;

const FIXTURE_V1: &str = "6170692d6f726967696e6170692e6578616d706c652e636f6d030405fdffffff030000006f76657264726976652d7075626c69632d6365727469666965642d6b657908090a00000000000000000000008a000000b0ffffff8f000000b2ffffff010101010101010101010101010101010101010101010101010101010101010102020202020202020202020202020202020202020202020202020202020202027cffffff01000000e8030000000000000000000000000000d00700000000000000000000000000000000000000000000000000009e00000050ffffff06060606060606060606060606060606060606060606060606060606060606060707070707070707070707073affffff03000000";

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER typed protected public Certified Key store codec"]
fn public_certified_key_v1_decodes_through_the_production_codec() {
    let bytes = hex::decode(FIXTURE_V1).expect("pinned V1 hex");
    let decoded = PublicCertifiedKeyV1::from_store_bytes(
        &bytes,
        Path::new("/redacted/intent.redb"),
        Some("public-ingress/certified-key/api-origin"),
    )
    .expect("V1 protected key remains readable");
    assert_eq!(decoded.certified_key_id.as_str(), "api-origin");
    assert_eq!(decoded.public_hostname.as_str(), "api.example.com");
    assert_eq!(decoded.certificate_chain_der, vec![vec![3, 4, 5]]);
    assert_eq!(decoded.not_before, UnixInstant::from_unix_duration(Duration::from_secs(1_000)));
    assert_eq!(decoded.not_after, UnixInstant::from_unix_duration(Duration::from_secs(2_000)));
    assert_eq!(decoded.provenance, CertifiedKeyProvenance::Manual);
}
