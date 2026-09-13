//! S-PIG-31 and S-PIG-32 — seeded Gateway Application acceptance contracts.
//!
//! S-PIG-31 mutation universe: the singleton Public Route Set; Gateway
//! Application staged/current/draining generations and retained-connection
//! lease; derived frontend-demand entries/revision; Application status row;
//! restart-owned listener/task state. Workload/Service intent, allocation and
//! non-gateway identity state are the preserved complement.
//!
//! S-PIG-32 mutation universe: listener admission, retained request/connection
//! leases, gateway join set, gateway connect intent/receipt cleanup ledger,
//! derived frontend demand, dedicated Gateway Identity Slot and typed shutdown
//! report. Public Route/certified-key records and every allocation identity are
//! the preserved complement.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use overdrive_sim::{Harness, Invariant, InvariantStatus};

const GENERATION_SEED: u64 = 54_104;
const SHUTDOWN_SEED: u64 = 54_121;

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER activation DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS"]
fn stale_completion_withdrawal_restart_and_retained_demand_are_safe() {
    let report = Harness::new()
        .only(Invariant::GatewayApplicationGenerationLifecycleIsSafe)
        .run(GENERATION_SEED)
        .expect("the production owner composes through Sim ports");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "gateway-application-generation-lifecycle-is-safe");
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "seed {}: {:?}",
        GENERATION_SEED,
        report.invariants[0].cause,
    );
    assert!(report.is_green());
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER activation DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS"]
fn shutdown_joins_users_before_demand_and_gateway_identity_retire() {
    let report = Harness::new()
        .only(Invariant::GatewayApplicationShutdownConverges)
        .run(SHUTDOWN_SEED)
        .expect("the production shutdown owner composes through Sim ports");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "gateway-application-shutdown-converges");
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "seed {}: {:?}",
        SHUTDOWN_SEED,
        report.invariants[0].cause,
    );
    assert!(report.is_green());
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER activation DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS"]
fn twin_generation_runs_at_one_seed_have_identical_ordered_observation_traces() {
    let first = Harness::new()
        .only(Invariant::GatewayApplicationGenerationLifecycleIsSafe)
        .run(GENERATION_SEED)
        .expect("first seeded owner composition");
    let second = Harness::new()
        .only(Invariant::GatewayApplicationGenerationLifecycleIsSafe)
        .run(GENERATION_SEED)
        .expect("second seeded owner composition");
    assert_eq!(first.seed, second.seed);
    assert_eq!(first.invariants, second.invariants);
    assert_eq!(first.failures, second.failures);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER activation DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS"]
fn twin_shutdown_runs_at_one_seed_have_identical_ordered_observation_traces() {
    let first = Harness::new()
        .only(Invariant::GatewayApplicationShutdownConverges)
        .run(SHUTDOWN_SEED)
        .expect("first seeded owner composition");
    let second = Harness::new()
        .only(Invariant::GatewayApplicationShutdownConverges)
        .run(SHUTDOWN_SEED)
        .expect("second seeded owner composition");
    assert_eq!(first.seed, second.seed);
    assert_eq!(first.invariants, second.invariants);
    assert_eq!(first.failures, second.failures);
}
