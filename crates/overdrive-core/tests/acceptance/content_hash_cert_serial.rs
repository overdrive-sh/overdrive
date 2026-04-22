//! Acceptance scenarios for US-02 §3.1 (`ContentHash` round-trip and
//! `ContentHash::of` determinism, `CorrelationKey` determinism) and §3.2
//! (`ContentHash` wrong-length, `CertSerial` uppercase-hex and odd-length
//! error boundaries).
//!
//! Translates the seven scenarios below from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` directly
//! into Rust `#[test]` bodies:
//!
//! * §3.1 — `ContentHash` round-trips through its 64-character hex form.
//! * §3.1 — `ContentHash::of` is stable across invocations for any byte
//!   payload (universal invariant — mirrored as a proptest in
//!   `tests/newtype_proptest.rs` per `.claude/rules/testing.md` *Hash
//!   determinism paths*).
//! * §3.1 — `CorrelationKey` derived from the same `(target, spec_hash,
//!   purpose)` triple twice is equal.
//! * §3.2 — A hex input three characters long is rejected with
//!   `IdParseError::ContentHashWrongLength { expected: 64, actual: 3 }`.
//! * §3.2 — A cert serial containing uppercase hex is rejected with
//!   `InvalidChar`.
//! * §3.2 — A cert serial with an odd number of hex digits is rejected
//!   with `InvalidFormat`.
//!
//! In addition, this module asserts the §2 `SchematicId` rustdoc
//! commitment to rkyv-archived-bytes canonicalisation per ADR-0002.
//! The struct that `SchematicId` canonicalises is deferred to Phase 2;
//! Phase 1 ships the newtype with the rule documented in rustdoc only.
//!
//! Enters through each newtype's driving port (its public
//! `FromStr` / `from_hex` / `new` / `derive` impls) and asserts the
//! observable outcome: the `Display` byte form, the structured `Err`
//! variant shape, and — for the rustdoc commitment — a byte scan of
//! the source file at the fixed path agreed with ADR-0002.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{CertSerial, ContentHash, CorrelationKey, IdParseError, SchematicId};

// -----------------------------------------------------------------------------
// §3.1 — `ContentHash` round-trips through its 64-character hex form.
// -----------------------------------------------------------------------------

#[test]
fn content_hash_round_trips_through_sixty_four_char_hex_form() {
    // Given the ContentHash of the payload "overdrive".
    let payload: &[u8] = b"overdrive";
    let original = ContentHash::of(payload);

    // When Ana formats it via Display and parses the result with FromStr.
    let rendered = original.to_string();
    let reparsed = ContentHash::from_str(&rendered).expect("canonical hex form must re-parse");

    // Then the hex form is exactly 64 characters — 32 bytes * 2 hex digits.
    assert_eq!(rendered.len(), 64, "ContentHash Display must emit 64 hex characters");

    // And Ana receives the original ContentHash.
    assert_eq!(reparsed, original, "Display -> FromStr round-trip must be lossless");

    // And the raw 32-byte digest survives the trip intact.
    assert_eq!(
        reparsed.as_bytes(),
        original.as_bytes(),
        "underlying 32-byte digest must survive the round-trip"
    );
}

// -----------------------------------------------------------------------------
// §3.1 — `ContentHash::of` is stable across invocations for any byte payload.
//
// This is the single-case witness of the proptest in
// `tests/newtype_proptest.rs`. Both must hold: the proptest guards
// arbitrary inputs, this acceptance test names the contract in the
// scenario set.
// -----------------------------------------------------------------------------

#[test]
fn content_hash_of_is_deterministic_across_two_invocations() {
    // Given any byte payload.
    let payload: &[u8] = b"schematic-body-v1\nkey = \"value\"\n";

    // When Ana computes ContentHash::of on two separate invocations.
    let first = ContentHash::of(payload);
    let second = ContentHash::of(payload);

    // Then the two resulting ContentHash values are equal.
    assert_eq!(first, second, "ContentHash::of must be deterministic over the same payload");

    // And their underlying bytes are identical — not merely compared by
    // the derived `Eq`, but digest-equal.
    assert_eq!(
        first.as_bytes(),
        second.as_bytes(),
        "underlying SHA-256 digest must be bit-identical across invocations"
    );
}

// -----------------------------------------------------------------------------
// §3.1 — `CorrelationKey::derive` is deterministic for the same triple.
// -----------------------------------------------------------------------------

#[test]
fn correlation_key_derive_is_deterministic_across_two_invocations() {
    // Given a target, a SHA-256 hash of a known spec, and a purpose.
    let target = "payments";
    let spec_hash = ContentHash::of(b"spec-v1");
    let purpose = "register";

    // When Ana derives a CorrelationKey twice from those three inputs.
    let first = CorrelationKey::derive(target, &spec_hash, purpose);
    let second = CorrelationKey::derive(target, &spec_hash, purpose);

    // Then the two derived CorrelationKey values are equal.
    assert_eq!(first, second, "CorrelationKey::derive must be deterministic");
}

// -----------------------------------------------------------------------------
// §3.2 — A content-hash hex string of the wrong length is rejected.
// -----------------------------------------------------------------------------

#[test]
fn content_hash_wrong_length_is_rejected_with_structured_error() {
    // Given a hex input three characters long.
    let input = "abc";

    // When Ana constructs a ContentHash from the hex string.
    let outcome = ContentHash::from_str(input);

    // Then Ana receives a parse error naming the expected and actual
    // lengths — the expected length is 64 (a SHA-256 hex form) and the
    // actual is the length of the input.
    match outcome {
        Err(IdParseError::ContentHashWrongLength { expected, actual }) => {
            assert_eq!(expected, 64, "expected hex length must be 64 chars (32-byte SHA-256)");
            assert_eq!(actual, input.len(), "actual hex length must match the input length");
        }
        Err(other) => panic!("expected IdParseError::ContentHashWrongLength, got {other:?}"),
        Ok(value) => panic!("wrong-length hex input must not construct a ContentHash; got {value}"),
    }
}

// A companion at the other length boundary. `ContentHashWrongLength`
// ships as a single variant — both "too short" and "too long" reach
// through the same branch. A mutation that collapses the length check
// into a one-sided comparison would stay caught only on one leg without
// this pair.
#[test]
fn content_hash_too_long_hex_is_rejected_with_structured_error() {
    // Given a hex input that is too long to be a SHA-256 digest — 65
    // lowercase hex digits, one more than the canonical 64.
    let input = "a".repeat(65);

    // When Ana constructs a ContentHash from the hex string.
    let outcome = ContentHash::from_str(&input);

    // Then Ana receives the same structured error variant.
    match outcome {
        Err(IdParseError::ContentHashWrongLength { expected, actual }) => {
            assert_eq!(expected, 64);
            assert_eq!(actual, 65);
        }
        Err(other) => panic!("expected IdParseError::ContentHashWrongLength, got {other:?}"),
        Ok(value) => panic!("over-long hex input must not construct a ContentHash; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §3.2 — A cert serial containing uppercase hex is rejected.
// -----------------------------------------------------------------------------

#[test]
fn cert_serial_uppercase_hex_is_rejected_with_invalid_char_variant() {
    // Given the input — an all-uppercase hex serial.
    let input = "ABCD";

    // When Ana constructs a CertSerial from that input.
    let outcome = CertSerial::from_str(input);

    // Then Ana receives a parse error naming the invalid character. And
    // no CertSerial is constructed.
    match outcome {
        Err(IdParseError::InvalidChar { kind, ch, index }) => {
            assert_eq!(kind, "CertSerial", "InvalidChar.kind must name the CertSerial newtype");
            // The first offending character is the leading 'A' at byte 0.
            assert_eq!(ch, 'A', "InvalidChar.ch must carry the first uppercase hex digit");
            assert_eq!(index, 0, "InvalidChar.index must point at the first offending byte");
        }
        Err(other) => panic!("expected IdParseError::InvalidChar, got {other:?}"),
        Ok(value) => panic!("uppercase hex input must not construct a CertSerial; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §3.2 — A cert serial with an odd number of hex digits is rejected.
// -----------------------------------------------------------------------------

#[test]
fn cert_serial_odd_length_is_rejected_with_invalid_format_variant() {
    // Given the input — three lowercase hex digits, an odd number.
    let input = "abc";

    // When Ana constructs a CertSerial from that input.
    let outcome = CertSerial::from_str(input);

    // Then Ana receives a parse error naming the format violation. And
    // no CertSerial is constructed.
    match outcome {
        Err(IdParseError::InvalidFormat { kind, expected }) => {
            assert_eq!(kind, "CertSerial", "InvalidFormat.kind must name the CertSerial newtype");
            assert_eq!(
                expected, "even number of hex digits",
                "InvalidFormat.expected must name the pair-of-hex-digits contract"
            );
        }
        Err(other) => panic!("expected IdParseError::InvalidFormat, got {other:?}"),
        Ok(value) => panic!("odd-length input must not construct a CertSerial; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// ADR-0002 — `SchematicId` rustdoc commits to rkyv-archived-bytes
// canonicalisation.
//
// The Phase 1 `SchematicId` is a transparent `ContentHash` newtype. The
// `Schematic` struct it hashes is deferred to Phase 2; Phase 1's
// contribution is the canonicalisation rule itself, expressed in rustdoc
// at the item level so future implementers cannot pick a different
// canonicalisation without superseding ADR-0002.
//
// We assert the rule survives in source form. A byte scan of the
// `id.rs` source file is the lightest possible check that survives
// Extract Method / Rename refactors on the struct itself. If the
// canonicalisation rule gets removed — say, by a future edit that
// reflows the rustdoc and drops the anchor phrase — this test fails.
// -----------------------------------------------------------------------------

#[test]
fn schematic_id_rustdoc_commits_to_rkyv_archived_bytes_canonicalisation() {
    // Given the source file that defines `SchematicId`.
    let src_path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/id.rs");
    let source = std::fs::read_to_string(src_path)
        .expect(&format!("must be able to read the id.rs source file at {src_path}"));

    // When Ana inspects the rustdoc commentary around `SchematicId`.
    // Then the commentary must carry the ADR-0002 anchor and name the
    // rkyv-archived-bytes canonicalisation rule verbatim. Two anchors are
    // asserted rather than one so a future edit cannot silently weaken
    // the commitment by dropping either half.
    assert!(
        source.contains("ADR-0002"),
        "SchematicId rustdoc must name ADR-0002 as the canonicalisation ADR"
    );
    assert!(
        source.contains("rkyv-archived bytes"),
        "SchematicId rustdoc must name rkyv-archived bytes as the canonicalisation input"
    );

    // And `SchematicId` must construct a value in the usual way — this
    // is a smoke test that the newtype still exists as a compile-time
    // artefact, so a future edit that deletes the struct (and hence
    // trivially satisfies the string scan on an empty file) still fails
    // here.
    let hash = ContentHash::of(b"any-schematic-body");
    let id = SchematicId::new(hash);
    assert_eq!(
        id.content_hash().as_bytes(),
        hash.as_bytes(),
        "SchematicId must expose the wrapped ContentHash transparently"
    );
}
