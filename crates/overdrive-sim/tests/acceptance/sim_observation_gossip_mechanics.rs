//! Focused unit tests for the multi-peer gossip mechanics introduced in
//! step 04-02. These tests drive the public cluster API at a finer
//! granularity than the §5.1 / §5.3 acceptance scenarios — they exist to
//! pin down internal rules the acceptance tests exercise only
//! incidentally:
//!
//! 1. **LWW tiebreak determinism** — when two writes share a counter,
//!    the writer's `NodeId` deterministically resolves the tie.
//! 2. **Partition is bidirectional** — `partition(A, B)` blocks both
//!    A→B and B→A gossip.
//! 3. **Repair is idempotent** — calling `repair` on an unpartitioned
//!    pair is a no-op, not a panic.
//!
//! Behaviour budget: 3 tests, each one behaviour. No duplication with
//! `sim_observation_gossip.rs`.

use std::str::FromStr;
use std::time::Duration;

use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

const STEP_SEED: u64 = 0x04_02_BB_BB_BB_BB_BB_BB;
const GOSSIP_WINDOW: Duration = Duration::from_millis(50);
const PAST_CONVERGENCE: Duration = Duration::from_millis(100);

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid node id")
}

fn row_at(writer: &NodeId, counter: u64, state: AllocState) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-tiebreak").expect("valid alloc id"),
        job_id: JobId::from_str("payments").expect("valid job id"),
        node_id: node("node-a"),
        state,
        updated_at: LogicalTimestamp { counter, writer: writer.clone() },
        reason: None,
        detail: None,
    }
}

/// When two writes carry the same counter, the lexicographically greater
/// writer's `NodeId` wins. This is the §4 "deterministic tiebreak" rule
/// and is what makes LWW seed-reproducible across arrival orders.
#[tokio::test(flavor = "current_thread")]
async fn lww_tiebreak_uses_writer_node_id_for_equal_counters() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    // Same counter (1), different writers. "node-b" > "node-a"
    // lexicographically, so peer-B's row must win on every peer.
    let row_from_a = row_at(&node("node-a"), 1, AllocState::Running);
    let row_from_b = row_at(&node("node-b"), 1, AllocState::Draining);

    peer_a
        .write(ObservationRow::AllocStatus(row_from_a.clone()))
        .await
        .expect("write on A succeeds");
    peer_b
        .write(ObservationRow::AllocStatus(row_from_b.clone()))
        .await
        .expect("write on B succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    for (name, peer) in [("node-a", &peer_a), ("node-b", &peer_b), ("node-c", &peer_c)] {
        let latest = peer
            .latest_alloc_status(&row_from_a.alloc_id)
            .expect("peer must have observed some row");
        assert_eq!(
            latest.updated_at.writer,
            node("node-b"),
            "peer {name} must tiebreak to node-b (lexicographically greater writer)"
        );
        assert_eq!(
            latest.state,
            AllocState::Draining,
            "peer {name} must hold the Draining row from node-b"
        );
    }
}

/// `partition(A, B)` blocks gossip in both directions: A→B AND B→A.
/// Without this the partition API would be a footgun that silently only
/// blocks one direction of a conceptually bidirectional separation.
#[tokio::test(flavor = "current_thread")]
async fn partition_blocks_gossip_bidirectionally() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b")])
        .gossip_delay(GOSSIP_WINDOW)
        .partition(node("node-a"), node("node-b"))
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));

    // Write on B, not on A — if partition only blocked A→B (and not
    // B→A) this write would still reach A.
    let row = row_at(&node("node-b"), 1, AllocState::Running);
    peer_b.write(ObservationRow::AllocStatus(row.clone())).await.expect("write on B succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    assert!(
        peer_a.latest_alloc_status(&row.alloc_id).is_none(),
        "peer A must NOT observe B's row — partition must be bidirectional"
    );
}

/// Runtime `.partition()` installed after `build()` takes effect on the
/// next `advance`. Without this test the cluster's runtime partition
/// method is a body-less no-op under mutation; the builder-time
/// partition alone would satisfy the acceptance scenarios.
#[tokio::test(flavor = "current_thread")]
async fn runtime_partition_blocks_subsequent_gossip() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));

    // Install the partition AT RUNTIME, after the cluster is built.
    cluster.partition(&node("node-a"), &node("node-b")).await;

    let row = row_at(&node("node-a"), 1, AllocState::Running);
    peer_a.write(ObservationRow::AllocStatus(row.clone())).await.expect("write on A succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    assert!(
        peer_b.latest_alloc_status(&row.alloc_id).is_none(),
        "peer B must NOT observe A's row — runtime partition must actually install the block"
    );
}

/// Equal-timestamp rows (same counter AND same writer) are idempotent —
/// re-delivery must not flip `LogicalTimestamp::dominates` to `true`.
/// Pins the strict `>` in the tiebreak branch; `>=` would misclassify a
/// re-delivered row as dominant.
#[tokio::test(flavor = "current_thread")]
async fn lww_equal_timestamps_are_idempotent_no_redelivery_flip() {
    // Construct two single-peer stores directly — this test is about
    // the `apply` LWW merge, not the cluster/gossip path.
    let store = SimObservationStore::single_peer(node("node-a"), STEP_SEED);
    let row_v1 = AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-dup").expect("valid alloc id"),
        job_id: JobId::from_str("payments").expect("valid job id"),
        node_id: node("node-a"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 5, writer: node("node-a") },
        reason: None,
        detail: None,
    };
    // Identical timestamp, but a different payload — represents the
    // same logical row being re-delivered via gossip. Under LWW, this
    // is a losing write (not a tiebreak winner), so the ORIGINAL row
    // must be retained on the peer.
    let row_v2_same_ts = AllocStatusRow { state: AllocState::Draining, ..row_v1.clone() };

    store.write(ObservationRow::AllocStatus(row_v1.clone())).await.expect("first write succeeds");
    store
        .write(ObservationRow::AllocStatus(row_v2_same_ts))
        .await
        .expect("second write at same timestamp succeeds but loses LWW");

    let latest = store.latest_alloc_status(&row_v1.alloc_id).expect("row observed");
    assert_eq!(
        latest.state,
        AllocState::Running,
        "equal timestamps must not flip the stored row — strict `>` tiebreak required"
    );
}

/// `repair` is idempotent: calling it on a pair that was never
/// partitioned is a no-op. The sim must not require callers to track
/// partition state externally.
#[tokio::test(flavor = "current_thread")]
async fn repair_on_unpartitioned_pair_is_a_noop() {
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    // Repair a pair that was never partitioned — must not panic.
    cluster.repair(&node("node-a"), &node("node-b")).await;
    // And a second repair — still a no-op.
    cluster.repair(&node("node-a"), &node("node-b")).await;

    // Gossip still flows normally after the no-op repairs.
    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let row = row_at(&node("node-a"), 1, AllocState::Running);
    peer_a.write(ObservationRow::AllocStatus(row.clone())).await.expect("write on A succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    assert_eq!(
        peer_b.latest_alloc_status(&row.alloc_id).expect("peer B must observe A's row").updated_at,
        row.updated_at,
        "gossip must still flow after a no-op repair"
    );
}
