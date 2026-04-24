//! Integration tests for the ephemeral CA + trust triple bootstrap
//! implementation (ADR-0010, step 02-01).
//!
//! Each `#[test]` drives the `mint_ephemeral_ca` / `write_trust_triple`
//! driving ports and asserts observable outcomes:
//!
//! * The five PEM fields are populated and parse through `rustls_pemfile`.
//! * The server leaf cert carries EXACTLY the four SANs listed in
//!   ADR-0010 §R3: `IP:127.0.0.1`, `IP:::1`, `DNS:localhost`,
//!   `DNS:<hostname::get()>`.
//! * The on-disk config file is Talos-shape YAML that round-trips
//!   through `serde_yaml` and base64-decodes back to the original PEM
//!   bytes (ADR-0010 §R2).
//! * Re-running `mint_ephemeral_ca` produces different material bytewise
//!   without prompting — the function signature is the port-level proof
//!   that no stdin is consumed (no parameters).
//!
//! Tests live under `tests/integration/` because every case opens real
//! files in a `TempDir`. The entrypoint at `tests/integration.rs`
//! gates the whole binary behind the `integration-tests` feature; this
//! module inherits the gate and does not repeat the cfg attribute.

use std::collections::HashSet;
use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use overdrive_control_plane::tls_bootstrap::{CaMaterial, mint_ephemeral_ca, write_trust_triple};
use serde::Deserialize;
use tempfile::TempDir;
use x509_parser::prelude::*;

/// Minimal serde shape for the Talos-style `~/.overdrive/config`. This
/// intentionally ignores unknown fields so ADR-0010 can extend the
/// schema without breaking the assertion surface.
#[derive(Debug, Deserialize)]
struct TalosConfig {
    context: String,
    contexts: std::collections::BTreeMap<String, TalosContext>,
}

#[derive(Debug, Deserialize)]
struct TalosContext {
    endpoint: String,
    ca: String,
    crt: String,
    key: String,
}

/// Helper — parse one or more PEM certificates from a buffer; asserts
/// non-empty. Returns the raw DER of the first cert, which downstream
/// assertions hand to `x509-parser`.
fn assert_parseable_certs(label: &str, pem: &str) -> Vec<Vec<u8>> {
    let mut reader = Cursor::new(pem.as_bytes());
    let certs: Vec<Vec<u8>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("{label} failed to parse as PEM certs: {e}"))
        .into_iter()
        .map(|c| c.as_ref().to_vec())
        .collect();
    assert!(!certs.is_empty(), "{label} produced zero certs — expected at least one");
    certs
}

/// Helper — parse a PKCS#8 / RSA / SEC1 private key from a PEM buffer.
fn assert_parseable_key(label: &str, pem: &str) {
    let mut reader = Cursor::new(pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut reader)
        .unwrap_or_else(|e| panic!("{label} failed to parse as PEM key: {e}"));
    assert!(key.is_some(), "{label} did not contain a recognisable private key");
}

#[test]
fn mint_ephemeral_ca_returns_material_with_all_five_pems() {
    let material: CaMaterial =
        mint_ephemeral_ca().expect("mint_ephemeral_ca must succeed on a freshly-provisioned host");

    assert_parseable_certs("ca_cert_pem", &material.ca_cert_pem);
    assert_parseable_certs("server_leaf_cert_pem", &material.server_leaf_cert_pem);
    assert_parseable_key("server_leaf_key_pem", &material.server_leaf_key_pem);
    assert_parseable_certs("client_leaf_cert_pem", &material.client_leaf_cert_pem);
    assert_parseable_key("client_leaf_key_pem", &material.client_leaf_key_pem);
}

#[test]
fn server_leaf_cert_has_exactly_four_san_entries_per_adr_0010_r3() {
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca must succeed");

    let der_list = assert_parseable_certs("server_leaf_cert_pem", &material.server_leaf_cert_pem);
    let der = der_list.first().expect("server leaf PEM must contain at least one cert");

    let (_, cert) =
        X509Certificate::from_der(der).expect("server leaf DER must parse via x509-parser");

    // Collect all SAN entries regardless of kind into a set of string
    // forms. ADR-0010 §R3 specifies four entries by semantic identity;
    // we canonicalise every entry so duplicates collapse.
    let san_extension = cert
        .extensions()
        .iter()
        .find_map(|ext| match ext.parsed_extension() {
            ParsedExtension::SubjectAlternativeName(san) => Some(san),
            _ => None,
        })
        .expect("server leaf MUST carry a subjectAltName extension per ADR-0010 §R3");

    let sans: HashSet<String> = san_extension
        .general_names
        .iter()
        .filter_map(|gn| match gn {
            GeneralName::IPAddress(bytes) => match bytes.len() {
                4 => Some(format!(
                    "IP:{}.{}.{}.{}",
                    bytes[0], bytes[1], bytes[2], bytes[3]
                )),
                16 => {
                    // Canonicalise IPv6 loopback `::1` — 15 zero bytes + 0x01.
                    let is_loopback =
                        bytes.iter().take(15).all(|b| *b == 0) && bytes[15] == 1;
                    Some(if is_loopback {
                        "IP:::1".to_string()
                    } else {
                        format!(
                            "IP:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}",
                            bytes[0], bytes[1], bytes[2], bytes[3],
                            bytes[4], bytes[5], bytes[6], bytes[7],
                            bytes[8], bytes[9], bytes[10], bytes[11],
                            bytes[12], bytes[13], bytes[14], bytes[15],
                        )
                    })
                }
                _ => None,
            },
            GeneralName::DNSName(name) => Some(format!("DNS:{name}")),
            _ => None,
        })
        .collect();

    let host = hostname::get()
        .expect("hostname must resolve on the test host")
        .to_string_lossy()
        .to_string();
    let expected: HashSet<String> = [
        "IP:127.0.0.1".to_string(),
        "IP:::1".to_string(),
        "DNS:localhost".to_string(),
        format!("DNS:{host}"),
    ]
    .into_iter()
    .collect();

    assert_eq!(
        sans, expected,
        "server leaf SANs must be EXACTLY the ADR-0010 §R3 set — no more, no fewer"
    );
    assert_eq!(
        sans.len(),
        4,
        "server leaf must carry exactly four SAN entries (got {})",
        sans.len()
    );
}

#[test]
fn write_trust_triple_creates_config_in_talos_yaml_shape() {
    let tmp = TempDir::new().expect("TempDir");
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");
    let endpoint = "https://127.0.0.1:7001";

    write_trust_triple(tmp.path(), endpoint, &material).expect("write_trust_triple must succeed");

    let config_path = tmp.path().join(".overdrive").join("config");
    assert!(
        config_path.is_file(),
        "~/.overdrive/config must be a regular file at {}",
        config_path.display()
    );

    let bytes = std::fs::read(&config_path).expect("read config");
    let parsed: TalosConfig = serde_yaml::from_slice(&bytes).expect(
        "config must parse as Talos-shape YAML per ADR-0010 §R2 (keys: \
         context / contexts / endpoint / ca / crt / key)",
    );

    assert_eq!(parsed.context, "local", "current-context must be `local`");
    let ctx = parsed.contexts.get("local").expect("contexts must include the `local` context");
    assert_eq!(
        ctx.endpoint, endpoint,
        "endpoint field must round-trip the value passed to write_trust_triple"
    );
    assert!(!ctx.ca.is_empty(), "ca field must be populated");
    assert!(!ctx.crt.is_empty(), "crt field must be populated");
    assert!(!ctx.key.is_empty(), "key field must be populated");
}

#[test]
fn trust_triple_base64_fields_decode_to_original_pem_bytes() {
    let tmp = TempDir::new().expect("TempDir");
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");
    write_trust_triple(tmp.path(), "https://127.0.0.1:7001", &material)
        .expect("write_trust_triple");

    let bytes = std::fs::read(tmp.path().join(".overdrive").join("config")).expect("read config");
    let parsed: TalosConfig = serde_yaml::from_slice(&bytes).expect("parse yaml");
    let ctx = parsed.contexts.get("local").expect("local context");

    let ca_decoded =
        BASE64.decode(ctx.ca.as_bytes()).expect("ca field must be valid base64 per ADR-0010 §R2");
    assert_eq!(
        ca_decoded,
        material.ca_cert_pem.as_bytes(),
        "base64-decoded `ca` must equal the CA cert PEM bytes"
    );

    let crt_decoded = BASE64.decode(ctx.crt.as_bytes()).expect("crt field must be valid base64");
    assert_eq!(
        crt_decoded,
        material.client_leaf_cert_pem.as_bytes(),
        "base64-decoded `crt` must equal the client leaf cert PEM bytes"
    );

    let key_decoded = BASE64.decode(ctx.key.as_bytes()).expect("key field must be valid base64");
    assert_eq!(
        key_decoded,
        material.client_leaf_key_pem.as_bytes(),
        "base64-decoded `key` must equal the client leaf key PEM bytes"
    );
}

#[test]
fn re_minting_produces_different_ca_material_without_prompting() {
    // The function signature takes no input — the "no prompt" property
    // is load-bearing in the type system, not an assertion. What we
    // DO assert is that successive mints produce distinct CA material,
    // proving ephemerality (ADR-0010 §R1 — no CA key persistence).
    let first = mint_ephemeral_ca().expect("first mint");
    let second = mint_ephemeral_ca().expect("second mint");

    assert_ne!(
        first.ca_cert_pem, second.ca_cert_pem,
        "re-mint must produce a bytewise-different CA cert (ADR-0010 §R1)"
    );
    assert_ne!(
        first.server_leaf_key_pem, second.server_leaf_key_pem,
        "re-mint must produce a bytewise-different server leaf key"
    );
    assert_ne!(
        first.client_leaf_key_pem, second.client_leaf_key_pem,
        "re-mint must produce a bytewise-different client leaf key"
    );
}

#[test]
fn write_trust_triple_is_idempotent_overwrite() {
    let tmp = TempDir::new().expect("TempDir");

    let first = mint_ephemeral_ca().expect("first mint");
    write_trust_triple(tmp.path(), "https://127.0.0.1:7001", &first).expect("first write");
    let first_bytes =
        std::fs::read(tmp.path().join(".overdrive").join("config")).expect("first read");

    let second = mint_ephemeral_ca().expect("second mint");
    write_trust_triple(tmp.path(), "https://127.0.0.1:7001", &second).expect("second write");
    let second_bytes =
        std::fs::read(tmp.path().join(".overdrive").join("config")).expect("second read");

    assert_ne!(
        first_bytes, second_bytes,
        "second write must overwrite the first — config must reflect the \
         latest minted material, not a merged state"
    );

    // Prove overwrite semantics: decoded `ca` of the on-disk file must
    // equal the SECOND material, not the first.
    let parsed: TalosConfig = serde_yaml::from_slice(&second_bytes).expect("parse yaml");
    let ctx = parsed.contexts.get("local").expect("local context");
    let ca_decoded = BASE64.decode(ctx.ca.as_bytes()).expect("base64");
    assert_eq!(
        ca_decoded,
        second.ca_cert_pem.as_bytes(),
        "overwritten config must carry the SECOND CA, not the first"
    );
}
