//! GH #372 / ADR-0340 — the CPU quantity grammar (`cpu = "500m"`, `cpu = "0.5"`, `cpu = "2"`).
//!
//! Driven through the type's own validating surface (`FromStr`, `Display`, serde): the parser
//! is a pure function and is itself the library entry point for AC-2.3 / AC-2.5. The
//! classification order the design pins is asserted by variant.
//!
//! Evidence tier: unit and property (layer 1, Mandate 9). Pending bodies are complete and
//! marked `#[should_panic(expected = ...)]` with the semantic RED each fails on today (the
//! scaffold refuses every input and renders nothing); DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::cpu_quantity::{CpuQuantity, CpuQuantityParseError};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 256,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

fn parse(text: &str) -> Result<CpuQuantity, CpuQuantityParseError> {
    text.parse::<CpuQuantity>()
}

/// Reference model of the canonical spelling, independent of the implementation.
fn model_canonical(millis: u32) -> String {
    if millis.is_multiple_of(1000) { format!("{}", millis / 1000) } else { format!("{millis}m") }
}

// ----- accepted spellings (pinned table rows) ---------------------------------------------

/// AC-2.1 / AC-2.5: every row of the accept table parses to the pinned millicores and renders
/// its canonical form: equal quantities in different spellings share one canonical text, a whole
/// number of cores renders as the integer, anything else as `<n>m`, zero as `0`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_pinned_accept_table_parses_to_the_pinned_quantity_and_canonical_form() {
    let table: &[(&str, u32, &str)] = &[
        ("500m", 500, "500m"),
        ("0.5", 500, "500m"),
        ("0.500", 500, "500m"),
        ("2", 2000, "2"),
        ("2000m", 2000, "2"),
        ("2.0", 2000, "2"),
        ("1.5", 1500, "1500m"),
        ("1.25", 1250, "1250m"),
        ("0", 0, "0"),
        ("0m", 0, "0"),
        ("0.0", 0, "0"),
        ("0.000", 0, "0"),
        ("0.001", 1, "1m"),
        ("007m", 7, "7m"),
        ("1000m", 1000, "1"),
        ("10", 10_000, "10"),
        ("4294967.295", 4_294_967_295, "4294967295m"),
        ("4294967295m", 4_294_967_295, "4294967295m"),
    ];
    for (input, millis, canonical) in table {
        let q = parse(input).unwrap_or_else(|e| {
            panic!("the cpu grammar must accept the pinned input {input:?}: {e}")
        });
        assert_eq!(q.millis(), *millis, "{input:?} must be {millis} millicores");
        assert_eq!(q.to_string(), *canonical, "{input:?} must render canonically");
    }
}

// ----- rejection classes, in the pinned classification order ----------------------------------

/// Control: the grammar still accepts a valid quantity, so a rejection class below cannot be
/// satisfied by a parser that refuses everything as `Malformed`.
fn assert_control_accepted() {
    let q = parse("500m").expect("the cpu grammar must accept the pinned control input 500m");
    assert_eq!(q.millis(), 500, "the cpu grammar must accept the pinned control input 500m");
}

fn assert_class(inputs: &[&str], class: &str, check: impl Fn(&CpuQuantityParseError) -> bool) {
    for input in inputs {
        match parse(input) {
            Ok(v) => panic!("the cpu grammar must refuse {input:?} as {class}; it accepted {v:?}"),
            Err(e) => assert!(
                check(&e),
                "the cpu grammar must classify {input:?} as {class}; it reported {e:?}"
            ),
        }
    }
}

/// AC-2.3: text that is not a quantity at all — empty, signs, padding, a missing whole part,
/// words, an exponent, a trailing dot, a fractional millicore, a unit after a decimal — is
/// `Malformed`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_text_outside_the_grammar_is_malformed() {
    assert_control_accepted();
    assert_class(
        &[
            "",
            ".5",
            " 1",
            "1 ",
            "5 cores",
            "1e3",
            "-1",
            "+1",
            "500.5m",
            "1.",
            "m",
            "500m ",
            "1.5m",
            "0.5.5",
            "1,5",
            "five",
            "five hundred",
        ],
        "Malformed",
        |e| matches!(e, CpuQuantityParseError::Malformed { .. }),
    );
}

/// AC-2.3: an alphabetic suffix other than lowercase `m` is `UnknownSuffix` carrying the suffix;
/// the uppercase `M` message says the milli suffix is the lowercase `m`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_an_unknown_suffix_is_named() {
    for (input, suffix) in [("500M", "M"), ("500mm", "mm"), ("2c", "c"), ("1cores", "cores")] {
        match parse(input) {
            Err(CpuQuantityParseError::UnknownSuffix { suffix: got, .. }) => {
                assert_eq!(got, suffix, "the unknown suffix must be carried verbatim");
            }
            other => panic!(
                "the cpu grammar must classify {input:?} as UnknownSuffix; it reported {other:?}"
            ),
        }
    }
    let text = error_text("500M");
    assert!(text.contains("lowercase"), "the uppercase M message must say lowercase m: {text}");
}

/// AC-2.3: a value above the `u32` millicore ceiling is `Overflow`, in every spelling that can
/// reach it, including huge digit runs (no panic).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_a_value_above_u32_millicores_is_overflow() {
    let long = format!("{}m", "9".repeat(400));
    assert_class(
        &["4294967296m", "4294967.296", "4294968", "99999999999999999999m", long.as_str()],
        "Overflow",
        |e| matches!(e, CpuQuantityParseError::Overflow { .. }),
    );
}

fn error_text(input: &str) -> String {
    match parse(input) {
        Ok(v) => panic!("the error text of {input:?} cannot be read: it parsed as {v:?}"),
        Err(e) => e.to_string(),
    }
}

/// AC-2.3: the rejection text carries the wording DESIGN pins per class (four fractional digits
/// say three, overflow says the value is too large; the uppercase-`M` wording is pinned by
/// `sad_an_unknown_suffix_is_named`) and the four messages differ.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_every_rejection_names_its_remedy_and_the_messages_differ() {
    let malformed = error_text("five");
    let suffix = error_text("500mm");
    let decimals = error_text("1.0005").to_lowercase();
    assert!(
        decimals.contains("three"),
        "the error text must name the three-digit limit: {decimals}"
    );
    let overflow = error_text("4294967296m").to_lowercase();
    assert!(
        overflow.contains("too large"),
        "the error text must say the value is too large: {overflow}"
    );
    let all = [malformed, suffix, decimals, overflow];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "the error text must differ per rejection class");
        }
    }
}

// ----- serde -----------------------------------------------------------------------------------

/// AC-2.3: a bare JSON number is a deserialization error, while the quoted quantity still
/// deserializes. CONTRACT_SHAPE: pure-function.
#[test]
fn sad_serde_refuses_a_bare_number() {
    let quoted = serde_json::from_str::<CpuQuantity>("\"500m\"")
        .expect("a quoted quantity must deserialize");
    assert_eq!(quoted.millis(), 500, "a quoted quantity must deserialize to its millicores");
    for wire in ["500", "0.5", "true", "null"] {
        assert!(
            serde_json::from_str::<CpuQuantity>(wire).is_err(),
            "a non-string quantity must not deserialize: {wire}"
        );
    }
}

// ----- properties ------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// AC-2.5: for every quantity, `parse(display(v)) == v`, and the canonical text matches the
    /// independent model. CONTRACT_SHAPE: pure-function.
    #[test]
    fn display_is_canonical_and_round_trips(millis in any::<u32>()) {
        let q = CpuQuantity::from_millis(millis);
        prop_assert_eq!(q.to_string(), model_canonical(millis), "a cpu quantity must display canonically");
        prop_assert_eq!(q.to_string().parse::<CpuQuantity>().expect("the displayed quantity must parse"), q);
    }

    /// Equal quantities spelled as millicores, as three-digit decimal cores, and in canonical form
    /// are one value with one canonical text (the precondition of AC-2.5's equal digests).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_spelling_of_one_quantity_is_the_same_value(millis in any::<u32>()) {
        let milli_form = format!("{millis}m");
        let core_form = format!("{}.{:03}", millis / 1000, millis % 1000);
        let canonical = model_canonical(millis);
        let parse_or_fail = |s: &str| {
            s.parse::<CpuQuantity>().map_err(|e| TestCaseError::fail(format!("every spelling of one quantity must parse: {s}: {e}")))
        };
        let a = parse_or_fail(&milli_form)?;
        let b = parse_or_fail(&core_form)?;
        let c = parse_or_fail(&canonical)?;
        prop_assert_eq!(a.millis(), millis);
        prop_assert_eq!(a, b);
        prop_assert_eq!(b, c);
        prop_assert_eq!(a.to_string(), c.to_string());
    }

    /// Serde JSON is exactly the quoted canonical text and reads back. CONTRACT_SHAPE: pure-function.
    #[test]
    fn serde_json_is_the_quoted_canonical_text_and_round_trips(millis in any::<u32>()) {
        let q = CpuQuantity::from_millis(millis);
        let json = serde_json::to_string(&q).expect("serialises");
        prop_assert_eq!(&json, &format!("\"{}\"", model_canonical(millis)), "serde must emit the quoted canonical text");
        prop_assert_eq!(serde_json::from_str::<CpuQuantity>(&json).expect("reads back"), q);
    }

    /// More than three fractional digits is always `TooManyDecimals`, whatever the digits.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn four_or_more_fractional_digits_are_always_too_many_decimals(whole in 0u32..=4000, frac in "[0-9]{4,12}") {
        let input = format!("{whole}.{frac}");
        prop_assert!(
            matches!(parse(&input), Err(CpuQuantityParseError::TooManyDecimals { .. })),
            "four or more fractional digits must be TooManyDecimals: {input}"
        );
    }

    /// Text without a leading ASCII digit is `Malformed`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn text_without_a_leading_digit_is_always_malformed(text in "[^0-9][ -~]{0,12}") {
        assert_control_accepted();
        let outcome = parse(&text);
        prop_assert!(
            matches!(&outcome, Err(CpuQuantityParseError::Malformed { .. })),
            "text without a leading digit must be Malformed: {text:?}: {outcome:?}"
        );
    }

    /// The `u32` millicore boundary is exact: a millicore count above `u32::MAX` is `Overflow`,
    /// one that fits is accepted (checked against `u64` arithmetic). The count is drawn from a
    /// window of two either side of `u32::MAX`, so every case sits on the boundary.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_u32_boundary_is_exact(offset in 0u64..=4) {
        let millis = u64::from(u32::MAX) - 2 + offset;
        let input = format!("{millis}m");
        let parsed = parse(&input);
        if millis > u64::from(u32::MAX) {
            prop_assert!(matches!(parsed, Err(CpuQuantityParseError::Overflow { .. })), "the u32 boundary must hold: {input} overflows");
        } else {
            let q = parsed.map_err(|e| TestCaseError::fail(format!("the u32 boundary must hold: {input} fits: {e}")))?;
            prop_assert_eq!(u64::from(q.millis()), millis);
        }
    }
}
