//! Integration — `SimCa` fixture leaf cert↔key correspondence (ADR-0063 D9
//! review P2).
//!
//! The host adapter proves cert↔key correspondence cryptographically
//! (`rcgen_ca_chain_verify::rcgen_svid_returns_matching_leaf_private_key_for_node_custody`
//! — reload the returned key, compare its SPKI DER against the cert's embedded
//! SPKI). The `SimCa` fixtures (`FIXTURE_SVID_CERT_*` + `FIXTURE_SVID_KEY_PEM`)
//! were regenerated as a "matched pair by construction" — but nothing GUARDED
//! that pairing, so a future fixture edit could silently desync cert and key and
//! DST would never catch it (the sim never signs). This test is that guard,
//! ported from the host proof to the sim consts.
//!
//! It runs through the [`Ca::issue_svid`] **driving port** (so the sim's private
//! fixture `const`s stay private — the returned `SvidMaterial` carries both the
//! leaf cert DER and the node-held leaf key), then parses both with `rcgen` /
//! `x509-parser` and asserts their `SubjectPublicKeyInfo` DER is byte-identical.
//! Real-crypto byte parsing → gated behind `integration-tests`, out of the
//! default unit lane.
//!
//! Falsifiability: swap `FIXTURE_SVID_KEY_PEM` for any other P-256 key (or edit a
//! byte of `FIXTURE_SVID_CERT_DER`'s SPKI) and the two SPKI DERs diverge — the
//! assertion fails. It passes only while the fixture cert and key remain the
//! genuine matched pair.

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::traits::ca::{Ca, SvidRequest};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::SimCa;
use overdrive_sim::adapters::entropy::SimEntropy;
use rcgen::{KeyPair, PublicKeyData};
use x509_parser::prelude::FromDer;

/// `@integration` `@P2` — the `SimCa` fixture leaf certificate's embedded public
/// key corresponds to the returned node-held leaf private key, exactly as the
/// host adapter guarantees. Guards the fixture matched-pair invariant against
/// silent desync.
#[test]
fn sim_ca_fixture_leaf_cert_and_key_are_a_matched_pair() {
    // GIVEN a SimCa and a workload SVID request. The fixture cert/key returned
    // are request-independent (the sim holds frozen fixture bytes), so any valid
    // identity exercises the same matched pair.
    let ca = SimCa::new(Arc::new(SimEntropy::new(0x5EED)));
    let workload = SpiffeId::new("spiffe://overdrive.local/workload/sim-svid")
        .expect("workload SpiffeId parses");
    // The validity window rides on the request (ADR-0063 rev 2 amendment); the
    // sim carries `not_after` onto the material but its fixture cert bytes are
    // fixed, so any fixed window exercises the same matched pair.
    let not_before = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let not_after = not_before + Duration::from_secs(3600);
    let req = SvidRequest::new(workload, not_before, not_after);

    // WHEN the SVID is minted through the driving port (returns cert + node-held
    // leaf key per ADR-0063 D9).
    let svid = ca.issue_svid(&req).expect("SimCa::issue_svid returns fixture material");

    // THEN the returned leaf key is a PKCS#8 PRIVATE KEY PEM block.
    let leaf_key_pem = svid.leaf_key().as_pem();
    assert!(
        leaf_key_pem.contains("-----BEGIN PRIVATE KEY-----"),
        "fixture leaf key must be a PKCS#8 PRIVATE KEY PEM block, got: {leaf_key_pem}"
    );

    // AND the public key DERIVED FROM the returned private key (its SPKI DER)
    // equals the public key EMBEDDED IN the fixture cert (its SPKI DER) —
    // byte-for-byte. A mismatched/edited fixture makes these diverge.
    let reloaded = KeyPair::from_pem(leaf_key_pem)
        .expect("fixture leaf key reloads as a valid PKCS#8 keypair");
    let key_spki_der = reloaded.subject_public_key_info();

    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(svid.cert_der().as_der())
        .expect("parse fixture svid DER");
    let cert_spki_der = cert.tbs_certificate.subject_pki.raw;

    assert_eq!(
        cert_spki_der,
        key_spki_der.as_slice(),
        "the SimCa fixture cert's embedded public key (SPKI) must equal the public half of the \
         returned fixture leaf private key — the matched-pair invariant that a fixture edit could \
         silently break"
    );
}
