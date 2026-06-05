//! Acceptance — `CertSpec` pure policy (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 1 (pure, no I/O, default lane). `CertSpec` lives in
//! `overdrive-core` and encodes the cert-profile *decision* per role
//! (ADR-0063 D5, reconciliation B): the single-URI-SAN invariant and the
//! role→extension mapping are pure policy here, so they are DST-testable
//! and dst-lint-clean, and the sim adapter shares the exact same policy
//! surface as the host adapter.
//!
//! Per Mandate 9 these layer-1 tests are the home for PBT-full coverage:
//! the single-URI-SAN rejection (`@property`) is the highest-value
//! invariant in the feature (KPI K2) and is a universal property
//! ("for ANY `SpiffeId` yielding 0 or >=2 URI SANs, issuance is rejected
//! before any cert is produced"). DELIVER replaces each `#[should_panic]`
//! scaffold with a real `proptest!` body (workspace pins `proptest = "1"`).
//!
//! Scenarios trace to: US-CA-04 (single-URI-SAN, leaf profile),
//! US-CA-03 (intermediate pathLen=0 profile), US-CA-01 (root profile).
//! Tags: `@in-memory` `@property` (S-04 surfaces) and `@in-memory` (S-01/S-03).
//!
//! NOTE (RED scaffold convention, .claude/rules/testing.md): each test
//! body is a self-contained `panic!("Not yet implemented -- RED
//! scaffold (...)")` under `#[should_panic(expected = "RED scaffold")]`.
//! It does NOT import unbuilt `CertSpec` production code — nextest reports
//! PASS (expected panic), clippy is clean, lefthook needs no `--no-verify`.

// ---------------------------------------------------------------------------
// US-CA-04 / S-04 — Workload SVID leaf profile + single-URI-SAN invariant
// ---------------------------------------------------------------------------

/// `@in-memory` `@property` `@S-04` — PROPERTY (KPI K2): for ANY `SpiffeId`
/// whose SAN projection yields exactly one `spiffe://` URI, `CertSpec::svid`
/// accepts it and the produced spec carries CA:FALSE, exactly that one URI
/// SAN, keyUsage=digitalSignature critical, and NO keyCertSign/cRLSign.
///
/// DELIVER: `proptest!` over a `SpiffeId` strategy
/// (`spiffe://overdrive.local/job/<name>/alloc/<id>`), assert the
/// `CertSpec::svid(..)` Ok-profile invariants. The Universe is the spec's
/// port-exposed observable surface (role, `san_uris`, `is_ca`, `key_usages`) —
/// never internal builder fields.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_spec_carries_exactly_one_uri_san_and_leaf_key_usage() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / CertSpec::svid accepts one-URI SpiffeId, \
         profile = CA:FALSE + single URI SAN + digitalSignature-critical, no keyCertSign/cRLSign)"
    );
}

/// `@in-memory` `@property` `@S-04` `@error` — PROPERTY (KPI K2, the
/// SPIFFE spec's hardest rule, research Finding 2): for ANY SAN projection
/// that would yield ZERO or TWO-OR-MORE `spiffe://` URI SANs,
/// `CertSpec::svid` is rejected with `CertSpecError::InvalidSan` BEFORE any
/// certificate material is produced.
///
/// DELIVER: `proptest!` over a strategy generating `0` and `>=2` URI-SAN
/// inputs; assert every case returns `Err(CertSpecError::InvalidSan)` and
/// that no partial spec escapes. This is the negative-testing instrument
/// for the single-URI invariant (Hebert ch.6).
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_spec_rejects_zero_or_multiple_uri_sans_before_any_cert() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / CertSpec::svid rejects 0 or >=2 URI SANs \
         with CertSpecError::InvalidSan before any cert material is produced)"
    );
}

/// `@in-memory` `@S-04` — the SVID subject in the produced spec equals the
/// requested `SpiffeId` exactly (canonical-lowercase form preserved through
/// the newtype). Example-pinned readability companion to the property above.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_spec_subject_uri_equals_requested_spiffe_id() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / CertSpec::svid for \
         spiffe://overdrive.local/job/payments/alloc/a1b2c3 carries that exact URI as its sole SAN)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-03 / S-03 — Node intermediate profile (pathLen=0, CA:TRUE)
// ---------------------------------------------------------------------------

/// `@in-memory` `@S-03` — `CertSpec::intermediate` produces a CA:TRUE
/// profile with pathLenConstraint=0, keyCertSign set, keyUsage critical.
/// The pathLen value is carried as `CertRole::Intermediate { path_len: 0 }`
/// (sum type makes an unbounded intermediate unrepresentable, per
/// `development.md` § "Type-driven design").
#[test]
#[should_panic(expected = "RED scaffold")]
fn intermediate_spec_is_ca_true_with_path_len_zero_and_key_cert_sign() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / CertSpec::intermediate profile = \
         CA:TRUE + pathLenConstraint=0 + keyCertSign + keyUsage critical)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-01 / S-01 — Root profile (self-signed CA, no pathLen)
// ---------------------------------------------------------------------------

/// `@in-memory` `@S-01` — `CertSpec::root` produces a CA:TRUE profile with
/// keyCertSign|cRLSign, keyUsage critical, and NO pathLen constraint
/// (`CertRole::Root`). The subject carries the trust domain only (no path
/// component — research Finding 2).
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_spec_is_self_signed_ca_with_key_cert_sign_and_crl_sign() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01 / CertSpec::root profile = \
         CA:TRUE + keyCertSign|cRLSign + keyUsage critical + no pathLen, trust-domain-only subject)"
    );
}

// ---------------------------------------------------------------------------
// Cross-role — CaError / CertSpecError taxonomy is distinct, not a catch-all
// ---------------------------------------------------------------------------

/// `@in-memory` `@error` `@S-01` `@S-04` — distinct failure modes get
/// distinct error variants (`development.md` § "Distinct failure modes get
/// distinct error variants"): an invalid SAN cardinality surfaces
/// `CertSpecError::InvalidSan`, distinct from any other validation failure.
/// Guards against a single `Internal(String)` catch-all swallowing the
/// load-bearing single-URI signal.
#[test]
#[should_panic(expected = "RED scaffold")]
fn cert_spec_error_variants_are_distinct_per_failure_mode() {
    panic!(
        "Not yet implemented -- RED scaffold (S-04 / CertSpecError::InvalidSan is a distinct \
         variant, not flattened into a generic Internal(String) catch-all)"
    );
}
