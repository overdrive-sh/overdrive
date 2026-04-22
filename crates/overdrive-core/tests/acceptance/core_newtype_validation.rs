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

use overdrive_core::id::{IdParseError, JobId, NodeId};

// -----------------------------------------------------------------------------
// §2.2 — scenario 1: Empty identifier input is rejected at the constructor.
// -----------------------------------------------------------------------------

#[test]
fn empty_input_is_rejected_with_empty_variant_naming_the_kind() {
    // Given the empty string.
    let input = "";

    // When Ana calls JobId::from_str on that input.
    let outcome = JobId::from_str(input);

    // Then Ana receives a parse error naming the empty input — specifically
    // the `Empty` variant, with the `kind` field carrying the newtype name.
    // And no JobId value is constructed (enforced by pattern-matching the Err
    // arm; the Ok arm is a test failure).
    match outcome {
        Err(IdParseError::Empty { kind }) => {
            assert_eq!(kind, "JobId", "Empty.kind must name the rejecting newtype; got {kind:?}");
        }
        Err(other) => panic!("expected IdParseError::Empty, got {other:?}"),
        Ok(value) => panic!("empty input must not construct a JobId; got {value}"),
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

    // When Ana calls JobId::from_str on that input.
    let outcome = JobId::from_str(input);

    // Then Ana receives a parse error naming the invalid character and its
    // position. And no JobId value is constructed.
    match outcome {
        Err(IdParseError::InvalidChar { kind, ch, index }) => {
            assert_eq!(kind, "JobId", "InvalidChar.kind must name the newtype");
            assert_eq!(ch, expected_char, "InvalidChar.ch must carry the offending character");
            assert_eq!(
                index, expected_position,
                "InvalidChar.index must point at the offending byte"
            );
        }
        Err(other) => panic!("expected IdParseError::InvalidChar, got {other:?}"),
        Ok(value) => panic!("space in input must not construct a JobId; got {value}"),
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

    // When Ana calls JobId::from_str on that input.
    let outcome = JobId::from_str(&input);

    // Then Ana receives a parse error naming the length violation. The
    // structured variant carries `kind` and `max` (the ceiling — one less
    // than the offending length). And no JobId value is constructed.
    match outcome {
        Err(IdParseError::TooLong { kind, max }) => {
            assert_eq!(kind, "JobId", "TooLong.kind must name the newtype");
            assert_eq!(max, 253, "TooLong.max must carry the DNS-label ceiling (253)");
        }
        Err(other) => panic!("expected IdParseError::TooLong, got {other:?}"),
        Ok(value) => panic!("254-char input must not construct a JobId; got {value}"),
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

    // When Ana calls JobId::from_str on that input.
    let outcome = JobId::from_str(&input);

    // Then Ana receives a valid JobId. The positive side of the boundary
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
