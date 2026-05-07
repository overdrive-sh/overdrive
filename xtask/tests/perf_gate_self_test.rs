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

file=target/xtask/bpf-objects/overdrive_bpf.o prog=xdp_service_map_lookup verdict=success verified_insns=151379
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
file=target/xtask/bpf-objects/overdrive_bpf.o prog=xdp_service_map_lookup verdict=success verified_insns=151400
file=target/xtask/bpf-objects/overdrive_bpf.o prog=xdp_reverse_nat_lookup verdict=success verified_insns=148166
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
