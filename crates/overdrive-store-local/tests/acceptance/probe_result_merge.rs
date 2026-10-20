//! R5-PROBE-MERGE — the probe row merges by `run_seq` identically in both
//! `ObservationStore` adapters (netns-density-295 PROBE-RUN-COUNT, ADR-0191;
//! the `write_probe_result` postconditions).
//!
//! One sequence of `write_probe_result` calls on one key — a greater
//! `run_seq`, an equal one, a smaller one, a greater `run_seq` with an
//! earlier `last_observed_at_unix_ms`, and a failed write of a greater
//! `run_seq` followed by a write of the same `run_seq` — is driven through
//! `LocalObservationStore` (real redb in a `TempDir`) and
//! `SimObservationStore`. After every call `list_probe_results_for_alloc`
//! returns the expected row from both, and a `SubscriptionEvent::Row` is
//! emitted exactly for the accepted writes.
//!
//! This is the real-adapter pairing for `SimObservationStore` as the named
//! arbiter of the stored row in the `probe-count` conformance driver. The
//! host adapter's failed-write and undecodable-stored-row cases need its
//! source-local fault seam and live beside it
//! (`src/observation_backend.rs`, `probe_merge_tests`); the sim adapter's
//! failure is `SimObservationStore::inject_write_failure`.

#![allow(
    clippy::doc_markdown,
    clippy::manual_is_multiple_of,
    reason = "DISTILL test support and conformance drivers: handler signatures follow the action map and fixtures assert their preconditions"
)]

use futures::FutureExt as _;
use futures::StreamExt as _;
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::traits::observation_store::{
    LagAwareSubscription, ObservationRow, ObservationStore, ObservationStoreError,
    SubscriptionEvent,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

fn alloc() -> AllocationId {
    AllocationId::new("alloc-merge").expect("valid alloc id")
}

/// A liveness row of probe 0 with the given run sequence and observation
/// time. The status alternates with the run so an accepted replacement is
/// visible in the stored row.
fn row(run_seq: u64, observed_at: u64) -> ProbeResultRow {
    ProbeResultRow {
        alloc_id: alloc(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Liveness,
        status: if run_seq % 2 == 0 {
            ProbeStatus::Pass
        } else {
            ProbeStatus::Fail { last_fail_reason: format!("refused at run {run_seq}") }
        },
        last_observed_at_unix_ms: observed_at,
        inferred: false,
        run_seq: ProbeRunSeq::new(run_seq).expect("non-zero run sequence"),
        same_status_runs: SameStatusRuns::ONE,
    }
}

/// One call of the merge sequence and its oracle.
enum Call {
    /// `write_probe_result(row)` returns `Ok`; `accepted` says whether the
    /// merge replaces the stored row (and emits exactly one event).
    Write { row: ProbeResultRow, accepted: bool },
    /// `write_probe_result(row)` fails: `Err`, no state change, no event.
    Failed { row: ProbeResultRow },
}

/// The sequence, with the stored row expected after each call.
fn merge_sequence() -> Vec<(Call, ProbeResultRow)> {
    vec![
        // First row of the key: stored.
        (Call::Write { row: row(1, 1_000), accepted: true }, row(1, 1_000)),
        // Greater run_seq: replaces.
        (Call::Write { row: row(2, 2_000), accepted: true }, row(2, 2_000)),
        // Equal run_seq, later observation time: dropped.
        (Call::Write { row: row(2, 9_000), accepted: false }, row(2, 2_000)),
        // Smaller run_seq, later observation time: dropped.
        (Call::Write { row: row(1, 9_500), accepted: false }, row(2, 2_000)),
        // Greater run_seq, EARLIER observation time (a backward wall-clock
        // step between runs): replaces — the observation time does not
        // decide the merge.
        (Call::Write { row: row(3, 500), accepted: true }, row(3, 500)),
        // A failed write of a greater run_seq: nothing changes.
        (Call::Failed { row: row(4, 4_000) }, row(3, 500)),
        // The same run_seq written after the failure: merged against the
        // row stored before it, and accepted.
        (Call::Write { row: row(4, 4_000), accepted: true }, row(4, 4_000)),
    ]
}

/// Drain every event already delivered to `sub` without waiting.
fn drain(sub: &mut LagAwareSubscription) -> Vec<ObservationRow> {
    let mut rows = Vec::new();
    while let Some(Some(event)) = sub.next().now_or_never() {
        match event {
            SubscriptionEvent::Row(row) => rows.push(row),
            SubscriptionEvent::Lagged { missed } => {
                panic!("subscription lagged ({missed}) on a handful of writes")
            }
        }
    }
    rows
}

/// Both adapters, after every call of one sequence, hold the same expected
/// row and emit an event exactly for the accepted writes.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn probe_row_merges_by_run_sequence_identically_in_both_stores() {
    let temp = TempDir::new().expect("tempdir");
    let local = LocalObservationStore::open(temp.path().join("observation.redb"))
        .expect("open local observation store");
    let sim = SimObservationStore::single_peer(NodeId::new("merge-node").expect("node id"), 0);
    let mut local_sub = local.subscribe_all_events().await.expect("subscribe local");
    let mut sim_sub = sim.subscribe_all_events().await.expect("subscribe sim");

    for (step, (call, expected_stored)) in merge_sequence().into_iter().enumerate() {
        match call {
            Call::Write { row, accepted } => {
                local.write_probe_result(row.clone()).await.expect("local write");
                sim.write_probe_result(row.clone()).await.expect("sim write");
                let expected_events =
                    if accepted { vec![ObservationRow::ProbeResult(row)] } else { Vec::new() };
                assert_eq!(drain(&mut local_sub), expected_events, "local events at step {step}");
                assert_eq!(drain(&mut sim_sub), expected_events, "sim events at step {step}");
            }
            Call::Failed { row } => {
                // The host adapter's failure needs its source-local seam
                // (`probe_merge_tests`); the sim's is the injected failure.
                sim.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
                    "injected probe-row write failure",
                )));
                sim.write_probe_result(row).await.expect_err("sim write must fail");
                assert!(drain(&mut sim_sub).is_empty(), "a failed write emits no event");
                assert!(drain(&mut local_sub).is_empty());
            }
        }
        let local_rows = local.list_probe_results_for_alloc(&alloc()).await.expect("local read");
        let sim_rows = sim.list_probe_results_for_alloc(&alloc()).await.expect("sim read");
        assert_eq!(local_rows, vec![expected_stored.clone()], "local stored row at step {step}");
        assert_eq!(sim_rows, vec![expected_stored], "sim stored row at step {step}");
    }
}

/// Rows differing only in role, or only in probe index, are different keys:
/// each keeps its own greatest-`run_seq` row in both stores.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn each_key_keeps_its_own_greatest_run_in_both_stores() {
    let temp = TempDir::new().expect("tempdir");
    let local = LocalObservationStore::open(temp.path().join("observation.redb"))
        .expect("open local observation store");
    let sim = SimObservationStore::single_peer(NodeId::new("merge-node").expect("node id"), 0);

    let keyed = |role: ProbeRole, idx: u32, run_seq: u64, observed_at: u64| ProbeResultRow {
        role,
        probe_idx: ProbeIdx::new(idx),
        ..row(run_seq, observed_at)
    };
    // Each key receives a later run with an earlier observation time.
    let writes = [
        keyed(ProbeRole::Readiness, 0, 1, 5_000),
        keyed(ProbeRole::Liveness, 0, 1, 5_000),
        keyed(ProbeRole::Liveness, 1, 1, 5_000),
        keyed(ProbeRole::Readiness, 0, 2, 100),
        keyed(ProbeRole::Liveness, 0, 2, 100),
        keyed(ProbeRole::Liveness, 1, 2, 100),
    ];
    for write in writes {
        local.write_probe_result(write.clone()).await.expect("local write");
        sim.write_probe_result(write).await.expect("sim write");
    }
    let expected = vec![
        keyed(ProbeRole::Readiness, 0, 2, 100),
        keyed(ProbeRole::Liveness, 0, 2, 100),
        keyed(ProbeRole::Liveness, 1, 2, 100),
    ];
    assert_eq!(local.list_probe_results_for_alloc(&alloc()).await.expect("local"), expected);
    assert_eq!(sim.list_probe_results_for_alloc(&alloc()).await.expect("sim"), expected);
}
