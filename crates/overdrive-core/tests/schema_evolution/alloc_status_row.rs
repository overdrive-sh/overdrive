//! Golden-bytes coverage for the current allocation-status V1 envelope.
//!
//! The greenfield cut deliberately has one incompatible V1 payload. The
//! fixture below is the sole current baseline; historical payloads and
//! compatibility readers are not part of this contract.

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, LogicalTimestamp,
};
use overdrive_core::transition_reason::TransitionReason;

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 416;

fn canonical_v1_payload() -> AllocStatusRowLatest {
    AllocStatusRowV1 {
        alloc_id: AllocationId::new("alloc-test-01").expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: Some(TransitionReason::Started),
        detail: Some("current V1 allocation status".to_owned()),
        terminal: None,
        stderr_tail: Some("bounded VMM stderr".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: Some(Ipv4Addr::new(10, 99, 0, 6)),
        last_terminated: None,
        restart_count: 0,
    }
}

// Generated from `AllocStatusRowEnvelope::latest(canonical_v1_payload())`.
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031010000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &canonical_v1_payload());
}

#[test]
fn alloc_status_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

#[test]
fn alloc_status_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        "AllocStatusRowEnvelope",
        0,
    );
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when the current V1 payload changes"]
#[allow(clippy::print_stdout, reason = "fixture regeneration output")]
fn print_fixture_and_discriminant_offset() {
    let envelope = AllocStatusRowEnvelope::latest(canonical_v1_payload());
    let bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("archive allocation status");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
    for offset in 1..=bytes.len() {
        let mut candidate = bytes.as_ref().to_vec();
        let idx = candidate.len() - offset;
        candidate[idx] = 99;
        if let Err(error) =
            rkyv::from_bytes::<AllocStatusRowEnvelope, rkyv::rancor::Error>(&candidate)
            && format!("{error}")
                == "invalid discriminant '99' for enum 'ArchivedAllocStatusRowEnvelope'"
        {
            println!("GOLDEN_DISCRIMINANT_OFFSET_V1 = {offset}");
        }
    }
}
