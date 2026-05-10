//! Render-function tests for `format_stopped_summary` — the
//! operator-facing exit-0 success render fired when the streaming
//! consumer observes `SubmitEvent::ConvergedStopped`.
//!
//! Pure-function fixture — no I/O, no server. One test per
//! `StoppedBy` variant; each asserts the rendered string contains the
//! job name and the initiator label.
//!
//! Companion to the streaming-consumer regression in
//! `tests/integration/streaming_submit_converged_stopped.rs` (the
//! end-to-end shape that closes the bug). RCA:
//! `docs/feature/fix-converged-stopped-cli-arm/deliver/rca.md`.

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::transition_reason::StoppedBy;

#[test]
fn format_stopped_summary_for_operator_names_initiator_and_workload() {
    // Slice 04 — kind-aware. Service kind reuses the rendered Service
    // vocabulary; the existing "operator" initiator label survives.
    let rendered = overdrive_cli::render::format_stopped_summary(
        "payments",
        WorkloadKind::Service,
        StoppedBy::Operator,
    );

    assert!(
        rendered.contains("payments"),
        "rendered must mention the workload name; got:\n{rendered}"
    );
    assert!(
        rendered.contains("operator"),
        "rendered must name the operator initiator; got:\n{rendered}",
    );
}

#[test]
fn format_stopped_summary_for_reconciler_names_initiator_and_workload() {
    let rendered = overdrive_cli::render::format_stopped_summary(
        "payments",
        WorkloadKind::Service,
        StoppedBy::Reconciler,
    );

    assert!(
        rendered.contains("payments"),
        "rendered must mention the workload name; got:\n{rendered}"
    );
    assert!(
        rendered.contains("reconciler"),
        "rendered must name the reconciler initiator; got:\n{rendered}",
    );
}

#[test]
fn format_stopped_summary_for_process_names_initiator_and_workload() {
    let rendered = overdrive_cli::render::format_stopped_summary(
        "payments",
        WorkloadKind::Service,
        StoppedBy::Process,
    );

    assert!(
        rendered.contains("payments"),
        "rendered must mention the workload name; got:\n{rendered}"
    );
    assert!(
        rendered.contains("process"),
        "rendered must name the process initiator; got:\n{rendered}",
    );
}

// S-04-04 — `format_stopped_summary` is kind-aware: Service / Job /
// Schedule branches each pick the right vocabulary. The `WorkloadKind`
// argument is the discriminator; downstream callers (post-WorkloadSpec
// wiring in slice 02+) pass the kind from the parsed spec.

#[test]
fn s_04_04_format_stopped_summary_for_service_uses_service_vocabulary() {
    let rendered = overdrive_cli::render::format_stopped_summary(
        "payments",
        WorkloadKind::Service,
        StoppedBy::Operator,
    );

    assert!(
        rendered.contains("Service 'payments' was stopped by"),
        "S-04-04 Service branch must render `Service '<name>' was stopped by ...`; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Job 'payments'"),
        "S-04-04 Service branch must NOT render Job vocabulary; got:\n{rendered}",
    );
}

#[test]
fn s_04_04_format_stopped_summary_for_job_uses_job_vocabulary() {
    let rendered = overdrive_cli::render::format_stopped_summary(
        "coinflip",
        WorkloadKind::Job,
        StoppedBy::Process,
    );

    assert!(
        rendered.contains("Job 'coinflip' was stopped by"),
        "S-04-04 Job branch must render `Job '<name>' was stopped by ...`; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Service 'coinflip'"),
        "S-04-04 Job branch must NOT render Service vocabulary; got:\n{rendered}",
    );
}

#[test]
fn s_04_04_format_stopped_summary_for_schedule_uses_deregistered_vocabulary() {
    let rendered = overdrive_cli::render::format_stopped_summary(
        "nightly-backup",
        WorkloadKind::Schedule,
        StoppedBy::Operator,
    );

    // Schedule kind deregisters; the vocabulary is intentionally
    // distinct from Service/Job stop semantics.
    assert!(
        rendered.contains("Schedule 'nightly-backup' was deregistered by"),
        "S-04-04 Schedule branch must render `Schedule '<name>' was deregistered by ...`; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Job 'nightly-backup'"),
        "S-04-04 Schedule branch must NOT render Job vocabulary; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Service 'nightly-backup'"),
        "S-04-04 Schedule branch must NOT render Service vocabulary; got:\n{rendered}",
    );
}
