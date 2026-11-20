//! Node-local persistent volume (GH #348, ADR-0317): an allocation row whose reason is
//! `TransitionReason::VmVolumeUnavailable` crosses the observation-store boundary.
//!
//! `LocalObservationStore::write_alloc_lifecycle` archives the row through
//! `AllocStatusRowEnvelope` into redb; `alloc_status_rows` projects it back through
//! `into_latest()`. Every `VolumeUnavailableCause` is written, the store is dropped and
//! reopened, and each row must read back equal: a cause whose archived layout the envelope
//! cannot carry would surface as a skipped or refused row. The variant is appended to the
//! enum `AllocStatusRowV1` archives, so no new row envelope version is expected; the
//! envelope's `discriminant_offset_from_end` is pinned by
//! `overdrive-core/tests/schema_evolution/alloc_status_row.rs`.
//!
//! The variant and `VolumeUnavailableCause` are already scaffolded with their archive
//! derives, so this is a GUARD: it holds before DELIVER and must keep holding when the
//! cause grammar and wording land.
//!
//! Real redb file I/O via `tempfile::TempDir`, as `local_observation_store`.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE lines are machine-read")]

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeUnavailableCause};
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

fn failed_row(index: u32, cause: VolumeUnavailableCause) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(&format!("alloc-dev-box-{index}")).expect("valid alloc id"),
        workload_id: WorkloadId::new("dev-box").expect("valid workload id"),
        node_id: NodeId::new("control-plane-0").expect("valid node id"),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp {
            counter: u64::from(index) + 1,
            writer: NodeId::new("control-plane-0").expect("valid writer node id"),
        },
        reason: Some(TransitionReason::VmVolumeUnavailable { cause }),
        detail: Some("the volume could not be attached".to_owned()),
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// Every cause survives the envelope, a restart of the store included.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_volume_unavailable_row_round_trips_every_cause_across_a_store_reopen() {
    let causes = [
        VolumeUnavailableCause::PoolAbsent,
        VolumeUnavailableCause::InsufficientSpace {
            needed: overdrive_core::testing::volume::volume_size_gib(40),
            free: ByteCount::new(10 * 1024 * 1024 * 1024),
        },
        VolumeUnavailableCause::PoolCommandFailed { operation: VolumeOperation::Create },
        VolumeUnavailableCause::PoolCommandFailed { operation: VolumeOperation::Claim },
        VolumeUnavailableCause::Held,
    ];
    let rows: Vec<AllocStatusRow> =
        causes.into_iter().zip(0_u32..).map(|(cause, index)| failed_row(index, cause)).collect();

    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("observation");
    {
        let store = LocalObservationStore::open(&path).expect("open observation store");
        for row in &rows {
            store
                .write_alloc_lifecycle(row.clone(), TransitionSource::Reconciler)
                .await
                .expect("a volume-unavailable row is written through the envelope");
        }
    }
    let reopened = LocalObservationStore::open(&path).expect("reopen observation store");
    let mut read = reopened.alloc_status_rows().await.expect("read alloc rows");
    read.sort_by(|a, b| a.alloc_id.cmp(&b.alloc_id));
    let mut expected = rows;
    expected.sort_by(|a, b| a.alloc_id.cmp(&b.alloc_id));
    assert_eq!(read, expected, "every volume-unavailable row must read back equal after a reopen");
}
