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

/// OUTBOUND CLIENT config: present the held client SVID (leaf + intermediate chain
/// material), verify the server cert chains to the trust bundle's root anchor.
/// Secret extraction enabled (kTLS-arm seam).
pub(super) fn client_config(
    svid: &SvidMaterial,
    bundle: &TrustBundle,
) -> Result<Arc<ClientConfig>> {
    let roots = root_store(bundle)?;
    let (certs, key) = present_chain(svid, bundle)?;
    let mut cfg = ClientConfig::builder()
        .with_root_certificates(roots)
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
