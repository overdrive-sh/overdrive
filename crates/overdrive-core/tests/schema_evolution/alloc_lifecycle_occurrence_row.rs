//! Schema-evolution golden-bytes test for the V1 allocation lifecycle
//! occurrence envelope introduced by the recovery amendment R1.

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRowEnvelope, AllocLifecycleOccurrenceRowV1, AllocLifecyclePredecessor,
    AllocState, LogicalTimestamp, TransitionSource,
};
use overdrive_core::transition_reason::TransitionReason;

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 160;
const FIXTURE_V1: &str = "616c6c6f632d6c6966656379636c652d303163616e6f6e6963616c206f6363757272656e63650000000000000000000092000000d0ffffff7061796d656e74730101000005000000010000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000010000009400000096ffffff010100002a000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

fn canonical_v1_payload() -> AllocLifecycleOccurrenceRowV1 {
    AllocLifecycleOccurrenceRowV1 {
        alloc_id: AllocationId::new("alloc-lifecycle-01").expect("valid allocation id"),
        workload_id: WorkloadId::new("payments").expect("valid workload id"),
        from: AllocLifecyclePredecessor::State(AllocState::Running),
        to: AllocState::Failed,
        reason: Some(TransitionReason::Started),
        detail: Some("canonical occurrence".to_owned()),
        source: TransitionSource::Driver(DriverType::Vm),
        at: LogicalTimestamp {
            counter: 42,
            writer: NodeId::new("node-001").expect("valid writer"),
        },
        terminal: None,
    }
}

#[test]
fn alloc_lifecycle_occurrence_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<AllocLifecycleOccurrenceRowEnvelope>(FIXTURE_V1, &expected);
}

#[test]
fn alloc_lifecycle_occurrence_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<AllocLifecycleOccurrenceRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

#[test]
fn alloc_lifecycle_occurrence_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<AllocLifecycleOccurrenceRowEnvelope>(
        canonical_v1_payload(),
        "AllocLifecycleOccurrenceRowEnvelope",
        0,
    );
}

#[test]
#[ignore = "fixture/offset regeneration tool"]
#[allow(clippy::print_stdout, reason = "fixture regeneration output")]
fn print_fixture_and_discriminant_offset() {
    let envelope = AllocLifecycleOccurrenceRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("archive occurrence");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
    for offset in 1..=bytes.len() {
        let mut candidate = bytes.as_ref().to_vec();
        let idx = candidate.len() - offset;
        candidate[idx] = 99;
        if let Err(error) =
            rkyv::from_bytes::<AllocLifecycleOccurrenceRowEnvelope, rkyv::rancor::Error>(&candidate)
            && format!("{error}")
                == "invalid discriminant '99' for enum \
                    'ArchivedAllocLifecycleOccurrenceRowEnvelope'"
        {
            println!("GOLDEN_DISCRIMINANT_OFFSET_V1 = {offset}");
        }
    }
}
