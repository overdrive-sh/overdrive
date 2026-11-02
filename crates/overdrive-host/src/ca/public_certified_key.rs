//! `PublicCertifiedKeyAeadCodec` — HKDF-SHA256 → AES-256-GCM seal/open for the
//! public-ingress origin private key.
//!
//! SCAFFOLD: true. Seal and open are DELIVER's and fail with SCAFFOLD text. The
//! source-local acceptance tests below specify S-PIG-47.
//!
//! The byte-exact contract (DESIGN, "Persistence, custody and redaction"): the
//! record's `kek_id` is `"overdrive-public-certified-key"`; HKDF-SHA256 uses the
//! record's 32 random salt and info
//! `b"overdrive/public-certified-key/aes-256-gcm/v1"`; GCM uses a fresh 12-byte
//! nonce; the AAD is `b"overdrive/public-certified-key/record/v1\0"` followed
//! by the certified-key ID and hostname as `u32::to_be_bytes(len) || bytes` and
//! the 32 generation bytes; the plaintext is exactly the canonical PKCS#8 DER.

#![allow(
    clippy::todo,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::doc_markdown,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::sync::Arc;

use overdrive_core::ca::kek::Kek;
use overdrive_core::public_ingress::{
    ProtectedOriginKeyV1, PublicCertifiedKeyAeadError, PublicCertifiedKeyProtectionContext,
};
use zeroize::Zeroizing;

/// Host adapter protecting public-origin private keys under their distinct KEK
/// domain.
pub struct PublicCertifiedKeyAeadCodec {
    _kek: Arc<dyn Kek>,
}

impl PublicCertifiedKeyAeadCodec {
    /// Bind the mandatory KEK provider; salt and nonce come from the
    /// crypto-backend CSPRNG.
    pub fn new(_kek: Arc<dyn Kek>) -> Self {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::new")
    }

    /// Bind the KEK provider and a deterministic random-fill closure so salt
    /// and nonce draws, and draw failure, are testable without a
    /// default-feature entropy seam.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[allow(clippy::type_complexity, reason = "exact DESIGN-pinned random-fill test constructor")]
    pub fn with_random_fill_for_test(
        _kek: Arc<dyn Kek>,
        _fill: Arc<dyn Fn(&mut [u8]) -> Result<(), PublicCertifiedKeyAeadError> + Send + Sync>,
    ) -> Self {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::with_random_fill_for_test")
    }

    /// Seal the canonical PKCS#8 DER under the context-bound AAD.
    pub fn seal(
        &self,
        _context: &PublicCertifiedKeyProtectionContext,
        _key_pkcs8: &Zeroizing<Vec<u8>>,
    ) -> Result<ProtectedOriginKeyV1, PublicCertifiedKeyAeadError> {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::seal")
    }

    /// Authenticate and open a protected record into a zeroizing buffer.
    pub fn open(
        &self,
        _context: &PublicCertifiedKeyProtectionContext,
        _protected: &ProtectedOriginKeyV1,
    ) -> Result<Zeroizing<Vec<u8>>, PublicCertifiedKeyAeadError> {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::open")
    }
}

#[cfg(test)]
mod acceptance {
    #![allow(
        clippy::expect_used,
        clippy::doc_markdown,
        reason = "acceptance fixture preconditions and Contract Shape metadata"
    )]

    use overdrive_core::ca::kek::{KekError, KekMaterial};
    use overdrive_core::ca::root_key_envelope::KekId;
    use overdrive_core::public_ingress::{
        CertifiedKeyGeneration, PublicCertifiedKeyId, PublicHostname,
    };

    use super::*;

    struct FixedKek;

    impl Kek for FixedKek {
        fn resolve(&self, _kek_id: &KekId) -> Result<KekMaterial, KekError> {
            Ok(KekMaterial::new([0x5a; 32]))
        }
    }

    fn context(
        id: &str,
        hostname: &str,
        generation_byte: &str,
    ) -> PublicCertifiedKeyProtectionContext {
        PublicCertifiedKeyProtectionContext {
            certified_key_id: PublicCertifiedKeyId::new(id).expect("certified-key ID"),
            public_hostname: PublicHostname::new(hostname).expect("hostname"),
            generation: CertifiedKeyGeneration::new(&generation_byte.repeat(32))
                .expect("generation"),
        }
    }

    fn deterministic_codec() -> PublicCertifiedKeyAeadCodec {
        PublicCertifiedKeyAeadCodec::with_random_fill_for_test(
            Arc::new(FixedKek),
            Arc::new(|bytes| {
                bytes.fill(0x33);
                Ok(())
            }),
        )
    }

    /// S-PIG-47: tampering with the ciphertext, tag or nonce, or opening under
    /// a different AAD context (another hostname, key ID or generation), fails
    /// authentication and returns no plaintext; the sealed record never
    /// contains the plaintext key.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER certified-key-custody"]
    fn tamper_and_aad_mismatch_fail_authentication_without_plaintext() {
        let codec = deterministic_codec();
        let ctx = context("api-origin", "api.example.com", "01");
        let plaintext = Zeroizing::new((0u8..48).map(|b| 0x30 + b).collect::<Vec<u8>>());
        let protected = codec.seal(&ctx, &plaintext).expect("seal");

        assert_eq!(protected.kek_id, "overdrive-public-certified-key");
        assert!(
            !protected
                .ciphertext_and_tag
                .windows(plaintext.len())
                .any(|window| window == plaintext.as_slice()),
            "the sealed record must not contain the plaintext key",
        );
        assert_eq!(codec.open(&ctx, &protected).expect("open"), plaintext);

        let mut tampered = protected.clone();
        tampered.ciphertext_and_tag[0] ^= 1;
        assert_eq!(
            codec.open(&ctx, &tampered),
            Err(PublicCertifiedKeyAeadError::AuthenticationFailed)
        );

        let mut tag_tampered = protected.clone();
        let last = tag_tampered.ciphertext_and_tag.len() - 1;
        tag_tampered.ciphertext_and_tag[last] ^= 1;
        assert_eq!(
            codec.open(&ctx, &tag_tampered),
            Err(PublicCertifiedKeyAeadError::AuthenticationFailed),
        );

        let mut nonce_tampered = protected.clone();
        nonce_tampered.nonce[0] ^= 1;
        assert_eq!(
            codec.open(&ctx, &nonce_tampered),
            Err(PublicCertifiedKeyAeadError::AuthenticationFailed),
        );

        for other in [
            context("api-origin", "other.example.com", "01"),
            context("other-origin", "api.example.com", "01"),
            context("api-origin", "api.example.com", "02"),
        ] {
            assert_eq!(
                codec.open(&other, &protected),
                Err(PublicCertifiedKeyAeadError::AuthenticationFailed),
            );
        }
    }

    /// S-PIG-47: a failed CSPRNG draw leaves no protected record and no
    /// plaintext, whichever of the salt and nonce draws fails.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER certified-key-custody"]
    fn csprng_draw_failure_has_no_protected_record() {
        let codec = PublicCertifiedKeyAeadCodec::with_random_fill_for_test(
            Arc::new(FixedKek),
            Arc::new(|_| Err(PublicCertifiedKeyAeadError::SealFailed)),
        );
        let result = codec
            .seal(&context("api-origin", "api.example.com", "01"), &Zeroizing::new(vec![1, 2, 3]));
        assert_eq!(
            result.map(|record| record.kek_id),
            Err(PublicCertifiedKeyAeadError::SealFailed)
        );
    }
}
