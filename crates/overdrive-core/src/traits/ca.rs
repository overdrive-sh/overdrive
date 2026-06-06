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
/// Composed by [`Ca::trust_bundle`] (ADR-0063 D1 wire-format): the root
/// certificate is the **trust anchor**; the intermediate, when present, is
/// **untrusted chain material** the verifier uses to build the path but does
/// not itself anchor trust on.
///
/// The two components are observable **separately** — [`root_anchor`] and
/// [`intermediate_chain`] — so a relying party (or an adapter-equivalence
/// test) can inspect the composition *shape* (anchor present, chain present /
/// absent, order) without re-parsing the concatenated PEM. The combined
/// **root-anchor-first** PEM (`<root>\n<intermediate>`) is exposed via
/// [`bundle_pem`] for the relying-party `verify` path — `openssl verify
/// -CAfile <bundle.pem> <leaf.pem>` builds the chain from a single file
/// because the anchor and chain material are concatenated anchor-first.
///
/// [`root_anchor`]: TrustBundle::root_anchor
/// [`intermediate_chain`]: TrustBundle::intermediate_chain
/// [`bundle_pem`]: TrustBundle::bundle_pem
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustBundle {
    root_anchor: CaCertPem,
    intermediate_chain: Option<CaCertPem>,
}

impl TrustBundle {
    /// Build a trust bundle from its composed parts: the **root anchor**
    /// (always present) and, when an intermediate has been issued, the
    /// **intermediate as untrusted chain material**.
    #[must_use]
    pub const fn new(root_anchor: CaCertPem, intermediate_chain: Option<CaCertPem>) -> Self {
        Self { root_anchor, intermediate_chain }
    }

    /// The root certificate — the **trust anchor** the relying party pins.
    /// Always present; composition order is root-anchor-first.
    #[must_use]
    pub const fn root_anchor(&self) -> &CaCertPem {
        &self.root_anchor
    }

    /// The intermediate certificate as **untrusted chain material**, when one
    /// has been issued — the verifier uses it to build the path but does not
    /// anchor trust on it. `None` when only the root exists.
    #[must_use]
    pub const fn intermediate_chain(&self) -> Option<&CaCertPem> {
        self.intermediate_chain.as_ref()
    }

    /// The combined bundle PEM: the root anchor first, the intermediate chain
    /// material appended when present (`<root>\n<intermediate>`).
    ///
    /// This is the single-file relying-party verification material — `openssl
    /// verify -CAfile <bundle.pem> <leaf.pem>` builds `root → intermediate →
    /// leaf` from the one concatenated file because the anchor and chain
    /// material are present anchor-first.
    #[must_use]
    pub fn bundle_pem(&self) -> TrustBundlePem {
        let mut pem = self.root_anchor.as_pem().to_owned();
        if let Some(chain) = self.intermediate_chain.as_ref() {
            if !pem.ends_with('\n') {
                pem.push('\n');
            }
            pem.push_str(chain.as_pem());
        }
        TrustBundlePem::new(pem)
    }
}

/// A certificate-authority operation failure.
///
/// Distinct per failure mode (`.claude/rules/development.md` § "Distinct
/// failure modes get distinct error variants"). The pure-policy
/// [`CertSpecError`] passes through via `#[from]` (the [`Policy`](CaError::Policy)
/// variant) so a policy rejection keeps its structured shape across the adapter
/// boundary — both adapters surface a `CertSpec` rejection identically. There is
/// no dedicated SAN-cardinality variant: under Option A (ADR-0063 D5 amendment)
/// the single-URI-SAN invariant is honored by the [`SvidRequest`] type itself,
/// so a bad-cardinality request is unrepresentable at the [`Ca::issue_svid`]
/// boundary; the one fallible SAN-cardinality parse is the pure-core
/// [`CertSpec::svid`](crate::ca::CertSpec::svid) policy, which surfaces
/// [`CertSpecError::InvalidSan`] through the `Policy` pass-through.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaError {
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
    /// crypto backend's CSPRNG on first boot. On subsequent boots the boot
    /// path (`ca_boot`) decrypts the persisted envelope under the KEK and
    /// re-seeds the adapter via [`adopt_persisted_root`](Ca::adopt_persisted_root)
    /// BEFORE any issuance; `root()` then returns that adopted material rather
    /// than lazily minting a fresh (ephemeral) root (ADR-0063 D3).
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
    /// The intermediate ([`issue_intermediate`](Ca::issue_intermediate)) exists
    /// and signs the leaf. The single-URI-SAN invariant is **honored by
    /// construction**, not by a runtime cardinality guard: a [`SvidRequest`]
    /// holds exactly one validated [`SpiffeId`], so a request projecting to
    /// zero or two-or-more `spiffe://` URI SANs is *unrepresentable* at this
    /// boundary — there is no `CaError::InvalidSan` branch in `issue_svid` to
    /// reach. The single fallible parse of raw SAN cardinality is the pure-core
    /// [`CertSpec::svid`](crate::ca::CertSpec::svid) policy, which rejects 0 or
    /// ≥2 with [`CertSpecError`] (ADR-0063 D5; SPIFFE X.509-SVID §2). The
    /// SPIFFE-spec-mandated *runtime* reject (§5.2) lives at the relying-party
    /// verifier (#26 sockops/kTLS mTLS), not at this issuer.
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`SvidMaterial`] is `CA:FALSE` with `keyUsage` =
    /// `digitalSignature` marked **critical**, carries **exactly one** URI SAN
    /// equal to `req.spiffe_id()`, NO `keyCertSign`/`cRLSign`, and a CSPRNG
    /// serial drawn via the `Entropy` port (≥64 bits, CA/B Forum floor —
    /// research Finding 10). The single URI SAN is a structural consequence of
    /// the single-identity request, not a checked-then-asserted property.
    ///
    /// # Edge cases
    /// There is **no bad-SAN-cardinality edge case at this method** — the
    /// request type forecloses it (see Preconditions). **Re-issue is not
    /// cached**: calling `issue_svid` twice for the *same* [`SpiffeId`] yields a
    /// **fresh** certificate each time — a distinct serial and a new validity
    /// window (the re-issue mechanism the #40 rotation workflow drives). A
    /// signing-backend failure surfaces [`CaError::SigningFailed`]; an issuance
    /// whose audit row cannot be written surfaces a [`CaError`] rather than
    /// handing out an unaudited certificate (no silent issuance; ADR-0063 D6).
    ///
    /// # Observable invariants
    /// Every minted [`SvidMaterial`] carries exactly one URI SAN equal to
    /// `req.spiffe_id()` and is `CA:FALSE` — across both adapters, this is the
    /// equivalence the `ca_equivalence` DST test pins via S-04-06 (the
    /// SVID-profile equivalence, SAN cardinality included). Determinism is
    /// **per-call-sequence**, not per-`SpiffeId`-cached: under the same seed the
    /// same call sequence yields the same serials, but two calls within one
    /// sequence draw distinct serials.
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

    /// Re-seed the adapter with the persisted root after a restart.
    ///
    /// # Why this exists
    /// An adapter that *lazily generates* its root (the host `RcgenCa`) holds
    /// the root signing key only in memory. After a control-plane restart a
    /// fresh adapter's cache is empty; the boot path (`ca_boot`) decrypts the
    /// persisted root under the KEK, but unless that material is fed back into
    /// the adapter the adapter's first signing call mints a BRAND-NEW
    /// (ephemeral) root — and every certificate signed under it
    /// ([`issue_intermediate`](Ca::issue_intermediate),
    /// [`issue_svid`](Ca::issue_svid), [`trust_bundle`](Ca::trust_bundle))
    /// fails to chain to the persisted anchor relying parties pin. This method
    /// is the seam that closes that chain-break: the boot path calls it to
    /// install the persisted root into the adapter before any issuance.
    ///
    /// # Preconditions
    /// `root` is the **byte-identical persisted root**: `signing_key()` is the
    /// decrypted root private key (PEM), and the cert PEM / DER / serial are the
    /// persisted public material. The boot path adopts exactly once, on a fresh
    /// adapter, BEFORE any issuance.
    ///
    /// # Postconditions
    /// On `Ok`, every subsequent [`root`](Ca::root) returns the adopted handle,
    /// and [`issue_intermediate`](Ca::issue_intermediate) /
    /// [`issue_svid`](Ca::issue_svid) / [`trust_bundle`](Ca::trust_bundle) sign
    /// under the adopted root key.
    ///
    /// # Edge cases / idempotency / ordering
    /// Adoption is idempotent for the SAME root: adopting the byte-identical
    /// root a second time is a no-op `Ok(())`. Adopting AFTER the adapter has
    /// already minted a *different* root is a logic error (issuance ran before
    /// adoption — the ephemeral-root chain-break has already occurred) and MUST
    /// fail loud with a typed [`CaError`], never silently retain the ephemeral
    /// root.
    ///
    /// # Default
    /// A no-op `Ok(())` — correct for adapters whose root is **stable by
    /// construction** across instances. The sim adapter (`SimCa`) loads a
    /// fixture `const` root identical on every boot, so there is no ephemeral
    /// divergence to repair and the default is sound. Adapters that *lazily
    /// generate* a root (the host `RcgenCa`) MUST override this.
    fn adopt_persisted_root(&self, root: &RootCaHandle) -> Result<()> {
        let _ = root;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CaCertDer, CaCertPem, CaKeyPem, TrustBundle, TrustBundlePem};

    /// The PEM/key/bundle string byte-newtypes round-trip raw text through
    /// their `as_pem` accessor. Newtype-completeness mandate
    /// (`.claude/rules/development.md` § "Newtype completeness"): the accessor
    /// returns the wrapped text verbatim, so a mutation replacing the body
    /// with `""` or a stub literal is killed by a non-trivial multi-line PEM.
    #[test]
    fn pem_string_newtypes_borrow_their_wrapped_text_verbatim() {
        let cert_text =
            "-----BEGIN CERTIFICATE-----\nMIIBcert\n-----END CERTIFICATE-----\n".to_owned();
        let key_text =
            "-----BEGIN PRIVATE KEY-----\nMIIBkey\n-----END PRIVATE KEY-----\n".to_owned();
        let bundle_text = format!("{cert_text}{cert_text}");

        assert_eq!(CaCertPem::new(cert_text.clone()).as_pem(), cert_text);
        assert_eq!(CaKeyPem::new(key_text.clone()).as_pem(), key_text);
        assert_eq!(TrustBundlePem::new(bundle_text.clone()).as_pem(), bundle_text);
    }

    /// `CaCertDer::as_der` borrows the wrapped DER bytes verbatim. The value is
    /// a multi-byte non-trivial slice so the `vec![0]` / `vec![1]` / leaked-vec
    /// mutations on the accessor body all fail (each would return a slice that
    /// is not equal to the input).
    #[test]
    fn cert_der_newtype_borrows_its_wrapped_bytes_verbatim() {
        let der = vec![0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(CaCertDer::new(der.clone()).as_der(), der.as_slice());
    }

    /// `TrustBundle::intermediate_chain` exposes the intermediate when present
    /// (`Some(&intermediate)`) and `None` when the bundle is root-only. The
    /// Some-case kills the `-> None` mutant on the accessor; the None-case pins
    /// the root-only branch.
    #[test]
    fn intermediate_chain_reflects_whether_an_intermediate_is_present() {
        let root =
            CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into());
        let intermediate = CaCertPem::new(
            "-----BEGIN CERTIFICATE-----\nINTERMEDIATE\n-----END CERTIFICATE-----\n".into(),
        );

        let with_intermediate = TrustBundle::new(root.clone(), Some(intermediate.clone()));
        assert_eq!(with_intermediate.intermediate_chain(), Some(&intermediate));

        let root_only = TrustBundle::new(root, None);
        assert_eq!(root_only.intermediate_chain(), None);
    }

    /// `TrustBundle::bundle_pem` composes the root anchor first, the
    /// intermediate chain material appended (`<root>\n<intermediate>`), with a
    /// single separating newline inserted only when the root does not already
    /// end in one (ADR-0063 D1 root-anchor-first wire-format).
    ///
    /// The root anchor here deliberately does NOT end in `\n`, so the
    /// `if !pem.ends_with('\n')` branch fires and inserts the separator. Deleting
    /// the `!` flips the branch — the separator is skipped — and the composed
    /// output becomes `<root><intermediate>` with no newline between the two
    /// PEM blocks, which is not equal to the asserted bytes. The mutant dies
    /// deterministically (no timeout, no L3 path).
    #[test]
    fn bundle_pem_composes_root_anchor_first_with_intermediate_separator() {
        // No trailing newline on the root — forces the separator-insertion branch.
        let root =
            CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----".into());
        let intermediate = CaCertPem::new(
            "-----BEGIN CERTIFICATE-----\nINTERMEDIATE\n-----END CERTIFICATE-----\n".into(),
        );

        let with_intermediate = TrustBundle::new(root.clone(), Some(intermediate.clone()));
        let expected = format!("{}\n{}", root.as_pem(), intermediate.as_pem());
        assert_eq!(with_intermediate.bundle_pem().as_pem(), expected);

        // Root-only: the composed PEM is exactly the root anchor, with no
        // phantom intermediate material appended.
        let root_only = TrustBundle::new(root.clone(), None);
        assert_eq!(root_only.bundle_pem().as_pem(), root.as_pem());
    }
}
