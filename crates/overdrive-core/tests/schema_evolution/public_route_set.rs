//! V1 golden for the singleton public Route Set envelope.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_core::public_ingress::PublicRouteSetV1;
use std::path::Path;

const FIXTURE_V1: &str = "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER typed Public Route Set store codec"]
fn public_route_set_v1_decodes_through_the_production_codec() {
    let bytes = hex::decode(FIXTURE_V1).expect("pinned V1 hex");
    let decoded = PublicRouteSetV1::from_store_bytes(
        &bytes,
        Path::new("/redacted/intent.redb"),
        Some("public-ingress/route-set"),
    )
    .expect("V1 Route Set remains readable");
    assert!(decoded.route.is_none(), "canonical V1 fixture is the persisted Empty state");
}
