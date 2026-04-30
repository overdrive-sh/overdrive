//! Acceptance tests for
//! `SimObservationStore::{alloc_status_rows, node_health_rows}`.
//!
//! These trait-method implementations are the reader surface that
//! `overdrive-control-plane::handlers::{alloc_status, node_list}`
//! project onto the REST wire. A mutation that replaces the body with
//! `Ok(vec![])` leaves the wire empty regardless of what the
//! observation store actually holds — an integrity bug that every
//! operator-facing read would mask.
//!
//! The integration suite exercises this end-to-end over real HTTP,
//! but the default mutation run does not compile the integration
//! lane. These acceptance tests exercise the reader methods directly
//! against in-memory `SimObservationStore` instances so the "reads
//! preserve writes" contract is pinned in the default lane.

use std::str::FromStr;

use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

const STEP_SEED: u64 = 0xA110_0A11_A11A_0A11;

fn peer() -> NodeId {
    NodeId::from_str("node-alloc-test").expect("valid node id")
}

fn alloc_row(alloc: &str, state: AllocState, counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str(alloc).expect("valid alloc id"),
        job_id: JobId::from_str("payments").expect("valid job id"),
        node_id: peer(),
        state,
        updated_at: LogicalTimestamp { counter, writer: peer() },
        reason: None,
        detail: None,
    }
}

fn node_row(node: &str, region: &str, counter: u64) -> NodeHealthRow {
    NodeHealthRow {
        node_id: NodeId::from_str(node).expect("valid node id"),
        region: Region::from_str(region).expect("valid region"),
        last_heartbeat: LogicalTimestamp { counter, writer: peer() },
    }
}

// ---------------------------------------------------------------------------
// alloc_status_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn alloc_status_rows_on_fresh_store_is_empty() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let rows = store.alloc_status_rows().await.expect("alloc_status_rows");
    assert!(rows.is_empty(), "fresh store must produce zero alloc rows; got {rows:?}");
}

#[tokio::test]
async fn alloc_status_rows_returns_written_row_exactly() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let row = alloc_row("alloc-r1", AllocState::Running, 1);
    store.write(ObservationRow::AllocStatus(row.clone())).await.expect("write");

    let rows = store.alloc_status_rows().await.expect("alloc_status_rows");

    // Must contain EXACTLY the row written. An `Ok(vec![])` mutation
    // would return zero rows — caught immediately.
    assert_eq!(
        rows.len(),
        1,
        "alloc_status_rows must surface the single written row; got {} rows",
        rows.len(),
    );
    assert_eq!(
        rows[0], row,
        "surfaced row must equal the written row byte-for-byte (field-by-field)",
    );
}

#[tokio::test]
async fn alloc_status_rows_surfaces_each_distinct_alloc_id() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let r1 = alloc_row("alloc-1", AllocState::Pending, 1);
    let r2 = alloc_row("alloc-2", AllocState::Running, 2);
    let r3 = alloc_row("alloc-3", AllocState::Terminated, 3);
    store.write(ObservationRow::AllocStatus(r1.clone())).await.expect("write r1");
    store.write(ObservationRow::AllocStatus(r2.clone())).await.expect("write r2");
    store.write(ObservationRow::AllocStatus(r3.clone())).await.expect("write r3");

    let rows = store.alloc_status_rows().await.expect("alloc_status_rows");
    assert_eq!(
        rows.len(),
        3,
        "three distinct alloc_ids must produce three surfaced rows; got {}",
        rows.len(),
    );

    // Every written row must appear in the output. Ordering is a
    // BTreeMap snapshot (deterministic by AllocationId) — we assert
    // set membership so future ordering tweaks do not spuriously
    // regress this check.
    let alloc_ids: std::collections::HashSet<AllocationId> =
        rows.iter().map(|r| r.alloc_id.clone()).collect();
    assert!(alloc_ids.contains(&r1.alloc_id), "alloc-1 must appear");
    assert!(alloc_ids.contains(&r2.alloc_id), "alloc-2 must appear");
    assert!(alloc_ids.contains(&r3.alloc_id), "alloc-3 must appear");
}

// ---------------------------------------------------------------------------
// node_health_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn node_health_rows_on_fresh_store_is_empty() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let rows = store.node_health_rows().await.expect("node_health_rows");
    assert!(rows.is_empty(), "fresh store must produce zero node rows; got {rows:?}");
}

#[tokio::test]
async fn node_health_rows_returns_written_row_exactly() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let row = node_row("node-a", "eu-west-1", 1);
    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("write");

    let rows = store.node_health_rows().await.expect("node_health_rows");

    assert_eq!(
        rows.len(),
        1,
        "node_health_rows must surface the single written row; got {} rows",
        rows.len(),
    );
    assert_eq!(rows[0], row, "surfaced row must equal the written row byte-for-byte");
}

#[tokio::test]
async fn node_health_rows_surfaces_each_distinct_node_id() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    let n1 = node_row("node-a", "eu-west-1", 1);
    let n2 = node_row("node-b", "us-east-1", 2);
    store.write(ObservationRow::NodeHealth(n1.clone())).await.expect("write n1");
    store.write(ObservationRow::NodeHealth(n2.clone())).await.expect("write n2");

    let rows = store.node_health_rows().await.expect("node_health_rows");
    assert!(rows.len() >= 2, "both node rows must appear; got {} rows: {rows:?}", rows.len());

    let ids: std::collections::HashSet<NodeId> = rows.iter().map(|r| r.node_id.clone()).collect();
    assert!(ids.contains(&n1.node_id), "node-a must appear");
    assert!(ids.contains(&n2.node_id), "node-b must appear");
}

// ---------------------------------------------------------------------------
// Writer projection: alloc writes do NOT surface via node_health_rows
// and vice versa. Catches a mutation that swaps the two implementations
// (both would return `Ok(vec![])`, so this is belt-and-braces — if
// only ONE mutates, we still distinguish).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn alloc_and_node_rows_do_not_cross_project() {
    let store = SimObservationStore::single_peer(peer(), STEP_SEED);
    store
        .write(ObservationRow::AllocStatus(alloc_row("alloc-r1", AllocState::Running, 1)))
        .await
        .expect("write alloc");
    store
        .write(ObservationRow::NodeHealth(node_row("node-a", "eu-west-1", 1)))
        .await
        .expect("write node");

    let allocs = store.alloc_status_rows().await.expect("alloc_status_rows");
    let nodes = store.node_health_rows().await.expect("node_health_rows");

    assert_eq!(allocs.len(), 1, "alloc_status_rows must only surface alloc writes; got {allocs:?}");
    assert_eq!(nodes.len(), 1, "node_health_rows must only surface node writes; got {nodes:?}");
}
