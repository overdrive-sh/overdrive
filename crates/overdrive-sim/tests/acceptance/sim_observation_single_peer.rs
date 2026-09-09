//! §5.1 scenario 1 — reduced to one peer for step 04-01.
//!
//! A row written on a one-peer `SimObservationStore` is observable on that
//! peer. Multi-peer gossip is step 04-02; LWW convergence is 04-03.
//!
//! The acceptance criterion:
//!
//! > Constructing a one-peer `SimObservationStore` with a fixed seed,
//! > writing an `alloc_status` row for `alloc/a1b2c3`, reading via the
//! > `ObservationStore` subscription surface yields the same row bytes
//! > written.
//!
//! "Bytes" here is interpreted as *row equality after a typed
//! round-trip*: the write is a typed `write_alloc_lifecycle(current, source)`
//! value and the subscription yields the same typed value. Strict byte
//! equality (rkyv archive) becomes load-bearing once production
//! `CorrosionStore` is introduced (Phase 2+); for the sim path, value
//! equality over the typed row is the contract the §4 guardrail
//! ("full-row writes") actually exercises.

#![allow(clippy::doc_markdown)]

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationStore, SubscriptionEvent,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Fixed seed for this step. Multi-peer / gossip determinism lives in
/// steps 04-02 / 04-03.
const STEP_SEED: u64 = 0x04_01_AA_AA_AA_AA_AA_AA;

/// Canonical node id used as the sole peer in this scenario.
fn peer_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn sample_alloc_status() -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-a1b2c3").expect("valid alloc id"),
        workload_id: WorkloadId::from_str("payments").expect("valid job id"),
        node_id: peer_node(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: peer_node() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        // GAP-1 subsidiary: Running state carries fixed wall-clock.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

#[tokio::test]
async fn written_alloc_status_is_observable_on_same_peer() {
    // Given a single-peer SimObservationStore with a fixed seed.
    let store = SimObservationStore::single_peer(peer_node(), STEP_SEED);

    // Given a subscription opened before the write so no event is
    // silently dropped on the peer's fan-out.
    let mut subscription: LagAwareSubscription =
        store.subscribe_all_events().await.expect("subscribe succeeds");

    // When the peer writes an alloc_status row for alloc/a1b2c3.
    let row = sample_alloc_status();
    store
        .write_alloc_lifecycle(
            row.clone(),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("write succeeds on sole peer");

    // Then the subscription yields the row that was written.
    let delivered = tokio::time::timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("subscription delivers within deadline")
        .expect("subscription stream is not closed");

    // Single write, immediate drain — lag is structurally impossible; a
    // `Lagged` here is a real bug, surfaced loudly rather than skipped.
    let SubscriptionEvent::Row(delivered) = delivered else {
        panic!("subscription lagged draining a single written row: {delivered:?}");
    };
    assert_eq!(
        delivered,
        ObservationRow::AllocStatus(Box::new(row)),
        "subscription must yield the same typed row the peer wrote"
    );
}

/// E11 amendment — an accepted probe-result LWW winner is delivered as the
/// dedicated live observation event; stale/equal writes remain silent while
/// the latest row remains available through the existing point-in-alloc list.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn accepted_probe_result_emits_once_and_lww_losers_are_silent() {
    let node = peer_node();
    let store = SimObservationStore::single_peer(node, STEP_SEED);
    let alloc_id = AllocationId::from_str("alloc-probe-event").expect("valid alloc id");
    let initial = ProbeResultRow {
        alloc_id: alloc_id.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Readiness,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 42_000,
        inferred: false,
    };
    let newer = ProbeResultRow {
        last_observed_at_unix_ms: 50_000,
        status: ProbeStatus::Fail { last_fail_reason: "not ready".to_owned() },
        ..initial.clone()
    };
    let stale = ProbeResultRow { last_observed_at_unix_ms: 40_000, ..initial.clone() };
    let equal = ProbeResultRow {
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 42_000,
        ..initial.clone()
    };

    let mut events = store.subscribe_all_events().await.expect("subscribe");
    store.write_probe_result(initial.clone()).await.expect("initial write");
    let first = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("initial accepted event arrives")
        .expect("subscription remains open");
    assert_eq!(
        first,
        SubscriptionEvent::Row(ObservationRow::ProbeResult(initial.clone())),
        "the first event is the accepted probe-result row",
    );

    store.write_probe_result(stale).await.expect("stale write is a no-op");
    store.write_probe_result(equal).await.expect("equal write is a no-op");
    assert!(
        tokio::time::timeout(Duration::from_millis(25), events.next()).await.is_err(),
        "LWW losers must not emit probe-result events",
    );

    store.write_probe_result(newer.clone()).await.expect("newer write");
    let second = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("newer accepted event arrives")
        .expect("subscription remains open");
    assert_eq!(
        second,
        SubscriptionEvent::Row(ObservationRow::ProbeResult(newer.clone())),
        "the newer accepted row is emitted exactly once",
    );
    let durable =
        store.list_probe_results_for_alloc(&alloc_id).await.expect("list probe-result winner");
    assert_eq!(durable, vec![newer], "durable LWW winner matches the event");
}
