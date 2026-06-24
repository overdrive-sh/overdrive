//! Acceptance scenarios for US-01 §2.2 — core newtype validation
//! (error-boundary cases).
//!
//! Translates the four §2.2 scenarios from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` directly
//! into Rust `#[test]` bodies. Each scenario asserts that the `FromStr`
//! constructor returns the correct structured `IdParseError` variant,
//! naming the `kind` field plus the offending character/position/length
//! where applicable, and that **no newtype value is constructed** on
//! failure (Rust's type system enforces this for free because the
//! constructor returns `Result` and we pattern-match on `Err`).
//!
//! Enters through the driving port for each newtype (its public
//! `FromStr` impl) and asserts the observable outcome (the `Err` variant
//! shape). No internal state is peeked.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{IdParseError, LABEL_MAX, MeshServiceName, NodeId, WorkloadId};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// §2.2 — scenario 1: Empty identifier input is rejected at the constructor.
// -----------------------------------------------------------------------------

#[test]
fn empty_input_is_rejected_with_empty_variant_naming_the_kind() {
    // Given the empty string.
    let input = "";

    // When Ana calls WorkloadId::from_str on that input.
    let outcome = WorkloadId::from_str(input);

    // Then Ana receives a parse error naming the empty input — specifically
    // the `Empty` variant, with the `kind` field carrying the newtype name.
    // And no WorkloadId value is constructed (enforced by pattern-matching the Err
    // arm; the Ok arm is a test failure).
    match outcome {
        Err(IdParseError::Empty { kind }) => {
            assert_eq!(
                kind, "WorkloadId",
                "Empty.kind must name the rejecting newtype; got {kind:?}"
            );
        }
        Err(other) => panic!("expected IdParseError::Empty, got {other:?}"),
        Ok(value) => panic!("empty input must not construct a WorkloadId; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §2.2 — scenario 2: Identifier input containing a forbidden character is
// rejected with the offending character and its byte position.
// -----------------------------------------------------------------------------

#[test]
fn space_in_identifier_is_rejected_with_invalid_char_naming_position() {
    // Given the input "payments api" with a space at byte position 8.
    let input = "payments api";
    let expected_position = 8_usize;
    let expected_char = ' ';

    // When Ana calls WorkloadId::from_str on that input.
    let outcome = WorkloadId::from_str(input);

    // Then Ana receives a parse error naming the invalid character and its
    // position. And no WorkloadId value is constructed.
    match outcome {
        Err(IdParseError::InvalidChar { kind, ch, index }) => {
            assert_eq!(kind, "WorkloadId", "InvalidChar.kind must name the newtype");
            assert_eq!(ch, expected_char, "InvalidChar.ch must carry the offending character");
            assert_eq!(
                index, expected_position,
                "InvalidChar.index must point at the offending byte"
            );
        }
        Err(other) => panic!("expected IdParseError::InvalidChar, got {other:?}"),
        Ok(value) => panic!("space in input must not construct a WorkloadId; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §2.2 — scenario 3: Identifier input that exceeds the length ceiling is
// rejected.
// -----------------------------------------------------------------------------

#[test]
fn input_of_254_chars_is_rejected_with_too_long_naming_the_max() {
    // Given an input string 254 characters long.
    // Use 'a' to avoid tripping the character-class or start/end-alnum rules
    // first — we want `TooLong` specifically, not some other variant.
    let input: String = std::iter::repeat_n('a', 254).collect();
    assert_eq!(input.len(), 254, "fixture must match the scenario length");

    // When Ana calls WorkloadId::from_str on that input.
    let outcome = WorkloadId::from_str(&input);

    // Then Ana receives a parse error naming the length violation. The
    // structured variant carries `kind` and `max` (the ceiling — one less
    // than the offending length). And no WorkloadId value is constructed.
    match outcome {
        Err(IdParseError::TooLong { kind, max }) => {
            assert_eq!(kind, "WorkloadId", "TooLong.kind must name the newtype");
            assert_eq!(max, 253, "TooLong.max must carry the DNS-label ceiling (253)");
        }
        Err(other) => panic!("expected IdParseError::TooLong, got {other:?}"),
        Ok(value) => panic!("254-char input must not construct a WorkloadId; got {value}"),
    }
}

// Boundary companion to the 254-char rejection above.
//
// The §2.2 scenario only names the rejection (`> 253`). Without a
// positive test at the boundary, a mutation `> → >=` in the validator
// would reject 253-char inputs and still pass the single-sided test.
// This pair pins both sides of the inequality: 253 accepted, 254
// rejected — a mutation to `>=` flips 253 to rejected and trips this
// test. (Kill rate on validate_label rises from 15/16 to 16/16 per
// cargo-mutants.)
#[test]
fn input_of_253_chars_at_the_boundary_is_accepted() {
    // Given an input string at the length ceiling.
    let input: String = std::iter::repeat_n('a', 253).collect();
    assert_eq!(input.len(), 253, "fixture must match the boundary length");

    // When Ana calls WorkloadId::from_str on that input.
    let outcome = WorkloadId::from_str(&input);

    // Then Ana receives a valid WorkloadId. The positive side of the boundary
    // is load-bearing — without it, `> 253` and `>= 253` are
    // observationally identical under the 254-char rejection test alone.
    let id = outcome.expect("253-char all-alnum input must parse");
    assert_eq!(id.to_string().len(), 253);
}

// -----------------------------------------------------------------------------
// §2.2 — scenario 4: Identifier input that does not start with an
// alphanumeric is rejected.
// -----------------------------------------------------------------------------

#[test]
fn leading_hyphen_is_rejected_with_invalid_format_naming_the_rule() {
    // Given an input string starting with a hyphen.
    let input = "-leading-hyphen";

    // When Ana calls NodeId::from_str on that input.
    let outcome = NodeId::from_str(input);

    // Then Ana receives a parse error naming the format violation. The
    // `InvalidFormat` variant carries `kind` (newtype name) and `expected`
    // (the rule that was broken). And no NodeId value is constructed.
    match outcome {
        Err(IdParseError::InvalidFormat { kind, expected }) => {
            assert_eq!(kind, "NodeId", "InvalidFormat.kind must name the newtype");
            assert!(
                expected.contains("alphanumeric"),
                "InvalidFormat.expected must describe the start-and-end-alnum rule; got {expected:?}"
            );
        }
        Err(other) => panic!("expected IdParseError::InvalidFormat, got {other:?}"),
        Ok(value) => panic!("leading-hyphen input must not construct a NodeId; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-03 — Suffix grammar accepts `<job>.svc.overdrive.local`, rejects
// wrong / missing suffix.
//
// The bespoke FromStr the design notes `validate_label` alone cannot provide
// (validate_label permits `.`, id.rs:102) — it accepts the canonical mesh-DNS
// name and rejects every malformation of the `.svc.overdrive.local` suffix.
// Which IdParseError variant each rejection maps to is a DELIVER detail; the
// scenario asserts is_err() for rejections and pins the accepted-case <job>
// extraction via as_str(). (ADR-0072 / US-DBN-2.)
// -----------------------------------------------------------------------------

#[test]
fn mesh_service_name_suffix_grammar_accepts_canonical_and_rejects_malformed() {
    // Accepted: canonical names yield Ok with as_str() == the expected <job>.
    let accepted: &[(&str, &str)] = &[
        ("server.svc.overdrive.local", "server"),
        ("payments-api.svc.overdrive.local", "payments-api"),
    ];
    for (input, expected_job) in accepted {
        let name = MeshServiceName::new(input)
            .unwrap_or_else(|e| panic!("{input:?} must be accepted; got {e:?}"));
        assert_eq!(
            name.as_str(),
            *expected_job,
            "as_str() must extract the <job> label for {input:?}"
        );
        // Display reconstructs the canonical full name.
        assert_eq!(name.to_string(), *input);
    }

    // Rejected: every malformation of the suffix grammar. The return type is
    // Result<_, IdParseError>, so is_err() proves the error is an IdParseError;
    // the per-case variant is a GREEN refinement.
    let rejected: &[(&str, &str)] = &[
        ("server.svc.example.com", "wrong suffix"),
        ("server.svc.overdrive.local.evil", "suffix not terminal"),
        ("server", "missing suffix"),
        ("server.overdrive.local", "missing .svc segment"),
        (".svc.overdrive.local", "empty <job> label"),
    ];
    for (input, why) in rejected {
        let outcome = MeshServiceName::new(input);
        assert!(outcome.is_err(), "{input:?} must be rejected ({why}); got {outcome:?}");
    }
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-04 — Over-long label and empty / malformed `<job>` are rejected
// with a typed IdParseError.
//
// PROPERTY: for every <job> label L that violates the DNS-1123-label rules
// (empty, > LABEL_MAX, leading/trailing non-alphanumeric, out-of-class char),
// "<L>.svc.overdrive.local" returns Err(IdParseError::<variant>) — never
// panics, never silently truncates. The ceiling is sized off LABEL_MAX (253),
// never a bespoke smaller magic number (the "one shared length ceiling" rule,
// development.md § "One shared length ceiling for label-shaped ids"; reuse
// validate_label). Hebert ch.6 negative testing: relax the happy-path
// assumption to surface any under-specified accept path.
// -----------------------------------------------------------------------------

proptest! {
    /// S-DBN-NAME-04: malformed `<job>` labels are rejected with a typed
    /// IdParseError, never accepted, never panic.
    #[test]
    fn mesh_service_name_rejects_malformed_job_labels(
        malformed in malformed_job_label(),
    ) {
        let full = format!("{malformed}.{}", MeshServiceName::SUFFIX);
        let outcome = MeshServiceName::new(&full);
        prop_assert!(
            outcome.is_err(),
            "malformed <job> label {malformed:?} must be rejected; got {outcome:?}"
        );
    }
}

/// A `<job>` label that violates at least one DNS-1123-label rule:
/// empty, over-long (> `LABEL_MAX`), leading/trailing non-alphanumeric, or
/// containing an out-of-class character. Each arm targets a distinct
/// `validate_label` reject branch.
fn malformed_job_label() -> impl Strategy<Value = String> {
    // LABEL_MAX is 253; an over-long label exceeds it. Use 254..=300 to stay
    // bounded while crossing the ceiling.
    prop_oneof![
        // Empty label (-> Empty variant).
        Just(String::new()),
        // Over-long label (-> TooLong variant): sized against LABEL_MAX (253),
        // never a bespoke smaller ceiling.
        (254usize..=300).prop_map(|n| "a".repeat(n)),
        // Leading non-alphanumeric (-> InvalidFormat).
        "[-_.][a-z0-9]{1,10}",
        // Trailing non-alphanumeric (-> InvalidFormat).
        "[a-z0-9]{1,10}[-_.]",
        // Out-of-class character (space / uppercase-after-fold is still ascii,
        // so use chars outside [a-z0-9._-]: e.g. `!`, `/`, `:` ) (-> InvalidChar).
        "[a-z0-9]{0,5}[!/:@ ][a-z0-9]{0,5}",
    ]
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-03 (design-fidelity refinement) — a multi-label `<job>` prefix is
// rejected: the v1 contract is a SINGLE `<job>` label, NO namespace segment.
//
// ADR-0072:279 pins the newtype as "a single `<job>` label in v1 (single-node,
// NO namespace segment)". `validate_label` PERMITS `.` (id.rs:102) because
// other label newtypes (`WorkloadId`/`NodeId`) legitimately carry dotted
// forms (`region.eu-west-1`), so delegating the post-suffix `<job>` straight
// to `validate_label` would wrongly accept a two-label prefix. The single-
// label guard lives in `MeshServiceName::new`, NOT in `validate_label`. A
// dotted `<job>` maps to `IdParseError::InvalidChar { kind: "MeshServiceName",
// ch: '.', index }` — the `.`'s position within the `<job>` part.
// -----------------------------------------------------------------------------

#[test]
fn mesh_service_name_rejects_multi_label_job_prefix() {
    // "foo.bar.svc.overdrive.local" strips to <job> = "foo.bar" — a two-label
    // prefix the v1 contract forbids. It must be rejected, NOT accepted with
    // <job> = "foo.bar".
    let outcome = MeshServiceName::new("foo.bar.svc.overdrive.local");
    assert!(
        matches!(
            outcome,
            Err(IdParseError::InvalidChar { kind: "MeshServiceName", ch: '.', index: 3 })
        ),
        "multi-label <job> 'foo.bar' must be rejected as InvalidChar at the '.'; got {outcome:?}"
    );

    // A deeper prefix is rejected the same way (the first '.' is the offender).
    let deeper = MeshServiceName::new("a.b.c.svc.overdrive.local");
    assert!(
        matches!(
            deeper,
            Err(IdParseError::InvalidChar { kind: "MeshServiceName", ch: '.', index: 1 })
        ),
        "multi-label <job> 'a.b.c' must be rejected as InvalidChar at the first '.'; got {deeper:?}"
    );
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-04 (length-boundary refinement) — the positive length boundary
// for `MeshServiceName` specifically is pinned on BOTH sides.
//
// S-DBN-NAME-04's proptest exercises the over-long REJECT side via the generic
// generator, but never pins the max-VALID `<job>` ACCEPT side for
// `MeshServiceName` — a regression that wrongly rejected a long-but-valid name
// would pass the suite. The ceiling is `<job>` label ≤ `LABEL_MAX` (253), the
// ADR-0072:281 contract and the development.md "one shared length ceiling"
// rule (sized off `LABEL_MAX`, never a bespoke smaller magic number). This
// pins both sides of the inequality the way the existing 253-accepted /
// 254-rejected `WorkloadId` pair already does.
// -----------------------------------------------------------------------------

#[test]
fn mesh_service_name_label_length_boundary_is_label_max() {
    // Max-valid: a single-label all-alphanumeric <job> at exactly LABEL_MAX
    // chars is ACCEPTED. The boundary is derived from the shared
    // `overdrive_core::id::LABEL_MAX` const (no bespoke literal), per the
    // development.md "one shared length ceiling" rule.
    let max_job = "a".repeat(LABEL_MAX);
    let full_max = format!("{max_job}.{}", MeshServiceName::SUFFIX);
    let accepted = MeshServiceName::new(&full_max);
    assert!(
        matches!(&accepted, Ok(name) if name.as_str().len() == LABEL_MAX),
        "a {LABEL_MAX}-char single-label <job> must be accepted at the LABEL_MAX boundary; got {accepted:?}"
    );

    // Max+1: a (LABEL_MAX + 1)-char <job> is REJECTED with TooLong (the
    // ceiling, not a silent truncation). The `max` field is bound and compared
    // against LABEL_MAX in the guard — a bare const in the pattern position
    // would bind a fresh variable rather than match by value.
    let over_job = "a".repeat(LABEL_MAX + 1);
    let full_over = format!("{over_job}.{}", MeshServiceName::SUFFIX);
    let rejected = MeshServiceName::new(&full_over);
    assert!(
        matches!(&rejected, Err(IdParseError::TooLong { kind: "MeshServiceName", max }) if *max == LABEL_MAX),
        "a (LABEL_MAX + 1)-char <job> must be rejected as TooLong at the LABEL_MAX boundary; got {rejected:?}"
    );
}
