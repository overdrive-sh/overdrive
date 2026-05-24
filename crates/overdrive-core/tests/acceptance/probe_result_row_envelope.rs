//! Tier 1 acceptance — `ProbeResultRowEnvelope` V1 round-trip +
//! discriminant pinning per ADR-0054 §5 QR1.
//!
//! Slice 01 (US-01).
//!
//! Per `.claude/rules/testing.md` § "Property-based testing
//! (proptest)" → "Mandatory call sites" → "rkyv roundtrip" + "Archive
//! schema-evolution roundtrip": every rkyv envelope ships a per-
//! version golden-bytes fixture AND a proptest roundtrip.
//!
//! Per ADR-0054 §5 QR1: the V1 fixture pins BOTH the archived bytes
//! AND `const FIXTURE_V1_DISCRIMINANT: u8 = 0;`.
//!
//! Full golden-bytes fixture lives at
//! `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeResultRowEnvelope, ProbeResultRowV1, ProbeRole, ProbeStatus,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators — exercise the full ProbeResultRowV1 product type.
// ---------------------------------------------------------------------------

fn arb_alloc_id() -> impl Strategy<Value = AllocationId> {
    // Regex constrains both ends to alphanumeric per `validate_label`
    // (DNS-1123-label rule). A pattern like `[a-z][a-z0-9-]{0,40}`
    // would allow trailing `-`, which `validate_label` rejects.
    "[a-z]([a-z0-9-]{0,38}[a-z0-9])?"
        .prop_map(|s| AllocationId::new(&s).expect("generator produces valid label"))
}

fn arb_probe_idx() -> impl Strategy<Value = ProbeIdx> {
    any::<u32>().prop_map(ProbeIdx::new)
}

fn arb_probe_role() -> impl Strategy<Value = ProbeRole> {
    prop_oneof![Just(ProbeRole::Startup), Just(ProbeRole::Readiness), Just(ProbeRole::Liveness),]
}

fn arb_probe_status() -> impl Strategy<Value = ProbeStatus> {
    prop_oneof![
        Just(ProbeStatus::Pass),
        "[a-zA-Z0-9 :_./]{0,80}".prop_map(|reason| ProbeStatus::Fail { last_fail_reason: reason }),
    ]
}

fn arb_probe_result_row() -> impl Strategy<Value = ProbeResultRowV1> {
    (
        arb_alloc_id(),
        arb_probe_idx(),
        arb_probe_role(),
        arb_probe_status(),
        any::<u64>(),
        any::<bool>(),
    )
        .prop_map(|(alloc_id, probe_idx, role, status, last_observed_at_unix_ms, inferred)| {
            ProbeResultRowV1 {
                alloc_id,
                probe_idx,
                role,
                status,
                last_observed_at_unix_ms,
                inferred,
            }
        })
}

// ---------------------------------------------------------------------------
// S-SHCP-ENV-01 — V1 round-trips bit-equivalent through rkyv codec.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-SHCP-ENV-01 (US-01 / ADR-0048 + ADR-0054 §5 QR1) —
    /// `ProbeResultRowEnvelope::V1` round-trips through rkyv archive +
    /// access + deserialize bit-equivalent to the original.
    #[test]
    fn probe_result_row_envelope_v1_rkyv_roundtrip_bit_equivalent(
        row in arb_probe_result_row(),
    ) {
        let envelope = ProbeResultRowEnvelope::latest(row.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .expect("rkyv archive must succeed");

        let mut aligned = rkyv::util::AlignedVec::<8>::new();
        aligned.extend_from_slice(&bytes);

        let decoded: ProbeResultRowEnvelope =
            rkyv::from_bytes::<ProbeResultRowEnvelope, rkyv::rancor::Error>(&aligned)
                .expect("rkyv deserialize must succeed");
        let projected: ProbeResultRow = decoded
            .into_latest()
            .expect("envelope into_latest projection must succeed");
        prop_assert_eq!(projected, row);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-ENV-02 — Discriminant byte position pinned to 0 (V1 tag).
// ---------------------------------------------------------------------------

/// S-SHCP-ENV-02 (ADR-0054 §5 QR1 — load-bearing discriminant pin)
/// — `ProbeResultRowEnvelope::V1` archived bytes have first
/// discriminant byte == 0. Future V2/V3 append at the tail only.
///
/// The schema-evolution fixture at
/// `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
/// declares `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` and pins this
/// invariant.
#[test]
fn probe_result_row_envelope_v1_discriminant_is_pinned_to_zero() {
    let row = ProbeResultRowV1 {
        alloc_id: AllocationId::new("alloc-pinned-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 0,
        inferred: false,
    };
    let envelope = ProbeResultRowEnvelope::latest(row);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");

    let offset = ProbeResultRowEnvelope::discriminant_offset_from_end()
        .expect("ProbeResultRowEnvelope must pin discriminant_offset_from_end per ADR-0054 §5 QR1");
    let n = bytes.len();
    assert!(n >= offset, "buffer ({n}) must accommodate discriminant offset ({offset})");
    let discriminant = bytes[n - offset];
    assert_eq!(
        discriminant, 0,
        "V1 discriminant must be pinned to 0 per ADR-0054 §5 QR1; observed {discriminant}"
    );
}

// ---------------------------------------------------------------------------
// S-SHCP-ENV-03 — Unknown variant surfaces as UnknownVersion (asymmetric
// handling per ADR-0048 — observation gossips and converges, NOT
// `health.startup.refused` fail-fast).
// ---------------------------------------------------------------------------

/// S-SHCP-ENV-03 (ADR-0048 § "intent fail-fast policy" + ADR-0054 §5
/// — observation surface is gossiped, not fail-fast) — malformed
/// archived bytes for `ProbeResultRowEnvelope` whose discriminant
/// byte names an unknown variant surface
/// `EnvelopeError::UnknownVersion` via the pre-decode probe per
/// ADR-0048 § 3. The observation-layer consumer logs + skips
/// (warn-not-fail-fast) per ADR-0048 § "Unknown / malformed handling
/// is asymmetric by layer".
#[test]
fn probe_result_row_envelope_unknown_variant_surfaces_unknown_version() {
    use overdrive_core::codec::{EnvelopeError, decode_envelope_bytes};

    let row = ProbeResultRowV1 {
        alloc_id: AllocationId::new("alloc-unk-01").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 0,
        inferred: false,
    };
    let envelope = ProbeResultRowEnvelope::latest(row);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");

    // Synthesise an unknown-variant byte sequence: flip the
    // discriminant at the pinned offset to a tag outside
    // known_discriminants(). Tag 99 mirrors the cross-test convention
    // used by AllocStatusRowEnvelope's unknown-variant test.
    let offset = ProbeResultRowEnvelope::discriminant_offset_from_end().expect("offset pinned");
    let mut synthesised = bytes.as_ref().to_vec();
    let n = synthesised.len();
    synthesised[n - offset] = 99;

    let err = decode_envelope_bytes::<ProbeResultRowEnvelope>(&synthesised)
        .expect_err("synthesised unknown-tag bytes must error");

    match err {
        EnvelopeError::UnknownVersion { observed, type_name, supported_max } => {
            assert_eq!(observed, 99, "probe must surface observed discriminant verbatim");
            assert_eq!(
                type_name, "ProbeResultRowEnvelope",
                "probe must name the envelope whose decode path surfaced the unknown tag"
            );
            assert_eq!(supported_max, 0, "supported_max must reflect highest known tag (V1 = 0)");
        }
        EnvelopeError::Malformed { .. } => panic!(
            "expected EnvelopeError::UnknownVersion for unknown-variant bytes; got Malformed"
        ),
    }
}
