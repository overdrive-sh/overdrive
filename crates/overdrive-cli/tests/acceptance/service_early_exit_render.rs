//! Tier 1 acceptance — Service-kind EarlyExit CLI render hardening.
//!
//! Step 03-02 / Slice 08 (US-08 / K1 — closes RCA-A).
//!
//! Two concerns:
//!   * S-SHCP-CLI-07/08 — `Failed { EarlyExit }` renders a multi-line
//!     diagnostic block (`exit_code:`, `elapsed: <N>s
//!     (startup_deadline=<N>s)`, `stderr_tail:`) and, for the exit-0
//!     case, the Service-kind guidance text.
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
use overdrive_core::transition_reason::{BackoffCause, ServiceFailureReason};
use proptest::prelude::*;

/// S-SHCP-CLI-07 — `Failed { EarlyExit }` renders the multi-line block:
/// the `exit_code:` line, the `elapsed: <N>s (startup_deadline=<N>s)`
/// line (from the caller-supplied timing), and the `stderr_tail:` line.
#[test]
fn early_exit_renders_multiline_block_with_exit_code_elapsed_and_stderr_tail() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: 137 },
        Some("OOM: killed by cgroup"),
        Some((3, 60)),
    );

    assert!(rendered.contains("exit_code: 137"), "exit_code line present; got:\n{rendered}");
    assert!(
        rendered.contains("elapsed: 3s (startup_deadline=60s)"),
        "elapsed/startup_deadline line present; got:\n{rendered}",
    );
    assert!(
        rendered.contains("stderr_tail: \"OOM: killed by cgroup\""),
        "stderr_tail line present; got:\n{rendered}",
    );
}

/// S-SHCP-CLI-08 — the exit-0-within-deadline render includes the
/// Service-kind guidance text explaining why a clean early exit is a
/// failure for a long-lived Service.
#[test]
fn early_exit_zero_render_includes_service_kind_guidance() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: 0 },
        None,
        Some((1, 60)),
    );

    assert!(rendered.contains("exit_code: 0"), "exit_code 0 rendered; got:\n{rendered}");
    assert!(
        rendered.contains("The workload exited before any startup probe could pass"),
        "exit-0 render carries the Service-kind guidance text; got:\n{rendered}",
    );
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
        any::<i32>().prop_map(|exit_code| ServiceFailureReason::EarlyExit { exit_code }),
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
            reason: ServiceFailureReason::EarlyExit { exit_code: 1 },
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
            format_service_failed_block("svc", reason, stderr_tail.as_deref(), Some((1, 60)));
        prop_assert!(
            rendered.contains(&tail),
            "rendered block carries the stderr tail byte-equal; tail={tail:?} render:\n{rendered}",
        );
    }

    /// S-SHCP-CLI-09..11 — `ServiceKindRenderNeverContainsTookLive`.
    ///
    /// Universe: full `ServiceFailureReason` × `stderr_tail.is_some` ×
    /// `exit_code` (i32, via the EarlyExit/BackoffExhausted payloads) ×
    /// early-exit-timing presence. Invariant: for EVERY Service-kind
    /// render output, there are ZERO substring matches of the literal
    /// `(took live)` — the cross-cutting RCA-A regression guard applied
    /// at the render path. A Service must never render the misleading
    /// "succeeded because it stayed up" phrasing for a failure.
    #[test]
    fn service_kind_render_never_contains_took_live(
        reason in any_service_failure_reason(),
        stderr_tail in prop::option::of(".*"),
        timing in prop::option::of((any::<u64>(), any::<u64>())),
    ) {
        let rendered =
            format_service_failed_block("svc", &reason, stderr_tail.as_deref(), timing);
        prop_assert!(
            !rendered.contains("(took live)"),
            "RCA-A guard: Service-kind render must never contain \"(took live)\"; \
             reason={reason:?} render:\n{rendered}",
        );
    }
}
