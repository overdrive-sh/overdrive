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

use crate::histogram::{NotStorable, storable};
use crate::record::{Command as LoggedCommand, GuestView, MetricEntry};
use crate::registry::BenchmarkDecl;
use crate::{BpfStatsHold, Result};

/// The profile name of a benchmark that declares none.
const IMPLICIT_PROFILE: &str = "default";

/// The substrate a benchmark declares to run in a kit guest.
pub const GUEST_SUBSTRATE: &str = "guest";

/// Why a guest-substrate replicate that returned `Ok` without a view is failed (M-1).
const NO_GUEST_VIEW: &str = "the replicate reported no guest view";

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
    /// A series metric some contributions of which were measured and others failed.
    PartlyMeasured {
        values: Vec<f64>,
        failures: Vec<String>,
    },
    Absent(String),
}

impl Entry {
    fn stored(&self) -> std::result::Result<MetricEntry, NotStorable> {
        match self {
            Self::Scalar(value) => storable(*value).map(MetricEntry::scalar),
            Self::Series(values) => MetricEntry::from_series(values),
            Self::Failures(reasons) => Ok(MetricEntry::failures(reasons.clone())),
            Self::PartlyMeasured { values, failures } => MetricEntry::from_series(values)
                .map(|measured| MetricEntry { failures: failures.clone(), ..measured }),
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
    /// The guest view the replicate reported (its first call), if any.
    pub(crate) view: Option<GuestView>,
    /// The console the replicate retained, if any.
    pub(crate) console: Option<String>,
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
    /// Whether the benchmark declares the `guest` substrate.
    guest: bool,
    view: Option<GuestView>,
    console: Option<String>,
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
            guest: decl.substrate == GUEST_SUBSTRATE,
            view: None,
            console: None,
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
        require_storable(metric, value)?;
        self.set_once(metric, Entry::Scalar(value))
    }

    /// A `series` metric. May be called more than once; the values accumulate, whether or not
    /// the metric already holds failures. A scalar metric is a failure.
    pub fn record_series(&mut self, metric: &str, values: &[f64]) -> Result<(), ReplicateFailure> {
        self.require_shape(metric, true)?;
        for value in values {
            require_storable(metric, *value)?;
        }
        match self.entries.get_mut(metric) {
            None => {
                self.entries.insert(metric.to_owned(), Entry::Series(values.to_vec()));
                Ok(())
            }
            Some(Entry::Series(held) | Entry::PartlyMeasured { values: held, .. }) => {
                held.extend_from_slice(values);
                Ok(())
            }
            Some(entry @ Entry::Failures(_)) => {
                let Entry::Failures(failures) = std::mem::replace(entry, Entry::Series(Vec::new()))
                else {
                    unreachable!("the arm matched a failures entry");
                };
                *entry = Entry::PartlyMeasured { values: values.to_vec(), failures };
                Ok(())
            }
            Some(_) => Err(already_holds(metric)),
        }
    }

    /// The metric could not be measured in this replicate, for `reason`. On a `series` metric
    /// the failure and the values recorded with `record_series` are kept together, in either
    /// order (a partly measured series); a scalar metric holding a value is a failure.
    pub fn record_failure(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure> {
        self.require_declared(metric)?;
        match self.entries.get_mut(metric) {
            None => {
                self.entries.insert(metric.to_owned(), Entry::Failures(vec![reason.to_owned()]));
                Ok(())
            }
            Some(Entry::Failures(held) | Entry::PartlyMeasured { failures: held, .. }) => {
                held.push(reason.to_owned());
                Ok(())
            }
            Some(entry @ Entry::Series(_)) => {
                let Entry::Series(values) = std::mem::replace(entry, Entry::Failures(Vec::new()))
                else {
                    unreachable!("the arm matched a series entry");
                };
                *entry = Entry::PartlyMeasured { values, failures: vec![reason.to_owned()] };
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

    /// Reports the guest view of this run (DDD-4). The first call of a run fixes the run's view;
    /// later calls return `Ok` and change nothing.
    ///
    /// # Errors
    /// A `ReplicateFailure` when the benchmark does not declare the `guest` substrate.
    pub fn report_guest_view(&mut self, view: GuestView) -> Result<(), ReplicateFailure> {
        if !self.guest {
            return Err(ReplicateFailure::fixture(
                "a guest view was reported by a benchmark that does not declare the guest substrate",
            ));
        }
        self.view.get_or_insert(view);
        Ok(())
    }

    /// Retains `text` as this replicate's console beside the record (DDD-9). The runner writes it
    /// create-new once the body returns, in every outcome.
    ///
    /// # Errors
    /// A `ReplicateFailure` on a second call by the same replicate.
    pub fn retain_console(&mut self, text: &str) -> Result<(), ReplicateFailure> {
        if self.console.is_some() {
            return Err(ReplicateFailure::fixture(
                "the replicate retained its console twice; a console is written once",
            ));
        }
        self.console = Some(text.to_owned());
        Ok(())
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
        if self.guest && self.view.is_none() {
            failure.get_or_insert_with(|| NO_GUEST_VIEW.to_owned());
        }
        Finished {
            metrics,
            notes: self.notes,
            commands,
            failure,
            view: self.view,
            console: self.console,
        }
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

/// A negative, NaN or infinite value is a measurement failure (DDD-5 item 1).
fn require_storable(metric: &str, value: f64) -> Result<(), ReplicateFailure> {
    storable(value)
        .map(drop)
        .map_err(|refused| ReplicateFailure::measurement(&format!("metric `{metric}`: {refused}")))
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
            let outcome = (decl.run)(&mut replicate);
            let failure = outcome
                .err()
                .map(|failure| failure.to_string())
                .or_else(|| replicate.finish().failure);
            if let Some(failure) = failure {
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
            service_clock: None,
            guest_clock: None,
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

    /// The record's form of `metric` once the replicate is finished; the replicate itself stays ok.
    fn stored(replicate: Replicate, metric: &str) -> MetricEntry {
        let mut finished = replicate.finish();
        assert!(
            finished.failure.is_none(),
            "a partly measured series leaves the replicate ok: {:?}",
            finished.failure
        );
        finished
            .metrics
            .remove(metric)
            .unwrap_or_else(|| panic!("the declared metric `{metric}` is in the record's form"))
    }

    fn held(values: &[f64], failures: &[&str]) -> MetricEntry {
        MetricEntry {
            values: values.to_vec(),
            histogram: None,
            failures: failures.iter().map(|reason| (*reason).to_owned()).collect(),
            absent: None,
        }
    }

    #[test]
    fn a_failure_after_a_series_leaves_the_metric_holding_both() {
        let mut replicate = replicate();
        assert!(replicate.record_series("stream", &[1.0, 2.0, 3.0]).is_ok());
        assert!(
            replicate.record_failure("stream", "the fourth workload failed").is_ok(),
            "a failure recorded after a series on the same series metric must be accepted"
        );
        assert_eq!(
            stored(replicate, "stream"),
            held(&[1.0, 2.0, 3.0], &["the fourth workload failed"])
        );
    }

    #[test]
    fn a_series_after_a_failure_leaves_the_metric_holding_both() {
        let mut replicate = replicate();
        assert!(replicate.record_failure("stream", "the first workload failed").is_ok());
        assert!(
            replicate.record_series("stream", &[2.0, 3.0, 4.0]).is_ok(),
            "a series recorded after a failure on the same series metric must be accepted"
        );
        assert_eq!(
            stored(replicate, "stream"),
            held(&[2.0, 3.0, 4.0], &["the first workload failed"])
        );
    }

    /// Every sequence of up to four `record_series` / `record_failure` calls on one series metric is
    /// accepted, and the stored entry holds every value and every reason in call order.
    #[test]
    fn a_series_metric_accepts_series_and_failures_in_every_order_and_keeps_all_of_them() {
        for length in 1_u32..=4 {
            for pattern in 0..(1_u32 << length) {
                let mut replicate = replicate();
                let (mut values, mut failures) = (Vec::new(), Vec::new());
                for call in 0..length {
                    let accepted = if (pattern >> call) & 1 == 0 {
                        let value = f64::from(call) + 1.0;
                        values.push(value);
                        replicate.record_series("stream", &[value]).is_ok()
                    } else {
                        let reason = format!("failure {call}");
                        let accepted = replicate.record_failure("stream", &reason).is_ok();
                        failures.push(reason);
                        accepted
                    };
                    assert!(
                        accepted,
                        "every order of series and failure calls on one series metric is accepted (call {call} of pattern {pattern:#b}, length {length})"
                    );
                }
                let reasons: Vec<&str> = failures.iter().map(String::as_str).collect();
                assert_eq!(stored(replicate, "stream"), held(&values, &reasons));
            }
        }
    }

    #[test]
    fn a_scalar_metric_never_holds_a_value_and_a_failure() {
        let mut after_value = replicate();
        assert!(after_value.record("scalar", 1.5).is_ok());
        assert!(after_value.record_failure("scalar", "late").is_err());
        assert_eq!(stored(after_value, "scalar"), held(&[1.5], &[]));
        let mut before_value = replicate();
        assert!(before_value.record_failure("scalar", "early").is_ok());
        assert!(before_value.record("scalar", 1.5).is_err());
        assert_eq!(stored(before_value, "scalar"), held(&[], &["early"]));
    }

    #[test]
    fn an_absence_excludes_values_and_failures_and_is_excluded_by_them() {
        for (metric, before) in [("scalar", 0), ("stream", 0), ("scalar", 1), ("stream", 1)] {
            let mut replicate = replicate();
            if before == 0 {
                assert!(replicate.record_failure(metric, "failed").is_ok());
            } else if metric == "scalar" {
                assert!(replicate.record(metric, 1.0).is_ok());
            } else {
                assert!(replicate.record_series(metric, &[1.0]).is_ok());
            }
            assert!(
                replicate.record_absent(metric, "does not apply").is_err(),
                "an absence on `{metric}` that already holds values or failures is a failure"
            );
        }
        let mut absent = replicate();
        assert!(absent.record_absent("stream", "no statistics").is_ok());
        assert!(absent.record_series("stream", &[1.0]).is_err());
        assert!(absent.record_failure("stream", "failed").is_err());
        assert!(absent.record_absent("stream", "again").is_err());
        let entry = stored(absent, "stream");
        assert_eq!(
            (entry.values.is_empty(), entry.failures.is_empty(), entry.absent.as_deref()),
            (true, true, Some("no statistics")),
            "the refused calls leave the absence alone"
        );
    }

    #[test]
    fn an_absence_is_refused_on_a_series_metric_holding_both_values_and_failures() {
        let mut replicate = replicate();
        assert!(replicate.record_series("stream", &[1.0]).is_ok());
        assert!(
            replicate.record_failure("stream", "failed").is_ok(),
            "a failure recorded after a series on the same series metric must be accepted"
        );
        assert!(replicate.record_absent("stream", "does not apply").is_err());
        assert_eq!(stored(replicate, "stream"), held(&[1.0], &["failed"]));
    }

    #[test]
    fn a_series_above_the_raw_limit_is_a_histogram_with_the_failures_alongside() {
        let above: Vec<f64> = (1..=1001).map(f64::from).collect();
        let at_limit: Vec<f64> = (1..=1000).map(f64::from).collect();
        // The limit itself stays raw; one value more is a histogram; the values of two calls add up.
        for (what, first, second, histogram_count) in [
            ("1,000 values", &at_limit[..], &[][..], None),
            ("1,001 values", &above[..], &[][..], Some(1001)),
            ("600 and 600 values", &above[..600], &above[..600], Some(1200)),
        ] {
            let mut series_first = replicate();
            assert!(series_first.record_series("stream", first).is_ok());
            assert!(
                series_first.record_failure("stream", "workload 4 failed").is_ok(),
                "a failure recorded beside a series above the raw limit must be accepted ({what}, series first)"
            );
            if !second.is_empty() {
                assert!(series_first.record_series("stream", second).is_ok());
            }
            let mut failure_first = replicate();
            assert!(failure_first.record_failure("stream", "workload 4 failed").is_ok());
            assert!(
                failure_first.record_series("stream", first).is_ok(),
                "a failure recorded beside a series above the raw limit must be accepted ({what}, failure first)"
            );
            if !second.is_empty() {
                assert!(failure_first.record_series("stream", second).is_ok());
            }
            for replicate in [series_first, failure_first] {
                let entry = stored(replicate, "stream");
                assert_eq!(entry.failures, ["workload 4 failed"], "{what}");
                assert_eq!(entry.absent, None, "{what}");
                if let Some(count) = histogram_count {
                    assert!(entry.values.is_empty(), "{what}: the values are in the histogram");
                    assert_eq!(entry.histogram.map(|h| h.count), Some(count), "{what}");
                } else {
                    assert_eq!(entry.values.len(), 1000, "{what}: the limit is stored raw");
                    assert!(entry.histogram.is_none(), "{what}");
                }
            }
        }
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
    fn a_negative_nan_or_infinite_value_is_refused_when_recorded() {
        let mut replicate = replicate();
        for value in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(replicate.record("scalar", value).is_err());
            assert!(replicate.record_series("stream", &[1.0, value]).is_err());
        }
        assert!(replicate.entries.is_empty());
    }

    #[test]
    #[should_panic(expected = "failed its smoke run")]
    fn smoke_fails_a_body_that_records_an_empty_series() {
        fn empty(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.record_series("stream", &[])
        }
        smoke(&decl(empty));
    }

    #[test]
    #[should_panic(expected = "benchmark `unit_bench` (family `unit-family`) failed its smoke run")]
    fn smoke_names_the_benchmark_when_a_replicate_fails() {
        fn broken(_: &mut Replicate) -> Result<(), ReplicateFailure> {
            Err(ReplicateFailure::fixture("no dataplane"))
        }
        smoke(&decl(broken));
    }

    // ---- The smoke variant of a guest benchmark (DDD-9, DDD-10) ----

    #[allow(clippy::expect_used, reason = "fixed non-zero literals")]
    fn guest_view() -> crate::GuestView {
        crate::GuestView {
            vcpus: std::num::NonZeroU8::new(2).expect("two"),
            memory_mib: crate::GuestMemoryMib::new(std::num::NonZeroU32::new(512).expect("512")),
            kernel: crate::HostValue::Text("7.0.0-38-generic".to_owned()),
            clocksource: crate::HostValue::Text("kvm-clock".to_owned()),
            artifacts: crate::GuestArtifacts {
                overdrive_init: crate::HostValue::Text("a".repeat(64)),
                overdrive_guest_peer: crate::HostValue::Text("b".repeat(64)),
                ovd_fwd: crate::HostValue::Text("c".repeat(64)),
            },
        }
    }

    /// Under the smoke variant a guest benchmark's body may report its view and retain its console:
    /// both return `Ok` and nothing is recorded or written, so the smoke test of a guest family
    /// passes after one replicate per arm and asserts nothing about time. GREEN guard (the
    /// well-formed control of the three smoke refusals below).
    /// `CONTRACT_SHAPE`: unbounded-preservation (nothing is written).
    #[test]
    fn a_guest_benchmarks_smoke_run_reports_and_retains_without_effect() {
        fn guest_body(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(guest_view())?;
            replicate.retain_console("a console")?;
            replicate.record_series("stream", &[100.0, 200.0])?;
            replicate.record("scalar", 1.0)
        }
        let mut guest = decl(guest_body);
        guest.substrate = "guest";
        smoke(&guest);
    }

    // Under `smoke` only the effect is skipped (DDD-9): a guest-substrate precondition failure,
    // the M-1 failure (a guest body that returns `Ok` without reporting a view) and a second
    // `retain_console` call fail the replicate exactly as in a real run, so the smoke test of a
    // guest family catches them. Each test first proves the well-formed guest body passes
    // `smoke` (the control), then the one broken rule must panic naming the benchmark.
    // `CONTRACT_SHAPE`: pure-function (declaration in, smoke verdict out).

    /// What `smoke` panicked with, or `None` when it passed.
    fn smoke_failure(decl: &BenchmarkDecl) -> Option<String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| smoke(decl))).err().map(
            |payload| {
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                    .unwrap_or_default()
            },
        )
    }

    fn well_formed_guest_body(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
        replicate.report_guest_view(guest_view())?;
        replicate.retain_console("a console")?;
        replicate.record_series("stream", &[100.0, 200.0])?;
        replicate.record("scalar", 1.0)
    }

    /// M-1 under smoke: a guest body that returns `Ok` without reporting a view fails the smoke
    /// run naming the benchmark and the pinned reason.
    #[test]
    fn smoke_fails_a_guest_body_that_returns_ok_without_reporting_a_view() {
        fn no_view(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.retain_console("a console")?;
            replicate.record_series("stream", &[100.0, 200.0])?;
            replicate.record("scalar", 1.0)
        }
        let mut control = decl(well_formed_guest_body);
        control.substrate = "guest";
        assert_eq!(smoke_failure(&control), None, "the well-formed guest body must pass smoke");
        let mut guest = decl(no_view);
        guest.substrate = "guest";
        let failure = smoke_failure(&guest).unwrap_or_else(|| {
            panic!("a guest body that returns Ok without a view must fail its smoke run")
        });
        assert!(
            failure.contains("unit_bench")
                && failure.contains("the replicate reported no guest view"),
            "the smoke failure names the benchmark and the reason: {failure}"
        );
    }

    /// A second `retain_console` in one replicate fails the replicate under smoke as in a real run.
    #[test]
    fn smoke_fails_a_guest_body_that_retains_its_console_twice() {
        fn twice(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(guest_view())?;
            replicate.retain_console("first")?;
            replicate.retain_console("second")?;
            replicate.record_series("stream", &[100.0, 200.0])?;
            replicate.record("scalar", 1.0)
        }
        let mut control = decl(well_formed_guest_body);
        control.substrate = "guest";
        assert_eq!(smoke_failure(&control), None, "the well-formed guest body must pass smoke");
        let mut guest = decl(twice);
        guest.substrate = "guest";
        let failure = smoke_failure(&guest)
            .unwrap_or_else(|| panic!("a second retain_console must fail its smoke run"));
        assert!(failure.contains("unit_bench"), "the smoke failure names the benchmark: {failure}");
    }

    /// The guest-substrate precondition under smoke: a native benchmark that reports a guest view
    /// fails its smoke run (the view of a benchmark that does not declare the guest substrate).
    #[test]
    fn smoke_fails_a_native_body_that_reports_a_guest_view() {
        fn reports(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(guest_view())?;
            replicate.record_series("stream", &[100.0, 200.0])?;
            replicate.record("scalar", 1.0)
        }
        let mut control = decl(well_formed_guest_body);
        control.substrate = "guest";
        assert_eq!(smoke_failure(&control), None, "the well-formed guest body must pass smoke");
        let failure = smoke_failure(&decl(reports)).unwrap_or_else(|| {
            panic!("a view reported by a native benchmark must fail its smoke run")
        });
        assert!(failure.contains("unit_bench"), "the smoke failure names the benchmark: {failure}");
    }
}
