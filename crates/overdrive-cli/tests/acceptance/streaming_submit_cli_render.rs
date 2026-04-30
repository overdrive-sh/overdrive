//! S-CLI-04 — `overdrive_cli::render::format_failed_block` Failed block render.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria. Pure-function fixture — no I/O, no
//! server. The streaming submit handler reaches a `ConvergedFailed`
//! event whose `reason` is the cause-class
//! `TransitionReason::ExecBinaryNotFound { path: "/no/such" }` with
//! `error = "stat /no/such: no such file or directory"`. The renderer
//! must produce an `Error:` block naming:
//!
//!  * the literal `Error: job '<name>' did not converge to running.`
//!  * a `reason:` line rendered via `TransitionReason::human_readable()`
//!    (per ADR-0033 §4 amendment 2026-04-30) — `binary not found: /no/such`
//!  * a `last-event:` line carrying the verbatim driver text
//!  * a `reproducer:` line referencing `alloc status --job <name>`
//!  * a `Hint:` line — for `ExecBinaryNotFound` / `ExecPermissionDenied`
//!    the hint is the `fix the spec's exec.command path and re-run` form
//!    per the criteria mapping table.
//!
//! The output is the operator-visible Error block produced when
//! `commands::job::submit_streaming` observes a `ConvergedFailed`
//! terminal event; the CLI maps the same event to exit code 1.

use overdrive_control_plane::api::TerminalReason;
use overdrive_core::TransitionReason;

#[test]
fn failed_block_for_exec_binary_not_found_contains_required_lines() {
    let reason = TransitionReason::ExecBinaryNotFound { path: "/no/such".to_owned() };
    let terminal_reason = TerminalReason::BackoffExhausted { attempts: 5, cause: reason.clone() };
    let driver_error = "stat /no/such: no such file or directory";

    let rendered = overdrive_cli::render::format_failed_block(
        "payments",
        Some(&reason),
        Some(driver_error),
        &terminal_reason,
    );

    // Header line — names the job and the failure mode.
    assert!(
        rendered.contains("Error: job 'payments' did not converge to running."),
        "rendered must contain the Error header line; got:\n{rendered}",
    );

    // `reason:` line — rendered via `TransitionReason::human_readable()`.
    // For `ExecBinaryNotFound { path }` the rendering is `binary not found: <path>`.
    assert!(
        rendered.contains("reason:") && rendered.contains("binary not found: /no/such"),
        "rendered must contain a `reason:` line carrying the cause-class \
         human_readable rendering; got:\n{rendered}",
    );

    // `last-event:` line — verbatim driver text.
    assert!(
        rendered.contains("last-event:") && rendered.contains(driver_error),
        "rendered must contain a `last-event:` line carrying the verbatim \
         driver text; got:\n{rendered}",
    );

    // `reproducer:` line — points at `alloc status --job <name>`.
    assert!(
        rendered.contains("reproducer:")
            && rendered.contains("overdrive alloc status --job payments"),
        "rendered must contain a `reproducer:` line referencing the alloc \
         status command; got:\n{rendered}",
    );

    // `Hint:` line — for `ExecBinaryNotFound` / `ExecPermissionDenied` the
    // criteria-specified hint is "fix the spec's exec.command path and
    // re-run". Case-insensitive substring match on the load-bearing tokens
    // so minor wording changes do not invalidate.
    let lower = rendered.to_lowercase();
    assert!(
        lower.contains("hint:") && (lower.contains("exec.command") || lower.contains("spec")),
        "rendered must contain a `Hint:` line referencing the spec's \
         exec.command path; got:\n{rendered}",
    );
}

#[test]
fn failed_block_for_exec_permission_denied_uses_path_fix_hint() {
    let reason =
        TransitionReason::ExecPermissionDenied { path: "/usr/local/bin/payments".to_owned() };
    let terminal_reason = TerminalReason::BackoffExhausted { attempts: 5, cause: reason.clone() };

    let rendered = overdrive_cli::render::format_failed_block(
        "payments",
        Some(&reason),
        Some("permission denied: /usr/local/bin/payments"),
        &terminal_reason,
    );

    let lower = rendered.to_lowercase();
    assert!(
        lower.contains("hint:") && (lower.contains("exec.command") || lower.contains("spec")),
        "ExecPermissionDenied must share the `fix the spec's exec.command path` \
         hint per the criteria mapping; got:\n{rendered}",
    );
    assert!(
        rendered.contains("permission denied: /usr/local/bin/payments"),
        "rendered must contain the human_readable rendering for \
         ExecPermissionDenied; got:\n{rendered}",
    );
}

#[test]
fn failed_block_for_timeout_cites_streaming_cap_and_neutral_hint() {
    let terminal_reason = TerminalReason::Timeout { after_seconds: 60 };

    let rendered = overdrive_cli::render::format_failed_block(
        "long-running-batch",
        None,
        Some("did not converge in 60s"),
        &terminal_reason,
    );

    assert!(
        rendered.contains("did not converge"),
        "rendered must surface the timeout cause text; got:\n{rendered}",
    );
    // Neutral hint — not the `exec.command` form. The criteria says
    // `terminal_reason::Timeout` gets a hint about the server cap or
    // `--detach`. Match the two operative tokens loosely.
    let lower = rendered.to_lowercase();
    assert!(lower.contains("hint:"), "rendered must contain a `Hint:` line; got:\n{rendered}");
}

#[test]
fn failed_block_renders_without_reason_falls_back_to_terminal_reason_cause() {
    // `reason` is None — only `terminal_reason` carries a cause.
    let inner = TransitionReason::ExecBinaryNotFound { path: "/no/such".to_owned() };
    let terminal_reason = TerminalReason::BackoffExhausted { attempts: 5, cause: inner };

    let rendered = overdrive_cli::render::format_failed_block(
        "payments",
        None,
        Some("stat /no/such: no such file or directory"),
        &terminal_reason,
    );

    // The renderer must still produce a `reason:` line — this time
    // derived from the terminal_reason's inner cause.
    assert!(
        rendered.contains("binary not found: /no/such"),
        "renderer must derive `reason:` from the terminal_reason's inner \
         cause when standalone reason is absent; got:\n{rendered}",
    );
    // Reproducer present regardless of which cause source was used.
    assert!(
        rendered.contains("overdrive alloc status --job payments"),
        "reproducer line must always be present; got:\n{rendered}",
    );
}
