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

use std::ffi::OsString;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};

use crate::error::ControlPlaneError;

/// Abstraction over "get the host's hostname".
///
/// `mint_ephemeral_ca` needs the OS hostname for the fourth SAN entry
/// (ADR-0010 §R3), but `hostname::get()` can fail on obscure platforms
/// and inside restricted containers. A failing hostname source must NOT
/// crash the CA mint — the server leaf degrades to the remaining three
/// SANs (`127.0.0.1`, `::1`, `localhost`) and the minted material is
/// still valid for all loopback traffic.
///
/// Injecting the source as a trait keeps the degradation path
/// unit-testable: `SystemHostname` is the production binding; tests
/// can supply an implementation that deliberately returns `Err`.
pub trait HostnameSource {
    /// Return the host's hostname, or an `io::Error` if it cannot be
    /// resolved.
    ///
    /// # Errors
    ///
    /// Propagates whatever `hostname::get()` (or the test double)
    /// returns.
    fn get(&self) -> io::Result<OsString>;
}

/// Production `HostnameSource` — delegates to the `hostname` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemHostname;

impl HostnameSource for SystemHostname {
    fn get(&self) -> io::Result<OsString> {
        hostname::get()
    }
}

/// Material produced by `mint_ephemeral_ca`. All fields are in-memory
/// PEM; callers write them to the trust triple via `write_trust_triple`.
///
/// The struct intentionally exposes plain `String` fields — the trust
/// triple is a one-shot artefact handed back to the CLI bootstrap
/// code; wrapping each PEM in a newtype would create types with no
/// additional invariants worth encoding.
// The `_pem` postfix is a deliberate encoding-format marker (vs. DER,
// raw bytes, etc.), not redundant naming — every field carries PEM
// text and readers need to know that at the call site.
#[allow(clippy::struct_field_names)]
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
/// leaves memory, and the function takes no configuration input by
/// design: there is no prompt and no file to read. Successive calls
/// produce distinct material.
///
/// Uses `SystemHostname` for the fourth SAN. On hosts where
/// `hostname::get()` fails, the fourth SAN is omitted — see
/// `mint_ephemeral_ca_with_hostname` for the injectable variant
/// covered by a unit test.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if `rcgen` key generation or
/// PEM serialisation fails. Hostname resolution failures are tolerated
/// — the leaf degrades to the remaining three SANs.
pub fn mint_ephemeral_ca() -> Result<CaMaterial, ControlPlaneError> {
    mint_ephemeral_ca_with_hostname(&SystemHostname)
}

/// Injectable-hostname variant of [`mint_ephemeral_ca`].
///
/// Split into its own function so the "hostname unavailable" degradation
/// path is unit-testable without stubbing out the OS. Production code
/// calls [`mint_ephemeral_ca`], which wires `SystemHostname` in.
///
/// # Errors
///
/// Same as [`mint_ephemeral_ca`].
pub fn mint_ephemeral_ca_with_hostname(
    hostname_source: &dyn HostnameSource,
) -> Result<CaMaterial, ControlPlaneError> {
    // --- CA -----------------------------------------------------------
    let ca_key =
        KeyPair::generate().map_err(|e| ControlPlaneError::internal("ca keypair generation", e))?;

    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::internal("ca params", e))?;
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
        .map_err(|e| ControlPlaneError::internal("ca self-sign", e))?;
    let ca_cert_pem = ca_cert.pem();

    // --- Server leaf --------------------------------------------------
    // ADR-0010 §R3: the production path targets four SAN entries
    // (`127.0.0.1`, `::1`, `localhost`, and the host hostname). If
    // `hostname_source.get()` fails — obscure platforms, restricted
    // containers — we degrade gracefully to the remaining three SANs
    // rather than fail the whole mint. The minted material is still
    // valid for all loopback traffic; a missing fourth SAN affects
    // only cross-host reachability, which Phase 1 does not ship.
    let mut server_sans = vec![
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        SanType::DnsName(
            "localhost"
                .try_into()
                .map_err(|e| ControlPlaneError::internal("dns name `localhost`", e))?,
        ),
    ];

    match hostname_source.get() {
        Ok(hostname_os) => {
            let hostname_str = hostname_os.to_string_lossy().into_owned();
            match hostname_str.clone().try_into() {
                Ok(dns) => server_sans.push(SanType::DnsName(dns)),
                Err(e) => {
                    tracing::warn!(
                        hostname = %hostname_str,
                        error = %e,
                        "hostname could not be encoded as a DNS SAN; degrading to three SANs",
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "hostname::get failed; degrading server leaf to three SANs \
                 (127.0.0.1, ::1, localhost)",
            );
        }
    }

    let server_key =
        KeyPair::generate().map_err(|e| ControlPlaneError::internal("server keypair", e))?;
    let mut server_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::internal("server params", e))?;
    server_params.subject_alt_names = server_sans;
    server_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "overdrive-control-plane");
        dn
    };
    server_params.key_usages =
        vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .map_err(|e| ControlPlaneError::internal("server sign", e))?;
    let server_leaf_cert_pem = server_cert.pem();
    let server_leaf_key_pem = server_key.serialize_pem();

    // --- Client leaf (local operator per ADR-0010 Phase 1) -----------
    let client_key =
        KeyPair::generate().map_err(|e| ControlPlaneError::internal("client keypair", e))?;
    let mut client_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| ControlPlaneError::internal("client params", e))?;
    client_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "local-operator");
        dn
    };
    client_params.key_usages =
        vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];

    let client_cert = client_params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .map_err(|e| ControlPlaneError::internal("client sign", e))?;
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

/// Internal serde shape for the operator config at
/// `~/.overdrive/config` (ADR-0019 canonical TOML shape).
///
/// `current-context` is a bare TOML key carrying the name of the
/// active context; `contexts` is an array-of-tables where each entry
/// carries its own `name`, `endpoint`, and the base64-PEM trust
/// triple.
///
/// `deny_unknown_fields` is applied to the `*In` deserialisation
/// counterparts below — it only affects `Deserialize`, so putting it
/// on these Serialize-only writer structs would be a no-op. Malformed
/// TOML on load surfaces as a loud parse error via the `*In` structs;
/// per ADR-0019 Consequences → Enforcement the rejection shape
/// matches ADR-0010 §Enforcement (reject any context missing
/// `ca`/`crt`/`key`).
#[derive(Debug, Serialize)]
struct OperatorConfigOut<'a> {
    #[serde(rename = "current-context")]
    current_context: &'static str,
    contexts: Vec<OperatorContextOut<'a>>,
}

#[derive(Debug, Serialize)]
struct OperatorContextOut<'a> {
    name: &'static str,
    endpoint: &'a str,
    ca: String,
    crt: String,
    key: String,
}

/// Write the trust triple to `<config_dir>/.overdrive/config` in the
/// ADR-0019 canonical TOML shape:
///
/// ```toml
/// current-context = "local"
///
/// [[contexts]]
/// name     = "local"
/// endpoint = "https://127.0.0.1:7001"
/// ca       = "<base64 PEM>"
/// crt      = "<base64 PEM>"
/// key      = "<base64 PEM>"
/// ```
///
/// The file is written with mode 0600 on Unix (owner read/write only)
/// to match Talos' `talosconfig` discipline — the client-leaf key is
/// a credential.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the parent directory
/// cannot be created, the file cannot be written, or TOML
/// serialisation fails.
pub fn write_trust_triple(
    config_dir: &Path,
    endpoint: &str,
    material: &CaMaterial,
) -> Result<(), ControlPlaneError> {
    let overdrive_dir = config_dir.join(".overdrive");
    std::fs::create_dir_all(&overdrive_dir).map_err(|e| {
        ControlPlaneError::internal(format!("create_dir_all({})", overdrive_dir.display()), e)
    })?;

    let config_path = overdrive_dir.join("config");

    let doc = OperatorConfigOut {
        current_context: "local",
        contexts: vec![OperatorContextOut {
            name: "local",
            endpoint,
            ca: BASE64.encode(material.ca_cert_pem.as_bytes()),
            crt: BASE64.encode(material.client_leaf_cert_pem.as_bytes()),
            key: BASE64.encode(material.client_leaf_key_pem.as_bytes()),
        }],
    };

    let toml_text =
        toml::to_string(&doc).map_err(|e| ControlPlaneError::internal("toml serialise", e))?;

    write_file_owner_only(&config_path, toml_text.as_bytes())?;

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
        .map_err(|e| ControlPlaneError::internal(format!("open({})", path.display()), e))?;
    file.write_all(bytes)
        .map_err(|e| ControlPlaneError::internal(format!("write({})", path.display()), e))?;
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
        .map_err(|e| ControlPlaneError::internal(format!("write({})", path.display()), e))
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
            .map_err(|e| ControlPlaneError::internal("parse server cert PEM", e))?;
    if cert_chain.is_empty() {
        return Err(ControlPlaneError::Internal(
            "server leaf PEM contained no certificates".into(),
        ));
    }

    // Parse private key from PEM (accepts PKCS#8, PKCS#1, or SEC1).
    let mut key_reader = Cursor::new(material.server_leaf_key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| ControlPlaneError::internal("parse server key PEM", e))?
        .ok_or_else(|| {
            ControlPlaneError::Internal("server key PEM contained no private key".into())
        })?;

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .map_err(|e| ControlPlaneError::internal("rustls with_single_cert", e))?;

    // ADR-0008 §ALPN: prefer h2, fall back to http/1.1.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(config)
}

/// In-memory representation of a loaded trust triple — the
/// counterpart to `write_trust_triple`.
///
/// Fields are decoded PEM byte buffers, NOT base64. Construct via
/// [`load_trust_triple`]; the public accessor methods expose borrowed
/// views so callers cannot mutate the loaded material.
#[derive(Debug, Clone)]
pub struct TrustTriple {
    endpoint: String,
    ca_cert_pem: Vec<u8>,
    client_cert_pem: Vec<u8>,
    client_key_pem: Vec<u8>,
}

impl TrustTriple {
    /// Return the `endpoint` field recorded in the config file.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return the decoded CA certificate PEM bytes.
    #[must_use]
    pub fn ca_cert_pem(&self) -> &[u8] {
        &self.ca_cert_pem
    }

    /// Return the decoded client leaf certificate PEM bytes.
    #[must_use]
    pub fn client_cert_pem(&self) -> &[u8] {
        &self.client_cert_pem
    }

    /// Return the decoded client leaf key PEM bytes.
    #[must_use]
    pub fn client_key_pem(&self) -> &[u8] {
        &self.client_key_pem
    }
}

/// Deserialisation shape for loading the ADR-0019 TOML config —
/// mirror image of `OperatorConfigOut`.
///
/// `deny_unknown_fields` matches ADR-0019 Consequences → Enforcement:
/// malformed input surfaces as a loud parse error rather than a
/// silent coercion. A context missing `ca` / `crt` / `key` fails the
/// TOML parse (each is a required field, not `Option<String>`) rather
/// than surviving into a runtime check — preserving ADR-0010
/// §Enforcement last bullet at the serde layer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorConfigIn {
    #[serde(rename = "current-context")]
    current_context: String,
    contexts: Vec<OperatorContextIn>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorContextIn {
    name: String,
    endpoint: String,
    ca: String,
    crt: String,
    key: String,
}

/// Load and validate a trust triple from the operator config file on
/// disk (ADR-0019 TOML shape). Pairs with [`write_trust_triple`].
///
/// On any parse or decode failure, returns
/// `ControlPlaneError::Internal` whose message names the file path AND
/// the offending field name (`ca`, `crt`, or `key`) so operators can
/// locate and repair the bad config without attaching a debugger.
///
/// # Errors
///
/// - File-not-found, read errors: `Internal` naming the path.
/// - TOML parse errors: `Internal` naming the path.
/// - Missing current-context entry in `contexts`: `Internal` naming
///   the path and the missing context name.
/// - Base64 decode errors on `ca` / `crt` / `key`: `Internal` naming
///   the path and the field.
pub fn load_trust_triple(path: &Path) -> Result<TrustTriple, ControlPlaneError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ControlPlaneError::internal(format!("failed to read trust triple at {}", path.display()), e)
    })?;

    let parsed: OperatorConfigIn = toml::from_str(&text).map_err(|e| {
        ControlPlaneError::internal(
            format!("failed to parse trust triple at {}: invalid TOML", path.display()),
            e,
        )
    })?;

    let ctx = parsed.contexts.iter().find(|c| c.name == parsed.current_context).ok_or_else(
        || {
            ControlPlaneError::Internal(format!(
                "failed to parse trust triple at {}: current-context `{}` not present in `contexts`",
                path.display(),
                parsed.current_context,
            ))
        },
    )?;

    let ca_cert_pem = BASE64.decode(ctx.ca.as_bytes()).map_err(|e| {
        ControlPlaneError::internal(
            format!(
                "failed to parse trust triple at {}: field `ca` is not valid base64",
                path.display()
            ),
            e,
        )
    })?;
    let client_cert_pem = BASE64.decode(ctx.crt.as_bytes()).map_err(|e| {
        ControlPlaneError::internal(
            format!(
                "failed to parse trust triple at {}: field `crt` is not valid base64",
                path.display()
            ),
            e,
        )
    })?;
    let client_key_pem = BASE64.decode(ctx.key.as_bytes()).map_err(|e| {
        ControlPlaneError::internal(
            format!(
                "failed to parse trust triple at {}: field `key` is not valid base64",
                path.display()
            ),
            e,
        )
    })?;

    Ok(TrustTriple { endpoint: ctx.endpoint.clone(), ca_cert_pem, client_cert_pem, client_key_pem })
}

#[cfg(test)]
mod tests {
    //! Unit tests for `tls_bootstrap` — exercise pure paths through
    //! the hostname injection surface. Integration tests for the
    //! real I/O paths (file read, `TempDir`) live under
    //! `tests/integration/tls_bootstrap.rs`.
    //
    // `expect` is the standard idiom for test preconditions — a panic
    // with a message is the correct failure mode when a fixture
    // invariant is violated.
    #![allow(clippy::expect_used)]
    use super::*;
    use std::collections::HashSet;
    use std::io::Cursor;

    /// Test-only `HostnameSource` that always fails with a specified
    /// `io::ErrorKind`. Models the "obscure platform without a
    /// hostname" case the production code must tolerate.
    struct FailingHostnameSource {
        kind: io::ErrorKind,
    }

    impl HostnameSource for FailingHostnameSource {
        fn get(&self) -> io::Result<OsString> {
            Err(io::Error::new(self.kind, "test: hostname unavailable"))
        }
    }

    /// Extract the set of SAN entries from a PEM-encoded leaf cert in
    /// a format comparable across tests. Returns canonical string
    /// forms (`IP:<addr>`, `DNS:<name>`).
    fn san_set_from_pem(pem: &str) -> HashSet<String> {
        use x509_parser::prelude::*;

        let mut reader = Cursor::new(pem.as_bytes());
        let certs: Vec<Vec<u8>> = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .expect("parse PEM certs")
            .into_iter()
            .map(|c| c.as_ref().to_vec())
            .collect();

        let der = certs.first().expect("at least one cert");
        let (_, cert) = X509Certificate::from_der(der).expect("parse DER");

        let san_ext = cert
            .extensions()
            .iter()
            .find_map(|ext| match ext.parsed_extension() {
                ParsedExtension::SubjectAlternativeName(san) => Some(san),
                _ => None,
            })
            .expect("leaf must carry a SAN extension");

        san_ext
            .general_names
            .iter()
            .filter_map(|gn| match gn {
                GeneralName::IPAddress(bytes) => match bytes.len() {
                    4 => Some(format!("IP:{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])),
                    16 if bytes.iter().take(15).all(|b| *b == 0) && bytes[15] == 1 => {
                        Some("IP:::1".to_string())
                    }
                    _ => None,
                },
                GeneralName::DNSName(name) => Some(format!("DNS:{name}")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mint_ephemeral_ca_with_failing_hostname_source_still_produces_three_sans() {
        let failing = FailingHostnameSource { kind: io::ErrorKind::Other };
        let material = mint_ephemeral_ca_with_hostname(&failing)
            .expect("mint must succeed even when hostname::get fails");

        let sans = san_set_from_pem(&material.server_leaf_cert_pem);

        // Exactly the three ADR-0010 §R3 SANs that do not depend on
        // hostname. The fourth (`DNS:<hostname>`) is dropped because
        // the injected source returned Err.
        let expected: HashSet<String> =
            ["IP:127.0.0.1".to_string(), "IP:::1".to_string(), "DNS:localhost".to_string()]
                .into_iter()
                .collect();

        assert_eq!(
            sans, expected,
            "failing-hostname path must retain loopback SANs, nothing else \
             (got {sans:?})",
        );
        assert_eq!(sans.len(), 3, "failing hostname must produce EXACTLY three SANs, not four");
    }

    #[test]
    fn mint_ephemeral_ca_with_failing_hostname_source_produces_parseable_material() {
        // Positive control for the degradation path — a server leaf
        // with only three SANs must still be valid PEM that parses
        // through `rustls_pemfile`. If this fails, the degradation
        // broke the cert entirely; if it passes, the three-SAN leaf
        // is usable material.
        let failing = FailingHostnameSource { kind: io::ErrorKind::PermissionDenied };
        let material = mint_ephemeral_ca_with_hostname(&failing).expect("mint must succeed");

        let mut cert_reader = Cursor::new(material.server_leaf_cert_pem.as_bytes());
        let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .expect("server leaf must parse as PEM");
        assert!(!certs.is_empty(), "degraded leaf must contain at least one cert");

        let mut key_reader = Cursor::new(material.server_leaf_key_pem.as_bytes());
        let key =
            rustls_pemfile::private_key(&mut key_reader).expect("server key must parse as PEM");
        assert!(key.is_some(), "degraded leaf must carry a parseable key");
    }
}
