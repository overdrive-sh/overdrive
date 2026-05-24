//! Tier 1 acceptance — `ServiceSubmitEvent` V2 wire shape.
//!
//! Slices 01 + 08 (US-01 / US-08). RED scaffolds.
//!
//! Per ADR-0056 / DDD-11: V1→V2 single-cut migration. DELETE
//! `ConvergedRunning` / `ConvergedFailed`; ADD `Stable { settled_in
//! }` + `Failed { reason: ServiceFailureReason, stderr_tail }`.
//!
//! Per ADR-0056 §4 / DDD-10: `ServiceFailureReason` is single
//! per-kind enum; wire projection is in lockstep via property test.

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

/// S-SHCP-WIRE-01 (US-01 / DDD-11) — `ServiceSubmitEvent::Stable
/// { settled_in: Duration, witness: ProbeWitness }` serde
/// round-trip preserves bit-equal payload.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_submit_event_stable_serde_roundtrip_preserves_payload() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-WIRE-01 / ServiceSubmitEvent::Stable serde roundtrip)"
    );
}

/// S-SHCP-WIRE-02 (US-01 / DDD-11) — `ServiceSubmitEvent::Failed
/// { reason: ServiceFailureReason, stderr_tail: Option<String> }`
/// serde round-trip preserves bit-equal payload for each
/// `ServiceFailureReason` variant.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_submit_event_failed_serde_roundtrip_preserves_each_reason_variant() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-WIRE-02 / ServiceSubmitEvent::Failed serde roundtrip for each ServiceFailureReason variant)"
    );
}

/// S-SHCP-WIRE-03 (DDD-10 / DDD-11) — lockstep property: every
/// `ServiceFailureReason` variant has a corresponding
/// `ServiceFailureReasonWire` projection. Adding a new typed
/// variant without adding the wire projection is a structural
/// review rejection.
///
/// (Tagged `@property` — implemented as proptest in DELIVER per
/// `.claude/rules/testing.md` § "Property-based testing".)
#[test]
#[should_panic(expected = "RED scaffold")]
fn every_typed_service_failure_reason_has_wire_projection() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-WIRE-03 / lockstep: ServiceFailureReason ↔ ServiceFailureReasonWire)"
    );
}
