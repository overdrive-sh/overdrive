//! Feature `service-allocation-readiness-and-ending`, decisions D7 (item 5) and D9
//! (item 6), obligation S-26 — which `Stopped { by }` terminals end the Service
//! deploy stream.
//!
//! A startup-probe stop and a liveness-probe stop are restartable: the platform
//! restarts the instance under its single budget, so neither is a verdict and
//! neither projects to a stream line. Every other `Stopped { by }` projects as
//! before. The CLI must never reach "stopped by an unrecognised initiator" with exit
//! 0 for a startup failure.
//!
//! CONTRACT_SHAPE: pure-function (a `ServiceSubmitEvent` option for a terminal; no
//! state, no I/O).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_control_plane::streaming::{ServiceSubmitEvent, service_event_from_terminal};
use overdrive_core::transition_reason::{
    BackoffCause, ProbeWitness, ServiceFailureReason, StoppedBy, TerminalCondition,
};

const ALLOC: &str = "alloc-web-0";

/// Every `StoppedBy` variant the platform authors, with whether it ends the stream.
const STOPPED_BY: [(StoppedBy, bool); 7] = [
    (StoppedBy::Operator, true),
    (StoppedBy::Reconciler, true),
    (StoppedBy::Process, true),
    (StoppedBy::SystemGc, true),
    (StoppedBy::PlatformReclaimed, true),
    (StoppedBy::LivenessProbe, false),
    (StoppedBy::StartupProbe, false),
];

/// S-26 — `Stopped { StartupProbe }` and `Stopped { LivenessProbe }` project to no
/// stream line: the stream continues.
#[test]
/// CONTRACT_SHAPE: pure-function.
#[ignore = "pending DELIVER: D7 and D9 a restartable probe stop is not a stream terminal"]
fn a_restartable_probe_stop_projects_to_no_stream_line() {
    for (by, ends_stream) in STOPPED_BY {
        if ends_stream {
            continue;
        }
        let line =
            service_event_from_terminal(ALLOC, &TerminalCondition::Stopped { by }, None, None);
        assert_eq!(line, None, "Stopped {{ by: {by:?} }} must not end the stream");
    }
}

/// S-26 unaffected control — every other stop still projects to the same `Stopped`
/// line, naming its initiator and allocation.
#[test]
fn every_other_stop_projects_to_a_stopped_line_naming_its_initiator() {
    for (by, ends_stream) in STOPPED_BY {
        if !ends_stream {
            continue;
        }
        let line =
            service_event_from_terminal(ALLOC, &TerminalCondition::Stopped { by }, None, None);
        assert_eq!(
            line,
            Some(ServiceSubmitEvent::Stopped { alloc_id: ALLOC.to_owned(), by }),
            "Stopped {{ by: {by:?} }} still ends the stream with its initiator",
        );
    }
}

/// S-26 unaffected control — the verdict terminals (`Stable`, the four give-ups) still
/// project to a line.
#[test]
fn the_verdict_terminals_still_project_to_a_stream_line() {
    let witness = ProbeWitness {
        probe_idx: 0,
        role: "startup".to_owned(),
        mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
        inferred: false,
    };
    let verdicts = [
        TerminalCondition::Stable { settled_in_ms: 700, witness },
        TerminalCondition::BackoffExhausted { attempts: 5 },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: "connection refused".to_owned(),
                attempts: 5,
            },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts: 5 },
        },
    ];
    for terminal in verdicts {
        assert!(
            service_event_from_terminal(ALLOC, &terminal, None, None).is_some(),
            "{terminal:?} is a verdict and ends the stream",
        );
    }
    // The `BackoffExhausted` projection keeps its attempt-budget cause.
    let Some(ServiceSubmitEvent::Failed { reason, .. }) = service_event_from_terminal(
        ALLOC,
        &TerminalCondition::BackoffExhausted { attempts: 5 },
        None,
        Some(1),
    ) else {
        panic!("BackoffExhausted projects to Failed");
    };
    assert_eq!(
        reason,
        ServiceFailureReason::BackoffExhausted {
            attempts: 5,
            cause: BackoffCause::AttemptBudget,
            last_exit_code: Some(1),
        },
    );
}
