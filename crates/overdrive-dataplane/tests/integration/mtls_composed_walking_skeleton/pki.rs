//! Test PKI for the composed transparent-mTLS walking skeleton (step 01-01).
//!
//! Mints a shared CA, an outbound CLIENT leaf (the workload-as-client SVID the
//! agent presents on leg B), and an inbound SERVER leaf (the server-workload SVID
//! the agent presents on leg C). All chain to the one CA, so the agent's
//! `WebPkiClientVerifier` (inbound) and the peer's server-cert verification
//! (outbound) both succeed against the shared trust bundle.
//!
//! DEV-ONLY: the production adapter consumes the held SVID via `IdentityRead` and
//! NEVER mints. This module exists only so the test can populate the
//! `HeldIdentities` double the adapter reads through.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The PKI fixture exposes more surface (`ca_cert_pem`, `untrusted_client_leaf`,
// the per-leaf `key_der`) than the 01-01 happy-path acceptance test consumes —
// they back the fail-closed `wrongca`/`nocert` negatives Slice 05 reuses. Allowed
// dead-code here so the fixture is complete at first landing rather than grown
// piecemeal.
#![allow(dead_code, clippy::unused_self)]

use std::time::Duration;

use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// A minted leaf — the PEM cert + key + the SPIFFE-shaped SAN, plus the DER forms
/// rustls consumes directly.
pub struct Leaf {
    pub cert_pem: String,
    pub key_pem: String,
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub spiffe: overdrive_core::SpiffeId,
    pub serial: CertSerial,
}

/// The shared test PKI.
pub struct TestPki {
    ca_cert_pem: String,
    pub client_leaf: Leaf,
    pub server_leaf: Leaf,
    /// The OUTBOUND real-peer leaf: a SERVER cert with a DNS SAN matching the SNI
    /// (`peer.overdrive.local`) the adapter's leg-B client handshake presents, so
    /// the adapter's peer-verification (cert chains to the bundle AND the SNI
    /// matches a SAN) succeeds. Used by `OutboundPeer` (the test-side mTLS server
    /// the agent's leg B dials), NOT read through `IdentityRead`.
    pub peer_leaf: Leaf,
    pub client_alloc: AllocationId,
    pub server_alloc: AllocationId,
}

impl TestPki {
    /// Mint the CA + the client and server leaves. The SANs are SPIFFE-shaped
    /// (`spiffe://overdrive.local/...`) so the verified peer identity is a real
    /// `SpiffeId` (authn — chain-to-bundle; `expected_peer` pinning is #178).
    /// The DNS SAN the OUTBOUND peer presents (matches the SNI the adapter's leg-B
    /// client handshake uses in `mtls::outbound::client_handshake`).
    pub const PEER_SNI: &'static str = "peer.overdrive.local";
    /// The DNS SAN the INBOUND server SVID carries (matches the SNI the inbound
    /// client presents toward the server's virtual address).
    pub const SERVER_SNI: &'static str = "server.overdrive.local";

    #[must_use]
    pub fn mint() -> Self {
        let ca = MintedCa::mint("overdrive-mtls-ws-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        // The client leaf is clientAuth-only (no SNI to match — it is the client).
        let client_leaf = ca.mint_leaf(client_spiffe, None, true);
        // The server SVID is serverAuth + carries the SERVER_SNI DNS SAN so the
        // inbound client's SNI matches when it verifies the adapter's server cert.
        let server_leaf = ca.mint_leaf(server_spiffe, Some(Self::SERVER_SNI), false);
        // The outbound real-peer leaf: serverAuth + the PEER_SNI DNS SAN.
        let peer_leaf = ca.mint_leaf(
            "spiffe://overdrive.local/ns/default/sa/peer",
            Some(Self::PEER_SNI),
            false,
        );

        Self {
            ca_cert_pem: ca.cert_pem,
            client_leaf,
            server_leaf,
            peer_leaf,
            client_alloc: AllocationId::new("alloc-mtls-client").expect("valid alloc"),
            server_alloc: AllocationId::new("alloc-mtls-server").expect("valid alloc"),
        }
    }

    /// The CA cert PEM (the trust anchor the bundle wraps).
    #[must_use]
    pub fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    /// The shared trust bundle (root anchor = the CA; no intermediate).
    #[must_use]
    pub fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(CaCertPem::new(self.ca_cert_pem.clone()), None)
    }

    /// The client-leg SVID material the adapter reads via `IdentityRead`.
    #[must_use]
    pub fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
    }

    /// The server-leg SVID material the adapter reads via `IdentityRead`.
    #[must_use]
    pub fn server_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.server_leaf)
    }

    /// Mint a fresh, UNTRUSTED client leaf (a different CA) — for the fail-closed
    /// `wrongca` negative (out of 01-01 scope, but the harness exposes it so
    /// Slice 05 reuses the PKI).
    #[must_use]
    pub fn untrusted_client_leaf(&self) -> Leaf {
        let wca = MintedCa::mint("overdrive-mtls-ws-ROGUE-CA");
        wca.mint_leaf("spiffe://rogue.local/ns/x/sa/rogue", None, true)
    }
}

/// A minted CA retaining its `CertificateParams` + `KeyPair` so it can build a
/// reusable rcgen 0.14 `Issuer` for signing leaves (`Issuer::from_params`).
struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
}

impl MintedCa {
    fn mint(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert_pem = cert.pem();
        Self { params, key, cert_pem }
    }

    fn mint_leaf(&self, spiffe: &str, dns_san: Option<&str>, client_auth: bool) -> Leaf {
        // The SPIFFE id is carried as a URI SAN (rcgen 0.14 `SanType::URI` over an
        // `Ia5String`), mirroring `overdrive-host`'s `RcgenCa::issue_svid`. A
        // `dns_san` (when given) is added so a peer's SNI can match (rustls verifies
        // the server cert against the SNI; SPIFFE-URI SANs alone do not satisfy it).
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
        let mut sans = vec![SanType::URI(uri)];
        if let Some(dns) = dns_san {
            let dns_ia5 = Ia5String::try_from(dns).expect("dns SAN is a valid IA5 string");
            sans.push(SanType::DnsName(dns_ia5));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        // rcgen 0.14 2-arg `signed_by(leaf_key, &Issuer)`.
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).unwrap();
        let cert_pem = cert.pem();
        let key_pem = leaf_key.serialize_pem();
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Leaf {
            cert_pem,
            key_pem,
            cert_der,
            key_der,
            spiffe: spiffe.parse().expect("valid spiffe id"),
            serial: CertSerial::new("0a0b0c0d").expect("valid serial"),
        }
    }
}

/// Assemble `SvidMaterial` from a minted leaf — the cert PEM/DER + the matching
/// leaf private key PEM (ADR-0063 D9, node-held) + a far-future `not_after`.
fn svid_from_leaf(leaf: &Leaf) -> SvidMaterial {
    let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)); // 2100
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        leaf.serial.clone(),
        leaf.spiffe.clone(),
        CaKeyPem::new(leaf.key_pem.clone()),
        not_after,
    )
}
