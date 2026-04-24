//! Acceptance tests for `TrustTriple::{endpoint, ca_cert_pem,
//! client_cert_pem, client_key_pem}` — the accessors exposed by
//! `overdrive_control_plane::tls_bootstrap`.
//!
//! The getters are load-bearing: `overdrive-cli`'s `ApiClient` reads
//! `triple.endpoint()` to set the base URL and the three PEM byte
//! slices to pin the CA root and client identity for the reqwest
//! rustls stack. Mutations that replace these accessors with empty
//! strings / `"xyzzy"` / `Vec::leak(vec![0|1])` silently degrade every
//! downstream TLS handshake.
//!
//! The integration suite (`tests/integration/tls_bootstrap.rs`) runs a
//! real axum+rustls handshake and catches the full chain, but that
//! lane is gated by `integration-tests` and skipped by the default
//! cargo-mutants run. These acceptance tests mirror the getter
//! assertions with an in-process `write_trust_triple` +
//! `load_trust_triple` round-trip (`TempDir` + synchronous filesystem
//! I/O only, no network, no TLS handshake) so the getter contract is
//! exercised in the default lane.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use overdrive_control_plane::tls_bootstrap::{
    load_trust_triple, mint_ephemeral_ca, write_trust_triple,
};
use tempfile::TempDir;

const TEST_ENDPOINT: &str = "https://127.0.0.1:7001";

/// Mint a CA + write + load a trust triple from disk. Returns the
/// loaded triple alongside the original CA material so each getter
/// assertion can compare against the source bytes.
fn load_round_trip() -> (
    overdrive_control_plane::tls_bootstrap::TrustTriple,
    overdrive_control_plane::tls_bootstrap::CaMaterial,
    TempDir,
) {
    let tmp = TempDir::new().expect("TempDir");
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");
    write_trust_triple(tmp.path(), TEST_ENDPOINT, &material).expect("write_trust_triple");

    let config_path = tmp.path().join(".overdrive").join("config");
    let triple = load_trust_triple(&config_path).expect("load_trust_triple");
    (triple, material, tmp)
}

// ---------------------------------------------------------------------------
// endpoint()
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_endpoint_returns_the_endpoint_recorded_on_disk() {
    let (triple, _material, _tmp) = load_round_trip();

    // Pin the EXACT endpoint string passed to write_trust_triple. A
    // mutation that returns "" or "xyzzy" would fail; so would any
    // future drift that mangles the endpoint through normalisation.
    assert_eq!(
        triple.endpoint(),
        TEST_ENDPOINT,
        "TrustTriple::endpoint must round-trip the exact string written by \
         write_trust_triple; mutation to \"\" or \"xyzzy\" must fail here",
    );
    assert!(!triple.endpoint().is_empty(), "endpoint must not be empty");
    assert!(
        triple.endpoint().starts_with("https://"),
        "endpoint must preserve the https:// scheme; got `{}`",
        triple.endpoint(),
    );
}

#[test]
fn trust_triple_endpoint_round_trips_alternate_endpoint() {
    // Second endpoint — different bytes — proves the getter is not a
    // constant. Catches a mutation like
    // `endpoint -> &str with "xyzzy"` even if "xyzzy" happened to
    // equal the first test's endpoint (it doesn't, but defensive).
    let tmp = TempDir::new().expect("TempDir");
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");
    let alt_endpoint = "https://overdrive.example.com:8443";
    write_trust_triple(tmp.path(), alt_endpoint, &material).expect("write_trust_triple");

    let config_path = tmp.path().join(".overdrive").join("config");
    let triple = load_trust_triple(&config_path).expect("load_trust_triple");

    assert_eq!(
        triple.endpoint(),
        alt_endpoint,
        "distinct endpoint input must round-trip distinctly; a constant \
         endpoint getter would fail here",
    );
}

// ---------------------------------------------------------------------------
// ca_cert_pem()
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_ca_cert_pem_returns_the_decoded_ca_pem_bytes() {
    let (triple, material, _tmp) = load_round_trip();

    let ca = triple.ca_cert_pem();

    // Exact byte match against the source CA PEM. A mutation that
    // returns `Vec::leak(Vec::new())` / `vec![0]` / `vec![1]` would
    // fail here because none of those byte sequences equal the
    // source PEM.
    assert_eq!(
        ca,
        material.ca_cert_pem.as_bytes(),
        "ca_cert_pem() must return the decoded CA PEM bytes byte-for-byte",
    );

    // PEM shape validation — catches "xyzzy" / `Vec::leak(vec![1])`
    // mutations that slipped through byte-equality (they wouldn't,
    // but the shape check is defensive and cheap).
    assert!(!ca.is_empty(), "ca_cert_pem must not be empty");
    assert!(
        ca.starts_with(b"-----BEGIN "),
        "ca_cert_pem must begin with PEM armour; got first 16 bytes = {:?}",
        &ca[..ca.len().min(16)],
    );
    // A `Vec::leak(vec![0])` mutation is a single byte — explicitly
    // reject any slice too short to be a PEM blob.
    assert!(
        ca.len() > 32,
        "ca_cert_pem must be more than 32 bytes (a PEM cert is hundreds of bytes at minimum); got {}",
        ca.len(),
    );
}

// ---------------------------------------------------------------------------
// client_cert_pem()
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_client_cert_pem_returns_the_decoded_client_cert_pem_bytes() {
    let (triple, material, _tmp) = load_round_trip();

    let crt = triple.client_cert_pem();
    assert_eq!(
        crt,
        material.client_leaf_cert_pem.as_bytes(),
        "client_cert_pem() must return the decoded client leaf cert bytes byte-for-byte",
    );
    assert!(!crt.is_empty(), "client_cert_pem must not be empty");
    assert!(crt.starts_with(b"-----BEGIN "), "client_cert_pem must begin with PEM armour");
    assert!(crt.len() > 32, "client_cert_pem must be more than 32 bytes");
}

// ---------------------------------------------------------------------------
// client_key_pem()
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_client_key_pem_returns_the_decoded_client_key_pem_bytes() {
    let (triple, material, _tmp) = load_round_trip();

    let key = triple.client_key_pem();
    assert_eq!(
        key,
        material.client_leaf_key_pem.as_bytes(),
        "client_key_pem() must return the decoded client leaf key bytes byte-for-byte",
    );
    assert!(!key.is_empty(), "client_key_pem must not be empty");
    assert!(key.starts_with(b"-----BEGIN "), "client_key_pem must begin with PEM armour");
    assert!(key.len() > 32, "client_key_pem must be more than 32 bytes");
}

// ---------------------------------------------------------------------------
// Accessors return DISTINCT material — proves no getter is returning
// a shared constant (e.g. if three getters collapsed onto the same
// body).
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_accessors_return_distinct_material() {
    let (triple, _material, _tmp) = load_round_trip();

    let ca = triple.ca_cert_pem();
    let crt = triple.client_cert_pem();
    let key = triple.client_key_pem();

    // CA != client cert, CA != client key, cert != key. Catches a
    // mutation that makes two getters alias the same field.
    assert_ne!(ca, crt, "CA cert and client cert must be distinct bytes");
    assert_ne!(ca, key, "CA cert and client key must be distinct bytes");
    assert_ne!(crt, key, "client cert and client key must be distinct bytes");
}

// ---------------------------------------------------------------------------
// The raw base64 field mapping is an implicit invariant of
// write_trust_triple; reaffirming it in the acceptance lane (rather
// than the integration lane) catches a Talos-shape drift.
// ---------------------------------------------------------------------------

#[test]
fn trust_triple_raw_base64_fields_map_to_ca_crt_key_in_that_order() {
    use serde::Deserialize;
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OperatorConfig {
        #[serde(rename = "current-context")]
        current_context: String,
        contexts: Vec<OperatorContext>,
    }
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OperatorContext {
        name: String,
        endpoint: String,
        ca: String,
        crt: String,
        key: String,
    }

    let (triple, material, tmp) = load_round_trip();

    let config_path = tmp.path().join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path).expect("read config");
    let doc: OperatorConfig = toml::from_str(&text).expect("parse ADR-0019 TOML");
    let ctx = doc
        .contexts
        .iter()
        .find(|c| c.name == doc.current_context)
        .expect("current context present");

    // The ADR-0019 TOML fields must map to the three PEM components in
    // ca / crt / key order. A mutation that swaps fields would be
    // caught here.
    assert_eq!(
        BASE64.decode(ctx.ca.as_bytes()).expect("decode ca"),
        material.ca_cert_pem.as_bytes(),
        "`ca` field must round-trip to the CA cert PEM",
    );
    assert_eq!(
        BASE64.decode(ctx.crt.as_bytes()).expect("decode crt"),
        material.client_leaf_cert_pem.as_bytes(),
        "`crt` field must round-trip to the client leaf cert PEM",
    );
    assert_eq!(
        BASE64.decode(ctx.key.as_bytes()).expect("decode key"),
        material.client_leaf_key_pem.as_bytes(),
        "`key` field must round-trip to the client leaf key PEM",
    );
    assert_eq!(ctx.endpoint, triple.endpoint(), "endpoint field consistency");
}
