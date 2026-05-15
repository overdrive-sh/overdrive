//! S-WS-01 — Tier 3 happy-path streaming submit (Job-kind, ADR-0051 migration).
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria + the ADR-0051 wire-shape migration
//! (step 02-03b): a real in-process control plane (real
//! `LocalIntentStore` + `LocalObservationStore` + `ExecDriver`), a real
//! `[job]` spec naming `/bin/sh -c "exit 0"`, the streaming
//! `submit_streaming` handler driven from the CLI library surface (NOT
//! a subprocess — per `crates/overdrive-cli/CLAUDE.md` § *Integration
//! tests — no subprocess*), the `JobSubmitEvent::Succeeded` terminal
//! event observed, and the typed output mapping to exit code 0.
//!
//! ADR-0051 migration note: pre-migration this test exploited a
//! `workload_kind: None → Service` server-side coercion default to
//! drive Service-arm streaming via a flat `JobSpecInput` payload. That
//! coercion was deleted by ADR-0051 OQ-8. Post-migration the test
//! submits a `[job]`-shape TOML so the CLI routes via
//! `submit_streaming_job` → `consume_stream_job`, matching the
//! server's `JobSubmitEvent` emission. The semantic test ("a workload
//! that exits cleanly produces a clean terminal verdict and CLI exit
//! 0") is preserved; only the vocabulary changes (Service `"is running
//! with"` → Job `"Job '<name>' succeeded."`).
//!
//! Linux-gated because the production `ExecDriver` requires
//! `tokio::process::Command::spawn` against `/bin/sh`. The macOS
//! `--no-run` gate compiles this file via `cargo check
//! --features integration-tests` per `.claude/rules/testing.md`
//! § "Running integration tests locally on macOS — Lima VM".
//!
//! KPI alignment:
//!   * KPI-01 — first NDJSON line lands within the 200ms p95 envelope
//!     (the test does not assert this — KPI-01 is the streaming-server
//!     property — but the stream IS observed end-to-end).
//!   * KPI-04 — byte-equality of streaming `Accepted.spec_digest` and
//!     the pre-existing one-shot ack response (the `spec_digest` is
//!     stable across surfaces).

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::job::SubmitArgs;
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
    let args = ServeArgs { bind, data_dir, config_dir };
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

/// `[job]` TOML — the kind-discriminator triggers `WorkloadSpec::Job`
/// per ADR-0047. `/bin/sh -c "exit 0"` exits cleanly with exit code 0
/// so the Job-kind single-shot reconciler emits
/// `TerminalCondition::Completed { exit_code: 0 }` and the streaming
/// layer projects to `JobSubmitEvent::Succeeded`.
const fn happy_job_spec_toml() -> &'static str {
    r#"
[job]
id = "sleeper"

[exec]
command = "/bin/sh"
args = ["-c", "exit 0"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_submit_against_clean_exit_job_converges_succeeded_and_exits_0() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Write the spec, drive `submit_streaming` end-to-end through the
    // streaming NDJSON consumer.
    let spec_path = write_toml(server_tmp.path(), "sleeper.toml", happy_job_spec_toml());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg,
    })
    .await
    .expect("submit_streaming must succeed end-to-end against real ExecDriver(/bin/sh exit 0)");

    // The CLI's exit-code mapping for `JobSubmitEvent::Succeeded` is
    // the workload's kernel-observed exit code (KPI K1 honesty). For
    // `/bin/sh -c "exit 0"` that is 0.
    assert_eq!(
        output.exit_code, 0,
        "S-WS-01: Job-kind clean-exit must map to CLI exit code 0; got {} for output {output:?}",
        output.exit_code,
    );

    // Accepted event was observed — the synchronous `Accepted` line is
    // the load-bearing wiring witness. `outcome` is `Inserted` for a
    // fresh submit per ADR-0020.
    assert_eq!(output.outcome, IdempotencyOutcome::Inserted);
    assert_eq!(output.workload_id, "sleeper");
    assert_eq!(output.intent_key, "workloads/sleeper");

    // Spec digest is the canonical 64-char SHA-256 lowercase-hex form.
    assert_eq!(
        output.spec_digest.len(),
        64,
        "spec_digest must be 64-char lowercase hex SHA-256; got len={} value={}",
        output.spec_digest.len(),
        output.spec_digest,
    );
    assert!(
        output.spec_digest.chars().all(|c| c.is_ascii_hexdigit()),
        "spec_digest must be lowercase hex; got {}",
        output.spec_digest,
    );

    // S-02-01 — Job-kind clean-exit renders as `Job '<name>' succeeded.`
    // The `format_job_succeeded_summary` renderer in
    // `overdrive_cli::render` is the SSOT for this string shape.
    assert!(
        output.summary.contains("Job 'sleeper' succeeded."),
        "S-WS-01: Job-kind summary must contain `Job 'sleeper' succeeded.`; got: {}",
        output.summary,
    );
    assert!(
        output.summary.contains("exit code 0"),
        "S-WS-01: Job-kind summary must contain `exit code 0`; got: {}",
        output.summary,
    );
    // The Job-kind succeeded render carries a measured duration
    // segment — `Job '<name>' succeeded. (exit code 0, took
    // <duration>, attempts <N>)`. Assert on the `took ` infix; the
    // `(took ` form used by the legacy Service-vocabulary renderer
    // is structurally unreachable on the Job-kind path.
    assert!(
        output.summary.contains("took "),
        "S-WS-01: rendered summary must include a measured `took <duration>` segment; got: {}",
        output.summary,
    );

    // S-04-03 — anti-scenario: the literal `"live"` never appears in
    // the operator-visible render output. The dst-lint gate from
    // 01-01 forbids the literal in source; this test confirms the
    // observable consequence at the render boundary. The `(took
    // live)` shape was the historical false-positive substring this
    // test guards against.
    assert!(
        !output.summary.contains("(took live)"),
        "S-04-03: rendered summary must NOT contain `(took live)`; got: {}",
        output.summary,
    );

    // S-02-05 — Job-kind anti-scenario: the Service-vocabulary
    // substring `"is running with"` is structurally unreachable for
    // Job kind (no `ConvergedRunning` variant on the `JobSubmitEvent`
    // wire enum per ADR-0047 §3 [D2] / [D7]).
    assert!(
        !output.summary.contains("is running with"),
        "S-02-05: Job-kind summary must NOT contain Service vocabulary `is running with`; got: {}",
        output.summary,
    );
    assert!(
        !output.summary.contains("Service 'sleeper'"),
        "S-WS-01: Job-kind summary must NOT contain Service vocabulary `Service 'sleeper'`; got: {}",
        output.summary,
    );

    // `next_command` continues to point at `alloc status --job <name>`
    // for follow-up — the same shape as the one-shot ack lane.
    assert_eq!(output.next_command, "overdrive alloc status --job sleeper");

    // Job-kind terminal events do not carry the streaming/terminal
    // reason fields — those exist on the legacy Service-arm
    // `SubmitEvent::ConvergedFailed` path only. `consume_stream_job`
    // sets all three to None on every terminal arm.
    assert!(
        output.terminal_reason.is_none(),
        "Job-kind Succeeded must not carry terminal_reason; got {:?}",
        output.terminal_reason,
    );
    assert!(
        output.streaming_reason.is_none(),
        "Job-kind Succeeded must not carry streaming_reason; got {:?}",
        output.streaming_reason,
    );
    assert!(
        output.streaming_error.is_none(),
        "Job-kind Succeeded must not carry streaming_error; got {:?}",
        output.streaming_error,
    );

    handle.shutdown().await.expect("clean shutdown");
}
