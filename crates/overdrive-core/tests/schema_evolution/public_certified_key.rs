//! Schema-evolution golden-bytes test — `PublicCertifiedKeyEnvelope` (public
//! ingress). Pins the V1 archived layout of the protected certified-key record.
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds a new
//! `FIXTURE_V2` constant and a new assertion in the same commit
//! (`development.md` § "Version-bump procedure"). The payload is a frozen
//! hand-pinned `PublicCertifiedKeyEnvelope::V1`, never built through an alias.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::public_ingress::{CertifiedKeyProvenance, PublicCertifiedKeyEnvelope};

use super::harness::decode_envelope_v_latest;

/// Hex-encoded rkyv-archived bytes of `PublicCertifiedKeyEnvelope::V1`: key ID
/// `api-origin`, hostname `api.example.com`, generation `[1; 32]`, fingerprint
/// `[2; 32]`, a two-certificate chain `[[3, 4, 5], [6, 7]]`, validity 1 000 s to
/// 2 000 s, manual provenance and a protected key record (KEK ID
/// `overdrive-public-certified-key`, salt `[8; 32]`, nonce `[9; 12]`,
/// ciphertext `[10, 11, 12]`).
const FIXTURE_V1: &str = "6170692d6f726967696e6170692e6578616d706c652e636f6d03040506070000f9ffffff03000000f4ffffff020000006f76657264726976652d7075626c69632d6365727469666965642d6b65790a0b0c0000000000000000000000000000008a000000a0ffffff8f000000a2ffffff0101010101010101010101010101010101010101010101010101010101010101020202020202020202020202020202020202020202020202020202020202020270ffffff02000000e8030000000000000000000000000000d00700000000000000000000000000000000000000000000000000009e0000004cffffff080808080808080808080808080808080808080808080808080808080808080809090909090909090909090936ffffff03000000";

/// CONTRACT_SHAPE: pure-function.
#[test]
fn public_certified_key_v1_decodes_through_current_envelope() {
    let latest = decode_envelope_v_latest::<PublicCertifiedKeyEnvelope>(FIXTURE_V1);

    assert_eq!(latest.certified_key_id.as_str(), "api-origin");
    assert_eq!(latest.public_hostname.as_str(), "api.example.com");
    assert_eq!(latest.certified_key_generation.as_bytes(), &[1u8; 32]);
    assert_eq!(latest.certificate_fingerprint.as_bytes(), &[2u8; 32]);
    assert_eq!(latest.certificate_chain_der, vec![vec![3, 4, 5], vec![6, 7]]);
    assert_eq!(latest.not_before, UnixInstant::from_unix_duration(Duration::from_secs(1_000)));
    assert_eq!(latest.not_after, UnixInstant::from_unix_duration(Duration::from_secs(2_000)));
    assert_eq!(latest.provenance, CertifiedKeyProvenance::Manual);
    assert_eq!(latest.protected_origin_key.kek_id, "overdrive-public-certified-key");
    assert_eq!(latest.protected_origin_key.salt, [8u8; 32]);
    assert_eq!(latest.protected_origin_key.nonce, [9u8; 12]);
    assert_eq!(latest.protected_origin_key.ciphertext_and_tag, vec![10, 11, 12]);
}
