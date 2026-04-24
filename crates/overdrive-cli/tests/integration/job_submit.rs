//! Integration tests for `overdrive_cli::commands::job::submit` —
//! step 05-04.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handler directly
//! (NO subprocess). A real in-process control-plane server stands up via
//! `commands::serve::run(...)` (step 05-02), then the handler reads a
//! TOML file from disk, validates locally through `Job::from_spec`
//! (ADR-0011 constructor), POSTs `SubmitJobRequest` via the `ApiClient`
//! from step 05-01, and returns a typed `SubmitOutput` with `job_id`,
//! `intent_key`, `commit_index`, `endpoint`, and `next_command`.
//!
//! Acceptance coverage:
//!   (a) valid TOML against in-process server returns `SubmitOutput`
//!       with `job_id = "payments"`, `intent_key = "jobs/payments"`,
//!       `commit_index >= 1`, `next_command` naming
//!       `overdrive alloc status --job payments`.
//!   (b) `replicas = 0` returns `CliError::InvalidSpec { field:
//!       "replicas", message }` WITHOUT issuing any HTTP — the handler
//!       runs `Job::from_spec` locally and fails fast.
//!   (c) malformed TOML syntax returns `CliError::InvalidSpec` with a
//!       parse-error message naming the TOML problem.
//!   (d) connection-refused endpoint returns `CliError::Transport`
//!       whose Display form (rendered via `render::cli_error`) names
//!       the endpoint, explains the endpoint is unreachable, and lists
//!       three concrete next steps.
//!   (e) the rendered Transport form MUST NOT contain the raw token
//!       `reqwest` — operator-facing errors do not leak reqwest Debug.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::{SubmitArgs, SubmitOutput};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::CliError;
use tempfile::TempDir;
use url::Url;

/// Spin up a real in-process control-plane server on `127.0.0.1:0` and
/// return the handle, the `TempDir` backing the data directory, and the
/// endpoint URL pointing at the ephemeral port. The `TempDir` is
/// returned so the caller can keep it alive for the duration of the
/// test — dropping it deletes the config.
async fn spawn_server() -> (ServeHandle, TempDir, Url) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = ServeArgs { bind, data_dir: tmp.path().to_path_buf() };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    let port = handle.endpoint().port().expect("endpoint port");
    let endpoint = Url::parse(&format!("https://localhost:{port}")).expect("parse endpoint");
    (handle, tmp, endpoint)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<data_dir>/.overdrive/config`.
fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".overdrive").join("config")
}

/// Write a valid `payments.toml` into `dir` and return its path.
fn write_valid_payments_toml(dir: &Path) -> PathBuf {
    let spec = r#"
id = "payments"
replicas = 3
cpu_milli = 500
memory_bytes = 536870912
"#;
    let path = dir.join("payments.toml");
    std::fs::write(&path, spec).expect("write payments.toml");
    path
}

// -------------------------------------------------------------------
// (a) submit with valid TOML against in-process server → SubmitOutput
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_with_valid_toml_against_in_process_server_returns_submit_output_with_intent_key_and_next_command()
 {
    let (handle, tmp, endpoint) = spawn_server().await;
    let spec_path = write_valid_payments_toml(tmp.path());

    let args = SubmitArgs {
        spec: spec_path,
        endpoint: endpoint.clone(),
        config_path: config_path(tmp.path()),
    };
    let output: SubmitOutput =
        overdrive_cli::commands::job::submit(args).await.expect("job::submit");

    assert_eq!(output.job_id, "payments", "SubmitOutput.job_id must be 'payments'");
    assert_eq!(
        output.intent_key, "jobs/payments",
        "SubmitOutput.intent_key must be 'jobs/payments'",
    );
    assert!(
        output.commit_index >= 1,
        "SubmitOutput.commit_index must be >= 1; got {}",
        output.commit_index,
    );
    assert_eq!(output.endpoint, endpoint, "SubmitOutput.endpoint must echo the input endpoint");
    assert_eq!(
        output.next_command, "overdrive alloc status --job payments",
        "SubmitOutput.next_command must guide the operator to alloc status",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// -------------------------------------------------------------------
// (b) replicas = 0 returns InvalidSpec BEFORE any HTTP call
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_with_zero_replicas_returns_invalid_spec_before_any_http_call() {
    // No server spawned — point at a port nobody listens on. If
    // local validation works, we never reach HTTP and do not see a
    // transport error.
    let tmp = TempDir::new().expect("tempdir");

    // Need a trust-triple file on disk so `from_config_with_endpoint`
    // doesn't fail with ConfigLoad before we even reach validation.
    // Spawn-and-shutdown to write a valid config, then point at a dead
    // port. Early validation should short-circuit before any connect.
    let (handle, tmp2, _endpoint) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");

    let broken_spec = r#"
id = "payments"
replicas = 0
cpu_milli = 500
memory_bytes = 536870912
"#;
    let spec_path = tmp.path().join("broken.toml");
    std::fs::write(&spec_path, broken_spec).expect("write broken.toml");

    // Unreachable port. If the handler ever issues an HTTP request,
    // we'd get a Transport error instead of InvalidSpec.
    let dead_endpoint: Url = "https://127.0.0.1:1".parse().expect("parse dead endpoint");
    let args = SubmitArgs {
        spec: spec_path,
        endpoint: dead_endpoint,
        config_path: config_path(tmp2.path()),
    };
    let err = overdrive_cli::commands::job::submit(args)
        .await
        .expect_err("replicas=0 must fail local validation");

    match &err {
        CliError::InvalidSpec { field, message } => {
            assert_eq!(field, "replicas", "field must name 'replicas'");
            assert!(
                message.contains("replicas") || message.contains("non-zero"),
                "message must explain the violation; got {message}",
            );
            assert!(
                message.contains('0') || message.contains("non-zero"),
                "message must name the value; got {message}",
            );
        }
        other => {
            panic!(
                "expected CliError::InvalidSpec (local validation), got {other:?} — \
                 this usually means validation leaked through to HTTP"
            );
        }
    }
}

// -------------------------------------------------------------------
// (c) malformed TOML syntax returns InvalidSpec
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_with_malformed_toml_syntax_returns_invalid_spec() {
    let tmp = TempDir::new().expect("tempdir");
    let (handle, tmp2, _endpoint) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");

    // Unclosed array bracket — malformed TOML syntax.
    let broken_syntax = r#"
id = "payments"
replicas = [1, 2, 3
cpu_milli = 500
"#;
    let spec_path = tmp.path().join("broken_syntax.toml");
    std::fs::write(&spec_path, broken_syntax).expect("write broken_syntax.toml");

    let dead_endpoint: Url = "https://127.0.0.1:1".parse().expect("parse dead endpoint");
    let args = SubmitArgs {
        spec: spec_path,
        endpoint: dead_endpoint,
        config_path: config_path(tmp2.path()),
    };
    let err =
        overdrive_cli::commands::job::submit(args).await.expect_err("malformed TOML must fail");

    match &err {
        CliError::InvalidSpec { .. } => (),
        other => panic!("expected CliError::InvalidSpec for malformed TOML, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// (d) connection-refused endpoint → Transport with three suggestions
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_against_unreachable_endpoint_returns_transport_error_naming_endpoint_with_three_suggestions()
 {
    let tmp = TempDir::new().expect("tempdir");
    // Write a valid trust triple so `ApiClient::from_config_with_endpoint`
    // succeeds and we exercise the transport layer (not ConfigLoad).
    let (handle, tmp2, _endpoint) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");

    let spec_path = write_valid_payments_toml(tmp.path());

    // Port 1 — nobody listens there; reqwest will fail to connect.
    let dead_endpoint: Url = "https://127.0.0.1:1".parse().expect("parse dead endpoint");
    let args = SubmitArgs {
        spec: spec_path,
        endpoint: dead_endpoint.clone(),
        config_path: config_path(tmp2.path()),
    };
    let err = overdrive_cli::commands::job::submit(args)
        .await
        .expect_err("unreachable endpoint must fail");

    match &err {
        CliError::Transport { endpoint, .. } => {
            assert!(
                endpoint.contains("127.0.0.1:1"),
                "Transport.endpoint must name the endpoint; got {endpoint}",
            );
        }
        other => panic!("expected CliError::Transport, got {other:?}"),
    }

    // Render through `render::cli_error` — must name the endpoint and
    // list three concrete next-step suggestions.
    let rendered = overdrive_cli::render::cli_error(&err);
    assert!(
        rendered.contains("127.0.0.1:1"),
        "rendered error must name the endpoint; got:\n{rendered}",
    );
    // Three suggestions — flexible match on recognisable phrases.
    let suggestion_markers = [
        ("start", "Start the control plane"),
        ("verify", "Verify the endpoint"),
        ("override", "Override the endpoint"),
    ];
    for (key, marker) in suggestion_markers {
        assert!(
            rendered.to_lowercase().contains(key),
            "rendered error must contain suggestion '{marker}' (key '{key}'); got:\n{rendered}",
        );
    }
}

// -------------------------------------------------------------------
// (e) rendered Transport error does NOT leak raw reqwest token
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_transport_error_display_does_not_contain_raw_reqwest_token() {
    let tmp = TempDir::new().expect("tempdir");
    let (handle, tmp2, _endpoint) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");

    let spec_path = write_valid_payments_toml(tmp.path());

    let dead_endpoint: Url = "https://127.0.0.1:1".parse().expect("parse dead endpoint");
    let args = SubmitArgs {
        spec: spec_path,
        endpoint: dead_endpoint,
        config_path: config_path(tmp2.path()),
    };
    let err = overdrive_cli::commands::job::submit(args)
        .await
        .expect_err("unreachable endpoint must fail");

    let display_form = format!("{err}");
    let rendered = overdrive_cli::render::cli_error(&err);
    assert!(
        !display_form.contains("reqwest"),
        "Display form must not leak `reqwest` token; got:\n{display_form}",
    );
    assert!(
        !rendered.contains("reqwest"),
        "render::cli_error must not leak `reqwest` token; got:\n{rendered}",
    );
}
