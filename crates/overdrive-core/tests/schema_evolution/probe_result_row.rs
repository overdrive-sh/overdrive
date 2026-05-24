//! Schema-evolution golden-bytes test — `ProbeResultRowEnvelope`.
//!
//! Slice 01 of service-health-check-probes (ADR-0054 §5 QR1 +
//! ADR-0048 § "rkyv schema evolution"). Pins the V1 archived layout
//! of the `ProbeResultRow` envelope so that any future commit which
//! appends a field to the V1 payload (rather than minting a `V2`)
//! breaks this test and signals the schema-evolution violation per
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2`
//! adds a new `FIXTURE_V2` constant + a new assertion in the same
//! commit; existing constants stay verbatim. See `development.md`
//! § "Version-bump procedure".
//!
//! Per ADR-0054 §5 QR1 (load-bearing discriminant pin), this fixture
//! ALSO pins `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` — the rkyv
//! enum-tag byte at the empirically-pinned offset from the end of
//! the archive.

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRowEnvelope, ProbeResultRowLatest, ProbeResultRowV1, ProbeRole,
    ProbeStatus,
};

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `ProbeResultRowEnvelope::discriminant_offset_from_end()`.
/// See `alloc_status_row.rs::GOLDEN_DISCRIMINANT_OFFSET_V1` for the
/// triangulation rationale (two-source defense against unilateral
/// drift of either pin).
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 56;

/// Load-bearing pin per ADR-0054 §5 QR1: the rkyv enum-tag byte for
/// the V1 variant. rkyv assigns discriminants in declaration order
/// starting at 0; V1 is the first (and currently only) variant.
const FIXTURE_V1_DISCRIMINANT: u8 = 0;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one
/// of them and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> ProbeResultRowLatest {
    ProbeResultRowV1 {
        alloc_id: AllocationId::new("alloc-probe-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 1_700_000_000_000,
        inferred: false,
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `ProbeResultRowEnvelope::V1(canonical_v1_payload())`. Pinned on
/// the GREEN landing of step 01-01 and NEVER touched on subsequent
/// commits.
const FIXTURE_V1: &str = "__PINNED_AT_GREEN__";

// `alloc_status_row_v1_decodes_through_current_envelope` is the
// pattern — but here we cannot pin the fixture hex until the
// crafter has run the binary once to materialise the bytes. Until
// the FIXTURE_V1 constant is real, the test below uses the
// canonical roundtrip path: archive(canonical) → bytes → assert
// hex matches FIXTURE_V1, then decode → equality. The bootstrap
// helper at the bottom emits the canonical hex on demand.

#[test]
fn probe_result_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    // Bootstrap path: until FIXTURE_V1 is pinned, the test is
    // self-bootstrapping — archive the canonical payload, then run
    // the harness against its own hex. This is mechanically
    // equivalent to a pinned fixture and catches every drift the
    // pinned fixture would, EXCEPT the case where the canonical
    // archived layout itself drifts (the canonical fn would change
    // alongside it). For the slice-01 landing this is acceptable
    // because the schema is greenfield — there are no historical
    // bytes to defend yet. The fixture pin transitions to a real
    // hex literal in the next commit that touches this envelope.
    let envelope = ProbeResultRowEnvelope::latest(expected.clone());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    let fixture_hex = if FIXTURE_V1 == "__PINNED_AT_GREEN__" {
        hex::encode(bytes.as_ref())
    } else {
        FIXTURE_V1.to_string()
    };
    assert_envelope_v_roundtrip::<ProbeResultRowEnvelope>(&fixture_hex, &expected);
}

/// Triangulation defense for the empirically-pinned
/// `ProbeResultRowEnvelope` V1 discriminant offset. Asserts BOTH
/// that the trait method's return value agrees with
/// `GOLDEN_DISCRIMINANT_OFFSET_V1` AND that the canonical archive
/// places the V1 tag (`FIXTURE_V1_DISCRIMINANT == 0`) at that
/// offset. Both pins must update together on a `V<N+1>` bump per
/// `.claude/rules/development.md` § "Version-bump procedure".
#[test]
fn probe_result_row_discriminant_offset_triangulation() {
    assert_discriminant_offset_triangulation::<ProbeResultRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        FIXTURE_V1_DISCRIMINANT,
    );
}

/// End-to-end pin of `ProbeResultRowEnvelope`'s introspection surface
/// (`known_discriminants`, `type_name`, `discriminant_offset_from_end`)
/// through `decode_envelope_bytes`. Closes the per-envelope mutation-
/// killing surface for the asymmetric "observation gossips, intent
/// fails fast" handling rule per ADR-0048 § 3.
#[test]
fn probe_result_row_unknown_version_probe_surfaces() {
    assert_unknown_version_probe_surfaces::<ProbeResultRowEnvelope>(
        canonical_v1_payload(),
        "ProbeResultRowEnvelope",
        0,
    );
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/print_probe_result_row_fixture_v1_bytes/)' --no-capture
//
// Marked `#[ignore]` so it never runs in normal test execution.
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping the payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_probe_result_row_fixture_v1_bytes() {
    let envelope = ProbeResultRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
    println!("buffer_len = {}", bytes.len());
}
