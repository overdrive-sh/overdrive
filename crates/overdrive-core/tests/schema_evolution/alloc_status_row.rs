//! Golden-bytes coverage for the current allocation-status V1 envelope.
//!
//! Current-layout goldens pin the V1 envelope with no terminal claim, a
//! Stable witness in both status carriers, and each StoppedBy initiator.
//! Historical `FIXTURE_V*` bytes stay unchanged and are not backward-read
//! promises under the greenfield waiver.

#![allow(
    clippy::doc_markdown,
    reason = "required Contract Shape declarations use the exact linted spelling"
)]

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, LastTerminated,
    LogicalTimestamp,
};
use overdrive_core::transition_reason::{
    ProbeWitness, StoppedBy, TerminalCondition, TransitionReason,
};

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

#[allow(
    dead_code,
    reason = "the greenfield waiver preserves pre-D16 bytes without a backward-read promise"
)]
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031010000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

const CURRENT_LAYOUT_ALLOC_STATUS_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031010000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const CURRENT_LAYOUT_ALLOC_STATUS_V1_STABLE: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e2073746174757374637020302e302e302e303a38303830626f756e64656420564d4d207374646572720074637020302e302e302e303a3830383000000000000000008d00000090ffffff8c00000095ffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031010000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000045ffffff0000000001000000000000000500000000000000d2040000000000000000000073746172747570ff9000000031ffffff00000000010000009200000031ffffff0000000038ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000010000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000500000000000000d2040000000000000000000073746172747570ff900000009cfeffff000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000006e6f64652d3030310000000000000000";

fn stable_terminal() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 1_234,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "tcp 0.0.0.0:8080".to_owned(),
        },
    }
}

fn payload_with_stable_witness() -> AllocStatusRowLatest {
    let mut row = canonical_v1_payload();
    row.terminal = Some(stable_terminal());
    row.last_terminated = Some(LastTerminated {
        state: AllocState::Terminated,
        reason: None,
        detail: None,
        terminal: Some(stable_terminal()),
        stderr_tail: None,
        started_at: None,
        terminated_at: LogicalTimestamp {
            counter: 2,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
    });
    row
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(
        CURRENT_LAYOUT_ALLOC_STATUS_V1,
        &canonical_v1_payload(),
    );
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(
        CURRENT_LAYOUT_ALLOC_STATUS_V1_STABLE,
        &payload_with_stable_witness(),
    );
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
    println!("CURRENT_LAYOUT_ALLOC_STATUS_V1 = \"{}\"", hex::encode(bytes.as_ref()));
    let stable = AllocStatusRowEnvelope::latest(payload_with_stable_witness());
    let stable_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&stable)
        .expect("archive allocation status with Stable witness");
    println!("CURRENT_LAYOUT_ALLOC_STATUS_V1_STABLE = \"{}\"", hex::encode(stable_bytes.as_ref()));
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

// ---------------------------------------------------------------------------
// `StoppedBy` initiators in the archived `terminal` (feature
// `service-allocation-readiness-and-ending`, D7 and F-4; S-25). `StoppedBy` is a
// fieldless, tail-appended enum (`LivenessProbe` is discriminant 5, `StartupProbe`
// 6), so neither initiator mints an envelope version. The historical fixtures
// below remain unchanged; current-layout golden values are separate.
// ---------------------------------------------------------------------------

/// The canonical payload with a `Stopped { by }` terminal and the matching reason.
fn payload_stopped_by(by: StoppedBy) -> AllocStatusRowLatest {
    AllocStatusRowV1 {
        state: AllocState::Terminated,
        reason: Some(TransitionReason::Stopped { by }),
        terminal: Some(TerminalCondition::Stopped { by }),
        ..canonical_v1_payload()
    }
}

/// Historical pre-D16 LivenessProbe archive bytes, preserved verbatim.
#[allow(
    dead_code,
    reason = "the greenfield waiver preserves pre-D16 bytes without a backward-read promise"
)]
const FIXTURE_V1_STOPPED_BY_LIVENESS_PROBE: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031040000000000000001000000000000006e6f64652d303031010000000000000004050000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000010000000000000001050000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// Historical pre-D16 StartupProbe archive bytes, preserved verbatim.
#[allow(
    dead_code,
    reason = "the greenfield waiver preserves pre-D16 bytes without a backward-read promise"
)]
const FIXTURE_V1_STOPPED_BY_STARTUP_PROBE: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031040000000000000001000000000000006e6f64652d303031010000000000000004060000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000010000000000000001060000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

const CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_LIVENESS_PROBE: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031040000000000000001000000000000006e6f64652d303031010000000000000004050000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000010000000000000001050000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_STARTUP_PROBE: &str = "616c6c6f632d746573742d30317376632d7061796d656e747363757272656e7420563120616c6c6f636174696f6e20737461747573626f756e64656420564d4d207374646572720000000000000000008d000000b0ffffff8c000000b5ffffff6e6f64652d303031040000000000000001000000000000006e6f64652d303031010000000000000004060000000000000000000000000000000000000000000000000000000000000000000000000000010000009c00000065ffffff00000000010000000000000001060000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000041ffffff0000000048ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// CONTRACT_SHAPE: pure-function.
#[test]
fn alloc_status_row_stopped_by_liveness_probe_decodes_through_current_envelope() {
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(
        CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_LIVENESS_PROBE,
        &payload_stopped_by(StoppedBy::LivenessProbe),
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn alloc_status_row_stopped_by_startup_probe_decodes_through_current_envelope() {
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(
        CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_STARTUP_PROBE,
        &payload_stopped_by(StoppedBy::StartupProbe),
    );
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when the archived StoppedBy layout changes"]
#[allow(clippy::print_stdout, reason = "fixture regeneration output")]
fn print_stopped_by_fixtures() {
    for (name, by) in [
        ("CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_LIVENESS_PROBE", StoppedBy::LivenessProbe),
        ("CURRENT_LAYOUT_ALLOC_STATUS_V1_STOPPED_BY_STARTUP_PROBE", StoppedBy::StartupProbe),
    ] {
        let envelope = AllocStatusRowEnvelope::latest(payload_stopped_by(by));
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("archive allocation status");
        println!("{name} = \"{}\"", hex::encode(bytes.as_ref()));
    }
}
