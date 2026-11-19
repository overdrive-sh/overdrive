//! The derived report (ADR-0301, DDD-6 "Report"): a pure function of the record.
//!
//! One renderer serves the runner and `bench-report render`: the record in, Markdown out. Nothing
//! derived is stored, so every interval, p-value and verdict is computed here from the record's
//! own seed, and the same record always renders to the same bytes.

use std::fmt::Display;

use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::Result;
use crate::canary::{CanaryVerdict, verdict};
use crate::record::{
    BpfStatsMode, Canary, CanaryPhase, EvidenceClass, GUEST_SETTINGS, GuestObserved, HOST_KEYS,
    HostValue, Line, MetricDecl, MetricEntry, NOT_VISIBLE_FROM_A_GUEST, Outcome, ProfileDecl,
    ReplicateLine, Role, RunRefused, RunStarted, parse,
};
use crate::registry::{GuestClock, ServiceClock};
use crate::stats::{
    Comparison, Distribution, FIRST_QUARTILE, MEDIAN, Pool, THIRD_QUARTILE, compare, percentile,
    sorted,
};

const P90: f64 = 0.90;
const P95: f64 = 0.95;
const P99: f64 = 0.99;

const SCALAR_COLUMNS: [&str; 6] = ["metric", "arm", "median", "IQR", "replicates", "failures"];
const SERIES_COLUMNS: [&str; 9] =
    ["metric", "arm", "p50", "p90", "p95", "p99", "values", "replicates", "failures"];
const NOT_COMPARED: &str = "n/a";
const NO_GUEST_VIEW: &str = "absent: no replicate returned a guest view";

/// Render the report of a record, validating it first.
pub fn render(text: &str) -> Result<String> {
    let lines = parse(text)?;
    let run = Run::from_lines(&lines);
    Ok(Renderer::new(&run).markdown())
}

// ---------------------------------------------------------------------------------------
// The record, seen as a run.

enum Terminal<'a> {
    Finished,
    Refused(&'a RunRefused),
    Incomplete,
}

struct Run<'a> {
    started: &'a RunStarted,
    start_canary: Option<&'a Canary>,
    end_canary: Option<&'a Canary>,
    guest_view: Option<&'a GuestObserved>,
    replicates: Vec<&'a ReplicateLine>,
    terminal: Terminal<'a>,
}

impl<'a> Run<'a> {
    /// The run a validated record describes: it begins with `run_started` and holds it once.
    fn from_lines(lines: &'a [Line]) -> Self {
        let Some(Line::RunStarted(started)) = lines.first() else {
            unreachable!("a validated record begins with run_started")
        };
        let mut run = Self {
            started,
            start_canary: None,
            end_canary: None,
            guest_view: None,
            replicates: Vec::new(),
            terminal: Terminal::Incomplete,
        };
        for line in lines.iter().skip(1) {
            match line {
                Line::RunStarted(_) => unreachable!("a validated record holds run_started once"),
                Line::Canary(canary) => run.keep_canary(canary),
                Line::GuestObserved(view) => run.guest_view = Some(view),
                Line::Replicate(replicate) => run.replicates.push(replicate),
                Line::RunFinished(_) => run.terminal = Terminal::Finished,
                Line::RunRefused(refused) => run.terminal = Terminal::Refused(refused),
            }
        }
        run
    }

    const fn keep_canary(&mut self, canary: &'a Canary) {
        match canary.phase {
            CanaryPhase::Start => self.start_canary = Some(canary),
            CanaryPhase::End => self.end_canary = Some(canary),
        }
    }

    fn count(&self, outcome: Outcome) -> usize {
        self.replicates.iter().filter(|replicate| replicate.outcome == outcome).count()
    }
}

// ---------------------------------------------------------------------------------------
// One arm's figures for one metric.

/// What the `ok` replicates of one arm (or of the A/A control) hold for one metric.
#[derive(Default)]
struct Cell<'a> {
    /// The value of each `ok` replicate holding one (its median for a series).
    per_replicate: Vec<f64>,
    /// A series' pooled distribution.
    pooled: Option<Distribution>,
    failures: usize,
    /// The first absent reason in schedule order.
    absent: Option<&'a str>,
}

impl<'a> Cell<'a> {
    fn gather(run: &Run<'a>, profile: &str, arm: &str, role: Role, metric: &MetricDecl) -> Self {
        let mut cell = Self::default();
        let mut pool = Pool::default();
        let held = run
            .replicates
            .iter()
            .filter(|r| r.profile == profile && r.arm == arm && r.role == role);
        for replicate in held {
            if replicate.outcome == Outcome::Failed {
                cell.failures += 1;
                continue;
            }
            let Some(entry) = replicate.metrics.get(&metric.name) else { continue };
            cell.failures += usize::from(!entry.failures.is_empty());
            if cell.absent.is_none() {
                cell.absent = entry.absent.as_deref();
            }
            let mut single = Pool::default();
            add_entry(&mut single, entry);
            if let Some(distribution) = single.finish() {
                cell.per_replicate.push(distribution.percentile(MEDIAN));
                if metric.series {
                    add_entry(&mut pool, entry);
                }
            }
        }
        cell.pooled = pool.finish();
        cell
    }

    /// `absent: <reason>` when no replicate holds a value, else `None`.
    fn absence(&self) -> Option<String> {
        self.per_replicate
            .is_empty()
            .then(|| format!("absent: {}", self.absent.unwrap_or("no stored value")))
    }
}

fn add_entry(pool: &mut Pool, entry: &MetricEntry) {
    pool.add_values(&entry.values);
    if let Some(histogram) = &entry.histogram {
        pool.add_histogram(histogram);
    }
}

/// The cells of one metric: one per declared arm in order, and the A/A control of the reference arm.
struct MetricCells<'a> {
    metric: &'a MetricDecl,
    arms: Vec<Cell<'a>>,
    control: Option<Cell<'a>>,
}

// ---------------------------------------------------------------------------------------
// Markdown text.

fn num(value: f64) -> String {
    value.to_string()
}

fn row(cells: &[&str]) -> String {
    let escaped: Vec<String> = cells.iter().map(|cell| cell.replace('|', "\\|")).collect();
    format!("| {} |", escaped.join(" | "))
}

fn header_rows(columns: &[&str]) -> [String; 2] {
    [row(columns), format!("|{}", "---|".repeat(columns.len()))]
}

fn host_text(value: &HostValue) -> String {
    match value {
        HostValue::Text(text) => text.clone(),
        HostValue::Absent { absent } => format!("absent: {absent}"),
        HostValue::Unreadable { unreadable } => format!("unreadable: {unreadable}"),
    }
}

fn scalar_row(metric: &str, arm: &str, cell: &Cell) -> String {
    let (median, spread) = cell.absence().map_or_else(
        || {
            let values = sorted(&cell.per_replicate);
            let iqr = percentile(&values, THIRD_QUARTILE) - percentile(&values, FIRST_QUARTILE);
            (num(percentile(&values, MEDIAN)), num(iqr))
        },
        |absent| (absent.clone(), absent),
    );
    row(&[
        metric,
        arm,
        &median,
        &spread,
        &cell.per_replicate.len().to_string(),
        &cell.failures.to_string(),
    ])
}

fn series_row(metric: &str, arm: &str, cell: &Cell) -> String {
    let (percentiles, values) = match (&cell.pooled, cell.absence()) {
        (Some(pooled), _) => {
            ([MEDIAN, P90, P95, P99].map(|p| num(pooled.percentile(p))), pooled.count().to_string())
        }
        (None, absent) => {
            let reason = absent.unwrap_or_default();
            ([reason.clone(), reason.clone(), reason.clone(), reason], "0".to_owned())
        }
    };
    let [p50, p90, p95, p99] = &percentiles;
    row(&[
        metric,
        arm,
        p50,
        p90,
        p95,
        p99,
        &values,
        &cell.per_replicate.len().to_string(),
        &cell.failures.to_string(),
    ])
}

fn interval_text(lower: f64, upper: f64) -> String {
    format!("[{}, {}]", num(lower), num(upper))
}

// ---------------------------------------------------------------------------------------
// The renderer.

struct Renderer<'a> {
    run: &'a Run<'a>,
    rng: StdRng,
    out: Vec<String>,
}

impl<'a> Renderer<'a> {
    fn new(run: &'a Run<'a>) -> Self {
        Self { run, rng: StdRng::seed_from_u64(run.started.seed), out: Vec::new() }
    }

    fn markdown(mut self) -> String {
        self.header();
        if !matches!(self.run.terminal, Terminal::Refused(_)) {
            self.profiles();
            self.failures();
        }
        while self.out.last().is_some_and(String::is_empty) {
            self.out.pop();
        }
        self.out.join("\n") + "\n"
    }

    fn line(&mut self, text: impl Into<String>) {
        self.out.push(text.into());
    }

    fn blank(&mut self) {
        self.out.push(String::new());
    }

    fn bullet(&mut self, key: &str, value: impl Display) {
        self.out.push(format!("- {key}: {value}"));
    }

    // ----- Header.

    fn header(&mut self) {
        let run = self.run;
        let started = run.started;
        self.line(format!("# Benchmark report: {} / {}", started.family, started.benchmark));
        self.blank();
        self.bullet("run id", &started.run_id);
        self.bullet("family", &started.family);
        self.bullet("benchmark", &started.benchmark);
        self.bullet("evidence", evidence_text(started.evidence));
        self.bullet("substrate", started.substrate.as_str());
        self.bullet("commit", &started.commit);
        self.bullet("dirty", started.dirty);
        self.bullet("methodology", started.methodology);
        self.bullet("seed", started.seed);
        self.bullet("debug_assertions", started.debug_assertions);
        for key in HOST_KEYS {
            let value = started.host.0.get(key).map(host_text).unwrap_or_default();
            self.bullet(key, value);
        }
        if started.guest.is_some() {
            self.guest_lines();
        }
        let statistics = match started.bpf_stats {
            BpfStatsMode::PerProcess => "on (per process)",
            BpfStatsMode::Off => "off",
        };
        self.bullet("bpf stats", statistics);
        self.bullet("clock", clock_text(started));
        self.bullet("canary", self.canary_text());
        self.bullet("replicates ok", run.count(Outcome::Ok));
        self.bullet("replicates failed", run.count(Outcome::Failed));
        self.bullet("status", status_text(&run.terminal));
        if let Terminal::Refused(refused) = run.terminal {
            self.bullet("refused", refusal_text(refused));
        }
    }

    /// The guest's conditions: those true before any guest booted, then what the first reporting
    /// replicate's view observed (or that none did).
    fn guest_lines(&mut self) {
        self.bullet("guest launch", "metal host (not nested)");
        self.bullet("guest boot", "fresh guest per replicate");
        self.bullet("guest vcpu scheduling", "unpinned");
        for setting in GUEST_SETTINGS {
            self.bullet(&format!("guest {setting}"), format!("absent: {NOT_VISIBLE_FROM_A_GUEST}"));
        }
        let keys = [
            "guest vcpus",
            "guest memory mib",
            "guest kernel",
            "guest clocksource",
            "guest artifact overdrive_init",
            "guest artifact overdrive_guest_peer",
            "guest artifact ovd_fwd",
        ];
        let values: [String; 7] = self.run.guest_view.map_or_else(
            || std::array::from_fn(|_| NO_GUEST_VIEW.to_owned()),
            |view| {
                [
                    view.vcpus.to_string(),
                    view.memory_mib.to_string(),
                    host_text(&view.kernel),
                    host_text(&view.clocksource),
                    host_text(&view.artifacts.overdrive_init),
                    host_text(&view.artifacts.overdrive_guest_peer),
                    host_text(&view.artifacts.ovd_fwd),
                ]
            },
        );
        for (key, value) in keys.into_iter().zip(values) {
            self.bullet(key, value);
        }
    }

    fn canary_text(&self) -> String {
        let run = self.run;
        let (Some(start), Some(end)) = (run.start_canary, run.end_canary) else {
            return if run.start_canary.is_some() { "not finished" } else { "not run" }.to_owned();
        };
        match verdict(&start.samples_ns, &end.samples_ns, run.started.seed) {
            Some(CanaryVerdict::Steady) => "steady".to_owned(),
            Some(CanaryVerdict::Unstable { start_median, end_median }) => format!(
                "unstable (start median {} ns, end median {} ns)",
                num(start_median),
                num(end_median)
            ),
            None => "no verdict".to_owned(),
        }
    }

    // ----- Profiles and tables.

    fn profiles(&mut self) {
        let run = self.run;
        let declaration = &run.started.declaration;
        for profile in &declaration.profiles {
            self.line(format!("## Profile: {}", profile.name));
            self.blank();
            if !profile.preparation.is_empty() {
                self.line(&profile.preparation);
                self.blank();
            }
            let mut labels: Vec<&str> = Vec::new();
            for metric in &declaration.metrics {
                if !labels.contains(&metric.table.as_str()) {
                    labels.push(&metric.table);
                }
            }
            for label in labels {
                self.table(profile, label);
            }
        }
    }

    fn table(&mut self, profile: &ProfileDecl, label: &str) {
        let run = self.run;
        let declaration = &run.started.declaration;
        let metrics: Vec<&MetricDecl> =
            declaration.metrics.iter().filter(|metric| metric.table == label).collect();
        let cells: Vec<MetricCells> =
            metrics.iter().map(|metric| Self::gather(run, &profile.name, metric)).collect();
        self.line(format!("### Table: {label}"));
        self.blank();
        self.arm_tables(&cells);
        self.comparisons(&cells);
        for reference in &declaration.references {
            let Some(metric) = metrics.iter().find(|metric| metric.name == reference.metric) else {
                continue;
            };
            self.line(format!(
                "reference: {}: {} {} (source: {})",
                reference.label,
                num(reference.value),
                metric.unit.as_str(),
                reference.source
            ));
            self.blank();
        }
    }

    fn gather<'r>(run: &'r Run<'r>, profile: &str, metric: &'r MetricDecl) -> MetricCells<'r> {
        let arms = &run.started.declaration.arms;
        let cell = |arm: &str, role| Cell::gather(run, profile, arm, role, metric);
        MetricCells {
            metric,
            arms: arms.iter().map(|arm| cell(arm, Role::Arm)).collect(),
            control: arms
                .first()
                .filter(|_| arms.len() > 1)
                .map(|reference| cell(reference, Role::Control)),
        }
    }

    /// The scalar table, then the series table with its histogram notes (each only if the table
    /// declares such a metric).
    fn arm_tables(&mut self, cells: &[MetricCells]) {
        let arms = &self.run.started.declaration.arms;
        let scalars: Vec<&MetricCells> = cells.iter().filter(|c| !c.metric.series).collect();
        let series: Vec<&MetricCells> = cells.iter().filter(|c| c.metric.series).collect();
        if !scalars.is_empty() {
            self.out.extend(header_rows(&SCALAR_COLUMNS));
            for metric_cells in &scalars {
                for (arm, cell) in arms.iter().zip(&metric_cells.arms) {
                    self.out.push(scalar_row(&metric_cells.metric.name, arm, cell));
                }
            }
            self.blank();
        }
        if series.is_empty() {
            return;
        }
        self.out.extend(header_rows(&SERIES_COLUMNS));
        for metric_cells in &series {
            for (arm, cell) in arms.iter().zip(&metric_cells.arms) {
                self.out.push(series_row(&metric_cells.metric.name, arm, cell));
            }
        }
        self.blank();
        for metric_cells in series.iter().filter(|c| has_histogram(c)) {
            self.line(format!(
                "percentiles of {} are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)",
                metric_cells.metric.name
            ));
            self.blank();
        }
    }

    /// Below the arm tables, each non-reference arm against the reference for every metric both hold.
    fn comparisons(&mut self, cells: &[MetricCells]) {
        let arms = &self.run.started.declaration.arms;
        let [reference, compared @ ..] = arms.as_slice() else { return };
        let arm_label = match compared {
            [only] => only.as_str(),
            _ => "arm",
        };
        let mut rows = Vec::new();
        for metric_cells in cells {
            let Some(reference_cell) = metric_cells.arms.first() else { continue };
            let control: &[f64] =
                metric_cells.control.as_ref().map_or(&[][..], |cell| cell.per_replicate.as_slice());
            for (arm, cell) in compared.iter().zip(metric_cells.arms.iter().skip(1)) {
                if reference_cell.per_replicate.is_empty() || cell.per_replicate.is_empty() {
                    continue;
                }
                let comparison = compare(
                    &mut self.rng,
                    &reference_cell.per_replicate,
                    &cell.per_replicate,
                    control,
                );
                rows.push(comparison_row(&metric_cells.metric.name, arm, &comparison));
            }
        }
        if rows.is_empty() {
            return;
        }
        let delta = format!("delta ({arm_label} minus {reference})");
        self.out.extend(header_rows(&[
            "metric",
            "arm",
            &delta,
            "95% bootstrap interval",
            "A/A 95% interval",
            "Mann-Whitney U",
            "p",
            "verdict",
        ]));
        self.out.extend(rows);
        self.blank();
    }

    // ----- Failures.

    fn failures(&mut self) {
        let mut lines = Vec::new();
        for replicate in &self.run.replicates {
            let place = format!(
                "- profile {} block {} position {} arm {}",
                replicate.profile, replicate.block, replicate.position, replicate.arm
            );
            if let Some(reason) = &replicate.failure {
                lines.push(format!("{place}: {reason}"));
                continue;
            }
            for (metric, entry) in &replicate.metrics {
                lines.extend(
                    entry
                        .failures
                        .iter()
                        .map(|reason| format!("{place} metric {metric}: {reason}")),
                );
            }
        }
        if lines.is_empty() {
            return;
        }
        self.line("## Failures");
        self.blank();
        self.out.extend(lines);
    }
}

fn comparison_row(metric: &str, arm: &str, comparison: &Comparison) -> String {
    match comparison {
        Comparison::Insufficient => row(&[
            metric,
            arm,
            NOT_COMPARED,
            NOT_COMPARED,
            NOT_COMPARED,
            NOT_COMPARED,
            NOT_COMPARED,
            "insufficient replicates",
        ]),
        Comparison::Compared { shift, test } => {
            let verdict = if shift.distinguishable() {
                "distinguishable"
            } else {
                "not distinguishable from noise"
            };
            row(&[
                metric,
                arm,
                &num(shift.delta),
                &interval_text(shift.interval.lower, shift.interval.upper),
                &interval_text(shift.floor.lower, shift.floor.upper),
                &num(test.u),
                &num(test.p),
                verdict,
            ])
        }
    }
}

fn has_histogram(metric_cells: &MetricCells) -> bool {
    metric_cells
        .arms
        .iter()
        .any(|cell| cell.pooled.as_ref().is_some_and(Distribution::is_histogram))
}

const fn evidence_text(evidence: EvidenceClass) -> &'static str {
    match evidence {
        EvidenceClass::Maintained => "maintained",
        EvidenceClass::Spike => "spike evidence",
    }
}

fn clock_text(started: &RunStarted) -> String {
    let service = started.clock.service.map_or("none", ServiceClock::as_str);
    let guest = started.clock.guest.map_or("none", GuestClock::as_str);
    format!("host {}; service {service}; guest {guest}", started.clock.host.as_str())
}

const fn status_text(terminal: &Terminal) -> &'static str {
    match terminal {
        Terminal::Finished => "finished",
        Terminal::Refused(_) => "refused",
        Terminal::Incomplete => "incomplete (no run_finished or run_refused)",
    }
}

fn refusal_text(refused: &RunRefused) -> String {
    match (&refused.found, &refused.pinned, &refused.reason) {
        (Some(found), Some(pinned), _) => {
            format!("{} found {found} pinned {pinned}", refused.setting)
        }
        (_, _, Some(reason)) => format!("{}: {reason}", refused.setting),
        _ => unreachable!("a validated refusal carries found and pinned, or a reason"),
    }
}
