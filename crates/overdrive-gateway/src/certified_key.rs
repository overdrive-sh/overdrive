//! Public Certified-Key Custody command/error surface.
//!
//! SCAFFOLD: true. Custody behavior remains RED until DELIVER.

#![allow(clippy::unused_self, reason = "exact accessor RED signatures retain self")]
#![expect(clippy::unused_async, reason = "exact command-handle RED signatures are async")]

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
    _private: (),
}

impl CertifiedKeyCandidate {
    pub fn from_pkcs8(
        _chain_der: Vec<Vec<u8>>,
        _private_key_pkcs8: Zeroizing<Vec<u8>>,
        _now: UnixInstant,
    ) -> Result<Self, PublicCertifiedKeyError> {
        todo!("SCAFFOLD: CertifiedKeyCandidate::from_pkcs8")
    }
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
    #[expect(dead_code, reason = "activated by the DELIVER public TLS runtime")]
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

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER public Certified Key candidate validation"]
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
