//! Schema-evolution golden-bytes test — `AllocStatusRowEnvelope`.
//!
//! S-EV-01.1. Pins the V1 archived layout of the `AllocStatusRow`
//! envelope so that any future commit which appends a field to the
//! V1 payload (rather than minting a `V2`) breaks this test and
//! signals the schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2`
//! adds a new `FIXTURE_V2` constant + a new assertion in the same
//! commit; existing constants stay verbatim. See `development.md`
//! § "Version-bump procedure".

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, LogicalTimestamp,
};

use super::harness::{assert_discriminant_byte_at_pinned_offset, assert_envelope_v_roundtrip};

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one
/// of them and the test fails until `FIXTURE_V1` is regenerated.
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
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `AllocStatusRowEnvelope::V1(canonical_v1_payload())`. Generated
/// once at the GREEN landing of step 01-03 and pinned verbatim from
/// that moment onward. NEVER touched on subsequent commits.
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000000000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d30303100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005affffff00000000";

#[test]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &expected);
}

/// Structural defense for the empirically-pinned
/// `AllocStatusRowEnvelope::discriminant_offset_from_end()` value
/// (168). Archives a canonical V1 payload through `latest()` and
/// asserts the byte at the pinned offset is V1's tag (0). Catches a
/// stale offset on the next `V<N+1>` bump where rkyv's archived
/// layout shifts but the pinned offset constant wasn't updated.
#[test]
fn alloc_status_row_discriminant_byte_at_pinned_offset() {
    assert_discriminant_byte_at_pinned_offset::<AllocStatusRowEnvelope>(canonical_v1_payload(), 0);
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/print_fixture_v1_bytes/)' --no-capture
//
// Marked `#[ignore]` so it never runs in normal test execution; the
// pinned `FIXTURE_V1` constant is the load-bearing artifact, this is a
// one-shot regeneration aid. Per `.claude/rules/testing.md` §
// "RED scaffolds" #[ignore] requires a reason — the reason is "fixture
// regeneration tool; not a runtime assertion".
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = AllocStatusRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
