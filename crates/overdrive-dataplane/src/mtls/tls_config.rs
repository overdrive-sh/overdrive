//! rustls config builders for the mTLS proxy legs — consuming the held SVID +
//! trust bundle read via `IdentityRead`.
//!
//! OUTBOUND (leg B, CLIENT): present the held client SVID, verify the peer's
//! server cert chains to the trust bundle. INBOUND (leg C, SERVER): present the
//! held server SVID, REQUIRE+VERIFY the client SVID chains to the bundle via
//! `WebPkiClientVerifier`. The presented chain is `[leaf] ++ intermediate_chain`
//! (production issuance signs leaves from a node intermediate — root → intermediate
//! → leaf — so a root-anchor-only verifier needs the intermediate appended to build
//! the path); the verifier `root_store` stays **root-anchor-only** (the intermediate
//! is untrusted chain material, not a trust anchor). `enable_secret_extraction = true` everywhere (the
//! kTLS-arm seam); `send_tls13_tickets = 0` on the server (suppress
//! NewSessionTicket — raw kTLS-RX hits EIO on a post-handshake ticket record,
//! `findings.md` #4 / `findings-inbound-intercept.md` Mechanics #3).

use std::sync::Arc;

use overdrive_core::SpiffeId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcementError, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::verify_server_cert_signed_by_trust_anchor;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ParsedCertificate, WebPkiClientVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore,
    ServerConfig, SignatureScheme,
};
use x509_parser::prelude::FromDer as _;

/// Parse a held [`SvidMaterial`] PEM pair into the rustls DER cert chain + key.
pub(super) fn parse_svid_pem(
    svid: &SvidMaterial,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert_pem = svid.cert_pem().as_pem();
    let mut rd = std::io::BufReader::new(cert_pem.as_bytes());
    let certs: std::result::Result<Vec<_>, _> = rustls_pemfile::certs(&mut rd).collect();
    let certs = certs.map_err(|e| MtlsEnforcementError::HandshakeFailed {
        reason: format!("parsing held SVID cert PEM: {e}"),
    })?;
    if certs.is_empty() {
        return Err(MtlsEnforcementError::HandshakeFailed {
            reason: "held SVID cert PEM contained no certificate".into(),
        });
    }
    let key_pem = svid.leaf_key().as_pem();
    let mut kr = std::io::BufReader::new(key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut kr)
        .map_err(|e| MtlsEnforcementError::HandshakeFailed {
            reason: format!("parsing held SVID key PEM: {e}"),
        })?
        .ok_or_else(|| MtlsEnforcementError::HandshakeFailed {
            reason: "held SVID key PEM contained no private key".into(),
        })?;
    Ok((certs, key))
}

/// Build a `RootCertStore` from the trust bundle's **root anchor ONLY**. Empty
/// bundle ⇒ `AbsentBundle`.
///
/// The bundle's intermediate (when present) is **untrusted chain material**
/// (`TrustBundle::intermediate_chain` / ca.rs D1 wire-format) — the verifier uses
/// it to *build* the `leaf → intermediate → root` path but anchors trust solely on
/// the root. Adding the intermediate to the trust store would make it a trust
/// anchor, which it is not; the chain material is presented by the peer (appended
/// to its leaf via [`present_chain`]), not anchored here.
fn root_store(bundle: &TrustBundle) -> Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    let anchor_pem = bundle.root_anchor().as_pem();
    let mut rd = std::io::BufReader::new(anchor_pem.as_bytes());
    let mut added = 0usize;
    for cert in rustls_pemfile::certs(&mut rd) {
        let cert = cert.map_err(|e| MtlsEnforcementError::PeerVerificationFailed {
            reason: format!("parsing trust-bundle anchor PEM: {e}"),
        })?;
        roots.add(cert).map_err(|e| MtlsEnforcementError::PeerVerificationFailed {
            reason: format!("adding trust-bundle anchor: {e}"),
        })?;
        added += 1;
    }
    if added == 0 {
        return Err(MtlsEnforcementError::AbsentBundle);
    }
    Ok(roots)
}

/// The certificate chain a leg PRESENTS in its handshake: the held leaf SVID
/// (chain position 0 — the verified identity), followed by the bundle's
/// intermediate chain material when present, so a peer trusting only the root
/// anchor can build `leaf → intermediate → root`.
///
/// Production issuance signs workload leaves from a node **intermediate**
/// (root → intermediate → leaf); presenting the leaf alone leaves a
/// root-anchor-only verifier unable to complete the path and the handshake fails.
/// Appending `bundle.intermediate_chain()` is exactly what closes that gap. When
/// the bundle carries no intermediate (a root-signs-leaf deployment) the chain is
/// just the leaf certs.
fn present_chain(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let (mut certs, key) = parse_svid_pem(svid)?;
    if let Some(intermediate) = bundle.intermediate_chain() {
        let mut rd = std::io::BufReader::new(intermediate.as_pem().as_bytes());
        for cert in rustls_pemfile::certs(&mut rd) {
            let cert = cert.map_err(|e| MtlsEnforcementError::HandshakeFailed {
                reason: format!("parsing trust-bundle intermediate chain PEM: {e}"),
            })?;
            certs.push(cert);
        }
    }
    Ok((certs, key))
}

/// Authn-only v1 peer verifier for an outbound (leg-B) server SVID.
///
/// Rustls' ordinary server verifier couples chain validation to DNS/IP SAN
/// matching. Workload SVIDs instead identify peers with exactly one SPIFFE URI
/// SAN; production issuance deliberately adds no synthetic DNS name. This
/// verifier therefore asks rustls WebPKI to validate the server certificate's
/// chain, validity, server purpose, and handshake signatures without its
/// separate DNS/IP-name check, then enforces the SPIFFE URI cardinality and
/// syntax directly on the same verified leaf.
///
/// It intentionally does not compare the URI to an expected destination:
/// authn-only v1 accepts any bundle-chained valid workload SVID. The
/// `expected_peer` join and equality check remain #242; this verifier neither
/// pretends the transport SNI is an identity nor relaxes chain authentication.
#[derive(Debug)]
struct SpiffeServerVerifier {
    roots: RootCertStore,
    provider: Arc<CryptoProvider>,
}

impl SpiffeServerVerifier {
    fn new(roots: RootCertStore) -> Self {
        Self { roots, provider: Arc::new(rustls::crypto::ring::default_provider()) }
    }
}

impl ServerCertVerifier for SpiffeServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<ServerCertVerified, RustlsError> {
        let parsed = ParsedCertificate::try_from(end_entity)?;
        verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        validate_single_spiffe_uri(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn validate_single_spiffe_uri(
    end_entity: &CertificateDer<'_>,
) -> std::result::Result<SpiffeId, RustlsError> {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(end_entity.as_ref())
        .map_err(|_| RustlsError::InvalidCertificate(CertificateError::BadEncoding))?;
    let san = cert
        .subject_alternative_name()
        .map_err(|_| RustlsError::InvalidCertificate(CertificateError::BadEncoding))?
        .ok_or(RustlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure))?;
    let uris = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [uri] = uris.as_slice() else {
        return Err(RustlsError::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ));
    };
    SpiffeId::new(uri).map_err(|_| {
        RustlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
    })
}

/// OUTBOUND CLIENT config: present the held client SVID (leaf + intermediate
/// chain material), verify the peer chains to the trust-bundle root, and
/// require exactly one syntactically-valid SPIFFE URI SAN. Secret extraction
/// enabled (kTLS-arm seam). Intended-peer URI equality is #242; v1 is
/// authenticated-cluster-peer only.
pub(super) fn client_config(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<Arc<ClientConfig>> {
    let verifier = Arc::new(SpiffeServerVerifier::new(root_store(bundle)?));
    let (certs, key) = present_chain(svid, bundle)?;
    let mut cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(certs, key)
        .map_err(|e| MtlsEnforcementError::HandshakeFailed {
            reason: format!("client config with SVID: {e}"),
        })?;
    cfg.enable_secret_extraction = true;
    Ok(Arc::new(cfg))
}

/// INBOUND SERVER config: present the held server SVID (leaf + intermediate chain
/// material), REQUIRE+VERIFY the client SVID chains to the bundle's root anchor via
/// `WebPkiClientVerifier`. Secret extraction + ticket suppression.
pub(super) fn server_config(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<Arc<ServerConfig>> {
    let roots = root_store(bundle)?;
    let (certs, key) = present_chain(svid, bundle)?;
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots)).build().map_err(|e| {
        MtlsEnforcementError::PeerVerificationFailed {
            reason: format!("building client verifier: {e}"),
        }
    })?;
    let mut cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| MtlsEnforcementError::HandshakeFailed {
            reason: format!("server config with SVID: {e}"),
        })?;
    cfg.enable_secret_extraction = true;
    cfg.send_tls13_tickets = 0; // suppress NewSessionTicket (kTLS-RX EIO on tickets)
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "focused PKI fixtures fail fast when deterministic certificate construction fails"
    )]

    use std::net::Ipv4Addr;

    use rcgen::{CertificateParams, Issuer, KeyPair, SanType, string::Ia5String};
    use rustls::pki_types::{ServerName, UnixTime};

    use super::*;

    struct TestChain {
        root: CertificateDer<'static>,
        intermediate: CertificateDer<'static>,
        leaf: CertificateDer<'static>,
    }

    fn mint_chain(uri_sans: &[&str]) -> TestChain {
        let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
        root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
        let root = root_params.self_signed(&root_key).expect("root certificate");

        let mut intermediate_params =
            CertificateParams::new(Vec::<String>::new()).expect("intermediate params");
        intermediate_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        intermediate_params.use_authority_key_identifier_extension = true;
        let intermediate_key =
            KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("intermediate key");
        let root_issuer = Issuer::from_params(&root_params, &root_key);
        let intermediate = intermediate_params
            .signed_by(&intermediate_key, &root_issuer)
            .expect("intermediate certificate");

        let mut leaf_params = CertificateParams::new(Vec::<String>::new()).expect("leaf params");
        leaf_params.subject_alt_names = uri_sans
            .iter()
            .map(|uri| SanType::URI(Ia5String::try_from(*uri).expect("URI IA5 string")))
            .collect();
        leaf_params.use_authority_key_identifier_extension = true;
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
        let intermediate_issuer = Issuer::from_params(&intermediate_params, &intermediate_key);
        let leaf =
            leaf_params.signed_by(&leaf_key, &intermediate_issuer).expect("leaf certificate");

        TestChain {
            root: root.der().clone(),
            intermediate: intermediate.der().clone(),
            leaf: leaf.der().clone(),
        }
    }

    fn verifier_for(root: CertificateDer<'static>) -> SpiffeServerVerifier {
        let mut roots = RootCertStore::empty();
        roots.add(root).expect("trusted root");
        SpiffeServerVerifier::new(roots)
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn spiffe_server_verifier_accepts_uri_only_svid_and_rejects_ambiguous_identity() {
        let valid = mint_chain(&["spiffe://overdrive.local/workload/server/alloc/server-0"]);
        let verifier = verifier_for(valid.root);
        let server_name = ServerName::IpAddress(Ipv4Addr::LOCALHOST.into());
        verifier
            .verify_server_cert(
                &valid.leaf,
                &[valid.intermediate],
                &server_name,
                &[],
                UnixTime::now(),
            )
            .expect("production URI-only SVID chains without a synthetic DNS SAN");

        let ambiguous = mint_chain(&[
            "spiffe://overdrive.local/workload/server/alloc/server-0",
            "spiffe://overdrive.local/workload/other/alloc/other-0",
        ]);
        let verifier = verifier_for(ambiguous.root);
        assert!(
            verifier
                .verify_server_cert(
                    &ambiguous.leaf,
                    &[ambiguous.intermediate],
                    &server_name,
                    &[],
                    UnixTime::now(),
                )
                .is_err(),
            "a trusted leaf with two URI SANs has ambiguous workload identity and must fail closed"
        );
    }
}
