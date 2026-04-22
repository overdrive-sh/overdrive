//! §5.1 scenario 3 (property) — LWW convergence is deterministic across
//! seeded delivery orders. Step 04-03.
//!
//! Given any set of concurrent writes produced by the observation
//! generator, running the seeded sim twice with the same seed yields
//! **bit-identical final row sets on every peer**, and every peer
//! within one run agrees on the final row for every alloc it has seen.
//!
//! # Why a `ConvergenceReport` helper
//!
//! Per the roadmap binding: "This test file's name is used verbatim for
//! the `SimObservationLwwConverges` invariant enum variant (US-06). The
//! same assertion will be re-invoked by the harness in step 06-02 as an
//! `assert_always!` invariant — **do not duplicate the logic; factor it
//! into a helper that both the proptest and the invariant call.**"
//!
//! The proptest drives
//! [`overdrive_sim::adapters::observation_store::check_lww_convergence`]
//! — the single source of truth for "peers agree on an alloc's LWW
//! winner." Step 06-02 re-invokes the same helper inside the
//! `SimObservationLwwConverges` invariant evaluator; the assertion is
//! never duplicated.
//!
//! # Seed discipline
//!
//! * The proptest generator draws a `u64` scenario seed on every case.
//! * `PROPTEST_CASES` defaults to 1024 (CI env); an in-crate override
//!   is set to 256 to keep the per-case runtime manageable given each
//!   case constructs two full clusters and drives them to convergence.
//! * On failure, proptest prints both the shrunken `ConcurrentWriteScenario`
//!   and the replay command — `PROPTEST_CASES=1 PROPTEST_REPLAY=<seed>
//!   cargo test -p overdrive-sim`.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;
use std::time::Duration;

use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::{
    ConvergenceReport, SimObservationStore, check_lww_convergence,
};
use proptest::prelude::*;

/// Fixed step seed used by the hand-written witness test. The proptest
/// draws its own scenario seed per case.
const STEP_SEED: u64 = 0x04_03_CC_CC_CC_CC_CC_CC;

/// Gossip window short enough to keep per-case runtime low; two advances
/// past this value drain the full FIFO across every non-partitioned edge
/// for the scenarios we generate.
const GOSSIP_WINDOW: Duration = Duration::from_millis(50);

/// An advance that dwarfs the gossip window so we know every queued
/// write has become eligible for delivery after one tick. Two advances
/// past this value make the FIFO fully drain even on the largest
/// scenario we generate.
const PAST_CONVERGENCE: Duration = Duration::from_millis(500);

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid node id")
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::from_str(name).expect("valid alloc id")
}

fn row(
    alloc_id: &AllocationId,
    writer: &NodeId,
    counter: u64,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc_id.clone(),
        job_id: JobId::from_str("payments").expect("valid job id"),
        node_id: writer.clone(),
        state,
        updated_at: LogicalTimestamp { counter, writer: writer.clone() },
    }
}

// ---------------------------------------------------------------------------
// Hand-written witness — five writes, three peers, overlapping alloc_ids
// ---------------------------------------------------------------------------

/// Concrete scenario used as the RED→GREEN witness for the helper.
///
/// Three peers; five writes touching three `alloc_ids`; two of them
/// contend on the same `alloc_id` with different counters. A passing
/// assertion proves the helper recognises convergence on a fully
/// converged cluster.
#[tokio::test(flavor = "current_thread")]
async fn witness_three_peer_cluster_converges_on_overlapping_writes() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    // alloc-1: peer-A at T1, peer-B at T2 (T2 wins)
    peer_a
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-1"),
            &node("node-a"),
            1,
            AllocState::Running,
        )))
        .await
        .expect("write alloc-1 T1");
    peer_b
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-1"),
            &node("node-b"),
            2,
            AllocState::Draining,
        )))
        .await
        .expect("write alloc-1 T2");

    // alloc-2: peer-C only
    peer_c
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-2"),
            &node("node-c"),
            1,
            AllocState::Running,
        )))
        .await
        .expect("write alloc-2 T1");

    // alloc-3: tiebreak — same counter, two writers. "node-c" > "node-a" lex.
    peer_a
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-3"),
            &node("node-a"),
            7,
            AllocState::Running,
        )))
        .await
        .expect("write alloc-3 tiebreak-a");
    peer_c
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-3"),
            &node("node-c"),
            7,
            AllocState::Draining,
        )))
        .await
        .expect("write alloc-3 tiebreak-c");

    // Advance twice past convergence — first drains the writes, second
    // drains any gossip that was enqueued by the first drain.
    cluster.advance(PAST_CONVERGENCE).await;
    cluster.advance(PAST_CONVERGENCE).await;

    let report = check_lww_convergence(&cluster);
    assert!(report.is_converged(), "witness cluster must converge — report = {report:?}",);

    // Every peer must hold rows for all three alloc_ids — pins that
    // `alloc_status_snapshot` returns the actual stored rows (not an
    // empty map). Without this assertion, a helper that returned
    // `BTreeMap::new()` from every peer would still pass `is_converged()`
    // (vacuous: no disagreement if no rows exist).
    for (node_id, view) in report.peer_views() {
        assert_eq!(
            view.len(),
            3,
            "peer {node_id} must hold rows for all three allocs — view = {view:?}",
        );
    }

    // Peer agreement is also testable per-alloc: the LWW winner for
    // `alloc-3` must be node-c's row on every peer (same counter 7 but
    // writer "node-c" > "node-a" lexicographically — §4 tiebreak rule).
    let winners: Vec<_> = report
        .peer_views()
        .values()
        .map(|view| view.get(&alloc("alloc-3")).expect("alloc-3 row present").state)
        .collect();
    assert!(
        winners.iter().all(|s| *s == AllocState::Draining),
        "every peer must hold the Draining row from node-c (tiebreak) — winners = {winners:?}",
    );
}

// ---------------------------------------------------------------------------
// Negative witness — disagreement is detected as non-converged
// ---------------------------------------------------------------------------

/// Pins `is_converged() == false` when two peers hold **different**
/// rows for the **same** `alloc_id`. Without this test, a mutation
/// replacing `is_converged()` with `true` — or replacing the match
/// guard `*existing == row` with `true` — would survive undetected:
/// every other assertion in this file runs on fully-converged clusters.
///
/// Uses a partition to block gossip so each peer's local write stays
/// local. Without the partition, the LWW merge would eventually pick
/// one winner and the cluster would converge. With the partition,
/// peer A's row and peer B's row coexist at different peers — the
/// definition of "not converged" in our report.
#[tokio::test(flavor = "current_thread")]
async fn partitioned_cluster_with_competing_rows_is_not_converged() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("peer-a"), node("peer-b")])
        .gossip_delay(GOSSIP_WINDOW)
        .partition(node("peer-a"), node("peer-b"))
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("peer-a"));
    let peer_b = cluster.peer(&node("peer-b"));

    // Both peers write conflicting rows for the same alloc_id. Because
    // they are partitioned, neither row reaches the other peer.
    peer_a
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-split"),
            &node("peer-a"),
            1,
            AllocState::Running,
        )))
        .await
        .expect("write on peer-a");
    peer_b
        .write(ObservationRow::AllocStatus(row(
            &alloc("alloc-split"),
            &node("peer-b"),
            1,
            AllocState::Draining,
        )))
        .await
        .expect("write on peer-b");

    cluster.advance(PAST_CONVERGENCE).await;
    cluster.advance(PAST_CONVERGENCE).await;

    let report = check_lww_convergence(&cluster);
    assert!(
        !report.is_converged(),
        "cluster with partitioned peers holding competing rows for the same alloc \
         must NOT be converged — report = {report:?}",
    );
}

// ---------------------------------------------------------------------------
// Proptest — determinism under seeded reordering
// ---------------------------------------------------------------------------

/// Generator bounds — pinned to the roadmap AC:
/// 1..=20 writes, 2..=5 peers, timestamps 1..=100, alloc ids drawn from
/// a fixed 1..=10 pool.
const MIN_PEERS: usize = 2;
const MAX_PEERS: usize = 5;
const MIN_WRITES: usize = 1;
const MAX_WRITES: usize = 20;
const MIN_COUNTER: u64 = 1;
const MAX_COUNTER: u64 = 100;
const ALLOC_POOL_SIZE: usize = 10;

/// One write in a generated scenario. The writer index is an offset
/// into the scenario's peer set; the alloc index is an offset into the
/// fixed 10-alloc pool.
#[derive(Debug, Clone)]
struct GeneratedWrite {
    writer_idx: usize,
    alloc_idx: usize,
    counter: u64,
    /// Boolean selector for `AllocState::Running` vs `AllocState::Draining`.
    /// Only two states are modelled — the LWW merge does not branch on
    /// state, and richer state variation would not add coverage.
    draining: bool,
}

/// A fully-specified scenario. The `seed` is the harness seed carried
/// on every peer; `peer_count` and `writes` shape what actually happens.
#[derive(Debug, Clone)]
struct ConcurrentWriteScenario {
    seed: u64,
    peer_count: usize,
    writes: Vec<GeneratedWrite>,
}

fn arb_write(peer_count: usize) -> impl Strategy<Value = GeneratedWrite> {
    (0..peer_count, 0..ALLOC_POOL_SIZE, MIN_COUNTER..=MAX_COUNTER, any::<bool>()).prop_map(
        |(writer_idx, alloc_idx, counter, draining)| GeneratedWrite {
            writer_idx,
            alloc_idx,
            counter,
            draining,
        },
    )
}

fn arb_scenario() -> impl Strategy<Value = ConcurrentWriteScenario> {
    (any::<u64>(), MIN_PEERS..=MAX_PEERS).prop_flat_map(|(seed, peer_count)| {
        prop::collection::vec(arb_write(peer_count), MIN_WRITES..=MAX_WRITES)
            .prop_map(move |writes| ConcurrentWriteScenario { seed, peer_count, writes })
    })
}

fn scenario_peers(scenario: &ConcurrentWriteScenario) -> Vec<NodeId> {
    // Canonical peer names — `peer-0`, `peer-1`, ... Only the first
    // `peer_count` are used; the fixed naming makes two runs on the
    // same scenario wire up identical node ids.
    (0..scenario.peer_count).map(|i| node(&format!("peer-{i}"))).collect()
}

fn scenario_allocs() -> Vec<AllocationId> {
    (0..ALLOC_POOL_SIZE).map(|i| alloc(&format!("alloc-{i}"))).collect()
}

/// Build one cluster per call — returned `ConvergenceReport` is what
/// the proptest compares across runs. Fully synchronous wrt simulated
/// time: every `advance(...)` is a logical-clock bump, not a wall-clock
/// sleep.
async fn run_scenario(scenario: &ConcurrentWriteScenario) -> ConvergenceReport {
    let peers = scenario_peers(scenario);
    let allocs = scenario_allocs();

    let cluster = SimObservationStore::cluster_builder()
        .peers(peers.clone())
        .gossip_delay(GOSSIP_WINDOW)
        .seed(scenario.seed)
        .build();

    for w in &scenario.writes {
        let peer = cluster.peer(&peers[w.writer_idx]);
        let alloc_id = &allocs[w.alloc_idx];
        let writer = &peers[w.writer_idx];
        let state = if w.draining { AllocState::Draining } else { AllocState::Running };
        peer.write(ObservationRow::AllocStatus(row(alloc_id, writer, w.counter, state)))
            .await
            .expect("scenario write must succeed");
    }

    // Drain twice. One advance flushes the writes; a second advance
    // flushes any gossip enqueued by downstream apply paths (there is
    // no re-enqueue in Phase 1, but this matches the future §6 invariant
    // body which will run against the full DST tick budget).
    cluster.advance(PAST_CONVERGENCE).await;
    cluster.advance(PAST_CONVERGENCE).await;

    check_lww_convergence(&cluster)
}

proptest! {
    // Halve the default to keep per-case runtime under control — each
    // case builds two clusters, dispatches up to 20 writes per cluster,
    // and advances twice. At PROPTEST_CASES=1024 the wall-clock per run
    // was north of ten seconds; 256 keeps the feedback loop tight while
    // still exercising more ground than the hand-picked witness.
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Property — two runs of the seeded sim on the same scenario yield
    /// **bit-identical** final row sets on every peer, AND both runs
    /// converge internally (every peer agrees with every other peer
    /// on every observed alloc).
    #[test]
    fn lww_converges_deterministically_under_seeded_reordering(
        scenario in arb_scenario()
    ) {
        // Proptest bodies are synchronous; construct a tokio runtime
        // per case. The sim is fully in-memory so the cost is the
        // cluster construction and per-case tick drain — small enough
        // at 256 cases.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let (report_a, report_b) = rt.block_on(async {
            let report_a = run_scenario(&scenario).await;
            let report_b = run_scenario(&scenario).await;
            (report_a, report_b)
        });

        // Bit-identical across runs: same seed, same scenario ⇒ same state.
        prop_assert_eq!(
            &report_a, &report_b,
            "two runs of the same seeded scenario must produce identical convergence reports"
        );

        // Internally converged within each run: every peer agrees with
        // every other peer on the LWW winner for every alloc either has
        // observed.
        prop_assert!(
            report_a.is_converged(),
            "run A must be internally converged — {:?}", report_a
        );
        prop_assert!(
            report_b.is_converged(),
            "run B must be internally converged — {:?}", report_b
        );
    }
}
