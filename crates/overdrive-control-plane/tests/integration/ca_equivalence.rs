//! Integration — `Ca` trait equivalence: `RcgenCa` (host) vs `SimCa` (sim).
//! THE central trait-contract enforcement test (DISTILL RED scaffolds,
//! built-in-ca / GH #28).
//!
//! Per `.claude/rules/development.md` § "Trait definitions specify behavior"
//! → "The DST equivalence test is the structural guard" and ADR-0063 D8:
//! a port trait whose contract differs across adapters in any non-trivial
//! way ships a `tests/integration/<trait>_equivalence.rs` that drives BOTH
//! implementations through the SAME call sequence and asserts observable
//! equivalence through the trait's own accessors. When the equivalence test
//! fails, exactly one of {contract, host adapter, sim adapter} is wrong —
//! the test isolates which.
//!
//! WHY THIS CRATE: `overdrive-control-plane` is the only crate that
//! dev-deps BOTH `overdrive-host` (`RcgenCa`) and `overdrive-sim` (`SimCa`)
//! — it owns the CA boot/issuance wiring (ADR-0063 component decomposition).
//! Host and sim do NOT depend on each other (sim/host split is load-bearing,
//! CLAUDE.md), so the equivalence harness has no other natural home.
//!
//! Layer 3 (gated `integration-tests`, runs via Lima — `RcgenCa` does real
//! crypto + keyring). Per Mandate 11 this is EXAMPLE-ONLY: a fixed call
//! sequence with fixed inputs (the sim side uses fixture keys + a seed; the
//! observable-equivalence claim is over the trait accessors, NOT over
//! generated inputs).
//!
//! Observable-equivalence Universe (trait accessors only, NEVER internal
//! adapter fields — refactor-resilient):
//!   - root: subject (trust domain), `is_ca`, `key_usages`, NOT serial/key bytes
//!     (sim fixture key differs from host-generated key by construction —
//!     research Finding 11; equivalence is over the *contract-observable*
//!     profile, not the key material)
//!   - intermediate: `is_ca`, `path_len=0`, chains-to-root, `key_usages`
//!   - svid: `is_ca=false`, `san_uris` (cardinality + value), `key_usages`,
//!     issuer linkage
//!
//! There is no bad-SAN error-parity scenario: under the ratified Option A
//! (ADR-0063 D5 amendment, 2026-06-06) the single-URI-SAN invariant is honored
//! BY CONSTRUCTION — a [`SvidRequest`] holds exactly one validated `SpiffeId`,
//! so a 0-or-≥2-URI-SAN request is unrepresentable at the `Ca` boundary and
//! there is no adapter cardinality-reject path to compare. The one fallible
//! parse of raw SAN cardinality is the pure-core `CertSpec::svid` policy,
//! tested at L1.
//!
//! Tags: `@real-io` `@adapter-integration` `@S-01` `@S-03` `@S-04`.
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt
//! `RcgenCa` / `SimCa`. DELIVER replaces with the real twin-adapter
//! call-sequence + accessor-equivalence assertions.

use std::sync::Arc;

use overdrive_core::traits::ca::{Ca, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest};
use overdrive_core::{NodeId, SpiffeId};
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use overdrive_sim::adapters::SimCa;
use overdrive_sim::adapters::entropy::SimEntropy;
use x509_parser::prelude::FromDer;

/// The contract-observable root profile, parsed from a [`RootCaHandle`]'s real
/// cert DER via the trait accessor (`cert_der`). This is the equivalence
/// Universe: `is_ca`, `path_len`, the key-usage set, and `key_usage_critical`
/// — NEVER the serial / key bytes (the sim fixture key differs from the
/// host-generated key by construction, research Finding 11) and NEVER the
/// subject DN (a sim-fixture concern that differs by construction in the same
/// way the key does). Reading the public cert bytes (not internal adapter
/// fields) keeps the assertion refactor-resilient.
#[derive(Debug, PartialEq, Eq)]
struct RootProfile {
    is_ca: bool,
    path_len: Option<u32>,
    /// The key-usage set in canonical order — the contract-observable set, not
    /// individual bits, so the comparison is over the whole profile.
    key_usages: Vec<&'static str>,
    key_usage_critical: bool,
    /// A self-signed root is its own issuer — a structural property both
    /// adapters share even though the concrete DN differs by construction.
    self_issued: bool,
}

fn root_profile(handle: &RootCaHandle) -> RootProfile {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(handle.cert_der().as_der())
        .expect("parse root DER");
    let bc = cert
        .basic_constraints()
        .expect("basicConstraints parses")
        .expect("basicConstraints present");
    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    let mut key_usages = Vec::new();
    if ku.value.key_cert_sign() {
        key_usages.push("keyCertSign");
    }
    if ku.value.crl_sign() {
        key_usages.push("cRLSign");
    }
    if ku.value.digital_signature() {
        key_usages.push("digitalSignature");
    }
    RootProfile {
        is_ca: bc.value.ca,
        path_len: bc.value.path_len_constraint,
        key_usages,
        key_usage_critical: ku.critical,
        self_issued: cert.issuer().to_string() == cert.subject().to_string(),
    }
}

/// `@real-io` `@adapter-integration` `@S-01` — root profile equivalence:
/// `RcgenCa::root()` and `SimCa::root()` produce roots whose
/// CONTRACT-OBSERVABLE profile is equivalent — both CA:TRUE, both carry
/// keyCertSign|cRLSign, both keyUsage-critical, both self-signed roots.
/// (Key bytes AND the subject DN differ by construction; the profile does not.)
#[test]
fn ca_equivalence_root_profile_matches_across_host_and_sim() {
    // GIVEN both adapters: the host CA over a real OS entropy source + a
    // trust-domain subject, and the sim CA over a seeded entropy source +
    // its embedded fixture root.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let host: RcgenCa = RcgenCa::new(Arc::new(OsEntropy), subject);
    let sim: SimCa = SimCa::new(Arc::new(SimEntropy::new(0xCA_5E)));

    // WHEN each produces its persistent self-signed root (driving port).
    let host_root = host.root().expect("RcgenCa::root() self-signs a real root");
    let sim_root = sim.root().expect("SimCa::root() loads its fixture root");

    // THEN the contract-observable profile is equivalent across host and sim —
    // both CA:TRUE, no pathLen, keyCertSign + cRLSign, keyUsage critical, each
    // a self-signed root. This proves the host adapter derives the SAME profile
    // the sim shares (both from the one core `CertSpec::root` policy), with the
    // serial/key/subject explicitly excluded from the Universe (differ by
    // construction — research Finding 11; ADR-0063 D8).
    let host_profile = root_profile(&host_root);
    let sim_profile = root_profile(&sim_root);
    assert_eq!(
        host_profile, sim_profile,
        "host and sim roots must agree on the contract-observable profile"
    );

    // AND the shared profile is the root profile the contract pins.
    assert_eq!(
        host_profile,
        RootProfile {
            is_ca: true,
            path_len: None,
            key_usages: vec!["keyCertSign", "cRLSign"],
            key_usage_critical: true,
            self_issued: true,
        },
        "the shared root profile matches the Ca::root contract"
    );
}

/// The contract-observable intermediate profile, parsed from an
/// [`IntermediateHandle`]'s real cert DER via the trait accessor (`cert_der`).
/// The equivalence Universe for an intermediate: `is_ca`, `path_len`, the
/// key-usage set, `key_usage_critical`, and `chains_to_root` (issuer DN equals
/// the adapter's own root subject DN) — NEVER the serial / key bytes (differ by
/// construction — research Finding 11) and NEVER the concrete subject/issuer DN
/// strings (the sim fixture DN differs from the host-derived DN by
/// construction; only the *chains-to-root* relationship is contract-observable).
#[derive(Debug, PartialEq, Eq)]
struct IntermediateProfile {
    is_ca: bool,
    path_len: Option<u32>,
    key_usages: Vec<&'static str>,
    key_usage_critical: bool,
    /// The intermediate's issuer DN equals its adapter's root subject DN — the
    /// chains-to-root linkage, a structural property both adapters share even
    /// though the concrete DN differs by construction.
    chains_to_root: bool,
}

fn intermediate_profile(handle: &IntermediateHandle, root: &RootCaHandle) -> IntermediateProfile {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(handle.cert_der().as_der())
        .expect("parse intermediate DER");
    let (_, root_cert) =
        x509_parser::certificate::X509Certificate::from_der(root.cert_der().as_der())
            .expect("parse root DER");
    let bc = cert
        .basic_constraints()
        .expect("basicConstraints parses")
        .expect("basicConstraints present");
    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    let mut key_usages = Vec::new();
    if ku.value.key_cert_sign() {
        key_usages.push("keyCertSign");
    }
    if ku.value.crl_sign() {
        key_usages.push("cRLSign");
    }
    if ku.value.digital_signature() {
        key_usages.push("digitalSignature");
    }
    IntermediateProfile {
        is_ca: bc.value.ca,
        path_len: bc.value.path_len_constraint,
        key_usages,
        key_usage_critical: ku.critical,
        chains_to_root: cert.issuer().to_string() == root_cert.subject().to_string(),
    }
}

/// `@real-io` `@adapter-integration` `@S-03` — intermediate profile
/// equivalence: both adapters' `issue_intermediate(&node)` produce
/// CA:TRUE + pathLenConstraint=0 intermediates that chain to their
/// respective roots, with identical key-usage profile.
#[test]
fn ca_equivalence_intermediate_profile_matches_across_host_and_sim() {
    // GIVEN both adapters and a node identity.
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let host: RcgenCa = RcgenCa::new(Arc::new(OsEntropy), subject);
    let sim: SimCa = SimCa::new(Arc::new(SimEntropy::new(0xCA_5E)));
    let node = NodeId::new("node-a").expect("NodeId parses");

    // WHEN each produces its root + node intermediate (driving port). Each
    // intermediate chains to its OWN adapter's root (cross-adapter mixing is not
    // asserted — different roots by construction).
    let host_root = host.root().expect("RcgenCa::root() self-signs a real root");
    let sim_root = sim.root().expect("SimCa::root() loads its fixture root");
    let host_inter =
        host.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");
    let sim_inter = sim.issue_intermediate(&node).expect("SimCa::issue_intermediate loads fixture");

    // THEN the contract-observable intermediate profile is equivalent across
    // host and sim — both CA:TRUE, both pathLen=0, both keyCertSign, both
    // keyUsage critical, each chaining to its own root. This proves both
    // adapters derive the SAME profile from the one core `CertSpec::intermediate`
    // policy (ADR-0063 D8), with serial/key/DN excluded from the Universe.
    let host_profile = intermediate_profile(&host_inter, &host_root);
    let sim_profile = intermediate_profile(&sim_inter, &sim_root);
    assert_eq!(
        host_profile, sim_profile,
        "host and sim intermediates must agree on the contract-observable profile"
    );

    // AND the shared profile is the intermediate profile the contract pins.
    assert_eq!(
        host_profile,
        IntermediateProfile {
            is_ca: true,
            path_len: Some(0),
            key_usages: vec!["keyCertSign"],
            key_usage_critical: true,
            chains_to_root: true,
        },
        "the shared intermediate profile matches the Ca::issue_intermediate contract"
    );
}

/// The contract-observable SVID leaf profile, parsed from a [`SvidMaterial`]'s
/// real cert DER via the trait accessor (`cert_der`). The equivalence Universe
/// for a leaf: `is_ca` (false), the URI-SAN set (cardinality + value), the
/// key-usage set, `key_usage_critical`, and `chains_to_issuer` (issuer DN
/// equals the adapter's own intermediate subject DN) — NEVER the serial / key
/// bytes (the sim fixture key differs from the host-generated key by
/// construction — research Finding 11) and NEVER the concrete subject/issuer DN
/// strings (the sim fixture DN differs from the host-derived DN by
/// construction; only the *chains-to-issuer* relationship is
/// contract-observable). Reading the public cert bytes (not internal adapter
/// fields) keeps the assertion refactor-resilient.
#[derive(Debug, PartialEq, Eq)]
struct SvidProfile {
    is_ca: bool,
    /// The `spiffe://` URI SANs the leaf carries — both cardinality and value
    /// are contract-observable (the single-URI-SAN rule is the headline
    /// invariant, K2).
    san_uris: Vec<String>,
    key_usages: Vec<&'static str>,
    key_usage_critical: bool,
    /// The leaf's issuer DN equals its adapter's intermediate subject DN — the
    /// chains-to-issuer linkage, a structural property both adapters share even
    /// though the concrete DN differs by construction.
    chains_to_issuer: bool,
}

fn svid_profile(svid: &SvidMaterial, intermediate: &IntermediateHandle) -> SvidProfile {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(svid.cert_der().as_der())
        .expect("parse svid DER");
    let (_, inter_cert) =
        x509_parser::certificate::X509Certificate::from_der(intermediate.cert_der().as_der())
            .expect("parse intermediate DER");

    // basicConstraints CA:FALSE — a leaf signs nothing. The extension may be
    // absent (treated as CA:FALSE) or present with the CA bit unset.
    let is_ca =
        cert.basic_constraints().expect("basicConstraints parses").is_some_and(|ext| ext.value.ca);

    let san = cert
        .subject_alternative_name()
        .expect("subjectAltName parses")
        .expect("subjectAltName present");
    let san_uris: Vec<String> = san
        .value
        .general_names
        .iter()
        .filter_map(|gn| match gn {
            x509_parser::extensions::GeneralName::URI(uri) => Some((*uri).to_owned()),
            _ => None,
        })
        .collect();

    let ku = cert.key_usage().expect("keyUsage parses").expect("keyUsage present");
    let mut key_usages = Vec::new();
    if ku.value.key_cert_sign() {
        key_usages.push("keyCertSign");
    }
    if ku.value.crl_sign() {
        key_usages.push("cRLSign");
    }
    if ku.value.digital_signature() {
        key_usages.push("digitalSignature");
    }

    SvidProfile {
        is_ca,
        san_uris,
        key_usages,
        key_usage_critical: ku.critical,
        chains_to_issuer: cert.issuer().to_string() == inter_cert.subject().to_string(),
    }
}

/// `@real-io` `@adapter-integration` `@S-04` — SVID profile equivalence:
/// both adapters' `issue_svid(&req)` for the same `SpiffeId` produce a leaf
/// with CA:FALSE, exactly ONE URI SAN equal to that `SpiffeId`,
/// keyUsage=digitalSignature critical (NO keyCertSign/cRLSign), and chaining
/// to their respective intermediates. This pins the highest-value invariant
/// (single URI SAN, K2) as a SHARED contract — proving the sim adapter does
/// not diverge on the leaf profile (it consumes the same core `CertSpec`).
#[test]
fn ca_equivalence_svid_profile_matches_across_host_and_sim() {
    // GIVEN both adapters and a single workload identity. Each adapter mints its
    // OWN root + intermediate (cross-adapter chain mixing is NOT asserted —
    // different roots by construction); the leaf chains to its own intermediate.
    //
    // The workload identity is the one the SIM fixture leaf actually carries
    // as its embedded URI SAN (`spiffe://overdrive.local/workload/sim-svid`):
    // `SimCa::issue_svid` returns an opaque, pre-minted fixture cert whose SAN
    // is fixed at the byte level (research Finding 11 — the sim never re-mints
    // crypto), so the only identity for which BOTH adapters carry the SAME
    // single URI SAN in their REAL cert bytes is the fixture's own identity.
    // The host adapter mints a genuine leaf for whatever SpiffeId is requested,
    // so requesting the fixture identity makes the host's real SAN equal the
    // sim's — the honest cross-adapter byte-level equivalence the SVID-profile
    // postcondition pins (the SAN-value-equals-request contract is proven
    // per-adapter in the S-04-08 host test and the sim acceptance suite).
    let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
        .expect("trust-domain SpiffeId parses");
    let host: RcgenCa = RcgenCa::new(Arc::new(OsEntropy), subject);
    let sim: SimCa = SimCa::new(Arc::new(SimEntropy::new(0xCA_5E)));
    let node = NodeId::new("node-a").expect("NodeId parses");
    let workload = SpiffeId::new("spiffe://overdrive.local/workload/sim-svid")
        .expect("workload SpiffeId parses");
    let req = SvidRequest::new(workload.clone());

    // WHEN each produces its intermediate + workload SVID (driving port). The
    // intermediate is captured so the leaf's chains-to-issuer linkage is
    // observable against the SAME adapter's intermediate.
    let host_inter =
        host.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");
    let sim_inter = sim.issue_intermediate(&node).expect("SimCa::issue_intermediate loads fixture");
    let host_svid = host.issue_svid(&req).expect("RcgenCa::issue_svid mints a leaf");
    let sim_svid = sim.issue_svid(&req).expect("SimCa::issue_svid mints a leaf");

    // THEN the contract-observable SVID profile is equivalent across host and
    // sim — both CA:FALSE, both carrying exactly one URI SAN, both
    // keyUsage=digitalSignature critical (NO keyCertSign/cRLSign), each chaining
    // to its own intermediate. This proves both adapters derive the SAME leaf
    // profile from the one core `CertSpec::svid` policy (ADR-0063 D8), with
    // serial/key/DN excluded from the Universe (differ by construction —
    // research Finding 11).
    let host_profile = svid_profile(&host_svid, &host_inter);
    let sim_profile = svid_profile(&sim_svid, &sim_inter);
    assert_eq!(
        host_profile, sim_profile,
        "host and sim SVIDs must agree on the contract-observable profile"
    );

    // AND the shared profile is the leaf profile the contract pins: CA:FALSE,
    // exactly one URI SAN equal to the requested SpiffeId, digitalSignature
    // critical, chaining to the intermediate.
    assert_eq!(
        host_profile,
        SvidProfile {
            is_ca: false,
            san_uris: vec![workload.as_str().to_owned()],
            key_usages: vec!["digitalSignature"],
            key_usage_critical: true,
            chains_to_issuer: true,
        },
        "the shared SVID profile matches the Ca::issue_svid contract"
    );
}

/// `@real-io` `@adapter-integration` `@S-05` — trust-bundle equivalence: a
/// leaf minted by an adapter verifies against THAT adapter's
/// `trust_bundle()`, and the bundle composition shape (root anchor +
/// intermediate as untrusted chain material) is equivalent across host and
/// sim. (Cross-adapter chain mixing is NOT asserted — different roots.)
#[test]
#[should_panic(expected = "RED scaffold")]
fn ca_equivalence_trust_bundle_shape_matches_across_host_and_sim() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / trust_bundle() composition shape (root anchor \
         + intermediate untrusted chain) is equivalent across RcgenCa and SimCa; each adapter's leaf \
         verifies against its own bundle)"
    );
}
