//! Tier 1 acceptance — `ProbeResultRowEnvelope` V1 round-trip +
//! discriminant pinning per ADR-0054 §5 QR1.
//!
//! Slice 01 (US-01). RED scaffolds.
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
//! `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
//! (created in slice 01 DELIVER).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

/// S-SHCP-ENV-01 (US-01 / ADR-0048 + ADR-0054 §5 QR1) —
/// `ProbeResultRowEnvelope::V1` round-trips through rkyv archive +
/// access + deserialize bit-equivalent to the original.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probe_result_row_envelope_v1_rkyv_roundtrip_bit_equivalent() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-ENV-01 / ProbeResultRowEnvelope::V1 rkyv roundtrip)"
    );
}

/// S-SHCP-ENV-02 (ADR-0054 §5 QR1 — load-bearing discriminant pin)
/// — `ProbeResultRowEnvelope::V1` archived bytes have first
/// discriminant byte == 0. Future V2/V3 append at the tail only.
///
/// The schema-evolution fixture at
/// `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
/// declares `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` and pins this
/// invariant.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probe_result_row_envelope_v1_discriminant_is_pinned_to_zero() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-ENV-02 / ProbeResultRowEnvelope::V1 discriminant pinned to 0)"
    );
}

/// S-SHCP-ENV-03 (ADR-0048 § "intent fail-fast policy" + ADR-0054 §5
/// — observation surface is gossiped, not fail-fast) — malformed
/// archived bytes for ProbeResultRowEnvelope yield
/// `EnvelopeError::UnsupportedVariant` (logged via
/// `tracing::warn!`, NOT `health.startup.refused`). Per ADR-0048 §
/// "Reads up-convert via into_latest()" + "Unknown / malformed
/// handling is asymmetric by layer" — observation is gossiped,
/// converges.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probe_result_row_envelope_unknown_variant_is_warn_skip_not_refuse() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-ENV-03 / unknown envelope variant warn-skips, does NOT refuse startup)"
    );
}
