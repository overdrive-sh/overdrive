//! The retained record: schema version 1, one JSON object per line (ADR-0301, DDD-6).
//!
//! The typed schema is the one source of the writer's output and of the validator's parse:
//! unknown fields are refused, no required field is ever omitted to mean `null`, and the
//! closed vocabularies are enums. [`validate`] adds the semantic rules in line order.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::histogram::{Histogram, NotStorable, RAW_VALUE_LIMIT, storable};
use crate::{Error, Result};

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const METHODOLOGY_VERSION: u32 = 1;

pub(crate) const HOST_KEYS: [&str; 9] = [
    "kernel",
    "cpu_model",
    "cpu_count",
    "governor",
    "smt",
    "boost",
    "bpf_stats_enabled",
    "clocksource",
    "irq_time_accounting",
];

/// Check a whole record: the typed schema of every line, then the semantic rules and the
/// line order. A violation names its 1-based line and the offending field or name.
pub fn validate(text: &str) -> Result<()> {
    parse(text).map(drop)
}

/// The lines of a record that passed [`validate`], in order.
pub(crate) fn parse(text: &str) -> Result<Vec<Line>> {
    let mut checker = Checker::Fresh;
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line: Line =
            serde_json::from_str(raw).map_err(|source| invalid(number, parse_reason(&source)))?;
        checker.accept(&line).map_err(|reason| invalid(number, reason))?;
        lines.push(line);
    }
    if lines.is_empty() {
        return Err(invalid(1, "the record is empty; it must begin with run_started".to_owned()));
    }
    Ok(lines)
}

const fn invalid(line: usize, reason: String) -> Error {
    Error::InvalidRecord { line, reason }
}

/// `serde_json` appends ` at line L column C`, which names a position in the single parsed
/// line; the record's own line number is the one that matters.
fn parse_reason(error: &serde_json::Error) -> String {
    let message = error.to_string();
    match message.rsplit_once(" at line ") {
        Some((reason, _)) => reason.to_owned(),
        None => message,
    }
}

fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

// ---------------------------------------------------------------------------------------
// The typed schema.

/// One line of the record; field `t` is the line type.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum Line {
    RunStarted(Box<RunStarted>),
    Canary(Canary),
    Replicate(Box<ReplicateLine>),
    RunFinished(RunFinished),
    RunRefused(RunRefused),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceClass {
    Maintained,
    Spike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Substrate {
    Native,
}

impl Substrate {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BpfStatsMode {
    PerProcess,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ClockSource {
    #[serde(rename = "CLOCK_MONOTONIC_RAW")]
    MonotonicRaw,
}

impl ClockSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MonotonicRaw => "CLOCK_MONOTONIC_RAW",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Clock {
    pub(crate) host: ClockSource,
    #[serde(deserialize_with = "nullable")]
    pub(crate) service: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) guest: Option<String>,
}

/// One value of the `host` block: a string, an absent source, or an unreadable one.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum HostValue {
    Text(String),
    Absent { absent: String },
    Unreadable { unreadable: String },
}

impl HostValue {
    fn from_json(value: &serde_json::Value) -> std::result::Result<Self, &'static str> {
        const FORMS: &str =
            "must be a string, {\"absent\": <string>} or {\"unreadable\": <string>}";
        match value {
            serde_json::Value::String(text) => Ok(Self::Text(text.clone())),
            serde_json::Value::Object(map) if map.len() == 1 => {
                match (map.get("absent"), map.get("unreadable")) {
                    (Some(serde_json::Value::String(reason)), None) => {
                        Ok(Self::Absent { absent: reason.clone() })
                    }
                    (None, Some(serde_json::Value::String(error))) => {
                        Ok(Self::Unreadable { unreadable: error.clone() })
                    }
                    _ => Err(FORMS),
                }
            }
            _ => Err(FORMS),
        }
    }
}

/// The `host` block: exactly the nine pinned keys, each in one of the three forms.
#[derive(Debug, Serialize)]
pub(crate) struct Host(pub(crate) BTreeMap<String, HostValue>);

impl<'de> Deserialize<'de> for Host {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        if let Some(key) = raw.keys().find(|key| !HOST_KEYS.contains(&key.as_str())) {
            return Err(D::Error::custom(format!("host has unknown key `{key}`")));
        }
        let mut entries = BTreeMap::new();
        for key in HOST_KEYS {
            let value = raw
                .get(key)
                .ok_or_else(|| D::Error::custom(format!("host is missing key `{key}`")))?;
            let parsed = HostValue::from_json(value)
                .map_err(|form| D::Error::custom(format!("host key `{key}` {form}")))?;
            entries.insert(key.to_owned(), parsed);
        }
        Ok(Self(entries))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unit {
    Ns,
    Bytes,
    BytesPerSecond,
    Qps,
    Ratio,
    Count,
}

impl Unit {
    /// The unit a declaration names (`ns bytes bytes_per_second qps ratio count`).
    pub(crate) fn parse(name: &str) -> Option<Self> {
        [Self::Ns, Self::Bytes, Self::BytesPerSecond, Self::Qps, Self::Ratio, Self::Count]
            .into_iter()
            .find(|unit| unit.as_str() == name)
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Ns => "ns",
            Self::Bytes => "bytes",
            Self::BytesPerSecond => "bytes_per_second",
            Self::Qps => "qps",
            Self::Ratio => "ratio",
            Self::Count => "count",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileDecl {
    pub(crate) name: String,
    pub(crate) preparation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricDecl {
    pub(crate) name: String,
    pub(crate) unit: Unit,
    pub(crate) table: String,
    pub(crate) series: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceDecl {
    pub(crate) metric: String,
    pub(crate) value: f64,
    pub(crate) label: String,
    pub(crate) source: String,
}

/// The echo of the benchmark attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub(crate) arms: Vec<String>,
    pub(crate) replicates: u32,
    pub(crate) profiles: Vec<ProfileDecl>,
    pub(crate) metrics: Vec<MetricDecl>,
    pub(crate) references: Vec<ReferenceDecl>,
}

impl Declaration {
    fn reference_arm(&self) -> Option<&str> {
        self.arms.first().map(String::as_str)
    }

    fn declares_profile(&self, name: &str) -> bool {
        self.profiles.iter().any(|profile| profile.name == name)
    }

    fn metric(&self, name: &str) -> Option<&MetricDecl> {
        self.metrics.iter().find(|metric| metric.name == name)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunStarted {
    pub(crate) schema: u32,
    pub(crate) methodology: u32,
    pub(crate) run_id: String,
    pub(crate) family: String,
    pub(crate) benchmark: String,
    pub(crate) evidence: EvidenceClass,
    pub(crate) substrate: Substrate,
    pub(crate) commit: String,
    pub(crate) dirty: bool,
    pub(crate) debug_assertions: bool,
    pub(crate) seed: u64,
    pub(crate) started: String,
    pub(crate) host: Host,
    pub(crate) clock: Clock,
    pub(crate) bpf_stats: BpfStatsMode,
    pub(crate) declaration: Declaration,
}

impl RunStarted {
    fn check(&self) -> std::result::Result<(), String> {
        if self.schema != SCHEMA_VERSION {
            return Err(format!("schema must be {SCHEMA_VERSION}, found {}", self.schema));
        }
        if self.methodology != METHODOLOGY_VERSION {
            return Err(format!(
                "methodology must be {METHODOLOGY_VERSION}, found {}",
                self.methodology
            ));
        }
        if self.declaration.arms.is_empty() {
            return Err("declaration.arms must name at least the reference arm".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanaryPhase {
    Start,
    End,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Canary {
    pub(crate) phase: CanaryPhase,
    pub(crate) samples_ns: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Arm,
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Ok,
    Failed,
}

/// One metric of one replicate: exactly one of raw values, a histogram, failures or an absence.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricEntry {
    pub(crate) values: Vec<f64>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) histogram: Option<Histogram>,
    pub(crate) failures: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) absent: Option<String>,
}

impl MetricEntry {
    /// A series metric's stored form: raw values up to 1,000, a histogram of all of them above.
    /// Every value is kept; the switch is a representation choice, not a cap (DDD-5).
    pub(crate) fn from_series(values: &[f64]) -> std::result::Result<Self, NotStorable> {
        if values.len() > RAW_VALUE_LIMIT {
            return Ok(Self {
                values: Vec::new(),
                histogram: Some(Histogram::from_values(values)?),
                failures: Vec::new(),
                absent: None,
            });
        }
        if values.is_empty() {
            return Err(NotStorable::Empty);
        }
        let raw =
            values.iter().map(|value| storable(*value)).collect::<std::result::Result<_, _>>()?;
        Ok(Self { values: raw, histogram: None, failures: Vec::new(), absent: None })
    }

    /// A scalar metric's one value.
    pub(crate) fn scalar(value: f64) -> Self {
        Self { values: vec![value], histogram: None, failures: Vec::new(), absent: None }
    }

    /// A metric that could not be measured, once for each of `reasons`.
    pub(crate) const fn failures(reasons: Vec<String>) -> Self {
        Self { values: Vec::new(), histogram: None, failures: reasons, absent: None }
    }

    /// A metric that failed with the single `reason`.
    pub(crate) fn failed(reason: &str) -> Self {
        Self::failures(vec![reason.to_owned()])
    }

    /// A metric that does not apply to the replicate.
    pub(crate) fn absent(reason: &str) -> Self {
        Self { values: Vec::new(), histogram: None, failures: Vec::new(), absent: Some(reason.to_owned()) }
    }

    fn shape_error(&self, series: bool, exactly_one: bool) -> Option<String> {
        if let Some(histogram) = &self.histogram {
            if !series {
                return Some("a scalar metric never holds a histogram".to_owned());
            }
            if !self.values.is_empty() {
                return Some("a histogram's values must be empty".to_owned());
            }
            if let Some(error) = histogram.shape_error() {
                return Some(error);
            }
        }
        if self.values.len() > RAW_VALUE_LIMIT {
            return Some(format!("values holds more than {RAW_VALUE_LIMIT} values"));
        }
        if !series && self.values.len() > 1 {
            return Some("a scalar metric holds at most one value".to_owned());
        }
        let held = [
            !self.values.is_empty(),
            self.histogram.is_some(),
            !self.failures.is_empty(),
            self.absent.is_some(),
        ]
        .into_iter()
        .filter(|held| *held)
        .count();
        (exactly_one && held != 1).then(|| {
            "an entry must hold exactly one of values, histogram, failures or absent".to_owned()
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub(crate) status: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplicateLine {
    pub(crate) profile: String,
    pub(crate) arm: String,
    pub(crate) block: u32,
    pub(crate) position: u32,
    pub(crate) role: Role,
    pub(crate) outcome: Outcome,
    #[serde(deserialize_with = "nullable")]
    pub(crate) failure: Option<String>,
    pub(crate) metrics: BTreeMap<String, MetricEntry>,
    pub(crate) notes: BTreeMap<String, String>,
    pub(crate) commands: Vec<Command>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunFinished {
    pub(crate) finished: String,
    pub(crate) replicates_ok: u64,
    pub(crate) replicates_failed: u64,
}

/// The terminal line of a refused run: a difference (`found` and `pinned`) or a `reason`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRefused {
    pub(crate) setting: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) found: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pinned: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

impl RunRefused {
    fn check(&self) -> std::result::Result<(), String> {
        let difference = self.found.is_some() && self.pinned.is_some() && self.reason.is_none();
        let reason_only = self.found.is_none() && self.pinned.is_none() && self.reason.is_some();
        if difference || reason_only {
            return Ok(());
        }
        Err("run_refused must carry both found and pinned, or a reason, and nothing else"
            .to_owned())
    }
}

// ---------------------------------------------------------------------------------------
// The writer.

/// Append-only writer of a record: one JSON object per line, flushed per line.
pub(crate) struct RecordWriter<W: Write> {
    out: W,
    path: PathBuf,
}

impl<W: Write> RecordWriter<W> {
    pub(crate) const fn new(out: W, path: PathBuf) -> Self {
        Self { out, path }
    }

    /// Write `line` and flush it, so a killed run leaves every completed line on disk.
    pub(crate) fn append(&mut self, line: &Line) -> Result<()> {
        let mut text = serde_json::to_string(line).map_err(|source| self.fail(source.into()))?;
        text.push('\n');
        self.out
            .write_all(text.as_bytes())
            .and_then(|()| self.out.flush())
            .map_err(|source| self.fail(source))
    }

    fn fail(&self, source: std::io::Error) -> Error {
        Error::Record { path: self.path.clone(), source }
    }
}

// ---------------------------------------------------------------------------------------
// The semantic rules, in line order.

/// Where the record stands: before `run_started`, inside the run, or past its terminal line.
enum Checker {
    Fresh,
    Open(Open),
    Closed,
}

enum Stage {
    Started,
    Measuring,
    Measured,
}

enum Step {
    Continue,
    Terminal,
}

struct Open {
    declaration: Declaration,
    stage: Stage,
    ok: u64,
    failed: u64,
}

impl Checker {
    fn accept(&mut self, line: &Line) -> std::result::Result<(), String> {
        match self {
            Self::Closed => Err("no line may follow the terminal line".to_owned()),
            Self::Fresh => {
                let Line::RunStarted(started) = line else {
                    return Err("the record must begin with run_started".to_owned());
                };
                started.check()?;
                *self = Self::Open(Open {
                    declaration: started.declaration.clone(),
                    stage: Stage::Started,
                    ok: 0,
                    failed: 0,
                });
                Ok(())
            }
            Self::Open(open) => {
                if matches!(open.accept(line)?, Step::Terminal) {
                    *self = Self::Closed;
                }
                Ok(())
            }
        }
    }
}

impl Open {
    fn accept(&mut self, line: &Line) -> std::result::Result<Step, String> {
        match (&self.stage, line) {
            (_, Line::RunStarted(_)) => Err("run_started may only be the first line".to_owned()),
            (Stage::Started, Line::Canary(canary)) if canary.phase == CanaryPhase::Start => {
                self.stage = Stage::Measuring;
                Ok(Step::Continue)
            }
            (Stage::Started, Line::RunRefused(refused)) => {
                refused.check()?;
                Ok(Step::Terminal)
            }
            (Stage::Started, _) => {
                Err("run_started must be followed by the start canary or run_refused".to_owned())
            }
            (Stage::Measuring, Line::Replicate(replicate)) => {
                self.replicate(replicate)?;
                Ok(Step::Continue)
            }
            (Stage::Measuring, Line::Canary(canary)) if canary.phase == CanaryPhase::End => {
                self.stage = Stage::Measured;
                Ok(Step::Continue)
            }
            (Stage::Measuring, _) => Err("expected a replicate or the end canary".to_owned()),
            (Stage::Measured, Line::RunFinished(finished)) => {
                self.finish(finished)?;
                Ok(Step::Terminal)
            }
            (Stage::Measured, _) => Err("expected run_finished after the end canary".to_owned()),
        }
    }

    fn replicate(&mut self, replicate: &ReplicateLine) -> std::result::Result<(), String> {
        let declaration = &self.declaration;
        if !declaration.declares_profile(&replicate.profile) {
            return Err(format!("replicate names undeclared profile `{}`", replicate.profile));
        }
        if !declaration.arms.contains(&replicate.arm) {
            return Err(format!("replicate names undeclared arm `{}`", replicate.arm));
        }
        let reference = declaration.reference_arm().unwrap_or_default();
        if replicate.role == Role::Control && replicate.arm != reference {
            return Err(format!(
                "a control must carry the reference arm `{reference}` in arm, found `{}`",
                replicate.arm
            ));
        }
        match (replicate.outcome, &replicate.failure) {
            (Outcome::Failed, None) => {
                return Err("a failed replicate must carry a failure reason".to_owned());
            }
            (Outcome::Ok, Some(_)) => {
                return Err("an ok replicate must not carry a failure reason".to_owned());
            }
            _ => {}
        }
        for (name, entry) in &replicate.metrics {
            let Some(metric) = declaration.metric(name) else {
                return Err(format!("replicate names undeclared metric `{name}`"));
            };
            if let Some(error) = entry.shape_error(metric.series, replicate.outcome == Outcome::Ok)
            {
                return Err(format!("metric `{name}`: {error}"));
            }
        }
        match replicate.outcome {
            Outcome::Ok => self.ok += 1,
            Outcome::Failed => self.failed += 1,
        }
        Ok(())
    }

    fn finish(&self, finished: &RunFinished) -> std::result::Result<(), String> {
        if finished.replicates_ok != self.ok {
            return Err(format!(
                "replicates_ok is {} but the record holds {} ok replicates",
                finished.replicates_ok, self.ok
            ));
        }
        if finished.replicates_failed != self.failed {
            return Err(format!(
                "replicates_failed is {} but the record holds {} failed replicates",
                finished.replicates_failed, self.failed
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;

    struct Refusing;
    impl Write for Refusing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn host() -> Value {
        let keys: BTreeMap<&str, &str> = HOST_KEYS.iter().map(|key| (*key, "x")).collect();
        json!(keys)
    }

    fn started(series: bool) -> Value {
        json!({
            "t": "run_started", "schema": 1, "methodology": 1,
            "run_id": "0123456789abcdef01234567", "family": "f", "benchmark": "b",
            "evidence": "maintained", "substrate": "native",
            "commit": "e3b42c92e0123456789abcdef0123456789abcde", "dirty": false,
            "debug_assertions": false, "seed": 1, "started": "2026-11-17T10:00:00Z",
            "host": host(),
            "clock": { "host": "CLOCK_MONOTONIC_RAW", "service": null, "guest": null },
            "bpf_stats": "off",
            "declaration": {
                "arms": ["arm"], "replicates": 9,
                "profiles": [{ "name": "default", "preparation": "" }],
                "metrics": [{ "name": "m", "unit": "ns", "table": "t", "series": series }],
                "references": []
            }
        })
    }

    fn canary(phase: &str) -> Value {
        json!({ "t": "canary", "phase": phase, "samples_ns": [1.0] })
    }

    fn replicate_of(entry: &MetricEntry) -> Value {
        json!({
            "t": "replicate", "profile": "default", "arm": "arm", "block": 0, "position": 0,
            "role": "arm", "outcome": "ok", "failure": null,
            "metrics": { "m": entry }, "notes": {}, "commands": []
        })
    }

    fn finished() -> Value {
        json!({ "t": "run_finished", "finished": "2026-11-17T10:05:00Z",
                "replicates_ok": 1, "replicates_failed": 0 })
    }

    fn record(lines: &[Value]) -> String {
        lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n"
    }

    fn series_record(entry: &MetricEntry) -> String {
        record(&[started(true), canary("start"), replicate_of(entry), canary("end"), finished()])
    }

    #[test]
    fn a_series_of_exactly_1000_values_is_stored_raw_and_1001_as_a_histogram() {
        let thousand: Vec<f64> = (1..=1000).map(f64::from).collect();
        let raw = MetricEntry::from_series(&thousand).expect("storable");
        assert_eq!(raw.values.len(), 1000);
        assert!(raw.histogram.is_none());
        let over: Vec<f64> = (1..=1001).map(f64::from).collect();
        let binned = MetricEntry::from_series(&over).expect("storable");
        assert!(binned.values.is_empty());
        assert_eq!(binned.histogram.expect("histogram").count, 1001);
    }

    #[test]
    fn a_series_with_a_refused_value_or_no_value_is_not_storable() {
        assert_eq!(MetricEntry::from_series(&[]), Err(NotStorable::Empty));
        assert!(matches!(MetricEntry::from_series(&[1.0, -2.0]), Err(NotStorable::Value(_))));
        let mut over: Vec<f64> = vec![1.0; 1001];
        over[500] = f64::NAN;
        assert!(matches!(MetricEntry::from_series(&over), Err(NotStorable::Value(_))));
    }

    #[test]
    fn the_writer_appends_one_flushed_line_and_names_the_path_when_it_fails() {
        let line: Line = serde_json::from_value(canary("start")).expect("line");
        let mut written = RecordWriter::new(Vec::new(), PathBuf::from("record.jsonl"));
        written.append(&line).expect("append");
        written.append(&line).expect("append");
        let text = String::from_utf8(written.out).expect("utf-8");
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with("}\n"));

        let mut failing = RecordWriter::new(Refusing, PathBuf::from("record.jsonl"));
        let error = failing.append(&line).expect_err("the write fails");
        assert!(
            matches!(error, Error::Record { ref path, .. } if path == &PathBuf::from("record.jsonl"))
        );
    }

    #[test]
    fn a_nullable_field_may_not_be_omitted() {
        let mut line = replicate_of(&MetricEntry::from_series(&[1.0]).expect("entry"));
        line.as_object_mut().expect("object").remove("failure");
        let text = record(&[started(true), canary("start"), line]);
        let error = validate(&text).expect_err("failure is required");
        assert!(
            matches!(error, Error::InvalidRecord { line: 3, ref reason } if reason.contains("failure"))
        );
    }

    #[test]
    fn a_host_value_object_carries_exactly_one_key() {
        let mut line = started(true);
        line["host"]["smt"] = json!({ "absent": "no source", "extra": "x" });
        assert!(validate(&record(&[line])).is_err());
    }

    #[test]
    fn a_killed_run_is_a_valid_prefix() {
        for lines in [
            vec![started(true)],
            vec![started(true), canary("start")],
            vec![started(true), canary("start"), canary("end")],
        ] {
            assert!(validate(&record(&lines)).is_ok());
        }
    }

    #[test]
    fn the_line_order_is_enforced() {
        let entry = MetricEntry::from_series(&[1.0]).expect("entry");
        let cases = [
            (vec![started(true), replicate_of(&entry)], 2),
            (vec![started(true), canary("end")], 2),
            (vec![started(true), canary("start"), canary("start")], 3),
            (vec![started(true), canary("start"), finished()], 3),
            (vec![started(true), canary("start"), canary("end"), replicate_of(&entry)], 4),
            (vec![started(true), started(true)], 2),
            (
                vec![
                    started(true),
                    canary("start"),
                    json!({"t":"run_refused","setting":"s","reason":"r"}),
                ],
                3,
            ),
        ];
        for (lines, at) in cases {
            let error = validate(&record(&lines)).expect_err("out of order");
            assert!(
                matches!(error, Error::InvalidRecord { line, .. } if line == at),
                "{lines:?}: {error}"
            );
        }
    }

    #[test]
    fn a_refusal_carries_a_difference_or_a_reason_and_nothing_else() {
        for (refusal, valid) in [
            (json!({"t":"run_refused","setting":"s","found":"a","pinned":"b"}), true),
            (json!({"t":"run_refused","setting":"s","reason":"r"}), true),
            (json!({"t":"run_refused","setting":"s"}), false),
            (json!({"t":"run_refused","setting":"s","found":"a"}), false),
            (json!({"t":"run_refused","setting":"s","pinned":"b"}), false),
            (json!({"t":"run_refused","setting":"s","reason":"r","found":"a","pinned":"b"}), false),
            (json!({"t":"run_refused","setting":"s","reason":"r","found":"a"}), false),
        ] {
            let result = validate(&record(&[started(true), refusal.clone()]));
            assert_eq!(result.is_ok(), valid, "{refusal}");
        }
    }

    #[test]
    fn a_metric_entry_in_a_failed_replicate_may_be_partial() {
        let mut line = replicate_of(&MetricEntry::from_series(&[1.0]).expect("entry"));
        line["outcome"] = json!("failed");
        line["failure"] = json!("fixture broke");
        line["metrics"] =
            json!({ "m": { "values": [], "histogram": null, "failures": [], "absent": null } });
        let failed = json!({ "t": "run_finished", "finished": "x",
                             "replicates_ok": 0, "replicates_failed": 1 });
        let text = record(&[started(true), canary("start"), line, canary("end"), failed]);
        assert!(validate(&text).is_ok());
    }

    proptest! {
        #[test]
        fn what_the_writer_stores_validates_at_any_series_length(len in 1usize..2200) {
            let values: Vec<f64> = (0..len).map(|i| (i % 9000) as f64 * 1.37).collect();
            let entry = MetricEntry::from_series(&values).expect("storable");
            prop_assert_eq!(entry.histogram.is_some(), len > RAW_VALUE_LIMIT);
            prop_assert_eq!(entry.values.len(), if len > RAW_VALUE_LIMIT { 0 } else { len });
            prop_assert!(validate(&series_record(&entry)).is_ok());
        }

        #[test]
        fn a_stored_entry_survives_a_write_and_read(
            quarters in proptest::collection::vec(0u32..4_000_000, 1..1300),
        ) {
            // Quarter-integers are exact in binary and in decimal, so the round trip is exact.
            let values: Vec<f64> = quarters.iter().map(|q| f64::from(*q) / 4.0).collect();
            let entry = MetricEntry::from_series(&values).expect("storable");
            let text = serde_json::to_string(&entry).expect("serialise");
            let back: MetricEntry = serde_json::from_str(&text).expect("parse");
            prop_assert_eq!(back, entry);
        }
    }
}
