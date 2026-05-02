//! Acceptance tests for `overdrive_cli::render::job_stop_accepted`.
//!
//! Pins the rendered output for both `StopOutcome::Stopped` and
//! `StopOutcome::AlreadyStopped`. Kills the two body-replacement
//! mutations on `render::job_stop_accepted`:
//!
//!   - body → `String::new()` — empty string
//!   - body → `"xyzzy".into()` — wrong literal
//!
//! Asserting on specific substrings (the operator-facing labels and
//! values per ADR-0027) rejects both mutations: an empty string
//! contains nothing; "xyzzy" doesn't contain `Stopped job` or
//! `Endpoint:` or the job id.

#![allow(clippy::expect_used)]

use overdrive_cli::commands::job::StopOutput;
use overdrive_control_plane::api::StopOutcome;
use url::Url;

fn fixture_stop_output(outcome: StopOutcome) -> StopOutput {
    StopOutput {
        job_id: "payments".to_string(),
        outcome,
        endpoint: Url::parse("https://127.0.0.1:7001").expect("parse endpoint"),
    }
}

#[test]
fn render_job_stop_accepted_for_stopped_outcome() {
    let out = fixture_stop_output(StopOutcome::Stopped);
    let rendered = overdrive_cli::render::job_stop_accepted(&out);

    assert!(
        rendered.contains("Stopped job 'payments'"),
        "rendered output must contain `Stopped job 'payments'`; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Endpoint:"),
        "rendered output must contain `Endpoint:` label; got:\n{rendered}",
    );
    assert!(
        rendered.contains("https://127.0.0.1:7001"),
        "rendered output must contain endpoint URL; got:\n{rendered}",
    );
    assert!(!rendered.is_empty(), "rendered output must not be empty");
    // Belt-and-braces — explicitly reject the `xyzzy` mutant body.
    assert_ne!(rendered, "xyzzy", "rendered output must not be the mutant marker `xyzzy`");
}

#[test]
fn render_job_stop_accepted_for_already_stopped_outcome() {
    let out = fixture_stop_output(StopOutcome::AlreadyStopped);
    let rendered = overdrive_cli::render::job_stop_accepted(&out);

    assert!(
        rendered.contains("Job 'payments' was already stopped"),
        "rendered output must mention idempotent path; got:\n{rendered}",
    );
    assert!(
        rendered.contains("(no-op)"),
        "rendered output must label idempotent path as `(no-op)`; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Endpoint:"),
        "rendered output must contain `Endpoint:` label; got:\n{rendered}",
    );
    assert!(!rendered.is_empty(), "rendered output must not be empty");
}
