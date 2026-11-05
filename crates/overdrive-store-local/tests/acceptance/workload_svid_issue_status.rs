//! The workload SVID issuance failure record is the same operation in both
//! `ObservationStore` adapters, and it is atomic (R-46; O-SIF-W2).
//!
//! `record_workload_svid_issue_failure` is the only authoring route of the
//! `observation_workload_svid_issue_status` table: it reads the prior row, installs
//! `issue_failures = prior + 1` and the latest failure, and stamps the row's
//! logical timestamp, as one mutation. These cases drive the operation through
//! `LocalObservationStore` (real redb in a `TempDir`) and `SimObservationStore`
//! (the real-adapter pairing for the Sim store, which the control-plane
//! executor and handler tests use) and assert the point read after every call,
//! that no other observation family changes, and that concurrent calls on one
//! allocation never lose a count.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required scenario metadata")]

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use overdrive_core::UnixInstant;
use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::observation_store::{
    LagAwareSubscription, ObservationStore, SubscriptionEvent,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

fn alloc(raw: &str) -> AllocationId {
    AllocationId::new(raw).expect("valid allocation id")
}

fn node() -> NodeId {
    NodeId::new("record-node").expect("valid node id")
}

const fn failure(cause: SvidIssueFailureCause, secs: u64) -> SvidIssueFailure {
    SvidIssueFailure { cause, at: UnixInstant::from_unix_duration(Duration::from_secs(secs)) }
}

/// Drain every event already delivered to `sub` without waiting.
fn drain(sub: &mut LagAwareSubscription) -> Vec<SubscriptionEvent> {
    let mut events = Vec::new();
    while let Some(Some(event)) = sub.next().now_or_never() {
        events.push(event);
    }
    events
}

/// One store of each adapter, over the same node identity.
struct Pair {
    _dir: TempDir,
    local: LocalObservationStore,
    sim: SimObservationStore,
}

impl Pair {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let local = LocalObservationStore::open(dir.path().join("observation.redb"))
            .expect("open local observation store");
        let sim = SimObservationStore::single_peer(node(), 0x51F);
        Self { _dir: dir, local, sim }
    }

    fn stores(&self) -> [(&'static str, &dyn ObservationStore); 2] {
        [("local", &self.local), ("sim", &self.sim)]
    }
}

/// O-SIF-W2 (equivalence, bounded change): one sequence of recordings for two
/// allocations gives, in both adapters after every call, the same point read:
/// a count that rises by exactly one, the latest failure, a logical counter that
/// rises by one from 1 (writer = the recording node), no row for an allocation
/// that never failed; and the operation changes no other observation family and
/// emits no event. The port-expressible universe of "other families" is every
/// family with an unscoped bulk read (`alloc_status_rows`, `node_health_rows`,
/// `issued_certificate_rows`, `all_service_backends_rows`,
/// `workflow_terminal_rows`) plus the `subscribe_all_events` stream; a key-only
/// family is not visible through the port and is bounded by the operation
/// opening one table.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_failure_record_is_the_same_operation_in_the_local_and_sim_stores() {
    let pair = Pair::new();
    let mut subscriptions = Vec::new();
    for (_, store) in pair.stores() {
        subscriptions.push(store.subscribe_all_events().await.expect("subscribe"));
    }
    let sequence = [
        (alloc("alloc-a-0"), failure(SvidIssueFailureCause::Ca, 100)),
        (alloc("alloc-a-0"), failure(SvidIssueFailureCause::Audit, 200)),
        (alloc("alloc-b-0"), failure(SvidIssueFailureCause::Ca, 300)),
        (alloc("alloc-a-0"), failure(SvidIssueFailureCause::Audit, 400)),
    ];
    let mut expected_count = std::collections::BTreeMap::<AllocationId, u64>::new();

    for (step, (target, recorded)) in sequence.into_iter().enumerate() {
        *expected_count.entry(target.clone()).or_default() += 1;
        for (name, store) in pair.stores() {
            store
                .record_workload_svid_issue_failure(&target, &node(), recorded)
                .await
                .unwrap_or_else(|error| panic!("{name} step {step}: record failed: {error}"));
            let row = store
                .workload_svid_issue_status_row(&target)
                .await
                .unwrap_or_else(|error| panic!("{name} step {step}: read failed: {error}"))
                .unwrap_or_else(|| panic!("{name} step {step}: a recorded failure has a row"));
            assert_eq!(row.alloc_id, target, "{name} step {step}");
            assert_eq!(row.node_id, node(), "{name} step {step}");
            assert_eq!(row.issue_failures, expected_count[&target], "{name} step {step}");
            assert_eq!(row.last_issue_failure, recorded, "{name} step {step}");
            assert_eq!(
                row.updated_at.counter, expected_count[&target],
                "{name} step {step}: the logical counter rises by one from one",
            );
            assert_eq!(row.updated_at.writer, node(), "{name} step {step}");
        }
    }

    for (name, store) in pair.stores() {
        assert_eq!(
            store
                .workload_svid_issue_status_row(&alloc("alloc-never-failed-0"))
                .await
                .expect("read"),
            None,
            "{name}: no row for an allocation that never failed",
        );
        assert!(store.alloc_status_rows().await.expect("alloc rows").is_empty(), "{name}");
        assert!(store.issued_certificate_rows().await.expect("audit rows").is_empty(), "{name}");
        assert!(store.node_health_rows().await.expect("node rows").is_empty(), "{name}");
        assert!(
            store.all_service_backends_rows().await.expect("service backend rows").is_empty(),
            "{name}",
        );
        assert!(
            store.workflow_terminal_rows().await.expect("workflow terminal rows").is_empty(),
            "{name}",
        );
    }
    for subscription in &mut subscriptions {
        assert!(drain(subscription).is_empty(), "the record operation emits no observation event");
    }
}

/// O-SIF-W2 (atomicity): concurrent recordings on one allocation never lose a
/// count, in either adapter: after `CALLS` simultaneous calls (released by one
/// barrier) the count equals `CALLS`, whatever the interleaving.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_recordings_on_one_allocation_never_lose_a_count() {
    const CALLS: u64 = 32;

    let pair = Pair::new();
    for (name, store) in pair.stores() {
        let store: &dyn ObservationStore = store;
        let target = alloc("alloc-concurrent-0");
        let barrier = Arc::new(tokio::sync::Barrier::new(usize::try_from(CALLS).expect("calls")));
        let recordings = (0..CALLS).map(|call| {
            let barrier = Arc::clone(&barrier);
            let target = target.clone();
            async move {
                barrier.wait().await;
                store
                    .record_workload_svid_issue_failure(
                        &target,
                        &node(),
                        failure(SvidIssueFailureCause::Audit, 1_000 + call),
                    )
                    .await
                    .expect("record");
            }
        });
        futures::future::join_all(recordings).await;
        let row = store
            .workload_svid_issue_status_row(&target)
            .await
            .expect("read")
            .expect("a row after recordings");
        assert_eq!(row.issue_failures, CALLS, "{name}: a recording was lost");
        assert_eq!(row.updated_at.counter, CALLS, "{name}");
    }
}
