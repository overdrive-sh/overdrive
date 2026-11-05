//! Public Certified-Key Custody command/error surface.
//!
//! Candidate validation is live; the custody owner and snapshot bodies stay
//! scaffolded until the Gateway Application owner step lands them.

use std::io::Read as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::NodeId;
use overdrive_core::public_ingress::{
    CertificateFingerprint, CertifiedKeyFailure, CertifiedKeyGeneration, CertifiedKeyProvenance,
    CertifiedKeyUsabilityFailure, PublicCertifiedKeyId, PublicCertifiedKeyProtectionContext,
    PublicCertifiedKeyStatusRowV1, PublicCertifiedKeyStatusState, PublicCertifiedKeyV1,
    PublicHostname,
};
use overdrive_core::traits::IntentStore;
use overdrive_core::traits::ObservationStore;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStoreError;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStoreError, ObservationWrite,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ResolvesServerCertUsingSni;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
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
    certified_key_id: PublicCertifiedKeyId,
    public_hostname: PublicHostname,
    generation: CertifiedKeyGeneration,
    fingerprint: CertificateFingerprint,
    not_before: UnixInstant,
    not_after: UnixInstant,
    server_config: Arc<rustls::ServerConfig>,
}

impl PublicCertifiedKeySnapshot {
    /// Build the snapshot from validated material: the one public
    /// `ServerConfig` builder (TLS 1.3, the two AES-GCM suites, ALPN
    /// `http/1.1`, exact SNI, no tickets, no 0-RTT, secret extraction on).
    fn build(
        certified_key_id: PublicCertifiedKeyId,
        record: &PublicCertifiedKeyV1,
        private_key_pkcs8: &[u8],
    ) -> Result<Self, PublicCertifiedKeyError> {
        let server_config = Self::server_config_for(
            &record.public_hostname,
            &record.certificate_chain_der,
            private_key_pkcs8,
        )?;
        Ok(Self {
            certified_key_id,
            public_hostname: record.public_hostname.clone(),
            generation: record.certified_key_generation,
            fingerprint: record.certificate_fingerprint,
            not_before: record.not_before,
            not_after: record.not_after,
            server_config,
        })
    }

    fn server_config_for(
        hostname: &PublicHostname,
        chain_der: &[Vec<u8>],
        private_key_pkcs8: &[u8],
    ) -> Result<Arc<rustls::ServerConfig>, PublicCertifiedKeyError> {
        let failed = || PublicCertifiedKeyError::RustlsConfiguration;
        let chain: Vec<CertificateDer<'static>> =
            chain_der.iter().map(|der| CertificateDer::from(der.clone())).collect();
        let key_der = PrivateKeyDer::try_from(private_key_pkcs8).map_err(|_| failed())?;
        let signing =
            rustls::crypto::ring::sign::any_supported_type(&key_der).map_err(|_| failed())?;
        let mut resolver = ResolvesServerCertUsingSni::new();
        resolver
            .add(hostname.as_str(), rustls::sign::CertifiedKey::new(chain, signing))
            .map_err(|_| failed())?;
        let mut provider = rustls::crypto::ring::default_provider();
        provider.cipher_suites = vec![
            rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256,
            rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384,
        ];
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| failed())?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config.send_tls13_tickets = 0;
        config.max_early_data_size = 0;
        config.enable_secret_extraction = true;
        config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        Ok(Arc::new(config))
    }

    pub fn certified_key_id(&self) -> &PublicCertifiedKeyId {
        &self.certified_key_id
    }
    pub fn public_hostname(&self) -> &PublicHostname {
        &self.public_hostname
    }
    pub const fn generation(&self) -> CertifiedKeyGeneration {
        self.generation
    }
    pub const fn fingerprint(&self) -> CertificateFingerprint {
        self.fingerprint
    }
    pub const fn not_before(&self) -> UnixInstant {
        self.not_before
    }
    pub const fn not_after(&self) -> UnixInstant {
        self.not_after
    }
    pub fn is_usable_at(&self, now: UnixInstant) -> bool {
        self.usability_at(now).is_none()
    }
    pub(crate) fn server_config(&self) -> Arc<rustls::ServerConfig> {
        Arc::clone(&self.server_config)
    }

    /// Why the generation is not usable at `now`, or `None` when it is.
    fn usability_at(&self, now: UnixInstant) -> Option<CertifiedKeyUsabilityFailure> {
        if now < self.not_before {
            Some(CertifiedKeyUsabilityFailure::NotYetValid)
        } else if now >= self.not_after {
            Some(CertifiedKeyUsabilityFailure::Expired)
        } else {
            None
        }
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

impl PublicCertifiedKeyAvailability {
    fn generation(&self) -> Option<CertifiedKeyGeneration> {
        match self {
            Self::Absent => None,
            Self::Usable(snapshot) => Some(snapshot.generation()),
            Self::Unusable { generation, .. } => Some(*generation),
        }
    }
}

/// Prefix of the protected custody records.
pub(crate) const CUSTODY_PREFIX: &str = "public-ingress/certified-key/";

/// Private finite maintenance cadence of the custody owner and the manual
/// source: expiry rechecks, status retry and file refresh.
const MAINTENANCE_CADENCE: Duration = Duration::from_secs(30);

/// Label handed to the intent codec where a store file path would go.
const INTENT_STORE_LABEL: &str = "<intent store>";

/// Host name used only by the non-persisted canary seal/open.
const CANARY_HOSTNAME: &str = "canary.example";

const COMMAND_CAPACITY: usize = 4;

enum CustodyCommand {
    Install {
        command: InstallPublicCertifiedKey,
        reply: oneshot::Sender<Result<PublicCertifiedKeyInstallOutcome, PublicCertifiedKeyError>>,
    },
    RecordFailure {
        command: RecordPublicCertifiedKeyFailure,
        reply: oneshot::Sender<
            Result<PublicCertifiedKeyFailureRecordOutcome, PublicCertifiedKeyError>,
        >,
    },
}

/// Clone-safe exact command/watch capability; no withdrawal command exists.
#[derive(Clone)]
pub struct PublicCertifiedKeyCustodyHandle {
    id: PublicCertifiedKeyId,
    commands: mpsc::Sender<CustodyCommand>,
    availability: watch::Receiver<PublicCertifiedKeyAvailability>,
}

impl PublicCertifiedKeyCustodyHandle {
    pub fn id(&self) -> &PublicCertifiedKeyId {
        &self.id
    }
    pub async fn install(
        &self,
        command: InstallPublicCertifiedKey,
    ) -> Result<PublicCertifiedKeyInstallOutcome, PublicCertifiedKeyError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(CustodyCommand::Install { command, reply })
            .await
            .map_err(|_| PublicCertifiedKeyError::OwnerUnavailable)?;
        answer.await.map_err(|_| PublicCertifiedKeyError::OwnerUnavailable)?
    }
    pub async fn record_install_failure(
        &self,
        command: RecordPublicCertifiedKeyFailure,
    ) -> Result<PublicCertifiedKeyFailureRecordOutcome, PublicCertifiedKeyError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(CustodyCommand::RecordFailure { command, reply })
            .await
            .map_err(|_| PublicCertifiedKeyError::OwnerUnavailable)?;
        answer.await.map_err(|_| PublicCertifiedKeyError::OwnerUnavailable)?
    }
    pub fn snapshot(&self) -> PublicCertifiedKeyAvailability {
        self.availability.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<PublicCertifiedKeyAvailability> {
        self.availability.clone()
    }
}

/// The custody owner state: opened at builder construction, run as one task
/// at start. Every mutation is serialized through it.
pub(crate) struct PublicCertifiedKeyCustody {
    id: PublicCertifiedKeyId,
    node_id: NodeId,
    intent: Arc<dyn IntentStore>,
    observations: Arc<dyn ObservationStore>,
    codec: Arc<PublicCertifiedKeyAeadCodec>,
    clock: Arc<dyn Clock>,
    current: Option<CurrentKey>,
    availability: watch::Sender<PublicCertifiedKeyAvailability>,
    last_failure: Option<CertifiedKeyFailure>,
    status: StatusProjection,
    commands: mpsc::Receiver<CustodyCommand>,
}

struct CurrentKey {
    record: PublicCertifiedKeyV1,
    snapshot: PublicCertifiedKeySnapshot,
}

/// The last status row the owner wrote (or read) and whether the newest
/// projection still has to be written.
#[derive(Default)]
struct StatusProjection {
    stamp: Option<LogicalTimestamp>,
    stamp_read: bool,
    written: Option<(PublicCertifiedKeyStatusState, Option<CertifiedKeyFailure>)>,
}

impl PublicCertifiedKeyCustody {
    /// Earned-Trust open: adopt the stored record (decrypting and rebuilding
    /// it) or, when none is stored, prove the codec with a non-persisted
    /// canary seal/open.
    pub(crate) async fn open(
        id: PublicCertifiedKeyId,
        node_id: NodeId,
        intent: Arc<dyn IntentStore>,
        observations: Arc<dyn ObservationStore>,
        codec: Arc<PublicCertifiedKeyAeadCodec>,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, PublicCertifiedKeyCustodyHandle), PublicCertifiedKeyError> {
        let key = Self::record_key(&id);
        let stored = intent.get(key.as_bytes()).await.map_err(PublicCertifiedKeyError::Intent)?;
        let current = if let Some(bytes) = stored {
            Some(Self::adopt(&id, &bytes, &key, &codec)?)
        } else {
            Self::canary(&id, &codec)?;
            None
        };
        let now = UnixInstant::from_clock(clock.as_ref());
        let initial = Self::availability_of(current.as_ref(), now);
        let (availability, receiver) = watch::channel(initial);
        let (sender, commands) = mpsc::channel(COMMAND_CAPACITY);
        let handle = PublicCertifiedKeyCustodyHandle {
            id: id.clone(),
            commands: sender,
            availability: receiver,
        };
        let custody = Self {
            id,
            node_id,
            intent,
            observations,
            codec,
            clock,
            current,
            availability,
            last_failure: None,
            status: StatusProjection::default(),
            commands,
        };
        Ok((custody, handle))
    }

    fn record_key(id: &PublicCertifiedKeyId) -> String {
        format!("{CUSTODY_PREFIX}{}", id.as_str())
    }

    fn adopt(
        id: &PublicCertifiedKeyId,
        bytes: &[u8],
        key: &str,
        codec: &PublicCertifiedKeyAeadCodec,
    ) -> Result<CurrentKey, PublicCertifiedKeyError> {
        let record =
            PublicCertifiedKeyV1::from_store_bytes(bytes, Path::new(INTENT_STORE_LABEL), Some(key))
                .map_err(PublicCertifiedKeyError::Intent)?;
        let context = PublicCertifiedKeyProtectionContext {
            certified_key_id: id.clone(),
            public_hostname: record.public_hostname.clone(),
            generation: record.certified_key_generation,
        };
        let plaintext = codec
            .open(&context, &record.protected_origin_key)
            .map_err(PublicCertifiedKeyError::Protection)?;
        let snapshot = PublicCertifiedKeySnapshot::build(id.clone(), &record, &plaintext)?;
        Ok(CurrentKey { record, snapshot })
    }

    fn canary(
        id: &PublicCertifiedKeyId,
        codec: &PublicCertifiedKeyAeadCodec,
    ) -> Result<(), PublicCertifiedKeyError> {
        let context = PublicCertifiedKeyProtectionContext {
            certified_key_id: id.clone(),
            public_hostname: PublicHostname::new(CANARY_HOSTNAME)
                .unwrap_or_else(|_| unreachable!("the canary hostname is a valid public hostname")),
            generation: CertifiedKeyGeneration::from_ordered_chain_der(&[]),
        };
        let canary = Zeroizing::new(vec![0x5a_u8; 32]);
        let protected =
            codec.seal(&context, &canary).map_err(PublicCertifiedKeyError::Protection)?;
        codec.open(&context, &protected).map_err(PublicCertifiedKeyError::Protection)?;
        Ok(())
    }

    fn availability_of(
        current: Option<&CurrentKey>,
        now: UnixInstant,
    ) -> PublicCertifiedKeyAvailability {
        let Some(current) = current else {
            return PublicCertifiedKeyAvailability::Absent;
        };
        current.snapshot.usability_at(now).map_or_else(
            || PublicCertifiedKeyAvailability::Usable(current.snapshot.clone()),
            |cause| PublicCertifiedKeyAvailability::Unusable {
                generation: current.snapshot.generation(),
                cause,
            },
        )
    }

    fn now(&self) -> UnixInstant {
        UnixInstant::from_clock(self.clock.as_ref())
    }

    pub(crate) fn current_generation(&self) -> Option<CertifiedKeyGeneration> {
        self.current.as_ref().map(|current| current.snapshot.generation())
    }

    /// The install transaction (design steps 1-6): guard, validate, build the
    /// snapshot, seal, persist, publish, then project status.
    pub(crate) async fn install(
        &mut self,
        command: InstallPublicCertifiedKey,
    ) -> Result<PublicCertifiedKeyInstallOutcome, PublicCertifiedKeyError> {
        let current = self.current_generation();
        if command.expected_generation != current {
            return Err(PublicCertifiedKeyError::ExpectedGenerationMismatch {
                expected: command.expected_generation,
                current,
            });
        }
        let candidate = command.candidate;
        let now = self.now();
        if now < candidate.not_before || now >= candidate.not_after {
            return Err(profile_error());
        }
        if current == Some(candidate.generation) {
            self.last_failure = None;
            self.project_status().await;
            return Ok(PublicCertifiedKeyInstallOutcome::Unchanged);
        }
        let context = PublicCertifiedKeyProtectionContext {
            certified_key_id: self.id.clone(),
            public_hostname: candidate.public_hostname.clone(),
            generation: candidate.generation,
        };
        let protected = self
            .codec
            .seal(&context, &candidate.private_key_pkcs8)
            .map_err(PublicCertifiedKeyError::Protection)?;
        let record = PublicCertifiedKeyV1 {
            certified_key_id: self.id.clone(),
            public_hostname: candidate.public_hostname.clone(),
            certified_key_generation: candidate.generation,
            certificate_fingerprint: candidate.fingerprint,
            certificate_chain_der: candidate.chain_der.clone(),
            not_before: candidate.not_before,
            not_after: candidate.not_after,
            provenance: command.provenance,
            protected_origin_key: protected,
        };
        let snapshot = PublicCertifiedKeySnapshot::build(
            self.id.clone(),
            &record,
            &candidate.private_key_pkcs8,
        )?;
        let bytes =
            record.archive_for_store().map_err(|_| PublicCertifiedKeyError::RustlsConfiguration)?;
        self.intent
            .put(Self::record_key(&self.id).as_bytes(), bytes.as_slice())
            .await
            .map_err(PublicCertifiedKeyError::Intent)?;
        let outcome = if self.current.is_some() {
            PublicCertifiedKeyInstallOutcome::Replaced
        } else {
            PublicCertifiedKeyInstallOutcome::Installed
        };
        self.availability.send_replace(PublicCertifiedKeyAvailability::Usable(snapshot.clone()));
        self.current = Some(CurrentKey { record, snapshot });
        self.last_failure = None;
        self.project_status().await;
        Ok(outcome)
    }

    /// Redacted failure record: status only, generation-checked.
    pub(crate) async fn record_install_failure(
        &mut self,
        command: RecordPublicCertifiedKeyFailure,
    ) -> PublicCertifiedKeyFailureRecordOutcome {
        if command.expected_generation != self.current_generation() {
            return PublicCertifiedKeyFailureRecordOutcome::Superseded;
        }
        self.last_failure = Some(command.failure);
        self.project_status().await;
        PublicCertifiedKeyFailureRecordOutcome::Recorded
    }

    /// Recheck both validity bounds: an expired or not-yet-valid current
    /// generation is first removed from live publication, then projected.
    async fn maintain(&mut self) {
        let now = self.now();
        let next = Self::availability_of(self.current.as_ref(), now);
        let changed = match (&*self.availability.borrow(), &next) {
            (
                PublicCertifiedKeyAvailability::Usable(_),
                PublicCertifiedKeyAvailability::Usable(_),
            )
            | (PublicCertifiedKeyAvailability::Absent, PublicCertifiedKeyAvailability::Absent) => {
                false
            }
            (
                PublicCertifiedKeyAvailability::Unusable { generation: held, cause: held_cause },
                PublicCertifiedKeyAvailability::Unusable { generation, cause },
            ) => held != generation || held_cause != cause,
            _ => true,
        };
        if changed {
            self.availability.send_replace(next);
        }
        self.project_status().await;
    }

    fn projected_state(&self) -> PublicCertifiedKeyStatusState {
        let Some(current) = &self.current else {
            return PublicCertifiedKeyStatusState::Absent;
        };
        let now = self.now();
        current.snapshot.usability_at(now).map_or_else(
            || PublicCertifiedKeyStatusState::Usable {
                generation: current.record.certified_key_generation,
                certificate_fingerprint: current.record.certificate_fingerprint,
                public_hostname: current.record.public_hostname.clone(),
                not_before: current.record.not_before,
                not_after: current.record.not_after,
                provenance: current.record.provenance.clone(),
            },
            |cause| PublicCertifiedKeyStatusState::Unusable {
                generation: current.record.certified_key_generation,
                cause,
            },
        )
    }

    /// Write the status row when its projection differs from what was last
    /// written. A failed write is retried on the next maintenance turn; it
    /// never rolls back the install.
    pub(crate) async fn project_status(&mut self) {
        let projection = (self.projected_state(), self.last_failure);
        if self.status.written.as_ref() == Some(&projection) {
            return;
        }
        if !self.status.stamp_read {
            match self.observations.public_certified_key_status_row(&self.id).await {
                Ok(prior) => {
                    self.status.stamp = prior.map(|row| row.updated_at);
                    self.status.stamp_read = true;
                }
                Err(_) => return,
            }
        }
        let updated_at =
            LogicalTimestamp::dominating(0, self.node_id.clone(), self.status.stamp.as_ref());
        let row = PublicCertifiedKeyStatusRowV1 {
            certified_key_id: self.id.clone(),
            state: projection.0.clone(),
            last_install_failure: projection.1,
            updated_at: updated_at.clone(),
        };
        if self.observations.write(ObservationWrite::PublicCertifiedKeyStatus(row)).await.is_ok() {
            self.status.stamp = Some(updated_at);
            self.status.written = Some(projection);
        }
    }

    /// Serve commands and the maintenance cadence until `shutdown`.
    pub(crate) async fn run(mut self, shutdown: CancellationToken) {
        loop {
            let cadence = self.clock.sleep(MAINTENANCE_CADENCE);
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return,
                command = self.commands.recv() => match command {
                    None => return,
                    Some(CustodyCommand::Install { command, reply }) => {
                        let _ = reply.send(self.install(command).await);
                    }
                    Some(CustodyCommand::RecordFailure { command, reply }) => {
                        let _ = reply.send(Ok(self.record_install_failure(command).await));
                    }
                },
                () = cadence => self.maintain().await,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manual certified-key source
// ---------------------------------------------------------------------------

/// A leaf-first DER chain and its PKCS#8 private key.
type ManualMaterial = (Vec<Vec<u8>>, Zeroizing<Vec<u8>>);

/// Largest accepted private-key file.
const KEY_FILE_MAX: u64 = 64 * 1024;
/// Largest accepted certificate-chain file.
const CHAIN_FILE_MAX: u64 = 1024 * 1024;

/// The host-local file producer: the only first-slice caller of
/// `CertifiedKeyCandidate::from_pkcs8`.
pub(crate) struct ManualCertifiedKeySource {
    chain_path: PathBuf,
    key_path: PathBuf,
}

impl ManualCertifiedKeySource {
    pub(crate) fn new(chain_path: PathBuf, key_path: PathBuf) -> Self {
        Self { chain_path, key_path }
    }

    /// Read, parse and validate both files at `now`, off the async workers.
    pub(crate) async fn candidate(
        &self,
        now: UnixInstant,
    ) -> Result<CertifiedKeyCandidate, PublicCertifiedKeyError> {
        let (chain_path, key_path) = (self.chain_path.clone(), self.key_path.clone());
        let material =
            tokio::task::spawn_blocking(move || Self::read_material(&chain_path, &key_path))
                .await
                .map_err(|_| PublicCertifiedKeyError::Source(CertifiedKeyFailure::SourceOpen))??;
        CertifiedKeyCandidate::from_pkcs8(material.0, material.1, now)
    }

    fn read_material(
        chain_path: &Path,
        key_path: &Path,
    ) -> Result<ManualMaterial, PublicCertifiedKeyError> {
        let chain_bytes = Self::read_file(chain_path, CHAIN_FILE_MAX, false)?;
        let key_bytes = Zeroizing::new(Self::read_file(key_path, KEY_FILE_MAX, true)?);
        let chain = CertificateDer::pem_slice_iter(&chain_bytes)
            .map(|item| item.map(|der| der.as_ref().to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| source_error(CertifiedKeyFailure::PemShape))?;
        if !CHAIN_LENGTH.contains(&chain.len()) {
            return Err(source_error(CertifiedKeyFailure::PemShape));
        }
        let mut keys = PrivatePkcs8KeyDer::pem_slice_iter(&key_bytes);
        let (Some(Ok(key)), None) = (keys.next(), keys.next()) else {
            return Err(source_error(CertifiedKeyFailure::PemShape));
        };
        Ok((chain, Zeroizing::new(key.secret_pkcs8_der().to_vec())))
    }

    /// One bounded read: absolute, final-component non-symlink, regular,
    /// owned by the effective UID, no group/other mode bits for a key.
    fn read_file(
        path: &Path,
        bound: u64,
        is_private_key: bool,
    ) -> Result<Vec<u8>, PublicCertifiedKeyError> {
        let metadata_failure = || source_error(CertifiedKeyFailure::SourceMetadata);
        if !path.is_absolute() {
            return Err(metadata_failure());
        }
        let link = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                source_error(CertifiedKeyFailure::SourceOpen)
            }
            _ => metadata_failure(),
        })?;
        if !link.file_type().is_file() {
            return Err(metadata_failure());
        }
        let effective_uid = rustix::process::geteuid().as_raw();
        if link.uid() != effective_uid {
            return Err(source_error(CertifiedKeyFailure::SourceOwnership));
        }
        if is_private_key && link.mode() & 0o077 != 0 {
            return Err(source_error(CertifiedKeyFailure::SourcePermissions));
        }
        if link.len() > bound {
            return Err(source_error(CertifiedKeyFailure::SourceBounds));
        }
        let file =
            std::fs::File::open(path).map_err(|_| source_error(CertifiedKeyFailure::SourceOpen))?;
        let opened = file.metadata().map_err(|_| metadata_failure())?;
        if (opened.dev(), opened.ino()) != (link.dev(), link.ino()) {
            return Err(metadata_failure());
        }
        let mut bytes = Vec::new();
        file.take(bound.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| source_error(CertifiedKeyFailure::SourceOpen))?;
        if bytes.len() as u64 > bound {
            return Err(source_error(CertifiedKeyFailure::SourceBounds));
        }
        Ok(bytes)
    }

    /// Maintenance: re-read on the private cadence and install, replay or
    /// record the redacted failure. Never mutates custody on an invalid read.
    pub(crate) async fn run(
        self,
        custody: PublicCertifiedKeyCustodyHandle,
        clock: Arc<dyn Clock>,
        shutdown: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return,
                () = clock.sleep(MAINTENANCE_CADENCE) => {}
            }
            let expected_generation = custody.snapshot().generation();
            let now = UnixInstant::from_clock(clock.as_ref());
            let failure = match self.candidate(now).await {
                Ok(candidate) => {
                    let command = InstallPublicCertifiedKey {
                        expected_generation,
                        candidate,
                        provenance: CertifiedKeyProvenance::Manual,
                    };
                    match custody.install(command).await {
                        Ok(_) | Err(PublicCertifiedKeyError::ExpectedGenerationMismatch { .. }) => {
                            continue;
                        }
                        Err(error) => error.failure_category(),
                    }
                }
                Err(error) => error.failure_category(),
            };
            let _ = custody
                .record_install_failure(RecordPublicCertifiedKeyFailure {
                    expected_generation,
                    provenance: CertifiedKeyProvenance::Manual,
                    failure,
                })
                .await;
        }
    }
}

fn source_error(failure: CertifiedKeyFailure) -> PublicCertifiedKeyError {
    PublicCertifiedKeyError::Source(failure)
}

impl PublicCertifiedKeyError {
    /// The redacted category this error is recorded under in status.
    fn failure_category(&self) -> CertifiedKeyFailure {
        match self {
            Self::Source(failure) => *failure,
            Self::Protection(_) => CertifiedKeyFailure::Protection,
            Self::RustlsConfiguration => CertifiedKeyFailure::RustlsConfiguration,
            Self::ExpectedGenerationMismatch { .. }
            | Self::Intent(_)
            | Self::Observation(_)
            | Self::OwnerUnavailable => CertifiedKeyFailure::Persistence,
        }
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
