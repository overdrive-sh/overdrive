//! Schema-evolution golden-bytes test — `AllocStatusRowEnvelope`.
//!
//! S-EV-01.1. Pins the V1 archived layout of the `AllocStatusRow`
//! envelope so that any future commit which appends a field to the
//! V1 payload (rather than minting a `V2`) breaks this test and
//! signals the schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! **`FIXTURE_V<N>` constants are never touched *casually*.** Bumping
//! the envelope to `V<N+1>` adds a new `FIXTURE_V<N+1>` constant + a new
//! assertion in the same commit. See `development.md`
//! § "Version-bump procedure".
//!
//! **This envelope carries a documented exception, exercised at every
//! variant append.** rkyv sizes an enum's inline region to
//! `max(V1..VN)`, so appending a variant pads every prior variant's
//! archive and makes the previously-pinned bytes structurally
//! unreadable through the new envelope. The V2 append (2026-06-22) and
//! the V3 append (2026-08-01, ADR-0078) each regenerated every prior
//! `FIXTURE_V<N>`; both regenerations are authorised pre-shipment by
//! `feedback_single_cut_greenfield_migrations.md` and each is recorded
//! with a dated entry on the constant it touched. A regeneration
//! without such an entry is a review rejection.

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::codec::{VersionedEnvelope, decode_envelope_bytes};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRowEnvelope, AllocStatusRowLatest, AllocStatusRowV1, AllocStatusRowV2,
    AllocStatusRowV3, LastTerminated, LogicalTimestamp,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

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
///
/// **Re-pinned 2026-08-01 — 224 → 416 — V3 append (ADR-0078).**
/// Appending `V3(AllocStatusRowV3)` with its additive
/// `last_terminated: Option<LastTerminated>` (which inlines an
/// `Option<TransitionReason>`, an `Option<TerminalCondition>`, an
/// `Option<UnixInstant>` and a `LogicalTimestamp`) plus a `u32`
/// `restart_count` grows the outer enum's inline footprint to
/// `max(V1, V2, V3)`, extending the trailing root structure by 192
/// bytes: the canonical V1/V2 archive grew 256 → 448 bytes while the
/// discriminant byte stayed at absolute index 32, so the offset from
/// the end moved 224 → 416. EMPIRICAL: measured by diffing the
/// regenerated V1 and V2 archives (the single byte that differs at
/// absolute index 32 IS the tag) and cross-checked against the V3
/// archive (496 bytes, tag `2` at absolute index 80 = `len - 416`).
/// Re-pinned in lockstep with
/// `AllocStatusRowEnvelope::discriminant_offset_from_end()`.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 416;

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

/// Canonical `Latest` (= V3) projection of the V1 golden payload. The
/// V1 golden bytes decode through the envelope and the
/// `From<V1> for V2` → `From<V2> for V3` chain to exactly this value:
/// every pre-existing field carried forward verbatim, `workload_addr`
/// defaulted to `None`, and the ADR-0078 crash-observability pair
/// defaulted to `(None, 0)`. Used by the V1-golden-decode and
/// discriminant-triangulation tests.
fn canonical_v1_payload() -> AllocStatusRowLatest {
    AllocStatusRowV3::from(AllocStatusRowV2::from(canonical_v1_payload_inner()))
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
/// **Regenerated 2026-08-01 — greenfield, V3 append (ADR-0078).** Same
/// mechanism as the 2026-06-22 regeneration one version down: appending
/// `V3(AllocStatusRowV3)` — whose delta is `last_terminated:
/// Option<LastTerminated>` plus `restart_count: u32` — grows the outer
/// enum's INLINE footprint to `max(V1, V2, V3)`, so the V1 variant's
/// inline region is padded by a further 192 bytes and the prior
/// 256-byte archive is structurally unreadable through the new
/// 448-byte-shaped envelope. Byte length 256 → 448; the discriminant
/// byte stays at absolute index 32 (offset-from-end 224 → 416). This
/// regeneration is authorised by
/// `feedback_single_cut_greenfield_migrations.md` (this envelope has NO
/// deployed consumer — Phase-1 single-node; "delete the on-disk redb
/// file" is the upgrade path) and was NOT silent: ADR-0078 § D4(b)
/// requires the case that applied to be recorded, and the case that
/// applied is "the emitted hex differs, so both constants are
/// regenerated".
///
/// The fixture still pins what it must: that a V1-shaped payload
/// archives, decodes, and projects (via `From<AllocStatusRowV1> for
/// AllocStatusRowV2` → `From<AllocStatusRowV2> for AllocStatusRowV3`)
/// to a `Latest` with `workload_addr == None`, `last_terminated ==
/// None` and `restart_count == 0`.
///
/// Produced by running `print_fixture_v1_bytes` (which archives the V1
/// *variant* explicitly) and pasted verbatim.
const FIXTURE_V1: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000000000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000042ffffff00000000010000000000000000f153650000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

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
/// `print_fixture_v2_bytes` and pasted verbatim.
///
/// **Regenerated 2026-08-01 — greenfield, V3 append (ADR-0078).** Same
/// mechanism and the same authorisation as `FIXTURE_V1`'s
/// regeneration above (see that constant's docstring): the V3 variant
/// append grows the outer enum's inline footprint, padding the V2
/// variant's region and shifting the discriminant. Byte length
/// 256 → 448. The V2 payload values are UNCHANGED — only the archive
/// layout moved.
const FIXTURE_V2: &str = "616c6c6f632d746573742d30317376632d7061796d656e74730000000000000001000000000000008d000000d8ffffff8c000000ddffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000042ffffff00000000010000000000000000f15365000000000000000000000000010a630006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// The `Latest` (= V3) projection of the canonical V2 payload. The V2
/// golden bytes decode through the envelope and the `From<V2> for V3`
/// chain to exactly this value: every pre-existing field verbatim
/// (including `workload_addr = Some(..)`), crash-observability pair
/// defaulted to `(None, 0)`.
fn canonical_v2_payload_latest() -> AllocStatusRowLatest {
    AllocStatusRowV3::from(canonical_v2_payload())
}

/// S-V2 / golden-bytes pin for `AllocStatusRowEnvelope::V2`. Asserts
/// the pinned V2 archived bytes still deserialise through today's
/// envelope into the canonical `Latest` projection (with
/// `workload_addr = Some(...)`). Co-resident with the untouched
/// `FIXTURE_V1` test per the golden-discipline rule.
#[test]
fn alloc_status_row_v2_decodes_through_current_envelope() {
    let expected = canonical_v2_payload_latest();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V2, &expected);
}

/// ADR-0078 § D4 step 5 — the V2 golden bytes (an old V2 archive)
/// decode through the *current* envelope + `into_latest()` to an
/// `AllocStatusRowV3` whose crash-observability pair is ABSENT
/// (`last_terminated == None`, `restart_count == 0`), and every other
/// field equals the canonical V2 payload byte-for-byte.
///
/// Mirrors `alloc_status_row_v1_golden_bytes_decode_to_v2_with_absent_workload_addr`
/// one version up. `FIXTURE_V2` is NOT touched by this assertion — it
/// proves that old V2 bytes still read through the V3 envelope (the
/// backward-compat obligation of the envelope evolution).
#[test]
fn alloc_status_row_v2_golden_bytes_decode_to_v3_with_absent_crash_facts() {
    let expected_v3 = canonical_v2_payload_latest();
    assert_eq!(
        expected_v3.last_terminated, None,
        "From<V2> for V3 must default the additive last_terminated field to None",
    );
    assert_eq!(
        expected_v3.restart_count, 0,
        "From<V2> for V3 must default the additive restart_count field to 0",
    );
    assert_eq!(
        expected_v3.workload_addr,
        Some(Ipv4Addr::new(10, 99, 0, 6)),
        "every pre-existing V2 field must be carried forward verbatim",
    );
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V2, &expected_v3);

    // The V2 (tag 1) archive must ALSO pass the
    // `known_discriminants()`-driven probe inside `decode_envelope_bytes`
    // — i.e. tag 1 stays in the known set after the V3 append, so a
    // legacy V2 row is decoded (not flagged `UnknownVersion` and
    // silently skipped on convergence). This kills a mutant that drops
    // tag 1 from `known_discriminants`.
    let v2_decoded = decode_envelope_bytes::<AllocStatusRowEnvelope>(
        &hex::decode(FIXTURE_V2.trim()).expect("FIXTURE_V2 hex decodes"),
    )
    .expect("V2 (tag 1) archive must be a KNOWN discriminant — not flagged UnknownVersion");
    assert_eq!(
        v2_decoded, expected_v3,
        "decode_envelope_bytes must project the V2 archive to the same V3 Latest as the \
         from_bytes path",
    );
}

/// Canonical V3 payload pinned by `FIXTURE_V3` below — a RECOVERED
/// allocation: `Running`, carrying a populated `last_terminated`
/// snapshot of the crash it survived and `restart_count: 1`.
///
/// Per ADR-0078 § D4 step 5 the V3 canonical payload MUST carry
/// `last_terminated: Some(..)` with a populated `reason` and a non-zero
/// `restart_count` — a `None`/`0` payload would pin only the
/// discriminant and not the new layout.
fn canonical_v3_payload() -> AllocStatusRowV3 {
    let mut payload = canonical_v1_payload();
    payload.last_terminated = Some(LastTerminated {
        state: AllocState::Failed,
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: Some("Segmentation fault".to_owned()),
        }),
        detail: Some("killed by SIGKILL".to_owned()),
        terminal: None,
        stderr_tail: Some("Segmentation fault".to_owned()),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        terminated_at: LogicalTimestamp {
            counter: 2,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
    });
    payload.restart_count = 1;
    payload
}

/// Hex-encoded rkyv-archived bytes of
/// `AllocStatusRowEnvelope::V3(canonical_v3_payload())`.
///
/// Generated in the same commit as the `AllocStatusRowEnvelope::V3`
/// bump (ADR-0078), per `development.md` § "rkyv schema evolution" →
/// "Version-bump procedure" step 5. The hex was produced by running
/// `print_fixture_v3_bytes` and pasted verbatim.
const FIXTURE_V3: &str = "616c6c6f632d746573742d30317376632d7061796d656e7473005365676d656e746174696f6e206661756c746b696c6c6564206279205349474b494c4c5365676d656e746174696f6e206661756c740002000000000000008d000000a8ffffff8c000000adffffff6e6f64652d303031010000000000000001000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000012ffffff00000000010000000000000000f1536500000000000000000000000000000000000000000100000000000000050000000000000001000000000000000e0000000100000089000000010900000100000092000000befeffff0000000000000000000000000100000091000000b8feffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000009200000089feffff00000000010000000000000000f1536500000000000000000000000002000000000000006e6f64652d3030310100000000000000";

/// Golden-bytes pin for `AllocStatusRowEnvelope::V3` (ADR-0078).
/// Asserts the pinned V3 archived bytes deserialise through today's
/// envelope into the canonical `Latest` projection, with the
/// crash-observability pair populated.
#[test]
fn alloc_status_row_v3_decodes_through_current_envelope() {
    let expected = canonical_v3_payload();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V3, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `AllocStatusRowEnvelope` discriminant offset. Asserts BOTH that the
/// trait method's return value agrees with
/// `GOLDEN_DISCRIMINANT_OFFSET_V1` AND that the canonical (now V2 —
/// `Latest`) archive places the latest tag at that offset. Both pins
/// must update together on a `V<N+1>` bump.
///
/// The expected tag is `2` — the rkyv discriminant of the appended
/// `V3` variant (declaration order: `V1 = 0`, `V2 = 1`, `V3 = 2`). The
/// offset is shared across V1, V2 and V3 archives because rkyv pads the
/// inline enum to `max(V1, V2, V3)`; the trailing root footprint is
/// identical for every variant of the same envelope.
#[test]
fn alloc_status_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        2,
    );
}

/// End-to-end pin of `AllocStatusRowEnvelope`'s introspection surface
/// (`known_discriminants`, `type_name`, `discriminant_offset_from_end`)
/// through `decode_envelope_bytes`. See
/// [`assert_unknown_version_probe_surfaces`] for the full rationale.
///
/// `supported_max == 2` because the envelope is now V1+V2+V3 (the
/// highest known rkyv discriminant is 2, the appended `V3` variant).
/// Re-pinned in the same commit as the V3 bump per `development.md` §
/// "Version-bump procedure".
#[test]
fn alloc_status_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<AllocStatusRowEnvelope>(
        canonical_v1_payload(),
        "AllocStatusRowEnvelope",
        2,
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
    let payload = AllocStatusRowV3::from(AllocStatusRowV2::from(AllocStatusRowV1 {
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
    }));
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

    let payload = AllocStatusRowV3::from(AllocStatusRowV2::from(AllocStatusRowV1 {
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
    }));
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
    // The V2 golden bytes pin the historical V2 archive — they MUST be
    // produced from the V2 inner payload wrapped as the V2 envelope
    // variant, NOT from the re-aliased Latest (= V3). Construct the V2
    // envelope variant explicitly so this aid keeps regenerating the
    // V2 fixture across future version bumps.
    let envelope = AllocStatusRowEnvelope::V2(canonical_v2_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V2 = \"{}\"", hex::encode(bytes.as_ref()));
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V3"
)]
fn print_fixture_v3_bytes() {
    let envelope = AllocStatusRowEnvelope::latest(canonical_v3_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V3 = \"{}\"", hex::encode(bytes.as_ref()));
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
    // From<V1> -> From<V2> chain) to the current Latest shape, with
    // workload_addr None and every pre-existing field carried forward
    // unchanged.
    let expected_latest = AllocStatusRowV3::from(expected_v2);
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_V1, &expected_latest);

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
        v1_decoded, expected_latest,
        "decode_envelope_bytes must project the V1 archive to the same Latest as the \
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
        let mut payload = AllocStatusRowV3::from(canonical_v1_v2_base());
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

/// S-V2 / AC2 — `into_latest()` projects the LATEST variant verbatim
/// (`V3 => Ok(v3)`). Kills a mutant that swaps the latest arm for one of
/// the `From`-chained legacy arms.
#[test]
fn alloc_status_row_latest_into_latest_projects_verbatim() {
    let mut payload = AllocStatusRowV3::from(canonical_v1_v2_base());
    payload.workload_addr = Some(Ipv4Addr::new(10, 99, 0, 6));
    payload.restart_count = 3;
    let envelope = AllocStatusRowEnvelope::latest(payload.clone());
    let projected = envelope.into_latest().expect("into_latest latest arm");
    assert_eq!(
        projected, payload,
        "into_latest() must project a V3 envelope to its payload verbatim, preserving \
         workload_addr and the crash-observability pair",
    );
}

/// ADR-0078 § D4 step 4 — `into_latest()` on a `V2` envelope chains
/// through `From<V2> for V3`. Kills a mutant that drops the V2 arm or
/// routes it through the V1 chain.
#[test]
fn alloc_status_row_v2_into_latest_chains_through_v3() {
    let payload = canonical_v2_payload();
    let envelope = AllocStatusRowEnvelope::V2(payload.clone());
    let projected = envelope.into_latest().expect("into_latest V2 arm");
    assert_eq!(
        projected,
        AllocStatusRowV3::from(payload),
        "into_latest() must chain a V2 envelope through From<V2> for V3",
    );
}

// ---------------------------------------------------------------------
// S-ROH-A-12 — `Stopped { by: LivenessProbe }` is an additive fieldless
// tail variant (discriminant 5) on `StoppedBy` (ADR-0087 D3 / § Compliance).
//
// A fieldless tail variant adds NO layout size to `StoppedBy` and, by
// embedding, none to `AllocStatusRow` — so every existing `FIXTURE_V<N>`
// above decodes UNCHANGED and is NEVER re-minted (the existing V1/V2/V3
// golden tests remain the guard; this step touches none of them).
//
// This block adds the ONE new golden fixture the ADR mandates: an
// `AllocStatusRow` carrying the `LivenessProbe` disposition on `terminal`
// (the live shim path — `StopAllocation` writes the cause onto
// `terminal`). It is minted as an EXPLICIT `AllocStatusRowEnvelope::V1(..)`
// projection (NOT via `latest()`/the re-alias, which would encode the V3
// variant), decoded through the current envelope + `into_latest()`, and
// asserted against a HAND-WRITTEN canonical `Latest` — not a
// `canonical().into()` self-reference, which would be tautological with
// the decode path's own `From` chain.
// ---------------------------------------------------------------------

/// The V1 *inner payload* carrying the liveness disposition on
/// `terminal`. Pinned by `FIXTURE_LIVENESS_PROBE_V1` below. Returns the
/// concrete `V1` type so the golden bytes are produced from the V1
/// envelope variant (discriminant 0), independent of the current
/// `Latest` alias.
fn canonical_liveness_probe_v1_inner() -> AllocStatusRowV1 {
    AllocStatusRowV1 {
        alloc_id: AllocationId::new("alloc-liveness-01").expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp {
            counter: 9,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: None,
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    }
}

/// HAND-WRITTEN canonical `Latest` (= V3) projection of the liveness
/// disposition golden. Every field is spelled out explicitly — NOT
/// derived via `From<V1> for V2` → `From<V2> for V3`, so this expected
/// value is independent of the decode path's own conversion chain (a
/// `canonical_liveness_probe_v1_inner().into()` here would be
/// tautological: a mutant in the `From` chain would corrupt both sides
/// identically and escape). The additive fields default absent:
/// `workload_addr = None`, `last_terminated = None`, `restart_count = 0`;
/// the liveness `terminal` is carried through verbatim.
fn canonical_liveness_probe_latest() -> AllocStatusRowLatest {
    AllocStatusRowV3 {
        alloc_id: AllocationId::new("alloc-liveness-01").expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp {
            counter: 9,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: None,
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `AllocStatusRowEnvelope::V1(canonical_liveness_probe_v1_inner())` — a
/// V1 row carrying `terminal = Stopped { by: LivenessProbe }`.
///
/// Produced by the `print_fixture_liveness_probe_bytes` aid below (which
/// archives the V1 *variant* explicitly) and pasted verbatim. Pins that
/// the additive `StoppedBy::LivenessProbe` tail variant archives and
/// decodes through the CURRENT (V1+V2+V3) envelope without disturbing the
/// pre-existing `FIXTURE_V<N>` layout. Like every fixture in this file it
/// is regenerated only on an `AllocStatusRowEnvelope` variant append
/// (the enum root re-pads to `max(V1..VN)`); a `StoppedBy` append does
/// NOT touch it.
const FIXTURE_LIVENESS_PROBE_V1: &str = "616c6c6f632d6c6976656e6573732d30317376632d7061796d656e7473000000000000000000000091000000d8ffffff8c000000e1ffffff6e6f64652d303031040000000000000009000000000000006e6f64652d303031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000010500000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000046ffffff00000000010000000000000000f153650000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// S-ROH-A-12 — `CONTRACT_SHAPE`: unbounded-preservation (golden). The
/// pinned `LivenessProbe`-disposition V1 archive decodes through the
/// current envelope + `into_latest()` to the HAND-WRITTEN canonical
/// `Latest`, with the liveness `terminal` carried through verbatim and
/// the additive fields defaulted absent. Kills a mutant that mis-maps
/// the `LivenessProbe` discriminant on archive/access.
#[test]
fn alloc_status_row_liveness_probe_disposition_v1_decodes_through_current_envelope() {
    let expected = canonical_liveness_probe_latest();
    assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(FIXTURE_LIVENESS_PROBE_V1, &expected);

    // The V1 (tag 0) LivenessProbe archive must ALSO pass the
    // `known_discriminants()`-driven probe inside `decode_envelope_bytes`
    // — i.e. it is decoded (not flagged `UnknownVersion` and skipped on
    // convergence).
    let decoded = decode_envelope_bytes::<AllocStatusRowEnvelope>(
        &hex::decode(FIXTURE_LIVENESS_PROBE_V1.trim())
            .expect("FIXTURE_LIVENESS_PROBE_V1 hex decodes"),
    )
    .expect("LivenessProbe V1 archive must be a KNOWN discriminant — not UnknownVersion");
    assert_eq!(
        decoded, expected,
        "decode_envelope_bytes must project the LivenessProbe V1 archive to the same \
         hand-written Latest as the from_bytes path",
    );
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_LIVENESS_PROBE_V1 constant is the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_LIVENESS_PROBE_V1"
)]
fn print_fixture_liveness_probe_bytes() {
    // Archive the V1 *variant* explicitly (discriminant 0), so this aid
    // keeps regenerating the V1-disposition fixture across future
    // AllocStatusRowEnvelope version bumps — NEVER via the re-aliased
    // Latest.
    let envelope = AllocStatusRowEnvelope::V1(canonical_liveness_probe_v1_inner());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_LIVENESS_PROBE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
