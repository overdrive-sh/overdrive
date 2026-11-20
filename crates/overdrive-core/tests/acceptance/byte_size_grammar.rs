//! GH #372 / ADR-0339 — the shared binary byte-size grammar (`memory = "128MiB"`).
//!
//! Driven through the type's own validating surface (`FromStr`, `Display`,
//! serde): the parser is a pure function and is itself the library entry point
//! for AC-1.3 / AC-1.4. The classification order the design pins is asserted by
//! variant, so a reordered scanner is caught.
//!
//! Evidence tier: unit and property (layer 1, Mandate 9). Scenario matrix:
//! `docs/feature/workload-resources-human-readable/feature-delta.md`
//! § "Wave: DISTILL / [REF] Scenario list".
//!
//! Pending bodies are complete and marked `#[should_panic(expected = ...)]` with
//! the semantic RED each fails on today (the scaffold refuses every input and
//! renders nothing); DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::byte_size::{ByteSize, ByteSizeParseError};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 256,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

fn parse(text: &str) -> Result<ByteSize, ByteSizeParseError> {
    text.parse::<ByteSize>()
}

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const TIB: u64 = 1 << 40;

/// Reference model of the canonical spelling, independent of the implementation:
/// the integer count in the largest of TiB, GiB, MiB, KiB, B that divides exactly.
fn model_canonical(bytes: u64) -> String {
    for (mult, unit) in [(TIB, "TiB"), (GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")] {
        if bytes != 0 && bytes.is_multiple_of(mult) {
            return format!("{}{unit}", bytes / mult);
        }
    }
    format!("{bytes}B")
}

// ----- accepted spellings (pinned table rows) ---------------------------------------------

/// Every row of the accept table parses to the pinned byte count and renders its
/// canonical form: unit case does not matter, equal sizes share one spelling, zero is `0B`.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the binary byte-size grammar must accept the pinned input")]
fn the_pinned_accept_table_parses_to_the_pinned_size_and_canonical_form() {
    let table: &[(&str, u64, &str)] = &[
        ("128MiB", 134_217_728, "128MiB"),
        ("128mib", 134_217_728, "128MiB"),
        ("128MIB", 134_217_728, "128MiB"),
        ("1024MiB", 1_073_741_824, "1GiB"),
        ("1GiB", 1_073_741_824, "1GiB"),
        ("1073741824B", 1_073_741_824, "1GiB"),
        ("1536MiB", 1_610_612_736, "1536MiB"),
        ("1024KiB", 1_048_576, "1MiB"),
        ("1536KiB", 1_572_864, "1536KiB"),
        ("0B", 0, "0B"),
        ("0MiB", 0, "0B"),
        ("0TiB", 0, "0B"),
        ("0128MiB", 134_217_728, "128MiB"),
        ("1b", 1, "1B"),
        ("1kib", 1024, "1KiB"),
        ("2TiB", 2 * TIB, "2TiB"),
        ("16777215TiB", 16_777_215 * TIB, "16777215TiB"),
        ("18446744073709551615B", u64::MAX, "18446744073709551615B"),
    ];
    for (input, bytes, canonical) in table {
        let size = parse(input).unwrap_or_else(|e| {
            panic!("the binary byte-size grammar must accept the pinned input {input:?}: {e}")
        });
        assert_eq!(size.bytes(), *bytes, "{input:?} must be {bytes} bytes");
        assert_eq!(size.to_string(), *canonical, "{input:?} must render canonically");
    }
}

// ----- rejection classes, in the pinned classification order ----------------------------------

fn assert_class(inputs: &[&str], class: &str, check: impl Fn(&ByteSizeParseError) -> bool) {
    for input in inputs {
        match parse(input) {
            Ok(v) => panic!("the grammar must refuse {input:?} as {class}; it accepted {v:?}"),
            Err(e) => assert!(
                check(&e),
                "the grammar must classify {input:?} as {class}; it reported {e:?}"
            ),
        }
    }
}

/// AC-1.3: decimal (SI) units are refused, in any case, as `DecimalUnit`.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the grammar must classify")]
fn sad_decimal_units_are_decimal_unit() {
    assert_class(
        &["128MB", "1kb", "1KB", "1Gb", "1tb", "1GB", "99999999999999999999999MB"],
        "DecimalUnit",
        |e| matches!(e, ByteSizeParseError::DecimalUnit { .. }),
    );
}

/// AC-1.3: a fractional count is `Fractional`, whatever follows the dot.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the grammar must classify")]
fn sad_fractional_counts_are_fractional() {
    assert_class(&["1.5MiB", "1.MiB", "1.", "1.5", "0.5GiB"], "Fractional", |e| {
        matches!(e, ByteSizeParseError::Fractional { .. })
    });
}

/// AC-1.3: everything that is not `<digits><unit>` is `Malformed`: empty, signs, padding,
/// a missing count, a unit that is not one of the five, separators and exponents inside the number.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the grammar must classify")]
fn sad_text_outside_the_grammar_is_malformed() {
    assert_class(
        &[
            "",
            " 128MiB",
            "128MiB ",
            "128MiB\n",
            "-1MiB",
            "+1MiB",
            ".5MiB",
            "MiB",
            "128Mi",
            "128M",
            "1PiB",
            "1ZB",
            "1e3MiB",
            "128 MiB",
            "1_000MiB",
            "128MiBx",
            "\u{663}MiB",
        ],
        "Malformed naming the grammar",
        |e| matches!(e, ByteSizeParseError::Malformed { .. }) && e.to_string().contains("MiB"),
    );
}

/// AC-1.3: a valid input whose value exceeds `u64::MAX` bytes is `Overflow`, at both the
/// unit boundary (`16777216TiB`) and the byte boundary; a very long digit run does not panic.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the grammar must classify")]
fn sad_a_value_above_u64_bytes_is_overflow() {
    let long = format!("{}MiB", "9".repeat(400));
    assert_class(
        &["16777216TiB", "18446744073709551616B", "17179869184GiB", long.as_str()],
        "Overflow",
        |e| matches!(e, ByteSizeParseError::Overflow { .. }),
    );
}

/// The classification order is pinned: a structural defect outranks overflow. Digits too large
/// for `u64` with no unit are `MissingUnit`; with a decimal unit, `DecimalUnit`; a fractional
/// count is `Fractional` even when huge. Overflow is reserved for otherwise-valid input.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the grammar must classify")]
fn sad_structure_outranks_overflow() {
    let huge = "9".repeat(60);
    let cases = [
        (huge.clone(), "MissingUnit"),
        (format!("{huge}MB"), "DecimalUnit"),
        (format!("{huge}.5MiB"), "Fractional"),
        (format!("{huge}Mi"), "Malformed"),
    ];
    for (input, class) in &cases {
        assert_class(&[input.as_str()], class, |e| {
            matches!(
                (class, e),
                (&"MissingUnit", ByteSizeParseError::MissingUnit { .. })
                    | (&"DecimalUnit", ByteSizeParseError::DecimalUnit { .. })
                    | (&"Fractional", ByteSizeParseError::Fractional { .. })
                    | (&"Malformed", ByteSizeParseError::Malformed { .. })
            )
        });
    }
}

// ----- error text names the grammar and the remedy ---------------------------------------------

fn error_text(input: &str) -> String {
    match parse(input) {
        Ok(v) => panic!("the error text of {input:?} cannot be read: it parsed as {v:?}"),
        Err(e) => e.to_string(),
    }
}

/// AC-1.3 / D-4: each rejection names the remedy DESIGN pins for its class (text outside the grammar
/// and a decimal unit name the binary units, a fractional count says to write the smaller unit,
/// overflow says the value is too large) and the five messages differ (`rust.md` — distinct failure
/// modes, distinct text). CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the error text must name")]
fn sad_every_rejection_names_its_remedy_and_the_five_messages_differ() {
    let malformed = error_text("128Mi");
    let missing = error_text("128");
    let decimal = error_text("128MB");
    for unit in ["KiB", "MiB", "GiB", "TiB"] {
        assert!(
            malformed.contains(unit),
            "the error text must name the unit {unit} for a malformed size: {malformed}"
        );
        assert!(
            decimal.contains(unit),
            "the error text must name the binary unit {unit} for a decimal unit: {decimal}"
        );
    }
    let fractional = error_text("1.5MiB").to_lowercase();
    assert!(
        fractional.contains("smaller"),
        "the error text must tell the operator to write the smaller unit: {fractional}"
    );
    let overflow = error_text("16777216TiB").to_lowercase();
    assert!(
        overflow.contains("too large"),
        "the error text must say the value is too large: {overflow}"
    );
    let all = [malformed, missing, decimal, fractional, overflow];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "the error text must differ per rejection class");
        }
    }
}

// ----- serde: the wire form is the canonical string --------------------------------------------

/// AC-1.3: a bare JSON number is a deserialization error whose text names the expected
/// grammar; so is a bool and null. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the expected string form")]
fn sad_serde_refuses_a_bare_number_and_names_the_expected_form() {
    for wire in ["134217728", "true", "null"] {
        let err = serde_json::from_str::<ByteSize>(wire)
            .expect_err("a non-string size must not deserialize")
            .to_string();
        assert!(
            err.contains("MiB"),
            "the error must name the expected string form (a count and a unit such as 128MiB) for {wire}: {err}"
        );
    }
}

// ----- properties ------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// AC-1.4: for every size, `parse(display(v)) == v`, and the canonical text matches the
    /// independent model (largest exact unit, zero is `0B`). CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "a byte size must display canonically")]
    fn display_is_canonical_and_round_trips(bytes in any::<u64>()) {
        let size = ByteSize::new(bytes);
        prop_assert_eq!(size.to_string(), model_canonical(bytes), "a byte size must display canonically");
        prop_assert_eq!(
            size.to_string().parse::<ByteSize>().expect("the displayed size must parse"),
            size
        );
    }

    /// Any whole count in any unit, in any letter case, parses to count x unit, equals the
    /// same size spelled in bytes, and renders the independent model's canonical text.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "a count in any unit and case must parse")]
    fn any_count_in_any_unit_and_case_parses_to_count_times_unit(
        count in 0u64..=1_000_000,
        unit in prop::sample::select(vec![("B", 1u64), ("KiB", KIB), ("MiB", MIB), ("GiB", GIB), ("TiB", TIB)]),
        upper in any::<bool>(),
    ) {
        let (name, mult) = unit;
        let spelled = if upper { format!("{count}{}", name.to_uppercase()) } else { format!("{count}{}", name.to_lowercase()) };
        let size = spelled.parse::<ByteSize>().map_err(|e| TestCaseError::fail(format!("a count in any unit and case must parse: {spelled}: {e}")))?;
        prop_assert_eq!(size.bytes(), count * mult);
        let in_bytes = format!("{}B", count * mult).parse::<ByteSize>().expect("the byte spelling parses");
        prop_assert_eq!(size, in_bytes);
        prop_assert_eq!(size.to_string(), model_canonical(count * mult));
    }

    /// Serde round trip: JSON is exactly the quoted canonical text and reads back to the same size.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "serde must emit the quoted canonical text")]
    fn serde_json_is_the_quoted_canonical_text_and_round_trips(bytes in any::<u64>()) {
        let size = ByteSize::new(bytes);
        let json = serde_json::to_string(&size).expect("serialises");
        prop_assert_eq!(&json, &format!("\"{}\"", model_canonical(bytes)), "serde must emit the quoted canonical text");
        prop_assert_eq!(serde_json::from_str::<ByteSize>(&json).expect("reads back"), size);
    }

    /// A digit run with no unit is always `MissingUnit`, however long: a bare number is never a size.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "a digit run without a unit must be MissingUnit")]
    fn a_digit_run_without_a_unit_is_always_missing_unit(digits in "[0-9]{1,40}") {
        prop_assert!(
            matches!(parse(&digits), Err(ByteSizeParseError::MissingUnit { .. })),
            "a digit run without a unit must be MissingUnit: {digits}"
        );
    }

    /// Any text that does not start with an ASCII digit is `Malformed`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "text without a leading digit must be Malformed")]
    fn text_without_a_leading_digit_is_always_malformed(text in "[^0-9][ -~]{0,12}") {
        let outcome = parse(&text);
        prop_assert!(
            matches!(&outcome, Err(e @ ByteSizeParseError::Malformed { .. }) if e.to_string().contains("MiB")),
            "text without a leading digit must be Malformed, naming the grammar: {text:?}: {outcome:?}"
        );
    }

    /// A count whose value exceeds `u64::MAX` bytes is `Overflow`; one that fits is accepted
    /// (checked against `u128` arithmetic). The count is drawn from a window of two either side of
    /// the largest count that fits in each unit, so every case sits on the boundary.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "the u64 boundary must hold")]
    fn the_u64_boundary_is_exact_in_every_unit(
        offset in 0u128..=4,
        unit in prop::sample::select(vec![("B", 1u128), ("KiB", 1u128 << 10), ("MiB", 1u128 << 20), ("GiB", 1u128 << 30), ("TiB", 1u128 << 40)]),
    ) {
        let (name, mult) = unit;
        let count = u128::from(u64::MAX) / mult - 2 + offset;
        let input = format!("{count}{name}");
        let total = count * mult;
        let parsed = parse(&input);
        if total > u128::from(u64::MAX) {
            prop_assert!(matches!(parsed, Err(ByteSizeParseError::Overflow { .. })), "the u64 boundary must hold: {input} overflows");
        } else {
            let size = parsed.map_err(|e| TestCaseError::fail(format!("the u64 boundary must hold: {input} fits: {e}")))?;
            prop_assert_eq!(u128::from(size.bytes()), total);
        }
    }
}

// ----- the volume size refines the same grammar (ADR-0310 corrected, DD-2) -------------------

/// DD-2: the volume size parses through the shared grammar and refines it to whole GiB, at least
/// 1 GiB: `1024MiB` is a 1 GiB volume, `512MiB`, `0GiB`, `1.5GiB` and `1GB` are refused.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the volume size must accept the pinned input")]
fn the_volume_size_is_the_shared_grammar_refined_to_whole_gib() {
    use overdrive_core::volume::VolumeSize;
    for (input, canonical) in [("40GiB", "40GiB"), ("2TiB", "2TiB"), ("1024MiB", "1GiB")] {
        let size = input.parse::<VolumeSize>().unwrap_or_else(|e| {
            panic!("the volume size must accept the pinned input {input:?}: {e}")
        });
        assert_eq!(size.to_string(), canonical);
    }
    for input in ["512MiB", "0GiB", "1.5GiB", "1GB"] {
        assert!(input.parse::<VolumeSize>().is_err(), "the volume size must refuse {input:?}");
    }
}
