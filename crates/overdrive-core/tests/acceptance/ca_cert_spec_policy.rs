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
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// SpiffeId strategies — generators live next to the test that consumes them
// (`SpiffeId` exposes no `Arbitrary` impl; the canonical-lowercase form is the
// newtype's own responsibility, so we generate already-lowercase components and
// never re-normalise here — a `normalize_spiffe_id()` helper would be a
// missing-newtype-constructor smell per `.claude/rules/development.md`).
// ---------------------------------------------------------------------------

/// Generate a single valid workload `SpiffeId` of the canonical shape
/// `spiffe://overdrive.local/job/<name>/alloc/<id>`. Components are drawn from
/// the lowercase DNS-label class so the generated value is always a valid SVID
/// identity (never `""`, never a fixed fixture).
fn workload_spiffe_id() -> impl Strategy<Value = SpiffeId> {
    ("[a-z][a-z0-9-]{0,20}", "[a-z0-9][a-z0-9]{0,15}").prop_map(|(name, alloc)| {
        SpiffeId::new(&format!("spiffe://overdrive.local/job/{name}/alloc/{alloc}"))
            .expect("generated SpiffeId components are valid")
    })
}

// ---------------------------------------------------------------------------
// US-CA-04 / S-04 — Workload SVID leaf profile + single-URI-SAN invariant
// ---------------------------------------------------------------------------

proptest! {
    /// `@in-memory` `@property` `@S-04` — PROPERTY (KPI K2): for ANY `SpiffeId`
    /// whose SAN projection yields exactly one `spiffe://` URI, `CertSpec::svid`
    /// accepts it and the produced spec carries CA:FALSE, exactly that one URI
    /// SAN, keyUsage=digitalSignature critical, and NO keyCertSign/cRLSign.
    ///
    /// The Universe is the spec's port-exposed observable surface — `role`,
    /// `san_uris`, `is_ca`, `key_usages`, `key_usage_critical`, `path_len`,
    /// `subject` — asserted by EXACT equality (fail-closed: a stray
    /// `keyCertSign` / `cRLSign` flips the `key_usages` assertion). Never reads
    /// internal builder fields.
    #[test]
    fn svid_spec_carries_exactly_one_uri_san_and_leaf_key_usage(id in workload_spiffe_id()) {
        let spec = CertSpec::svid(vec![id.clone()]).expect("exactly one URI SAN is accepted");

        // role — the SVID leaf carries no pathLen field by construction.
        prop_assert_eq!(spec.role(), CertRole::Svid);

        // is_ca — CA:FALSE (a leaf cannot sign other certificates).
        prop_assert!(!spec.is_ca(), "SVID must be CA:FALSE");

        // path_len — a non-CA leaf carries no pathLenConstraint.
        prop_assert_eq!(spec.path_len(), None, "SVID carries no pathLen constraint");

        // san_uris — EXACTLY one URI SAN, equal to the requested identity.
        prop_assert_eq!(spec.san_uris(), vec![id.clone()], "SVID carries exactly one URI SAN");

        // key_usages — EXACTLY digitalSignature, fail-closed: NO keyCertSign,
        // NO cRLSign (the load-bearing leaf-vs-CA distinction).
        prop_assert_eq!(
            spec.key_usages(),
            vec![KeyUsage::DigitalSignature],
            "SVID carries exactly digitalSignature, no keyCertSign/cRLSign"
        );

        // key_usage_critical — keyUsage marked critical.
        prop_assert!(spec.key_usage_critical(), "keyUsage must be marked critical");

        // subject — preserved through the SpiffeId newtype.
        prop_assert_eq!(spec.subject(), &id);
    }

    /// `@in-memory` `@property` `@S-04` `@error` — PROPERTY (KPI K2, the SPIFFE
    /// spec's hardest rule, research Finding 2): for ANY SAN projection that
    /// would yield ZERO or TWO-OR-MORE `spiffe://` URI SANs, `CertSpec::svid`
    /// is rejected with `CertSpecError::InvalidSan { found }` carrying the
    /// offending cardinality, BEFORE any certificate material is produced.
    ///
    /// The strategy generates projections of length `0` and `2..=8` (every
    /// non-one cardinality); the assertion pins `Err(InvalidSan { found })`
    /// with `found == projection.len()`. `svid` returns `Result`, so no partial
    /// spec can escape — there is no `CertSpec` value to inspect on the `Err`
    /// path. This is the negative-testing instrument for the single-URI
    /// invariant (Hebert ch.6 generalising-example via the cardinality axis).
    #[test]
    fn svid_spec_rejects_zero_or_multiple_uri_sans_before_any_cert(
        sans in prop_oneof![
            Just(Vec::<SpiffeId>::new()),
            proptest::collection::vec(workload_spiffe_id(), 2..=8),
        ],
    ) {
        let found = sans.len();
        prop_assert!(found == 0 || found >= 2, "strategy yields only non-one cardinalities");

        let result = CertSpec::svid(sans);

        // Rejected with the distinct InvalidSan variant carrying the exact
        // offending count — never a flattened catch-all, never a partial spec.
        prop_assert_eq!(
            result,
            Err(CertSpecError::InvalidSan { found }),
            "0 or >=2 URI SANs is rejected with InvalidSan carrying the cardinality"
        );
    }
}

/// `@in-memory` `@S-04` — the SVID's sole URI SAN equals the requested
/// `SpiffeId` exactly (canonical-lowercase form preserved through the newtype).
/// Example-pinned readability companion to the property above.
#[test]
fn svid_spec_subject_uri_equals_requested_spiffe_id() {
    let requested = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
        .expect("valid workload SVID subject");

    let spec = CertSpec::svid(vec![requested.clone()]).expect("exactly one URI SAN is accepted");

    // The sole URI SAN is exactly the requested identity, canonical form intact.
    assert_eq!(spec.san_uris(), vec![requested.clone()]);
    assert_eq!(spec.subject(), &requested);
    assert_eq!(
        spec.subject().as_str(),
        "spiffe://overdrive.local/job/payments/alloc/a1b2c3",
        "canonical-lowercase URI is preserved verbatim as the sole SAN"
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
///
/// Universe (port-exposed observable surface, never internal builder fields):
/// `{role, is_ca, path_len, key_usages, key_usage_critical, subject}`. The
/// assertion is exact-equality over that whole surface — fail-closed on any
/// stray key usage (e.g. a `cRLSign` that belongs to the root, or a
/// `digitalSignature` that would blur the CA profile).
#[test]
fn intermediate_spec_is_ca_true_with_path_len_zero_and_key_cert_sign() {
    // The intermediate (node) CA subject identifies the per-node authority
    // under the trust domain. The single-node intermediate is pathLen=0:
    // exactly one intermediate, leaves only, no further intermediates.
    let subject = SpiffeId::new("spiffe://overdrive.local/ca/node").expect("valid node subject");
    let spec = CertSpec::intermediate(subject.clone());

    // role — the sum type carries Intermediate with an explicit pathLen=0
    // field (an unbounded intermediate is unrepresentable by construction).
    assert_eq!(spec.role(), CertRole::Intermediate { path_len: 0 });

    // is_ca — CA:TRUE.
    assert!(spec.is_ca(), "intermediate must be CA:TRUE");

    // path_len — pathLenConstraint=0 (issues leaves only).
    assert_eq!(spec.path_len(), Some(0), "intermediate carries pathLenConstraint=0");

    // key_usages — exactly keyCertSign, nothing else (fail-closed: no cRLSign,
    // no digitalSignature).
    assert_eq!(
        spec.key_usages(),
        vec![KeyUsage::KeyCertSign],
        "intermediate carries exactly keyCertSign"
    );

    // key_usage_critical — keyUsage marked critical.
    assert!(spec.key_usage_critical(), "keyUsage must be marked critical");

    // subject — preserved through the SpiffeId newtype.
    assert_eq!(spec.subject(), &subject);
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
