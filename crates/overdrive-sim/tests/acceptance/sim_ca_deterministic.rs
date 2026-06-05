//! Acceptance — `SimCa` DST determinism (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 2 (in-memory sim adapter, default lane, ~10ms). `SimCa`
//! (`overdrive-sim`, `adapter-sim`, ADR-0063 D1) loads pre-generated
//! fixture P-256 keys via PEM (research Finding 11 — key generation is a
//! host-adapter concern and NOT injectable; DST uses fixture keys) and
//! draws certificate serials through `SeededEntropy` (the seeded `Entropy`
//! port, research Finding 10). The whole point: issuance composes
//! bit-identically from a seed (KPI K5).
//!
//! Per Mandate 9 (layer 2) these are EXAMPLE-ONLY with a fixed seed — DST
//! determinism is a same-seed-same-bytes claim, not a generative-input
//! property. The PBT-full coverage of the *policy* (single-URI-SAN etc.)
//! lives at layer 1 in `overdrive-core/tests/acceptance/ca_cert_spec_policy.rs`;
//! here we assert the sim adapter's deterministic *issuance* surface.
//!
//! Scenarios trace to: US-CA-01 (root determinism), US-CA-04 (SVID serial
//! determinism), US-CA-03 (intermediate determinism).
//! Tags: `@in-memory` `@S-NN`.
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt
//! `SimCa`. DELIVER replaces with a `SeededEntropy`-driven twin-run
//! identity assertion (mirror `tests/acceptance/sim_adapters_deterministic.rs`).

/// `@in-memory` `@S-01` — KPI K5: `SimCa::root()` at seed `0x5EED` (fixture
/// P-256 key) produces bit-identical root material across two independent
/// runs.
///
/// Port-to-port: enters through the `Ca` driving port (`root()`), asserts on
/// the observable `RootCaHandle` byte surface (cert PEM, cert DER, serial).
/// Two `SimCa` instances each over their own `SimEntropy::new(0x5EED)` draw
/// the serial from the same seeded sequence, so the whole handle is
/// byte-identical — the load-bearing DST determinism claim.
#[test]
fn sim_ca_root_is_bit_identical_across_two_runs_at_same_seed() {
    use std::sync::Arc;

    use overdrive_core::traits::ca::Ca;
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::entropy::SimEntropy;

    const SEED: u64 = 0x5EED;

    let ca_a = SimCa::new(Arc::new(SimEntropy::new(SEED)));
    let ca_b = SimCa::new(Arc::new(SimEntropy::new(SEED)));

    let root_a = ca_a.root().expect("sim root issuance succeeds for the fixture key");
    let root_b = ca_b.root().expect("sim root issuance succeeds for the fixture key");

    // Observable byte surface of the root handle, drawn through the trait
    // accessors only — never internal fields.
    assert_eq!(
        root_a.cert_pem().as_pem(),
        root_b.cert_pem().as_pem(),
        "root cert PEM must be bit-identical across two same-seed runs",
    );
    assert_eq!(
        root_a.cert_der().as_der(),
        root_b.cert_der().as_der(),
        "root cert DER must be bit-identical across two same-seed runs",
    );
    assert_eq!(
        root_a.serial().as_str(),
        root_b.serial().as_str(),
        "root serial (drawn via seeded Entropy) must be identical across two same-seed runs",
    );
}

/// `@in-memory` `@S-03` — `SimCa::issue_intermediate(&node)` at a fixed
/// seed is deterministic across two runs (same intermediate material,
/// same serial via `SeededEntropy`), and chains to the fixture root.
///
/// Port-to-port: enters through the `Ca` driving port (`issue_intermediate`),
/// asserts on the observable `IntermediateHandle` byte surface (cert PEM, cert
/// DER, serial) plus the `RootCaHandle` surface for the chains-to-root
/// linkage. Two `SimCa` instances each over their own `SimEntropy::new(SEED)`
/// draw the second serial from the same seeded sequence (the root draw is the
/// first), so the whole intermediate handle is byte-identical.
///
/// Chains-to-fixture-root is observed WITHOUT parsing crypto (the sim is
/// opaque): the fixture intermediate is a real `openssl`-minted cert signed by
/// the fixture root key, so its X.509 **issuer** field carries the root's
/// subject DN (`O=overdrive-sim-fixture`). That DN's DER-encoded RDN sequence
/// is therefore a substring of BOTH the root cert DER (self-signed: issuer ==
/// subject) and the intermediate cert DER (issuer == root subject) — the
/// accessor-observable linkage. A self-signed or wrongly-issued intermediate
/// would NOT carry the root DN as its issuer and this assertion would fail RED.
#[test]
fn sim_ca_intermediate_is_deterministic_and_chains_to_fixture_root() {
    use std::sync::Arc;

    use overdrive_core::NodeId;
    use overdrive_core::traits::ca::Ca;
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::entropy::SimEntropy;

    const SEED: u64 = 0x5EED;

    // The DER-encoded RDN sequence for `O=overdrive-sim-fixture` — the fixture
    // root's subject DN. An intermediate that chains to the root carries this
    // exact byte sequence as its issuer field.
    const ROOT_SUBJECT_DN_RDN: &[u8] = &[
        0x31, 0x1e, 0x30, 0x1c, 0x06, 0x03, 0x55, 0x04, 0x0a, 0x0c, 0x15, 0x6f, 0x76, 0x65, 0x72,
        0x64, 0x72, 0x69, 0x76, 0x65, 0x2d, 0x73, 0x69, 0x6d, 0x2d, 0x66, 0x69, 0x78, 0x74, 0x75,
        0x72, 0x65,
    ];

    fn is_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    let node = NodeId::new("node-a").expect("`node-a` is a valid NodeId");

    let ca_a = SimCa::new(Arc::new(SimEntropy::new(SEED)));
    let ca_b = SimCa::new(Arc::new(SimEntropy::new(SEED)));

    let int_a = ca_a.issue_intermediate(&node).expect("sim intermediate issuance succeeds");
    let int_b = ca_b.issue_intermediate(&node).expect("sim intermediate issuance succeeds");

    // (a) cert PEM byte-equal across two same-seed runs.
    assert_eq!(
        int_a.cert_pem().as_pem(),
        int_b.cert_pem().as_pem(),
        "intermediate cert PEM must be bit-identical across two same-seed runs",
    );
    // (b) cert DER byte-equal across two same-seed runs.
    assert_eq!(
        int_a.cert_der().as_der(),
        int_b.cert_der().as_der(),
        "intermediate cert DER must be bit-identical across two same-seed runs",
    );
    // (c) serial (drawn via seeded Entropy) equal across two same-seed runs.
    assert_eq!(
        int_a.serial().as_str(),
        int_b.serial().as_str(),
        "intermediate serial (drawn via seeded Entropy) must be identical across two same-seed runs",
    );

    // (d) chains-to-fixture-root linkage, observed through the trait accessors
    // only (no PEM/DER parsing — the sim is opaque).
    let root = ca_a.root().expect("sim root issuance succeeds for the fixture key");

    // The intermediate's issuer field carries the root's subject DN.
    assert!(
        is_subsequence(int_a.cert_der().as_der(), ROOT_SUBJECT_DN_RDN),
        "intermediate cert DER must carry the fixture root subject DN as its issuer (chains to root)",
    );
    // The self-signed root carries the same DN (issuer == subject) — the anchor
    // the intermediate's issuer points at.
    assert!(
        is_subsequence(root.cert_der().as_der(), ROOT_SUBJECT_DN_RDN),
        "fixture root cert DER must carry its own subject DN as issuer (self-signed anchor)",
    );
    // The intermediate is a distinct certificate, not the root re-returned: it
    // has its own material and its own signing key.
    assert_ne!(
        int_a.cert_der().as_der(),
        root.cert_der().as_der(),
        "intermediate must be a distinct cert from the root, not the root re-returned",
    );
    assert_ne!(
        int_a.signing_key().as_pem(),
        root.signing_key().as_pem(),
        "intermediate must hold its own signing key, distinct from the root's",
    );
}

/// The fixture SVID workload identity the sim adapter mints a leaf for. The
/// embedded fixture leaf cert carries exactly this `spiffe://` URI as its sole
/// SAN (a real `openssl`-minted leaf signed by the fixture intermediate).
const FIXTURE_SVID_SPIFFE: &str = "spiffe://overdrive.local/workload/sim-svid";

/// `@in-memory` `@S-04` — KPI K5: `SimCa::issue_svid(&req)` serial (drawn
/// via `SeededEntropy::fill`) is identical across two runs at the same
/// seed AND is at least 64 bits wide (CA/B Forum floor, research Finding 10).
///
/// Port-to-port: enters through the `Ca` driving port (`issue_svid`), asserts
/// on the observable `SvidMaterial` byte surface (cert PEM, cert DER, serial).
/// Two `SimCa` instances each over their own `SimEntropy::new(SEED)` draw the
/// SVID serial from the same seeded sequence (the same draw ordinal), so the
/// whole leaf handle is byte-identical — the load-bearing DST determinism claim
/// (testing.md K3 seed → bit-identical). The serial width assertion pins the
/// CA/B Forum 64-bit floor.
#[test]
fn sim_ca_svid_serial_is_deterministic_and_at_least_64_bits() {
    use std::sync::Arc;

    use overdrive_core::SpiffeId;
    use overdrive_core::traits::ca::{Ca, SvidRequest};
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::entropy::SimEntropy;

    const SEED: u64 = 0x5EED;

    let spiffe = SpiffeId::new(FIXTURE_SVID_SPIFFE).expect("fixture SVID SPIFFE id is valid");
    let req = SvidRequest::new(spiffe);

    let ca_a = SimCa::new(Arc::new(SimEntropy::new(SEED)));
    let ca_b = SimCa::new(Arc::new(SimEntropy::new(SEED)));

    let svid_a = ca_a.issue_svid(&req).expect("sim SVID issuance succeeds for the fixture leaf");
    let svid_b = ca_b.issue_svid(&req).expect("sim SVID issuance succeeds for the fixture leaf");

    // (a) cert PEM byte-equal across two same-seed runs.
    assert_eq!(
        svid_a.cert_pem().as_pem(),
        svid_b.cert_pem().as_pem(),
        "SVID cert PEM must be bit-identical across two same-seed runs",
    );
    // (b) cert DER byte-equal across two same-seed runs.
    assert_eq!(
        svid_a.cert_der().as_der(),
        svid_b.cert_der().as_der(),
        "SVID cert DER must be bit-identical across two same-seed runs",
    );
    // (c) serial (drawn via seeded Entropy) equal across two same-seed runs.
    assert_eq!(
        svid_a.serial().as_str(),
        svid_b.serial().as_str(),
        "SVID serial (drawn via seeded Entropy) must be identical across two same-seed runs",
    );

    // (d) the serial is at least 64 bits wide (CA/B Forum floor). The serial is
    // lowercase hex of the drawn bytes, so its byte width is `len() / 2`.
    let serial_byte_width = svid_a.serial().as_str().len() / 2;
    assert!(
        serial_byte_width * 8 >= 64,
        "SVID serial must be >= 64 bits (CA/B Forum floor); got {} bits",
        serial_byte_width * 8,
    );
}

/// `@in-memory` `@S-04` — the `SimCa` SVID carries the chain-shape invariant:
/// exactly one URI SAN equal to the requested `SpiffeId`, CA:FALSE. (Sim shares
/// the core `CertSpec` policy, so this is the same invariant the host adapter
/// enforces — the seam the `ca_equivalence` contract test pins.)
///
/// Port-to-port: enters through the `Ca` driving port (`issue_svid`), asserts on
/// the observable `SvidMaterial` surface. The single-URI-SAN identity is
/// observed through `spiffe_id()` (the contract accessor: the SVID's sole URI
/// SAN). The CA:FALSE + single-URI-SAN cert *shape* is observed WITHOUT parsing
/// crypto (the sim is opaque) via DER byte substrings, mirroring the
/// intermediate chains-to-root assertion: the fixture leaf is a real
/// `openssl`-minted cert whose DER carries the `spiffe://` URI exactly once and
/// the CA:FALSE basicConstraints DER fragment. A CA leaf or a multi-SAN leaf
/// would NOT carry these byte sequences and the assertion would fail RED.
#[test]
fn sim_ca_svid_carries_single_uri_san_and_is_not_a_ca() {
    use std::sync::Arc;

    use overdrive_core::SpiffeId;
    use overdrive_core::traits::ca::{Ca, SvidRequest};
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::entropy::SimEntropy;

    const SEED: u64 = 0x5EED;

    /// The DER `subjectAltName` fragment for the single `spiffe://` URI SAN:
    /// the IA5String-tagged (`0x86`) URI bytes for [`FIXTURE_SVID_SPIFFE`]. A
    /// leaf carrying exactly this once has exactly one URI SAN equal to the
    /// `SpiffeId`.
    const SAN_URI_DER: &[u8] = b"\x86\x2aspiffe://overdrive.local/workload/sim-svid";

    /// The DER `basicConstraints` CA:FALSE fragment: extnID `2.5.29.19`
    /// (`06 03 55 1d 13`), critical (`01 01 ff`), value `04 02 30 00` — an
    /// empty SEQUENCE, i.e. cA defaults to FALSE. A CA leaf would instead carry
    /// `30 03 01 01 ff` (cA TRUE) inside the value.
    const BASIC_CONSTRAINTS_CA_FALSE_DER: &[u8] =
        &[0x06, 0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x02, 0x30, 0x00];

    fn count_subsequences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack.windows(needle.len()).filter(|w| *w == needle).count()
    }

    let spiffe = SpiffeId::new(FIXTURE_SVID_SPIFFE).expect("fixture SVID SPIFFE id is valid");
    let req = SvidRequest::new(spiffe.clone());

    let ca = SimCa::new(Arc::new(SimEntropy::new(SEED)));
    let svid = ca.issue_svid(&req).expect("sim SVID issuance succeeds for the fixture leaf");

    // (a) the SVID's sole URI SAN identity, observed through the contract
    // accessor, equals the requested SpiffeId.
    assert_eq!(
        svid.spiffe_id(),
        &spiffe,
        "SVID's URI SAN identity must equal the requested SpiffeId",
    );

    // (b) the leaf cert DER carries the `spiffe://` URI SAN EXACTLY ONCE — the
    // single-URI-SAN cardinality the shared core `CertSpec::svid` policy
    // enforces, observed on the opaque cert bytes.
    assert_eq!(
        count_subsequences(svid.cert_der().as_der(), SAN_URI_DER),
        1,
        "SVID leaf cert DER must carry the spiffe:// URI SAN exactly once (single URI SAN)",
    );

    // (c) the leaf cert DER carries the CA:FALSE basicConstraints fragment — the
    // CA:FALSE decision the shared core policy made, observed on the opaque
    // cert bytes (a CA leaf would carry the cA-TRUE fragment instead).
    assert!(
        count_subsequences(svid.cert_der().as_der(), BASIC_CONSTRAINTS_CA_FALSE_DER) == 1,
        "SVID leaf cert DER must carry the CA:FALSE basicConstraints fragment (not a CA)",
    );
}

/// `@in-memory` `@S-05` — `SimCa` re-issue: calling `issue_svid` twice for
/// the SAME `SpiffeId` yields DISTINCT serials / validity windows (fresh leaf
/// each time) even under the sim adapter; the re-issue mechanism the #40
/// rotation workflow will later drive. Determinism is per-call-sequence,
/// not per-SpiffeId-cached.
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_ca_reissue_for_same_spiffe_id_yields_a_fresh_distinct_leaf() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / SimCa re-issue for the same SpiffeId yields \
         a fresh leaf with a distinct serial and new validity window, no caching)"
    );
}
