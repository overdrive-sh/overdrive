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

use overdrive_core::traits::ca::{Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, RootCaHandle};
use overdrive_core::traits::ca::{IntermediateHandle, SvidMaterial, SvidRequest, TrustBundle};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::{CertSerial, CertSpec, KeyUsage, NodeId, SpiffeId};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SerialNumber,
};

/// Number of random bytes drawn for a certificate serial — 128 bits, well
/// above the CA/B Forum 64-bit floor (research Finding 10). Matches `SimCa`'s
/// draw width so the entropy contract is identical across adapters.
const SERIAL_BYTES: usize = 16;

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

/// The production [`Ca`] host adapter.
///
/// Holds the injected [`Entropy`] source and the trust-domain subject the root
/// is minted for, plus a [`OnceLock`] caching the persistent root material so
/// the root key is generated once and reused for intermediate signing. `Send +
/// Sync` (the `Arc<dyn Entropy>` is `Send + Sync`, `OnceLock` is `Sync`), so it
/// can be shared across async tasks in the composition root.
pub struct RcgenCa {
    entropy: Arc<dyn Entropy>,
    subject: SpiffeId,
    root_material: OnceLock<RootMaterial>,
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
        Self { entropy, subject, root_material: OnceLock::new() }
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
        // Precondition: the persistent root exists. `root_material()` mints it on
        // first call (idempotent thereafter), so the intermediate is always
        // signed by the same root key the caller observes via `root()`.
        let root = self.root_material()?;

        // Rebuild the root key + params so the rcgen `Issuer` carries the root's
        // DN and key-usages — the issuer field stamped on the intermediate then
        // matches the root's subject, which is what makes the chain verify
        // (S-03 / KPI K1). The serial drawn for the root is re-stamped on the
        // rebuilt root params purely so `Issuer::from_params` has a well-formed
        // params object; it does not affect the intermediate's own serial.
        let root_key = KeyPair::from_pem(&root.key_pem).map_err(|source| {
            CaError::signing_failed(format!("root key reload for issuer: {source}"))
        })?;
        // A placeholder serial for the rebuilt root params (the real root serial
        // already lives on the cached root cert; the issuer only needs DN +
        // key-usages, not a serial).
        let root_params = self.root_params(&[0u8; SERIAL_BYTES])?;
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&root_params, &root_key);

        // The pure decision: the intermediate profile for this trust domain.
        // Derived from `CertSpec::intermediate` exactly the way `root()` derives
        // from `CertSpec::root` — CA:TRUE, pathLen=0, keyCertSign critical.
        let spec = CertSpec::intermediate(self.subject.clone());

        let inter_key = KeyPair::generate().map_err(|source| {
            CaError::signing_failed(format!("intermediate keypair generation: {source}"))
        })?;

        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|source| CaError::signing_failed(format!("intermediate params: {source}")))?;

        // basicConstraints: CA:TRUE bounded by pathLen=0 (`CertSpec::path_len()`
        // is `Some(0)` for an intermediate) — it signs leaves only, never a
        // further CA. The 0.14.8 translation of pathLen=0 is
        // `IsCa::Ca(BasicConstraints::Constrained(0))`.
        params.is_ca = match spec.path_len() {
            Some(path_len) => IsCa::Ca(BasicConstraints::Constrained(path_len)),
            None if spec.is_ca() => IsCa::Ca(BasicConstraints::Unconstrained),
            None => IsCa::NoCa,
        };

        // keyUsage: derived from the spec (keyCertSign only for an
        // intermediate). rcgen marks the extension critical automatically when
        // the set is non-empty — the integration test asserts `.critical`.
        params.key_usages = spec.key_usages().into_iter().map(to_rcgen_key_usage).collect();

        // Subject: the trust domain as the DN organisation (research Finding 2),
        // PLUS a per-node CommonName so the intermediate's subject DN differs
        // from the root's (which carries the organisation alone). Without a
        // distinguishing component the intermediate's subject equals the root's
        // subject, and `openssl verify` treats the intermediate as self-signed
        // and refuses to build the chain. The CN mirrors the sim fixture's
        // `O=…, CN=node-intermediate` shape.
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::OrganizationName, spec.subject().trust_domain());
            dn.push(DnType::CommonName, node.as_str());
            dn
        };

        // Serial: drawn via the injected Entropy port, stamped onto the cert so
        // the intermediate serial is genuinely entropy-sourced.
        let (serial_bytes, serial) = self.draw_serial();
        params.serial_number = Some(SerialNumber::from_slice(&serial_bytes));

        // Sign the intermediate by the root (0.14 2-arg `signed_by`).
        let cert = params.signed_by(&inter_key, &issuer).map_err(|source| {
            CaError::signing_failed(format!("intermediate sign by root: {source}"))
        })?;

        Ok(IntermediateHandle::new(
            CaCertPem::new(cert.pem()),
            CaCertDer::new(cert.der().to_vec()),
            serial,
            CaKeyPem::new(inter_key.serialize_pem()),
        ))
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 04")]
    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        todo!("RED scaffold: RcgenCa::issue_svid (single URI SAN, CA:FALSE, CSPRNG serial)")
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 03")]
    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        todo!("RED scaffold: RcgenCa::trust_bundle (root anchor; intermediate chain material)")
    }
}
