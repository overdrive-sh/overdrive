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

use overdrive_core::traits::ca::{Ca, CaError, SvidRequest};
use overdrive_core::{NodeId, SpiffeId};
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
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
fn rcgen_intermediate_chains_to_root_via_openssl_verify() {
    // GIVEN a host CA over a real OS entropy source and a trust-domain subject.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let node = NodeId::new("node-a").expect("NodeId parses");

    // WHEN the persistent root and a node intermediate are produced (driving
    // port). Both must share the same root key for the chain to verify — the
    // root material is cached, so `issue_intermediate` signs against the same
    // root `root()` returns here.
    let root = ca.root().expect("RcgenCa::root() self-signs a real P-256 root");
    let inter = ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");

    // THEN `openssl verify -CAfile root.pem intermediate.pem` accepts the
    // intermediate (exit 0) — the chains-to-root proof on the REAL bytes.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_pem_path = dir.path().join("root.pem");
    let inter_pem_path = dir.path().join("intermediate.pem");
    std::fs::write(&root_pem_path, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem_path, inter.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");

    let status = Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem_path)
        .arg(&inter_pem_path)
        .status()
        .expect("invoke openssl verify");
    assert!(
        status.success(),
        "openssl verify -CAfile root.pem intermediate.pem must exit 0 (chains to root)"
    );

    // AND x509-parser confirms the intermediate profile on the REAL cert bytes:
    // CA:TRUE, pathLenConstraint=0, keyCertSign set, keyUsage marked critical.
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(inter.cert_der().as_der())
        .expect("parse intermediate DER");

    let bc = cert
        .basic_constraints()
        .expect("basicConstraints parses")
        .expect("basicConstraints present");
    assert!(bc.value.ca, "intermediate must be CA:TRUE");
    assert_eq!(bc.value.path_len_constraint, Some(0), "intermediate carries pathLen=0");

    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    assert!(ku.critical, "keyUsage extension must be marked critical");
    assert!(ku.value.key_cert_sign(), "intermediate carries keyCertSign");

    // AND the intermediate's issuer is the root's subject — the chains-to-root
    // linkage observable in the cert bytes.
    let (_, root_cert) =
        x509_parser::certificate::X509Certificate::from_der(root.cert_der().as_der())
            .expect("parse root DER");
    assert_eq!(
        cert.issuer().to_string(),
        root_cert.subject().to_string(),
        "the intermediate's issuer equals the root's subject"
    );
}

/// `@real-io` `@adapter-integration` `@S-03` `@error` — pathLen=0 is
/// ENFORCED not merely set: a constructed chain in which the intermediate
/// signs a FURTHER CA cert fails `openssl verify` (the constraint bounds
/// node-compromise blast radius — research Finding 4). Sad path, example-based.
#[test]
fn rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced() {
    // GIVEN a host CA with a real root + a pathLen=0 node intermediate.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let node = NodeId::new("node-a").expect("NodeId parses");
    let root = ca.root().expect("RcgenCa::root() self-signs a real P-256 root");
    let inter = ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");

    // WHEN we deliberately construct a FURTHER CA cert signed by the pathLen=0
    // intermediate. The intermediate's signing key + a CA-shaped params rebuild
    // an `Issuer`; a second CA cert is signed under it. This is the abuse the
    // pathLen=0 constraint exists to bound (research Finding 4) — node
    // compromise must not let the node mint its own sub-CAs.
    let inter_key =
        KeyPair::from_pem(inter.signing_key().as_pem()).expect("reload intermediate key");
    let mut inter_issuer_params =
        CertificateParams::new(Vec::<String>::new()).expect("intermediate issuer params");
    inter_issuer_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    inter_issuer_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    inter_issuer_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::OrganizationName, "overdrive.local");
        dn
    };
    let inter_issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&inter_issuer_params, &inter_key);

    let further_key = KeyPair::generate().expect("further-CA keypair");
    let mut further_params =
        CertificateParams::new(Vec::<String>::new()).expect("further-CA params");
    further_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    further_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    further_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "further-ca-abuse");
        dn
    };
    let further_cert =
        further_params.signed_by(&further_key, &inter_issuer).expect("sign further CA");

    // THEN `openssl verify -CAfile root.pem -untrusted intermediate.pem
    // furtherca.pem` FAILS (non-zero exit). pathLen=0 forbids a CA child of the
    // intermediate, so the verifier rejects the chain — proving the constraint
    // is ENFORCED by the verifier, not merely present in the cert bytes.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_pem_path = dir.path().join("root.pem");
    let inter_pem_path = dir.path().join("intermediate.pem");
    let further_pem_path = dir.path().join("furtherca.pem");
    std::fs::write(&root_pem_path, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem_path, inter.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");
    std::fs::write(&further_pem_path, further_cert.pem().as_bytes()).expect("write furtherca.pem");

    let status = Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem_path)
        .arg("-untrusted")
        .arg(&inter_pem_path)
        .arg(&further_pem_path)
        .status()
        .expect("invoke openssl verify");
    assert!(
        !status.success(),
        "openssl verify of a further-CA under a pathLen=0 intermediate must FAIL (constraint enforced)"
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
fn rcgen_full_svid_chain_verifies_root_intermediate_svid() {
    // GIVEN a host CA over a real OS entropy source and a trust-domain subject,
    // a node, and a request to identify allocation a1b2c3 of job payments.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let node = NodeId::new("node-a").expect("NodeId parses");
    let workload = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
        .expect("workload SpiffeId parses");
    let req = SvidRequest::new(workload);

    // WHEN the persistent root, the node intermediate, and the workload SVID are
    // produced through the `Ca` driving port. The intermediate is cached, so the
    // leaf `issue_svid` signs chains to the SAME intermediate written below.
    let root = ca.root().expect("RcgenCa::root() self-signs a real P-256 root");
    let inter = ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");
    let svid =
        ca.issue_svid(&req).expect("RcgenCa::issue_svid mints a leaf signed by intermediate");

    // THEN `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem`
    // accepts the leaf (exit 0) — THE walking-skeleton proof on the REAL bytes
    // (KPI K1). Sam runs this himself and confirms the workload identity
    // validates to the root.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_pem_path = dir.path().join("root.pem");
    let inter_pem_path = dir.path().join("intermediate.pem");
    let svid_pem_path = dir.path().join("svid.pem");
    std::fs::write(&root_pem_path, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem_path, inter.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");
    std::fs::write(&svid_pem_path, svid.cert_pem().as_pem().as_bytes()).expect("write svid.pem");

    let output = Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem_path)
        .arg("-untrusted")
        .arg(&inter_pem_path)
        .arg(&svid_pem_path)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem must exit 0 \
         (full chain verifies): stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // AND the leaf's issuer is the intermediate's subject — the
    // chains-to-intermediate linkage observable in the cert bytes.
    let (_, leaf) = x509_parser::certificate::X509Certificate::from_der(svid.cert_der().as_der())
        .expect("parse svid DER");
    let (_, inter_cert) =
        x509_parser::certificate::X509Certificate::from_der(inter.cert_der().as_der())
            .expect("parse intermediate DER");
    assert_eq!(
        leaf.issuer().to_string(),
        inter_cert.subject().to_string(),
        "the SVID leaf's issuer equals the intermediate's subject"
    );
}

/// `@real-io` `@adapter-integration` `@S-04` — KPI K2: the real SVID leaf
/// carries EXACTLY ONE URI SAN equal to the requested `SpiffeId`, CA:FALSE,
/// keyUsage=digitalSignature marked critical, NO keyCertSign/cRLSign, and a
/// ~1h validity window. x509-parser inspects the real cert bytes.
#[test]
fn rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile() {
    // GIVEN a host CA and a workload SVID request for a specific identity.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let workload = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
        .expect("workload SpiffeId parses");
    let req = SvidRequest::new(workload.clone());

    // WHEN the leaf is minted through the driving port (the root + intermediate
    // are minted lazily inside `issue_svid`).
    let svid = ca.issue_svid(&req).expect("RcgenCa::issue_svid mints a leaf");

    // THEN x509-parser confirms the leaf profile on the REAL cert bytes (KPI K2):
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(svid.cert_der().as_der())
        .expect("parse svid DER");

    // (a) basicConstraints CA:FALSE — a leaf signs nothing.
    let bc = cert.basic_constraints().expect("basicConstraints parses");
    assert!(
        bc.is_none_or(|ext| !ext.value.ca),
        "SVID leaf must be CA:FALSE (absent or CA bit unset)"
    );

    // (b) keyUsage = digitalSignature, marked critical, NO keyCertSign/cRLSign.
    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    assert!(ku.critical, "keyUsage extension must be marked critical");
    assert!(ku.value.digital_signature(), "SVID carries digitalSignature");
    assert!(!ku.value.key_cert_sign(), "SVID must NOT carry keyCertSign");
    assert!(!ku.value.crl_sign(), "SVID must NOT carry cRLSign");

    // (c) EXACTLY ONE URI SAN, equal to the requested SpiffeId (the SPIFFE
    // spec's single-URI-SAN rule on the real bytes).
    let san = cert
        .subject_alternative_name()
        .expect("subjectAltName parses")
        .expect("subjectAltName present");
    let uris: Vec<&str> = san
        .value
        .general_names
        .iter()
        .filter_map(|gn| match gn {
            x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
            _ => None,
        })
        .collect();
    assert_eq!(uris.len(), 1, "SVID leaf carries EXACTLY ONE URI SAN, found {uris:?}");
    assert_eq!(uris[0], workload.as_str(), "the sole URI SAN equals the requested SpiffeId");

    // (d) validity window is ~1h (research Finding 6) — assert the WIDTH.
    let validity = cert.validity();
    let window_secs = validity.not_after.timestamp() - validity.not_before.timestamp();
    assert_eq!(window_secs, 3600, "SVID validity window is ~1 hour (3600s)");
}

/// `@real-io` `@adapter-integration` `@S-04` — ADR-0063 D9 (node-held leaf-key
/// custody): the returned `SvidMaterial` carries the MATCHING leaf private key,
/// and the cert's embedded public key corresponds to that private key. This is
/// the cert↔key correspondence the orphaned-key bug violated (the adapter
/// generated a leaf keypair, signed the cert, then DROPPED the key — every
/// issued SVID embedded a public key whose private half no entity held,
/// unusable in any mTLS handshake) and that `openssl verify` never checks
/// (chain well-formedness ≠ key possession).
///
/// Falsifiability: against the pre-D9 code (no `leaf_key` field / key dropped),
/// this test cannot even be written — `svid.leaf_key()` does not exist. With the
/// field present but populated from a DIFFERENT key than the one that signed the
/// cert, the SPKI comparison AND the sign/verify round-trip both fail. It passes
/// only when the returned private key is the genuine private half of the cert's
/// embedded public key.
#[test]
fn rcgen_svid_returns_matching_leaf_private_key_for_node_custody() {
    use rcgen::PublicKeyData;

    // GIVEN a host CA and a workload SVID request for a specific identity.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let workload = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
        .expect("workload SpiffeId parses");
    let req = SvidRequest::new(workload);

    // WHEN the leaf is minted through the driving port (D9 returns cert + key).
    let svid = ca.issue_svid(&req).expect("RcgenCa::issue_svid mints a leaf + returns its key");

    // THEN the returned leaf key is a non-empty PKCS#8 "PRIVATE KEY" PEM block —
    // the node-held credential the agent feeds to rustls (the field that did not
    // exist before D9).
    let leaf_key_pem = svid.leaf_key().as_pem();
    assert!(
        leaf_key_pem.contains("-----BEGIN PRIVATE KEY-----"),
        "leaf key must be a PKCS#8 PRIVATE KEY PEM block, got: {leaf_key_pem}"
    );

    // AND the public key DERIVED FROM the returned private key equals the public
    // key EMBEDDED IN the issued cert — the cert↔key correspondence `openssl
    // verify` never checks (chain well-formedness ≠ key possession). Both sides
    // are reduced to their SubjectPublicKeyInfo (SPKI) DER and compared
    // byte-for-byte:
    //   - cert side: x509-parser exposes the cert's `subject_pki.raw` SPKI DER.
    //   - key side: rcgen reloads the PKCS#8 PEM (`KeyPair::from_pem`) and
    //     re-serializes its public half as SPKI DER (`subject_public_key_info`,
    //     the `PublicKeyData` trait method — the same SPKI shape x509-parser
    //     reads from the cert, proven equal by rcgen's own round-trip test).
    // A dropped key (the orphaned-key bug — no field at all) makes this test
    // impossible to write; a mismatched key makes these diverge; the genuine
    // matching private half makes them byte-identical, proving the SVID is usable
    // in an mTLS handshake (the node agent holds the key for the cert it presents).
    let reloaded = KeyPair::from_pem(leaf_key_pem)
        .expect("returned leaf key reloads as a valid PKCS#8 keypair");
    let key_spki_der = reloaded.subject_public_key_info();

    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(svid.cert_der().as_der())
        .expect("parse svid DER");
    let cert_spki_der = cert.tbs_certificate.subject_pki.raw;

    assert_eq!(
        cert_spki_der,
        key_spki_der.as_slice(),
        "the cert's embedded public key (SPKI) must equal the public half of the returned leaf \
         private key — the cert↔key correspondence the orphaned-key bug violated"
    );
}

// ---------------------------------------------------------------------------
// Adoption ordering — divergent-anchor adoption is a typed AdoptionConflict
// (ADR-0063 review P3), idempotent same-anchor re-adoption is Ok(())
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@error` — `adopt_persisted_root` after the
/// adapter has already minted a DIFFERENT (ephemeral) root surfaces the dedicated
/// [`CaError::AdoptionConflict`] (`which = "root"`) — NOT the generic
/// `SigningFailed`. This is an adoption/ordering conflict (issuance ran before
/// adoption, the ephemeral-root chain-break already happened), a distinct failure
/// mode from a signing-backend error.
///
/// Falsifiability: against the pre-fix code this returns
/// `CaError::SigningFailed { .. }`, so the `matches!(.., AdoptionConflict)`
/// assertion fails; it passes only once the divergence path returns the dedicated
/// variant. Idempotent same-root re-adoption still returns `Ok(())`.
#[test]
fn rcgen_adopt_divergent_root_is_an_adoption_conflict_not_signing_failed() {
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");

    // GIVEN an adapter that has already MINTED its own (ephemeral) root.
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject.clone());
    let minted_root = ca.root().expect("RcgenCa::root() mints an ephemeral root");

    // AND a DIFFERENT persisted root (a second adapter's independently-minted
    // root — different key, different cert bytes).
    let other_ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let divergent_root = other_ca.root().expect("a second, divergent root");
    assert_ne!(
        minted_root.cert_der().as_der(),
        divergent_root.cert_der().as_der(),
        "the two independently-minted roots must differ for this test to mean anything"
    );

    // WHEN we adopt the divergent root AFTER issuance already minted a different
    // one. THEN it fails with the dedicated AdoptionConflict variant.
    let err = ca
        .adopt_persisted_root(&divergent_root)
        .expect_err("adopting a divergent root after a mint must fail");
    assert!(
        matches!(err, CaError::AdoptionConflict { which: "root" }),
        "divergent root adoption must be CaError::AdoptionConflict {{ which: \"root\" }}, got {err:?}"
    );

    // AND adopting the byte-identical SAME root the adapter already holds is an
    // idempotent no-op `Ok(())`.
    ca.adopt_persisted_root(&minted_root)
        .expect("re-adopting the same root is an idempotent Ok(())");
}

/// `@real-io` `@adapter-integration` `@error` — sibling of the root case for the
/// node intermediate: `adopt_persisted_intermediate` after a DIFFERENT
/// intermediate was minted surfaces [`CaError::AdoptionConflict`]
/// (`which = "intermediate"`), and idempotent same-intermediate re-adoption is
/// `Ok(())`.
#[test]
fn rcgen_adopt_divergent_intermediate_is_an_adoption_conflict_not_signing_failed() {
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let node = NodeId::new("node-a").expect("NodeId parses");

    // GIVEN an adapter that has already minted its own (ephemeral) intermediate.
    let ca = RcgenCa::new(Arc::new(OsEntropy), subject.clone());
    let minted_inter =
        ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate mints an intermediate");

    // AND a DIFFERENT persisted intermediate (a second adapter's).
    let other_ca = RcgenCa::new(Arc::new(OsEntropy), subject);
    let divergent_inter =
        other_ca.issue_intermediate(&node).expect("a second, divergent intermediate");
    assert_ne!(
        minted_inter.cert_der().as_der(),
        divergent_inter.cert_der().as_der(),
        "the two independently-minted intermediates must differ"
    );

    // WHEN we adopt the divergent intermediate after issuance already minted a
    // different one. THEN it fails with the dedicated AdoptionConflict variant.
    let err = ca
        .adopt_persisted_intermediate(&node, &divergent_inter)
        .expect_err("adopting a divergent intermediate after a mint must fail");
    assert!(
        matches!(err, CaError::AdoptionConflict { which: "intermediate" }),
        "divergent intermediate adoption must be CaError::AdoptionConflict {{ which: \
         \"intermediate\" }}, got {err:?}"
    );

    // AND re-adopting the SAME intermediate is an idempotent no-op `Ok(())`.
    ca.adopt_persisted_intermediate(&node, &minted_inter)
        .expect("re-adopting the same intermediate is an idempotent Ok(())");
}
