//! S-WS-02 — Tier 3 broken-binary regression target (US-02 KPI-02 / KPI-04).
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
//! step 02-04 acceptance criteria: this is the load-bearing scenario
//! for the entire feature. A real spec naming a binary path that does
//! not exist (`/usr/local/bin/no-such-binary`) drives a real
//! `tokio::process::Command::spawn` ENOENT, the cause-class
//! `TransitionReason::ExecBinaryNotFound { path }` payload travels
//! through the broadcast bus + the streaming projection + the
//! observation-store snapshot lane, and BOTH surfaces (`SubmitEvent`
//! NDJSON line + `AllocStatusResponse` snapshot) carry byte-equal
//! cause data — INCLUDING the typed `data: { path: ... }` payload, not
//! just the `kind` discriminator.
//!
//! Regression assertion (KPI-02): the streaming
//! `LifecycleTransition.reason` for the last `exec_binary_not_found`
//! event byte-equals the snapshot's `last_transition.reason` AND the
//! streaming `ConvergedFailed.error` byte-equals the snapshot's
//! per-row `error`. Each comparison is `==` on the typed Rust value,
//! which the type-identity pin (`SubmitEvent::LifecycleTransition.reason:
//! TransitionReason` and `TransitionRecord.reason: TransitionReason`)
//! makes structural rather than discipline.
//!
//! Linux-gated — production `ExecDriver` requires real
//! `tokio::process::Command::spawn`. macOS dev runs via `cargo xtask
//! lima run --` per `crates/overdrive-cli/CLAUDE.md`.

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use overdrive_cli::commands::alloc::StatusArgs;
use overdrive_cli::commands::job::SubmitArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_control_plane::api::TerminalReason;
use overdrive_core::TransitionReason;
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

const BROKEN_BINARY_PATH: &str = "/usr/local/bin/no-such-binary";

const fn broken_binary_spec_toml() -> &'static str {
    r#"
id = "doomed"
replicas = 1

[resources]
cpu_milli = 100
memory_bytes = 67108864

[exec]
command = "/usr/local/bin/no-such-binary"
args = []
"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn streaming_submit_against_missing_binary_emits_backoff_exhausted_with_cause_class_payload()
{
    let (handle, server_tmp) = spawn_server().await;
    let server_cfg = config_path(server_tmp.path());

    let spec_path = write_toml(server_tmp.path(), "doomed.toml", broken_binary_spec_toml());

    // Phase 1 — drive the streaming submit end-to-end. The reconciler
    // exhausts the restart budget (5 attempts in Phase 1), the streaming
    // handler observes `state == Failed && restart_budget.exhausted`,
    // and emits `ConvergedFailed { BackoffExhausted { attempts, cause:
    // ExecBinaryNotFound { path } } }`.
    let output = overdrive_cli::commands::job::submit_streaming(SubmitArgs {
        spec: spec_path,
        config_path: server_cfg.clone(),
    })
    .await
    .expect(
        "submit_streaming must complete end-to-end (the failure is on the workload, not the \
         submit lane)",
    );

    // KPI-02 — exit code 1 for the broken-binary scenario.
    assert_eq!(
        output.exit_code, 1,
        "S-WS-02: ConvergedFailed must map to exit code 1 per ADR-0032 §9; got {}",
        output.exit_code,
    );

    // The CLI's stdout (the `summary` field carries the rendered
    // operator-visible block) MUST contain the literal driver text and
    // the reproducer line.
    assert!(
        output.summary.contains("reproducer:"),
        "S-WS-02: summary must contain `reproducer:`; got:\n{}",
        output.summary,
    );

    // The terminal_reason carries the inner cause-class payload —
    // `BackoffExhausted { attempts, cause: ExecBinaryNotFound { path } }`.
    let terminal = output
        .terminal_reason
        .as_ref()
        .expect("S-WS-02: terminal_reason must be Some for ConvergedFailed");
    let cause_path = match terminal {
        TerminalReason::BackoffExhausted { attempts, cause } => {
            assert!(
                *attempts >= 1,
                "S-WS-02: BackoffExhausted.attempts must be >= 1; got {attempts}",
            );
            match cause {
                TransitionReason::ExecBinaryNotFound { path } => path.clone(),
                other => panic!(
                    "S-WS-02: BackoffExhausted.cause must be ExecBinaryNotFound; got {other:?}"
                ),
            }
        }
        TerminalReason::DriverError { cause } => match cause {
            TransitionReason::ExecBinaryNotFound { path } => path.clone(),
            other => panic!("S-WS-02: DriverError.cause must be ExecBinaryNotFound; got {other:?}"),
        },
        TerminalReason::Timeout { .. } => panic!(
            "S-WS-02: terminal_reason must NOT be Timeout — the regression target asserts the \
             reconciler reaches BackoffExhausted before the streaming cap fires"
        ),
        other => panic!("S-WS-02: unexpected terminal_reason variant: {other:?}"),
    };
    assert_eq!(
        cause_path, BROKEN_BINARY_PATH,
        "S-WS-02: cause-class typed payload must carry the verbatim spec path; got: {cause_path}",
    );

    // KPI-02 stream-side: streaming output records the cause-class
    // `TransitionReason::ExecBinaryNotFound { path: BROKEN_BINARY_PATH }`
    // verbatim. The `streaming_reason` field captures the LAST cause-class
    // TransitionReason observed on the streaming bus before terminal.
    let streaming_reason = output
        .streaming_reason
        .as_ref()
        .expect("S-WS-02: streaming_reason must be Some — the broadcast bus emitted at least one cause-class transition");
    let TransitionReason::ExecBinaryNotFound { path: streaming_path } = streaming_reason else {
        panic!(
            "S-WS-02: streaming_reason must be ExecBinaryNotFound (the last cause-class \
             transition before BackoffExhausted); got {streaming_reason:?}",
        );
    };
    assert_eq!(
        streaming_path, BROKEN_BINARY_PATH,
        "S-WS-02: streaming-side path must equal {BROKEN_BINARY_PATH}; got {streaming_path}",
    );

    // The verbatim driver error captured on the streaming side.
    let streaming_error = output
        .streaming_error
        .as_ref()
        .expect("S-WS-02: streaming_error must be Some on ConvergedFailed");

    // Phase 2 — drive `alloc::status --job doomed` against the same
    // observation store. The snapshot must carry the cause-class payload
    // byte-equal to the streaming surface.
    let snapshot = overdrive_cli::commands::alloc::status_snapshot(StatusArgs {
        job: "doomed".to_owned(),
        config_path: server_cfg,
    })
    .await
    .expect("alloc::status_snapshot must return a typed AllocStatusResponse");

    // Snapshot's restart budget must reflect exhaustion.
    let budget = snapshot
        .restart_budget
        .expect("S-WS-02: snapshot.restart_budget must be Some after backoff exhausts");
    assert!(
        budget.exhausted,
        "S-WS-02: snapshot.restart_budget.exhausted must be true; got {budget:?}",
    );

    // KPI-02 — byte-equal cause-class payload across surfaces. This is
    // the load-bearing assertion for the entire feature: the streaming
    // and snapshot lanes share the SAME `TransitionReason` enum (per the
    // type-identity test in
    // `crates/overdrive-control-plane/tests/acceptance/transition_reason_type_identity.rs`),
    // so the comparison is structural rather than convention.
    let row = snapshot.rows.first().expect("S-WS-02: snapshot must carry at least one alloc row");
    let last_transition = row
        .last_transition
        .as_ref()
        .expect("S-WS-02: row.last_transition must be Some after at least one transition");

    assert_eq!(
        &last_transition.reason, streaming_reason,
        "S-WS-02 KPI-02: snapshot.last_transition.reason MUST byte-equal the streaming \
         LifecycleTransition.reason — including the typed `path` payload, not just the \
         `kind` discriminator. snapshot={:?}, stream={streaming_reason:?}",
        last_transition.reason,
    );

    // Per-row `error` byte-equality with the streaming `error` field.
    // The streaming `error` carries the verbatim driver-text rendering;
    // the snapshot's per-row `error` is the same source. Both populated
    // from the action shim's `record_lifecycle` path with the same
    // `Display` of the underlying `DriverError`.
    let row_error = row.error.as_ref().expect(
        "S-WS-02: row.error must be Some for a Failed row whose backoff exhausted (the \
         verbatim driver text)",
    );
    assert_eq!(
        row_error, streaming_error,
        "S-WS-02 KPI-02: snapshot.row.error MUST byte-equal streaming.error (the verbatim \
         driver text). snapshot={row_error:?}, stream={streaming_error:?}",
    );

    handle.shutdown().await.expect("clean shutdown");
}
