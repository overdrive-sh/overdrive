//! §5.1 scenarios 1, 2, 4 and §5.3 partition scenario — multi-peer
//! gossip for step 04-02.
//!
//! A three-peer `SimObservationStore` cluster is constructed with an
//! injectable gossip delay and optional bidirectional partitions. The
//! cluster exposes a per-peer `ObservationStore` face; the harness
//! advances simulated time to let gossip drain.
//!
//! Acceptance criteria covered:
//!
//! * **§5.1 scenario 1 (converges):** peer A writes, harness advances
//!   past the gossip convergence window, peers B and C each read the same
//!   row.
//! * **§5.1 scenario 2 (LWW higher-timestamp wins):** peer A writes at
//!   `T1`, peer B writes a competing row at `T2 > T1`; every peer
//!   converges to the `T2` value, regardless of the order gossip
//!   delivered them in.
//! * **§5.1 scenario 4 (full-row precedence):** the winning row is a
//!   complete replacement — no field-by-field merge is applied.
//! * **§5.3 partition:** a write by peer A is NOT observable on B / C
//!   while A is partitioned from them; after `repair` and another
//!   advance, B and C each read the row.
//!
//! Seeded determinism: every helper here takes a fixed seed and advances
//! logical time through `SimObservationCluster::advance`. No wall-clock
//! reads, no `tokio::time::sleep`.

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationSubscription,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Fixed seed for this step. Step 04-03's proptest will sweep seeds;
/// this step uses one deterministic value per test.
const STEP_SEED: u64 = 0x04_02_AA_AA_AA_AA_AA_AA;

/// Gossip convergence window. A single `advance(Duration)` past this
/// value must be enough to flush the per-peer FIFO across non-partitioned
/// edges.
const GOSSIP_WINDOW: Duration = Duration::from_millis(50);

/// One advance beyond the window gives gossip slack to drain.
const PAST_CONVERGENCE: Duration = Duration::from_millis(100);

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid node id")
}

fn alloc_status(state: AllocState, writer: &NodeId, counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-a1b2c3").expect("valid alloc id"),
        job_id: JobId::from_str("payments").expect("valid job id"),
        // `node_id` records which node owns the alloc; it is the same
        // across all rows for this alloc. The *writer* field on the
        // logical timestamp is what identifies who emitted the update.
        node_id: node("node-a"),
        state,
        updated_at: LogicalTimestamp { counter, writer: writer.clone() },
    }
}

/// Pull every row the subscription currently has buffered, without
/// blocking. Returns them in delivery order.
///
/// The subscription is a `Stream` backed by a `tokio::sync::broadcast`
/// channel under the hood; calling `next().await` on an empty channel
/// would block, so we wrap each poll in a short timeout and stop on the
/// first `None` / timeout.
async fn drain_subscription(subscription: &mut ObservationSubscription) -> Vec<ObservationRow> {
    let mut out = Vec::new();
    // A 25ms budget is far larger than any intra-process broadcast
    // delivery; if we timeout, the channel is genuinely empty right now.
    while let Ok(Some(row)) =
        tokio::time::timeout(Duration::from_millis(25), subscription.next()).await
    {
        out.push(row);
    }
    out
}

// ---------------------------------------------------------------------------
// §5.1 scenario 1 — converges across peers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn row_written_on_one_peer_is_observable_on_every_peer_after_convergence() {
    // Given a three-peer SimObservationStore cluster with a fixed gossip
    // delay.
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    // Open subscriptions before the write so no event is dropped on the
    // peer's fan-out (the §4 guardrail requires we see every row).
    let mut sub_b: ObservationSubscription =
        peer_b.subscribe_all().await.expect("subscribe on peer B succeeds");
    let mut sub_c: ObservationSubscription =
        peer_c.subscribe_all().await.expect("subscribe on peer C succeeds");

    // When peer A writes a full alloc_status row.
    let row = alloc_status(AllocState::Running, &node("node-a"), 1);
    peer_a.write(ObservationRow::AllocStatus(row.clone())).await.expect("write on peer A succeeds");

    // And the simulation advances past the gossip convergence window.
    cluster.advance(PAST_CONVERGENCE).await;

    // Then peers B and C each read the same row A wrote.
    let delivered_b = drain_subscription(&mut sub_b).await;
    let delivered_c = drain_subscription(&mut sub_c).await;

    assert_eq!(
        delivered_b,
        vec![ObservationRow::AllocStatus(row.clone())],
        "peer B must observe the row peer A wrote after convergence"
    );
    assert_eq!(
        delivered_c,
        vec![ObservationRow::AllocStatus(row)],
        "peer C must observe the row peer A wrote after convergence"
    );
}

// ---------------------------------------------------------------------------
// §5.1 scenario 2 — LWW chooses the higher-timestamp update
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn lww_chooses_higher_timestamp_regardless_of_arrival_order() {
    // Given a three-peer cluster.
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    // When two peers write competing rows for the same alloc_id, with
    // peer B's write dominating on the logical timestamp (T2 > T1).
    let row_t1 = alloc_status(AllocState::Running, &node("node-a"), 1);
    let row_t2 = alloc_status(AllocState::Draining, &node("node-b"), 2);

    // Write peer B's later row FIRST, then peer A's earlier row — this
    // forces the LWW merge to happen on receive, on every peer, despite
    // A's write arriving after B's in wall-clock order.
    peer_b
        .write(ObservationRow::AllocStatus(row_t2.clone()))
        .await
        .expect("write on peer B succeeds");
    peer_a
        .write(ObservationRow::AllocStatus(row_t1.clone()))
        .await
        .expect("write on peer A succeeds");

    cluster.advance(PAST_CONVERGENCE).await;

    // Then every peer's final state for this alloc is Draining (the T2
    // value). The LWW merge uses (counter, writer) lex order; counter 2
    // dominates counter 1 regardless of arrival order.
    for (name, peer) in [("node-a", &peer_a), ("node-b", &peer_b), ("node-c", &peer_c)] {
        let latest =
            peer.latest_alloc_status(&row_t2.alloc_id).expect("peer must have observed the T2 row");
        assert_eq!(
            latest.state,
            AllocState::Draining,
            "peer {name} must converge to the T2 Draining row (LWW higher timestamp wins)",
        );
        assert_eq!(
            latest.updated_at.counter, 2,
            "peer {name} must retain the T2 logical timestamp",
        );
    }
}

// ---------------------------------------------------------------------------
// §5.1 scenario 4 — full-row writes take precedence; no partial merge
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn full_row_writes_take_precedence_with_no_partial_merge() {
    // Given a three-peer cluster where every peer already holds a prior
    // alloc_status row at T0 (counter 1, writer node-a).
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    // Seed every peer with a T0 row, then wait for convergence.
    let t0_row = alloc_status(AllocState::Running, &node("node-a"), 1);
    peer_a.write(ObservationRow::AllocStatus(t0_row.clone())).await.expect("seed T0 write");
    cluster.advance(PAST_CONVERGENCE).await;

    // When a third peer writes a full updated row at T1 > T0. The new
    // row differs from the prior in state (Draining), job_id (scheduler
    // moved the alloc), and timestamp — a combination a partial-field
    // merge would never reconstruct correctly.
    let t1_row = AllocStatusRow {
        alloc_id: t0_row.alloc_id.clone(),
        // Different job_id — if any peer applied a partial merge keeping
        // the old job_id, the assertion below would catch it.
        job_id: JobId::from_str("billing").expect("valid job id"),
        node_id: node("node-a"),
        state: AllocState::Draining,
        updated_at: LogicalTimestamp { counter: 2, writer: node("node-c") },
    };
    peer_c
        .write(ObservationRow::AllocStatus(t1_row.clone()))
        .await
        .expect("T1 write on peer C succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    // Then every peer converges to the row peer C wrote. No partial
    // merge: the winning row's job_id is "billing" (not "payments" from
    // T0), its state is Draining (not Running from T0), and the
    // timestamp is T1 wholesale.
    for (name, peer) in [("node-a", &peer_a), ("node-b", &peer_b), ("node-c", &peer_c)] {
        let latest =
            peer.latest_alloc_status(&t1_row.alloc_id).expect("peer must have observed the T1 row");
        assert_eq!(
            latest, t1_row,
            "peer {name} must hold the exact T1 row — no partial-field merge",
        );
    }
}

// ---------------------------------------------------------------------------
// §5.3 partition scenario
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn partition_blocks_gossip_delivery_until_repair() {
    // Given a three-peer cluster with peer A partitioned from B and C.
    let cluster = SimObservationStore::cluster_builder()
        .peers([node("node-a"), node("node-b"), node("node-c")])
        .gossip_delay(GOSSIP_WINDOW)
        .partition(node("node-a"), node("node-b"))
        .partition(node("node-a"), node("node-c"))
        .seed(STEP_SEED)
        .build();

    let peer_a = cluster.peer(&node("node-a"));
    let peer_b = cluster.peer(&node("node-b"));
    let peer_c = cluster.peer(&node("node-c"));

    let mut sub_b: ObservationSubscription =
        peer_b.subscribe_all().await.expect("subscribe on peer B succeeds");
    let mut sub_c: ObservationSubscription =
        peer_c.subscribe_all().await.expect("subscribe on peer C succeeds");

    // When peer A writes a row and the sim advances past the usual
    // gossip window.
    let row = alloc_status(AllocState::Running, &node("node-a"), 1);
    peer_a.write(ObservationRow::AllocStatus(row.clone())).await.expect("write on peer A succeeds");
    cluster.advance(PAST_CONVERGENCE).await;

    // Then peers B and C do NOT yet observe the row.
    let pre_b = drain_subscription(&mut sub_b).await;
    let pre_c = drain_subscription(&mut sub_c).await;
    assert!(pre_b.is_empty(), "peer B must NOT observe A's row while partitioned (got {pre_b:?})");
    assert!(pre_c.is_empty(), "peer C must NOT observe A's row while partitioned (got {pre_c:?})");

    // When the partition heals and the sim advances past convergence.
    cluster.repair(&node("node-a"), &node("node-b")).await;
    cluster.repair(&node("node-a"), &node("node-c")).await;
    cluster.advance(PAST_CONVERGENCE).await;

    // Then peers B and C each read the row A wrote.
    let post_b = drain_subscription(&mut sub_b).await;
    let post_c = drain_subscription(&mut sub_c).await;

    assert_eq!(
        post_b,
        vec![ObservationRow::AllocStatus(row.clone())],
        "peer B must observe A's row after partition heals and gossip converges"
    );
    assert_eq!(
        post_c,
        vec![ObservationRow::AllocStatus(row)],
        "peer C must observe A's row after partition heals and gossip converges"
    );
}
