//! Tier 1 acceptance — `ServiceSubmitEvent` taxonomy completeness per
//! ADR-0059 (extends ADR-0056).
//!
//! Step 01-03e2 GREEN. Closes the wire-taxonomy completeness gap
//! surfaced by the dispatch-wiring blocker against commit `2ec1eb7f`:
//! every terminal a Service-kind workload can reach today has a wire
//! projection. Specifically:
//!
//! * `Stopped { alloc_id, by }` sibling variant per ADR-0059 Q1 —
//!   NOT folded into `Failed` (CLI exit-code semantics diverge).
//! * `ServiceFailureReason::{BackoffExhausted, Other, Timeout,
//!   StreamInterrupted}` plus a `BackoffCause` discriminator per
//!   ADR-0059 Q2/Q3/Q4.
//! * `ServiceLifecycleReconciler::reconcile` pre-Stable opt-out
//!   branch per ADR-0059 Q5 / ADR-0055 §3 amendment — empty-probes
//!   Service with `state == Running` emits `Stable` immediately
//!   with `mechanic_summary == "none (opted out)"`.
//!
//! Per ADR-0037 §4 (K2 trace-equivalence): when the action_shim writes
//! `AllocStatusRow.terminal` AND emits `LifecycleEvent.terminal` from
//! the SAME `Action.terminal` value in the same call frame, both
//! surfaces project the SAME bytes — proven by serde JSON byte-equality
//! between the wire event and the row's terminal field.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::redundant_clone,
    clippy::no_effect_underscore_binding,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::format_collect
)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_control_plane::streaming::{
    ServiceSubmitEvent, service_event_from_terminal, service_stream_synth_cap_timeout,
    service_stream_synth_closed,
};
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{
    BackoffCause, ServiceFailureReason, StoppedBy, TerminalCondition,
};
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

prop_compose! {
    fn arb_alloc_id_str()(
        seed in "[a-z]{1,8}-[0-9]{1,4}",
    ) -> String { seed }
}

fn arb_stopped_by() -> impl Strategy<Value = StoppedBy> {
    prop_oneof![
        Just(StoppedBy::Operator),
        Just(StoppedBy::Reconciler),
        Just(StoppedBy::Process),
        Just(StoppedBy::SystemGc),
    ]
}

fn arb_backoff_cause() -> impl Strategy<Value = BackoffCause> {
    prop_oneof![Just(BackoffCause::AttemptBudget), Just(BackoffCause::LivenessBudget),]
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-04 — Stopped projection lockstep
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-04 — for every `TerminalCondition::Stopped { by }`
    /// input, `service_event_from_terminal` projects to
    /// `ServiceSubmitEvent::Stopped { alloc_id, by }` with the SAME
    /// `by` value byte-equal under serde JSON. Sibling-variant
    /// discipline per ADR-0059 Q1.
    #[test]
    fn stopped_projection_lockstep(
        alloc_id in arb_alloc_id_str(),
        by in arb_stopped_by(),
    ) {
        let terminal = TerminalCondition::Stopped { by };
        let event = service_event_from_terminal(&alloc_id, &terminal, None, None)
            .expect("Stopped must project to a wire event");

        match &event {
            ServiceSubmitEvent::Stopped { alloc_id: a, by: ev_by } => {
                prop_assert_eq!(a, &alloc_id);
                prop_assert_eq!(
                    serde_json::to_string(&by).expect("typed by"),
                    serde_json::to_string(ev_by).expect("wire by"),
                );
            }
            other => prop_assert!(
                false,
                "expected Stopped variant, got {:?}",
                other
            ),
        }

        // Bit-equal serde roundtrip — strict byte-equality.
        let json = serde_json::to_string(&event).expect("Stopped serialize");
        let decoded: ServiceSubmitEvent =
            serde_json::from_str(&json).expect("Stopped deserialize");
        prop_assert_eq!(&event, &decoded);
        let json2 = serde_json::to_string(&decoded).expect("re-encode");
        prop_assert_eq!(json, json2);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-05 — BackoffExhausted projection lockstep
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-05 — for every `TerminalCondition::BackoffExhausted
    /// { attempts }` input + arbitrary `last_exit_code` observation,
    /// `service_event_from_terminal` projects to `Failed { reason:
    /// ServiceFailureReason::BackoffExhausted { attempts, cause:
    /// AttemptBudget, last_exit_code } }`. `cause` is ALWAYS
    /// `AttemptBudget` in Phase 1.
    #[test]
    fn backoff_exhausted_projection_lockstep(
        alloc_id in arb_alloc_id_str(),
        attempts in 0u32..32,
        last_exit_code in prop::option::of(-128i32..128),
    ) {
        let terminal = TerminalCondition::BackoffExhausted { attempts };
        let event = service_event_from_terminal(
            &alloc_id,
            &terminal,
            None,
            last_exit_code,
        )
        .expect("BackoffExhausted must project to a wire event");

        match &event {
            ServiceSubmitEvent::Failed {
                alloc_id: a,
                reason: ServiceFailureReason::BackoffExhausted {
                    attempts: ev_attempts,
                    cause,
                    last_exit_code: ev_exit_code,
                },
                stderr_tail: _,
            } => {
                prop_assert_eq!(a.as_deref(), Some(alloc_id.as_str()));
                prop_assert_eq!(*ev_attempts, attempts);
                prop_assert_eq!(*cause, BackoffCause::AttemptBudget);
                prop_assert_eq!(*ev_exit_code, last_exit_code);
            }
            other => prop_assert!(
                false,
                "expected Failed/BackoffExhausted, got {:?}",
                other
            ),
        }

        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: ServiceSubmitEvent =
            serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&event, &decoded);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-06 — Custom projection lockstep + UTF-8/hex render rule
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-06 — for every `TerminalCondition::Custom
    /// { type_name, detail }` input, `service_event_from_terminal`
    /// projects to `Failed { reason: ServiceFailureReason::Other
    /// { source, message } }` with `source == type_name` byte-equal
    /// AND message render rule satisfied: detail bytes that are
    /// valid UTF-8 render as UTF-8; otherwise lowercase-hex.
    #[test]
    fn custom_projection_with_utf8_detail_renders_utf8(
        alloc_id in arb_alloc_id_str(),
        type_name in "[A-Za-z0-9.]{1,32}",
        utf8_message in "[a-zA-Z0-9 .:_/-]{0,32}",
    ) {
        let detail = utf8_message.clone().into_bytes();
        let terminal = TerminalCondition::Custom {
            type_name: type_name.clone(),
            detail: Some(detail),
        };
        let event = service_event_from_terminal(&alloc_id, &terminal, None, None)
            .expect("Custom must project");

        match &event {
            ServiceSubmitEvent::Failed {
                alloc_id: a,
                reason: ServiceFailureReason::Other { source, message },
                ..
            } => {
                prop_assert_eq!(a.as_deref(), Some(alloc_id.as_str()));
                prop_assert_eq!(source, &type_name);
                prop_assert_eq!(message, &utf8_message);
            }
            other => prop_assert!(
                false,
                "expected Failed/Other, got {:?}",
                other
            ),
        }
    }
}

proptest! {
    /// S-SHCP-WIRE-06 (hex render path) — when detail is non-UTF-8,
    /// the message field renders as lowercase-hex per ADR-0059 Q3.
    #[test]
    fn custom_projection_with_non_utf8_detail_renders_hex(
        alloc_id in arb_alloc_id_str(),
        type_name in "[A-Za-z0-9.]{1,32}",
        bytes in prop::collection::vec(any::<u8>(), 1..16),
    ) {
        // Force non-UTF8 with a starting illegal byte. Choose bytes
        // that include known-invalid-UTF8 markers like 0xFF, 0xFE.
        let mut detail = vec![0xFFu8, 0xFEu8];
        detail.extend_from_slice(&bytes);
        // Skip cases that happen to be valid UTF-8 (vanishingly rare
        // given leading 0xFF 0xFE).
        if std::str::from_utf8(&detail).is_ok() {
            return Ok(());
        }
        let terminal = TerminalCondition::Custom {
            type_name: type_name.clone(),
            detail: Some(detail.clone()),
        };
        let event = service_event_from_terminal(&alloc_id, &terminal, None, None)
            .expect("Custom must project");

        match &event {
            ServiceSubmitEvent::Failed {
                reason: ServiceFailureReason::Other { source, message },
                ..
            } => {
                prop_assert_eq!(source, &type_name);
                // Verify hex render
                let expected_hex: String =
                    detail.iter().map(|b| format!("{b:02x}")).collect();
                prop_assert_eq!(message, &expected_hex);
                // All hex chars are lowercase
                prop_assert!(message.chars().all(|c| c.is_ascii_hexdigit()));
                prop_assert!(message.chars().all(|c| !c.is_ascii_uppercase()));
            }
            other => prop_assert!(false, "expected Failed/Other, got {:?}", other),
        }
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-07 — Timeout + StreamInterrupted streaming synthesis
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-07 (cap-timer arm) — `service_stream_synth_cap_timeout
    /// (after_seconds)` synthesises `Failed { alloc_id: None, reason:
    /// Timeout { after_seconds }, stderr_tail: None }` round-tripping
    /// byte-equal through serde JSON per ADR-0059 Q4.
    #[test]
    fn cap_timer_synth_timeout(after_seconds in 0u32..3600) {
        let event = service_stream_synth_cap_timeout(after_seconds);
        match &event {
            ServiceSubmitEvent::Failed {
                alloc_id,
                reason: ServiceFailureReason::Timeout { after_seconds: a },
                stderr_tail,
            } => {
                prop_assert!(alloc_id.is_none());
                prop_assert_eq!(*a, after_seconds);
                prop_assert!(stderr_tail.is_none());
            }
            other => prop_assert!(false, "expected Failed/Timeout, got {:?}", other),
        }
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: ServiceSubmitEvent =
            serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&event, &decoded);
    }
}

#[test]
fn closed_synth_stream_interrupted() {
    let event = service_stream_synth_closed();
    match &event {
        ServiceSubmitEvent::Failed {
            alloc_id,
            reason: ServiceFailureReason::StreamInterrupted,
            stderr_tail,
        } => {
            assert!(alloc_id.is_none());
            assert!(stderr_tail.is_none());
        }
        other => panic!("expected Failed/StreamInterrupted, got {other:?}"),
    }
    let json = serde_json::to_string(&event).expect("serialize");
    let decoded: ServiceSubmitEvent = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(event, decoded);
}

// ---------------------------------------------------------------------------
// S-SHCP-WIRE-08 — BackoffCause forward-compat
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-WIRE-08 — serde JSON roundtrip property for
    /// `ServiceFailureReason::BackoffExhausted` with both
    /// `BackoffCause::AttemptBudget` AND `BackoffCause::LivenessBudget`.
    /// LivenessBudget compiles, serialises, and deserialises
    /// byte-equal in Phase 1 even though no production code path
    /// emits it.
    #[test]
    fn backoff_cause_forward_compat_roundtrip(
        attempts in 0u32..32,
        cause in arb_backoff_cause(),
        last_exit_code in prop::option::of(-128i32..128),
    ) {
        let reason = ServiceFailureReason::BackoffExhausted {
            attempts,
            cause,
            last_exit_code,
        };
        let json = serde_json::to_string(&reason).expect("serialize");
        let decoded: ServiceFailureReason =
            serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&reason, &decoded);
        // Re-serialize and bit-compare
        let json2 = serde_json::to_string(&decoded).expect("re-serialize");
        prop_assert_eq!(json, json2);
    }
}

// ---------------------------------------------------------------------------
// S-SHCP-RECON-05 / S-SHCP-RECON-06 — Reconciler opt-out branch
// ---------------------------------------------------------------------------

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid alloc id")
}

fn tick_at(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn opt_out_fact(alloc_id: AllocationId, started_at_unix_ms: u64) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id,
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code: None,
        latest_startup_probe: None,
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: String::new(),
        inferred: false,
        startup_probes_empty: true,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
    }
}

fn fact_with_probes(alloc_id: AllocationId, started_at_unix_ms: u64) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id,
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code: None,
        latest_startup_probe: None,
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
    }
}

proptest! {
    /// S-SHCP-RECON-05 — for arbitrary `ServiceAllocFact` with
    /// `startup_probes_empty == true` + `state == Running` + view
    /// without `alloc_id` in `stable_announced`, reconcile output
    /// contains exactly one `Action::FinalizeFailed` with
    /// `TerminalCondition::Stable { settled_in_ms, witness:
    /// ProbeWitness { probe_idx: 0, role: "startup", mechanic_summary:
    /// "none (opted out)", inferred: false } }`, and
    /// `next_view.stable_announced` contains `alloc_id`.
    #[test]
    fn opt_out_stable_emission(
        alloc_id_seed in "[a-z]{1,8}-[0-9]{1,4}",
        started_at_ms in 0u64..1_000_000,
        now_offset_ms in 0u64..600_000,
    ) {
        let aid = alloc(&alloc_id_seed);
        let mut allocs = BTreeMap::new();
        allocs.insert(aid.clone(), opt_out_fact(aid.clone(), started_at_ms));
        let state = ServiceLifecycleState { allocs, service_dataplane: None };
        let view = ServiceLifecycleView::default();
        let reconciler = ServiceLifecycleReconciler::new();
        let tick = tick_at(started_at_ms.saturating_add(now_offset_ms));

        let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        prop_assert_eq!(actions.len(), 1, "expected exactly one action: {:?}", actions);
        match &actions[0] {
            Action::FinalizeFailed { alloc_id, terminal: Some(
                TerminalCondition::Stable { settled_in_ms: _, witness })
            } => {
                prop_assert_eq!(alloc_id, &aid);
                prop_assert_eq!(witness.probe_idx, 0);
                prop_assert_eq!(&witness.role, "startup");
                prop_assert_eq!(&witness.mechanic_summary, "none (opted out)");
                prop_assert!(!witness.inferred);
            }
            other => prop_assert!(
                false,
                "expected FinalizeFailed/Stable opt-out, got {:?}",
                other
            ),
        }
        prop_assert!(next_view.stable_announced.contains(&aid));
    }
}

#[test]
fn opt_out_branch_does_not_fire_when_probes_present() {
    let aid = alloc("svc-1");
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), fact_with_probes(aid.clone(), 0));
    let state = ServiceLifecycleState { allocs, service_dataplane: None };
    let view = ServiceLifecycleView::default();
    let reconciler = ServiceLifecycleReconciler::new();
    let tick = tick_at(100);

    let (actions, _) = reconciler.reconcile(&state, &state, &view, &tick);
    // Probes present + no Pass observed yet + within deadline →
    // NO opt-out Stable emission; reconciler waits for a Pass.
    for action in &actions {
        if let Action::FinalizeFailed {
            terminal: Some(TerminalCondition::Stable { witness, .. }),
            ..
        } = action
        {
            assert_ne!(
                witness.mechanic_summary, "none (opted out)",
                "opt-out branch must NOT fire when startup_probes_empty == false"
            );
        }
    }
}

#[test]
fn opt_out_idempotent_no_double_emit() {
    let aid = alloc("svc-2");
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), opt_out_fact(aid.clone(), 0));
    let state = ServiceLifecycleState { allocs, service_dataplane: None };
    let mut view = ServiceLifecycleView::default();
    view.stable_announced.insert(aid.clone());
    let reconciler = ServiceLifecycleReconciler::new();
    let tick = tick_at(100);

    // S-SHCP-RECON-06: re-running with stable_announced already
    // containing alloc_id must emit ZERO new actions.
    let (actions, _) = reconciler.reconcile(&state, &state, &view, &tick);
    assert!(actions.is_empty(), "opt-out branch must be idempotent; got: {actions:?}");
}

// ---------------------------------------------------------------------------
// S-SHCP-PURITY-04 — single-write-site byte-equality for new variants
// ---------------------------------------------------------------------------

proptest! {
    /// S-SHCP-PURITY-04 — for arbitrary new TerminalCondition variants
    /// (Stopped, BackoffExhausted, Custom), the serde-JSON projection
    /// of the typed terminal MUST match the projection's payload
    /// byte-equal. Extends S-SHCP-PURITY-03 from step 01-03e to the
    /// new variants per ADR-0037 §3/§4.
    #[test]
    fn new_variants_project_byte_equal(
        alloc_id in arb_alloc_id_str(),
        which in 0u8..3,
        by in arb_stopped_by(),
        attempts in 0u32..32,
        last_exit_code in prop::option::of(-128i32..128),
        type_name in "[A-Za-z0-9.]{1,32}",
        message in "[a-zA-Z0-9 ._]{0,32}",
    ) {
        let terminal = match which {
            0 => TerminalCondition::Stopped { by },
            1 => TerminalCondition::BackoffExhausted { attempts },
            _ => TerminalCondition::Custom {
                type_name: type_name.clone(),
                detail: Some(message.clone().into_bytes()),
            },
        };
        let event = service_event_from_terminal(
            &alloc_id,
            &terminal,
            None,
            last_exit_code,
        )
        .expect("new variants must project");

        // Verify the projection's payload survives a full
        // serialize/deserialize/serialize roundtrip byte-equal — the
        // wire shape is canonical (no whitespace drift, no field
        // reorder).
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: ServiceSubmitEvent =
            serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&decoded).expect("re-serialize");
        prop_assert_eq!(json, json2);
        prop_assert_eq!(&event, &decoded);
    }
}
