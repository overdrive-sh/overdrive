//! Tier 1 acceptance — `ExecProber` exit-code classification +
//! `ProbeRunner` → `SimExecProber` wiring.
//!
//! Slice 02 (step 02-02 / US-03). Two surfaces, both port-to-port at
//! the worker boundary:
//!
//! 1. `ExecExitCodeClassification` (S-SHCP-03-{01,02}) — a proptest
//!    over the FULL exit-code universe `{0, 1..=255}` pins the
//!    classification contract exposed by the production
//!    `CgroupExecProber` via its public `classify_exit_status` /
//!    `not_found_reason` / `timeout_reason` surfaces:
//!    - exit 0 → `Pass`
//!    - exit N≠0 → `Fail { "exit N" }`
//!    These are the properties the Tier-3 real-cgroup tests exercise
//!    against `/bin/true` (exit 0) and `/bin/sleep` (timeout); the
//!    proptest here covers every exit status the kernel can carry.
//!    S-SHCP-03-03 (command-not-found) and S-SHCP-03-04 (timeout) pin
//!    the named-reason strings the production adapter emits.
//!
//! 2. `ProbeRunner` → `SimExecProber` wiring — `probe_once_and_record`
//!    with a `ProbeMechanic::Exec` descriptor dispatches to the
//!    injected `ExecProber`, and writes one `ProbeResultRow` to the
//!    `ObservationStore`. The queued outcome flows through verbatim
//!    (no silent transformation between adapter return, in-memory
//!    return value, and stored row). Pins the `ProbeMechanic::Exec`
//!    dispatch arm in `probe_once_and_record` (the arm that was
//!    `MechanicNotYetImplemented` before this slice).
//!
//! Per ADR-0059 §2: the Sim adapter does NOT assert cgroup membership
//! — that's a Tier 3 concern (`tests/integration/probe_runner/
//! real_exec_probe_cgroup.rs`). The SIGKILL-via-`cgroup.kill` timeout
//! cleanup invariant is likewise a Tier 3 concern (real cgroup); the
//! Tier-1 surface here pins only the operator-facing `timeout after
//! <N>s` reason string the production adapter emits on timeout.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    reason = "operator-readable test module docs naming reason strings + ADR refs"
)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::probe_runner::exec_prober::{
    classify_exit_status, not_found_reason, timeout_reason,
};
use proptest::prelude::*;

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id parses")
}

fn node_id_for_obs_store() -> overdrive_core::id::NodeId {
    overdrive_core::id::NodeId::new("node-test").expect("node id parses")
}

fn descriptor_exec(command: &[&str]) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Exec {
            command: command.iter().map(|s| (*s).to_owned()).collect(),
        },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

/// Reference oracle for the exit-code classification contract per
/// US-03 AC. The SUT (`classify_exit_status`) MUST agree with this for
/// every code in the universe.
fn expected_outcome_for_exit(code: i32) -> ProbeOutcome {
    if code == 0 {
        ProbeOutcome::Pass
    } else {
        ProbeOutcome::Fail { reason: format!("exit {code}") }
    }
}

// ---------------------------------------------------------------------
// S-SHCP-03-{01,02} — ExecExitCodeClassification proptest.
// Universe = exit_code i32 ∈ {0, 1..=255} (the POSIX wait-status exit
// range; classification must be total over that range).
// ---------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-SHCP-03-01 / 02 — every exit code classifies into the US-03
    /// outcome contract:
    /// - 0 → Pass
    /// - N≠0 → Fail "exit N"
    ///
    /// Universe slot: the single returned `ProbeOutcome`. The oracle
    /// (`expected_outcome_for_exit`) is the spec; `classify_exit_status`
    /// is the SUT. Equality over {0, 1..=255} is the property.
    #[test]
    fn exec_exit_classification_matches_us03_contract(code in 0i32..=255i32) {
        let observed = classify_exit_status(code);
        let expected = expected_outcome_for_exit(code);
        prop_assert_eq!(observed, expected, "classification diverged for exit {}", code);
    }

    /// S-SHCP-03-01 — exit 0 is the ONLY Pass: a code is Pass IFF it is
    /// exactly 0. Pins the zero boundary against off-by-one / sign-flip
    /// mutants on the exit-status guard.
    #[test]
    fn exec_pass_is_exactly_exit_zero(code in 0i32..=255i32) {
        let is_pass = matches!(classify_exit_status(code), ProbeOutcome::Pass);
        prop_assert_eq!(is_pass, code == 0,
            "Pass is exactly exit 0; exit {} disagreed", code);
    }

    /// S-SHCP-03-02 — every non-zero exit's reason names the exact code
    /// it carried. The reason string `"exit N"` is the operator-facing
    /// contract; renaming it is a wire-shape change.
    #[test]
    fn exec_nonzero_reason_names_the_code(code in 1i32..=255i32) {
        match classify_exit_status(code) {
            ProbeOutcome::Fail { reason } => {
                prop_assert_eq!(reason, format!("exit {code}"),
                    "non-zero exit reason must be `exit <code>`");
            }
            ProbeOutcome::Pass => prop_assert!(false, "non-zero exit {} must Fail", code),
        }
    }
}

/// S-SHCP-03-03 — command-not-found maps to the named reason
/// `"exec: command not found"`. Single-example: the not-found reason is
/// a fixed operator-facing string, not a property over a universe.
// bypass: fixed wire-shape string — the contract is one exact value,
// not a quantified invariant. Compensated by the exit-code proptest
// above + the Tier-3 real-cgroup classification tests.
#[test]
fn exec_command_not_found_reason_is_named() {
    assert_eq!(not_found_reason(), "exec: command not found");
}

// S-SHCP-03-04 — timeout maps to the named reason
// `"timeout after <N>s"`. Proptest over the timeout-seconds universe:
// the reason names the exact whole-seconds budget the probe was given.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn exec_timeout_reason_names_whole_seconds(secs in 1u64..=120u64) {
        let reason = timeout_reason(Duration::from_secs(secs));
        prop_assert_eq!(reason, format!("timeout after {secs}s"),
            "timeout reason must be `timeout after <N>s` for whole-seconds budgets");
    }
}

// ---------------------------------------------------------------------
// S-SHCP-03 wiring — ProbeRunner → SimExecProber → ObservationStore.
// The ProbeMechanic::Exec dispatch arm in probe_once_and_record (was
// MechanicNotYetImplemented) carries the queued outcome through to the
// stored row verbatim.
// ---------------------------------------------------------------------

fn runner_with_exec(exec: Arc<SimExecProber>) -> ProbeRunner {
    ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        exec,
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    )
}

/// S-SHCP-03-01 (wiring) — a `Pass` outcome on the Exec queue flows
/// through `ProbeRunner` → `SimExecProber` and records
/// `ProbeStatus::Pass`. Pins the `ProbeMechanic::Exec` dispatch arm.
///
/// Universe (port-exposed observable surface at the worker boundary):
/// - return value of `probe_once_and_record` (`ProbeResultRow.status`)
/// - state delta on `ObservationStore.list_probe_results_for_alloc`
///   (one new row at `(alloc_id, probe_idx)`).
#[tokio::test]
async fn exec_pass_outcome_flows_through_probe_runner_to_store() {
    let exec = Arc::new(SimExecProber::new());
    exec.enqueue_outcome(ProbeOutcome::Pass);
    let runner = runner_with_exec(exec);
    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-exec-pass");
    let descriptor = descriptor_exec(&["/bin/true"]);

    let before = obs.list_probe_results_for_alloc(&alloc).await.expect("list before");
    assert!(before.is_empty(), "no rows before first probe");

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Exec Pass outcome");

    assert_eq!(returned_row.status, ProbeStatus::Pass);
    assert_eq!(returned_row.role, ProbeRole::Startup);
    assert!(!returned_row.inferred);

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(after[0], returned_row, "stored row matches returned row");
}

/// S-SHCP-03-02 (wiring) — a non-zero exit `Fail { "exit 7" }` outcome
/// on the Exec queue flows through `ProbeRunner` → `SimExecProber` and
/// records `ProbeStatus::Fail { last_fail_reason: "exit 7" }` verbatim.
#[tokio::test]
async fn exec_nonzero_exit_outcome_flows_through_probe_runner_to_store() {
    let exec = Arc::new(SimExecProber::new());
    exec.enqueue_outcome(ProbeOutcome::Fail { reason: "exit 7".to_owned() });
    let runner = runner_with_exec(exec);
    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-exec-fail");
    let descriptor = descriptor_exec(&["/bin/false"]);

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Exec Fail outcome");

    assert_eq!(
        returned_row.status,
        ProbeStatus::Fail { last_fail_reason: "exit 7".to_owned() },
        "Exec non-zero-exit outcome carried into row.status verbatim"
    );

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(after[0].status, returned_row.status, "stored row carries the Fail reason");
}

/// Regression — exec cgroup-placement failure must still write a
/// `ProbeResultRow::Fail` to the observation store so the
/// `ServiceLifecycleReconciler` can observe the failure and
/// eventually fire `StartupProbeFailed`. Before the fix,
/// `probe_tick` returned `Err(ProbeAdapterFailed)` without writing
/// any row, leaving `startup_attempts_per_alloc` at 0 and the
/// startup window open indefinitely.
#[tokio::test]
async fn exec_cgroup_placement_error_still_writes_fail_row_to_store() {
    let exec = Arc::new(SimExecProber::new());
    exec.enqueue_error(ProbeFailure::ExecSpawnFailed {
        reason: "cgroup placement failed: EACCES".to_owned(),
    });
    let runner = runner_with_exec(Arc::clone(&exec));
    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-exec-cgroup-fail");
    let descriptor = descriptor_exec(&["/bin/true"]);

    let before = obs.list_probe_results_for_alloc(&alloc).await.expect("list before");
    assert!(before.is_empty(), "no rows before first probe");

    // probe_once_and_record is expected to return Err (the adapter
    // error propagates), but the observation store MUST contain a
    // Fail row written before the error was returned.
    let result =
        runner.probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs).await;
    assert!(result.is_err(), "adapter error propagates to caller");

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list after");
    assert_eq!(after.len(), 1, "exactly one Fail row written despite adapter error");
    assert!(
        matches!(after[0].status, ProbeStatus::Fail { .. }),
        "stored row is Fail, not Pass — got {:?}",
        after[0].status,
    );
    assert_eq!(after[0].role, ProbeRole::Startup);
}
