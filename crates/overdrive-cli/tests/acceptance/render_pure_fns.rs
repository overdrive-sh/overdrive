//! Unit tests for pure render helper functions that were flagged as
//! missed mutations: `format_human_duration`, `derive_job_verdict`,
//! and the spec-digest branches of `alloc_status_kind_aware`.

#![allow(clippy::expect_used)]

use std::time::Duration;

use overdrive_cli::render::{
    JobVerdict, alloc_status_kind_aware, derive_job_verdict, format_human_duration,
    is_backoff_exhausted,
};
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, ResourcesBody, RestartBudget,
};
use overdrive_core::aggregate::WorkloadKind;

// ---------------------------------------------------------------------------
// format_human_duration — four-branch coverage
// ---------------------------------------------------------------------------

#[test]
fn human_duration_zero_renders_sub_1ms() {
    assert_eq!(format_human_duration(Duration::ZERO), "<1ms");
}

#[test]
fn human_duration_500ms() {
    assert_eq!(format_human_duration(Duration::from_millis(500)), "500ms");
}

#[test]
fn human_duration_boundary_999ms() {
    assert_eq!(format_human_duration(Duration::from_millis(999)), "999ms");
}

#[test]
fn human_duration_boundary_1000ms_enters_seconds_branch() {
    assert_eq!(format_human_duration(Duration::from_secs(1)), "1.0s");
}

#[test]
fn human_duration_sub_minute_with_tenths() {
    assert_eq!(format_human_duration(Duration::from_millis(5_500)), "5.5s");
}

#[test]
fn human_duration_boundary_59s() {
    assert_eq!(format_human_duration(Duration::from_secs(59)), "59.0s");
}

#[test]
fn human_duration_boundary_60s_enters_minutes_branch() {
    assert_eq!(format_human_duration(Duration::from_secs(60)), "1m0s");
}

#[test]
fn human_duration_minutes_and_seconds() {
    assert_eq!(format_human_duration(Duration::from_secs(90)), "1m30s");
}

#[test]
fn human_duration_tenths_truncation() {
    // 2345ms → 2s with 345ms remainder → tenths = 345/100 = 3
    assert_eq!(format_human_duration(Duration::from_millis(2_345)), "2.3s");
}

// ---------------------------------------------------------------------------
// derive_job_verdict — conjunction coverage
// ---------------------------------------------------------------------------

fn minimal_row(state: AllocStateWire, exit_code: Option<i32>) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: "alloc-test-0".to_string(),
        workload_id: "test".to_string(),
        node_id: "node-a".to_string(),
        state,
        reason: None,
        resources: ResourcesBody { cpu_milli: 100, memory_bytes: 1024 },
        started_at: None,
        exit_code,
        last_transition: None,
        error: None,
    }
}

#[test]
fn verdict_terminated_zero_is_succeeded() {
    let rows = vec![minimal_row(AllocStateWire::Terminated, Some(0))];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Succeeded);
}

#[test]
fn verdict_terminated_nonzero_is_failed() {
    let rows = vec![minimal_row(AllocStateWire::Terminated, Some(1))];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Failed);
}

#[test]
fn verdict_running_is_in_progress() {
    let rows = vec![minimal_row(AllocStateWire::Running, None)];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::InProgress);
}

#[test]
fn verdict_empty_rows_is_in_progress_not_failed() {
    let rows: Vec<AllocStatusRowBody> = vec![];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::InProgress);
}

#[test]
fn verdict_terminated_nonzero_and_running_sibling_is_succeeded_not_in_progress() {
    // Terminated(0) takes priority over Running.
    let rows = vec![
        minimal_row(AllocStateWire::Terminated, Some(0)),
        minimal_row(AllocStateWire::Running, None),
    ];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Succeeded);
}

// ---------------------------------------------------------------------------
// alloc_status_kind_aware — spec_digest conditional rendering
// ---------------------------------------------------------------------------

fn status_fixture(kind: WorkloadKind, spec_digest: &str) -> AllocStatusResponse {
    AllocStatusResponse {
        workload_id: Some("test-wl".to_string()),
        spec_digest: Some(spec_digest.to_string()),
        replicas_desired: 1,
        replicas_running: 0,
        rows: vec![minimal_row(AllocStateWire::Pending, None)],
        restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
        kind: Some(kind),
        vip: None,
    }
}

#[test]
fn service_branch_renders_spec_digest_when_present() {
    let out = status_fixture(WorkloadKind::Service, "abcd1234");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        rendered.contains("Spec digest: abcd1234"),
        "Service with non-empty spec_digest must render it; got:\n{rendered}",
    );
}

#[test]
fn service_branch_omits_spec_digest_when_empty() {
    let out = status_fixture(WorkloadKind::Service, "");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        !rendered.contains("Spec digest:"),
        "Service with empty spec_digest must omit the line; got:\n{rendered}",
    );
}

#[test]
fn schedule_branch_renders_spec_digest_when_present() {
    let out = status_fixture(WorkloadKind::Schedule, "abcd1234");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        rendered.contains("Spec digest: abcd1234"),
        "Schedule with non-empty spec_digest must render it; got:\n{rendered}",
    );
}

#[test]
fn schedule_branch_omits_spec_digest_when_empty() {
    let out = status_fixture(WorkloadKind::Schedule, "");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        !rendered.contains("Spec digest:"),
        "Schedule with empty spec_digest must omit the line; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// is_backoff_exhausted — conjunction + boundary coverage
// ---------------------------------------------------------------------------

#[test]
fn backoff_exhausted_when_attempts_equal_max_and_max_gt_1() {
    assert!(is_backoff_exhausted(3, 3));
}

#[test]
fn backoff_exhausted_when_attempts_exceed_max() {
    assert!(is_backoff_exhausted(5, 3));
}

#[test]
fn not_exhausted_when_max_is_one() {
    assert!(!is_backoff_exhausted(1, 1));
}

#[test]
fn not_exhausted_when_attempts_below_max() {
    assert!(!is_backoff_exhausted(1, 3));
}

#[test]
fn not_exhausted_when_max_is_zero() {
    assert!(!is_backoff_exhausted(0, 0));
}

#[test]
fn conjunction_both_branches_matter() {
    // attempts >= max_attempts is true, but max_attempts <= 1 → false
    assert!(!is_backoff_exhausted(1, 1));
    // max_attempts > 1 is true, but attempts < max_attempts → false
    assert!(!is_backoff_exhausted(1, 2));
    // both true → true
    assert!(is_backoff_exhausted(2, 2));
}

// ---------------------------------------------------------------------------
// format_service_stable_summary + format_service_failed_block — per-variant
// reason-text coverage.
//
// These pure render helpers were promoted in step 01-03e3 (ADR-0056 /
// ADR-0059) but their per-variant `reason_text` branches and the
// stable-summary body were left without unit coverage when the legacy
// `service_early_exit_render` suite was deleted in that same step.
// The `--diff origin/main` mutation gate for step 02-03 surfaced the
// gap (15 missed match-arm / return-value mutants in
// `format_service_failed_block` / `format_service_stable_summary`).
// Per CLAUDE.md § "Clippy warnings ... are NOT deferrals — they are
// in-scope fixes", the killing tests land here (the designated home
// for missed-mutation pure-render-helper coverage per this module's
// docstring).
// ---------------------------------------------------------------------------

use overdrive_cli::render::{format_service_failed_block, format_service_stable_summary};
use overdrive_core::transition_reason::{BackoffCause, ProbeWitness, ServiceFailureReason};

#[test]
fn stable_summary_names_settled_ms_and_witness_role_idx_mechanic() {
    let witness = ProbeWitness {
        probe_idx: 2,
        role: "startup".to_string(),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: false,
    };
    let rendered = format_service_stable_summary("payments", 1234, &witness);
    assert!(rendered.contains("payments"), "names workload; got:\n{rendered}");
    assert!(rendered.contains("1234ms"), "names settled-in ms; got:\n{rendered}");
    assert!(rendered.contains("startup"), "names witness role; got:\n{rendered}");
    assert!(rendered.contains("probe[2]"), "names witness probe_idx; got:\n{rendered}");
    assert!(rendered.contains("tcp 127.0.0.1:8080"), "names mechanic; got:\n{rendered}");
    assert!(!rendered.contains("inferred"), "non-inferred witness omits marker; got:\n{rendered}");
}

#[test]
fn stable_summary_marks_inferred_witness() {
    let witness = ProbeWitness {
        probe_idx: 0,
        role: "startup".to_string(),
        mechanic_summary: "tcp 0.0.0.0:80".to_string(),
        inferred: true,
    };
    let rendered = format_service_stable_summary("web", 50, &witness);
    assert!(rendered.contains("inferred"), "inferred witness marks the witness; got:\n{rendered}");
}

#[test]
fn failed_block_renders_header_reproducer_and_hint() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: 7 },
        None,
    );
    assert!(
        rendered.contains("workload 'payments' did not converge to stable"),
        "header names workload; got:\n{rendered}",
    );
    assert!(
        rendered.contains("reproducer: overdrive alloc status --job payments"),
        "reproducer names the workload; got:\n{rendered}",
    );
    assert!(rendered.contains("Hint:"), "hint section present; got:\n{rendered}");
}

#[test]
fn failed_block_includes_last_event_when_stderr_tail_present() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: 7 },
        Some("segfault at 0xdead"),
    );
    assert!(
        rendered.contains("last-event: segfault at 0xdead"),
        "stderr tail rendered as last-event; got:\n{rendered}",
    );
}

#[test]
fn failed_block_omits_last_event_when_stderr_tail_empty() {
    let rendered = format_service_failed_block(
        "payments",
        &ServiceFailureReason::EarlyExit { exit_code: 7 },
        Some(""),
    );
    assert!(
        !rendered.contains("last-event:"),
        "empty stderr tail omits last-event line; got:\n{rendered}",
    );
}

/// Each `ServiceFailureReason` variant's `reason_text` branch renders a
/// distinct, variant-specific substring. Parametrised across every
/// non-exhaustive variant so a deleted match arm (which would fall
/// through to a sibling or the catch-all) flips at least one assertion.
#[test]
fn failed_block_reason_text_is_variant_specific() {
    let cases: Vec<(ServiceFailureReason, &str)> = vec![
        (
            ServiceFailureReason::StartupTimeout { probe_idx: 1, attempts: 30 },
            "startup probe[1] timed out after 30 attempts",
        ),
        (
            ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: "connection refused".to_string(),
                attempts: 5,
            },
            "startup probe[0] failed after 5 attempts: connection refused",
        ),
        (ServiceFailureReason::EarlyExit { exit_code: 137 }, "workload exited early with code 137"),
        (
            ServiceFailureReason::LivenessProbeFailed { probe_idx: 2, attempts: 3 },
            "liveness probe[2] failed after 3 attempts",
        ),
        (
            ServiceFailureReason::BackoffExhausted {
                attempts: 5,
                cause: BackoffCause::AttemptBudget,
                last_exit_code: Some(1),
            },
            "backoff exhausted after 5 attempts (attempt budget) (last exit code 1)",
        ),
        (
            ServiceFailureReason::BackoffExhausted {
                attempts: 4,
                cause: BackoffCause::LivenessBudget,
                last_exit_code: None,
            },
            "backoff exhausted after 4 attempts (liveness budget)",
        ),
        (
            ServiceFailureReason::Other {
                source: "custom".to_string(),
                message: "boom".to_string(),
            },
            "custom failure 'custom': boom",
        ),
        (
            ServiceFailureReason::Timeout { after_seconds: 90 },
            "workload did not converge within 90s",
        ),
        (
            ServiceFailureReason::StreamInterrupted,
            "server-side stream interrupted before convergence",
        ),
    ];

    for (reason, expected) in cases {
        let rendered = format_service_failed_block("svc", &reason, None);
        assert!(
            rendered.contains(expected),
            "reason {reason:?} must render '{expected}'; got:\n{rendered}",
        );
    }
}
