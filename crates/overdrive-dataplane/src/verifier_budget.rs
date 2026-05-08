//! Tier 4 verifier-complexity regression gate — pure decision fn +
//! parsers. Mirror in shape to the `xdp_perf` gate that still lives in
//! xtask.
//!
//! # Architecture
//!
//! Three concerns, three pure functions, plus a shell-side wrapper in
//! `crates/overdrive-dataplane/src/bin/verifier_regress.rs`:
//!
//! 1. [`parse_baseline_file`] — line-by-line key=value parser that
//!    skips `#`-comment and blank lines; returns one
//!    [`BaselineRecord`] per data line.
//! 2. [`parse_measured_output`] — same key=value parser used to
//!    re-parse measurements written by the gate binary itself (kept as
//!    a separate entry point so a future format pivot — e.g. JSON dump
//!    of `bpf_prog_info` — only changes this fn and its call sites).
//! 3. [`evaluate`] — pure decision fn: given baselines + candidates +
//!    policy, returns [`GateOutcome::Pass`] or `Fail { breaches }`.
//!    No I/O, no subprocess. Self-tested at
//!    `crates/overdrive-dataplane/tests/integration/verifier_budget_gate.rs`.
//!
//! # Signal source
//!
//! Measurements come from aya's
//! [`ProgramInfo::verified_instruction_count`](aya::programs::ProgramInfo)
//! — kernel ≥5.16 reads `bpf_prog_info.verified_insns` via
//! `BPF_OBJ_GET_INFO_BY_FD`. This is the same field veristat surfaces
//! as its `TOTAL_INSNS` column; both come from the kernel verifier's
//! own accounting. We bypass libbpf-based tools because libbpf 1.0+
//! rejects aya's emitted ELFs on the legacy `maps` section
//! (`libbpf: elf: legacy map definitions in 'maps' section are not
//! supported by libbpf v1.0+` — aya issue #913, no opt-out exists).
//!
//! # Gate policy
//!
//! - **Growth gate**: fail if `measured > baseline * (1 + max_growth_fraction)`.
//!   Default `max_growth_fraction = 0.05` (>5%).
//! - **Ceiling-proximity gate**: fail if `measured >= ceiling *
//!   (1 - ceiling_proximity_fraction)`. Default
//!   `ceiling_proximity_fraction = 0.10`, `ceiling_insns = 1_000_000`
//!   (the kernel `CAP_BPF` complexity ceiling) — i.e. fail when
//!   measured is within 10% of the ceiling.
//!
//! Both clauses are evaluated per program; either tripping is a
//! breach. A program present in baselines but missing from candidates
//! is a separate `MissingProgram` breach. A candidate without a
//! baseline is *not* a breach — new programs are baselined in their
//! introducing commit.

#![allow(
    clippy::cast_precision_loss,
    reason = "verified_insns values are bounded by the 1M CAP_BPF ceiling, well below 2^53; all casts are exact-representable. See module docs."
)]

use std::fmt::Write as _;

use thiserror::Error;

/// One row of `perf-baseline/main/verifier-budget/<prog>.txt`.
///
/// Per-file format documented inline in the baseline files: leading
/// `#` comments + one or more `key=value` data lines. The parser keys
/// off `prog=` and `verified_insns=` — every other key (`file=`,
/// `verdict=`) is metadata and is ignored at the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineRecord {
    /// Program name (e.g. `xdp_service_map_lookup`). Matches the
    /// `prog=<name>` field in baseline files and in measured output.
    pub program: String,
    /// Verifier instruction count recorded in the baseline. Matches
    /// the `verified_insns=<N>` field.
    pub verified_insns: u64,
}

/// One row of measured output captured at gate time.
///
/// Same shape as [`BaselineRecord`] but kept as a distinct type so a
/// future signal-source pivot only touches [`parse_measured_output`]
/// and call sites, not the baseline-file surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredRecord {
    /// Program name (matches [`BaselineRecord::program`] for join).
    pub program: String,
    /// Measured verifier instruction count for this gate run.
    pub verified_insns: u64,
}

/// Threshold policy for the gate. `Default` matches the project rules:
/// >5% growth, >10% of 1M ceiling.
#[derive(Debug, Clone)]
pub struct GatePolicy {
    /// Fraction of the baseline above which growth is a breach.
    /// 0.05 = >5%.
    pub max_growth_fraction: f64,
    /// Per-program kernel complexity ceiling. `1_000_000` is the
    /// `CAP_BPF` ceiling for kernels 5.10+ (`.claude/rules/testing.md`
    /// § Tier 4 / `BPF_COMPLEXITY_LIMIT_INSNS`).
    pub ceiling_insns: u64,
    /// Fraction of the ceiling: a measurement at or above
    /// `ceiling_insns * (1 - ceiling_proximity_fraction)` is a
    /// breach. 0.10 = within 10% of the ceiling.
    pub ceiling_proximity_fraction: f64,
}

impl Default for GatePolicy {
    fn default() -> Self {
        Self {
            max_growth_fraction: 0.05,
            ceiling_insns: 1_000_000,
            ceiling_proximity_fraction: 0.10,
        }
    }
}

/// Why a program failed the gate. Each variant captures the
/// triggering numbers so the renderer can produce structured
/// per-breach messages without recomputing.
#[derive(Debug, Clone, PartialEq)]
pub enum BreachKind {
    /// `measured > baseline * (1 + max_growth_fraction)`.
    GrowthExceeded {
        /// Threshold fraction the policy was configured with (e.g.
        /// 0.05 for >5%); pinned in the breach so the renderer can
        /// echo it verbatim.
        threshold_fraction: f64,
    },
    /// `measured >= ceiling * (1 - ceiling_proximity_fraction)`.
    CeilingProximity {
        ceiling_insns: u64,
        /// Threshold fraction the policy was configured with (e.g.
        /// 0.10 for "within 10% of ceiling"); pinned for renderer
        /// symmetry with [`BreachKind::GrowthExceeded`].
        threshold_fraction: f64,
    },
    /// Baseline names a program the candidates do not — typically
    /// the BPF object dropped or renamed a program without updating
    /// the baseline directory. Always a breach: silent baseline
    /// rot is exactly what this gate exists to catch.
    MissingFromCandidates,
}

/// One per program that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Breach {
    pub program: String,
    pub baseline_insns: u64,
    /// Measured value when present. `0` for
    /// [`BreachKind::MissingFromCandidates`].
    pub measured_insns: u64,
    /// Growth ratio `(measured - baseline) / baseline`. Computed
    /// once and pinned so renderer / test assertions agree on the
    /// value. `0.0` for missing-from-candidates breaches.
    pub growth_fraction: f64,
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
                baseline_insns: baseline.verified_insns,
                measured_insns: 0,
                growth_fraction: 0.0,
                kind: BreachKind::MissingFromCandidates,
            });
            continue;
        };

        let baseline_f = baseline.verified_insns as f64;
        let measured_f = candidate.verified_insns as f64;
        let growth_fraction =
            if baseline.verified_insns == 0 { 0.0 } else { (measured_f - baseline_f) / baseline_f };

        if growth_fraction > policy.max_growth_fraction {
            breaches.push(Breach {
                program: baseline.program.clone(),
                baseline_insns: baseline.verified_insns,
                measured_insns: candidate.verified_insns,
                growth_fraction,
                kind: BreachKind::GrowthExceeded { threshold_fraction: policy.max_growth_fraction },
            });
            continue;
        }

        let ceiling_threshold =
            (policy.ceiling_insns as f64) * (1.0 - policy.ceiling_proximity_fraction);
        if measured_f >= ceiling_threshold {
            breaches.push(Breach {
                program: baseline.program.clone(),
                baseline_insns: baseline.verified_insns,
                measured_insns: candidate.verified_insns,
                growth_fraction,
                kind: BreachKind::CeilingProximity {
                    ceiling_insns: policy.ceiling_insns,
                    threshold_fraction: policy.ceiling_proximity_fraction,
                },
            });
        }
    }

    if breaches.is_empty() { GateOutcome::Pass } else { GateOutcome::Fail { breaches } }
}

/// Errors from the parsers. Distinct typed variants per failure mode
/// per `.claude/rules/development.md` § Errors.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("line {lineno}: missing `prog=<name>` in {line:?}")]
    MissingProg { lineno: usize, line: String },
    #[error("line {lineno}: missing `verified_insns=<N>` in {line:?}")]
    MissingVerifiedInsns { lineno: usize, line: String },
    #[error("line {lineno}: verified_insns={value:?} not a u64: {source}")]
    NotAU64 {
        lineno: usize,
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
}

pub type Result<T, E = ParseError> = std::result::Result<T, E>;

/// Parse a `perf-baseline/main/verifier-budget/<prog>.txt` file.
///
/// Skips `#`-comment lines and blank lines; every remaining line MUST
/// contain `prog=<name>` and `verified_insns=<N>` somewhere in its
/// space-separated key=value pairs. Other keys (`file=`, `verdict=`)
/// are ignored.
///
/// # Errors
///
/// Returns [`ParseError::MissingProg`], [`ParseError::MissingVerifiedInsns`],
/// or [`ParseError::NotAU64`] per the variant docs.
pub fn parse_baseline_file(text: &str) -> Result<Vec<BaselineRecord>> {
    let mut records = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (program, verified_insns) = parse_kv_line(line, lineno + 1)?;
        records.push(BaselineRecord { program, verified_insns });
    }
    Ok(records)
}

/// Parse measured output captured at gate time. Same shape as
/// [`parse_baseline_file`] today — see module-level docs for the
/// rationale for the separate name.
///
/// # Errors
///
/// Same as [`parse_baseline_file`].
pub fn parse_measured_output(text: &str) -> Result<Vec<MeasuredRecord>> {
    let mut records = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (program, verified_insns) = parse_kv_line(line, lineno + 1)?;
        records.push(MeasuredRecord { program, verified_insns });
    }
    Ok(records)
}

/// Parse one `key=value` data line into `(prog, verified_insns)`.
/// `lineno` is 1-based and used only for error context.
fn parse_kv_line(line: &str, lineno: usize) -> Result<(String, u64)> {
    let mut prog: Option<String> = None;
    let mut insns: Option<u64> = None;
    for token in line.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "prog" => prog = Some(value.to_string()),
            "verified_insns" => {
                let parsed: u64 = value.parse().map_err(|source| ParseError::NotAU64 {
                    lineno,
                    value: value.to_string(),
                    source,
                })?;
                insns = Some(parsed);
            }
            _ => {}
        }
    }
    let prog = prog.ok_or_else(|| ParseError::MissingProg { lineno, line: line.to_string() })?;
    let insns =
        insns.ok_or_else(|| ParseError::MissingVerifiedInsns { lineno, line: line.to_string() })?;
    Ok((prog, insns))
}

/// Render a [`GateOutcome::Fail`] as a structured human-readable
/// report. Format: program / metric / baseline / measured / threshold
/// per breach.
#[must_use]
pub fn render_failure(breaches: &[Breach]) -> String {
    let mut out = String::new();
    out.push_str("verifier-regress: gate failed — verifier-budget regression detected\n");
    for breach in breaches {
        match &breach.kind {
            BreachKind::GrowthExceeded { threshold_fraction } => {
                let _ = writeln!(
                    out,
                    "  • {} — verified_insns: baseline={}, measured={}, growth={:.2}% (threshold > {:.0}%)",
                    breach.program,
                    breach.baseline_insns,
                    breach.measured_insns,
                    breach.growth_fraction * 100.0,
                    threshold_fraction * 100.0,
                );
            }
            BreachKind::CeilingProximity { ceiling_insns, threshold_fraction } => {
                let proximity = (breach.measured_insns as f64) / (*ceiling_insns as f64) * 100.0;
                let _ = writeln!(
                    out,
                    "  • {} — verified_insns={} approaches ceiling {} ({:.2}% of ceiling; threshold ≥ {:.0}%)",
                    breach.program,
                    breach.measured_insns,
                    ceiling_insns,
                    proximity,
                    (1.0 - threshold_fraction) * 100.0,
                );
            }
            BreachKind::MissingFromCandidates => {
                let _ = writeln!(
                    out,
                    "  • {} — baseline names this program but the gate produced no measurement (baseline={}); silent baseline rot — re-baseline or remove the file",
                    breach.program, breach.baseline_insns,
                );
            }
        }
    }
    out
}

/// Render a measured-record list as the canonical key=value lines the
/// gate writes alongside the verdict (one line per program). Format
/// matches what [`parse_measured_output`] consumes — round-trip safe.
#[must_use]
pub fn render_measured(records: &[MeasuredRecord]) -> String {
    let mut out = String::new();
    for r in records {
        let _ = writeln!(out, "prog={} verified_insns={}", r.program, r.verified_insns);
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn evaluate_passes_when_candidates_match_baseline_exactly() {
        let baselines = vec![BaselineRecord { program: "x".to_string(), verified_insns: 1000 }];
        let candidates = vec![MeasuredRecord { program: "x".to_string(), verified_insns: 1000 }];
        let outcome = evaluate(&baselines, &candidates, &GatePolicy::default());
        assert!(matches!(outcome, GateOutcome::Pass));
    }

    #[test]
    fn evaluate_fails_when_baseline_program_missing_from_candidates() {
        let baselines =
            vec![BaselineRecord { program: "missing_prog".to_string(), verified_insns: 1000 }];
        let candidates: Vec<MeasuredRecord> = vec![];
        let outcome = evaluate(&baselines, &candidates, &GatePolicy::default());
        let breaches = match outcome {
            GateOutcome::Pass => panic!("missing program must fail"),
            GateOutcome::Fail { breaches } => breaches,
        };
        assert_eq!(breaches.len(), 1);
        assert!(matches!(breaches[0].kind, BreachKind::MissingFromCandidates));
    }

    #[test]
    fn parse_baseline_file_rejects_line_missing_verified_insns() {
        let text = "file=foo.o prog=bar verdict=success\n";
        let err = parse_baseline_file(text).expect_err("missing verified_insns must error");
        assert!(matches!(err, ParseError::MissingVerifiedInsns { .. }));
    }

    #[test]
    fn parse_baseline_file_rejects_line_missing_prog() {
        let text = "file=foo.o verdict=success verified_insns=42\n";
        let err = parse_baseline_file(text).expect_err("missing prog must error");
        assert!(matches!(err, ParseError::MissingProg { .. }));
    }

    #[test]
    fn render_measured_round_trips_through_parse() {
        let original = vec![
            MeasuredRecord { program: "a".to_string(), verified_insns: 100 },
            MeasuredRecord { program: "b".to_string(), verified_insns: 200 },
        ];
        let rendered = render_measured(&original);
        let parsed = parse_measured_output(&rendered).expect("round-trip");
        assert_eq!(parsed, original);
    }
}
