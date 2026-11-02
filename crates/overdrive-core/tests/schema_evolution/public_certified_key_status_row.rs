//! Schema-evolution golden-bytes test — `PublicCertifiedKeyStatusRowEnvelope`
//! (public ingress). Pins the V1 archived layout of the redacted custody
//! status row.
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds a new
//! `FIXTURE_V2` constant and a new assertion in the same commit
//! (`development.md` § "Version-bump procedure"). The payload is a frozen
//! hand-pinned `PublicCertifiedKeyStatusRowEnvelope::V1`, never built through
//! an alias.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::public_ingress::{
    CertifiedKeyFailure, CertifiedKeyProvenance, PublicCertifiedKeyStatusRowEnvelope,
    PublicCertifiedKeyStatusState,
};

use super::harness::decode_envelope_v_latest;

/// Hex-encoded rkyv-archived bytes of
/// `PublicCertifiedKeyStatusRowEnvelope::V1`: key ID `api-origin`, a `Usable`
/// state (generation `[1; 32]`, fingerprint `[2; 32]`, hostname
/// `api.example.com`, validity 1 000 s to 2 000 s, manual provenance), the
/// latest install failure `PemShape`, and the logical timestamp
/// `(1, gateway-node)`.
const FIXTURE_V1: &str = "6170692d6f726967696e6170692e6578616d706c652e636f6d676174657761792d6e6f646500000000000000000000008a000000d0ffffff01010101010101010101010101010101010101010101010101010101010101010102020202020202020202020202020202020202020202020202020202020202020000008f0000008effffff00000000e8030000000000000000000000000000d007000000000000000000000000000000000000000000000000000000000000010500000000000001000000000000008c00000051ffffff";

/// CONTRACT_SHAPE: pure-function.
#[test]
fn public_certified_key_status_v1_decodes_through_current_envelope() {
    let latest = decode_envelope_v_latest::<PublicCertifiedKeyStatusRowEnvelope>(FIXTURE_V1);

    assert_eq!(latest.certified_key_id.as_str(), "api-origin");
    match &latest.state {
        PublicCertifiedKeyStatusState::Usable {
            generation,
            certificate_fingerprint,
            public_hostname,
            not_before,
            not_after,
            provenance,
        } => {
            assert_eq!(generation.as_bytes(), &[1u8; 32]);
            assert_eq!(certificate_fingerprint.as_bytes(), &[2u8; 32]);
            assert_eq!(public_hostname.as_str(), "api.example.com");
            assert_eq!(*not_before, UnixInstant::from_unix_duration(Duration::from_secs(1_000)));
            assert_eq!(*not_after, UnixInstant::from_unix_duration(Duration::from_secs(2_000)));
            assert_eq!(provenance, &CertifiedKeyProvenance::Manual);
        }
        other => panic!("expected the Usable state, got {other:?}"),
    }
    assert_eq!(latest.last_install_failure, Some(CertifiedKeyFailure::PemShape));
    assert_eq!(latest.updated_at.counter, 1);
    assert_eq!(latest.updated_at.writer.as_str(), "gateway-node");
}
