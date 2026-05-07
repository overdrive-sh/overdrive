//! Self-test for `cargo xtask verifier-regress`'s gate logic.
//!
//! Per slice-07's KPI: the gate's own correctness check, not a
//! measurement. The xtask subcommand returns non-zero on a synthetic
//! greater-than-5% regression input, zero on a synthetic 2% input.
//! This file pins both sides.
//!
//! Architecture (per step 07-01):
//! - The gate's pure decision fn lives in `xtask::perf_gate::verifier_regress::evaluate`.
//! - The binary `verifier_regress()` in `main.rs` is the shell-side
//!   wiring (find baselines on disk, run veristat, parse output,
//!   render) and calls `evaluate` for the verdict.
//! - This self-test calls `evaluate` directly with synthetic inputs
//!   so it runs on macOS and Linux without veristat installed.
//!
//! Gated behind the `integration-tests` feature per the workspace
//! convention in `.claude/rules/testing.md` § "Integration vs unit
//! gating". The xtask integration-tests feature already gates the
//! sibling `dst_lint_self_test.rs` and `acceptance.rs` binaries.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

use xtask::perf_gate::verifier_regress::{
    BaselineRecord, GateOutcome, GatePolicy, VeristatRecord, evaluate, parse_baseline_file,
    parse_veristat_output,
};

// Step 07-02 — xdp-perf gate's pure decision fn lives in
// `xtask::perf_gate::xdp_perf`. Same architecture as verifier-regress:
// the binary `xdp_perf()` in `main.rs` handles the I/O (running
// `xdp-bench` per mode, parsing stdout); the pure `evaluate` fn here
// is the gate logic that runs anywhere with synthetic inputs. The
// shape mirrors verifier-regress one-to-one but tracks two metrics
// per record (pps, p99 latency) keyed by mode (Drop / Tx / LbForward)
// rather than a single instruction count keyed by program name.
use xtask::perf_gate::xdp_perf::{
    BaselineRecord as XdpBaselineRecord, BenchMode, BreachKind as XdpBreachKind,
    GateOutcome as XdpGateOutcome, GatePolicy as XdpGatePolicy, XdpBenchRecord,
    evaluate as xdp_evaluate, parse_baseline_file as parse_xdp_baseline_file,
    parse_xdp_bench_output,
};

/// Slice-07 KPI pinned: a synthetic 12% growth (5000 → 5600) MUST
/// trip the gate (`evaluate` returns `GateOutcome::Fail`). The
/// shell-side `verifier_regress()` translates `Fail` into a non-zero
/// exit, but this test asserts on the structured verdict directly so
/// it can run anywhere (no veristat required).
#[test]
fn verifier_regress_returns_nonzero_on_synthetic_twelve_percent_growth() {
    let baselines =
        vec![BaselineRecord { program: "synthetic_program".to_string(), verified_insns: 5000 }];
    let candidates = vec![VeristatRecord {
        program: "synthetic_program".to_string(),
        verified_insns: 5600, // +12% vs baseline
    }];
    let policy = GatePolicy::default();

    let outcome = evaluate(&baselines, &candidates, &policy);

    let breaches = match outcome {
        GateOutcome::Pass => panic!("12% growth (5000 -> 5600) MUST fail the >5% gate; got Pass"),
        GateOutcome::Fail { breaches } => breaches,
    };
    assert_eq!(breaches.len(), 1, "expected exactly one breach, got {breaches:?}");
    let breach = &breaches[0];
    assert_eq!(breach.program, "synthetic_program");
    assert_eq!(breach.baseline_insns, 5000);
    assert_eq!(breach.measured_insns, 5600);
    // Growth ratio: (5600 - 5000) / 5000 = 0.12 = 12.0%.
    assert!(
        (breach.growth_fraction - 0.12).abs() < 1e-9,
        "growth_fraction must be 0.12, got {}",
        breach.growth_fraction
    );
}

/// Baseline-file parser MUST skip `#`-prefixed comments and blank
/// lines, and extract `prog=<name>` and `verified_insns=<N>` from
/// the single `key=value` data line. This pins the parser contract
/// against the literal shape of `veristat-service-map.txt`.
#[test]
fn parse_baseline_file_extracts_prog_and_insns_from_real_baseline_shape() {
    // Excerpt of the real veristat-service-map.txt format — leading
    // comments are skipped; the data line uses space-separated
    // key=value pairs and is the only non-comment, non-blank line.
    let text = "\
# Verifier-budget baseline — xdp_service_map_lookup
#
# History: ...
#

file=target/bpf/overdrive_bpf.o prog=xdp_service_map_lookup verdict=success verified_insns=151379
";

    let records = parse_baseline_file(text).expect("parse_baseline_file must succeed");
    assert_eq!(records.len(), 1, "expected exactly one record, got {records:?}");
    let r = &records[0];
    assert_eq!(r.program, "xdp_service_map_lookup");
    assert_eq!(r.verified_insns, 151_379);
}

/// veristat output parser MUST extract program name + `verified_insns`
/// from the same `key=value` shape veristat emits in `-o csv`-equivalent
/// or our project's recorded baseline shape. The parser is shared
/// between baseline-file and veristat-output paths because both use
/// the same line format.
#[test]
fn parse_veristat_output_extracts_one_record_per_program() {
    let text = "\
file=target/bpf/overdrive_bpf.o prog=xdp_service_map_lookup verdict=success verified_insns=151400
file=target/bpf/overdrive_bpf.o prog=xdp_reverse_nat_lookup verdict=success verified_insns=148166
";
    let records = parse_veristat_output(text).expect("parse_veristat_output must succeed");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].program, "xdp_service_map_lookup");
    assert_eq!(records[0].verified_insns, 151_400);
    assert_eq!(records[1].program, "xdp_reverse_nat_lookup");
    assert_eq!(records[1].verified_insns, 148_166);
}

/// Ceiling-proximity gate: a measurement that approaches the
/// per-program complexity ceiling by >10% (i.e. measured ≥ 90% of
/// the 1M `CAP_BPF` ceiling = `900_000` insns) MUST trip the gate
/// regardless of growth-fraction. Pins the second clause of AC 4
/// per slice-07.
#[test]
fn verifier_regress_fails_when_measured_approaches_ceiling_within_ten_percent() {
    let baselines =
        vec![BaselineRecord { program: "ceiling_hugger".to_string(), verified_insns: 880_000 }];
    let candidates = vec![VeristatRecord {
        program: "ceiling_hugger".to_string(),
        // Growth ratio: (920_000 - 880_000) / 880_000 ≈ 4.5% — UNDER
        // the >5% growth threshold. But 920_000 / 1_000_000 = 92% of
        // the 1M ceiling — within 10% of the ceiling, which the
        // ceiling-proximity clause MUST trip.
        verified_insns: 920_000,
    }];
    let policy = GatePolicy::default();

    let outcome = evaluate(&baselines, &candidates, &policy);
    let breaches = match outcome {
        GateOutcome::Pass => {
            panic!("920_000 / 1_000_000 = 92% (within 10% of ceiling) MUST fail; got Pass")
        }
        GateOutcome::Fail { breaches } => breaches,
    };
    assert_eq!(breaches.len(), 1);
    assert_eq!(breaches[0].measured_insns, 920_000);
}

/// Companion to the 12%-growth test: a synthetic 2% growth (5000 →
/// 5100) MUST pass the gate (`evaluate` returns `GateOutcome::Pass`).
/// Together the two tests form the slice-07 KPI's two-sided
/// invariant: the gate fires above the threshold AND stays silent
/// below it.
#[test]
fn verifier_regress_returns_zero_on_synthetic_two_percent_growth() {
    let baselines =
        vec![BaselineRecord { program: "synthetic_program".to_string(), verified_insns: 5000 }];
    let candidates = vec![VeristatRecord {
        program: "synthetic_program".to_string(),
        verified_insns: 5100, // +2% vs baseline
    }];
    let policy = GatePolicy::default();

    let outcome = evaluate(&baselines, &candidates, &policy);
    assert!(
        matches!(outcome, GateOutcome::Pass),
        "2% growth (5000 -> 5100) MUST pass the >5% gate; got {outcome:?}"
    );
}

// ---------------------------------------------------------------------
// xdp-perf gate self-tests (step 07-02)
// ---------------------------------------------------------------------
//
// Slice-07 KPI pinned: a synthetic 6% pps regression
// (5.0 → 4.7 Mpps) MUST trip the gate; a synthetic 2% pps regression
// (5.0 → 4.9 Mpps) MUST pass. Per `.claude/rules/testing.md` § Tier 4,
// the gate is RELATIVE-DELTA only — never absolute pps. The threshold
// is 5% pps regression (drop in throughput) and 10% p99 latency growth
// (rise in latency); both are independently evaluated per mode.

/// Slice-07 KPI: synthetic 6% pps regression in LB-forward mode
/// (5.0 → 4.7 Mpps) MUST fail the >5% gate. Tests the structured
/// breach output names both numbers and the threshold so the
/// renderer / human-readable surface is pinned.
#[test]
fn xdp_perf_returns_nonzero_on_synthetic_six_percent_pps_regression() {
    let baselines = vec![XdpBaselineRecord {
        mode: BenchMode::LbForward,
        pps: 5_000_000.0, // 5.0 Mpps
        p99_ns: 2_400,
    }];
    let candidates = vec![XdpBenchRecord {
        mode: BenchMode::LbForward,
        pps: 4_700_000.0, // 4.7 Mpps — 6% drop vs baseline
        p99_ns: 2_400,    // p99 unchanged so the breach is unambiguously the pps clause
    }];
    let policy = XdpGatePolicy::default();

    let outcome = xdp_evaluate(&baselines, &candidates, &policy);

    let breaches = match outcome {
        XdpGateOutcome::Pass => {
            panic!("6% pps regression (5.0 -> 4.7 Mpps) MUST fail the >5% gate; got Pass")
        }
        XdpGateOutcome::Fail { breaches } => breaches,
    };
    assert_eq!(breaches.len(), 1, "expected exactly one breach, got {breaches:?}");
    let breach = &breaches[0];
    assert_eq!(breach.mode, BenchMode::LbForward);
    assert!(
        matches!(breach.kind, XdpBreachKind::PpsRegression { .. }),
        "expected PpsRegression breach, got {:?}",
        breach.kind
    );
    // The breach MUST carry both numbers (5.0 Mpps baseline,
    // 4.7 Mpps measured) and the 5% threshold so the renderer can
    // emit them verbatim per slice-07's structured-output KPI.
    assert!((breach.baseline_pps - 5_000_000.0).abs() < 1e-3);
    assert!((breach.candidate_pps - 4_700_000.0).abs() < 1e-3);
    if let XdpBreachKind::PpsRegression { threshold_fraction } = breach.kind {
        assert!(
            (threshold_fraction - 0.05).abs() < 1e-9,
            "threshold_fraction must be 0.05 (5%), got {threshold_fraction}"
        );
    }
    // Regression fraction: (5.0 - 4.7) / 5.0 = 0.06 = 6%.
    assert!(
        (breach.regression_fraction - 0.06).abs() < 1e-9,
        "regression_fraction must be 0.06, got {}",
        breach.regression_fraction
    );
}

/// Companion to the 6%-regression test: a synthetic 2% pps regression
/// (5.0 → 4.9 Mpps) MUST pass the gate. Together the two tests form
/// the slice-07 KPI's two-sided invariant: the gate fires above the
/// threshold AND stays silent below it.
#[test]
fn xdp_perf_returns_zero_on_synthetic_two_percent_pps_regression() {
    let baselines =
        vec![XdpBaselineRecord { mode: BenchMode::LbForward, pps: 5_000_000.0, p99_ns: 2_400 }];
    let candidates = vec![XdpBenchRecord {
        mode: BenchMode::LbForward,
        pps: 4_900_000.0, // 2% regression — below the 5% threshold
        p99_ns: 2_400,
    }];
    let policy = XdpGatePolicy::default();

    let outcome = xdp_evaluate(&baselines, &candidates, &policy);
    assert!(
        matches!(outcome, XdpGateOutcome::Pass),
        "2% regression (5.0 -> 4.9 Mpps) MUST pass the >5% gate; got {outcome:?}"
    );
}

/// p99 latency clause: a >10% rise in p99 (2400 → 2700 ns ≈ +12.5%)
/// MUST trip the gate even when pps is unchanged. Pins the second
/// half of the policy per `.claude/rules/testing.md` § Tier 4 (pps
/// within 5%, p99 within 10%).
#[test]
fn xdp_perf_fails_when_p99_latency_rises_above_ten_percent() {
    let baselines =
        vec![XdpBaselineRecord { mode: BenchMode::Drop, pps: 10_000_000.0, p99_ns: 2_400 }];
    let candidates = vec![XdpBenchRecord {
        mode: BenchMode::Drop,
        pps: 10_000_000.0, // pps unchanged
        p99_ns: 2_700,     // +12.5% vs baseline
    }];
    let policy = XdpGatePolicy::default();

    let outcome = xdp_evaluate(&baselines, &candidates, &policy);
    let breaches = match outcome {
        XdpGateOutcome::Pass => panic!("p99 12.5% rise (2400 -> 2700) MUST fail the >10% gate"),
        XdpGateOutcome::Fail { breaches } => breaches,
    };
    assert_eq!(breaches.len(), 1);
    assert!(
        matches!(breaches[0].kind, XdpBreachKind::P99Regression { .. }),
        "expected P99Regression breach, got {:?}",
        breaches[0].kind
    );
}

/// Baseline-file parser for `perf-baseline/main/xdp-perf-<mode>.txt`
/// MUST skip `#`-comment + blank lines and extract `mode=`, `pps=`,
/// and `p99_ns=` from the single space-separated key=value data line.
#[test]
fn parse_xdp_baseline_file_extracts_mode_pps_and_p99() {
    let text = "\
# XDP perf baseline — LB-forward mode
# (placeholder values seeded by step 07-02; first follow-on PR
# overwrites with real measurements)

mode=lb-forward pps=5000000 p99_ns=2400
";

    let records = parse_xdp_baseline_file(text).expect("parse_xdp_baseline_file must succeed");
    assert_eq!(records.len(), 1, "expected exactly one record, got {records:?}");
    let r = &records[0];
    assert_eq!(r.mode, BenchMode::LbForward);
    assert!((r.pps - 5_000_000.0).abs() < 1e-3);
    assert_eq!(r.p99_ns, 2_400);
}

/// xdp-bench output parser: same key=value shape as the baseline
/// file. Project canonical line:
/// `mode=<drop|tx|lb-forward> pps=<f64> p99_ns=<u64>`.
#[test]
fn parse_xdp_bench_output_extracts_one_record_per_mode() {
    let text = "\
mode=drop pps=12500000 p99_ns=1800
mode=tx pps=8200000 p99_ns=2100
mode=lb-forward pps=4750000 p99_ns=2450
";
    let records = parse_xdp_bench_output(text).expect("parse_xdp_bench_output must succeed");
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].mode, BenchMode::Drop);
    assert_eq!(records[1].mode, BenchMode::Tx);
    assert_eq!(records[2].mode, BenchMode::LbForward);
    assert_eq!(records[2].p99_ns, 2_450);
}

/// Missing-mode breach: the baseline names a mode the candidates do
/// not (xdp-bench harness dropped a mode without the baseline being
/// updated). MUST always be a breach — silent baseline rot is exactly
/// what this gate exists to catch (mirror of verifier-regress's
/// `MissingFromCandidates` path).
#[test]
fn xdp_perf_fails_when_baseline_mode_missing_from_candidates() {
    let baselines =
        vec![XdpBaselineRecord { mode: BenchMode::LbForward, pps: 5_000_000.0, p99_ns: 2_400 }];
    let candidates: Vec<XdpBenchRecord> = vec![];
    let outcome = xdp_evaluate(&baselines, &candidates, &XdpGatePolicy::default());

    let breaches = match outcome {
        XdpGateOutcome::Pass => panic!("missing mode must fail the gate"),
        XdpGateOutcome::Fail { breaches } => breaches,
    };
    assert_eq!(breaches.len(), 1);
    assert!(matches!(breaches[0].kind, XdpBreachKind::MissingFromCandidates));
}

/// Parser MUST reject malformed input (missing `pps=` key) with a
/// structured error rather than silently producing a default record.
#[test]
fn parse_xdp_baseline_file_rejects_line_missing_pps() {
    let text = "mode=drop p99_ns=1800\n";
    let err = parse_xdp_baseline_file(text).expect_err("missing pps must error");
    let msg = format!("{err:?}");
    assert!(msg.contains("pps"), "error must name missing field; got {msg}");
}

/// Parser MUST reject unknown mode values with a structured error.
#[test]
fn parse_xdp_baseline_file_rejects_unknown_mode() {
    let text = "mode=bogus pps=1000 p99_ns=100\n";
    let err = parse_xdp_baseline_file(text).expect_err("unknown mode must error");
    let msg = format!("{err:?}");
    assert!(msg.contains("mode") || msg.contains("bogus"), "error must name mode; got {msg}");
}
