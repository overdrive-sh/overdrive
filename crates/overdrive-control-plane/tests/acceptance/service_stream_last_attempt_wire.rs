//! Feature `service-allocation-readiness-and-ending`, decision D10 item 4 and D7
//! (format), obligations S-36 (wire half) and S-25 (OpenAPI half) — the additive
//! `last_attempt` field of `ServiceSubmitEvent::Failed`, its new `LastAttemptWire`
//! struct, and the `startup_probe` initiator, in the generated and the checked-in
//! `api/openapi.yaml`.
//!
//! CONTRACT_SHAPE: pure-function (serde and schema derivation over values; the only
//! file read is the checked-in spec).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use overdrive_control_plane::openapi::generate_yaml;
use overdrive_control_plane::streaming::{LastAttemptWire, ServiceSubmitEvent};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};

const fn timeout_with(last_attempt: Option<LastAttemptWire>) -> ServiceSubmitEvent {
    ServiceSubmitEvent::Failed {
        alloc_id: None,
        reason: ServiceFailureReason::Timeout { after_seconds: 90 },
        stderr_tail: None,
        last_attempt,
    }
}

fn startup_stop_attempt() -> LastAttemptWire {
    LastAttemptWire {
        alloc_id: "alloc-web-0".to_owned(),
        reason: TransitionReason::Stopped { by: StoppedBy::StartupProbe },
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        detail: Some("startup probe[0] connection refused".to_owned()),
    }
}

/// S-36 — a `Failed` line written before the field existed (no `last_attempt`) still
/// decodes, with `last_attempt: None`.
#[test]
fn a_failed_line_without_last_attempt_still_decodes() {
    let line =
        r#"{"kind":"failed","data":{"reason":{"reason":"timeout","data":{"after_seconds":90}}}}"#;

    let event: ServiceSubmitEvent = serde_json::from_str(line).expect("an older Failed line");

    assert_eq!(event, timeout_with(None));
}

/// S-36 — `last_attempt: None` is omitted on the wire (the field is additive), and a
/// present one carries its four labelled parts and round-trips.
#[test]
fn a_last_attempt_is_omitted_when_absent_and_round_trips_when_present() {
    let absent = serde_json::to_value(timeout_with(None)).expect("serialise");
    assert!(absent["data"].get("last_attempt").is_none(), "omitted when None; got {absent}");

    let present = timeout_with(Some(startup_stop_attempt()));
    let value = serde_json::to_value(&present).expect("serialise");
    let attempt = &value["data"]["last_attempt"];
    assert_eq!(attempt["alloc_id"], "alloc-web-0");
    assert_eq!(attempt["detail"], "startup probe[0] connection refused");
    assert!(attempt.get("reason").is_some() && attempt.get("terminal").is_some(), "{attempt}");
    let decoded: ServiceSubmitEvent = serde_json::from_value(value).expect("round trip");
    assert_eq!(decoded, present);
}

/// S-25 / S-36 — the schema derived from the types names the new wire struct, the new
/// `Failed` field and the `startup_probe` initiator.
#[test]
fn the_derived_schema_names_the_last_attempt_and_the_startup_probe_initiator() {
    let generated = generate_yaml().expect("generate_yaml");

    assert!(generated.contains("LastAttemptWire:"), "LastAttemptWire schema present");
    assert!(generated.contains("last_attempt:"), "the Failed.last_attempt property present");
    assert!(generated.contains("- startup_probe"), "StoppedBy lists startup_probe");
}
