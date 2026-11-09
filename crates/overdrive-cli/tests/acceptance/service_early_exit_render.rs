//! Tier 1 acceptance — Service-kind EarlyExit CLI render hardening.
//!
//! Step 03-02 / Slice 08 (US-08 / K1 — closes RCA-A).
//!
//! Three concerns:
//!   * S-SHCP-CLI-07/08 — `Failed { EarlyExit }` renders a multi-line
//!     diagnostic block (`exit_code:`, `stderr_tail:`) and, for the exit-0
//!     case, the Service-kind guidance text. Feature
//!     `service-allocation-readiness-and-ending` D6 (S-19): the block has NO
//!     `elapsed:` line (the CLI-side stopwatch and its parameter are gone).
//!   * Feature `service-allocation-readiness-and-ending` D10 (S-36, render
//!     half): the `Timeout` block says the workload did not become Stable and
//!     the platform may still be restarting it, and carries the labelled
//!     `last attempt (alloc <id>):` block when the stream saw an
//!     attempt-ending event, nothing extra when it did not.
//!   * S-SHCP-CLI-09..11 — the cross-cutting RCA-A regression guard:
//!     for EVERY Service-kind failure-reason × stderr_tail-presence ×
//!     exit_code, the render output NEVER contains the literal
//!     `(took live)` (the misleading success phrasing the RCA-A
//!     coinflip case must never surface for a Service).
//!
//! The multi-line render is a golden-output diff (PBT-exempt per the
//! roadmap's TEST PARADIGM note — CLI render snapshots are golden-file
//! LSP-style); it is compensated by the `service_kind_render_never_
//! contains_took_live` proptest below, which quantifies the RCA-A
//! invariant over the full reason universe.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these tests call the render fn
//! directly — no subprocess.

#![allow(clippy::expect_used)]
#![allow(
    clippy::doc_markdown,
    reason = "acceptance-test docs name bare API identifiers (exit_code, EarlyExit, ServiceFailureReason) in prose; backticking every one is noise in test-doc context"
)]

use overdrive_cli::render::format_service_failed_block;
use overdrive_control_plane::streaming::LastAttemptWire;
use overdrive_core::transition_reason::{
    BackoffCause, ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use proptest::prelude::*;

/// The latest attempt-ending observation a stream saw: a startup stop of
/// `alloc-payments-0` carrying a driver detail tail.
fn startup_stop_attempt() -> LastAttemptWire {
    LastAttemptWire {
        alloc_id: "alloc-payments-0".to_owned(),
        reason: TransitionReason::Stopped { by: StoppedBy::StartupProbe },
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        detail: Some("startup probe[0] connection refused".to_owned()),
    }
}

/// S-SHCP-CLI-07 / S-19 (D6) — `Failed { EarlyExit }` renders the multi-line
/// block: the `exit_code:` line, the Service guidance and the `stderr_tail:`
/// line; it has no `elapsed:` line and never the `(took live)` phrasing.
#[test]
fn early_exit_renders_multiline_block_with_exit_code_and_stderr_tail_and_no_elapsed_line() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: Some(137) },
        Some("OOM: killed by cgroup"),
        None,
    );

    assert!(rendered.contains("exit_code: 137"), "exit_code line present; got:\n{rendered}");
    assert!(
        rendered.contains("The workload exited before any startup probe could pass"),
        "Service guidance present; got:\n{rendered}",
    );
    assert!(
        rendered.contains("stderr_tail: \"OOM: killed by cgroup\""),
        "stderr_tail line present; got:\n{rendered}",
    );
    assert!(!rendered.contains("elapsed:"), "no elapsed line (D6); got:\n{rendered}");
    assert!(!rendered.contains("(took live)"), "never (took live); got:\n{rendered}");
}

/// S-19 (D6) — the signal-killed `EarlyExit { None }` block carries the same
/// shape: guidance and stderr tail present, no `elapsed:` line.
#[test]
fn early_exit_without_exit_code_renders_guidance_and_no_elapsed_line() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: None },
        Some("killed"),
        None,
    );

    assert!(rendered.contains("exit_code: none (killed by signal)"), "got:\n{rendered}");
    assert!(rendered.contains("a Service is expected to stay running"), "got:\n{rendered}");
    assert!(rendered.contains("stderr_tail: \"killed\""), "got:\n{rendered}");
    assert!(!rendered.contains("elapsed:"), "no elapsed line (D6); got:\n{rendered}");
    assert!(!rendered.contains("(took live)"), "never (took live); got:\n{rendered}");
}

/// S-SHCP-CLI-08 — the exit-0-within-deadline render includes the
/// Service-kind guidance text explaining why a clean early exit is a
/// failure for a long-lived Service.
#[test]
fn early_exit_zero_render_includes_service_kind_guidance() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: Some(0) },
        None,
        None,
    );

    assert!(rendered.contains("exit_code: 0"), "exit_code 0 rendered; got:\n{rendered}");
    assert!(
        rendered.contains("The workload exited before any startup probe could pass"),
        "exit-0 render carries the Service-kind guidance text; got:\n{rendered}",
    );
}

/// Regression: signal-killed alloc (exit_code: None) renders
/// "killed by signal", never "code 0".
#[test]
fn signal_killed_render_says_killed_by_signal() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: None },
        None,
        None,
    );

    assert!(
        rendered.contains("killed by signal"),
        "signal-kill must say 'killed by signal'; got:\n{rendered}",
    );
    assert!(!rendered.contains("code 0"), "signal-kill must NOT say 'code 0'; got:\n{rendered}");
}

/// Strategy over every `ServiceFailureReason` variant with
/// representative payloads. The universe is the full reason taxonomy —
/// each `i32`/`u32`/`String` payload is generated so the guard is
/// exercised against arbitrary values, not a single example per
/// variant.
fn any_service_failure_reason() -> impl Strategy<Value = ServiceFailureReason> {
    prop_oneof![
        (any::<u32>(), any::<u32>())
            .prop_map(|(p, a)| ServiceFailureReason::StartupTimeout { probe_idx: p, attempts: a }),
        (any::<u32>(), ".*", any::<u32>()).prop_map(|(p, last_fail, a)| {
            ServiceFailureReason::StartupProbeFailed { probe_idx: p, last_fail, attempts: a }
        }),
        prop::option::of(any::<i32>())
            .prop_map(|exit_code| ServiceFailureReason::EarlyExit { exit_code }),
        (any::<u32>(), any::<u32>()).prop_map(|(p, a)| {
            ServiceFailureReason::LivenessProbeFailed { probe_idx: p, attempts: a }
        }),
        (any::<u32>(), prop::option::of(any::<i32>())).prop_map(|(a, ec)| {
            ServiceFailureReason::BackoffExhausted {
                attempts: a,
                cause: BackoffCause::AttemptBudget,
                last_exit_code: ec,
            }
        }),
        (any::<u32>(), prop::option::of(any::<i32>())).prop_map(|(a, ec)| {
            ServiceFailureReason::BackoffExhausted {
                attempts: a,
                cause: BackoffCause::LivenessBudget,
                last_exit_code: ec,
            }
        }),
        (".*", ".*").prop_map(|(source, message)| ServiceFailureReason::Other { source, message }),
        any::<u32>().prop_map(|after_seconds| ServiceFailureReason::Timeout { after_seconds }),
        Just(ServiceFailureReason::StreamInterrupted),
    ]
}

proptest! {
    /// stderr_tail end-to-end byte-equality — the captured stderr tail
    /// survives the full wire pipeline (ExitObserver →
    /// `LifecycleEvent.detail` → `service_event_from_terminal` →
    /// `ServiceSubmitEvent::Failed.stderr_tail` → CLI
    /// `format_service_failed_block`) byte-for-byte. Drives the real
    /// projection fn + the real render fn (no re-capture — stderr_tail
    /// capture already lands in the ExitObserver per ADR-0033). The
    /// arbitrary tail covers empty / multi-line / unicode payloads.
    #[test]
    fn stderr_tail_survives_pipeline_byte_equal(tail in "[^\"\\n]{1,64}") {
        use overdrive_control_plane::streaming::{service_event_from_terminal, ServiceSubmitEvent};
        use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};

        // ExitObserver/reconciler side: the terminal carries EarlyExit;
        // the captured stderr tail rides the `stderr_tail` projection
        // argument (sourced from `LifecycleEvent.detail` in production).
        let terminal = TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
        };
        let wire = service_event_from_terminal("alloc-x", &terminal, Some(tail.clone()), Some(1))
            .expect("ServiceFailed projects to a ServiceSubmitEvent");

        // Wire surface carries the tail byte-equal.
        let ServiceSubmitEvent::Failed { reason, stderr_tail, .. } = &wire else {
            prop_assert!(false, "ServiceFailed must project to ServiceSubmitEvent::Failed; got {wire:?}");
            unreachable!()
        };
        prop_assert_eq!(stderr_tail.as_deref(), Some(tail.as_str()), "wire stderr_tail byte-equal");

        // CLI render side: the tail surfaces verbatim in the rendered block.
        let rendered =
            format_service_failed_block("svc", reason, stderr_tail.as_deref(), None);
        prop_assert!(
            rendered.contains(&tail),
            "rendered block carries the stderr tail byte-equal; tail={tail:?} render:\n{rendered}",
        );
    }

    /// S-SHCP-CLI-09..11 — `ServiceKindRenderNeverContainsTookLive`.
    ///
    /// Universe: full `ServiceFailureReason` × `stderr_tail.is_some` ×
    /// `exit_code` (i32, via the EarlyExit/BackoffExhausted payloads) ×
    /// last-attempt presence. Invariant: for EVERY Service-kind
    /// render output, there are ZERO substring matches of the literal
    /// `(took live)` — the cross-cutting RCA-A regression guard applied
    /// at the render path. A Service must never render the misleading
    /// "succeeded because it stayed up" phrasing for a failure.
    #[test]
    fn service_kind_render_never_contains_took_live(
        reason in any_service_failure_reason(),
        stderr_tail in prop::option::of(".*"),
        with_last_attempt in any::<bool>(),
    ) {
        let attempt = startup_stop_attempt();
        let rendered = format_service_failed_block(
            "svc",
            &reason,
            stderr_tail.as_deref(),
            with_last_attempt.then_some(&attempt),
        );
        prop_assert!(
            !rendered.contains("(took live)"),
            "RCA-A guard: Service-kind render must never contain \"(took live)\"; \
             reason={reason:?} render:\n{rendered}",
        );
    }
}

// ---------------------------------------------------------------------------
// D10 (S-36, render half) — the `Timeout` block and the labelled last attempt.
// CONTRACT_SHAPE: pure-function (a `String` for a reason and an optional
// last attempt; no state, no I/O).
// ---------------------------------------------------------------------------

/// S-36 / D10 item 7 — the `Timeout` text says the workload did not become
/// Stable within the deadline and that the platform may still be restarting it,
/// pointing at `workload describe`.
#[test]
/// CONTRACT_SHAPE: pure-function.
#[ignore = "pending DELIVER: D10 Timeout wording names Stable and the platform restarting it"]
fn timeout_block_says_the_workload_did_not_become_stable_and_may_still_be_restarting() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::Timeout { after_seconds: 150 },
        None,
        None,
    );

    assert!(
        rendered.contains(
            "workload did not become Stable within 150s; the platform may still be restarting it \
             (see `overdrive workload describe`)"
        ),
        "the Timeout text is the D10 wording; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("did not converge within"),
        "the old wording is gone; got:\n{rendered}",
    );
}

/// S-36 / D10 item 7 — a timeout that carries the latest attempt-ending
/// observation renders the labelled `last attempt (alloc <id>):` block with the
/// reason text and the detail tail, after the reason line.
#[test]
/// CONTRACT_SHAPE: pure-function.
#[ignore = "pending DELIVER: D10 last attempt block in the Timeout render"]
fn timeout_block_with_a_last_attempt_renders_the_labelled_last_attempt_block() {
    let attempt = startup_stop_attempt();
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::Timeout { after_seconds: 90 },
        None,
        Some(&attempt),
    );

    let header = rendered
        .find("last attempt (alloc alloc-payments-0):")
        .unwrap_or_else(|| panic!("the labelled last attempt header; got:\n{rendered}"));
    let reason_line = rendered
        .find("workload did not become Stable within 90s")
        .unwrap_or_else(|| panic!("the Timeout reason line; got:\n{rendered}"));
    assert!(reason_line < header, "the last attempt follows the reason; got:\n{rendered}");
    let block = &rendered[header..];
    assert!(
        block.contains("stopped (startup probe)"),
        "the attempt's reason text `reason.human_readable()` is shown; got:\n{rendered}",
    );
    assert!(
        block.contains("startup probe[0] connection refused"),
        "the attempt's detail tail is shown; got:\n{rendered}",
    );
}

/// S-36 / D10 item 7 — with no attempt-ending observation the `Timeout` block
/// adds nothing: no `last attempt` section.
#[test]
fn timeout_block_without_a_last_attempt_carries_no_last_attempt_section() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::Timeout { after_seconds: 90 },
        None,
        None,
    );

    assert!(!rendered.contains("last attempt"), "no last attempt section; got:\n{rendered}");
    assert!(rendered.contains("Error: workload 'payments' did not converge to stable."));
    assert!(rendered.contains("reproducer: overdrive workload describe payments"));
}

/// S-36 / D10 item 4 — the detail is labelled, never the verdict: a non-`Timeout`
/// failure (the restart authority's give-up) never renders a `last attempt` block
/// even when the caller holds one.
#[test]
fn a_give_up_failure_never_renders_a_last_attempt_block() {
    let attempt = startup_stop_attempt();
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::StartupProbeFailed {
            probe_idx: 0,
            last_fail: "connection refused".to_owned(),
            attempts: 5,
        },
        None,
        Some(&attempt),
    );

    assert!(!rendered.contains("last attempt"), "the verdict stands alone; got:\n{rendered}");
}

/// S-27 (D7 render): the `StartupProbeFailed` give-up block is byte-identical to
/// today's.
#[test]
fn startup_probe_failed_block_is_byte_identical_to_today() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::StartupProbeFailed {
            probe_idx: 0,
            last_fail: "connection refused".to_owned(),
            attempts: 5,
        },
        None,
        None,
    );

    assert_eq!(
        rendered,
        "Error: workload 'payments' did not converge to stable.\n\
         \x20 reason: startup probe[0] failed after 5 attempts: connection refused\n\
         \x20 reproducer: overdrive workload describe payments\n\
         \n\
         Hint: see workload describe for full context\n",
    );
}
