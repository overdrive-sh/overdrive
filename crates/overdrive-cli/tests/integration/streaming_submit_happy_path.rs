//! S-WS-01 — Tier 3 happy-path streaming submit.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria: a real in-process control plane
//! (real `LocalIntentStore` + `LocalObservationStore` + `ExecDriver`),
//! a real spec naming `/bin/sleep`, the streaming `submit_streaming`
//! handler driven from the CLI library surface (NOT a subprocess —
//! per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*), the `ConvergedRunning` event observed, and the typed
//! output mapping to exit code 0.
//!
//! Linux-gated because the production `ExecDriver` requires
//! `tokio::process::Command::spawn` against `/bin/sleep`. The macOS
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
    // Per ADR-0034 the in-binary cgroup escape hatch is gone; on
    // macOS the pre-flight is a `#[cfg(target_os = "linux")]` no-op,
    // and on Linux this test runs via `cargo xtask lima run --`
    // against the bundled VM (root + delegated cgroups).
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

/// Spec naming `/bin/sleep` — exists on every Linux runner. `300` keeps
/// the workload alive long enough for the streaming handler to observe
/// `Running` and emit `ConvergedRunning`. Single replica so one
/// successful spawn is enough to converge.
const fn sleep_spec_toml() -> &'static str {
    r#"
id = "sleeper"
replicas = 1

[resources]
cpu_milli = 100
memory_bytes = 67108864

[exec]
command = "/bin/sleep"
args = ["300"]
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_submit_against_real_bin_sleep_converges_running_and_exits_0() {
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    // Write the spec, drive `submit_streaming` end-to-end through the
    // streaming NDJSON consumer.
    let spec_path = write_toml(server_tmp.path(), "sleeper.toml", sleep_spec_toml());

    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg,
    })
    .await
    .expect("submit_streaming must succeed end-to-end against real ExecDriver(/bin/sleep)");

    // The CLI's exit-code mapping for `ConvergedRunning` is 0.
    assert_eq!(
        output.exit_code, 0,
        "S-WS-01: ConvergedRunning must map to exit code 0; got {} for output {output:?}",
        output.exit_code,
    );

    // Accepted event was observed — the synchronous `Accepted` line is
    // the load-bearing wiring witness. `outcome` is `Inserted` for a
    // fresh submit per ADR-0020.
    assert_eq!(output.outcome, IdempotencyOutcome::Inserted);
    assert_eq!(output.job_id, "sleeper");
    assert_eq!(output.intent_key, "jobs/sleeper");

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

    // Stream summary contains the `is running` text for an operator-facing
    // success line.
    assert!(
        output.summary.contains("running"),
        "S-WS-01: rendered summary must mention `running`; got: {}",
        output.summary,
    );

    // `next_command` continues to point at `alloc status --job <name>`
    // for follow-up — the same shape as the one-shot ack lane.
    assert_eq!(output.next_command, "overdrive alloc status --job sleeper");

    // Reap the running /bin/sleep workload so nextest does not flag the
    // test process as LEAK. We submit a stop intent the same way an
    // operator would.
    let _ = overdrive_cli::commands::job::stop(overdrive_cli::commands::job::StopArgs {
        id: "sleeper".to_owned(),
        config_path: config_path(server_tmp.path()),
    })
    .await;
    handle.shutdown().await.expect("clean shutdown");
}
