//! `bench-report render <record>`: the derived report (ADR-0301, ADR-0300, DDD-6 "Report").
//!
//! Driving port: the `bench-report` binary (DDD-1 driving port 6), host-independent.
//! Production owner path: the report renderer, a pure function of the record.
//! Evidence lane: integration (subprocess; examples only, Mandate 9). Each scenario
//! builds a record from `support` and reads the rendered report; nothing is measured.
//! The wording asserted is the wording DDD-6 pins: header bullets `- <key>: <value>`,
//! `## Profile: <name>`, `### Table: <label>`, the scalar, series and comparison table
//! columns, the verdict cells and the `## Failures` lines. Traces: US-BENCH-01/02/03/04/06/09, K3, K5.
use serde_json::json;

use super::support::*;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn report_of(lines: &Lines) -> String {
    let d = dir();
    render(&write(d.path(), "r.jsonl", lines))
}

fn number(cell: &str, what: &str) -> f64 {
    cell.parse().unwrap_or_else(|_| panic!("{what} must be a number, found `{cell}`"))
}

/// Per arm the report shows the median, the IQR (R-7 quartiles), the replicate count and the
/// failure count in the scalar table. US-BENCH-01 AC (a).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn the_report_shows_median_iqr_and_replicate_count_per_arm() {
    let reference: Vec<f64> = (1..=9).map(|i| f64::from(i) * 1000.0).collect();
    let report = report_of(&two_arm(&reference, &shifted(&reference, 500.0), &reference));
    let table = section(&report, "Table: t");
    assert!(
        table.contains("| metric | arm | median | IQR | replicates | failures |"),
        "the scalar table carries the pinned columns: {table}"
    );
    assert_eq!(
        &scalar_row(table, "m", "ref")[2..],
        ["5000", "4000", "9", "0"],
        "reference arm: median, IQR, replicates, failures"
    );
    assert_eq!(
        &scalar_row(table, "m", "alt")[2..],
        ["5500", "4000", "9", "0"],
        "alternative arm: median, IQR, replicates, failures"
    );
    assert_eq!(
        table_rows(table).iter().filter(|c| c.len() == 6 && c[0] == "m").count(),
        2,
        "metric `m` has one row per arm (`ref`, `alt`) and none for the A/A control: {table}"
    );
}

/// The header is a bullet list carrying the provenance every record promises (K3): run id,
/// family, benchmark, evidence, substrate, commit with its dirty flag, methodology, seed,
/// build flag, every host key, the statistics flag, the clock sources, the canary verdict,
/// the replicate counts and the status.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn the_report_header_carries_the_run_provenance_and_host_state() {
    let report = report_of(&two_arm(&tight(), &shifted(&tight(), 500.0), &tight()));
    assert!(
        report.starts_with("# Benchmark report: cgroup-connect-hook / connect_hook_cost"),
        "the report must start with the pinned title: {report}"
    );
    for (key, value) in [
        ("run id", RUN_ID),
        ("family", "cgroup-connect-hook"),
        ("benchmark", "connect_hook_cost"),
        ("evidence", "maintained"),
        ("substrate", "native"),
        ("commit", COMMIT),
        ("dirty", "false"),
        ("methodology", "1"),
        ("seed", "424242"),
        ("debug_assertions", "false"),
        ("kernel", "7.0.0-38-generic"),
        ("cpu_model", "AMD EPYC 8024P"),
        ("cpu_count", "8"),
        ("governor", "schedutil"),
        ("smt", "on"),
        ("boost", "on"),
        ("bpf_stats_enabled", "0"),
        ("clocksource", "tsc"),
        ("irq_time_accounting", "y"),
        ("bpf stats", "on (per process)"),
        ("canary", "steady"),
        ("replicates ok", "27"),
        ("replicates failed", "0"),
        ("status", "finished"),
    ] {
        assert_eq!(header(&report, key), value, "header line `{key}`");
    }
    assert!(
        header(&report, "clock").contains("CLOCK_MONOTONIC_RAW"),
        "the clock line names the host clock source: {report}"
    );
}

/// A host setting with no source prints its reason, and one that could not be read prints its
/// error: never a blank or a value. DDD-6: `host` values are a string, `{ "absent": r }` or
/// `{ "unreadable": e }`.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn an_absent_or_unreadable_host_setting_prints_its_reason_in_the_header() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines[0]["host"]["irq_time_accounting"] =
        json!({ "absent": "the image kernel configuration is not readable" });
    lines[0]["host"]["clocksource"] = json!({ "unreadable": "permission denied" });
    let report = report_of(&lines);
    assert_eq!(
        header(&report, "irq_time_accounting"),
        "absent: the image kernel configuration is not readable",
        "an absent host setting prints `absent: <reason>`"
    );
    assert_eq!(
        header(&report, "clocksource"),
        "unreadable: permission denied",
        "an unreadable host setting prints `unreadable: <error>`"
    );
}

/// A run that needed no statistics says so.
/// US-BENCH-03 AC (e).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_run_without_statistics_reports_them_off() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    set(&mut lines, 0, "bpf_stats", json!("off"));
    let report = report_of(&lines);
    assert_eq!(header(&report, "bpf stats"), "off", "a run without statistics reports them off");
}

/// A clear difference is reported with its delta, its interval, the A/A interval, the
/// nonparametric result and the verdict `distinguishable`.
/// US-BENCH-02 AC (b). Contract: DDD-5 verdict rule, DDD-6 comparison columns.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_clear_difference_is_reported_with_its_intervals_and_a_distinguishable_verdict() {
    let reference = tight();
    let control: Vec<f64> = reference.iter().rev().copied().collect();
    let report = report_of(&two_arm(&reference, &shifted(&reference, 500.0), &control));
    let table = section(&report, "Table: t");
    assert!(
        table.contains(
            "| metric | arm | delta (alt minus ref) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |"
        ),
        "the comparison table carries the pinned columns: {table}"
    );
    let row = comparison_row(table, "m", "alt");
    assert_eq!(
        row[2], "500",
        "the delta is the alternative arm's median minus the reference arm's"
    );
    assert!(
        row[3].starts_with('[') && row[3].ends_with(']'),
        "the delta interval prints as [lo, hi]: {row:?}"
    );
    assert!(
        row[4].starts_with('[') && row[4].ends_with(']'),
        "the A/A interval prints as [lo, hi]: {row:?}"
    );
    assert_eq!(row[7], "distinguishable", "a clear difference is distinguishable");
}

/// A difference inside the noise floor is labelled as noise, never as an effect.
/// US-BENCH-02 AC (c). Fault stimulus: a +1 ns delta over a 9 ns spread.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_difference_inside_the_noise_floor_is_labelled_not_distinguishable_from_noise() {
    let reference = tight();
    let control: Vec<f64> = reference.iter().rev().copied().collect();
    let report = report_of(&two_arm(&reference, &shifted(&reference, 1.0), &control));
    let row = comparison_row(section(&report, "Table: t"), "m", "alt");
    assert_eq!(
        row[7], "not distinguishable from noise",
        "a difference inside the noise floor is labelled as noise: {row:?}"
    );
}

/// An arm, or the A/A control, with one to eight `ok` replicates holding a value keeps its
/// comparison row with the verdict `insufficient replicates` rather than being compared.
/// US-BENCH-02 AC (d). Boundary: six, eight (insufficient); nine is compared (the clear-difference
/// scenario). Fault stimulus: three, then one, failed replicates of the alternative arm; one failed
/// control replicate.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn an_arm_with_fewer_than_nine_successes_is_reported_as_insufficient_replicates() {
    // Lines: run_started, canary, then three per block: reference, alternative, control.
    for (what, failed_indices, alt_ok) in [
        ("three failed alternative replicates", vec![3, 6, 9], 6),
        ("one failed alternative replicate", vec![3], 8),
        ("one failed control replicate", vec![4], 9),
    ] {
        let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
        for index in failed_indices {
            fail_replicate(&mut lines, index, "cannot create the fixture", json!({}));
        }
        let report = report_of(&lines);
        let table = section(&report, "Table: t");
        let row = comparison_row(table, "m", "alt");
        assert_eq!(
            row[7], "insufficient replicates",
            "{what}: the arm or the control is not compared: {row:?}"
        );
        assert_eq!(
            scalar_row(table, "m", "alt")[4],
            alt_ok.to_string(),
            "{what}: the arm row counts the replicates holding a value"
        );
    }
}

/// A failed replicate is listed under `## Failures` with its profile, block, position, arm
/// and reason; the remaining replicates are still reported, and the arm's `failures` cell counts it.
/// US-BENCH-01 AC (b).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_failed_replicate_is_listed_with_its_arm_and_the_rest_are_reported() {
    let reference: Vec<f64> = (0..10).map(|i| 1000.0 + f64::from(i)).collect();
    let mut lines = two_arm(&reference, &shifted(&reference, 500.0), &reference);
    fail_replicate(&mut lines, 2 + 4 * 3 + 1, "cannot create the cgroup", json!({})); // block 4, position 1, the alt arm
    let report = report_of(&lines);
    let failures = section(&report, "Failures");
    assert!(
        failures
            .lines()
            .any(|l| l == "- profile default block 4 position 1 arm alt: cannot create the cgroup"),
        "the failed replicate is listed with its profile, block, position, arm and reason: {failures}"
    );
    assert_eq!(header(&report, "replicates ok"), "29");
    assert_eq!(header(&report, "replicates failed"), "1");
    let table = section(&report, "Table: t");
    assert_eq!(
        scalar_row(table, "m", "ref")[2],
        "1004.5",
        "the remaining replicates are still reported"
    );
    assert_eq!(
        &scalar_row(table, "m", "alt")[4..],
        ["9", "1"],
        "nine replicates hold a value; the failed replicate is counted as a failure"
    );
}

/// A metric that failed inside an otherwise ok replicate is listed with its metric name and counted
/// in the arm's `failures` cell, without counting as a stored value.
/// DDD-6 report items 3 and 6. Fault stimulus: a recorded metric failure.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_failed_metric_is_listed_with_its_arm_and_metric_and_reason() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    set_metrics(&mut lines, 3, json!({ "m": failure_entry("connect refused") })); // block 0, position 1, the alt arm
    let report = report_of(&lines);
    assert!(
        section(&report, "Failures")
            .lines()
            .any(|l| l == "- profile default block 0 position 1 arm alt metric m: connect refused"),
        "the metric failure is listed with its profile, block, position, arm, metric and reason: {report}"
    );
    assert_eq!(
        &scalar_row(section(&report, "Table: t"), "m", "alt")[4..],
        ["8", "1"],
        "eight values stored; one metric failure counted"
    );
}

/// A canary that shifted between the start and the end of the run flags the run unstable,
/// naming both medians; the report still carries every arm. US-BENCH-02 AC (e).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_shifting_canary_flags_the_run_unstable() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    let end = lines.len() - 2;
    lines[end] = canary("end", 2000.0);
    let report = report_of(&lines);
    assert_eq!(
        header(&report, "canary"),
        "unstable (start median 1007.5 ns, end median 2007.5 ns)",
        "a canary that shifted flags the run unstable"
    );
    assert_eq!(report.matches("unstable").count(), 1, "the word `unstable` appears nowhere else");
}

/// A steady canary does not.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_steady_canary_does_not_flag_the_run() {
    let report = report_of(&two_arm(&tight(), &shifted(&tight(), 500.0), &tight()));
    assert_eq!(header(&report, "canary"), "steady");
    assert!(!report.contains("unstable"), "a steady canary must not flag the run: {report}");
}

/// A declared reference is shown under its metric's table as
/// `reference: <label>: <value> <unit> (source: <source>)`, labelled as a different run.
/// US-BENCH-02 AC (f).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn the_spike_reference_is_shown_beside_the_measurement_labelled_as_a_different_run() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines[0]["declaration"]["references"] = json!([{
        "metric": "m", "value": 117.0, "label": "spike evidence, different run",
        "source": "docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md"
    }]);
    let report = report_of(&lines);
    assert!(
        section(&report, "Table: t").lines().any(|l| l
            == "reference: spike evidence, different run: 117 ns (source: docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md)"),
        "the spike reference must be shown under its metric's table, labelled: {report}"
    );
}

/// A spike probe's record is labelled spike evidence; a maintained record is not.
/// US-BENCH-04 AC (a).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_spike_record_is_labelled_spike_evidence_and_a_maintained_one_is_not() {
    let mut spike = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    set(&mut spike, 0, "evidence", json!("spike"));
    let spike_report = report_of(&spike);
    let maintained_report = report_of(&two_arm(&tight(), &shifted(&tight(), 500.0), &tight()));
    assert_eq!(header(&spike_report, "evidence"), "spike evidence");
    assert_eq!(header(&maintained_report, "evidence"), "maintained");
    assert!(
        !maintained_report.contains("spike evidence"),
        "a maintained record is not spike evidence: {maintained_report}"
    );
}

/// A record with no `run_finished` and no `run_refused` is reported incomplete (the run was
/// killed): what was written is still reported, the replicate counts are those of the `replicate`
/// lines so far, and the canary reads `not finished` (the start line exists, the end line does not).
/// US-BENCH-03 AC (d).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_record_without_run_finished_is_reported_incomplete() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines.truncate(8); // run_started, the start canary and six replicates
    let report = report_of(&lines);
    assert_eq!(header(&report, "status"), "incomplete (no run_finished or run_refused)");
    assert_eq!(
        report.matches("incomplete").count(),
        1,
        "the word `incomplete` appears nowhere else"
    );
    assert_eq!(
        header(&report, "canary"),
        "not finished",
        "a start canary with no end canary is `not finished`"
    );
    assert_eq!(header(&report, "replicates ok"), "6", "counted from the replicate lines so far");
    assert_eq!(header(&report, "replicates failed"), "0");
    assert_eq!(
        header(&report, "bpf stats"),
        "on (per process)",
        "the run held its statistics before it was killed"
    );
}

/// A refused run's report names the refused setting with its found and pinned values (or its
/// reason), is in the terminal state `refused`, and has no further section. The `setting` is one
/// of the pinned identifiers of DDD-7 (or `bpf_stats`). US-BENCH-03 AC (a).
/// Cases: a difference, a pinned setting with no source (found `absent`), disagreeing governors
/// (distinct governors sorted and joined by `,`), a statistics holder, a debug build, an unreadable
/// source and a failed statistics enable; a failed host probe is its own test below (every `host` key
/// unreadable). DDD-7.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_refused_run_names_the_setting_with_its_found_and_pinned_values() {
    for (line, expected) in [
        (refusal("bpf_stats_enabled", "1", "0"), "bpf_stats_enabled found 1 pinned 0"),
        (refusal("governor", "absent", "schedutil"), "governor found absent pinned schedutil"),
        (
            refusal("governor", "performance,schedutil", "schedutil"),
            "governor found performance,schedutil pinned schedutil",
        ),
        (
            refusal("bpf_stats_holder", "held by pid 4242", "none"),
            "bpf_stats_holder found held by pid 4242 pinned none",
        ),
        (refusal("debug_assertions", "on", "off"), "debug_assertions found on pinned off"),
        (
            refusal_reason("smt", "unreadable: permission denied"),
            "smt: unreadable: permission denied",
        ),
        (
            refusal_reason("bpf_stats", "BPF_ENABLE_STATS failed: operation not permitted"),
            "bpf_stats: BPF_ENABLE_STATS failed: operation not permitted",
        ),
    ] {
        let report = report_of(&vec![refused_start(&["ref", "alt"]), line]);
        assert_eq!(header(&report, "refused"), expected);
        assert_eq!(
            header(&report, "status"),
            "refused",
            "a refusal is a terminal state, not an incomplete run"
        );
        assert!(!report.contains("incomplete"), "a refusal is not incomplete: {report}");
        assert!(!report.contains("## Profile:"), "a refused run has no further section: {report}");
    }
}

/// A failed host probe writes `run_started` with every `host` key set to `{ "unreadable": "<probe error>" }`
/// (no setting was read), then `run_refused` with setting `host_probe`, `found` = the probe's error and
/// `pinned` = `probe succeeds`. The report prints the refusal and all nine host keys unreadable (DDD-7).
/// Fault stimulus: the probe's own self-check fails before any setting is read.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_failed_host_probe_refuses_with_every_host_key_unreadable() {
    let probe_error = "cannot read /proc/self/stat: permission denied";
    let mut started = refused_start(&["ref", "alt"]);
    for key in [
        "kernel",
        "cpu_model",
        "cpu_count",
        "governor",
        "smt",
        "boost",
        "bpf_stats_enabled",
        "clocksource",
        "irq_time_accounting",
    ] {
        started["host"][key] = json!({ "unreadable": probe_error });
    }
    let report = report_of(&vec![started, refusal("host_probe", probe_error, "probe succeeds")]);
    assert_eq!(
        header(&report, "refused"),
        format!("host_probe found {probe_error} pinned probe succeeds"),
        "the probe's error is `found` and `probe succeeds` is `pinned`"
    );
    for key in [
        "kernel",
        "cpu_model",
        "cpu_count",
        "governor",
        "smt",
        "boost",
        "bpf_stats_enabled",
        "clocksource",
        "irq_time_accounting",
    ] {
        assert_eq!(
            header(&report, key),
            format!("unreadable: {probe_error}"),
            "host key `{key}` is unreadable because no setting was read"
        );
    }
    assert_eq!(header(&report, "bpf stats"), "off", "nothing was held");
    assert_eq!(header(&report, "canary"), "not run");
    assert_eq!(header(&report, "status"), "refused");
    assert!(!report.contains("## Profile:"), "a refused run has no further section: {report}");
}

/// A refused run carries the full `host` block (every key, with its absent and unreadable forms),
/// reports the statistics `off` (none was held), the canary `not run` and zero replicates.
/// DDD-6 report item 1, DDD-7. Fault stimulus: a host with no cpufreq source (governor and boost
/// absent) and an unreadable recorded-only setting.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_refused_run_reports_the_full_host_block_no_statistics_no_canary_and_no_replicates() {
    let mut started = refused_start(&["ref", "alt"]);
    started["host"]["governor"] = json!({ "absent": "no cpufreq directory" });
    started["host"]["boost"] =
        json!({ "absent": "neither cpufreq/boost nor intel_pstate/no_turbo exists" });
    started["host"]["irq_time_accounting"] = json!({ "unreadable": "input/output error" });
    let report = report_of(&vec![started, refusal("governor", "absent", "schedutil")]);
    for (key, value) in [
        ("kernel", "7.0.0-38-generic"),
        ("cpu_model", "AMD EPYC 8024P"),
        ("cpu_count", "8"),
        ("governor", "absent: no cpufreq directory"),
        ("smt", "on"),
        ("boost", "absent: neither cpufreq/boost nor intel_pstate/no_turbo exists"),
        ("bpf_stats_enabled", "0"),
        ("clocksource", "tsc"),
        ("irq_time_accounting", "unreadable: input/output error"),
        ("bpf stats", "off"),
        ("canary", "not run"),
        ("replicates ok", "0"),
        ("replicates failed", "0"),
        ("status", "refused"),
    ] {
        assert_eq!(header(&report, key), value, "refused-run header line `{key}`");
    }
}

/// A metric none of whose entries holds a value prints its reason in the median and IQR cells and
/// never a zero there; its `replicates` is 0. US-BENCH-06 AC (c), US-BENCH-09 AC (d).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn an_absent_metric_prints_its_reason_and_never_a_zero() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    for block in 0..9 {
        set_metrics(&mut lines, 2 + block * 3, json!({ "m": absent_entry("no hardware counter") }));
    }
    let report = report_of(&lines);
    let row = scalar_row(section(&report, "Table: t"), "m", "ref");
    assert_eq!(
        row[2], "absent: no hardware counter",
        "an absent metric must print `absent: <reason>` as its median: {row:?}"
    );
    assert_eq!(row[3], "absent: no hardware counter", "and as its IQR: {row:?}");
    assert_eq!(&row[4..], ["0", "0"], "no replicate holds a value and none failed: {row:?}");
}

/// A metric with no stored value in either compared arm has no comparison row (no delta, no
/// interval, no verdict): the arm row above prints `absent: <reason>`. Another metric of the same
/// table is still compared. US-BENCH-08 AC (b), US-BENCH-09 AC (d). Fault stimulus: the reference
/// arm (a handshake that a plaintext baseline does not have) and then the non-reference arm hold
/// no value.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_metric_with_no_stored_value_in_a_compared_arm_has_no_comparison_row() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines[0]["declaration"]["metrics"] = json!([
        metric_decl("m", "ns", "t", false),
        metric_decl("h", "ns", "t", false),
        metric_decl("g", "ns", "t", false),
    ]);
    for line in lines.iter_mut().filter(|l| l["t"] == "replicate") {
        let arm = line["arm"].as_str().unwrap_or_default().to_owned();
        let role = line["role"].as_str().unwrap_or_default().to_owned();
        let value = line["metrics"]["m"]["values"][0].as_f64().expect("a value of m");
        let h = if arm == "ref" {
            absent_entry("no handshake on a plaintext connection")
        } else {
            values_entry(&[value])
        };
        let g = if arm == "alt" && role == "arm" {
            absent_entry("no forward thread on this path")
        } else {
            values_entry(&[value])
        };
        line["metrics"] = json!({ "m": values_entry(&[value]), "h": h, "g": g });
    }
    let report = report_of(&lines);
    let table = section(&report, "Table: t");
    assert!(has_comparison_row(table, "m", "alt"), "a metric both arms hold is compared: {table}");
    assert!(
        !has_comparison_row(table, "h", "alt"),
        "no comparison row when the reference arm holds no value: {table}"
    );
    assert!(
        !has_comparison_row(table, "g", "alt"),
        "no comparison row when the compared arm holds no value: {table}"
    );
    assert_eq!(
        scalar_row(table, "h", "ref")[2],
        "absent: no handshake on a plaintext connection",
        "the arm row prints the reason"
    );
    assert_eq!(
        scalar_row(table, "g", "alt")[2],
        "absent: no forward thread on this path",
        "the arm row prints the reason"
    );
}

/// A series metric whose replicates exceed 1,000 values is stored as a histogram: the report's
/// percentiles equal the pinned DDD-5 computation (bucket interpolation, clamp to the stored min and
/// max, then R-7), stay within the stated error of the exact R-7 percentiles (3.125% relative for values
/// of 1 and above, absolute error below 1 for smaller values, which fall in bucket 0), and a note states
/// both. Raw values of up to 1,000 carry no note and are exact. Boundary: 1,000 raw values; 2,000 in a
/// histogram. DDD-5, DDD-6 item 3.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_series_stored_as_a_histogram_renders_percentiles_within_the_stated_error_and_says_so() {
    let above_one: Vec<f64> = (1..=2000).map(f64::from).collect();
    let below_one: Vec<f64> = (1..=2000).map(|i| f64::from(i) / 4000.0).collect();
    for (values, what) in [(&above_one, "values of 1 and above"), (&below_one, "values below 1")] {
        let report = report_of(&one_arm_series_record(histogram_entry(values)));
        let table = section(&report, "Table: t");
        assert!(
            table.contains(
                "| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |"
            ),
            "the series table carries the pinned columns: {table}"
        );
        let row = series_row(table, "m", "lifecycle");
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        for (cell, q, name) in
            [(2, 0.50, "p50"), (3, 0.90, "p90"), (4, 0.95, "p95"), (5, 0.99, "p99")]
        {
            assert_eq!(
                row[cell],
                histogram_percentile(values, q).to_string(),
                "{name} of a histogram of {what} must equal the pinned DDD-5 computation"
            );
            let reported = number(&row[cell], name);
            let exact = r7(values, q);
            if exact < 1.0 {
                assert!(
                    (reported - exact).abs() < 1.0,
                    "{name} ({what}) must be within an absolute error of 1: {reported} vs {exact}"
                );
            } else {
                assert!(
                    (reported - exact).abs() <= 0.03125 * exact,
                    "{name} ({what}) must be within 3.125% of the exact R-7 value: {reported} vs {exact}"
                );
            }
            assert!(
                (low..=high).contains(&reported),
                "{name} must stay within the stored min and max: {reported}"
            );
        }
        assert_eq!(&row[6..], ["2000", "1", "0"], "values, replicates, failures");
        assert!(
            table.lines().any(|l| l
                == "percentiles of m are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)"),
            "a histogram-derived series states both errors: {table}"
        );
    }

    let raw: Vec<f64> = (1..=1000).map(f64::from).collect();
    let raw_report = report_of(&one_arm_series_record(values_entry(&raw)));
    let raw_row = series_row(section(&raw_report, "Table: t"), "m", "lifecycle");
    assert!(
        (number(&raw_row[2], "p50") - r7(&raw, 0.50)).abs() < 1e-9,
        "raw values give the exact R-7 percentile: {raw_row:?}"
    );
    assert!(
        !raw_report.contains("histogram-derived"),
        "raw values carry no histogram note: {raw_report}"
    );
}

/// Series percentiles use linear interpolation between closest ranks (R-7): the series
/// 1..=10 has p50 5.5, p90 9.1, p95 9.55 and p99 9.91 (compared numerically).
/// A one-arm record with a single replicate holds the pooled values exactly. DDD-5.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn series_percentiles_use_linear_interpolation_between_closest_ranks() {
    let series: Vec<f64> = (1..=10).map(f64::from).collect();
    let report = report_of(&one_arm_series_record(values_entry(&series)));
    let row = series_row(section(&report, "Table: t"), "m", "lifecycle");
    for (cell, expected, name) in
        [(2, 5.5, "p50"), (3, 9.1, "p90"), (4, 9.55, "p95"), (5, 9.91, "p99")]
    {
        let value = number(&row[cell], name);
        assert!(
            (value - expected).abs() < 1e-9,
            "{name} must use linear interpolation between closest ranks: {value} != {expected}"
        );
    }
    assert_eq!(&row[6..], ["10", "1", "0"], "values, replicates, failures");
}

/// A metric is a series exactly when the declaration echo marks it `series`: one marked series
/// with a single value reports the series columns, one not marked reports the scalar columns.
/// DDD-2 (`series` marker), DDD-6 report item 3.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn the_series_marker_alone_selects_the_series_columns() {
    let mut lines = vec![run_started(&["only"]), canary("start", 1000.0)];
    lines[0]["declaration"] = declaration(
        &["only"],
        &[metric_decl("m", "ns", "t", false), metric_decl("s", "ns", "s", true)],
    );
    lines.push(replicate(
        0,
        0,
        "only",
        "arm",
        json!({ "m": values_entry(&[7.0]), "s": values_entry(&[7.0]) }),
    ));
    lines.push(canary("end", 1000.0));
    lines.push(finished(1, 0));
    let report = report_of(&lines);
    assert_eq!(
        &scalar_row(section(&report, "Table: t"), "m", "only")[2..],
        ["7", "0", "1", "0"],
        "not marked: median, IQR, replicates, failures"
    );
    assert_eq!(
        &series_row(section(&report, "Table: s"), "s", "only")[2..],
        ["7", "7", "7", "7", "1", "1", "0"],
        "marked: percentiles, values, replicates, failures"
    );
}

/// One table per declared table label, so client-observed and resolver-internal timings are
/// separate tables and neither is a column of the other. US-BENCH-07 AC (a).
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn metrics_are_reported_in_separate_tables_by_their_declared_label() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines[0]["declaration"]["metrics"] = json!([
        metric_decl("client_latency_ns", "ns", "client-observed", true),
        metric_decl("resolver_service_ns", "ns", "resolver-internal", true),
    ]);
    for line in lines.iter_mut().filter(|l| l["t"] == "replicate") {
        line["metrics"] = json!({
            "client_latency_ns": values_entry(&[100.0, 200.0]),
            "resolver_service_ns": values_entry(&[10.0, 20.0])
        });
    }
    let report = report_of(&lines);
    let client = section(&report, "Table: client-observed");
    let resolver = section(&report, "Table: resolver-internal");
    assert!(
        client.contains("client_latency_ns")
            && !client.contains("resolver_service_ns")
            && resolver.contains("resolver_service_ns")
            && !resolver.contains("client_latency_ns"),
        "metrics must be reported in the table they declare, never as a column of another: {report}"
    );
    assert!(report.contains("## Profile: default"), "each profile has its own heading: {report}");
}

/// Only `ok` replicates feed a statistic: a failed replicate's partial metrics are recorded but
/// never used in a median, a pooled percentile, a value count or a delta, and the arm's `failures`
/// cell counts the failed replicate. DDD-5 ("Only `ok` replicates feed any statistic"), DDD-6 item 3.
/// Fault stimulus: a failed replicate holding an extreme partial value.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn only_ok_replicates_feed_the_statistics() {
    let reference: Vec<f64> = (0..10).map(|i| 1000.0 + f64::from(i)).collect();
    let mut lines = two_arm(&reference, &shifted(&reference, 500.0), &reference);
    fail_replicate(
        &mut lines,
        2 + 4 * 3,
        "the fixture was killed",
        json!({ "m": values_entry(&[1.0e9]) }),
    ); // block 4, the reference arm
    let report = report_of(&lines);
    let table = section(&report, "Table: t");
    assert_eq!(
        &scalar_row(table, "m", "ref")[2..],
        ["1005", "5", "9", "1"],
        "median and IQR of the nine ok values; the failed replicate counts only as a failure"
    );
    assert_eq!(
        comparison_row(table, "m", "alt")[2],
        "499.5",
        "the delta uses the nine ok reference values: 1504.5 - 1005"
    );

    let mut series =
        one_arm_series_record(values_entry(&(1..=10).map(f64::from).collect::<Vec<_>>()));
    let mut partial = replicate(1, 0, "lifecycle", "arm", json!({ "m": values_entry(&[1.0e9]) }));
    partial["outcome"] = json!("failed");
    partial["failure"] = json!("the guest never became ready");
    series.insert(3, partial);
    recount(&mut series);
    let series_report = report_of(&series);
    let row = series_row(section(&series_report, "Table: t"), "m", "lifecycle");
    assert!(
        number(&row[5], "p99") <= 10.0,
        "the pooled p99 ignores the failed replicate's partial values: {row:?}"
    );
    assert_eq!(
        &row[6..],
        ["10", "1", "1"],
        "values, replicates and failures count only the ok replicate"
    );
}

/// A series metric entry of an `ok` replicate that holds measured values and failures together (a
/// partly measured series) feeds its values into the pooled percentiles exactly as an entry without
/// failures does; the arm's `replicates` counts the replicate, its `failures` counts it once however many
/// reasons it holds, and `## Failures` lists one line per reason in call order. DDD-6 "Partly measured
/// series", DDD-5 failure accounting. Fault stimulus: one workload's stage failed (two reasons in the first
/// replicate, one in the second) while the others measured; a third replicate measured cleanly.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_partly_measured_series_pools_its_values_and_is_counted_in_replicates_and_failures() {
    let all: Vec<f64> = (1..=15).map(f64::from).collect();
    let mut lines = one_arm_series_record(partly_measured_entry(
        &all[0..5],
        &["workload 2 never reached the stable row", "workload 3 never reached the stable row"],
    ));
    lines.insert(
        3,
        replicate(
            1,
            0,
            "lifecycle",
            "arm",
            json!({ "m": partly_measured_entry(&all[5..10], &["workload 1 was never reaped"]) }),
        ),
    );
    lines
        .insert(4, replicate(2, 0, "lifecycle", "arm", json!({ "m": values_entry(&all[10..15]) })));
    recount(&mut lines);
    let report = report_of(&lines);
    let row = series_row(section(&report, "Table: t"), "m", "lifecycle");
    for (cell, q, name) in [(2, 0.5, "p50"), (3, 0.9, "p90"), (4, 0.95, "p95"), (5, 0.99, "p99")] {
        let value = number(&row[cell], name);
        assert!(
            (value - r7(&all, q)).abs() < 1e-9,
            "{name} must pool the values of the partly measured entries with the clean one: {value} != {}",
            r7(&all, q)
        );
    }
    assert_eq!(
        &row[6..],
        ["15", "3", "2"],
        "values (all fifteen), replicates (the partly measured entries count) and failures (one per replicate, not per reason)"
    );
    assert_eq!(header(&report, "replicates ok"), "3", "a partly measured replicate stays ok");
    assert_eq!(header(&report, "replicates failed"), "0");
    let failures: Vec<&str> =
        section(&report, "Failures").lines().filter(|l| l.starts_with('-')).collect();
    assert_eq!(
        failures,
        [
            "- profile default block 0 position 0 arm lifecycle metric m: workload 2 never reached the stable row",
            "- profile default block 0 position 0 arm lifecycle metric m: workload 3 never reached the stable row",
            "- profile default block 1 position 0 arm lifecycle metric m: workload 1 was never reaped",
        ],
        "one line per failure reason, in replicate then call order"
    );
}

/// A partly measured entry whose values are stored as a histogram pools through the histogram and
/// counts its failure like a raw one. DDD-5, DDD-6 "Partly measured series". Boundary: 2,000 values (above
/// the raw limit) with one failure alongside.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_partly_measured_histogram_entry_pools_through_the_histogram_and_counts_its_failure() {
    let series: Vec<f64> = (1..=2000).map(f64::from).collect();
    let report = report_of(&one_arm_series_record(partly_measured_histogram_entry(
        &series,
        &["workload 4 never reached the stable row"],
    )));
    let row = series_row(section(&report, "Table: t"), "m", "lifecycle");
    for (cell, q, name) in [(2, 0.5, "p50"), (3, 0.9, "p90"), (4, 0.95, "p95"), (5, 0.99, "p99")] {
        let value = number(&row[cell], name);
        let expected = histogram_percentile(&series, q);
        assert!(
            (value - expected).abs() < 1e-9,
            "{name} must be the pinned histogram percentile of the stored values: {value} != {expected}"
        );
    }
    assert_eq!(&row[6..], ["2000", "1", "1"], "values, replicates and failures");
    assert!(
        section(&report, "Failures").contains(
            "- profile default block 0 position 0 arm lifecycle metric m: workload 4 never reached the stable row"
        ),
        "the failure alongside the histogram is listed: {report}"
    );
}

/// A partly measured replicate stays `ok`: it counts toward the nine-replicate minimum, its replicate median
/// feeds the delta, and the report counts it in `replicates` and in `failures`. Boundary: the alternative
/// arm has exactly nine `ok` replicates, one of them partly measured; were it counted failed the arm would
/// read `insufficient replicates`. DDD-5, DDD-6 "Partly measured series".
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_partly_measured_replicate_counts_toward_the_nine_replicate_minimum_and_feeds_the_delta() {
    let mut lines = two_arm(&tight(), &shifted(&tight(), 500.0), &tight());
    lines[0]["declaration"] = declaration(&["ref", "alt"], &[metric_decl("m", "ns", "t", true)]);
    for line in lines.iter_mut().filter(|line| line["t"] == "replicate") {
        let value = line["metrics"]["m"]["values"][0].as_f64().expect("a scalar value");
        line["metrics"] = json!({ "m": values_entry(&[value, value + 2.0]) });
    }
    // Block 0, position 1: the alternative arm's first replicate lost one contribution.
    set_metrics(
        &mut lines,
        3,
        json!({ "m": partly_measured_entry(&[1500.0, 1502.0], &["workload 3 never reached the stable row"]) }),
    );
    let report = report_of(&lines);
    let table = section(&report, "Table: t");
    assert_eq!(
        &series_row(table, "m", "alt")[6..],
        ["18", "9", "1"],
        "all nine replicates hold values (two each) and the partly measured one is counted as a failure too"
    );
    let row = comparison_row(table, "m", "alt");
    assert_eq!(
        [row[2].as_str(), row[7].as_str()],
        ["500", "distinguishable"],
        "the delta of the nine replicate medians (1505 against 1005) is reported, not `insufficient replicates`: {row:?}"
    );
    assert_eq!(header(&report, "replicates ok"), "27");
    assert_eq!(header(&report, "replicates failed"), "0");
}

/// The report is a pure function of the record: rendering twice gives identical bytes and
/// never modifies the record. ADR-0301 (nothing derived is stored).
/// CONTRACT_SHAPE: unbounded-preservation (the record file is unchanged).
#[test]
fn rendering_the_same_record_twice_is_byte_identical_and_leaves_the_record_untouched() {
    let d = dir();
    let path = write(d.path(), "r.jsonl", &two_arm(&tight(), &shifted(&tight(), 500.0), &tight()));
    let before = std::fs::read(&path).expect("read record");
    let first = render(&path);
    let second = render(&path);
    assert!(
        !first.is_empty(),
        "rendering the same record twice must give identical, non-empty reports"
    );
    assert_eq!(first, second, "rendering the same record twice must give identical reports");
    assert_eq!(
        std::fs::read(&path).expect("read record"),
        before,
        "rendering must not modify the record"
    );
}

/// A single-arm benchmark (a distribution benchmark) reports its distribution and no
/// comparison table, no A/A text and no verdict (there is no second arm and no control).
/// Edge: arm cardinality 1. DDD-5 ("none for one arm"), DDD-6 report item 4.
/// CONTRACT_SHAPE: pure-function (record in, report out).
#[test]
fn a_one_arm_benchmark_reports_a_distribution_and_no_comparison() {
    let mut lines = vec![run_started(&["lifecycle"]), canary("start", 1000.0)];
    for (block, value) in tight().iter().enumerate() {
        lines.push(replicate(block, 0, "lifecycle", "arm", scalar(*value)));
    }
    lines.push(canary("end", 1000.0));
    lines.push(finished(9, 0));
    let report = report_of(&lines);
    assert_eq!(
        &scalar_row(section(&report, "Table: t"), "m", "lifecycle")[2..],
        ["1004", "4", "9", "0"],
        "a one-arm benchmark reports its distribution"
    );
    for forbidden in ["A/A", "Mann-Whitney", "95% bootstrap interval", "distinguishable"] {
        assert!(
            !report.contains(forbidden),
            "no comparison is made for one arm (`{forbidden}`): {report}"
        );
    }
}

// ---- The guest substrate (guest-benchmark-substrate, ADR-0326, DDD-2 "Report") ----
//
// The report of a guest record states `substrate: guest` and, between the nine host lines and
// `bpf stats`, the guest's conditions: how it was launched, booted and scheduled, the three
// settings a guest cannot see (absent, with the reason), and what the first replicate's view
// observed. A native report is byte-identical to what it was. Evidence lane: integration
// (subprocess). CONTRACT_SHAPE: pure-function (record in, report out) unless a test says otherwise.

/// The report lines from the one after `from` (a line prefix) to the one before `to`.
fn between<'a>(report: &'a str, from: &str, to: &str) -> Vec<&'a str> {
    let lines: Vec<&str> = report.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with(from))
        .unwrap_or_else(|| panic!("the report must have a line starting `{from}`:\n{report}"));
    let end = lines
        .iter()
        .position(|line| line.starts_with(to))
        .unwrap_or_else(|| panic!("the report must have a line starting `{to}`:\n{report}"));
    lines[start + 1..end].to_vec()
}

/// The guest conditions in the report, in order: after `irq_time_accounting` (the last host line)
/// and before `bpf stats`, one line each, the settings printing `absent: <reason>`.
/// US-GBS-02 AC (b), (c), (d); K2.
#[test]
fn the_report_states_the_guests_conditions_between_the_host_lines_and_the_statistics_line() {
    let report = report_of(&guest_record(9));
    assert_eq!(header(&report, "substrate"), "guest");
    let expected = [
        "- guest launch: metal host (not nested)".to_owned(),
        "- guest boot: fresh guest per replicate".to_owned(),
        "- guest vcpu scheduling: unpinned".to_owned(),
        format!("- guest governor: absent: {NOT_VISIBLE}"),
        format!("- guest smt: absent: {NOT_VISIBLE}"),
        format!("- guest boost: absent: {NOT_VISIBLE}"),
        "- guest vcpus: 2".to_owned(),
        "- guest memory mib: 512".to_owned(),
        "- guest kernel: 7.0.0-38-generic".to_owned(),
        "- guest clocksource: kvm-clock".to_owned(),
        format!("- guest artifact overdrive_init: {}", digest('a')),
        format!("- guest artifact overdrive_guest_peer: {}", digest('b')),
        format!("- guest artifact ovd_fwd: {}", digest('c')),
    ];
    assert_eq!(
        between(&report, "- irq_time_accounting: ", "- bpf stats: "),
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "the guest lines sit between the last host line and the statistics line:\n{report}"
    );
    assert!(
        header(&report, "clock").contains(GUEST_MONOTONIC_CLOCK),
        "the clock line names the guest clock source: {report}"
    );
}

/// A guest record with no `guest_observed` line (every replicate failed before reporting a view)
/// prints each observed value as `absent: no replicate returned a guest view`, never a zero or a
/// blank, while the conditions stated before any guest booted are still printed.
/// CA-4 (a record in which every replicate failed before reporting a view).
#[test]
fn a_guest_record_without_a_view_prints_the_observed_values_as_absent() {
    let reason = "guest boot: no READY within 60s";
    let mut lines = vec![guest_started(), canary("start", 1000.0)];
    lines.extend((0..9).map(guest_replicate));
    lines.push(canary("end", 1000.0));
    lines.push(finished(9, 0));
    for index in 2..11 {
        fail_replicate(&mut lines, index, reason, json!({ GUEST_METRIC: failure_entry(reason) }));
    }
    let report = report_of(&lines);
    let none = "absent: no replicate returned a guest view";
    for key in [
        "guest vcpus",
        "guest memory mib",
        "guest kernel",
        "guest clocksource",
        "guest artifact overdrive_init",
        "guest artifact overdrive_guest_peer",
        "guest artifact ovd_fwd",
    ] {
        assert_eq!(header(&report, key), none, "header line `{key}`");
    }
    assert_eq!(header(&report, "guest launch"), "metal host (not nested)");
    assert_eq!(header(&report, "guest vcpu scheduling"), "unpinned");
    assert_eq!(header(&report, "replicates ok"), "0");
    assert_eq!(header(&report, "replicates failed"), "9");
    assert!(
        report.contains(&format!("arm forwarding: {reason}")),
        "each failed replicate is listed with its arm and reason:\n{report}"
    );
}

/// A host refused before any guest booted still reports the guest conditions known beforehand,
/// the refusal naming the setting with its found and pinned values, and no guest view.
/// US-GBS-02 AC (a): the run refuses before any guest boots.
#[test]
fn a_guest_run_refused_on_a_drifted_host_reports_the_refusal_and_the_static_conditions() {
    let report = report_of(&vec![guest_started(), refusal("bpf_stats_enabled", "1", "0")]);
    assert_eq!(header(&report, "status"), "refused");
    assert_eq!(header(&report, "refused"), "bpf_stats_enabled found 1 pinned 0");
    assert_eq!(header(&report, "guest launch"), "metal host (not nested)");
    assert_eq!(header(&report, "guest kernel"), "absent: no replicate returned a guest view");
    assert_eq!(header(&report, "replicates ok"), "0");
    assert!(!report.contains("incomplete"), "a refusal is not incomplete: {report}");
}

/// A guest run killed before its terminal line is reported incomplete and keeps the replicates
/// written so far; a replicate that failed in the guest is counted with its arm and reason and the
/// others are reported. US-GBS-03 AC (b), (c) as CA-1 restates them.
#[test]
fn a_killed_guest_run_is_incomplete_and_a_failed_guest_replicate_is_counted() {
    let reason = "guest run: no EXIT within the bound";
    let mut lines = guest_record(9);
    fail_replicate(&mut lines, 8, reason, json!({ GUEST_METRIC: failure_entry(reason) }));
    lines.truncate(lines.len() - 2);
    let report = report_of(&lines);
    assert_eq!(header(&report, "status"), "incomplete (no run_finished or run_refused)");
    assert_eq!(header(&report, "replicates failed"), "1");
    let failures = section(&report, "Failures");
    assert!(
        failures.contains(&format!("arm forwarding: {reason}")),
        "the failed replicate is listed with its arm and reason: {failures}"
    );
}

/// A one-arm guest benchmark reports its distribution and no comparison: the series row carries
/// percentiles, the value count and the replicate count over every retained sample, and only `ok`
/// replicates feed it. US-GBS-01 AC (a); the report statistics derive only from retained samples.
#[test]
fn a_one_arm_guest_report_shows_the_distribution_of_the_retained_samples() {
    let report = report_of(&guest_record(9));
    let table = section(&report, "Table: forwarded-path");
    let row = series_row(table, GUEST_METRIC, "forwarding");
    assert_eq!(
        &row[6..],
        ["27", "9", "0"],
        "values, replicates, failures of nine replicates of three samples"
    );
    assert!(
        !report.contains("distinguishable") && !report.contains("A/A"),
        "a one-arm family has no comparison and no A/A control: {report}"
    );
}

/// Rendering a guest record twice is byte-identical and leaves the record untouched.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn rendering_a_guest_record_twice_is_byte_identical_and_leaves_the_record_untouched() {
    let d = dir();
    let path = write(d.path(), "r.jsonl", &guest_record(9));
    let before = std::fs::read(&path).expect("read");
    let first = render(&path);
    let second = render(&path);
    assert_eq!(first, second);
    assert_eq!(std::fs::read(&path).expect("read"), before);
}

/// A native report prints no guest line and keeps its substrate: the guest conditions exist only
/// for a record that has a guest block. GREEN guard (US-GBS-01 scenario "native unchanged").
#[test]
fn a_native_report_prints_no_guest_line_and_states_substrate_native() {
    let report = report_of(&two_arm(&tight(), &shifted(&tight(), 500.0), &tight()));
    assert_eq!(header(&report, "substrate"), "native");
    assert!(
        !report.lines().any(|line| line.starts_with("- guest ")),
        "a native report has no guest line:\n{report}"
    );
}

/// Every committed record that retains a `report.md` re-renders to exactly those bytes: the
/// change adds nothing to a native report (K4, DDD-2 "A native report is byte-identical"), and
/// every committed native record (every record under a native family directory, and any record
/// stating substrate native) has no guest block and no `guest_observed` line. Guest records are
/// admitted and re-render byte-identical too. GREEN guard.
/// CONTRACT_SHAPE: unbounded-preservation (retained bytes against a fresh render).
#[test]
fn every_committed_native_report_re_renders_byte_identical() {
    fn records(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.map(|entry| entry.expect("read a results entry")) {
            let path = entry.path();
            if path.is_dir() {
                records(&path, found);
            } else if path.file_name().is_some_and(|name| name == "record.jsonl") {
                found.push(path);
            }
        }
    }
    let root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    let mut found = Vec::new();
    records(&root, &mut found);
    let native_families = ["cgroup-connect-hook", "dns-responder", "mtls-path", "vm-lifecycle"];
    let mut compared = 0;
    let mut native_compared = 0;
    for record in &found {
        let text = std::fs::read_to_string(record).expect("read the record");
        let in_native_family = record
            .strip_prefix(&root)
            .ok()
            .and_then(|rel| rel.components().next())
            .is_some_and(|family| native_families.iter().any(|n| family.as_os_str() == *n));
        let mut substrate = None;
        for line in text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).expect("a record line");
            if value["t"] == "run_started" {
                substrate = Some(value["substrate"].clone());
                if in_native_family || value["substrate"] == "native" {
                    assert_eq!(value["substrate"], "native", "{} is native", record.display());
                    assert!(
                        value.get("guest").is_none(),
                        "{} has no guest block",
                        record.display()
                    );
                }
            }
            if in_native_family || substrate.as_ref().is_some_and(|s| s == "native") {
                assert_ne!(value["t"], "guest_observed", "{} has no guest view", record.display());
            }
        }
        let report = record.with_file_name("report.md");
        if let Ok(retained) = std::fs::read(&report) {
            let rendered = render(record);
            assert!(
                rendered.as_bytes() == retained.as_slice(),
                "re-rendering {} must give the retained report.md byte for byte",
                record.display()
            );
            compared += 1;
            if substrate.is_some_and(|s| s == "native") {
                native_compared += 1;
            }
        }
    }
    assert!(compared > 0, "at least one committed record retains a report to compare");
    assert!(native_compared > 0, "at least one native report is compared");
}
