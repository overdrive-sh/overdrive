//! `bench-report validate <record>`: the typed schema and its semantic rules (ADR-0301,
//! DDD-6): schema version 1, one JSON object per line, field `t` the line type, unknown
//! fields refused, every replicate naming a declared profile, arm and metric, a control
//! carrying the reference arm, `failure` non-null iff the replicate failed, the line order.
//!
//! Driving port: the `bench-report` binary. Evidence lane: integration (examples only,
//! Mandate 11: one named sad path per failure mode). Error taxonomy (DDD-6): success
//! prints nothing and exits 0; a violation prints
//! `bench-report: invalid record: line <n>: <reason>` to stderr and exits 65
//! (`Error::InvalidRecord`); a usage error exits 64; an unreadable record exits 70.
//! Never 101 (a panic). Every test is CONTRACT_SHAPE pure-function (a record in, a verdict and a
//! diagnostic out) except where it says otherwise. Traces: K3, K5.
use serde_json::json;

use super::support::*;

const INVALID: &str = "bench-report: invalid record: line ";

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn valid() -> Lines {
    two_arm(&tight(), &shifted(&tight(), 500.0), &tight())
}

/// Assert `validate` refused the record: exit 65, the pinned message prefix with line
/// `line`, and the reason naming `needle`.
fn refused(lines: &Lines, line: usize, needle: &str, why: &str) {
    let d = dir();
    let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", lines));
    assert!(!ok, "validate must refuse a record with {why}");
    assert_eq!(code, Some(65), "an invalid record exits 65, never a panic ({why}): {stderr}");
    assert!(
        stderr.contains(&format!("{INVALID}{line}: ")),
        "the refusal for {why} must read `{INVALID}{line}: <reason>`: {stderr}"
    );
    assert!(stderr.contains(needle), "the reason for {why} must name `{needle}`: {stderr}"); // an empty needle pins only the prefix and the line
}

/// A well-formed record validates silently in every terminal state and evidence class (one
/// schema, no fork); host settings with no source or unreadable are accepted; a series stored as a
/// histogram is accepted, at the 1,001-value boundary and with values below 1 (bucket 0). US-BENCH-04 AC (a); K3, K5.
/// CONTRACT_SHAPE: pure-function (record in, verdict out); GREEN guard against an over-strict validator.
#[test]
fn a_wellformed_record_validates_in_every_terminal_state_and_evidence_class() {
    let d = dir();
    let mut spike = valid();
    set(&mut spike, 0, "evidence", json!("spike"));
    let mut forms = valid();
    forms[0]["host"]["irq_time_accounting"] =
        json!({ "absent": "the image kernel configuration is not readable" });
    forms[0]["host"]["clocksource"] = json!({ "unreadable": "permission denied" });
    let mut histogram = vec![run_started(&["lifecycle"]), canary("start", 1000.0)];
    histogram[0]["declaration"] = declaration(&["lifecycle"], &[metric_decl("m", "ns", "t", true)]);
    let series: Vec<f64> = (1..=2000).map(f64::from).collect();
    histogram.push(replicate(0, 0, "lifecycle", "arm", json!({ "m": histogram_entry(&series) })));
    histogram.push(canary("end", 1000.0));
    histogram.push(finished(1, 0));
    let boundary: Vec<f64> = (1..=1001).map(f64::from).collect();
    let bucket_zero: Vec<f64> = (1..=2000).map(|i| f64::from(i) / 1000.0).collect();
    let mut refused_forms = refused_start(&["ref", "alt"]);
    refused_forms["host"]["governor"] = json!({ "absent": "no cpufreq directory" });
    for (name, lines) in [
        ("finished", valid()),
        ("spike", spike),
        ("host-setting-forms", forms),
        ("histogram-series", histogram),
        ("histogram-1001-values", one_arm_series_record(histogram_entry(&boundary))),
        ("histogram-bucket-zero", one_arm_series_record(histogram_entry(&bucket_zero))),
        (
            "refused-difference",
            vec![refused_start(&["ref", "alt"]), refusal("bpf_stats_enabled", "1", "0")],
        ),
        ("refused-absent", vec![refused_forms, refusal("governor", "absent", "schedutil")]),
        (
            "refused-reason",
            vec![
                refused_start(&["ref", "alt"]),
                refusal_reason("smt", "unreadable: permission denied"),
            ],
        ),
        (
            "refused-statistics",
            vec![
                refused_start(&["ref", "alt"]),
                refusal_reason("bpf_stats", "BPF_ENABLE_STATS failed"),
            ],
        ),
    ] {
        let output = validate_output(&write(d.path(), &format!("{name}.jsonl"), &lines));
        assert!(
            output.status.success(),
            "a well-formed record must validate ({name}): {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "validate prints nothing on success ({name})"
        );
    }
}

/// A line that is not JSON is refused naming its line number.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_malformed_line_is_refused_naming_its_line_number() {
    let d = dir();
    let mut body =
        valid().iter().take(2).map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n";
    body.push_str("{ this is not json\n");
    let (ok, stderr, code) = validate(&write_raw(d.path(), "r.jsonl", &body));
    assert!(
        !ok && code == Some(65) && stderr.contains(&format!("{INVALID}3: ")),
        "a malformed line must be refused naming line 3: status {code:?}: {stderr}"
    );
}

/// An unknown field is refused (the schema is closed), naming the field and its line: on a line,
/// and inside a metric entry (the retired `truncated` field is unknown).
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn an_unknown_field_is_refused_naming_the_field() {
    let mut lines = valid();
    set(&mut lines, 3, "surprise", json!(true));
    refused(&lines, 4, "surprise", "an unknown field on a replicate line");
    let mut entry = valid();
    entry[3]["metrics"]["m"]["truncated"] = json!(0);
    refused(&entry, 4, "truncated", "an unknown field inside a metric entry");
}

/// A schema version other than 1 is refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn an_unsupported_schema_version_is_refused() {
    let mut lines = valid();
    set(&mut lines, 0, "schema", json!(2));
    refused(&lines, 1, "schema", "schema version 2");
}

/// Provenance is mandatory: a `run_started` missing any field is refused, naming it (K3:
/// every retained record carries commit, dirty flag, methodology, substrate, host, statistics
/// flag, seed). No field is ever omitted to mean `null`.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_record_missing_a_provenance_field_is_refused() {
    for field in [
        "commit",
        "dirty",
        "methodology",
        "substrate",
        "host",
        "bpf_stats",
        "seed",
        "clock",
        "debug_assertions",
    ] {
        let mut lines = valid();
        lines[0].as_object_mut().expect("object").remove(field);
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65) && stderr.contains(field),
            "a record missing its provenance must be refused, naming `{field}`: status {code:?}: {stderr}"
        );
    }
}

/// `host` has exactly the nine pinned keys: a missing key and an extra key are both refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_host_object_without_exactly_the_pinned_keys_is_refused() {
    let mut missing = valid();
    missing[0]["host"].as_object_mut().expect("host").remove("irq_time_accounting");
    let mut extra = valid();
    extra[0]["host"]["turbo_mode"] = json!("on");
    for (lines, name) in [(missing, "irq_time_accounting"), (extra, "turbo_mode")] {
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65) && stderr.contains(name),
            "a host object without exactly the nine keys must be refused, naming `{name}`: status {code:?}: {stderr}"
        );
    }
}

/// Every `host` value is a string, `{ "absent": <string> }` or `{ "unreadable": <string> }`: a number, an
/// `absent` that is not a string and an object with another key are refused, naming the key.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_host_value_in_none_of_the_three_forms_is_refused() {
    for (value, what) in [
        (json!(5), "a number"),
        (json!({ "absent": 5 }), "an absent that is not a string"),
        (json!({ "gone": "x" }), "an unknown form"),
    ] {
        let mut lines = valid();
        lines[0]["host"]["smt"] = value;
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65) && stderr.contains("smt"),
            "a host value in none of the three forms must be refused ({what}), naming `smt`: status {code:?}: {stderr}"
        );
    }
}

/// An `ok` replicate's metric entry holds exactly one of non-empty `values`, a non-null `histogram`,
/// a non-empty `failures` or a non-null `absent`; a scalar metric's entry holds at most one value and
/// never a histogram. One case each. DDD-6 (the metric entry rule).
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_metric_entry_that_does_not_hold_exactly_one_thing_is_refused() {
    let two_things =
        json!({ "values": [1.0], "histogram": null, "failures": ["x"], "absent": null });
    let nothing = json!({ "values": [], "histogram": null, "failures": [], "absent": null });
    let two_values =
        json!({ "values": [1.0, 2.0], "histogram": null, "failures": [], "absent": null });
    let scalar_histogram = histogram_entry(&[1.0, 2.0]);
    for (entry, what) in [
        (two_things, "a value and a failure"),
        (nothing, "nothing at all"),
        (two_values, "two values of a scalar metric"),
        (scalar_histogram, "a histogram of a scalar metric"),
    ] {
        let mut lines = valid();
        set_metrics(&mut lines, 3, json!({ "m": entry }));
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65) && stderr.contains(&format!("{INVALID}4: ")),
            "a metric entry of the wrong shape must be refused at line 4 ({what}): status {code:?}: {stderr}"
        );
    }
}

/// A histogram's bucket counts must sum to its `count`, and `run_finished` must carry the
/// counts of the `replicate` lines. DDD-6 (semantic rules). Fault stimulus: a count one too high.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn counts_that_do_not_add_up_are_refused() {
    let series: Vec<f64> = (1..=2000).map(f64::from).collect();
    let mut entry = histogram_entry(&series);
    entry["histogram"]["count"] = json!(2001);
    let mut started = run_started(&["lifecycle"]);
    started["declaration"] = declaration(&["lifecycle"], &[metric_decl("m", "ns", "t", true)]);
    let histogram = vec![
        started,
        canary("start", 1000.0),
        replicate(0, 0, "lifecycle", "arm", json!({ "m": entry })),
        canary("end", 1000.0),
        finished(1, 0),
    ];
    refused(&histogram, 3, "count", "a histogram whose buckets do not sum to its count");
    let mut finished_wrong = valid();
    let last = finished_wrong.len() - 1;
    set(&mut finished_wrong, last, "replicates_ok", json!(26));
    refused(
        &finished_wrong,
        last + 1,
        "replicates_ok",
        "run_finished counts that differ from the replicate lines",
    );
}

/// A valid histogram of 2,000 values in two buckets: bucket 1 is `[1, 1.03125)` and bucket 351
/// (`e` 10, `s` 30) is `[1984, 2016)`; `min` 1 and `max` 2000 lie inside them. Each histogram rule
/// below breaks exactly one thing of this shape (DDD-6, "each histogram satisfies").
fn histogram_record(count: u64, min: f64, max: f64, buckets: serde_json::Value) -> Lines {
    one_arm_series_record(raw_histogram_entry(count, min, max, buckets))
}

/// Control: the shape the malformed cases start from validates.
/// CONTRACT_SHAPE: pure-function (record in, verdict out); GREEN guard against an over-strict validator.
#[test]
fn the_histogram_shape_the_malformed_cases_start_from_is_valid() {
    let d = dir();
    let lines = histogram_record(2000, 1.0, 2000.0, json!([[1, 1000], [351, 1000]]));
    let (ok, stderr, _) = validate(&write(d.path(), "r.jsonl", &lines));
    assert!(ok, "the base histogram must validate: {stderr}");
}

/// Every bucket index is at most 2048 (bucket 2049 would start at 2^64, past the clamp).
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_bucket_index_above_2048_is_refused() {
    // max is 2^64, inside bucket 2049's own range, so only the index rule is broken.
    let lines =
        histogram_record(2000, 1.0, 18_446_744_073_709_551_616.0, json!([[1, 1000], [2049, 1000]]));
    refused(&lines, 3, "buckets", "a histogram bucket index of 2049");
}

/// Bucket indices ascend and are distinct: a repeated index and a descending pair are both refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_whose_bucket_indices_do_not_ascend_or_repeat_is_refused() {
    let repeated = histogram_record(2000, 1.0, 2000.0, json!([[1, 500], [1, 500], [351, 1000]]));
    refused(&repeated, 3, "buckets", "a repeated histogram bucket index");
    let descending = histogram_record(2000, 1.0, 2000.0, json!([[351, 1000], [1, 1000]]));
    refused(&descending, 3, "buckets", "descending histogram bucket indices");
}

/// Every bucket `count` is at least 1: a stored empty bucket is refused (the buckets still sum to `count`).
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_bucket_holding_no_values_is_refused() {
    let lines = histogram_record(2000, 1.0, 2000.0, json!([[1, 1000], [100, 0], [351, 1000]]));
    refused(&lines, 3, "buckets", "a histogram bucket holding no values");
}

/// `histogram` is non-null only when `count` exceeds 1,000: a histogram of exactly 1,000 values (the
/// writer stores those raw) and a histogram of one value are refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_of_1000_or_fewer_values_is_refused() {
    let thousand = histogram_record(1000, 1.0, 2000.0, json!([[1, 500], [351, 500]]));
    refused(&thousand, 3, "histogram", "a histogram of 1000 values");
    let one = histogram_record(1, 1.0, 1.0, json!([[1, 1]]));
    refused(&one, 3, "histogram", "a histogram of one value");
}

/// `min` is at most `max`. Both lie in the one occupied bucket `[1984, 2016)`, so only this rule is broken.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_whose_min_exceeds_its_max_is_refused() {
    let lines = histogram_record(2000, 2010.0, 1990.0, json!([[351, 2000]]));
    refused(&lines, 3, "min", "a histogram whose min exceeds its max");
}

/// `min` lies within the range of the lowest occupied bucket: a `min` below the bucket's range and one
/// above it (bucket 1 is `[1, 1.03125)`; a value below 1 belongs to bucket 0) are both refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_min_outside_its_lowest_bucket_is_refused() {
    let below = histogram_record(2000, 0.5, 2000.0, json!([[1, 1000], [351, 1000]]));
    refused(&below, 3, "min", "a histogram min below its lowest bucket");
    let above = histogram_record(2000, 1.5, 2000.0, json!([[1, 1000], [351, 1000]]));
    refused(&above, 3, "min", "a histogram min above its lowest bucket");
}

/// `max` lies within the range of the highest occupied bucket: a `max` above the bucket's range and one
/// below it (bucket 351 is `[1984, 2016)`) are both refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_histogram_max_outside_its_highest_bucket_is_refused() {
    let above = histogram_record(2000, 1.0, 3000.0, json!([[1, 1000], [351, 1000]]));
    refused(&above, 3, "max", "a histogram max above its highest bucket");
    let below = histogram_record(2000, 1.0, 1000.0, json!([[1, 1000], [351, 1000]]));
    refused(&below, 3, "max", "a histogram max below its highest bucket");
}

/// A replicate naming an arm, a profile or a metric the declaration does not declare is
/// refused: the semantic rules, one case each.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_replicate_naming_an_undeclared_arm_profile_or_metric_is_refused() {
    let mut arm = valid();
    set(&mut arm, 2, "arm", json!("ghost-arm"));
    let mut profile = valid();
    set(&mut profile, 2, "profile", json!("ghost-profile"));
    let mut metric = valid();
    set_metrics(&mut metric, 2, json!({ "ghost_metric": values_entry(&[1.0]) }));
    for (lines, name) in [(arm, "ghost-arm"), (profile, "ghost-profile"), (metric, "ghost_metric")]
    {
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65)
                && stderr.contains(&format!("{INVALID}3: "))
                && stderr.contains(name),
            "a replicate naming an undeclared name must be refused at line 3, naming `{name}`: status {code:?}: {stderr}"
        );
    }
}

/// A control must carry the reference arm (the first declared) in `arm`.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_control_that_does_not_carry_the_reference_arm_is_refused() {
    let mut lines = valid();
    set(&mut lines, 4, "arm", json!("alt")); // block 0, position 2: the control, naming the non-reference arm
    refused(&lines, 5, "arm", "a control that names the wrong arm");
}

/// `failure` is non-null iff the replicate failed: a failed replicate without a reason and an
/// ok replicate with one are both refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_failure_reason_is_present_exactly_when_the_replicate_failed() {
    let mut no_reason = valid();
    set(&mut no_reason, 3, "outcome", json!("failed"));
    refused(&no_reason, 4, "failure", "a failed replicate that has no reason");
    let mut spurious = valid();
    set(&mut spurious, 3, "failure", json!("left over"));
    refused(&spurious, 4, "failure", "an ok replicate that carries a failure reason");
}

/// Nothing may follow the terminal line (`run_finished` or `run_refused`): the line order
/// is part of the schema.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_line_after_the_terminal_line_is_refused() {
    let mut lines = valid();
    let last = lines.len();
    lines.push(canary("end", 1000.0));
    refused(&lines, last + 1, "", "a line after its terminal line");
}

/// A record whose first line is not `run_started`, and an empty record, are refused.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_record_that_does_not_begin_with_run_started_is_refused() {
    let d = dir();
    let lines = valid();
    let headless: Lines = lines[1..].to_vec();
    let (ok, stderr, code) = validate(&write(d.path(), "headless.jsonl", &headless));
    assert!(
        !ok && code == Some(65) && stderr.contains(&format!("{INVALID}1: ")),
        "a record must begin with run_started: status {code:?}: {stderr}"
    );
    let (ok, stderr, code) = validate(&write_raw(d.path(), "empty.jsonl", ""));
    assert!(!ok && code != Some(101), "an empty record must be refused: {stderr}");
}

/// A value outside a closed vocabulary is refused: `role`, `outcome`, `evidence`, `bpf_stats`, and the
/// substrate `guest` (the registry is `native` only, DDD-4).
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_value_outside_a_closed_vocabulary_is_refused() {
    for (field, value) in [
        ("role", "referee"),
        ("outcome", "skipped"),
        ("evidence", "folklore"),
        ("bpf_stats", "sometimes"),
        ("substrate", "guest"),
    ] {
        let mut lines = valid();
        let index = usize::from(field == "role" || field == "outcome") * 2;
        set(&mut lines, index, field, json!(value));
        let d = dir();
        let (ok, stderr, code) = validate(&write(d.path(), "r.jsonl", &lines));
        assert!(
            !ok && code == Some(65),
            "a closed vocabulary must refuse `{field}` = `{value}`: status {code:?}: {stderr}"
        );
    }
}

/// The tool's own errors are typed: no subcommand, an unknown subcommand and a missing
/// argument are usage errors (exit 64); a record that cannot be read exits 70. Never a panic.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn the_tools_usage_errors_are_typed_refusals() {
    let d = dir();
    let missing = d.path().join("no-such-record.jsonl");
    for args in [vec![], vec!["frobnicate"], vec!["validate"], vec!["render"]] {
        let output = bench_report(&args);
        assert_eq!(output.status.code(), Some(64), "a usage error must exit 64 (args {args:?})");
    }
    for subcommand in ["validate", "render"] {
        let output = bench_report(&[subcommand, missing.to_str().expect("utf-8")]);
        assert_eq!(
            output.status.code(),
            Some(70),
            "an unreadable record must exit 70 ({subcommand})"
        );
    }
}

/// The pinned `clock.service` string (DDD-2 "Clock sources", `ServiceClock::InjectedMonotonic`).
const SERVICE_CLOCK: &str = "Clock::now (monotonic)";
/// The pinned `clock.guest` string (`GuestClock::InitElapsedMs`).
const GUEST_CLOCK: &str = "CLOCK_MONOTONIC (overdrive-init Instant::elapsed, 1 ms)";

fn with_clock(service: &serde_json::Value, guest: &serde_json::Value) -> Lines {
    let mut lines = valid();
    set(
        &mut lines,
        0,
        "clock",
        json!({ "host": "CLOCK_MONOTONIC_RAW", "service": service, "guest": guest }),
    );
    lines
}

/// `clock.service` is `null` or the one pinned string; any other string, including the pinned
/// guest string, is refused naming `clock.service` (exit 65).
/// Pending until the closed vocabulary lands: today any string validates.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_clock_service_string_outside_the_pinned_one_is_refused() {
    for value in ["monotonic", "Clock::now (wall)", "", GUEST_CLOCK] {
        refused(
            &with_clock(&json!(value), &json!(null)),
            1,
            "clock.service",
            &format!("clock.service `{value}`"),
        );
    }
}

/// `clock.guest` is `null` or the one pinned string; any other string, including the pinned
/// service string, is refused naming `clock.guest` (exit 65).
/// Pending until the closed vocabulary lands: today any string validates.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_clock_guest_string_outside_the_pinned_one_is_refused() {
    for value in ["tsc", "CLOCK_MONOTONIC", "", SERVICE_CLOCK] {
        refused(
            &with_clock(&json!(null), &json!(value)),
            1,
            "clock.guest",
            &format!("clock.guest `{value}`"),
        );
    }
}

/// A record carrying each pinned clock string, alone and together, validates silently, and so does
/// one carrying neither. GREEN guard against an over-strict closed vocabulary.
/// CONTRACT_SHAPE: pure-function (record in, verdict out).
#[test]
fn a_record_carrying_each_pinned_clock_string_validates() {
    for (service, guest) in [
        (json!(SERVICE_CLOCK), json!(null)),
        (json!(null), json!(GUEST_CLOCK)),
        (json!(SERVICE_CLOCK), json!(GUEST_CLOCK)),
        (json!(null), json!(null)),
    ] {
        let d = dir();
        let (ok, stderr, code) =
            validate(&write(d.path(), "r.jsonl", &with_clock(&service, &guest)));
        assert!(
            ok && code == Some(0) && stderr.is_empty(),
            "a record with clock.service {service} and clock.guest {guest} must validate: status {code:?}: {stderr}"
        );
    }
}
