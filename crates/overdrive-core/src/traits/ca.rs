//! [`Ca`] — the built-in certificate-authority port trait.
//!
//! The persistent workload-identity trust hierarchy (GH #28, ADR-0063):
//!
//! ```text
//! Root CA (self-signed, P-256, CA:TRUE, keyCertSign|cRLSign)
//!   └── Node Intermediate CA (signed by root, pathLen=0, one per node)
//!         └── Workload SVID (leaf, exactly ONE spiffe:// URI SAN, CA:FALSE,
//!                            keyUsage=digitalSignature critical)
//! ```
//!
//! Per ADR-0063 D1 this is a **pure port trait in `overdrive-core`** (class
//! `core`): no `rcgen`, no `ring`, no FFI, no entropy backend — the dst-lint
//! gate rejects all of those on a core compile path. The trait surface speaks
//! **project newtypes** (`SpiffeId` / `CertSerial` / `NodeId`) plus the typed
//! cert/key/bundle **byte newtypes** defined in this module ([`CaCertPem`],
//! [`CaCertDer`], [`TrustBundlePem`], …). An `rcgen` type never crosses this
//! boundary — that keeps `rcgen` out of core's compile graph while core still
//! owns the *decision* (the pure [`CertSpec`](crate::ca::CertSpec) policy from
//! step 01-01) of what each certificate carries.
//!
//! Two adapters implement it: `RcgenCa` in `overdrive-host` (owns all crypto)
//! and `SimCa` in `overdrive-sim` (loads fixture keys, draws serials via the
//! seeded `Entropy` port → DST-deterministic). Consumers take `Arc<dyn Ca>`
//! as a **required constructor parameter** — never defaulted to a production
//! binding (`.claude/rules/development.md` § "Port-trait dependencies").

use crate::ca::root_key_envelope::KekId;
use crate::{CertSerial, CertSpecError, NodeId, SpiffeId};

/// Result alias used throughout the CA port surface.
pub type Result<T, E = CaError> = std::result::Result<T, E>;

/// PEM-encoded certificate text (the `-----BEGIN CERTIFICATE-----` form).
///
/// An opaque byte newtype: core never parses the PEM (no crypto backend on a
/// core compile path). Adapters produce it; consumers observe it via
/// [`as_pem`](CaCertPem::as_pem).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaCertPem(String);

impl CaCertPem {
    /// Wrap raw PEM text.
    #[must_use]
    pub const fn new(pem: String) -> Self {
        Self(pem)
    }

    /// Borrow the PEM text — the contract-observable accessor.
    #[must_use]
    pub fn as_pem(&self) -> &str {
        &self.0
    }
}

/// DER-encoded certificate bytes (the binary X.509 form).
///
/// Opaque byte newtype; observed via [`as_der`](CaCertDer::as_der).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaCertDer(Vec<u8>);

impl CaCertDer {
    /// Wrap raw DER bytes.
    #[must_use]
    pub const fn new(der: Vec<u8>) -> Self {
        Self(der)
    }

    /// Borrow the DER bytes — the contract-observable accessor.
    #[must_use]
    pub fn as_der(&self) -> &[u8] {
        &self.0
    }
}

/// PEM-encoded private-key text held only inside a signer adapter.
///
/// This is the **sign-capability material** a [`RootCaHandle`] /
/// [`IntermediateHandle`] holds internally. Per ADR-0063 D1 (research
/// Finding 5 — "keys never leave the signer") the private key never crosses
/// the trait boundary as raw bytes for *issued* material; this newtype exists
/// so the root/intermediate adapter can carry its own signing key as opaque
/// bytes without leaking it through `issue_svid` output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaKeyPem(String);

impl CaKeyPem {
    /// Wrap raw private-key PEM text.
    #[must_use]
    pub const fn new(pem: String) -> Self {
        Self(pem)
    }

    /// Borrow the key PEM text.
    #[must_use]
    pub fn as_pem(&self) -> &str {
        &self.0
    }
}

/// PEM-encoded trust bundle (one or more concatenated certificates).
///
/// The relying-party verification material composed by
/// [`Ca::trust_bundle`]. Opaque byte newtype; observed via
/// [`as_pem`](TrustBundlePem::as_pem).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrustBundlePem(String);

impl TrustBundlePem {
    /// Wrap raw bundle PEM text.
    #[must_use]
    pub const fn new(pem: String) -> Self {
        Self(pem)
    }

    /// Borrow the bundle PEM text — the contract-observable accessor.
    #[must_use]
    pub fn as_pem(&self) -> &str {
        &self.0
    }
}

/// The persistent self-signed root CA.
///
/// Exposes the root certificate (PEM + DER) and its serial through
/// contract-observable accessors. Holds the root signing key internally as a
/// [`CaKeyPem`] sign-capability handle — the private key is NOT exposed as a
/// trait-boundary output (research Finding 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCaHandle {
    cert_pem: CaCertPem,
    cert_der: CaCertDer,
    serial: CertSerial,
    signing_key: CaKeyPem,
}

impl RootCaHandle {
    /// Assemble a root handle from its observable material plus the internal
    /// signing key.
    #[must_use]
    pub const fn new(
        cert_pem: CaCertPem,
        cert_der: CaCertDer,
        serial: CertSerial,
        signing_key: CaKeyPem,
    ) -> Self {
        Self { cert_pem, cert_der, serial, signing_key }
    }

    /// The root certificate in PEM form.
    #[must_use]
    pub const fn cert_pem(&self) -> &CaCertPem {
        &self.cert_pem
    }

    /// The root certificate in DER form.
    #[must_use]
    pub const fn cert_der(&self) -> &CaCertDer {
        &self.cert_der
    }

    /// The root certificate serial number.
    #[must_use]
    pub const fn serial(&self) -> &CertSerial {
        &self.serial
    }

    /// The internal sign-capability handle (root signing key, PEM).
    ///
    /// Used by the adapter to sign intermediates; not part of the
    /// relying-party observable surface.
    #[must_use]
    pub const fn signing_key(&self) -> &CaKeyPem {
        &self.signing_key
    }
}

/// A node intermediate CA, `pathLen=0`, signed by the root.
///
/// Same shape as [`RootCaHandle`] — observable cert PEM/DER + serial, internal
/// signing key — but `CA:TRUE` with `pathLen=0` (issues leaves only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntermediateHandle {
    cert_pem: CaCertPem,
    cert_der: CaCertDer,
    serial: CertSerial,
    signing_key: CaKeyPem,
}

impl IntermediateHandle {
    /// Assemble an intermediate handle from its observable material plus the
    /// internal signing key.
    #[must_use]
    pub const fn new(
        cert_pem: CaCertPem,
        cert_der: CaCertDer,
        serial: CertSerial,
        signing_key: CaKeyPem,
    ) -> Self {
        Self { cert_pem, cert_der, serial, signing_key }
    }

    /// The intermediate certificate in PEM form.
    #[must_use]
    pub const fn cert_pem(&self) -> &CaCertPem {
        &self.cert_pem
    }

    /// The intermediate certificate in DER form.
    #[must_use]
    pub const fn cert_der(&self) -> &CaCertDer {
        &self.cert_der
    }

    /// The intermediate certificate serial number.
    #[must_use]
    pub const fn serial(&self) -> &CertSerial {
        &self.serial
    }

    /// The internal sign-capability handle (intermediate signing key, PEM).
    #[must_use]
    pub const fn signing_key(&self) -> &CaKeyPem {
        &self.signing_key
    }
}

/// A request to mint a workload SVID leaf.
///
/// Carries the workload's [`SpiffeId`] — the single URI SAN the minted
/// certificate must carry. The single-URI-SAN invariant ([`Ca::issue_svid`])
/// is enforced against this identity before any certificate is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvidRequest {
    spiffe_id: SpiffeId,
}

impl SvidRequest {
    /// Build an SVID request for a workload identity.
    #[must_use]
    pub const fn new(spiffe_id: SpiffeId) -> Self {
        Self { spiffe_id }
    }

    /// The workload identity this SVID is requested for.
    #[must_use]
    pub const fn spiffe_id(&self) -> &SpiffeId {
        &self.spiffe_id
    }
}

/// Minted workload-SVID material returned by [`Ca::issue_svid`].
///
/// Carries the leaf certificate (PEM + DER), its serial, and the SPIFFE
/// identity it was minted for — all contract-observable. Per research
/// Finding 5 the leaf's *private key* is generated and held by the requesting
/// workload's keypair flow, NOT by the CA, so it is not part of this output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvidMaterial {
    cert_pem: CaCertPem,
    cert_der: CaCertDer,
    serial: CertSerial,
    spiffe_id: SpiffeId,
}

impl SvidMaterial {
    /// Assemble SVID material from its observable parts.
    #[must_use]
    pub const fn new(
        cert_pem: CaCertPem,
        cert_der: CaCertDer,
        serial: CertSerial,
        spiffe_id: SpiffeId,
    ) -> Self {
        Self { cert_pem, cert_der, serial, spiffe_id }
    }

    /// The SVID leaf certificate in PEM form.
    #[must_use]
    pub const fn cert_pem(&self) -> &CaCertPem {
        &self.cert_pem
    }

    /// The SVID leaf certificate in DER form.
    #[must_use]
    pub const fn cert_der(&self) -> &CaCertDer {
        &self.cert_der
    }

    /// The SVID serial number (CSPRNG via the `Entropy` port).
    #[must_use]
    pub const fn serial(&self) -> &CertSerial {
        &self.serial
    }

    /// The workload identity carried as the single URI SAN.
    #[must_use]
    pub const fn spiffe_id(&self) -> &SpiffeId {
        &self.spiffe_id
    }
}

/// The trust bundle a relying party verifies an SVID chain against.
///
/// Composed by [`Ca::trust_bundle`]: the root certificate is the **trust
/// anchor**; the intermediate is **untrusted chain material** the verifier
/// uses to build the path but does not itself anchor trust on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustBundle {
    anchor_pem: TrustBundlePem,
}

impl TrustBundle {
    /// Build a trust bundle from its composed anchor PEM.
    #[must_use]
    pub const fn new(anchor_pem: TrustBundlePem) -> Self {
        Self { anchor_pem }
    }

    /// The bundle PEM (root anchor; intermediate chain material appended).
    #[must_use]
    pub const fn anchor_pem(&self) -> &TrustBundlePem {
        &self.anchor_pem
    }
}

/// A certificate-authority operation failure.
///
/// Distinct per failure mode (`.claude/rules/development.md` § "Distinct
/// failure modes get distinct error variants"). In particular an invalid URI
/// SAN cardinality surfaces [`InvalidSan`](CaError::InvalidSan) — it is NEVER
/// flattened into a generic `Internal(String)`, so the load-bearing
/// single-URI-SAN signal (KPI K2) cannot be swallowed. The pure-policy
/// [`CertSpecError`] passes through via `#[from]` so a policy rejection keeps
/// its structured shape across the adapter boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaError {
    /// The request would yield an invalid number of `spiffe://` URI SANs.
    /// A SPIFFE-compliant SVID carries exactly one; zero or two-or-more is
    /// rejected before any certificate is produced (KPI K2, research
    /// Finding 2).
    #[error("invalid SAN cardinality: expected exactly one spiffe:// URI SAN, found {found}")]
    InvalidSan {
        /// The number of URI SANs the rejected request would have produced.
        found: usize,
    },

    /// The subject identity is invalid for the requested role (e.g. a CA
    /// subject carrying a path component where the trust domain alone is
    /// required).
    #[error("invalid subject for {role}: {reason}")]
    InvalidSubject {
        /// The role whose subject contract was violated.
        role: &'static str,
        /// Human-readable explanation of the contract violation.
        reason: &'static str,
    },

    /// The pure certificate-profile policy rejected the request. Embedded via
    /// `#[from]` (pass-through, not duplicated) so the structured
    /// [`CertSpecError`] survives the adapter boundary.
    #[error(transparent)]
    Policy {
        /// The underlying policy rejection.
        #[from]
        source: CertSpecError,
    },

    /// The signing operation failed inside the adapter (malformed fixture
    /// key, signing-backend error). Carries a human-readable reason; adapters
    /// map their backend-specific errors into this variant.
    #[error("certificate signing failed: {reason}")]
    SigningFailed {
        /// Human-readable explanation of the signing failure.
        reason: String,
    },

    /// The persisted root-key envelope failed AES-GCM authentication when
    /// opened under the **correct** KEK — the ciphertext or tag was tampered
    /// with / corrupted at rest. Distinct from [`WrongKek`](CaError::WrongKek):
    /// the supplied KEK's id matches the record's `kek_id` (the AAD), so this
    /// is integrity failure, not KEK-confusion (ADR-0063 D3/D4). The boot path
    /// refuses to start on this error — never a silent re-mint.
    #[error("root-key envelope is corrupt or tampered (AES-GCM auth failed) for kek_id `{kek_id}`")]
    TamperedEnvelope {
        /// The KEK identity the (failed-to-authenticate) record was sealed under.
        kek_id: KekId,
    },

    /// The persisted root-key envelope was opened with the **wrong** KEK — the
    /// supplied KEK's identity does not match the record's `kek_id`. AAD =
    /// `kek_id` binds the ciphertext to its KEK identity, so a KEK-confusion
    /// attempt is detected structurally and surfaces distinctly from
    /// [`TamperedEnvelope`](CaError::TamperedEnvelope) (ADR-0063 D3/D4).
    #[error(
        "root-key envelope sealed under kek_id `{sealed_under}` cannot be opened with kek_id `{supplied}`"
    )]
    WrongKek {
        /// The `kek_id` the record was sealed under (from the record).
        sealed_under: KekId,
        /// The `kek_id` of the KEK the caller supplied.
        supplied: KekId,
    },
}

impl CaError {
    /// Construct an [`InvalidSan`](CaError::InvalidSan) for a rejected SAN
    /// cardinality.
    #[must_use]
    pub const fn invalid_san(found: usize) -> Self {
        Self::InvalidSan { found }
    }

    /// Construct an [`InvalidSubject`](CaError::InvalidSubject).
    #[must_use]
    pub const fn invalid_subject(role: &'static str, reason: &'static str) -> Self {
        Self::InvalidSubject { role, reason }
    }

    /// Construct a [`SigningFailed`](CaError::SigningFailed).
    #[must_use]
    pub fn signing_failed(reason: impl Into<String>) -> Self {
        Self::SigningFailed { reason: reason.into() }
    }

    /// Construct a [`TamperedEnvelope`](CaError::TamperedEnvelope) for a record
    /// that failed AES-GCM authentication under its own KEK.
    #[must_use]
    pub const fn tampered_envelope(kek_id: KekId) -> Self {
        Self::TamperedEnvelope { kek_id }
    }

    /// Construct a [`WrongKek`](CaError::WrongKek) for an open attempted under
    /// a KEK whose identity differs from the record's `kek_id`.
    #[must_use]
    pub const fn wrong_kek(sealed_under: KekId, supplied: KekId) -> Self {
        Self::WrongKek { sealed_under, supplied }
    }
}

/// The built-in certificate-authority port.
///
/// A pure trait — no impl, no `rcgen`, no crypto backend (ADR-0063 D1). The
/// host adapter (`RcgenCa`) owns all crypto; the sim adapter (`SimCa`) loads
/// fixture keys and draws serials via the seeded `Entropy` port so issuance is
/// DST-deterministic. Both adapters honor the contracts pinned on each method
/// below — these rustdoc blocks are the SSOT the `ca_equivalence` DST test
/// (ADR-0063 D8) enforces.
pub trait Ca: Send + Sync {
    /// Generate or load the persistent self-signed root CA.
    ///
    /// # Preconditions
    /// The adapter holds (or can mint) a root signing key. The sim adapter
    /// loads a fixture P-256 key; the host adapter generates one via the
    /// crypto backend's CSPRNG on first boot and decrypts the persisted
    /// envelope on subsequent boots (ADR-0063 D3).
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`RootCaHandle`] is a self-signed P-256
    /// certificate with `basicConstraints` CA:TRUE, `keyUsage` =
    /// `keyCertSign` + `cRLSign` marked **critical**, and **no** pathLen
    /// constraint. The subject is the **trust domain only** — no path
    /// component (research Finding 2). The serial is drawn via the `Entropy`
    /// port.
    ///
    /// # Edge cases
    /// A signing-backend failure (malformed fixture key, decrypt failure)
    /// surfaces [`CaError::SigningFailed`] — never a panic, never a silent
    /// re-mint (a re-mint would orphan every issued identity, ADR-0063 D3).
    ///
    /// # Observable invariants
    /// Under DST (seeded `Entropy`), two adapters constructed over the same
    /// seed produce a **byte-identical** `RootCaHandle` — same cert PEM, same
    /// cert DER, same serial (KPI K5). This is the determinism contract the
    /// `sim_ca_root_is_bit_identical_across_two_runs_at_same_seed` acceptance
    /// scenario pins.
    fn root(&self) -> Result<RootCaHandle>;

    /// Issue (or re-issue) the node intermediate CA, signed by the root.
    ///
    /// # Preconditions
    /// [`root`](Ca::root) has succeeded (the intermediate is signed by the
    /// root key). Single-node (Phase 2.6): one `node` → one intermediate.
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`IntermediateHandle`] is `CA:TRUE` with
    /// `basicConstraints` **pathLen=0** (it may sign leaves only — no further
    /// intermediates), `keyUsage` = `keyCertSign` marked critical, and is
    /// signed by the root (chains to the root anchor). The serial is drawn via
    /// the `Entropy` port.
    ///
    /// # Edge cases
    /// A signing failure surfaces [`CaError::SigningFailed`]. An invalid
    /// `node` subject (path component where the trust domain alone is
    /// required) surfaces [`CaError::InvalidSubject`].
    ///
    /// # Observable invariants
    /// Under DST the intermediate is deterministic across two same-seed runs
    /// (same material, same serial) and always chains to the fixture root.
    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle>;

    /// Mint a workload SVID leaf.
    ///
    /// # Preconditions
    /// The `req`'s [`SpiffeId`] projects to **exactly one** `spiffe://` URI
    /// SAN. **Zero or two-or-more URI SANs is rejected with
    /// [`CaError::InvalidSan`] before any certificate is produced** — the
    /// SPIFFE spec's hardest rule and the highest-value invariant in the
    /// feature (KPI K2, research Finding 2). The intermediate
    /// ([`issue_intermediate`](Ca::issue_intermediate)) exists and signs the
    /// leaf.
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`SvidMaterial`] is `CA:FALSE` with `keyUsage` =
    /// `digitalSignature` marked **critical**, carries **exactly one** URI SAN
    /// equal to `req.spiffe_id()`, and a CSPRNG serial drawn via the `Entropy`
    /// port (≥64 bits, CA/B Forum floor — research Finding 10).
    ///
    /// # Edge cases
    /// **Re-issue is not cached**: calling `issue_svid` twice for the *same*
    /// [`SpiffeId`] yields a **fresh** certificate each time — a distinct
    /// serial and a new validity window (the re-issue mechanism the #40
    /// rotation workflow drives). A signing failure surfaces
    /// [`CaError::SigningFailed`].
    ///
    /// # Observable invariants
    /// Determinism is **per-call-sequence**, not per-`SpiffeId`-cached: under
    /// the same seed the same call sequence yields the same serials, but two
    /// calls within one sequence draw distinct serials.
    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial>;

    /// Compose the trust bundle a relying party verifies an SVID against.
    ///
    /// # Preconditions
    /// [`root`](Ca::root) has succeeded. The intermediate may or may not exist
    /// yet; the bundle always carries the root anchor.
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`TrustBundle`] carries the **root certificate as
    /// the trust anchor** and, when present, the **intermediate as untrusted
    /// chain material** (the verifier uses it to build the path but anchors
    /// trust only on the root). Composition order is root-anchor-first.
    ///
    /// # Edge cases
    /// A bundle requested before any root exists surfaces
    /// [`CaError::SigningFailed`] (the adapter cannot compose a bundle with no
    /// anchor). The bundle never contains a leaf SVID.
    ///
    /// # Observable invariants
    /// The bundle is deterministic for a given root/intermediate pair — same
    /// anchors compose to byte-identical bundle PEM.
    fn trust_bundle(&self) -> Result<TrustBundle>;
}
