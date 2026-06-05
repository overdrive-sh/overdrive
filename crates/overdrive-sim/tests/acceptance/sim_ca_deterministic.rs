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
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_ca_intermediate_is_deterministic_and_chains_to_fixture_root() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / SimCa::issue_intermediate at a fixed seed \
         is deterministic across two runs and chains to the fixture root)"
    );
}

/// `@in-memory` `@S-04` — KPI K5: `SimCa::issue_svid(&req)` serial (drawn
/// via `SeededEntropy::fill`) is identical across two runs at the same
/// seed AND is at least 64 bits wide (CA/B Forum floor, research Finding 10).
///
/// DELIVER: twin-run at seed `0x5EED`, assert serial bytes equal and
/// `serial.len() * 8 >= 64`. Wraps `CertSerial`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_ca_svid_serial_is_deterministic_and_at_least_64_bits() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / SimCa::issue_svid serial via SeededEntropy \
         is identical across two same-seed runs and >= 64 bits)"
    );
}

/// `@in-memory` `@S-04` — the `SimCa` SVID carries the chain-shape invariant
/// observable through the trait accessors: exactly one URI SAN equal to the
/// requested `SpiffeId`, CA:FALSE. (Sim shares the core `CertSpec` policy, so
/// this is the same invariant the host adapter enforces — the seam the
/// `ca_equivalence` contract test pins.)
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_ca_svid_carries_single_uri_san_and_is_not_a_ca() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / SimCa SVID carries exactly one URI SAN \
         equal to the requested SpiffeId and is CA:FALSE, via shared core CertSpec policy)"
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
