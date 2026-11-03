//! Gateway Application lifecycle invariants through the DST harness
//! (S-PIG-31 and S-PIG-32; ADR-0222).
//!
//! The driving port is the harness (`Harness::new().only(..).run(seed)`), the
//! same surface `cargo dst --only <name>` drives once the pending activation key
//! `gateway-application-invariants` lands. The evaluators compose the real
//! `GatewayApplicationOwner` through `UnboundGatewayBuilder` -> `GatewayBuilder`
//! -> `GatewayRouteCommands` / `GatewayStatusReads` / `GatewayHandle` over a real redb Route store and the Sim
//! relay, connect dataplane, client mTLS and identity ports; the only fault is the
//! Sim connect-probe hold/release. Neither invariant is in `Invariant::ALL`
//! during DISTILL, so the default catalogue and its expected-name arrays are
//! unchanged and only `.only(..)` reaches the evaluators.
//!
//! # Bind addresses (R-6)
//!
//! The evaluators bind `127.120.x.y` (generation lifecycle) and `127.121.x.y`
//! (shutdown) derived from the seed, so each seed of these tests is a distinct
//! composition address.
//!
//! # Oracle
//!
//! A passing run reports `ordered-trace len=N digest=..` in the result's cause
//! (the pinned `InvariantResult` carries no other field a twin-run comparison
//! can read); a failing run reports the seed, the schedule and the offending
//! observation.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "CONTRACT_SHAPE metadata, fixture preconditions and the printed seed"
)]

use overdrive_sim::harness::{Harness, InvariantResult, InvariantStatus, RunReport};
use overdrive_sim::invariants::Invariant;

/// Seeds covering every seed-selected legal tail of the lifecycle case and every
/// permutation of the shutdown case.
const SEEDS: std::ops::RangeInclusive<u64> = 1..=8;
/// The seed of the twin-run comparisons.
const TWIN_SEED: u64 = 0x3132;

fn run_only(invariant: Invariant, seed: u64) -> RunReport {
    eprintln!("gateway application invariant {invariant} seed={seed}");
    Harness::new().only(invariant).run(seed).expect("harness composes")
}

fn result_of(report: &RunReport, invariant: Invariant) -> InvariantResult {
    report
        .invariants
        .iter()
        .find(|result| result.name == invariant.as_canonical())
        .cloned()
        .expect("the selected invariant ran")
}

fn assert_holds(invariant: Invariant, seed: u64) -> InvariantResult {
    let result = result_of(&run_only(invariant, seed), invariant);
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "{invariant} must hold at seed {seed}: {:?}",
        result.cause
    );
    result
}

/// S-PIG-31: stale probe completion, replacement, withdrawal, a retained
/// connection and restart preserve generation safety at every seed.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-application-invariants"]
fn stale_completion_withdrawal_restart_and_retained_generation_are_safe() {
    for seed in SEEDS {
        assert_holds(Invariant::GatewayApplicationGenerationLifecycleIsSafe, seed);
    }
}

/// S-PIG-32: shutdown joins the admitted users before the gateway identity slot
/// clears, and converges to an empty ledger and zero live intents and receipts,
/// at every permutation of the probe release, the connection release and the
/// shutdown begin.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-application-invariants"]
fn shutdown_joins_users_before_gateway_identity_retire() {
    for seed in SEEDS {
        assert_holds(Invariant::GatewayApplicationShutdownConverges, seed);
    }
}

/// S-PIG-31 determinism: twin runs at one seed produce the identical ordered
/// observation trace.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-application-invariants"]
fn twin_generation_runs_at_one_seed_have_identical_ordered_observation_traces() {
    let invariant = Invariant::GatewayApplicationGenerationLifecycleIsSafe;
    let first = assert_holds(invariant, TWIN_SEED);
    let second = assert_holds(invariant, TWIN_SEED);
    assert!(first.cause.is_some(), "a passing run reports its ordered trace");
    assert_eq!(first, second, "twin runs at seed {TWIN_SEED} diverged");
}

/// S-PIG-32 determinism: twin shutdown runs at one seed produce the identical
/// ordered observation trace.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER gateway-application-invariants"]
fn twin_shutdown_runs_at_one_seed_have_identical_ordered_observation_traces() {
    let invariant = Invariant::GatewayApplicationShutdownConverges;
    let first = assert_holds(invariant, TWIN_SEED);
    let second = assert_holds(invariant, TWIN_SEED);
    assert!(first.cause.is_some(), "a passing run reports its ordered trace");
    assert_eq!(first, second, "twin runs at seed {TWIN_SEED} diverged");
}
