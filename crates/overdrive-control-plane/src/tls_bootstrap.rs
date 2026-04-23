//! Ephemeral CA + trust triple bootstrap per ADR-0010.
//!
//! On first `overdrive cluster init`:
//! - Mint a self-signed CA (P-256, `rcgen`).
//! - Mint a server leaf cert with SANs `127.0.0.1`, `::1`, `localhost`,
//!   and the host's own hostname.
//! - Mint a client leaf cert for CLI use.
//! - Write `~/.overdrive/config` with base64-encoded PEM CA / crt / key
//!   (the "trust triple").
//!
//! Re-running `cluster init` re-mints everything. **No persisted CA
//! key.** `mint_ephemeral_ca` is self-contained — it takes no inputs
//! and every successive call produces fresh, bytewise-distinct
//! material. That is the ADR-0010 §R1 ephemerality property expressed
//! at the type level.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::Serialize;

use crate::error::ControlPlaneError;

/// Material produced by `mint_ephemeral_ca`. All fields are in-memory
/// PEM; callers write them to the trust triple via `write_trust_triple`.
///
/// The struct intentionally exposes plain `String` fields — the trust
/// triple is a one-shot artefact handed back to the CLI bootstrap
/// code; wrapping each PEM in a newtype would create types with no
/// additional invariants worth encoding.
#[derive(Debug, Clone)]
pub struct CaMaterial {
    /// PEM-encoded self-signed CA certificate.
    pub ca_cert_pem: String,
    /// PEM-encoded server leaf certificate, signed by the CA.
    pub server_leaf_cert_pem: String,
    /// PEM-encoded server leaf private key (PKCS#8).
    pub server_leaf_key_pem: String,
    /// PEM-encoded client leaf certificate, signed by the CA.
    pub client_leaf_cert_pem: String,
    /// PEM-encoded client leaf private key (PKCS#8).
    pub client_leaf_key_pem: String,
}

/// Mint the ephemeral CA + server leaf + client leaf. Multi-SAN on the
/// server cert per ADR-0010 §R3.
///
/// Every call generates a fresh CA keypair — the key material never
/// leaves memory, and the function takes no inputs by design: there
/// is no configuration surface, no prompt, no file to read. Successive
/// calls produce distinct material.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if `rcgen` key generation or
/// PEM serialisation fails, or if the local hostname cannot be
/// resolved.
pub fn mint_ephemeral_ca() -> Result<CaMaterial, ControlPlaneError> {
    // --- CA -----------------------------------------------------------
    let ca_key = KeyPair::generate()
        .map_err(|e| ControlPlaneError::Internal(format!("ca keypair generation: {e}")))?;

    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::Internal(format!("ca params: {e}")))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "overdrive-ephemeral-ca");
        dn
    };
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];

    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| ControlPlaneError::Internal(format!("ca self-sign: {e}")))?;
    let ca_cert_pem = ca_cert.pem();

    // --- Server leaf --------------------------------------------------
    let hostname_os = hostname::get()
        .map_err(|e| ControlPlaneError::Internal(format!("hostname::get: {e}")))?;
    let hostname_str = hostname_os.to_string_lossy().into_owned();

    // ADR-0010 §R3: exactly four SAN entries. The acceptance test
    // asserts EXACT set equality, so any deviation (wrong count, wrong
    // address, missing DNS entry) will fail the test.
    let server_sans = vec![
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        SanType::DnsName("localhost".try_into().map_err(|e| {
            ControlPlaneError::Internal(format!("dns name `localhost`: {e}"))
        })?),
        SanType::DnsName(hostname_str.clone().try_into().map_err(|e| {
            ControlPlaneError::Internal(format!("dns name `{hostname_str}`: {e}"))
        })?),
    ];

    let server_key = KeyPair::generate()
        .map_err(|e| ControlPlaneError::Internal(format!("server keypair: {e}")))?;
    let mut server_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::Internal(format!("server params: {e}")))?;
    server_params.subject_alt_names = server_sans;
    server_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "overdrive-control-plane");
        dn
    };
    server_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .map_err(|e| ControlPlaneError::Internal(format!("server sign: {e}")))?;
    let server_leaf_cert_pem = server_cert.pem();
    let server_leaf_key_pem = server_key.serialize_pem();

    // --- Client leaf (local operator per ADR-0010 Phase 1) -----------
    let client_key = KeyPair::generate()
        .map_err(|e| ControlPlaneError::Internal(format!("client keypair: {e}")))?;
    let mut client_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::Internal(format!("client params: {e}")))?;
    client_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "local-operator");
        dn
    };
    client_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];

    let client_cert = client_params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .map_err(|e| ControlPlaneError::Internal(format!("client sign: {e}")))?;
    let client_leaf_cert_pem = client_cert.pem();
    let client_leaf_key_pem = client_key.serialize_pem();

    Ok(CaMaterial {
        ca_cert_pem,
        server_leaf_cert_pem,
        server_leaf_key_pem,
        client_leaf_cert_pem,
        client_leaf_key_pem,
    })
}

/// Internal serde shape for the Talos-style `~/.overdrive/config`.
#[derive(Debug, Serialize)]
struct TalosConfigOut<'a> {
    context: &'static str,
    contexts: std::collections::BTreeMap<&'static str, TalosContextOut<'a>>,
}

#[derive(Debug, Serialize)]
struct TalosContextOut<'a> {
    endpoint: &'a str,
    ca: String,
    crt: String,
    key: String,
}

/// Write the trust triple to `<config_dir>/.overdrive/config` in the
/// Talos-shape YAML per ADR-0010 §R2:
///
/// ```yaml
/// context: local
/// contexts:
///   local:
///     endpoint: https://127.0.0.1:7001
///     ca: <base64 PEM>
///     crt: <base64 PEM>
///     key: <base64 PEM>
/// ```
///
/// The file is written with mode 0600 on Unix (owner read/write only)
/// to match Talos' `talosconfig` discipline — the client-leaf key is
/// a credential.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the parent directory
/// cannot be created, the file cannot be written, or YAML
/// serialisation fails.
pub fn write_trust_triple(
    config_dir: &Path,
    endpoint: &str,
    material: &CaMaterial,
) -> Result<(), ControlPlaneError> {
    let overdrive_dir = config_dir.join(".overdrive");
    std::fs::create_dir_all(&overdrive_dir).map_err(|e| {
        ControlPlaneError::Internal(format!(
            "create_dir_all({}): {e}",
            overdrive_dir.display()
        ))
    })?;

    let config_path = overdrive_dir.join("config");

    let mut contexts = std::collections::BTreeMap::new();
    contexts.insert(
        "local",
        TalosContextOut {
            endpoint,
            ca: BASE64.encode(material.ca_cert_pem.as_bytes()),
            crt: BASE64.encode(material.client_leaf_cert_pem.as_bytes()),
            key: BASE64.encode(material.client_leaf_key_pem.as_bytes()),
        },
    );
    let doc = TalosConfigOut {
        context: "local",
        contexts,
    };

    let yaml = serde_yaml::to_string(&doc)
        .map_err(|e| ControlPlaneError::Internal(format!("yaml serialise: {e}")))?;

    write_file_owner_only(&config_path, yaml.as_bytes())?;

    Ok(())
}

#[cfg(unix)]
fn write_file_owner_only(path: &Path, bytes: &[u8]) -> Result<(), ControlPlaneError> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| ControlPlaneError::Internal(format!("open({}): {e}", path.display())))?;
    file.write_all(bytes)
        .map_err(|e| ControlPlaneError::Internal(format!("write({}): {e}", path.display())))?;
    Ok(())
}

// The `cfg(not(unix))` branch is the Windows fallback: it cannot be
// exercised by tests running on the Linux/macOS CI hosts. Phase 1 has
// no Windows target in the support matrix, so any cargo-mutants
// "MISSED" result against this function is a platform-coverage gap,
// not a test-suite gap. Revisit if Windows enters the matrix — at
// that point a Windows CI runner will naturally catch mutations here.
#[cfg(not(unix))]
fn write_file_owner_only(path: &Path, bytes: &[u8]) -> Result<(), ControlPlaneError> {
    std::fs::write(path, bytes)
        .map_err(|e| ControlPlaneError::Internal(format!("write({}): {e}", path.display())))
}

/// Load a `rustls::ServerConfig` from minted server material. Pure on
/// PEM inputs; no filesystem reads.
///
/// Parses `server_leaf_cert_pem` and `server_leaf_key_pem` via
/// `rustls_pemfile`, constructs a `rustls::ServerConfig` with no client
/// authentication (Phase 1 operator-auth via client cert lands later —
/// see ADR-0010 Phase 2+), and sets ALPN to `h2, http/1.1` per
/// ADR-0008 §Transport.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the PEM parse fails, if
/// the private key is missing, or if `rustls` rejects the cert/key
/// combination (e.g. mismatched key algorithm vs certificate).
pub fn load_server_tls_config(
    material: &CaMaterial,
) -> Result<rustls::ServerConfig, ControlPlaneError> {
    use std::io::Cursor;

    // Parse server leaf certificate chain from PEM.
    let mut cert_reader = Cursor::new(material.server_leaf_cert_pem.as_bytes());
    let cert_chain: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                ControlPlaneError::Internal(format!("parse server cert PEM: {e}"))
            })?;
    if cert_chain.is_empty() {
        return Err(ControlPlaneError::Internal(
            "server leaf PEM contained no certificates".into(),
        ));
    }

    // Parse private key from PEM (accepts PKCS#8, PKCS#1, or SEC1).
    let mut key_reader = Cursor::new(material.server_leaf_key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| ControlPlaneError::Internal(format!("parse server key PEM: {e}")))?
        .ok_or_else(|| {
            ControlPlaneError::Internal("server key PEM contained no private key".into())
        })?;

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .map_err(|e| ControlPlaneError::Internal(format!("rustls with_single_cert: {e}")))?;

    // ADR-0008 §ALPN: prefer h2, fall back to http/1.1.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(config)
}
