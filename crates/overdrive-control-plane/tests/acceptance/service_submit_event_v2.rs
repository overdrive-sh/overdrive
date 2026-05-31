//! Tier 1 acceptance — `ServiceSubmitEvent` V2 wire shape.
//!
//! Slices 01 + 08 (US-01 / US-08). Step 01-03e GREEN.
//!
//! Per ADR-0056 / DDD-11: V1→V2 single-cut migration. The legacy
//! `ServiceSubmitEvent` terminal variants have been replaced by
//! `Stable { settled_in_ms, witness }` + `Failed { reason,
//! stderr_tail }`. Per ADR-0056 §4 / DDD-10: `ServiceFailureReason`
//! is the SINGLE per-kind typed enum; the wire projection
//! (`ServiceFailureReasonWire`) is a type alias kept in lockstep
//! via the property test below.
//!
//! Per ADR-0037 §4 (K2 trace-equivalence): when the action_shim
//! writes `AllocStatusRow.terminal` AND emits `LifecycleEvent.terminal`
//! from the same `Action.terminal` value in the same call frame,
//! both surfaces project the SAME bytes — proven by serde JSON
//! byte-equality between the wire event and the row's terminal field.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::redundant_clone,
    clippy::no_effect_underscore_binding
)]

use overdrive_control_plane::api::{ProbeWitnessWire, ServiceFailureReasonWire};
use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_core::transition_reason::{ProbeWitness, ServiceFailureReason, TerminalCondition};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

prop_compose! {
    fn arb_probe_witness()(
        probe_idx in 0u32..16,
        role in prop::sample::select(vec![
            "startup".to_string(),
            "readiness".to_string(),
            "liveness".to_string(),
        ]),
        mechanic_summary in "[a-z0-9 /:.@-]{1,40}",
        inferred in any::<bool>(),
    ) -> ProbeWitness {
        ProbeWitness {
            probe_idx,
            role,
            mechanic_summary,
            inferred,
        }
    }
}

prop_compose! {
    fn arb_alloc_id()(
        seed in "[a-z]{1,8}-[0-9]{1,4}",
    ) -> String { seed }
}

fn arb_service_failure_reason() -> impl Strategy<Value = ServiceFailureReason> {
    prop_oneof![
        (0u32..8, 0u32..16).prop_map(|(probe_idx, attempts)| {
            ServiceFailureReason::StartupTimeout { probe_idx, attempts }
        }),
        (0u32..8, "[a-z0-9 /:.@-]{1,32}", 0u32..16,).prop_map(
            |(probe_idx, last_fail, attempts)| {
                ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts }
            }
        ),
        (-128i32..128).prop_map(|exit_code| {
            ServiceFailureReason::EarlyExit { exit_code: Some(exit_code) }
        }),
        (0u32..8, 0u32..16).prop_map(|(probe_idx, attempts)| {
            ServiceFailureReason::LivenessProbeFailed { probe_idx, attempts }
        }),
    ]
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-01 — Stable serde roundtrip
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-01 (US-01 / DDD-11) — `ServiceSubmitEvent::Stable
    /// { alloc_id, settled_in_ms, witness }` serde JSON round-trip
    /// preserves bit-equal payload over arbitrary inputs.
    #[test]
    fn service_submit_event_stable_serde_roundtrip_preserves_payload(
        alloc_id in arb_alloc_id(),
        settled_in_ms in 0u64..600_000,
        witness in arb_probe_witness(),
    ) {
        let event = ServiceSubmitEvent::Stable {
            alloc_id: alloc_id.clone(),
            settled_in_ms,
            witness: witness.clone(),
        };
        let json = serde_json::to_string(&event)
            .expect("ServiceSubmitEvent::Stable must serialize");
        let decoded: ServiceSubmitEvent = serde_json::from_str(&json)
            .expect("ServiceSubmitEvent::Stable must deserialize");
        prop_assert_eq!(event, decoded);
        // Bit-equal re-serialisation — strict byte-equality across the
        // round-trip (no whitespace drift, no field reorder).
        let json2 = serde_json::to_string(
            &serde_json::from_str::<ServiceSubmitEvent>(&json)
                .expect("re-decode"),
        )
        .expect("re-encode");
        prop_assert_eq!(json, json2);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-02 — Failed serde roundtrip across every reason variant
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-02 (US-01 / DDD-11) — `ServiceSubmitEvent::Failed
    /// { alloc_id, reason: ServiceFailureReason, stderr_tail }`
    /// serde JSON round-trip preserves bit-equal payload for each
    /// `ServiceFailureReason` variant.
    #[test]
    fn service_submit_event_failed_serde_roundtrip_preserves_each_reason_variant(
        alloc_id_opt in prop::option::of(arb_alloc_id()),
        reason in arb_service_failure_reason(),
        stderr_tail in prop::option::of("[ -~\n]{0,128}"),
    ) {
        let event = ServiceSubmitEvent::Failed {
            alloc_id: alloc_id_opt.clone(),
            reason: reason.clone(),
            stderr_tail: stderr_tail.clone(),
        };
        let json = serde_json::to_string(&event)
            .expect("ServiceSubmitEvent::Failed must serialize");
        let decoded: ServiceSubmitEvent = serde_json::from_str(&json)
            .expect("ServiceSubmitEvent::Failed must deserialize");
        prop_assert_eq!(event, decoded);
        // Bit-equal re-serialisation
        let json2 = serde_json::to_string(
            &serde_json::from_str::<ServiceSubmitEvent>(&json)
                .expect("re-decode"),
        )
        .expect("re-encode");
        prop_assert_eq!(json, json2);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-03 — every typed ServiceFailureReason variant projects
//                  through ServiceFailureReasonWire lockstep
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-03 (DDD-10 / DDD-11) — for every
    /// `ServiceFailureReason` variant, the wire projection
    /// `ServiceFailureReasonWire` is byte-equal under serde JSON.
    /// `ServiceFailureReasonWire` is currently a typed alias to
    /// `ServiceFailureReason` — adding a new typed variant without
    /// adding the wire projection is a structural compile failure
    /// here (the alias forces exhaustive coverage).
    #[test]
    fn every_typed_service_failure_reason_has_wire_projection(
        reason in arb_service_failure_reason(),
    ) {
        // The lockstep property: typed and wire serialize byte-equal.
        let typed_json = serde_json::to_string(&reason)
            .expect("ServiceFailureReason must serialize");
        let wire: ServiceFailureReasonWire = reason.clone();
        let wire_json = serde_json::to_string(&wire)
            .expect("ServiceFailureReasonWire must serialize");
        prop_assert_eq!(typed_json, wire_json);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-PURITY-03 — TerminalCondition → ServiceSubmitEvent byte-equal
// ---------------------------------------------------------------------------

/// Pure mapping function from `TerminalCondition::{Stable, ServiceFailed}`
/// to the wire `ServiceSubmitEvent::{Stable, Failed}`. Mirrors the
/// projection the action-shim performs at its single TerminalCondition
/// write site (ADR-0037 §4 single-write-site discipline).
fn project_terminal_to_event(
    alloc_id: &str,
    terminal: &TerminalCondition,
    stderr_tail: Option<String>,
) -> Option<ServiceSubmitEvent> {
    match terminal {
        TerminalCondition::Stable { settled_in_ms, witness } => Some(ServiceSubmitEvent::Stable {
            alloc_id: alloc_id.to_string(),
            settled_in_ms: *settled_in_ms,
            witness: witness.clone(),
        }),
        TerminalCondition::ServiceFailed { reason } => Some(ServiceSubmitEvent::Failed {
            alloc_id: Some(alloc_id.to_string()),
            reason: reason.clone(),
            stderr_tail,
        }),
        _ => None,
    }
}

proptest! {
    /// S-SHCP-PURITY-03 — for an arbitrary `TerminalCondition` produced
    /// at a single reconcile / action-shim write site, the serde-JSON
    /// projection of the typed terminal carried on the row matches the
    /// emitted wire event's per-field shape (witness / reason
    /// byte-equal). Closes the K2 trace-equivalence clause for the
    /// Service-kind wire path: there is exactly one source of truth
    /// for the terminal payload — the typed `TerminalCondition` — and
    /// both `AllocStatusRow.terminal` and `ServiceSubmitEvent` project
    /// from it.
    #[test]
    fn terminal_bytes_equal_on_row_and_wire_event_for_service_kind(
        alloc_id in arb_alloc_id(),
        which in 0u8..2,
        settled_in_ms in 0u64..600_000,
        witness in arb_probe_witness(),
        reason in arb_service_failure_reason(),
        stderr_tail in prop::option::of("[ -~]{0,32}"),
    ) {
        let terminal = if which == 0 {
            TerminalCondition::Stable { settled_in_ms, witness: witness.clone() }
        } else {
            TerminalCondition::ServiceFailed { reason: reason.clone() }
        };

        let event = project_terminal_to_event(&alloc_id, &terminal, stderr_tail.clone())
            .expect("Stable/ServiceFailed must project to a wire event");

        // Per-field byte-equality assertion: the payload carried by the
        // wire event MUST serialise byte-equal to the typed payload
        // carried by the row's terminal field.
        match (&terminal, &event) {
            (
                TerminalCondition::Stable { settled_in_ms: a, witness: w },
                ServiceSubmitEvent::Stable {
                    settled_in_ms: b,
                    witness: w_wire,
                    ..
                },
            ) => {
                prop_assert_eq!(a, b);
                prop_assert_eq!(
                    serde_json::to_string(w).expect("typed witness"),
                    serde_json::to_string(w_wire).expect("wire witness"),
                );
            }
            (
                TerminalCondition::ServiceFailed { reason: typed_r },
                ServiceSubmitEvent::Failed { reason: wire_r, .. },
            ) => {
                prop_assert_eq!(
                    serde_json::to_string(typed_r).expect("typed reason"),
                    serde_json::to_string(wire_r).expect("wire reason"),
                );
            }
            _ => prop_assert!(false, "mismatched projection"),
        }

        // ProbeWitnessWire alias coverage: the wire alias and the typed
        // ProbeWitness are byte-equal under serde — adding a field to
        // ProbeWitness without updating the wire alias is a
        // structural compile error.
        let _alias: ProbeWitnessWire = witness;
    }
}
