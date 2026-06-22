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

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, LogicalTimestamp,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `AllocStatusRowEnvelope::discriminant_offset_from_end()`.
/// See `job.rs::GOLDEN_DISCRIMINANT_OFFSET_V1` for the full rationale
/// (two-source triangulation guards against unilateral drift of
/// either pin).
///
/// Re-pinned 2026-05-24 from 168 → 192 — greenfield, no shipped
/// consumers; layout shifted by `TerminalCondition::{Stable,
/// ServiceFailed}` variant append per user directive (see
/// `feedback_single_cut_greenfield_migrations.md` — pre-shipment the
/// V1 fixture is the canonical spec, regenerated when the spec
/// changes).
///
/// Re-pinned 2026-05-29 — greenfield retype of the GAP-1 subsidiary
/// field from `started_at_unix_ms: Option<u64>` to
/// `started_at: Option<UnixInstant>` (corrective patch closing the
/// newtype-discipline violation in commit 6f2b2cb9). `UnixInstant`
/// wraps `Duration` (12-byte inline layout: 8 bytes for seconds + 4
/// bytes for nanos), so the inlined `Option<UnixInstant>` payload
/// grows relative to the prior `Option<u64>` (8 bytes), shifting the
/// outer enum's discriminant byte from 208 to its new empirical
/// position. The new value is determined by regenerating `FIXTURE_V1`
/// and observing where `0x00` lives in the trailing root structure.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 212;

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
        // Subsidiary GAP-1 fix: canonical payload carries the
        // wall-clock at the Pending → Running transition. Pinned
        // value is arbitrary but stable — re-pin on every
        // FIXTURE_V<N+1> bump.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `AllocStatusRowEnvelope::V1(canonical_v1_payload())`.
///
/// Regenerated 2026-05-29 — greenfield retype of GAP-1 subsidiary
/// field from `Option<u64>` to `Option<UnixInstant>` (the corrective
/// patch closing the newtype-discipline violation in commit
/// 6f2b2cb9). The inlined `Option<UnixInstant>` payload uses a
/// `Duration` (12 bytes — `u64` seconds + `u32` nanos) where the
/// prior `Option<u64>` used 8 bytes, growing the trailing root
/// structure by 4 bytes per `Some`. The new hex was produced by
/// running `print_fixture_v1_bytes` and pasted verbatim.
///
/// Pre-shipment regeneration is allowed under
/// `feedback_single_cut_greenfield_migrations.md`. Once V1 has
/// shipped to a deployed consumer, this constant becomes immutable
/// per `.claude/rules/development.md` § "rkyv schema evolution" —
/// future variants would need a `V2` envelope.
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000000000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000042ffffff00000000010000000000000000f15365000000000000000000000000";

#[test]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `AllocStatusRowEnvelope` V1 discriminant offset. Asserts BOTH
/// that the trait method's return value agrees with
/// `GOLDEN_DISCRIMINANT_OFFSET_V1` AND that the canonical archive
/// places the V1 tag (0) at that offset. Both pins must update
/// together on a `V<N+1>` bump.
#[test]
fn alloc_status_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

/// End-to-end pin of `AllocStatusRowEnvelope`'s introspection surface
/// (`known_discriminants`, `type_name`, `discriminant_offset_from_end`)
/// through `decode_envelope_bytes`. See
/// [`assert_unknown_version_probe_surfaces`] for the full rationale.
///
/// `supported_max == 0` because today's envelope is V1-only; bumping
/// to V2 means re-pinning this assertion in the same commit per
/// `development.md` § "Version-bump procedure".
#[test]
fn alloc_status_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        "AllocStatusRowEnvelope",
        0,
    );
}

/// Forward-roundtrip pin for the `StoppedBy::SystemGc` variant
/// (ADR-0037 Amendment 2026-05-14, step 01-01 of
/// `workload-gc-absent-stale-allocs`). Constructs a fresh
/// `AllocStatusRow` carrying
/// `terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })`,
/// archives through the *current* `AllocStatusRowEnvelope` (V1 — the
/// rkyv layout is unchanged because the new variant is appended at
/// the tail of `StoppedBy`'s discriminant space), deserialises, and
/// asserts `Eq` against the source.
///
/// This is NOT a `FIXTURE_V<N>` constant — appending an enum variant
/// does not bump the envelope version per
/// `.claude/rules/development.md` § "rkyv schema evolution"; the
/// existing `FIXTURE_V1` test continues to defend the discriminant
/// layout of pre-existing variants. This test pins that the new
/// variant encodes/decodes through the same envelope.
///
/// Mutation-killability: a mutant swapping `SystemGc` for `Process`
/// in the constructor below fails the equality assertion.
#[test]
fn fresh_alloc_status_row_stopped_by_system_gc_round_trips_through_v1_envelope() {
    let payload = AllocStatusRowV1 {
        alloc_id: AllocationId::new("alloc-gc-01").expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp {
            counter: 7,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: None,
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        // Subsidiary GAP-1 fix: this test exercises a Terminated row
        // (SystemGc), which by lifecycle ordering must have reached
        // Running first — the field is `Some(_)` to reflect that.
        // Value is arbitrary; the test asserts round-trip equality.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    let envelope = AllocStatusRowEnvelope::latest(payload.clone());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    let decoded: AllocStatusRowEnvelope =
        rkyv::from_bytes::<AllocStatusRowEnvelope, rkyv::rancor::Error>(bytes.as_ref())
            .expect("rkyv deserialize");
    let projected: AllocStatusRowLatest =
        decoded.into_latest().expect("envelope into_latest projection");
    assert_eq!(
        projected, payload,
        "AllocStatusRow with StoppedBy::SystemGc must round-trip through the current V1 envelope unchanged"
    );
}

/// Forward-roundtrip pin for the `TransitionReason::WorkloadNetnsProvisionFailed`
/// variant (transparent-mtls-enrollment D-TME-12 / AC14, step 04-01).
/// Constructs a `Failed` `AllocStatusRow` whose `reason` carries the new
/// cause-class variant, archives through the *current* `AllocStatusRowEnvelope`
/// (V1 — the rkyv layout is unchanged because the new variant is appended at the
/// tail of `TransitionReason`'s discriminant space), deserialises, and asserts
/// `Eq` against the source.
///
/// This is NOT a `FIXTURE_V<N>` constant — appending an enum variant does not
/// bump the envelope version per `.claude/rules/development.md` § "rkyv schema
/// evolution"; the existing `FIXTURE_V1` test (which pins `reason: None`)
/// continues to defend the discriminant layout of pre-existing variants. This
/// test pins that the new variant encodes/decodes through the same envelope.
///
/// Mutation-killability: a mutant swapping the `stage`/`detail` strings in the
/// constructor below fails the equality assertion.
#[test]
fn fresh_alloc_status_row_workload_netns_provision_failed_round_trips_through_v1_envelope() {
    use overdrive_core::transition_reason::TransitionReason;

    let payload = AllocStatusRowV1 {
        alloc_id: AllocationId::new("alloc-netns-fail-01").expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp {
            counter: 3,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: Some(TransitionReason::WorkloadNetnsProvisionFailed {
            stage: "net_slot_assign".to_owned(),
            detail: "no free network slot (capacity 4096 exhausted)".to_owned(),
        }),
        detail: Some("no free network slot (capacity 4096 exhausted)".to_owned()),
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        // The provision seam fires PRE-Running, so a Failed row from this cause
        // never reached Running — `started_at` is `None`.
        started_at: None,
    };
    let envelope = AllocStatusRowEnvelope::latest(payload.clone());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    let decoded: AllocStatusRowEnvelope =
        rkyv::from_bytes::<AllocStatusRowEnvelope, rkyv::rancor::Error>(bytes.as_ref())
            .expect("rkyv deserialize");
    let projected: AllocStatusRowLatest =
        decoded.into_latest().expect("envelope into_latest projection");
    assert_eq!(
        projected, payload,
        "AllocStatusRow with WorkloadNetnsProvisionFailed must round-trip through the current V1 envelope unchanged"
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
