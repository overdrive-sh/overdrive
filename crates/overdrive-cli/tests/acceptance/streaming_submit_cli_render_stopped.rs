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

use overdrive_core::transition_reason::StoppedBy;

#[test]
fn format_stopped_summary_for_operator_names_initiator_and_job() {
    let rendered = overdrive_cli::render::format_stopped_summary("payments", StoppedBy::Operator);

    assert!(rendered.contains("payments"), "rendered must mention the job name; got:\n{rendered}");
    assert!(
        rendered.contains("operator"),
        "rendered must name the operator initiator; got:\n{rendered}",
    );
}

#[test]
fn format_stopped_summary_for_reconciler_names_initiator_and_job() {
    let rendered = overdrive_cli::render::format_stopped_summary("payments", StoppedBy::Reconciler);

    assert!(rendered.contains("payments"), "rendered must mention the job name; got:\n{rendered}");
    assert!(
        rendered.contains("reconciler"),
        "rendered must name the reconciler initiator; got:\n{rendered}",
    );
}

#[test]
fn format_stopped_summary_for_process_names_initiator_and_job() {
    let rendered = overdrive_cli::render::format_stopped_summary("payments", StoppedBy::Process);

    assert!(rendered.contains("payments"), "rendered must mention the job name; got:\n{rendered}");
    assert!(
        rendered.contains("process"),
        "rendered must name the process initiator; got:\n{rendered}",
    );
}
