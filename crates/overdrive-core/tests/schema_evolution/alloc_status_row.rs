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

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::{VersionedEnvelope, decode_envelope_bytes};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, AllocStatusRowV2,
    LogicalTimestamp,
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
///
/// **Re-pinned 2026-06-22 — 212 → 224 — V2 append
/// (canonical-workload-address-inbound-tproxy, GH #241).** Appending
/// `V2(AllocStatusRowV2)` with its additive `workload_addr:
/// Option<Ipv4Addr>` grows the outer enum's inline footprint to
/// `max(V1, V2)`, extending the trailing root structure by 8 bytes and
/// shifting the discriminant offset 212 → 224. EMPIRICAL: derived from
/// the actual archived bytes via the triangulation test below (which
/// archives `E::latest(canonical)` and asserts tag `1` — the V2
/// discriminant — at `len - 224`), NOT guessed. Re-pinned in lockstep
/// with `AllocStatusRowEnvelope::discriminant_offset_from_end()`.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 224;

/// Canonical V1 *inner payload* pinned by `FIXTURE_V1` below. This is
/// the historical `AllocStatusRowV1` shape — its archived bytes are
/// `FIXTURE_V1`. The function returns the concrete `V1` type (NOT the
/// re-aliased `Latest`), because the V1 golden bytes were produced from
/// exactly these field values and the `From<V1> for V2` chain consumes
/// this value to derive the expected V2 projection.
///
/// Change any one of these values and the V1 golden test fails until
/// `FIXTURE_V1` is regenerated.
fn canonical_v1_payload_inner() -> AllocStatusRowV1 {
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

/// Canonical `Latest` (= V2) projection of the V1 golden payload. The
/// V1 golden bytes decode through the envelope and the `From<V1> for
/// V2` chain to exactly this value: every pre-existing field carried
/// forward verbatim, `workload_addr` defaulted to `None`. Used by the
/// V1-golden-decode and discriminant-triangulation tests.
fn canonical_v1_payload() -> AllocStatusRowLatest {
    AllocStatusRowV2::from(canonical_v1_payload_inner())
}

/// A canonical V2 payload (`workload_addr: None`) sharing the V1
/// golden field values. The `Some(addr)` round-trip tests start from
/// this base and set `workload_addr` to the address under test.
fn canonical_v1_v2_base() -> AllocStatusRowV2 {
    AllocStatusRowV2::from(canonical_v1_payload_inner())
}

/// Hex-encoded rkyv-archived bytes of the **V1 variant** under the
/// CURRENT envelope shape — `AllocStatusRowEnvelope::V1(<canonical V1
/// inner>)`.
///
/// **Regenerated 2026-06-22 — greenfield, V2 append
/// (canonical-workload-address-inbound-tproxy, GH #241).** Appending
/// `V2(AllocStatusRowV2)` — whose only delta is the additive
/// `workload_addr: Option<Ipv4Addr>` field — grows the outer enum's
/// INLINE footprint to `max(V1, V2)`. rkyv archives are fixed
/// positional layouts: the V1 variant's inline region is now padded by
/// 8 bytes (the `Option<Ipv4Addr>` footprint, aligned), so the prior
/// 248-byte V1-only archive is structurally unreadable through the new
/// 256-byte-shaped envelope (rkyv reads the discriminant at the new
/// position and rejects). The fixture is therefore regenerated to the
/// V1 *variant* archive under the V1+V2 envelope (256 bytes).
///
/// This is the same greenfield re-pin the prior two layout shifts
/// performed (168 → 192 → 212 → **224**); pre-shipment regeneration is
/// authorized by `feedback_single_cut_greenfield_migrations.md` (this
/// envelope has NO deployed consumer — Phase-1 single-node;
/// "delete the on-disk redb file" is the upgrade path). The fixture
/// still pins what it must: that a V1-shaped payload archives, decodes,
/// and projects (via `From<AllocStatusRowV1> for AllocStatusRowV2`) to
/// a V2 `Latest` with `workload_addr == None`. Once V1 ships to a
/// deployed consumer, this constant becomes immutable per
/// `.claude/rules/development.md` § "rkyv schema evolution".
///
/// Produced by running `print_fixture_v1_bytes` (which archives the V1
/// *variant* explicitly) and pasted verbatim.
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000000000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000042ffffff00000000010000000000000000f153650000000000000000000000000000000000000000";

#[test]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &expected);
}

/// Canonical V2 payload pinned by `FIXTURE_V2` below — a Path-A alloc
/// carrying `Some(workload_addr)`. The `FIXTURE_V2` golden bytes are
/// produced from exactly these field values; change any one and the
/// V2 golden test fails until `FIXTURE_V2` is regenerated.
fn canonical_v2_payload() -> AllocStatusRowV2 {
    let mut payload = canonical_v1_v2_base();
    payload.workload_addr = Some(Ipv4Addr::new(10, 99, 0, 6));
    payload
}

/// Hex-encoded rkyv-archived bytes of
/// `AllocStatusRowEnvelope::V2(canonical_v2_payload())`.
///
/// Generated in the same commit as the `AllocStatusRowEnvelope::V2`
/// bump (canonical-workload-address-inbound-tproxy, GH #241), per
/// `development.md` § "rkyv schema evolution" → "Version-bump
/// procedure" step 5. The hex was produced by running
/// `print_fixture_v2_bytes` and pasted verbatim. `FIXTURE_V1` is NOT
/// touched.
const FIXTURE_V2: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000001000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000042ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000";

/// S-V2 / golden-bytes pin for `AllocStatusRowEnvelope::V2`. Asserts
/// the pinned V2 archived bytes still deserialise through today's
/// envelope into the canonical V2 `Latest` projection (with
/// `workload_addr = Some(...)`). Co-resident with the untouched
/// `FIXTURE_V1` test per the golden-discipline rule.
#[test]
fn alloc_status_row_v2_decodes_through_current_envelope() {
    let expected = canonical_v2_payload();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V2, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `AllocStatusRowEnvelope` discriminant offset. Asserts BOTH that the
/// trait method's return value agrees with
/// `GOLDEN_DISCRIMINANT_OFFSET_V1` AND that the canonical (now V2 —
/// `Latest`) archive places the latest tag at that offset. Both pins
/// must update together on a `V<N+1>` bump.
///
/// The expected tag is `1` — the rkyv discriminant of the appended
/// `V2` variant (declaration order: `V1 = 0`, `V2 = 1`). The offset is
/// shared across V1 and V2 archives because rkyv pads the inline enum
/// to `max(V1, V2)`; the trailing root footprint is identical for both
/// variants of the same envelope.
#[test]
fn alloc_status_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        1,
    );
}

/// End-to-end pin of `AllocStatusRowEnvelope`'s introspection surface
/// (`known_discriminants`, `type_name`, `discriminant_offset_from_end`)
/// through `decode_envelope_bytes`. See
/// [`assert_unknown_version_probe_surfaces`] for the full rationale.
///
/// `supported_max == 1` because the envelope is now V1+V2 (the highest
/// known rkyv discriminant is 1, the appended `V2` variant). Re-pinned
/// in the same commit as the V2 bump per `development.md` §
/// "Version-bump procedure".
#[test]
fn alloc_status_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        "AllocStatusRowEnvelope",
        1,
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
    let payload = AllocStatusRowV2::from(AllocStatusRowV1 {
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
    });
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

    let payload = AllocStatusRowV2::from(AllocStatusRowV1 {
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
    });
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
    // The V1 golden bytes pin the historical V1 archive — they MUST be
    // produced from the V1 inner payload wrapped as the V1 envelope
    // variant, NOT from the re-aliased Latest (= V2). Construct the V1
    // envelope variant explicitly so this aid keeps regenerating the
    // immutable V1 fixture across future version bumps.
    let envelope = AllocStatusRowEnvelope::V1(canonical_v1_payload_inner());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V2"
)]
fn print_fixture_v2_bytes() {
    let envelope = AllocStatusRowEnvelope::latest(canonical_v2_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V2 = \"{}\"", hex::encode(bytes.as_ref()));
}

// ---------------------------------------------------------------------
// S-V2 — `AllocStatusRowEnvelope::V2` schema-evolution scaffolds
// (canonical-workload-address-inbound-tproxy, GH #241; DISTILL RED).
//
// D-BLOCKER2: persist `workload_addr: Option<Ipv4Addr>` directly on the row
// (the materialized `slot x base-at-provision` join the inbound nft rule was
// keyed on), via an additive `AllocStatusRowEnvelope::V2`. Mandatory per
// `.claude/rules/testing.md` § "Archive schema-evolution roundtrip" and
// `development.md` § "rkyv schema evolution" 6-step procedure.
//
// DELIVER fills these bodies and lands the bump as a SINGLE commit:
//   1. Append `V2(AllocStatusRowV2)`; re-alias `AllocStatusRow = AllocStatusRowV2`.
//   2. `AllocStatusRowLatest = AllocStatusRowV2`.
//   3. `latest(p) -> Self::V2(p)`.
//   4. `From<AllocStatusRowV1> for AllocStatusRowV2` (additive: `workload_addr:
//      None`); `into_latest()` chains `V1 => Ok(v1.into())`, `V2 => Ok(v2)`.
//   5. Add `FIXTURE_V2` (regenerated via the `print_fixture_v1_bytes`-shaped
//      aid) WITHOUT touching `FIXTURE_V1` (the existing fixture stays verbatim —
//      it is the V1-backward-compat error/edge guard: old bytes must still read).
//   6. Re-pin `GOLDEN_DISCRIMINANT_OFFSET_V1` via the triangulation test
//      (adding `Option<Ipv4Addr>` — 4 bytes behind the `Option` discriminant —
//      shifts the trailing root footprint).
//
// DELIVER obligation #5 (from `design/wave-decisions.md`): the
// `AllocStatusRowV2.workload_addr` field carries a rustdoc comment naming it a
// materialized `slot x base-at-provision` join (a frozen snapshot, immutable
// except under redeploy) + the #239 Phase-1 single-cut constraint (a base change
// is a full redeploy / re-provision / re-observe, NOT a live re-tune) — so a
// future "just recompute it at the bridge" refactor cannot silently reintroduce
// the install/advertise divergence the design rejected.
//
// Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-V2.
// ---------------------------------------------------------------------

/// S-V2 / AC1 — the pinned `FIXTURE_V1` golden bytes (an old V1
/// archive) decode through the *current* envelope + `into_latest()` to
/// an `AllocStatusRowV2` whose `workload_addr` is `None` (the additive
/// `From<AllocStatusRowV1> for AllocStatusRowV2` defaults the new field
/// absent), and every other field equals the canonical V1 payload
/// byte-for-byte.
///
/// `FIXTURE_V1` is NOT touched by this step — this test proves that
/// old V1 bytes still read through the V2 envelope (the
/// backward-compat obligation of the envelope evolution).
#[test]
fn alloc_status_row_v1_golden_bytes_decode_to_v2_with_absent_workload_addr() {
    let expected_v2 = AllocStatusRowV2::from(canonical_v1_payload_inner());
    assert_eq!(
        expected_v2.workload_addr, None,
        "From<V1> for V2 must default the additive workload_addr field to None",
    );
    // The V1 golden bytes project (via decode -> into_latest -> the
    // From<V1> chain) to the V2 Latest shape, with workload_addr None
    // and every pre-existing field carried forward unchanged.
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &expected_v2);

    // The V1 (tag 0) archive must ALSO pass the
    // `known_discriminants()`-driven probe inside `decode_envelope_bytes`
    // — i.e. tag 0 stays in the known set after the V2 append, so a
    // legacy V1 row is decoded (not flagged `UnknownVersion` and
    // silently skipped on convergence). This kills a mutant that drops
    // tag 0 from `known_discriminants` (`&[1]` instead of `&[0, 1]`);
    // the V2-only `unknown_version_probe` test cannot catch it because a
    // `&[1]` set still recognises the V2 tag it round-trips.
    let v1_decoded = decode_envelope_bytes::<AllocStatusRowEnvelope>(
        &hex::decode(FIXTURE_V1.trim()).expect("FIXTURE_V1 hex decodes"),
    )
    .expect("V1 (tag 0) archive must be a KNOWN discriminant — not flagged UnknownVersion");
    assert_eq!(
        v1_decoded, expected_v2,
        "decode_envelope_bytes must project the V1 archive to the same V2 Latest as the \
         from_bytes path",
    );
}

/// S-V2 / AC2 — an `AllocStatusRowV2` carrying `Some(workload_addr)`
/// round-trips archive -> access -> deserialize -> `into_latest()`
/// equal to the original. Property-based over an arbitrary `Ipv4Addr`
/// (the V1 arm stays example-pinned via the golden `FIXTURE_V1`; the
/// V2 `Some(addr)` arm is the property arm per the step's `RED_UNIT`
/// guidance).
#[test]
fn alloc_status_row_v2_with_workload_addr_round_trips_archive_access_deserialize() {
    // Hand-picked representative addresses spanning the octet space —
    // a full proptest generator is overkill for a structural rkyv
    // round-trip whose correctness does not depend on the IP value.
    for octets in [[10, 99, 0, 2], [192, 168, 1, 254], [0, 0, 0, 0], [255, 255, 255, 255]] {
        let addr = Ipv4Addr::from(octets);
        let mut payload = canonical_v1_v2_base();
        payload.workload_addr = Some(addr);

        let envelope = AllocStatusRowEnvelope::latest(payload.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
        let decoded: AllocStatusRowEnvelope =
            rkyv::from_bytes::<AllocStatusRowEnvelope, rkyv::rancor::Error>(bytes.as_ref())
                .expect("rkyv deserialize");
        let projected: AllocStatusRowLatest =
            decoded.into_latest().expect("envelope into_latest projection");
        assert_eq!(
            projected, payload,
            "AllocStatusRowV2 with Some(workload_addr) must round-trip archive -> access -> \
             deserialize -> into_latest equal to the original (addr {addr})",
        );
    }
}

/// S-V2 / AC2 — `into_latest()` projects a `V2` variant verbatim
/// (`V2 => Ok(v2)`). Kills a mutant that swaps the V2 arm for the V1
/// `From` chain.
#[test]
fn alloc_status_row_v2_into_latest_projects_verbatim() {
    let mut payload = canonical_v1_v2_base();
    payload.workload_addr = Some(Ipv4Addr::new(10, 99, 0, 6));
    let envelope = AllocStatusRowEnvelope::latest(payload.clone());
    let projected = envelope.into_latest().expect("into_latest V2 arm");
    assert_eq!(
        projected, payload,
        "into_latest() must project a V2 envelope to its payload verbatim, preserving workload_addr",
    );
}
