//! Shared fixtures: a record builder and the `bench-report` driver.
//!
//! Every shape below is the one DDD-6 pins: `host` has exactly the nine keys, each a
//! string, `{ "absent": "<reason>" }` or `{ "unreadable": "<error>" }`; `clock` is
//! `{ host, service, guest }`; `declaration` echoes the attribute (a metric echoes its
//! `series` marker); a metric entry is `{ values, histogram, failures, absent }`; a
//! replicate's `commands` are `{ program, args, status }`. Line numbers in the tests are
//! 1-based file lines.
#![allow(dead_code, reason = "each scenario file uses a subset of the builders")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    reason = "fixture arithmetic mirrors the DDD-5 formulas on small, positive, exactly representable values"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// A record as its lines.
pub type Lines = Vec<Value>;

pub const RUN_ID: &str = "0123456789abcdef01234567";
pub const COMMIT: &str = "e3b42c92e0123456789abcdef0123456789abcde";

/// The nine-key `host` block of a conforming host.
pub fn host_block() -> Value {
    json!({
        "kernel": "7.0.0-38-generic", "cpu_model": "AMD EPYC 8024P", "cpu_count": "8",
        "governor": "schedutil", "smt": "on", "boost": "on",
        "bpf_stats_enabled": "0", "clocksource": "tsc", "irq_time_accounting": "y"
    })
}

/// One metric of the declaration echo.
pub fn metric_decl(name: &str, unit: &str, table: &str, series: bool) -> Value {
    json!({ "name": name, "unit": unit, "table": table, "series": series })
}

/// The declaration echo of `arms` and `metrics` with one implicit profile and no reference.
pub fn declaration(arms: &[&str], metrics: &[Value]) -> Value {
    json!({
        "arms": arms, "replicates": 9,
        "profiles": [{ "name": "default", "preparation": "" }],
        "metrics": metrics, "references": []
    })
}

/// `run_started` for a family with the given arms and one scalar metric `m` (table `t`).
pub fn run_started(arms: &[&str]) -> Value {
    json!({
        "t": "run_started", "schema": 1, "methodology": 1, "run_id": RUN_ID,
        "family": "cgroup-connect-hook", "benchmark": "connect_hook_cost",
        "evidence": "maintained", "substrate": "native", "commit": COMMIT, "dirty": false,
        "debug_assertions": false, "seed": 424_242, "started": "2026-11-17T10:00:00Z",
        "host": host_block(),
        "clock": { "host": "CLOCK_MONOTONIC_RAW", "service": null, "guest": null },
        "bpf_stats": "per_process",
        "declaration": declaration(arms, &[metric_decl("m", "ns", "t", false)])
    })
}

/// `run_started` of a refused run: nothing was held, so `bpf_stats` is `off`.
pub fn refused_start(arms: &[&str]) -> Value {
    let mut started = run_started(arms);
    started["bpf_stats"] = json!("off");
    started
}

/// A `run_refused` line naming a difference.
pub fn refusal(setting: &str, found: &str, pinned: &str) -> Value {
    json!({ "t": "run_refused", "setting": setting, "found": found, "pinned": pinned })
}

/// A `run_refused` line naming a reason.
pub fn refusal_reason(setting: &str, reason: &str) -> Value {
    json!({ "t": "run_refused", "setting": setting, "reason": reason })
}

pub fn canary(phase: &str, base: f64) -> Value {
    let samples: Vec<f64> = (0..16).map(|i| base + f64::from(i)).collect();
    json!({ "t": "canary", "phase": phase, "samples_ns": samples })
}

/// A metric entry holding raw values.
pub fn values_entry(values: &[f64]) -> Value {
    json!({ "values": values, "histogram": null, "failures": [], "absent": null })
}

/// A metric entry holding one recorded failure.
pub fn failure_entry(reason: &str) -> Value {
    json!({ "values": [], "histogram": null, "failures": [reason], "absent": null })
}

/// A series metric entry that was partly measured: raw `values` together with the `failures` of the
/// contributions that failed (DDD-6 "Partly measured series").
pub fn partly_measured_entry(values: &[f64], failures: &[&str]) -> Value {
    json!({ "values": values, "histogram": null, "failures": failures, "absent": null })
}

/// A metric entry that was absent, with its reason.
pub fn absent_entry(reason: &str) -> Value {
    json!({ "values": [], "histogram": null, "failures": [], "absent": reason })
}

/// The log-linear bucket of `value` (DDD-5 step 1): 0 below 1 (including 0 and every subnormal);
/// else `1 + 32e + s`, with `e` the unbiased binary exponent read from the `f64` bits (the `frexp`
/// exponent minus 1, never `log2`) and `s` the top 5 bits of the 52-bit fraction; 2048 from 2^64.
fn bucket_of(value: f64) -> u32 {
    if value < 1.0 {
        return 0;
    }
    let bits = value.to_bits();
    let e = ((bits >> 52) & 0x7ff).saturating_sub(1023);
    if e >= 64 {
        return 2048;
    }
    let s = (bits >> 47) & 0x1f;
    1 + 32 * (e as u32) + (s as u32)
}

/// The `[lower, upper)` range of `bucket` (DDD-5 step 1): `[0, 1)` for bucket 0, else
/// `lower = 2^e (1 + s/32)` and `upper = 2^e (1 + (s+1)/32)`.
pub fn bucket_range(bucket: u32) -> (f64, f64) {
    if bucket == 0 {
        return (0.0, 1.0);
    }
    let index = bucket - 1;
    let (e, s) = (index / 32, index % 32);
    let base = 2f64.powi(i32::try_from(e).expect("the exponent is at most 64"));
    (base * (1.0 + f64::from(s) / 32.0), base * (1.0 + f64::from(s + 1) / 32.0))
}

/// A metric entry holding `values` as the histogram DDD-5 defines: the exact `count`,
/// `min` and `max` and the sparse `[bucket, count]` pairs in ascending bucket order.
pub fn histogram_entry(values: &[f64]) -> Value {
    let mut buckets = std::collections::BTreeMap::<u32, u64>::new();
    for value in values {
        *buckets.entry(bucket_of(*value)).or_default() += 1;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let pairs: Vec<Value> = buckets.into_iter().map(|(b, c)| json!([b, c])).collect();
    json!({
        "values": [],
        "histogram": { "count": values.len(), "min": min, "max": max, "buckets": pairs },
        "failures": [], "absent": null
    })
}

/// A series metric entry partly measured above the raw limit: `values` stored as the DDD-5 histogram,
/// together with the `failures` of the contributions that failed.
pub fn partly_measured_histogram_entry(values: &[f64], failures: &[&str]) -> Value {
    let mut entry = histogram_entry(values);
    entry["failures"] = json!(failures);
    entry
}

/// The exact R-7 percentile (`q` in 0..=1) of `values`: linear interpolation between closest ranks.
pub fn r7(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = q * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    sorted[lower] + (rank - lower as f64) * (sorted[upper] - sorted[lower])
}

/// The percentile `q` (0..=1) of `values` stored as a histogram, computed exactly as DDD-5 pins it:
/// the 0-based rank `k` value is `lower + ((k - r0) as f64 + 0.5) / (c as f64) * (upper - lower)`
/// (step 2), clamped to `[min, max]` (step 3), then R-7 with `h = (n - 1) * p`, `f = floor(h)`,
/// `v(f) + (h - f) * (v(f + 1) - v(f))`, or `v(n - 1)` when `f = n - 1` (step 4).
pub fn histogram_percentile(values: &[f64], q: f64) -> f64 {
    let mut buckets = std::collections::BTreeMap::<u32, u64>::new();
    for value in values {
        *buckets.entry(bucket_of(*value)).or_default() += 1;
    }
    let n = values.len();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let value_at = |k: usize| -> f64 {
        let k = k as u64;
        let mut r0 = 0u64;
        for (bucket, count) in &buckets {
            if k < r0 + count {
                let (lower, upper) = bucket_range(*bucket);
                let v = lower + ((k - r0) as f64 + 0.5) / (*count as f64) * (upper - lower);
                return if v < min {
                    min
                } else if v > max {
                    max
                } else {
                    v
                };
            }
            r0 += count;
        }
        unreachable!("rank {k} is below the value count")
    };
    let h = (n - 1) as f64 * q;
    let f = h.floor();
    let rank = f as usize;
    if rank == n - 1 {
        value_at(n - 1)
    } else {
        value_at(rank) + (h - f) * (value_at(rank + 1) - value_at(rank))
    }
}

/// A metric entry holding a histogram written out by hand, for the malformed cases: `buckets` is the
/// JSON array of `[bucket, count]` pairs exactly as given, in the order given.
#[allow(clippy::needless_pass_by_value, reason = "the entry owns its bucket array")]
pub fn raw_histogram_entry(count: u64, min: f64, max: f64, buckets: Value) -> Value {
    json!({
        "values": [],
        "histogram": { "count": count, "min": min, "max": max, "buckets": buckets },
        "failures": [], "absent": null
    })
}

/// A one-arm record whose only metric `m` is a declared series holding `entry` in one replicate:
/// `run_started` (line 1), canary (2), the replicate (3), canary (4), `run_finished` (5).
#[allow(clippy::needless_pass_by_value, reason = "the replicate line owns its metric entry")]
pub fn one_arm_series_record(entry: Value) -> Lines {
    let mut started = run_started(&["lifecycle"]);
    started["declaration"] = declaration(&["lifecycle"], &[metric_decl("m", "ns", "t", true)]);
    vec![
        started,
        canary("start", 1000.0),
        replicate(0, 0, "lifecycle", "arm", json!({ "m": entry })),
        canary("end", 1000.0),
        finished(1, 0),
    ]
}

/// The metrics object of a replicate recording one value of the scalar metric `m`.
pub fn scalar(value: f64) -> Value {
    json!({ "m": values_entry(&[value]) })
}

#[allow(clippy::needless_pass_by_value, reason = "the line owns its metrics")]
pub fn replicate(block: usize, position: usize, arm: &str, role: &str, metrics: Value) -> Value {
    json!({
        "t": "replicate", "profile": "default", "block": block, "position": position,
        "arm": arm, "role": role, "outcome": "ok", "failure": null,
        "metrics": metrics, "notes": {}, "commands": []
    })
}

pub fn finished(ok: usize, failed: usize) -> Value {
    json!({ "t": "run_finished", "finished": "2026-11-17T10:05:00Z",
            "replicates_ok": ok, "replicates_failed": failed })
}

/// Recompute the last line's counts from the `replicate` lines, as the writer does.
pub fn recount(lines: &mut Lines) {
    let ok = lines.iter().filter(|l| l["t"] == "replicate" && l["outcome"] == "ok").count();
    let failed = lines.iter().filter(|l| l["t"] == "replicate" && l["outcome"] == "failed").count();
    let last = lines.len() - 1;
    lines[last]["replicates_ok"] = json!(ok);
    lines[last]["replicates_failed"] = json!(failed);
}

/// Mark the replicate at `index` failed with `reason` and the partial `metrics`, and recount.
pub fn fail_replicate(lines: &mut Lines, index: usize, reason: &str, metrics: Value) {
    lines[index]["outcome"] = json!("failed");
    lines[index]["failure"] = json!(reason);
    lines[index]["metrics"] = metrics;
    recount(lines);
}

/// A complete two-arm record (`ref` is the reference arm): per-replicate values of
/// `m` for the reference arm, the alternative arm and the A/A control, one block each.
pub fn two_arm(reference: &[f64], alt: &[f64], control: &[f64]) -> Lines {
    let mut lines = vec![run_started(&["ref", "alt"]), canary("start", 1000.0)];
    for (block, ((r, a), c)) in reference.iter().zip(alt).zip(control).enumerate() {
        lines.push(replicate(block, 0, "ref", "arm", scalar(*r)));
        lines.push(replicate(block, 1, "alt", "arm", scalar(*a)));
        lines.push(replicate(block, 2, "ref", "control", scalar(*c)));
    }
    lines.push(canary("end", 1000.0));
    let n = reference.len() * 3;
    lines.push(finished(n, 0));
    lines
}

/// Nine tightly spread replicates: 1000..=1008.
pub fn tight() -> Vec<f64> {
    (0..9).map(|i| 1000.0 + f64::from(i)).collect()
}

pub fn shifted(values: &[f64], by: f64) -> Vec<f64> {
    values.iter().map(|v| v + by).collect()
}

/// Replace the `metrics` of line `index`.
pub fn set_metrics(lines: &mut Lines, index: usize, metrics: Value) {
    lines[index]["metrics"] = metrics;
}

/// Set `field` on line `index`.
pub fn set(lines: &mut Lines, index: usize, field: &str, value: Value) {
    lines[index][field] = value;
}

/// Write the record under `dir` and return its path.
pub fn write(dir: &Path, name: &str, lines: &Lines) -> PathBuf {
    let path = dir.join(name);
    let body = lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(&path, body).expect("write record");
    path
}

pub fn write_raw(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write record");
    path
}

/// Run the report tool.
pub fn bench_report(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bench-report")).args(args).output().expect("run bench-report")
}

pub fn render(path: &Path) -> String {
    let output = bench_report(&["render", path.to_str().expect("utf-8 path")]);
    assert!(
        output.status.success(),
        "the report must render for a valid record; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Validate; returns (success, stderr, exit code).
pub fn validate(path: &Path) -> (bool, String, Option<i32>) {
    let output = bench_report(&["validate", path.to_str().expect("utf-8 path")]);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Validate; returns the whole output (for the "prints nothing on success" contract).
pub fn validate_output(path: &Path) -> Output {
    bench_report(&["validate", path.to_str().expect("utf-8 path")])
}

/// The cells of every table row of `text`, trimmed (separator rows dropped).
pub fn table_rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .map(|l| {
            l.trim().trim_matches('|').split('|').map(|c| c.trim().to_owned()).collect::<Vec<_>>()
        })
        .filter(|cells| !cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':')))
        .collect()
}

/// The scalar-table row (`metric | arm | median | IQR | replicates | failures`) for `metric` and `arm`.
pub fn scalar_row(text: &str, metric: &str, arm: &str) -> Vec<String> {
    table_rows(text)
        .into_iter()
        .find(|c| c.len() == 6 && c[0] == metric && c[1] == arm)
        .unwrap_or_else(|| {
            panic!("the report must have a scalar row for `{metric}` / `{arm}`:\n{text}")
        })
}

/// The series-table row (`metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures`).
pub fn series_row(text: &str, metric: &str, arm: &str) -> Vec<String> {
    table_rows(text)
        .into_iter()
        .find(|c| c.len() == 9 && c[0] == metric && c[1] == arm)
        .unwrap_or_else(|| {
            panic!("the report must have a series row for `{metric}` / `{arm}`:\n{text}")
        })
}

/// The comparison-table row for a non-reference `arm` and `metric` (eight columns).
pub fn comparison_row(text: &str, metric: &str, arm: &str) -> Vec<String> {
    table_rows(text)
        .into_iter()
        .find(|c| c.len() == 8 && c[0] == metric && c[1] == arm)
        .unwrap_or_else(|| {
            panic!("the report must have a comparison row for `{metric}` / `{arm}`:\n{text}")
        })
}

/// Whether the comparison table has a row for a non-reference `arm` and `metric`.
pub fn has_comparison_row(text: &str, metric: &str, arm: &str) -> bool {
    table_rows(text).into_iter().any(|c| c.len() == 8 && c[0] == metric && c[1] == arm)
}

/// The value of the header bullet `- <key>: <value>`.
pub fn header(report: &str, key: &str) -> String {
    let prefix = format!("- {key}: ");
    report
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("the report header must have the line `- {key}: ...`:\n{report}"))
        .to_owned()
}

/// The text of the report section whose heading names `label` (to the next heading).
pub fn section<'a>(report: &'a str, label: &str) -> &'a str {
    let mut offset = 0usize;
    let mut found = None;
    for line in report.lines() {
        if line.starts_with('#') && line.contains(label) {
            found = Some(offset);
            break;
        }
        offset += line.len() + 1;
    }
    let Some(start) = found else {
        panic!("the report must have a section for `{label}`:\n{report}")
    };
    let rest = &report[start..];
    let end = rest[1..].find("\n#").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}
