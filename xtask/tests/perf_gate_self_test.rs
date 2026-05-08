//! Self-test for `cargo xtask xdp-perf`'s gate logic.
//!
//! Per slice-07's KPI: the gate's own correctness check, not a
//! measurement. The xtask subcommand returns non-zero on a synthetic
//! greater-than-5% pps regression input, zero on a synthetic 2%
//! input. This file pins both sides.
//!
//! Architecture (per step 07-02):
//! - The gate's pure decision fn lives in `xtask::perf_gate::xdp_perf::evaluate`.
//! - The binary `xdp_perf()` in `main.rs` is the shell-side wiring
//!   (resolve iface, run `xdp-bench`, parse output, render) and calls
//!   `evaluate` for the verdict.
//! - This self-test calls `evaluate` directly with synthetic inputs
//!   so it runs on macOS and Linux without `xdp-bench` installed.
//!
//! The companion `verifier-regress` gate moved out of xtask to
//! `crates/overdrive-dataplane` (because it must load the BPF object
//! via aya, which xtask's deps cannot include per the xtask-purity
//! rule). Its self-tests live as unit tests in
//! `crates/overdrive-dataplane/src/verifier_budget.rs`.
//!
//! Gated behind the `integration-tests` feature per the workspace
//! convention in `.claude/rules/testing.md` § "Integration vs unit
//! gating". The xtask integration-tests feature already gates the
//! sibling `dst_lint_self_test.rs` and `acceptance.rs` binaries.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

use xtask::perf_gate::xdp_perf::{
    BaselineRecord as XdpBaselineRecord, BenchMode, BreachKind as XdpBreachKind,
    GateOutcome as XdpGateOutcome, GatePolicy as XdpGatePolicy, XdpBenchRecord,
    evaluate as xdp_evaluate, parse_baseline_file as parse_xdp_baseline_file,
    parse_xdp_bench_output,
};
// Iface-resolution helper for xdp-perf — pure logic that decides
// which interface the shell-side wrapper hands to `xdp-bench` and
// whether to auto-provision a veth pair before invocation. Exists
// because the prior shape silently defaulted to `lo` (which does not
// support native XDP, so the gate exited 4 from libbpf with
// "Underlying driver does not support XDP in native mode" on every
// uncustomised invocation in Lima or CI). The pure resolution lives
// in a sibling module so the wrapper's I/O stays thin and this file
// stays runnable on macOS without `ip` / `xdp-bench` installed.
use xtask::perf_gate::xdp_perf_setup::{
    DEFAULT_VETH_PEER, DEFAULT_VETH_PRIMARY, IfaceResolution, ProvisioningPlan,
    resolve_iface_config, veth_create_argv, veth_link_up_argv, veth_show_argv,
};

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

// ---------------------------------------------------------------------
// Iface-resolution self-tests — captures the bugfix that defaulted to
// `lo` (which rejects native XDP). The pure resolver decides between
// auto-provisioning a `xdp0/xdp1` veth pair and using a caller-staged
// interface; the shell wrapper in `main.rs::xdp_perf` runs `ip link`
// per the resolution. These tests pin the contract; subprocess
// invocation stays untested per existing self-test discipline.
// ---------------------------------------------------------------------

/// Regression: with no `OVERDRIVE_XDP_PERF_IFACE` set and auto-setup
/// enabled (the default), the resolver MUST hand back the
/// project's reserved veth-primary name and ask the wrapper to
/// auto-provision the `xdp0/xdp1` pair. Prior to the fix the
/// effective default was `lo`, which the loopback driver rejects for
/// native XDP — every uncustomised `cargo xtask xdp-perf` run on
/// Lima or `ubuntu-latest` exited 4 from libbpf.
#[test]
fn resolve_iface_config_auto_provisions_xdp_veth_when_env_unset() {
    let resolution = resolve_iface_config(None, false);

    assert_eq!(
        resolution.iface, DEFAULT_VETH_PRIMARY,
        "default iface must be the project's reserved veth primary, never `lo`"
    );
    match resolution.provisioning {
        ProvisioningPlan::AutoProvisionVeth { ref primary, ref peer } => {
            assert_eq!(primary, DEFAULT_VETH_PRIMARY);
            assert_eq!(peer, DEFAULT_VETH_PEER);
        }
        ProvisioningPlan::UseAsIs => {
            panic!("auto-provision path expected when env unset and no_auto_setup=false")
        }
    }
}

/// When the operator explicitly names an interface via
/// `OVERDRIVE_XDP_PERF_IFACE`, the resolver MUST honour it verbatim
/// and stay out of the provisioning path — that interface's lifecycle
/// is the operator's, not the gate's. This is the existing CI escape
/// hatch and the dev-loop knob for "I already wired up a different
/// veth pair (or a real NIC)."
#[test]
fn resolve_iface_config_honours_explicit_env_var_without_provisioning() {
    let resolution = resolve_iface_config(Some("eth0".to_string()), false);

    assert_eq!(resolution.iface, "eth0");
    assert!(
        matches!(resolution.provisioning, ProvisioningPlan::UseAsIs),
        "explicit env var must skip auto-provisioning"
    );
}

/// `--no-auto-setup` MUST disable the auto-provisioning path even
/// when the env var is unset. Used by harnesses that pre-stage the
/// interface (e.g. an ops runbook, a test rig, a future Tier 3 nested
/// VM) and don't want xtask mutating `/sys/class/net`.
#[test]
fn resolve_iface_config_no_auto_setup_disables_provisioning_with_unset_env() {
    let resolution = resolve_iface_config(None, true);

    assert_eq!(resolution.iface, DEFAULT_VETH_PRIMARY);
    assert!(
        matches!(resolution.provisioning, ProvisioningPlan::UseAsIs),
        "no_auto_setup=true must skip auto-provisioning"
    );
}

/// `--no-auto-setup` paired with an explicit env var is the
/// unambiguous "stay out of my plumbing" combination — both signals
/// agree and the resolver stays out of provisioning.
#[test]
fn resolve_iface_config_no_auto_setup_with_explicit_env_uses_as_is() {
    let resolution = resolve_iface_config(Some("custom-veth".to_string()), true);

    assert_eq!(resolution.iface, "custom-veth");
    assert!(matches!(resolution.provisioning, ProvisioningPlan::UseAsIs));
}

/// Veth-pair creation argv MUST match the canonical iproute2 shape
/// for `ip link add <primary> type veth peer name <peer>`. Pinned
/// here so a future refactor that drops the `peer name` token (or
/// transposes argv positions) breaks the test rather than the
/// runtime — the runtime error surface from a bad argv is generic
/// "Error: argument ...; try 'ip link help'".
#[test]
fn veth_create_argv_matches_iproute2_canonical_shape() {
    let argv = veth_create_argv("xdp0", "xdp1");

    assert_eq!(
        argv,
        vec![
            "ip".to_string(),
            "link".to_string(),
            "add".to_string(),
            "xdp0".to_string(),
            "type".to_string(),
            "veth".to_string(),
            "peer".to_string(),
            "name".to_string(),
            "xdp1".to_string(),
        ]
    );
}

/// Bring-up argv: `ip link set <iface> up`. The wrapper calls this
/// once per side of the veth pair (both must be UP for native XDP
/// attach to succeed; `LOWER_UP` follows automatically once both
/// peers are UP).
#[test]
fn veth_link_up_argv_matches_iproute2_canonical_shape() {
    let argv = veth_link_up_argv("xdp0");

    assert_eq!(
        argv,
        vec![
            "ip".to_string(),
            "link".to_string(),
            "set".to_string(),
            "xdp0".to_string(),
            "up".to_string(),
        ]
    );
}

/// Existence-check argv: `ip link show <iface>`. The wrapper uses
/// the exit status (0 = exists, non-zero = absent) to decide whether
/// to skip creation, so the argv stays minimal — no `--json` or
/// other format flags that vary across iproute2 releases.
#[test]
fn veth_show_argv_matches_iproute2_canonical_shape() {
    let argv = veth_show_argv("xdp0");

    assert_eq!(
        argv,
        vec!["ip".to_string(), "link".to_string(), "show".to_string(), "xdp0".to_string(),]
    );
}

/// Default veth names MUST be stable across runs — the gate writes
/// no record of which iface it used, so a name churn between runs
/// would silently produce orphaned veth pairs in the host netns.
/// Pinned here as the contract: changing these is a breaking
/// operational change that needs a deliberate migration.
#[test]
fn default_veth_names_are_stable() {
    assert_eq!(DEFAULT_VETH_PRIMARY, "xdp0");
    assert_eq!(DEFAULT_VETH_PEER, "xdp1");
}

/// The resolved `iface` MUST always equal the auto-provision
/// primary when the auto-provision path is taken. Pins the cross-
/// field invariant so a future refactor that changes one without
/// the other (e.g. resolves to a peer-side iface but provisions
/// the primary side) breaks the test rather than the runtime
/// attach.
#[test]
fn auto_provision_primary_matches_resolved_iface() {
    let resolution = resolve_iface_config(None, false);

    let ProvisioningPlan::AutoProvisionVeth { primary, peer: _ } = resolution.provisioning else {
        panic!("auto-provision path expected");
    };
    assert_eq!(primary, resolution.iface);
}

/// Sanity: `IfaceResolution` MUST be constructible without a
/// peer — i.e. `UseAsIs` is a valid provisioning plan even when the
/// iface name happens to match the default. This guards against
/// the resolver accidentally promoting an explicit-env case into
/// auto-provisioning just because the operator named the iface
/// `xdp0`.
#[test]
fn explicit_env_named_xdp0_still_uses_as_is() {
    let resolution = resolve_iface_config(Some(DEFAULT_VETH_PRIMARY.to_string()), false);

    assert_eq!(resolution.iface, DEFAULT_VETH_PRIMARY);
    assert!(
        matches!(resolution.provisioning, ProvisioningPlan::UseAsIs),
        "explicit env var (even when matching the default name) must not trigger auto-provisioning — operator owns lifecycle"
    );
}

/// Direct construction of `IfaceResolution` (used by the wrapper
/// to plumb the resolution through to the runner) MUST work
/// regardless of the variant. Compile-shape assertion only.
#[test]
fn iface_resolution_struct_is_constructible_directly() {
    let _: IfaceResolution =
        IfaceResolution { iface: "anything".to_string(), provisioning: ProvisioningPlan::UseAsIs };
    let _: IfaceResolution = IfaceResolution {
        iface: DEFAULT_VETH_PRIMARY.to_string(),
        provisioning: ProvisioningPlan::AutoProvisionVeth {
            primary: DEFAULT_VETH_PRIMARY.to_string(),
            peer: DEFAULT_VETH_PEER.to_string(),
        },
    };
}
