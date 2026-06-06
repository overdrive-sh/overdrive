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

use overdrive_core::ca::{SKEW_TOLERANCE, WORKLOAD_SVID_TTL};
use overdrive_core::traits::ca::{Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, RootCaHandle};
use overdrive_core::traits::ca::{IntermediateHandle, SvidMaterial, SvidRequest, TrustBundle};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::{CertSerial, CertSpec, KeyUsage, NodeId, SpiffeId};
use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType, SerialNumber, date_time_ymd,
};

/// Number of random bytes drawn for a certificate serial — 128 bits, well
/// above the CA/B Forum 64-bit floor (research Finding 10). Matches `SimCa`'s
/// draw width so the entropy contract is identical across adapters.
const SERIAL_BYTES: usize = 16;

// `WORKLOAD_SVID_TTL` (leaf validity width) and `SKEW_TOLERANCE` (the
// `not_before` back-off) are the SINGLE source of truth in
// `overdrive_core::ca` — see imports above. The control-plane `ca_issuance`
// auditor records the audit window from the SAME two constants, so the window
// the leaf is SIGNED with and the window the `issued_certificates` audit row
// RECORDS cannot drift (ADR-0063 D6).

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
        // On a lost race `set` returns Err and our `material` is dropped here.
        // The two racers generate DIFFERENT keys/serials, so the values are NOT
        // interchangeable — correctness comes solely from OnceLock: every caller
        // reads back the single winning material via `get()` below. Do not add a
        // "trust domains match, reuse either value" short-circuit; that would be
        // incorrect because the key bytes and serial differ between racers.
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

    /// Install `material` into `lock`, or — when a concurrent writer won the
    /// race — verify the winner is byte-identical (idempotent adoption) and fail
    /// loud with [`CaError::adoption_conflict`] otherwise.
    ///
    /// [`OnceLock::set`] returns `Err` ONLY on a lost race. The racer that won
    /// may be a concurrent `root_material()` / `intermediate_material()` caller,
    /// which mints a FRESH EPHEMERAL key/serial (`KeyPair::generate` draws new
    /// randomness) — NOT interchangeable with the persisted material we are
    /// adopting. Discarding the `set` error (`let _ = lock.set(material)`) would
    /// silently leave the lock holding the ephemeral material, so every later
    /// `issue_intermediate` / `issue_svid` / `trust_bundle` would sign under the
    /// WRONG anchor, orphaning every cert relying parties pinned. The lost-race
    /// branch therefore enforces the SAME divergence contract as the pre-check
    /// guard in the adoption callers, one race-window later: identical winner →
    /// idempotent `Ok(())`; divergent winner → `adoption_conflict`.
    fn set_or_verify_winner<T>(
        lock: &OnceLock<T>,
        material: T,
        cert_der: impl Fn(&T) -> &[u8],
        kind: &'static str,
    ) -> Result<(), CaError> {
        if let Err(rejected) = lock.set(material) {
            let winner = lock.get().unwrap_or_else(|| {
                unreachable!("OnceLock is populated immediately after a failed set")
            });
            if cert_der(winner) != cert_der(&rejected) {
                return Err(CaError::adoption_conflict(kind));
            }
        }
        Ok(())
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
        // the CA:FALSE leaf-profile policy lives in core `CertSpec::svid`
        // (ADR-0063 D5) — the host does NOT fork the SAN rule. Under Option A
        // (ADR-0063 D5 amendment) the SAN projection is the singleton derived
        // from the request's single validated `SpiffeId`, so the cardinality is
        // always exactly one and the `CertSpec::svid` cardinality reject is
        // unreachable here — the bad-cardinality case is foreclosed by the
        // `SvidRequest` type, not by an adapter guard. Any policy rejection
        // surfaces through `CaError`'s `#[from] CertSpecError` (the `Policy`
        // variant) via `?` — identical to `SimCa`, so both adapters map a
        // `CertSpec` rejection uniformly (no divergent error mapping).
        let spec = CertSpec::svid(Self::project_sans(req))?;

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

        // The leaf keypair: generated here (D7 — the crypto-backend CSPRNG via
        // rcgen), used to sign the cert, and per ADR-0063 D9 RETAINED and
        // RETURNED on `SvidMaterial` — NOT dropped. Custody is node-held: the
        // node agent that runs the TLS 1.3 handshake on the workload's behalf
        // (whitepaper §7) is the holder. Dropping it (the pre-D9 bug) orphaned
        // every issued SVID — the cert embedded `leaf_key`'s public half but no
        // entity held the matching private half, so the cert was unusable in any
        // mTLS handshake. Finding 5's "keys never leave the signer" applies to
        // the root/intermediate signing keys, not to this leaf credential.
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

        // Serialize the leaf private key to PKCS#8 "PRIVATE KEY" PEM and return
        // it on `SvidMaterial` (ADR-0063 D9 — node-held). The public half is the
        // one embedded in `cert` above, so `leaf_key` is the matching private
        // half the node agent feeds to rustls.
        Ok(SvidMaterial::new(
            CaCertPem::new(cert.pem()),
            CaCertDer::new(cert.der().to_vec()),
            serial,
            spec.subject().clone(),
            CaKeyPem::new(leaf_key.serialize_pem()),
        ))
    }

    fn adopt_persisted_root(&self, root: &RootCaHandle) -> Result<(), CaError> {
        // Re-seed the lazily-generated root cache with the persisted root the
        // boot path decrypted, BEFORE any signing call. Without this, a fresh
        // `RcgenCa` (empty `OnceLock`) mints a new ephemeral root on first
        // issuance and nothing chains to the persisted anchor (the chain-break
        // this closes). Mirrors `RootMaterial`'s chain-determinism rationale:
        // a stable root key is what makes `issue_intermediate` sign against the
        // SAME root relying parties pin.
        let material = RootMaterial {
            key_pem: root.signing_key().as_pem().to_string(),
            cert_pem: root.cert_pem().as_pem().to_string(),
            cert_der: root.cert_der().as_der().to_vec(),
            serial: root.serial().clone(),
        };

        // Fail-loud guard (contract: adoption after a DIVERGENT root was minted
        // means issuance ran before adoption — the ephemeral-root chain-break
        // already happened). First adoption wins; re-adopting the SAME root is
        // an idempotent no-op.
        if let Some(existing) = self.root_material.get() {
            if existing.cert_der == material.cert_der {
                return Ok(());
            }
            return Err(CaError::adoption_conflict("root"));
        }

        // `set` only fails on a lost race, and the winner may be an ephemeral
        // root minted by a concurrent `root_material()` caller — a DIFFERENT key
        // that is NOT interchangeable with the persisted root. Verify the winner
        // is byte-identical (idempotent adoption) or fail loud, enforcing the
        // same divergence contract as the pre-check guard above one race-window
        // later — never silently adopt the ephemeral racer's anchor.
        Self::set_or_verify_winner(&self.root_material, material, |m| &m.cert_der, "root")
    }

    fn adopt_persisted_intermediate(
        &self,
        node: &NodeId,
        intermediate: &IntermediateHandle,
    ) -> Result<(), CaError> {
        // Re-seed the lazily-generated intermediate cache with the persisted
        // intermediate the boot path decrypted, BEFORE any signing call. Without
        // this, a fresh `RcgenCa` (empty `OnceLock`) mints a new ephemeral
        // intermediate on first issuance and every SVID signed under the prior
        // boot's intermediate fails to chain to the refreshed trust bundle (the
        // chain-break this closes). The `node` is retained so the rebuilt issuer
        // params in `issue_svid` carry the SAME per-node `CommonName` the cert
        // was signed under — mirrors how `intermediate_material` caches it.
        let material = IntermediateMaterial {
            node: node.clone(),
            key_pem: intermediate.signing_key().as_pem().to_string(),
            cert_pem: intermediate.cert_pem().as_pem().to_string(),
            cert_der: intermediate.cert_der().as_der().to_vec(),
            serial: intermediate.serial().clone(),
        };

        // Fail-loud guard (contract: adoption after a DIVERGENT intermediate was
        // minted means issuance ran before adoption — the ephemeral-intermediate
        // chain-break already happened). First adoption wins; re-adopting the
        // SAME intermediate is an idempotent no-op.
        if let Some(existing) = self.intermediate_material.get() {
            if existing.cert_der == material.cert_der {
                return Ok(());
            }
            return Err(CaError::adoption_conflict("intermediate"));
        }

        // `set` only fails on a lost race, and the winner may be an ephemeral
        // intermediate minted by a concurrent `intermediate_material()` caller —
        // a DIFFERENT key that is NOT interchangeable with the persisted one.
        // Verify the winner is byte-identical (idempotent adoption) or fail loud,
        // enforcing the same divergence contract as the pre-check guard above one
        // race-window later — never silently adopt the ephemeral racer's anchor.
        Self::set_or_verify_winner(
            &self.intermediate_material,
            material,
            |m| &m.cert_der,
            "intermediate",
        )
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        // The trust anchor is the persistent root certificate (minted once,
        // cached) — the relying party pins it. The chain material is the cached
        // single-node intermediate, included as UNTRUSTED chain material so the
        // verifier can build root → intermediate → leaf from the one bundle but
        // anchors trust only on the root (ADR-0063 D1 wire-format). Both are
        // produced through the same cached material `root()` / `issue_svid`'s
        // intermediate observe, so a leaf this adapter minted verifies against
        // this adapter's own bundle. Composition order is root-anchor-first;
        // `TrustBundle::bundle_pem` concatenates the two for the single-file
        // `openssl verify -CAfile <bundle.pem> <leaf.pem>` path.
        let root = self.root_material()?;
        let intermediate = self.svid_intermediate()?;
        Ok(TrustBundle::new(
            CaCertPem::new(root.cert_pem.clone()),
            Some(CaCertPem::new(intermediate.cert_pem.clone())),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{IntermediateMaterial, RcgenCa, RootMaterial};
    use overdrive_core::traits::ca::CaError;
    use overdrive_core::{CertSerial, NodeId};
    use std::sync::OnceLock;

    fn serial() -> CertSerial {
        CertSerial::new("ab").unwrap_or_else(|_| unreachable!("`ab` is a valid CertSerial"))
    }

    fn root_with(cert_der: Vec<u8>) -> RootMaterial {
        RootMaterial {
            key_pem: "key".to_string(),
            cert_pem: "cert".to_string(),
            cert_der,
            serial: serial(),
        }
    }

    fn intermediate_with(cert_der: Vec<u8>) -> IntermediateMaterial {
        IntermediateMaterial {
            node: NodeId::new("node-a").unwrap_or_else(|_| unreachable!("valid NodeId literal")),
            key_pem: "key".to_string(),
            cert_pem: "cert".to_string(),
            cert_der,
            serial: serial(),
        }
    }

    // ---------------------------------------------------------------------
    // Branch 1 — empty lock installs the material and returns Ok.
    // ---------------------------------------------------------------------

    /// An empty `OnceLock` accepts the material: the `set` succeeds (no `Err`
    /// branch taken), the call returns `Ok(())`, and the lock now holds exactly
    /// the bytes we passed.
    #[test]
    fn set_or_verify_winner_installs_into_empty_lock() {
        let lock: OnceLock<RootMaterial> = OnceLock::new();

        let result =
            RcgenCa::set_or_verify_winner(&lock, root_with(vec![1, 2, 3]), |m| &m.cert_der, "root");

        assert!(result.is_ok(), "empty lock must accept the material");
        assert_eq!(
            lock.get().map(|m| m.cert_der.as_slice()),
            Some([1u8, 2, 3].as_slice()),
            "the lock must now hold the installed material",
        );
    }

    // ---------------------------------------------------------------------
    // Branch 2 — lost race against a byte-identical winner is idempotent Ok.
    // ---------------------------------------------------------------------

    /// A lost race (`set` returns `Err`) whose winner is byte-identical to the
    /// material being adopted is the idempotent same-anchor case: it returns
    /// `Ok(())` and leaves the winner in place. This is the "both racers adopt
    /// the same persisted root" path the old comment described — the only case
    /// in which discarding the `set` error was ever harmless.
    #[test]
    fn set_or_verify_winner_is_idempotent_for_byte_identical_winner() {
        let lock: OnceLock<RootMaterial> = OnceLock::new();
        // Pre-populate: a concurrent writer already won the race with the SAME
        // persisted bytes.
        lock.set(root_with(vec![9, 9, 9])).unwrap_or_else(|_| unreachable!("first set wins"));

        let result =
            RcgenCa::set_or_verify_winner(&lock, root_with(vec![9, 9, 9]), |m| &m.cert_der, "root");

        assert!(result.is_ok(), "byte-identical winner is an idempotent no-op");
        assert_eq!(
            lock.get().map(|m| m.cert_der.as_slice()),
            Some([9u8, 9, 9].as_slice()),
            "the winning material stays in place",
        );
    }

    // ---------------------------------------------------------------------
    // Branch 3 — lost race against a DIVERGENT winner fails loud. REGRESSION.
    // ---------------------------------------------------------------------

    /// THE REGRESSION ASSERTION. A lost race whose winner is a DIFFERENT cert
    /// (simulating a concurrent ephemeral `root_material()` caller that minted a
    /// fresh key between the pre-check and the `set`) MUST surface
    /// `CaError::AdoptionConflict { which: "root" }` — never silently adopt the
    /// ephemeral racer's anchor.
    ///
    /// Falsifiability: the pre-fix `let _ = lock.set(material); Ok(())` returned
    /// `Ok(())` here, silently leaving the WRONG (ephemeral) root in the lock so
    /// every subsequent signature chained to an orphaned anchor. This assertion
    /// fails against that code and passes only with the verify-winner fix.
    /// Pre-populating the lock before the call faithfully reproduces "the lock
    /// got populated between the pre-check and the set".
    #[test]
    fn set_or_verify_winner_rejects_divergent_root_winner() {
        // Distinct byte pairs exercise the `!=` comparison against several
        // shapes (different content, different length, single-byte flip) so a
        // mutant flipping `!=`→`==` or short-circuiting the comparison is killed.
        for (winner_bytes, adopted_bytes) in [
            (vec![1u8, 2, 3], vec![4u8, 5, 6]),
            (vec![1u8, 2, 3], vec![1u8, 2, 4]),
            (vec![0u8], vec![0u8, 0]),
            (vec![], vec![7u8]),
        ] {
            let lock: OnceLock<RootMaterial> = OnceLock::new();
            lock.set(root_with(winner_bytes.clone()))
                .unwrap_or_else(|_| unreachable!("first set wins"));

            let result = RcgenCa::set_or_verify_winner(
                &lock,
                root_with(adopted_bytes.clone()),
                |m| &m.cert_der,
                "root",
            );

            assert!(
                matches!(result, Err(CaError::AdoptionConflict { which: "root" })),
                "divergent winner {winner_bytes:?} vs adopted {adopted_bytes:?} must be an \
                 AdoptionConflict(\"root\"), got {result:?}",
            );
            // The ephemeral winner stays in the lock; the helper does NOT
            // overwrite it (OnceLock cannot) — the failure is the signal, not a
            // silent swap.
            assert_eq!(
                lock.get().map(|m| m.cert_der.as_slice()),
                Some(winner_bytes.as_slice()),
                "the ephemeral winner is unchanged; the conflict is surfaced, not absorbed",
            );
        }
    }

    /// The intermediate adoption path threads its own `kind` through the helper:
    /// a divergent winner surfaces `AdoptionConflict { which: "intermediate" }`,
    /// proving the `kind` argument is propagated (not hardcoded to "root").
    #[test]
    fn set_or_verify_winner_rejects_divergent_intermediate_winner_with_its_kind() {
        let lock: OnceLock<IntermediateMaterial> = OnceLock::new();
        lock.set(intermediate_with(vec![1, 1, 1]))
            .unwrap_or_else(|_| unreachable!("first set wins"));

        let result = RcgenCa::set_or_verify_winner(
            &lock,
            intermediate_with(vec![2, 2, 2]),
            |m| &m.cert_der,
            "intermediate",
        );

        assert!(
            matches!(result, Err(CaError::AdoptionConflict { which: "intermediate" })),
            "divergent intermediate winner must surface AdoptionConflict(\"intermediate\"), \
             got {result:?}",
        );
    }
}
