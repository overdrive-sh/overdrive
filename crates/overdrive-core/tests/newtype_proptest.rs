//! Proptest — mandatory newtype round-trip call site.
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites*:
//!
//! > **Newtype roundtrip.** Every newtype's `Display` / `FromStr` / serde
//! > must round-trip bit-equivalent for every valid input, and every
//! > invalid input must be rejected by `FromStr` with a structured
//! > `ParseError`.
//!
//! This file covers `JobId`, `NodeId`, and `AllocationId` — the three
//! identifiers under step 01-01. The extended identifier set (US-02)
//! lands in a later step with its own proptest module.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{
    AllocationId, ContentHash, IdParseError, JobId, NodeId, Region, SpiffeId,
};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators — only valid inputs.
//
// The label newtypes accept DNS-1123-label-like strings:
//   * lowercase ASCII letters, digits, `-`, `_`, `.`
//   * first and last char must be alphanumeric
//   * non-empty, ≤ 253 chars
//
// We build the generator as (first_alnum, middle_body, last_alnum) so
// every drawn string satisfies the start-and-end-alnum rule by
// construction. The middle body draws from the full allowed class.
//
// `FromStr` is case-insensitive — uppercase inputs are lowercased in
// canonical form. We therefore draw mixed-case inputs to exercise that
// leg of the contract: the round-trip must still be lossless because
// `Display` emits the canonical (lowercase) form.
// -----------------------------------------------------------------------------

const ALNUM_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const INTERIOR_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.";

/// A single alphanumeric character (upper- or lowercase letter, or digit).
fn alnum_char() -> impl Strategy<Value = char> {
    proptest::sample::select(ALNUM_CHARS.chars().collect::<Vec<_>>())
}

/// One character from the allowed interior class: alphanumeric (any case),
/// `-`, `_`, `.`.
fn interior_char() -> impl Strategy<Value = char> {
    proptest::sample::select(INTERIOR_CHARS.chars().collect::<Vec<_>>())
}

/// A valid label-newtype input with length in `1..=32`.
///
/// Keeping the upper bound small (vs the 253-char ceiling) keeps
/// shrinking fast; the length ceiling is covered by a dedicated
/// boundary test in `id.rs`.
///
/// The validator requires both the first AND last character to be
/// alphanumeric. We encode this in the generator by producing:
///
///   * the 1-char case: a single alphanumeric
///   * the ≥ 2-char case: alnum + interior-body + alnum
///
/// so no drawn string can end with `-`, `_`, or `.`.
fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        // Single-character case.
        alnum_char().prop_map(|c| c.to_string()),
        // Two-or-more case: alnum, optional interior body, terminal alnum.
        (alnum_char(), prop::collection::vec(interior_char(), 0..=30), alnum_char(),).prop_map(
            |(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }
        ),
    ]
}

// -----------------------------------------------------------------------------
// Round-trip properties — Display / FromStr / serde must compose to
// identity for every valid input.
//
// The default CI budget is PROPTEST_CASES=1024 per `.claude/rules/testing.md`;
// each proptest below implicitly inherits that via the env.
// -----------------------------------------------------------------------------

proptest! {
    /// JobId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 1 under the property budget.
    #[test]
    fn job_id_display_from_str_round_trip(raw in valid_label()) {
        let original = JobId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = JobId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// NodeId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 2 under the property budget.
    #[test]
    fn node_id_display_from_str_round_trip(raw in valid_label()) {
        let original = NodeId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = NodeId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// AllocationId round-trips through Display -> FromStr.
    ///
    /// Covers §2.1 scenario 3 under the property budget.
    #[test]
    fn allocation_id_display_from_str_round_trip(raw in valid_label()) {
        let original = AllocationId::new(&raw).expect("generator yields valid input");
        let rendered = original.to_string();
        let reparsed = AllocationId::from_str(&rendered).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, original);
    }

    /// serde JSON output equals the Display form surrounded by quotes
    /// for every valid JobId.
    ///
    /// Covers §2.1 scenario 4 (the JSON-byte-equivalence leg).
    #[test]
    fn job_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = JobId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: JobId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }

    #[test]
    fn node_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = NodeId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: NodeId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }

    #[test]
    fn allocation_id_serde_matches_display_quoted(raw in valid_label()) {
        let id = AllocationId::new(&raw).expect("generator yields valid input");
        let json = serde_json::to_string(&id).expect("serialises");
        let expected = format!("\"{id}\"");
        prop_assert_eq!(&json, &expected);
        let back: AllocationId = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(back, id);
    }
}

// -----------------------------------------------------------------------------
// Invalid-input rejection — the other half of the mandatory call site.
//
// Every invalid input must be rejected by `FromStr` with a structured
// `ParseError`. We cover the three canonical rejection shapes:
//   * empty string          -> Empty
//   * forbidden character   -> InvalidChar
//   * leading non-alnum     -> InvalidFormat
//
// Length-ceiling rejection is covered by a dedicated `#[test]` in
// `id.rs`; the generator here stays under the ceiling so the property
// stays focused.
// -----------------------------------------------------------------------------

/// A character NOT in the label allowed class.
fn forbidden_char() -> impl Strategy<Value = char> {
    // Pick from a hand-curated set of clearly-invalid printable
    // characters. Keeps shrinking tight and avoids the generator-vs-
    // validator tautology trap.
    prop_oneof![
        Just(' '),
        Just('!'),
        Just('@'),
        Just('#'),
        Just('/'),
        Just('\\'),
        Just('*'),
        Just(':'),
    ]
}

proptest! {
    /// An input containing a forbidden character must be rejected with
    /// a structured `InvalidChar` variant.
    #[test]
    fn job_id_rejects_forbidden_character(
        prefix in valid_label(),
        bad in forbidden_char(),
    ) {
        let mut raw = prefix;
        raw.push(bad);
        raw.push('x'); // keep last char valid so only the forbidden one trips
        let err = JobId::from_str(&raw).expect_err("forbidden char must reject");
        let is_invalid_char = matches!(err, IdParseError::InvalidChar { .. });
        prop_assert!(is_invalid_char, "expected InvalidChar variant");
    }
}

#[test]
fn job_id_rejects_empty_input_with_structured_error() {
    // Empty is a single case, not a property.
    let err = JobId::from_str("").expect_err("empty must reject");
    assert!(matches!(err, IdParseError::Empty { .. }));
}

#[test]
fn node_id_rejects_leading_hyphen_with_structured_error() {
    // Leading-non-alnum is a structural boundary case, not a property.
    let err = NodeId::from_str("-leading-hyphen").expect_err("leading hyphen must reject");
    assert!(matches!(err, IdParseError::InvalidFormat { .. }));
}

// -----------------------------------------------------------------------------
// Region case-insensitivity — property per §3.1.
//
// For any valid label input `r`, upper- and lower-case variants must parse
// to the same canonical form under `Display`. The case-insensitivity
// contract is declared on the newtype (see `.claude/rules/development.md`
// — *Newtype completeness*) and is the only way call sites avoid ad-hoc
// normalisation helpers.
// -----------------------------------------------------------------------------

proptest! {
    #[test]
    fn region_case_insensitive_canonical_form_round_trip(raw in valid_label()) {
        // `to_ascii_uppercase` / `to_ascii_lowercase` preserve non-alphabetic
        // characters (digits, `-`, `_`, `.`), so generator constraints on
        // the label shape remain satisfied after case-folding.
        let upper = raw.to_ascii_uppercase();
        let lower = raw.to_ascii_lowercase();

        let from_upper =
            Region::from_str(&upper).expect("upper-case valid label must parse as Region");
        let from_lower =
            Region::from_str(&lower).expect("lower-case valid label must parse as Region");

        // Equal values: case-insensitivity is bidirectional — both inputs
        // reach the same canonical form.
        prop_assert_eq!(&from_upper, &from_lower);

        // And canonical form is lowercase. Render once, compare via
        // references so no intermediate clone is needed and `prop_assert_eq!`
        // still sees matching owned-vs-owned types.
        let rendered_upper = from_upper.to_string();
        let rendered_lower = from_lower.to_string();
        prop_assert_eq!(&rendered_upper, &lower);
        prop_assert_eq!(&rendered_lower, &lower);
    }
}

// -----------------------------------------------------------------------------
// SpiffeId round-trip — property per §3.1.
//
// For any valid `(trust_domain, path)` tuple, the SPIFFE URI built from
// those parts must round-trip losslessly through `Display` and `FromStr`.
// The trust domain is a DNS-like label; the path is one or more segments
// separated by `/`, each a valid label.
//
// The generator deliberately stays lowercase to keep the round-trip
// Display-output-equals-input property strong: case-insensitivity for
// SpiffeId is a separate concern handled by construction ordering
// (`to_ascii_lowercase` happens inside `new`), and mixing it here would
// weaken the byte-for-byte equality check the acceptance test in §3.1
// asserts on the whitepaper canonical example.
// -----------------------------------------------------------------------------

/// A single lowercase-alphanumeric character — the strict subset used in
/// the SPIFFE generator to keep canonical Display byte-equal to input.
fn lower_alnum_char() -> impl Strategy<Value = char> {
    proptest::sample::select("abcdefghijklmnopqrstuvwxyz0123456789".chars().collect::<Vec<_>>())
}

/// One character from the allowed label-interior class, lowercase-only.
fn lower_interior_char() -> impl Strategy<Value = char> {
    proptest::sample::select("abcdefghijklmnopqrstuvwxyz0123456789-_.".chars().collect::<Vec<_>>())
}

/// A single label — valid under the DNS-1123-like rules, lowercase-only,
/// ≥ 1 char. Mirrors `valid_label()` above but with no uppercase branch,
/// so `to_string()` output equals the generated input byte-for-byte.
fn lower_label() -> impl Strategy<Value = String> {
    prop_oneof![
        lower_alnum_char().prop_map(|c| c.to_string()),
        (
            lower_alnum_char(),
            prop::collection::vec(lower_interior_char(), 0..=8),
            lower_alnum_char(),
        )
            .prop_map(|(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }),
    ]
}

/// A SPIFFE path — one or more leading-slash-separated label segments.
fn spiffe_path() -> impl Strategy<Value = String> {
    prop::collection::vec(lower_label(), 1..=4).prop_map(|segments| {
        let mut p = String::new();
        for seg in segments {
            p.push('/');
            p.push_str(&seg);
        }
        p
    })
}

proptest! {
    #[test]
    fn spiffe_id_round_trips_through_display_and_from_str(
        trust_domain in lower_label(),
        path in spiffe_path(),
    ) {
        let uri = format!("spiffe://{trust_domain}{path}");

        let parsed = SpiffeId::from_str(&uri).expect("generator yields valid SPIFFE URI");

        // Display output equals the input byte-for-byte because the
        // generator stays in the already-canonical (lowercase) form.
        // Render once, compare by reference — avoids both a clone and
        // the `String == &String` type mismatch inside `prop_assert_eq!`.
        let rendered = parsed.to_string();
        prop_assert_eq!(&rendered, &uri);

        // Accessors return the exact segments, never a re-parse of the
        // stored string — this is the anti-string-splitting guard from AC.
        prop_assert_eq!(parsed.trust_domain(), &trust_domain);
        prop_assert_eq!(parsed.path(), &path);

        // And re-parsing the rendered form yields an equal value — the
        // round-trip leg proper.
        let reparsed =
            SpiffeId::from_str(&parsed.to_string()).expect("canonical form re-parses");
        prop_assert_eq!(reparsed, parsed);
    }
}

// -----------------------------------------------------------------------------
// ContentHash — hex round-trip and `ContentHash::of` determinism.
//
// Per `.claude/rules/testing.md` *Mandatory call sites — Hash
// determinism paths*: "Any content hash under `development.md`'s
// 'Hashing requires deterministic serialization' rule — N permutations
// of the same logical value must produce one hash." `ContentHash::of`
// is the primitive that anchors every content-addressed ID in the
// platform (WASM modules, chunks, SchematicId, diagnostic-probe
// catalogue entries); a non-deterministic implementation silently
// breaks content-addressed routing everywhere.
// -----------------------------------------------------------------------------

proptest! {
    /// For any valid 32-byte digest, rendering via `Display` and
    /// re-parsing via `FromStr` round-trips to the same digest.
    ///
    /// Covers §3.1 "A ContentHash round-trips through its 64-character
    /// hex form" under the property budget.
    #[test]
    fn content_hash_hex_round_trip(bytes in proptest::array::uniform32(any::<u8>())) {
        let original = ContentHash::from_bytes(bytes);
        let rendered = original.to_string();
        // The canonical hex form is always 64 characters — 32 bytes * 2
        // hex digits per byte. Shape check before the parse guards the
        // length-ceiling branch against mutations that silently break
        // the Display impl.
        prop_assert_eq!(rendered.len(), 64);

        let reparsed = ContentHash::from_str(&rendered).expect("canonical hex form re-parses");
        prop_assert_eq!(reparsed, original);

        // The underlying bytes survive the trip byte-for-byte — not
        // merely `Eq`-equivalent under the derived impl.
        prop_assert_eq!(reparsed.as_bytes(), &bytes);
    }

    /// For any byte payload, `ContentHash::of(&p) == ContentHash::of(&p)`
    /// and the underlying digest is bit-identical across invocations.
    ///
    /// Covers §3.1 "A content hash is stable across invocations for any
    /// byte payload" under the property budget. This is the mandatory
    /// hash-determinism call site per `.claude/rules/testing.md`.
    #[test]
    fn content_hash_of_is_deterministic_over_any_byte_payload(
        payload in prop::collection::vec(any::<u8>(), 0..=256),
    ) {
        let first = ContentHash::of(&payload);
        let second = ContentHash::of(&payload);

        // Equal under the derived `Eq` impl — catches hash-instability
        // in the declarative sense.
        prop_assert_eq!(first, second);

        // And bit-identical in the underlying digest — catches a
        // mutation that preserves `Eq` but corrupts the bytes. The raw
        // 32-byte array carries the actual contract.
        prop_assert_eq!(first.as_bytes(), second.as_bytes());
    }
}
