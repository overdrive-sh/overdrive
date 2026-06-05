//! Integration — `RcgenCa` real X.509 over `openssl verify` (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 3 (real crypto, real `openssl verify` subprocess; gated
//! `integration-tests`, runs via Lima per `.claude/rules/testing.md`).
//! `RcgenCa` (`overdrive-host`, `adapter-host`, ADR-0063 D1) owns ALL
//! rcgen / crypto-backend usage. The workspace pins `rcgen = { version =
//! "0.14", features = ["ring", "pem"] }` (Cargo.toml) and `mint_ephemeral_ca`
//! in `tls_bootstrap.rs` already exercises the adjacent 0.14 builder APIs
//! (`Issuer::from_params`, `params.signed_by(&key, &issuer)`, `SanType`,
//! `KeyUsagePurpose`, `IsCa`, P-256) — so this is re-shaping proven code
//! behind the `Ca` port trait, not discovering new crypto.
//!
//! This file is where KPI K1 (chain verifies) and K2 (spec-compliant SAN)
//! are PROVEN against REAL certificate bytes — the headline
//! walking-skeleton proof. Per Mandate 11 these layer-3 tests are
//! EXAMPLE-ONLY (one example per behaviour / failure mode); no PBT
//! machinery — sad paths are enumerated explicitly.
//!
//! Tooling: `openssl verify` is the standard-tool gate (subprocess);
//! `x509-parser = "0.18"` (in-graph) inspects extensions where the AC
//! needs byte-level cert assertions (CA bit, SAN cardinality, keyUsage
//! critical).
//!
//! Scenarios trace to the slice walking-skeleton ACs: S-01 (root
//! self-verifies), S-03 (intermediate chains, pathLen=0 enforced),
//! S-04 (full 3-tier chain verifies, single-URI SAN).
//! Tags: `@real-io` `@adapter-integration` `@walking_skeleton` (S-04 chain)
//! · `@real-io` `@adapter-integration` `@S-NN` (others).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt
//! `RcgenCa`. DELIVER replaces with real `RcgenCa` calls + `openssl verify`
//! subprocess assertions.

use std::process::Command;
use std::sync::Arc;

use overdrive_core::SpiffeId;
use overdrive_core::traits::ca::Ca;
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use x509_parser::prelude::FromDer;

// ---------------------------------------------------------------------------
// S-01 — Root CA self-verifies (US-CA-01)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-01` — KPI K1: `RcgenCa::root()`
/// produces a real P-256 self-signed root that `openssl verify -CAfile
/// root.pem root.pem` accepts (exit 0). x509-parser confirms CA:TRUE,
/// keyCertSign set, keyUsage marked critical.
#[test]
fn rcgen_root_is_a_valid_self_signed_ca_via_openssl_verify() {
    // GIVEN a host CA over a real OS entropy source and a trust-domain subject.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);

    // WHEN the persistent self-signed root is produced (driving port).
    let root = ca.root().expect("RcgenCa::root() self-signs a real P-256 root");

    // THEN `openssl verify -CAfile root.pem root.pem` accepts the cert (exit 0)
    // — KPI K1, the real-tool gate on the real bytes.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_pem_path = dir.path().join("root.pem");
    std::fs::write(&root_pem_path, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");

    let status = Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem_path)
        .arg(&root_pem_path)
        .status()
        .expect("invoke openssl verify");
    assert!(status.success(), "openssl verify -CAfile root.pem root.pem must exit 0");

    // AND x509-parser confirms the root profile on the REAL cert bytes:
    // CA:TRUE, keyCertSign set, keyUsage marked critical (research Finding 2).
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(root.cert_der().as_der())
        .expect("parse root DER");

    let bc = cert
        .basic_constraints()
        .expect("basicConstraints parses")
        .expect("basicConstraints present");
    assert!(bc.value.ca, "root must be CA:TRUE");
    assert_eq!(bc.value.path_len_constraint, None, "root carries NO pathLen");

    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    assert!(ku.critical, "keyUsage extension must be marked critical");
    assert!(ku.value.key_cert_sign(), "root carries keyCertSign");
    assert!(ku.value.crl_sign(), "root carries cRLSign");

    // AND a self-signed root is its own issuer.
    assert_eq!(
        cert.issuer().to_string(),
        cert.subject().to_string(),
        "a self-signed root's issuer equals its subject"
    );
}

// ---------------------------------------------------------------------------
// S-03 — Intermediate chains to root, pathLen=0 enforced (US-CA-03)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-03` — `RcgenCa::issue_intermediate`
/// produces a real intermediate signed by the root; `openssl verify
/// -CAfile root.pem intermediate.pem` exits 0. x509-parser confirms
/// CA:TRUE, pathLenConstraint=0, keyCertSign, keyUsage critical.
#[test]
#[should_panic(expected = "RED scaffold")]
fn rcgen_intermediate_chains_to_root_via_openssl_verify() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / RcgenCa intermediate chains to root: \
         openssl verify -CAfile root.pem intermediate.pem exits 0; CA:TRUE + pathLen=0)"
    );
}

/// `@real-io` `@adapter-integration` `@S-03` `@error` — pathLen=0 is
/// ENFORCED not merely set: a constructed chain in which the intermediate
/// signs a FURTHER CA cert fails `openssl verify` (the constraint bounds
/// node-compromise blast radius — research Finding 4). Sad path, example-based.
#[test]
#[should_panic(expected = "RED scaffold")]
fn rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / a chain where the pathLen=0 intermediate \
         signs a further CA fails openssl verify; constraint enforced, not merely set)"
    );
}

// ---------------------------------------------------------------------------
// S-04 — Full 3-tier SVID chain verifies, single URI SAN (US-CA-04)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@walking_skeleton` `@S-04` — KPI K1
/// (THE headline walking-skeleton proof, D2 completion): a real workload
/// SVID minted by `RcgenCa::issue_svid` for
/// `spiffe://overdrive.local/job/payments/alloc/a1b2c3` chain-verifies
/// through the full hierarchy: `openssl verify -CAfile root.pem -untrusted
/// intermediate.pem svid.pem` exits 0.
///
/// Walking-skeleton litmus (Dim 5): Sam, the security engineer, can run
/// `openssl verify` himself and confirm the workload identity validates to
/// the root — the genuine user-observable outcome (no operator CLI verb
/// exists; `openssl verify` is the honest external entry point per the
/// DISCUSS elevator-pitch caveat).
#[test]
#[should_panic(expected = "RED scaffold")]
fn rcgen_full_svid_chain_verifies_root_intermediate_svid() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / walking skeleton: full Root -> Intermediate \
         -> SVID chain verifies: openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem exits 0)"
    );
}

/// `@real-io` `@adapter-integration` `@S-04` — KPI K2: the real SVID leaf
/// carries EXACTLY ONE URI SAN equal to the requested `SpiffeId`, CA:FALSE,
/// keyUsage=digitalSignature marked critical, NO keyCertSign/cRLSign, and a
/// ~1h validity window. x509-parser inspects the real cert bytes.
#[test]
#[should_panic(expected = "RED scaffold")]
fn rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / real SVID leaf: exactly one URI SAN = \
         spiffe://overdrive.local/job/payments/alloc/a1b2c3, CA:FALSE, digitalSignature critical, \
         no keyCertSign/cRLSign, ~1h TTL)"
    );
}

/// `@real-io` `@adapter-integration` `@S-04` `@error` — KPI K2 rejection
/// path at the real adapter: a `SvidRequest` whose `SpiffeId` would yield 0
/// or >=2 URI SANs is rejected by `RcgenCa::issue_svid` with
/// `CaError::InvalidSan` BEFORE any certificate bytes are produced (the
/// core `CertSpec` guard, D5, fires in the host adapter too). Sad path,
/// example-based.
#[test]
#[should_panic(expected = "RED scaffold")]
fn rcgen_svid_request_with_bad_san_cardinality_is_rejected_pre_issuance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / RcgenCa::issue_svid rejects 0 or >=2 URI-SAN \
         request with CaError::InvalidSan before producing any cert bytes)"
    );
}
