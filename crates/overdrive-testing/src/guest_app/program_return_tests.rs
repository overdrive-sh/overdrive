//! The guest benchmark program's console return (guest-benchmark-substrate, DDD-6, ADR-0330):
//! the readings and the framed series block, and the host half's strict parser of them.
//!
//! Driving port: [`parse_program_return`], the host half's parse of a replicate's console, with
//! [`print_program_return`], the printer the guest program uses, beside it: one owner holds both
//! halves of the grammar, so every console below is printed by the real printer and read by the
//! real parser (the round trip pins the grammar). The checksum definition is pinned independently
//! by two fixed vectors computed outside both (GA3).
//!
//! The pinned grammar, after the program's timed phase: one `\n`, then
//! `guest-reading: kernel=<v>` and `guest-reading: clocksource=<v>` (a value is text,
//! `absent:<reason>` or `unreadable:<error>`), then `bench-series-begin: <metric>`, lines
//! `bench-values: <metric> <v1>,<v2>,...` of at most 16 decimal values, and
//! `bench-series-end: <metric> <sha256 hex of the values as little-endian u64 bytes>`. A reading
//! line and a begin line match only at the start of a console line.
//!
//! Layers 1-2: PBT full for the input space (Mandate 9); the sad paths are also named examples.
//! CONTRACT_SHAPE: pure-function (console text in, return or error out). Each pending test is
//! `#[should_panic]` on the failure of its own assertion, so it proves RED against the scaffold
//! parser, which finds no block.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::format_collect,
    clippy::format_push_string,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    reason = "the scenarios assert their own preconditions and name protocol terms"
)]

use proptest::prelude::*;
use sha2::{Digest, Sha256};

use super::{
    GuestReading, ProgramReturn, ProgramReturnError, parse_program_return, print_program_return,
};

const METRIC: &str = "forwarded_round_trip_ns";

/// The pinned checksum, written here only to prove the vectors of GA3 independently of the
/// printer: SHA-256, lowercase hex, of the values as consecutive little-endian `u64`.
fn checksum(values: &[u64]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.to_le_bytes());
    }
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn text(value: &str) -> GuestReading {
    GuestReading::Text(value.to_owned())
}

fn intact(values: &[u64]) -> String {
    print_program_return(&text("7.0.0-38-generic"), &text("kvm-clock"), METRIC, values)
}

/// `altered` with its end line replaced by the one `original` printed: the damage a block can
/// suffer in transit, values changed under the checksum of the values that were printed.
fn with_end_line_of(altered: &str, original: &str) -> String {
    let end =
        original.lines().find(|line| line.starts_with("bench-series-end:")).unwrap_or_default();
    altered
        .lines()
        .map(|line| if line.starts_with("bench-series-end:") { end } else { line })
        .map(|line| format!("{line}\n"))
        .collect()
}

/// What a guest console holds around the program's output: kernel lines, and an
/// `overdrive-init` log fragment left unterminated when the program writes its first byte.
fn console_around(printed: &str) -> String {
    format!(
        "[    0.000000] Linux version 7.0.0-38-generic\n[    1.104212] overdrive-init: READY sent\nexec peer\noverdrive-init: stdio forwarded{printed}[    9.9] reboot: Power down\n"
    )
}

fn values_of(count: usize) -> Vec<u64> {
    (0..count as u64).map(|index| 41_000 + index * 7).collect()
}

/// An intact console parses to its two readings and its values in measurement order, whatever
/// kernel lines surround it, and the unterminated `overdrive-init` fragment before the program's
/// leading `\n` is ended by it.
#[test]
fn an_intact_console_parses_to_its_readings_and_values_in_order() {
    let values = values_of(5);
    let parsed = parse_program_return(&console_around(&intact(&values)), METRIC, 5)
        .expect("an intact console must parse");
    assert_eq!(
        parsed,
        ProgramReturn { kernel: text("7.0.0-38-generic"), clocksource: text("kvm-clock"), values }
    );
}

/// A reading the guest could not provide is `absent:<reason>` or `unreadable:<error>` and parses
/// to the matching form, never to text and never to a default.
#[test]
fn a_reading_the_guest_could_not_provide_parses_as_absent_or_unreadable() {
    let values = values_of(3);
    let printed = print_program_return(
        &GuestReading::Absent("no clocksource file".to_owned()),
        &GuestReading::Unreadable("permission denied".to_owned()),
        METRIC,
        &values,
    );
    let parsed = parse_program_return(&console_around(&printed), METRIC, 3)
        .expect("a console with absent and unreadable readings must parse");
    assert_eq!(parsed.kernel, GuestReading::Absent("no clocksource file".to_owned()));
    assert_eq!(parsed.clocksource, GuestReading::Unreadable("permission denied".to_owned()));
}

/// The checksum is SHA-256 of the values as consecutive little-endian `u64` bytes, in lowercase
/// hex: two fixed vectors computed outside the parser (`[1, 2, 3]` and `[42, u64::MAX]`).
#[test]
fn the_checksum_is_the_sha256_of_the_values_as_little_endian_bytes() {
    for (values, digest) in [
        (vec![1_u64, 2, 3], "e2e2033ae7e19d680599d4eb0a1359a2b48ec5baac75066c317fbf85159c54ef"),
        (
            vec![42_u64, u64::MAX],
            "bd30c5e80aba4350aba2734fcefe7b2c4646e0f737338761263dcc138070a6e5",
        ),
    ] {
        assert_eq!(checksum(&values), digest, "the reference checksum is the pinned definition");
        let line: Vec<String> = values.iter().map(u64::to_string).collect();
        let console = format!(
            "\nguest-reading: kernel=k\nguest-reading: clocksource=c\nbench-series-begin: {METRIC}\nbench-values: {METRIC} {}\nbench-series-end: {METRIC} {digest}\n",
            line.join(",")
        );
        let parsed = parse_program_return(&console, METRIC, values.len())
            .expect("a block with the pinned checksum must parse");
        assert_eq!(parsed.values, values);
    }
}

/// Boundaries of the value-line grouping (at most 16 per line): 1, 15, 16, 17, 32, 33 and the
/// family's 5,000 values all parse to exactly what was printed.
#[test]
fn a_block_parses_at_every_value_line_boundary() {
    for count in [1_usize, 2, 15, 16, 17, 31, 32, 33, 5_000] {
        let values = values_of(count);
        let parsed = parse_program_return(&console_around(&intact(&values)), METRIC, count)
            .unwrap_or_else(|error| {
                panic!("a block of every size must parse ({count}): {error:?}")
            });
        assert_eq!(parsed.values, values, "{count} values");
    }
}

/// A return that starts on its own line after an unterminated `overdrive-init` fragment parses
/// (the program's leading `\n` ended the fragment); the same return glued to the fragment, without
/// that `\n`, finds no first reading line and fails: the strict start-of-line rule, V-B2's finding.
#[test]
fn a_return_after_an_unterminated_fragment_parses_only_with_the_leading_newline() {
    let values = values_of(8);
    let printed = intact(&values);
    let fragment = "overdrive-init: forwarding 3 flows";
    let with_newline = format!("{fragment}{printed}");
    assert_eq!(
        parse_program_return(&with_newline, METRIC, 8)
            .expect("a return after a fragment and its newline must parse")
            .values,
        values
    );
    let glued = format!("{fragment}{}", printed.trim_start_matches('\n'));
    assert_eq!(
        parse_program_return(&glued, METRIC, 8),
        Err(ProgramReturnError::ReadingMissing { key: "kernel".to_owned() }),
        "the first reading line glued to a fragment is not at the start of a line"
    );
}

/// A begin line glued to foreign text is not a begin line: the block is not found (or is
/// malformed) and the return fails; it is never parsed from the middle of a line.
#[test]
fn a_begin_line_glued_to_foreign_text_is_refused() {
    let values = values_of(8);
    parse_program_return(&intact(&values), METRIC, 8).expect("the unglued control must parse");
    let glued = intact(&values).replace(
        &format!("bench-series-begin: {METRIC}"),
        &format!("[ 5.1] noise bench-series-begin: {METRIC}"),
    );
    assert!(parse_program_return(&glued, METRIC, 8).is_err(), "a glued begin line must be refused");
}

/// A console with no block for the metric is `BlockMissing`: an empty console, readings only, and
/// a block of another metric.
#[test]
fn a_console_with_no_block_for_the_metric_is_refused_as_missing() {
    parse_program_return(&intact(&values_of(4)), METRIC, 4)
        .expect("the control console must parse");
    let other = intact(&values_of(4)).replace(METRIC, "another_metric_ns");
    for console in [
        String::new(),
        "\nguest-reading: kernel=k\nguest-reading: clocksource=c\n".to_owned(),
        other,
    ] {
        assert_eq!(
            parse_program_return(&console, METRIC, 4),
            Err(ProgramReturnError::BlockMissing),
            "no block for the metric must be BlockMissing"
        );
    }
}

/// Two blocks of the metric are `BlockRepeated`: the console never yields either one.
#[test]
fn a_block_printed_twice_is_refused_as_repeated() {
    let values = values_of(20);
    let printed = intact(&values);
    let block = printed.find("bench-series-begin").map_or("", |at| &printed[at..]);
    let twice = format!("{printed}{block}");
    assert_eq!(
        parse_program_return(&twice, METRIC, 20),
        Err(ProgramReturnError::BlockRepeated),
        "two blocks must be BlockRepeated"
    );
}

/// A reading that is missing is `ReadingMissing` naming its key, and one printed twice is
/// `ReadingRepeated` naming its key; each of the two keys is checked.
#[test]
fn a_reading_that_is_missing_or_repeated_is_refused_naming_its_key() {
    let printed = intact(&values_of(6));
    for key in ["kernel", "clocksource"] {
        let line = printed
            .lines()
            .find(|line| line.starts_with(&format!("guest-reading: {key}=")))
            .unwrap_or_default();
        let missing = printed.replace(&format!("{line}\n"), "");
        assert_eq!(
            parse_program_return(&missing, METRIC, 6),
            Err(ProgramReturnError::ReadingMissing { key: key.to_owned() }),
            "a missing reading must name its key ({key})"
        );
        let repeated = printed.replace(&format!("{line}\n"), &format!("{line}\n{line}\n"));
        assert_eq!(
            parse_program_return(&repeated, METRIC, 6),
            Err(ProgramReturnError::ReadingRepeated { key: key.to_owned() }),
            "a repeated reading must name its key ({key})"
        );
    }
}

/// A reading line with no `=` and so no value is not a reading: the return is refused, never read
/// with an empty value or a default (the variant, missing or malformed, is the crafter's).
#[test]
fn a_reading_line_without_a_value_is_refused() {
    let printed = intact(&values_of(6));
    parse_program_return(&printed, METRIC, 6).expect("the control console must parse");
    let broken =
        printed.replacen("guest-reading: kernel=7.0.0-38-generic", "guest-reading: kernel", 1);
    let parsed = parse_program_return(&broken, METRIC, 6);
    assert!(parsed.is_err(), "a reading line with no `=` must be refused: {parsed:?}");
}

/// A block that is cut short, or whose value lines are not decimal `u64` values, is refused and
/// never read as a shorter series: no end line, an unterminated final value line, a non-digit, a
/// negative number, a value above `u64::MAX`, an empty value list.
#[test]
fn a_cut_or_malformed_block_is_refused_and_never_read_as_a_shorter_series() {
    let printed = intact(&values_of(20));
    parse_program_return(&printed, METRIC, 20).expect("the control console must parse");
    let without_end = printed[..printed.find("bench-series-end").expect("end")].to_owned();
    let cases = [
        ("no end line", without_end.clone()),
        ("cut mid-line", without_end[..without_end.len() - 9].to_owned()),
        ("a non-digit", printed.replacen("41007", "41o07", 1)),
        ("a negative value", printed.replacen("41007", "-41007", 1)),
        ("above u64::MAX", printed.replacen("41007", "18446744073709551616", 1)),
        (
            "a leading empty value",
            printed.replacen(
                "bench-values: forwarded_round_trip_ns 41000",
                "bench-values: forwarded_round_trip_ns ",
                1,
            ),
        ),
    ];
    for (why, console) in cases {
        let parsed = parse_program_return(&console, METRIC, 20);
        assert!(parsed.is_err(), "a malformed block must be refused ({why}): {parsed:?}");
    }
}

/// A value line or the end line with foreign text spliced into it (a kernel message or an
/// `overdrive-init` fragment arriving mid-line) breaks the count or the checksum, so the replicate
/// fails; it is never a short series or an altered one.
#[test]
fn foreign_text_spliced_into_a_block_line_is_refused_never_shortened() {
    let printed = intact(&values_of(20));
    parse_program_return(&printed, METRIC, 20).expect("the control console must parse");
    for (why, console) in [
        ("into a value line", printed.replacen("41014,", "41014,[ 12.3] kernel: noise\n41021,", 1)),
        ("into the middle of a value", printed.replacen("41035", "410\n[ 7.0] noise\n35", 1)),
        (
            "into the end line",
            printed.replacen("bench-series-end: ", "bench-series-end: [ 1.0] noise ", 1),
        ),
    ] {
        let parsed = parse_program_return(&console, METRIC, 20);
        assert!(parsed.is_err(), "a spliced block must be refused ({why}): {parsed:?}");
    }
}

/// A block holding a different number of values from the one asked for is `CountMismatch` with
/// both numbers, and its `Display` names them; the boundary 4,999 and 5,001 against 5,000.
#[test]
fn a_block_of_a_different_count_is_refused_naming_both_counts() {
    for found in [4_999_usize, 5_001, 1, 2] {
        let printed = intact(&values_of(found));
        let error = parse_program_return(&printed, METRIC, 5_000)
            .expect_err("a short or long block must be a count mismatch");
        assert_eq!(
            error,
            ProgramReturnError::CountMismatch { expected: 5_000, found },
            "a short or long block must be a count mismatch"
        );
        let shown = error.to_string();
        assert!(
            shown.contains("5000") || shown.contains("5,000"),
            "the count error names the expected count: {shown}"
        );
        assert!(
            shown.contains(&found.to_string()),
            "the count error names the found count: {shown}"
        );
    }
}

/// A block whose values changed but whose count is right is `ChecksumMismatch`.
#[test]
fn a_block_whose_values_do_not_match_its_checksum_is_refused() {
    let printed = intact(&values_of(20));
    let altered = printed.replacen("41007", "41008", 1);
    assert_eq!(
        parse_program_return(&altered, METRIC, 20),
        Err(ProgramReturnError::ChecksumMismatch),
        "a changed value must be a checksum mismatch"
    );
}

/// Every error names its cause in its `Display`: the key of a reading, the two counts of a
/// mismatch and a distinct text for each block and checksum variant.
#[test]
fn every_error_names_its_cause() {
    for error in [
        ProgramReturnError::ReadingMissing { key: "kernel".to_owned() },
        ProgramReturnError::ReadingRepeated { key: "kernel".to_owned() },
        ProgramReturnError::ReadingMalformed { key: "kernel".to_owned() },
    ] {
        assert!(
            error.to_string().contains("kernel"),
            "ReadingMissing must name its key: {error:?}"
        );
    }
    let texts: Vec<String> = [
        ProgramReturnError::BlockMissing,
        ProgramReturnError::BlockRepeated,
        ProgramReturnError::BlockMalformed,
        ProgramReturnError::ChecksumMismatch,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    for (at, shown) in texts.iter().enumerate() {
        assert!(!shown.is_empty());
        assert!(
            texts.iter().enumerate().all(|(other, text)| other == at || text != shown),
            "{texts:?}"
        );
    }
}

fn kernel_text() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._-]{0,23}".prop_filter("not a reading form", |text| {
        !text.starts_with("absent") && !text.starts_with("unreadable")
    })
}

fn noise_line() -> impl Strategy<Value = String> {
    "[ -~]{0,60}".prop_filter("not a grammar line", |line| {
        !line.starts_with("guest-reading:") && !line.starts_with("bench-")
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    /// For every non-empty series of `u64` values, every kernel and clocksource text, any foreign
    /// lines before and after and any unterminated fragment before the program's `\n`, what the
    /// printer printed is what the parser returns.
    #[test]
    fn a_printed_return_parses_back_to_exactly_what_was_printed(
        values in proptest::collection::vec(any::<u64>(), 1..300),
        kernel in kernel_text(),
        clocksource in kernel_text(),
        before in proptest::collection::vec(noise_line(), 0..4),
        after in proptest::collection::vec(noise_line(), 0..4),
        fragment in "[ -~]{0,40}",
    ) {
        let printed = print_program_return(&text(&kernel), &text(&clocksource), METRIC, &values);
        let console = format!("{}\n{fragment}{printed}{}\n", before.join("\n"), after.join("\n"));
        let parsed = parse_program_return(&console, METRIC, values.len());
        prop_assert!(
            parsed == Ok(ProgramReturn { kernel: text(&kernel), clocksource: text(&clocksource), values }),
            "what was printed must parse back: {parsed:?}"
        );
    }

    /// For every series, changing one value is a checksum mismatch, dropping one is a count
    /// mismatch (or a checksum mismatch), and adding one likewise: the parser never returns a
    /// series other than the one the checksum covers.
    #[test]
    fn a_damaged_block_never_parses_to_a_different_series(
        values in proptest::collection::vec(1_u64..u64::MAX - 1, 2..200),
        at in any::<proptest::sample::Index>(),
    ) {
        let n = values.len();
        let i = at.index(n);
        let mut changed = values.clone();
        changed[i] += 1;
        // Reprint with the altered values but the ORIGINAL end line (the checksum of `values`).
        let original = print_program_return(&text("k"), &text("c"), METRIC, &values);
        let damaged = |altered: &[u64]| {
            with_end_line_of(&print_program_return(&text("k"), &text("c"), METRIC, altered), &original)
        };
        let flipped = parse_program_return(&damaged(&changed), METRIC, n);
        prop_assert!(
            flipped == Err(ProgramReturnError::ChecksumMismatch),
            "a damaged block must never parse to a different series (flip): {flipped:?}"
        );
        let mut shorter = values.clone();
        shorter.remove(i);
        let dropped = parse_program_return(&damaged(&shorter), METRIC, n);
        prop_assert!(
            matches!(dropped, Err(ProgramReturnError::CountMismatch { expected, found }) if expected == n && found == n - 1)
                || dropped == Err(ProgramReturnError::ChecksumMismatch),
            "a damaged block must never parse to a different series (drop): {dropped:?}"
        );
        let mut longer = values;
        longer.insert(i, 7);
        let added = parse_program_return(&damaged(&longer), METRIC, n);
        prop_assert!(
            matches!(added, Err(ProgramReturnError::CountMismatch { expected, found }) if expected == n && found == n + 1)
                || added == Err(ProgramReturnError::ChecksumMismatch),
            "a damaged block must never parse to a different series (add): {added:?}"
        );
    }
}
