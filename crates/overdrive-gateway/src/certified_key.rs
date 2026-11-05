//! Public Certified-Key Custody command/error surface.
//!
//! Candidate validation is live; the custody owner and snapshot bodies stay
//! scaffolded until the Gateway Application owner step lands them.

use std::sync::Arc;

use overdrive_core::public_ingress::{
    CertificateFingerprint, CertifiedKeyFailure, CertifiedKeyGeneration, CertifiedKeyProvenance,
    CertifiedKeyUsabilityFailure, PublicCertifiedKeyId, PublicHostname,
};
use overdrive_core::traits::intent_store::IntentStoreError;
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_core::wall_clock::UnixInstant;
use thiserror::Error;
use tokio::sync::watch;
use zeroize::Zeroizing;

pub use overdrive_core::public_ingress::PublicCertifiedKeyAeadError;

/// Producer-neutral install command serialized by the custody owner.
pub struct InstallPublicCertifiedKey {
    pub expected_generation: Option<CertifiedKeyGeneration>,
    pub candidate: CertifiedKeyCandidate,
    pub provenance: CertifiedKeyProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCertifiedKeyInstallOutcome {
    Installed,
    Replaced,
    Unchanged,
}

/// Redacted failure-record command; it carries no path or credential bytes.
pub struct RecordPublicCertifiedKeyFailure {
    pub expected_generation: Option<CertifiedKeyGeneration>,
    pub provenance: CertifiedKeyProvenance,
    pub failure: CertifiedKeyFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCertifiedKeyFailureRecordOutcome {
    Recorded,
    Superseded,
}

/// Validated in-memory chain/key candidate with fixed-redaction Debug.
pub struct CertifiedKeyCandidate {
    chain_der: Vec<Vec<u8>>,
    private_key_pkcs8: Zeroizing<Vec<u8>>,
    public_hostname: PublicHostname,
    generation: CertifiedKeyGeneration,
    fingerprint: CertificateFingerprint,
    not_before: UnixInstant,
    not_after: UnixInstant,
}

/// Accepted certificate-chain length: one leaf plus at most seven issuers.
const CHAIN_LENGTH: std::ops::RangeInclusive<usize> = 1..=8;

impl CertifiedKeyCandidate {
    /// Validate a leaf-first chain and its PKCS#8 key at `now`: chain length,
    /// the single dNSName leaf, key match, and `not_before <= now < not_after`.
    ///
    /// # Errors
    /// `Source(CertificateProfile)` for a chain, leaf profile or validity
    /// interval that fails; `Source(KeyMismatch)` for a key that is not the
    /// leaf's.
    pub fn from_pkcs8(
        chain_der: Vec<Vec<u8>>,
        private_key_pkcs8: Zeroizing<Vec<u8>>,
        now: UnixInstant,
    ) -> Result<Self, PublicCertifiedKeyError> {
        let profile = PublicCertifiedKeyError::Source(CertifiedKeyFailure::CertificateProfile);
        let leaf_der = chain_der
            .first()
            .filter(|_| CHAIN_LENGTH.contains(&chain_der.len()))
            .ok_or_else(profile_error)?;
        let (_, leaf) =
            x509_parser::parse_x509_certificate(leaf_der).map_err(|_| profile_error())?;
        let public_hostname = Self::single_dns_name(&leaf)?;
        let not_before = Self::instant(leaf.validity().not_before.timestamp())?;
        let not_after = Self::instant(leaf.validity().not_after.timestamp())?;
        if now < not_before || now >= not_after {
            return Err(profile);
        }
        Self::require_key_match(&leaf, &private_key_pkcs8)?;
        Ok(Self {
            generation: CertifiedKeyGeneration::from_ordered_chain_der(&chain_der),
            fingerprint: CertificateFingerprint::from_leaf_der(leaf_der),
            chain_der,
            private_key_pkcs8,
            public_hostname,
            not_before,
            not_after,
        })
    }

    /// The leaf's one dNSName SAN as the canonical public hostname.
    fn single_dns_name(
        leaf: &x509_parser::certificate::X509Certificate<'_>,
    ) -> Result<PublicHostname, PublicCertifiedKeyError> {
        let names = leaf
            .subject_alternative_name()
            .ok()
            .flatten()
            .map(|extension| &extension.value.general_names)
            .ok_or_else(profile_error)?;
        match names.as_slice() {
            [x509_parser::extensions::GeneralName::DNSName(name)] => {
                PublicHostname::new(name).map_err(|_| profile_error())
            }
            _ => Err(profile_error()),
        }
    }

    fn instant(seconds: i64) -> Result<UnixInstant, PublicCertifiedKeyError> {
        let seconds = u64::try_from(seconds).map_err(|_| profile_error())?;
        Ok(UnixInstant::from_unix_duration(std::time::Duration::from_secs(seconds)))
    }

    /// Fail closed unless the signing key's public half is the leaf's.
    fn require_key_match(
        leaf: &x509_parser::certificate::X509Certificate<'_>,
        private_key_pkcs8: &Zeroizing<Vec<u8>>,
    ) -> Result<(), PublicCertifiedKeyError> {
        let mismatch = PublicCertifiedKeyError::Source(CertifiedKeyFailure::KeyMismatch);
        let der = rustls::pki_types::PrivateKeyDer::try_from(private_key_pkcs8.as_slice())
            .map_err(|_| PublicCertifiedKeyError::Source(CertifiedKeyFailure::KeyMismatch))?;
        let signing = rustls::crypto::ring::sign::any_supported_type(&der)
            .map_err(|_| PublicCertifiedKeyError::Source(CertifiedKeyFailure::KeyMismatch))?;
        match signing.public_key() {
            Some(public) if public.as_ref() == leaf.public_key().raw => Ok(()),
            _ => Err(mismatch),
        }
    }
}

fn profile_error() -> PublicCertifiedKeyError {
    PublicCertifiedKeyError::Source(CertifiedKeyFailure::CertificateProfile)
}

impl std::fmt::Debug for CertifiedKeyCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CertifiedKeyCandidate").field("material", &"[REDACTED]").finish()
    }
}

/// Opaque usable custody generation retained by Gateway Application.
#[derive(Clone)]
pub struct PublicCertifiedKeySnapshot {
    _private: Arc<()>,
}

impl PublicCertifiedKeySnapshot {
    pub fn certified_key_id(&self) -> &PublicCertifiedKeyId {
        todo!("SCAFFOLD: PublicCertifiedKeySnapshot::certified_key_id")
    }
    pub fn public_hostname(&self) -> &PublicHostname {
        todo!("SCAFFOLD: PublicCertifiedKeySnapshot::public_hostname")
    }
    pub const fn generation(&self) -> CertifiedKeyGeneration {
        panic!("SCAFFOLD: PublicCertifiedKeySnapshot::generation")
    }
    pub const fn fingerprint(&self) -> CertificateFingerprint {
        panic!("SCAFFOLD: PublicCertifiedKeySnapshot::fingerprint")
    }
    pub const fn not_before(&self) -> UnixInstant {
        panic!("SCAFFOLD: PublicCertifiedKeySnapshot::not_before")
    }
    pub const fn not_after(&self) -> UnixInstant {
        panic!("SCAFFOLD: PublicCertifiedKeySnapshot::not_after")
    }
    pub fn is_usable_at(&self, _now: UnixInstant) -> bool {
        todo!("SCAFFOLD: PublicCertifiedKeySnapshot::is_usable_at")
    }
    pub(crate) fn server_config(&self) -> Arc<rustls::ServerConfig> {
        todo!("SCAFFOLD: PublicCertifiedKeySnapshot::server_config")
    }
}

impl std::fmt::Debug for PublicCertifiedKeySnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PublicCertifiedKeySnapshot")
            .field("material", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum PublicCertifiedKeyAvailability {
    Absent,
    Usable(PublicCertifiedKeySnapshot),
    Unusable { generation: CertifiedKeyGeneration, cause: CertifiedKeyUsabilityFailure },
}

/// Clone-safe exact command/watch capability; no withdrawal command exists.
#[derive(Clone)]
pub struct PublicCertifiedKeyCustodyHandle {
    _private: Arc<()>,
}

impl PublicCertifiedKeyCustodyHandle {
    pub fn id(&self) -> &PublicCertifiedKeyId {
        todo!("SCAFFOLD: PublicCertifiedKeyCustodyHandle::id")
    }
    pub async fn install(
        &self,
        _command: InstallPublicCertifiedKey,
    ) -> Result<PublicCertifiedKeyInstallOutcome, PublicCertifiedKeyError> {
        todo!("SCAFFOLD: PublicCertifiedKeyCustodyHandle::install")
    }
    pub async fn record_install_failure(
        &self,
        _command: RecordPublicCertifiedKeyFailure,
    ) -> Result<PublicCertifiedKeyFailureRecordOutcome, PublicCertifiedKeyError> {
        todo!("SCAFFOLD: PublicCertifiedKeyCustodyHandle::record_install_failure")
    }
    pub fn snapshot(&self) -> PublicCertifiedKeyAvailability {
        todo!("SCAFFOLD: PublicCertifiedKeyCustodyHandle::snapshot")
    }
    pub fn subscribe(&self) -> watch::Receiver<PublicCertifiedKeyAvailability> {
        todo!("SCAFFOLD: PublicCertifiedKeyCustodyHandle::subscribe")
    }
}

/// Closed Public Certified-Key Custody failures.
#[derive(Debug, Error)]
pub enum PublicCertifiedKeyError {
    #[error("public certified-key source failed: {0:?}")]
    Source(CertifiedKeyFailure),
    #[error("public certified-key generation changed")]
    ExpectedGenerationMismatch {
        expected: Option<CertifiedKeyGeneration>,
        current: Option<CertifiedKeyGeneration>,
    },
    #[error("public certified-key protection failed: {0}")]
    Protection(PublicCertifiedKeyAeadError),
    #[error("public certified-key intent failed: {0}")]
    Intent(IntentStoreError),
    #[error("public certified-key status failed: {0}")]
    Observation(ObservationStoreError),
    #[error("public certified-key rustls configuration failed")]
    RustlsConfiguration,
    #[error("public certified-key owner unavailable")]
    OwnerUnavailable,
}

#[cfg(test)]
mod acceptance {
    #![allow(clippy::doc_markdown, reason = "Contract Shape metadata")]

    use std::time::Duration;

    use overdrive_core::wall_clock::UnixInstant;
    use zeroize::Zeroizing;

    use super::*;

    /// Inside the validity interval of every generated certificate.
    const VALID_NOW_UNIX_SECONDS: u64 = 1_893_456_000;

    fn candidate_material(chain_len: usize) -> (Vec<Vec<u8>>, Vec<u8>) {
        let root_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("candidate root key");
        let mut root_params =
            rcgen::CertificateParams::new(Vec::<String>::new()).expect("candidate root params");
        root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        root_params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        root_params.not_after = rcgen::date_time_ymd(2040, 1, 1);
        let root = root_params.self_signed(&root_key).expect("candidate root certificate");

        let leaf_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("candidate leaf key");
        let mut leaf_params = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
            .expect("candidate leaf params");
        leaf_params.is_ca = rcgen::IsCa::NoCa;
        leaf_params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        leaf_params.not_after = rcgen::date_time_ymd(2040, 1, 1);
        let root_issuer = rcgen::Issuer::from_params(&root_params, &root_key);
        let leaf =
            leaf_params.signed_by(&leaf_key, &root_issuer).expect("candidate leaf certificate");
        let mut chain = Vec::with_capacity(chain_len);
        if chain_len > 0 {
            chain.push(leaf.der().as_ref().to_vec());
        }
        if chain_len > 1 {
            chain.push(root.der().as_ref().to_vec());
            chain.resize(chain_len, root.der().as_ref().to_vec());
        }
        (chain, leaf_key.serialize_der())
    }

    fn candidate(chain_len: usize) -> Result<CertifiedKeyCandidate, PublicCertifiedKeyError> {
        let (chain, key) = candidate_material(chain_len);
        CertifiedKeyCandidate::from_pkcs8(
            chain,
            Zeroizing::new(key),
            UnixInstant::from_unix_duration(Duration::from_secs(VALID_NOW_UNIX_SECONDS)),
        )
    }

    /// S-PIG-45: certificate chains of 0, 1, many and 9 inside one validity
    /// interval have distinct outcomes (the accepted chain length is 1..=8).
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn certificate_chain_zero_one_many_and_over_limit_have_distinct_outcomes() {
        assert!(matches!(
            candidate(0),
            Err(PublicCertifiedKeyError::Source(CertifiedKeyFailure::CertificateProfile))
        ));
        assert!(candidate(1).is_ok(), "one complete leaf chain is valid");
        assert!(candidate(2).is_ok(), "a leaf-plus-issuer chain is the many partition");
        assert!(matches!(
            candidate(9),
            Err(PublicCertifiedKeyError::Source(CertifiedKeyFailure::CertificateProfile))
        ));
    }
}
