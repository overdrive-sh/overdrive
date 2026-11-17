//! The replicate handle a benchmark body drives, its failure type, the host clock and the
//! smoke variant (DDD-2, DDD-6, DDD-10).
//!
//! A [`Replicate`] collects what one replicate records: metric entries, notes and the command
//! log. The smoke variant runs one replicate per arm with no statistics descriptor and no
//! timing assertion, so a broken fixture or a mistyped metric name fails a plain test run.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::registry::BenchmarkDecl;
use crate::histogram::{NotStorable, storable};
use crate::record::{Command as LoggedCommand, MetricEntry};
use crate::{BpfStatsHold, Result};

/// The profile name of a benchmark that declares none.
const IMPLICIT_PROFILE: &str = "default";

/// A failed replicate: the body's report that a fixture did not build or a measurement
/// could not be taken. The runner counts it and continues.
#[derive(Debug)]
pub struct ReplicateFailure {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Fixture(String),
    Measurement(String),
    UndeclaredMetric(String),
    StatsNotEffective(u32),
}

impl ReplicateFailure {
    /// The benchmark's fixture could not be built or driven.
    pub fn fixture(msg: &str) -> Self {
        Self { kind: Kind::Fixture(msg.to_owned()) }
    }

    /// The fixture ran but the measurement could not be taken.
    pub fn measurement(msg: &str) -> Self {
        Self { kind: Kind::Measurement(msg.to_owned()) }
    }

    fn undeclared(metric: &str) -> Self {
        Self { kind: Kind::UndeclaredMetric(metric.to_owned()) }
    }

    /// Statistics were requested but the program's `run_count` did not move (DDD-8).
    pub(crate) const fn stats_not_effective(program_id: u32) -> Self {
        Self { kind: Kind::StatsNotEffective(program_id) }
    }
}

impl std::fmt::Display for ReplicateFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            Kind::Fixture(reason) | Kind::Measurement(reason) => formatter.write_str(reason),
            Kind::UndeclaredMetric(metric) => {
                write!(formatter, "metric `{metric}` is not declared by the benchmark")
            }
            Kind::StatsNotEffective(program_id) => write!(
                formatter,
                "BPF statistics were requested but program {program_id} recorded no runs (run_count delta 0)"
            ),
        }
    }
}

impl std::error::Error for ReplicateFailure {}

/// What a replicate holds for one metric.
#[derive(Debug, PartialEq)]
enum Entry {
    Scalar(f64),
    Series(Vec<f64>),
    Failures(Vec<String>),
    Absent(String),
}

impl Entry {
    fn stored(&self) -> std::result::Result<MetricEntry, NotStorable> {
        match self {
            Self::Scalar(value) => storable(*value).map(MetricEntry::scalar),
            Self::Series(values) => MetricEntry::from_series(values),
            Self::Failures(reasons) => Ok(MetricEntry::failures(reasons.clone())),
            Self::Absent(reason) => Ok(MetricEntry::absent(reason)),
        }
    }
}

/// A replicate's recording in the record's form.
pub struct Finished {
    pub(crate) metrics: BTreeMap<String, MetricEntry>,
    pub(crate) notes: BTreeMap<String, String>,
    pub(crate) commands: Vec<LoggedCommand>,
    /// Why the recording cannot be stored whole, when it cannot.
    pub(crate) failure: Option<String>,
}

/// One `exec` of the replicate's command log.
#[derive(Debug, PartialEq)]
struct Executed {
    program: String,
    args: Vec<String>,
    /// `None` when the process was killed by a signal.
    status: Option<i32>,
}

/// A replicate's handle: the body records what it measures through it.
pub struct Replicate {
    arm: String,
    profile: String,
    /// Declared metric name to whether it is a `series` metric.
    declared: BTreeMap<&'static str, bool>,
    clock: HostClock,
    stats: Option<Arc<BpfStatsHold>>,
    entries: BTreeMap<String, Entry>,
    notes: BTreeMap<String, String>,
    commands: Vec<Executed>,
}

impl Replicate {
    pub(crate) fn new(
        decl: &BenchmarkDecl,
        arm: &str,
        profile: &str,
        stats: Option<Arc<BpfStatsHold>>,
    ) -> Self {
        Self {
            arm: arm.to_owned(),
            profile: profile.to_owned(),
            declared: decl.metrics.iter().map(|metric| (metric.name, metric.series)).collect(),
            clock: HostClock,
            stats,
            entries: BTreeMap::new(),
            notes: BTreeMap::new(),
            commands: Vec::new(),
        }
    }

    /// The arm this replicate runs; for the A/A control, the reference arm's name (the body
    /// cannot tell).
    pub fn arm(&self) -> &str {
        &self.arm
    }

    /// The profile this replicate runs.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The host clock the body takes its stamps from.
    pub const fn clock(&self) -> &HostClock {
        &self.clock
    }

    /// A scalar metric, once per replicate. A second `record` of the metric, or a `record`
    /// of a `series` metric, is a failure.
    pub fn record(&mut self, metric: &str, value: f64) -> Result<(), ReplicateFailure> {
        self.require_shape(metric, false)?;
        self.set_once(metric, Entry::Scalar(value))
    }

    /// A `series` metric. May be called more than once; the values accumulate. A scalar
    /// metric is a failure.
    pub fn record_series(&mut self, metric: &str, values: &[f64]) -> Result<(), ReplicateFailure> {
        self.require_shape(metric, true)?;
        match self.entries.get_mut(metric) {
            None => {
                self.entries.insert(metric.to_owned(), Entry::Series(values.to_vec()));
                Ok(())
            }
            Some(Entry::Series(held)) => {
                held.extend_from_slice(values);
                Ok(())
            }
            Some(_) => Err(already_holds(metric)),
        }
    }

    /// The metric could not be measured in this replicate, for `reason`.
    pub fn record_failure(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure> {
        self.require_declared(metric)?;
        match self.entries.get_mut(metric) {
            None => {
                self.entries.insert(metric.to_owned(), Entry::Failures(vec![reason.to_owned()]));
                Ok(())
            }
            Some(Entry::Failures(held)) => {
                held.push(reason.to_owned());
                Ok(())
            }
            Some(_) => Err(already_holds(metric)),
        }
    }

    /// The metric does not apply to this replicate, for `reason` (for example a statistics
    /// metric under the smoke variant).
    pub fn record_absent(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure> {
        self.require_declared(metric)?;
        self.set_once(metric, Entry::Absent(reason.to_owned()))
    }

    /// Header context of the replicate (cache state, versions, concurrency).
    pub fn note(&mut self, key: &str, value: &str) {
        self.notes.insert(key.to_owned(), value.to_owned());
    }

    /// Runs `program` with `args`, logging program, arguments and status, and returns its
    /// standard output. A program that cannot start or exits unsuccessfully is a fixture failure.
    pub fn exec(&mut self, program: &str, args: &[&str]) -> Result<String, ReplicateFailure> {
        let output = Command::new(program).args(args).output().map_err(|source| {
            ReplicateFailure::fixture(&format!("`{program}` could not start: {source}"))
        })?;
        let status = output.status.code();
        self.commands.push(Executed {
            program: program.to_owned(),
            args: args.iter().map(|argument| (*argument).to_owned()).collect(),
            status,
        });
        if !output.status.success() {
            return Err(ReplicateFailure::fixture(&format!(
                "`{program}` exited unsuccessfully ({})",
                status.map_or_else(
                    || "killed by a signal".to_owned(),
                    |code| format!("status {code}")
                )
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The BPF statistics hold: `Some` iff the benchmark declares `bpf_stats` and the runner
    /// holds the descriptor; `None` under the smoke variant.
    pub fn bpf_stats(&self) -> Option<&BpfStatsHold> {
        self.stats.as_deref()
    }

    /// What the replicate recorded, in the record's form. A declared metric never recorded is
    /// one failure `not recorded`. A value the record cannot hold (negative, NaN, infinite, or
    /// an empty series) fails the replicate with its reason and the metric is left out.
    pub(crate) fn finish(self) -> Finished {
        let mut failure = None;
        let mut metrics = BTreeMap::new();
        for name in self.declared.keys() {
            let stored = self
                .entries
                .get(*name)
                .map_or_else(|| Ok(MetricEntry::failed("not recorded")), Entry::stored);
            match stored {
                Ok(entry) => {
                    metrics.insert((*name).to_owned(), entry);
                }
                Err(reason) => {
                    failure.get_or_insert_with(|| format!("metric `{name}`: {reason}"));
                }
            }
        }
        let commands = self
            .commands
            .into_iter()
            .map(|executed| LoggedCommand {
                program: executed.program,
                args: executed.args,
                status: executed.status,
            })
            .collect();
        Finished { metrics, notes: self.notes, commands, failure }
    }

    fn require_declared(&self, metric: &str) -> Result<bool, ReplicateFailure> {
        self.declared.get(metric).copied().ok_or_else(|| ReplicateFailure::undeclared(metric))
    }

    fn require_shape(&self, metric: &str, series: bool) -> Result<(), ReplicateFailure> {
        if self.require_declared(metric)? == series {
            return Ok(());
        }
        let (declared, used) =
            if series { ("scalar", "record_series") } else { ("series", "record") };
        Err(ReplicateFailure::measurement(&format!(
            "metric `{metric}` is a {declared} metric; `{used}` records the other kind"
        )))
    }

    fn set_once(&mut self, metric: &str, entry: Entry) -> Result<(), ReplicateFailure> {
        if self.entries.contains_key(metric) {
            return Err(already_holds(metric));
        }
        self.entries.insert(metric.to_owned(), entry);
        Ok(())
    }
}

fn already_holds(metric: &str) -> ReplicateFailure {
    ReplicateFailure::measurement(&format!(
        "metric `{metric}` already holds a value in this replicate"
    ))
}

/// The host monotonic clock (`CLOCK_MONOTONIC_RAW`, DDD-10).
#[derive(Debug)]
pub struct HostClock;

impl HostClock {
    /// The current host instant.
    #[allow(clippy::unused_self, reason = "DDD-6 pins `now` as a method of the clock handle")]
    pub fn now(&self) -> HostInstant {
        let mut stamp = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `stamp` is a valid, exclusively borrowed `timespec` for the call's duration.
        let status = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &raw mut stamp) };
        if status != 0 {
            unreachable!("CLOCK_MONOTONIC_RAW is supported by every Linux kernel Overdrive runs");
        }
        let seconds = u64::try_from(stamp.tv_sec).unwrap_or_default();
        let nanoseconds = u32::try_from(stamp.tv_nsec).unwrap_or_default();
        HostInstant(Duration::new(seconds, nanoseconds))
    }
}

/// An instant of the host monotonic clock.
#[derive(Clone, Copy, Debug)]
pub struct HostInstant(Duration);

impl HostInstant {
    /// The time from `earlier` to this instant; zero if `earlier` is later.
    pub const fn duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// The smoke variant (DDD-2): one replicate per arm and profile, no statistics descriptor held,
/// no timing asserted, nothing written. A failed replicate panics naming the benchmark.
pub fn smoke(decl: &BenchmarkDecl) {
    let implicit = [IMPLICIT_PROFILE];
    let profiles: Vec<&str> = if decl.profiles.is_empty() {
        implicit.to_vec()
    } else {
        decl.profiles.iter().map(|profile| profile.name).collect()
    };
    for profile in profiles {
        for arm in decl.arms {
            let mut replicate = Replicate::new(decl, arm, profile, None);
            if let Err(failure) = (decl.run)(&mut replicate) {
                panic!(
                    "benchmark `{}` (family `{}`) failed its smoke run, profile `{profile}` arm `{arm}`: {failure}",
                    decl.benchmark, decl.family
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::MetricDecl;

    const METRICS: [MetricDecl; 2] = [
        MetricDecl { name: "scalar", unit: "ns", table: "t", series: false },
        MetricDecl { name: "stream", unit: "ns", table: "t", series: true },
    ];

    #[allow(clippy::unnecessary_wraps, reason = "the signature is the benchmark body's")]
    fn nothing(_: &mut Replicate) -> Result<(), ReplicateFailure> {
        Ok(())
    }

    fn decl(run: fn(&mut Replicate) -> Result<(), ReplicateFailure>) -> BenchmarkDecl {
        BenchmarkDecl {
            family: "unit-family",
            benchmark: "unit_bench",
            substrate: "native",
            arms: &["first", "second"],
            replicates: 9,
            profiles: &[],
            metrics: &METRICS,
            bpf_stats: true,
            references: &[],
            run,
        }
    }

    fn replicate() -> Replicate {
        Replicate::new(&decl(nothing), "first", "default", None)
    }

    #[test]
    fn a_scalar_is_recorded_once() {
        let mut replicate = replicate();
        assert!(replicate.record("scalar", 1.5).is_ok());
        assert!(replicate.record("scalar", 2.5).is_err());
        assert_eq!(replicate.entries["scalar"], Entry::Scalar(1.5));
    }

    #[test]
    fn a_series_accumulates_and_a_scalar_refuses_it() {
        let mut replicate = replicate();
        assert!(replicate.record_series("stream", &[1.0, 2.0]).is_ok());
        assert!(replicate.record_series("stream", &[3.0]).is_ok());
        assert_eq!(replicate.entries["stream"], Entry::Series(vec![1.0, 2.0, 3.0]));
        assert!(replicate.record_series("scalar", &[1.0]).is_err());
        assert!(replicate.record("stream", 1.0).is_err());
    }

    #[test]
    fn an_undeclared_metric_is_a_failure_for_every_record_call() {
        let mut replicate = replicate();
        assert!(replicate.record("typo", 1.0).is_err());
        assert!(replicate.record_series("typo", &[1.0]).is_err());
        assert!(replicate.record_failure("typo", "x").is_err());
        let refused = replicate.record_absent("typo", "x");
        assert!(matches!(refused, Err(failure) if failure.to_string().contains("`typo`")));
        assert!(replicate.entries.is_empty());
    }

    #[test]
    fn failures_accumulate_and_an_absent_metric_holds_its_reason() {
        let mut replicate = replicate();
        assert!(replicate.record_failure("scalar", "first").is_ok());
        assert!(replicate.record_failure("scalar", "second").is_ok());
        assert_eq!(
            replicate.entries["scalar"],
            Entry::Failures(vec!["first".to_owned(), "second".to_owned()])
        );
        assert!(replicate.record_absent("stream", "no statistics").is_ok());
        assert!(replicate.record_absent("stream", "again").is_err());
        assert!(replicate.record_series("stream", &[1.0]).is_err());
    }

    #[test]
    fn notes_and_the_command_log_are_kept() {
        let mut replicate = replicate();
        replicate.note("cache", "warm");
        assert_eq!(replicate.notes["cache"], "warm");
        assert!(replicate.exec("true", &[]).is_ok());
        assert!(replicate.exec("false", &[]).is_err());
        assert!(replicate.exec("/nonexistent/program", &[]).is_err());
        assert_eq!(
            replicate.commands,
            [
                Executed { program: "true".to_owned(), args: vec![], status: Some(0) },
                Executed { program: "false".to_owned(), args: vec![], status: Some(1) },
            ]
        );
        let echoed = replicate.exec("echo", &["hello"]);
        assert!(matches!(echoed, Ok(text) if text == "hello\n"));
    }

    #[test]
    fn the_host_clock_never_runs_backwards() {
        let clock = HostClock;
        let earlier = clock.now();
        let later = clock.now();
        assert!(later.duration_since(earlier) >= Duration::ZERO);
        assert_eq!(earlier.duration_since(later), Duration::ZERO);
    }

    #[test]
    fn smoke_runs_once_per_arm_and_holds_no_statistics() {
        fn body(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            if replicate.bpf_stats().is_some() {
                return Err(ReplicateFailure::measurement("statistics held"));
            }
            replicate.record("scalar", 1.0)
        }
        smoke(&decl(body));
    }

    #[test]
    #[should_panic(expected = "benchmark `unit_bench` (family `unit-family`) failed its smoke run")]
    fn smoke_names_the_benchmark_when_a_replicate_fails() {
        fn broken(_: &mut Replicate) -> Result<(), ReplicateFailure> {
            Err(ReplicateFailure::fixture("no dataplane"))
        }
        smoke(&decl(broken));
    }
}
