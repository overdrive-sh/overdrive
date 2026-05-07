//! `cargo xtask xdp-perf` — Tier 4 XDP throughput / p99 latency
//! regression gate.
//!
//! # Architecture
//!
//! Mirrors the [`super::verifier_regress`] split: three pure
//! functions (`parse_baseline_file`, `parse_xdp_bench_output`,
//! `evaluate`) plus a shell-side wrapper in
//! `xtask/src/main.rs::xdp_perf` that runs `xdp-bench` per mode,
//! captures stdout, parses to candidates, and calls [`evaluate`].
//!
//! 1. [`parse_baseline_file`] — line-by-line key=value parser that
//!    skips `#`-comment and blank lines; returns one
//!    [`BaselineRecord`] per data line. Format pinned by the
//!    documented shape in `perf-baseline/main/xdp-perf-<mode>.txt`.
//! 2. [`parse_xdp_bench_output`] — same shape parser for `xdp-bench`
//!    output captured at gate time. Today identical to the baseline
//!    parser — kept as a separately-named entry point so a future
//!    `xdp-bench` upgrade (e.g. a switch to `--json` output) only
//!    touches this fn.
//! 3. [`evaluate`] — pure decision fn: given baselines + candidates +
//!    policy, returns [`GateOutcome::Pass`] or `Fail { breaches }`.
//!    No I/O, no subprocess. Self-tested at
//!    `xtask/tests/perf_gate_self_test.rs`.
//!
//! # Why this gate is delta-only, never absolute
//!
//! Per `.claude/rules/testing.md` § "Tier 4 — Verifier and
//! Performance Gates":
//!
//! > Never gate on absolute numbers — runner hardware varies enough
//! > to make absolute gates flaky. Deltas only.
//!
//! Baselines record best-current-measurement on a known runner class;
//! the gate compares the current run to the recorded baseline as a
//! ratio. Hardware drift between runners stays out of the verdict.
//!
//! # Gate policy
//!
//! Per the slice-07 KPI:
//! - **PPS regression**: fail when
//!   `candidate.pps < baseline.pps * (1 - max_pps_regression_fraction)`.
//!   Default `max_pps_regression_fraction = 0.05` (>5% drop).
//! - **P99 latency rise**: fail when
//!   `candidate.p99 > baseline.p99 * (1 + max_p99_growth_fraction)`.
//!   Default `max_p99_growth_fraction = 0.10` (>10% rise).
//!
//! Both are evaluated per mode independently. Either tripping is a
//! breach. A baseline mode missing from candidates is a separate
//! [`BreachKind::MissingFromCandidates`] breach. A candidate mode
//! without a baseline is *not* a breach (new modes are baselined in
//! their introducing commit; absence simply means "no contract yet").
//!
//! # Numeric precision
//!
//! pps is stored as `f64` because xdp-bench reports fractional Mpps
//! (e.g. `4.7 Mpps`). The values stay well below 2^53; the
//! `clippy::cast_precision_loss` lint is allowed at the module level
//! for the same reason as `verifier_regress`. p99 latency is `u64`
//! nanoseconds.

#![allow(
    clippy::cast_precision_loss,
    reason = "p99_ns values are bounded by realistic kernel-scheduler ticks (sub-second), well below 2^53; pps values are bounded by single-NIC line rate, also well below 2^53. All casts are exact-representable."
)]

use std::fmt::Write as _;
use std::str::FromStr;

use color_eyre::eyre::{Result, WrapErr, eyre};

/// XDP-bench mode: one of three modes the slice-07 KPI exercises.
///
/// - `Drop` — `XDP_DROP` fast path; no map lookup, no rewrite. Tests
///   the verifier+JIT cost of the program prologue alone.
/// - `Tx` — `XDP_TX` bounce-back. Tests the per-packet rewrite
///   (Ethernet/IP/L4 mutate + checksum) without forwarding-table
///   logic.
/// - `LbForward` — full LB-forward path: `SERVICE_MAP` lookup +
///   `MAGLEV_MAP` slot derivation + `BACKEND_MAP` backend resolution +
///   header rewrite + L4 csum + `bpf_fib_lookup` + L2 MAC rewrite +
///   `bpf_redirect_neigh`. The end-to-end production hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BenchMode {
    Drop,
    Tx,
    LbForward,
}

impl BenchMode {
    /// Canonical string form used in baseline files and `xdp-bench`
    /// output. Lowercase, hyphenated for multi-word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Tx => "tx",
            Self::LbForward => "lb-forward",
        }
    }
}

impl std::fmt::Display for BenchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BenchMode {
    type Err = color_eyre::eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "drop" => Ok(Self::Drop),
            "tx" => Ok(Self::Tx),
            "lb-forward" => Ok(Self::LbForward),
            other => {
                Err(eyre!("unknown mode {other:?}; expected one of `drop`, `tx`, `lb-forward`"))
            }
        }
    }
}

/// One row of `perf-baseline/main/xdp-perf-<mode>.txt`.
///
/// File-level format: leading `#` comments documenting purpose +
/// runner-class + recording date, followed by one space-separated
/// `key=value` data line carrying `mode=`, `pps=`, `p99_ns=`. Other
/// keys are accepted and ignored at the gate (room for future
/// metadata without breaking the parser contract).
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineRecord {
    pub mode: BenchMode,
    /// Packets-per-second baseline. Recorded as the `xdp-bench`
    /// average for the mode on the runner class named in the file's
    /// `# Recorded against:` header.
    pub pps: f64,
    /// p99 latency in nanoseconds. Recorded alongside pps; rises
    /// independently under the gate's second clause.
    pub p99_ns: u64,
}

/// One row of `xdp-bench` output captured at gate time.
///
/// Same shape as [`BaselineRecord`] today; kept as a distinct type so
/// a future xdp-bench output-format pivot only touches
/// [`parse_xdp_bench_output`] and call sites, not the baseline-file
/// surface.
#[derive(Debug, Clone, PartialEq)]
pub struct XdpBenchRecord {
    pub mode: BenchMode,
    pub pps: f64,
    pub p99_ns: u64,
}

/// Threshold policy for the gate. `Default` matches the slice-07
/// KPI: >5% pps drop, >10% p99 latency rise.
#[derive(Debug, Clone)]
pub struct GatePolicy {
    /// Fraction of the baseline below which pps is a regression.
    /// 0.05 = >5% drop.
    pub max_pps_regression_fraction: f64,
    /// Fraction of the baseline above which p99 latency is a
    /// regression. 0.10 = >10% rise.
    pub max_p99_growth_fraction: f64,
}

impl Default for GatePolicy {
    fn default() -> Self {
        Self { max_pps_regression_fraction: 0.05, max_p99_growth_fraction: 0.10 }
    }
}

/// Why a mode failed the gate. Each variant captures the triggering
/// numbers so the renderer can produce structured per-breach messages
/// without recomputing.
#[derive(Debug, Clone, PartialEq)]
pub enum BreachKind {
    /// `candidate.pps < baseline.pps * (1 - max_pps_regression_fraction)`.
    PpsRegression {
        /// Threshold fraction the policy was configured with (e.g.
        /// 0.05 for >5%); pinned in the breach so the renderer can
        /// echo it verbatim per slice-07's structured-output KPI.
        threshold_fraction: f64,
    },
    /// `candidate.p99 > baseline.p99 * (1 + max_p99_growth_fraction)`.
    P99Regression { threshold_fraction: f64 },
    /// Baseline names a mode the candidates do not — the xdp-bench
    /// harness dropped a mode without the baseline being updated.
    /// Always a breach: silent baseline rot is exactly what this gate
    /// exists to catch.
    MissingFromCandidates,
}

/// One per mode that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Breach {
    pub mode: BenchMode,
    pub baseline_pps: f64,
    pub candidate_pps: f64,
    pub baseline_p99_ns: u64,
    pub candidate_p99_ns: u64,
    /// Pps regression ratio `(baseline - candidate) / baseline`.
    /// Positive when candidate pps dropped. `0.0` for missing-from
    /// -candidates and p99-only breaches.
    pub regression_fraction: f64,
    /// p99 growth ratio `(candidate - baseline) / baseline`.
    /// Positive when p99 grew. `0.0` for missing-from-candidates and
    /// pps-only breaches.
    pub p99_growth_fraction: f64,
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

/// Pure gate decision. Per-step contract pinned by
/// `xtask/tests/perf_gate_self_test.rs`:
/// - 6% pps regression in LB-forward (5.0 → 4.7 Mpps) above the 5%
///   threshold ⇒ Fail with one [`BreachKind::PpsRegression`] breach.
/// - 2% pps regression below the threshold ⇒ Pass.
/// - 12.5% p99 rise above the 10% threshold ⇒ Fail with one
///   [`BreachKind::P99Regression`] breach.
/// - Baseline mode missing from candidates ⇒ Fail with one
///   [`BreachKind::MissingFromCandidates`] breach.
pub fn evaluate(
    baselines: &[BaselineRecord],
    candidates: &[XdpBenchRecord],
    policy: &GatePolicy,
) -> GateOutcome {
    let mut breaches: Vec<Breach> = Vec::new();

    for baseline in baselines {
        let candidate = candidates.iter().find(|c| c.mode == baseline.mode);
        let Some(candidate) = candidate else {
            breaches.push(Breach {
                mode: baseline.mode,
                baseline_pps: baseline.pps,
                candidate_pps: 0.0,
                baseline_p99_ns: baseline.p99_ns,
                candidate_p99_ns: 0,
                regression_fraction: 0.0,
                p99_growth_fraction: 0.0,
                kind: BreachKind::MissingFromCandidates,
            });
            continue;
        };

        // PPS regression: candidate dropped below threshold
        // (regression_fraction > policy.max_pps_regression_fraction).
        let regression_fraction =
            if baseline.pps == 0.0 { 0.0 } else { (baseline.pps - candidate.pps) / baseline.pps };
        let p99_growth_fraction = if baseline.p99_ns == 0 {
            0.0
        } else {
            ((candidate.p99_ns as f64) - (baseline.p99_ns as f64)) / (baseline.p99_ns as f64)
        };

        if regression_fraction > policy.max_pps_regression_fraction {
            breaches.push(Breach {
                mode: baseline.mode,
                baseline_pps: baseline.pps,
                candidate_pps: candidate.pps,
                baseline_p99_ns: baseline.p99_ns,
                candidate_p99_ns: candidate.p99_ns,
                regression_fraction,
                p99_growth_fraction,
                kind: BreachKind::PpsRegression {
                    threshold_fraction: policy.max_pps_regression_fraction,
                },
            });
            continue;
        }

        if p99_growth_fraction > policy.max_p99_growth_fraction {
            breaches.push(Breach {
                mode: baseline.mode,
                baseline_pps: baseline.pps,
                candidate_pps: candidate.pps,
                baseline_p99_ns: baseline.p99_ns,
                candidate_p99_ns: candidate.p99_ns,
                regression_fraction,
                p99_growth_fraction,
                kind: BreachKind::P99Regression {
                    threshold_fraction: policy.max_p99_growth_fraction,
                },
            });
        }
    }

    if breaches.is_empty() { GateOutcome::Pass } else { GateOutcome::Fail { breaches } }
}

/// Parse a `perf-baseline/main/xdp-perf-<mode>.txt` file.
///
/// Skips `#`-comment and blank lines; every remaining line MUST
/// contain `mode=<drop|tx|lb-forward>`, `pps=<f64>`, and
/// `p99_ns=<u64>` somewhere in its space-separated key=value pairs.
/// Other keys are accepted and ignored.
///
/// Strict on:
/// - Missing `mode=` ⇒ error.
/// - Unknown mode value ⇒ error (via [`BenchMode::from_str`]).
/// - Missing `pps=` ⇒ error.
/// - Missing `p99_ns=` ⇒ error.
/// - Non-f64 `pps=` ⇒ error.
/// - Non-u64 `p99_ns=` ⇒ error.
pub fn parse_baseline_file(text: &str) -> Result<Vec<BaselineRecord>> {
    let mut records = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (mode, pps, p99_ns) = parse_kv_line(line, lineno + 1)?;
        records.push(BaselineRecord { mode, pps, p99_ns });
    }
    Ok(records)
}

/// Parse `xdp-bench` output captured at gate time. Same shape as
/// [`parse_baseline_file`] today — see module-level docs for the
/// rationale for the separate name.
pub fn parse_xdp_bench_output(text: &str) -> Result<Vec<XdpBenchRecord>> {
    let mut records = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (mode, pps, p99_ns) = parse_kv_line(line, lineno + 1)?;
        records.push(XdpBenchRecord { mode, pps, p99_ns });
    }
    Ok(records)
}

/// Parse one `key=value` data line into `(mode, pps, p99_ns)`.
/// `lineno` is 1-based and used only for error context.
fn parse_kv_line(line: &str, lineno: usize) -> Result<(BenchMode, f64, u64)> {
    let mut mode: Option<BenchMode> = None;
    let mut pps: Option<f64> = None;
    let mut p99_ns: Option<u64> = None;
    for token in line.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "mode" => {
                let parsed: BenchMode =
                    value.parse().wrap_err_with(|| format!("line {lineno}: mode={value:?}"))?;
                mode = Some(parsed);
            }
            "pps" => {
                let parsed: f64 = value
                    .parse()
                    .wrap_err_with(|| format!("line {lineno}: pps={value:?} not a f64"))?;
                pps = Some(parsed);
            }
            "p99_ns" => {
                let parsed: u64 = value
                    .parse()
                    .wrap_err_with(|| format!("line {lineno}: p99_ns={value:?} not a u64"))?;
                p99_ns = Some(parsed);
            }
            _ => {}
        }
    }
    let mode = mode.ok_or_else(|| eyre!("line {lineno}: missing `mode=<...>` in {line:?}"))?;
    let pps = pps.ok_or_else(|| eyre!("line {lineno}: missing `pps=<f64>` in {line:?}"))?;
    let p99_ns =
        p99_ns.ok_or_else(|| eyre!("line {lineno}: missing `p99_ns=<u64>` in {line:?}"))?;
    Ok((mode, pps, p99_ns))
}

/// Render a [`GateOutcome::Fail`] as a structured human-readable
/// report. Format pinned by slice-07: mode / metric / baseline /
/// measured / threshold per breach.
pub fn render_failure(breaches: &[Breach]) -> String {
    let mut out = String::new();
    out.push_str("xdp-perf: gate failed — pps / p99 regression detected\n");
    for breach in breaches {
        match &breach.kind {
            BreachKind::PpsRegression { threshold_fraction } => {
                let _ = writeln!(
                    out,
                    "  • mode={} — pps: baseline={:.2} Mpps, measured={:.2} Mpps, regression={:.2}% (threshold > {:.0}%)",
                    breach.mode,
                    breach.baseline_pps / 1_000_000.0,
                    breach.candidate_pps / 1_000_000.0,
                    breach.regression_fraction * 100.0,
                    threshold_fraction * 100.0,
                );
            }
            BreachKind::P99Regression { threshold_fraction } => {
                let _ = writeln!(
                    out,
                    "  • mode={} — p99: baseline={} ns, measured={} ns, growth={:.2}% (threshold > {:.0}%)",
                    breach.mode,
                    breach.baseline_p99_ns,
                    breach.candidate_p99_ns,
                    breach.p99_growth_fraction * 100.0,
                    threshold_fraction * 100.0,
                );
            }
            BreachKind::MissingFromCandidates => {
                let _ = writeln!(
                    out,
                    "  • mode={} — baseline names this mode but xdp-bench output does not (baseline pps={:.2} Mpps, p99={} ns); silent baseline rot — re-baseline or remove the file",
                    breach.mode,
                    breach.baseline_pps / 1_000_000.0,
                    breach.baseline_p99_ns,
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_passes_when_candidates_match_baseline_exactly() {
        let baselines =
            vec![BaselineRecord { mode: BenchMode::Drop, pps: 10_000_000.0, p99_ns: 2_000 }];
        let candidates =
            vec![XdpBenchRecord { mode: BenchMode::Drop, pps: 10_000_000.0, p99_ns: 2_000 }];
        let outcome = evaluate(&baselines, &candidates, &GatePolicy::default());
        assert!(matches!(outcome, GateOutcome::Pass));
    }

    #[test]
    fn bench_mode_round_trips_via_string() {
        for mode in [BenchMode::Drop, BenchMode::Tx, BenchMode::LbForward] {
            let s = mode.as_str();
            let parsed: BenchMode = s.parse().expect("round-trip parse");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn bench_mode_rejects_unknown() {
        let err = "bogus".parse::<BenchMode>().expect_err("unknown mode must error");
        assert!(format!("{err:?}").contains("bogus"));
    }
}
