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

use std::sync::Arc;

use overdrive_core::traits::ca::{Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, RootCaHandle};
use overdrive_core::traits::ca::{IntermediateHandle, SvidMaterial, SvidRequest, TrustBundle};
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::{CertSerial, CertSpec, KeyUsage, NodeId, SpiffeId};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
    SerialNumber,
};

/// Number of random bytes drawn for a certificate serial — 128 bits, well
/// above the CA/B Forum 64-bit floor (research Finding 10). Matches `SimCa`'s
/// draw width so the entropy contract is identical across adapters.
const SERIAL_BYTES: usize = 16;

/// The production [`Ca`] host adapter.
///
/// Holds the injected [`Entropy`] source and the trust-domain subject the root
/// is minted for. `Send + Sync` (the `Arc<dyn Entropy>` is `Send + Sync`), so
/// it can be shared across async tasks in the composition root.
pub struct RcgenCa {
    entropy: Arc<dyn Entropy>,
    subject: SpiffeId,
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
        Self { entropy, subject }
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
        // The pure decision: the root profile for this trust domain. The host
        // adapter derives every X.509 property from this CertSpec rather than
        // hardcoding it — host and sim share the one policy surface (criterion
        // 3 / ADR-0063 D8).
        let spec = CertSpec::root(self.subject.clone());

        let key = KeyPair::generate().map_err(|source| {
            CaError::signing_failed(format!("root keypair generation: {source}"))
        })?;

        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|source| CaError::signing_failed(format!("root params: {source}")))?;

        // basicConstraints: derived from the spec. A root carries CA:TRUE with
        // NO pathLen (`CertSpec::path_len()` is `None` for a root) → an
        // Unconstrained CA.
        params.is_ca = match spec.path_len() {
            None if spec.is_ca() => IsCa::Ca(BasicConstraints::Unconstrained),
            Some(path_len) => IsCa::Ca(BasicConstraints::Constrained(path_len)),
            None => IsCa::NoCa,
        };

        // keyUsage: derived from the spec's key-usage set. rcgen marks the
        // keyUsage extension critical automatically when the set is non-empty —
        // the integration test asserts `.critical == true` on the real bytes.
        params.key_usages = spec.key_usages().into_iter().map(to_rcgen_key_usage).collect();

        // Subject: the trust domain only (research Finding 2), carried as the
        // DN organisation. No path component.
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::OrganizationName, spec.subject().trust_domain());
            dn
        };

        // Serial: drawn via the injected Entropy port (contract requirement),
        // stamped onto the rcgen cert so the serial is genuinely entropy-sourced.
        let (serial_bytes, serial) = self.draw_serial();
        params.serial_number = Some(SerialNumber::from_slice(&serial_bytes));

        let cert = params
            .self_signed(&key)
            .map_err(|source| CaError::signing_failed(format!("root self-sign: {source}")))?;

        Ok(RootCaHandle::new(
            CaCertPem::new(cert.pem()),
            CaCertDer::new(cert.der().to_vec()),
            serial,
            CaKeyPem::new(key.serialize_pem()),
        ))
    }

    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 02-03")]
    fn issue_intermediate(&self, _node: &NodeId) -> Result<IntermediateHandle, CaError> {
        todo!("RED scaffold: RcgenCa::issue_intermediate (pathLen=0, signed by root)")
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
