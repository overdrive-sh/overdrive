//! `RcgenCa` — the production [`Ca`] host adapter (ADR-0063 D1).
//!
//! `RcgenCa` owns ALL `rcgen` / crypto-backend usage in the platform: it
//! translates the pure [`CertSpec`] policy (from `overdrive-core`, step 01-01)
//! into `rcgen::CertificateParams` and self-signs a P-256 root via the **ring**
//! backend. The pure policy stays in core (dst-lint keeps `rcgen` / `ring` off
//! the core compile path); this adapter is where the decision becomes real
//! X.509 bytes.
//!
//! # Dependency discipline
//!
//! `RcgenCa::new` takes its [`Entropy`] source AND its trust-domain subject as
//! **required constructor parameters** — no builder, no production-binding
//! default (`.claude/rules/development.md` § "Port-trait dependencies"). A
//! caller that forgets to inject entropy fails to compile. The serial is drawn
//! through the injected [`Entropy`] port, matching the trait contract (the same
//! port `SimCa` draws from), so issuance is genuinely entropy-sourced rather
//! than rcgen-default.

use std::sync::{Arc, OnceLock};

use std::time::Duration;

use overdrive_core::traits::ca::{Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, RootCaHandle};
use overdrive_core::traits::ca::{IntermediateHandle, SvidMaterial, SvidRequest, TrustBundle};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::{CertSerial, CertSpec, CertSpecError, KeyUsage, NodeId, SpiffeId};
use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType, SerialNumber, date_time_ymd,
};

/// Number of random bytes drawn for a certificate serial — 128 bits, well
/// above the CA/B Forum 64-bit floor (research Finding 10). Matches `SimCa`'s
/// draw width so the entropy contract is identical across adapters.
const SERIAL_BYTES: usize = 16;

/// SVID leaf validity window — ~1 hour (research Finding 6). Short-lived
/// workload identities keep the node-compromise / rotation blast radius small;
/// the #40 rotation workflow re-issues before expiry. The leaf's `not_after` is
/// `not_before + WORKLOAD_SVID_TTL`.
const WORKLOAD_SVID_TTL: Duration = Duration::from_secs(3600);

/// Clock-skew back-off applied to the SVID `not_before`. A freshly-minted leaf
/// must verify under a relying party whose clock is marginally behind the
/// issuer's; backing `not_before` off by this margin avoids a spurious
/// "certificate is not yet valid" rejection at the verify boundary.
const SKEW_TOLERANCE: Duration = Duration::from_secs(60);

/// The canonical node identity whose intermediate signs workload SVIDs when no
/// intermediate has been explicitly issued yet. Single-node (Phase 2.6) has
/// exactly one node beneath the root; `issue_svid` reuses the cached
/// intermediate when the caller has pre-issued one, and otherwise mints the
/// intermediate for this canonical node.
const DEFAULT_SVID_NODE: &str = "node-local";

/// The persistent root material, minted once and cached.
///
/// `KeyPair::generate()` draws fresh OS randomness on every call, so two
/// `root()` invocations would otherwise produce *different* root keys — and an
/// intermediate signed by a throwaway root would not chain to the root the
/// caller observed. Caching the root key PEM (the one serialisable carrier of
/// the signing capability) makes `root()` idempotent and lets
/// `issue_intermediate` rebuild the *same* [`Issuer`] the root was signed under,
/// so `openssl verify -CAfile root.pem intermediate.pem` succeeds (S-03 / KPI
/// K1). The cert PEM/DER and serial are cached alongside so repeated `root()`
/// calls return a stable handle.
struct RootMaterial {
    key_pem: String,
    cert_pem: String,
    cert_der: Vec<u8>,
    serial: CertSerial,
}

/// The persistent node-intermediate material, minted once and cached.
///
/// Like [`RootMaterial`], `issue_intermediate` would otherwise mint a *fresh*
/// intermediate key on every call (`KeyPair::generate` draws new randomness),
/// so a leaf signed by `issue_svid`'s intermediate would NOT chain to the
/// intermediate a prior `issue_intermediate` call returned. Caching the
/// intermediate (single-node, Phase 2.6: one node → one intermediate) makes
/// `issue_intermediate` idempotent and lets `issue_svid` reuse the SAME
/// intermediate the caller wrote to `intermediate.pem` — so the full
/// `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem`
/// chain verifies (S-04-07 / KPI K1). The `node` is retained so the rebuilt
/// issuer params carry the same per-node `CommonName` the cert was signed under.
struct IntermediateMaterial {
    node: NodeId,
    key_pem: String,
    cert_pem: String,
    cert_der: Vec<u8>,
    serial: CertSerial,
}

/// The production [`Ca`] host adapter.
///
/// Holds the injected [`Entropy`] source and the trust-domain subject the root
/// is minted for, plus [`OnceLock`]s caching the persistent root and
/// node-intermediate material so the signing keys are generated once and
/// reused — the root for intermediate signing, the intermediate for leaf
/// signing. `Send + Sync` (the `Arc<dyn Entropy>` is `Send + Sync`, `OnceLock`
/// is `Sync`), so it can be shared across async tasks in the composition root.
pub struct RcgenCa {
    entropy: Arc<dyn Entropy>,
    subject: SpiffeId,
    root_material: OnceLock<RootMaterial>,
    intermediate_material: OnceLock<IntermediateMaterial>,
}

impl RcgenCa {
    /// Construct an `RcgenCa` over a required [`Entropy`] source and the
    /// trust-domain `subject` the root certificate is minted for.
    ///
    /// No builder, no default — both dependencies are mandatory at construction
    /// so a caller that forgets either fails to compile (`.claude/rules/
    /// development.md` § "Port-trait dependencies").
    #[must_use]
    pub fn new(entropy: Arc<dyn Entropy>, subject: SpiffeId) -> Self {
        Self {
            entropy,
            subject,
            root_material: OnceLock::new(),
            intermediate_material: OnceLock::new(),
        }
    }

    /// Build the unsigned root [`CertificateParams`] for this trust domain.
    ///
    /// The single derivation site for the root profile, shared by `root()` (to
    /// self-sign) and `issue_intermediate` (to rebuild the [`Issuer`] the
    /// intermediate is signed under, so the issuer DN / key-usages stamped on
    /// the intermediate match the root). Every X.509 property is derived from
    /// the pure [`CertSpec::root`] policy rather than hardcoded.
    fn root_params(&self, serial_bytes: &[u8; SERIAL_BYTES]) -> Result<CertificateParams, CaError> {
        let spec = CertSpec::root(self.subject.clone());

        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|source| CaError::signing_failed(format!("root params: {source}")))?;

        params.is_ca = match spec.path_len() {
            None if spec.is_ca() => IsCa::Ca(BasicConstraints::Unconstrained),
            Some(path_len) => IsCa::Ca(BasicConstraints::Constrained(path_len)),
            None => IsCa::NoCa,
        };
        params.key_usages = spec.key_usages().into_iter().map(to_rcgen_key_usage).collect();
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::OrganizationName, spec.subject().trust_domain());
            dn
        };
        params.serial_number = Some(SerialNumber::from_slice(serial_bytes));
        Ok(params)
    }

    /// Mint the persistent root once, returning the cached material on every
    /// subsequent call.
    ///
    /// The root key is generated exactly once (the first call) and cached as
    /// PEM; the matching cert PEM/DER and serial are cached alongside so `root()`
    /// is idempotent and `issue_intermediate` signs against the *same* root.
    fn root_material(&self) -> Result<&RootMaterial, CaError> {
        if let Some(material) = self.root_material.get() {
            return Ok(material);
        }

        let key = KeyPair::generate().map_err(|source| {
            CaError::signing_failed(format!("root keypair generation: {source}"))
        })?;
        let (serial_bytes, serial) = self.draw_serial();
        let params = self.root_params(&serial_bytes)?;
        let cert = params
            .self_signed(&key)
            .map_err(|source| CaError::signing_failed(format!("root self-sign: {source}")))?;

        let material = RootMaterial {
            key_pem: key.serialize_pem(),
            cert_pem: cert.pem(),
            cert_der: cert.der().to_vec(),
            serial,
        };
        // `set` only fails on a lost race; the winning value is equivalent
        // (same trust domain), so reading back the stored material is correct.
        let _ = self.root_material.set(material);
        Ok(self
            .root_material
            .get()
            .unwrap_or_else(|| unreachable!("root_material is populated immediately after set")))
    }

    /// Build the unsigned node-intermediate [`CertificateParams`].
    ///
    /// The single derivation site for the intermediate profile, shared by
    /// `issue_intermediate` (to self-issue the intermediate) and `issue_svid`
    /// (to rebuild the [`Issuer`] the leaf is signed under, so the issuer DN
    /// stamped on the leaf matches the intermediate's subject — the
    /// chains-to-intermediate linkage). Every X.509 property is derived from the
    /// pure [`CertSpec::intermediate`] policy: CA:TRUE, pathLen=0, keyCertSign
    /// critical, with a per-node `CommonName` so the intermediate's subject DN
    /// differs from the root's (otherwise `openssl verify` treats the
    /// intermediate as self-signed and refuses to build the chain).
    fn intermediate_params(
        &self,
        node: &NodeId,
        serial_bytes: &[u8; SERIAL_BYTES],
    ) -> Result<CertificateParams, CaError> {
        let spec = CertSpec::intermediate(self.subject.clone());

        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|source| CaError::signing_failed(format!("intermediate params: {source}")))?;

        params.is_ca = match spec.path_len() {
            Some(path_len) => IsCa::Ca(BasicConstraints::Constrained(path_len)),
            None if spec.is_ca() => IsCa::Ca(BasicConstraints::Unconstrained),
            None => IsCa::NoCa,
        };
        params.key_usages = spec.key_usages().into_iter().map(to_rcgen_key_usage).collect();
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::OrganizationName, spec.subject().trust_domain());
            dn.push(DnType::CommonName, node.as_str());
            dn
        };
        params.serial_number = Some(SerialNumber::from_slice(serial_bytes));
        Ok(params)
    }

    /// Mint the persistent node intermediate once, returning the cached material
    /// on every subsequent call.
    ///
    /// Single-node (Phase 2.6): the first `node` to request an intermediate wins
    /// the cache; later calls (and `issue_svid`'s internal use) return that same
    /// material so the intermediate the caller writes to `intermediate.pem` is
    /// the SAME one the leaf was signed under — the chain verifies. The
    /// intermediate is signed by the cached root key (idempotent root).
    fn intermediate_material(&self, node: &NodeId) -> Result<&IntermediateMaterial, CaError> {
        if let Some(material) = self.intermediate_material.get() {
            return Ok(material);
        }

        // Precondition: the persistent root exists. Rebuild the root issuer so
        // the intermediate's issuer field matches the root's subject.
        let root = self.root_material()?;
        let root_key = KeyPair::from_pem(&root.key_pem).map_err(|source| {
            CaError::signing_failed(format!("root key reload for issuer: {source}"))
        })?;
        let root_params = self.root_params(&[0u8; SERIAL_BYTES])?;
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&root_params, &root_key);

        let inter_key = KeyPair::generate().map_err(|source| {
            CaError::signing_failed(format!("intermediate keypair generation: {source}"))
        })?;
        let (serial_bytes, serial) = self.draw_serial();
        let params = self.intermediate_params(node, &serial_bytes)?;
        let cert = params.signed_by(&inter_key, &issuer).map_err(|source| {
            CaError::signing_failed(format!("intermediate sign by root: {source}"))
        })?;

        let material = IntermediateMaterial {
            node: node.clone(),
            key_pem: inter_key.serialize_pem(),
            cert_pem: cert.pem(),
            cert_der: cert.der().to_vec(),
            serial,
        };
        // `set` only fails on a lost race; the winning value chains to the same
        // root, so reading back the stored material is correct.
        let _ = self.intermediate_material.set(material);
        Ok(self.intermediate_material.get().unwrap_or_else(|| {
            unreachable!("intermediate_material is populated immediately after set")
        }))
    }

    /// The node intermediate that signs workload SVIDs.
    ///
    /// Returns the cached single-node intermediate (Phase 2.6: one node → one
    /// intermediate) so a leaf chains to the SAME intermediate a prior
    /// [`issue_intermediate`](Ca::issue_intermediate) returned. When no
    /// intermediate has been issued yet, one is minted for the canonical default
    /// node — `issue_svid` does not require the caller to pre-issue the
    /// intermediate, but reuses the cached one when present.
    fn svid_intermediate(&self) -> Result<&IntermediateMaterial, CaError> {
        let default_node = NodeId::new(DEFAULT_SVID_NODE)
            .unwrap_or_else(|_| unreachable!("DEFAULT_SVID_NODE is a valid NodeId literal"));
        self.intermediate_material(&default_node)
    }

    /// Project the [`SvidRequest`] to the set of `spiffe://` URI SANs the leaf
    /// would carry.
    ///
    /// A workload SVID carries the requested identity as its SOLE URI SAN, so
    /// the projection is the singleton `[req.spiffe_id()]`. The single-URI-SAN
    /// cardinality DECISION is NOT made here — it is delegated to the pure core
    /// [`CertSpec::svid`] guard (ADR-0063 D5); this method only assembles the
    /// projection the guard validates.
    fn project_sans(req: &SvidRequest) -> Vec<SpiffeId> {
        vec![req.spiffe_id().clone()]
    }

    /// Map a [`CertSpecError`] from the core SVID policy to the dedicated
    /// [`CaError`] variant the trait contract names.
    ///
    /// A SAN-cardinality rejection ([`CertSpecError::InvalidSan`]) surfaces as
    /// the dedicated [`CaError::InvalidSan`] (carrying the offending count) —
    /// NOT flattened into a `Policy` pass-through — so the load-bearing
    /// single-URI signal (KPI K2) keeps its structured cardinality across the
    /// adapter boundary (S-04-09). Any other policy rejection passes through as
    /// [`CaError::Policy`] via `#[from]`.
    fn map_svid_policy_error(error: CertSpecError) -> CaError {
        match error {
            CertSpecError::InvalidSan { found } => CaError::invalid_san(found),
            other @ CertSpecError::InvalidSubject { .. } => CaError::from(other),
        }
    }

    /// Seconds of wall-clock elapsed since the Unix epoch, read at the signing
    /// boundary.
    ///
    /// The host adapter injects no `Clock`, so wall-clock is read here (the
    /// production I/O boundary — an `adapter-host` crate, not core). The caller
    /// composes `date_time_ymd(1970, 1, 1) + Duration::from_secs(elapsed)` —
    /// using the only `OffsetDateTime` constructor rcgen re-exports plus
    /// `std::time::Duration` — so no direct dependency on the `time` crate's
    /// own constructors is needed. A pre-1970 system clock is structurally
    /// impossible on the platform; `duration_since` only errors before the
    /// epoch, hence the `unreachable!`.
    fn seconds_since_epoch() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| unreachable!("system clock is at or after the Unix epoch"))
            .as_secs()
    }

    /// Draw `SERIAL_BYTES` random bytes from the injected entropy source.
    ///
    /// Returns the raw bytes (for the rcgen `SerialNumber`) paired with their
    /// lowercase-hex [`CertSerial`] rendering (for the [`RootCaHandle`]). Two
    /// `RcgenCa` over the same seeded entropy draw identical bytes — the
    /// contract's determinism dependency, matching `SimCa`.
    fn draw_serial(&self) -> ([u8; SERIAL_BYTES], CertSerial) {
        let mut bytes = [0u8; SERIAL_BYTES];
        self.entropy.fill(&mut bytes);
        let hex = bytes.iter().fold(String::with_capacity(SERIAL_BYTES * 2), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        });
        let serial = CertSerial::new(&hex).unwrap_or_else(|_| {
            unreachable!("{SERIAL_BYTES}-byte lowercase hex is a valid CertSerial")
        });
        (bytes, serial)
    }
}

/// Translate a core [`KeyUsage`] into its `rcgen` counterpart.
///
/// The pure policy owns the vocabulary; this is the single adapter-side mapping
/// from the project enum to the crypto backend's enum, so the host adapter
/// never hardcodes the key-usage set — it derives it from [`CertSpec`].
const fn to_rcgen_key_usage(usage: KeyUsage) -> KeyUsagePurpose {
    match usage {
        KeyUsage::KeyCertSign => KeyUsagePurpose::KeyCertSign,
        KeyUsage::CrlSign => KeyUsagePurpose::CrlSign,
        KeyUsage::DigitalSignature => KeyUsagePurpose::DigitalSignature,
    }
}

impl Ca for RcgenCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        // The persistent root is minted once and cached (`root_material`); every
        // X.509 property is derived from the pure `CertSpec::root` policy in
        // `root_params` rather than hardcoded — host and sim share the one
        // policy surface (criterion 3 / ADR-0063 D8). Caching makes the root key
        // stable so `issue_intermediate` signs against the SAME root the caller
        // observes here.
        let material = self.root_material()?;
        Ok(RootCaHandle::new(
            CaCertPem::new(material.cert_pem.clone()),
            CaCertDer::new(material.cert_der.clone()),
            material.serial.clone(),
            CaKeyPem::new(material.key_pem.clone()),
        ))
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        // The persistent node intermediate is minted once and cached
        // (`intermediate_material`); every X.509 property is derived from the
        // pure `CertSpec::intermediate` policy in `intermediate_params` rather
        // than hardcoded — CA:TRUE, pathLen=0, keyCertSign critical, with a
        // per-node CommonName so the subject DN differs from the root's.
        // Caching makes the intermediate key stable so `issue_svid` signs leaves
        // against the SAME intermediate the caller observes here, which is what
        // makes the full root -> intermediate -> leaf chain verify (S-04-07 /
        // KPI K1).
        let material = self.intermediate_material(node)?;
        Ok(IntermediateHandle::new(
            CaCertPem::new(material.cert_pem.clone()),
            CaCertDer::new(material.cert_der.clone()),
            material.serial.clone(),
            CaKeyPem::new(material.key_pem.clone()),
        ))
    }

    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        // The pure DECISION FIRST, before any certificate material is produced:
        // the single-URI-SAN cardinality + CA:FALSE leaf-profile policy lives in
        // core `CertSpec::svid` (ADR-0063 D5 reconciliation B) — the host does
        // NOT fork the SAN rule. The SAN projection is derived from the request;
        // a projection that is not exactly one URI SAN is rejected by the policy
        // BEFORE the intermediate is even consulted, so no cert bytes escape on
        // the reject path. `CertSpecError::InvalidSan` maps to the dedicated
        // `CaError::InvalidSan` variant (NOT a flattened `Policy` pass-through)
        // so the load-bearing single-URI signal (KPI K2) keeps its structured
        // cardinality across the adapter boundary (S-04-09).
        let spec = CertSpec::svid(Self::project_sans(req)).map_err(Self::map_svid_policy_error)?;

        // Precondition: the persistent root + a node intermediate exist. The
        // leaf is signed by the INTERMEDIATE (not the root), so the SVID chains
        // root -> intermediate -> leaf (S-04-07 / KPI K1). The cached
        // single-node intermediate is reused (or minted for the canonical node
        // if none has been issued yet), so the issuer stamped on the leaf
        // matches the intermediate the caller wrote to `intermediate.pem`, which
        // is what makes `openssl verify -CAfile root.pem -untrusted
        // intermediate.pem svid.pem` build the path.
        let intermediate = self.svid_intermediate()?;

        // Rebuild the intermediate signing key + a CA-shaped params object so
        // the rcgen `Issuer` carries the intermediate's DN and key-usages — the
        // issuer field stamped on the leaf then equals the intermediate's
        // subject, the chains-to-intermediate linkage the verifier follows.
        let issuer_key = KeyPair::from_pem(&intermediate.key_pem).map_err(|source| {
            CaError::signing_failed(format!("intermediate key reload for issuer: {source}"))
        })?;
        let issuer_params = self.intermediate_params(&intermediate.node, &[0u8; SERIAL_BYTES])?;
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&issuer_params, &issuer_key);

        // The leaf keypair: the workload's own signing key. Per research
        // Finding 5 the leaf's private key is NOT a CA-boundary output, so it is
        // generated here only to sign the cert and then dropped — `SvidMaterial`
        // carries no key.
        let leaf_key = KeyPair::generate().map_err(|source| {
            CaError::signing_failed(format!("svid leaf keypair generation: {source}"))
        })?;

        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|source| CaError::signing_failed(format!("svid params: {source}")))?;

        // basicConstraints: CA:FALSE — a leaf signs nothing. Derived from the
        // pure spec (`CertSpec::svid` is `CertRole::Svid`, `is_ca() == false`,
        // `path_len() == None`).
        params.is_ca = match spec.path_len() {
            Some(path_len) => IsCa::Ca(BasicConstraints::Constrained(path_len)),
            None if spec.is_ca() => IsCa::Ca(BasicConstraints::Unconstrained),
            None => IsCa::NoCa,
        };

        // keyUsage: digitalSignature only (NO keyCertSign / cRLSign), derived
        // from the spec. rcgen marks the extension critical when the set is
        // non-empty — S-04-08 asserts `.critical`.
        params.key_usages = spec.key_usages().into_iter().map(to_rcgen_key_usage).collect();

        // The sole URI SAN: the requested workload identity, in its canonical
        // form. The single-element projection is the cardinality the core policy
        // already validated; the spec's subject IS that sole URI SAN.
        let uri = Ia5String::try_from(spec.subject().as_str()).map_err(|source| {
            CaError::signing_failed(format!("svid URI SAN is not a valid IA5 string: {source}"))
        })?;
        params.subject_alt_names = vec![SanType::URI(uri)];

        // Subject DN: the workload's CommonName so the leaf's subject differs
        // from the intermediate's. The identity-bearing assertion is the URI
        // SAN (above); the DN is a human-readable label.
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, spec.subject().as_str());
            dn
        };

        // Validity: a ~1h window straddling now (research Finding 6). The host
        // adapter injects no `Clock`, so wall-clock `now` is read at the
        // signing boundary via `SystemTime::now()` (this IS the production I/O
        // boundary — an adapter, not core). `not_before` is backed off by
        // `SKEW_TOLERANCE` so the freshly-minted leaf verifies under a verifier
        // whose clock is marginally behind; `not_after = not_before +
        // WORKLOAD_SVID_TTL` keeps the window width exactly the TTL (S-04-08
        // asserts the width; S-04-07's `openssl verify` requires the leaf be
        // valid *now*).
        let now = date_time_ymd(1970, 1, 1) + Duration::from_secs(Self::seconds_since_epoch());
        params.not_before = now - SKEW_TOLERANCE;
        params.not_after = params.not_before + WORKLOAD_SVID_TTL;

        // Serial: drawn via the injected Entropy port (CSPRNG, >=64 bits) and
        // stamped on the leaf — genuinely entropy-sourced, not rcgen-default.
        let (serial_bytes, serial) = self.draw_serial();
        params.serial_number = Some(SerialNumber::from_slice(&serial_bytes));

        // Sign the leaf by the node intermediate (0.14 2-arg `signed_by`).
        let cert = params.signed_by(&leaf_key, &issuer).map_err(|source| {
            CaError::signing_failed(format!("svid sign by intermediate: {source}"))
        })?;

        Ok(SvidMaterial::new(
            CaCertPem::new(cert.pem()),
            CaCertDer::new(cert.der().to_vec()),
            serial,
            spec.subject().clone(),
        ))
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 03")]
    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        todo!("RED scaffold: RcgenCa::trust_bundle (root anchor; intermediate chain material)")
    }
}
