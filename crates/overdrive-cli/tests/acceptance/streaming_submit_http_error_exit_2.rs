//! S-CLI-05 — pre-Accepted HTTP-error → exit code 2 mapping.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria:
//!
//! > pre-Accepted HTTP errors (4xx/5xx ErrorBody, transport failures)
//! > → print message and exit 2 (S-CLI-05 parametrised over
//! >   400/404/409/500/transport_err)
//!
//! The streaming submit handler runs through the same
//! `ApiClient::submit_job_streaming` plumbing as the original
//! one-shot `submit_job`. Pre-Accepted errors are HTTP responses with
//! non-2xx status that arrive BEFORE any `SubmitEvent::Accepted` line
//! has been observed. The renderer's exit-code helper must map every
//! such variant — and `CliError::Transport` — to exit code 2.
//!
//! This test covers the exit-code mapping side. The render-block side
//! (the CLI's stderr message naming the offending status) is exercised
//! by the existing `render_job_submit::render_cli_error_transport_*`
//! tests and the per-status tests in `tests/integration/job_submit.rs`.

use overdrive_cli::http_client::CliError;
use overdrive_control_plane::api::ErrorBody;

/// Exit code the CLI must return for pre-Accepted HTTP errors and
/// transport failures. Matches the criteria's parametrised expectation:
/// every `CliError::HttpStatus { 400 | 404 | 409 | 500 }` AND
/// `CliError::Transport` → 2.
const EXIT_CODE_HTTP_ERROR: i32 = 2;

fn fake_error_body(status: u16) -> ErrorBody {
    ErrorBody {
        error: format!("status_{status}"),
        message: format!("control plane returned HTTP {status} fake-fixture"),
        field: None,
    }
}

/// Drives the `cli_error_to_exit_code` helper across every status code
/// the criteria parametrises, then `transport_err`. Each call is one
/// "Example" row in the parametrised gherkin scenario.
#[test]
fn http_status_400_maps_to_exit_code_2() {
    let err = CliError::HttpStatus { status: 400, body: fake_error_body(400) };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "400 must map to exit code 2",
    );
}

#[test]
fn http_status_404_maps_to_exit_code_2() {
    let err = CliError::HttpStatus { status: 404, body: fake_error_body(404) };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "404 must map to exit code 2",
    );
}

#[test]
fn http_status_409_maps_to_exit_code_2() {
    let err = CliError::HttpStatus { status: 409, body: fake_error_body(409) };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "409 must map to exit code 2",
    );
}

#[test]
fn http_status_500_maps_to_exit_code_2() {
    let err = CliError::HttpStatus { status: 500, body: fake_error_body(500) };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "500 must map to exit code 2",
    );
}

#[test]
fn transport_failure_maps_to_exit_code_2() {
    let err = CliError::Transport {
        endpoint: "https://127.0.0.1:1".to_owned(),
        cause: "could not connect to server".to_owned(),
    };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "transport failure must map to exit code 2",
    );
}

#[test]
fn body_decode_failure_maps_to_exit_code_2() {
    // BodyDecode is the protocol-violation variant — server returned 2xx
    // but body did not parse into the expected typed shape. Map to exit
    // 2 alongside HTTP-status / transport variants because the CLI never
    // received an Accepted event.
    let err =
        CliError::BodyDecode { cause: "expected SubmitEvent line, got partial JSON".to_owned() };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "body decode failure must map to exit code 2",
    );
}

#[test]
fn invalid_spec_maps_to_exit_code_2_no_http_call_made() {
    // InvalidSpec fires BEFORE any HTTP call (client-side `Job::from_spec`
    // validation). The criteria pin every pre-Accepted failure to exit 2
    // — InvalidSpec is the earliest pre-Accepted failure shape.
    let err =
        CliError::InvalidSpec { field: "replicas".to_owned(), message: "must be > 0".to_owned() };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "client-side InvalidSpec must map to exit code 2",
    );
}

#[test]
fn config_load_failure_maps_to_exit_code_2() {
    let err = CliError::ConfigLoad {
        path: "/no/such/config".to_owned(),
        cause: "file not found".to_owned(),
    };
    assert_eq!(
        overdrive_cli::render::cli_error_to_exit_code(&err),
        EXIT_CODE_HTTP_ERROR,
        "config-load failure must map to exit code 2",
    );
}
