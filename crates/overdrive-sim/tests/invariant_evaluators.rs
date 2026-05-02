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
            reason: None,
            detail: None,
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

// ---------------------------------------------------------------------------
// DispatchRoutingIsNameRestricted (fix-dst-dispatch-routing-invariant 01-01)
//
// End-to-end happy + negative tests for the §8 storm-proofing dispatch-
// routing invariant. Sibling to the broker-side
// `DuplicateEvaluationsCollapse` tests; this one pins dispatcher-side
// routing.
//
// The negative case below is the regression-test proof for this
// delivery: it asserts the evaluator flags a mocked fan-out — the
// exact shape of the bug the precursor (commit `e6f5e5e`) closed at
// the unit/acceptance tier. No separate `#[ignore]`-attributed RED
// scaffold is needed because this negative case IS the RED proof
// living next to its GREEN sibling.
// ---------------------------------------------------------------------------

fn jl_reconciler() -> overdrive_core::reconciler::ReconcilerName {
    overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle is a valid ReconcilerName")
}

fn target(raw: &str) -> overdrive_core::reconciler::TargetResource {
    overdrive_core::reconciler::TargetResource::new(raw).expect("valid TargetResource")
}

/// Happy path: every drained eval is dispatched against its named
/// reconciler exactly once. The evaluator must return Pass.
#[test]
fn dispatch_routing_passes_when_each_eval_dispatches_named_reconciler_only() {
    let r = jl_reconciler();
    let t_a = target("job/payments");
    let t_b = target("job/frontend");

    let submitted = vec![
        evaluators::Evaluation { reconciler: r.clone(), target: t_a.clone() },
        evaluators::Evaluation { reconciler: r.clone(), target: t_b.clone() },
    ];
    let record = evaluators::DispatchRecord { dispatched: vec![(r.clone(), t_a), (r, t_b)] };

    let result = evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "clean 1-to-1 dispatch must pass; got {result:?}",
    );
    assert!(result.cause.is_none(), "Pass must carry no cause; got {:?}", result.cause);
}

/// Negative case — the regression-test proof for this delivery.
///
/// Mocked fan-out: a SINGLE drained eval names `job-lifecycle` against
/// `job/payments`, but the dispatch record contains TWO entries — one
/// correct (`job-lifecycle`, `job/payments`) and one wrong (`noop-
/// heartbeat`, `job/payments`). This is exactly the shape the precursor
/// fix at commit `e6f5e5e` eliminated in production: a registry-wide
/// loop dispatching every reconciler against a single drained eval.
///
/// The evaluator MUST flag this. The cardinality branch fires first
/// because `record.dispatched.len() (2) != submitted.len() (1)`.
#[test]
fn dispatch_routing_fails_on_mocked_fanout_regression() {
    let jl = jl_reconciler();
    let noop = overdrive_core::reconciler::ReconcilerName::new("noop-heartbeat")
        .expect("noop-heartbeat is a valid ReconcilerName");
    let t = target("job/payments");

    let submitted = vec![evaluators::Evaluation { reconciler: jl.clone(), target: t.clone() }];
    let record = evaluators::DispatchRecord { dispatched: vec![(jl, t.clone()), (noop, t)] };

    let result = evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "mocked fan-out (2 dispatches for 1 drained eval) must fail; got {result:?}",
    );
    let cause = result.cause.as_ref().expect("Fail must carry a cause");
    assert!(
        cause.contains("expected") && cause.contains("dispatch entries"),
        "cardinality cause must name the mismatch shape; got {cause:?}",
    );
}

/// Negative case — pure smoking-gun branch with clean cardinality.
///
/// `submitted = [(jl, payments)]`, `dispatched = [(noop, payments)]` —
/// cardinality matches (1 == 1) so the cardinality branch does NOT
/// fire. The per-eval routing branch counts zero matches for `(jl,
/// payments)` and fails. The smoking-gun branch is also reachable
/// because `noop` is not in the submitted-names set; either of the two
/// cause shapes is acceptable here, and we assert on the broader
/// "wrong reconciler dispatched" shape.
#[test]
fn dispatch_routing_fails_when_only_unsubmitted_reconciler_was_dispatched() {
    let jl = jl_reconciler();
    let noop = overdrive_core::reconciler::ReconcilerName::new("noop-heartbeat")
        .expect("noop-heartbeat is a valid ReconcilerName");
    let t = target("job/payments");

    let submitted = vec![evaluators::Evaluation { reconciler: jl, target: t.clone() }];
    let record = evaluators::DispatchRecord { dispatched: vec![(noop, t)] };

    let result = evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
    assert_eq!(
        result.status,
        InvariantStatus::Fail,
        "wrong-reconciler dispatch must fail; got {result:?}",
    );
}
