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
//!   - error parity: a bad-SAN `SvidRequest` yields the SAME `CaError` variant
//!     (`InvalidSan`) from BOTH adapters, before any cert
//!
//! Tags: `@real-io` `@adapter-integration` `@S-01` `@S-03` `@S-04`.
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt
//! `RcgenCa` / `SimCa`. DELIVER replaces with the real twin-adapter
//! call-sequence + accessor-equivalence assertions.

/// `@real-io` `@adapter-integration` `@S-01` — root profile equivalence:
/// `RcgenCa::root()` and `SimCa::root()` produce roots whose
/// CONTRACT-OBSERVABLE profile is equivalent — both CA:TRUE, both carry
/// keyCertSign|cRLSign, both keyUsage-critical, both trust-domain-only
/// subject. (Key bytes differ by construction; the profile does not.)
#[test]
#[should_panic(expected = "RED scaffold")]
fn ca_equivalence_root_profile_matches_across_host_and_sim() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01 / RcgenCa::root and SimCa::root agree on the \
         contract-observable profile: CA:TRUE + keyCertSign|cRLSign + keyUsage critical + trust-domain subject)"
    );
}

/// `@real-io` `@adapter-integration` `@S-03` — intermediate profile
/// equivalence: both adapters' `issue_intermediate(&node)` produce
/// CA:TRUE + pathLenConstraint=0 intermediates that chain to their
/// respective roots, with identical key-usage profile.
#[test]
#[should_panic(expected = "RED scaffold")]
fn ca_equivalence_intermediate_profile_matches_across_host_and_sim() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / RcgenCa and SimCa issue_intermediate agree: \
         CA:TRUE + pathLen=0 + keyUsage critical, each chaining to its own root)"
    );
}

/// `@real-io` `@adapter-integration` `@S-04` — SVID profile equivalence:
/// both adapters' `issue_svid(&req)` for the same `SpiffeId` produce a leaf
/// with CA:FALSE, exactly ONE URI SAN equal to that `SpiffeId`,
/// keyUsage=digitalSignature critical. This pins the highest-value
/// invariant (single URI SAN, K2) as a SHARED contract — proving the sim
/// adapter does not diverge on policy (it consumes the same core `CertSpec`).
#[test]
#[should_panic(expected = "RED scaffold")]
fn ca_equivalence_svid_profile_matches_across_host_and_sim() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / RcgenCa and SimCa issue_svid agree: CA:FALSE \
         + exactly one URI SAN = requested SpiffeId + digitalSignature critical)"
    );
}

/// `@real-io` `@adapter-integration` `@S-04` `@error` — error-parity: a
/// `SvidRequest` whose `SpiffeId` yields 0 or >=2 URI SANs is rejected by
/// BOTH adapters with the SAME `CaError::InvalidSan` variant, before any
/// cert is produced. Divergent error behaviour here would mean the policy
/// lives in the adapter (rejected design A2) rather than in core `CertSpec`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn ca_equivalence_bad_san_request_rejected_identically_by_both() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / a 0-or->=2-URI-SAN SvidRequest is rejected by \
         BOTH RcgenCa and SimCa with CaError::InvalidSan, before any cert; shared core CertSpec policy)"
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
