//! Integration tests for `overdrive_cli::commands::job::submit` —
//! step 05-04.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` these call the handler directly
//! (NO subprocess). A real in-process control-plane server stands up via
//! `commands::serve::run(...)` (step 05-02), then the handler reads a
//! TOML file from disk, validates locally through `Job::from_spec`
//! (ADR-0011 constructor), POSTs `SubmitJobRequest` via the `ApiClient`
//! from step 05-01, and returns a typed `SubmitOutput` with `job_id`,
//! `intent_key`, `spec_digest`, `outcome`, `endpoint`, and
//! `next_command` (per ADR-0020 the `commit_index` field is dropped).
//!
//! Acceptance coverage:
//!   (a) valid TOML against in-process server returns `SubmitOutput`
//!       with `job_id = "payments"`, `intent_key = "jobs/payments"`,
//!       `outcome = IdempotencyOutcome::Inserted`, a 64-char
//!       `spec_digest`, and `next_command` naming
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
use overdrive_control_plane::api::IdempotencyOutcome;
use tempfile::TempDir;

/// Spin up a real in-process control-plane server on `127.0.0.1:0` and
/// return the handle and the `TempDir` backing both directories. The
/// `TempDir` is returned so the caller can keep it alive for the
/// duration of the test — dropping it deletes the config.
///
/// `data_dir` and `config_dir` are SEPARATE subdirectories of the
/// tempdir per `fix-cli-cannot-reach-control-plane` Step 01-02
/// (RCA §WHY 4C).
async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    // CLI integration tests don't start real workloads; bypass the
    // cgroup pre-flight so they run uniformly on macOS and on Linux
    // without delegation.
    let args = ServeArgs { bind, data_dir, config_dir, allow_no_cgroups: true };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    (handle, tmp)
}

/// Path of the trust-triple config written by `serve::run` into
/// `<config_dir>/.overdrive/config` — given the tempdir root from
/// [`spawn_server`].
fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// Rewrite the `endpoint` field in the on-disk trust-triple TOML.
/// Used only by the transport-error tests below — they start a real
/// server (so the trust material is valid), shut it down, and then
/// overwrite the endpoint with an unreachable one to exercise the
/// `CliError::Transport` path. The operator config is the sole source
/// of the endpoint, so changing it here is the only way to point the
/// handler at a chosen dead endpoint.
fn rewrite_config_endpoint(config_path: &Path, new_endpoint: &str) {
    let original = std::fs::read_to_string(config_path).expect("read existing trust-triple config");
    let mut doc: toml::Value = toml::from_str(&original).expect("parse existing config toml");
    let contexts =
        doc.get_mut("contexts").and_then(|c| c.as_array_mut()).expect("contexts array present");
    for ctx in contexts.iter_mut() {
        if let Some(tbl) = ctx.as_table_mut() {
            tbl.insert("endpoint".to_owned(), toml::Value::String(new_endpoint.to_owned()));
        }
    }
    let rewritten = toml::to_string(&doc).expect("reserialise config toml");
    std::fs::write(config_path, rewritten).expect("write rewritten config");
}

/// Overwrite the on-disk config's endpoint with a chosen one and return
/// the config path. Used only by transport-error tests — valid-case
/// tests read the endpoint `run_server` already recorded.
fn point_config_at(tmp: &Path, endpoint: &str) -> PathBuf {
    let cfg = config_path(tmp);
    rewrite_config_endpoint(&cfg, endpoint);
    cfg
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
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_valid_payments_toml(tmp.path());

    let args = SubmitArgs { spec: spec_path, config_path: cfg };
    let output: SubmitOutput =
        overdrive_cli::commands::job::submit(args).await.expect("job::submit");

    assert_eq!(output.job_id, "payments", "SubmitOutput.job_id must be 'payments'");
    assert_eq!(
        output.intent_key, "jobs/payments",
        "SubmitOutput.intent_key must be 'jobs/payments'",
    );
    assert_eq!(
        output.outcome,
        IdempotencyOutcome::Inserted,
        "SubmitOutput.outcome must be `Inserted` on a fresh submit; got {:?}",
        output.outcome,
    );
    assert_eq!(
        output.spec_digest.len(),
        64,
        "SubmitOutput.spec_digest must be 64 hex chars (SHA-256); got {} chars",
        output.spec_digest.len(),
    );
    assert_eq!(
        output.endpoint,
        *handle.endpoint(),
        "SubmitOutput.endpoint must echo the endpoint recorded in the operator config",
    );
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
    // No server spawned — point the on-disk config at a dead port. If
    // local validation works, we never reach HTTP and do not see a
    // transport error.
    let tmp = TempDir::new().expect("tempdir");

    // Need a trust-triple file on disk so `from_config` doesn't fail
    // with ConfigLoad before we even reach validation. Spawn-and-shutdown
    // to write a valid config, then rewrite its endpoint to an unreachable
    // port. Early validation should short-circuit before any connect.
    let (handle, tmp2) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg = point_config_at(tmp2.path(), "https://127.0.0.1:1");

    let broken_spec = r#"
id = "payments"
replicas = 0
cpu_milli = 500
memory_bytes = 536870912
"#;
    let spec_path = tmp.path().join("broken.toml");
    std::fs::write(&spec_path, broken_spec).expect("write broken.toml");

    let args = SubmitArgs { spec: spec_path, config_path: cfg };
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
    let (handle, tmp2) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg = point_config_at(tmp2.path(), "https://127.0.0.1:1");

    // Unclosed array bracket — malformed TOML syntax.
    let broken_syntax = r#"
id = "payments"
replicas = [1, 2, 3
cpu_milli = 500
"#;
    let spec_path = tmp.path().join("broken_syntax.toml");
    std::fs::write(&spec_path, broken_syntax).expect("write broken_syntax.toml");

    let args = SubmitArgs { spec: spec_path, config_path: cfg };
    let err =
        overdrive_cli::commands::job::submit(args).await.expect_err("malformed TOML must fail");

    match &err {
        CliError::InvalidSpec { .. } => (),
        other => panic!("expected CliError::InvalidSpec for malformed TOML, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// (d) connection-refused endpoint → Transport with two suggestions
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_against_unreachable_endpoint_returns_transport_error_naming_endpoint_with_actionable_suggestions()
 {
    let tmp = TempDir::new().expect("tempdir");
    // Write a valid trust triple so `ApiClient::from_config` succeeds
    // and we exercise the transport layer (not ConfigLoad). Then
    // rewrite its endpoint to a port nobody listens on — reqwest will
    // fail to connect.
    let (handle, tmp2) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg = point_config_at(tmp2.path(), "https://127.0.0.1:1");

    let spec_path = write_valid_payments_toml(tmp.path());

    let args = SubmitArgs { spec: spec_path, config_path: cfg };
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
    // list the two concrete next-step suggestions (no override surface
    // exists, so no third suggestion about `--endpoint`).
    let rendered = overdrive_cli::render::cli_error(&err);
    assert!(
        rendered.contains("127.0.0.1:1"),
        "rendered error must name the endpoint; got:\n{rendered}",
    );
    let suggestion_markers = [
        ("verify", "Verify the endpoint in `~/.overdrive/config`"),
        ("start", "Start the control plane"),
    ];
    for (key, marker) in suggestion_markers {
        assert!(
            rendered.to_lowercase().contains(key),
            "rendered error must contain suggestion '{marker}' (key '{key}'); got:\n{rendered}",
        );
    }
    // Negative check: the pre-fix override suggestion must not appear.
    assert!(
        !rendered.contains("--endpoint") && !rendered.contains("OVERDRIVE_ENDPOINT"),
        "rendered error must NOT mention the removed --endpoint / OVERDRIVE_ENDPOINT override; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (e) rendered Transport error does NOT leak raw reqwest token
// -------------------------------------------------------------------

#[tokio::test]
async fn submit_transport_error_display_does_not_contain_raw_reqwest_token() {
    let tmp = TempDir::new().expect("tempdir");
    let (handle, tmp2) = spawn_server().await;
    handle.shutdown().await.expect("clean shutdown");
    let cfg = point_config_at(tmp2.path(), "https://127.0.0.1:1");

    let spec_path = write_valid_payments_toml(tmp.path());

    let args = SubmitArgs { spec: spec_path, config_path: cfg };
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
