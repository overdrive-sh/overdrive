//! `PublicCertifiedKeyAeadCodec` — HKDF-SHA256 → AES-256-GCM seal/open for the
//! public-ingress origin private key.
//!
//! The byte-exact contract (DESIGN, "Persistence, custody and redaction"): the
//! record's `kek_id` is `"overdrive-public-certified-key"`; HKDF-SHA256 uses the
//! record's 32 random salt and info
//! `b"overdrive/public-certified-key/aes-256-gcm/v1"`; GCM uses a fresh 12-byte
//! nonce; the AAD is `b"overdrive/public-certified-key/record/v1\0"` followed
//! by the certified-key ID and hostname as `u32::to_be_bytes(len) || bytes` and
//! the 32 generation bytes; the plaintext is exactly the canonical PKCS#8 DER.

use std::sync::Arc;

use overdrive_core::ca::kek::Kek;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::public_ingress::{
    ProtectedOriginKeyV1, PublicCertifiedKeyAeadError, PublicCertifiedKeyProtectionContext,
};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

/// The KEK domain that protects public-origin private keys.
const KEK_ID: &str = "overdrive-public-certified-key";
/// HKDF salt length (bytes), drawn fresh per seal.
const SALT_LEN: usize = 32;
/// Derived AES-256-GCM key length (bytes).
const SUBKEY_LEN: usize = 32;
/// AES-GCM authentication tag length (bytes).
const TAG_LEN: usize = 16;
/// HKDF domain-separation label.
const HKDF_INFO: &[u8] = b"overdrive/public-certified-key/aes-256-gcm/v1";
/// AAD context prefix.
const AAD_CONTEXT: &[u8] = b"overdrive/public-certified-key/record/v1\0";

#[cfg(any(test, feature = "integration-tests"))]
type RandomFill = Arc<dyn Fn(&mut [u8]) -> Result<(), PublicCertifiedKeyAeadError> + Send + Sync>;

/// Fixed-length HKDF output type.
struct SubkeyLen;

impl KeyType for SubkeyLen {
    fn len(&self) -> usize {
        SUBKEY_LEN
    }
}

/// Source of salt and nonce bytes.
enum RandomSource {
    System(SystemRandom),
    #[cfg(any(test, feature = "integration-tests"))]
    Filled(RandomFill),
}

impl RandomSource {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), PublicCertifiedKeyAeadError> {
        match self {
            Self::System(rng) => {
                rng.fill(bytes).map_err(|_| PublicCertifiedKeyAeadError::SealFailed)
            }
            #[cfg(any(test, feature = "integration-tests"))]
            Self::Filled(fill) => fill(bytes),
        }
    }
}

/// Host adapter protecting public-origin private keys under their distinct KEK
/// domain.
pub struct PublicCertifiedKeyAeadCodec {
    kek: Arc<dyn Kek>,
    random: RandomSource,
}

impl PublicCertifiedKeyAeadCodec {
    /// Bind the mandatory KEK provider; salt and nonce come from the
    /// crypto-backend CSPRNG.
    pub fn new(kek: Arc<dyn Kek>) -> Self {
        Self { kek, random: RandomSource::System(SystemRandom::new()) }
    }

    /// Bind the KEK provider and a deterministic random-fill closure so salt
    /// and nonce draws, and draw failure, are testable without a
    /// default-feature entropy seam.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[allow(clippy::type_complexity, reason = "exact DESIGN-pinned random-fill test constructor")]
    pub fn with_random_fill_for_test(
        kek: Arc<dyn Kek>,
        fill: Arc<dyn Fn(&mut [u8]) -> Result<(), PublicCertifiedKeyAeadError> + Send + Sync>,
    ) -> Self {
        Self { kek, random: RandomSource::Filled(fill) }
    }

    /// Seal the canonical PKCS#8 DER under the context-bound AAD.
    ///
    /// # Errors
    /// `KekUnavailable` when the KEK cannot be resolved; `SealFailed` on a
    /// failed random draw or AES-GCM seal.
    pub fn seal(
        &self,
        context: &PublicCertifiedKeyProtectionContext,
        key_pkcs8: &Zeroizing<Vec<u8>>,
    ) -> Result<ProtectedOriginKeyV1, PublicCertifiedKeyAeadError> {
        let mut salt = [0u8; SALT_LEN];
        self.random.fill(&mut salt)?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.random.fill(&mut nonce_bytes)?;
        let key = self.derive_key(&salt)?;
        let mut in_out = key_pkcs8.to_vec();
        let tag = key
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(aad_bytes(context)),
                &mut in_out,
            )
            .map_err(|_| PublicCertifiedKeyAeadError::SealFailed)?;
        in_out.extend_from_slice(tag.as_ref());
        Ok(ProtectedOriginKeyV1 {
            kek_id: KEK_ID.to_owned(),
            salt,
            nonce: nonce_bytes,
            ciphertext_and_tag: in_out,
        })
    }

    /// Authenticate and open a protected record into a zeroizing buffer.
    ///
    /// # Errors
    /// `OpenFailed` for a different KEK ID or a record too short to carry a
    /// tag; `AuthenticationFailed` when the tag, nonce, salt or AAD context
    /// does not authenticate; `KekUnavailable` when the KEK cannot be resolved.
    pub fn open(
        &self,
        context: &PublicCertifiedKeyProtectionContext,
        protected: &ProtectedOriginKeyV1,
    ) -> Result<Zeroizing<Vec<u8>>, PublicCertifiedKeyAeadError> {
        if protected.kek_id != KEK_ID || protected.ciphertext_and_tag.len() < TAG_LEN {
            return Err(PublicCertifiedKeyAeadError::OpenFailed);
        }
        let key = self.derive_key(&protected.salt)?;
        let mut in_out = Zeroizing::new(protected.ciphertext_and_tag.clone());
        let plaintext_len = key
            .open_in_place(
                Nonce::assume_unique_for_key(protected.nonce),
                Aad::from(aad_bytes(context)),
                &mut in_out,
            )
            .map_err(|_| PublicCertifiedKeyAeadError::AuthenticationFailed)?
            .len();
        in_out.truncate(plaintext_len);
        Ok(in_out)
    }

    fn derive_key(&self, salt: &[u8]) -> Result<LessSafeKey, PublicCertifiedKeyAeadError> {
        let kek_id = KekId::new(KEK_ID).map_err(|_| PublicCertifiedKeyAeadError::KekUnavailable)?;
        let material =
            self.kek.resolve(&kek_id).map_err(|_| PublicCertifiedKeyAeadError::KekUnavailable)?;
        let prk = Salt::new(HKDF_SHA256, salt).extract(material.expose_secret());
        let info = [HKDF_INFO];
        let okm = prk
            .expand(&info, SubkeyLen)
            .map_err(|_| PublicCertifiedKeyAeadError::KekUnavailable)?;
        let mut subkey = Zeroizing::new([0u8; SUBKEY_LEN]);
        okm.fill(subkey.as_mut_slice()).map_err(|_| PublicCertifiedKeyAeadError::KekUnavailable)?;
        let unbound = UnboundKey::new(&AES_256_GCM, subkey.as_slice())
            .map_err(|_| PublicCertifiedKeyAeadError::KekUnavailable)?;
        Ok(LessSafeKey::new(unbound))
    }
}

/// AAD: context label, then the key ID and hostname length-prefixed
/// (big-endian `u32`), then the 32 generation bytes.
fn aad_bytes(context: &PublicCertifiedKeyProtectionContext) -> Vec<u8> {
    let id = context.certified_key_id.as_str().as_bytes();
    let hostname = context.public_hostname.as_str().as_bytes();
    let mut aad = Vec::with_capacity(AAD_CONTEXT.len() + 8 + id.len() + hostname.len() + 32);
    aad.extend_from_slice(AAD_CONTEXT);
    for part in [id, hostname] {
        aad.extend_from_slice(&u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
        aad.extend_from_slice(part);
    }
    aad.extend_from_slice(context.generation.as_bytes());
    aad
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
