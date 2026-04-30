//! S-CLI-03 — Tier 3 `overdrive job submit | jq -r .spec_digest`
//! pipeline-equivalent.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/03-02`
//! step 03-02 acceptance criteria:
//!
//! > Given a control plane is running
//! > When the operator runs
//! >   `overdrive job submit ./payments.toml | jq -r .spec_digest`
//! > Then jq's output is a single line of 64 hex characters
//! > And the CLI exits with status 0
//!
//! ## In-process equivalent (per `crates/overdrive-cli/CLAUDE.md`)
//!
//! `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess* is unambiguous: this crate does not spawn `overdrive` as
//! a child process under any circumstance. The structural property the
//! shell-pipeline form exercises is:
//!
//! > a non-TTY stdout (`stdin` of the next pipeline stage) MUST
//! > auto-select the JSON-ack lane and emit a single parseable JSON
//! > object whose `spec_digest` field is a 64-char lowercase-hex SHA-256.
//!
//! We exercise that property in-process by:
//!
//! 1. Spinning up the real `serve::run` control plane (real
//!    `LocalIntentStore`, real `LocalObservationStore`, real
//!    `ExecDriver`) — same shape as
//!    `streaming_submit_happy_path.rs` and the JSON-ack
//!    `job_submit.rs`.
//! 2. Driving the dispatch decision through `should_stream(detach=false,
//!    is_terminal=false)` — `false` simulates the pipe-redirected
//!    stdout the real shell pipeline produces. This is the same
//!    decision main.rs makes; the pure function is the SSOT.
//! 3. Calling `commands::job::submit` (the JSON-ack lane) — the lane
//!    `should_stream == false` selects.
//! 4. Asserting on the typed `SubmitOutput.spec_digest` — a 64-char
//!    lowercase-hex SHA-256 — which is what `jq -r .spec_digest` would
//!    extract from the real JSON response body.
//!
//! The `Accept: application/json` header pinning is structural in
//! `ApiClient::submit_job` (the only public API the dispatched handler
//! reaches); the JSON-body shape is structural in
//! `SubmitJobResponse` (the typed response). Calling the dispatched
//! handler IS the wire-level witness — short of `Command::spawn`,
//! which CLAUDE.md forbids.
//!
//! Linux-gated because the production `ExecDriver` requires
//! `tokio::process::Command::spawn` against a real binary path; macOS
//! dev runs via `cargo xtask lima run --` per `crates/overdrive-cli/CLAUDE.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::{SubmitArgs, SubmitOutput, should_stream};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::IdempotencyOutcome;
use tempfile::TempDir;

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir, allow_no_cgroups: true };
    let handle = overdrive_cli::commands::serve::run(args).await.expect("serve::run");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

/// Minimal valid spec — `/bin/true` exists on every Linux runner and
/// terminates immediately. The JSON-ack lane returns as soon as the
/// `IntentStore` commit lands, so the workload's runtime behaviour is
/// not load-bearing — only that the spec validates and submits.
const fn payments_spec_toml() -> &'static str {
    r#"
id = "payments"
replicas = 1

[resources]
cpu_milli = 500
memory_bytes = 536870912

[exec]
command = "/bin/true"
args = []
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipe_redirected_submit_emits_64_char_hex_spec_digest_via_json_lane() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    let spec_path = write_toml(tmp.path(), "payments.toml", payments_spec_toml());

    // Step 1 — the dispatch decision under a pipe-redirected stdout.
    // `should_stream` is the SSOT main.rs branches on. With detach=false
    // and is_terminal=false (a pipe), the lane selected is the JSON-ack
    // lane. This is the truth-table row 3 from architecture.md §6.
    assert!(
        !should_stream(false, false),
        "S-CLI-03 precondition: pipe-redirected stdout (is_terminal=false) without --detach \
         MUST select the JSON-ack lane — got `should_stream == true`, which would route to the \
         streaming consumer instead of the single JSON object the pipeline expects",
    );

    // Step 2 — call the lane the dispatch decision selects (JSON-ack).
    // This IS what main.rs would do under
    //   `overdrive job submit ./payments.toml | jq -r .spec_digest`
    // — the pipe makes stdout non-TTY, `should_stream` returns false,
    // and main.rs invokes `commands::job::submit` (which sets
    // `Accept: application/json` on the request).
    let output: SubmitOutput =
        overdrive_cli::commands::job::submit(SubmitArgs { spec: spec_path, config_path: cfg })
            .await
            .expect(
                "S-CLI-03: pipe-redirected `overdrive job submit ./payments.toml` MUST succeed — \
                 the JSON-ack lane against an in-process control plane is the same wire path the \
                 shell pipeline would exercise",
            );

    // Step 3 — assert on the field `jq -r .spec_digest` would extract.
    // The shell pipeline would observe a single JSON object on stdout,
    // pipe it to jq, and read 64 hex chars. The typed `SubmitOutput`
    // is what `submit` returns from parsing the same JSON body — so
    // asserting on `output.spec_digest` IS asserting on what jq would
    // print.
    assert_eq!(
        output.spec_digest.len(),
        64,
        "S-CLI-03 KPI: `jq -r .spec_digest` output must be 64 chars (SHA-256 lowercase-hex); \
         got {} chars: `{}`",
        output.spec_digest.len(),
        output.spec_digest,
    );
    assert!(
        output.spec_digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "S-CLI-03 KPI: `jq -r .spec_digest` output must be lowercase ASCII hex; got `{}`",
        output.spec_digest,
    );

    // Step 4 — outcome is `Inserted` for a fresh submission. This
    // mirrors the shell pipeline's exit status 0: a successful
    // `Inserted` outcome is the JSON-ack happy path that main.rs maps
    // to `Ok(())` (process exit 0). A non-2xx response would have
    // surfaced as `Err(CliError::HttpStatus)` above and failed the
    // `expect` — there is no path through the pipeline-equivalent that
    // reaches a 64-hex-char digest with a non-zero exit code.
    assert_eq!(
        output.outcome,
        IdempotencyOutcome::Inserted,
        "S-CLI-03: a fresh submit must produce `Inserted` outcome (which main.rs maps to exit 0); \
         got {:?}",
        output.outcome,
    );

    handle.shutdown().await.expect("clean shutdown");
}
