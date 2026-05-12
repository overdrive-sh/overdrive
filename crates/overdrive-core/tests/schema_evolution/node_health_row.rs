//! Schema-evolution golden-bytes test — `NodeHealthRowEnvelope`.
//!
//! S-EV-01.2. Pins the V1 archived layout of the `NodeHealthRow`
//! envelope so that any future commit which appends a field to the
//! V1 payload (rather than minting a `V2`) breaks this test and
//! signals the schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2`
//! adds a new `FIXTURE_V2` constant + a new assertion in the same
//! commit; existing constants stay verbatim. See `development.md`
//! § "Version-bump procedure".

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{NodeId, Region};
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, NodeHealthRowEnvelope, NodeHealthRowLatest, NodeHealthRowV1,
};

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `NodeHealthRowEnvelope::discriminant_offset_from_end()`.
/// See `job.rs::GOLDEN_DISCRIMINANT_OFFSET_V1` for the full
/// rationale.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 40;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one
/// of them and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> NodeHealthRowLatest {
    NodeHealthRowV1 {
        node_id: NodeId::new("node-001").expect("valid node id"),
        region: Region::new("us-east-1").expect("valid region"),
        last_heartbeat: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `NodeHealthRowEnvelope::V1(canonical_v1_payload())`. Generated
/// once at the GREEN landing of step 02-01 and pinned verbatim from
/// that moment onward. NEVER touched on subsequent commits.
const FIXTURE_V1: &str = "75732d656173742d310000000000000000000000000000006e6f64652d30303189000000e0ffffff01000000000000006e6f64652d303031";

#[test]
fn node_health_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<NodeHealthRowEnvelope>(FIXTURE_V1, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `NodeHealthRowEnvelope` V1 discriminant offset. Both the trait
/// method and `GOLDEN_DISCRIMINANT_OFFSET_V1` must agree, and the
/// canonical archive must place the V1 tag (0) at that offset.
#[test]
fn node_health_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<NodeHealthRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

/// End-to-end pin of `NodeHealthRowEnvelope`'s introspection surface
/// (`known_discriminants`, `type_name`, `discriminant_offset_from_end`)
/// through `decode_envelope_bytes`. See
/// [`assert_unknown_version_probe_surfaces`] for the full rationale.
///
/// `supported_max == 0` because today's envelope is V1-only; bumping
/// to V2 means re-pinning this assertion in the same commit per
/// `development.md` § "Version-bump procedure".
#[test]
fn node_health_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<NodeHealthRowEnvelope>(
        canonical_v1_payload(),
        "NodeHealthRowEnvelope",
        0,
    );
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
// one-shot regeneration aid.
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = NodeHealthRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
