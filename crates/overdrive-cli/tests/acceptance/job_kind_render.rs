//! Slice 02 of `workload-kind-discriminator` — Job-kind render
//! function unit tests.
//!
//! Per ADR-0047 §3 [D2] / [D7]: Job-kind workloads render via
//! `format_job_succeeded_summary`, `format_job_failed_summary`, and
//! `format_job_attempt_failed`. These functions MUST NOT emit the
//! historical "is running with" or "(took live)" substrings — that's
//! the structural fix that closes the bug under audit (RCA: B+C+D
//! conjunction).

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use overdrive_cli::render::{
    format_job_attempt_failed, format_job_failed_summary, format_job_submit_echo,
    format_job_succeeded_summary,
};

/// S-02-01 unit — `format_job_succeeded_summary` renders the canonical
/// `Job '<name>' succeeded.` line with exit code and duration.
#[test]
fn job_succeeded_summary_names_exit_code_and_duration() {
    let rendered = format_job_succeeded_summary("coinflip", 0, "1.2s", 1);
    assert!(
        rendered.contains("Job 'coinflip' succeeded."),
        "expected Job succeeded line; got: {rendered}",
    );
    assert!(rendered.contains("exit code 0"), "expected exit code 0; got: {rendered}");
    assert!(rendered.contains("1.2s"), "expected duration; got: {rendered}");
}

/// S-02-02 unit — `format_job_failed_summary` renders the canonical
/// `Job '<name>' failed.` line with exit code, attempts, and stderr
/// tail. Backoff-exhausted shape includes "X of Y (backoff exhausted)".
#[test]
fn job_failed_summary_names_exit_code_attempts_and_stderr_tail() {
    let stderr_tail = "ERROR\nERROR\nERROR\n";
    let rendered = format_job_failed_summary("coinflip", Some(1), "1.0s", 3, 3, true, stderr_tail);
    assert!(
        rendered.contains("Job 'coinflip' failed."),
        "expected Job failed line; got: {rendered}",
    );
    assert!(rendered.contains("exit code 1"), "expected exit code 1; got: {rendered}");
    assert!(
        rendered.contains("3 of 3 (backoff exhausted)"),
        "expected attempts as '3 of 3 (backoff exhausted)'; got: {rendered}",
    );
    assert!(rendered.contains("ERROR"), "expected stderr tail; got: {rendered}");
}

/// S-02-03 unit — `format_job_attempt_failed` renders an intermediate
/// attempt-failed line. Stream stays open semantics — verb is
/// "attempt N failed (exit X). Retrying in Y."
#[test]
fn job_attempt_failed_intermediate_line() {
    let rendered = format_job_attempt_failed("coinflip", 1, 1, "200ms");
    assert!(
        rendered.contains("Job 'coinflip' attempt 1 failed"),
        "expected attempt line; got: {rendered}",
    );
    assert!(rendered.contains("exit 1"), "expected exit code; got: {rendered}");
    assert!(rendered.contains("Retrying in 200ms"), "expected retry hint; got: {rendered}");
}

/// S-02-05 anti-scenario — the literal "is running with" never
/// appears in any Job-kind render output. Anti-scenario; pinned at
/// the pure-function boundary so a future regression is caught
/// before the integration test runs.
#[test]
fn job_renders_never_contain_is_running_with() {
    let succeeded = format_job_succeeded_summary("any", 0, "1ms", 1);
    let failed = format_job_failed_summary("any", Some(1), "1ms", 1, 1, true, "stderr");
    let attempt = format_job_attempt_failed("any", 1, 1, "1ms");

    for r in [&succeeded, &failed, &attempt] {
        assert!(
            !r.contains("is running with"),
            "S-02-05: Job render must NOT contain 'is running with'; got: {r}",
        );
        assert!(
            !r.contains("(took live)"),
            "S-02-05: Job render must NOT contain '(took live)'; got: {r}",
        );
    }
}

/// S-02-06 unit — `format_job_submit_echo` renders `"Submitting job
/// '<name>' (kind=Job, run-to-completion)"`.
#[test]
fn job_submit_echo_names_kind_upfront() {
    let rendered = format_job_submit_echo("coinflip");
    assert!(
        rendered.contains("Submitting job 'coinflip' (kind=Job, run-to-completion)"),
        "S-02-06: submit echo must name kind upfront; got: {rendered}",
    );
}
