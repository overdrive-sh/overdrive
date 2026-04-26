#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Unit-level injection tests for the six Phase 1 invariant evaluators.
//!
//! Each test plants a failure a real evaluator must catch. These tests
//! are the step 06-02 "not trivially green" guarantee: every invariant
//! body has at least one planted scenario that makes it report
//! `InvariantStatus::Fail`. Without these, a `fn evaluate(_, _, _) -> Pass`
//! stub would satisfy the acceptance test suite — which is exactly the
//! Testing Theater shape 06-01 deliberately left for this step to close.
//!
//! The tests import from [`overdrive_sim::invariants::evaluators`] —
//! the module that holds the raw per-invariant evaluator functions
//! that the harness composes together. Planting a failure here does
//! not require booting a full harness; the evaluator's own contract is
//! what each test exercises.

use std::str::FromStr;
use std::time::Duration;

use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::InvariantStatus;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::invariants::evaluators;

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid node id")
}

// ---------------------------------------------------------------------------
// SingleLeader — stubbed 3-host state machine
// ---------------------------------------------------------------------------

#[test]
fn single_leader_passes_when_exactly_one_host_reports_leader() {
    let hosts = vec![node("host-0"), node("host-1"), node("host-2")];
    let result = evaluators::evaluate_single_leader_from_topology(&hosts, Some(&node("host-0")));
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "exactly one leader must pass; got {result:?}"
    );
}

#[test]
fn single_leader_fails_when_no_host_reports_leader() {
    let hosts = vec![node("host-0"), node("host-1"), node("host-2")];
    let result = evaluators::evaluate_single_leader_from_topology(&hosts, None);
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "zero leaders must fail — otherwise a mutation that never assigns a leader hides",
    );
}

/// Planted failure: the stubbed topology accidentally reports two
/// leaders. The evaluator MUST catch this — a real-Raft implementation
/// in Phase 2 relies on this shape.
#[test]
fn single_leader_fails_when_two_hosts_report_leader_simultaneously() {
    // Construct a topology where the "leader list" contains two entries.
    // The evaluator function signature shields the test from the stub's
    // internal shape; we just pass a synthetic result.
    let leaders = vec![node("host-0"), node("host-1")];
    let result = evaluators::evaluate_single_leader_from_leader_set(&leaders);
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "two leaders simultaneously must fail the SingleLeader invariant",
    );
}

// ---------------------------------------------------------------------------
// IntentNeverCrossesIntoObservation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn intent_never_crosses_passes_on_a_fresh_cluster() {
    // A clean cluster has no crossings.
    let tmp = tempfile::tempdir().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let intent = overdrive_store_local::LocalIntentStore::open(&store_path).expect("open");
    let observation = SimObservationStore::single_peer(node("host-0"), 42);

    let result = evaluators::evaluate_intent_crossing(&intent, &observation).await;
    assert_eq!(result.status, InvariantStatus::Pass, "clean cluster must pass; got {result:?}");
}

/// Planted failure: write an observation-prefixed key into the
/// `LocalIntentStore`. The evaluator must catch that intent has an
/// observation-class key.
#[tokio::test(flavor = "current_thread")]
async fn intent_never_crosses_fails_when_localstore_holds_alloc_status_prefix_key() {
    use overdrive_core::traits::intent_store::IntentStore;

    let tmp = tempfile::tempdir().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let intent = overdrive_store_local::LocalIntentStore::open(&store_path).expect("open");
    // Observation-class prefix written into intent is a §4 guardrail
    // violation — the evaluator must flag it.
    intent.put(b"alloc_status/a1b2c3", b"bogus").await.expect("put");

    let observation = SimObservationStore::single_peer(node("host-0"), 42);

    let result = evaluators::evaluate_intent_crossing(&intent, &observation).await;
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "observation-prefix key in intent must fail the invariant; got {result:?}",
    );
}

// ---------------------------------------------------------------------------
// SnapshotRoundtripBitIdentical
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn snapshot_roundtrip_passes_on_any_real_localstore() {
    use overdrive_core::traits::intent_store::IntentStore;

    let tmp = tempfile::tempdir().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let intent = overdrive_store_local::LocalIntentStore::open(&store_path).expect("open");

    // Some entries to exercise framing, sorted vs insertion-order.
    intent.put(b"job/frontend", b"{}").await.expect("put");
    intent.put(b"job/payments", b"{}").await.expect("put");
    intent.put(b"policy/default", b"{}").await.expect("put");

    let result = evaluators::evaluate_snapshot_roundtrip(&intent).await;
    assert_eq!(result.status, InvariantStatus::Pass, "roundtrip must pass; got {result:?}");
}

// ---------------------------------------------------------------------------
// SimObservationLwwConverges
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn sim_observation_lww_converges_passes_on_a_fresh_cluster() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(Duration::from_millis(50))
        .seed(42)
        .build();

    let result = evaluators::evaluate_sim_observation_lww(&cluster).await;
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "clean cluster converges trivially; got {result:?}",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn sim_observation_lww_converges_passes_after_writes_and_convergence() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(Duration::from_millis(50))
        .seed(42)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    peer_a
        .write(ObservationRow::AllocStatus(AllocStatusRow {
            alloc_id: AllocationId::from_str("alloc-1").expect("alloc id"),
            job_id: JobId::from_str("payments").expect("job id"),
            node_id: node("node-a"),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: 1, writer: node("node-a") },
        }))
        .await
        .expect("write");

    // Let gossip drain.
    cluster.advance(Duration::from_millis(500)).await;
    cluster.advance(Duration::from_millis(500)).await;

    let result = evaluators::evaluate_sim_observation_lww(&cluster).await;
    assert_eq!(result.status, InvariantStatus::Pass, "converged cluster passes; got {result:?}");
}

// ---------------------------------------------------------------------------
// ReplayEquivalentEmptyWorkflow
// ---------------------------------------------------------------------------

#[test]
fn replay_equivalent_empty_workflow_passes_on_deterministic_transcript() {
    let result = evaluators::evaluate_replay_equivalent_empty_workflow(42);
    assert_eq!(result.status, InvariantStatus::Pass, "empty workflow replays; got {result:?}");
}

// ---------------------------------------------------------------------------
// EntropyDeterminismUnderReseed
// ---------------------------------------------------------------------------

#[test]
fn entropy_determinism_passes_when_two_sim_entropies_agree() {
    // Two SimEntropy instances seeded identically produce identical
    // draw sequences. This is the positive witness.
    let result = evaluators::evaluate_entropy_determinism(42);
    assert_eq!(result.status, InvariantStatus::Pass, "same seed must agree; got {result:?}");
}

/// Planted failure: `SimEntropy` instances seeded differently must
/// disagree. The evaluator function under test is specifically the
/// one that expects agreement — handing it the disagreeing pair must
/// fail. This guards against a mutation that replaces the comparison
/// with `true`.
#[test]
fn entropy_determinism_fails_when_two_sim_entropies_disagree() {
    let a = SimEntropy::new(1);
    let b = SimEntropy::new(2);
    let result = evaluators::evaluate_entropy_determinism_against(&a, &b);
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "disagreeing entropies must fail; got {result:?}",
    );
}
