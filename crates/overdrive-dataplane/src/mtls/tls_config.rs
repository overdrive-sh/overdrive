//! rustls config builders for the mTLS proxy legs — consuming the held SVID +
//! trust bundle read via `IdentityRead`.
//!
//! OUTBOUND (leg B, CLIENT): present the held client SVID, verify the peer's
//! server cert chains to the trust bundle. INBOUND (leg C, SERVER): present the
//! held server SVID, REQUIRE+VERIFY the client SVID chains to the bundle via
//! `WebPkiClientVerifier`. `enable_secret_extraction = true` everywhere (the
//! kTLS-arm seam); `send_tls13_tickets = 0` on the server (suppress
//! NewSessionTicket — raw kTLS-RX hits EIO on a post-handshake ticket record,
//! `findings.md` #4 / `findings-inbound-intercept.md` Mechanics #3).

use std::sync::Arc;

use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcementError, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};

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

/// Build a `RootCertStore` from the trust bundle's root anchor (+ any
/// intermediate chain material). Empty bundle ⇒ `AbsentBundle`.
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

/// OUTBOUND CLIENT config: present the held client SVID, verify the server cert
/// chains to the trust bundle. Secret extraction enabled (kTLS-arm seam).
pub(super) fn client_config(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<Arc<ClientConfig>> {
    let roots = root_store(bundle)?;
    let (certs, key) = parse_svid_pem(svid)?;
    let mut cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)
        .map_err(|e| MtlsEnforcementError::HandshakeFailed {
            reason: format!("client config with SVID: {e}"),
        })?;
    cfg.enable_secret_extraction = true;
    Ok(Arc::new(cfg))
}

/// INBOUND SERVER config: present the held server SVID, REQUIRE+VERIFY the client
/// SVID chains to the bundle via `WebPkiClientVerifier`. Secret extraction +
/// ticket suppression.
pub(super) fn server_config(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<Arc<ServerConfig>> {
    let roots = root_store(bundle)?;
    let (certs, key) = parse_svid_pem(svid)?;
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
