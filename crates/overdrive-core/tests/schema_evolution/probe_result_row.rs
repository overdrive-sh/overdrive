//! Schema-evolution golden-bytes test — `ProbeResultRowEnvelope`
//! (R5-PROBE-V2, netns-density-295 PROBE-RUN-COUNT / ADR-0191).
//!
//! Pins the archived layout of both envelope variants:
//!
//! - `V1` (discriminant 0) — the pre-PROBE-RUN-COUNT payload, with its
//!   current D16-G1 layout.
//! - `V2` (discriminant 1) — `run_seq` and `same_status_runs`, each other
//!   than 1 and different from each other.
//!
//! Per ADR-0054 §5 QR1 the discriminants are load-bearing pins; the tag
//! offset from the archive end is triangulated against
//! `ProbeResultRowEnvelope::discriminant_offset_from_end()`.

#![allow(
    clippy::doc_markdown,
    reason = "DISTILL test support and conformance drivers: handler signatures follow the action map and fixtures assert their preconditions"
)]

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRowEnvelope, ProbeResultRowLatest, ProbeResultRowV1, ProbeResultRowV2,
    ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the envelope's discriminant offset (from the archive
/// end) for triangulation against `discriminant_offset_from_end()`. The
/// archived envelope root is sized to its largest variant (V2), so the tag
/// sits at the same offset for a V1 and a V2 archive.
const GOLDEN_DISCRIMINANT_OFFSET: usize = 64;

/// rkyv enum tag of the V1 variant (declaration order).
const FIXTURE_V1_DISCRIMINANT: u8 = 0;

/// rkyv enum tag of the V2 variant (declaration order).
const FIXTURE_V2_DISCRIMINANT: u8 = 1;

/// Canonical V1 payload for the current-layout golden.
fn canonical_v1_payload() -> ProbeResultRowV1 {
    ProbeResultRowV1 {
        alloc_id: AllocationId::new("alloc-probe-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 1_700_000_000_000,
    }
}

/// The `Latest` projection of `canonical_v1_payload()`, written out by hand:
/// every V1 field copied, `run_seq = FIRST`, `same_status_runs = ONE`.
fn expected_v1_up_converted() -> ProbeResultRowLatest {
    ProbeResultRowV2 {
        alloc_id: AllocationId::new("alloc-probe-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 1_700_000_000_000,
        run_seq: ProbeRunSeq::FIRST,
        same_status_runs: SameStatusRuns::ONE,
    }
}

/// Canonical V2 payload for the current-layout golden. `run_seq` (7) and
/// `same_status_runs` (3) are both other than 1 and differ from each other.
fn canonical_v2_payload() -> ProbeResultRowV2 {
    ProbeResultRowV2 {
        alloc_id: AllocationId::new("alloc-probe-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(1),
        role: ProbeRole::Liveness,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 1_700_000_000_000,
        run_seq: ProbeRunSeq::new(7).expect("non-zero run sequence"),
        same_status_runs: SameStatusRuns::new(3).expect("non-zero streak"),
    }
}

/// Historical pre-D16 V1 archive bytes. Preserved verbatim under the
/// greenfield waiver and not asserted as a current-layout read fixture.
#[allow(
    dead_code,
    reason = "the greenfield waiver preserves pre-D16 bytes without a backward-read promise"
)]
const FIXTURE_V1: &str = "616c6c6f632d70726f62652d3031000000000000000000008e000000e8ffffff0000000000000000000000000000000000000000000000000068e5cf8b010000000000000000000000000000000000000000000000000000";

/// Historical pre-D16 V2 archive bytes. Preserved verbatim under the
/// greenfield waiver and not asserted as a current-layout read fixture.
#[allow(
    dead_code,
    reason = "the greenfield waiver preserves pre-D16 bytes without a backward-read promise"
)]
const FIXTURE_V2: &str = "616c6c6f632d70726f62652d3031000001000000000000008e000000e8ffffff0100000002000000000000000000000000000000000000000068e5cf8b010000010000000000000007000000000000000300000000000000";

/// Current-layout V1 bytes after D16-G1 removes the inferred flag. The old
/// `FIXTURE_V1` bytes remain unchanged as a historical artifact and are not a
/// backward-read promise for this greenfield carrier.
const CURRENT_LAYOUT_V1: &str = "616c6c6f632d70726f62652d3031000000000000000000008e000000e8ffffff0000000000000000000000000000000000000000000000000068e5cf8b01000000000000000000000000000000000000";

/// Current-layout V2 bytes after D16-G1 removes the inferred flag. The old
/// `FIXTURE_V2` bytes remain unchanged as a historical artifact and are not a
/// backward-read promise for this greenfield carrier.
const CURRENT_LAYOUT_V2: &str = "616c6c6f632d70726f62652d3031000001000000000000008e000000e8ffffff0100000002000000000000000000000000000000000000000068e5cf8b01000007000000000000000300000000000000";

/// The current payload layouts omit the inferred flag while retaining the
/// existing V1/V2 envelope tags and V1-to-V2 run-count projection.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_fixtures_match_current_encoding() {
    assert_eq!(
        hex::encode(archive(&ProbeResultRowEnvelope::V1(canonical_v1_payload()))),
        CURRENT_LAYOUT_V1,
        "current V1 row layout must omit the inferred flag",
    );
    assert_eq!(
        hex::encode(archive(&ProbeResultRowEnvelope::V2(canonical_v2_payload()))),
        CURRENT_LAYOUT_V2,
        "current V2 row layout must omit the inferred flag",
    );
    assert_eq!(ProbeResultRowEnvelope::known_discriminants(), &[0, 1]);
}

fn archive(envelope: &ProbeResultRowEnvelope) -> Vec<u8> {
    rkyv::to_bytes::<rkyv::rancor::Error>(envelope).expect("rkyv archive").to_vec()
}

fn decode_variant(fixture_hex: &str) -> ProbeResultRowEnvelope {
    let bytes = hex::decode(fixture_hex).expect("fixture is hex");
    // On-disk reads land at unknown alignment; copy into an aligned buffer
    // as the production read path and the harness do.
    let mut aligned = rkyv::util::AlignedVec::<16>::new();
    aligned.extend_from_slice(&bytes);
    rkyv::from_bytes::<ProbeResultRowEnvelope, rkyv::rancor::Error>(&aligned)
        .expect("fixture decodes as the envelope")
}

/// A V1 row persisted before PROBE-RUN-COUNT decodes as the V1 variant and
/// up-converts to a V2 row with `run_seq` 1 and `same_status_runs` 1.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_v1_decodes_and_up_converts_with_first_run() {
    assert!(
        matches!(decode_variant(CURRENT_LAYOUT_V1), ProbeResultRowEnvelope::V1(ref v1) if *v1 == canonical_v1_payload()),
        "current-layout V1 bytes decode as the V1 variant carrying the canonical V1 payload",
    );
    assert_envelope_v_roundtrip::<ProbeResultRowEnvelope>(
        CURRENT_LAYOUT_V1,
        &expected_v1_up_converted(),
    );
}

/// A V2 row decodes as the V2 variant, field for field.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_v2_decodes_through_current_envelope() {
    assert!(
        matches!(decode_variant(CURRENT_LAYOUT_V2), ProbeResultRowEnvelope::V2(_)),
        "current-layout V2 bytes decode as the V2 variant",
    );
    assert_envelope_v_roundtrip::<ProbeResultRowEnvelope>(
        CURRENT_LAYOUT_V2,
        &canonical_v2_payload(),
    );
}

/// `latest()` wraps the V2 variant: a fresh write never encodes V1.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_latest_wraps_v2() {
    assert!(matches!(
        ProbeResultRowEnvelope::latest(canonical_v2_payload()),
        ProbeResultRowEnvelope::V2(_)
    ));
    assert_eq!(ProbeResultRowEnvelope::known_discriminants(), &[0, 1]);
}

/// Triangulation of the discriminant offset on a canonical V2 archive (the
/// `latest()` encoding), and the V1 tag at the same offset of an explicit V1
/// archive.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<ProbeResultRowEnvelope>(
        canonical_v2_payload(),
        GOLDEN_DISCRIMINANT_OFFSET,
        FIXTURE_V2_DISCRIMINANT,
    );
    let v1_bytes = archive(&ProbeResultRowEnvelope::V1(canonical_v1_payload()));
    assert!(v1_bytes.len() >= GOLDEN_DISCRIMINANT_OFFSET);
    assert_eq!(
        v1_bytes[v1_bytes.len() - GOLDEN_DISCRIMINANT_OFFSET],
        FIXTURE_V1_DISCRIMINANT,
        "the V1 tag must sit at the envelope's pinned offset",
    );
}

/// End-to-end pin of the envelope's introspection surface through
/// `decode_envelope_bytes`: valid V2 bytes round-trip, an unknown tag
/// surfaces `UnknownVersion { supported_max: 1 }`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn probe_result_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<ProbeResultRowEnvelope>(
        canonical_v2_payload(),
        "ProbeResultRowEnvelope",
        1,
    );
}

// ---------------------------------------------------------------------
// Current-layout golden regeneration helpers. Run on demand only:
//
//   cargo xtask lima run -- cargo nextest run -p overdrive-core \
//       --test schema_evolution --run-ignored only \
//       -E 'test(/print_probe_result_row_fixture_v/)' --no-capture
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when pinning a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits current-layout hex for review"
)]
fn print_probe_result_row_fixture_v1_bytes() {
    let bytes = archive(&ProbeResultRowEnvelope::V1(canonical_v1_payload()));
    println!("CURRENT_LAYOUT_V1 = \"{}\"", hex::encode(&bytes));
    println!("buffer_len = {}", bytes.len());
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when pinning a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits current-layout hex for review"
)]
fn print_probe_result_row_fixture_v2_bytes() {
    let bytes = archive(&ProbeResultRowEnvelope::V2(canonical_v2_payload()));
    println!("CURRENT_LAYOUT_V2 = \"{}\"", hex::encode(&bytes));
    println!("buffer_len = {}", bytes.len());
}
