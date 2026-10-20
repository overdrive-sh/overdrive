//! Tier 1 acceptance — `ProbeResultRowEnvelope` latest (V2) round-trip +
//! discriminant pinning per ADR-0054 §5 QR1 and the R5-PROBE-V2 bump
//! (netns-density-295, ADR-0191).
//!
//! Slice 01 (US-01).
//!
//! Per `.claude/rules/testing.md` § "Property-based testing
//! (proptest)" → "Mandatory call sites" → "rkyv roundtrip" + "Archive
//! schema-evolution roundtrip": every rkyv envelope ships a per-
//! version golden-bytes fixture AND a proptest roundtrip.
//!
//! Per ADR-0054 §5 QR1 the historical V1 fixture pins discriminant 0; the
//! V2 bump appends discriminant 1, which `latest()` now writes.
//!
//! Full golden-bytes fixture lives at
//! `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeResultRowEnvelope, ProbeResultRowV2, ProbeRole, ProbeRunSeq,
    ProbeStatus, SameStatusRuns,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators — exercise the full ProbeResultRowV2 product type.
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

fn arb_probe_result_row() -> impl Strategy<Value = ProbeResultRowV2> {
    (
        (arb_alloc_id(), arb_probe_idx(), arb_probe_role(), arb_probe_status()),
        (any::<u64>(), any::<bool>(), 1_u64..=u64::MAX, 1_u32..=u32::MAX),
    )
        .prop_map(
            |(
                (alloc_id, probe_idx, role, status),
                (last_observed_at_unix_ms, inferred, run_seq, same_status_runs),
            )| ProbeResultRowV2 {
                alloc_id,
                probe_idx,
                role,
                status,
                last_observed_at_unix_ms,
                inferred,
                run_seq: ProbeRunSeq::new(run_seq).expect("generator yields a non-zero run"),
                same_status_runs: SameStatusRuns::new(same_status_runs)
                    .expect("generator yields a non-zero streak"),
            },
        )
}

fn pinned_row(alloc: &str) -> ProbeResultRowV2 {
    ProbeResultRowV2 {
        alloc_id: AllocationId::new(alloc).expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 0,
        inferred: false,
        run_seq: ProbeRunSeq::FIRST,
        same_status_runs: SameStatusRuns::ONE,
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-ENV-01 — the latest variant round-trips bit-equivalent.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-SHCP-ENV-01 (US-01 / ADR-0048 + ADR-0054 §5 QR1) —
    /// `ProbeResultRowEnvelope::latest` (V2) round-trips through rkyv archive +
    /// access + deserialize bit-equivalent to the original.
    #[test]
    fn probe_result_row_envelope_latest_rkyv_roundtrip_bit_equivalent(
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
// S-SHCP-ENV-02 — the latest variant's discriminant byte is pinned to 1 (V2).
// ---------------------------------------------------------------------------

/// S-SHCP-ENV-02 (ADR-0054 §5 QR1 + R5-PROBE-V2) — `latest()` writes the
/// V2 variant, whose archived discriminant byte at the pinned offset is 1.
/// The historical V1 bytes (discriminant 0) are pinned by the
/// schema-evolution fixture at
/// `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`.
#[test]
fn probe_result_row_envelope_latest_discriminant_is_pinned_to_one() {
    let envelope = ProbeResultRowEnvelope::latest(pinned_row("alloc-pinned-01"));
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");

    let offset = ProbeResultRowEnvelope::discriminant_offset_from_end()
        .expect("ProbeResultRowEnvelope must pin discriminant_offset_from_end per ADR-0054 §5 QR1");
    let n = bytes.len();
    assert!(n >= offset, "buffer ({n}) must accommodate discriminant offset ({offset})");
    let discriminant = bytes[n - offset];
    assert_eq!(
        discriminant, 1,
        "the V2 discriminant must be pinned to 1 (R5-PROBE-V2); observed {discriminant}"
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

    let envelope = ProbeResultRowEnvelope::latest(pinned_row("alloc-unk-01"));
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
            assert_eq!(supported_max, 1, "supported_max must reflect highest known tag (V2 = 1)");
        }
        EnvelopeError::Malformed { .. } => panic!(
            "expected EnvelopeError::UnknownVersion for unknown-variant bytes; got Malformed"
        ),
    }
}
