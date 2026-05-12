//! S-EV-01.5 — Schema-evolution roundtrip for `JobEnvelope` against
//! its V1 golden-bytes fixture.
//!
//! Per `.claude/rules/testing.md` § "Property-based testing
//! (proptest)" → "Mandatory call sites" → "Archive schema-evolution
//! roundtrip": every rkyv versioned envelope ships at least one
//! historical-version golden fixture pinning the archived bytes.
//! `FIXTURE_V1` is generated once at the GREEN landing of step 01-04
//! and pinned verbatim from that moment onward. NEVER touched on
//! subsequent commits — touching it collapses the schema-evolution
//! signal.
//!
//! When bumping to `JobEnvelope::V2`, append a new `FIXTURE_V2`
//! constant + a new test that round-trips the V2 payload; the V1
//! constant + test stay verbatim, asserting V1 bytes continue to
//! decode through the bumped envelope via `From<JobV1> for JobV2`.

use std::num::NonZeroU32;

use overdrive_core::aggregate::{Exec, Job, JobEnvelope, JobLatest, WorkloadDriver};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::driver::Resources;

use super::harness::{assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `JobEnvelope::discriminant_offset_from_end()`. The trait
/// method and this constant must agree; a V2 bump that shifts the
/// archived layout updates BOTH in the same commit (per
/// `development.md` § Version-bump procedure). A unilateral edit to
/// either source trips the triangulation assertion in
/// `job_discriminant_offset_triangulation`.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 64;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one
/// of them and the test fails until `FIXTURE_V1` is regenerated via
/// `print_fixture_v1_bytes`.
fn canonical_v1_payload() -> JobLatest {
    Job {
        id: WorkloadId::new("svc-payments").expect("valid workload id"),
        replicas: NonZeroU32::new(3).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 250, memory_bytes: 256 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `JobEnvelope::V1(canonical_v1_payload())`. Generated once at the
/// GREEN landing of step 01-04 and pinned verbatim from that moment
/// onward. NEVER touched on subsequent commits.
const FIXTURE_V1: &str = "7376632d7061796d656e74732f62696e2f736c656570000033363030ffffffff00000000000000008c000000d8ffffff0300000000000000fa000000000000000000001000000000000000008a000000c0ffffffc4ffffff0100000000000000";

#[test]
fn job_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<JobEnvelope>(FIXTURE_V1, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `JobEnvelope` V1 discriminant offset. Asserts BOTH that the
/// trait method `JobEnvelope::discriminant_offset_from_end()` agrees
/// with the per-fixture `GOLDEN_DISCRIMINANT_OFFSET_V1` constant AND
/// that the canonical V1 archived layout places the V1 tag (0) at
/// that offset. On a `V<N+1>` bump where rkyv's archived layout
/// shifts, the developer must update BOTH the trait method's return
/// value AND `GOLDEN_DISCRIMINANT_OFFSET_V1` in the same commit; a
/// unilateral edit to either source trips this assertion.
#[test]
fn job_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<JobEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = JobEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
