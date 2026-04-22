//! Acceptance scenarios for US-02 §3.1 (`SpiffeId` happy-path + `Region`
//! normalisation) and §3.2 (`SpiffeId` + `Region` error boundaries).
//!
//! Translates the five scenarios below from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` directly
//! into Rust `#[test]` bodies:
//!
//! * §3.1 — SPIFFE identity parses from the whitepaper canonical example,
//!   exposing `trust_domain()` and `path()` accessors; `Display` round-trips
//!   byte-for-byte.
//! * §3.1 — Region parses case-insensitively; `Display` emits lowercase
//!   canonical form.
//! * §3.2 — SPIFFE missing scheme rejected with `SpiffeMissingScheme`.
//! * §3.2 — SPIFFE empty trust domain rejected with `SpiffeEmptyTrustDomain`.
//! * §3.2 — SPIFFE empty path rejected with `SpiffeEmptyPath`.
//! * §3.2 — Region with whitespace rejected with `InvalidChar`.
//!
//! Enters through the driving port for each newtype (its public `FromStr`
//! impl) and asserts the observable outcome: the accessors, the canonical
//! `Display` form, and the structured `Err` variant shape. No internal
//! state is peeked; call sites never string-split the SPIFFE URI.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{IdParseError, Region, SpiffeId};

// -----------------------------------------------------------------------------
// §3.1 — SPIFFE identity parses from the whitepaper canonical example.
// -----------------------------------------------------------------------------

#[test]
fn spiffe_id_parses_canonical_whitepaper_example_and_exposes_accessors() {
    // Given the input from whitepaper §8.
    let input = "spiffe://overdrive.local/job/payments/alloc/a1b2c3";

    // When Ana constructs a SpiffeId from that input.
    let id = SpiffeId::from_str(input).expect("canonical SPIFFE URI must parse");

    // Then Ana receives a SpiffeId whose trust domain accessor returns the
    // domain segment — no string-splitting at the call site.
    assert_eq!(
        id.trust_domain(),
        "overdrive.local",
        "trust_domain() must return the segment between `spiffe://` and the first path `/`"
    );

    // And whose path accessor returns everything from (and including) the
    // leading `/` onward.
    assert_eq!(
        id.path(),
        "/job/payments/alloc/a1b2c3",
        "path() must return the leading-`/`-prefixed path"
    );

    // And whose Display output equals the input byte-for-byte — the input
    // is already lowercase, so canonicalisation is a no-op here; §3.1
    // asserts the round-trip specifically.
    assert_eq!(
        id.to_string(),
        input,
        "Display must round-trip byte-for-byte on already-canonical input"
    );
}

// -----------------------------------------------------------------------------
// §3.1 — Region parses case-insensitively and emits a lowercase canonical
// form.
// -----------------------------------------------------------------------------

#[test]
fn region_parses_case_insensitively_and_emits_lowercase_canonical() {
    // Given the input read from a cluster config file.
    let mixed_case = "EU-West-1";

    // When Ana constructs a Region from that input.
    let region = Region::from_str(mixed_case).expect("mixed-case region must parse");

    // Then its Display output is the lowercase canonical form.
    assert_eq!(
        region.to_string(),
        "eu-west-1",
        "Region Display must emit the lowercase canonical form"
    );

    // And the lowercase-input variant parses to the same value — proving the
    // case-insensitivity contract is bidirectional (both inputs reach the
    // same canonical form).
    let lowercase = Region::from_str("eu-west-1").expect("lowercase region must parse");
    assert_eq!(region, lowercase, "mixed-case and lowercase inputs must canonicalise equally");
    assert_eq!(region.to_string(), lowercase.to_string());
}

// -----------------------------------------------------------------------------
// §3.2 — SPIFFE string without the scheme is rejected with
// `SpiffeMissingScheme`.
// -----------------------------------------------------------------------------

#[test]
fn spiffe_missing_scheme_is_rejected_with_missing_scheme_variant() {
    // Given the input — a path that looks like a SPIFFE body but lacks the
    // `spiffe://` scheme.
    let input = "overdrive.local/job/payments";

    // When Ana constructs a SpiffeId from that input.
    let outcome = SpiffeId::from_str(input);

    // Then Ana receives a SpiffeMissingScheme error naming the input. And
    // no SpiffeId is constructed (enforced by pattern-matching the Err arm).
    match outcome {
        Err(IdParseError::SpiffeMissingScheme(raw)) => {
            assert_eq!(raw, input, "SpiffeMissingScheme must carry the offending input verbatim");
        }
        Err(other) => panic!("expected IdParseError::SpiffeMissingScheme, got {other:?}"),
        Ok(value) => panic!("missing-scheme input must not construct a SpiffeId; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §3.2 — SPIFFE string with an empty trust domain is rejected with
// `SpiffeEmptyTrustDomain`.
// -----------------------------------------------------------------------------

#[test]
fn spiffe_empty_trust_domain_is_rejected_with_empty_trust_domain_variant() {
    // Given the input — a scheme-prefixed string with an empty trust domain.
    let input = "spiffe:///job/payments";

    // When Ana constructs a SpiffeId from that input.
    let outcome = SpiffeId::from_str(input);

    // Then Ana receives a SpiffeEmptyTrustDomain error naming the input. And
    // no SpiffeId is constructed.
    match outcome {
        Err(IdParseError::SpiffeEmptyTrustDomain(raw)) => {
            assert_eq!(
                raw, input,
                "SpiffeEmptyTrustDomain must carry the offending input verbatim"
            );
        }
        Err(other) => panic!("expected IdParseError::SpiffeEmptyTrustDomain, got {other:?}"),
        Ok(value) => panic!("empty-trust-domain input must not construct a SpiffeId; got {value}"),
    }
}

// -----------------------------------------------------------------------------
// §3.2 — SPIFFE string with an empty path is rejected with
// `SpiffeEmptyPath`.
// -----------------------------------------------------------------------------

#[test]
fn spiffe_empty_path_is_rejected_with_empty_path_variant() {
    // Given the input — a scheme + trust domain with a trailing `/` and
    // nothing after. §3.2 names the empty-path boundary explicitly.
    let input = "spiffe://overdrive.local/";

    // When Ana constructs a SpiffeId from that input.
    let outcome = SpiffeId::from_str(input);

    // Then Ana receives a SpiffeEmptyPath error naming the input. And no
    // SpiffeId is constructed.
    match outcome {
        Err(IdParseError::SpiffeEmptyPath(raw)) => {
            assert_eq!(raw, input, "SpiffeEmptyPath must carry the offending input verbatim");
        }
        Err(other) => panic!("expected IdParseError::SpiffeEmptyPath, got {other:?}"),
        Ok(value) => panic!("empty-path input must not construct a SpiffeId; got {value}"),
    }
}

// Companion: a trust-domain-only input with no path `/` at all is also an
// empty-path boundary. Without this pair, the `SpiffeEmptyPath` branch in
// the validator has two distinct reach-paths (missing-slash vs
// trailing-slash) and a mutation that collapses one into the other would
// stay caught only on one leg. Both legs are named in §3.2 by implication
// ("empty path"), so the pair belongs in the acceptance suite.
#[test]
fn spiffe_trust_domain_only_is_rejected_with_empty_path_variant() {
    // Given the input — scheme + trust domain, no path separator.
    let input = "spiffe://overdrive.local";

    // When Ana constructs a SpiffeId from that input.
    let outcome = SpiffeId::from_str(input);

    // Then Ana receives a SpiffeEmptyPath error.
    match outcome {
        Err(IdParseError::SpiffeEmptyPath(raw)) => {
            assert_eq!(raw, input);
        }
        Err(other) => panic!("expected IdParseError::SpiffeEmptyPath, got {other:?}"),
        Ok(value) => {
            panic!("trust-domain-only input must not construct a SpiffeId; got {value}")
        }
    }
}

// -----------------------------------------------------------------------------
// §3.2 — Region containing a space is rejected with `InvalidChar`.
// -----------------------------------------------------------------------------

#[test]
fn region_with_whitespace_is_rejected_with_invalid_char_variant() {
    // Given the input — a region-like string with interior spaces.
    let input = "eu west 1";
    let expected_char = ' ';
    let expected_position = 2_usize;

    // When Ana constructs a Region from that input.
    let outcome = Region::from_str(input);

    // Then Ana receives a parse error naming the invalid character and its
    // position. And no Region is constructed.
    match outcome {
        Err(IdParseError::InvalidChar { kind, ch, index }) => {
            assert_eq!(kind, "Region", "InvalidChar.kind must name the Region newtype");
            assert_eq!(ch, expected_char, "InvalidChar.ch must carry the offending whitespace");
            assert_eq!(
                index, expected_position,
                "InvalidChar.index must point at the first offending byte"
            );
        }
        Err(other) => panic!("expected IdParseError::InvalidChar, got {other:?}"),
        Ok(value) => panic!("whitespace input must not construct a Region; got {value}"),
    }
}
