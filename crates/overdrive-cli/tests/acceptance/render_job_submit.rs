//! Acceptance tests for `overdrive_cli::render::{job_submit_accepted,
//! cli_error}` — step 05-04.
//!
//! Rendering functions are pure string-builders — no I/O, no server
//! dependency — so they stay in the default acceptance lane alongside
//! the step-05-03 render acceptance tests. Separating rendering from
//! handler correctness prevents rendering drift from bleeding into
//! handler tests.
//!
//! Acceptance coverage:
//!   (f) `render::job_submit_accepted` emits a multi-line string with
//!       `Accepted.`, `Job ID:`, `Intent key:`, `Spec digest:`,
//!       `Outcome:`, `Endpoint:`, `Next:` labels and the corresponding
//!       values (per ADR-0020 the `Commit index:` line was dropped).
//!   (g) `render::cli_error` for `CliError::Transport` contains the
//!       endpoint plus the two suggestion markers ("Verify" the config,
//!       "Start" the control plane) and does NOT leak the raw `reqwest`
//!       token, and does NOT mention the removed `--endpoint` /
//!       `OVERDRIVE_ENDPOINT` override surface.

use overdrive_cli::commands::job::SubmitOutput;
use overdrive_cli::http_client::CliError;
use overdrive_control_plane::api::IdempotencyOutcome;
use url::Url;

const FIXTURE_DIGEST: &str = "deadbeefcafebabe0123456789abcdefdeadbeefcafebabe0123456789abcdef";

fn fixture_submit_output() -> SubmitOutput {
    SubmitOutput {
        job_id: "payments".to_string(),
        intent_key: "jobs/payments".to_string(),
        spec_digest: FIXTURE_DIGEST.to_string(),
        outcome: IdempotencyOutcome::Inserted,
        endpoint: Url::parse("https://127.0.0.1:7001").expect("parse endpoint"),
        next_command: "overdrive alloc status --job payments".to_string(),
    }
}

// -------------------------------------------------------------------
// (f) render::job_submit_accepted contains required labels + values
// -------------------------------------------------------------------

#[test]
fn render_job_submit_accepted_contains_required_labels() {
    let out = fixture_submit_output();
    let rendered = overdrive_cli::render::job_submit_accepted(&out);

    for label in
        ["Accepted.", "Job ID:", "Intent key:", "Spec digest:", "Outcome:", "Endpoint:", "Next:"]
    {
        assert!(
            rendered.contains(label),
            "rendered job_submit_accepted must contain `{label}`; got:\n{rendered}",
        );
    }
    assert!(
        rendered.contains("payments"),
        "rendered block must contain job_id value `payments`; got:\n{rendered}",
    );
    assert!(
        rendered.contains("jobs/payments"),
        "rendered block must contain intent_key `jobs/payments`; got:\n{rendered}",
    );
    assert!(
        rendered.contains(FIXTURE_DIGEST),
        "rendered block must contain spec_digest value; got:\n{rendered}",
    );
    assert!(
        rendered.contains("127.0.0.1:7001"),
        "rendered block must contain endpoint; got:\n{rendered}",
    );
    assert!(
        rendered.contains("overdrive alloc status --job payments"),
        "rendered block must contain next_command; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (g) render::cli_error for Transport lists two suggestions
// -------------------------------------------------------------------

#[test]
fn render_cli_error_transport_contains_actionable_suggestions_and_endpoint() {
    let err = CliError::Transport {
        endpoint: "https://127.0.0.1:7001".to_string(),
        cause: "could not connect to server".to_string(),
    };
    let rendered = overdrive_cli::render::cli_error(&err);

    assert!(
        rendered.contains("127.0.0.1:7001"),
        "rendered cli_error must name the endpoint; got:\n{rendered}",
    );
    // Two concrete suggestion phrases — the override surface was removed
    // (no `--endpoint` / `OVERDRIVE_ENDPOINT`), so the third suggestion
    // about runtime overrides no longer applies. Case-insensitive match
    // on recognisable keywords so minor wording changes don't invalidate.
    let rendered_lower = rendered.to_lowercase();
    for (key, label) in
        [("verify", "Verify the endpoint in the config"), ("start", "Start the control plane")]
    {
        assert!(
            rendered_lower.contains(key),
            "rendered cli_error must contain suggestion '{label}' (key '{key}'); got:\n{rendered}",
        );
    }
    assert!(
        !rendered.contains("reqwest"),
        "rendered cli_error must not leak `reqwest` token; got:\n{rendered}",
    );
    // The override surface was removed; the render must not advertise it.
    assert!(
        !rendered.contains("--endpoint") && !rendered.contains("OVERDRIVE_ENDPOINT"),
        "rendered cli_error must NOT mention the removed --endpoint / OVERDRIVE_ENDPOINT override; got:\n{rendered}",
    );
}
