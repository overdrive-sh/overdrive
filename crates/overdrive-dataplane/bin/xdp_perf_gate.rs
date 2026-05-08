//! Tier 4 XDP throughput / mean-latency regression gate — pure
//! decision fn + parsers.
//!
//! # Architecture
//!
//! Same shape as [`super::verifier_budget`]: three pure functions
//! plus a shell-side wrapper in
//! `crates/overdrive-dataplane/bin/xdp_perf.rs`:
//!
//! 1. [`parse_baseline_file`] — line-by-line key=value parser that
//!    skips `#`-comment and blank lines; returns one
//!    [`BaselineRecord`] per data line.
//! 2. [`parse_measured_output`] — same key=value parser used to
//!    re-parse measurements written by the gate binary itself (kept
//!    as a separate entry point so a future format pivot — e.g.
//!    JSON dump of `bpf_prog_info` — only changes this fn and its
//!    call sites).
//! 3. [`evaluate`] — pure decision fn: given baselines + candidates +
//!    policy, returns [`GateOutcome::Pass`] or `Fail { breaches }`.
//!    No I/O, no subprocess. Self-tested at
//!    `crates/overdrive-dataplane/tests/integration/xdp_perf_gate.rs`.
//!
//! # Signal source
//!
//! Measurements come from aya's
//! [`ProgramInfo::run_time`](aya::programs::ProgramInfo) and
//! [`ProgramInfo::run_count`](aya::programs::ProgramInfo) — kernel
//! ≥5.8 reads `bpf_prog_info.run_time_ns` and `run_cnt` via
//! `BPF_OBJ_GET_INFO_BY_FD`, gated on
//! [`aya::sys::enable_stats(Stats::RunTime)`](aya::sys::enable_stats).
//! `bpftool prog profile` is the libbpf-shaped alternative; the aya
//! ELF's legacy `maps` section makes it unavailable
//! (aya issue #913, no opt-out). `pps` is computed as
//! `run_count / duration`; `mean_ns` as `run_time_ns / run_count`.
//!
//! True per-packet p99 is **not** in `bpf_prog_info` UAPI — the
//! kernel only exposes cumulative `run_time_ns` and `run_cnt`. This
//! gate measures *mean* per-invocation cost; tail latency requires a
//! separate BPF probe (out of scope today).
//!
//! # Gate policy
//!
//! - **PPS regression**: fail if
//!   `measured.pps < baseline.pps * (1 - max_pps_regression_fraction)`.
//!   Default `max_pps_regression_fraction = 0.05` (>5%).
//! - **Mean-latency rise**: fail if
//!   `measured.mean_ns > baseline.mean_ns * (1 + max_mean_growth_fraction)`.
//!   Default `max_mean_growth_fraction = 0.10` (>10%).
//!
//! Both clauses are evaluated per program; either tripping is a
//! breach. A program present in baselines but missing from
//! candidates is a separate `MissingFromCandidates` breach. A
//! candidate without a baseline is *not* a breach — new programs are
//! baselined in their introducing commit.

#![allow(
    clippy::cast_precision_loss,
    reason = "pps values are bounded by physical NIC line rate (~10s of Mpps), well below 2^53; mean_ns values are bounded by the per-packet wall-clock budget (low microseconds at most), also well below 2^53. All casts are exact-representable."
)]

use std::fmt::Write as _;

use thiserror::Error;

/// One row of `perf-baseline/main/xdp-perf/<program>.txt`.
///
/// Per-file format documented inline in the baseline files: leading
/// `#` comments + one or more `key=value` data lines. The parser
/// keys off `prog=`, `pps=`, `mean_ns=` — every other key
/// (`tool=`, `kernel-min=`, `iface=`) is metadata and ignored at the
/// gate.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineRecord {
    /// Program name (e.g. `xdp_service_map_lookup`). Matches the
    /// `prog=<name>` field in baseline files and the program name
    /// the binary loads via aya.
    pub program: String,
    /// Recorded packets-per-second (post `BPF_ENABLE_STATS`
    /// `run_cnt / duration`). f64 because xdp-trafficgen routinely
    /// hits fractional Mpps; the gate keeps the precision rather
    /// than rounding to integer pps.
    pub pps: f64,
    /// Recorded mean nanoseconds per program invocation
    /// (`run_time_ns / run_cnt`). u64 — the kernel records this as
    /// integer ns; rounding to integer happens in userspace, not
    /// here.
    pub mean_ns: u64,
}

/// One row of measured output captured at gate time.
///
/// Same shape as [`BaselineRecord`] but kept as a distinct type so a
/// future signal-source pivot only touches [`parse_measured_output`]
/// and the binary's `measure()` entry point, not the baseline-file
/// surface.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredRecord {
    pub program: String,
    pub pps: f64,
    pub mean_ns: u64,
}

/// Threshold policy for the gate. `Default` matches
/// `.claude/rules/testing.md` § "XDP performance" — >5% pps
/// regression, >10% mean-latency rise.
#[derive(Debug, Clone)]
pub struct GatePolicy {
    /// Fraction of the baseline pps below which the measured value
    /// is a breach. 0.05 = >5% drop.
    pub max_pps_regression_fraction: f64,
    /// Fraction of the baseline `mean_ns` above which the measured
    /// value is a breach. 0.10 = >10% rise.
    pub max_mean_growth_fraction: f64,
}

impl Default for GatePolicy {
    fn default() -> Self {
        Self { max_pps_regression_fraction: 0.05, max_mean_growth_fraction: 0.10 }
    }
}

/// Why a program failed the gate. Each variant captures the
/// triggering numbers so the renderer can produce structured
/// per-breach messages without recomputing.
#[derive(Debug, Clone, PartialEq)]
pub enum BreachKind {
    /// `measured.pps < baseline.pps * (1 - max_pps_regression_fraction)`.
    PpsRegression {
        /// Threshold fraction the policy was configured with (e.g.
        /// 0.05 for >5%); pinned in the breach so the renderer can
        /// echo it verbatim.
        threshold_fraction: f64,
    },
    /// `measured.mean_ns > baseline.mean_ns * (1 + max_mean_growth_fraction)`.
    MeanLatencyRise {
        /// Threshold fraction the policy was configured with (e.g.
        /// 0.10 for >10%).
        threshold_fraction: f64,
    },
    /// Baseline names a program the candidates do not — typically
    /// the BPF object dropped or renamed a program without updating
    /// the baseline directory. Always a breach: silent baseline rot
    /// is exactly what this gate exists to catch.
    MissingFromCandidates,
}

/// One per program that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Breach {
    pub program: String,
    pub baseline_pps: f64,
    pub baseline_mean_ns: u64,
    /// Measured pps when present. `0.0` for
    /// [`BreachKind::MissingFromCandidates`].
    pub measured_pps: f64,
    /// Measured `mean_ns` when present. `0` for
    /// [`BreachKind::MissingFromCandidates`].
    pub measured_mean_ns: u64,
    /// Pps drop ratio `(baseline - measured) / baseline`. Positive
    /// when measured < baseline (i.e. a regression). `0.0` for
    /// missing-from-candidates breaches.
    pub pps_drop_fraction: f64,
    /// Mean-latency growth ratio `(measured - baseline) / baseline`.
    /// Positive when measured > baseline. `0.0` for
    /// missing-from-candidates breaches.
    pub mean_growth_fraction: f64,
    pub kind: BreachKind,
}

/// Verdict from [`evaluate`]. The shell-side wrapper translates
/// `Fail` into a non-zero exit; tests and renderers consume the
/// structured form directly.
#[derive(Debug, Clone, PartialEq)]
pub enum GateOutcome {
    Pass,
    Fail { breaches: Vec<Breach> },
}

/// Pure gate decision.
#[must_use]
pub fn evaluate(
    baselines: &[BaselineRecord],
    candidates: &[MeasuredRecord],
    policy: &GatePolicy,
) -> GateOutcome {
    let mut breaches: Vec<Breach> = Vec::new();

    for baseline in baselines {
        let candidate = candidates.iter().find(|c| c.program == baseline.program);
        let Some(candidate) = candidate else {
            breaches.push(Breach {
                program: baseline.program.clone(),
                baseline_pps: baseline.pps,
                baseline_mean_ns: baseline.mean_ns,
                measured_pps: 0.0,
                measured_mean_ns: 0,
                pps_drop_fraction: 0.0,
                mean_growth_fraction: 0.0,
                kind: BreachKind::MissingFromCandidates,
            });
            continue;
        };

        let pps_drop_fraction =
            if baseline.pps == 0.0 { 0.0 } else { (baseline.pps - candidate.pps) / baseline.pps };
        let pps_threshold = baseline.pps * (1.0 - policy.max_pps_regression_fraction);
        if candidate.pps < pps_threshold {
            breaches.push(Breach {
                program: baseline.program.clone(),
                baseline_pps: baseline.pps,
                baseline_mean_ns: baseline.mean_ns,
                measured_pps: candidate.pps,
                measured_mean_ns: candidate.mean_ns,
                pps_drop_fraction,
                mean_growth_fraction: 0.0,
                kind: BreachKind::PpsRegression {
                    threshold_fraction: policy.max_pps_regression_fraction,
                },
            });
            continue;
        }

        let baseline_mean_f = baseline.mean_ns as f64;
        let measured_mean_f = candidate.mean_ns as f64;
        let mean_growth_fraction = if baseline.mean_ns == 0 {
            0.0
        } else {
            (measured_mean_f - baseline_mean_f) / baseline_mean_f
        };
        let mean_threshold = baseline_mean_f * (1.0 + policy.max_mean_growth_fraction);
        if measured_mean_f > mean_threshold {
            breaches.push(Breach {
                program: baseline.program.clone(),
                baseline_pps: baseline.pps,
                baseline_mean_ns: baseline.mean_ns,
                measured_pps: candidate.pps,
                measured_mean_ns: candidate.mean_ns,
                pps_drop_fraction,
                mean_growth_fraction,
                kind: BreachKind::MeanLatencyRise {
                    threshold_fraction: policy.max_mean_growth_fraction,
                },
            });
        }
    }

    if breaches.is_empty() { GateOutcome::Pass } else { GateOutcome::Fail { breaches } }
}

/// Errors from the parsers. Distinct typed variants per failure
/// mode per `.claude/rules/development.md` § Errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("line {lineno}: missing `prog=<name>` in {line:?}")]
    MissingProg { lineno: usize, line: String },
    #[error("line {lineno}: missing `pps=<f64>` in {line:?}")]
    MissingPps { lineno: usize, line: String },
    #[error("line {lineno}: missing `mean_ns=<u64>` in {line:?}")]
    MissingMeanNs { lineno: usize, line: String },
    #[error("line {lineno}: malformed `pps=` value in {line:?}: {source}")]
    PpsParse { lineno: usize, line: String, source: std::num::ParseFloatError },
    #[error("line {lineno}: malformed `mean_ns=` value in {line:?}: {source}")]
    MeanNsParse { lineno: usize, line: String, source: std::num::ParseIntError },
}

/// Parse a baseline file. Skips `#`-prefixed comment lines and blank
/// lines; treats every other line as a `key=value` data row.
pub fn parse_baseline_file(text: &str) -> Result<Vec<BaselineRecord>, ParseError> {
    let mut records = Vec::new();
    for (lineno, line) in text.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (program, pps, mean_ns) = parse_data_line(trimmed, lineno)?;
        records.push(BaselineRecord { program, pps, mean_ns });
    }
    Ok(records)
}

/// Parse measured output captured at gate time. Same shape as
/// [`parse_baseline_file`] today; held as a separate entry point
/// for future format-pivot insulation.
pub fn parse_measured_output(text: &str) -> Result<Vec<MeasuredRecord>, ParseError> {
    let mut records = Vec::new();
    for (lineno, line) in text.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (program, pps, mean_ns) = parse_data_line(trimmed, lineno)?;
        records.push(MeasuredRecord { program, pps, mean_ns });
    }
    Ok(records)
}

fn parse_data_line(line: &str, lineno: usize) -> Result<(String, f64, u64), ParseError> {
    let mut program: Option<String> = None;
    let mut pps_str: Option<&str> = None;
    let mut mean_ns_str: Option<&str> = None;
    for token in line.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else { continue };
        match key {
            "prog" => program = Some(value.to_string()),
            "pps" => pps_str = Some(value),
            "mean_ns" => mean_ns_str = Some(value),
            _ => {}
        }
    }
    let program =
        program.ok_or_else(|| ParseError::MissingProg { lineno, line: line.to_string() })?;
    let pps_str =
        pps_str.ok_or_else(|| ParseError::MissingPps { lineno, line: line.to_string() })?;
    let mean_ns_str =
        mean_ns_str.ok_or_else(|| ParseError::MissingMeanNs { lineno, line: line.to_string() })?;
    let pps = pps_str.parse::<f64>().map_err(|source| ParseError::PpsParse {
        lineno,
        line: line.to_string(),
        source,
    })?;
    let mean_ns = mean_ns_str.parse::<u64>().map_err(|source| ParseError::MeanNsParse {
        lineno,
        line: line.to_string(),
        source,
    })?;
    Ok((program, pps, mean_ns))
}

/// Render a list of breaches as multi-line stderr text.
///
/// Each breach produces a paragraph naming the program, the kind,
/// and the triggering numbers — no surrounding `error:` framing
/// because the caller adds its own.
#[must_use]
pub fn render_failure(breaches: &[Breach]) -> String {
    let mut out = String::new();
    for breach in breaches {
        match breach.kind {
            BreachKind::PpsRegression { threshold_fraction } => {
                let _ = writeln!(
                    out,
                    "  {prog}: pps regressed by {got:.1}% (baseline={base:.0}, measured={meas:.0}, threshold=>{thr:.0}%)",
                    prog = breach.program,
                    got = breach.pps_drop_fraction * 100.0,
                    base = breach.baseline_pps,
                    meas = breach.measured_pps,
                    thr = threshold_fraction * 100.0,
                );
            }
            BreachKind::MeanLatencyRise { threshold_fraction } => {
                let _ = writeln!(
                    out,
                    "  {prog}: mean latency rose by {got:.1}% (baseline={base}ns, measured={meas}ns, threshold=>{thr:.0}%)",
                    prog = breach.program,
                    got = breach.mean_growth_fraction * 100.0,
                    base = breach.baseline_mean_ns,
                    meas = breach.measured_mean_ns,
                    thr = threshold_fraction * 100.0,
                );
            }
            BreachKind::MissingFromCandidates => {
                let _ = writeln!(
                    out,
                    "  {prog}: missing from measured candidates (baseline names a program the BPF object does not contain)",
                    prog = breach.program,
                );
            }
        }
    }
    out
}

/// Render measured records in the canonical baseline-file shape, one
/// per line. Suitable for re-recording baselines after a deliberate
/// regression-acknowledgement merge: copy stderr → baseline file.
#[must_use]
pub fn render_measured(records: &[MeasuredRecord]) -> String {
    let mut out = String::new();
    for r in records {
        let _ = writeln!(
            out,
            "  prog={prog} pps={pps} mean_ns={mean}",
            prog = r.program,
            pps = r.pps,
            mean = r.mean_ns,
        );
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::single_char_pattern, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn evaluate_passes_when_candidates_match_baseline_exactly() {
        let baselines = vec![BaselineRecord {
            program: "xdp_service_map_lookup".to_string(),
            pps: 5_000_000.0,
            mean_ns: 200,
        }];
        let candidates = vec![MeasuredRecord {
            program: "xdp_service_map_lookup".to_string(),
            pps: 5_000_000.0,
            mean_ns: 200,
        }];
        assert_eq!(evaluate(&baselines, &candidates, &GatePolicy::default()), GateOutcome::Pass,);
    }

    #[test]
    fn evaluate_fails_when_pps_drops_more_than_threshold() {
        let baselines =
            vec![BaselineRecord { program: "p".to_string(), pps: 1_000_000.0, mean_ns: 100 }];
        // 6% drop — over the default 5% threshold.
        let candidates =
            vec![MeasuredRecord { program: "p".to_string(), pps: 940_000.0, mean_ns: 100 }];
        let outcome = evaluate(&baselines, &candidates, &GatePolicy::default());
        let breaches = match outcome {
            GateOutcome::Fail { breaches } => breaches,
            GateOutcome::Pass => panic!("6% pps drop must trip the >5% gate"),
        };
        assert_eq!(breaches.len(), 1);
        assert!(matches!(breaches[0].kind, BreachKind::PpsRegression { .. }));
    }

    #[test]
    fn evaluate_passes_when_pps_drops_within_threshold() {
        let baselines =
            vec![BaselineRecord { program: "p".to_string(), pps: 1_000_000.0, mean_ns: 100 }];
        // 4% drop — under the default 5% threshold.
        let candidates =
            vec![MeasuredRecord { program: "p".to_string(), pps: 960_000.0, mean_ns: 100 }];
        assert_eq!(evaluate(&baselines, &candidates, &GatePolicy::default()), GateOutcome::Pass,);
    }

    #[test]
    fn evaluate_fails_when_mean_grows_more_than_threshold() {
        let baselines = vec![BaselineRecord { program: "p".to_string(), pps: 1.0, mean_ns: 100 }];
        // 12% rise — over the default 10% threshold.
        let candidates = vec![MeasuredRecord { program: "p".to_string(), pps: 1.0, mean_ns: 112 }];
        let outcome = evaluate(&baselines, &candidates, &GatePolicy::default());
        let breaches = match outcome {
            GateOutcome::Fail { breaches } => breaches,
            GateOutcome::Pass => panic!("12% mean rise must trip the >10% gate"),
        };
        assert_eq!(breaches.len(), 1);
        assert!(matches!(breaches[0].kind, BreachKind::MeanLatencyRise { .. }));
    }

    #[test]
    fn evaluate_passes_when_mean_grows_within_threshold() {
        let baselines = vec![BaselineRecord { program: "p".to_string(), pps: 1.0, mean_ns: 100 }];
        // 9% rise — under the default 10% threshold.
        let candidates = vec![MeasuredRecord { program: "p".to_string(), pps: 1.0, mean_ns: 109 }];
        assert_eq!(evaluate(&baselines, &candidates, &GatePolicy::default()), GateOutcome::Pass,);
    }

    #[test]
    fn evaluate_fails_when_baseline_program_missing_from_candidates() {
        let baselines =
            vec![BaselineRecord { program: "ghost".to_string(), pps: 1.0, mean_ns: 100 }];
        let outcome = evaluate(&baselines, &[], &GatePolicy::default());
        let breaches = match outcome {
            GateOutcome::Fail { breaches } => breaches,
            GateOutcome::Pass => panic!("missing program must always be a breach"),
        };
        assert_eq!(breaches.len(), 1);
        assert!(matches!(breaches[0].kind, BreachKind::MissingFromCandidates));
    }

    #[test]
    fn evaluate_ignores_candidates_without_a_baseline() {
        // New programs are baselined in their introducing commit; the
        // gate must not fire on a candidate present without a baseline.
        let candidates =
            vec![MeasuredRecord { program: "newcomer".to_string(), pps: 1.0, mean_ns: 100 }];
        assert_eq!(evaluate(&[], &candidates, &GatePolicy::default()), GateOutcome::Pass);
    }

    #[test]
    fn parse_baseline_file_skips_comments_and_blanks() {
        let text = "\
# header comment
#
# more headers


prog=p pps=1000 mean_ns=200
";
        let records = parse_baseline_file(text).expect("parse must succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].program, "p");
        assert!((records[0].pps - 1000.0).abs() < f64::EPSILON);
        assert_eq!(records[0].mean_ns, 200);
    }

    #[test]
    fn parse_baseline_file_extracts_multiple_data_lines() {
        let text = "\
prog=a pps=1000 mean_ns=100
prog=b pps=2000 mean_ns=50
";
        let records = parse_baseline_file(text).expect("parse must succeed");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].program, "a");
        assert_eq!(records[1].program, "b");
    }

    #[test]
    fn parse_baseline_file_rejects_line_missing_pps() {
        let text = "prog=p mean_ns=100\n";
        let err = parse_baseline_file(text).expect_err("missing pps must error");
        assert!(matches!(err, ParseError::MissingPps { .. }));
    }

    #[test]
    fn parse_baseline_file_rejects_line_missing_mean_ns() {
        let text = "prog=p pps=1000\n";
        let err = parse_baseline_file(text).expect_err("missing mean_ns must error");
        assert!(matches!(err, ParseError::MissingMeanNs { .. }));
    }

    #[test]
    fn parse_baseline_file_rejects_line_missing_prog() {
        let text = "pps=1000 mean_ns=100\n";
        let err = parse_baseline_file(text).expect_err("missing prog must error");
        assert!(matches!(err, ParseError::MissingProg { .. }));
    }

    #[test]
    fn parse_baseline_file_rejects_malformed_pps() {
        let text = "prog=p pps=not-a-number mean_ns=100\n";
        let err = parse_baseline_file(text).expect_err("malformed pps must error");
        assert!(matches!(err, ParseError::PpsParse { .. }));
    }

    #[test]
    fn parse_baseline_file_rejects_malformed_mean_ns() {
        let text = "prog=p pps=1000 mean_ns=oops\n";
        let err = parse_baseline_file(text).expect_err("malformed mean_ns must error");
        assert!(matches!(err, ParseError::MeanNsParse { .. }));
    }

    #[test]
    fn parse_baseline_file_ignores_unknown_keys() {
        // Metadata fields like `tool=` / `iface=` are valid in the
        // baseline file format but ignored at the gate.
        let text = "prog=p pps=1000 mean_ns=100 tool=aya iface=xdp0\n";
        let records = parse_baseline_file(text).expect("parse must succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].program, "p");
    }

    #[test]
    fn render_failure_pps_breach_includes_program_and_threshold() {
        let breaches = vec![Breach {
            program: "p".to_string(),
            baseline_pps: 1_000_000.0,
            baseline_mean_ns: 100,
            measured_pps: 900_000.0,
            measured_mean_ns: 100,
            pps_drop_fraction: 0.10,
            mean_growth_fraction: 0.0,
            kind: BreachKind::PpsRegression { threshold_fraction: 0.05 },
        }];
        let out = render_failure(&breaches);
        assert!(out.contains("p"), "must name program; got {out}");
        assert!(out.contains("10.0%") || out.contains("10%"), "must show drop; got {out}");
        assert!(out.contains("5"), "must reference threshold; got {out}");
    }

    #[test]
    fn render_measured_round_trips_through_parse_measured_output() {
        let records = vec![
            MeasuredRecord { program: "a".to_string(), pps: 1234.5, mean_ns: 50 },
            MeasuredRecord { program: "b".to_string(), pps: 6789.0, mean_ns: 100 },
        ];
        let rendered = render_measured(&records);
        let parsed = parse_measured_output(&rendered).expect("round-trip must succeed");
        assert_eq!(parsed, records);
    }
}
