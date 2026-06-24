//! F-D — REAL broadcast-overflow → `SubscriptionEvent::Lagged` on
//! `SimObservationStore` (the F4 trigger, sim side).
//!
//! `ServiceBackendsResolve`'s relist-on-`Lagged` recovery exists because the
//! watch can drop rows under load. The synthetic resolve-adapter doubles inject
//! `Lagged` directly; this test exercises the REAL
//! `BroadcastStreamRecvError::Lagged(n) → SubscriptionEvent::Lagged` mapping in
//! `SimObservationStore::subscribe_all_events` — the production trigger those
//! doubles bypass (the sim counterpart of the `LocalObservationStore`
//! `real_broadcast_overflow_yields_lagged` acceptance test).
//!
//! Mechanism: open the subscription, then write CAPACITY+k distinct rows WITHOUT
//! draining. The broadcast channel (capacity 1024) overflows; the undrained
//! receiver falls > capacity behind, and the first drained event is a real
//! `Lagged`. Distinct `alloc_id`s ensure every write is an LWW winner that is
//! actually broadcast (a duplicate key would be suppressed and not consume a
//! slot). Default lane (in-memory broadcast).

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationStore, SubscriptionEvent,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Fixed seed — overflow detection is independent of the seed, but a pinned
/// seed keeps the run reproducible.
const STEP_SEED: u64 = 0x01_03_FD_FD_FD_FD_FD_FD;

fn peer_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn flood_row(i: usize) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str(&format!("alloc-flood-{i}")).expect("valid alloc id"),
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
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    }
}

#[tokio::test]
async fn real_broadcast_overflow_yields_lagged() {
    // The `SimObservationStore` fan-out capacity is 1024; write past it.
    const CAPACITY: usize = 1024;
    const OVERFLOW: usize = CAPACITY + 16;

    let store = SimObservationStore::single_peer(peer_node(), STEP_SEED);

    // Subscribe BEFORE the writes — but NEVER drain until after the overflow.
    let mut subscription: LagAwareSubscription =
        store.subscribe_all_events().await.expect("subscribe succeeds");

    // Flood the fan-out with distinct LWW-winner rows; the held-but-undrained
    // receiver falls past capacity and the broadcast drops the oldest values.
    for i in 0..OVERFLOW {
        store
            .write(ObservationRow::AllocStatus(Box::new(flood_row(i))))
            .await
            .expect("write flood row");
    }

    // Now drain. The REAL `BroadcastStreamRecvError::Lagged` mapping must
    // surface the loss as `SubscriptionEvent::Lagged { missed }` — the loss is
    // never silent (the C4 / D-TME-11 completeness contract this surface exists
    // to honour). `missed > 0` and at most the overflow count.
    let event = tokio::time::timeout(Duration::from_secs(2), subscription.next())
        .await
        .expect("subscription yields within deadline")
        .expect("stream is not closed");
    let SubscriptionEvent::Lagged { missed } = event else {
        panic!(
            "an undrained subscription past broadcast capacity must yield a real Lagged, \
             got {event:?}"
        );
    };
    assert!(missed > 0, "a real overflow must report a positive missed count, got {missed}");
    assert!(
        missed <= OVERFLOW as u64,
        "missed ({missed}) cannot exceed the number of rows written ({OVERFLOW})"
    );
}
