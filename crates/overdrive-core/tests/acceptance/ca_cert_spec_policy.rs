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
//! NOTE (RED scaffold convention, .claude/rules/testing.md): the remaining
//! `#[should_panic(expected = "RED scaffold")]` bodies (owned by slices 03 /
//! 04) are self-contained panics that import no unbuilt production code —
//! nextest reports PASS (expected panic), clippy is clean, lefthook needs no
//! `--no-verify`. Step 01-01 has activated S-01-01 and S-01-05 (real bodies
//! below); they import the now-built `CertSpec` policy surface.

use overdrive_core::{CertRole, CertSpec, CertSpecError, KeyUsage, SpiffeId};

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
///
/// Universe (port-exposed observable surface, never internal builder fields):
/// `{role, is_ca, key_usages, key_usage_critical, path_len, subject}`. The
/// assertion is exact-equality over that whole surface — fail-closed on any
/// unexpected extra key usage (e.g. a stray `digitalSignature` that would
/// blur the CA profile).
#[test]
fn root_spec_is_self_signed_ca_with_key_cert_sign_and_crl_sign() {
    // The root subject identifies the trust domain `overdrive.local`. The
    // `SpiffeId` newtype requires a path component, so the trust domain is
    // carried as the authority and the trust-domain-only semantic is the
    // observable `subject().trust_domain()` (asserted below).
    let subject = SpiffeId::new("spiffe://overdrive.local/ca/root").expect("valid root subject");
    let spec = CertSpec::root(subject.clone());

    // role — the sum type carries Root (no pathLen field by construction).
    assert_eq!(spec.role(), CertRole::Root);

    // is_ca — CA:TRUE.
    assert!(spec.is_ca(), "root must be CA:TRUE");

    // path_len — NO pathLen constraint on the root.
    assert_eq!(spec.path_len(), None, "root carries no pathLen constraint");

    // key_usages — exactly keyCertSign + cRLSign, nothing else (fail-closed).
    assert_eq!(
        spec.key_usages(),
        vec![KeyUsage::KeyCertSign, KeyUsage::CrlSign],
        "root carries exactly keyCertSign + cRLSign"
    );

    // key_usage_critical — keyUsage marked critical.
    assert!(spec.key_usage_critical(), "keyUsage must be marked critical");

    // subject — the trust domain, preserved through the SpiffeId newtype.
    assert_eq!(spec.subject(), &subject);
    assert_eq!(spec.subject().trust_domain(), "overdrive.local");
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
///
/// Universe: the `CertSpecError` variant per failure mode. Asserts (a) a bad
/// SAN cardinality IS the `InvalidSan` variant carrying the offending count,
/// and (b) it is `!=` a structurally-different failure (`InvalidSubject`) —
/// i.e. the taxonomy distinguishes failure modes rather than flattening them
/// into one catch-all.
#[test]
fn cert_spec_error_variants_are_distinct_per_failure_mode() {
    // A SAN cardinality of 0 and of 2 both surface InvalidSan, carrying the
    // offending count (the load-bearing single-URI-SAN signal, KPI K2).
    let zero_san = CertSpecError::invalid_san(0);
    let two_san = CertSpecError::invalid_san(2);
    assert!(matches!(zero_san, CertSpecError::InvalidSan { found: 0 }));
    assert!(matches!(two_san, CertSpecError::InvalidSan { found: 2 }));

    // A different failure mode is a DISTINCT variant — not the same
    // catch-all. InvalidSan != InvalidSubject proves the taxonomy
    // distinguishes failure modes (a single Internal(String) catch-all could
    // not satisfy this — both would collapse to the same shape).
    let bad_subject =
        CertSpecError::invalid_subject("Root", "subject must be the trust domain only");
    assert_ne!(zero_san, bad_subject);
    assert!(matches!(bad_subject, CertSpecError::InvalidSubject { .. }));

    // The two InvalidSan instances differ by their carried count, so the
    // variant is not a content-free marker — it preserves the cardinality.
    assert_ne!(zero_san, two_san);
}
