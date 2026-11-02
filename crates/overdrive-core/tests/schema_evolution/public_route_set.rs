//! Schema-evolution golden-bytes test — `PublicRouteSetEnvelope` (public
//! ingress). Pins the V1 archived layout of the singleton Route Set record.
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds a new
//! `FIXTURE_V2` constant and a new assertion in the same commit
//! (`development.md` § "Version-bump procedure"). The payload is a frozen
//! hand-pinned `PublicRouteSetEnvelope::V1`, never built through an alias.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_core::dataplane::Proto;
use overdrive_core::public_ingress::{PathMatch, PublicRouteSetEnvelope};

use super::harness::decode_envelope_v_latest;

/// Hex-encoded rkyv-archived bytes of `PublicRouteSetEnvelope::V1` holding one
/// occupied Route: ID `public-api`, generation `[9; 32]`, hostname
/// `api.example.com`, segment prefix `/api`, Service `api` on TCP 8080, key ID
/// `api-origin`.
const FIXTURE_V1: &str = "7075626c69632d6170696170692e6578616d706c652e636f6d6170692d6f726967696e0000000000010000008a000000d4ffffff09090909090909090909090909090909090909090909090909090909090909098f000000b6ffffff010000002f617069ffffffff617069ffffffffff901f00008a000000a5ffffff";

/// CONTRACT_SHAPE: pure-function.
#[test]
fn public_route_set_v1_decodes_through_current_envelope() {
    let latest = decode_envelope_v_latest::<PublicRouteSetEnvelope>(FIXTURE_V1);
    let route = latest.route.expect("the fixture is the occupied Route Set");

    assert_eq!(route.route_id().as_str(), "public-api");
    assert_eq!(route.route_generation().as_bytes(), &[9u8; 32]);
    assert_eq!(route.public_hostname().as_str(), "api.example.com");
    match route.path_match() {
        PathMatch::SegmentPrefix(path) => assert_eq!(path.as_str(), "/api"),
        PathMatch::Exact(path) => panic!("expected a segment prefix, got exact {}", path.as_str()),
    }
    let listener = route.service_listener();
    assert_eq!(listener.service().as_str(), "api");
    assert_eq!(listener.port().get(), 8080);
    assert_eq!(listener.protocol(), Proto::Tcp);
    assert_eq!(route.public_certified_key_id().as_str(), "api-origin");
}
