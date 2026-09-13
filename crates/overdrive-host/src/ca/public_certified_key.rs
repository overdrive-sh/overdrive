//! Public-certified-key AEAD codec.
//!
//! SCAFFOLD: true. Seal/open remain RED until DELIVER.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffold")]

use std::sync::Arc;

use overdrive_core::ca::kek::Kek;
use overdrive_core::public_ingress::{
    ProtectedOriginKeyV1, PublicCertifiedKeyAeadError, PublicCertifiedKeyProtectionContext,
};
use zeroize::Zeroizing;

/// Host adapter protecting public-origin private keys under their distinct KEK domain.
pub struct PublicCertifiedKeyAeadCodec {
    _kek: Arc<dyn Kek>,
}

impl PublicCertifiedKeyAeadCodec {
    /// Bind the mandatory KEK provider.
    pub fn new(_kek: Arc<dyn Kek>) -> Self {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::new")
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[allow(clippy::type_complexity, reason = "exact DESIGN-pinned random-fill test constructor")]
    pub fn with_random_fill_for_test(
        _kek: Arc<dyn Kek>,
        _fill: Arc<dyn Fn(&mut [u8]) -> Result<(), PublicCertifiedKeyAeadError> + Send + Sync>,
    ) -> Self {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::with_random_fill_for_test")
    }

    pub fn seal(
        &self,
        _context: &PublicCertifiedKeyProtectionContext,
        _key_pkcs8: &Zeroizing<Vec<u8>>,
    ) -> Result<ProtectedOriginKeyV1, PublicCertifiedKeyAeadError> {
        todo!("SCAFFOLD: PublicCertifiedKeyAeadCodec::seal")
    }

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
        clippy::doc_markdown,
        clippy::expect_used,
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

    fn context(hostname: &str) -> PublicCertifiedKeyProtectionContext {
        PublicCertifiedKeyProtectionContext {
            certified_key_id: PublicCertifiedKeyId::new("api-origin").expect("ID"),
            public_hostname: PublicHostname::new(hostname).expect("hostname"),
            generation: CertifiedKeyGeneration::new(&"01".repeat(32)).expect("generation"),
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER public Certified Key AEAD codec"]
    fn tamper_and_aad_mismatch_fail_authentication_without_plaintext() {
        let codec = PublicCertifiedKeyAeadCodec::with_random_fill_for_test(
            Arc::new(FixedKek),
            Arc::new(|bytes| {
                bytes.fill(0x33);
                Ok(())
            }),
        );
        let key = Zeroizing::new(vec![0x30, 0x01, 0x00]);
        let protected = codec.seal(&context("api.example.com"), &key).expect("seal");
        assert_eq!(codec.open(&context("api.example.com"), &protected).expect("open"), key);

        let mut tampered = protected.clone();
        tampered.ciphertext_and_tag[0] ^= 1;
        assert_eq!(
            codec.open(&context("api.example.com"), &tampered),
            Err(PublicCertifiedKeyAeadError::AuthenticationFailed),
        );
        assert_eq!(
            codec.open(&context("other.example.com"), &protected),
            Err(PublicCertifiedKeyAeadError::AuthenticationFailed),
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER public Certified Key CSPRNG failure"]
    fn csprng_draw_failure_has_no_protected_record() {
        let codec = PublicCertifiedKeyAeadCodec::with_random_fill_for_test(
            Arc::new(FixedKek),
            Arc::new(|_| Err(PublicCertifiedKeyAeadError::SealFailed)),
        );
        assert_eq!(
            codec.seal(&context("api.example.com"), &Zeroizing::new(vec![1, 2, 3])),
            Err(PublicCertifiedKeyAeadError::SealFailed),
        );
    }
}
