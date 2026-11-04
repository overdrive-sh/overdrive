//! Schema-evolution golden-bytes test: `WorkloadSvidIssueStatusRowEnvelope`
//! (SVID issuance failure occurrence, table `observation_workload_svid_issue_status`).
//! Pins the V1 archived layout of the per-allocation failure row.
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds a new
//! `FIXTURE_V2` constant and a new assertion in the same commit
//! (`development.md` § "Version-bump procedure"). The payload is a frozen
//! hand-pinned `WorkloadSvidIssueStatusRowEnvelope::V1`, never built through an
//! alias.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, WorkloadSvidIssueStatusRowEnvelope, WorkloadSvidIssueStatusRowV1,
};

use super::harness::{assert_envelope_v_roundtrip, decode_envelope_v_latest};

/// Hex-encoded rkyv-archived bytes of `WorkloadSvidIssueStatusRowEnvelope::V1`:
/// allocation `alloc-payments-0` on node `node-001`, four failed issuances whose
/// latest is a `Ca` failure at 1 700 000 123 s, and the logical timestamp
/// `(4, node-001)`.
const FIXTURE_V1: &str = "616c6c6f632d7061796d656e74732d30000000000000000090000000e8ffffff6e6f64652d303031040000000000000000000000000000007bf1536500000000000000000000000004000000000000006e6f64652d303031";

fn alloc() -> AllocationId {
    AllocationId::new("alloc-payments-0").expect("valid allocation id")
}

fn node() -> NodeId {
    NodeId::new("node-001").expect("valid node id")
}

fn canonical_v1_payload() -> WorkloadSvidIssueStatusRowV1 {
    WorkloadSvidIssueStatusRowV1 {
        alloc_id: alloc(),
        node_id: node(),
        issue_failures: 4,
        last_issue_failure: SvidIssueFailure {
            cause: SvidIssueFailureCause::Ca,
            at: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_123)),
        },
        updated_at: LogicalTimestamp { counter: 4, writer: node() },
    }
}

/// O-SIF-W5: the frozen V1 bytes decode through the current envelope into the
/// canonical payload.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_svid_issue_status_v1_decodes_through_current_envelope() {
    assert_envelope_v_roundtrip::<WorkloadSvidIssueStatusRowEnvelope>(
        FIXTURE_V1,
        &canonical_v1_payload(),
    );
    let latest = decode_envelope_v_latest::<WorkloadSvidIssueStatusRowEnvelope>(FIXTURE_V1);
    assert_eq!(latest.alloc_id, alloc());
    assert_eq!(latest.issue_failures, 4);
    assert_eq!(latest.last_issue_failure.cause, SvidIssueFailureCause::Ca);
}

mod round_trip {
    use std::time::Duration;

    use overdrive_core::UnixInstant;
    use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
    use overdrive_core::codec::{VersionedEnvelope, decode_envelope_bytes};
    use overdrive_core::traits::observation_store::{
        LogicalTimestamp, WorkloadSvidIssueStatusRowEnvelope,
    };
    use proptest::prelude::*;

    use super::{FIXTURE_V1, decode_envelope_v_latest};

    fn cause() -> impl Strategy<Value = SvidIssueFailureCause> {
        prop_oneof![Just(SvidIssueFailureCause::Ca), Just(SvidIssueFailureCause::Audit)]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// O-SIF-W5: for every recorded occurrence (a count of at least one
        /// with its latest failure and logical timestamp), the row round-trips
        /// bit-equivalent through the envelope archive and its decoder.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn the_row_round_trips_every_occurrence_through_the_envelope(
            count in 1_u64..=u64::MAX,
            failure_cause in cause(),
            at_secs in 0_u64..4_000_000_000,
            counter in 1_u64..=u64::MAX,
        ) {
            let mut row =
                decode_envelope_v_latest::<WorkloadSvidIssueStatusRowEnvelope>(FIXTURE_V1);
            row.issue_failures = count;
            row.last_issue_failure = SvidIssueFailure {
                cause: failure_cause,
                at: UnixInstant::from_unix_duration(Duration::from_secs(at_secs)),
            };
            row.updated_at = LogicalTimestamp { counter, writer: row.node_id.clone() };
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(
                &WorkloadSvidIssueStatusRowEnvelope::latest(row.clone()),
            )
            .expect("archive the row");
            let decoded =
                decode_envelope_bytes::<WorkloadSvidIssueStatusRowEnvelope>(bytes.as_ref())
                    .expect("decode the archived row");
            prop_assert_eq!(decoded, row);
        }
    }
}
